//! Opt-in transport instrumentation for driver contracts, never an abandon guard.
//! A Context is immutable and local to one Cloud; no global routing registry.

use std::collections::BTreeMap;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context as _, bail};
use http::{Method, Request};
use http_body_util::Full;
use rustls::pki_types::CertificateDer;
use rustls::{ClientConfig, RootCertStore};
use tokio::net::TcpStream;
use tokio::task::JoinHandle;

use crate::Body;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    BeforeTcp,
    AfterTcp,
    AfterTls,
    BeforeSend,
}

#[derive(Clone, Debug)]
pub struct Observation {
    pub scenario: u64,
    pub stage: Stage,
    pub method: Method,
    pub host: String,
    pub path: String,
}

pub type Observer =
    Arc<dyn Fn(Observation) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

pub struct Context {
    scenario: u64,
    routes: BTreeMap<String, SocketAddr>,
    tls: Arc<ClientConfig>,
    observer: Observer,
    connections: Mutex<Tracking>,
}

impl std::fmt::Debug for Context {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContractContext")
            .field("scenario", &self.scenario)
            .finish_non_exhaustive()
    }
}

impl Context {
    pub fn new(
        scenario: u64,
        routes: BTreeMap<String, SocketAddr>,
        trusted_ca_der: Vec<u8>,
        observer: Observer,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            !routes.is_empty() && routes.len() <= 2,
            "invalid contract routes"
        );
        for (host, address) in &routes {
            anyhow::ensure!(
                matches!(
                    host.as_str(),
                    "api.myvoltalis.com" | "www.ariston-net.remotethermo.com"
                ) && address.ip().is_loopback()
                    && address.port() != 0,
                "only fixed heater API hosts on loopback are permitted"
            );
        }
        let mut roots = RootCertStore::empty();
        roots.add(CertificateDer::from(trusted_ca_der))?;
        let tls =
            ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()?
                .with_root_certificates(roots)
                .with_no_client_auth();
        Ok(Self {
            scenario,
            routes,
            tls: Arc::new(tls),
            observer,
            connections: Mutex::new(Tracking::default()),
        })
    }

    fn destination(&self, host: &str, port: u16, tls: bool) -> anyhow::Result<SocketAddr> {
        anyhow::ensure!(
            port == 443 && tls,
            "contract requires the original HTTPS endpoint"
        );
        self.routes
            .get(host)
            .copied()
            .context("unexpected contract host: no public fallback")
    }

    fn track(&self, label: ConnectionLabel, task: JoinHandle<Result<(), ConnectionFailure>>) {
        let mut state = self
            .connections
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let id = state.next_id;
        state.next_id += 1;
        if state.finishing {
            state.spawned_during_finish = true;
        }
        state.labels.push((id, label));
        state.pending.push(Tracked { id, task });
        state.complete = false;
    }

    /// The caller records the chosen id only after its server has received
    /// the complete request. These labels carry no headers or credentials.
    pub fn connection_labels(&self) -> Vec<(u64, ConnectionLabel)> {
        self.connections
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .labels
            .clone()
    }

    /// Join after driver/workers and server shutdown. Dropping this future
    /// aborts and RETURNS all unharvested handles to this Context, recording
    /// interruption. Retrying reconciles resources; it never erases the fault.
    pub async fn finish_connections(
        self: &Arc<Self>,
        within: Duration,
    ) -> anyhow::Result<FinishReport> {
        let mut lease = DrainLease::begin(Arc::clone(self))?;
        let deadline = tokio::time::Instant::now() + within;
        while !lease.tasks.is_empty() {
            if !lease.harvest_one(deadline).await {
                {
                    let mut state = self
                        .connections
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    state.timed_out = true;
                }
                lease.abort_remaining();
                let abort_by = tokio::time::Instant::now() + Duration::from_secs(1);
                while !lease.tasks.is_empty() {
                    if !lease.harvest_one(abort_by).await {
                        // Drop restores these still-owned handles. No claim
                        // of joined resources or a healthy fixture is made.
                        bail!("contract connection abort could not be joined; ownership retained");
                    }
                }
                break;
            }
        }
        Ok(lease.commit())
    }
}

