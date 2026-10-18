//! Real hub, guard and driver against the synthetic local HTTP server.
use super::*;
use crate::api::tests::Server;
use moli_core::{Event, InstanceId, Origin, Outcome, PointId, Value};
use moli_runtime::{CommandError, Hub, HubOptions, spawn_driver};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

fn point(key: &str) -> PointId {
    PointId::from(format!("lights:aabbccddeeff/{key}"))
}

async fn until(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("observable driver state within the synthetic deadline");
}

async fn start(
    server: &Server,
    guarded: bool,
) -> (Hub, CancellationToken, tokio::task::JoinHandle<()>) {
    let mut options = HubOptions::default();
    options.guard.protect_unassigned = guarded;
    let hub = Hub::new(options).unwrap();
    let cancel = CancellationToken::new();
    let config = Config {
        host: std::net::Ipv4Addr::LOCALHOST,
        port: server.port,
        mac: "aa:bb:cc:dd:ee:ff".into(),
        resume_mode: None,
    };
    let handle = spawn_driver(
        &hub,
        InstanceId::from("lights"),
        Arc::new(Twinkly::new(config).unwrap()),
        cancel.clone(),
    );
    until(|| hub.state(&point("on")).is_some()).await;
    (hub, cancel, handle)
}

#[tokio::test]
async fn twinkly_runtime_publishes_reread_never_requested_brightness() {
    let server = Server::start().await;
    let (hub, cancel, handle) = start(&server, false).await;
    let mut events = hub.subscribe();
    hub.command(&point("brightness"), Value::Int(84), Origin::Ui, None)
        .await
        .unwrap();
    until(|| server.count("GET", "/xled/v1/led/out/brightness") >= 3).await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(
        hub.state(&point("brightness")).unwrap().value,
        Value::Int(23)
    );
    while let Ok(event) = events.try_recv() {
        assert!(
            !matches!(event,Event::State {point:p,value:Value::Int(84),..} if p==point("brightness"))
        );
    }
    assert_eq!(
        server.writes(),
        vec![serde_json::json!({"mode":"enabled","type":"A","value":84})]
    );
    cancel.cancel();
    handle.await.unwrap();
}

#[tokio::test]
async fn twinkly_runtime_guard_holds_agent_without_physical_write() {
    let server = Server::start().await;
    let (hub, cancel, handle) = start(&server, true).await;
    assert!(matches!(
        hub.command(&point("on"), Value::Bool(false), Origin::Mcp, None)
            .await,
        Err(CommandError::NeedsApproval { .. })
    ));
    assert!(server.writes().is_empty());
    cancel.cancel();
    handle.await.unwrap();
}

#[tokio::test]
async fn twinkly_runtime_fifo_skips_caller_abandoned_while_queued() {
    let server = Server::start().await;
    let (hub, cancel, handle) = start(&server, false).await;
    server.hold_post("/xled/v1/led/mode");
    let h = hub.clone();
    let first = tokio::spawn(async move {
        h.command(&point("on"), Value::Bool(false), Origin::Ui, None)
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), server.received.notified())
        .await
        .unwrap();
    let h = hub.clone();
    let abandoned = tokio::spawn(async move {
        h.command(&point("brightness"), Value::Int(40), Origin::Ui, None)
            .await
    });
    tokio::time::sleep(Duration::from_millis(30)).await;
    let h = hub.clone();
    let last = tokio::spawn(async move {
        h.command(&point("brightness"), Value::Int(60), Origin::Ui, None)
            .await
    });
    abandoned.abort();
    let _ = abandoned.await;
    server.unhold();
    server.release.notify_one();
    first.await.unwrap().unwrap();
    last.await.unwrap().unwrap();
    assert_eq!(
        server.writes(),
        vec![
            serde_json::json!({"mode":"off"}),
            serde_json::json!({"mode":"enabled","type":"A","value":60})
        ]
    );
    cancel.cancel();
    handle.await.unwrap();
}

