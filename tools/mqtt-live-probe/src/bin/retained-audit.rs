//! Independent read-only retained audit. Optional private snapshot is explicit,
//! mode 0600, outside the public checkout; stdout contains only counts/timings.
use anyhow::{Context, Result, ensure};
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use serde_json::{Value as Json, json};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mut host = None;
    let mut snapshot = None;
    let mut seconds = 5_u64;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--host" => host = args.next(),
            "--snapshot" => snapshot = args.next().map(PathBuf::from),
            "--seconds" => seconds = args.next().context("missing seconds")?.parse()?,
            _ => anyhow::bail!("unknown argument"),
        }
    }
    ensure!(
        (2..=30).contains(&seconds),
        "audit duration must be 2..30 seconds"
    );
    let password = std::env::var("MQTT_PASSWORD").context("MQTT_PASSWORD is required")?;
    ensure!(
        !password.is_empty() && !password.starts_with("{{secret:"),
        "unresolved MQTT_PASSWORD"
    );
    let mut options = MqttOptions::new(
        format!("moli-retained-audit-{}", std::process::id()),
        host.context("host required")?,
        1883,
    );
    options.set_credentials("test-mqtt", password);
    options.set_max_packet_size(4 * 1024 * 1024, 1024);
    let (client, mut events) = AsyncClient::new(options, 16);
    let end = Instant::now() + Duration::from_secs(seconds);
    let mut documents = BTreeMap::<String, Json>::new();
    let mut retained = 0;
    let mut malformed = 0;
    let mut legacy = 0;
    while Instant::now() < end {
        let Ok(event) =
            tokio::time::timeout(end.saturating_duration_since(Instant::now()), events.poll())
                .await
        else {
            break;
        };
        match event.context("audit transport failed")? {
            Event::Incoming(Packet::ConnAck(_)) => {
                client.try_subscribe("moli-probe/maison/#", QoS::AtLeastOnce)?
            }
            Event::Incoming(Packet::SubAck(ack)) => ensure!(
                ack.return_codes
                    .iter()
                    .all(|c| matches!(c, rumqttc::SubscribeReasonCode::Success(_))),
                "audit subscription refused"
            ),
            Event::Incoming(Packet::Publish(p)) => {
                if p.retain {
                    retained += 1;
                }
                if p.payload.is_empty() {
                    documents.remove(&p.topic);
                    continue;
                }
                if documents.len() >= 4096 {
                    anyhow::bail!("audit topic bound reached");
                }
                match serde_json::from_slice::<Json>(&p.payload) {
                    Ok(data) if data["v"] == 2 || data["v"] == 3 => {
                        documents.insert(p.topic, data);
                    }
                    Ok(_) => legacy += 1,
                    Err(_) => malformed += 1,
                }
            }
            _ => {}
        }
    }
    let bridges: BTreeSet<String> = documents
        .values()
        .filter_map(|d| d["pont"].as_str().map(str::to_owned))
        .collect();
    let mut reports = Vec::new();
    for bridge in bridges {
        let config = documents.get(&format!("{bridge}/config"));
        let mut hydrated: Vec<Json> = documents
            .values()
            .filter(|d| d["pont"] == bridge)
            .cloned()
            .collect();
        if let Some(config) = config.filter(|c| c["v"] == 3) {
            for node in hydrated.iter_mut().filter(|d| d["type"] == "noeud") {
                if let Some(info) = config["noeuds"]
                    .as_array()
                    .and_then(|a| a.iter().find(|n| n["id"] == node["id"]))
                {
                    for key in ["genre", "parent", "nom"] {
                        node[key] = info[key].clone();
                    }
                }
            }
            for device in config["appareils"].as_array().into_iter().flatten() {
                let mut d = device.clone();
                d["type"] = json!("appareil");
                let channels: Vec<Json> = device["canaux"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|id| {
                        let id = id.as_str()?;
                        let mut c = config["canaux"][id].clone();
                        if let Some(state) = hydrated
                            .iter()
                            .find(|p| p["type"] == "canal" && p["id"] == id)
                        {
                            c.as_object_mut()?.extend(state.as_object()?.clone());
                        }
                        c.as_object_mut()?.entry("valeur").or_insert(Json::Null);
                        Some(c)
                    })
                    .collect();
                d["canaux"] = json!(channels);
                hydrated.push(d);
            }
            for system in hydrated.iter_mut().filter(|d| d["type"] == "systeme") {
                for gauge in system["canaux"].as_array_mut().into_iter().flatten() {
                    if let Some(info) = config["systeme"]
                        .as_array()
                        .and_then(|a| a.iter().find(|m| m["id"] == gauge["id"]))
                    {
                        gauge["unite"] = info["unite"].clone();
                    }
                }
            }
        }
        let docs: Vec<_> = hydrated.iter().collect();
        let nodes: BTreeMap<String, &Json> = docs
            .iter()
            .filter(|d| d["type"] == "noeud")
            .filter_map(|d| Some((d["id"].as_str()?.into(), *d)))
            .collect();
        let devices: Vec<_> = docs
            .iter()
            .filter(|d| d["type"] == "appareil")
            .copied()
            .collect();
        let channels: Vec<_> = devices
            .iter()
            .filter_map(|d| d["canaux"].as_array())
            .flatten()
            .collect();
        let parent_errors = nodes
            .values()
            .filter(|n| n["parent"].as_str().is_some_and(|p| !nodes.contains_key(p)))
            .count();
        let orphan_channels = channels
            .iter()
            .filter(|c| !nodes.contains_key(c["piece"].as_str().unwrap_or("")))
            .count();
        let label_errors = channels
            .iter()
            .filter(|c| {
                c["nom"].as_str().is_none_or(str::is_empty)
                    || !c.as_object().is_some_and(|c| c.contains_key("unite"))
                    || !c.as_object().is_some_and(|c| c.contains_key("valeur"))
            })
            .count();
        let duplicates = channels.len()
            - channels
                .iter()
                .filter_map(|c| c["id"].as_str())
                .collect::<BTreeSet<_>>()
                .len();
        let system = docs.iter().find(|d| d["type"] == "systeme");
        let gauges = system.and_then(|s| s["canaux"].as_array());
        let means: BTreeMap<_, _> = gauges
            .into_iter()
            .flatten()
            .filter_map(|c| {
                let id = c["id"].as_str()?;
                matches!(
                    id,
                    "transformation_moyenne" | "calcul_moyen" | "reponse_moyenne"
                )
                .then(|| {
                    (
                        id,
                        json!({"value":c["valeur"],"unit":c["unite"],"samples":c["echantillons"]}),
                    )
                })
            })
            .collect();
        reports.push(json!({"bridge_index":reports.len(),"config":config.is_some(),"nodes":nodes.len(),"rooms":nodes.values().filter(|n|n["genre"]=="piece").count(),"floors":nodes.values().filter(|n|n["genre"]=="etage").count(),"expected_devices":config.and_then(|c|c["appareils"].as_array()).map(Vec::len),"received_devices":devices.len(),"physical_devices":devices.iter().filter(|d|d["genre"]=="physique").count(),"channels":channels.len(),"observed_channels":channels.iter().filter(|c|!c["recu_ms"].is_null()).count(),"multichannel_devices":devices.iter().filter(|d|d["canaux"].as_array().is_some_and(|c|c.len()>1)).count(),"parent_errors":parent_errors,"orphan_channels":orphan_channels,"label_errors":label_errors,"duplicate_channels":duplicates,"telemetry_channels":gauges.map_or(0,Vec::len),"command_contracts":channels.iter().filter(|c|c["commande"].is_object()).count(),"channel_states":docs.iter().filter(|d|d["type"]=="canal").count(),"metadata_bytes":config.map_or(0,|c|c.to_string().len()),"dynamic_bytes":documents.values().filter(|d|d["pont"]==bridge&&d["type"]!="description").map(|d|d.to_string().len()).sum::<usize>(),"means":means}));
    }
    if let Some(path) = snapshot {
        let parent = path
            .parent()
            .context("snapshot parent required")?
            .canonicalize()?;
        let checkout = std::env::current_dir()?.canonicalize()?;
        ensure!(
            !parent.starts_with(checkout),
            "private snapshot must stay outside the checkout"
        );
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(path)
            .context("private snapshot cannot be created")?;
        serde_json::to_writer(&mut file, &documents)?;
        file.write_all(b"\n")?;
    }
    println!(
        "{}",
        serde_json::to_string(
            &json!({"retained":retained,"documents":documents.len(),"malformed":malformed,"legacy_ignored":legacy,"bridges":reports})
        )?
    );
    Ok(())
}
