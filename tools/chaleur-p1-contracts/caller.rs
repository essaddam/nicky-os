// Included in each driver's test module; adapters supply only fixture names,
// route recognition and the existing Control point. No production seam here.
// These tests stop at Cloud::exchange: they do NOT prove TCP/TLS cancellation.

use std::sync::atomic::{AtomicBool, AtomicU16};

use moli_core::{Action, Outcome};
use moli_runtime::CommandError;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub(super) const FIXTURE_FAILURE: &str = "CHA_FIXTURE_FAILURE";

#[derive(Clone, Copy, Debug, Default)]
pub(super) enum Fault {
    #[default]
    None,
    WorkerPanicDuringDrain,
    ServeErrorAfterAssertions,
}

#[derive(Debug)]
enum ServeExit {
    IntentionalStop,
    ReturnedOk,
    ReturnedErr(String),
}

#[derive(Debug)]
enum WorkerExit {
    Completed,
    IntentionallyCancelled,
    Panic(String),
    Error(String),
}

impl WorkerExit {
    fn failure(&self) -> Option<&str> {
        match self {
            Self::Completed | Self::IntentionallyCancelled => None,
            Self::Panic(cause) | Self::Error(cause) => Some(cause),
        }
    }
}

#[derive(Debug)]
struct DrainReport {
    serve: ServeExit,
    workers: Vec<WorkerExit>,
}

impl DrainReport {
    fn healthy(&self) -> bool {
        matches!(self.serve, ServeExit::IntentionalStop)
            && self.workers.iter().all(|w| w.failure().is_none())
    }
}

#[derive(Debug, Default)]
struct Gate {
    entered: Notify,
    release: Notify,
}

impl Gate {
    async fn wait(&self) {
        self.entered.notify_one();
        self.release.notified().await;
    }

    async fn reached(&self) {
        tokio::time::timeout(Duration::from_secs(3), self.entered.notified())
            .await
            .expect("the production path reached the barrier");
    }
}

#[derive(Debug, Default)]
pub(super) struct Control {
    login: Gate,
    answer: Gate,
    expire_session: AtomicBool,
    hold_login: AtomicBool,
    hold_answer: AtomicBool,
    writes: AtomicU32,
    // 200: positive reply; 500: explicit refusal; 0: lost connection.
    reply_status: AtomicU16,
    stop: CancellationToken,
    drained: CancellationToken,
    report: Mutex<Option<DrainReport>>,
    fault: Fault,
    serve_failure: Notify,
}

fn inspect_worker(
    result: Result<Done, tokio::task::JoinError>,
    abort_requested: bool,
) -> WorkerExit {
    match result {
        Ok(Done::Read(_, Ok(_)) | Done::Sent(_, Ok(()))) => WorkerExit::Completed,
        Ok(Done::Read(_, Err(error)) | Done::Sent(_, Err(error))) => WorkerExit::Error(format!(
            "worker returned an error while draining: {error:#}"
        )),
        Err(error) if error.is_panic() => WorkerExit::Panic(error.to_string()),
        Err(error) if error.is_cancelled() && abort_requested => WorkerExit::IntentionallyCancelled,
        Err(error) => WorkerExit::Error(error.to_string()),
    }
}

async fn drain_workers(link: &mut Link) -> Vec<WorkerExit> {
    let mut outcomes = Vec::new();
    // Observe already completed tasks before requesting cancellation.
    while let Some(result) = link.jobs.try_join_next() {
        outcomes.push(inspect_worker(result, false));
    }
    link.jobs.abort_all();
    while let Some(result) = link.jobs.join_next().await {
        outcomes.push(inspect_worker(result, true));
    }
    outcomes
}

struct PanicOnDrain;

impl Drop for PanicOnDrain {
    fn drop(&mut self) {
        panic!("injected worker panic during fixture drain");
    }
}

