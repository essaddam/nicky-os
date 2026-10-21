//! Counter cancellation contracts at the actual hub commit boundary.

use super::*;
use crate::driver::{Claim, CommandMetadata};
use crate::state_file::WritePhase;
use moli_core::{Access, CounterOperation, CounterSpec, Kind, PointSpec, Semantic};
use tokio_util::sync::CancellationToken;

fn spec() -> CounterSpec {
    CounterSpec {
        initial: 7,
        step: 2,
        min: 0,
        max: 99,
    }
}

async fn fixture() -> (Hub, Arc<Device>, PathBuf) {
    let path = std::env::temp_dir()
        .join(format!(
            "moli-h2-hub-{}",
            crate::occurrences::random_id().unwrap()
        ))
        .join("counters.json");
    let hub = Hub::new(HubOptions {
        counter_state_path: Some(path.clone()),
        ..Default::default()
    })
    .unwrap();
    let device = Arc::new(Device {
        id: "fixture:count".into(),
        instance: "fixture".into(),
        native_name: "Counter".into(),
        manufacturer: None,
        model: None,
        description: None,
        native_room: None,
        members: Vec::new(),
        points: vec![
            PointSpec {
                key: "value".into(),
                label: "Counter".into(),
                kind: Kind::Counter { spec: spec() },
                access: Access {
                    read: true,
                    write: false,
                },
                semantic: Semantic::Other,
                unit: None,
            },
            PointSpec {
                key: "increment".into(),
                label: "Increment".into(),
                kind: Kind::CounterCommand {
                    operation: CounterOperation::Increment,
                },
                access: Access {
                    read: false,
                    write: true,
                },
                semantic: Semantic::Other,
                unit: None,
            },
        ],
    });
    hub.upsert_device((*device).clone());
    hub.initialize_counter(&device.id, spec()).await.unwrap();
    (hub, device, path)
}

fn command(device: &Arc<Device>) -> (CommandRequest, oneshot::Receiver<Result<(), CommandError>>) {
    let (reply, receive) = oneshot::channel();
    (
        CommandRequest {
            point: PointId::new(&device.id, "increment"),
            device: device.clone(),
            key: "increment".into(),
            value: Value::Null,
            deadline: Instant::now() + Duration::from_secs(3600),
            metadata: CommandMetadata {
                origin: Origin::Ui,
                actor: None,
                request_id: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:1".parse().unwrap()),
                correlation: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:1".into()),
            },
            reply: Arc::new(std::sync::Mutex::new(Some(reply))),
            claim: Arc::new(AtomicU8::new(Claim::TAKEN)),
        },
        receive,
    )
}

#[derive(Debug)]
struct CounterDriver {
    taken: Arc<tokio::sync::Notify>,
}

impl crate::Driver for CounterDriver {
    fn kind(&self) -> &'static str {
        "simulated-counter"
    }

    fn run<'a>(
        &'a self,
        ctx: &'a mut crate::DriverCtx,
    ) -> crate::BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            ctx.ready();
            while let Some(command) = ctx.next_command().await {
                self.taken.notify_one();
                ctx.counter_command(command).await;
            }
            Ok(())
        })
    }
}

#[tokio::test]
async fn auto_h2_stopping_driver_before_commit_does_not_mutate_a_counter() {
    let (hub, device, path) = fixture().await;
    let taken = Arc::new(tokio::sync::Notify::new());
    let cancel = tokio_util::sync::CancellationToken::new();
    let driver = crate::spawn_driver(
        &hub,
        "fixture".into(),
        Arc::new(CounterDriver {
            taken: taken.clone(),
        }),
        cancel.clone(),
    );
    let lock = hub.0.counters.lock().await;
    let waiter = {
        let hub = hub.clone();
        let point = PointId::new(&device.id, "increment");
        tokio::spawn(async move { hub.command(&point, Value::Null, Origin::Ui, None).await })
    };
    tokio::time::timeout(Duration::from_secs(5), taken.notified())
        .await
        .unwrap();
    cancel.cancel();
    driver.await.unwrap();
    drop(lock);
    let _ = waiter.await.unwrap();
    assert_eq!(
        hub.state(&PointId::new(&device.id, "value")).unwrap().value,
        Value::Int(7),
        "a stopped driver must not start its queued commit"
    );
    tokio::fs::remove_dir_all(path.parent().unwrap())
        .await
        .unwrap();
}

#[tokio::test]
async fn auto_h2_abandonment_or_deadline_while_waiting_for_owner_never_commits() {
    for expire in [false, true] {
        let (hub, device, path) = fixture().await;
        let lock = hub.0.counters.lock().await;
        let (mut request, receive) = command(&device);
        if expire {
            request.deadline = Instant::now();
        }
        let waiter = {
            let hub = hub.clone();
            tokio::spawn(async move {
                hub.counter_command(request, CancellationToken::new()).await;
            })
        };
        if !expire {
            drop(receive);
        }
        drop(lock);
        waiter.await.unwrap();
        assert_eq!(
            hub.state(&PointId::new(&device.id, "value")).unwrap().value,
            Value::Int(7)
        );
        // The abandoned request did not reserve its identity either.
        let (request, receive) = command(&device);
        hub.counter_command(request, CancellationToken::new()).await;
        receive.await.unwrap().unwrap();
        assert_eq!(
            hub.state(&PointId::new(&device.id, "value")).unwrap().value,
            Value::Int(9)
        );
        tokio::fs::remove_dir_all(path.parent().unwrap())
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn auto_h2_lost_waiter_during_save_does_not_cancel_commit_or_publication() {
    let (hub, device, path) = fixture().await;
    let entered = Arc::new(tokio::sync::Notify::new());
    let resume = Arc::new(tokio::sync::Notify::new());
    hub.0.counters.lock().await.file.pause_at(
        WritePhase::BeforeWrite,
        entered.clone(),
        resume.clone(),
    );
    let (request, receive) = command(&device);
    let cancel = CancellationToken::new();
    let writer_cancel = cancel.clone();
    let waiter = {
        let hub = hub.clone();
        tokio::spawn(async move {
            hub.counter_command(request, writer_cancel).await;
        })
    };
    tokio::time::timeout(Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    cancel.cancel();
    waiter.abort();
    drop(receive);
    resume.notify_one();
    // Acquiring the owner waits for its independent task to finish the commit.
    let guard = hub.0.counters.lock().await;
    assert_eq!(
        hub.state(&PointId::new(&device.id, "value")).unwrap().value,
        Value::Int(9)
    );
    drop(guard);
    assert!(hub.journal(10).iter().any(
        |entry| matches!(entry.action, Action::Counter { .. }) && entry.outcome == Outcome::Ok
    ));
    let mut events = hub.subscribe();
    let (request, receive) = command(&device);
    hub.counter_command(request, CancellationToken::new()).await;
    receive.await.unwrap().unwrap();
    while let Ok(event) = events.try_recv() {
        assert!(!matches!(
            event,
            Event::State { .. } | Event::Occurrence { .. }
        ));
    }
    let mut restored = crate::counters::Counters::new(Some(path.clone()));
    assert_eq!(restored.initialize(&device.id, spec()).await.unwrap(), 9);
    tokio::fs::remove_dir_all(path.parent().unwrap())
        .await
        .unwrap();
}
