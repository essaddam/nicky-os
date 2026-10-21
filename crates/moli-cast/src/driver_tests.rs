use super::*;
use crate::discovery::Candidate;
use crate::gateway::{Approved, Discover, Error, Phase};
use crate::transport::{Connector, Event, Guard, Link, Wire};
use moli_androidtv::{cast, tracked::Delivery};
use moli_core::{InstanceId, Origin, PointId, Value};
use moli_runtime::{Hub, HubOptions, MasterKey, spawn_driver};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::sync::{Notify, mpsc};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
#[derive(Default)]
struct Fake {
    opens: AtomicUsize,
    queries: AtomicUsize,
    sent: Mutex<Vec<String>>,
    events: Mutex<Option<mpsc::Sender<Event>>>,
    block: AtomicBool,
    after: AtomicBool,
    entered: Notify,
    emitted: Notify,
    release: Notify,
}
fn candidate() -> Candidate {
    Candidate {
        id: "00000000000000000000000000000001".into(),
        name: "Example screen".into(),
        host: "192.168.1.10".parse().unwrap(),
        hostname: "screen.local".into(),
        video: true,
    }
}
impl Discover for Fake {
    fn discover(&self, _: Config) -> BoxFuture<'static, Result<Vec<Candidate>, Error>> {
        self.queries.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(vec![candidate()]) })
    }
}
struct Adapter(Arc<Fake>);
impl Wire for Adapter {
    fn bind(&self, _: bool) -> Option<Guard> {
        Some(Arc::new(|| false))
    }
    fn send(
        &self,
        order: cast::ReceiverOrder,
        deadline: Instant,
        abandoned: Guard,
    ) -> BoxFuture<'static, Delivery> {
        let fake = self.0.clone();
        Box::pin(async move {
            fake.entered.notify_one();
            if fake.block.load(Ordering::SeqCst) {
                fake.release.notified().await;
            }
            if Instant::now() >= deadline {
                return Delivery::Expired;
            }
            if abandoned() {
                return Delivery::Abandoned;
            }
            fake.sent.lock().unwrap().push(format!("{order:?}"));
            fake.emitted.notify_one();
            if fake.after.load(Ordering::SeqCst) {
                fake.release.notified().await;
            }
            Delivery::Acknowledged
        })
    }
}
impl Connector for Adapter {
    fn open(&self, _: Arc<Approved>, _: gateway::Gateway) -> Link {
        self.0.opens.fetch_add(1, Ordering::SeqCst);
        let (tx, events) = mpsc::channel(32);
        *self.0.events.lock().unwrap() = Some(tx.clone());
        Link {
            wire: Arc::new(Adapter(self.0.clone())),
            events,
            run: Box::pin(async move {
                tx.send(Event::Ready).await.unwrap();
                tx.send(Event::Media(cast::Event::Playing(cast::Playing {
                    app: Some("Example player".into()),
                    state: Some("playing".into()),
                    title: Some("Example title".into()),
                    ..cast::Playing::default()
                })))
                .await
                .unwrap();
                tx.send(Event::Volume(cast::ReceiverVolume {
                    level: Some(0.2),
                    muted: Some(false),
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
    gateway: gateway::Gateway,
    fake: Arc<Fake>,
    cancel: CancellationToken,
    dir: std::path::PathBuf,
}
fn point(key: &str) -> PointId {
    PointId::from(format!("example:00000000000000000000000000000001/{key}"))
}
impl Harness {
    async fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(1);
        let dir = std::env::temp_dir().join(format!(
            "moli-cast-hub-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let hub = Hub::new(HubOptions {
            secrets: Some((dir.join("secrets.enc"), MasterKey::new("42".repeat(32)))),
            ..HubOptions::default()
        })
        .unwrap();
        let fake = Arc::new(Fake::default());
        let config = Config {
            host: None,
            interface: Some("192.168.1.2".parse().unwrap()),
            prefix: 24,
            external_ids: Vec::new(),
        };
        let mut driver = Cast::new(config).unwrap();
        driver.connector = Arc::new(Adapter(fake.clone()));
        driver.discover = fake.clone();
        let gateway = driver.gateway();
        let cancel = CancellationToken::new();
        spawn_driver(
            &hub,
            InstanceId::from("example"),
            Arc::new(driver),
            cancel.clone(),
        );
        for _ in 0..200 {
            if gateway.state().phase != Phase::Unavailable {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(gateway.state().phase, Phase::Idle);
        Self {
            hub,
            gateway,
            fake,
            cancel,
            dir,
        }
    }
    async fn approve(&self) -> u64 {
        let view = self.gateway.discover().await.unwrap();
        self.gateway
            .approve(view.generation, candidate().id, true)
            .await
            .unwrap();
        for _ in 0..1000 {
            if self
                .hub
                .state(&point("volume"))
                .is_some_and(|s| s.value.as_f64() == Some(20.0))
            {
                return view.generation;
            }
            tokio::task::yield_now().await;
        }
        panic!("validated receiver was not published with its actual volume");
    }
    async fn send(&self, key: &str, value: Value) -> Result<(), moli_runtime::CommandError> {
        self.hub
            .command(&point(key), value, Origin::Api, None)
            .await
    }
    async fn event(&self, event: Event) {
        let tx = self.fake.events.lock().unwrap().clone().unwrap();
        tx.send(event).await.unwrap();
        for _ in 0..30 {
            tokio::task::yield_now().await;
        }
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        self.cancel.cancel();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
#[tokio::test]
async fn a_discovered_receiver_stays_out_of_the_hub_and_off_the_network_until_approved() {
    let h = Harness::new().await;
    assert!(h.hub.snapshot().devices.is_empty());
    assert_eq!(h.fake.opens.load(Ordering::SeqCst), 0);
    assert_eq!(h.fake.queries.load(Ordering::SeqCst), 0);
    h.gateway.discover().await.unwrap();
    assert!(h.hub.snapshot().devices.is_empty());
    assert_eq!(h.fake.opens.load(Ordering::SeqCst), 0);
    h.approve().await;
    assert_eq!(h.fake.opens.load(Ordering::SeqCst), 1);
    h.send("volume", Value::Int(40)).await.unwrap();
    h.send("mute", Value::Bool(true)).await.unwrap();
    h.send("media_control", Value::from("pause")).await.unwrap();
    assert_eq!(
        *h.fake.sent.lock().unwrap(),
        ["Volume(0.4)", "Mute(true)", "Media(Pause)"]
    );
    assert_eq!(
        h.hub.state(&point("media_title")).unwrap().value,
        Value::from("Example title")
    );
}
#[tokio::test]
async fn revoke_removes_the_device_and_invalidates_a_request_before_emission() {
    let h = Harness::new().await;
    let generation = h.approve().await;
    h.fake.block.store(true, Ordering::SeqCst);
    let hub = h.hub.clone();
    let request = tokio::spawn(async move {
        hub.command(&point("volume"), Value::Int(40), Origin::Api, None)
            .await
    });
    h.fake.entered.notified().await;
    h.gateway.revoke(generation).await.unwrap();
    h.fake.block.store(false, Ordering::SeqCst);
    h.fake.release.notify_one();
    assert!(request.await.unwrap().is_err());
    for _ in 0..200 {
        if h.hub.snapshot().devices.is_empty() {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(h.hub.snapshot().devices.is_empty());
    assert!(h.fake.sent.lock().unwrap().is_empty());
}
#[tokio::test]
async fn queued_media_cannot_cross_a_lost_connection_and_new_orders_still_work() {
    let h = Harness::new().await;
    h.approve().await;
    h.fake.block.store(true, Ordering::SeqCst);
    let hub = h.hub.clone();
    let first = tokio::spawn(async move {
        hub.command(&point("volume"), Value::Int(40), Origin::Api, None)
            .await
    });
    h.fake.entered.notified().await;
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
    for _ in 0..30 {
        tokio::task::yield_now().await;
    }
    h.event(Event::Lost).await;
    h.event(Event::Ready).await;
    h.event(Event::Media(cast::Event::Playing(cast::Playing {
        state: Some("playing".into()),
        ..cast::Playing::default()
    })))
    .await;
    h.fake.block.store(false, Ordering::SeqCst);
    h.fake.release.notify_one();
    assert!(first.await.unwrap().is_err());
    assert!(pause.await.unwrap().is_err());
    assert!(h.fake.sent.lock().unwrap().is_empty());
    h.send("media_control", Value::from("pause")).await.unwrap();
    assert_eq!(*h.fake.sent.lock().unwrap(), ["Media(Pause)"]);
}
#[tokio::test]
async fn a_caller_leaving_after_emission_keeps_the_actual_result_in_the_journal() {
    let h = Harness::new().await;
    h.approve().await;
    h.fake.after.store(true, Ordering::SeqCst);
    let hub = h.hub.clone();
    let caller = tokio::spawn(async move {
        hub.command(&point("volume"), Value::Int(40), Origin::Api, None)
            .await
    });
    h.fake.emitted.notified().await;
    caller.abort();
    let _ = caller.await;
    h.fake.release.notify_one();
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
    panic!("emitted result was lost");
}

#[tokio::test]
async fn fifo_limits_all_callers_to_four_and_preserves_their_order() {
    let h = Harness::new().await;
    h.approve().await;
    h.fake.block.store(true, Ordering::SeqCst);
    let hub = h.hub.clone();
    let first = tokio::spawn(async move {
        hub.command(&point("volume"), Value::Int(10), Origin::Api, None)
            .await
    });
    h.fake.entered.notified().await;
    let mut queued = Vec::new();
    for value in [20, 30, 40] {
        let hub = h.hub.clone();
        queued.push(tokio::spawn(async move {
            hub.command(&point("volume"), Value::Int(value), Origin::Api, None)
                .await
        }));
        for _ in 0..30 {
            tokio::task::yield_now().await;
        }
    }
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            h.send("volume", Value::Int(50))
        )
        .await
        .unwrap()
        .is_err()
    );
    assert!(h.fake.sent.lock().unwrap().is_empty());
    h.fake.block.store(false, Ordering::SeqCst);
    h.fake.release.notify_one();
    first.await.unwrap().unwrap();
    for request in queued {
        request.await.unwrap().unwrap();
    }
    assert_eq!(
        *h.fake.sent.lock().unwrap(),
        ["Volume(0.1)", "Volume(0.2)", "Volume(0.3)", "Volume(0.4)"]
    );
}