async fn install_fault(link: &mut Link, control: &Arc<Control>) {
    match control.fault {
        Fault::None => {}
        Fault::WorkerPanicDuringDrain => {
            let (started, ready) = tokio::sync::oneshot::channel();
            link.jobs.spawn(async move {
                let _panic_on_abort = PanicOnDrain;
                let _ = started.send(());
                std::future::pending::<Done>().await
            });
            ready
                .await
                .expect("drain counter-case worker entered its future");
        }
        Fault::ServeErrorAfterAssertions => {
            let control = Arc::clone(control);
            link.jobs.spawn(async move {
                control.serve_failure.notified().await;
                // The real Link::serve maps this JoinError into Err.
                panic!("injected worker failure observed by Link::serve");
            });
        }
    }
}

#[derive(Debug)]
struct ContractDriver {
    fake: Arc<Fake>,
    control: Arc<Control>,
}

impl Driver for ContractDriver {
    fn kind(&self) -> &'static str {
        "p1-contract"
    }

    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        let mut cloud = Cloud::new("user@example.com".into(), "x".into(), Arc::default());
        let (fake, control) = (Arc::clone(&self.fake), Arc::clone(&self.control));
        cloud.exchange = Some(Arc::new(move |request| {
            let (fake, control) = (Arc::clone(&fake), Arc::clone(&control));
            Box::pin(async move {
                if is_login(&request) && control.hold_login.swap(false, SeqCst) {
                    control.login.wait().await;
                }
                if is_write(&request) {
                    control.writes.fetch_add(1, SeqCst);
                    if control.expire_session.swap(false, SeqCst) {
                        return Ok((StatusCode::UNAUTHORIZED, Body::new()));
                    }
                    if control.hold_answer.swap(false, SeqCst) {
                        // The simulated peer has the complete body before
                        // notifying the test that emission has occurred.
                        let _body = request.body().clone().collect().await?.to_bytes();
                        control.answer.wait().await;
                    }
                    match control.reply_status.load(SeqCst) {
                        0 => anyhow::bail!("simulated connection lost after request reception"),
                        200 => {}
                        status => return Ok((StatusCode::from_u16(status)?, Body::new())),
                    }
                }
                let (status, body) = fake.answer(request).await;
                let body = if body.is_null() {
                    Body::new()
                } else {
                    Body::from(body.to_string())
                };
                Ok((StatusCode::from_u16(status)?, body))
            })
        }));
        // No idle poll races the session-renewal setup. Follow-up reads
        // still exercise the real confirmation path after a positive ACK.
        let pace = Pace {
            idle: Duration::from_secs(600),
            follow: Duration::from_millis(20),
            confirm: Duration::from_millis(400),
        };
        let control = Arc::clone(&self.control);
        Box::pin(async move {
            // Own the production loop's workers in the fixture so shutdown
            // can join them before the supervisor drops Driver::run.
            let mut link = Link::new(cloud, pace);
            install_fault(&mut link, &control).await;
            let (result, serve) = tokio::select! {
                result = link.serve(ctx) => {
                    let exit = match &result {
                        Ok(()) => ServeExit::ReturnedOk,
                        Err(error) => ServeExit::ReturnedErr(format!("{error:#}")),
                    };
                    (result, exit)
                },
                () = control.stop.cancelled() => (Ok(()), ServeExit::IntentionalStop),
            };
            let workers = drain_workers(&mut link).await;
            link.line.close();
            *control.report.lock().unwrap() = Some(DrainReport { serve, workers });
            control.drained.cancel();
            // Do not let the fixture restart between drain and supervisor stop.
            control.stop.cancelled().await;
            result
        })
    }
}

pub(super) struct Rig {
    pub(super) hub: Hub,
    pub(super) fake: Arc<Fake>,
    control: Arc<Control>,
    cancel: CancellationToken,
    driver: Option<JoinHandle<()>>,
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.control.login.release.notify_one();
        self.control.answer.release.notify_one();
        self.control.stop.cancel();
        self.cancel.cancel();
        // Emergency only; guard cases use with_rig's awaited shutdown even
        // after an assertion panics. Drop alone is not a drain proof.
        if let Some(driver) = &self.driver {
            driver.abort();
        }
    }
}

