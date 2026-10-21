use moli_helpers::{Config, Helpers};
#[test]
fn auto_h3_temporal_configuration_is_explicit_and_legacy_configuration_survives() {
    for (kind, initial, extra) in [
        ("date", "2024-02-29", ""),
        ("time", "23:59:59", ""),
        (
            "datetime",
            "2026-10-25T02:30:00+02:00",
            "timezone='Europe/Paris'",
        ),
    ] {
        let parsed = toml::from_str::<Config>(&format!(
            "[[helper]]\nid='sample'\nname='Synthetic value'\nkind='{kind}'\ninitial='{initial}'\n{extra}"
        ));
        assert!(parsed.is_ok(), "{parsed:?}");
        assert!(Helpers::new(parsed.unwrap()).is_ok());
    }
    for kind in ["switch", "text", "number"] {
        assert!(
            Helpers::new(
                toml::from_str(&format!(
                    "[[helper]]\nid='sample'\nname='Synthetic value'\nkind='{kind}'"
                ))
                .unwrap()
            )
            .is_ok()
        );
    }
}

use moli_core::{Origin, Value};
use moli_runtime::{Hub, HubOptions, spawn_driver};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
fn datetime_config() -> Config {
    toml::from_str("[[helper]]\nid='sample'\nname='Synthetic value'\nkind='datetime'\ntimezone='Europe/Paris'\ninitial='2000-01-01T00:00:00Z'").unwrap()
}
async fn wait(hub: &Hub, point: &moli_core::PointId) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while hub.state(point).is_none() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn auto_h3_two_instances_share_durable_state_and_restore_without_the_cache() {
    let root = std::env::temp_dir().join(format!(
        "moli-h3-helpers-{}-{}",
        std::process::id(),
        moli_core::now_ms()
    ));
    let options = HubOptions {
        temporal_state_path: Some(root.join("dates.json")),
        state_path: Some(root.join("state.json")),
        ..Default::default()
    };
    let hub = Hub::new(options.clone()).unwrap();
    let cancel = CancellationToken::new();
    let mut tasks = Vec::new();
    for instance in ["first", "second"] {
        tasks.push(spawn_driver(
            &hub,
            instance.into(),
            Arc::new(Helpers::new(datetime_config()).unwrap()),
            cancel.clone(),
        ));
        wait(&hub, &format!("{instance}:sample/value").into()).await;
    }
    let first = "first:sample/value".into();
    let second = "second:sample/value".into();
    let (a, b) = tokio::join!(
        hub.command(
            &first,
            Value::Text("2026-10-25T02:30:00+02:00".into()),
            Origin::Ui,
            None
        ),
        hub.command(
            &second,
            Value::Text("2026-10-25T02:30:00+01:00".into()),
            Origin::Ui,
            None
        )
    );
    a.unwrap();
    b.unwrap();
    assert_eq!(
        hub.state(&first).unwrap().value,
        Value::Text("2026-10-25T00:30:00Z".into())
    );
    assert_eq!(
        hub.state(&second).unwrap().value,
        Value::Text("2026-10-25T01:30:00Z".into())
    );
    assert!(!hub.persist_state().await.unwrap());
    cancel.cancel();
    for task in tasks {
        task.await.unwrap();
    }
    let fresh = Hub::new(options).unwrap();
    let cancel = CancellationToken::new();
    let driver = spawn_driver(
        &fresh,
        "first".into(),
        Arc::new(Helpers::new(datetime_config()).unwrap()),
        cancel.clone(),
    );
    wait(&fresh, &first).await;
    assert_eq!(
        fresh.state(&first).unwrap().value,
        Value::Text("2026-10-25T00:30:00Z".into())
    );
    cancel.cancel();
    driver.await.unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
#[tokio::test]
async fn auto_h3_guard_holds_the_canonical_instant_and_invalid_times_do_not_change_it() {
    let hub = Hub::new(HubOptions {
        guard: moli_runtime::guard::GuardPolicy {
            protect_unassigned: true,
            ..Default::default()
        },
        ..Default::default()
    })
    .unwrap();
    let cancel = CancellationToken::new();
    let task = spawn_driver(
        &hub,
        "fixture".into(),
        Arc::new(Helpers::new(datetime_config()).unwrap()),
        cancel.clone(),
    );
    let point = "fixture:sample/value".into();
    wait(&hub, &point).await;
    let error = hub
        .command(
            &point,
            Value::Text("2026-10-25T02:30:00+01:00".into()),
            Origin::Mcp,
            None,
        )
        .await
        .unwrap_err();
    let moli_runtime::CommandError::NeedsApproval { id, .. } = error else {
        panic!("not held");
    };
    assert_eq!(
        hub.approvals()[0].value,
        Value::Text("2026-10-25T01:30:00Z".into())
    );
    hub.resolve_approval(id, &point, true, Origin::Ui, None)
        .await
        .unwrap();
    assert_eq!(
        hub.command(
            &point,
            Value::Text("2026-03-29T02:30:00".into()),
            Origin::Ui,
            None
        )
        .await
        .unwrap_err(),
        moli_runtime::TemporalError::Gap.into()
    );
    assert_eq!(
        hub.state(&point).unwrap().value,
        Value::Text("2026-10-25T01:30:00Z".into())
    );
    cancel.cancel();
    task.await.unwrap();
}
