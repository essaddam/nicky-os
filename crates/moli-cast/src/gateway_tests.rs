use super::*;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
#[derive(Default)]
struct Memory {
    value: Mutex<Option<String>>,
    writes: AtomicUsize,
    fail: std::sync::atomic::AtomicBool,
    hold: std::sync::atomic::AtomicBool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
impl Vault for Memory {
    fn available(&self) -> bool {
        true
    }
    fn get(&self) -> Option<String> {
        self.value.lock().unwrap().clone()
    }
    fn store(&self, value: String) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move {
            self.writes.fetch_add(1, Ordering::SeqCst);
            if self.hold.load(Ordering::SeqCst) {
                self.entered.notify_one();
                self.release.notified().await;
            }
            if self.fail.load(Ordering::SeqCst) {
                return Err(Error::Vault);
            }
            *self.value.lock().unwrap() = Some(value);
            Ok(())
        })
    }
}
#[derive(Default)]
struct Catalog(AtomicUsize);
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
    fn discover(&self, _config: Config) -> BoxFuture<'static, Result<Vec<Candidate>, Error>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(vec![candidate()]) })
    }
}
fn config() -> Config {
    Config {
        host: None,
        interface: Some("192.168.1.2".parse().unwrap()),
        prefix: 24,
        external_ids: Vec::new(),
    }
}
struct Harness {
    gateway: Gateway,
    vault: Arc<Memory>,
    catalog: Arc<Catalog>,
    task: tokio::task::JoinHandle<()>,
}
impl Harness {
    async fn new(owners: Ownership) -> Self {
        let gateway = Gateway::new(config(), owners);
        let vault = Arc::new(Memory::default());
        let catalog = Arc::new(Catalog::default());
        let g = gateway.clone();
        let v = vault.clone();
        let c = catalog.clone();
        let task = tokio::spawn(async move {
            g.run(v, c).await;
        });
        for _ in 0..100 {
            if gateway.state().phase != Phase::Unavailable {
                break;
            }
            tokio::task::yield_now().await;
        }
        Self {
            gateway,
            vault,
            catalog,
            task,
        }
    }
    async fn discover(&self) -> View {
        self.gateway
            .discover()
            .await
            .expect("explicit discovery must succeed")
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        self.task.abort();
    }
}
#[tokio::test]
async fn construction_and_idle_never_discover_or_activate_a_receiver() {
    let h = Harness::new(Ownership::default()).await;
    assert_eq!(h.gateway.state().phase, Phase::Idle);
    assert_eq!(h.catalog.0.load(Ordering::SeqCst), 0);
    assert!(h.gateway.selected().borrow().is_none());
    assert_eq!(h.vault.writes.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn discovery_does_not_activate_and_only_an_exact_validated_generation_can_publish() {
    let h = Harness::new(Ownership::default()).await;
    let view = h.discover().await;
    assert_eq!(view.candidates.len(), 1);
    assert!(h.gateway.selected().borrow().is_none());
    assert_eq!(h.vault.writes.load(Ordering::SeqCst), 0);
    assert_eq!(
        h.gateway
            .approve(view.generation + 1, candidate().id, true)
            .await
            .unwrap_err(),
        Error::Stale
    );
    assert_eq!(
        h.gateway
            .approve(view.generation, candidate().id, false)
            .await
            .unwrap_err(),
        Error::Invalid
    );
    let active = h
        .gateway
        .approve(view.generation, candidate().id, true)
        .await
        .unwrap();
    assert_eq!(active.phase, Phase::Active);
    assert!(h.gateway.selected().borrow().is_some());
    assert_eq!(h.vault.writes.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn a_receiver_reserved_by_a_tv_or_music_assistant_cannot_be_activated() {
    let owners = Ownership::default();
    owners.reserve_external(&candidate().id, "screen.local");
    let h = Harness::new(owners).await;
    let view = h.discover().await;
    assert_eq!(
        h.gateway
            .approve(view.generation, candidate().id, true)
            .await
            .unwrap_err(),
        Error::OwnedElsewhere
    );
    assert!(h.gateway.selected().borrow().is_none());
    assert_eq!(h.vault.writes.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn two_explicit_cast_instances_cannot_claim_the_same_identity_or_address() {
    let owners = Ownership::default();
    let first = owners.claim(&candidate()).expect("first explicit claim");
    assert!(owners.claim(&candidate()).is_none());
    let mut alias = candidate();
    alias.id = "00000000000000000000000000000002".into();
    assert!(owners.claim(&alias).is_none());
    drop(first);
    assert!(owners.claim(&alias).is_some());
}
#[tokio::test]
async fn failed_durable_activation_never_publishes_and_revoke_invalidates_old_selection() {
    let h = Harness::new(Ownership::default()).await;
    let view = h.discover().await;
    h.vault.fail.store(true, Ordering::SeqCst);
    assert_eq!(
        h.gateway
            .approve(view.generation, candidate().id, true)
            .await
            .unwrap_err(),
        Error::Vault
    );
    assert!(h.gateway.selected().borrow().is_none());
    h.vault.fail.store(false, Ordering::SeqCst);
    h.gateway
        .approve(view.generation, candidate().id, true)
        .await
        .unwrap();
    let revoked = h.gateway.revoke(view.generation).await.unwrap();
    assert!(revoked.generation > view.generation);
    assert!(h.gateway.selected().borrow().is_none());
    assert_eq!(
        h.gateway
            .approve(view.generation, candidate().id, true)
            .await
            .unwrap_err(),
        Error::Stale
    );
}

#[tokio::test]
async fn restarting_actor_waits_for_an_accepted_write_without_rediscovering() {
    let mut h = Harness::new(Ownership::default()).await;
    let view = h.discover().await;
    h.vault.hold.store(true, Ordering::SeqCst);
    let gateway = h.gateway.clone();
    let call =
        tokio::spawn(async move { gateway.approve(view.generation, candidate().id, true).await });
    h.vault.entered.notified().await;
    assert!(h.gateway.selected().borrow().is_none());
    h.task.abort();
    let _ = (&mut h.task).await;
    assert_eq!(call.await.unwrap().unwrap_err(), Error::Unavailable);
    let gateway = h.gateway.clone();
    let vault = h.vault.clone();
    let catalog = h.catalog.clone();
    h.task = tokio::spawn(async move {
        gateway.run(vault, catalog).await;
    });
    for _ in 0..30 {
        tokio::task::yield_now().await;
    }
    assert_eq!(h.gateway.state().phase, Phase::Unavailable);
    assert!(h.gateway.selected().borrow().is_none());
    h.vault.hold.store(false, Ordering::SeqCst);
    h.vault.release.notify_one();
    for _ in 0..200 {
        if h.gateway.state().phase == Phase::Active {
            break;
        }
        tokio::task::yield_now().await;
    }
    let active = h.gateway.selected().borrow().clone().unwrap();
    assert_eq!(active.candidate, candidate());
    assert!(active.generation > view.generation);
    assert_eq!(h.catalog.0.load(Ordering::SeqCst), 1);
    h.gateway
        .pin(active.generation, "aa".repeat(32))
        .await
        .unwrap();
    assert_eq!(
        h.gateway
            .pin(active.generation, "bb".repeat(32))
            .await
            .unwrap_err(),
        Error::Invalid
    );
    assert!(
        !serde_json::to_string(&h.gateway.state())
            .unwrap()
            .contains(&"aa".repeat(32))
    );
    h.gateway.revoke(active.generation).await.unwrap();
    assert_eq!(
        h.gateway
            .pin(active.generation, "aa".repeat(32))
            .await
            .unwrap_err(),
        Error::Stale
    );
}

#[test]
fn manual_targets_share_address_exclusion_without_colliding_on_their_local_key() {
    let owners = Ownership::default();
    let mut first = candidate();
    first.id = "receiver".into();
    let lease = owners.claim(&first).unwrap();
    assert!(owners.claim(&first).is_none());
    let mut second = first.clone();
    second.host = "192.168.1.11".parse().unwrap();
    let second_lease = owners
        .claim(&second)
        .expect("a different manual receiver belongs to its own instance");
    assert!(owners.claim(&candidate()).is_none());
    drop(lease);
    drop(second_lease);
    assert!(owners.claim(&candidate()).is_some());
}

#[tokio::test]
async fn durable_revocation_requires_a_new_discovery_and_explicit_activation() {
    let h = Harness::new(Ownership::default()).await;
    let first = h.discover().await;
    h.gateway
        .approve(first.generation, candidate().id, true)
        .await
        .unwrap();
    let idle = h.gateway.revoke(first.generation).await.unwrap();
    assert_eq!(idle.phase, Phase::Idle);
    assert!(idle.candidates.is_empty());
    assert!(idle.generation > first.generation);
    assert!(h.gateway.selected().borrow().is_none());
    let stored: serde_json::Value = serde_json::from_str(&h.vault.get().unwrap()).unwrap();
    assert!(stored["approved"].is_null());
    let fresh = h.discover().await;
    assert!(fresh.generation > idle.generation);
    assert_eq!(fresh.phase, Phase::Ready);
    assert!(h.gateway.selected().borrow().is_none());
    assert_eq!(
        h.gateway
            .approve(first.generation, candidate().id, true)
            .await
            .unwrap_err(),
        Error::Stale
    );
    let active = h
        .gateway
        .approve(fresh.generation, candidate().id, true)
        .await
        .unwrap();
    assert_eq!(active.phase, Phase::Active);
    assert_eq!(
        h.gateway.selected().borrow().as_ref().unwrap().generation,
        fresh.generation
    );
    assert_eq!(h.vault.writes.load(Ordering::SeqCst), 3);
}