#[derive(Debug)]
struct ShutdownReport {
    drained: bool,
    supervisor_joined: bool,
    report: Option<DrainReport>,
}

impl ShutdownReport {
    fn resources_joined(&self) -> bool {
        self.drained && self.supervisor_joined && self.report.is_some()
    }

    fn healthy(&self) -> bool {
        self.resources_joined() && self.report.as_ref().is_some_and(DrainReport::healthy)
    }
}

impl Rig {
    async fn shutdown(mut self) -> ShutdownReport {
        if matches!(self.control.fault, Fault::ServeErrorAfterAssertions) {
            // Counter-case only: wait for the real serve error, not the stop
            // branch, after the assertion task has already completed.
            self.control.serve_failure.notify_one();
        } else {
            self.control.stop.cancel();
        }
        self.control.login.release.notify_one();
        self.control.answer.release.notify_one();
        let drained =
            tokio::time::timeout(Duration::from_secs(3), self.control.drained.cancelled())
                .await
                .is_ok();
        self.control.stop.cancel();
        self.cancel.cancel();
        let joined = if let Some(mut driver) = self.driver.take() {
            if let Ok(result) = tokio::time::timeout(Duration::from_secs(3), &mut driver).await {
                result.is_ok()
            } else {
                driver.abort();
                let _ = tokio::time::timeout(Duration::from_secs(3), driver).await;
                false
            }
        } else {
            false
        };
        let report = self.control.report.lock().unwrap().take();
        ShutdownReport {
            drained,
            supervisor_joined: joined,
            report,
        }
    }
}

pub(super) struct FixtureReport {
    assertion: Result<(), tokio::task::JoinError>,
    shutdown: ShutdownReport,
    writes_before: u32,
    writes_after: u32,
}

impl FixtureReport {
    pub(super) fn resources_joined(&self) -> bool {
        self.shutdown.resources_joined()
    }

    pub(super) fn assertions_passed(&self) -> bool {
        self.assertion.is_ok()
    }

    pub(super) fn worker_panic_recorded(&self) -> bool {
        self.shutdown.report.as_ref().is_some_and(|r| {
            r.workers.iter().any(|w| {
                matches!(w, WorkerExit::Panic(cause) if cause.contains("injected worker panic during fixture drain"))
            })
        })
    }

    pub(super) fn serve_error_recorded(&self) -> bool {
        self.shutdown.report.as_ref().is_some_and(|r| {
            matches!(
                &r.serve,
                ServeExit::ReturnedErr(cause) if cause.contains("injected worker failure observed by Link::serve")
            )
        })
    }

    pub(super) fn enforce(self) {
        assert!(
            self.shutdown.healthy(),
            "{FIXTURE_FAILURE}: {:?}; assertion task={:?}",
            self.shutdown,
            self.assertion
        );
        assert_eq!(
            self.writes_after, self.writes_before,
            "{FIXTURE_FAILURE}: mutation count changed through the completed drain"
        );
        if let Err(error) = self.assertion {
            if error.is_panic() {
                std::panic::resume_unwind(error.into_panic());
            }
            panic!("{FIXTURE_FAILURE}: assertion task did not finish within its bound: {error}");
        }
    }
}

async fn fixture_report<F, Fut>(
    fake: Fake,
    options: HubOptions,
    fault: Fault,
    test: F,
) -> FixtureReport
where
    F: FnOnce(Hub, Arc<Fake>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    // The fixture remains outside the assertion task. An assertion panic is
    // resumed only once the driver and its JoinSet have been joined.
    let rig = new_rig(fake, options, fault);
    let (hub, fake) = (rig.hub.clone(), Arc::clone(&rig.fake));
    let mut body = tokio::spawn(async move {
        until(|| ready(&hub)).await;
        test(hub, fake).await;
    });
    let result = if let Ok(result) = tokio::time::timeout(Duration::from_secs(15), &mut body).await
    {
        result
    } else {
        body.abort();
        if let Ok(result) = tokio::time::timeout(Duration::from_secs(3), &mut body).await {
            result
        } else {
            let cleanup = rig.shutdown().await;
            panic!(
                "{FIXTURE_FAILURE}: assertion task could not be joined after abort; cleanup={cleanup:?}"
            );
        }
    };
    let control = Arc::clone(&rig.control);
    let writes_before_drain = control.writes.load(SeqCst);
    let cleanup = rig.shutdown().await;
    FixtureReport {
        assertion: result,
        shutdown: cleanup,
        writes_before: writes_before_drain,
        writes_after: control.writes.load(SeqCst),
    }
}

