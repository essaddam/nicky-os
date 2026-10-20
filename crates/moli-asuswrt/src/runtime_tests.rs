use super::*;
use crate::network::tests::Server;
use moli_core::{InstanceId, Origin, PointId, Value};
use moli_runtime::{CommandError, Hub, HubOptions, MasterKey, spawn_driver};
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use tokio_util::sync::CancellationToken;
fn point(key: &str) -> PointId {
    PointId::from(format!("network:router_aabbccddee01/{key}"))
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
        "moli-asuswrt-{}-{suffix}-{index}",
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
async fn vault_and_credentials_are_required_before_any_network_request() {
    let server = Server::start().await;
    for hub in [
        Hub::new(HubOptions::default()).unwrap(),
        vault_with_secrets(false).await.hub.clone(),
    ] {
        let cancel = CancellationToken::new();
        let task = spawn_driver(
            &hub,
            InstanceId::from("network"),
            Arc::new(Asuswrt::new(server.config()).unwrap()),
            cancel.clone(),
        );
        until(|| {
            hub.snapshot()
                .drivers
                .iter()
                .any(|d| matches!(d.status, moli_core::DriverStatus::Waiting { .. }))
        })
        .await;
        assert!(server.calls.lock().unwrap().is_empty());
        cancel.cancel();
        task.await.unwrap();
    }
}
#[tokio::test]
async fn selected_clients_only_readonly_points_no_presence_and_identity_persisted() {
    let server = Server::start().await;
    let vault = vault().await;
    let hub = &vault.hub;
    let mut config = server.config();
    config.selected = vec!["aa:bb:cc:dd:ee:02".into()];
    let cancel = CancellationToken::new();
    let task = spawn_driver(
        hub,
        InstanceId::from("network"),
        Arc::new(Asuswrt::new(config).unwrap()),
        cancel.clone(),
    );
    until(|| hub.state(&point("client_count")).is_some()).await;
    assert_eq!(
        hub.state(&point("memory")).unwrap().value,
        Value::Float(25.0)
    );
    assert_eq!(
        hub.state(&point("client_count")).unwrap().value,
        Value::Int(2)
    );
    assert_eq!(hub.snapshot().devices.len(), 2);
    for d in &hub.snapshot().devices {
        for spec in &d.device.points {
            assert!(!spec.access.write);
            assert!(
                ![
                    "ip",
                    "mac",
                    "ssid",
                    "name",
                    "presence",
                    "occupancy",
                    "associated"
                ]
                .contains(&spec.key.as_ref())
            );
            assert!(matches!(
                spec.semantic,
                moli_core::Semantic::Diagnostic | moli_core::Semantic::SignalStrength
            ));
        }
    }
    let count = server.calls.lock().unwrap().len();
    assert_eq!(
        hub.command(&point("memory"), Value::Int(0), Origin::Mcp, None)
            .await,
        Err(CommandError::ReadOnly)
    );
    assert_eq!(server.calls.lock().unwrap().len(), count);
    assert_eq!(
        hub.secret(&InstanceId::from("network"), "device_identity")
            .as_deref(),
        Some("aabbccddee01")
    );
    cancel.cancel();
    task.await.unwrap();
    assert!(
        hub.snapshot()
            .devices
            .iter()
            .all(|d| d.online == Some(false))
    );
    assert_eq!(hub.state(&point("memory")).unwrap().value, Value::Null);
}
async fn advance(seconds: u64) {
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(seconds)).await;
    tokio::time::resume();
}
#[tokio::test]
async fn cadences_stable_events_missing_clients_and_invalid_inventory_remain_atomic() {
    let server = Server::start().await;
    let vault = vault().await;
    let hub = &vault.hub;
    let mut config = server.config();
    config.selected = vec!["aa:bb:cc:dd:ee:02".into()];
    let cancel = CancellationToken::new();
    let task = spawn_driver(
        hub,
        InstanceId::from("network"),
        Arc::new(Asuswrt::new(config).unwrap()),
        cancel.clone(),
    );
    until(|| hub.state(&point("client_count")).is_some()).await;
    tokio::time::sleep(Duration::from_millis(10)).await;
    let mut events = hub.subscribe();
    let first = server.calls.lock().unwrap().len();
    advance(60).await;
    until(|| server.calls.lock().unwrap().len() == first + 1).await;
    tokio::time::sleep(Duration::from_millis(10)).await;
    while let Ok(e) = events.try_recv() {
        assert!(!matches!(e, moli_core::Event::State { .. }));
    }
    server.mode.store(4, Ordering::SeqCst);
    advance(300).await;
    let client = PointId::from("network:client_aabbccddee02/signal");
    until(|| hub.state(&client).is_some_and(|s| s.value == Value::Null)).await;
    assert!(
        hub.state(&PointId::from("network:client_aabbccddee02/associated"))
            .is_none()
    );
    server.mode.store(1, Ordering::SeqCst);
    advance(300).await;
    until(|| {
        hub.state(&point("memory"))
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
async fn terminal_refusal_never_reauthenticates_and_inflight_requests_cancel() {
    for mode in [3, 5] {
        let server = Server::start().await;
        server.mode.store(mode, Ordering::SeqCst);
        let vault = vault().await;
        let hub = &vault.hub;
        let cancel = CancellationToken::new();
        let task = spawn_driver(
            hub,
            InstanceId::from("network"),
            Arc::new(Asuswrt::new(server.config()).unwrap()),
            cancel.clone(),
        );
        if mode == 5 {
            until(|| {
                hub.snapshot()
                    .drivers
                    .iter()
                    .any(|d| matches!(d.status, moli_core::DriverStatus::Waiting { .. }))
            })
            .await;
            assert_eq!(
                *server.calls.lock().unwrap(),
                vec!["login", "info", "telemetry"]
            );
        } else {
            until(|| !server.calls.lock().unwrap().is_empty()).await;
        }
        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap();
        assert!(hub.snapshot().devices.is_empty());
    }
}
#[tokio::test]
async fn initial_tls_transport_failure_backs_off_without_human_intervention() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            drop(stream);
        }
    });
    let vault = vault().await;
    let hub = &vault.hub;
    let config: Config = serde_json::from_value(serde_json::json!({
        "host": "127.0.0.1", "port": port, "transport": "https"
    }))
    .unwrap();
    let cancel = CancellationToken::new();
    let task = spawn_driver(
        hub,
        InstanceId::from("network"),
        Arc::new(Asuswrt::new(config).unwrap()),
        cancel.clone(),
    );
    until(|| {
        hub.snapshot().drivers.iter().any(|d| {
            matches!(
                d.status,
                moli_core::DriverStatus::Backoff { .. } | moli_core::DriverStatus::Waiting { .. }
            )
        })
    })
    .await;
    let status = hub.snapshot().drivers[0].status.clone();
    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap();
    server.abort();
    let _ = server.await;
    assert!(
        matches!(status, moli_core::DriverStatus::Backoff { .. }),
        "{status:?}"
    );
}

