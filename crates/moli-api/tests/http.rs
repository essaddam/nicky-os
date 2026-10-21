//! End-to-end: a real HTTP server over a hub with a fake driver.

#[path = "support/http.rs"]
mod http_client;

use std::sync::Arc;
use std::time::Duration;

use moli_core::{Access, Device, InstanceId, Kind, PointSpec, Semantic, Value};
use moli_runtime::{BoxFuture, Driver, DriverCtx, Hub, HubOptions, spawn_driver};
use serde_json::{Value as Json, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

struct Lamp;

impl Driver for Lamp {
    fn kind(&self) -> &'static str {
        "fake"
    }

    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let id = ctx.device_id("lamp");
            ctx.upsert_device(Device {
                id: id.clone(),
                instance: ctx.instance().clone(),
                native_name: "lamp".into(),
                manufacturer: None,
                model: Some("L1".into()),
                description: None,
                native_room: None,
                members: Vec::new(),
                points: vec![PointSpec {
                    key: "state".into(),
                    label: "State".into(),
                    kind: Kind::Binary,
                    access: Access {
                        read: true,
                        write: true,
                    },
                    unit: None,
                    semantic: Semantic::OnOff,
                }],
            });
            ctx.set_state(&id, "state", Value::Bool(false));
            ctx.ready();
            while let Some(cmd) = ctx.next_command().await {
                ctx.set_state(&cmd.device.id, &cmd.key, cmd.value.clone());
                cmd.reply(Ok(()));
            }
            Ok(())
        })
    }
}

async fn server() -> (String, Hub) {
    let hub = lamp_hub().await;
    let addr = serve(&hub, &moli_api::Options::default()).await;
    (addr, hub)
}

/// A hub with the fake lamp running.
async fn lamp_hub() -> Hub {
    let hub = Hub::new(HubOptions::default()).unwrap();
    spawn_driver(
        &hub,
        InstanceId::from("fake"),
        Arc::new(Lamp),
        CancellationToken::new(),
    );
    for _ in 0..200 {
        if hub.stats().drivers_running == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    hub
}

/// The API over `hub`, listening; its address.
async fn serve(hub: &Hub, options: &moli_api::Options) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let app = moli_api::router(hub.clone(), CancellationToken::new(), options);
    let app = app.into_make_service_with_connect_info::<std::net::SocketAddr>();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

/// Minimal HTTP/1.1 client: enough for tests, no extra dependency.
async fn request(addr: &str, method: &str, path: &str, body: Option<&Json>) -> (u16, String) {
    request_with(addr, method, path, body, "").await
}

/// `extra`: more header lines, each ending with a CRLF.
async fn request_with(
    addr: &str,
    method: &str,
    path: &str,
    body: Option<&Json>,
    extra: &str,
) -> (u16, String) {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let body = body.map(ToString::to_string).unwrap_or_default();
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\
         Content-Type: application/json\r\nAccept: application/json, text/event-stream\r\n\
         {extra}Content-Length: {}\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(body.as_bytes()).await.unwrap();
    let raw = http_client::read_response(&mut stream)
        .await
        .unwrap_or_else(|error| panic!("{method} {path}: {error}"));
    let raw = String::from_utf8_lossy(&raw).to_string();
    let status = raw[9..12].parse().unwrap();
    let (headers, body) = raw.split_once("\r\n\r\n").unwrap();
    let body = if headers
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        dechunk(body)
    } else {
        body.to_owned()
    };
    (status, body)
}

fn dechunk(mut body: &str) -> String {
    let mut out = String::new();
    while let Some((size, rest)) = body.split_once("\r\n") {
        let size = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
        if size == 0 {
            break;
        }
        out.push_str(&rest[..size]);
        body = &rest[size + 2..];
    }
    out
}

