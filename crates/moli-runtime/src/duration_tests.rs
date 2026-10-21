use super::*;
use crate::state_file::WritePhase;
use crate::{DependencyDefinition, DependencyError};
use moli_core::{DependencyKind, DependencyRef, DurationSpec, DurationStats, Kind, Unit};

fn hub(options: HubOptions) -> Hub {
    let hub = Hub::new(options).unwrap();
    // Tests explicitly drive every checkpoint, without wall-clock sleeps.
    hub.0.durations.started.store(true, Ordering::Release);
    hub.0.durations.book.lock().unwrap().wall = 0;
    hub
}
fn clock(hub: &Hub, ms: u64) {
    hub.0.durations.book.lock().unwrap().anchor = Instant::now()
        .checked_sub(Duration::from_millis(ms))
        .unwrap();
}
fn spec() -> DurationSpec {
    DurationSpec {
        source: "fixture:lamp/on".into(),
        source_unit: None,
        value: serde_json::json!(true),
        max_age_s: 1,
        window_s: 10,
    }
}
async fn define(hub: &Hub) -> DependencyRef {
    let (fixture, _, source, _) = super::scene_tests::fixture();
    hub.upsert_device((*fixture.describe(&source).unwrap().0).clone());
    let mut output = (*fixture.describe(&source).unwrap().0).clone();
    output.id = "fixture:duration".into();
    output.points[0].key = "value".into();
    output.points[0].kind = Kind::Computed;
    output.points[0].access.write = false;
    hub.upsert_device(output.clone());
    hub.initialize_duration(&output.id, spec()).await.unwrap();
    DependencyRef {
        kind: DependencyKind::Computed,
        id: output.id,
    }
}
async fn publish(hub: &Hub, r: &DependencyRef) {
    hub.publish_dependency(
        hub.preview_dependency(r).await.unwrap(),
        true,
        Origin::Ui,
        None,
    )
    .await
    .unwrap();
}
async fn checkpoint(hub: &Hub, r: &DependencyRef, at: u64) -> DurationStats {
    clock(hub, at);
    hub.checkpoint_durations().await.unwrap();
    hub.duration_stats(&r.id).unwrap()
}
fn near(actual: u64, wanted: u64) {
    assert!(actual.abs_diff(wanted) <= 20, "{actual} != {wanted}");
}

