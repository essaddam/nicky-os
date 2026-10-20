//! Human pairing control. The public view contains no code or credentials.
use moli_androidtv::pairing::{Identity, PairingError};
use moli_runtime::{BoxFuture, ScopedSecrets};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{Semaphore, mpsc, oneshot, watch};
use tokio::time::{Duration, Instant};

const WINDOW: Duration = Duration::from_secs(120);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Unavailable,
    Idle,
    Preparing,
    AwaitingCode,
    Verifying,
    Persisting,
    Paired,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Error {
    Unavailable,
    Busy,
    Stale,
    InvalidCode,
    Refused,
    Expired,
    Network,
    Vault,
}

#[derive(Clone, Debug, Serialize)]
pub struct View {
    pub phase: Phase,
    pub generation: u64,
    pub expires_at: Option<u64>,
    pub error: Option<Error>,
}
impl Default for View {
    fn default() -> Self {
        Self {
            phase: Phase::Unavailable,
            generation: 0,
            expires_at: None,
            error: None,
        }
    }
}

pub(crate) struct Credential {
    pub identity: Arc<Identity>,
    pub pin: String,
}
impl Credential {
    fn encode(&self) -> Result<String, Error> {
        serde_json::to_string(&Saved {
            version: 1,
            cert: self.identity.certificate_pem().into(),
            key: self.identity.key_pem().into(),
            remote_pin: self.pin.clone(),
        })
        .map_err(|_| Error::Vault)
    }
    fn decode(raw: &str) -> Option<Self> {
        if raw.len() > 40 * 1024 {
            return None;
        }
        let saved: Saved = serde_json::from_str(raw).ok()?;
        if saved.version != 1
            || saved.remote_pin.len() != 64
            || !saved.remote_pin.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return None;
        }
        Some(Self {
            identity: Arc::new(Identity::from_pem(saved.cert, saved.key).ok()?),
            pin: saved.remote_pin,
        })
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    version: u8,
    cert: String,
    key: String,
    remote_pin: String,
}

pub(crate) trait Vault: Send + Sync {
    fn available(&self) -> bool;
    fn get(&self) -> Option<String>;
    fn store<'a>(&'a self, value: &'a str) -> BoxFuture<'a, Result<(), Error>>;
}
impl Vault for ScopedSecrets {
    fn available(&self) -> bool {
        self.available()
    }
    fn get(&self) -> Option<String> {
        self.get("identity")
    }
    fn store<'a>(&'a self, value: &'a str) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            self.store("identity", value)
                .await
                .map_err(|_| Error::Vault)
        })
    }
}
pub(crate) trait HumanSession: Send {
    fn finish(self: Box<Self>, code: String) -> BoxFuture<'static, Result<(), Error>>;
}
pub(crate) struct Prepared {
    credential: Arc<Credential>,
    session: Box<dyn HumanSession>,
}
pub(crate) trait Factory: Send + Sync {
    fn prepare(&self) -> BoxFuture<'static, Result<Prepared, Error>>;
}

struct Request {
    action: Action,
    reply: oneshot::Sender<Result<View, Error>>,
}
enum Action {
    Start,
    Submit(u64, String),
    Cancel(u64),
}
struct Inner {
    sender: Mutex<Option<mpsc::Sender<Request>>>,
    view: watch::Sender<View>,
    credentials: watch::Sender<Option<Arc<Credential>>>,
    counter: AtomicU64,
    generation_slot: Arc<Semaphore>,
    writes: crate::vault::Writes,
}
#[derive(Clone)]
pub struct PairingGateway(Arc<Inner>);
impl std::fmt::Debug for PairingGateway {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PairingGateway")
            .field("state", &self.state())
            .finish()
    }
}
impl Default for PairingGateway {
    fn default() -> Self {
        Self::new()
    }
}
impl PairingGateway {
    #[must_use]
    pub fn new() -> Self {
        Self(Arc::new(Inner {
            sender: Mutex::new(None),
            view: watch::channel(View::default()).0,
            credentials: watch::channel(None).0,
            counter: AtomicU64::new(0),
            generation_slot: Arc::new(Semaphore::new(1)),
            writes: crate::vault::Writes::default(),
        }))
    }
    #[must_use]
    pub fn state(&self) -> View {
        self.0.view.borrow().clone()
    }
    async fn request(&self, action: Action) -> Result<View, Error> {
        if self.state().phase == Phase::Unavailable {
            return Err(Error::Unavailable);
        }
        let sender = self
            .0
            .sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .ok_or(Error::Unavailable)?;
        let (reply, answer) = oneshot::channel();
        sender
            .try_send(Request { action, reply })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => Error::Busy,
                mpsc::error::TrySendError::Closed(_) => Error::Unavailable,
            })?;
        answer.await.map_err(|_| Error::Unavailable)?
    }
    pub async fn start(&self) -> Result<View, Error> {
        self.request(Action::Start).await
    }
    pub async fn submit(&self, generation: u64, code: String) -> Result<View, Error> {
        if code.len() != 6 || !code.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::InvalidCode);
        }
        self.request(Action::Submit(generation, code)).await
    }
    pub async fn cancel(&self, generation: u64) -> Result<View, Error> {
        self.request(Action::Cancel(generation)).await
    }
    pub(crate) fn writes(&self) -> crate::vault::Writes {
        self.0.writes.clone()
    }
    pub(crate) fn credentials(&self) -> watch::Receiver<Option<Arc<Credential>>> {
        self.0.credentials.subscribe()
    }
    pub(crate) async fn run(&self, vault: Arc<dyn Vault>, factory: Arc<dyn Factory>) {
        actor(self, vault, factory).await;
    }
    pub(crate) fn network_factory(&self, host: String) -> Arc<dyn Factory> {
        Arc::new(NetworkFactory {
            host,
            slot: self.0.generation_slot.clone(),
        })
    }
}