#[derive(Clone, Debug)]
pub struct ConnectionLabel {
    pub scenario: u64,
    pub method: Method,
    pub host: String,
    pub path: String,
}

#[derive(Clone, Debug)]
pub struct ConnectionFailure {
    pub message: String,
    pub loss_compatible: bool,
}

impl ConnectionFailure {
    pub fn from_hyper(error: &hyper::Error) -> Self {
        let mut loss_compatible = error.is_closed() || error.is_incomplete_message();
        let mut source = std::error::Error::source(error);
        for _ in 0..16 {
            let Some(cause) = source else { break };
            if let Some(io) = cause.downcast_ref::<std::io::Error>() {
                loss_compatible |= matches!(
                    io.kind(),
                    std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::ConnectionAborted
                        | std::io::ErrorKind::UnexpectedEof
                        | std::io::ErrorKind::BrokenPipe
                );
            }
            source = cause.source();
        }
        Self {
            message: error.to_string(),
            loss_compatible,
        }
    }
}

#[derive(Clone, Debug)]
pub enum ConnectionOutcome {
    Complete,
    HyperError(ConnectionFailure),
    TaskPanic(String),
    TaskCancelled(String),
}

#[derive(Clone, Copy, Debug)]
pub struct ExpectedLoss {
    connection: u64,
}

impl ExpectedLoss {
    /// Fixture evidence, supplied ONLY after the loopback server confirms a
    /// complete request. It never licenses a task panic, timeout or abort.
    pub fn after_complete_request(connection: u64) -> Self {
        Self { connection }
    }
}

#[derive(Clone, Debug)]
// Independent fault receipts: retaining several simultaneous faults is intentional.
#[allow(clippy::struct_excessive_bools)]
pub struct FinishReport {
    pub resources_joined: bool,
    pub interrupted: bool,
    pub timed_out: bool,
    pub spawned_during_finish: bool,
    pub outcomes: Vec<(u64, ConnectionOutcome)>,
}

impl FinishReport {
    /// Raw results are retained even for the explicitly expected loss.
    pub fn healthy(&self, losses: &[ExpectedLoss]) -> bool {
        if !self.resources_joined
            || self.interrupted
            || self.timed_out
            || self.spawned_during_finish
        {
            return false;
        }
        let expected: std::collections::BTreeSet<u64> =
            losses.iter().map(|l| l.connection).collect();
        if expected.len() != losses.len() {
            return false;
        }
        let mut observed = std::collections::BTreeSet::new();
        for (id, outcome) in &self.outcomes {
            match outcome {
                ConnectionOutcome::Complete => {
                    // A voluntarily dropped peer can finish Hyper cleanly;
                    // the RPC result is asserted separately by its contract.
                    if expected.contains(id) {
                        observed.insert(*id);
                    }
                }
                ConnectionOutcome::HyperError(error)
                    if expected.contains(id) && error.loss_compatible =>
                {
                    observed.insert(*id);
                }
                _ => return false,
            }
        }
        observed == expected
    }
}

struct Tracked {
    id: u64,
    task: JoinHandle<Result<(), ConnectionFailure>>,
}

#[derive(Default)]
// Lifecycle ownership and independent fault receipts must not mask one another.
#[allow(clippy::struct_excessive_bools)]
struct Tracking {
    next_id: u64,
    pending: Vec<Tracked>,
    labels: Vec<(u64, ConnectionLabel)>,
    outcomes: Vec<(u64, ConnectionOutcome)>,
    leased: usize,
    finishing: bool,
    complete: bool,
    interrupted: bool,
    timed_out: bool,
    spawned_during_finish: bool,
}

struct DrainLease {
    owner: Arc<Context>,
    tasks: Vec<Tracked>,
    committed: bool,
}

impl DrainLease {
    fn begin(owner: Arc<Context>) -> anyhow::Result<Self> {
        let tasks = {
            let mut state = owner
                .connections
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            anyhow::ensure!(
                !state.finishing,
                "another connection finish owns the handles"
            );
            state.finishing = true;
            state.complete = false;
            let tasks = std::mem::take(&mut state.pending);
            state.leased = tasks.len();
            tasks
        };
        Ok(Self {
            owner,
            tasks,
            committed: false,
        })
    }

    fn abort_remaining(&self) {
        for tracked in &self.tasks {
            tracked.task.abort();
        }
    }