#[tokio::test]
async fn auto_k3_draft_pin_equal_refresh_expiry_and_online_do_not_invent_coverage() {
    let hub = hub(HubOptions::default());
    let r = define(&hub).await;
    assert!(!hub.0.durations.active.load(Ordering::Acquire));
    let preview = hub.preview_dependency(&r).await.unwrap();
    assert_eq!(
        hub.publish_dependency(preview, false, Origin::Mcp, None)
            .await,
        Err(DependencyError::Human)
    );
    publish(&hub, &r).await;
    clock(&hub, 1000);
    hub.set_state(&"fixture:lamp".into(), "on", Value::Bool(true));
    clock(&hub, 2000);
    hub.set_state(&"fixture:lamp".into(), "on", Value::Bool(true));
    let stats = checkpoint(&hub, &r, 3500).await;
    assert!(hub.journal(100).iter().any(|entry|matches!(&entry.action,Action::Computed {point,coverage:Some(summary),..} if point==&r.point().unwrap() && summary==&stats)));
    near(stats.acquired_ms, 2000);
    near(stats.known_ms, 2000);
    assert!(!stats.observing);
    clock(&hub, 4000);
    hub.set_availability(&"fixture:lamp".into(), true);
    let stats = checkpoint(&hub, &r, 4500).await;
    near(stats.known_ms, 2000);
    clock(&hub, 5000);
    hub.set_state(&"fixture:lamp".into(), "on", Value::Bool(false));
    clock(&hub, 5500);
    hub.set_availability(&"fixture:lamp".into(), false);
    let stats = checkpoint(&hub, &r, 6000).await;
    near(stats.known_ms, 2500);
    near(stats.acquired_ms, 2000);
    assert!(!stats.observing);
}
#[tokio::test]
async fn auto_k3_wrong_types_and_description_changes_end_coverage_without_cache_reuse() {
    let hub = hub(HubOptions::default());
    let r = define(&hub).await;
    publish(&hub, &r).await;
    clock(&hub, 1000);
    hub.set_state(&"fixture:lamp".into(), "on", Value::Bool(true));
    clock(&hub, 1500);
    hub.set_state(&"fixture:lamp".into(), "on", Value::from("true"));
    near(checkpoint(&hub, &r, 2000).await.known_ms, 500);
    clock(&hub, 2500);
    hub.set_state(&"fixture:lamp".into(), "on", Value::Bool(true));
    let mut descriptor = (*hub.describe(&"fixture:lamp".into()).unwrap().0).clone();
    descriptor.native_name = "Changed synthetic source".into();
    clock(&hub, 2750);
    hub.upsert_device(descriptor);
    near(checkpoint(&hub, &r, 3500).await.known_ms, 750);
}
fn directory() -> PathBuf {
    std::env::temp_dir().join(format!(
        "moli-k3-{}",
        crate::occurrences::random_id().unwrap()
    ))
}
fn options(path: &std::path::Path) -> HubOptions {
    HubOptions {
        dependency_state_path: Some(path.join("dependencies.json")),
        duration_state_path: Some(path.join("durations.json")),
        ..Default::default()
    }
}
#[tokio::test]
async fn auto_k3_restart_keeps_acquired_open_checkpoint_but_never_the_interruption() {
    let path = directory();
    let first = hub(options(&path));
    let r = define(&first).await;
    publish(&first, &r).await;
    clock(&first, 1000);
    first.set_state(&"fixture:lamp".into(), "on", Value::Bool(true));
    near(checkpoint(&first, &r, 1500).await.acquired_ms, 500);
    let bytes = std::fs::read(path.join("durations.json")).unwrap();
    for _ in 0..2 {
        let restored = hub(options(&path));
        clock(&restored, 5000);
        let rr = define(&restored).await;
        // A cache snapshot is present before the durable definition is read.
        let stats = restored.duration_stats(&rr.id).unwrap();
        near(stats.acquired_ms, 500);
        assert!(!stats.observing);
        let later = checkpoint(&restored, &rr, 6000).await;
        near(later.acquired_ms, 500);
        assert!(later.unknown_ms >= 4000);
    }
    assert_ne!(bytes, std::fs::read(path.join("durations.json")).unwrap());
    std::fs::remove_dir_all(path).unwrap();
}
#[tokio::test]
async fn auto_k3_write_cuts_never_acknowledge_and_recovery_never_recounts() {
    for phase in [
        WritePhase::BeforeWrite,
        WritePhase::BeforeRename,
        WritePhase::AfterRename,
    ] {
        let path = directory();
        let first = hub(options(&path));
        let r = define(&first).await;
        publish(&first, &r).await;
        clock(&first, 1000);
        first.set_state(&"fixture:lamp".into(), "on", Value::Bool(true));
        let before = checkpoint(&first, &r, 1250).await;
        first
            .0
            .durations
            .writer
            .lock()
            .await
            .file
            .interrupt_at(Some(phase));
        clock(&first, 1750);
        assert!(first.checkpoint_durations().await.is_err());
        assert_eq!(first.duration_stats(&r.id), Err(DependencyError::Uncertain));
        assert_eq!(first.state(&r.point().unwrap()).unwrap().value, Value::Null);
        let restored = hub(options(&path));
        clock(&restored, 5000);
        let rr = define(&restored).await;
        let stats = restored.duration_stats(&rr.id).unwrap();
        near(
            stats.acquired_ms,
            if phase == WritePhase::AfterRename {
                750
            } else {
                before.acquired_ms
            },
        );
        near(
            checkpoint(&restored, &rr, 6000).await.acquired_ms,
            stats.acquired_ms,
        );
        std::fs::remove_dir_all(path).unwrap();
    }
}
#[tokio::test]
async fn auto_k3_new_version_resets_measurement_and_mixed_graph_cycles_are_rejected() {
    let hub = hub(HubOptions::default());
    let r = define(&hub).await;
    publish(&hub, &r).await;
    clock(&hub, 1000);
    hub.set_state(&"fixture:lamp".into(), "on", Value::Bool(true));
    near(checkpoint(&hub, &r, 1500).await.acquired_ms, 500);
    let mut next = spec();
    next.value = serde_json::json!(false);
    hub.stage_dependency(
        r.clone(),
        1,
        DependencyDefinition::Duration(next),
        Origin::Mcp,
        None,
    )
    .await
    .unwrap();
    near(hub.duration_stats(&r.id).unwrap().acquired_ms, 500);
    publish(&hub, &r).await;
    assert_eq!(hub.duration_stats(&r.id).unwrap().acquired_ms, 0);
    let mut cycle = spec();
    cycle.source = r.point().unwrap();
    cycle.source_unit = Some(Unit::Second);
    cycle.value = serde_json::json!(0);
    hub.stage_dependency(
        r.clone(),
        2,
        DependencyDefinition::Duration(cycle),
        Origin::Mcp,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        hub.preview_dependency(&r).await,
        Err(DependencyError::Cycle)
    );
}

