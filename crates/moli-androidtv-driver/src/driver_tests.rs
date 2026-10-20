use crate::pairing::Credential;
use crate::transport::{Abandoned, Connector, Event, Link, Wire};
use crate::{AndroidTv, Config};
use moli_androidtv::{cast, remote, tracked::Delivery};
use moli_core::{InstanceId, Origin, PointId, Value};
use moli_runtime::{BoxFuture, Hub, HubOptions, ScopedSecrets, spawn_driver};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{Notify, mpsc, watch};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct FakeWire {
    sent: Mutex<Vec<String>>,
    sent_signal: Notify,
    after_send: std::sync::atomic::AtomicBool,
    final_reply: Notify,
    level: std::sync::atomic::AtomicU64,
    refused_link: Mutex<Option<String>>,
    entered: Notify,
    release: Notify,
    block: std::sync::atomic::AtomicBool,
    events: Mutex<Option<mpsc::Sender<Event>>>,
}
struct FakeAdapter(Arc<FakeWire>);
impl Wire for FakeAdapter {
    fn remote(
        &self,
        order: remote::Order,
        deadline: Instant,
        abandoned: Abandoned,
    ) -> BoxFuture<'static, Delivery> {
        let fake = self.0.clone();
        Box::pin(async move {
            fake.entered.notify_one();
            if fake.block.load(std::sync::atomic::Ordering::SeqCst) {
                fake.release.notified().await;
            }
            if Instant::now() >= deadline {
                return Delivery::Expired;
            }
            if abandoned() {
                return Delivery::Abandoned;
            }
            fake.sent.lock().unwrap().push(format!("{order:?}"));
            fake.sent_signal.notify_one();
            if fake.after_send.load(std::sync::atomic::Ordering::SeqCst) {
                fake.final_reply.notified().await;
            }
            if let remote::Order::Launch(link) = &order {
                let refuse = fake.refused_link.lock().unwrap().as_ref() == Some(link);
                if refuse {
                    let events = fake.events.lock().unwrap().clone().unwrap();
                    events
                        .send(Event::Remote(remote::Event::Refused(link.clone())))
                        .await
                        .unwrap();
                }
            }
            if matches!(order, remote::Order::Key(24 | 25)) {
                let event = fake.events.lock().unwrap().clone().unwrap();
                event
                    .send(Event::Remote(remote::Event::Volume {
                        level: if matches!(order, remote::Order::Key(24)) {
                            fake.level.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1
                        } else {
                            fake.level.fetch_sub(1, std::sync::atomic::Ordering::SeqCst) - 1
                        },
                        max: 60,
                        muted: false,
                    }))
                    .await
                    .unwrap();
            }
            Delivery::Sent
        })
    }
    fn cast(
        &self,
        order: cast::Order,
        _deadline: Instant,
        abandoned: Abandoned,
    ) -> BoxFuture<'static, Delivery> {
        let fake = self.0.clone();
        Box::pin(async move {
            if abandoned() {
                return Delivery::Abandoned;
            }
            fake.sent.lock().unwrap().push(format!("Cast{order:?}"));
            Delivery::Acknowledged
        })
    }
}
struct FakeConnector(Arc<FakeWire>);
impl Connector for FakeConnector {
    fn open(
        &self,
        _host: String,
        _vault: ScopedSecrets,
        _credentials: watch::Receiver<Option<Arc<Credential>>>,
        _writes: crate::vault::Writes,
    ) -> Link {
        let (events, receiver) = mpsc::channel(32);
        *self.0.events.lock().unwrap() = Some(events.clone());
        Link {
            wire: Arc::new(FakeAdapter(self.0.clone())),
            events: receiver,
            run: Box::pin(async move {
                events
                    .send(Event::Cast(cast::Event::Playing(cast::Playing {
                        state: Some("playing".into()),
                        ..cast::Playing::default()
                    })))
                    .await
                    .unwrap();
                events
                    .send(Event::Remote(remote::Event::Ready))
                    .await
                    .unwrap();
                events
                    .send(Event::Remote(remote::Event::Volume {
                        level: 10,
                        max: 60,
                        muted: false,
                    }))
                    .await
                    .unwrap();
                std::future::pending::<()>().await;
            }),
        }
    }
}
struct Harness {
    hub: Hub,
    cancel: CancellationToken,
    wire: Arc<FakeWire>,
}
impl Harness {
    async fn new() -> Self {
        let hub = Hub::new(HubOptions::default()).unwrap();
        let cancel = CancellationToken::new();
        let wire = Arc::new(FakeWire {
            level: std::sync::atomic::AtomicU64::new(10),
            ..FakeWire::default()
        });
        let mut driver = AndroidTv::new(Config {
            host: "tv.invalid".into(),
        })
        .unwrap();
        driver.connector = Arc::new(FakeConnector(wire.clone()));
        spawn_driver(
            &hub,
            InstanceId::from("example"),
            Arc::new(driver),
            cancel.clone(),
        );
        // The public observation is the synchronization barrier, not a sleep.
        for _ in 0..200 {
            if hub
                .state(&point("volume"))
                .is_some_and(|s| s.value == Value::Int(10))
            {
                return Self { hub, cancel, wire };
            }
            tokio::task::yield_now().await;
        }
        panic!("driver did not publish the fake transport observation");
    }
    async fn send(&self, key: &str, value: Value) -> Result<(), moli_runtime::CommandError> {
        self.hub
            .command(&point(key), value, Origin::Api, None)
            .await
    }
    async fn event(&self, event: Event) {
        let tx = self.wire.events.lock().unwrap().clone().unwrap();
        tx.send(event).await.unwrap();
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
fn point(key: &str) -> PointId {
    PointId::from(format!("example:tv/{key}"))
}

#[tokio::test]
async fn true_hub_routes_remote_and_cast_through_the_same_fifo() {
    let h = Harness::new().await;
    h.send("power", Value::Bool(true)).await.unwrap();
    h.send("media_control", Value::from("pause")).await.unwrap();
    assert_eq!(*h.wire.sent.lock().unwrap(), ["Key(224)", "CastPause"]);
}
#[tokio::test]
async fn observed_volume_stops_at_target_and_matching_mute_emits_nothing() {
    let h = Harness::new().await;
    h.send("volume", Value::Int(11)).await.unwrap();
    h.send("mute", Value::Bool(false)).await.unwrap();
    assert_eq!(*h.wire.sent.lock().unwrap(), ["Key(24)"]);
}
#[tokio::test]
async fn remote_loss_makes_states_unknown_and_commands_fail_without_replay() {
    let h = Harness::new().await;
    h.event(Event::Remote(remote::Event::Lost)).await;
    assert_eq!(h.hub.state(&point("power")).unwrap().value, Value::Null);
    assert_eq!(h.hub.state(&point("volume")).unwrap().value, Value::Null);
    assert!(h.send("power", Value::Bool(true)).await.is_err());
    h.event(Event::Remote(remote::Event::Ready)).await;
    assert!(h.wire.sent.lock().unwrap().is_empty());
}
#[tokio::test]
async fn content_transmission_does_not_create_playing_metadata() {
    let h = Harness::new().await;
    h.send(
        "play_media",
        Value::from(r#"{"service":"netflix","kind":"episode","target":"12345678"}"#),
    )
    .await
    .unwrap();
    assert_eq!(h.wire.sent.lock().unwrap().len(), 1);
    assert_eq!(
        h.hub.state(&point("media_title")).unwrap().value,
        Value::Null
    );
}
#[tokio::test]
async fn abandoned_first_order_is_not_emitted_and_fifo_capacity_includes_it() {
    let h = Harness::new().await;
    h.wire
        .block
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let hub = h.hub.clone();
    let first = tokio::spawn(async move {
        hub.command(&point("key"), Value::from("Home"), Origin::Api, None)
            .await
    });
    h.wire.entered.notified().await;
    let mut waiting = Vec::new();
    for key in ["Back", "Confirm", "CursorDown"] {
        let hub = h.hub.clone();
        waiting.push(tokio::spawn(async move {
            hub.command(&point("key"), Value::from(key), Origin::Api, None)
                .await
        }));
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
    }
    let excess = tokio::time::timeout(
        Duration::from_millis(200),
        h.send("key", Value::from("Info")),
    )
    .await
    .expect("full queue must refuse immediately");
    assert!(excess.is_err());
    first.abort();
    let _ = first.await;
    h.wire
        .block
        .store(false, std::sync::atomic::Ordering::SeqCst);
    h.wire.release.notify_one();
    for request in waiting {
        request.await.unwrap().unwrap();
    }
    assert_eq!(
        *h.wire.sent.lock().unwrap(),
        ["Key(4)", "Key(23)", "Key(20)"]
    );
}

#[tokio::test]
async fn volume_uses_each_observation_and_stops_after_exactly_two_steps() {
    let h = Harness::new().await;
    h.send("volume", Value::Int(12)).await.unwrap();
    assert_eq!(*h.wire.sent.lock().unwrap(), ["Key(24)", "Key(24)"]);
    assert_eq!(h.hub.state(&point("volume")).unwrap().value, Value::Int(12));
}
#[tokio::test]
async fn precise_refused_episode_never_falls_back_even_when_app_already_open() {
    let h = Harness::new().await;
    h.event(Event::Remote(remote::Event::App(
        "com.netflix.ninja".into(),
    )))
    .await;
    *h.wire.refused_link.lock().unwrap() =
        Some("https://www.netflix.com/title/12345678?source=30".into());
    assert!(
        h.send(
            "play_media",
            Value::from(r#"{"service":"netflix","kind":"episode","target":"12345678"}"#)
        )
        .await
        .is_err()
    );
    assert_eq!(h.wire.sent.lock().unwrap().len(), 1);
    assert_eq!(
        h.hub.state(&point("media_title")).unwrap().value,
        Value::Null
    );
}
#[tokio::test]
async fn a_generic_app_uses_its_second_link_only_after_exact_refusal() {
    let h = Harness::new().await;
    let links = remote::app_links("com.netflix.ninja");
    assert_eq!(links.len(), 2);
    *h.wire.refused_link.lock().unwrap() = Some(links[0].clone());
    h.send("app", Value::from("netflix")).await.unwrap();
    assert_eq!(
        *h.wire.sent.lock().unwrap(),
        links
            .iter()
            .map(|link| format!("{:?}", remote::Order::Launch(link.clone())))
            .collect::<Vec<_>>()
    );
}
#[tokio::test]
async fn caller_leaving_after_emission_keeps_the_actual_result_in_the_hub_journal() {
    let h = Harness::new().await;
    h.wire
        .after_send
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let hub = h.hub.clone();
    let caller = tokio::spawn(async move {
        hub.command(&point("power"), Value::Bool(true), Origin::Api, None)
            .await
    });
    h.wire.sent_signal.notified().await;
    caller.abort();
    let _ = caller.await;
    h.wire.final_reply.notify_one();
    for _ in 0..200 {
        if h.hub
            .journal(10)
            .iter()
            .any(|entry| entry.outcome == moli_core::Outcome::Ok)
        {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("the emitted order lost its successful result");
}

#[tokio::test]
async fn queued_cast_during_loss_cannot_move_to_the_next_connection() {
    queued_cast_across_loss(false).await;
}

#[tokio::test]
async fn queued_cast_from_a_live_connection_is_invalidated_by_loss() {
    queued_cast_across_loss(true).await;
}

async fn queued_cast_across_loss(initially_available: bool) {
    let h = Harness::new().await;
    if initially_available {
        h.event(Event::Cast(cast::Event::Playing(cast::Playing {
            state: Some("playing".into()),
            ..cast::Playing::default()
        })))
        .await;
    } else {
        h.event(Event::Cast(cast::Event::Lost)).await;
    }
    h.wire
        .block
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let hub = h.hub.clone();
    let remote = tokio::spawn(async move {
        hub.command(&point("power"), Value::Bool(true), Origin::Api, None)
            .await
    });
    h.wire.entered.notified().await;
    let hub = h.hub.clone();
    let pause = tokio::spawn(async move {
        hub.command(
            &point("media_control"),
            Value::from("pause"),
            Origin::Api,
            None,
        )
        .await
    });
    // Let the hub deliver the command while the Remote order is retained.
    for _ in 0..30 {
        tokio::task::yield_now().await;
    }
    h.event(Event::Cast(cast::Event::Lost)).await;
    h.event(Event::Cast(cast::Event::Playing(cast::Playing {
        state: Some("playing".into()),
        ..cast::Playing::default()
    })))
    .await;
    h.wire
        .block
        .store(false, std::sync::atomic::Ordering::SeqCst);
    h.wire.release.notify_one();
    remote.await.unwrap().unwrap();
    assert!(
        pause.await.unwrap().is_err(),
        "old Pause reached a future Cast connection"
    );
    assert_eq!(*h.wire.sent.lock().unwrap(), ["Key(224)"]);
    // A new command admitted to the healthy connection still works.
    h.send("media_control", Value::from("pause")).await.unwrap();
    assert_eq!(*h.wire.sent.lock().unwrap(), ["Key(224)", "CastPause"]);
}

struct CaptureCommand(mpsc::Sender<moli_runtime::CommandRequest>);
impl moli_runtime::Driver for CaptureCommand {
    fn kind(&self) -> &'static str {
        "fake"
    }
    fn run<'a>(
        &'a self,
        ctx: &'a mut moli_runtime::DriverCtx,
    ) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            ctx.upsert_device(crate::model::device(ctx.instance(), Some(60)));
            ctx.set_availability(&ctx.device_id("tv"), true);
            ctx.ready();
            while let Some(command) = ctx.next_command().await {
                self.0.send(command).await.unwrap();
            }
            Ok(())
        })
    }
}

#[tokio::test]
async fn a_connection_change_before_the_first_job_poll_cannot_rebind_an_order() {
    for cast_order in [false, true] {
        let hub = Hub::new(HubOptions::default()).unwrap();
        let cancel = CancellationToken::new();
        let (tx, mut rx) = mpsc::channel(1);
        let driver = spawn_driver(
            &hub,
            InstanceId::from("example"),
            Arc::new(CaptureCommand(tx)),
            cancel.clone(),
        );
        for _ in 0..100 {
            if hub.state(&point("power")).is_some() || !hub.snapshot().devices.is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
        let caller = tokio::spawn(async move {
            let (key, value) = if cast_order {
                ("media_control", Value::from("pause"))
            } else {
                ("power", Value::Bool(true))
            };
            hub.command(&point(key), value, Origin::Api, None).await
        });
        let command = rx.recv().await.unwrap();
        let initial = crate::model::Observation {
            online: true,
            playing: cast::Playing {
                state: Some("playing".into()),
                ..cast::Playing::default()
            },
            ..crate::model::Observation::default()
        };
        let (states, observations) = watch::channel(initial.clone());
        let fake = Arc::new(FakeWire::default());
        let wire: Arc<dyn Wire> = Arc::new(FakeAdapter(fake.clone()));
        let plan = crate::commands::plan(&command.key, &command.value, &initial).unwrap();
        let job = crate::driver::execute(wire, observations, command, plan);
        // The future exists but has never been polled. Simulate loss and recovery.
        let mut recovered = initial;
        if cast_order {
            recovered.cast(cast::Event::Lost);
            recovered.cast(cast::Event::Playing(cast::Playing {
                state: Some("playing".into()),
                ..cast::Playing::default()
            }));
        } else {
            recovered.remote(remote::Event::Lost);
            recovered.remote(remote::Event::Ready);
        }
        states.send_replace(recovered);
        job.await;
        assert!(
            caller.await.unwrap().is_err(),
            "a not-yet-polled job rebound its session"
        );
        assert!(fake.sent.lock().unwrap().is_empty());
        cancel.cancel();
        driver.await.unwrap();
    }
}