#[tokio::test]
async fn inventory_of_512_creates_only_selected_points_under_the_hard_ceiling() {
    for selection in [0, 32] {
        let server = Server::start().await;
        server.mode.store(6, Ordering::SeqCst);
        let vault = vault().await;
        let hub = &vault.hub;
        let mut config = server.config();
        config.selected = (0..selection)
            .map(|i| format!("aa:bb:cc:dd:00:{i:02x}"))
            .collect();
        let cancel = CancellationToken::new();
        let task = spawn_driver(
            hub,
            InstanceId::from("network"),
            Arc::new(Asuswrt::new(config).unwrap()),
            cancel.clone(),
        );
        until(|| hub.state(&point("client_count")).is_some()).await;
        assert_eq!(
            hub.state(&point("client_count")).unwrap().value,
            Value::Int(512)
        );
        assert!(hub.state(&point("cpu")).is_none());
        tokio::time::sleep(Duration::from_millis(10)).await;
        advance(60).await;
        until(|| hub.state(&point("cpu")).is_some()).await;
        let snapshot = hub.snapshot();
        assert_eq!(snapshot.devices.len(), 1 + selection);
        let count = snapshot
            .devices
            .iter()
            .map(|d| d.device.points.len())
            .sum::<usize>();
        assert_eq!(count, 11 + selection * 4);
        assert!(count <= 139);
        cancel.cancel();
        task.await.unwrap();
    }
}