#[tokio::test]
async fn auto_k3_mixed_graph_bindings_units_and_cycles_preserve_k1() {
    let hub = hub(HubOptions::default());
    let r = define(&hub).await;
    publish(&hub, &r).await;
    let mut descriptor = (*hub.describe(&r.id).unwrap().0).clone();
    descriptor.id = "fixture:total".into();
    hub.upsert_device(descriptor.clone());
    hub.initialize_computed(
        &descriptor.id,
        moli_core::AggregateSpec {
            operation: moli_core::AggregateOp::Max,
            sources: vec![r.point().unwrap()],
            unit: Some(Unit::Second),
            max_age_s: 30,
        },
    )
    .await
    .unwrap();
    let child = DependencyRef {
        kind: DependencyKind::Computed,
        id: descriptor.id,
    };
    publish(&hub, &child).await;
    assert!(
        hub.preview_dependency(&r)
            .await
            .unwrap()
            .related
            .iter()
            .any(|b| b.reference == child && b.role == crate::BindingRole::Affected)
    );
    let mut next = spec();
    next.source = child.point().unwrap();
    next.source_unit = Some(Unit::Second);
    next.value = serde_json::json!(0);
    hub.stage_dependency(
        r.clone(),
        1,
        DependencyDefinition::Duration(next),
        Origin::Mcp,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        hub.preview_dependency(&r).await,
        Err(DependencyError::Cycle)
    );
}
#[tokio::test]
async fn auto_k3_fsync_does_not_lose_transitions_or_publish_uncommitted_suffixes() {
    let path = directory();
    let hub = hub(options(&path));
    let r = define(&hub).await;
    publish(&hub, &r).await;
    clock(&hub, 1000);
    hub.set_state(&"fixture:lamp".into(), "on", Value::Bool(true));
    let entered = Arc::new(tokio::sync::Notify::new());
    let resume = Arc::new(tokio::sync::Notify::new());
    hub.0.durations.writer.lock().await.file.pause_at(
        WritePhase::BeforeRename,
        entered.clone(),
        resume.clone(),
    );
    clock(&hub, 1250);
    let writer = hub.clone();
    let task = tokio::spawn(async move { writer.checkpoint_durations().await });
    entered.notified().await;
    clock(&hub, 1500);
    hub.set_state(&"fixture:lamp".into(), "on", Value::Bool(false));
    clock(&hub, 1750);
    hub.set_state(&"fixture:lamp".into(), "on", Value::Bool(true));
    // The observation path remains synchronous and does not wait for fsync.
    assert_eq!(hub.duration_stats(&r.id).unwrap().known_ms, 0);
    resume.notify_one();
    task.await.unwrap().unwrap();
    near(hub.duration_stats(&r.id).unwrap().acquired_ms, 250);
    hub.0.durations.writer.lock().await.file =
        crate::StateFile::new(Some(path.join("durations.json")), 4 * 1024 * 1024);
    let stats = checkpoint(&hub, &r, 2000).await;
    near(stats.acquired_ms, 750);
    near(stats.known_ms, 1000);
    std::fs::remove_dir_all(path).unwrap();
}

#[tokio::test]
async fn auto_k3_corrupt_state_is_not_reset_or_replaced_by_an_empty_checkpoint() {
    let path = directory();
    std::fs::create_dir_all(&path).unwrap();
    let file = path.join("durations.json");
    let bytes = b"{\"schema\":99,\"records\":{}}";
    std::fs::write(&file, bytes).unwrap();
    let hub = hub(options(&path));
    let r = define(&hub).await;
    assert!(
        hub.publish_dependency(
            hub.preview_dependency(&r).await.unwrap(),
            true,
            Origin::Ui,
            None
        )
        .await
        .is_err()
    );
    assert_eq!(std::fs::read(&file).unwrap(), bytes);
    assert_eq!(hub.state(&r.point().unwrap()).unwrap().value, Value::Null);
    std::fs::remove_dir_all(path).unwrap();
}
#[test]
fn auto_k3_bounded_capture_and_checkpoint_budget() {
    let durations = crate::durations::Durations::new(None);
    let mut book = durations.book.lock().unwrap();
    let source: moli_core::PointId = "fixture:source/on".into();
    for n in 0..128 {
        let id = DeviceId::from(format!("fixture:duration{n}"));
        let mut definition = spec();
        definition.source = source.clone();
        let mut ledger = moli_core::DurationLedger::new(0);
        for at in 0..600 {
            ledger.observe(at, Some(at % 2 == 0), 10000);
        }
        ledger.advance(600);
        book.records.insert(
            id.clone(),
            crate::durations::Record {
                version: 1,
                spec: definition,
                ledger,
            },
        );
        book.active.insert(id.clone());
        book.index.entry(source.clone()).or_default().insert(id);
    }
    let start = Instant::now();
    let bytes = serde_json::to_vec(&book.state()).unwrap();
    println!(
        "AUTO-K3 checkpoint: {} bytes, {:?}",
        bytes.len(),
        start.elapsed()
    );
    assert!(bytes.len() < 4 * 1024 * 1024);
    assert!(book.state().valid());
    let at = book.anchor + Duration::from_secs(1);
    drop(book);
    durations.active.store(true, Ordering::Release);
    let start = Instant::now();
    for _ in 0..1000 {
        durations.sample(
            &source,
            &Value::Bool(true),
            &Kind::Binary,
            None,
            true,
            at,
            Some(at),
        );
    }
    println!("AUTO-K3 capture 1000x128: {:?}", start.elapsed());
    assert_eq!(durations.book.lock().unwrap().records.len(), 128);
}

