//! The real loop ([`serve`]) with the real hub, against a broker in memory.

use std::sync::Mutex;

use moli_core::{InstanceId, Origin, PointId};
use moli_runtime::{CommandError, Hub, HubOptions};
use rumqttc::mqttbytes::QoS;
use rumqttc::mqttbytes::v4::{ConnAck, ConnectReturnCode, Packet, PubAck, Publish};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use super::*;
use crate::mqtt::fake;

const ITEMS: &[u8] = include_bytes!("../../../integrations/landroid/fixtures/product-items.json");
const MOWING: &[u8] = include_bytes!("../../../integrations/landroid/fixtures/status-mowing.json");
const SERIAL: &str = "20000000000000000001";
const IN: &str = "PRM100/AABBCCDDEEFF/commandIn";
const OUT: &str = "PRM100/AABBCCDDEEFF/commandOut";

/// Publishes the fixture's mowers, connects over the fake link, serves.
struct Harness {
    link: Mutex<Option<fake::Link>>,
    ended: Mutex<Option<oneshot::Sender<String>>>,
}

impl Driver for Harness {
    fn kind(&self) -> &'static str {
        "landroid"
    }

    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let Some(link) = self.link.lock().unwrap().take() else {
                park(ctx, "restarted").await;
                return Ok(());
            };
            let mut house = House::default();
            publish(ctx, &mut house, model::mowers(ITEMS).unwrap());
            ctx.ready();
            let mut mqtt = Mqtt::open(link, "c", "u", KEEP_ALIVE).await?;
            let media = ctx.media_publisher();
            let end = serve(
                ctx,
                &mut house,
                &mut mqtt,
                Instant::now() + Duration::from_secs(3600),
                &media,
            )
            .await;
            if let Some(tx) = self.ended.lock().unwrap().take() {
                let _ = tx.send(format!("{end:?}"));
            }
            park(ctx, "ended").await;
            Ok(())
        })
    }
}

struct Bench {
    hub: Hub,
    broker: fake::Broker,
    ended: oneshot::Receiver<String>,
    cancel: CancellationToken,
}

