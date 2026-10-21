//! Reconciliation contracts use the real hub and supervisor, without any network.
use crate::{
    model::{Read, Samples},
    protocol::Action,
    publication::Published,
};
use moli_core::{InstanceId, PointId, Value};
use moli_runtime::{BoxFuture, Driver, DriverCtx, Hub, HubOptions, spawn_driver};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_util::sync::CancellationToken;
struct Stage {
    selected: Vec<String>,
    partial: bool,
    absent: bool,
    done: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}
impl Driver for Stage {
    fn kind(&self) -> &'static str {
        "netgear"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let mut state = Published::new(ctx);
            let samples = if self.partial {
                Samples::from([("link", Value::from("wired"))])
            } else {
                Samples::from([
                    ("link", Value::from("wired")),
                    ("signal", Value::Int(75)),
                    ("rate", Value::Float(100.0)),
                ])
            };
            let clients = self
                .selected
                .iter()
                .filter(|_| !self.absent)
                .map(|id| (id.clone(), samples.clone()))
                .collect();
            let system = if self.partial {
                Samples::from([("cpu", Value::Int(12))])
            } else {
                Samples::from([("cpu", Value::Int(12)), ("memory", Value::Int(20))])
            };
            state
                .apply(
                    "synthetic-router-01",
                    &self.selected,
                    vec![
                        (
                            Action::Info,
                            Some(Read {
                                values: Samples::from([("firmware", Value::from("synthetic"))]),
                                ..Read::default()
                            }),
                        ),
                        (
                            Action::System,
                            Some(Read {
                                values: system,
                                ..Read::default()
                            }),
                        ),
                        (
                            Action::Ethernet,
                            Some(Read {
                                values: Samples::from([("ethernet", Value::Bool(true))]),
                                ..Read::default()
                            }),
                        ),
                        (
                            Action::Traffic,
                            Some(Read {
                                values: Samples::from([("today_connection", Value::Int(100))]),
                                ..Read::default()
                            }),
                        ),
                        (
                            Action::ClientsV2,
                            Some(Read {
                                clients: Some(clients),
                                ..Read::default()
                            }),
                        ),
                    ],
                )
                .map_err(|error| anyhow::anyhow!("{error:?}"))?;
            state.ctx.ready();
            if let Some(done) = self.done.lock().unwrap().take() {
                let _ = done.send(());
            }
            state.ctx.cancelled().await;
            Ok(())
        })
    }
}
async fn stage(
    hub: &Hub,
    selected: Vec<String>,
    partial: bool,
    absent: bool,
    verify: impl FnOnce(&Hub),
) {
    let (done, ready) = tokio::sync::oneshot::channel();
    let cancel = CancellationToken::new();
    let task = spawn_driver(
        hub,
        InstanceId::from("network"),
        Arc::new(Stage {
            selected,
            partial,
            absent,
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
fn selected(first: usize, count: usize) -> Vec<String> {
    (first..first + count)
        .map(|i| format!("aabbccdd{i:04x}"))
        .collect()
}
fn keys(hub: &Hub, native: &str) -> Vec<String> {
    hub.snapshot()
        .devices
        .iter()
        .find(|d| d.device.id.as_str() == format!("network:{native}"))
        .unwrap()
        .device
        .points
        .iter()
        .map(|p| p.key.to_string())
        .collect()
}
#[tokio::test]
async fn capabilities_survive_restart_partial_read_absence_and_return_at_null() {
    let hub = Hub::new(HubOptions::default()).unwrap();
    stage(&hub, selected(0, 1), false, false, |hub| {
        assert_eq!(keys(hub, "client_aabbccdd0000").len(), 3);
        assert_eq!(
            hub.state(&PointId::from("network:client_aabbccdd0000/signal"))
                .unwrap()
                .value,
            Value::Int(75)
        );
    })
    .await;
    stage(&hub, selected(0, 1), true, false, |hub| {
        assert_eq!(
            keys(hub, "client_aabbccdd0000").len(),
            3,
            "partial read must preserve known definitions"
        );
        assert_eq!(
            hub.state(&PointId::from("network:client_aabbccdd0000/signal"))
                .unwrap()
                .value,
            Value::Null
        );
        assert_eq!(
            hub.state(&PointId::from("network:client_aabbccdd0000/rate"))
                .unwrap()
                .value,
            Value::Null
        );
        assert_eq!(keys(hub, "router_synthetic-router-01").len(), 5);
        assert_eq!(
            hub.state(&PointId::from("network:router_synthetic-router-01/memory"))
                .unwrap()
                .value,
            Value::Null
        );
    })
    .await;
    stage(&hub, selected(0, 1), true, true, |hub| {
        assert_eq!(keys(hub, "client_aabbccdd0000").len(), 3);
        assert_eq!(
            hub.state(&PointId::from("network:client_aabbccdd0000/link"))
                .unwrap()
                .value,
            Value::Null
        );
    })
    .await;
    stage(&hub, selected(0, 1), false, false, |hub| {
        assert_eq!(keys(hub, "client_aabbccdd0000").len(), 3);
        assert_eq!(
            hub.state(&PointId::from("network:client_aabbccdd0000/rate"))
                .unwrap()
                .value,
            Value::Float(100.0)
        );
    })
    .await;
}
#[tokio::test]
async fn disjoint_human_selections_replace_old_clients_without_accumulation() {
    let hub = Hub::new(HubOptions::default()).unwrap();
    for first in [0, 0, 32, 64, 0] {
        let ids = selected(first, 32);
        let expected = ids.clone();
        stage(&hub, ids, false, false, |hub| {
            let snapshot = hub.snapshot();
            let count = snapshot
                .devices
                .iter()
                .map(|d| d.device.points.len())
                .sum::<usize>();
            assert_eq!(
                snapshot.devices.len(),
                33,
                "one router and the current 32 clients only"
            );
            assert!(count <= 144, "published instance budget exceeded: {count}");
            assert_eq!(count, 101);
            for device in &snapshot.devices {
                if let Some(id) = device.device.id.as_str().strip_prefix("network:client_") {
                    assert!(expected.iter().any(|expected| expected == id));
                }
            }
        })
        .await;
    }
    stage(&hub, vec![], false, false, |hub| {
        assert_eq!(
            hub.snapshot().devices.len(),
            1,
            "explicit empty selection retires client definitions"
        );
    })
    .await;
}
#[tokio::test]
async fn cached_keys_are_restored_before_a_partial_first_publication() {
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "moli-netgear-reconcile-{}-{suffix}.json",
        std::process::id()
    ));
    let cached = serde_json::json!({"network:client_aabbccdd0000":{"link":{"value":"wired","ts":1},"signal":{"value":75,"ts":1},"rate":{"value":100.0,"ts":1}},"network:router_synthetic-router-01":{"cpu":{"value":12,"ts":1},"memory":{"value":20,"ts":1}}});
    std::fs::write(&path, serde_json::to_vec(&cached).unwrap()).unwrap();
    let hub = Hub::new(HubOptions {
        state_path: Some(path.clone()),
        ..HubOptions::default()
    })
    .unwrap();
    assert!(hub.snapshot().devices.is_empty());
    stage(&hub, selected(0, 1), true, false, |hub| {
        assert_eq!(keys(hub, "client_aabbccdd0000").len(), 3);
        assert_eq!(
            hub.state(&PointId::from("network:client_aabbccdd0000/signal"))
                .unwrap()
                .value,
            Value::Null
        );
        assert_eq!(keys(hub, "router_synthetic-router-01").len(), 5);
        assert_eq!(
            hub.state(&PointId::from("network:router_synthetic-router-01/memory"))
                .unwrap()
                .value,
            Value::Null
        );
    })
    .await;
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn unobserved_capabilities_are_not_created_on_fresh_or_restarted_devices() {
    let hub = Hub::new(HubOptions::default()).unwrap();
    for _ in 0..2 {
        stage(&hub, selected(0, 1), true, false, |hub| {
            assert_eq!(keys(hub, "client_aabbccdd0000"), vec!["link"]);
            assert_eq!(keys(hub, "router_synthetic-router-01").len(), 4);
            assert!(
                hub.state(&PointId::from("network:client_aabbccdd0000/signal"))
                    .is_none()
            );
            assert!(
                hub.state(&PointId::from("network:router_synthetic-router-01/memory"))
                    .is_none()
            );
        })
        .await;
    }
}
struct Rejected {
    done: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}
impl Driver for Rejected {
    fn kind(&self) -> &'static str {
        "netgear"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let mut state = Published::new(ctx);
            let before = state.ctx.devices();
            assert_eq!(
                state.apply("synthetic-other", &[], vec![]),
                Err(crate::xml::Failure::Identity)
            );
            assert_eq!(
                state.ctx.devices(),
                before,
                "identity refusal must not retire any clients"
            );
            assert_eq!(
                state.reconcile(&selected(0, 33)),
                Err(crate::xml::Failure::Identity)
            );
            assert_eq!(
                state.ctx.devices(),
                before,
                "invalid selection must not alter definitions"
            );
            if let Some(done) = self.done.lock().unwrap().take() {
                let _ = done.send(());
            }
            state.ctx.cancelled().await;
            Ok(())
        })
    }
}
#[tokio::test]
async fn identity_and_selection_refusals_leave_published_definitions_intact() {
    let hub = Hub::new(HubOptions::default()).unwrap();
    stage(&hub, selected(0, 1), false, false, |_| {}).await;
    let (done, ready) = tokio::sync::oneshot::channel();
    let cancel = CancellationToken::new();
    let task = spawn_driver(
        &hub,
        InstanceId::from("network"),
        Arc::new(Rejected {
            done: Mutex::new(Some(done)),
        }),
        cancel.clone(),
    );
    tokio::time::timeout(Duration::from_secs(2), ready)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(hub.snapshot().devices.len(), 2);
    assert_eq!(keys(&hub, "client_aabbccdd0000").len(), 3);
    cancel.cancel();
    task.await.unwrap();
}
