//! Scenes and calculations share definitions and observations, not authority.
use super::*;
use crate::{SceneStatus, TargetStatus};
use moli_core::{AggregateOp, AggregateSpec, DependencyKind, DependencyRef, Kind};

async fn fixture() -> (
    Hub,
    mpsc::Receiver<CommandRequest>,
    DeviceId,
    PointId,
    DependencyRef,
    DeviceId,
) {
    let (hub, rx, id, point) = super::scene_tests::fixture();
    let mut source = (*hub.describe(&id).unwrap().0).clone();
    source.points[0].kind = Kind::Numeric {
        min: Some(0.0),
        max: Some(100.0),
        step: None,
    };
    hub.upsert_device(source.clone());
    hub.set_state(&id, "on", Value::Int(0));
    let computed = DeviceId::from("fixture:mean");
    source.id = computed.clone();
    source.points[0].key = "value".into();
    source.points[0].kind = Kind::Computed;
    source.points[0].access.write = false;
    hub.upsert_device(source);
    hub.initialize_computed(
        &computed,
        AggregateSpec {
            operation: AggregateOp::Mean,
            sources: vec![point.clone()],
            unit: None,
            max_age_s: 60,
        },
    )
    .await
    .unwrap();
    let reference = DependencyRef {
        kind: DependencyKind::Computed,
        id: computed,
    };
    hub.publish_dependency(
        hub.preview_dependency(&reference).await.unwrap(),
        true,
        Origin::Ui,
        None,
    )
    .await
    .unwrap();
    let mut spec = super::scene_tests::scene(&point);
    spec.targets[0].value = serde_json::json!(42);
    let scene = super::scene_tests::publish(&hub, spec).await;
    (hub, rx, id, point, reference, scene)
}

#[tokio::test]
async fn auto_s1_one_numeric_observation_confirms_the_scene_and_refreshes_the_calculation() {
    let (hub, mut rx, source, _, computed, scene) = fixture().await;
    assert!(
        hub.preview_dependency(&computed)
            .await
            .unwrap()
            .scene
            .is_none()
    );
    let scene_ref = DependencyRef {
        kind: DependencyKind::Scene,
        id: scene.clone(),
    };
    assert_eq!(scene_ref.point(), None);
    assert!(hub.describe(&scene).is_none());
    let run = super::scene_tests::start(&hub, &scene).await;
    let command = rx.recv().await.unwrap();
    assert!(!command.abandoned());
    hub.set_state(&source, "on", Value::Int(42));
    command.reply(Ok(()));
    hub.run_computations().unwrap();
    let ended = hub.wait_scene(&run.id).await.unwrap();
    assert_eq!(ended.status, SceneStatus::Done);
    assert_eq!(ended.targets[0].status, TargetStatus::Confirmed);
    assert_eq!(
        hub.state(&computed.point().unwrap()).unwrap().value,
        Value::Float(42.0)
    );
    assert!(hub.dependency_ready(&computed));
    assert!(hub.dependency_ready(&scene_ref));
}

#[tokio::test]
async fn auto_s1_a_computation_fence_preserves_scene_authority_but_a_global_fence_revokes_it() {
    let (hub, mut rx, _, _, computed, scene) = fixture().await;
    let run = super::scene_tests::start(&hub, &scene).await;
    let command = rx.recv().await.unwrap();
    hub.fence_dependencies(Some(DependencyKind::Computed));
    assert!(!hub.dependency_ready(&computed));
    assert_eq!(
        hub.state(&computed.point().unwrap()).unwrap().value,
        Value::Null
    );
    assert!(hub.current_scene(&scene).is_some());
    assert!(!command.abandoned());
    hub.dependency_fence();
    assert!(command.abandoned());
    assert!(hub.current_scene(&scene).is_none());
    assert!(hub.describe(&scene).is_none());
    command.reply(Err("definitions fenced".into()));
    assert_eq!(
        hub.wait_scene(&run.id).await.unwrap().status,
        SceneStatus::Cancelled
    );
}
