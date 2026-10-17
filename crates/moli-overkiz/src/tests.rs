//! The driver's loop against a box in memory and the real hub: answers on
//! time whatever the network does, orders sent once, pauses and refusals.

use std::sync::atomic::Ordering::SeqCst;

use moli_core::{InstanceId, Origin, PointId, Value};
use moli_runtime::{Hub, HubOptions, spawn_driver};
use serde_json::json;

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
async fn a_slow_order_never_blocks_another_nor_comes_back() {
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
    assert_eq!(fake.applied.load(SeqCst), 2, "each sent once, never again");
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