pub(super) async fn with_rig<F, Fut>(fake: Fake, options: HubOptions, test: F)
where
    F: FnOnce(Hub, Arc<Fake>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    fixture_report(fake, options, Fault::None, test)
        .await
        .enforce();
}

pub(super) async fn fault_report(fault: Fault) -> FixtureReport {
    fixture_report(
        Fake::default(),
        HubOptions::default(),
        fault,
        |_, _| async {},
    )
    .await
}

fn new_rig(fake: Fake, options: HubOptions, fault: Fault) -> Rig {
    let hub = Hub::new(options).unwrap();
    let fake = Arc::new(fake);
    let control = Arc::new(Control {
        fault,
        ..Control::default()
    });
    control.reply_status.store(200, SeqCst);
    let cancel = CancellationToken::new();
    let driver = spawn_driver(
        &hub,
        InstanceId::from(INSTANCE),
        Arc::new(ContractDriver {
            fake: Arc::clone(&fake),
            control: Arc::clone(&control),
        }),
        cancel.clone(),
    );
    Rig {
        hub,
        fake,
        control,
        cancel,
        driver: Some(driver),
    }
}

async fn rig(fake: Fake, options: HubOptions) -> Rig {
    let rig = new_rig(fake, options, Fault::None);
    until(|| ready(&rig.hub)).await;
    rig
}

fn point() -> PointId {
    PointId::new(&heater(), CONTROL_KEY)
}

async fn caller(hub: &Hub, approval: bool) -> JoinHandle<()> {
    let approval_id = if approval {
        let held = hub
            .command(&point(), control_value(), Origin::Mcp, None)
            .await;
        let Err(CommandError::NeedsApproval { id, .. }) = held else {
            panic!("existing Control point must be held: {held:?}");
        };
        Some(id)
    } else {
        None
    };
    let hub = hub.clone();
    tokio::spawn(async move {
        if let Some(id) = approval_id {
            let _ = hub
                .resolve_approval(id, &point(), true, Origin::Ui, None)
                .await;
        } else {
            let _ = hub
                .command(&point(), control_value(), Origin::Ui, None)
                .await;
        }
    })
}

fn last_id(hub: &Hub) -> u64 {
    hub.journal(100).iter().map(|e| e.id).max().unwrap_or(0)
}

fn outcomes(hub: &Hub, after: u64, approval: bool) -> Vec<Outcome> {
    hub.journal(100)
        .iter()
        .filter(|e| e.id > after)
        .filter(|e| match &e.action {
            Action::Command { point: p, .. } => !approval && *p == point(),
            Action::Approval { .. } => approval,
            _ => false,
        })
        .filter(|e| !matches!(e.outcome, Outcome::Pending(_)))
        .map(|e| e.outcome.clone())
        .collect()
}

async fn expired_session(rig: &Rig) {
    rig.control.expire_session.store(true, SeqCst);
    let result = rig
        .hub
        .command(&point(), control_value(), Origin::Ui, None)
        .await;
    assert!(matches!(result, Err(CommandError::Driver(_))), "{result:?}");
    assert_eq!(
        rig.control.writes.load(SeqCst),
        1,
        "one refused setup request, never replayed"
    );
}