    async fn harvest_one(&mut self, deadline: tokio::time::Instant) -> bool {
        let Some(tracked) = self.tasks.last_mut() else {
            return true;
        };
        let Ok(result) = tokio::time::timeout_at(deadline, &mut tracked.task).await else {
            return false;
        };
        // No await between harvesting a handle and durably recording it.
        // Completed handles are removed, so a resumed finish never repolls them.
        let tracked = self.tasks.pop().expect("owned handle just polled");
        let outcome = match result {
            Ok(Ok(())) => ConnectionOutcome::Complete,
            Ok(Err(error)) => ConnectionOutcome::HyperError(error),
            Err(error) if error.is_panic() => ConnectionOutcome::TaskPanic(error.to_string()),
            Err(error) => ConnectionOutcome::TaskCancelled(error.to_string()),
        };
        let mut state = self
            .owner
            .connections
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.leased -= 1;
        state.outcomes.push((tracked.id, outcome));
        true
    }

    fn commit(mut self) -> FinishReport {
        let report = {
            let mut state = self
                .owner
                .connections
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.finishing = false;
            state.complete = state.pending.is_empty()
                && state.leased == 0
                && state.outcomes.len() == state.labels.len();
            FinishReport {
                resources_joined: state.complete,
                interrupted: state.interrupted,
                timed_out: state.timed_out,
                spawned_during_finish: state.spawned_during_finish,
                outcomes: state.outcomes.clone(),
            }
        };
        self.committed = true;
        report
    }
}

impl Drop for DrainLease {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        self.abort_remaining();
        let mut state = self
            .owner
            .connections
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.pending.append(&mut self.tasks);
        state.leased = 0;
        state.finishing = false;
        state.complete = false;
        state.interrupted = true;
    }
}

tokio::task_local! {
    static CURRENT: Arc<Context>;
}

/// Establish this scope inside Cloud::exchange, in the JoinSet worker.
/// Tasks never inherit it implicitly; spawned Hyper work is scoped below.
pub async fn scope<F: Future>(context: Arc<Context>, future: F) -> F::Output {
    CURRENT.scope(context, future).await
}

pub(crate) struct Trace {
    context: Option<Arc<Context>>,
    destination: Option<SocketAddr>,
    method: Method,
    host: String,
    path: String,
}

impl Trace {
    pub(crate) fn capture(
        host: &str,
        port: u16,
        tls: bool,
        request: &Request<Full<Body>>,
    ) -> anyhow::Result<Self> {
        let context = CURRENT.try_with(Arc::clone).ok();
        let destination = context
            .as_ref()
            .map(|c| c.destination(host, port, tls))
            .transpose()?;
        Ok(Self {
            context,
            destination,
            method: request.method().clone(),
            host: host.to_owned(),
            path: request.uri().path().to_owned(),
        })
    }

    pub(crate) async fn stage(&self, stage: Stage) {
        if let Some(context) = &self.context {
            (context.observer)(Observation {
                scenario: context.scenario,
                stage,
                method: self.method.clone(),
                host: self.host.clone(),
                path: self.path.clone(),
            })
            .await;
        }
    }

    pub(crate) async fn connect(&self, host: &str, port: u16) -> anyhow::Result<TcpStream> {
        match self.destination {
            Some(address) => TcpStream::connect(address).await,
            None => TcpStream::connect((host, port)).await,
        }
        .with_context(|| format!("cannot reach {host}:{port}"))
    }

    pub(crate) fn tls_config(&self) -> Arc<ClientConfig> {
        self.context
            .as_ref()
            .map_or_else(crate::public_tls, |c| Arc::clone(&c.tls))
    }

