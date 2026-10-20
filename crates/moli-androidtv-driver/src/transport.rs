//! Connection ownership is separate from the single FIFO command executor.
use crate::pairing::Credential;
use crate::vault::Writes;
use moli_androidtv::{
    cast, remote,
    tracked::{Delivery, TrackedOrder},
};
use moli_runtime::{BoxFuture, ScopedSecrets};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio::sync::{mpsc, watch};
use tokio::time::{Duration, Instant};

pub(crate) type Abandoned = Arc<dyn Fn() -> bool + Send + Sync>;
pub(crate) enum Event {
    Remote(remote::Event),
    Cast(cast::Event),
}
pub(crate) trait Wire: Send + Sync {
    fn remote(
        &self,
        order: remote::Order,
        deadline: Instant,
        abandoned: Abandoned,
    ) -> BoxFuture<'static, Delivery>;
    fn cast(
        &self,
        order: cast::Order,
        deadline: Instant,
        abandoned: Abandoned,
    ) -> BoxFuture<'static, Delivery>;
}
pub(crate) struct Link {
    pub wire: Arc<dyn Wire>,
    pub events: mpsc::Receiver<Event>,
    pub run: BoxFuture<'static, ()>,
}
pub(crate) trait Connector: Send + Sync {
    fn open(
        &self,
        host: String,
        vault: ScopedSecrets,
        credentials: watch::Receiver<Option<Arc<Credential>>>,
        writes: Writes,
    ) -> Link;
}
pub(crate) struct Network;
#[derive(Default)]
struct SessionState {
    ready: AtomicBool,
    generation: AtomicU64,
}
struct SessionLease(Arc<SessionState>);
impl Drop for SessionLease {
    fn drop(&mut self) {
        self.0.ready.store(false, Ordering::Release);
        self.0.generation.fetch_add(1, Ordering::AcqRel);
    }
}
impl SessionState {
    fn enter(self: &Arc<Self>) -> SessionLease {
        self.ready.store(true, Ordering::Release);
        SessionLease(self.clone())
    }
    fn guard(self: &Arc<Self>, request: Abandoned) -> Abandoned {
        let generation = self.generation.load(Ordering::Acquire);
        let state = self.clone();
        Arc::new(move || {
            !state.ready.load(Ordering::Acquire)
                || state.generation.load(Ordering::Acquire) != generation
                || request()
        })
    }
}
struct NetworkWire {
    remote: mpsc::Sender<TrackedOrder<remote::Order>>,
    cast: mpsc::Sender<TrackedOrder<cast::Order>>,
    cast_state: Arc<SessionState>,
}
fn dispatch<T: Send + 'static>(
    sender: mpsc::Sender<TrackedOrder<T>>,
    payload: T,
    deadline: Instant,
    abandoned: Abandoned,
) -> BoxFuture<'static, Delivery> {
    Box::pin(async move {
        if Instant::now() >= deadline {
            return Delivery::Expired;
        }
        if abandoned() {
            return Delivery::Abandoned;
        }
        let (order, answer) = TrackedOrder::new(payload, deadline, abandoned);
        if sender.try_send(order).is_err() {
            return Delivery::Unavailable;
        }
        // The transport owns the absolute deadline once it accepts the order.
        match tokio::time::timeout_at(deadline, answer).await {
            Ok(Ok(delivery)) => delivery,
            _ => Delivery::Uncertain,
        }
    })
}
impl Wire for NetworkWire {
    fn remote(
        &self,
        order: remote::Order,
        deadline: Instant,
        abandoned: Abandoned,
    ) -> BoxFuture<'static, Delivery> {
        dispatch(self.remote.clone(), order, deadline, abandoned)
    }
    fn cast(
        &self,
        order: cast::Order,
        deadline: Instant,
        abandoned: Abandoned,
    ) -> BoxFuture<'static, Delivery> {
        if !self.cast_state.ready.load(Ordering::Acquire) {
            return Box::pin(async { Delivery::Unavailable });
        }
        dispatch(
            self.cast.clone(),
            order,
            deadline,
            self.cast_state.guard(abandoned),
        )
    }
}
impl Connector for Network {
    fn open(
        &self,
        host: String,
        vault: ScopedSecrets,
        credentials: watch::Receiver<Option<Arc<Credential>>>,
        writes: Writes,
    ) -> Link {
        let (remote, remote_rx) = mpsc::channel(4);
        let (cast, cast_rx) = mpsc::channel(4);
        let (events, receiver) = mpsc::channel(32);
        let cast_state = Arc::new(SessionState::default());
        Link {
            wire: Arc::new(NetworkWire {
                remote,
                cast,
                cast_state: cast_state.clone(),
            }),
            events: receiver,
            run: Box::pin(async move {
                // These futures own all sockets. Dropping their owner closes both.
                tokio::join!(
                    remote_loop(host.clone(), credentials, remote_rx, events.clone()),
                    cast_loop(host, vault, cast_rx, events, cast_state, writes)
                );
            }),
        }
    }
}
async fn remote_loop(
    host: String,
    mut credentials: watch::Receiver<Option<Arc<Credential>>>,
    mut orders: mpsc::Receiver<TrackedOrder<remote::Order>>,
    events: mpsc::Sender<Event>,
) {
    loop {
        let identity = credentials.borrow_and_update().clone();
        let Some(identity) = identity else {
            if credentials.changed().await.is_err() {
                return;
            }
            continue;
        };
        let Ok(tls) = moli_net::PinnedTls::with_client_cert(
            Some(identity.pin.clone()),
            identity.identity.certificate_pem(),
            identity.identity.key_pem(),
        ) else {
            return;
        };
        let (tx, mut rx) = mpsc::channel(16);
        {
            let session = remote::tracked_session(&tls, &host, &mut orders, &tx);
            tokio::pin!(session);
            loop {
                tokio::select! {
                    _=&mut session=>break,
                    changed=credentials.changed()=>{if changed.is_err(){return;}break;},
                    event=rx.recv()=>{if let Some(event)=event && events.send(Event::Remote(event)).await.is_err(){return;}}
                }
            }
        }
        if events
            .send(Event::Remote(remote::Event::Lost))
            .await
            .is_err()
        {
            return;
        }
        tokio::select! {()=tokio::time::sleep(Duration::from_secs(5))=>{},_=credentials.changed()=>{}}
    }
}
async fn cast_loop(
    host: String,
    vault: ScopedSecrets,
    mut orders: mpsc::Receiver<TrackedOrder<cast::Order>>,
    events: mpsc::Sender<Event>,
    state: Arc<SessionState>,
    writes: Writes,
) {
    if !vault.available() {
        return;
    }
    loop {
        writes.wait().await;
        let pin = vault.get("cast_pin");
        let result=async {
            let tls=moli_net::PinnedTls::new(pin.clone())?;
            let stream=tls.connect(&host,cast::PORT).await?;
            if pin.is_none() {
                let pin=tls.pinned_fingerprint().ok_or_else(||anyhow::anyhow!("missing cast fingerprint"))?;
                // No media command can use this connection before its TOFU pin is durable.
                let vault=vault.clone();
                writes.start(async move {vault.store("cast_pin",&pin).await}).await
                    .map_err(|_|anyhow::anyhow!("cast pin writer stopped"))??;
            }
            let _lease=state.enter();
            let (tx,mut rx)=mpsc::channel(16);
            let session=cast::tracked_stream(stream,&mut orders,&tx);
            tokio::pin!(session);
            loop {
                tokio::select! {
                    result=&mut session=>return result,
                    event=rx.recv()=>{if let Some(event)=event && events.send(Event::Cast(event)).await.is_err(){return Ok(());}}
                }
            }
        }.await;
        let _ = result;
        if events.send(Event::Cast(cast::Event::Lost)).await.is_err() {
            return;
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn disconnected_cast_refuses_instead_of_queueing_until_a_future_connection() {
        let (remote, _remote_rx) = mpsc::channel(4);
        let (cast, mut cast_rx) = mpsc::channel(4);
        let wire = NetworkWire {
            remote,
            cast,
            cast_state: Arc::new(SessionState::default()),
        };
        let result = wire
            .cast(
                cast::Order::Pause,
                Instant::now() + Duration::from_millis(10),
                Arc::new(|| false),
            )
            .await;
        assert_eq!(result, Delivery::Unavailable);
        assert!(cast_rx.try_recv().is_err());
    }
}

#[cfg(test)]
mod reconnect_tests {
    use super::*;
    #[tokio::test]
    async fn cast_order_cannot_cross_a_session_generation_before_first_poll() {
        let (remote, _remote_rx) = mpsc::channel(4);
        let (cast, mut cast_rx) = mpsc::channel(4);
        let state = Arc::new(SessionState::default());
        let wire = NetworkWire {
            remote,
            cast,
            cast_state: state.clone(),
        };
        let previous = state.enter();
        let pending = wire.cast(
            cast::Order::Pause,
            Instant::now() + Duration::from_secs(1),
            Arc::new(|| false),
        );
        drop(previous);
        let _next = state.enter();
        assert_eq!(pending.await, Delivery::Abandoned);
        assert!(cast_rx.try_recv().is_err());
    }
}
