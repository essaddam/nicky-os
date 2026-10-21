use crate::dependencies::*;
use crate::state_file::WritePhase;
use crate::{Hub, HubOptions};
use moli_core::{
    Access, DependencyKind, DependencyRef, Device, Kind, Origin, PointId, PointSpec, Semantic,
    Value, WeeklySchedule, WeeklySlot,
};

fn plan(on: bool) -> WeeklySchedule {
    WeeklySchedule {
        timezone: "UTC".into(),
        slots: if on {
            (1..=7)
                .map(|day| WeeklySlot {
                    day,
                    start: 0,
                    end: 1440,
                })
                .collect()
        } else {
            vec![]
        },
    }
}
async fn hub(path: std::path::PathBuf) -> (Hub, DependencyRef) {
    let hub = Hub::new(HubOptions {
        dependency_state_path: Some(path),
        ..Default::default()
    })
    .unwrap();
    let reference = DependencyRef {
        kind: DependencyKind::Schedule,
        id: "fixture:weekly".into(),
    };
    hub.upsert_device(Device {
        id: reference.id.clone(),
        instance: "fixture".into(),
        native_name: "Weekly".into(),
        manufacturer: None,
        model: None,
        description: None,
        native_room: None,
        members: vec![],
        points: vec![PointSpec {
            key: "active".into(),
            label: "Active".into(),
            kind: Kind::Schedule,
            access: Access {
                read: true,
                write: false,
            },
            unit: None,
            semantic: Semantic::Other,
        }],
    });
    hub.initialize_schedule(&reference.id, plan(false))
        .await
        .unwrap();
    (hub, reference)
}
fn path() -> std::path::PathBuf {
    std::env::temp_dir()
        .join(format!(
            "moli-h5-{}",
            crate::occurrences::random_id().unwrap()
        ))
        .join("dependencies.json")
}

#[tokio::test]
async fn auto_h5_draft_cannot_act_and_publication_requires_human_exact_impact() {
    let (hub, reference) = hub(path()).await;
    let point = PointId::new(&reference.id, "active");
    assert_eq!(hub.state(&point).unwrap().value, Value::Null);
    hub.stage_dependency(
        reference.clone(),
        1,
        DependencyDefinition::Schedule(plan(true)),
        Origin::Mcp,
        None,
    )
    .await
    .unwrap();
    assert_eq!(hub.state(&point).unwrap().value, Value::Null);
    let preview = hub.preview_dependency(&reference).await.unwrap();
    assert_eq!(
        hub.publish_dependency(preview.clone(), false, Origin::Ui, None)
            .await,
        Err(DependencyError::Human)
    );
    assert_eq!(hub.state(&point).unwrap().value, Value::Null);
    hub.set_dependency_consumers(std::collections::BTreeMap::from([(
        reference.clone(),
        vec![DependencyConsumer {
            id: "graph".into(),
            name: "Graph".into(),
            fingerprint: "changed".into(),
        }],
    )]));
    assert_eq!(
        hub.publish_dependency(preview, true, Origin::Ui, None)
            .await,
        Err(DependencyError::Conflict)
    );
    let preview = hub.preview_dependency(&reference).await.unwrap();
    assert_eq!(preview.consumers.len(), 1);
    hub.publish_dependency(preview, true, Origin::Ui, None)
        .await
        .unwrap();
    assert_eq!(hub.state(&point).unwrap().value, Value::Bool(true));
    hub.stage_dependency(
        reference.clone(),
        2,
        DependencyDefinition::Schedule(plan(false)),
        Origin::Mcp,
        None,
    )
    .await
    .unwrap();
    assert_eq!(hub.state(&point).unwrap().value, Value::Bool(true));
    assert!(hub.dependency_ready(&reference));
}

