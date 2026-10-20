//! The real loop ([`serve`]) with the real hub, against a vacuum simulated
//! on the loopback (real miIO packets, real encryption).

use std::collections::HashMap;
use std::sync::Arc;

use moli_core::{InstanceId, Origin, PointId};
use moli_runtime::{CommandError, Hub, HubOptions};
use tokio::sync::{Notify, mpsc};
use tokio_util::sync::CancellationToken;

use super::*;

const STATUS: &[u8] = include_bytes!("../../../integrations/roborock/fixtures/get_status.json");
const DEVICE: [u8; 4] = [1, 2, 3, 4];
const TOKEN_HEX: &str = "000102030405060708090a0b0c0d0e0f";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Answer {
    Ok,
    Silent,
    Invalid,
    /// Answered `ok` once [`Vacuum::release`] is notified.
    Hold,
}

/// The simulated vacuum: what it heard, and how to let a held answer go.
struct Vacuum {
    port: u16,
    heard: mpsc::UnboundedReceiver<String>,
    release: Arc<Notify>,
}

async fn vacuum(model: &'static str, answers: &[(&'static str, Answer)]) -> Vacuum {
    let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let port = socket.local_addr().unwrap().port();
    let (tx, heard) = mpsc::unbounded_channel();
    let release = Arc::new(Notify::new());
    let answers: HashMap<&str, Answer> = answers.iter().copied().collect();
    let (rel, keys) = (
        Arc::clone(&release),
        Arc::new(Keys::new(miio::token(TOKEN_HEX).unwrap())),
    );
    tokio::spawn(async move {
        let mut buf = vec![0u8; 4096];
        let stamp = 1000u32;
        while let Ok((n, from)) = socket.recv_from(&mut buf).await {
            let packet = &buf[..n];
            if packet == miio::hello() {
                let mut answer = miio::hello();
                answer[4..16].copy_from_slice(&[0, 0, 0, 0, 1, 2, 3, 4, 0, 0, 0x03, 0xe8]);
                let _ = socket.send_to(&answer, from).await;
                continue;
            }
            let Ok((_, plain)) = keys.decode(DEVICE, packet) else {
                continue;
            };
            let request: Json = serde_json::from_slice(&plain).unwrap();
            let id = request["id"].clone();
            let method = request["method"].as_str().unwrap().to_owned();
            let _ = tx.send(method.clone());
            let reply = match method.as_str() {
                "miIO.info" => {
                    json!({"id": id, "result": {"model": model, "fw_ver": "3.5.8_002034"}})
                }
                "get_status" if answers.get("get_status") == Some(&Answer::Invalid) => {
                    json!({"id": id, "result": []})
                }
                "get_status" => {
                    let mut status: Json = serde_json::from_slice(STATUS).unwrap();
                    status["id"] = id;
                    status
                }
                other => match answers.get(other).copied().unwrap_or(Answer::Ok) {
                    Answer::Silent | Answer::Invalid => continue,
                    Answer::Ok => json!({"id": id, "result": ["ok"]}),
                    Answer::Hold => {
                        let (socket, keys, rel) =
                            (Arc::clone(&socket), Arc::clone(&keys), Arc::clone(&rel));
                        tokio::spawn(async move {
                            rel.notified().await;
                            let reply = json!({"id": id, "result": ["ok"]});
                            let packet = keys.encode(DEVICE, stamp, reply.to_string().as_bytes());
                            let _ = socket.send_to(&packet, from).await;
                        });
                        continue;
                    }
                },
            };
            let packet = keys.encode(DEVICE, stamp, reply.to_string().as_bytes());
            let _ = socket.send_to(&packet, from).await;
        }
    });
    Vacuum {
        port,
        heard,
        release,
    }
}

/// Serves the simulated vacuum.
struct Harness {
    port: u16,
}