#[tokio::test]
async fn rest_command_round_trip() {
    let (addr, hub) = server().await;

    let (status, body) = request(&addr, "GET", "/api/devices", None).await;
    assert_eq!(status, 200);
    let snapshot: Json = serde_json::from_str(&body).unwrap();
    assert_eq!(snapshot["devices"][0]["id"], "fake:lamp");
    assert_eq!(snapshot["devices"][0]["state"]["state"]["value"], false);

    let (status, _) = request(
        &addr,
        "POST",
        "/api/command",
        Some(&json!({"point": "fake:lamp/state", "value": true})),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(
        hub.state(&"fake:lamp/state".into()).unwrap().value,
        Value::Bool(true)
    );

    let (status, body) = request(
        &addr,
        "POST",
        "/api/command",
        Some(&json!({"point": "fake:lamp/state", "value": "ON"})),
    )
    .await;
    assert_eq!(status, 422, "{body}");

    let (status, body) = request(&addr, "GET", "/api/health", None).await;
    assert_eq!(status, 200);
    assert_eq!(
        serde_json::from_str::<Json>(&body).unwrap()["stats"]["devices"],
        1
    );
}

#[tokio::test]
async fn auto_h1_rest_timer_requests_validate_duration_and_preserve_retries() {
    let hub = Hub::new(HubOptions::default()).unwrap();
    let config = serde_json::from_value(
        json!({"helper":[{"id":"clock","name":"Timer","kind":"timer","duration_s":3}]}),
    )
    .unwrap();
    let cancel = CancellationToken::new();
    spawn_driver(
        &hub,
        "fixture".into(),
        Arc::new(moli_helpers::Helpers::new(config).unwrap()),
        cancel.clone(),
    );
    for _ in 0..200 {
        if hub.state(&"fixture:clock/state".into()).is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let addr = serve(&hub, &moli_api::Options::default()).await;
    let command = |operation: &str, value: Json, sequence: u64| json!({"point":format!("fixture:clock/{operation}"),"value":value,"request_id":format!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:{sequence}")});
    for (body, expected) in [
        (command("start", json!(3), 1), 200),
        (command("start", json!(3), 1), 200),
        (command("start", json!(4), 1), 409),
        (command("start", json!(1.0), 2), 422),
        (command("start", json!(0), 2), 422),
        (command("start", json!(604_801), 2), 422),
        (command("cancel", Json::Null, 2), 200),
        (command("start", json!(3), 1), 200),
        (command("state", json!("running"), 3), 403),
    ] {
        let (status, response) = request(&addr, "POST", "/api/command", Some(&body)).await;
        assert_eq!(status, expected, "{body}: {response}");
    }
    assert_eq!(
        hub.state(&"fixture:clock/state".into()).unwrap().value,
        Value::Text("cancelled".into())
    );
    assert_eq!(
        hub.state(&"fixture:clock/generation".into()).unwrap().value,
        Value::Int(1)
    );
    cancel.cancel();
}

#[tokio::test]
async fn auto_h2_rest_counter_operations_preserve_intent_and_refusal() {
    let hub = Hub::new(HubOptions::default()).unwrap();
    let config = serde_json::from_value(json!({ "helper": [{
        "id": "count", "name": "Counter", "kind": "counter", "max": 1
    }] }))
    .unwrap();
    let cancel = CancellationToken::new();
    spawn_driver(
        &hub,
        "fixture".into(),
        Arc::new(moli_helpers::Helpers::new(config).unwrap()),
        cancel.clone(),
    );
    for _ in 0..200 {
        if hub.state(&"fixture:count/value".into()).is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let addr = serve(&hub, &moli_api::Options::default()).await;
    let command = |operation: &str, sequence: u64| {
        json!({
            "point": format!("fixture:count/{operation}"), "value": null,
            "request_id": format!("0123456789abcdef0123456789abcdef:{sequence}")
        })
    };
    for (body, expected) in [
        (command("increment", 1), 200),
        (command("increment", 2), 422),
        (command("reset", 3), 200),
        (command("increment", 1), 200),
        (command("increment", 2), 422),
        (command("reset", 1), 409),
        (json!({"point":"fixture:count/increment", "value":1}), 422),
        (json!({"point":"fixture:count/value", "value":1}), 403),
    ] {
        let (status, response) = request_with(
            &addr,
            "POST",
            "/api/command",
            Some(&body),
            "x-moli-origin: ui\r\n",
        )
        .await;
        assert_eq!(status, expected, "{body}: {response}");
    }
    assert_eq!(
        hub.state(&"fixture:count/value".into()).unwrap().value,
        Value::Int(0)
    );
    cancel.cancel();
}

#[tokio::test]
async fn auto_h4_rest_retries_do_not_publish_a_second_press() {
    let hub = Hub::new(HubOptions::default()).unwrap();
    let cancel = CancellationToken::new();
    let config = serde_json::from_value(json!({ "helper": [
        { "id": "button", "name": "Simulated button", "kind": "button" },
        { "id": "other", "name": "Another button", "kind": "button" }
    ] }))
    .unwrap();
    spawn_driver(
        &hub,
        "fixture".into(),
        Arc::new(moli_helpers::Helpers::new(config).unwrap()),
        cancel.clone(),
    );
    for _ in 0..200 {
        if hub.stats().drivers_running == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(hub.stats().drivers_running, 1);
    let addr = serve(&hub, &moli_api::Options::default()).await;
    let mut events = hub.subscribe();
    let order = json!({ "point": "fixture:button/press", "value": null,
                        "request_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:1" });
    for _ in 0..2 {
        let (status, body) = request(&addr, "POST", "/api/command", Some(&order)).await;
        assert_eq!(status, 200, "{body}");
    }
    let mut invalid = order.clone();
    invalid["value"] = true.into();
    assert_eq!(
        request(&addr, "POST", "/api/command", Some(&invalid))
            .await
            .0,
        422
    );
    invalid = order.clone();
    invalid["request_id"] = "invalid".into();
    assert_eq!(
        request(&addr, "POST", "/api/command", Some(&invalid))
            .await
            .0,
        422
    );
    invalid = order;
    invalid["point"] = "fixture:other/press".into();
    assert_eq!(
        request(&addr, "POST", "/api/command", Some(&invalid))
            .await
            .0,
        409
    );
    let mut count = 0;
    while let Ok(event) = events.try_recv() {
        count += usize::from(matches!(event, moli_core::Event::Occurrence { .. }));
    }
    assert_eq!(
        count, 1,
        "REST forwards the request identity through the real hub and driver"
    );
    assert!(hub.state(&"fixture:button/press".into()).is_none());
    cancel.cancel();
}

#[tokio::test]
async fn an_approved_routine_runs_from_the_household_dashboard_without_a_code() {
    use moli_automation::{Actor, Author, Automation, Automations};
    let hub = lamp_hub().await;
    let (autos, _engine) = Automations::start(
        &hub,
        moli_automation::Config::default(),
        None,
        CancellationToken::new(),
    )
    .await
    .unwrap();
    let person = Actor {
        human: true,
        author: Author::Human,
    };
    let routine: Automation = serde_json::from_value(json!({
        "id": "soir", "name": "Soir", "enabled": false,
        "graph": {
            "nodes": [
                { "id": "t", "type": "manual" },
                { "id": "s", "type": "set", "point": "fake:lamp/state", "value": true }
            ],
            "edges": [{ "from": "t", "to": "s" }]
        }
    }))
    .unwrap();
    let routine = autos.save(routine, person).await.unwrap();
    let fingerprint = routine.fingerprint();
    let options = moli_api::Options {
        automations: Some(autos.clone()),
        ..moli_api::Options::default()
    };
    let addr = serve(&hub, &options).await;
    let body = json!({ "fingerprint": fingerprint });

    // Not approved yet: even the dashboard cannot run it.
    let ui = "X-Moli-Origin: ui\r\n";
    let (status, _) =
        request_with(&addr, "POST", "/api/automations/soir/run", Some(&body), ui).await;
    assert_eq!(status, 422);

    autos.approve("soir", &fingerprint, person).await.unwrap();
    // Without the dashboard's mark (an agent): a person is needed.
    let (status, _) = request(&addr, "POST", "/api/automations/soir/run", Some(&body)).await;
    assert_eq!(status, 403);
    assert_eq!(
        hub.state(&"fake:lamp/state".into()).unwrap().value,
        Value::Bool(false)
    );
    // From the household's dashboard: a direct order, no code.
    let (status, body) =
        request_with(&addr, "POST", "/api/automations/soir/run", Some(&body), ui).await;
    assert_eq!(status, 200, "{body}");
    for _ in 0..200 {
        if hub.state(&"fake:lamp/state".into()).unwrap().value == Value::Bool(true) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("the routine did not switch the lamp on");
}

#[tokio::test]
async fn labels_via_rest() {
    let (addr, _hub) = server().await;
    let (status, body) = request(
        &addr,
        "PUT",
        "/api/labels/fake:lamp",
        Some(&json!({"name": "Lampe", "room": "Salon"})),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let (_, body) = request(&addr, "GET", "/api/devices/fake:lamp", None).await;
    assert_eq!(
        serde_json::from_str::<Json>(&body).unwrap()["label"]["room"],
        "Salon"
    );
}

#[tokio::test]
async fn foreign_host_is_rejected_everywhere() {
    let (addr, _hub) = server().await;
    for path in ["/api/devices", "/mcp", "/"] {
        let mut stream = tokio::net::TcpStream::connect(&addr).await.unwrap();
        let head =
            format!("GET {path} HTTP/1.1\r\nHost: evil.example\r\nConnection: close\r\n\r\n");
        stream.write_all(head.as_bytes()).await.unwrap();
        let mut raw = String::new();
        stream.read_to_string(&mut raw).await.unwrap();
        assert!(raw.starts_with("HTTP/1.1 403"), "{path}: {raw}");
    }
}

#[tokio::test]
async fn system_says_when_the_history_cannot_be_counted() {
    let dir = std::env::temp_dir().join(format!("moli-api-broken-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Arc::new(moli_history::Store::open(&dir.join("history.db")).unwrap());
    let options = moli_api::Options {
        history: Some(moli_history::History::from_store(store).unwrap()),
        bench_path: Some(dir.join("bench.json")),
        ..moli_api::Options::default()
    };
    rusqlite::Connection::open(dir.join("history.db"))
        .unwrap()
        .execute_batch("ALTER TABLE samples RENAME TO gone")
        .unwrap();
    let addr = serve(&Hub::new(HubOptions::default()).unwrap(), &options).await;
    let (status, body) = request(&addr, "GET", "/api/system", None).await;
    assert_eq!(status, 200, "{body}");
    let system: Json = serde_json::from_str(&body).unwrap();
    let facts = &system["bricks"][1]["facts"];
    assert!(
        facts["volume_error"]
            .as_str()
            .is_some_and(|e| e.contains("samples")),
        "{facts}"
    );
    assert!(
        facts.get("rows_24h").is_none(),
        "unknown, never zero: {facts}"
    );
    assert_eq!(facts["batches_written"], 0, "the counters are still there");
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn only_a_person_validates_a_history_reduction() {
    let dir = std::env::temp_dir().join(format!("moli-api-retention-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Arc::new(moli_history::Store::open(&dir.join("history.db")).unwrap());
    let options = moli_api::Options {
        history: Some(moli_history::History::from_store(store).unwrap()),
        ..moli_api::Options::default()
    };
    let addr = serve(&Hub::new(HubOptions::default()).unwrap(), &options).await;
    let (status, body) = request(&addr, "GET", "/api/history/retention", None).await;
    assert_eq!(status, 200, "{body}");
    let policies: Json = serde_json::from_str(&body).unwrap();
    assert_eq!(policies["configured"]["days"], 30);
    assert!(policies["reduction"].is_null(), "{policies}");
    // Without a PIN session, whatever it says it is: refused, before anything.
    let validate = json!({ "at": moli_core::now_ms(), "fingerprint": "x" });
    let (status, body) = request_with(
        &addr,
        "POST",
        "/api/history/retention/validate",
        Some(&validate),
        "x-moli-origin: ui\r\n",
    )
    .await;
    assert_eq!(status, 403, "{body}");
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn history_over_rest_and_mcp() {
    let dir = std::env::temp_dir().join(format!("moli-api-history-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Arc::new(moli_history::Store::open(&dir.join("history.db")).unwrap());
    let now = moli_core::now_ms();
    store
        .insert(
            &[
                ("fake:lamp/state".into(), Value::Bool(false), now - 60_000),
                ("fake:lamp/state".into(), Value::Bool(true), now - 30_000),
            ],
            false,
        )
        .unwrap();
    let hub = Hub::new(HubOptions::default()).unwrap();
    let options = moli_api::Options {
        history: Some(moli_history::History::from_store(store).unwrap()),
        bench_path: Some(dir.join("bench.json")),
        ..moli_api::Options::default()
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let app = moli_api::router(hub, CancellationToken::new(), &options)
        .into_make_service_with_connect_info::<std::net::SocketAddr>();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let (status, body) = request(
        &addr,
        "GET",
        "/api/history?point=fake:lamp/state&hours=1",
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let series: Json = serde_json::from_str(&body).unwrap();
    assert_eq!(series["total"], 2);
    assert_eq!(series["raw"][1][1], true);

    let call = rpc(
        1,
        "tools/call",
        &json!({"name": "history_sql", "arguments": {"sql": "SELECT COUNT(*) AS n FROM history WHERE num = 1"}}),
    );
    let (status, body) = request(&addr, "POST", "/mcp", Some(&call)).await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains(r#"\"rows\":[[1]]"#), "{body}");

    let write = rpc(
        2,
        "tools/call",
        &json!({"name": "history_sql", "arguments": {"sql": "DELETE FROM samples"}}),
    );
    let (_, body) = request(&addr, "POST", "/mcp", Some(&write)).await;
    assert!(body.contains("read-only"), "{body}");

    // How it grows: sizes, the last 24 hours, the chattiest points.
    let (status, body) = request(&addr, "GET", "/api/system", None).await;
    assert_eq!(status, 200, "{body}");
    let system: Json = serde_json::from_str(&body).unwrap();
    let facts = &system["bricks"][1]["facts"];
    assert_eq!(system["bricks"][1]["id"], "history");
    assert!(facts["db_bytes"].as_u64().unwrap() > 0, "{facts}");
    assert_eq!(facts["rows_24h"], 2);
    assert_eq!(facts["top_24h"], json!([["fake:lamp/state", 2]]));
    assert_eq!(facts["batches_failed"], 0);
    assert!(facts["query_ms"].is_number());
    #[cfg(unix)]
    assert!(facts["free_bytes"].as_u64().unwrap() > 0, "{facts}");
    let _ = std::fs::remove_dir_all(dir);
}

fn rpc(id: u64, method: &str, params: &Json) -> Json {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

#[tokio::test]
async fn mcp_tools_drive_the_home() {
    let (addr, hub) = server().await;

    let init = rpc(
        1,
        "initialize",
        &json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "0"}}),
    );
    let (status, body) = request(&addr, "POST", "/mcp", Some(&init)).await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("moli-os"), "{body}");

    let (status, body) = request(
        &addr,
        "POST",
        "/mcp",
        Some(&rpc(2, "tools/list", &json!({}))),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    for tool in [
        "system_status",
        "list_devices",
        "get_device",
        "send_command",
        "set_label",
        "get_journal",
    ] {
        assert!(body.contains(tool), "missing tool {tool}: {body}");
    }

    let call = rpc(
        3,
        "tools/call",
        &json!({"name": "send_command", "arguments": {"point": "fake:lamp/state", "value": true, "actor": "test-agent"}}),
    );
    let (status, body) = request(&addr, "POST", "/mcp", Some(&call)).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        hub.state(&"fake:lamp/state".into()).unwrap().value,
        Value::Bool(true)
    );
    let journal = hub.journal(1);
    assert_eq!(journal[0].actor.as_deref(), Some("test-agent"));
}

#[tokio::test]
async fn speaking_without_an_assistant_says_so() {
    let (addr, _hub) = server().await;
    let (status, body) = request(
        &addr,
        "POST",
        "/api/assistant/speak",
        Some(&json!({ "text": "Bonsoir" })),
    )
    .await;
    assert_eq!(status, 503, "{body}");
    let (status, body) = request(&addr, "GET", "/api/assistant", None).await;
    assert_eq!(status, 200);
    assert!(body.contains("\"speak\":false"), "{body}");
}
#[tokio::test]
async fn only_a_person_gives_the_assistant_a_key() {
    let (addr, _hub) = server().await;
    let (status, body) = request(
        &addr,
        "PUT",
        "/api/assistant/key",
        Some(&json!({ "api_key": "sk-proj-abcdefghijklmnopqrstuvwxyz" })),
    )
    .await;
    assert_eq!(status, 403, "{body}");
    let (status, _) = request(&addr, "GET", "/api/assistant/settings", None).await;
    assert_eq!(status, 503);
}

/// The response's `Set-Cookie` value for `name`, and the body.
async fn request_cookie(
    addr: &str,
    method: &str,
    path: &str,
    body: &Json,
    extra: &str,
    name: &str,
) -> (u16, Option<String>, String) {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let body = body.to_string();
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\
         Content-Type: application/json\r\n{extra}Content-Length: {}\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(body.as_bytes()).await.unwrap();
    let raw = http_client::read_response(&mut stream)
        .await
        .unwrap_or_else(|error| panic!("{method} {path}: {error}"));
    let raw = String::from_utf8_lossy(&raw).to_string();
    let status = raw[9..12].parse().unwrap();
    let (headers, body) = raw.split_once("\r\n\r\n").unwrap();
    let cookie = headers.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        (k.eq_ignore_ascii_case("set-cookie") && v.trim().starts_with(&format!("{name}=")))
            .then(|| v.trim().split(';').next().unwrap_or_default().to_owned())
    });
    (status, cookie, body.to_owned())
}

/// A human session opened with the house's code from this machine: the
/// header line that carries it.
async fn pin_session(addr: &str, pin: &str) -> String {
    let (status, cookie, body) = request_cookie(
        addr,
        "POST",
        "/api/session",
        &json!({ "pin": pin }),
        "",
        "moli_session",
    )
    .await;
    assert_eq!(status, 200, "{body}");
    format!("Cookie: {}\r\n", cookie.unwrap())
}

/// What the satellites heard: switched on and listened to by a person only,
/// never by any device of the house.
#[tokio::test]
async fn only_a_person_records_and_hears_the_satellites() {
    let dir = std::env::temp_dir().join(format!("moli-http-recordings-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let hub = lamp_hub().await;
    let moli = moli_assistant::Assistant::new(
        hub.clone(),
        None,
        None,
        None,
        moli_assistant::Config::default(),
    )
    .unwrap();
    moli.open_recordings(dir.join("recordings"));
    let name = format!("{}-7.wav", moli_core::now_ms());
    std::fs::write(dir.join("recordings").join(&name), b"RIFF....WAVE").unwrap();
    let options = moli_api::Options {
        ui_pin: Some(moli_api::UiPin::new("482915")),
        assistant: Some(moli.clone()),
        ..moli_api::Options::default()
    };
    let addr = serve(&hub, &options).await;
    let heard = format!("/api/assistant/recordings/{name}");
    let on = json!({ "days": 7 });
    let ui = "x-moli-origin: ui\r\n";

    // The household's dashboard without the code: nothing heard, nothing on.
    let (status, body) = request_with(&addr, "GET", &heard, None, ui).await;
    assert_eq!(status, 403, "{body}");
    let (status, body) =
        request_with(&addr, "PUT", "/api/assistant/recording", Some(&on), ui).await;
    assert_eq!(status, 403, "{body}");
    assert_eq!(moli.recording_until(), None);

    // A person (the code): both.
    let pin = pin_session(&addr, "482915").await;
    let (status, body) = request_with(&addr, "GET", &heard, None, &pin).await;
    assert_eq!(status, 200);
    assert!(body.starts_with("RIFF"));
    let (status, body) =
        request_with(&addr, "PUT", "/api/assistant/recording", Some(&on), &pin).await;
    assert_eq!(status, 200, "{body}");
    assert!(moli.recording_until().is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A phone, as the phones driver describes one: where it is, and a point
/// to notify it.
struct Phone;

impl Driver for Phone {
    fn kind(&self) -> &'static str {
        "phones"
    }

    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let id = ctx.device_id("p1");
            let spec = |key: &str, kind: Kind, write: bool| PointSpec {
                key: key.into(),
                label: key.into(),
                kind,
                access: Access {
                    read: !write,
                    write,
                },
                unit: None,
                semantic: Semantic::Other,
            };
            let number = Kind::Numeric {
                min: None,
                max: None,
                step: None,
            };
            ctx.upsert_device(Device {
                id: id.clone(),
                instance: ctx.instance().clone(),
                native_name: "phone".into(),
                manufacturer: None,
                model: None,
                description: None,
                native_room: None,
                members: Vec::new(),
                points: vec![
                    spec("latitude", number, false),
                    spec("push", Kind::Text, true),
                ],
            });
            ctx.set_state(&id, "latitude", Value::Float(48.85));
            ctx.ready();
            while let Some(cmd) = ctx.next_command().await {
                cmd.reply(Ok(()));
            }
            Ok(())
        })
    }
}

/// A house with a code and a store: a lamp in the guest room
/// (`fake:lamp`), one in the living room (`salon:lamp`), a phone labelled in
/// the guest room too (`tel:p1`, whose whereabouts are nobody else's), and a
/// guest of the guest room signed in from the Internet.
struct GuestHouse {
    addr: String,
    hub: Hub,
    dir: std::path::PathBuf,
    /// The owner's session (the house's code, from this machine).
    pin: String,
    /// The guest's headers: from the Internet, its session.
    guest: String,
}

const GUEST_ROOM: &str = "Chambre d'amis";

async fn guest_house(name: &str, more: impl FnOnce(&mut moli_api::Options)) -> GuestHouse {
    use moli_runtime::LabelPatch;
    let dir = std::env::temp_dir().join(format!("moli-http-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let hub = Hub::new(HubOptions {
        secrets: Some((
            dir.join("secrets.enc"),
            moli_runtime::MasterKey::generate().unwrap(),
        )),
        ..HubOptions::default()
    })
    .unwrap();
    let drivers: [(&str, Arc<dyn Driver>); 3] = [
        ("fake", Arc::new(Lamp)),
        ("salon", Arc::new(Lamp)),
        ("tel", Arc::new(Phone)),
    ];
    for (instance, driver) in drivers {
        spawn_driver(
            &hub,
            InstanceId::from(instance),
            driver,
            CancellationToken::new(),
        );
    }
    for _ in 0..200 {
        if hub.stats().drivers_running == 3 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    for (device, room) in [
        ("fake:lamp", GUEST_ROOM),
        ("salon:lamp", "Salon"),
        ("tel:p1", GUEST_ROOM),
    ] {
        let room = LabelPatch {
            name: None,
            room: Some(room.into()),
        };
        hub.set_label(&device.into(), room, moli_core::Origin::Ui, None)
            .await
            .unwrap();
    }
    let mut options = moli_api::Options {
        ui_pin: Some(moli_api::UiPin::new("482915")),
        home_path: Some(dir.join("home.json")),
        ..moli_api::Options::default()
    };
    more(&mut options);
    let addr = serve(&hub, &options).await;
    // The house's code proves the owner while nobody is one.
    let pin = pin_session(&addr, "482915").await;
    let invited = json!({ "name": "Invité", "role": "guest", "rooms": [GUEST_ROOM] });
    let (status, body) = request_with(&addr, "POST", "/api/people", Some(&invited), &pin).await;
    assert_eq!(status, 201, "{body}");
    let (status, body) = request_with(&addr, "POST", "/api/people/invite/invite", None, &pin).await;
    assert_eq!(status, 200, "{body}");
    let code = serde_json::from_str::<Json>(&body).unwrap()["code"].clone();
    let outside = "X-Forwarded-For: 203.0.113.9\r\n";
    let (status, cookie, body) = request_cookie(
        &addr,
        "POST",
        "/api/invitation",
        &json!({ "code": code, "password": "une valise bleue" }),
        outside,
        "moli_person",
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let guest = format!(
        "{outside}Cookie: {}\r\nx-moli-origin: ui\r\n",
        cookie.unwrap()
    );
    GuestHouse {
        addr,
        hub,
        dir,
        pin,
        guest,
    }
}

/// The ids of a snapshot's devices, sorted.
fn device_ids(snapshot: &Json) -> Vec<String> {
    let mut ids: Vec<String> = snapshot["devices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["id"].as_str().unwrap().to_owned())
        .collect();
    ids.sort();
    ids
}

/// A guest reads what is in its rooms, commands it, and nothing else of the
/// house: not the other rooms, not anyone's whereabouts, not the house's
/// settings, its energy, its journal or its assistant.
#[tokio::test]
async fn a_guest_reads_and_commands_its_rooms_only() {
    let store_dir = std::env::temp_dir().join(format!("moli-http-guest-h-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&store_dir);
    let store = Arc::new(moli_history::Store::open(&store_dir.join("h.db")).unwrap());
    let now = moli_core::now_ms();
    store
        .insert(
            &[
                ("fake:lamp/state".into(), Value::Bool(true), now - 60_000),
                ("salon:lamp/state".into(), Value::Bool(true), now - 60_000),
                ("tel:p1/latitude".into(), Value::Float(48.85), now - 60_000),
            ],
            false,
        )
        .unwrap();
    let house = guest_house("guest-reads", |options| {
        options.history = Some(moli_history::History::from_store(store).unwrap());
    })
    .await;
    let (addr, guest) = (&house.addr, &house.guest);
    let zone = json!([{ "id": "travail", "name": "Travail", "latitude": 48.0, "longitude": 2.0, "radius": 200.0 }]);
    let (status, body) = request_with(addr, "PUT", "/api/zones", Some(&zone), &house.pin).await;
    assert_eq!(status, 200, "{body}");
    let member = json!({ "name": "Sam", "role": "member" });
    let (status, body) = request_with(addr, "POST", "/api/people", Some(&member), &house.pin).await;
    assert_eq!(status, 201, "{body}");

    // The household sees the house; the guest, its room's lamp only.
    let (_, body) = request(addr, "GET", "/api/devices", None).await;
    assert_eq!(
        device_ids(&serde_json::from_str(&body).unwrap()),
        ["fake:lamp", "salon:lamp", "tel:p1"]
    );
    let (status, body) = request_with(addr, "GET", "/api/devices", None, guest).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        device_ids(&serde_json::from_str(&body).unwrap()),
        ["fake:lamp"]
    );
    for (path, expected) in [
        ("/api/devices/fake:lamp", 200),
        ("/api/devices/salon:lamp", 403),
        ("/api/devices/tel:p1", 403),
        ("/api/history?point=fake:lamp/state", 200),
        ("/api/history?point=salon:lamp/state", 403),
        ("/api/history?point=tel:p1/latitude", 403),
        ("/api/history?point=tel:p1", 403),
    ] {
        let (status, body) = request_with(addr, "GET", path, None, guest).await;
        assert_eq!(status, expected, "{path}: {body}");
    }

    // Itself among the people, and no place.
    let (status, body) = request_with(addr, "GET", "/api/people", None, guest).await;
    assert_eq!(status, 200, "{body}");
    let people: Json = serde_json::from_str(&body).unwrap();
    assert_eq!(people["people"].as_array().unwrap().len(), 1, "{body}");
    assert_eq!(people["people"][0]["id"], "invite");
    assert_eq!(people["zones"], json!([]));

    // Nothing of the house's settings, energy, journal or assistant.
    for path in [
        "/api/journal",
        "/api/assistant",
        "/api/assistant/exchanges",
        "/api/machines/machines:pc",
        "/api/energy",
        "/api/energy/series?step=day",
        "/api/bench",
        "/api/system",
        "/api/automations",
        "/api/mobile/phones",
        "/api/devices/fake:lamp/snapshot",
    ] {
        let (status, body) = request_with(addr, "GET", path, None, guest).await;
        assert_eq!(status, 403, "{path}: {body}");
    }
    let (status, _) = request_with(addr, "PUT", "/api/zones", Some(&zone), guest).await;
    assert_eq!(status, 403);
    // Its dashboard still opens: the session, the layout, the plan, the sky.
    for path in ["/api/session", "/api/health", "/api/home", "/api/plan"] {
        let (status, body) = request_with(addr, "GET", path, None, guest).await;
        assert_eq!(status, 200, "{path}: {body}");
    }
    // (Not built here, or no position: not refused all the same.)
    for path in ["/", "/api/weather/forecast"] {
        let (status, body) = request_with(addr, "GET", path, None, guest).await;
        assert_ne!(status, 403, "{path}: {body}");
    }

    // It commands its room, nothing else.
    for (point, value, expected) in [
        ("fake:lamp/state", json!(true), 200),
        ("salon:lamp/state", json!(true), 403),
        ("tel:p1/push", json!("Bonjour"), 403),
    ] {
        let order = json!({ "point": point, "value": value });
        let (status, body) = request_with(addr, "POST", "/api/command", Some(&order), guest).await;
        assert_eq!(status, expected, "{point}: {body}");
    }
    assert_eq!(
        house.hub.state(&"salon:lamp/state".into()).unwrap().value,
        Value::Bool(false)
    );
    let _ = std::fs::remove_dir_all(&house.dir);
    let _ = std::fs::remove_dir_all(&store_dir);
}

/// A live stream kept open: the connection, and what was read of it.
struct Live {
    stream: tokio::net::TcpStream,
    read: Vec<u8>,
}

/// Opens a live stream; its status and the stream.
async fn live(addr: &str, path: &str, extra: &str) -> (u16, Live) {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let head =
        format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nAccept: text/event-stream\r\n{extra}\r\n");
    stream.write_all(head.as_bytes()).await.unwrap();
    let mut live = Live {
        stream,
        read: Vec::new(),
    };
    let end = loop {
        if let Some(end) = live.read.windows(4).position(|w| w == b"\r\n\r\n") {
            break end;
        }
        live.more().await;
    };
    let status = String::from_utf8_lossy(&live.read[9..12]).parse().unwrap();
    live.read.drain(..end + 4);
    (status, live)
}

impl Live {
    async fn more(&mut self) {
        let mut chunk = vec![0u8; 16 * 1024];
        let n = tokio::time::timeout(Duration::from_secs(5), self.stream.read(&mut chunk))
            .await
            .expect("nothing came within 5 s")
            .unwrap();
        assert!(n > 0, "the stream ended");
        self.read.extend_from_slice(&chunk[..n]);
    }

    /// Reads on until `text` came.
    async fn until(&mut self, text: &str) {
        while !String::from_utf8_lossy(&self.read).contains(text) {
            self.more().await;
        }
    }

    /// The next event: its name and its data.
    async fn next(&mut self) -> (String, Json) {
        loop {
            let text = match std::str::from_utf8(&self.read) {
                Ok(text) => text,
                Err(e) => std::str::from_utf8(&self.read[..e.valid_up_to()]).unwrap(),
            };
            if let Some(start) = text.find("event: ") {
                let rest = &text[start + 7..];
                if let Some((name, rest)) = rest.split_once('\n')
                    && let Some(data) = rest.strip_prefix("data: ")
                    && let Some(end) = data.find('\n')
                {
                    let event = (name.to_owned(), serde_json::from_str(&data[..end]).unwrap());
                    let used = text.len() - data.len() + end + 1;
                    self.read.drain(..used);
                    return event;
                }
            }
            self.more().await;
        }
    }
}

/// The live stream of a guest: its rooms' devices and changes, never the
/// house's journal; a device moved into its room comes with a new snapshot.
#[tokio::test]
async fn a_guest_follows_its_rooms_only_live() {
    use moli_core::Origin;
    let house = guest_house("guest-live", |_| {}).await;
    let (status, mut guest) = live(&house.addr, "/api/events", &house.guest).await;
    assert_eq!(status, 200);
    let (name, snapshot) = guest.next().await;
    assert_eq!(name, "snapshot");
    assert_eq!(device_ids(&snapshot), ["fake:lamp"]);
    let (_, mut household) = live(&house.addr, "/api/events", "").await;
    assert_eq!(household.next().await.0, "snapshot");

    // The living room, then the guest room, twice: each order journaled.
    for (point, on) in [
        ("salon:lamp/state", true),
        ("fake:lamp/state", true),
        ("fake:lamp/state", false),
    ] {
        house
            .hub
            .command(&point.into(), Value::Bool(on), Origin::Ui, None)
            .await
            .unwrap();
    }
    let (_, first) = household.next().await;
    assert_eq!(first["point"], "salon:lamp/state", "{first}");
    for on in [true, false] {
        let (name, event) = guest.next().await;
        assert_eq!(name, "event");
        assert_eq!(
            (&event["type"], &event["point"], &event["value"]),
            (&json!("state"), &json!("fake:lamp/state"), &json!(on)),
            "{event}"
        );
    }

    // The living room's lamp moved into the guest room: now the guest's.
    let room = moli_runtime::LabelPatch {
        name: None,
        room: Some(GUEST_ROOM.into()),
    };
    house
        .hub
        .set_label(&"salon:lamp".into(), room, Origin::Ui, None)
        .await
        .unwrap();
    let (name, snapshot) = guest.next().await;
    assert_eq!(name, "snapshot");
    assert_eq!(device_ids(&snapshot), ["fake:lamp", "salon:lamp"]);
    let _ = std::fs::remove_dir_all(&house.dir);
}

/// A phone hears every notice of the house: a guest pairs none, removing
/// someone forgets theirs, and the phone of someone no longer of the house
/// (a guest whose stay is over) reports nothing and is told nothing.
#[tokio::test]
async fn a_phone_follows_its_person() {
    use moli_runtime::CommandError;
    let phones_dir = std::env::temp_dir().join(format!("moli-http-phones-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&phones_dir);
    std::fs::create_dir_all(&phones_dir).unwrap();
    let config = moli_phones::Config {
        home_wifi: Vec::new(),
        home: None,
        radius: 150.0,
    };
    let (gateway, driver) = moli_phones::open(config, &phones_dir).unwrap();
    let gate = gateway.clone();
    let house = guest_house("phones", move |options| options.phones = Some(gate)).await;
    spawn_driver(
        &house.hub,
        InstanceId::from("telephones"),
        Arc::new(driver),
        CancellationToken::new(),
    );
    let addr = &house.addr;

    // A guest pairs no phone.
    let pairing = json!({ "name": "Mon téléphone", "platform": "ios" });
    let (status, body) = request_with(
        addr,
        "POST",
        "/api/mobile/register",
        Some(&pairing),
        &house.guest,
    )
    .await;
    assert_eq!(status, 403, "{body}");
    assert!(gateway.list().is_empty());

    // Sam's phone, and the phone of a guest whose stay is over.
    for person in [
        json!({ "name": "Sam", "role": "member" }),
        json!({ "name": "Passage", "role": "guest", "expires": 1 }),
    ] {
        let (status, body) =
            request_with(addr, "POST", "/api/people", Some(&person), &house.pin).await;
        assert_eq!(status, 201, "{body}");
    }
    let sam = gateway
        .register("Téléphone de Sam", "ios", "", None, Some("sam"))
        .unwrap();
    let past = gateway
        .register("Téléphone de passage", "ios", "", None, Some("passage"))
        .unwrap();
    let report = |phone: &moli_phones::Paired| {
        (
            format!("/api/phones/{}/report", phone.id),
            format!("Authorization: Bearer {}\r\n", phone.token),
        )
    };
    let (path, bearer) = report(&sam);
    let (status, body) = request_with(addr, "POST", &path, Some(&json!({})), &bearer).await;
    assert_eq!(status, 200, "{body}");

    // The stay is over: its phone reports nothing, and is told nothing.
    let (path, bearer) = report(&past);
    let battery = json!({ "battery": 50 });
    let (status, body) = request_with(addr, "POST", &path, Some(&battery), &bearer).await;
    assert_eq!(status, 403, "{body}");
    let device = moli_core::DeviceId::from(format!("telephones:{}", past.id));
    for _ in 0..200 {
        if house.hub.device(&device).is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let push = moli_core::PointId::from(format!("{device}/push"));
    let told = house
        .hub
        .command(
            &push,
            Value::Text("Bonsoir".into()),
            moli_core::Origin::Automation,
            None,
        )
        .await;
    assert_eq!(
        told,
        Err(CommandError::Driver(moli_i18n::tr(
            "pilotes.phones.hors_maison"
        )))
    );

    // Sam leaves the house: Sam's phone is forgotten, its token dead.
    let (status, body) = request_with(addr, "DELETE", "/api/people/sam", None, &house.pin).await;
    assert_eq!(status, 200, "{body}");
    assert!(
        gateway
            .list()
            .iter()
            .all(|p| p.person.as_deref() != Some("sam"))
    );
    let (path, bearer) = report(&sam);
    let (status, body) = request_with(addr, "POST", &path, Some(&json!({})), &bearer).await;
    assert_eq!(status, 401, "{body}");
    let _ = std::fs::remove_dir_all(&house.dir);
    let _ = std::fs::remove_dir_all(&phones_dir);
}

/// Live streams are counted: a caller holds a few at most, and a stream
/// that ends gives its seat back.
#[tokio::test]
async fn live_streams_are_counted() {
    let (addr, hub) = server().await;
    let mut open = Vec::new();
    for _ in 0..8 {
        let (status, live) = live(&addr, "/api/events", "").await;
        assert_eq!(status, 200);
        open.push(live);
    }
    let (status, mut refused) = live(&addr, "/api/events", "").await;
    assert_eq!(status, 503);
    refused
        .until(&moli_i18n::tr("serveur.limite.trop_de_flux"))
        .await;
    // One closed: noticed at the house's next word, its seat is back.
    drop(open.pop());
    for on in (0..100).map(|i| i % 2 == 0) {
        hub.command(
            &"fake:lamp/state".into(),
            Value::Bool(on),
            moli_core::Origin::Ui,
            None,
        )
        .await
        .unwrap();
        if live(&addr, "/api/events", "").await.0 == 200 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the seat of a closed stream never came back");
}

/// Élodie, from the Internet: nothing until she is invited, then the house.
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn an_invited_person_reaches_the_house_from_the_internet() {
    let dir = std::env::temp_dir().join(format!("moli-http-people-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let hub = Hub::new(HubOptions {
        secrets: Some((
            dir.join("secrets.enc"),
            moli_runtime::MasterKey::generate().unwrap(),
        )),
        ..HubOptions::default()
    })
    .unwrap();
    spawn_driver(
        &hub,
        InstanceId::from("fake"),
        Arc::new(Lamp),
        CancellationToken::new(),
    );
    let options = moli_api::Options {
        ui_pin: Some(moli_api::UiPin::new("482915")),
        home_path: Some(dir.join("home.json")),
        ..moli_api::Options::default()
    };
    let addr = serve(&hub, &options).await;
    // From the Internet (a relayed request is never the home network).
    let outside = "X-Forwarded-For: 203.0.113.9\r\n";

    let (status, _) = request_with(&addr, "GET", "/api/devices", None, outside).await;
    assert_eq!(status, 403, "a stranger reads nothing");
    let (status, page) = request_with(&addr, "GET", "/api/session", None, outside).await;
    assert_eq!(status, 200);
    assert_eq!(
        serde_json::from_str::<Json>(&page).unwrap()["login_required"],
        true
    );
    let (status, _) = request_with(
        &addr,
        "POST",
        "/api/session",
        Some(&json!({ "pin": "482915" })),
        outside,
    )
    .await;
    assert_eq!(status, 403, "the house's code only from the house");

    // At home, the owner (proved by the code: nobody yet) invites Élodie.
    let (status, pin, _) = request_cookie(
        &addr,
        "POST",
        "/api/session",
        &json!({ "pin": "482915" }),
        "",
        "moli_session",
    )
    .await;
    assert_eq!(status, 200);
    let pin = format!("Cookie: {}\r\n", pin.unwrap());
    let (status, body) = request_with(
        &addr,
        "POST",
        "/api/people",
        Some(&json!({ "name": "Élodie", "role": "member" })),
        &pin,
    )
    .await;
    assert_eq!(status, 201, "{body}");
    let (status, body) = request_with(&addr, "POST", "/api/people/elodie/invite", None, &pin).await;
    assert_eq!(status, 200, "{body}");
    let code = serde_json::from_str::<Json>(&body).unwrap()["code"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(dir.join("people.json").exists());
    assert!(
        !std::fs::read_to_string(dir.join("people.json"))
            .unwrap()
            .contains(&code)
    );

    // Élodie, from the Internet, with the code.
    let (status, cookie, body) = request_cookie(
        &addr,
        "POST",
        "/api/invitation",
        &json!({ "code": code, "password": "le jardin fleuri" }),
        outside,
        "moli_person",
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let me = format!("{outside}Cookie: {}\r\n", cookie.unwrap());
    let (status, _) = request_with(&addr, "GET", "/api/devices", None, &me).await;
    assert_eq!(status, 200, "signed in: the house");
    let (_, session) = request_with(&addr, "GET", "/api/session", None, &me).await;
    let session: Json = serde_json::from_str(&session).unwrap();
    assert_eq!(session["person"]["id"], "elodie");
    assert_eq!(session["owner"], false);
    let (status, _) = request_with(
        &addr,
        "POST",
        "/api/people",
        Some(&json!({ "name": "X", "role": "owner" })),
        &me,
    )
    .await;
    assert_eq!(status, 403, "a member manages nobody");
    let (status, _) = request_with(
        &addr,
        "POST",
        "/api/command",
        Some(&json!({ "point": "fake:lamp/state", "value": true })),
        &format!("{me}x-moli-origin: ui\r\n"),
    )
    .await;
    assert_eq!(status, 200, "and commands it");
    let journal = hub.journal(10);
    assert!(
        journal
            .iter()
            .any(|e| e.actor.as_deref() == Some("tableau de bord (Élodie)")),
        "{journal:?}"
    );
    // Her password works the next time; a wrong one does not.
    let (status, _, _) = request_cookie(
        &addr,
        "POST",
        "/api/session",
        &json!({ "login": "elodie", "password": "le jardin fleuri" }),
        outside,
        "moli_person",
    )
    .await;
    assert_eq!(status, 200);
    let (status, _, _) = request_cookie(
        &addr,
        "POST",
        "/api/session",
        &json!({ "login": "elodie", "password": "le jardin fané" }),
        outside,
        "moli_person",
    )
    .await;
    assert_eq!(status, 401);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn androidtv_pairing_reads_are_public_to_household_and_never_start_an_attempt() {
    let hub = lamp_hub().await;
    let gateway = moli_androidtv_driver::pairing::PairingGateway::new();
    let options = moli_api::Options {
        androidtv: std::collections::BTreeMap::from([("example".into(), gateway.clone())]),
        ..moli_api::Options::default()
    };
    let addr = serve(&hub, &options).await;
    let (status, body) = request(
        &addr,
        "GET",
        "/api/integrations/androidtv/example/pairing",
        None,
    )
    .await;
    assert_eq!(status, 200);
    assert!(
        body.contains("\"phase\":\"unavailable\""),
        "public JSON state missing: {body}"
    );
    assert_eq!(gateway.state().generation, 0);
    for instance in ["unknown", "disabled"] {
        let (status, _) = request(
            &addr,
            "GET",
            &format!("/api/integrations/androidtv/{instance}/pairing"),
            None,
        )
        .await;
        assert_eq!(status, 404);
    }
}

#[tokio::test]
async fn androidtv_pairing_mutations_require_pin_and_refuse_invalid_bodies_without_echoing_codes() {
    let hub = lamp_hub().await;
    let gateway = moli_androidtv_driver::pairing::PairingGateway::new();
    let options = moli_api::Options {
        ui_pin: Some(moli_api::UiPin::new("482915")),
        androidtv: std::collections::BTreeMap::from([("example".into(), gateway.clone())]),
        ..moli_api::Options::default()
    };
    let addr = serve(&hub, &options).await;
    let path = "/api/integrations/androidtv/example/pairing";
    for method in ["POST", "PUT", "DELETE"] {
        let (status, _) = request(&addr, method, path, Some(&json!({}))).await;
        assert_eq!(status, 403);
    }
    let human = pin_session(&addr, "482915").await;
    let (status, body) = request_with(&addr, "POST", path, Some(&json!({})), &human).await;
    assert_eq!(
        status, 503,
        "no running actor must not start network: {body}"
    );
    for payload in [
        json!({"generation":1,"code":"SECRET-MARKER"}),
        json!({"generation":1,"code":"ABC123","unknown":"SECRET-MARKER"}),
    ] {
        let (status, body) = request_with(&addr, "PUT", path, Some(&payload), &human).await;
        assert_eq!(status, 422);
        assert!(!body.contains("SECRET-MARKER") && !body.contains("ABC123"));
    }
    let (status, _) = request_with(
        &addr,
        "PUT",
        path,
        Some(&json!({"generation":1,"code":"x".repeat(1100)})),
        &human,
    )
    .await;
    assert_eq!(status, 413);
    let (status, _) = request_with(
        &addr,
        "POST",
        "/api/integrations/androidtv/unknown/pairing",
        Some(&json!({})),
        &human,
    )
    .await;
    assert_eq!(status, 404);
    assert_eq!(gateway.state().generation, 0);
}

#[tokio::test]
async fn androidtv_pairing_is_never_mutable_in_demo_even_with_a_pin() {
    let hub = lamp_hub().await;
    let options = moli_api::Options {
        demo: true,
        ui_pin: Some(moli_api::UiPin::new("482915")),
        androidtv: std::collections::BTreeMap::from([(
            "example".into(),
            moli_androidtv_driver::pairing::PairingGateway::new(),
        )]),
        ..moli_api::Options::default()
    };
    let addr = serve(&hub, &options).await;
    let human = pin_session(&addr, "482915").await;
    for method in ["POST", "PUT", "DELETE"] {
        let (status, _) = request_with(
            &addr,
            method,
            "/api/integrations/androidtv/example/pairing",
            Some(&json!({})),
            &human,
        )
        .await;
        assert_eq!(status, 403);
    }
}

/// A synthetic calendar supplies admission through the real scoped driver context.
struct ReservationFixture {
    updates: std::sync::Mutex<Option<tokio::sync::mpsc::Receiver<ReservationUpdate>>>,
}

type ReservationUpdate = (
    Option<Vec<(String, moli_core::AccessWindow)>>,
    tokio::sync::oneshot::Sender<()>,
);
const RESERVATION_FIXTURE_ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

impl Driver for ReservationFixture {
    fn kind(&self) -> &'static str {
        "reservations"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let mut updates = self.updates.lock().unwrap().take().unwrap();
            ctx.ready();
            while let Some((windows, done)) = updates.recv().await {
                match windows {
                    Some(windows) => {
                        ctx.ready();
                        assert!(ctx.replace_reservations(windows, moli_core::now_ms() + 60_000));
                    }
                    None => ctx.wait_for("simulated calendar outage"),
                }
                let _ = done.send(());
            }
            ctx.cancelled().await;
            Ok(())
        })
    }
}

async fn reservation_update(
    tx: &tokio::sync::mpsc::Sender<ReservationUpdate>,
    windows: Option<Vec<(String, moli_core::AccessWindow)>>,
) {
    let (done, wait) = tokio::sync::oneshot::channel();
    tx.send((windows, done)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), wait)
        .await
        .unwrap()
        .unwrap();
}

async fn guest_devices_status(house: &GuestHouse) -> u16 {
    request_with(&house.addr, "GET", "/api/devices", None, &house.guest)
        .await
        .0
}

async fn guest_order_status(house: &GuestHouse, order: &Json) -> u16 {
    request_with(
        &house.addr,
        "POST",
        "/api/command",
        Some(order),
        &house.guest,
    )
    .await
    .0
}

#[tokio::test]
async fn reservations_ics_gate_an_existing_guest_session_and_live_stream() {
    let house = guest_house("reservations-ics", |_| {}).await;
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    let cancel = CancellationToken::new();
    let task = spawn_driver(
        &house.hub,
        "calendar".into(),
        Arc::new(ReservationFixture {
            updates: std::sync::Mutex::new(Some(rx)),
        }),
        cancel.clone(),
    );
    let now = moli_core::now_ms();
    let window = |start, end| {
        vec![(
            RESERVATION_FIXTURE_ID.into(),
            moli_core::AccessWindow::new(start, end).unwrap(),
        )]
    };
    reservation_update(&tx, Some(window(now + 60_000, now + 120_000))).await;
    let binding = json!({ "name":"Invité", "role":"guest", "rooms":[GUEST_ROOM], "reservation":{"instance":"calendar","id":RESERVATION_FIXTURE_ID} });
    let (status, body) = request_with(
        &house.addr,
        "PUT",
        "/api/people/invite",
        Some(&binding),
        &house.pin,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let legacy = json!({ "name":"Invité", "role":"guest", "rooms":[GUEST_ROOM] });
    assert_eq!(
        request_with(
            &house.addr,
            "PUT",
            "/api/people/invite",
            Some(&legacy),
            &house.pin
        )
        .await
        .0,
        200
    );

    assert_eq!(guest_devices_status(&house).await, 403);
    let order = json!({"point":"fake:lamp/state","value":true});
    assert_eq!(guest_order_status(&house, &order).await, 403);
    reservation_update(&tx, Some(window(now - 1, now + 60_000))).await;
    assert_eq!(guest_devices_status(&house).await, 200);
    assert_eq!(guest_order_status(&house, &order).await, 200);
    let (status, mut stream) = live(
        &house.addr,
        "/api/events",
        &format!("{}Connection: close\r\n", house.guest),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(stream.next().await.0, "snapshot");
    // Cancellation immediately denies the same session and stops new SSE data.
    reservation_update(&tx, Some(Vec::new())).await;
    house
        .hub
        .command(
            &"fake:lamp/state".into(),
            Value::Bool(false),
            moli_core::Origin::System,
            None,
        )
        .await
        .unwrap();
    let mut tail = Vec::new();
    tokio::time::timeout(Duration::from_secs(2), stream.stream.read_to_end(&mut tail))
        .await
        .unwrap()
        .unwrap();
    assert!(!String::from_utf8_lossy(&tail).contains("event: "));
    assert_eq!(guest_devices_status(&house).await, 403);
    reservation_update(&tx, Some(window(now - 1, now + 60_000))).await;
    assert_eq!(guest_devices_status(&house).await, 200);
    reservation_update(&tx, None).await;
    assert_eq!(guest_devices_status(&house).await, 403);
    let unbind =
        json!({ "name":"Invité", "role":"guest", "rooms":[GUEST_ROOM], "reservation":null });
    assert_eq!(
        request_with(
            &house.addr,
            "PUT",
            "/api/people/invite",
            Some(&unbind),
            &house.pin
        )
        .await
        .0,
        200
    );
    assert_eq!(guest_devices_status(&house).await, 200);
    cancel.cancel();
    task.await.unwrap();
    let _ = std::fs::remove_dir_all(&house.dir);
}

struct HeldReservationLamp {
    taken: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    emitted: Arc<std::sync::atomic::AtomicBool>,
}

impl Driver for HeldReservationLamp {
    fn kind(&self) -> &'static str {
        "fake"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let id = ctx.device_id("lamp");
            ctx.upsert_device(Device {
                id: id.clone(),
                instance: ctx.instance().clone(),
                native_name: "simulated transport".into(),
                manufacturer: None,
                model: None,
                description: None,
                native_room: Some(GUEST_ROOM.into()),
                members: Vec::new(),
                points: vec![PointSpec {
                    key: "state".into(),
                    label: "state".into(),
                    kind: Kind::Binary,
                    access: Access {
                        read: true,
                        write: true,
                    },
                    unit: None,
                    semantic: Semantic::OnOff,
                }],
            });
            ctx.set_state(&id, "state", Value::Bool(false));
            ctx.ready();
            while let Some(command) = ctx.next_command().await {
                self.taken.notify_one();
                self.release.notified().await;
                // The same live hook used by a physical transport before emission.
                if command.abandoned() {
                    command.reply_result(Err(moli_runtime::CommandError::AccessDenied));
                } else {
                    self.emitted
                        .store(true, std::sync::atomic::Ordering::Release);
                    ctx.set_state(&id, "state", command.value.clone());
                    command.reply(Ok(()));
                }
            }
            Ok(())
        })
    }
}

#[tokio::test]
async fn reservations_ics_revoke_an_order_waiting_before_transport_emission() {
    let house = guest_house("reservation-transport", |_| {}).await;
    let cancel = CancellationToken::new();
    let taken = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let emitted = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let lamp = spawn_driver(
        &house.hub,
        "held".into(),
        Arc::new(HeldReservationLamp {
            taken: taken.clone(),
            release: release.clone(),
            emitted: emitted.clone(),
        }),
        cancel.clone(),
    );
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    let calendar = spawn_driver(
        &house.hub,
        "calendar".into(),
        Arc::new(ReservationFixture {
            updates: std::sync::Mutex::new(Some(rx)),
        }),
        cancel.clone(),
    );
    let now = moli_core::now_ms();
    reservation_update(
        &tx,
        Some(vec![(
            RESERVATION_FIXTURE_ID.into(),
            moli_core::AccessWindow::new(now - 1, now + 60_000).unwrap(),
        )]),
    )
    .await;
    let binding = json!({"name":"Invité","role":"guest","rooms":[GUEST_ROOM],"reservation":{"instance":"calendar","id":RESERVATION_FIXTURE_ID}});
    assert_eq!(
        request_with(
            &house.addr,
            "PUT",
            "/api/people/invite",
            Some(&binding),
            &house.pin
        )
        .await
        .0,
        200
    );
    let addr = house.addr.clone();
    let guest = house.guest.clone();
    let request = tokio::spawn(async move {
        request_with(
            &addr,
            "POST",
            "/api/command",
            Some(&json!({"point":"held:lamp/state","value":true})),
            &guest,
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(2), taken.notified())
        .await
        .unwrap();
    reservation_update(&tx, Some(Vec::new())).await;
    release.notify_one();
    assert_eq!(request.await.unwrap().0, 403);
    assert!(!emitted.load(std::sync::atomic::Ordering::Acquire));
    assert_eq!(
        house.hub.state(&"held:lamp/state".into()).unwrap().value,
        Value::Bool(false)
    );
    cancel.cancel();
    lamp.await.unwrap();
    calendar.await.unwrap();
    let _ = std::fs::remove_dir_all(&house.dir);
}

#[tokio::test]
async fn reservations_ics_owner_reads_the_binding_needed_to_preserve_it_when_editing() {
    let house = guest_house("reservation-owner-edit", |_| {}).await;
    let reservation = json!({"instance":"calendar","id":RESERVATION_FIXTURE_ID});
    let draft =
        json!({"name":"Invité","role":"guest","rooms":[GUEST_ROOM],"reservation":reservation});
    assert_eq!(
        request_with(
            &house.addr,
            "PUT",
            "/api/people/invite",
            Some(&draft),
            &house.pin
        )
        .await
        .0,
        200
    );
    let (status, body) = request_with(&house.addr, "GET", "/api/people", None, &house.pin).await;
    assert_eq!(status, 200);
    let book: Json = serde_json::from_str(&body).unwrap();
    assert_eq!(
        book["people"][0]["reservation"], reservation,
        "the edit form must receive the stored binding"
    );
    let returned = &book["people"][0];
    let edited = json!({"name":"Invité exemple","role":returned["role"],"rooms":returned["rooms"],"reservation":returned["reservation"]});
    assert_eq!(
        request_with(
            &house.addr,
            "PUT",
            "/api/people/invite",
            Some(&edited),
            &house.pin
        )
        .await
        .0,
        200
    );
    let (_, body) = request_with(&house.addr, "GET", "/api/people", None, &house.pin).await;
    assert_eq!(
        serde_json::from_str::<Json>(&body).unwrap()["people"][0]["reservation"],
        reservation
    );
    let _ = std::fs::remove_dir_all(&house.dir);
}

#[path = "http/guest_policy.rs"]
mod guest_policy;

#[tokio::test]
async fn auto_h3_rest_temporal_values_are_typed_and_canonical() {
    let hub = Hub::new(HubOptions::default()).unwrap();
    let config=serde_json::from_value(json!({"helper":[{"id":"sample","name":"Synthetic instant","kind":"datetime","timezone":"Europe/Paris","initial":"2000-01-01T00:00:00Z"}]})).unwrap();
    let cancel = CancellationToken::new();
    let driver = spawn_driver(
        &hub,
        "fixture".into(),
        Arc::new(moli_helpers::Helpers::new(config).unwrap()),
        cancel.clone(),
    );
    let point = "fixture:sample/value".into();
    for _ in 0..200 {
        if hub.state(&point).is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let addr = serve(&hub, &moli_api::Options::default()).await;
    for value in [
        json!(null),
        json!(0),
        json!("2026-02-29T00:00:00"),
        json!("2026-03-29T02:30:00"),
        json!("2026-10-25T02:30:00"),
        json!("2026-10-25T02:30:00+03:00"),
    ] {
        let (status, body) = request(
            &addr,
            "POST",
            "/api/command",
            Some(&json!({"point":point,"value":value})),
        )
        .await;
        assert_eq!(status, 422, "{body}");
    }
    let (status, body) = request(
        &addr,
        "POST",
        "/api/command",
        Some(&json!({"point":point,"value":"2026-10-25T02:30:00+01:00"})),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        hub.state(&point).unwrap().value,
        Value::Text("2026-10-25T01:30:00Z".into())
    );
    let (status, body) = request(&addr, "GET", "/api/devices", None).await;
    assert_eq!(status, 200);
    let snapshot: Json = serde_json::from_str(&body).unwrap();
    assert_eq!(
        snapshot["devices"][0]["points"][0]["kind"]["spec"]["timezone"],
        "Europe/Paris"
    );
    cancel.cancel();
    driver.await.unwrap();
}

#[tokio::test]
async fn auto_h5_http_ui_cannot_publish_without_pin_and_stale_preview_conflicts() {
    let hub = lamp_hub().await;
    let cancel = CancellationToken::new();
    let config = serde_json::from_value(json!({"helper":[{"id":"weekly","name":"Weekly","kind":"schedule","weekly":{"timezone":"UTC","slots":[]}}]})).unwrap();
    spawn_driver(
        &hub,
        InstanceId::from("fixture"),
        Arc::new(moli_helpers::Helpers::new(config).unwrap()),
        cancel.clone(),
    );
    let point = moli_core::PointId::from("fixture:weekly/active");
    for _ in 0..200 {
        if hub.state(&point).is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(hub.state(&point).is_some());
    let addr = serve(
        &hub,
        &moli_api::Options {
            ui_pin: Some(moli_api::UiPin::new("482915")),
            ..Default::default()
        },
    )
    .await;
    let path = "/api/dependencies/schedule/fixture:weekly";
    let ui = "x-moli-origin: ui\r\n";
    let (status, body) = request(&addr, "GET", &format!("{path}/preview"), None).await;
    assert_eq!(status, 200, "{body}");
    let preview: Json = serde_json::from_str(&body).unwrap();
    let (status, body) = request_with(
        &addr,
        "POST",
        &format!("{path}/approve"),
        Some(&preview),
        ui,
    )
    .await;
    assert_eq!(status, 403, "{body}");
    let pin = pin_session(&addr, "482915").await;
    let draft = json!({"expected":1,"definition":{"type":"schedule","value":{"timezone":"UTC","slots":[{"day":1,"start":0,"end":1440}]}}});
    let (status, body) = request_with(&addr, "PUT", path, Some(&draft), ui).await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = request_with(&addr, "PUT", path, Some(&draft), ui).await;
    assert_eq!(status, 409, "{body}");
    let (status, body) = request_with(
        &addr,
        "POST",
        &format!("{path}/approve"),
        Some(&preview),
        &pin,
    )
    .await;
    assert_eq!(status, 409, "{body}");
    let (_, body) = request(&addr, "GET", &format!("{path}/preview"), None).await;
    let preview: Json = serde_json::from_str(&body).unwrap();
    let (status, body) = request_with(
        &addr,
        "POST",
        &format!("{path}/approve"),
        Some(&preview),
        &pin,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let view: Json = serde_json::from_str(&body).unwrap();
    assert_eq!(view["versions"]["current"]["version"], 2);
    assert_eq!(view["versions"]["draft"]["version"], 2);
    assert!(
        hub.command(&point, Value::Bool(true), moli_core::Origin::Ui, None)
            .await
            .is_err()
    );
    cancel.cancel();
}

async fn computed_fixture() -> (String, Hub, CancellationToken) {
    let hub = Hub::new(HubOptions::default()).unwrap();
    let cancel = CancellationToken::new();
    let config=serde_json::from_value(json!({"helper":[
        {"id":"x","name":"Input X","kind":"number","initial":10},
        {"id":"y","name":"Input Y","kind":"number","initial":30},
        {"id":"a","name":"Average","kind":"computed","aggregate":{"operation":"mean","sources":["fixture:x/value","fixture:y/value"],"max_age_s":60}},
        {"id":"b","name":"Downstream","kind":"computed","aggregate":{"operation":"max","sources":["fixture:a/value"],"max_age_s":60}}
    ]})).unwrap();
    spawn_driver(
        &hub,
        "fixture".into(),
        Arc::new(moli_helpers::Helpers::new(config).unwrap()),
        cancel.clone(),
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while hub
            .state(&moli_core::PointId::from("fixture:b/value"))
            .is_none()
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let addr = serve(
        &hub,
        &moli_api::Options {
            ui_pin: Some(moli_api::UiPin::new("482915")),
            ..Default::default()
        },
    )
    .await;
    (addr, hub, cancel)
}
async fn dependency_preview(addr: &str, path: &str) -> Json {
    let (status, body) = request(addr, "GET", &format!("{path}/preview"), None).await;
    assert_eq!(status, 200, "{body}");
    serde_json::from_str(&body).unwrap()
}
#[tokio::test]
async fn auto_k1_http_binds_current_sources_and_ui_header_never_approves() {
    let (addr, hub, cancel) = computed_fixture().await;
    let a = "/api/dependencies/computed/fixture:a";
    let b = "/api/dependencies/computed/fixture:b";
    let shown = dependency_preview(&addr, a).await;
    let (status, body) = request_with(
        &addr,
        "POST",
        &format!("{a}/approve"),
        Some(&shown),
        "x-moli-origin: ui\r\n",
    )
    .await;
    assert_eq!(status, 403, "{body}");
    let pin = pin_session(&addr, "482915").await;
    let (status, body) =
        request_with(&addr, "POST", &format!("{a}/approve"), Some(&shown), &pin).await;
    assert_eq!(status, 200, "{body}");
    let shown = dependency_preview(&addr, b).await;
    let (status, body) =
        request_with(&addr, "POST", &format!("{b}/approve"), Some(&shown), &pin).await;
    assert_eq!(status, 200, "{body}");
    let old_b = dependency_preview(&addr, b).await;
    let draft = json!({"expected":1,"definition":{"type":"aggregate","value":{"operation":"min","sources":["fixture:x/value","fixture:y/value"],"unit":null,"max_age_s":60}}});
    let (status, body) = request(&addr, "PUT", a, Some(&draft)).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        hub.state(&moli_core::PointId::from("fixture:a/value"))
            .unwrap()
            .value,
        Value::Float(20.0)
    );
    let shown = dependency_preview(&addr, a).await;
    assert!(
        shown["related"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["reference"]["id"] == "fixture:b" && r["role"] == "affected")
    );
    let (status, body) =
        request_with(&addr, "POST", &format!("{a}/approve"), Some(&shown), &pin).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        hub.state(&moli_core::PointId::from("fixture:b/value"))
            .unwrap()
            .value,
        Value::Float(10.0)
    );
    let (status, body) =
        request_with(&addr, "POST", &format!("{b}/approve"), Some(&old_b), &pin).await;
    assert_eq!(status, 409, "{body}");
    let cycle = json!({"expected":2,"definition":{"type":"aggregate","value":{"operation":"min","sources":["fixture:b/value"],"unit":null,"max_age_s":60}}});
    let (status, body) = request(&addr, "PUT", a, Some(&cycle)).await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = request(&addr, "GET", &format!("{a}/preview"), None).await;
    assert_eq!(status, 422, "{body}");
    cancel.cancel();
}

#[tokio::test]
async fn auto_s1_http_pin_exact_scope_and_current_execution() {
    let hub = lamp_hub().await;
    let id = moli_core::DeviceId::from("scenes:desired");
    let definition=serde_json::from_value(json!({"targets":[{"point":"fake:lamp/state","value":true}],"parallel":2,"timeout_s":10,"confirmation_s":2})).unwrap();
    hub.initialize_scene(id.clone(), definition).await.unwrap();
    let addr = serve(
        &hub,
        &moli_api::Options {
            ui_pin: Some(moli_api::UiPin::new("482915")),
            ..Default::default()
        },
    )
    .await;
    let path = "/api/dependencies/scene/scenes:desired";
    let (_, body) = request(&addr, "GET", &format!("{path}/preview"), None).await;
    let preview: Json = serde_json::from_str(&body).unwrap();
    assert_eq!(
        preview["scene"]["targets"][0]["devices"],
        json!(["fake:lamp"])
    );
    let (status, body) = request_with(
        &addr,
        "POST",
        &format!("{path}/approve"),
        Some(&preview),
        "x-moli-origin: ui\r\n",
    )
    .await;
    assert_eq!(status, 403, "{body}");
    let (status, _) = request(&addr, "GET", "/api/scenes/scenes:desired/preview", None).await;
    assert_eq!(status, 409);
    let pin = pin_session(&addr, "482915").await;
    let (status, body) = request_with(
        &addr,
        "POST",
        &format!("{path}/approve"),
        Some(&preview),
        &pin,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = request(&addr, "GET", "/api/scenes/scenes:desired/preview", None).await;
    assert_eq!(status, 200, "{body}");
    let current: Json = serde_json::from_str(&body).unwrap();
    let mut stale = current.clone();
    stale["version"] = json!(2);
    let (status, _) = request_with(
        &addr,
        "POST",
        "/api/scenes/scenes:desired/run",
        Some(&stale),
        &pin,
    )
    .await;
    assert_eq!(status, 409);
    let (status, body) = request_with(
        &addr,
        "POST",
        "/api/scenes/scenes:desired/run",
        Some(&current),
        &pin,
    )
    .await;
    assert_eq!(status, 202, "{body}");
    let run: Json = serde_json::from_str(&body).unwrap();
    let run = hub.wait_scene(run["id"].as_str().unwrap()).await.unwrap();
    assert_eq!(run.status, moli_runtime::SceneStatus::Done);
    assert!(run.targets[0].acknowledged);
    assert_eq!(
        hub.state(&moli_core::PointId::from("fake:lamp/state"))
            .unwrap()
            .value,
        Value::Bool(true)
    );
}
