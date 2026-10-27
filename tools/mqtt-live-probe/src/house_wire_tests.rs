//! Loopback proof of the actual MQTT transport, including pre-registry retained
//! replay, typed projection, publication and echo. No household data or secret.
use super::*;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

fn write_frame(stream: &mut TcpStream, header: u8, body: &[u8]) {
    let mut frame = vec![header];
    let mut remaining = body.len();
    loop {
        let mut b = (remaining % 128) as u8;
        remaining /= 128;
        if remaining > 0 {
            b |= 128;
        }
        frame.push(b);
        if remaining == 0 {
            break;
        }
    }
    frame.extend(body);
    stream.write_all(&frame).unwrap();
}

fn read_frame(stream: &mut TcpStream) -> std::io::Result<(u8, Vec<u8>)> {
    let mut byte = [0];
    stream.read_exact(&mut byte)?;
    let header = byte[0];
    let mut length = 0;
    let mut factor = 1;
    for _ in 0..4 {
        stream.read_exact(&mut byte)?;
        length += usize::from(byte[0] & 127) * factor;
        if byte[0] & 128 == 0 {
            break;
        }
        factor *= 128;
    }
    assert!(length < 1024 * 1024);
    let mut body = vec![0; length];
    stream.read_exact(&mut body)?;
    Ok((header, body))
}

fn publish(stream: &mut TcpStream, topic: &str, payload: &[u8], retained: bool) {
    let mut body = (topic.len() as u16).to_be_bytes().to_vec();
    body.extend(topic.bytes());
    body.extend(payload);
    write_frame(stream, if retained { 0x31 } else { 0x30 }, &body);
}

#[tokio::test]
async fn real_mqtt_loopback_replays_all_channels_and_publishes_metrics_with_echo() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let received = Arc::new(Mutex::new(Vec::<(String, Json)>::new()));
    let out = received.clone();
    let broker = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut subscriptions = 0;
        while let Ok((header, body)) = read_frame(&mut stream) {
            match header >> 4 {
                1 => write_frame(&mut stream, 0x20, &[0, 0]),
                8 => {
                    write_frame(&mut stream, 0x90, &[body[0], body[1], 1]);
                    subscriptions += 1;
                    if subscriptions == 3 {
                        publish(
                            &mut stream,
                            "homeassistant/statestream/sensor/ambient_temperature/state",
                            b"21.5",
                            true,
                        );
                        let registry = json!({"floors":[{"floor_id":"f1","name":"Niveau"}],"areas":[{"id":"r1","name":"Séjour","floor_id":"f1"}],"devices":[{"id":"d1","name":"Ambiance","area_id":"r1"}],"entities":[{"id":"c1","entity_id":"sensor.ambient_temperature","device_id":"d1","original_device_class":"temperature","unit_of_measurement":"°C"},{"id":"c2","entity_id":"sensor.ambient_humidity","device_id":"d1","original_device_class":"humidity","unit_of_measurement":"%"}]});
                        publish(
                            &mut stream,
                            "moli-probe/source/registry",
                            registry.to_string().as_bytes(),
                            true,
                        );
                        publish(
                            &mut stream,
                            "homeassistant/statestream/sensor/ambient_humidity/state",
                            b"48",
                            true,
                        );
                    }
                }
                3 => {
                    let length = usize::from(u16::from_be_bytes([body[0], body[1]]));
                    let topic = std::str::from_utf8(&body[2..2 + length]).unwrap();
                    let qos = (header >> 1) & 3;
                    let offset = 2 + length + if qos > 0 { 2 } else { 0 };
                    if qos == 1 {
                        write_frame(&mut stream, 0x40, &body[2 + length..offset]);
                    }
                    let payload = &body[offset..];
                    let parsed: Json = serde_json::from_slice(payload).unwrap();
                    out.lock().unwrap().push((topic.into(), parsed));
                    publish(&mut stream, topic, payload, false);
                }
                12 => write_frame(&mut stream, 0xd0, &[]),
                14 => break,
                _ => {}
            }
        }
    });
    let config = crate::Config {
        hosts: vec!["127.0.0.1".into()],
        port,
        username: "fixture".into(),
        base: "zigbee2mqtt".into(),
        seconds: 5,
        bridges: 1,
        benchmark: false,
        all_topics: false,
        republish: Some("moli-probe".into()),
        republish_retain: true,
        republish_clear: false,
        virtual_tree: true,
        self_test_commands: false,
        virtual_mirror: true,
        publish_config: true,
        destination: None,
        metadata_memory: false,
    };
    let hub = moli_runtime::Hub::new(moli_runtime::HubOptions::default()).unwrap();
    let cancel = tokio_util::sync::CancellationToken::new();
    let run = crate::start_bridge(&hub, &config, "fixture", 0, cancel.clone()).unwrap();
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            if received.lock().unwrap().iter().any(|(topic, p)| {
                topic.ends_with("/systeme/state")
                    && p["canaux"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|c| c["id"] == "echos_recus" && c["valeur"].as_u64().unwrap_or(0) > 0)
            }) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("typed retained publication and subsequent telemetry must arrive");
    cancel.cancel();
    run.driver.await.unwrap();
    run.events.await.unwrap();
    broker.join().unwrap();
    let outputs = received.lock().unwrap();
    let latest: BTreeMap<_, _> = outputs
        .iter()
        .filter(|(_, p)| p["type"] == "canal")
        .map(|(_, p)| (p["id"].as_str().unwrap(), p))
        .collect();
    let channels: Vec<_> = latest.values().copied().collect();
    let metadata = &outputs
        .iter()
        .rev()
        .find(|(t, _)| t.ends_with("/config"))
        .unwrap()
        .1;
    assert_eq!(channels.len(), 2);
    assert!(
        channels.iter().any(
            |c| metadata["canaux"][c["id"].as_str().unwrap()]["unite"] == "°C"
                && c["valeur"] == 21.5
        )
    );
    assert!(channels.iter().any(
        |c| metadata["canaux"][c["id"].as_str().unwrap()]["unite"] == "%" && c["valeur"] == 48
    ));
    let room = outputs
        .iter()
        .rev()
        .find(|(_, p)| p["type"] == "noeud" && p["id"] == id("piece", "r1"))
        .unwrap();
    assert_eq!(
        metadata["noeuds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["genre"] == "piece")
            .unwrap()["parent"],
        id("etage", "f1")
    );
    assert!(
        channels
            .iter()
            .all(|c| c.get("nom").is_none() && c.get("unite").is_none())
    );
    assert_eq!(metadata["stockage_description"], "fichier_prive");
    assert!(
        room.1["generiques"]
            .as_array()
            .unwrap()
            .iter()
            .any(|g| g["id"] == "temperature:°C" && g["valeur"] == 21.5)
    );
}