impl Drop for Bench {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

fn point(key: &str) -> PointId {
    PointId::from(format!("tondeuse:{SERIAL}/{key}"))
}

async fn bench() -> Bench {
    let hub = Hub::new(HubOptions::default()).unwrap();
    let (link, mut broker) = fake::pair();
    let (tx, ended) = oneshot::channel();
    let cancel = CancellationToken::new();
    moli_runtime::spawn_driver(
        &hub,
        InstanceId::from("tondeuse"),
        Arc::new(Harness {
            link: Mutex::new(Some(link)),
            ended: Mutex::new(Some(tx)),
        }),
        cancel.clone(),
    );
    let Some(Packet::Connect(_)) = broker.packet().await else {
        panic!("connect");
    };
    broker.send(&Packet::ConnAck(ConnAck::new(
        ConnectReturnCode::Success,
        false,
    )));
    // The cloud's last status is published at once.
    while hub.state(&point("state")).is_none() {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    Bench {
        hub,
        broker,
        ended,
        cancel,
    }
}

/// An order from the dashboard, in the background.
fn order(hub: &Hub, key: &str, value: Value) -> tokio::task::JoinHandle<Result<(), CommandError>> {
    let (hub, point) = (hub.clone(), point(key));
    tokio::spawn(async move { hub.command(&point, value, Origin::Ui, None).await })
}

/// The next order the broker hears (keep-alives skipped).
async fn published(broker: &mut fake::Broker) -> Publish {
    loop {
        match broker.packet().await {
            Some(Packet::Publish(p)) => return p,
            Some(_) => {}
            None => panic!("the client is gone"),
        }
    }
}

fn body(p: &Publish) -> Json {
    serde_json::from_slice(&p.payload).unwrap()
}

/// Nothing more is published for `wait`.
async fn silent(broker: &mut fake::Broker, wait: Duration) {
    let quiet = tokio::time::timeout(wait, async {
        loop {
            match broker.packet().await {
                Some(Packet::Publish(p)) => return Some(p),
                Some(_) => {}
                None => return None,
            }
        }
    })
    .await;
    if let Ok(Some(p)) = quiet {
        panic!("published again: {:?}", body(&p));
    }
}

async fn settle() {
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn an_order_leaves_once_and_the_mower_status_follows() {
    let mut b = bench().await;
    let call = order(&b.hub, "control", Value::from("start"));
    let p = published(&mut b.broker).await;
    assert_eq!(p.topic, IN);
    assert_eq!(p.qos, QoS::AtLeastOnce);
    let message = body(&p);
    assert_eq!(message["cmd"], 1);
    assert_eq!(message["sn"], SERIAL);
    assert!(message["tm"].is_string() && message["dt"].is_string());
    b.broker.send(&Packet::PubAck(PubAck::new(p.pkid)));
    assert_eq!(call.await.unwrap(), Ok(()));

    // The mower's answer: its status, on its topic.
    b.broker
        .send(&Packet::Publish(Publish::new(OUT, QoS::AtMostOnce, MOWING)));
    // Another mower's status on that topic is not this mower's.
    let foreign = String::from_utf8_lossy(MOWING).replace(SERIAL, "20000000000000000009");
    b.broker.send(&Packet::Publish(Publish::new(
        OUT,
        QoS::AtMostOnce,
        foreign.replace("\"ls\": 7", "\"ls\": 9"),
    )));
    settle().await;
    assert_eq!(
        b.hub.state(&point("state")).unwrap().value,
        Value::from("mowing")
    );
    assert_eq!(
        b.hub.state(&point("battery")).unwrap().value,
        Value::Int(74)
    );
    silent(&mut b.broker, Duration::from_millis(100)).await;
}

#[tokio::test]
async fn orders_leave_in_order_and_one_abandoned_before_leaving_never_leaves() {
    let mut b = bench().await;
    let first = order(&b.hub, "control", Value::from("start"));
    let p1 = published(&mut b.broker).await;
    // Queued behind the first: its caller leaves before it can go.
    let second = order(&b.hub, "control", Value::from("pause"));
    settle().await;
    second.abort();
    let _ = second.await;
    let third = order(&b.hub, "control", Value::from("home"));
    settle().await;
    b.broker.send(&Packet::PubAck(PubAck::new(p1.pkid)));
    assert_eq!(first.await.unwrap(), Ok(()));
    let p3 = published(&mut b.broker).await;
    assert_eq!(body(&p3)["cmd"], 3, "the pause never left");
    b.broker.send(&Packet::PubAck(PubAck::new(p3.pkid)));
    assert_eq!(third.await.unwrap(), Ok(()));
    silent(&mut b.broker, Duration::from_millis(100)).await;
}

#[tokio::test]
async fn a_full_queue_refuses_at_once() {
    let mut b = bench().await;
    let first = order(&b.hub, "control", Value::from("start"));
    let p = published(&mut b.broker).await;
    let queued: Vec<_> = (0..QUEUE)
        .map(|_| order(&b.hub, "rain_delay", Value::Int(30)))
        .collect();
    settle().await;
    let started = std::time::Instant::now();
    let refused = order(&b.hub, "rain_delay", Value::Int(30)).await.unwrap();
    assert!(
        matches!(refused, Err(CommandError::Driver(_))),
        "{refused:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(1));
    b.broker.send(&Packet::PubAck(PubAck::new(p.pkid)));
    assert_eq!(first.await.unwrap(), Ok(()));
    for _ in 0..QUEUE {
        let p = published(&mut b.broker).await;
        assert_eq!(body(&p)["rd"], 30);
        b.broker.send(&Packet::PubAck(PubAck::new(p.pkid)));
    }
    for call in queued {
        assert_eq!(call.await.unwrap(), Ok(()));
    }
}

#[tokio::test]
async fn a_lost_link_after_sending_is_maybe_sent_and_nothing_is_replayed() {
    let mut b = bench().await;
    let first = order(&b.hub, "control", Value::from("home"));
    let _ = published(&mut b.broker).await;
    let second = order(&b.hub, "control", Value::from("start"));
    settle().await;
    // The broker goes away.
    b.broker.say = mpsc::unbounded_channel().0;
    let Err(CommandError::Driver(why)) = first.await.unwrap() else {
        panic!("first");
    };
    assert!(why.contains("peut-être"), "{why}");
    let Err(CommandError::Driver(why)) = second.await.unwrap() else {
        panic!("second");
    };
    assert!(why.contains("pas été envoyé"), "{why}");
    assert!((&mut b.ended).await.unwrap().starts_with("Lost"));
    silent(&mut b.broker, Duration::from_millis(100)).await;
    // Without a link, an order is refused, never kept for later.
    let late = order(&b.hub, "control", Value::from("start"))
        .await
        .unwrap();
    assert!(matches!(late, Err(CommandError::Driver(_))), "{late:?}");
}

#[tokio::test]
async fn an_unacknowledged_order_is_answered_in_time_and_never_sent_again() {
    let mut b = bench().await;
    let call = order(&b.hub, "control", Value::from("pause"));
    let p = published(&mut b.broker).await;
    // No acknowledgement: the driver answers before the hub gives up.
    let Err(CommandError::Driver(why)) = call.await.unwrap() else {
        panic!("not a driver answer");
    };
    assert!(why.contains("non confirmé"), "{why}");
    // A late acknowledgement changes nothing, and nothing leaves again.
    b.broker.send(&Packet::PubAck(PubAck::new(p.pkid)));
    silent(&mut b.broker, Duration::from_millis(200)).await;
    let journal: Vec<moli_core::Outcome> = b
        .hub
        .journal(10)
        .iter()
        .filter(|e| matches!(e.action, moli_core::Action::Command { .. }))
        .map(|e| e.outcome.clone())
        .collect();
    assert_eq!(journal.len(), 1);
    assert_ne!(journal[0], moli_core::Outcome::Ok);
}

#[tokio::test]
async fn the_schedule_goes_back_whole_and_writes_are_checked() {
    let mut b = bench().await;
    let call = order(&b.hub, "schedule", Value::from("off"));
    let p = published(&mut b.broker).await;
    let sc = &body(&p)["sc"];
    assert_eq!(sc["m"], 0);
    assert_eq!(sc["d"][1], serde_json::json!(["10:00", 300, 1]));
    b.broker.send(&Packet::PubAck(PubAck::new(p.pkid)));
    assert_eq!(call.await.unwrap(), Ok(()));
    let read_only = order(&b.hub, "state", Value::from("home")).await.unwrap();
    assert_eq!(read_only, Err(CommandError::ReadOnly));
    silent(&mut b.broker, Duration::from_millis(100)).await;
}

#[tokio::test]
async fn an_agent_starting_the_mower_waits_for_a_person() {
    let b = bench().await;
    let held = b
        .hub
        .command(&point("control"), Value::from("start"), Origin::Mcp, None)
        .await;
    assert!(
        matches!(held, Err(CommandError::NeedsApproval { .. })),
        "{held:?}"
    );
}