impl Driver for Harness {
    fn kind(&self) -> &'static str {
        "roborock"
    }

    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let socket = UdpSocket::bind("127.0.0.1:0").await?;
            socket.connect(("127.0.0.1", self.port)).await?;
            let keys = Keys::new(miio::token(TOKEN_HEX).unwrap());
            serve(ctx, &socket, &keys, "Aspirateur").await;
            Ok(())
        })
    }
}

struct Bench {
    hub: Hub,
    vacuum: Vacuum,
    cancel: CancellationToken,
}

impl Drop for Bench {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

fn point(key: &str) -> PointId {
    PointId::from(format!("aspirateur:{}/{key}", u32::from_be_bytes(DEVICE)))
}

async fn bench(model: &'static str, answers: &[(&'static str, Answer)]) -> Bench {
    let hub = Hub::new(HubOptions::default()).unwrap();
    let vacuum = vacuum(model, answers).await;
    let cancel = CancellationToken::new();
    moli_runtime::spawn_driver(
        &hub,
        InstanceId::from("aspirateur"),
        Arc::new(Harness { port: vacuum.port }),
        cancel.clone(),
    );
    Bench {
        hub,
        vacuum,
        cancel,
    }
}

/// Waits for the first reading to be published.
async fn ready(b: &Bench) {
    for _ in 0..400 {
        if b.hub.state(&point("state")).is_some() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("never read");
}

fn order(hub: &Hub, value: &str) -> tokio::task::JoinHandle<Result<(), CommandError>> {
    let (hub, point, value) = (hub.clone(), point("control"), Value::from(value));
    tokio::spawn(async move { hub.command(&point, value, Origin::Ui, None).await })
}

/// What the vacuum heard, up to and including `method`.
async fn heard_until(v: &mut Vacuum, method: &str) -> Vec<String> {
    let mut heard = Vec::new();
    loop {
        let m = tokio::time::timeout(Duration::from_secs(6), v.heard.recv())
            .await
            .expect("heard in time")
            .unwrap();
        let done = m == method;
        heard.push(m);
        if done {
            return heard;
        }
    }
}

/// Everything heard for `wait`.
async fn heard_for(v: &mut Vacuum, wait: Duration) -> Vec<String> {
    let mut heard = Vec::new();
    let _ = tokio::time::timeout(wait, async {
        while let Some(m) = v.heard.recv().await {
            heard.push(m);
        }
    })
    .await;
    heard
}

#[tokio::test]
async fn the_vacuum_is_read_and_an_order_leaves_once() {
    let mut b = bench("roborock.vacuum.s6", &[]).await;
    ready(&b).await;
    assert_eq!(
        b.hub.state(&point("state")).unwrap().value,
        Value::from("cleaning")
    );
    assert_eq!(
        b.hub.state(&point("battery")).unwrap().value,
        Value::Int(83)
    );
    assert_eq!(order(&b.hub, "home").await.unwrap(), Ok(()));
    let heard = heard_until(&mut b.vacuum, "app_charge").await;
    assert_eq!(heard.iter().filter(|m| *m == "app_charge").count(), 1);
    let after = heard_for(&mut b.vacuum, Duration::from_millis(300)).await;
    assert!(!after.iter().any(|m| m.starts_with("app_")), "{after:?}");
}

#[tokio::test]
async fn an_unanswered_order_is_answered_in_time_and_never_sent_again() {
    let mut b = bench("roborock.vacuum.s6", &[("app_pause", Answer::Silent)]).await;
    ready(&b).await;
    let Err(CommandError::Driver(why)) = order(&b.hub, "pause").await.unwrap() else {
        panic!("not a driver answer");
    };
    assert!(why.contains("non confirmé"), "{why}");
    let heard = heard_for(&mut b.vacuum, Duration::from_millis(1500)).await;
    assert_eq!(
        heard.iter().filter(|m| *m == "app_pause").count(),
        1,
        "{heard:?}"
    );
}

#[tokio::test]
async fn orders_leave_in_order_and_one_abandoned_before_leaving_never_leaves() {
    let mut b = bench("roborock.vacuum.s6", &[("app_start", Answer::Hold)]).await;
    ready(&b).await;
    let first = order(&b.hub, "start");
    heard_until(&mut b.vacuum, "app_start").await;
    let second = order(&b.hub, "pause");
    tokio::time::sleep(Duration::from_millis(50)).await;
    second.abort();
    let _ = second.await;
    let third = order(&b.hub, "home");
    tokio::time::sleep(Duration::from_millis(50)).await;
    b.vacuum.release.notify_one();
    assert_eq!(first.await.unwrap(), Ok(()));
    assert_eq!(third.await.unwrap(), Ok(()));
    let heard = heard_for(&mut b.vacuum, Duration::from_millis(300)).await;
    assert!(heard.contains(&"app_charge".to_owned()), "{heard:?}");
    assert!(!heard.contains(&"app_pause".to_owned()), "{heard:?}");
}

#[tokio::test]
async fn another_kind_of_device_is_never_driven() {
    let b = bench("zhimi.airpurifier.m1", &[]).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        !b.hub
            .snapshot()
            .devices
            .iter()
            .any(|d| d.device.id.as_str().starts_with("aspirateur:")),
        "published"
    );
}

