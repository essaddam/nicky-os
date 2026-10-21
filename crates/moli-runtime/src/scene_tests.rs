use super::*;
use crate::{SceneStatus, TargetStatus};
use moli_core::{Access, Kind, PointSpec, SceneSpec, SceneTarget, Semantic};
use serde_json::json;

pub(super) fn fixture() -> (Hub, mpsc::Receiver<CommandRequest>, DeviceId, PointId) {
    let hub = Hub::new(HubOptions::default()).unwrap();
    let instance = InstanceId::from("fixture");
    let rx = hub.register_driver(&instance, "fake", "fake".into());
    let id = DeviceId::from("fixture:lamp");
    hub.upsert_device(Device {
        id: id.clone(),
        instance,
        native_name: "Synthetic lamp".into(),
        manufacturer: None,
        model: None,
        description: None,
        native_room: Some("Neutral".into()),
        members: vec![],
        points: vec![PointSpec {
            key: "on".into(),
            label: "On".into(),
            kind: Kind::Binary,
            access: Access {
                read: true,
                write: true,
            },
            unit: None,
            semantic: Semantic::OnOff,
        }],
    });
    let point = PointId::new(&id, "on");
    (hub, rx, id, point)
}
pub(super) fn scene(point: &PointId) -> SceneSpec {
    SceneSpec {
        targets: vec![SceneTarget {
            point: point.clone(),
            value: json!(true),
        }],
        parallel: 4,
        timeout_s: 2,
        confirmation_s: 1,
    }
}
pub(super) async fn publish(hub: &Hub, spec: SceneSpec) -> DeviceId {
    let id = DeviceId::from("scenes:desired");
    hub.initialize_scene(id.clone(), spec).await.unwrap();
    let reference = moli_core::DependencyRef {
        kind: moli_core::DependencyKind::Scene,
        id: id.clone(),
    };
    hub.publish_dependency(
        hub.preview_dependency(&reference).await.unwrap(),
        true,
        Origin::Ui,
        None,
    )
    .await
    .unwrap();
    id
}
pub(super) async fn start(hub: &Hub, id: &DeviceId) -> crate::SceneRun {
    hub.start_scene(
        hub.preview_scene(id).await.unwrap(),
        Origin::Ui,
        Some("Synthetic user".into()),
        None,
    )
    .await
    .unwrap()
}
#[tokio::test]
async fn auto_s1_equal_fresh_echo_confirms_without_deduplicated_state_event() {
    let (hub, mut rx, id, point) = fixture();
    hub.set_state(&id, "on", Value::Bool(true));
    hub.write()
        .states
        .get_mut(&id)
        .unwrap()
        .get_mut("on")
        .unwrap()
        .ts = moli_core::now_ms() - 60_000;
    let scene = publish(&hub, scene(&point)).await;
    let run = start(&hub, &scene).await;
    let cmd = rx.recv().await.unwrap();
    assert_eq!(cmd.metadata.origin, Origin::Ui);
    assert_eq!(cmd.metadata.correlation.as_deref(), Some(run.id.as_str()));
    let mut bus = hub.subscribe();
    assert!(!cmd.abandoned());
    hub.set_state(&id, "on", Value::Bool(true));
    while let Ok(event) = bus.try_recv() {
        assert!(!matches!(event, Event::State { .. }));
    }
    cmd.reply(Ok(()));
    let ended = hub.wait_scene(&run.id).await.unwrap();
    assert_eq!(ended.status, SceneStatus::Done);
    assert!(ended.targets[0].acknowledged);
    assert_eq!(ended.targets[0].status, TargetStatus::Confirmed);
}
#[tokio::test]
async fn auto_s1_acknowledgement_and_old_state_are_not_confirmation() {
    let (hub, mut rx, id, point) = fixture();
    hub.set_state(&id, "on", Value::Bool(true));
    hub.write()
        .states
        .get_mut(&id)
        .unwrap()
        .get_mut("on")
        .unwrap()
        .ts = moli_core::now_ms() - 60_000;
    let scene = publish(&hub, scene(&point)).await;
    let run = start(&hub, &scene).await;
    rx.recv().await.unwrap().reply(Ok(()));
    let ended = hub.wait_scene(&run.id).await.unwrap();
    assert_eq!(ended.status, SceneStatus::Partial);
    assert!(ended.targets[0].acknowledged);
    assert_eq!(ended.targets[0].status, TargetStatus::ConfirmationExpired);
}
#[tokio::test]
async fn auto_s1_cancel_revokes_taken_and_detached_transport_permits() {
    let (hub, mut rx, _, point) = fixture();
    let scene = publish(&hub, scene(&point)).await;
    let run = start(&hub, &scene).await;
    let cmd = rx.recv().await.unwrap();
    assert!(Claim::settle(&cmd.claim, Claim::TAKEN));
    let send = cmd.send_permit(cmd.deadline);
    assert!(send.wanted());
    hub.cancel_scene(&run.id, Origin::Ui, None).unwrap();
    assert!(cmd.abandoned());
    assert!(!send.wanted());
    cmd.reply(Err("cancelled".into()));
    assert_eq!(
        hub.wait_scene(&run.id).await.unwrap().status,
        SceneStatus::Cancelled
    );
}
#[tokio::test]
async fn auto_s1_external_order_supersedes_pending_intent_before_emission() {
    let (hub, mut rx, _, point) = fixture();
    let scene = publish(&hub, scene(&point)).await;
    let run = start(&hub, &scene).await;
    let cmd = rx.recv().await.unwrap();
    assert!(Claim::settle(&cmd.claim, Claim::TAKEN));
    let clone = hub.clone();
    let p = point.clone();
    let external = tokio::spawn(async move {
        clone
            .command(&p, Value::Bool(false), Origin::Ui, None)
            .await
    });
    let next = rx.recv().await.unwrap();
    assert!(cmd.abandoned());
    assert!(!next.abandoned());
    cmd.reply(Err("superseded".into()));
    next.reply(Ok(()));
    external.await.unwrap().unwrap();
    let ended = hub.wait_scene(&run.id).await.unwrap();
    assert_eq!(ended.status, SceneStatus::Partial);
    assert_eq!(ended.targets[0].status, TargetStatus::Superseded);
}
#[tokio::test]
async fn auto_s1_offline_and_partial_failures_do_not_roll_back_confirmed_targets() {
    let (hub, mut rx, id, point) = fixture();
    let other = DeviceId::from("fixture:other");
    let mut device = (*hub.describe(&id).unwrap().0).clone();
    device.id = other.clone();
    hub.upsert_device(device);
    let mut spec = scene(&point);
    spec.targets.push(SceneTarget {
        point: PointId::new(&other, "on"),
        value: json!(false),
    });
    let scene = publish(&hub, spec).await;
    hub.set_availability(&other, false);
    let run = start(&hub, &scene).await;
    let cmd = rx.recv().await.unwrap();
    assert_eq!(cmd.point, point);
    hub.set_state(&id, "on", Value::Bool(true));
    cmd.reply(Ok(()));
    let ended = hub.wait_scene(&run.id).await.unwrap();
    assert_eq!(ended.status, SceneStatus::Partial);
    assert_eq!(ended.targets[0].status, TargetStatus::Confirmed);
    assert_eq!(ended.targets[1].status, TargetStatus::Failed);
    assert!(rx.try_recv().is_err());
}
#[tokio::test]
async fn auto_s1_drafts_stale_scope_and_restart_do_not_act() {
    let (hub, _rx, id, point) = fixture();
    let definition = scene(&point);
    let scene = DeviceId::from("scenes:desired");
    hub.initialize_scene(scene.clone(), definition.clone())
        .await
        .unwrap();
    assert!(hub.preview_scene(&scene).await.is_err());
    let reference = moli_core::DependencyRef {
        kind: moli_core::DependencyKind::Scene,
        id: scene.clone(),
    };
    let old = hub.preview_dependency(&reference).await.unwrap();
    let mut device = (*hub.describe(&id).unwrap().0).clone();
    device.native_name = "Changed synthetic name".into();
    hub.upsert_device(device);
    assert_eq!(
        hub.publish_dependency(old, true, Origin::Ui, None).await,
        Err(crate::DependencyError::Conflict)
    );
    hub.publish_dependency(
        hub.preview_dependency(&reference).await.unwrap(),
        true,
        Origin::Ui,
        None,
    )
    .await
    .unwrap();
    let view = hub.dependency(&reference).await.unwrap();
    let mut changed = definition;
    changed.targets[0].value = json!(false);
    hub.stage_dependency(
        reference.clone(),
        view.versions.draft().version,
        crate::DependencyDefinition::Scene(changed),
        Origin::Mcp,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        hub.preview_scene(&scene).await.unwrap().scope.targets[0].value,
        json!(true)
    );
}

