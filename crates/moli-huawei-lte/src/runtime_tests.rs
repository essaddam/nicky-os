use super::*;
use crate::{model::Action, network::tests::Server, protocol::Call};
use moli_core::{DriverStatus, InstanceId, Origin, PointId, Value};
use moli_runtime::{CommandError, Hub, HubOptions, MasterKey, spawn_driver};
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use tokio_util::sync::CancellationToken;
fn point(key: &str) -> PointId {
    PointId::from(format!("network:modem_synthetic-modem-01/{key}"))
}
async fn until(mut ready: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("observable contract outcome");
}
pub(crate) struct Vault {
    pub hub: Hub,
    path: std::path::PathBuf,
}
impl Drop for Vault {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
pub(crate) async fn vault() -> Vault {
    vault_with_secrets(true).await
}
async fn vault_with_secrets(secrets: bool) -> Vault {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let index = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "moli-huawei-lte-{}-{suffix}-{index}",
        std::process::id()
    ));
    std::fs::create_dir(&path).unwrap();
    let hub = Hub::new(HubOptions {
        secrets: Some((path.join("vault.enc"), MasterKey::generate().unwrap())),
        ..HubOptions::default()
    })
    .unwrap();
    for key in ["username", "password"].into_iter().filter(|_| secrets) {
        hub.store_secret(
            &InstanceId::from("network"),
            key,
            &format!("{key}-{suffix}"),
        )
        .await
        .unwrap();
    }
    Vault { hub, path }
}

#[tokio::test]
async fn eleven_readonly_points_are_diagnostic_without_any_mobile_identifiers() {
    let server = Server::start().await;
    let vault = vault().await;
    let hub = &vault.hub;
    let cancel = CancellationToken::new();
    let task = spawn_driver(
        hub,
        InstanceId::from("network"),
        Arc::new(HuaweiLte::new(server.config()).unwrap()),
        cancel.clone(),
    );
    until(|| hub.state(&point("duration")).is_some()).await;
    assert_eq!(hub.snapshot().devices.len(), 1);
    let snapshot = hub.snapshot();
    let specs = &snapshot.devices[0].device.points;
    assert_eq!(specs.len(), 11);
    for spec in specs {
        assert!(!spec.access.write);
        assert_eq!(
            moli_history::Family::of(spec),
            moli_history::Family::Diagnostic
        );
        assert!(
            ![
                "imei", "iccid", "imsi", "cell_id", "phone", "sms", "operator", "presence"
            ]
            .contains(&spec.key.as_ref())
        );
    }
    assert_eq!(
        hub.state(&point("rx_bytes")).unwrap().value,
        Value::Int(1000)
    );
    assert_eq!(hub.state(&point("rx_rate")).unwrap().value, Value::Int(10));
    assert_eq!(
        hub.state(&point("rssi")).unwrap().value,
        Value::Float(-75.0)
    );
    assert_eq!(
        hub.secret(&InstanceId::from("network"), "device_identity")
            .as_deref(),
        Some("synthetic-modem-01")
    );
    let count = server.calls.lock().unwrap().len();
    assert_eq!(
        hub.command(&point("connected"), Value::Bool(false), Origin::Mcp, None)
            .await,
        Err(CommandError::ReadOnly)
    );
    assert_eq!(server.calls.lock().unwrap().len(), count);
    cancel.cancel();
    task.await.unwrap();
    assert_eq!(hub.state(&point("rx_rate")).unwrap().value, Value::Null);
    assert!(
        hub.snapshot()
            .devices
            .iter()
            .all(|d| d.online == Some(false))
    );
}
async fn advance(seconds: u64) {
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(seconds)).await;
    tokio::time::resume();
}
#[tokio::test]
async fn unchanged_samples_stay_quiet_then_missing_sim_and_network_clear_traffic() {
    let server = Server::start().await;
    let vault = vault().await;
    let hub = &vault.hub;
    let cancel = CancellationToken::new();
    let task = spawn_driver(
        hub,
        InstanceId::from("network"),
        Arc::new(HuaweiLte::new(server.config()).unwrap()),
        cancel.clone(),
    );
    until(|| hub.state(&point("duration")).is_some()).await;
    tokio::time::sleep(Duration::from_millis(10)).await;
    let mut events = hub.subscribe();
    let first = server.calls.lock().unwrap().len();
    advance(60).await;
    until(|| server.calls.lock().unwrap().len() == first + 3).await;
    tokio::time::sleep(Duration::from_millis(10)).await;
    while let Ok(event) = events.try_recv() {
        assert!(!matches!(event, moli_core::Event::State { .. }));
    }
    server.mode.store(4, Ordering::SeqCst);
    let before = server.calls.lock().unwrap().len();
    advance(60).await;
    until(|| {
        hub.state(&point("connected"))
            .is_some_and(|s| s.value == Value::Bool(false))
    })
    .await;
    assert_eq!(server.calls.lock().unwrap().len(), before + 1);
    assert_eq!(hub.state(&point("rssi")).unwrap().value, Value::Null);
    assert_eq!(hub.state(&point("rx_rate")).unwrap().value, Value::Null);
    server.mode.store(5, Ordering::SeqCst);
    let before = server.calls.lock().unwrap().len();
    advance(60).await;
    until(|| server.calls.lock().unwrap().len() == before + 2).await;
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(
        hub.state(&point("rssi")).unwrap().value,
        Value::Float(-75.0)
    );
    assert_eq!(hub.state(&point("rx_bytes")).unwrap().value, Value::Null);
    server.mode.store(0, Ordering::SeqCst);
    advance(60).await;
    until(|| {
        hub.state(&point("rx_rate"))
            .is_some_and(|s| s.value == Value::Int(10))
    })
    .await;
    server.mode.store(1, Ordering::SeqCst);
    advance(60).await;
    until(|| {
        hub.state(&point("connected"))
            .is_some_and(|s| s.value == Value::Null)
    })
    .await;
    assert!(
        hub.snapshot()
            .devices
            .iter()
            .all(|d| d.online == Some(false))
    );
    cancel.cancel();
    task.await.unwrap();
}
#[tokio::test]
async fn missing_credentials_refusal_and_cancellation_never_create_a_retry_loop() {
    for mode in [0, 3, 6] {
        let server = Server::start().await;
        server.mode.store(mode, Ordering::SeqCst);
        let vault = vault_with_secrets(mode != 0).await;
        let hub = &vault.hub;
        let cancel = CancellationToken::new();
        let task = spawn_driver(
            hub,
            InstanceId::from("network"),
            Arc::new(HuaweiLte::new(server.config()).unwrap()),
            cancel.clone(),
        );
        if mode == 3 {
            until(|| !server.calls.lock().unwrap().is_empty()).await;
        } else {
            until(|| {
                hub.snapshot()
                    .drivers
                    .iter()
                    .any(|d| matches!(d.status, DriverStatus::Waiting { .. }))
            })
            .await;
        }
        let calls = server.calls.lock().unwrap().clone();
        assert_eq!(
            calls.len(),
            match mode {
                0 => 0,
                3 => 1,
                6 => 3,
                _ => unreachable!(),
            }
        );
        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap();
        assert!(hub.snapshot().devices.is_empty());
    }
}
#[tokio::test]
async fn csrf_recovery_replays_only_traffic_and_preserves_the_point_ceiling() {
    let server = Server::start().await;
    server.mode.store(9, Ordering::SeqCst);
    let vault = vault().await;
    let hub = &vault.hub;
    let cancel = CancellationToken::new();
    let task = spawn_driver(
        hub,
        InstanceId::from("network"),
        Arc::new(HuaweiLte::new(server.config()).unwrap()),
        cancel.clone(),
    );
    until(|| hub.state(&point("duration")).is_some()).await;
    let calls = server.calls.lock().unwrap().clone();
    assert_eq!(calls.iter().filter(|c| **c == Call::Login).count(), 2);
    assert_eq!(
        calls
            .iter()
            .filter(|c| **c == Call::Read(Action::Status))
            .count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|c| **c == Call::Read(Action::Signal))
            .count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|c| **c == Call::Read(Action::Traffic))
            .count(),
        2
    );
    assert_eq!(calls.len(), 12);
    assert_eq!(hub.snapshot().devices[0].device.points.len(), 11);
    cancel.cancel();
    task.await.unwrap();
}