#[tokio::test]
async fn an_agent_starting_the_vacuum_waits_for_a_person() {
    let b = bench("roborock.vacuum.s6", &[]).await;
    ready(&b).await;
    let held = b
        .hub
        .command(&point("control"), Value::from("start"), Origin::Mcp, None)
        .await;
    assert!(
        matches!(held, Err(CommandError::NeedsApproval { .. })),
        "{held:?}"
    );
}

#[tokio::test]
async fn an_invalid_status_never_enables_orders() {
    let mut b = bench("roborock.vacuum.s6", &[("get_status", Answer::Invalid)]).await;
    heard_until(&mut b.vacuum, "get_status").await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let snapshot = b.hub.snapshot();
    let vacuum = snapshot
        .devices
        .iter()
        .find(|d| d.device.id.as_str().starts_with("aspirateur:"))
        .unwrap();
    assert_ne!(vacuum.online, Some(true));
    assert!(order(&b.hub, "start").await.unwrap().is_err());
    assert!(
        !heard_for(&mut b.vacuum, Duration::from_millis(100))
            .await
            .iter()
            .any(|m| m.starts_with("app_"))
    );
}

#[tokio::test]
async fn a_response_after_emission_is_journaled_when_the_caller_left() {
    let mut b = bench("roborock.vacuum.s6", &[("app_start", Answer::Hold)]).await;
    ready(&b).await;
    let call = order(&b.hub, "start");
    heard_until(&mut b.vacuum, "app_start").await;
    call.abort();
    let _ = call.await;
    b.vacuum.release.notify_one();
    heard_until(&mut b.vacuum, "get_status").await;
    let outcomes: Vec<_> = b
        .hub
        .journal(50)
        .iter()
        .filter(|entry| matches!(entry.action, moli_core::Action::Command { .. }))
        .map(|entry| entry.outcome.clone())
        .collect();
    assert_eq!(outcomes, [moli_core::Outcome::Ok]);
}

#[tokio::test]
async fn a_full_queue_refuses_immediately() {
    let mut b = bench("roborock.vacuum.s6", &[("app_start", Answer::Hold)]).await;
    ready(&b).await;
    let first = order(&b.hub, "start");
    heard_until(&mut b.vacuum, "app_start").await;
    let mut waiting = Vec::new();
    for _ in 0..QUEUE {
        waiting.push(order(&b.hub, "pause"));
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    let refused = tokio::time::timeout(Duration::from_millis(500), order(&b.hub, "home"))
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(refused, Err(CommandError::Driver(_))),
        "{refused:?}"
    );
    for call in waiting {
        call.abort();
        let _ = call.await;
    }
    b.vacuum.release.notify_one();
    assert_eq!(first.await.unwrap(), Ok(()));
    assert!(
        !heard_for(&mut b.vacuum, Duration::from_millis(100))
            .await
            .iter()
            .any(|m| m.starts_with("app_"))
    );
}
