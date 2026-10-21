//! Stateful credential boundary. Transactions survive the HTTP waiter's departure.
use crate::{
    PersonDirectory, ProfileView, crypto,
    state::{self, AlarmFile, Credentials, ProfileFile, Record},
    storage::Storage,
};
use moli_alarm::{AuthorizationFailure, Source};
use moli_runtime::Hub;
use std::{
    fmt,
    path::Path,
    sync::{
        Arc, RwLock, Weak,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::{Mutex, OwnedMutexGuard};

pub const SECRET_NAMESPACE: &str = "alarm";
pub(crate) const KEY: &str = "code_key";

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("forbidden")]
    Forbidden,
    #[error("invalid_profile")]
    InvalidProfile,
    #[error("duplicate_code")]
    DuplicateCode,
    #[error("not_found")]
    NotFound,
    #[error("missing_key")]
    MissingKey,
    #[error("corrupt_alarm_access_state")]
    Corrupt,
    #[error("alarm_access_unavailable")]
    Unavailable,
    #[error("alarm_access_capacity")]
    Capacity,
}

pub trait Clock: Send + Sync + fmt::Debug {
    fn now_ms(&self) -> u64;
}
#[derive(Debug)]
struct WallClock;
impl Clock for WallClock {
    fn now_ms(&self) -> u64 {
        moli_core::now_ms()
    }
}

/// A trusted surface supplies a live owner AND human-session check.
/// Never deserialize this capability or derive it from a body/header boolean.
#[derive(Clone)]
pub struct OwnerPermit(pub(crate) Arc<dyn Fn() -> bool + Send + Sync>);
impl OwnerPermit {
    pub fn new(check: impl Fn() -> bool + Send + Sync + 'static) -> Self {
        Self(Arc::new(check))
    }
    pub fn check_for_surface(&self) -> bool {
        (self.0)()
    }

    pub(crate) fn check(&self) -> Result<(), Error> {
        if (self.0)() {
            Ok(())
        } else {
            Err(Error::Forbidden)
        }
    }
}
impl fmt::Debug for OwnerPermit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("OwnerPermit(..)")
    }
}

/// Assigned by the trusted transport. Dashboard identity is unchanged by PIN login.
#[derive(Clone, Debug)]
pub struct RequestSource {
    pub(crate) source: Source,
    pub(crate) key: String,
}
impl RequestSource {
    pub fn dashboard(id: &str, human: bool) -> Result<Self, Error> {
        Self::make(
            if human { Source::Ui } else { Source::Household },
            "dashboard",
            id,
        )
    }
    pub fn keypad(id: &str) -> Result<Self, Error> {
        Self::make(Source::Keypad(id.into()), "keypad", id)
    }
    pub fn agent() -> Self {
        Self {
            source: Source::Agent,
            key: "agent".into(),
        }
    }
    pub fn automation() -> Self {
        Self {
            source: Source::Automation,
            key: "automation".into(),
        }
    }
    fn make(source: Source, kind: &str, id: &str) -> Result<Self, Error> {
        if id.is_empty() || id.len() > 128 || id.chars().any(char::is_control) {
            return Err(Error::InvalidProfile);
        }
        Ok(Self {
            source,
            key: format!("{kind}:{id}"),
        })
    }
    pub fn engine_source(&self) -> &Source {
        &self.source
    }
    pub(crate) fn digest(&self) -> String {
        crypto::hex(ring::digest::digest(&ring::digest::SHA256, self.key.as_bytes()).as_ref())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditEvent {
    pub source: String,
    pub profile: Option<String>,
    pub reason: Option<AuthorizationFailure>,
    pub lock_started: Option<u64>,
}
pub trait Audit: Send + Sync + fmt::Debug {
    fn record(&self, event: AuditEvent);
}

#[derive(Clone)]
pub struct ProfileService(pub(crate) Arc<Inner>);
pub(crate) struct Inner {
    pub hub: Hub,
    pub directory: Arc<dyn PersonDirectory>,
    pub storage: Storage,
    pub clock: Arc<dyn Clock>,
    pub audit: Arc<dyn Audit>,
    pub data: RwLock<Data>,
    pub write: Arc<Mutex<()>>,
    pub unavailable: AtomicBool,
    pub fenced: AtomicBool,
}
#[derive(Clone, Debug)]
pub(crate) struct Data {
    pub profiles: ProfileFile,
    pub alarm: AlarmFile,
}
impl fmt::Debug for ProfileService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProfileService").finish_non_exhaustive()
    }
}
impl Inner {
    pub(crate) fn read(&self) -> std::sync::RwLockReadGuard<'_, Data> {
        self.data
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    pub(crate) fn ready(&self) -> Result<(), Error> {
        if self.unavailable.load(Ordering::Acquire) || self.fenced.load(Ordering::Acquire) {
            Err(Error::Unavailable)
        } else {
            Ok(())
        }
    }
    pub(crate) fn key(&self) -> Result<[u8; 32], Error> {
        let raw = self
            .hub
            .secret(&SECRET_NAMESPACE.into(), KEY)
            .ok_or(Error::MissingKey)?;
        if raw.len() != 64 {
            return Err(Error::MissingKey);
        }
        crypto::unhex(&raw)
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or(Error::MissingKey)
    }
    pub(crate) fn credentials(&self) -> Result<Credentials, Error> {
        match self.hub.secret(&SECRET_NAMESPACE.into(), state::CODES) {
            Some(raw) if raw.len() <= 512 * 1024 => {
                serde_json::from_str(&raw).map_err(|_| Error::Corrupt)
            }
            Some(_) => Err(Error::Corrupt),
            None => Ok(Credentials::default()),
        }
    }
    pub(crate) fn fail(&self, error: Error) -> Error {
        self.unavailable.store(true, Ordering::Release);
        error
    }
    pub(crate) fn publish(&self, data: Data) {
        *self
            .data
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = data;
        self.fenced.store(false, Ordering::Release);
    }
}

