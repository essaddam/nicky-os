use super::*;
use crate::{DependencyDefinition, DependencyError};
use moli_core::{
    Access, AggregateOp, AggregateSpec, DependencyKind, DependencyRef, Kind, PointSpec, Semantic,
    Unit,
};

fn source(hub: &Hub, name: &str, value: f64) {
    let id = DeviceId::from(format!("fixture:{name}"));
    hub.upsert_device(device(
        id.clone(),
        Kind::Numeric {
            min: None,
            max: None,
            step: None,
        },
        Some(Unit::Watt),
    ));
    hub.set_availability(&id, true);
    hub.set_state(&id, "value", Value::Float(value));
}
fn device(id: DeviceId, kind: Kind, unit: Option<Unit>) -> Device {
    Device {
        id,
        instance: "fixture".into(),
        native_name: "Synthetic".into(),
        manufacturer: None,
        model: None,
        description: None,
        native_room: None,
        members: vec![],
        points: vec![PointSpec {
            key: "value".into(),
            label: "Value".into(),
            kind,
            access: Access {
                read: true,
                write: false,
            },
            unit,
            semantic: Semantic::Other,
        }],
    }
}
fn spec(sources: &[&str]) -> AggregateSpec {
    AggregateSpec {
        operation: AggregateOp::Mean,
        sources: sources.iter().map(|p| (*p).into()).collect(),
        unit: Some(Unit::Watt),
        max_age_s: 1,
    }
}
async fn define(hub: &Hub, name: &str, spec: AggregateSpec) -> DependencyRef {
    let id = DeviceId::from(format!("fixture:{name}"));
    hub.upsert_device(device(id.clone(), Kind::Computed, None));
    hub.set_availability(&id, true);
    hub.initialize_computed(&id, spec).await.unwrap();
    DependencyRef {
        kind: DependencyKind::Computed,
        id,
    }
}
async fn approve(
    hub: &Hub,
    reference: &DependencyRef,
) -> Result<crate::DependencyView, DependencyError> {
    hub.publish_dependency(
        hub.preview_dependency(reference).await?,
        true,
        Origin::Ui,
        None,
    )
    .await
}
fn value(hub: &Hub, reference: &DependencyRef) -> Value {
    hub.state(&reference.point()).unwrap().value
}

#[tokio::test]
async fn auto_k1_draft_is_inert_partial_inputs_and_missing_sources_are_not_zero() {
    let hub = Hub::new(HubOptions::default()).unwrap();
    source(&hub, "x", 10.0);
    source(&hub, "y", 30.0);
    let reference = define(&hub, "mean", spec(&["fixture:x/value", "fixture:y/value"])).await;
    assert_eq!(value(&hub, &reference), Value::Null);
    approve(&hub, &reference).await.unwrap();
    assert_eq!(value(&hub, &reference), Value::Float(20.0));
    hub.set_availability(&"fixture:y".into(), false);
    hub.run_computations().unwrap();
    assert_eq!(value(&hub, &reference), Value::Float(10.0));
    let mut draft = spec(&["fixture:x/value", "fixture:y/value"]);
    draft.operation = AggregateOp::Max;
    hub.stage_dependency(
        reference.clone(),
        1,
        DependencyDefinition::Aggregate(draft),
        Origin::Mcp,
        None,
    )
    .await
    .unwrap();
    assert_eq!(value(&hub, &reference), Value::Float(10.0));
    hub.remove_device(&"fixture:x".into());
    hub.run_computations().unwrap();
    assert_eq!(value(&hub, &reference), Value::Null);
    source(&hub, "x", 10.0);
    hub.run_computations().unwrap();
    assert_eq!(value(&hub, &reference), Value::Float(10.0));
    assert_eq!(
        hub.command(&reference.point(), Value::Float(1.0), Origin::Ui, None)
            .await,
        Err(CommandError::ReadOnly)
    );
}

