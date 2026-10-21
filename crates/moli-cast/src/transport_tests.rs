use super::*;
use crate::{
    Config,
    discovery::Candidate,
    gateway::{Discover, Error, Gateway, Phase, Vault},
    ownership::Ownership,
};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
use tokio::io::AsyncReadExt;
use tokio::sync::Notify;
#[derive(Default)]
struct Memory {
    raw: Mutex<Option<String>>,
    hold: AtomicBool,
    entered: Notify,
    release: Notify,
}
impl Vault for Memory {
    fn available(&self) -> bool {
        true
    }
    fn get(&self) -> Option<String> {
        self.raw.lock().unwrap().clone()
    }
    fn store(&self, value: String) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move {
            if self.hold.load(Ordering::SeqCst) && value.contains("\"pin\":\"") {
                self.entered.notify_one();
                self.release.notified().await;
            }
            *self.raw.lock().unwrap() = Some(value);
            Ok(())
        })
    }
}
struct Catalog;
fn candidate() -> Candidate {
    Candidate {
        id: "00000000000000000000000000000001".into(),
        name: "Example screen".into(),
        host: "192.168.1.10".parse().unwrap(),
        hostname: "screen.local".into(),
        video: true,
    }
}
impl Discover for Catalog {
    fn discover(&self, _: Config) -> BoxFuture<'static, Result<Vec<Candidate>, Error>> {
        Box::pin(async { Ok(vec![candidate()]) })
    }
}
struct FakeDialer {
    peers: mpsc::Sender<tokio::io::DuplexStream>,
    expected: Arc<Mutex<Vec<Option<String>>>>,
    pin: String,
}
impl Dialer for FakeDialer {
    fn connect(
        &self,
        _: std::net::Ipv4Addr,
        pin: Option<String>,
    ) -> BoxFuture<'static, Result<Connected, Error>> {
        let expected = self.expected.clone();
        let peers = self.peers.clone();
        let fingerprint = self.pin.clone();
        Box::pin(async move {
            expected.lock().unwrap().push(pin);
            let (stream, peer) = tokio::io::duplex(8192);
            peers.send(peer).await.unwrap();
            Ok(Connected {
                stream: Box::new(stream),
                pin: fingerprint,
            })
        })
    }
}
async fn approved(vault: Arc<Memory>) -> (Gateway, tokio::task::JoinHandle<()>) {
    let gateway = Gateway::new(
        Config {
            host: Some(candidate().host),
            interface: None,
            prefix: 24,
            external_ids: Vec::new(),
        },
        Ownership::default(),
    );
    let g = gateway.clone();
    let task = tokio::spawn(async move {
        g.run(vault, Arc::new(Catalog)).await;
    });
    for _ in 0..100 {
        if gateway.state().phase != Phase::Unavailable {
            break;
        }
        tokio::task::yield_now().await;
    }
    let view = gateway.discover().await.unwrap();
    gateway
        .approve(view.generation, candidate().id, true)
        .await
        .unwrap();
    (gateway, task)
}
#[tokio::test(start_paused = true)]
async fn tls_pin_is_durable_before_any_cast_protocol_or_ready_event() {
    let vault = Arc::new(Memory::default());
    let (gateway, actor) = approved(vault.clone()).await;
    vault.hold.store(true, Ordering::SeqCst);
    let (peers, mut incoming) = mpsc::channel(8);
    let expected = Arc::new(Mutex::new(Vec::new()));
    let network = Network {
        dialer: Arc::new(FakeDialer {
            peers,
            expected: expected.clone(),
            pin: "aa".repeat(32),
        }),
    };
    let target = gateway.selected().borrow().clone().unwrap();
    let mut link = network.open(target, gateway.clone());
    let task = tokio::spawn(link.run);
    tokio::time::timeout(std::time::Duration::from_secs(1), vault.entered.notified())
        .await
        .expect("pin persistence must precede readiness");
    let mut peer = incoming.recv().await.unwrap();
    assert!(link.wire.bind(false).is_none());
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(1), peer.read_u32())
            .await
            .is_err()
    );
    assert!(link.events.try_recv().is_err());
    vault.release.notify_one();
    assert!(matches!(link.events.recv().await, Some(Event::Ready)));
    assert!(link.wire.bind(false).is_some());
    assert!(vault.get().unwrap().contains(&"aa".repeat(32)));
    assert!(peer.read_u32().await.unwrap() > 0);
    assert_eq!(*expected.lock().unwrap(), [None]);
    task.abort();
    let _ = task.await;
    actor.abort();
    let _ = actor.await;
}
#[tokio::test(start_paused = true)]
async fn a_different_pin_cannot_make_a_reconnected_receiver_ready() {
    let vault = Arc::new(Memory::default());
    let (gateway, actor) = approved(vault).await;
    let target = gateway.selected().borrow().clone().unwrap();
    gateway
        .pin(target.generation, "aa".repeat(32))
        .await
        .unwrap();
    let (peers, mut incoming) = mpsc::channel(8);
    let expected = Arc::new(Mutex::new(Vec::new()));
    let network = Network {
        dialer: Arc::new(FakeDialer {
            peers,
            expected: expected.clone(),
            pin: "bb".repeat(32),
        }),
    };
    let target = gateway.selected().borrow().clone().unwrap();
    let mut link = network.open(target, gateway.clone());
    let task = tokio::spawn(link.run);
    let event = tokio::time::timeout(std::time::Duration::from_secs(1), link.events.recv())
        .await
        .unwrap();
    assert!(matches!(event, Some(Event::Lost)));
    assert!(link.wire.bind(false).is_none());
    let mut peer = incoming.recv().await.unwrap();
    assert!(peer.read_u32().await.is_err());
    assert_eq!(*expected.lock().unwrap(), [Some("aa".repeat(32))]);
    task.abort();
    let _ = task.await;
    actor.abort();
    let _ = actor.await;
}

#[tokio::test(start_paused = true)]
async fn bound_orders_are_checked_again_at_their_first_poll_without_emission() {
    use std::sync::atomic::Ordering;
    let state = Arc::new(State::default());
    state.ready.store(true, Ordering::Release);
    let (orders, mut pending) = mpsc::channel(4);
    let wire = NetworkWire {
        orders,
        state: state.clone(),
    };
    let media = wire.bind(true).unwrap();
    let volume = wire.bind(false).unwrap();
    let send = wire.send(
        ReceiverOrder::Media(moli_androidtv::cast::Order::Pause),
        Instant::now() + std::time::Duration::from_secs(5),
        media.clone(),
    );
    state.media_epoch.fetch_add(1, Ordering::AcqRel);
    assert!(media());
    assert!(!volume());
    assert_eq!(send.await, Delivery::Abandoned);
    assert!(pending.try_recv().is_err());
    let send = wire.send(
        ReceiverOrder::Volume(0.5),
        Instant::now() + std::time::Duration::from_secs(1),
        volume.clone(),
    );
    tokio::time::advance(std::time::Duration::from_secs(2)).await;
    assert_eq!(send.await, Delivery::Expired);
    assert!(pending.try_recv().is_err());
    state.epoch.fetch_add(1, Ordering::AcqRel);
    assert!(volume());
    state.ready.store(false, Ordering::Release);
    assert!(wire.bind(false).is_none());
}
