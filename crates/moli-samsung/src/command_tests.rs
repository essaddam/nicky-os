use super::*;
use moli_core::{DeviceId, InstanceId, Origin, PointId, Value};
use moli_runtime::{Driver, Hub, HubOptions};
use std::time::Duration;
use tokio::sync::{Notify, mpsc};
use tokio_util::sync::CancellationToken;

struct Fake {
    sent: mpsc::UnboundedSender<String>,
    release: Notify,
}
impl CommandSink for Fake {
    fn send<'a>(&'a self, c: &'a CommandRequest) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            if c.abandoned() {
                return Err("abandoned".into());
            }
            if let Value::Text(v) = &c.value {
                self.sent.send(v.to_string()).unwrap();
                if matches!(&**v, "Home" | "Info") {
                    self.release.notified().await;
                }
                if &**v == "Info" {
                    return Err("synthetic refusal".into());
                }
            }
            Ok(())
        })
    }
}
struct TestDriver(Arc<dyn CommandSink>);
impl Driver for TestDriver {
    fn kind(&self) -> &'static str {
        "samsung"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let id = ctx.device_id("native");
            ctx.upsert_device(crate::device::device(ctx, &id));
            ctx.set_availability(&id, true);
            ctx.ready();
            commands(ctx, &id, self.0.clone()).await;
            Ok(())
        })
    }
}
async fn setup(
    timeout: Duration,
) -> (
    Hub,
    Arc<Fake>,
    mpsc::UnboundedReceiver<String>,
    CancellationToken,
) {
    setup_guard(timeout, moli_runtime::guard::GuardPolicy::default()).await
}
async fn setup_guard(
    timeout: Duration,
    guard: moli_runtime::guard::GuardPolicy,
) -> (
    Hub,
    Arc<Fake>,
    mpsc::UnboundedReceiver<String>,
    CancellationToken,
) {
    let hub = Hub::new(HubOptions {
        command_timeout: timeout,
        guard,
        ..HubOptions::default()
    })
    .unwrap();
    let (sent, rx) = mpsc::unbounded_channel();
    let fake = Arc::new(Fake {
        sent,
        release: Notify::new(),
    });
    let cancel = CancellationToken::new();
    moli_runtime::spawn_driver(
        &hub,
        InstanceId::from("tv"),
        Arc::new(TestDriver(fake.clone())),
        cancel.clone(),
    );
    for _ in 0..200 {
        if hub.describe(&DeviceId::from("tv:native")).is_some() {
            return (hub, fake, rx, cancel);
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    panic!("not ready");
}
fn order(hub: &Hub, key: &str) -> tokio::task::JoinHandle<Result<(), moli_runtime::CommandError>> {
    let hub = hub.clone();
    let key = key.to_owned();
    tokio::spawn(async move {
        hub.command(
            &PointId::from("tv:native/key"),
            Value::from(key.as_str()),
            Origin::Ui,
            None,
        )
        .await
    })
}

#[tokio::test]
async fn fifo_one_in_flight_and_a_cancelled_queued_order_never_leaves() {
    let (hub, fake, mut rx, cancel) = setup(Duration::from_secs(2)).await;
    let first = order(&hub, "Home");
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .unwrap()
            .unwrap(),
        "Home"
    );
    let abandoned = order(&hub, "Mute");
    tokio::time::sleep(Duration::from_millis(10)).await;
    abandoned.abort();
    let _ = abandoned.await;
    let next = order(&hub, "Back");
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(rx.try_recv().is_err());
    fake.release.notify_one();
    assert!(first.await.unwrap().is_ok());
    assert!(next.await.unwrap().is_ok());
    assert_eq!(rx.recv().await.unwrap(), "Back");
    assert!(rx.try_recv().is_err());
    cancel.cancel();
}

#[tokio::test]
async fn pending_callers_are_bounded_and_expired_orders_are_not_replayed() {
    let (hub, _fake, mut rx, cancel) = setup(Duration::from_millis(150)).await;
    let first = order(&hub, "Home");
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .unwrap()
            .unwrap(),
        "Home"
    );
    let mut queued = Vec::new();
    for _ in 0..4 {
        queued.push(order(&hub, "Mute"));
        tokio::time::sleep(Duration::from_millis(3)).await;
    }
    let overflow = order(&hub, "Back");
    assert!(
        tokio::time::timeout(Duration::from_millis(60), overflow)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    assert!(first.await.unwrap().is_err());
    for task in queued {
        let _ = task.await;
    }
    while let Ok(key) = rx.try_recv() {
        assert_eq!(key, "Mute");
    }
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(rx.try_recv().is_err());
    cancel.cancel();
}

#[tokio::test]
async fn caller_leaving_after_emission_preserves_success_in_journal() {
    let (hub, fake, mut rx, cancel) = setup(Duration::from_secs(2)).await;
    let first = order(&hub, "Home");
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .unwrap()
            .unwrap(),
        "Home"
    );
    first.abort();
    let _ = first.await;
    fake.release.notify_one();
    for _ in 0..200 {
        if hub
            .journal(10)
            .iter()
            .any(|e| matches!(e.outcome, moli_core::Outcome::Ok))
        {
            cancel.cancel();
            return;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    cancel.cancel();
    panic!("success missing from journal");
}

#[tokio::test]
async fn guard_holds_agent_commands_before_the_transport() {
    let (hub, _fake, mut rx, cancel) = setup_guard(
        Duration::from_secs(1),
        moli_runtime::guard::GuardPolicy {
            protect_unassigned: true,
            ..moli_runtime::guard::GuardPolicy::default()
        },
    )
    .await;
    let result = hub
        .command(
            &PointId::from("tv:native/key"),
            Value::from("Mute"),
            Origin::Mcp,
            None,
        )
        .await;
    assert!(matches!(
        result,
        Err(moli_runtime::CommandError::NeedsApproval { .. })
    ));
    assert!(rx.try_recv().is_err());
    cancel.cancel();
}

#[tokio::test]
async fn shutdown_marks_device_offline_and_never_sends_the_remaining_queue() {
    let (hub, fake, mut rx, cancel) = setup(Duration::from_secs(1)).await;
    let first = order(&hub, "Home");
    assert_eq!(rx.recv().await.unwrap(), "Home");
    let next = order(&hub, "Back");
    tokio::time::sleep(Duration::from_millis(10)).await;
    cancel.cancel();
    fake.release.notify_one();
    let _ = first.await;
    let _ = next.await;
    for _ in 0..100 {
        if hub
            .snapshot()
            .devices
            .iter()
            .any(|d| d.device.id == DeviceId::from("tv:native") && d.online == Some(false))
        {
            assert!(rx.try_recv().is_err());
            return;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    panic!("device left online");
}

#[tokio::test]
async fn real_remote_sends_one_whitelisted_click_and_never_publishes_a_requested_state() {
    let (port, mut requests, server) = crate::transport::tests::server().await;
    let config = Config {
        host: "127.0.0.1".into(),
        port,
        upnp_port: 1,
        mac: None,
        every: 30,
    };
    let tv = Tv {
        config,
        remote: Mutex::new(crate::transport::Remote {
            host: "127.0.0.1".into(),
            port,
            tls: moli_net::PinnedTls::new(None).unwrap(),
            token: "12345678".into(),
        }),
    };
    let hub = Hub::new(HubOptions::default()).unwrap();
    let cancel = CancellationToken::new();
    moli_runtime::spawn_driver(
        &hub,
        InstanceId::from("tv"),
        Arc::new(TestDriver(Arc::new(tv))),
        cancel.clone(),
    );
    for _ in 0..200 {
        if hub.describe(&DeviceId::from("tv:native")).is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    assert!(order(&hub, "Mute").await.unwrap().is_ok());
    let handshake = requests.recv().await.unwrap();
    assert!(handshake.starts_with(
        "GET /api/v2/channels/samsung.remote.control?name=TW9saQ==&token=12345678 HTTP/1.1"
    ));
    let payload: serde_json::Value = serde_json::from_str(&requests.recv().await.unwrap()).unwrap();
    assert_eq!(payload["method"], "ms.remote.control");
    assert_eq!(payload["params"]["DataOfCmd"], "KEY_MUTE");
    assert_eq!(payload["params"]["Cmd"], "Click");
    assert!(hub.state(&PointId::from("tv:native/mute")).is_none());
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(requests.try_recv().is_err());
    assert!(order(&hub, "Back").await.unwrap().is_ok());
    let handshake = tokio::time::timeout(Duration::from_secs(1), requests.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(handshake.starts_with("GET /api/v2/channels/"));
    let payload: serde_json::Value = serde_json::from_str(&requests.recv().await.unwrap()).unwrap();
    assert_eq!(payload["params"]["DataOfCmd"], "KEY_RETURN");
    cancel.cancel();
    server.abort();
}

#[tokio::test]
async fn failure_after_caller_departure_is_journaled_and_emits_a_notice() {
    let (hub, fake, mut rx, cancel) = setup(Duration::from_secs(2)).await;
    let mut events = hub.subscribe();
    let task = order(&hub, "Info");
    assert_eq!(rx.recv().await.unwrap(), "Info");
    task.abort();
    let _ = task.await;
    fake.release.notify_one();
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if matches!(
                events.recv().await.unwrap(),
                moli_core::Event::Notice { .. }
            ) {
                break;
            }
        }
    })
    .await
    .unwrap();
    for _ in 0..100 {
        if hub.journal(10).iter().any(|e|matches!(&e.outcome,moli_core::Outcome::Err(message) if message.contains("synthetic refusal"))) {cancel.cancel();return;}
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    cancel.cancel();
    panic!("failure missing from journal");
}