pub(crate) struct NetworkFactory {
    pub host: String,
    pub slot: Arc<Semaphore>,
}
impl Factory for NetworkFactory {
    fn prepare(&self) -> BoxFuture<'static, Result<Prepared, Error>> {
        let host = self.host.clone();
        let slot = self.slot.clone();
        Box::pin(async move {
            let permit = slot.acquire_owned().await.map_err(|_| Error::Unavailable)?;
            let identity = tokio::task::spawn_blocking(move || {
                let _permit = permit;
                Identity::generate()
            })
            .await
            .map_err(|_| Error::Unavailable)?
            .map_err(map_error)?;
            let identity = Arc::new(identity);
            let tls = moli_net::PinnedTls::new(None).map_err(|_| Error::Network)?;
            let session = moli_androidtv::pairing::PairingSession::start(&tls, &host, &identity)
                .await
                .map_err(map_error)?;
            let pin = session.peer_fingerprint().ok_or(Error::Network)?.to_owned();
            Ok(Prepared {
                credential: Arc::new(Credential { identity, pin }),
                session: Box::new(FinishSession(Some(Box::new(move |code| {
                    Box::pin(async move {
                        let mut session = session;
                        session.finish(&code).await.map_err(map_error)
                    })
                })))),
            })
        })
    }
}

#[cfg(test)]
#[path = "pairing_tests.rs"]
mod tests;

type Finish = Box<dyn FnOnce(String) -> BoxFuture<'static, Result<(), Error>> + Send>;
struct FinishSession(Option<Finish>);
impl HumanSession for FinishSession {
    fn finish(mut self: Box<Self>, code: String) -> BoxFuture<'static, Result<(), Error>> {
        self.0.take().expect("owned pairing session")(code)
    }
}
fn map_error(error: PairingError) -> Error {
    match error {
        PairingError::InvalidCode => Error::InvalidCode,
        PairingError::Refused => Error::Refused,
        PairingError::Expired => Error::Expired,
        PairingError::InvalidKey | PairingError::Protocol | PairingError::Disconnected => {
            Error::Network
        }
    }
}

