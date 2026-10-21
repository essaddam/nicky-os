#![allow(dead_code)]
use moli_alarm::{Permission, RequestId};
use moli_alarm_access::{
    Audit, AuditEvent, Clock, Code, Mode, OwnerPermit, Person, PersonDirectory, ProfileDraft,
    ProfileService, RequestSource,
};
use moli_runtime::{Hub, HubOptions, MasterKey};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicU64, Ordering},
    },
};

#[derive(Debug)]
pub struct Time(pub AtomicU64);
impl Clock for Time {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::Acquire)
    }
}
#[derive(Debug, Default)]
pub struct Directory(pub RwLock<BTreeMap<String, Person>>);
impl PersonDirectory for Directory {
    fn person(&self, id: &str, _now: u64) -> Option<Person> {
        self.0.read().unwrap().get(id).cloned()
    }
}
#[derive(Debug, Default)]
pub struct Journal(pub Mutex<Vec<AuditEvent>>);
impl Audit for Journal {
    fn record(&self, event: AuditEvent) {
        self.0.lock().unwrap().push(event);
    }
}
pub struct House {
    pub dir: tempfile::TempDir,
    pub hub: Hub,
    pub key: MasterKey,
    pub time: Arc<Time>,
    pub people: Arc<Directory>,
    pub audit: Arc<Journal>,
    pub profiles: ProfileService,
}
impl House {
    pub async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let key = MasterKey::generate().unwrap();
        let hub = Hub::new(HubOptions {
            secrets: Some((dir.path().join("vault.enc"), key.clone())),
            ..Default::default()
        })
        .unwrap();
        let time = Arc::new(Time(AtomicU64::new(1_000)));
        let people = Arc::new(Directory::default());
        let audit = Arc::new(Journal::default());
        let profiles = ProfileService::open_with_clock(
            hub.clone(),
            people.clone(),
            dir.path(),
            time.clone(),
            audit.clone(),
        )
        .await
        .unwrap();
        Self {
            dir,
            hub,
            key,
            time,
            people,
            audit,
            profiles,
        }
    }
    pub async fn restart(&self) -> ProfileService {
        let hub = Hub::new(HubOptions {
            secrets: Some((self.dir.path().join("vault.enc"), self.key.clone())),
            ..Default::default()
        })
        .unwrap();
        ProfileService::open_with_clock(
            hub,
            self.people.clone(),
            self.dir.path(),
            self.time.clone(),
            self.audit.clone(),
        )
        .await
        .unwrap()
    }
    pub fn owner() -> OwnerPermit {
        OwnerPermit::new(|| true)
    }
    pub fn draft() -> ProfileDraft {
        ProfileDraft {
            name: "Synthetic profile".into(),
            person: None,
            can_arm: true,
            can_disarm: true,
            modes: [Mode::Away, Mode::Home, Mode::Night, Mode::Vacation]
                .into_iter()
                .collect(),
            from: None,
            until: None,
        }
    }
    pub fn source(id: &str) -> RequestSource {
        RequestSource::dashboard(id, true).unwrap()
    }
}
pub async fn disarm_status(
    service: &ProfileService,
    id: u64,
    source: RequestSource,
    code: Code<'_>,
) -> &'static str {
    service
        .verify(RequestId(id), source, code, Permission::Disarm, None)
        .await
        .unwrap()
        .public_status()
}
