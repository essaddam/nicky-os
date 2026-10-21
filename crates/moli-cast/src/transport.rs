//! A connection is owned independently of the hub's FIFO executor.
use crate::gateway::{Approved, Gateway};
use moli_androidtv::{
    cast::{Event as MediaEvent, ReceiverOrder, ReceiverVolume},
    tracked::Delivery,
};
use moli_runtime::BoxFuture;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::Instant;
pub(crate) type Guard = Arc<dyn Fn() -> bool + Send + Sync>;
pub(crate) enum Event {
    Ready,
    Lost,
    Media(MediaEvent),
    Volume(ReceiverVolume),
}
pub(crate) trait Wire: Send + Sync {
    fn bind(&self, media: bool) -> Option<Guard>;
    fn send(
        &self,
        order: ReceiverOrder,
        deadline: Instant,
        abandoned: Guard,
    ) -> BoxFuture<'static, Delivery>;
}
pub(crate) struct Link {
    pub wire: Arc<dyn Wire>,
    pub events: mpsc::Receiver<Event>,
    pub run: BoxFuture<'static, ()>,
}
pub(crate) trait Connector: Send + Sync {
    fn open(&self, target: Arc<Approved>, gateway: Gateway) -> Link;
}
pub(crate) trait Stream:
    tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send
{
}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send> Stream for T {}
pub(crate) struct Connected {
    pub stream: Box<dyn Stream>,
    pub pin: String,
}
pub(crate) trait Dialer: Send + Sync {
    fn connect(
        &self,
        host: std::net::Ipv4Addr,
        pin: Option<String>,
    ) -> BoxFuture<'static, Result<Connected, crate::gateway::Error>>;
}
pub(crate) struct Network {
    pub dialer: Arc<dyn Dialer>,
}
impl Default for Network {
    fn default() -> Self {
        Self {
            dialer: Arc::new(Tls),
        }
    }
}
struct Tls;
impl Dialer for Tls {
    fn connect(
        &self,
        host: std::net::Ipv4Addr,
        pin: Option<String>,
    ) -> BoxFuture<'static, Result<Connected, crate::gateway::Error>> {
        Box::pin(async move {
            let tls = moli_net::PinnedTls::new(pin).map_err(|_| crate::gateway::Error::Network)?;
            let stream = tls
                .connect(&host.to_string(), moli_androidtv::cast::PORT)
                .await
                .map_err(|_| crate::gateway::Error::Network)?;
            let pin = tls
                .seen_fingerprint()
                .ok_or(crate::gateway::Error::Network)?;
            Ok(Connected {
                stream: Box::new(stream),
                pin,
            })
        })
    }
}
#[derive(Default)]
struct State {
    ready: std::sync::atomic::AtomicBool,
    epoch: std::sync::atomic::AtomicU64,
    media_epoch: Arc<std::sync::atomic::AtomicU64>,
}
struct Session(Arc<State>);
impl Drop for Session {
    fn drop(&mut self) {
        self.0
            .ready
            .store(false, std::sync::atomic::Ordering::Release);
        self.0
            .epoch
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    }
}
struct NetworkWire {
    orders: mpsc::Sender<moli_androidtv::tracked::TrackedOrder<ReceiverOrder>>,
    state: Arc<State>,
}
impl Wire for NetworkWire {
    fn bind(&self, media: bool) -> Option<Guard> {
        use std::sync::atomic::Ordering;
        if !self.state.ready.load(Ordering::Acquire) {
            return None;
        }
        let epoch = self.state.epoch.load(Ordering::Acquire);
        let media_epoch = self.state.media_epoch.load(Ordering::Acquire);
        let state = self.state.clone();
        Some(Arc::new(move || {
            !state.ready.load(Ordering::Acquire)
                || state.epoch.load(Ordering::Acquire) != epoch
                || (media && state.media_epoch.load(Ordering::Acquire) != media_epoch)
        }))
    }
    fn send(
        &self,
        order: ReceiverOrder,
        deadline: Instant,
        abandoned: Guard,
    ) -> BoxFuture<'static, Delivery> {
        let orders = self.orders.clone();
        let state = self.state.clone();
        Box::pin(async move {
            if Instant::now() >= deadline {
                return Delivery::Expired;
            }
            if abandoned() {
                return Delivery::Abandoned;
            }
            if !state.ready.load(std::sync::atomic::Ordering::Acquire) {
                return Delivery::Unavailable;
            }
            let (order, result) =
                moli_androidtv::tracked::TrackedOrder::new(order, deadline, abandoned);
            if orders.try_send(order).is_err() {
                return Delivery::Unavailable;
            }
            match tokio::time::timeout_at(deadline, result).await {
                Ok(Ok(delivery)) => delivery,
                _ => Delivery::Uncertain,
            }
        })
    }
}
impl Connector for Network {
    fn open(&self, target: Arc<Approved>, gateway: Gateway) -> Link {
        let (orders, rx) = mpsc::channel(4);
        let (events, incoming) = mpsc::channel(32);
        let state = Arc::new(State::default());
        let dialer = self.dialer.clone();
        Link {
            wire: Arc::new(NetworkWire {
                orders,
                state: state.clone(),
            }),
            events: incoming,
            run: Box::pin(async move {
                tokio::select! {()=serve(target.clone(),gateway.clone(),dialer,rx,events,state)=>{},()=invalidated(gateway.selected(),target.generation)=>{}}
            }),
        }
    }
}
async fn invalidated(
    mut selected: tokio::sync::watch::Receiver<Option<Arc<Approved>>>,
    generation: u64,
) {
    loop {
        if selected
            .borrow_and_update()
            .as_ref()
            .is_none_or(|target| target.generation != generation)
        {
            return;
        }
        if selected.changed().await.is_err() {
            return;
        }
    }
}
async fn serve(
    target: Arc<Approved>,
    gateway: Gateway,
    dialer: Arc<dyn Dialer>,
    mut orders: mpsc::Receiver<moli_androidtv::tracked::TrackedOrder<ReceiverOrder>>,
    events: mpsc::Sender<Event>,
    state: Arc<State>,
) {
    loop {
        let current = gateway.selected().borrow().clone();
        let Some(current) = current.filter(|current| current.generation == target.generation)
        else {
            return;
        };
        let result=async {
            let connected=dialer.connect(current.candidate.host,current.pin.clone()).await?;
            // The gateway serializes this pin commit with approval/revocation.
            gateway.pin(current.generation,connected.pin).await?;
            let _session=Session(state.clone());state.ready.store(true,std::sync::atomic::Ordering::Release);
            events.send(Event::Ready).await.map_err(|_|crate::gateway::Error::Unavailable)?;
            let(media,mut incoming)=mpsc::channel(16);let(volumes,mut volume)=mpsc::channel(16);
            let session=moli_androidtv::cast::tracked_receiver_stream(connected.stream,&mut orders,&media,&volumes,state.media_epoch.clone());tokio::pin!(session);
            loop {
                tokio::select! {
                    result=&mut session=>return result.map_err(|_|crate::gateway::Error::Network),
                    event=incoming.recv()=>{let Some(event)=event else{return Err(crate::gateway::Error::Network);};events.send(Event::Media(event)).await.map_err(|_|crate::gateway::Error::Unavailable)?;}
                    sample=volume.recv()=>{let Some(sample)=sample else{return Err(crate::gateway::Error::Network);};events.send(Event::Volume(sample)).await.map_err(|_|crate::gateway::Error::Unavailable)?;}
                }
            }
        }.await;
        let _ = result;
        if events.send(Event::Lost).await.is_err() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
