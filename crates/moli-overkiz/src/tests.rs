//! The driver's loop against a box in memory and the real hub: answers on
//! time whatever the network does, orders sent once, pauses and refusals.

use std::sync::atomic::Ordering::SeqCst;

use moli_core::{InstanceId, Origin, PointId, Value};
use moli_runtime::{Hub, HubOptions, spawn_driver};
use serde_json::json;
use tokio::task::JoinSet;

use super::*;
use crate::fake::Fake;

const PACE: (Duration, Duration) = (Duration::from_millis(50), Duration::from_millis(50));
const BOX: &str = "1234-5678-9012";

#[derive(Debug)]
struct Test(Arc<Fake>);

impl Driver for Test {
    fn kind(&self) -> &'static str {
        "overkiz"
    }

    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(drive(Api::Fake(Arc::clone(&self.0)), ctx, PACE, ""))
    }
}

fn setup() -> Json {
    let path = format!(
        "{}/../../integrations/overkiz/fixtures/setup-devices.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut setup: Json = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    // A second box of the same account, with its own light.
    let mut other = setup
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["controllableName"] == "ogp:Light")
        .unwrap()
        .clone();
    other["deviceURL"] = json!("ogp://4444-5555-6666/00000001");
    setup.as_array_mut().unwrap().push(other);
    setup
}

fn fake() -> Fake {
    Fake {
        setup: setup(),
        ..Fake::default()
    }
}

fn light() -> DeviceId {
    DeviceId::new(
        &InstanceId::from("volets"),
        &format!("ogp://{BOX}/00000003"),
    )
}

async fn start(fake: Fake) -> (Hub, Arc<Fake>) {
    let hub = Hub::new(HubOptions::default()).unwrap();
    let fake = Arc::new(fake);
    spawn_driver(
        &hub,
        InstanceId::from("volets"),
        Arc::new(Test(Arc::clone(&fake))),
        tokio_util::sync::CancellationToken::new(),
    );
    until(|| hub.device(&light()).is_some()).await;
    (hub, fake)
}

async fn until(check: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !check() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("in time");
}

async fn switch(hub: &Hub, on: bool) -> Result<(), String> {
    hub.command(
        &PointId::new(&light(), "on"),
        Value::Bool(on),
        Origin::Ui,
        None,
    )
    .await
    .map_err(|e| e.to_string())
}

#[tokio::test]
async fn a_confirmed_order_is_done() {
    let (hub, fake) = start(Fake {
        ends: Some(("COMPLETED", None)),
        ..fake()
    })
    .await;
    let at = Instant::now();
    assert_eq!(switch(&hub, true).await, Ok(()));
    assert!(at.elapsed() < Duration::from_secs(1), "{:?}", at.elapsed());
    assert_eq!(fake.applied.load(SeqCst), 1);
}

#[tokio::test]
async fn a_failed_order_says_why() {
    let (hub, _) = start(Fake {
        ends: Some(("FAILED", Some("NONEXEC_OTHER"))),
        ..fake()
    })
    .await;
    let error = switch(&hub, true).await.unwrap_err();
    assert!(error.contains("NONEXEC_OTHER"), "{error}");
}

#[tokio::test]
async fn a_slow_fetch_never_delays_the_answer() {
    let (hub, fake) = start(Fake {
        slow_fetch: Some(Duration::from_secs(10)),
        ..fake()
    })
    .await;
    let at = Instant::now();
    // Taken by the box, never confirmed in time: « accepted », before the hub gives up.
    assert_eq!(switch(&hub, true).await, Ok(()));
    assert!(at.elapsed() < Duration::from_secs(5), "{:?}", at.elapsed());
    assert_eq!(fake.applied.load(SeqCst), 1);
    assert_eq!(fake.fetched.load(SeqCst), 1, "one fetch at a time");
}

#[tokio::test]
async fn a_slow_order_holds_the_next_which_never_leaves_late() {
    let (hub, fake) = start(Fake {
        slow_apply: Some(Duration::from_secs(10)),
        ..fake()
    })
    .await;
    let at = Instant::now();
    let (first, second) = tokio::join!(switch(&hub, true), switch(&hub, false));
    assert!(first.is_err() && second.is_err(), "{first:?} {second:?}");
    assert!(at.elapsed() < Duration::from_secs(5), "{:?}", at.elapsed());
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        fake.applied.load(SeqCst),
        1,
        "the second never left after its time"
    );
}

