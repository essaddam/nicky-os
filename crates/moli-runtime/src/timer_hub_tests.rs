use super::*;
use crate::HubOptions;
use crate::driver::Claim;
use crate::driver::CommandMetadata;
use crate::state_file::WritePhase;
use moli_core::{Access, Device, Kind, PointSpec, Semantic};
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8},
};
use tokio::sync::oneshot;

struct Fixture {
    hub: Hub,
    device: Arc<Device>,
    path: PathBuf,
}

async fn fixture() -> Fixture {
    let path = std::env::temp_dir().join(format!(
        "moli-timer-hub-{}",
        crate::occurrences::random_id().unwrap()
    ));
    let hub = Hub::new(HubOptions {
        timer_state_path: Some(path.join("timers.json")),
        state_path: Some(path.join("state.json")),
        ..Default::default()
    })
    .unwrap();
    let mut points: Vec<_> = [
        ("state", TimerField::State),
        ("remaining", TimerField::Remaining),
        ("generation", TimerField::Generation),
        ("deadline", TimerField::Deadline),
        ("finished", TimerField::Expired),
    ]
    .into_iter()
    .map(|(key, field)| PointSpec {
        key: key.into(),
        label: key.into(),
        kind: Kind::Timer { field },
        access: Access {
            read: field != TimerField::Expired,
            write: false,
        },
        unit: None,
        semantic: Semantic::Other,
    })
    .collect();
    for (key, operation) in [
        ("start", TimerOperation::Start),
        ("cancel", TimerOperation::Cancel),
    ] {
        points.push(PointSpec {
            key: key.into(),
            label: key.into(),
            kind: Kind::TimerCommand {
                operation,
                default_seconds: 5,
            },
            access: Access {
                read: false,
                write: true,
            },
            unit: None,
            semantic: Semantic::Other,
        });
    }
    let device = Arc::new(Device {
        id: "fixture:clock".into(),
        instance: "fixture".into(),
        native_name: "Timer".into(),
        manufacturer: None,
        model: None,
        description: None,
        native_room: None,
        members: Vec::new(),
        points,
    });
    hub.upsert_device((*device).clone());
    hub.initialize_timer(&device.id, 5).await.unwrap();
    Fixture { hub, device, path }
}

fn command(
    device: &Arc<Device>,
    key: &str,
    value: Value,
    sequence: u64,
) -> (CommandRequest, oneshot::Receiver<Result<(), CommandError>>) {
    let (reply, receive) = oneshot::channel();
    (
        CommandRequest {
            point: PointId::new(&device.id, key),
            device: device.clone(),
            key: key.into(),
            value,
            deadline: Instant::now() + Duration::from_secs(60),
            metadata: CommandMetadata {
                origin: Origin::Ui,
                actor: None,
                request_id: Some(
                    format!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:{sequence}")
                        .parse()
                        .unwrap(),
                ),
                correlation: Some(format!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:{sequence}")),
            },
            reply,
            claim: Arc::new(AtomicU8::new(Claim::TAKEN)),
        },
        receive,
    )
}

#[tokio::test]
async fn auto_h1_abandoned_waiter_during_save_keeps_commit_publication_and_original_deadline() {
    let f = fixture().await;
    let entered = Arc::new(tokio::sync::Notify::new());
    let resume = Arc::new(tokio::sync::Notify::new());
    f.hub.0.timers.lock().await.file.pause_at(
        WritePhase::BeforeWrite,
        entered.clone(),
        resume.clone(),
    );
    let (request, receive) = command(&f.device, "start", Value::Int(1), 1);
    let cancel = CancellationToken::new();
    let writer_cancel = cancel.clone();
    let waiting = {
        let h = f.hub.clone();
        tokio::spawn(async move {
            h.timer_command(request, writer_cancel).await;
        })
    };
    tokio::time::timeout(Duration::from_secs(3), entered.notified())
        .await
        .unwrap();
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(5)).await;
    cancel.cancel();
    waiting.abort();
    drop(receive);
    resume.notify_one();
    let owner = f.hub.0.timers.lock().await;
    assert_eq!(owner.projection(&f.device.id).unwrap().remaining, Some(0));
    assert_eq!(
        f.hub
            .state(&PointId::new(&f.device.id, "generation"))
            .unwrap()
            .value,
        Value::Int(1)
    );
    assert!(
        f.hub
            .journal(20)
            .iter()
            .any(|entry| matches!(entry.action, Action::Timer { .. })
                && entry.outcome == Outcome::Ok)
    );
    drop(owner);
    tokio::fs::remove_dir_all(f.path).await.unwrap();
}

