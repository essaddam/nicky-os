//! Lossless deferred bus consumption must not peek at future source states.
use super::{on_event, remember_all};
use crate::{Actor, Author, Automations, Config};
use moli_core::{DependencyKind, DependencyRef, Event, Origin, PointId, Value, WeeklySchedule};
use moli_runtime::{Hub, HubOptions, spawn_driver};
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Metadata {
    None,
    Publish,
    Initialize,
}

fn assert_queue_order(events: &[Event], point: &PointId, has_metadata: bool) {
    let source = events
        .iter()
        .position(
            |event| matches!(event,Event::State {point:p,value:Value::Bool(true),..} if p==point),
        )
        .unwrap();
    if has_metadata {
        let changed = events
            .iter()
            .position(|event| matches!(event, Event::DependencyChanged { .. }))
            .unwrap();
        assert!(
            changed < source,
            "metadata precedes the still-unconsumed source event"
        );
    }
}

async fn deferred(metadata: Metadata, first_state: bool, duplicate: bool) -> u64 {
    let hub = Hub::new(HubOptions::default()).unwrap();
    let cancel = CancellationToken::new();
    let config=serde_json::from_value(serde_json::json!({"helper":[
        {"id":"source","name":"Synthetic source","kind":"switch"},
        {"id":"weekly","name":"Synthetic schedule","kind":"schedule","weekly":{"timezone":"UTC","slots":[]}}
    ]})).unwrap();
    let driver = spawn_driver(
        &hub,
        "fixture".into(),
        Arc::new(moli_helpers::Helpers::new(config).unwrap()),
        cancel.clone(),
    );
    let point = PointId::from("fixture:source/on");
    tokio::time::timeout(Duration::from_secs(2), async {
        while hub.state(&PointId::from("fixture:weekly/active")).is_none() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let (autos, engine) = Automations::start(&hub, Config::default(), None, cancel.clone())
        .await
        .unwrap();
    engine.abort();
    let _ = engine.await;
    let graph=serde_json::from_value(serde_json::json!({"name":"Synthetic independent graph","graph":{"nodes":[{"id":"t","type":"when_state","point":"fixture:source/on","to":true}],"edges":[]}})).unwrap();
    let human = Actor {
        human: true,
        author: Author::Human,
    };
    let graph = autos.save(graph, human).await.unwrap();
    autos
        .approve(&graph.id, &graph.fingerprint(), human)
        .await
        .unwrap();
    remember_all(&autos);
    if first_state {
        crate::lock(&autos.0.last).remove(point.as_str());
    }
    let before = autos.0.next_run.load(Ordering::Relaxed);
    let mut bus = hub.subscribe();
    let reference = DependencyRef {
        kind: DependencyKind::Schedule,
        id: "fixture:weekly".into(),
    };
    match metadata {
        Metadata::None => {}
        Metadata::Publish => {
            hub.publish_dependency(
                hub.preview_dependency(&reference).await.unwrap(),
                true,
                Origin::Ui,
                None,
            )
            .await
            .unwrap();
        }
        Metadata::Initialize => {
            hub.initialize_schedule(
                &reference.id,
                WeeklySchedule {
                    timezone: "UTC".into(),
                    slots: vec![],
                },
            )
            .await
            .unwrap();
        }
    }
    hub.command(&point, Value::Bool(true), Origin::Ui, None)
        .await
        .unwrap();
    assert_eq!(hub.state(&point).unwrap().value, Value::Bool(true));
    let mut pending = Vec::new();
    while let Ok(event) = bus.try_recv() {
        pending.push(event);
    }
    assert_queue_order(&pending, &point, metadata != Metadata::None);
    for event in pending {
        let repeated = duplicate.then(|| event.clone());
        on_event(&autos, event).await;
        if let Some(event) = repeated {
            on_event(&autos, event).await;
        }
    }
    // Admission increments synchronously before the spawned run can execute.
    // No sleep or timing-dependent absence assertion is needed on this runtime.
    let admitted = autos.0.next_run.load(Ordering::Relaxed) - before;
    cancel.cancel();
    driver.await.unwrap();
    admitted
}

#[tokio::test(flavor = "current_thread")]
async fn auto_h5_metadata_never_consumes_an_independent_pending_state() {
    assert_eq!(deferred(Metadata::None, false, false).await, 1, "control");
    assert_eq!(
        deferred(Metadata::Publish, false, false).await,
        1,
        "publication"
    );
    assert_eq!(
        deferred(Metadata::Initialize, false, false).await,
        1,
        "initialization"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn auto_h5_metadata_preserves_first_state_and_duplicate_suppression() {
    assert_eq!(deferred(Metadata::None, true, false).await, 0);
    assert_eq!(deferred(Metadata::Publish, true, false).await, 0);
    assert_eq!(deferred(Metadata::Initialize, true, false).await, 0);
    assert_eq!(deferred(Metadata::Publish, false, true).await, 1);
}
