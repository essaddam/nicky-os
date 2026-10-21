use super::scene_tests::{fixture, publish, scene, start};
use super::*;
use crate::state_file::WritePhase;
use crate::{DependencyDefinition, DependencyError, SceneStatus};

async fn durable() -> (
    Hub,
    mpsc::Receiver<CommandRequest>,
    DeviceId,
    PointId,
    DeviceId,
    std::path::PathBuf,
) {
    let (original, _, id, point) = fixture();
    let descriptor = (*original.describe(&id).unwrap().0).clone();
    let path = std::env::temp_dir()
        .join(format!(
            "moli-s1-cut-{}",
            crate::occurrences::random_id().unwrap()
        ))
        .join("dependencies.json");
    let hub = Hub::new(HubOptions {
        dependency_state_path: Some(path.clone()),
        ..Default::default()
    })
    .unwrap();
    let rx = hub.register_driver(&"fixture".into(), "fake", "fake".into());
    hub.upsert_device(descriptor);
    let scene_id = publish(&hub, scene(&point)).await;
    (hub, rx, id, point, scene_id, path)
}
fn reference(id: DeviceId) -> moli_core::DependencyRef {
    moli_core::DependencyRef {
        kind: moli_core::DependencyKind::Scene,
        id,
    }
}

#[tokio::test]
async fn auto_s1_scope_and_head_commit_together_and_uncertainty_revokes_an_active_run() {
    let (hub, mut rx, id, point, scene_id, path) = durable().await;
    let descriptor = (*hub.describe(&id).unwrap().0).clone();
    let run = start(&hub, &scene_id).await;
    let command = rx.recv().await.unwrap();
    let mut next = scene(&point);
    next.targets[0].value = serde_json::json!(false);
    let reference = reference(scene_id.clone());
    hub.stage_dependency(
        reference.clone(),
        1,
        DependencyDefinition::Scene(next),
        Origin::Ui,
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
    assert_eq!(hub.preview_scene(&scene_id).await.unwrap().version, 1);
    assert!(!command.abandoned());
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
    assert!(command.abandoned());
    command.reply(Err("storage outcome uncertain".into()));
    assert_eq!(
        hub.wait_scene(&run.id).await.unwrap().status,
        SceneStatus::Cancelled
    );
    assert!(hub.current_scene(&scene_id).is_none());
    let restored = Hub::new(HubOptions {
        dependency_state_path: Some(path.clone()),
        ..Default::default()
    })
    .unwrap();
    let mut rx = restored.register_driver(&"fixture".into(), "fake", "fake".into());
    restored.upsert_device(descriptor);
    restored
        .initialize_scene(scene_id.clone(), scene(&point))
        .await
        .unwrap();
    let current = restored.preview_scene(&scene_id).await.unwrap();
    assert_eq!(current.version, 2);
    assert_eq!(current.scope.targets[0].value, serde_json::json!(false));
    assert!(rx.try_recv().is_err());
    assert!(restored.scene_run(&run.id).is_none());
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[tokio::test]
async fn auto_s1_scope_change_during_fsync_never_expands_the_published_authority() {
    let (hub, _rx, id, point, scene_id, path) = durable().await;
    let reference = reference(scene_id.clone());
    hub.stage_dependency(
        reference.clone(),
        1,
        DependencyDefinition::Scene(scene(&point)),
        Origin::Ui,
        None,
    )
    .await
    .unwrap();
    let preview = hub.preview_dependency(&reference).await.unwrap();
    let entered = Arc::new(tokio::sync::Notify::new());
    let resume = Arc::new(tokio::sync::Notify::new());
    hub.0.dependencies.lock().await.file.pause_at(
        WritePhase::BeforeRename,
        entered.clone(),
        resume.clone(),
    );
    let other = hub.clone();
    let publish = tokio::spawn(async move {
        other
            .publish_dependency(preview, true, Origin::Ui, None)
            .await
    });
    entered.notified().await;
    let mut descriptor = (*hub.describe(&id).unwrap().0).clone();
    descriptor.native_room = Some("Changed synthetic room".into());
    hub.upsert_device(descriptor);
    resume.notify_one();
    publish.await.unwrap().unwrap();
    assert!(matches!(
        hub.preview_scene(&scene_id).await,
        Err(crate::SceneError::Conflict)
    ));
    assert!(hub.current_scene(&scene_id).is_none());
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn auto_s1_maximum_scope_is_bounded_without_idle_workers() {
    let (hub, _rx, id, point) = fixture();
    let base = (*hub.describe(&id).unwrap().0).clone();
    let mut spec = scene(&point);
    spec.targets.clear();
    for i in 0..32 {
        let mut root = base.clone();
        root.id = DeviceId::from(format!("fixture:root_{i}"));
        for j in 0..3 {
            let mut member = base.clone();
            member.id = DeviceId::from(format!("fixture:member_{i}_{j}"));
            root.members.push(member.id.clone());
            hub.upsert_device(member);
        }
        spec.targets.push(moli_core::SceneTarget {
            point: PointId::new(&root.id, "on"),
            value: serde_json::json!(true),
        });
        hub.upsert_device(root);
    }
    let started = Instant::now();
    let scope = hub.scene_scope(&spec).unwrap();
    let elapsed = started.elapsed();
    assert_eq!(
        scope.targets.iter().map(|t| t.devices.len()).sum::<usize>(),
        128
    );
    assert!(scope.valid_for(&spec));
    assert!(!hub.0.scenes_active.load(Ordering::Acquire));
    assert!(hub.0.scene_sends.get().is_none());
    eprintln!("AUTO-S1: 32 cibles / 128 appareils, prévalidation debug {elapsed:?}");
}