#[tokio::test]
async fn auto_s1_guard_keeps_origin_and_revokes_expired_approval() {
    let (mut hub, _, id, point) = fixture();
    // Recreate the synthetic descriptor in a protected neutral room.
    let descriptor = (*hub.describe(&id).unwrap().0).clone();
    hub = Hub::new(HubOptions {
        guard: crate::guard::GuardPolicy {
            protected_rooms: vec!["Neutral".into()],
            protect_unassigned: false,
            ..Default::default()
        },
        ..Default::default()
    })
    .unwrap();
    let mut rx = hub.register_driver(&"fixture".into(), "fake", "fake".into());
    hub.upsert_device(descriptor);
    let scene = publish(&hub, scene(&point)).await;
    let run = hub
        .start_scene(
            hub.preview_scene(&scene).await.unwrap(),
            Origin::Mcp,
            Some("Synthetic agent".into()),
            None,
        )
        .await
        .unwrap();
    let mut events = hub.subscribe();
    let held = loop {
        if let Some(approval) = hub.approvals().first() {
            break approval.clone();
        }
        let _ = events.recv().await;
    };
    assert_eq!(held.origin, Origin::Mcp);
    assert_eq!(held.actor.as_deref(), Some("Synthetic agent"));
    assert!(rx.try_recv().is_err());
    hub.cancel_scene(&run.id, Origin::Mcp, None).unwrap();
    let ended = hub.wait_scene(&run.id).await.unwrap();
    assert_eq!(ended.status, SceneStatus::Cancelled);
    assert!(
        hub.resolve_approval(held.id, &point, true, Origin::Ui, None)
            .await
            .is_err()
    );
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn auto_s1_same_device_order_and_concurrency_are_bounded() {
    let (hub, mut rx, id, point) = fixture();
    let mut descriptor = (*hub.describe(&id).unwrap().0).clone();
    let mut second = descriptor.points[0].clone();
    second.key = "second".into();
    descriptor.points.push(second);
    hub.upsert_device(descriptor);
    let mut spec = scene(&point);
    spec.targets.push(SceneTarget {
        point: PointId::new(&id, "second"),
        value: json!(false),
    });
    let scene = publish(&hub, spec).await;
    let run = start(&hub, &scene).await;
    let first = rx.recv().await.unwrap();
    assert_eq!(first.key.as_ref(), "on");
    assert!(rx.try_recv().is_err());
    assert!(matches!(
        hub.start_scene(
            hub.preview_scene(&scene).await.unwrap(),
            Origin::Ui,
            None,
            None
        )
        .await,
        Err(crate::SceneError::Busy)
    ));
    hub.set_state(&id, "on", Value::Bool(true));
    first.reply(Ok(()));
    let second = rx.recv().await.unwrap();
    assert_eq!(second.key.as_ref(), "second");
    hub.set_state(&id, "second", Value::Bool(false));
    second.reply(Ok(()));
    assert_eq!(
        hub.wait_scene(&run.id).await.unwrap().status,
        SceneStatus::Done
    );
}

#[tokio::test]
async fn auto_s1_restart_restores_published_scene_but_never_replays_a_run() {
    let path = std::env::temp_dir()
        .join(format!(
            "moli-s1-{}",
            crate::occurrences::random_id().unwrap()
        ))
        .join("dependencies.json");
    let (original, _, id, point) = fixture();
    let descriptor = (*original.describe(&id).unwrap().0).clone();
    let hub = Hub::new(HubOptions {
        dependency_state_path: Some(path.clone()),
        ..Default::default()
    })
    .unwrap();
    let mut rx = hub.register_driver(&"fixture".into(), "fake", "fake".into());
    hub.upsert_device(descriptor.clone());
    let spec = scene(&point);
    let scene = publish(&hub, spec.clone()).await;
    let run = start(&hub, &scene).await;
    let first = rx.recv().await.unwrap();
    hub.set_state(&id, "on", Value::Bool(true));
    first.reply(Ok(()));
    hub.wait_scene(&run.id).await.unwrap();
    let reference = moli_core::DependencyRef {
        kind: moli_core::DependencyKind::Scene,
        id: scene.clone(),
    };
    let mut changed = spec.clone();
    changed.targets[0].value = json!(false);
    hub.stage_dependency(
        reference,
        1,
        crate::DependencyDefinition::Scene(changed.clone()),
        Origin::Mcp,
        None,
    )
    .await
    .unwrap();
    drop(hub);
    let hub = Hub::new(HubOptions {
        dependency_state_path: Some(path),
        ..Default::default()
    })
    .unwrap();
    let mut rx = hub.register_driver(&"fixture".into(), "fake", "fake".into());
    hub.upsert_device(descriptor);
    hub.initialize_scene(scene.clone(), changed).await.unwrap();
    assert_eq!(
        hub.preview_scene(&scene).await.unwrap().scope.targets[0].value,
        json!(true)
    );
    assert!(hub.scene_run(&run.id).is_none());
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn auto_s1_recent_already_conform_state_needs_no_order_or_fake_ack() {
    let (hub, mut rx, id, point) = fixture();
    hub.set_state(&id, "on", Value::Bool(true));
    let scene = publish(&hub, scene(&point)).await;
    let run = start(&hub, &scene).await;
    let ended = hub.wait_scene(&run.id).await.unwrap();
    assert_eq!(ended.status, SceneStatus::Done);
    assert!(!ended.targets[0].acknowledged);
    assert!(
        rx.try_recv().is_err(),
        "an already-conform target must not be toggled or rewritten"
    );
}

#[tokio::test]
async fn auto_s1_composite_follow_up_survives_parent_reply_but_not_a_new_member_order() {
    let (hub, mut rx, id, point) = fixture();
    let child = DeviceId::from("fixture:child");
    let mut descriptor = (*hub.describe(&id).unwrap().0).clone();
    descriptor.id = child.clone();
    hub.upsert_device(descriptor.clone());
    descriptor.id = id.clone();
    descriptor.members = vec![child.clone()];
    hub.upsert_device(descriptor);
    let scene = publish(&hub, scene(&point)).await;
    let run = start(&hub, &scene).await;
    let cmd = rx.recv().await.unwrap();
    let delegated = cmd.member_permit().unwrap();
    hub.set_state(&id, "on", Value::Bool(true));
    cmd.reply(Ok(()));
    assert_eq!(
        hub.wait_scene(&run.id).await.unwrap().status,
        SceneStatus::Done
    );
    let child_point = PointId::new(&child, "on");
    assert!(
        delegated.allows(&child_point),
        "normal parent completion must retain its bounded composite continuation"
    );
    let other = hub.clone();
    let p = child_point.clone();
    let external = tokio::spawn(async move {
        other
            .command(&p, Value::Bool(false), Origin::Ui, None)
            .await
    });
    let cmd = rx.recv().await.unwrap();
    assert!(
        !delegated.allows(&child_point),
        "a new member intent revokes the old composite continuation"
    );
    cmd.reply(Ok(()));
    external.await.unwrap().unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn auto_s1_four_global_lanes_bound_two_disjoint_scenes() {
    let (hub, mut rx, id, _) = fixture();
    let base = (*hub.describe(&id).unwrap().0).clone();
    let mut points = Vec::new();
    for i in 0..5 {
        let mut device = base.clone();
        device.id = DeviceId::from(format!("fixture:lamp_{i}"));
        points.push(PointId::new(&device.id, "on"));
        hub.upsert_device(device);
    }
    let mut scene_ids = Vec::new();
    for (i, range) in [0..3, 3..5].into_iter().enumerate() {
        let id = DeviceId::from(format!("scenes:desired_{i}"));
        let mut spec = scene(&points[0]);
        spec.timeout_s = 10;
        spec.confirmation_s = 5;
        spec.targets = range
            .map(|j| SceneTarget {
                point: points[j].clone(),
                value: json!(true),
            })
            .collect();
        hub.initialize_scene(id.clone(), spec).await.unwrap();
        let reference = moli_core::DependencyRef {
            kind: moli_core::DependencyKind::Scene,
            id: id.clone(),
        };
        hub.publish_dependency(
            hub.preview_dependency(&reference).await.unwrap(),
            true,
            Origin::Ui,
            None,
        )
        .await
        .unwrap();
        scene_ids.push(id);
    }
    let mut events = hub.subscribe();
    let mut runs = Vec::new();
    for id in scene_ids {
        runs.push(start(&hub, &id).await);
    }
    let mut commands = Vec::new();
    for _ in 0..4 {
        commands.push(rx.recv().await.unwrap());
    }
    // Each target emits synchronously before polling its semaphore. Once all
    // five starts are consumed, every lane has attempted admission.
    let mut started = 0;
    while started < 5 {
        if let Event::Journal { entry } = events.recv().await.unwrap()
            && matches!(&entry.action,Action::Scene{point:Some(_),status,..} if status=="waiting")
        {
            started += 1;
        }
    }
    assert!(
        rx.try_recv().is_err(),
        "the fifth command cannot enter before a lane is released"
    );
    let first = commands.remove(0);
    hub.set_state(&first.device.id, &first.key, Value::Bool(true));
    first.reply(Ok(()));
    commands.push(rx.recv().await.unwrap());
    for command in commands {
        hub.set_state(&command.device.id, &command.key, Value::Bool(true));
        command.reply(Ok(()));
    }
    for run in runs {
        assert_eq!(
            hub.wait_scene(&run.id).await.unwrap().status,
            SceneStatus::Done
        );
    }
}

#[tokio::test]
async fn auto_s1_next_target_supersedes_previous_deferred_commands_on_the_same_scope() {
    let (hub, mut rx, id, point) = fixture();
    let child = DeviceId::from("fixture:child");
    let mut descriptor = (*hub.describe(&id).unwrap().0).clone();
    descriptor.id = child.clone();
    hub.upsert_device(descriptor.clone());
    descriptor.id = id.clone();
    descriptor.members = vec![child.clone()];
    let mut second = descriptor.points[0].clone();
    second.key = "second".into();
    descriptor.points.push(second);
    hub.upsert_device(descriptor);
    let mut spec = scene(&point);
    spec.targets.push(SceneTarget {
        point: PointId::new(&id, "second"),
        value: json!(false),
    });
    let scene = publish(&hub, spec).await;
    let run = start(&hub, &scene).await;
    let first = rx.recv().await.unwrap();
    let delegated = first.member_permit().unwrap();
    hub.set_state(&id, "on", Value::Bool(true));
    first.reply(Ok(()));
    let second = rx.recv().await.unwrap();
    assert!(
        !delegated.allows(&PointId::new(&child, "on")),
        "a previous continuation cannot overtake the next target of this device"
    );
    hub.set_state(&id, "second", Value::Bool(false));
    second.reply(Ok(()));
    assert_eq!(
        hub.wait_scene(&run.id).await.unwrap().status,
        SceneStatus::Done
    );
}

#[tokio::test]
async fn auto_s1_confirmation_ignores_observations_before_admission_or_after_deadline() {
    for before in [true, false] {
        let (hub, mut rx, _, point) = fixture();
        let scene = publish(&hub, scene(&point)).await;
        let run = start(&hub, &scene).await;
        let command = rx.recv().await.unwrap();
        // Simulate a producer delayed before the model lock, or a consumer
        // resumed after the confirmation deadline. The signal uses its real
        // capture instant, not the consumer's eventual wakeup time.
        let at = if before {
            Instant::now().checked_sub(Duration::from_secs(60)).unwrap()
        } else {
            Instant::now() + Duration::from_secs(60)
        };
        hub.scene_observe(&point, &Value::Bool(true), at);
        command.reply(Ok(()));
        let ended = hub.wait_scene(&run.id).await.unwrap();
        assert_eq!(ended.targets[0].status, TargetStatus::ConfirmationExpired);
        assert!(ended.targets[0].acknowledged);
    }
}

#[tokio::test]
async fn auto_s1_guard_approval_preserves_metadata_and_confirms_a_fresh_observation() {
    let (original, _, id, point) = fixture();
    let descriptor = (*original.describe(&id).unwrap().0).clone();
    let hub = Hub::new(HubOptions {
        guard: crate::guard::GuardPolicy {
            protected_rooms: vec!["Neutral".into()],
            protect_unassigned: false,
            ..Default::default()
        },
        ..Default::default()
    })
    .unwrap();
    let mut rx = hub.register_driver(&"fixture".into(), "fake", "fake".into());
    hub.upsert_device(descriptor);
    let scene = publish(&hub, scene(&point)).await;
    let mut events = hub.subscribe();
    let run = hub
        .start_scene(
            hub.preview_scene(&scene).await.unwrap(),
            Origin::Mcp,
            Some("Synthetic agent".into()),
            None,
        )
        .await
        .unwrap();
    let held = loop {
        if let Some(r) = hub.approvals().first() {
            break r.clone();
        }
        let _ = events.recv().await;
    };
    let other = hub.clone();
    let p = point.clone();
    let approve = tokio::spawn(async move {
        other
            .resolve_approval(held.id, &p, true, Origin::Ui, None)
            .await
    });
    let command = rx.recv().await.unwrap();
    assert_eq!(command.metadata.origin, Origin::Mcp);
    assert_eq!(command.metadata.actor.as_deref(), Some("Synthetic agent"));
    hub.set_state(&id, "on", Value::Bool(true));
    command.reply(Ok(()));
    approve.await.unwrap().unwrap();
    let ended = hub.wait_scene(&run.id).await.unwrap();
    assert_eq!(ended.status, SceneStatus::Done);
    assert!(ended.targets[0].acknowledged);
}

#[tokio::test]
async fn auto_s1_parent_permission_and_correlation_remain_attached_until_emission() {
    let (hub, mut rx, _, point) = fixture();
    let scene = publish(&hub, scene(&point)).await;
    let allowed = Arc::new(AtomicBool::new(true));
    let check = allowed.clone();
    let parent = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let run = hub
        .start_scene_correlated(
            hub.preview_scene(&scene).await.unwrap(),
            Origin::Automation,
            Some("Synthetic graph".into()),
            Some(crate::CommandPermit::new(move |_| {
                check.load(Ordering::Acquire)
            })),
            Some(parent.into()),
        )
        .await
        .unwrap();
    let command = rx.recv().await.unwrap();
    assert!(!command.abandoned());
    assert_eq!(run.parent_correlation.as_deref(), Some(parent));
    assert_eq!(
        command.metadata.correlation.as_deref(),
        Some(run.id.as_str())
    );
    allowed.store(false, Ordering::Release);
    assert!(command.abandoned());
    command.reply(Err("parent revoked".into()));
    hub.wait_scene(&run.id).await.unwrap();
    assert!(
        hub.journal(100).iter().any(
            |e| matches!(&e.action,Action::Scene {parent_correlation:Some(c),..} if c==parent)
        )
    );
}

#[tokio::test]
async fn auto_s1_uncertain_definition_state_revokes_pending_scene_effects() {
    let (hub, mut rx, _, point) = fixture();
    let scene = publish(&hub, scene(&point)).await;
    let run = start(&hub, &scene).await;
    let command = rx.recv().await.unwrap();
    assert!(!command.abandoned());
    hub.dependency_fence();
    assert!(command.abandoned());
    command.reply(Err("definitions fenced".into()));
    assert_eq!(
        hub.wait_scene(&run.id).await.unwrap().status,
        SceneStatus::Cancelled
    );
    assert!(hub.current_scene(&scene).is_none());
}

#[tokio::test]
async fn auto_s1_published_scope_cannot_expand_without_a_new_human_validated_version() {
    let (hub, _rx, id, point) = fixture();
    let spec = scene(&point);
    let scene = publish(&hub, spec.clone()).await;
    let original = hub.preview_scene(&scene).await.unwrap();
    let mut descriptor = (*hub.describe(&id).unwrap().0).clone();
    descriptor.native_room = Some("Protected synthetic room".into());
    hub.upsert_device(descriptor);
    assert!(
        matches!(
            hub.preview_scene(&scene).await,
            Err(crate::SceneError::Conflict)
        ),
        "a new current scope cannot be silently inferred from changed metadata"
    );
    assert!(hub.current_scene(&scene).is_none());
    assert!(
        hub.start_scene(original, Origin::Automation, None, None)
            .await
            .is_err()
    );
    let reference = moli_core::DependencyRef {
        kind: moli_core::DependencyKind::Scene,
        id: scene.clone(),
    };
    hub.stage_dependency(
        reference.clone(),
        1,
        crate::DependencyDefinition::Scene(spec),
        Origin::Ui,
        None,
    )
    .await
    .unwrap();
    let preview = hub.preview_dependency(&reference).await.unwrap();
    assert_eq!(
        preview.scene.as_ref().unwrap().targets[0].rooms,
        vec!["Protected synthetic room".to_owned()]
    );
    assert!(
        hub.publish_dependency(preview.clone(), false, Origin::Mcp, None)
            .await
            .is_err()
    );
    hub.publish_dependency(preview, true, Origin::Ui, None)
        .await
        .unwrap();
    assert_eq!(hub.preview_scene(&scene).await.unwrap().version, 2);
}

#[tokio::test]
async fn auto_s1_approval_acknowledgement_is_visible_while_state_is_still_unconfirmed() {
    let (original, _, id, point) = fixture();
    let descriptor = (*original.describe(&id).unwrap().0).clone();
    let hub = Hub::new(HubOptions {
        guard: crate::guard::GuardPolicy {
            protected_rooms: vec!["Neutral".into()],
            protect_unassigned: false,
            ..Default::default()
        },
        ..Default::default()
    })
    .unwrap();
    let mut rx = hub.register_driver(&"fixture".into(), "fake", "fake".into());
    hub.upsert_device(descriptor);
    let scene = publish(&hub, scene(&point)).await;
    let mut events = hub.subscribe();
    let run = hub
        .start_scene(
            hub.preview_scene(&scene).await.unwrap(),
            Origin::Mcp,
            None,
            None,
        )
        .await
        .unwrap();
    let held = loop {
        if let Some(r) = hub.approvals().first() {
            break r.clone();
        }
        let _ = events.recv().await;
    };
    let other = hub.clone();
    let p = point.clone();
    let approve = tokio::spawn(async move {
        other
            .resolve_approval(held.id, &p, true, Origin::Ui, None)
            .await
    });
    rx.recv().await.unwrap().reply(Ok(()));
    approve.await.unwrap().unwrap();
    tokio::time::timeout(Duration::from_secs(1),async {
        loop {
            if let Event::Journal{entry}=events.recv().await.unwrap()
                && matches!(&entry.action,Action::Scene{point:Some(_),status,..} if status=="acknowledged") {break;}
        }
    }).await.unwrap();
    let waiting = hub.scene_run(&run.id).unwrap();
    assert_eq!(waiting.status, SceneStatus::Running);
    assert_eq!(waiting.targets[0].status, TargetStatus::Acknowledged);
    assert!(waiting.targets[0].acknowledged);
    hub.set_state(&id, "on", Value::Bool(true));
    assert_eq!(
        hub.wait_scene(&run.id).await.unwrap().status,
        SceneStatus::Done
    );
}

fn numeric_fixture() -> (Hub, mpsc::Receiver<CommandRequest>, DeviceId, PointId) {
    let (hub, rx, id, point) = fixture();
    let mut device = (*hub.describe(&id).unwrap().0).clone();
    device.points[0].kind = Kind::Numeric {
        min: Some(0.0),
        max: Some(100.0),
        step: Some(1.0),
    };
    device.points[0].semantic = Semantic::Other;
    hub.upsert_device(device);
    (hub, rx, id, point)
}
fn numeric_scene(point: &PointId) -> SceneSpec {
    let mut spec = scene(point);
    spec.targets[0].value = json!(42);
    spec
}

#[tokio::test]
async fn auto_s1_numeric_recent_integer_and_float_need_no_order() {
    for value in [Value::Float(42.0), Value::Int(42)] {
        let (hub, mut rx, id, point) = numeric_fixture();
        hub.set_state(&id, "on", value.clone());
        let scene = publish(&hub, numeric_scene(&point)).await;
        let run = start(&hub, &scene).await;
        let ended = hub.wait_scene(&run.id).await.unwrap();
        assert_eq!(ended.status, SceneStatus::Done, "{value:?}");
        assert_eq!(ended.targets[0].status, TargetStatus::AlreadyConform);
        assert!(!ended.targets[0].acknowledged);
        assert!(rx.try_recv().is_err());
    }
}

#[tokio::test]
async fn auto_s1_numeric_integer_and_float_observations_confirm_after_order() {
    for value in [Value::Float(42.0), Value::Int(42)] {
        let (hub, mut rx, id, point) = numeric_fixture();
        hub.set_state(&id, "on", Value::Int(0));
        let scene = publish(&hub, numeric_scene(&point)).await;
        let run = start(&hub, &scene).await;
        let command = rx.recv().await.unwrap();
        hub.set_state(&id, "on", value.clone());
        command.reply(Ok(()));
        let ended = hub.wait_scene(&run.id).await.unwrap();
        assert_eq!(ended.status, SceneStatus::Done, "{value:?}");
        assert_eq!(ended.targets[0].status, TargetStatus::Confirmed);
        assert!(ended.targets[0].acknowledged);
        assert!(rx.try_recv().is_err());
    }
}

#[tokio::test]
async fn auto_s1_numeric_different_or_invalid_observations_do_not_confirm() {
    for value in [
        Value::Int(43),
        Value::Float(43.0),
        Value::from("42"),
        Value::Bool(true),
        Value::Int(142),
        Value::Float(f64::NAN),
    ] {
        let (hub, mut rx, id, point) = numeric_fixture();
        hub.set_state(&id, "on", value.clone());
        let scene = publish(&hub, numeric_scene(&point)).await;
        let run = start(&hub, &scene).await;
        let command = rx.recv().await.unwrap();
        hub.set_state(&id, "on", value.clone());
        command.reply(Ok(()));
        let ended = hub.wait_scene(&run.id).await.unwrap();
        assert_eq!(ended.status, SceneStatus::Partial, "{value:?}");
        assert_eq!(ended.targets[0].status, TargetStatus::ConfirmationExpired);
        assert!(ended.targets[0].acknowledged);
        assert!(rx.try_recv().is_err());
    }
}