struct Reset(PairingGateway);
impl Drop for Reset {
    fn drop(&mut self) {
        *self
            .0
            .0
            .sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        self.0.0.credentials.send_replace(None);
        self.0.0.view.send_modify(|view| {
            view.phase = Phase::Unavailable;
            view.expires_at = None;
        });
    }
}
struct Attempt {
    generation: u64,
    deadline: Instant,
    prepared: Option<Prepared>,
    job: tokio::task::AbortHandle,
    reply: Option<oneshot::Sender<Result<View, Error>>>,
}
enum Completion {
    Prepared(Result<Prepared, Error>),
    Verified(Result<Arc<Credential>, Error>),
    Stored(Result<Arc<Credential>, Error>),
}
fn fail(gateway: &PairingGateway, attempt: &mut Option<Attempt>, error: Error) {
    if let Some(attempt) = attempt.take() {
        attempt.job.abort();
        if let Some(reply) = attempt.reply {
            let _ = reply.send(Err(error));
        }
    }
    gateway.0.view.send_modify(|view| {
        view.phase = Phase::Failed;
        view.expires_at = None;
        view.error = Some(error);
    });
}
async fn actor(gateway: &PairingGateway, vault: Arc<dyn Vault>, factory: Arc<dyn Factory>) {
    let (sender, mut requests) = mpsc::channel::<Request>(4);
    // A supervisor owns one actor. Refuse a second actor for the same gateway.
    {
        let mut previous = gateway
            .0
            .sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if previous.as_ref().is_some_and(|sender| !sender.is_closed()) {
            return;
        }
        *previous = Some(sender);
    }
    let _reset = Reset(gateway.clone());
    gateway.0.writes.wait().await;
    let loaded = vault
        .get()
        .as_deref()
        .and_then(Credential::decode)
        .map(Arc::new);
    gateway.0.credentials.send_replace(loaded.clone());
    gateway.0.view.send_modify(|view| {
        view.phase = if loaded.is_some() {
            Phase::Paired
        } else {
            Phase::Idle
        };
        view.expires_at = None;
        view.error = None;
    });
    let mut attempt: Option<Attempt> = None;
    let mut jobs = tokio::task::JoinSet::new();
    loop {
        let deadline = attempt
            .as_ref()
            .map_or_else(|| Instant::now() + WINDOW, |attempt| attempt.deadline);
        let committing = gateway.state().phase == Phase::Persisting;
        tokio::select! {
            biased;
            ()=tokio::time::sleep_until(deadline),if attempt.is_some() && !committing => {fail(gateway,&mut attempt,Error::Expired);}
            request=requests.recv()=> {
                let Some(request)=request else {return;};
                if request.reply.is_closed() {continue;}
                match request.action {
                    Action::Start=> {
                        let error=if attempt.is_some() || gateway.0.credentials.borrow().is_some(){Some(Error::Busy)}else if !vault.available(){Some(Error::Vault)}else{None};
                        if let Some(error)=error {let _=request.reply.send(Err(error));continue;}
                        let generation=gateway.0.counter.fetch_add(1,Ordering::Relaxed)+1;
                        gateway.0.view.send_replace(View {phase:Phase::Preparing,generation,expires_at:Some(moli_core::now_ms().saturating_add(120_000)),error:None});
                        let factory=factory.clone();
                        let job=jobs.spawn(async move {(generation,Completion::Prepared(factory.prepare().await))});
                        attempt=Some(Attempt {generation,deadline:Instant::now()+WINDOW,prepared:None,job,reply:None});
                        let _=request.reply.send(Ok(gateway.state()));
                    }
                    Action::Cancel(generation)=> {
                        if committing || attempt.as_ref().is_none_or(|active|active.generation!=generation) {let _=request.reply.send(Err(Error::Stale));continue;}
                        fail(gateway,&mut attempt,Error::Stale);
                        gateway.0.view.send_modify(|view| {view.phase=if gateway.0.credentials.borrow().is_some(){Phase::Paired}else{Phase::Idle};view.error=None;});
                        let _=request.reply.send(Ok(gateway.state()));
                    }
                    Action::Submit(generation,code)=> {
                        let Some(active)=attempt.as_mut().filter(|active|active.generation==generation) else {let _=request.reply.send(Err(Error::Stale));continue;};
                        if gateway.state().phase!=Phase::AwaitingCode {let _=request.reply.send(Err(Error::Busy));continue;}
                        let prepared=active.prepared.take().expect("awaiting owns its session");
                        active.reply=Some(request.reply);
                        gateway.0.view.send_modify(|view|view.phase=Phase::Verifying);
                        active.job=jobs.spawn(async move {
                            let result=prepared.session.finish(code).await.map(|()|prepared.credential);
                            (generation,Completion::Verified(result))
                        });
                    }
                }
            }
            completed=jobs.join_next(),if !jobs.is_empty()=> {
                let Some(Ok((generation,completed)))=completed else {continue;};
                let Some(active)=attempt.as_mut().filter(|active|active.generation==generation) else {continue;};
                match completed {
                    Completion::Prepared(Ok(prepared))=> {
                        active.prepared=Some(prepared);gateway.0.view.send_modify(|view|view.phase=Phase::AwaitingCode);
                    }
                    Completion::Verified(Ok(credential))=> {
                        let encoded=match credential.encode() {Ok(encoded)=>encoded,Err(error)=>{fail(gateway,&mut attempt,error);continue;}};
                        // Commit linearizes here: cancellation is no longer acknowledged.
                        gateway.0.view.send_modify(|view|view.phase=Phase::Persisting);
                        let vault=vault.clone();
                        // The commit owns its continuation through disk and cache update.
                        // Cancelling this actor drops its monitor, not the accepted write.
                        let commit=gateway.0.writes.start(async move {vault.store(&encoded).await.map(|()|credential)});
                        active.job=jobs.spawn(async move {(generation,Completion::Stored(commit.await.unwrap_or(Err(Error::Vault))))});
                    }
                    Completion::Stored(Ok(credential))=> {
                        gateway.0.credentials.send_replace(Some(credential));
                        gateway.0.view.send_modify(|view| {view.phase=Phase::Paired;view.expires_at=None;view.error=None;});
                        if let Some(reply)=attempt.take().and_then(|attempt|attempt.reply) {let _=reply.send(Ok(gateway.state()));}
                    }
                    Completion::Prepared(Err(error))|Completion::Verified(Err(error))|Completion::Stored(Err(error))=>fail(gateway,&mut attempt,error),
                }
            }
        }
    }
}