/// `on` then `off`, the first over a slow connection: they reach the box
/// in that order, whatever the transport.
#[tokio::test]
async fn orders_reach_the_box_in_the_order_given() {
    let fake = Fake {
        ends: Some(("COMPLETED", None)),
        ..fake()
    };
    fake.delays
        .lock()
        .unwrap()
        .push_back(Duration::from_millis(250));
    let (hub, fake) = start(fake).await;
    let first = {
        let hub = hub.clone();
        tokio::spawn(async move { switch(&hub, true).await })
    };
    until(|| fake.applied.load(SeqCst) == 1).await;
    let second = switch(&hub, false).await;
    assert_eq!(first.await.unwrap(), Ok(()));
    assert_eq!(second, Ok(()));
    let arrived: Vec<String> = fake
        .arrivals
        .lock()
        .unwrap()
        .iter()
        .map(|(c, _)| c.clone())
        .collect();
    assert_eq!(arrived, ["on", "off"]);
}

/// The hub's queue is not emptied into unbounded work: one order on its
/// way, a few waiting, the rest refused at once.
#[tokio::test]
async fn orders_in_flight_are_bounded() {
    let (hub, fake) = start(Fake {
        slow_apply: Some(Duration::from_secs(10)),
        ..fake()
    })
    .await;
    let mut callers = JoinSet::new();
    for n in 0..64 {
        let hub = hub.clone();
        callers.spawn(async move { switch(&hub, n % 2 == 0).await });
    }
    let at = Instant::now();
    let mut refused_at_once = 0;
    while let Some(result) = callers.join_next().await {
        let error = result.unwrap().unwrap_err();
        if at.elapsed() < Duration::from_secs(1) {
            refused_at_once += 1;
            assert!(!error.is_empty());
        }
    }
    assert!(refused_at_once >= 64 - 1 - MAX_WAITING, "{refused_at_once}");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        fake.applied.load(SeqCst),
        1,
        "one on its way, none sent late"
    );
}

/// The caller of the second order gives up while the first is on its way:
/// the second never leaves.
#[tokio::test]
async fn an_order_whose_caller_gave_up_never_leaves() {
    let fake = Fake {
        ends: Some(("COMPLETED", None)),
        ..fake()
    };
    fake.delays
        .lock()
        .unwrap()
        .push_back(Duration::from_millis(300));
    let (hub, fake) = start(fake).await;
    let first = {
        let hub = hub.clone();
        tokio::spawn(async move { switch(&hub, true).await })
    };
    until(|| fake.applied.load(SeqCst) == 1).await;
    let second = {
        let hub = hub.clone();
        tokio::spawn(async move { switch(&hub, false).await })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    second.abort();
    assert_eq!(first.await.unwrap(), Ok(()));
    tokio::time::sleep(Duration::from_millis(200)).await;
    let arrived: Vec<String> = fake
        .arrivals
        .lock()
        .unwrap()
        .iter()
        .map(|(c, _)| c.clone())
        .collect();
    assert_eq!(arrived, ["on"]);
}

/// The caller gives up while a fetch is on the network; that fetch brings
/// the order's failure: the household hears it, exactly once.
#[tokio::test]
async fn a_failure_after_the_caller_gave_up_is_told() {
    let (hub, fake) = start(Fake {
        slow_fetch: Some(Duration::from_millis(700)),
        ends: Some(("FAILED", Some("NONEXEC_OTHER"))),
        ..fake()
    })
    .await;
    let mut events = hub.subscribe();
    until(|| fake.fetched.load(SeqCst) == 1).await;
    let caller = {
        let hub = hub.clone();
        tokio::spawn(async move { switch(&hub, true).await })
    };
    until(|| fake.applied.load(SeqCst) == 1).await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    until(|| fake.fetched.load(SeqCst) >= 2).await;
    assert!(
        fake.news.lock().unwrap().is_empty(),
        "the failure was fetched"
    );
    let mut notices = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let moli_core::Event::Notice { notice } = event {
            notices.push(notice.message);
        }
    }
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert!(notices[0].contains("NONEXEC_OTHER"), "{notices:?}");
}