    pub(crate) fn spawn_connection(
        &self,
        future: impl Future<Output = Result<(), ConnectionFailure>> + Send + 'static,
    ) {
        if let Some(context) = &self.context {
            let owned = Arc::clone(context);
            let task = tokio::spawn(scope(owned, future));
            context.track(
                ConnectionLabel {
                    scenario: context.scenario,
                    method: self.method.clone(),
                    host: self.host.clone(),
                    path: self.path.clone(),
                },
                task,
            );
        } else {
            tokio::spawn(async move {
                // Same base behavior outside a fixture; RPC is unaffected.
                let _ = future.await;
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::task::Poll;

    // Ownership/result counter-cases only. No socket, DNS, TLS server or RPC.
    fn context() -> Arc<Context> {
        Arc::new(Context {
            scenario: 1,
            routes: BTreeMap::new(),
            tls: crate::public_tls(),
            observer: Arc::new(|_| Box::pin(async {})),
            connections: Mutex::new(Tracking::default()),
        })
    }

    fn trace(context: &Arc<Context>) -> Trace {
        Trace {
            context: Some(Arc::clone(context)),
            destination: None,
            method: Method::PUT,
            host: "api.myvoltalis.com".into(),
            path: "/api/site/100001/manualsetting/300001".into(),
        }
    }

    #[tokio::test]
    async fn cancelled_finish_returns_owned_handles_for_reconciliation() {
        let context = context();
        trace(&context).spawn_connection(std::future::pending());
        let mut finish = Box::pin(context.finish_connections(Duration::from_secs(3)));
        std::future::poll_fn(|cx| {
            assert!(matches!(finish.as_mut().poll(cx), Poll::Pending));
            Poll::Ready(())
        })
        .await;
        assert!(context.connections.lock().unwrap().finishing);
        drop(finish);
        {
            let state = context.connections.lock().unwrap();
            assert!(!state.finishing);
            assert!(state.interrupted);
            assert_eq!(
                state.pending.len(),
                1,
                "owner retained the unharvested handle"
            );
        }
        let report = context
            .finish_connections(Duration::from_secs(3))
            .await
            .unwrap();
        assert!(report.resources_joined);
        assert!(
            report.interrupted,
            "retry must not erase the cancelled finish"
        );
        assert!(!report.healthy(&[]));
        assert!(matches!(
            report.outcomes.as_slice(),
            [(_, ConnectionOutcome::TaskCancelled(_))]
        ));
    }

    #[tokio::test]
    async fn connection_error_is_retained_and_expected_loss_is_specific() {
        for loss_compatible in [false, true] {
            let context = context();
            trace(&context).spawn_connection(async move {
                Err(ConnectionFailure {
                    message: "injected connection error".into(),
                    loss_compatible,
                })
            });
            let report = context
                .finish_connections(Duration::from_secs(3))
                .await
                .unwrap();
            assert!(report.resources_joined);
            let [(id, ConnectionOutcome::HyperError(error))] = report.outcomes.as_slice() else {
                panic!("connection result lost: {report:?}");
            };
            assert_eq!(error.message, "injected connection error");
            assert!(
                !report.healthy(&[]),
                "unplanned error is always a fixture failure"
            );
            // Policy receipt is simulated here; actual server/body evidence
            // remains mandatory in the later real transport contracts.
            let receipt = ExpectedLoss::after_complete_request(*id);
            assert_eq!(report.healthy(&[receipt]), loss_compatible);
            assert!(
                !report.healthy(&[receipt, receipt]),
                "duplicate receipt cannot hide failures"
            );
        }
    }

    #[tokio::test]
    async fn connection_task_panic_remains_a_fixture_failure() {
        let context = context();
        trace(&context).spawn_connection(async {
            panic!("injected tracked Hyper task panic");
        });
        let report = context
            .finish_connections(Duration::from_secs(3))
            .await
            .unwrap();
        assert!(report.resources_joined);
        let [(id, ConnectionOutcome::TaskPanic(cause))] = report.outcomes.as_slice() else {
            panic!("task panic lost: {report:?}");
        };
        assert!(cause.contains("injected tracked Hyper task panic"));
        assert!(!report.healthy(&[ExpectedLoss::after_complete_request(*id)]));
    }

    #[tokio::test]
    async fn deadline_uses_abort_and_join_but_remains_a_fixture_failure() {
        let context = context();
        trace(&context).spawn_connection(std::future::pending());
        let report = context
            .finish_connections(Duration::from_millis(1))
            .await
            .unwrap();
        assert!(report.resources_joined);
        assert!(report.timed_out);
        assert!(!report.healthy(&[]));
        assert!(matches!(
            report.outcomes.as_slice(),
            [(_, ConnectionOutcome::TaskCancelled(_))]
        ));
    }
}
