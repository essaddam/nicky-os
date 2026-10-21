//! Reconciliation uses real hub definitions/cache, with no modem or transport.
use crate::{
    Failure,
    model::{self, Action},
    publication::{KEYS, Published, Reads},
};
use moli_core::{DeviceId, InstanceId, PointId, Value};
use moli_runtime::{BoxFuture, Driver, DriverCtx, Hub, HubOptions, spawn_driver};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
#[derive(Clone, Copy)]
enum Mode {
    Full,
    NoSim,
    Missing,
}
struct Stage {
    mode: Mode,
    done: Mutex<Option<tokio::sync::oneshot::Sender<Result<(), Failure>>>>,
}
fn reads(mode: Mode) -> Reads {
    let status = if matches!(mode, Mode::NoSim) {
        include_bytes!("../../../integrations/huawei-lte/fixtures/no-sim.xml").as_slice()
    } else {
        include_bytes!("../../../integrations/huawei-lte/fixtures/status.xml").as_slice()
    };
    let observed = Instant::now();
    let mut reads = vec![(
        Action::Status,
        Some(model::parse(Action::Status, status).unwrap()),
        1,
        observed,
    )];
    for (action, body) in [
        (
            Action::Signal,
            include_bytes!("../../../integrations/huawei-lte/fixtures/signal.xml").as_slice(),
        ),
        (
            Action::Traffic,
            include_bytes!("../../../integrations/huawei-lte/fixtures/traffic.xml").as_slice(),
        ),
    ] {
        reads.push((
            action,
            matches!(mode, Mode::Full).then(|| model::parse(action, body).unwrap()),
            1,
            observed,
        ));
    }
    reads
}
impl Driver for Stage {
    fn kind(&self) -> &'static str {
        "huawei-lte"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let mut state = Published::new(ctx);
            let result = state
                .check_known()
                .and_then(|()| state.apply("synthetic-modem-01", reads(self.mode), 1));
            if result.is_ok() {
                state.ctx.ready();
            }
            if let Some(done) = self.done.lock().unwrap().take() {
                let _ = done.send(result);
            }
            state.ctx.cancelled().await;
            Ok(())
        })
    }
}
async fn stage(hub: &Hub, mode: Mode, verify: impl FnOnce(&Hub)) -> Result<(), Failure> {
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
    let result = tokio::time::timeout(Duration::from_secs(2), ready)
        .await
        .unwrap()
        .unwrap();
    verify(hub);
    cancel.cancel();
    task.await.unwrap();
    result
}
fn assert_known(hub: &Hub, complete: bool, no_sim: bool) {
    let snapshot = hub.snapshot();
    assert_eq!(snapshot.devices.len(), 1);
    let device = &snapshot.devices[0].device;
    assert_eq!(
        device.id,
        DeviceId::from("network:modem_synthetic-modem-01")
    );
    assert_eq!(
        device.points.len(),
        11,
        "known capabilities must survive a restart"
    );
    for key in KEYS {
        assert!(device.point(key).is_some(), "lost definition: {key}");
        let value = &hub.state(&PointId::new(&device.id, key)).unwrap().value;
        if !complete && !matches!(key, "connected" | "technology") || no_sim && key == "technology"
        {
            assert_eq!(*value, Value::Null, "missing value: {key}");
        }
    }
    let rate = hub
        .state(&PointId::new(&device.id, "rx_rate"))
        .unwrap()
        .value;
    assert_eq!(
        rate,
        if complete {
            Value::Int(10)
        } else {
            Value::Null
        }
    );
}
#[tokio::test]
async fn known_capabilities_survive_restart_without_sim_or_optional_methods_then_return() {
    let hub = Hub::new(HubOptions::default()).unwrap();
    for mode in [Mode::Full, Mode::NoSim, Mode::Missing, Mode::Full] {
        assert_eq!(
            stage(&hub, mode, |hub| assert_known(
                hub,
                matches!(mode, Mode::Full),
                matches!(mode, Mode::NoSim)
            ))
            .await,
            Ok(())
        );
    }
}
#[tokio::test]
async fn a_fresh_modem_without_sim_does_not_gain_never_observed_capabilities() {
    let hub = Hub::new(HubOptions::default()).unwrap();
    for _ in 0..2 {
        assert_eq!(
            stage(&hub, Mode::NoSim, |hub| {
                let snapshot = hub.snapshot();
                assert_eq!(snapshot.devices.len(), 1);
                assert_eq!(snapshot.devices[0].device.points.len(), 1);
                assert!(snapshot.devices[0].device.point("connected").is_some());
                assert!(
                    hub.state(&PointId::from("network:modem_synthetic-modem-01/rssi"))
                        .is_none()
                );
            })
            .await,
            Ok(())
        );
    }
}
fn cache(values: &serde_json::Value) -> (Hub, std::path::PathBuf) {
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "moli-huawei-reconcile-{}-{suffix}.json",
        std::process::id()
    ));
    std::fs::write(&path, serde_json::to_vec(values).unwrap()).unwrap();
    let hub = Hub::new(HubOptions {
        state_path: Some(path.clone()),
        ..HubOptions::default()
    })
    .unwrap();
    (hub, path)
}
#[tokio::test]
async fn cold_cache_reconstructs_the_known_definition_even_with_no_signal_or_traffic() {
    let samples: serde_json::Map<String, serde_json::Value> = KEYS
        .iter()
        .map(|key| ((*key).to_owned(), serde_json::json!({"value":null,"ts":1})))
        .collect();
    let (hub, path) = cache(&serde_json::json!({"network:modem_synthetic-modem-01":samples}));
    assert!(hub.snapshot().devices.is_empty());
    assert_eq!(
        stage(&hub, Mode::NoSim, |hub| assert_known(hub, false, true)).await,
        Ok(())
    );
    assert_eq!(
        stage(&hub, Mode::Full, |hub| assert_known(hub, true, false)).await,
        Ok(())
    );
    std::fs::remove_file(path).unwrap();
}
#[tokio::test]
async fn incompatible_cache_keys_and_another_identity_cannot_be_published() {
    for (identity, key) in [
        ("synthetic-modem-01", "foreign_capability"),
        ("synthetic-other", "connected"),
    ] {
        let (hub, path) = cache(
            &serde_json::json!({format!("network:modem_{identity}"):{(key):{"value":null,"ts":1}}}),
        );
        assert_eq!(
            stage(&hub, Mode::Full, |hub| assert!(
                hub.snapshot().devices.is_empty()
            ))
            .await,
            Err(Failure::Identity)
        );
        std::fs::remove_file(path).unwrap();
    }
}