#[tokio::test]
async fn auto_k3_computed_cache_never_reopens_coverage_but_a_new_equal_sample_does() {
    let hub = hub(HubOptions::default());
    let r = define(&hub).await;
    let mut raw = (*hub.describe(&"fixture:lamp".into()).unwrap().0).clone();
    raw.id = "fixture:raw".into();
    raw.points[0].key = "value".into();
    raw.points[0].kind = Kind::Numeric {
        min: None,
        max: None,
        step: None,
    };
    raw.points[0].access.write = false;
    hub.upsert_device(raw.clone());
    hub.write()
        .states
        .entry(raw.id.clone())
        .or_default()
        .insert(
            "value".into(),
            Sample {
                value: Value::Int(10),
                ts: moli_core::now_ms(),
            },
        );
    let mut aggregate = raw.clone();
    aggregate.id = "fixture:average".into();
    aggregate.points[0].kind = Kind::Computed;
    hub.upsert_device(aggregate.clone());
    hub.initialize_computed(
        &aggregate.id,
        moli_core::AggregateSpec {
            operation: moli_core::AggregateOp::Mean,
            sources: vec![PointId::new(&raw.id, "value")],
            unit: None,
            max_age_s: 30,
        },
    )
    .await
    .unwrap();
    let parent = DependencyRef {
        kind: DependencyKind::Computed,
        id: aggregate.id,
    };
    publish(&hub, &parent).await;
    let mut duration = spec();
    duration.source = parent.point().unwrap();
    duration.value = serde_json::json!(10);
    duration.max_age_s = 30;
    hub.stage_dependency(
        r.clone(),
        1,
        DependencyDefinition::Duration(duration),
        Origin::Mcp,
        None,
    )
    .await
    .unwrap();
    publish(&hub, &r).await;
    hub.0
        .computed
        .signals
        .lock()
        .unwrap()
        .dirty
        .insert(parent.id.clone());
    hub.run_computations().unwrap();
    assert_eq!(checkpoint(&hub, &r, 500).await.known_ms, 0);
    clock(&hub, 1000);
    hub.set_state(&raw.id, "value", Value::Int(10));
    hub.run_computations().unwrap();
    near(checkpoint(&hub, &r, 1500).await.acquired_ms, 500);
}

#[tokio::test]
async fn auto_k3_state_head_mismatch_is_corrupt_instead_of_silently_reset() {
    let path = directory();
    let first = hub(options(&path));
    let r = define(&first).await;
    publish(&first, &r).await;
    let file = path.join("durations.json");
    let mut saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    saved["records"]["fixture:duration"]["spec"]["value"] = serde_json::json!(false);
    let bytes = serde_json::to_vec(&saved).unwrap();
    std::fs::write(&file, &bytes).unwrap();
    let restored = hub(options(&path));
    for id in [DeviceId::from("fixture:lamp"), r.id.clone()] {
        restored.upsert_device((*first.describe(&id).unwrap().0).clone());
    }
    assert_eq!(
        restored.initialize_duration(&r.id, spec()).await,
        Err(DependencyError::Corrupt)
    );
    assert_eq!(std::fs::read(file).unwrap(), bytes);
    std::fs::remove_dir_all(path).unwrap();
}

#[tokio::test]
async fn auto_k3_projection_does_not_write_a_point_whose_kind_changed() {
    let hub = hub(HubOptions::default());
    let r = define(&hub).await;
    publish(&hub, &r).await;
    let mut descriptor = (*hub.describe(&r.id).unwrap().0).clone();
    descriptor.points[0].kind = Kind::Binary;
    descriptor.points[0].access.write = true;
    hub.upsert_device(descriptor);
    hub.set_state(&r.id, "value", Value::Bool(false));
    clock(&hub, 1000);
    hub.checkpoint_durations().await.unwrap();
    assert_eq!(
        hub.state(&r.point().unwrap()).unwrap().value,
        Value::Bool(false)
    );
    assert_eq!(hub.duration_stats(&r.id), Err(DependencyError::Unknown));
}
