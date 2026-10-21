//! Real composite driver and hub; only the last transport is simulated.
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use moli_core::{Access, Action, Device, Kind, Origin, Outcome, PointSpec, Value};
use moli_runtime::{BoxFuture, CommandPermit, Driver, DriverCtx, Hub, HubOptions, spawn_driver};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Default)]
struct Transport {
    taken: Arc<Notify>,
    release: Arc<Notify>,
    emitted: Arc<AtomicUsize>,
    after_emission: bool,
}

struct Simulated {
    transport: Option<Transport>,
}
impl Driver for Simulated {
    fn kind(&self) -> &'static str {
        "fake"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let id = ctx.device_id("bulb");
            ctx.upsert_device(Device {
                id: id.clone(),
                instance: ctx.instance().clone(),
                native_name: "Synthetic light".into(),
                manufacturer: None,
                model: None,
                description: None,
                native_room: None,
                members: Vec::new(),
                points: vec![PointSpec {
                    key: "on".into(),
                    label: "on".into(),
                    kind: Kind::Binary,
                    access: Access {
                        read: true,
                        write: true,
                    },
                    unit: None,
                    semantic: moli_core::Semantic::OnOff,
                }],
            });
            ctx.set_state(&id, "on", Value::Bool(false));
            ctx.ready();
            while let Some(command) = ctx.next_command().await {
                if let Some(transport) = &self.transport {
                    if transport.after_emission {
                        assert!(!command.abandoned());
                        transport.emitted.fetch_add(1, Ordering::Release);
                    }
                    transport.taken.notify_one();
                    transport.release.notified().await;
                    if !transport.after_emission {
                        if command.abandoned() {
                            command.reply(Err("synthetic permission revoked".into()));
                            continue;
                        }
                        transport.emitted.fetch_add(1, Ordering::Release);
                    }
                }
                ctx.set_state(&id, "on", command.value.clone());
                command.reply(Ok(()));
            }
            Ok(())
        })
    }
}

#[derive(Clone, Copy)]
enum Path {
    Direct,
    Bulb,
    Relay,
    Deferred,
    AlreadySent,
}
struct Home {
    hub: Hub,
    cancel: CancellationToken,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    transport: Transport,
    point: &'static str,
}
impl Home {
    async fn new(path: Path) -> Self {
        let hub = Hub::new(HubOptions::default()).unwrap();
        let cancel = CancellationToken::new();
        let transport = Transport {
            after_emission: matches!(path, Path::AlreadySent),
            ..Transport::default()
        };
        let mut tasks = vec![spawn_driver(
            &hub,
            "physical".into(),
            Arc::new(Simulated {
                transport: Some(transport.clone()),
            }),
            cancel.clone(),
        )];
        wait_for(|| hub.device(&"physical:bulb".into()).is_some()).await;
        let power = if matches!(path, Path::Deferred) {
            tasks.push(spawn_driver(
                &hub,
                "relay".into(),
                Arc::new(Simulated { transport: None }),
                cancel.clone(),
            ));
            wait_for(|| hub.device(&"relay:bulb".into()).is_some()).await;
            Some("relay:bulb/on".to_owned())
        } else if matches!(path, Path::Relay) {
            Some("physical:bulb/on".to_owned())
        } else {
            None
        };
        let bulbs = if matches!(path, Path::Relay) {
            Vec::new()
        } else {
            vec!["physical:bulb".into()]
        };
        tasks.push(spawn_driver(
            &hub,
            "fixtures".into(),
            Arc::new(moli_lights::Fixtures::new(
                hub.clone(),
                vec![moli_lights::Fixture {
                    id: "light".into(),
                    name: "Synthetic fixture".into(),
                    room: Some("Synthetic room".into()),
                    bulbs,
                    power,
                }],
            )),
            cancel.clone(),
        ));
        wait_for(|| hub.device(&"fixtures:light".into()).is_some()).await;
        let point = if matches!(path, Path::Direct) {
            "physical:bulb/on"
        } else {
            "fixtures:light/on"
        };
        Self {
            hub,
            cancel,
            tasks,
            transport,
            point,
        }
    }
    fn order(
        &self,
        clock: Arc<AtomicU64>,
        denied: Arc<Notify>,
    ) -> tokio::task::JoinHandle<Result<(), moli_runtime::CommandError>> {
        let hub = self.hub.clone();
        let parent = self.point;
        tokio::spawn(async move {
            hub.command_with_permit(
                &parent.into(),
                Value::Bool(true),
                Origin::Ui,
                None,
                None,
                CommandPermit::new(move |point| {
                    let allowed = point.as_str() == parent
                        && moli_core::AccessWindow::new(1000, 2000)
                            .unwrap()
                            .allows(clock.load(Ordering::Acquire));
                    if !allowed {
                        denied.notify_one();
                    }
                    allowed
                }),
            )
            .await
        })
    }
    async fn stop(self) {
        self.cancel.cancel();
        for task in self.tasks {
            task.await.unwrap();
        }
    }
}
async fn wait_for(check: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !check() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
async fn notified(event: &Notify) {
    tokio::time::timeout(Duration::from_secs(3), event.notified())
        .await
        .unwrap();
}

async fn rejected_before_emission(path: Path) {
    let home = Home::new(path).await;
    let clock = Arc::new(AtomicU64::new(1000));
    let request = home.order(clock.clone(), Arc::new(Notify::new()));
    notified(&home.transport.taken).await;
    clock.store(2000, Ordering::Release);
    home.transport.release.notify_one();
    assert!(request.await.unwrap().is_err());
    assert_eq!(home.transport.emitted.load(Ordering::Acquire), 0);
    home.stop().await;
}
#[tokio::test]
async fn direct_order_refuses_the_expired_window() {
    rejected_before_emission(Path::Direct).await;
}
#[tokio::test]
async fn fixture_bulb_taken_before_expiry_never_leaves_after_it() {
    rejected_before_emission(Path::Bulb).await;
}
#[tokio::test]
async fn fixture_relay_taken_before_expiry_never_leaves_after_it() {
    rejected_before_emission(Path::Relay).await;
}

#[tokio::test]
async fn restoring_power_does_not_lose_permission_after_parent_reply() {
    let home = Home::new(Path::Deferred).await;
    let clock = Arc::new(AtomicU64::new(1000));
    let denied = Arc::new(Notify::new());
    assert!(
        home.order(clock.clone(), denied.clone())
            .await
            .unwrap()
            .is_ok()
    );
    clock.store(2000, Ordering::Release);
    tokio::select! {
        ()=notified(&denied)=>{},
        ()=home.transport.taken.notified()=>panic!("a deferred child was queued after revocation"),
    }
    assert_eq!(home.transport.emitted.load(Ordering::Acquire), 0);
    home.stop().await;
}

#[tokio::test]
async fn authorized_deferred_setting_still_runs_after_parent_reply() {
    let home = Home::new(Path::Deferred).await;
    assert!(
        home.order(Arc::new(AtomicU64::new(1000)), Arc::new(Notify::new()))
            .await
            .unwrap()
            .is_ok()
    );
    notified(&home.transport.taken).await;
    home.transport.release.notify_one();
    wait_for(|| home.transport.emitted.load(Ordering::Acquire) == 1).await;
    home.stop().await;
}

#[tokio::test]
async fn emitted_child_keeps_its_success_and_journal_after_revocation_and_caller_departure() {
    let home = Home::new(Path::AlreadySent).await;
    let clock = Arc::new(AtomicU64::new(1000));
    let request = home.order(clock.clone(), Arc::new(Notify::new()));
    notified(&home.transport.taken).await;
    clock.store(2000, Ordering::Release);
    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());
    home.transport.release.notify_one();
    wait_for(|| home.hub.journal(100).len() == 2).await;
    let journal = home.hub.journal(100);
    assert!(
        journal
            .iter()
            .all(|entry| matches!(entry.outcome, Outcome::Ok))
    );
    assert_eq!(journal.iter().filter(|entry|matches!(&entry.action,Action::Command {point,..} if point.as_str()=="physical:bulb/on")).count(),1);
    assert_eq!(home.transport.emitted.load(Ordering::Acquire), 1);
    home.stop().await;
}