#[derive(Debug)]
struct RemovalDirectory;
impl PersonDirectory for RemovalDirectory {
    fn person(&self, _id: &str, _now: u64) -> Option<crate::Person> {
        None
    }
}
#[derive(Debug)]
struct RemovalAudit;
impl Audit for RemovalAudit {
    fn record(&self, _event: AuditEvent) {}
}

impl ProfileService {
    /// Trusted household deletion hook, including when no alarm driver is running.
    /// It only revokes; it never creates a management or code authorization.
    pub async fn begin_person_removal_at(
        hub: Hub,
        path: &Path,
        person: &str,
    ) -> Result<RemovalGuard, Error> {
        let service = Self::open(
            hub,
            Arc::new(RemovalDirectory),
            path,
            Arc::new(RemovalAudit),
        )
        .await?;
        service.begin_person_removal(person).await
    }
    pub async fn open(
        hub: Hub,
        directory: Arc<dyn PersonDirectory>,
        path: &Path,
        audit: Arc<dyn Audit>,
    ) -> Result<Self, Error> {
        Self::open_with_clock(hub, directory, path, Arc::new(WallClock), audit).await
    }
    pub async fn open_with_clock(
        hub: Hub,
        directory: Arc<dyn PersonDirectory>,
        path: &Path,
        clock: Arc<dyn Clock>,
        audit: Arc<dyn Audit>,
    ) -> Result<Self, Error> {
        let path = path.to_owned();
        tokio::task::spawn_blocking(move || Self::load(hub, directory, path, clock, audit))
            .await
            .map_err(|_| Error::Unavailable)?
    }
    fn load(
        hub: Hub,
        directory: Arc<dyn PersonDirectory>,
        path: std::path::PathBuf,
        clock: Arc<dyn Clock>,
        audit: Arc<dyn Audit>,
    ) -> Result<Self, Error> {
        let storage = Storage { root: path };
        let profiles = storage.load::<ProfileFile>(state::PROFILES)?;
        let alarm = storage.load::<AlarmFile>(state::ALARM)?;
        if profiles.is_some() && alarm.is_none() {
            return Err(Error::Corrupt);
        }
        let profiles_missing = profiles.is_none();
        let profiles = profiles.unwrap_or_default();
        let alarm = alarm.unwrap_or_default();
        super::validation::files(&profiles, &alarm)?;
        if !hub.has_secret_store() {
            return Err(Error::MissingKey);
        }
        let service = Self(Arc::new(Inner {
            hub,
            directory,
            storage,
            clock,
            audit,
            data: RwLock::new(Data { profiles, alarm }),
            write: Arc::new(Mutex::new(())),
            unavailable: AtomicBool::new(false),
            fenced: AtomicBool::new(false),
        }));
        let credentials = service.0.credentials()?;
        if profiles_missing && !credentials.entries.is_empty() {
            return Err(Error::Corrupt);
        }
        if !service.0.read().profiles.profiles.is_empty() || !credentials.entries.is_empty() {
            service.0.key()?;
            super::validation::credentials(&service.0.read().profiles, &credentials)?;
        }
        Ok(service)
    }
    pub fn profiles(&self, owner: &OwnerPermit) -> Result<Vec<ProfileView>, Error> {
        owner.check()?;
        self.0.ready()?;
        let data = self.0.read();
        let now = self.0.clock.now_ms();
        data.profiles
            .profiles
            .iter()
            .map(|(id, p)| self.view(id, p, now))
            .collect()
    }
    pub(crate) fn view(&self, id: &str, p: &Record, now: u64) -> Result<ProfileView, Error> {
        let mut draft = p.profile.clone();
        let mut revoked = p.revoked;
        if let Some(person) = &draft.person {
            if let Some(person) = self.0.directory.person(person, now) {
                super::validation::bound(&mut draft, &person)?;
            } else {
                revoked = true;
            }
        }
        Ok(ProfileView {
            id: id.into(),
            profile: draft,
            revoked,
        })
    }
}

#[derive(Clone)]
pub struct ProfileRevoker(pub(crate) Weak<Inner>);
impl ProfileRevoker {
    pub fn upgrade(&self) -> Option<ProfileService> {
        self.0.upgrade().map(ProfileService)
    }
}
impl fmt::Debug for ProfileRevoker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ProfileRevoker(..)")
    }
}
pub struct RemovalGuard {
    pub(crate) _guard: OwnedMutexGuard<()>,
}
impl fmt::Debug for RemovalGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RemovalGuard(..)")
    }
}
