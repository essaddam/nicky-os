//! The real publish boundary, real public HTTP surfaces, synthetic data only.
use super::*;
use moli_core::{InstanceId, Origin, PointId, Value};
use moli_runtime::{Hub, HubOptions, spawn_driver};
use serde_json::{Value as Json, json};
use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

type Frame = (Publish, oneshot::Sender<()>);
struct Fixture {
    receiver: tokio::sync::Mutex<mpsc::Receiver<Frame>>,
}
fn definitions() -> Vec<u8> {
    serde_json::to_vec(&json!([{"ieee_address":"panel","friendly_name":"panel","type":"EndDevice","definition":{"exposes":[
        {"property":"action_code","type":"text","access":1},
        {"property":"pin_code","type":"composite","features":[{"property":"pin_code","type":"text","access":7}]},
        {"property":"settings.pin","type":"text","access":7},
        {"property":"action","type":"enum","values":["disarm","arm_all_zones"],"access":1},
        {"property":"battery","type":"numeric","access":1},
        {"property":"error_code","type":"numeric","access":1}
    ]}}])).unwrap()
}
impl Driver for Fixture {
    fn kind(&self) -> &'static str {
        "z2m"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let mut catalog = Catalog::default();
            on_publish(
                ctx,
                "fixture",
                &mut catalog,
                &Publish::new("fixture/bridge/devices", QoS::AtMostOnce, definitions()),
            );
            ctx.ready();
            let mut frames = self.receiver.lock().await;
            loop {
                tokio::select! {
                    frame = frames.recv() => match frame {
                        Some((publish, done)) => { on_publish(ctx,"fixture",&mut catalog,&publish); let _=done.send(()); },
                        None => return Ok(()),
                    },
                    request = ctx.next_command() => match request {
                        Some(request) => request.reply(Err("synthetic driver never sends commands".into())),
                        None => return Ok(()),
                    },
                }
            }
        })
    }
}
async fn send(sender: &mpsc::Sender<Frame>, publish: Publish) {
    let (tx, rx) = oneshot::channel();
    sender.send((publish, tx)).await.unwrap();
    rx.await.unwrap();
}
fn packet(code: &str, retained: bool, battery: u64) -> Publish {
    let mut publish=Publish::new("fixture/panel",QoS::AtMostOnce,
        serde_json::to_vec(&json!({"action":"disarm","action_code":code,"action_transaction":8,"battery":battery,"error_code":7})).unwrap());
    publish.retain = retained;
    publish
}
#[derive(Clone)]
struct Logs(Arc<Mutex<Vec<u8>>>);
impl Write for Logs {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
async fn request(
    address: String,
    method: &'static str,
    path: String,
    body: Option<Json>,
) -> String {
    tokio::task::spawn_blocking(move || {
        let mut stream=std::net::TcpStream::connect(&address).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let body=body.map_or_else(String::new,|body|body.to_string());
        write!(stream,"{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\n\r\n{body}",body.len()).unwrap();
        let mut bytes=Vec::new(); stream.read_to_end(&mut bytes).unwrap();
        let raw=String::from_utf8(bytes).unwrap();
        assert!(raw.starts_with("HTTP/1.1 200"),"synthetic HTTP request failed");
        raw
    }).await.unwrap()
}

// One fixture follows the same generated credential across all real sinks.
#[allow(clippy::too_many_lines)]
#[tokio::test]
async fn private_inputs_are_bounded_and_absent_from_every_public_surface() {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let code = format!("0{stamp}");
    let root =
        std::env::temp_dir().join(format!("moli-z2m-privacy-{}-{stamp}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let logs = Arc::new(Mutex::new(Vec::new()));
    let capture = logs.clone();
    let _trace = tracing::subscriber::set_default(
        tracing_subscriber::fmt()
            .with_ansi(false)
            .without_time()
            .with_max_level(tracing::Level::TRACE)
            .with_writer(move || Logs(capture.clone()))
            .finish(),
    );
    let hub = Hub::new(HubOptions {
        state_path: Some(root.join("state.json")),
        journal_path: Some(root.join("journal.jsonl")),
        ..HubOptions::default()
    })
    .unwrap();
    let cancel = CancellationToken::new();
    let options = moli_history::Options {
        path: root.join("history.db"),
        filter: moli_history::Filter::default(),
        retention: moli_history::Retention::uniform(30),
    };
    let (history, recorder) = moli_history::History::start(&hub, &options, cancel.clone()).unwrap();
    let mut events = hub.subscribe();
    let (tx, rx) = mpsc::channel(1);
    let driver = spawn_driver(
        &hub,
        InstanceId::from("fixture"),
        Arc::new(Fixture {
            receiver: tokio::sync::Mutex::new(rx),
        }),
        cancel.clone(),
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while hub.stats().drivers_running != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    send(&tx, packet(&code, false, 81)).await;
    assert_eq!(hub.rejected_keypad_inputs(), 1);
    let mut private = hub.take_keypad_inputs().unwrap();
    assert!(
        private.try_recv().is_err(),
        "an unconsumed code was buffered"
    );
    assert!(
        hub.take_keypad_inputs().is_none(),
        "a second consumer registered"
    );
    send(&tx, packet(&code, true, 81)).await;
    assert!(private.try_recv().is_err(), "retained input was replayed");
    send(&tx, packet(&code, false, 81)).await;
    let input = private.try_recv().unwrap();
    assert!(
        input.code.expose_for_verification() == code,
        "private code changed"
    );
    assert!(input.action == "disarm" && input.transaction == Some(8));
    assert!(
        !format!("{input:?}").contains(&code),
        "private input Debug exposed its code"
    );
    drop(input);
    let numeric = u64::try_from(stamp % 9_000_000_000_000_000).unwrap() + 1_000_000_000_000_000;
    send(
        &tx,
        Publish::new(
            "fixture/panel",
            QoS::AtMostOnce,
            serde_json::to_vec(
                &json!({"action":"disarm","action_code":numeric,"action_transaction":9}),
            )
            .unwrap(),
        ),
    )
    .await;
    let input = private.try_recv().unwrap();
    assert!(
        input.code.expose_for_verification() == numeric.to_string(),
        "numeric credential changed"
    );
    drop(input);
    // Every ambiguous, malformed or oversized representation stays off the channel.
    for value in [
        json!({"action":"disarm","action_code":code,"pin_code":code}),
        json!({"action_code":code}),
        json!({"action":"disarm","action_code":[code]}),
        json!({"action":"disarm","action_code":code.repeat(4)}),
        json!({"action":{"pin_code":code},"battery":81}),
        json!({"action":"disarm","pin_code":{"pin_code":code}}),
        json!({"action":"disarm","settings":{"pin":code}}),
        json!({"action":format!(r#"{{"pin":"{code}","tail":}}"#),"battery":81}),
        json!({"action":format!(r#"{{"tail":,"pin":"{code}"}}"#),"battery":81}),
        json!({"action":format!(r#"{{"pin":"{code}","n":1e400}}"#),"battery":81}),
        json!({"action":format!(r#"{{"n":1e400,"pin":"{code}"}}"#),"battery":81}),
        json!({"action":format!(r#"{{"pin":"{code}"}} trailing"#),"battery":81}),
    ] {
        send(
            &tx,
            Publish::new(
                "fixture/panel",
                QoS::AtMostOnce,
                serde_json::to_vec(&value).unwrap(),
            ),
        )
        .await;
        assert!(
            private.try_recv().is_err(),
            "malformed private input accepted"
        );
        assert!(
            !serde_json::to_string(&hub.snapshot())
                .unwrap()
                .contains(&code),
            "outer-valid MQTT frame exposed a code in public state"
        );
    }
    send(
        &tx,
        Publish::new(
            "fixture/panel",
            QoS::AtMostOnce,
            format!("{{\"action_code\":\"{code}\""),
        ),
    )
    .await;
    send(
        &tx,
        Publish::new(
            "fixture/bridge/devices",
            QoS::AtMostOnce,
            serde_json::to_vec(&code).unwrap(),
        ),
    )
    .await;
    let refused = hub.rejected_keypad_inputs();
    for _ in 0..17 {
        send(&tx, packet(&code, false, 81)).await;
    }
    assert_eq!(
        hub.rejected_keypad_inputs(),
        refused + 1,
        "private queue is not bounded to 16"
    );
    drop(private);
    send(&tx, packet(&code, false, 81)).await;
    let mut private = hub.take_keypad_inputs().unwrap();
    assert!(
        private.try_recv().is_err(),
        "old consumer inputs survived re-registration"
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let app = moli_api::router(
        hub.clone(),
        cancel.clone(),
        &moli_api::Options {
            history: Some(history.clone()),
            ..moli_api::Options::default()
        },
    )
    .into_make_service_with_connect_info::<std::net::SocketAddr>();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    // Observe the real SSE stream while another live frame changes a public value.
    let sse_address = address.clone();
    let (ready_tx, ready_rx) = oneshot::channel();
    let sse = tokio::task::spawn_blocking(move || {
        let mut stream = std::net::TcpStream::connect(&sse_address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        write!(
            stream,
            "GET /api/events HTTP/1.1\r\nHost: {sse_address}\r\nAccept: text/event-stream\r\n\r\n"
        )
        .unwrap();
        let mut raw = Vec::new();
        let mut buf = [0; 4096];
        let mut ready = Some(ready_tx);
        loop {
            let count = stream.read(&mut buf).unwrap();
            if count == 0 {
                break;
            }
            raw.extend_from_slice(&buf[..count]);
            if raw.windows(4).any(|part| part == b"\r\n\r\n")
                && let Some(ready) = ready.take()
            {
                ready.send(()).unwrap();
            }
            if String::from_utf8_lossy(&raw).contains("\"value\":82") {
                break;
            }
        }
        let raw = String::from_utf8(raw).unwrap();
        assert!(raw.starts_with("HTTP/1.1 200"), "SSE request failed");
        raw
    });
    ready_rx.await.unwrap();
    send(&tx, packet(&code, false, 82)).await;
    let sse = sse.await.unwrap();
    let received = private.try_recv().unwrap();
    // Never use assert_eq! here: its failure would print the credential.
    assert!(
        received.code.expose_for_verification() == code,
        "private code changed"
    );
    drop(received);
    assert_eq!(
        hub.state(&PointId::from("fixture:panel/battery"))
            .unwrap()
            .value,
        Value::Int(82)
    );
    // A legacy point cannot inject a code into an unknown-point command's journal.
    assert!(
        hub.command(
            &PointId::from("fixture:panel/action_code"),
            Value::from(code.as_str()),
            Origin::Mcp,
            None
        )
        .await
        .is_err()
    );
    hub.persist_state().await.unwrap();
    tokio::time::timeout(Duration::from_secs(6), async {
        loop {
            let rows = history
                .sql(
                    "SELECT point,num,txt FROM history WHERE num=82 LIMIT 10",
                    10,
                )
                .await
                .unwrap();
            if !rows.rows.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let mut outputs = vec![
        sse,
        serde_json::to_string(&hub.snapshot()).unwrap(),
        serde_json::to_string(&hub.journal(100)).unwrap(),
    ];
    for path in [
        "/api/devices",
        "/api/journal",
        "/api/history?point=fixture:panel/battery&hours=1",
    ] {
        let output = request(address.clone(), "GET", path.into(), None).await;
        let expected = if path == "/api/journal" {
            "keypad_input"
        } else {
            "battery"
        };
        assert!(
            output.contains(expected),
            "legitimate public HTTP control missing"
        );
        outputs.push(output);
    }
    for name in ["list_devices", "history_sql"] {
        let arguments = if name == "list_devices" {
            json!({})
        } else {
            json!({"sql":"SELECT point,num,txt FROM history LIMIT 100"})
        };
        let output = request(address.clone(),"POST","/mcp".into(),Some(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":arguments}}))).await;
        assert!(
            output.contains("fixture:panel") && output.contains("battery"),
            "MCP tool did not return the legitimate control"
        );
        assert!(
            !output.contains("\"error\":") && !output.contains("\"isError\":true"),
            "MCP tool returned an error"
        );
        outputs.push(output);
    }
    while let Ok(event) = events.try_recv() {
        outputs.push(serde_json::to_string(&event).unwrap());
    }
    outputs.push(String::from_utf8(std::fs::read(root.join("state.json")).unwrap()).unwrap());
    outputs.push(String::from_utf8(logs.lock().unwrap().clone()).unwrap());
    assert!(
        outputs
            .iter()
            .all(|output| !output.contains(&code) && !output.contains(&numeric.to_string())),
        "credential reached public state/cache/bus/REST/SSE/MCP/journal/logs"
    );
    assert!(
        outputs.iter().any(|output| output.contains("battery")),
        "legitimate public state disappeared"
    );
    cancel.cancel();
    driver.await.unwrap();
    recorder.await.unwrap().unwrap();
    server.abort();
    let _ = server.await;
    drop(history);
    drop(hub);
    std::fs::remove_dir_all(root).unwrap();
}
