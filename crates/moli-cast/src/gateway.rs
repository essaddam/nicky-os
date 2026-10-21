//! Explicit human activation. Public views never contain a TLS pin.
use crate::{
    Config,
    discovery::Candidate,
    ownership::{Lease, Ownership},
    vault::Writes,
};
use moli_runtime::{BoxFuture, ScopedSecrets};
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use tokio::sync::{mpsc, oneshot, watch};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Unavailable,
    Idle,
    Discovering,
    Ready,
    Persisting,
    Active,
    Failed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Error {
    Unavailable,
    Busy,
    Stale,
    Invalid,
    Network,
    Vault,
    OwnedElsewhere,
}
#[derive(Clone, Debug, Serialize)]
pub struct View {
    pub phase: Phase,
    pub generation: u64,
    pub candidates: Vec<Candidate>,
    pub error: Option<Error>,
}
#[derive(Clone, Debug)]
pub(crate) struct Approved {
    pub generation: u64,
    pub candidate: Candidate,
    pub pin: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    version: u8,
    approved: Option<SavedTarget>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedTarget {
    candidate: Candidate,
    pin: Option<String>,
}
pub(crate) trait Vault: Send + Sync {
    fn available(&self) -> bool;
    fn get(&self) -> Option<String>;
    fn store(&self, value: String) -> BoxFuture<'_, Result<(), Error>>;
}
impl Vault for ScopedSecrets {
    fn available(&self) -> bool {
        self.available()
    }
    fn get(&self) -> Option<String> {
        self.get("approval")
    }
    fn store(&self, value: String) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move {
            self.store("approval", &value)
                .await
                .map_err(|_| Error::Vault)
        })
    }
}
pub(crate) trait Discover: Send + Sync {
    fn discover(&self, config: Config) -> BoxFuture<'static, Result<Vec<Candidate>, Error>>;
}
pub(crate) struct NetworkDiscovery;
impl Discover for NetworkDiscovery {
    fn discover(&self, config: Config) -> BoxFuture<'static, Result<Vec<Candidate>, Error>> {
        Box::pin(async move {
            crate::discovery::discover(&config)
                .await
                .map_err(|_| Error::Network)
        })
    }
}
enum Action {
    Discover,
    Approve {
        generation: u64,
        candidate: String,
        exclusive: bool,
    },
    Revoke(u64),
    Pin(u64, String),
}
struct Request {
    action: Action,
    reply: oneshot::Sender<Result<View, Error>>,
}
struct Inner {
    config: Config,
    owners: Ownership,
    state: watch::Sender<View>,
    active: watch::Sender<Option<Arc<Approved>>>,
    sender: Mutex<Option<mpsc::Sender<Request>>>,
    counter: AtomicU64,
    writes: Writes,
}
#[derive(Clone)]
pub struct Gateway(Arc<Inner>);
impl std::fmt::Debug for Gateway {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CastGateway")
            .field("state", &self.state())
            .finish()
    }
}
impl Gateway {
    pub fn new(config: Config, ownership: Ownership) -> Self {
        Self(Arc::new(Inner {
            config,
            owners: ownership,
            state: watch::channel(View {
                phase: Phase::Unavailable,
                generation: 0,
                candidates: Vec::new(),
                error: None,
            })
            .0,
            active: watch::channel(None).0,
            sender: Mutex::new(None),
            counter: AtomicU64::new(0),
            writes: Writes::default(),
        }))
    }
    pub fn state(&self) -> View {
        self.0.state.borrow().clone()
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
        tokio::time::timeout(std::time::Duration::from_secs(5), answer)
            .await
            .map_err(|_| Error::Busy)?
            .map_err(|_| Error::Unavailable)?
    }
    pub async fn discover(&self) -> Result<View, Error> {
        self.request(Action::Discover).await
    }
    pub async fn approve(
        &self,
        generation: u64,
        candidate: String,
        exclusive: bool,
    ) -> Result<View, Error> {
        if !exclusive || candidate.len() > 64 {
            return Err(Error::Invalid);
        }
        self.request(Action::Approve {
            generation,
            candidate,
            exclusive,
        })
        .await
    }
    pub async fn revoke(&self, generation: u64) -> Result<View, Error> {
        self.request(Action::Revoke(generation)).await
    }
    pub(crate) async fn pin(&self, generation: u64, pin: String) -> Result<(), Error> {
        self.request(Action::Pin(generation, pin)).await.map(|_| ())
    }
    pub(crate) fn selected(&self) -> watch::Receiver<Option<Arc<Approved>>> {
        self.0.active.subscribe()
    }
    fn generation(&self) -> u64 {
        self.0.counter.fetch_add(1, Ordering::AcqRel) + 1
    }
    fn phase(&self, phase: Phase) {
        self.0.state.send_modify(|view| {
            view.phase = phase;
            view.error = None;
        });
    }
    async fn persist(&self, vault: Arc<dyn Vault>, target: Option<&Approved>) -> Result<(), Error> {
        if !vault.available() {
            return Err(Error::Vault);
        }
        let value = serde_json::to_string(&Saved {
            version: 1,
            approved: target.map(|target| SavedTarget {
                candidate: target.candidate.clone(),
                pin: target.pin.clone(),
            }),
        })
        .map_err(|_| Error::Vault)?;
        self.0
            .writes
            .start(async move { vault.store(value).await })
            .await
            .map_err(|_| Error::Vault)?
    }
    pub(crate) async fn run(&self, vault: Arc<dyn Vault>, discover: Arc<dyn Discover>) {
        let (sender, mut requests) = mpsc::channel::<Request>(4);
        {
            let mut previous = self
                .0
                .sender
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if previous.as_ref().is_some_and(|old| !old.is_closed()) {
                return;
            }
            *previous = Some(sender);
        }
        let mut active = None;
        let mut lease = None;
        let _reset = Reset(self.clone());
        self.0.writes.wait().await;
        if let Some(raw) = vault.get().filter(|raw| raw.len() <= 16 * 1024)
            && let Ok(saved) = serde_json::from_str::<Saved>(&raw)
            && saved.version == 1
            && let Some(saved) = saved.approved
            && self.valid(&saved.candidate)
            && saved.pin.as_ref().is_none_or(|pin| valid_pin(pin))
            && self.0.owners.aliases_allowed(&saved.candidate).await
        {
            lease = self.0.owners.claim(&saved.candidate);
            if lease.is_some() {
                active = Some(Arc::new(Approved {
                    generation: self.generation(),
                    candidate: saved.candidate,
                    pin: saved.pin,
                }));
            }
        }
        self.0.active.send_replace(active.clone());
        self.0.state.send_modify(|view| {
            view.phase = if active.is_some() {
                Phase::Active
            } else {
                Phase::Idle
            };
            view.generation = active
                .as_ref()
                .map_or_else(|| self.generation(), |a| a.generation);
            view.error = None;
            view.candidates = active
                .as_ref()
                .map_or_else(Vec::new, |target| vec![target.candidate.clone()]);
        });
        while let Some(request) = requests.recv().await {
            if request.reply.is_closed() {
                continue;
            }
            let result = self
                .handle(
                    request.action,
                    vault.clone(),
                    discover.as_ref(),
                    &mut active,
                    &mut lease,
                )
                .await;
            if let Err(error) = result {
                self.0.state.send_modify(|view| {
                    view.error = Some(error);
                    if view.phase == Phase::Discovering {
                        view.phase = Phase::Failed;
                    }
                    if view.phase == Phase::Persisting {
                        view.phase = if active.is_some() {
                            Phase::Active
                        } else {
                            Phase::Ready
                        };
                    }
                });
            }
            let _ = request.reply.send(result.map(|()| self.state()));
        }
    }
    fn valid(&self, candidate: &Candidate) -> bool {
        self.0.config.in_scope(candidate.host)
            && candidate.video
            && (candidate.id == "receiver"
                || crate::discovery::native_id(&candidate.id).as_deref() == Some(&candidate.id))
            && !self
                .0
                .config
                .external_ids
                .iter()
                .any(|id| crate::discovery::native_id(id).as_ref() == Some(&candidate.id))
    }
    async fn find(&self, discover: &dyn Discover) -> Result<(), Error> {
        let generation = self.generation();
        self.0.state.send_replace(View {
            phase: Phase::Discovering,
            generation,
            candidates: Vec::new(),
            error: None,
        });
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(4),
            discover.discover(self.0.config.clone()),
        )
        .await
        .map_err(|_| Error::Network)??;
        let mut unique = std::collections::BTreeMap::new();
        for candidate in result.into_iter().take(64) {
            if self.valid(&candidate) {
                unique.entry(candidate.id.clone()).or_insert(candidate);
            }
        }
        self.0.state.send_modify(|view| {
            view.phase = Phase::Ready;
            view.candidates = unique.into_values().collect();
        });
        Ok(())
    }
    async fn handle(
        &self,
        action: Action,
        vault: Arc<dyn Vault>,
        discover: &dyn Discover,
        active: &mut Option<Arc<Approved>>,
        lease: &mut Option<Lease>,
    ) -> Result<(), Error> {
        match action {
            Action::Discover => {
                if active.is_some() {
                    return Err(Error::Busy);
                }
                self.find(discover).await?;
            }
            Action::Approve {
                generation,
                candidate,
                exclusive,
            } => {
                if generation != self.state().generation {
                    return Err(Error::Stale);
                }
                if !exclusive {
                    return Err(Error::Invalid);
                }
                if active.is_some() {
                    return Err(Error::Busy);
                }
                let candidate = self
                    .state()
                    .candidates
                    .into_iter()
                    .find(|target| target.id == candidate)
                    .filter(|target| self.valid(target))
                    .ok_or(Error::Invalid)?;
                let held = self
                    .0
                    .owners
                    .claim(&candidate)
                    .ok_or(Error::OwnedElsewhere)?;
                if !self.0.owners.aliases_allowed(&candidate).await {
                    return Err(Error::OwnedElsewhere);
                }
                self.phase(Phase::Persisting);
                let target = Arc::new(Approved {
                    generation,
                    candidate,
                    pin: None,
                });
                self.persist(vault, Some(&target)).await?;
                *lease = Some(held);
                *active = Some(target.clone());
                self.0.active.send_replace(Some(target));
                self.phase(Phase::Active);
                self.0.state.send_modify(|view| {
                    view.candidates = active
                        .as_ref()
                        .map_or_else(Vec::new, |target| vec![target.candidate.clone()]);
                });
            }
            Action::Revoke(generation) => {
                if generation != self.state().generation {
                    return Err(Error::Stale);
                }
                *active = None;
                self.0.active.send_replace(None);
                *lease = None;
                let generation = self.generation();
                self.0.state.send_replace(View {
                    phase: Phase::Persisting,
                    generation,
                    candidates: Vec::new(),
                    error: None,
                });
                self.persist(vault, None).await?;
                self.phase(Phase::Idle);
            }
            Action::Pin(generation, pin) => {
                let old = active
                    .as_ref()
                    .filter(|target| target.generation == generation)
                    .ok_or(Error::Stale)?;
                if !valid_pin(&pin) || old.pin.as_ref().is_some_and(|old| old != &pin) {
                    return Err(Error::Invalid);
                }
                if old.pin.as_ref() != Some(&pin) {
                    let target = Arc::new(Approved {
                        generation,
                        candidate: old.candidate.clone(),
                        pin: Some(pin),
                    });
                    self.persist(vault, Some(&target)).await?;
                    *active = Some(target.clone());
                    self.0.active.send_replace(Some(target));
                }
            }
        }
        Ok(())
    }
}
fn valid_pin(pin: &str) -> bool {
    pin.len() == 64 && pin.bytes().all(|b| b.is_ascii_hexdigit())
}
struct Reset(Gateway);
impl Drop for Reset {
    fn drop(&mut self) {
        *self
            .0
            .0
            .sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        self.0.0.active.send_replace(None);
        self.0.phase(Phase::Unavailable);
    }
}
#[cfg(test)]
#[path = "gateway_tests.rs"]
mod tests;