async fn abandon_during_login(approval: bool) {
    let rig = rig(Fake::default(), HubOptions::default()).await;
    expired_session(&rig).await;
    let before = last_id(&rig.hub);
    rig.control.hold_login.store(true, SeqCst);
    let call = caller(&rig.hub, approval).await;
    rig.control.login.reached().await;
    call.abort();
    assert!(call.await.unwrap_err().is_cancelled());
    rig.control.login.release.notify_one();
    until(|| !outcomes(&rig.hub, before, approval).is_empty()).await;
    assert_eq!(
        rig.control.writes.load(SeqCst),
        1,
        "abandoned request must not join the refused setup request"
    );
    assert!(matches!(
        outcomes(&rig.hub, before, approval).as_slice(),
        [Outcome::Err(_)]
    ));
}

#[tokio::test]
async fn p1_abandoned_during_login_direct() {
    abandon_during_login(false).await;
}

#[tokio::test]
async fn p1_abandoned_during_login_approval() {
    abandon_during_login(true).await;
}

#[tokio::test]
async fn p1_deadline_during_login_prevents_emission() {
    for approval in [false, true] {
        let rig = rig(
            Fake::default(),
            HubOptions {
                command_timeout: Duration::from_millis(1800),
                ..HubOptions::default()
            },
        )
        .await;
        expired_session(&rig).await;
        rig.control.hold_login.store(true, SeqCst);
        let call = caller(&rig.hub, approval).await;
        rig.control.login.reached().await;
        tokio::time::timeout(Duration::from_secs(3), call)
            .await
            .unwrap()
            .unwrap();
        rig.control.login.release.notify_one();
        until(|| rig.fake.logins.load(SeqCst) == 2).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            rig.control.writes.load(SeqCst),
            1,
            "deadline prevents a second mutation"
        );
    }
}

#[tokio::test]
async fn p1_without_abandonment_login_emits_once() {
    for approval in [false, true] {
        let rig = rig(Fake::default(), HubOptions::default()).await;
        expired_session(&rig).await;
        let before = last_id(&rig.hub);
        rig.control.hold_login.store(true, SeqCst);
        let call = caller(&rig.hub, approval).await;
        rig.control.login.reached().await;
        rig.control.login.release.notify_one();
        tokio::time::timeout(Duration::from_secs(3), call)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            rig.control.writes.load(SeqCst),
            2,
            "one setup refusal, one requested mutation"
        );
        assert_eq!(outcomes(&rig.hub, before, approval), [Outcome::Ok]);
    }
}

#[tokio::test]
async fn p1_after_emission_keeps_the_result_when_the_caller_leaves() {
    for approval in [false, true] {
        for leaves in [false, true] {
            for status in [200, 500, 0] {
                let rig = rig(Fake::default(), HubOptions::default()).await;
                let before = last_id(&rig.hub);
                rig.control.reply_status.store(status, SeqCst);
                rig.control.hold_answer.store(true, SeqCst);
                let call = caller(&rig.hub, approval).await;
                rig.control.answer.reached().await;
                assert_eq!(
                    rig.control.writes.load(SeqCst),
                    1,
                    "complete mutation received before caller cancellation"
                );
                if leaves {
                    call.abort();
                    assert!(call.await.unwrap_err().is_cancelled());
                } else {
                    rig.control.answer.release.notify_one();
                    tokio::time::timeout(Duration::from_secs(3), call)
                        .await
                        .unwrap()
                        .unwrap();
                }
                if leaves {
                    rig.control.answer.release.notify_one();
                }
                until(|| !outcomes(&rig.hub, before, approval).is_empty()).await;
                let results = outcomes(&rig.hub, before, approval);
                if status == 200 {
                    assert_eq!(results, [Outcome::Ok]);
                    until(|| value(&rig.hub, &heater(), CONTROL_KEY) == Some(control_value()))
                        .await;
                } else {
                    let [Outcome::Err(message)] = results.as_slice() else {
                        panic!(
                            "a received refusal/lost response must remain an error: {results:?}"
                        );
                    };
                    let expected = if status == 500 {
                        "500"
                    } else {
                        "simulated connection lost"
                    };
                    assert!(
                        message.contains(expected),
                        "RPC result lost after caller departure: {message}"
                    );
                }
                assert_eq!(
                    rig.control.writes.load(SeqCst),
                    1,
                    "no replay after any outcome"
                );
            }
        }
    }
}
