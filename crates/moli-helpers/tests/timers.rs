use std::sync::Arc;
use std::time::Duration;

use moli_core::{Origin, Value};
use moli_helpers::{Config, Helpers};
use moli_runtime::{Hub, HubOptions, spawn_driver};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn auto_h1_guard_retains_distinct_starts_and_rejects_a_changed_retry() {
    let hub = Hub::new(HubOptions {
        guard: moli_runtime::guard::GuardPolicy {
            protect_unassigned: true,
            ..Default::default()
        },
        ..Default::default()
    })
    .unwrap();
    let config = toml::from_str("[[helper]]\nid='clock'\nname='Timer'\nkind='timer'").unwrap();
    let cancel = CancellationToken::new();
    let driver = spawn_driver(
        &hub,
        "fixture".into(),
        Arc::new(Helpers::new(config).unwrap()),
        cancel.clone(),
    );
    for _ in 0..200 {
        if hub.state(&"fixture:clock/state".into()).is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let send = |seconds, sequence| {
        let h = hub.clone();
        async move {
            h.command_with_request_id(
                &"fixture:clock/start".into(),
                Value::Int(seconds),
                Origin::Mcp,
                None,
                Some(format!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:{sequence}")),
            )
            .await
        }
    };
    let first = send(5, 1).await.unwrap_err();
    assert_eq!(send(5, 1).await.unwrap_err(), first);
    assert_eq!(
        send(6, 1).await.unwrap_err(),
        moli_runtime::CommandError::Timer(moli_runtime::TimerError::Conflict)
    );
    let second = send(6, 2).await.unwrap_err();
    let moli_runtime::CommandError::NeedsApproval { id: a, .. } = first else {
        panic!("not held")
    };
    let moli_runtime::CommandError::NeedsApproval { id: b, .. } = second else {
        panic!("not held")
    };
    assert_ne!(a, b);
    assert_eq!(hub.approvals().len(), 2);
    hub.resolve_approval(b, &"fixture:clock/start".into(), true, Origin::Ui, None)
        .await
        .unwrap();
    hub.resolve_approval(a, &"fixture:clock/start".into(), true, Origin::Ui, None)
        .await
        .unwrap();
    assert_eq!(
        hub.state(&"fixture:clock/generation".into()).unwrap().value,
        Value::Int(2)
    );
    cancel.cancel();
    driver.await.unwrap();
}

#[test]
fn auto_h1_configuration_rejects_non_integer_or_unbounded_default_duration() {
    for duration in ["0", "-1", "1.0", "604801"] {
        let config = toml::from_str::<Config>(&format!(
            "[[helper]]\nid='clock'\nname='Timer'\nkind='timer'\nduration_s={duration}"
        ));
        assert!(config.is_err() || Helpers::new(config.unwrap()).is_err());
    }
}

#[tokio::test]
async fn auto_h1_start_cancel_and_retry_do_not_resurrect_a_generation() {
    let config = toml::from_str::<Config>(
        "[[helper]]\nid = 'clock'\nname = 'Timer'\nkind = 'timer'\nduration_s = 5\n",
    );
    assert!(
        config.is_ok(),
        "timer configuration must be supported: {config:?}"
    );
    let hub = Hub::new(HubOptions::default()).unwrap();
    let cancel = CancellationToken::new();
    spawn_driver(
        &hub,
        "fixture".into(),
        Arc::new(Helpers::new(config.unwrap()).unwrap()),
        cancel.clone(),
    );
    let state = "fixture:clock/state".into();
    for _ in 0..100 {
        if hub.state(&state).is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(hub.state(&state).unwrap().value, Value::Text("idle".into()));
    let start = "fixture:clock/start".into();
    let identity = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:1".to_owned();
    hub.command_with_request_id(
        &start,
        Value::Int(5),
        Origin::Ui,
        None,
        Some(identity.clone()),
    )
    .await
    .unwrap();
    assert_eq!(
        hub.state(&state).unwrap().value,
        Value::Text("running".into())
    );
    let (first, second) = tokio::join!(
        hub.command(&start, Value::Int(6), Origin::Ui, None),
        hub.command(&start, Value::Int(7), Origin::Ui, None),
    );
    first.unwrap();
    second.unwrap();
    assert_eq!(
        hub.state(&"fixture:clock/generation".into()).unwrap().value,
        Value::Int(3)
    );
    assert!(matches!(
        hub.state(&"fixture:clock/remaining".into()).unwrap().value,
        Value::Int(6 | 7)
    ));
    hub.command(
        &"fixture:clock/cancel".into(),
        Value::Null,
        Origin::Ui,
        None,
    )
    .await
    .unwrap();
    hub.command_with_request_id(&start, Value::Int(5), Origin::Ui, None, Some(identity))
        .await
        .unwrap();
    assert_eq!(
        hub.state(&state).unwrap().value,
        Value::Text("cancelled".into())
    );
    assert_eq!(
        hub.state(&"fixture:clock/remaining".into()).unwrap().value,
        Value::Int(0)
    );
    cancel.cancel();
}
