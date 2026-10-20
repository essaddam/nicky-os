use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use tokio::sync::Notify;

#[derive(Default)]
struct MemoryVault {
    value: Mutex<Option<String>>,
    fail: AtomicBool,
    enabled: bool,
    hold: AtomicBool,
    entered: Notify,
    release: Notify,
    writes: AtomicUsize,
}
impl Vault for MemoryVault {
    fn available(&self) -> bool {
        self.enabled
    }
    fn get(&self) -> Option<String> {
        self.value.lock().unwrap().clone()
    }
    fn store<'a>(&'a self, value: &'a str) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            self.writes.fetch_add(1, Ordering::SeqCst);
            self.entered.notify_one();
            if self.hold.load(Ordering::SeqCst) {
                self.release.notified().await;
            }
            if self.fail.load(Ordering::SeqCst) {
                return Err(Error::Vault);
            }
            *self.value.lock().unwrap() = Some(value.into());
            Ok(())
        })
    }
}
struct FakeFactory {
    entered: Arc<Notify>,
    proceed: Arc<Notify>,
    finishes: Arc<AtomicUsize>,
    verify_hold: Arc<AtomicBool>,
    verifying: Arc<Notify>,
    verify_release: Arc<Notify>,
}
struct FakeSession {
    finishes: Arc<AtomicUsize>,
    hold: Arc<AtomicBool>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
}
impl HumanSession for FakeSession {
    fn finish(self: Box<Self>, _code: String) -> BoxFuture<'static, Result<(), Error>> {
        Box::pin(async move {
            self.entered.notify_one();
            if self.hold.load(Ordering::SeqCst) {
                self.release.notified().await;
            }
            self.finishes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
}
fn credential() -> Arc<Credential> {
    static ID: std::sync::OnceLock<Arc<Identity>> = std::sync::OnceLock::new();
    Arc::new(Credential {
        identity: ID
            .get_or_init(|| Arc::new(Identity::generate().unwrap()))
            .clone(),
        pin: "ab".repeat(32),
    })
}
impl Factory for FakeFactory {
    fn prepare(&self) -> BoxFuture<'static, Result<Prepared, Error>> {
        let entered = self.entered.clone();
        let proceed = self.proceed.clone();
        let finishes = self.finishes.clone();
        let hold = self.verify_hold.clone();
        let entered_verify = self.verifying.clone();
        let release = self.verify_release.clone();
        Box::pin(async move {
            entered.notify_one();
            proceed.notified().await;
            Ok(Prepared {
                credential: credential(),
                session: Box::new(FakeSession {
                    finishes,
                    hold,
                    entered: entered_verify,
                    release,
                }),
            })
        })
    }
}
struct Harness {
    gateway: PairingGateway,
    vault: Arc<MemoryVault>,
    factory: Arc<FakeFactory>,
    task: tokio::task::JoinHandle<()>,
}
impl Harness {
    async fn new(enabled: bool) -> Self {
        // Crypto itself is covered in moli-androidtv. Prepare this one test
        // identity outside the actor runtime and its measured human window.
        if enabled {
            let _ = tokio::task::spawn_blocking(credential).await.unwrap();
        }
        let gateway = PairingGateway::new();
        let vault = Arc::new(MemoryVault {
            enabled,
            ..MemoryVault::default()
        });
        let factory = Arc::new(FakeFactory {
            entered: Arc::new(Notify::new()),
            proceed: Arc::new(Notify::new()),
            finishes: Arc::new(AtomicUsize::new(0)),
            verify_hold: Arc::new(AtomicBool::new(false)),
            verifying: Arc::new(Notify::new()),
            verify_release: Arc::new(Notify::new()),
        });
        let (g, v, f) = (gateway.clone(), vault.clone(), factory.clone());
        let task = tokio::spawn(async move { g.run(v, f).await });
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
        Self {
            gateway,
            vault,
            factory,
            task,
        }
    }
    async fn wait(&self, phase: Phase) {
        tokio::time::timeout(Duration::from_secs(4), async {
            let mut state = self.gateway.0.view.subscribe();
            loop {
                if state.borrow_and_update().phase == phase {
                    break;
                }
                state.changed().await.unwrap();
            }
        })
        .await
        .expect("expected public pairing phase");
    }
    async fn awaiting(&self) -> View {
        let view = self.gateway.start().await.unwrap();
        self.factory.entered.notified().await;
        self.factory.proceed.notify_one();
        self.wait(Phase::AwaitingCode).await;
        view
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[tokio::test]
async fn no_actor_or_no_vault_cannot_start_network_pairing() {
    assert_eq!(
        PairingGateway::new().start().await.unwrap_err(),
        Error::Unavailable
    );
    let h = Harness::new(false).await;
    assert_eq!(h.gateway.start().await.unwrap_err(), Error::Vault);
    assert_eq!(h.vault.writes.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn cancel_preparation_rejects_old_code_and_only_new_generation_can_commit() {
    let h = Harness::new(true).await;
    let old = h.gateway.start().await.unwrap();
    h.factory.entered.notified().await;
    assert_eq!(h.gateway.start().await.unwrap_err(), Error::Busy);
    h.gateway.cancel(old.generation).await.unwrap();
    let current = h.awaiting().await;
    assert!(current.generation > old.generation);
    assert_eq!(
        h.gateway
            .submit(old.generation, "ABC123".into())
            .await
            .unwrap_err(),
        Error::Stale
    );
    assert_eq!(
        h.gateway
            .submit(current.generation, "bad".into())
            .await
            .unwrap_err(),
        Error::InvalidCode
    );
    assert_eq!(
        h.gateway
            .submit(current.generation, "ABC123".into())
            .await
            .unwrap()
            .phase,
        Phase::Paired
    );
    assert_eq!(h.factory.finishes.load(Ordering::SeqCst), 1);
    assert_eq!(h.vault.writes.load(Ordering::SeqCst), 1);
    assert_eq!(h.gateway.start().await.unwrap_err(), Error::Busy);
    let public = serde_json::to_string(&h.gateway.state()).unwrap();
    assert!(
        !public.contains("ABC123")
            && !public.contains("PRIVATE")
            && !public.contains("CERTIFICATE")
    );
    assert!(Credential::decode(h.vault.get().as_deref().unwrap()).is_some());
}
#[tokio::test]
async fn failed_persistence_keeps_previous_durable_identity_and_never_pairs() {
    let h = Harness::new(true).await;
    *h.vault.value.lock().unwrap() = Some("previous durable entry".into());
    h.vault.fail.store(true, Ordering::SeqCst);
    let view = h.awaiting().await;
    assert_eq!(
        h.gateway
            .submit(view.generation, "ABC123".into())
            .await
            .unwrap_err(),
        Error::Vault
    );
    assert_eq!(h.gateway.state().phase, Phase::Failed);
    assert!(h.gateway.credentials().borrow().is_none());
    assert_eq!(h.vault.get().as_deref(), Some("previous durable entry"));
}
#[tokio::test(start_paused = true)]
async fn human_window_expires_without_extending_on_submit() {
    let h = Harness::new(true).await;
    let view = h.gateway.start().await.unwrap();
    h.factory.entered.notified().await;
    tokio::time::advance(WINDOW + Duration::from_millis(1)).await;
    h.wait(Phase::Failed).await;
    assert_eq!(h.gateway.state().error, Some(Error::Expired));
    assert_eq!(
        h.gateway
            .submit(view.generation, "ABC123".into())
            .await
            .unwrap_err(),
        Error::Stale
    );
    assert_eq!(h.vault.writes.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn stopping_actor_disconnects_gateway_and_hides_credentials_from_debug() {
    let h = Harness::new(true).await;
    h.awaiting().await;
    h.task.abort();
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert_eq!(h.gateway.state().phase, Phase::Unavailable);
    assert_eq!(h.gateway.start().await.unwrap_err(), Error::Unavailable);
    assert!(!format!("{:?}", h.gateway).contains("PRIVATE"));
}

#[tokio::test]
async fn cancelling_verification_never_commits_an_old_success() {
    let h = Harness::new(true).await;
    let view = h.awaiting().await;
    h.factory.verify_hold.store(true, Ordering::SeqCst);
    let gateway = h.gateway.clone();
    let submitting =
        tokio::spawn(async move { gateway.submit(view.generation, "ABC123".into()).await });
    h.factory.verifying.notified().await;
    h.gateway.cancel(view.generation).await.unwrap();
    assert_eq!(submitting.await.unwrap().unwrap_err(), Error::Stale);
    h.factory.verify_release.notify_one();
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    assert_eq!(h.vault.writes.load(Ordering::SeqCst), 0);
    assert_eq!(h.factory.finishes.load(Ordering::SeqCst), 0);
    assert!(h.gateway.credentials().borrow().is_none());
}

#[tokio::test]
async fn cancellation_cannot_be_acknowledged_after_atomic_commit_has_started() {
    let h = Harness::new(true).await;
    let view = h.awaiting().await;
    h.vault.hold.store(true, Ordering::SeqCst);
    let gateway = h.gateway.clone();
    let request =
        tokio::spawn(async move { gateway.submit(view.generation, "ABC123".into()).await });
    h.vault.entered.notified().await;
    assert_eq!(h.gateway.state().phase, Phase::Persisting);
    assert!(h.gateway.credentials().borrow().is_none());
    assert_eq!(
        h.gateway.cancel(view.generation).await.unwrap_err(),
        Error::Stale
    );
    h.vault.release.notify_one();
    assert_eq!(request.await.unwrap().unwrap().phase, Phase::Paired);
    assert_eq!(h.vault.writes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn actor_restart_waits_for_an_accepted_commit_before_loading_identity() {
    let mut h = Harness::new(true).await;
    let view = h.awaiting().await;
    h.vault.hold.store(true, Ordering::SeqCst);
    let gateway = h.gateway.clone();
    let submitting =
        tokio::spawn(async move { gateway.submit(view.generation, "ABC123".into()).await });
    h.vault.entered.notified().await;
    h.task.abort();
    let _ = (&mut h.task).await;
    assert_eq!(submitting.await.unwrap().unwrap_err(), Error::Unavailable);
    let (gateway, vault, factory) = (h.gateway.clone(), h.vault.clone(), h.factory.clone());
    h.task = tokio::spawn(async move { gateway.run(vault, factory).await });
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        h.gateway.state().phase,
        Phase::Unavailable,
        "restart must wait for the accepted vault transaction"
    );
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(4), h.gateway.start())
            .await
            .unwrap()
            .unwrap_err(),
        Error::Unavailable
    );
    h.vault.release.notify_one();
    h.wait(Phase::Paired).await;
    assert!(h.gateway.credentials().borrow().is_some());
    assert!(Credential::decode(h.vault.get().as_deref().unwrap()).is_some());
    assert_eq!(h.vault.writes.load(Ordering::SeqCst), 1);
}