/// A box that takes orders at once but never confirms them: the callers
/// waiting for a confirmation are bounded too, the rest refused at once.
#[tokio::test]
async fn callers_waiting_for_a_confirmation_are_bounded() {
    let (hub, fake) = start(Fake {
        slow_fetch: Some(Duration::from_secs(10)),
        ..fake()
    })
    .await;
    let mut callers = JoinSet::new();
    for n in 0..48 {
        let hub = hub.clone();
        callers.spawn(async move { switch(&hub, n % 2 == 0).await });
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let at = Instant::now();
    let mut refused_at_once = 0;
    while let Some(result) = callers.join_next().await {
        if result.unwrap().is_err() && at.elapsed() < Duration::from_secs(1) {
            refused_at_once += 1;
        }
    }
    assert!(
        fake.applied.load(SeqCst) <= MAX_PENDING,
        "{}",
        fake.applied.load(SeqCst)
    );
    assert!(refused_at_once >= 48 - MAX_PENDING, "{refused_at_once}");
}

/// A task that starts after the order's instant sends nothing; a box that
/// is slow is left at that instant.
#[tokio::test]
async fn an_order_never_leaves_after_its_instant() {
    let fake = Arc::new(Fake {
        slow_apply: Some(Duration::from_secs(10)),
        ..fake()
    });
    let api = Api::Fake(Arc::clone(&fake));
    let order = Order {
        url: format!("ogp://{BOX}/00000003"),
        command: "on",
        parameters: Vec::new(),
    };
    let past = Instant::now()
        .checked_sub(Duration::from_millis(100))
        .unwrap();
    let e = emit(&api, &order, past).await.unwrap_err();
    assert!(e.is::<api::NotSent>(), "{e}");
    assert_eq!(fake.applied.load(SeqCst), 0, "nothing sent");
    let at = Instant::now();
    let e = emit(&api, &order, at + Duration::from_millis(100))
        .await
        .unwrap_err();
    assert!(e.is::<NoAnswer>(), "{e}");
    assert!(at.elapsed() < Duration::from_millis(500));
    assert_eq!(fake.applied.load(SeqCst), 1);
}

#[tokio::test]
async fn a_busy_box_is_left_alone() {
    let (hub, fake) = start(Fake {
        apply_refusal: Some(Refusal::Busy),
        ..fake()
    })
    .await;
    assert!(switch(&hub, true).await.is_err());
    let fetched = fake.fetched.load(SeqCst);
    let at = Instant::now();
    let error = switch(&hub, false).await.unwrap_err();
    assert!(at.elapsed() < Duration::from_millis(500), "refused at once");
    assert!(!error.is_empty());
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        fake.applied.load(SeqCst),
        1,
        "nothing sent during the pause"
    );
    assert!(fake.fetched.load(SeqCst) <= fetched + 1, "reads paused too");
}

#[tokio::test]
async fn a_refused_access_takes_the_devices_offline() {
    let fake = fake();
    fake.fetches.lock().unwrap().push_back(Err(Refusal::Denied));
    let (hub, _) = start(fake).await;
    until(|| hub.device(&light()).and_then(|d| d.online) == Some(false)).await;
}

#[tokio::test]
async fn a_box_down_is_only_that_box() {
    let fake = fake();
    fake.fetches.lock().unwrap().push_back(Ok(
        json!([{ "name": "GatewayDownEvent", "gatewayId": BOX }]),
    ));
    let (hub, fake) = start(fake).await;
    until(|| fake.fetched.load(SeqCst) >= 2).await;
    let other = DeviceId::new(&InstanceId::from("volets"), "ogp://4444-5555-6666/00000001");
    assert_eq!(hub.device(&light()).unwrap().online, Some(false));
    assert_eq!(hub.device(&other).unwrap().online, Some(true));
}

#[tokio::test]
async fn nothing_that_changes_the_box_is_sent_twice() {
    let (hub, fake) = start(Fake {
        ends: Some(("COMPLETED", None)),
        ..fake()
    })
    .await;
    assert_eq!(switch(&hub, true).await, Ok(()));
    until(|| fake.fetched.load(SeqCst) >= 3).await;
    assert_eq!(fake.replayable.load(SeqCst), 0);
}

fn config(text: &str) -> anyhow::Result<Overkiz> {
    Overkiz::new(toml::from_str(text)?)
}

#[test]
fn configurations() {
    let local = config("transport = \"local\"\nhost = \"192.168.1.x\"").unwrap();
    assert_eq!(local.config.port, 8443);
    assert!(config("transport = \"cloud\"").is_ok());
    assert!(
        config("transport = \"local\"").is_err(),
        "a box needs its address"
    );
    assert!(
        config("transport = \"cloud\"\nhost = \"example.org\"").is_err(),
        "the cloud's address is never configurable"
    );
    assert!(config("transport = \"radio\"").is_err());
    assert!(config("transport = \"cloud\"\nurl = \"x\"").is_err());
}
