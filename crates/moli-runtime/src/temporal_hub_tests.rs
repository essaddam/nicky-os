use super::*;
use crate::HubOptions;
use crate::driver::{Claim, CommandMetadata};
use crate::state_file::WritePhase;
use moli_core::{Access, Device, PointSpec, Semantic};
use std::sync::{Arc, atomic::AtomicU8};
use std::time::Duration;
use tokio::sync::{Notify, oneshot};
fn spec() -> TemporalSpec {
    TemporalSpec {
        format: moli_core::TemporalFormat::Date,
        timezone: None,
    }
}
fn device() -> Arc<Device> {
    Arc::new(Device {
        id: "fixture:sample".into(),
        instance: "fixture".into(),
        native_name: "Synthetic date".into(),
        manufacturer: None,
        model: None,
        description: None,
        native_room: None,
        members: Vec::new(),
        points: vec![PointSpec {
            key: "value".into(),
            label: "Date".into(),
            kind: Kind::Temporal { spec: spec() },
            unit: None,
            semantic: Semantic::Other,
            access: Access {
                read: true,
                write: true,
            },
        }],
    })
}

#[tokio::test]
async fn auto_h3_a_started_commit_survives_its_waiter_and_never_uses_the_periodic_cache() {
    let path = std::env::temp_dir().join(format!(
        "moli-h3-hub-{}",
        crate::occurrences::random_id().unwrap()
    ));
    let hub = Hub::new(HubOptions {
        temporal_state_path: Some(path.join("dates.json")),
        state_path: Some(path.join("state.json")),
        ..Default::default()
    })
    .unwrap();
    let device = device();
    hub.upsert_device((*device).clone());
    hub.initialize_temporal(&device.id, spec(), Value::Text("2024-02-29".into()))
        .await
        .unwrap();
    let entered = Arc::new(Notify::new());
    let resume = Arc::new(Notify::new());
    hub.0.temporals.lock().await.file.pause_at(
        WritePhase::BeforeWrite,
        entered.clone(),
        resume.clone(),
    );
    let (reply, answer) = oneshot::channel();
    let cancel = CancellationToken::new();
    let request = CommandRequest {
        point: PointId::new(&device.id, "value"),
        device: device.clone(),
        key: "value".into(),
        value: Value::Text("2026-01-01".into()),
        deadline: Instant::now() + Duration::from_secs(60),
        metadata: CommandMetadata {
            origin: moli_core::Origin::Ui,
            actor: None,
            request_id: None,
            correlation: None,
        },
        permit: None,
        reply: Arc::new(std::sync::Mutex::new(Some(reply))),
        claim: Arc::new(AtomicU8::new(Claim::TAKEN)),
    };
    let waiting = {
        let h = hub.clone();
        let c = cancel.clone();
        tokio::spawn(async move {
            h.temporal_command(request, c).await;
        })
    };
    tokio::time::timeout(Duration::from_secs(3), entered.notified())
        .await
        .unwrap();
    assert_eq!(
        hub.state(&PointId::new(&device.id, "value")).unwrap().value,
        Value::Text("2024-02-29".into())
    );
    cancel.cancel();
    waiting.abort();
    drop(answer);
    resume.notify_one();
    let owner = hub.0.temporals.lock().await;
    assert_eq!(
        owner.value(&device.id),
        Some(Value::Text("2026-01-01".into()))
    );
    assert_eq!(
        hub.state(&PointId::new(&device.id, "value")).unwrap().value,
        Value::Text("2026-01-01".into())
    );
    assert!(
        hub.journal(20)
            .iter()
            .any(|e| matches!(e.action, Action::Temporal { .. }) && e.outcome == Outcome::Ok)
    );
    drop(owner);
    assert!(!hub.persist_state().await.unwrap());
    let mut restored = crate::temporal::Temporals::new(Some(path.join("dates.json")));
    assert_eq!(
        restored
            .initialize(&device.id, spec(), Value::Text("2000-01-01".into()))
            .await
            .unwrap(),
        Value::Text("2026-01-01".into())
    );
    tokio::fs::remove_dir_all(path).await.unwrap();
}