#[tokio::test]
async fn auto_k1_cycles_units_and_stale_source_version_cannot_publish() {
    let hub = Hub::new(HubOptions::default()).unwrap();
    source(&hub, "x", 10.0);
    let a = define(&hub, "a", spec(&["fixture:x/value"])).await;
    approve(&hub, &a).await.unwrap();
    let b = define(&hub, "b", spec(&["fixture:a/value"])).await;
    approve(&hub, &b).await.unwrap();
    let old = hub.preview_dependency(&b).await.unwrap();
    let mut next = spec(&["fixture:x/value"]);
    next.operation = AggregateOp::Max;
    hub.stage_dependency(
        a.clone(),
        1,
        DependencyDefinition::Aggregate(next),
        Origin::Mcp,
        None,
    )
    .await
    .unwrap();
    let shown = hub.preview_dependency(&a).await.unwrap();
    assert!(
        shown
            .related
            .iter()
            .any(|r| r.reference == b && r.role == crate::BindingRole::Affected)
    );
    hub.publish_dependency(shown, true, Origin::Ui, None)
        .await
        .unwrap();
    assert_eq!(
        hub.publish_dependency(old, true, Origin::Ui, None).await,
        Err(DependencyError::Conflict)
    );
    hub.stage_dependency(
        a.clone(),
        2,
        DependencyDefinition::Aggregate(spec(&["fixture:b/value"])),
        Origin::Mcp,
        None,
    )
    .await
    .unwrap();
    assert_eq!(approve(&hub, &a).await, Err(DependencyError::Cycle));
    assert_eq!(
        hub.dependency(&a)
            .await
            .unwrap()
            .versions
            .current()
            .unwrap()
            .version,
        2
    );
    let mut wrong = spec(&["fixture:x/value"]);
    wrong.unit = Some(Unit::Celsius);
    let wrong = define(&hub, "wrong", wrong).await;
    assert_eq!(approve(&hub, &wrong).await, Err(DependencyError::Units));
}