#[tokio::test]
async fn restored_native_identity_prevents_a_second_modem_but_accepts_the_same_one() {
    for native in ["synthetic-other-01", "synthetic-modem-01"] {
        let server = Server::start().await;
        let vault = vault().await;
        let path = vault.path.join("synthetic-cache.json");
        let bytes = serde_json::to_vec(
            &serde_json::json!({format!("network:modem_{native}"):{"rssi":{"value":-55,"ts":1}}}),
        )
        .unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let hub = Hub::new(HubOptions {
            state_path: Some(path.clone()),
            secrets: Some((
                vault.path.join("restored-vault.enc"),
                MasterKey::generate().unwrap(),
            )),
            ..HubOptions::default()
        })
        .unwrap();
        for key in ["username", "password"] {
            hub.store_secret(
                &InstanceId::from("network"),
                key,
                &format!("synthetic-{key}"),
            )
            .await
            .unwrap();
        }
        let cancel = CancellationToken::new();
        let task = spawn_driver(
            &hub,
            InstanceId::from("network"),
            Arc::new(HuaweiLte::new(server.config()).unwrap()),
            cancel.clone(),
        );
        until(|| {
            hub.snapshot().drivers.iter().any(|d| {
                matches!(
                    d.status,
                    DriverStatus::Waiting { .. } | DriverStatus::Running
                )
            })
        })
        .await;
        if native == "synthetic-other-01" {
            assert!(matches!(
                hub.snapshot().drivers[0].status,
                DriverStatus::Waiting { .. }
            ));
            assert!(hub.snapshot().devices.is_empty());
            assert!(
                hub.secret(&InstanceId::from("network"), "device_identity")
                    .is_none()
            );
            assert_eq!(
                std::fs::read(&path).unwrap(),
                bytes,
                "no cache deletion to free a slot"
            );
        } else {
            assert_eq!(hub.snapshot().devices.len(), 1);
            assert_eq!(hub.snapshot().devices[0].device.points.len(), 11);
        }
        cancel.cancel();
        task.await.unwrap();
    }
}