#[tokio::test]
async fn twinkly_runtime_emitted_success_survives_direct_or_approved_caller_leaving() {
    for approval in [false, true] {
        let server = Server::start().await;
        let (hub, cancel, handle) = start(&server, approval).await;
        server.hold_post("/xled/v1/led/mode");
        let call = if approval {
            let held = hub
                .command(&point("on"), Value::Bool(false), Origin::Mcp, None)
                .await;
            let Err(CommandError::NeedsApproval { id, .. }) = held else {
                panic!("approval expected")
            };
            let h = hub.clone();
            tokio::spawn(async move {
                let _ = h
                    .resolve_approval(id, &point("on"), true, Origin::Ui, None)
                    .await;
            })
        } else {
            let h = hub.clone();
            tokio::spawn(async move {
                let _ = h
                    .command(&point("on"), Value::Bool(false), Origin::Ui, None)
                    .await;
            })
        };
        tokio::time::timeout(Duration::from_secs(2), server.received.notified())
            .await
            .unwrap();
        call.abort();
        let _ = call.await;
        server.unhold();
        server.release.notify_one();
        until(|| {
            hub.journal(20).iter().any(|entry| {
                matches!(&entry.action,
            moli_core::Action::Command {..} if !approval)
                    && entry.outcome == Outcome::Ok
            }) || approval
                && hub.journal(20).iter().any(|entry| {
                    matches!(&entry.action, moli_core::Action::Approval { .. })
                        && entry.outcome == Outcome::Ok
                })
        })
        .await;
        assert_eq!(server.writes().len(), 1);
        cancel.cancel();
        handle.await.unwrap();
    }
}

#[tokio::test]
async fn twinkly_runtime_failed_reread_is_offline_without_false_freshness() {
    let server = Server::start().await;
    let (hub, cancel, handle) = start(&server, false).await;
    server.hold_post("/xled/v1/led/mode");
    let h = hub.clone();
    let call = tokio::spawn(async move {
        h.command(&point("on"), Value::Bool(false), Origin::Ui, None)
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), server.received.notified())
        .await
        .unwrap();
    let before = hub.state(&point("brightness")).unwrap();
    server.fail("/xled/v1/led/out/brightness", 200, b"not json".to_vec());
    server.unhold();
    server.release.notify_one();
    call.await.unwrap().unwrap();
    until(|| {
        hub.snapshot()
            .devices
            .iter()
            .any(|d| d.online == Some(false))
    })
    .await;
    assert_eq!(hub.state(&point("brightness")).unwrap().ts, before.ts);
    assert_eq!(hub.state(&point("on")).unwrap().value, Value::Bool(true));
    cancel.cancel();
    handle.await.unwrap();
}

#[tokio::test]
async fn twinkly_runtime_stop_is_offline_and_restart_does_not_replay() {
    let server = Server::start().await;
    let (hub, cancel, handle) = start(&server, false).await;
    hub.command(&point("brightness"), Value::Int(60), Origin::Ui, None)
        .await
        .unwrap();
    cancel.cancel();
    handle.await.unwrap();
    assert!(
        hub.snapshot()
            .devices
            .iter()
            .all(|d| d.online == Some(false))
    );
    let config = Config {
        host: std::net::Ipv4Addr::LOCALHOST,
        port: server.port,
        mac: "aa:bb:cc:dd:ee:ff".into(),
        resume_mode: None,
    };
    let stop = CancellationToken::new();
    let task = spawn_driver(
        &hub,
        InstanceId::from("lights"),
        Arc::new(Twinkly::new(config).unwrap()),
        stop.clone(),
    );
    until(|| server.count("POST", "/xled/v1/verify") == 2).await;
    assert_eq!(server.writes().len(), 1);
    stop.cancel();
    task.await.unwrap();
}

#[test]
fn twinkly_runtime_configuration_refuses_unvalidated_destinations_and_identity() {
    for (host, mac, port) in [
        ("8.8.8.8", "aa:bb:cc:dd:ee:ff", 80),
        ("0.0.0.0", "aa:bb:cc:dd:ee:ff", 80),
        ("192.168.1.2", "not a MAC", 80),
        ("192.168.1.2", "aa:bb:cc:dd:ee:ff", 0),
    ] {
        assert!(
            Twinkly::new(Config {
                host: host.parse().unwrap(),
                port,
                mac: mac.into(),
                resume_mode: None
            })
            .is_err()
        );
    }
}

