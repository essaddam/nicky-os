use std::sync::Arc;
use std::time::Duration;

use moli_core::{InstanceId, Origin, PointId, Value};
use moli_helpers::{Config, Helpers};
use moli_runtime::{Hub, HubOptions, spawn_driver};
use tokio_util::sync::CancellationToken;

fn counter(extra: &str) -> Config {
    toml::from_str(&format!(
        "[[helper]]\nid = 'count'\nname = 'Counter'\nkind = 'counter'\n{extra}"
    ))
    .unwrap()
}

async fn start(hub: &Hub, instance: &str, config: Config) -> CancellationToken {
    let cancel = CancellationToken::new();
    spawn_driver(
        hub,
        InstanceId::from(instance),
        Arc::new(Helpers::new(config).unwrap()),
        cancel.clone(),
    );
    let point = PointId::from(format!("{instance}:count/value").as_str());
    for _ in 0..500 {
        if hub.state(&point).is_some() {
            return cancel;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    panic!("counter never became ready");
}

fn value(hub: &Hub, instance: &str) -> Value {
    hub.state(&PointId::from(format!("{instance}:count/value").as_str()))
        .unwrap()
        .value
}

async fn command(
    hub: &Hub,
    instance: &str,
    operation: &str,
    sequence: u64,
) -> Result<(), moli_runtime::CommandError> {
    hub.command_with_request_id(
        &PointId::from(format!("{instance}:count/{operation}").as_str()),
        Value::Null,
        Origin::Ui,
        None,
        Some(format!("0123456789abcdef0123456789abcdef:{sequence}")),
    )
    .await
}

#[test]
fn auto_h2_counter_configuration_rejects_fractional_and_rounded_inputs() {
    assert!(Helpers::new(counter("initial = 7\nstep = -2\nmin = -9\nmax = 10")).is_ok());
    for fields in [
        "max = 9007199254740992",
        "initial = 9007199254740992",
        "min = 0.0",
        "step = 0",
        "step = 1.5",
        "step = 1.0",
        "initial = 1.0",
        "min = 9007199254740993",
        "step = 9007199254740993",
        "max = -1\nmin = 2",
        "initial = 4\nmax = 3",
    ] {
        let parsed = toml::from_str::<Config>(&format!(
            "[[helper]]\nid='count'\nname='Counter'\nkind='counter'\n{fields}"
        ));
        assert!(
            parsed.map_or(true, |config| Helpers::new(config).is_err()),
            "accepted {fields}"
        );
    }
}

#[tokio::test]
async fn auto_h2_concurrent_increments_and_reset_are_serialized_without_replaying_old_values() {
    let hub = Hub::new(HubOptions::default()).unwrap();
    let cancel = start(&hub, "fixture", counter("initial = 7\nstep = 2")).await;
    assert_eq!(value(&hub, "fixture"), Value::Int(7));
    let mut tasks = Vec::new();
    for _ in 0..20 {
        let hub = hub.clone();
        tasks.push(tokio::spawn(async move {
            hub.command(
                &"fixture:count/increment".into(),
                Value::Null,
                Origin::Ui,
                None,
            )
            .await
            .unwrap();
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    assert_eq!(value(&hub, "fixture"), Value::Int(47));
    command(&hub, "fixture", "increment", 1).await.unwrap();
    command(&hub, "fixture", "reset", 2).await.unwrap();
    command(&hub, "fixture", "increment", 1).await.unwrap();
    assert_eq!(value(&hub, "fixture"), Value::Int(7));
    command(&hub, "fixture", "increment", 3).await.unwrap();
    command(&hub, "fixture", "reset", 2).await.unwrap();
    assert_eq!(value(&hub, "fixture"), Value::Int(9));
    assert!(
        hub.command(
            &"fixture:count/value".into(),
            Value::Int(99),
            Origin::Ui,
            None
        )
        .await
        .is_err()
    );
    cancel.cancel();
}

#[tokio::test]
async fn auto_h2_refused_increment_stays_refused_after_reset() {
    let hub = Hub::new(HubOptions::default()).unwrap();
    let cancel = start(&hub, "fixture", counter("initial = 0\nstep = 1\nmax = 1")).await;
    command(&hub, "fixture", "increment", 1).await.unwrap();
    assert!(command(&hub, "fixture", "increment", 2).await.is_err());
    command(&hub, "fixture", "reset", 3).await.unwrap();
    assert!(command(&hub, "fixture", "increment", 2).await.is_err());
    assert_eq!(value(&hub, "fixture"), Value::Int(0));
    assert!(command(&hub, "fixture", "reset", 1).await.is_err());
    cancel.cancel();
}

#[tokio::test]
async fn auto_h2_two_instances_have_independent_values_and_shared_request_identity_scope() {
    let hub = Hub::new(HubOptions::default()).unwrap();
    let a = start(&hub, "fixture_a", counter("initial = 3")).await;
    let b = start(&hub, "fixture_b", counter("initial = 9")).await;
    let (first, second) = tokio::join!(
        command(&hub, "fixture_a", "increment", 1),
        command(&hub, "fixture_b", "increment", 2),
    );
    first.unwrap();
    second.unwrap();
    assert_eq!(value(&hub, "fixture_a"), Value::Int(4));
    assert_eq!(value(&hub, "fixture_b"), Value::Int(10));
    assert!(command(&hub, "fixture_b", "increment", 1).await.is_err());
    assert_eq!(value(&hub, "fixture_b"), Value::Int(10));
    a.cancel();
    b.cancel();
}

#[tokio::test]
async fn auto_h2_guard_keeps_two_intents_and_retry_reuses_only_its_approval() {
    let hub = Hub::new(HubOptions {
        guard: moli_runtime::guard::GuardPolicy {
            protect_unassigned: true,
            ..Default::default()
        },
        ..Default::default()
    })
    .unwrap();
    let cancel = start(&hub, "fixture", counter("")).await;
    let point = PointId::from("fixture:count/increment");
    let send = |sequence| {
        let hub = hub.clone();
        let point = point.clone();
        async move {
            hub.command_with_request_id(
                &point,
                Value::Null,
                Origin::Mcp,
                Some("simulated-client".into()),
                Some(format!("0123456789abcdef0123456789abcdef:{sequence}")),
            )
            .await
        }
    };
    let first = send(1).await.unwrap_err();
    let second = send(2).await.unwrap_err();
    assert_eq!(send(1).await.unwrap_err(), first);
    let moli_runtime::CommandError::NeedsApproval { id: a, .. } = first else {
        panic!("not held");
    };
    let moli_runtime::CommandError::NeedsApproval { id: b, .. } = second else {
        panic!("not held");
    };
    assert_ne!(a, b);
    assert_eq!(hub.approvals().len(), 2);
    assert_eq!(value(&hub, "fixture"), Value::Int(0));
    hub.resolve_approval(b, &point, true, Origin::Ui, None)
        .await
        .unwrap();
    hub.resolve_approval(a, &point, true, Origin::Ui, None)
        .await
        .unwrap();
    assert_eq!(value(&hub, "fixture"), Value::Int(2));
    assert!(hub.journal(20).iter().any(|entry| entry.correlation.as_deref() == Some("0123456789abcdef0123456789abcdef:1")));
    cancel.cancel();
}

#[tokio::test]
async fn auto_h2_durable_instances_restore_values_and_receipts_instead_of_cached_state() {
    let root = std::env::temp_dir().join(format!(
        "moli-h2-helpers-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let state_path = root.join("state.json");
    let business = root.join("counters.json");
    let make_hub = || {
        Hub::new(HubOptions {
            counter_state_path: Some(business.clone()),
            state_path: Some(state_path.clone()),
            ..Default::default()
        })
        .unwrap()
    };
    let hub = make_hub();
    let a = start(&hub, "fixture_a", counter("initial = 7")).await;
    let b = start(&hub, "fixture_b", counter("initial = 20")).await;
    let (first, second) = tokio::join!(
        command(&hub, "fixture_a", "increment", 1),
        command(&hub, "fixture_b", "increment", 2)
    );
    first.unwrap();
    second.unwrap();
    assert_eq!(value(&hub, "fixture_a"), Value::Int(8));
    assert_eq!(value(&hub, "fixture_b"), Value::Int(21));
    a.cancel();
    b.cancel();
    std::fs::write(
        &state_path,
        br#"{"fixture_a:count":{"value":{"value":999,"ts":1}}}"#,
    )
    .unwrap();
    let fresh = make_hub();
    let mut events = fresh.subscribe();
    let a = start(&fresh, "fixture_a", counter("initial = 7")).await;
    let b = start(&fresh, "fixture_b", counter("initial = 20")).await;
    while let Ok(event) = events.try_recv() {
        assert!(!matches!(
            event,
            moli_core::Event::State {
                value: Value::Int(999),
                ..
            } | moli_core::Event::Occurrence { .. }
        ));
    }
    command(&fresh, "fixture_a", "increment", 1).await.unwrap();
    command(&fresh, "fixture_b", "increment", 2).await.unwrap();
    assert_eq!(value(&fresh, "fixture_a"), Value::Int(8));
    assert_eq!(value(&fresh, "fixture_b"), Value::Int(21));
    command(&fresh, "fixture_a", "increment", 3).await.unwrap();
    assert_eq!(value(&fresh, "fixture_a"), Value::Int(9));
    a.cancel();
    b.cancel();
    std::fs::remove_dir_all(root).unwrap();
}
