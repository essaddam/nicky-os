//! Restart and cache contracts against the real hub; no network or appliance.
use crate::{
    model::{self, Action, Reading},
    publication::Published,
};
use moli_core::{DeviceId, InstanceId, PointId, Value};
use moli_runtime::{BoxFuture, Driver, DriverCtx, Hub, HubOptions, spawn_driver};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_util::sync::CancellationToken;
const ROUTER: &str = "router_aabbccddee01";
const CLIENT: &str = "client_aabbccddee02";
#[derive(Clone, Copy)]
enum Mode {
    Full,
    Partial,
    Absent,
    NoInventory,
}
struct Stage {
    mode: Mode,
    done: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}
fn reads(mode: Mode) -> Vec<(Action, Option<Reading>, u64)> {
    let full = matches!(mode, Mode::Full);
    let info = if full {
        include_bytes!("../../../integrations/asuswrt/fixtures/info.json").as_slice()
    } else {
        br#"{"label_mac":"aa:bb:cc:dd:ee:01"}"#
    };
    let telemetry = if full {
        include_bytes!("../../../integrations/asuswrt/fixtures/telemetry.json").as_slice()
    } else {
        br#"{"memory_usage":{"mem_total":"100","mem_used":"25"}}"#
    };
    let mut result = vec![
        (
            Action::Info,
            Some(model::parse(Action::Info, info).unwrap()),
            1,
        ),
        (
            Action::Telemetry,
            Some(model::parse(Action::Telemetry, telemetry).unwrap()),
            1,
        ),
    ];
    if !matches!(mode, Mode::NoInventory) {
        let clients = match mode {
            Mode::Full => {
                include_bytes!("../../../integrations/asuswrt/fixtures/clients.json").as_slice()
            }
            Mode::Absent => {
                include_bytes!("../../../integrations/asuswrt/fixtures/clients-empty.json")
                    .as_slice()
            }
            _ => br#"{"get_clientlist":{"aa:bb:cc:dd:ee:02":{"isOnline":"1","isWL":"1"}}}"#,
        };
        result.push((
            Action::Clients,
            Some(model::parse(Action::Clients, clients).unwrap()),
            1,
        ));
    }
    result
}
impl Driver for Stage {
    fn kind(&self) -> &'static str {
        "asuswrt"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let mut state = Published::new(ctx);
            let selected = if matches!(self.mode, Mode::NoInventory) {
                vec![]
            } else {
                vec!["aabbccddee02".to_owned()]
            };
            state
                .apply("aabbccddee01", &selected, reads(self.mode), 1)
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
            state.ctx.ready();
            if let Some(done) = self.done.lock().unwrap().take() {
                let _ = done.send(());
            }
            state.ctx.cancelled().await;
            Ok(())
        })
    }
}
async fn stage(hub: &Hub, mode: Mode, verify: impl FnOnce(&Hub)) {
    let (done, ready) = tokio::sync::oneshot::channel();
    let cancel = CancellationToken::new();
    let task = spawn_driver(
        hub,
        InstanceId::from("network"),
        Arc::new(Stage {
            mode,
            done: Mutex::new(Some(done)),
        }),
        cancel.clone(),
    );
    tokio::time::timeout(Duration::from_secs(2), ready)
        .await
        .unwrap()
        .unwrap();
    verify(hub);
    cancel.cancel();
    task.await.unwrap();
}
fn keys(hub: &Hub, native: &str) -> Vec<String> {
    let id = DeviceId::from(format!("network:{native}"));
    hub.describe(&id)
        .unwrap()
        .0
        .points
        .iter()
        .map(|p| p.key.to_string())
        .collect()
}
fn value(hub: &Hub, native: &str, key: &str) -> Value {
    hub.state(&PointId::from(format!("network:{native}/{key}")))
        .unwrap()
        .value
}
#[tokio::test]
async fn restart_partial_read_absence_and_return_keep_the_same_definitions() {
    let hub = Hub::new(HubOptions::default()).unwrap();
    stage(&hub, Mode::Full, |hub| {
        assert_eq!(keys(hub, ROUTER).len(), 8);
        assert_eq!(keys(hub, CLIENT).len(), 4);
    })
    .await;
    let router_keys = keys(&hub, ROUTER);
    let client_keys = keys(&hub, CLIENT);
    stage(&hub, Mode::Partial, |hub| {
        assert_eq!(
            keys(hub, ROUTER),
            router_keys,
            "known router capabilities must survive restart"
        );
        assert_eq!(
            keys(hub, CLIENT),
            client_keys,
            "known client capabilities must survive restart"
        );
        for key in ["firmware", "memory_free", "rx_bytes", "tx_bytes"] {
            assert_eq!(value(hub, ROUTER, key), Value::Null);
        }
        for key in ["signal", "rate_rx", "rate_tx"] {
            assert_eq!(value(hub, CLIENT, key), Value::Null);
        }
    })
    .await;
    stage(&hub, Mode::Absent, |hub| {
        assert_eq!(keys(hub, CLIENT), client_keys);
        for key in &client_keys {
            assert_eq!(value(hub, CLIENT, key), Value::Null);
        }
        assert_eq!(
            hub.snapshot()
                .devices
                .iter()
                .find(|d| d.device.id.as_str() == format!("network:{CLIENT}"))
                .unwrap()
                .online,
            Some(false)
        );
    })
    .await;
    stage(&hub, Mode::Full, |hub| {
        assert_eq!(keys(hub, ROUTER), router_keys);
        assert_eq!(keys(hub, CLIENT), client_keys);
        assert_eq!(value(hub, CLIENT, "signal"), Value::Int(-55));
        assert_ne!(value(hub, CLIENT, "rate_rx"), Value::Null);
        assert!(
            hub.snapshot()
                .devices
                .iter()
                .map(|d| d.device.points.len())
                .sum::<usize>()
                <= 139
        );
    })
    .await;
}
fn cached(include_client: bool) -> (Hub, std::path::PathBuf) {
    let router: serde_json::Map<String, serde_json::Value> = ["firmware","cpu","memory","memory_total","memory_used","memory_free","rx_bytes","tx_bytes","rx_rate","tx_rate","client_count"].into_iter().map(|key| (key.to_owned(), serde_json::json!({"value": if key=="firmware" {serde_json::json!("stale")} else {serde_json::json!(999)},"ts":1}))).collect();
    let client: serde_json::Map<String, serde_json::Value> = ["link","signal","rate_rx","rate_tx"].into_iter().map(|key| (key.to_owned(), serde_json::json!({"value": if key=="link" {serde_json::json!("wireless")} else {serde_json::json!(999)},"ts":1}))).collect();
    let mut cache = serde_json::json!({format!("network:{ROUTER}"):router});
    // Cached keys from another role are never adopted into a device definition.
    cache[format!("network:{ROUTER}")]["signal"] = serde_json::json!({"value":999,"ts":1});
    if include_client {
        cache[format!("network:{CLIENT}")] = serde_json::json!(client);
        cache[format!("network:{CLIENT}")]["cpu"] = serde_json::json!({"value":999,"ts":1});
    }
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "moli-asus-reconcile-{}-{suffix}.json",
        std::process::id()
    ));
    std::fs::write(&path, serde_json::to_vec(&cache).unwrap()).unwrap();
    let hub = Hub::new(HubOptions {
        state_path: Some(path.clone()),
        ..HubOptions::default()
    })
    .unwrap();
    (hub, path)
}
#[tokio::test]
async fn cold_cache_retains_legal_keys_but_does_not_reuse_old_values_as_fresh() {
    let (hub, path) = cached(true);
    assert!(hub.snapshot().devices.is_empty());
    stage(&hub, Mode::Partial, |hub| {
        assert_eq!(
            keys(hub, ROUTER).len(),
            11,
            "cached capabilities need a first complete declaration"
        );
        assert_eq!(keys(hub, CLIENT).len(), 4);
        assert!(!keys(hub, CLIENT).iter().any(|key| key == "cpu"));
        assert!(!keys(hub, ROUTER).iter().any(|key| key == "signal"));
        for key in [
            "firmware",
            "cpu",
            "memory_free",
            "rx_bytes",
            "tx_bytes",
            "rx_rate",
            "tx_rate",
        ] {
            assert_eq!(value(hub, ROUTER, key), Value::Null);
        }
        for key in ["signal", "rate_rx", "rate_tx"] {
            assert_eq!(value(hub, CLIENT, key), Value::Null);
        }
        assert_eq!(value(hub, ROUTER, "memory_total"), Value::Int(102_400));
        assert_eq!(value(hub, ROUTER, "client_count"), Value::Int(1));
    })
    .await;
    stage(&hub, Mode::Full, |hub| {
        assert_eq!(keys(hub, ROUTER).len(), 11);
        assert_eq!(keys(hub, CLIENT).len(), 4);
        assert_eq!(value(hub, CLIENT, "signal"), Value::Int(-55));
        assert_eq!(
            value(hub, ROUTER, "cpu"),
            Value::Null,
            "delta reference resets at restart"
        );
    })
    .await;
    std::fs::remove_file(path).unwrap();
}
#[tokio::test]
async fn cached_count_is_unknown_when_the_first_cycle_does_not_read_inventory() {
    let (hub, path) = cached(false);
    stage(&hub, Mode::NoInventory, |hub| {
        assert_eq!(keys(hub, ROUTER).len(), 11);
        assert_eq!(
            value(hub, ROUTER, "client_count"),
            Value::Null,
            "an unrefreshed cached count is never fresh"
        );
        assert_eq!(hub.snapshot().devices.len(), 1);
    })
    .await;
    std::fs::remove_file(path).unwrap();
}
#[tokio::test]
async fn fresh_partial_devices_do_not_gain_never_observed_capabilities() {
    let hub = Hub::new(HubOptions::default()).unwrap();
    for _ in 0..2 {
        stage(&hub, Mode::Partial, |hub| {
            assert_eq!(keys(hub, ROUTER).len(), 4);
            assert_eq!(keys(hub, CLIENT), vec!["link"]);
            assert!(
                hub.state(&PointId::from(format!("network:{CLIENT}/signal")))
                    .is_none()
            );
            assert!(
                hub.state(&PointId::from(format!("network:{ROUTER}/cpu")))
                    .is_none()
            );
        })
        .await;
    }
}