type WindowUpdate = (
    Vec<(String, moli_core::AccessWindow)>,
    tokio::sync::oneshot::Sender<()>,
);
struct Calendar {
    updates: std::sync::Mutex<Option<tokio::sync::mpsc::Receiver<WindowUpdate>>>,
}
impl Driver for Calendar {
    fn kind(&self) -> &'static str {
        "reservations"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let mut updates = self.updates.lock().unwrap().take().unwrap();
            ctx.ready();
            while let Some((windows, done)) = updates.recv().await {
                assert!(ctx.replace_reservations(windows, moli_core::now_ms() + 60_000));
                let _ = done.send(());
            }
            ctx.cancelled().await;
            Ok(())
        })
    }
}
async fn windows(
    tx: &tokio::sync::mpsc::Sender<WindowUpdate>,
    value: Vec<(String, moli_core::AccessWindow)>,
) {
    let (done, wait) = tokio::sync::oneshot::channel();
    tx.send((value, done)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), wait)
        .await
        .unwrap()
        .unwrap();
}
#[tokio::test]
async fn cancelling_the_real_calendar_snapshot_revokes_a_taken_fixture_child() {
    let mut home = Home::new(Path::Bulb).await;
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    home.tasks.push(spawn_driver(
        &home.hub,
        "calendar".into(),
        Arc::new(Calendar {
            updates: std::sync::Mutex::new(Some(rx)),
        }),
        home.cancel.clone(),
    ));
    let id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let now = moli_core::now_ms();
    windows(
        &tx,
        vec![(
            id.into(),
            moli_core::AccessWindow::new(now - 1, now + 60_000).unwrap(),
        )],
    )
    .await;
    let binding = moli_runtime::ReservationRef {
        instance: "calendar".into(),
        id: id.into(),
    };
    let hub = home.hub.clone();
    let admission = hub.clone();
    let parent = home.point;
    let request = tokio::spawn(async move {
        hub.command_with_permit(
            &parent.into(),
            Value::Bool(true),
            Origin::Ui,
            None,
            None,
            CommandPermit::new(move |point| {
                point.as_str() == parent && admission.reservation_allows(&binding)
            }),
        )
        .await
    });
    notified(&home.transport.taken).await;
    windows(&tx, Vec::new()).await;
    home.transport.release.notify_one();
    assert!(request.await.unwrap().is_err());
    assert_eq!(home.transport.emitted.load(Ordering::Acquire), 0);
    home.stop().await;
}