#[tokio::test]
async fn twinkly_runtime_initial_off_has_no_arbitrary_resume_scene() {
    let server = Server::start().await;
    server.set_mode("off");
    let (hub, cancel, handle) = start(&server, false).await;
    assert!(
        hub.command(&point("on"), Value::Bool(true), Origin::Ui, None)
            .await
            .is_err()
    );
    assert!(server.writes().is_empty());
    cancel.cancel();
    handle.await.unwrap();
}

#[tokio::test]
async fn twinkly_runtime_queue_is_bounded_and_abandoned_slots_are_released() {
    let server = Server::start().await;
    let (hub, cancel, handle) = start(&server, false).await;
    server.hold_post("/xled/v1/led/mode");
    let h = hub.clone();
    let first = tokio::spawn(async move {
        h.command(&point("on"), Value::Bool(false), Origin::Ui, None)
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), server.received.notified())
        .await
        .unwrap();
    let mut queued = Vec::new();
    for n in 1..=8 {
        let h = hub.clone();
        queued.push(tokio::spawn(async move {
            h.command(&point("brightness"), Value::Int(n), Origin::Ui, None)
                .await
        }));
    }
    until(|| queued.iter().any(tokio::task::JoinHandle::is_finished)).await;
    let rejected = queued
        .iter()
        .position(tokio::task::JoinHandle::is_finished)
        .unwrap();
    assert!(queued.remove(rejected).await.unwrap().is_err());
    for task in queued {
        task.abort();
        let _ = task.await;
    }
    server.unhold();
    server.release.notify_one();
    first.await.unwrap().unwrap();
    until(|| server.count("GET", "/xled/v1/gestalt") >= 3).await;
    hub.command(&point("brightness"), Value::Int(70), Origin::Ui, None)
        .await
        .unwrap();
    assert_eq!(server.writes().len(), 2);
    cancel.cancel();
    handle.await.unwrap();
}

#[tokio::test]
async fn twinkly_runtime_abandon_between_color_steps_prevents_mode_change() {
    let server = Server::start().await;
    let (hub, cancel, handle) = start(&server, false).await;
    server.hold_post("/xled/v1/led/color");
    let h = hub.clone();
    let call = tokio::spawn(async move {
        h.command(&point("color"), Value::from("#102030"), Origin::Ui, None)
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), server.received.notified())
        .await
        .unwrap();
    call.abort();
    let _ = call.await;
    server.unhold();
    server.release.notify_one();
    until(|| {
        hub.journal(20)
            .iter()
            .any(|entry| matches!(&entry.action, moli_core::Action::Command { .. }))
    })
    .await;
    assert_eq!(
        server.writes(),
        vec![serde_json::json!({"red":16,"green":32,"blue":48})]
    );
    assert!(
        hub.journal(20)
            .iter()
            .any(|entry| matches!(&entry.outcome,Outcome::Err(message)
        if message!=&moli_i18n::tr("pilotes.twinkly.non_envoye")))
    );
    cancel.cancel();
    handle.await.unwrap();
}

#[tokio::test]
async fn twinkly_runtime_expired_queued_command_never_leaves_after_slow_predecessor() {
    let server = Server::start().await;
    let (hub, cancel, handle) = start(&server, false).await;
    server.hold("/xled/v1/led/mode");
    let h = hub.clone();
    let first = tokio::spawn(async move {
        h.command(&point("on"), Value::Bool(false), Origin::Ui, None)
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), server.received.notified())
        .await
        .unwrap();
    let h = hub.clone();
    let expired = tokio::spawn(async move {
        h.command(&point("brightness"), Value::Int(40), Origin::Ui, None)
            .await
    });
    tokio::time::sleep(Duration::from_millis(3500)).await;
    server.hold_post("/xled/v1/led/mode");
    server.release.notify_one();
    tokio::time::timeout(Duration::from_secs(2), server.received.notified())
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(3), expired)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    server.unhold();
    server.release.notify_one();
    let _ = first.await;
    until(|| server.count("GET", "/xled/v1/gestalt") >= 3).await;
    assert_eq!(server.writes(), vec![serde_json::json!({"mode":"off"})]);
    cancel.cancel();
    handle.await.unwrap();
}