#[tokio::test]
async fn auto_h5_concurrent_edits_conflict_and_restart_restores_current_not_draft() {
    let path = path();
    let (first, reference) = hub(path.clone()).await;
    first
        .publish_dependency(
            first.preview_dependency(&reference).await.unwrap(),
            true,
            Origin::Ui,
            None,
        )
        .await
        .unwrap();
    let (a, b) = tokio::join!(
        first.stage_dependency(
            reference.clone(),
            1,
            DependencyDefinition::Schedule(plan(true)),
            Origin::Mcp,
            None
        ),
        first.stage_dependency(
            reference.clone(),
            1,
            DependencyDefinition::Schedule(plan(false)),
            Origin::Mcp,
            None
        )
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert!(a == Err(DependencyError::Conflict) || b == Err(DependencyError::Conflict));
    let (second, restored) = hub(path.clone()).await;
    let view = second.dependency(&restored).await.unwrap();
    assert_eq!(view.versions.current().unwrap().version, 1);
    assert_eq!(view.versions.draft().version, 2);
    assert_eq!(
        second
            .state(&PointId::new(&restored.id, "active"))
            .unwrap()
            .value,
        Value::Bool(false)
    );
    assert!(!second.journal(100).iter().any(|entry| matches!(
        entry.action,
        moli_core::Action::Dependency {
            operation: moli_core::DependencyChange::Transition,
            ..
        }
    )));
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[tokio::test]
async fn auto_h5_cut_before_rename_preserves_current_after_rename_fences_every_projection() {
    let path = path();
    let (hub, reference) = hub(path.clone()).await;
    hub.publish_dependency(
        hub.preview_dependency(&reference).await.unwrap(),
        true,
        Origin::Ui,
        None,
    )
    .await
    .unwrap();
    hub.stage_dependency(
        reference.clone(),
        1,
        DependencyDefinition::Schedule(plan(true)),
        Origin::Mcp,
        None,
    )
    .await
    .unwrap();
    let preview = hub.preview_dependency(&reference).await.unwrap();
    hub.0
        .dependencies
        .lock()
        .await
        .file
        .interrupt_at(Some(WritePhase::BeforeRename));
    assert_eq!(
        hub.publish_dependency(preview.clone(), true, Origin::Ui, None)
            .await,
        Err(DependencyError::Storage)
    );
    let point = PointId::new(&reference.id, "active");
    assert_eq!(hub.state(&point).unwrap().value, Value::Bool(false));
    hub.0
        .dependencies
        .lock()
        .await
        .file
        .interrupt_at(Some(WritePhase::AfterRename));
    assert_eq!(
        hub.publish_dependency(preview, true, Origin::Ui, None)
            .await,
        Err(DependencyError::Uncertain)
    );
    assert!(!hub.dependency_ready(&reference));
    assert_eq!(hub.state(&point).unwrap().value, Value::Null);
    assert_eq!(
        hub.dependency(&reference).await,
        Err(DependencyError::Uncertain)
    );
    let (restored, reference) = self::hub(path.clone()).await;
    assert_eq!(
        restored
            .dependency(&reference)
            .await
            .unwrap()
            .versions
            .current()
            .unwrap()
            .version,
        2
    );
    assert_eq!(restored.state(&point).unwrap().value, Value::Bool(true));
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[tokio::test]
async fn auto_h5_transition_persistence_is_deduplicated_and_does_not_replay_missed_slots() {
    let path = path();
    let mut owner = Dependencies::new(Some(path.clone()));
    let reference = DependencyRef {
        kind: DependencyKind::Schedule,
        id: "fixture:weekly".into(),
    };
    let plan = WeeklySchedule {
        timezone: "UTC".into(),
        slots: vec![WeeklySlot {
            day: 1,
            start: 0,
            end: 60,
        }],
    };
    let midnight: jiff::Timestamp = "2026-10-19T00:00:00Z".parse().unwrap();
    owner
        .initialize(
            reference.clone(),
            DependencyDefinition::Schedule(plan),
            midnight,
        )
        .await
        .unwrap();
    owner
        .publish(
            &reference,
            owner.view(&reference).unwrap().versions.versions(),
            midnight,
        )
        .await
        .unwrap();
    let end: jiff::Timestamp = "2026-10-19T01:00:00Z".parse().unwrap();
    assert_eq!(
        owner.tick(end, false).await.unwrap(),
        vec![(reference.clone(), false)]
    );
    assert!(owner.tick(end, false).await.unwrap().is_empty());
    let jumped: jiff::Timestamp = "2026-10-19T00:30:00Z".parse().unwrap();
    assert_eq!(
        owner.tick(jumped, true).await.unwrap(),
        vec![(reference.clone(), true)]
    );
    let mut restored = Dependencies::new(Some(path.clone()));
    restored
        .initialize(
            reference.clone(),
            DependencyDefinition::Schedule(self::plan(false)),
            end,
        )
        .await
        .unwrap();
    assert_eq!(restored.projection(&reference).unwrap(), Some(false));
    assert!(restored.tick(end, false).await.unwrap().is_empty());
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[tokio::test]
async fn auto_h5_waiter_loss_after_engagement_keeps_commit_and_journal() {
    use std::sync::Arc;
    let (hub, reference) = hub(path()).await;
    let entered = Arc::new(tokio::sync::Notify::new());
    let resume = Arc::new(tokio::sync::Notify::new());
    hub.0.dependencies.lock().await.file.pause_at(
        WritePhase::BeforeRename,
        entered.clone(),
        resume.clone(),
    );
    let worker = hub.clone();
    let target = reference.clone();
    let task = tokio::spawn(async move {
        worker
            .stage_dependency(
                target,
                1,
                DependencyDefinition::Schedule(plan(true)),
                Origin::Mcp,
                None,
            )
            .await
    });
    entered.notified().await;
    task.abort();
    resume.notify_one();
    let view = hub.dependency(&reference).await.unwrap();
    assert_eq!(view.versions.draft().version, 2);
    assert!(view.versions.current().is_none());
    assert!(hub.journal(100).iter().any(|entry| matches!(
        entry.action,
        moli_core::Action::Dependency {
            operation: moli_core::DependencyChange::Drafted,
            version: Some(2),
            ..
        }
    ) && entry.outcome == moli_core::Outcome::Ok));
}

#[tokio::test]
async fn auto_h5_definition_capacity_and_corruption_do_not_reset_state() {
    let path = path();
    let mut owner = Dependencies::new(None);
    let now = jiff::Timestamp::now();
    for index in 0..128 {
        owner
            .initialize(
                DependencyRef {
                    kind: DependencyKind::Schedule,
                    id: format!("fixture:w{index}").into(),
                },
                DependencyDefinition::Schedule(plan(false)),
                now,
            )
            .await
            .unwrap();
    }
    assert_eq!(
        owner
            .initialize(
                DependencyRef {
                    kind: DependencyKind::Schedule,
                    id: "fixture:overflow".into()
                },
                DependencyDefinition::Schedule(plan(false)),
                now
            )
            .await,
        Err(DependencyError::Capacity)
    );
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"{broken").unwrap();
    let mut corrupt = Dependencies::new(Some(path.clone()));
    assert_eq!(
        corrupt
            .initialize(
                DependencyRef {
                    kind: DependencyKind::Schedule,
                    id: "fixture:weekly".into()
                },
                DependencyDefinition::Schedule(plan(false)),
                now
            )
            .await,
        Err(DependencyError::Corrupt)
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"{broken");
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[tokio::test]
async fn auto_h5_failed_calendar_transition_never_leaves_a_stale_executable_boolean() {
    let path = path();
    let (hub, reference) = hub(path.clone()).await;
    let plan = WeeklySchedule {
        timezone: "UTC".into(),
        slots: vec![WeeklySlot {
            day: 1,
            start: 0,
            end: 60,
        }],
    };
    hub.stage_dependency(
        reference.clone(),
        1,
        DependencyDefinition::Schedule(plan),
        Origin::Mcp,
        None,
    )
    .await
    .unwrap();
    hub.publish_dependency(
        hub.preview_dependency(&reference).await.unwrap(),
        true,
        Origin::Ui,
        None,
    )
    .await
    .unwrap();
    let mut owner = hub.0.dependencies.lock().await;
    let before = owner.projection(&reference).unwrap().unwrap();
    // Choose an absolute civil instant with the opposite state, independent of
    // the wall clock at which the test runs.
    let target: jiff::Timestamp = if before {
        "2026-10-20T01:00:00Z"
    } else {
        "2026-10-19T00:30:00Z"
    }
    .parse()
    .unwrap();
    owner.file.interrupt_at(Some(WritePhase::BeforeRename));
    assert!(hub.dependency_tick(&mut owner, target, true).await);
    drop(owner);
    assert!(!hub.dependency_ready(&reference));
    assert_eq!(
        hub.state(&PointId::new(&reference.id, "active"))
            .unwrap()
            .value,
        Value::Null
    );
    assert_eq!(
        hub.dependency(&reference).await,
        Err(DependencyError::Projection)
    );
    assert!(hub.journal(100).iter().any(|entry| matches!(
        entry.action,
        moli_core::Action::Dependency {
            operation: moli_core::DependencyChange::Transition,
            ..
        }
    ) && matches!(
        entry.outcome,
        moli_core::Outcome::Err(_)
    )));
    let (restored, reference) = self::hub(path.clone()).await;
    assert_eq!(
        restored
            .dependency(&reference)
            .await
            .unwrap()
            .versions
            .current()
            .unwrap()
            .version,
        2
    );
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}
