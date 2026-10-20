//! Driver and hub contracts, with an ephemeral encrypted vault and loopback service.
use super::*;
use crate::network::tests::Server;
use moli_core::{InstanceId, Origin, PointId, Value};
use moli_runtime::{CommandError, Hub, HubOptions, MasterKey, spawn_driver};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

fn point(key: &str) -> PointId {
    PointId::from(format!("network:router_synthetic-router-01/{key}"))
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
        "moli-netgear-{}-{suffix}-{index}",
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
async fn missing_vault_never_contacts_the_endpoint_and_stops_quickly() {
    let server = Server::start(0, vec![], Duration::ZERO).await;
    let hub = Hub::new(HubOptions::default()).unwrap();
    let cancel = CancellationToken::new();
    let task = spawn_driver(
        &hub,
        InstanceId::from("network"),
        Arc::new(Netgear::new(server.config()).unwrap()),
        cancel.clone(),
    );
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(server.actions.lock().unwrap().is_empty());
    assert!(hub.snapshot().devices.is_empty());
    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap();
}
#[tokio::test]
async fn route_and_selected_clients_are_read_only_without_metadata_points() {
    let server = Server::start(0, vec![], Duration::ZERO).await;
    let vault = vault().await;
    let hub = &vault.hub;
    let cancel = CancellationToken::new();
    let mut config = server.config();
    config.selected = vec!["aa:bb:cc:dd:ee:01".into()];
    let task = spawn_driver(
        hub,
        InstanceId::from("network"),
        Arc::new(Netgear::new(config).unwrap()),
        cancel.clone(),
    );
    until(|| hub.state(&point("cpu")).is_some()).await;
    assert_eq!(hub.state(&point("cpu")).unwrap().value, Value::Int(12));
    assert_eq!(hub.snapshot().devices.len(), 2);
    let count = server.actions.lock().unwrap().len();
    for device in &hub.snapshot().devices {
        for spec in &device.device.points {
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
        }
    }
    assert_eq!(
        hub.command(&point("cpu"), Value::Int(0), Origin::Mcp, None)
            .await,
        Err(CommandError::ReadOnly)
    );
    assert_eq!(server.actions.lock().unwrap().len(), count);
    assert_eq!(
        hub.secret(&InstanceId::from("network"), "device_identity")
            .as_deref(),
        Some("synthetic-router-01")
    );
    cancel.cancel();
    task.await.unwrap();
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
async fn stable_samples_do_not_emit_state_and_invalid_inventory_is_atomic() {
    let server = Server::start(0, vec![], Duration::ZERO).await;
    let vault = vault().await;
    let hub = &vault.hub;
    let cancel = CancellationToken::new();
    let mut config = server.config();
    config.selected = vec!["aa:bb:cc:dd:ee:01".into()];
    let task = spawn_driver(
        hub,
        InstanceId::from("network"),
        Arc::new(Netgear::new(config).unwrap()),
        cancel.clone(),
    );
    until(|| hub.state(&point("cpu")).is_some()).await;
    tokio::time::sleep(Duration::from_millis(10)).await;
    let first = server.actions.lock().unwrap().len();
    let mut events = hub.subscribe();
    advance(60).await;
    until(|| server.actions.lock().unwrap().len() >= first + 3).await;
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(
        server.actions.lock().unwrap().len(),
        first + 3,
        "only measurements are due"
    );
    while let Ok(event) = events.try_recv() {
        assert!(
            !matches!(event, moli_core::Event::State { .. }),
            "identical observations stay quiet"
        );
    }
    server.mode.store(1, std::sync::atomic::Ordering::SeqCst);
    advance(300).await;
    until(|| {
        hub.state(&point("cpu"))
            .is_some_and(|s| s.value == Value::Null)
    })
    .await;
    assert!(
        hub.snapshot()
            .devices
            .iter()
            .all(|d| d.online == Some(false))
    );
    assert!(
        hub.state(&PointId::from("network:client_aabbccddee01/associated"))
            .is_none()
    );
    cancel.cancel();
    task.await.unwrap();
}
#[tokio::test]
async fn maximum_selection_is_bounded_and_empty_selection_never_reads_inventory() {
    for selected in [false, true] {
        let server = Server::start(0, vec![], Duration::ZERO).await;
        server.mode.store(3, std::sync::atomic::Ordering::SeqCst);
        let vault = vault().await;
        let hub = &vault.hub;
        let cancel = CancellationToken::new();
        let mut config = server.config();
        if selected {
            config.selected = (0..32).map(|i| format!("aa:bb:cc:dd:ee:{i:02x}")).collect();
        }
        let task = spawn_driver(
            hub,
            InstanceId::from("network"),
            Arc::new(Netgear::new(config).unwrap()),
            cancel.clone(),
        );
        until(|| hub.state(&point("cpu")).is_some()).await;
        let snapshot = hub.snapshot();
        assert_eq!(snapshot.devices.len(), if selected { 33 } else { 1 });
        let points = snapshot
            .devices
            .iter()
            .map(|d| d.device.points.len())
            .sum::<usize>();
        assert_eq!(points, if selected { 101 } else { 5 });
        assert!(points <= 144);
        let actions = server.actions.lock().unwrap().clone();
        assert_eq!(actions.len(), if selected { 6 } else { 5 });
        assert_eq!(
            actions
                .iter()
                .filter(|a| a.ends_with("#GetAttachDevice2"))
                .count(),
            usize::from(selected)
        );
        cancel.cancel();
        task.await.unwrap();
    }
}
#[tokio::test]
async fn remote_authentication_errors_are_redacted_and_never_loop() {
    let marker = format!(
        "private-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let body = format!("<html>{marker}</html>").into_bytes();
    let server = Server::start(200, body, Duration::ZERO).await;
    let vault = vault().await;
    let hub = &vault.hub;
    let cancel = CancellationToken::new();
    let task = spawn_driver(
        hub,
        InstanceId::from("network"),
        Arc::new(Netgear::new(server.config()).unwrap()),
        cancel.clone(),
    );
    until(|| server.actions.lock().unwrap().len() == 1).await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(
        !serde_json::to_string(&hub.snapshot())
            .unwrap()
            .contains(&marker)
    );
    assert!(hub.snapshot().devices.is_empty());
    assert_eq!(server.actions.lock().unwrap().len(), 1);
    cancel.cancel();
    task.await.unwrap();
}
#[tokio::test]
async fn an_empty_vault_does_not_connect_and_inflight_login_and_reads_cancel() {
    let server = Server::start(0, vec![], Duration::ZERO).await;
    let empty_vault = vault_with_secrets(false).await;
    let cancel = CancellationToken::new();
    let task = spawn_driver(
        &empty_vault.hub,
        InstanceId::from("network"),
        Arc::new(Netgear::new(server.config()).unwrap()),
        cancel.clone(),
    );
    until(|| {
        empty_vault
            .hub
            .snapshot()
            .drivers
            .iter()
            .any(|d| matches!(d.status, moli_core::DriverStatus::Waiting { .. }))
    })
    .await;
    assert!(server.actions.lock().unwrap().is_empty());
    cancel.cancel();
    task.await.unwrap();
    for login in [true, false] {
        let server = Server::start(0, vec![], Duration::ZERO).await;
        let vault = vault().await;
        let cancel = CancellationToken::new();
        if login {
            server.mode.store(4, std::sync::atomic::Ordering::SeqCst);
        }
        let task = spawn_driver(
            &vault.hub,
            InstanceId::from("network"),
            Arc::new(Netgear::new(server.config()).unwrap()),
            cancel.clone(),
        );
        if !login {
            until(|| vault.hub.state(&point("cpu")).is_some()).await;
            server.mode.store(4, std::sync::atomic::Ordering::SeqCst);
            advance(60).await;
        }
        let minimum = if login { 1 } else { 6 };
        until(|| server.actions.lock().unwrap().len() >= minimum).await;
        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap();
        assert!(
            vault
                .hub
                .snapshot()
                .devices
                .iter()
                .all(|d| d.online == Some(false))
        );
        if !login {
            assert_eq!(vault.hub.state(&point("cpu")).unwrap().value, Value::Null);
        }
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
        "host": "127.0.0.1", "port": port, "transport": "https", "login": "v2", "inventory": "v2"
    }))
    .unwrap();
    let cancel = CancellationToken::new();
    let task = spawn_driver(
        hub,
        InstanceId::from("network"),
        Arc::new(Netgear::new(config).unwrap()),
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
async fn explicit_lock_parks_the_driver_without_sending_credentials_again() {
    let server = Server::start(0, vec![], Duration::ZERO).await;
    server.mode.store(5, std::sync::atomic::Ordering::SeqCst);
    let vault = vault().await;
    let hub = &vault.hub;
    let cancel = CancellationToken::new();
    let task = spawn_driver(
        hub,
        InstanceId::from("network"),
        Arc::new(Netgear::new(server.config()).unwrap()),
        cancel.clone(),
    );
    until(|| {
        hub.snapshot()
            .drivers
            .iter()
            .any(|d| matches!(d.status, moli_core::DriverStatus::Waiting { .. }))
    })
    .await;
    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap();
    let actions = server.actions.lock().unwrap();
    assert_eq!(
        actions.len(),
        3,
        "initial login, identity and refused read only"
    );
    assert!(actions[0].ends_with("#SOAPLogin"));
    assert!(actions[1].ends_with("#GetInfo"));
    assert!(actions[2].ends_with("#GetSystemInfo"));
    assert!(hub.snapshot().devices.is_empty());
}