#[tokio::test]
async fn auto_h1_final_admission_waits_for_rename_and_revocation_under_the_same_owner() {
    let f = fixture().await;
    let (request, receive) = command(&f.device, "start", Value::Int(1), 1);
    f.hub.timer_command(request, CancellationToken::new()).await;
    receive.await.unwrap().unwrap();
    tokio::time::sleep(Duration::from_millis(1010)).await;
    f.hub.tick_timers().await.unwrap();
    let event = f
        .hub
        .0
        .timers
        .lock()
        .await
        .events(&f.hub.0.occurrence_stream)
        .unwrap()
        .remove(0);
    let entered = Arc::new(tokio::sync::Notify::new());
    let resume = Arc::new(tokio::sync::Notify::new());
    f.hub.0.timers.lock().await.file.pause_at(
        WritePhase::AfterRename,
        entered.clone(),
        resume.clone(),
    );
    let (request, receive) = command(&f.device, "cancel", Value::Null, 2);
    let cancel = {
        let h = f.hub.clone();
        tokio::spawn(async move {
            h.timer_command(request, CancellationToken::new()).await;
        })
    };
    tokio::time::timeout(Duration::from_secs(3), entered.notified())
        .await
        .unwrap();
    let launched = Arc::new(AtomicBool::new(false));
    let admission = {
        let h = f.hub.clone();
        let launched = launched.clone();
        tokio::spawn(async move {
            h.with_timer_generation(&event, |_| launched.store(true, Ordering::SeqCst))
                .await
        })
    };
    tokio::task::yield_now().await;
    assert!(!admission.is_finished());
    resume.notify_one();
    cancel.await.unwrap();
    receive.await.unwrap().unwrap();
    assert_eq!(admission.await.unwrap(), Err(TimerError::Stale));
    assert!(!launched.load(Ordering::SeqCst));
    tokio::fs::remove_dir_all(f.path).await.unwrap();
}

#[tokio::test]
async fn auto_h1_timer_projections_do_not_dirty_or_enter_the_periodic_cache() {
    let f = fixture().await;
    let (request, receive) = command(&f.device, "start", Value::Int(5), 1);
    f.hub.timer_command(request, CancellationToken::new()).await;
    receive.await.unwrap().unwrap();
    assert!(!f.hub.persist_state().await.unwrap());
    let mut ordinary = (*f.device).clone();
    ordinary.id = "fixture:ordinary".into();
    ordinary.points = vec![PointSpec {
        key: "on".into(),
        label: "On".into(),
        kind: Kind::Binary,
        access: Access {
            read: true,
            write: true,
        },
        unit: None,
        semantic: Semantic::OnOff,
    }];
    f.hub.upsert_device(ordinary.clone());
    f.hub.set_state(&ordinary.id, "on", Value::Bool(true));
    assert!(f.hub.persist_state().await.unwrap());
    let cache: serde_json::Value =
        serde_json::from_slice(&tokio::fs::read(f.path.join("state.json")).await.unwrap()).unwrap();
    assert!(cache.get("fixture:clock").is_none());
    assert!(cache.get("fixture:ordinary").is_some());
    tokio::fs::remove_dir_all(f.path).await.unwrap();
}