#[tokio::test]
async fn auto_k1_freshness_expires_without_event_and_equal_value_can_return() {
    let hub = Hub::new(HubOptions::default()).unwrap();
    source(&hub, "x", 10.0);
    let reference = define(&hub, "a", spec(&["fixture:x/value"])).await;
    approve(&hub, &reference).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while value(&hub, &reference) != Value::Null {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let mut events = hub.subscribe();
    hub.set_state(&"fixture:x".into(), "value", Value::Float(10.0));
    tokio::time::timeout(Duration::from_secs(5), async {
        while value(&hub, &reference) != Value::Float(10.0) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let mut raw_changed = false;
    while let Ok(event) = events.try_recv() {
        if matches!(event,Event::State {point,..} if point.as_str()=="fixture:x/value") {
            raw_changed = true;
        }
    }
    assert!(
        !raw_changed,
        "refreshing identical raw telemetry must not fabricate a state change"
    );
}

#[tokio::test]
async fn auto_k1_restart_uses_current_and_keeps_restored_source_timestamp() {
    let root = std::env::temp_dir().join(format!(
        "moli-k1-restore-{}",
        crate::occurrences::random_id().unwrap()
    ));
    let path = root.join("dependencies.json");
    let first = Hub::new(HubOptions {
        dependency_state_path: Some(path.clone()),
        ..Default::default()
    })
    .unwrap();
    source(&first, "x", 10.0);
    let reference = define(&first, "a", spec(&["fixture:x/value"])).await;
    approve(&first, &reference).await.unwrap();
    let mut draft = spec(&["fixture:x/value"]);
    draft.operation = AggregateOp::Max;
    first
        .stage_dependency(
            reference.clone(),
            1,
            DependencyDefinition::Aggregate(draft),
            Origin::Mcp,
            None,
        )
        .await
        .unwrap();
    drop(first);
    let cache = root.join("state.json");
    std::fs::write(&cache, br#"{"fixture:x":{"value":{"value":10.0,"ts":1}}}"#).unwrap();
    let restored = Hub::new(HubOptions {
        dependency_state_path: Some(path),
        state_path: Some(cache.clone()),
        ..Default::default()
    })
    .unwrap();
    restored.upsert_device(device(
        "fixture:x".into(),
        Kind::Numeric {
            min: None,
            max: None,
            step: None,
        },
        Some(Unit::Watt),
    ));
    restored.set_availability(&"fixture:x".into(), true);
    let calc = define(&restored, "a", spec(&[])).await;
    assert_eq!(
        restored
            .state(&PointId::from("fixture:x/value"))
            .unwrap()
            .ts,
        1
    );
    assert_eq!(value(&restored, &calc), Value::Null);
    let view = restored.dependency(&calc).await.unwrap();
    assert_eq!(view.versions.current().unwrap().version, 1);
    assert_eq!(view.versions.draft().version, 2);
    source(&restored, "x", 10.0);
    restored.run_computations().unwrap();
    restored.persist_state().await.unwrap();
    let persisted: serde_json::Value =
        serde_json::from_slice(&std::fs::read(cache).unwrap()).unwrap();
    assert!(
        persisted
            .get("fixture:a")
            .is_none_or(|fields| fields.get("value").is_none())
    );
    drop(restored);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn auto_k1_publication_time_does_not_backdate_a_definition_change() {
    let hub = Hub::new(HubOptions::default()).unwrap();
    source(&hub, "x", 10.0);
    let observed = moli_core::now_ms().saturating_sub(1000);
    hub.write()
        .states
        .get_mut(&DeviceId::from("fixture:x"))
        .unwrap()
        .get_mut("value")
        .unwrap()
        .ts = observed;
    let mut definition = spec(&["fixture:x/value"]);
    definition.max_age_s = 60;
    let reference = define(&hub, "a", definition).await;
    let mut events = hub.subscribe();
    let published_after = moli_core::now_ms();
    approve(&hub, &reference).await.unwrap();
    let mut found = None;
    while let Ok(event) = events.try_recv() {
        if let Event::State { point, ts, .. } = event
            && point == reference.point()
        {
            found = Some(ts);
        }
    }
    assert!(
        found.unwrap() >= published_after,
        "the event represents the current publication, not a retrospective observation"
    );
    assert_eq!(
        hub.state(&reference.point()).unwrap().ts,
        observed,
        "source age stays unchanged in the live model"
    );
}

#[tokio::test]
async fn auto_k1_unit_metadata_does_not_reinterpret_an_old_observation() {
    let hub = Hub::new(HubOptions::default()).unwrap();
    let id = DeviceId::from("fixture:x");
    let numeric = Kind::Numeric {
        min: None,
        max: None,
        step: None,
    };
    hub.upsert_device(device(id.clone(), numeric.clone(), Some(Unit::WattHour)));
    hub.set_availability(&id, true);
    hub.set_state(&id, "value", Value::Float(1000.0));
    let mut definition = spec(&["fixture:x/value"]);
    definition.unit = Some(Unit::WattHour);
    definition.max_age_s = 604_800;
    let reference = define(&hub, "a", definition).await;
    approve(&hub, &reference).await.unwrap();
    assert_eq!(value(&hub, &reference), Value::Float(1000.0));
    hub.upsert_device(device(id.clone(), numeric, Some(Unit::KiloWattHour)));
    hub.run_computations().unwrap();
    assert_eq!(
        value(&hub, &reference),
        Value::Null,
        "a unit change needs a new observation in that unit"
    );
    hub.set_state(&id, "value", Value::Float(1.0));
    hub.run_computations().unwrap();
    assert_eq!(value(&hub, &reference), Value::Float(1000.0));
}

#[tokio::test]
async fn auto_k1_draft_writer_cannot_block_current_projection_or_expiration() {
    use crate::state_file::WritePhase;
    let path = std::env::temp_dir()
        .join(format!(
            "moli-k1-writer-{}",
            crate::occurrences::random_id().unwrap()
        ))
        .join("dependencies.json");
    let hub = Hub::new(HubOptions {
        dependency_state_path: Some(path.clone()),
        ..Default::default()
    })
    .unwrap();
    source(&hub, "x", 10.0);
    let reference = define(&hub, "a", spec(&["fixture:x/value"])).await;
    approve(&hub, &reference).await.unwrap();
    let entered = Arc::new(tokio::sync::Notify::new());
    let resume = Arc::new(tokio::sync::Notify::new());
    hub.0.dependencies.lock().await.file.pause_at(
        WritePhase::BeforeRename,
        entered.clone(),
        resume.clone(),
    );
    let writer = hub.clone();
    let target = reference.clone();
    let task = tokio::spawn(async move {
        writer
            .stage_dependency(
                target,
                1,
                DependencyDefinition::Aggregate(spec(&[])),
                Origin::Mcp,
                None,
            )
            .await
    });
    entered.notified().await;
    hub.set_state(&"fixture:x".into(), "value", Value::Float(20.0));
    let updated = tokio::time::timeout(Duration::from_secs(5), async {
        while value(&hub, &reference) != Value::Float(20.0) {
            tokio::task::yield_now().await;
        }
    })
    .await;
    let expired = if updated.is_ok() {
        tokio::time::timeout(Duration::from_secs(5), async {
            while value(&hub, &reference) != Value::Null {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .is_ok()
    } else {
        false
    };
    resume.notify_one();
    task.await.unwrap().unwrap();
    assert!(
        updated.is_ok(),
        "inert draft I/O must not block current calculations"
    );
    assert!(
        expired,
        "freshness must expire while the draft writer is suspended"
    );
    assert_eq!(
        hub.dependency(&reference)
            .await
            .unwrap()
            .versions
            .current()
            .unwrap()
            .version,
        1
    );
    drop(hub);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

async fn unindexed_unit_change(already_active: bool, refresh: bool) -> Value {
    let hub = Hub::new(HubOptions::default()).unwrap();
    if already_active {
        source(&hub, "unrelated", 10.0);
        let unrelated = define(&hub, "already_active", spec(&["fixture:unrelated/value"])).await;
        approve(&hub, &unrelated).await.unwrap();
    }
    let id = DeviceId::from("fixture:late_source");
    let numeric = Kind::Numeric {
        min: None,
        max: None,
        step: None,
    };
    hub.upsert_device(device(id.clone(), numeric.clone(), Some(Unit::WattHour)));
    hub.set_state(&id, "value", Value::Float(1000.0));
    // A compatible conversion does not prove the old observation's new unit.
    hub.upsert_device(device(id.clone(), numeric, Some(Unit::KiloWattHour)));
    if refresh {
        hub.set_state(&id, "value", Value::Float(1.0));
    }
    let mut definition = spec(&["fixture:late_source/value"]);
    definition.unit = Some(Unit::WattHour);
    definition.max_age_s = 60;
    let reference = define(&hub, "late", definition).await;
    approve(&hub, &reference).await.unwrap();
    value(&hub, &reference)
}
#[tokio::test]
async fn auto_k1_r1_unit_change_before_first_calculation_requires_a_new_measurement() {
    assert_eq!(
        unindexed_unit_change(false, true).await,
        Value::Float(1000.0),
        "new measurement control"
    );
    assert_eq!(
        unindexed_unit_change(false, false).await,
        Value::Null,
        "unit metadata before any calculation must invalidate the old number"
    );
}
#[tokio::test]
async fn auto_k1_r1_unit_change_of_not_yet_indexed_source_requires_a_new_measurement() {
    assert_eq!(
        unindexed_unit_change(true, true).await,
        Value::Float(1000.0),
        "new measurement control"
    );
    assert_eq!(
        unindexed_unit_change(true, false).await,
        Value::Null,
        "adding a previously untracked source must not relabel its old number"
    );
}

#[tokio::test]
async fn auto_k1_r1_unit_invalidation_preserves_age_and_same_unit_metadata_keeps_value() {
    let hub = Hub::new(HubOptions::default()).unwrap();
    let id = DeviceId::from("fixture:source");
    let numeric = Kind::Numeric {
        min: None,
        max: None,
        step: None,
    };
    let original = device(id.clone(), numeric.clone(), Some(Unit::WattHour));
    hub.upsert_device(original.clone());
    hub.set_state(&id, "value", Value::Float(1000.0));
    let point = PointId::new(&id, "value");
    let before = hub.state(&point).unwrap();
    let mut renamed = original.clone();
    renamed.native_name = "Renamed synthetic source".into();
    hub.upsert_device(renamed);
    assert_eq!(
        hub.state(&point).unwrap(),
        before,
        "same-unit labels do not invalidate measurements"
    );
    let mut events = hub.subscribe();
    hub.upsert_device(device(id.clone(), numeric, Some(Unit::KiloWattHour)));
    let invalid = hub.state(&point).unwrap();
    assert_eq!(invalid.value, Value::Null);
    assert_eq!(invalid.ts, before.ts, "metadata is not a new observation");
    let mut found = false;
    while let Ok(event) = events.try_recv() {
        if let Event::State {
            point: p,
            value: Value::Null,
            ts,
            source_ts,
        } = event
            && p == point
        {
            assert_eq!(source_ts, Some(before.ts));
            assert!(ts >= before.ts);
            found = true;
        }
    }
    assert!(
        found,
        "subscribers must also stop displaying the old number in the new unit"
    );
    hub.upsert_device(original);
    assert_eq!(
        hub.state(&point).unwrap().value,
        Value::Null,
        "changing back cannot resurrect the number"
    );
}

#[tokio::test]
async fn auto_k1_timer_numeric_fields_contribute_but_states_and_occurrences_do_not() {
    let hub = Hub::new(HubOptions::default()).unwrap();
    let timer: DeviceId = "fixture:timer-source".into();
    hub.upsert_device(device(
        timer.clone(),
        Kind::Timer {
            field: moli_core::TimerField::Remaining,
        },
        None,
    ));
    hub.set_state(&timer, "value", Value::Float(13.0));
    let mut definition = spec(&["fixture:timer-source/value"]);
    definition.unit = None;
    let reference = define(&hub, "timer-mean", definition.clone()).await;
    approve(&hub, &reference).await.unwrap();
    assert_eq!(value(&hub, &reference), Value::Float(13.0));
    hub.set_state(&timer, "value", Value::Float(14.0));
    hub.run_computations().unwrap();
    assert_eq!(value(&hub, &reference), Value::Float(14.0));
    for field in [moli_core::TimerField::State, moli_core::TimerField::Expired] {
        hub.upsert_device(device(timer.clone(), Kind::Timer { field }, None));
        hub.run_computations().unwrap();
        assert_eq!(value(&hub, &reference), Value::Null);
        assert_eq!(approve(&hub, &reference).await, Err(DependencyError::Units));
    }
}
