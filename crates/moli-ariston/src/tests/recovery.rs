//! Recovery contracts through the real serve loop; only the cloud is simulated.
use super::*;
use std::sync::atomic::AtomicBool;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

const CONFIRM: Duration = Duration::from_millis(1500);
const PAUSE: Duration = Duration::from_millis(1800);
const PACE: Pace = Pace {
    idle: Duration::from_millis(50),
    follow: Duration::from_millis(20),
    confirm: CONFIRM,
};

#[derive(Clone, Copy, Debug)]
enum Case {
    Quota,
    Other,
    Lost,
    External,
    StaleRead,
}

#[derive(Debug, Default)]
struct Control {
    reply_at: Mutex<Option<Instant>>,
    reads: Mutex<Vec<Instant>>,
    first_post: AtomicBool,
    held_read: AtomicBool,
    read_entered: Notify,
    read_release: Notify,
    stop: CancellationToken,
    finished: CancellationToken,
    failures: Mutex<Vec<String>>,
}

fn cloud(fake: Arc<Fake>, control: Arc<Control>, case: Case) -> Cloud {
    let mut cloud = Cloud::new("user@example.com".into(), "x".into(), Arc::default());
    cloud.exchange = Some(Arc::new(move |request| {
        let (fake, control) = (Arc::clone(&fake), Arc::clone(&control));
        Box::pin(async move {
            let setting = request.uri().path().ends_with("/plantSettings");
            let first = setting
                && request.method() == http::Method::POST
                && !control.first_post.swap(true, SeqCst);
            if setting && request.method() == http::Method::GET {
                let now = Instant::now();
                control.reads.lock().unwrap().push(now);
                if matches!(case, Case::StaleRead)
                    && control.reply_at.lock().unwrap().is_some()
                    && !control.held_read.swap(true, SeqCst)
                {
                    let (status, body) = fake.answer(request).await;
                    // Capture the matching 65 before the limit, then let
                    // another client change it while this GET is held.
                    fake.settings.lock().unwrap()["MedMaxSetpointTemperature"] = json!(75.0);
                    control.read_entered.notify_one();
                    control.read_release.notified().await;
                    return Ok((StatusCode::from_u16(status)?, Body::from(body.to_string())));
                }
                let expired = control
                    .reply_at
                    .lock()
                    .unwrap()
                    .is_some_and(|at| now >= at + CONFIRM);
                if expired && !control.held_read.swap(true, SeqCst) {
                    // A genuinely new base differs from both the cached 80
                    // and the unconfirmed requested value 65.
                    fake.settings.lock().unwrap()["MedMaxSetpointTemperature"] = json!(75.0);
                    control.read_entered.notify_one();
                    control.read_release.notified().await;
                }
            }
            let (status, body) = fake.answer(request).await;
            if first {
                if matches!(case, Case::External) {
                    fake.settings.lock().unwrap()["MedMaxSetpointTemperature"] = json!(75.0);
                }
                *control.reply_at.lock().unwrap() = Some(Instant::now());
            }
            if status == 599 {
                anyhow::bail!("simulated lost reply after a setting request without effect");
            }
            Ok((StatusCode::from_u16(status)?, Body::from(body.to_string())))
        })
    }));
    cloud
}

#[derive(Debug)]
struct RecoveryDriver {
    fake: Arc<Fake>,
    control: Arc<Control>,
    case: Case,
}

impl Driver for RecoveryDriver {
    fn kind(&self) -> &'static str {
        "recovery-contract"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let mut pace = PACE;
            if matches!(self.case, Case::StaleRead) {
                pace.idle = Duration::from_secs(10);
            }
            let mut link = Link::new(
                cloud(Arc::clone(&self.fake), Arc::clone(&self.control), self.case),
                pace,
            );
            link.busy_pause = PAUSE;
            let outcome = tokio::select! {
                result = link.serve(ctx) => Some(result),
                () = self.control.stop.cancelled() => None,
            };
            if let Some(outcome) = outcome {
                self.control
                    .failures
                    .lock()
                    .unwrap()
                    .push(format!("serve returned before fixture stop: {outcome:?}"));
            }
            link.jobs.abort_all();
            while let Some(result) = link.jobs.join_next().await {
                match result {
                    Ok(Done::Read(_, Ok(_)) | Done::Sent(_, Ok(()))) => {}
                    Err(error) if error.is_cancelled() => {}
                    other => self
                        .control
                        .failures
                        .lock()
                        .unwrap()
                        .push(format!("worker failed during fixture drain: {other:?}")),
                }
            }
            link.line.close();
            link.confirm.close();
            self.control.finished.cancel();
            self.control.stop.cancelled().await;
            Ok(())
        })
    }
}

async fn body(hub: Hub, fake: Arc<Fake>, control: Arc<Control>, case: Case) {
    until(|| value(&hub, &heater(), "max_setpoint").is_some()).await;
    let mut events = hub.subscribe();
    let first = command(&hub, "max_setpoint", Value::Float(65.0)).await;
    assert_eq!(
        first.is_ok(),
        matches!(case, Case::External | Case::StaleRead),
        "first result: {first:?}"
    );
    let old = command(&hub, "setpoint", Value::Float(74.0)).await;
    assert!(
        old.is_err(),
        "the obsolete intention must not leave before recovery"
    );
    let mut told = Vec::new();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            told.extend(notices(&mut events));
            if !told.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("confirmation expires even during quota pause");
    tokio::time::timeout(Duration::from_secs(3), control.read_entered.notified())
        .await
        .expect("post-limit read started");
    assert_eq!(
        fake.posts().len(),
        1,
        "old command is never replayed or released after expiry"
    );
    let caller = hub.clone();
    let next =
        tokio::spawn(async move { command(&caller, "max_setpoint", Value::Float(70.0)).await });
    tokio::task::yield_now().await;
    assert_eq!(
        fake.posts().len(),
        1,
        "a new intention awaits the held fresh read"
    );
    control.read_release.notify_one();
    let result = tokio::time::timeout(Duration::from_secs(3), next)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        result,
        Ok(()),
        "a new intention must recover on a healthy divergent base"
    );
    until(|| value(&hub, &heater(), "max_setpoint") == Some(Value::Float(70.0))).await;
    told.extend(notices(&mut events));
    assert_eq!(
        told.len(),
        1,
        "one notice for the unconfirmed old setting: {told:?}"
    );
    let posts = fake.posts();
    assert_eq!(
        posts.len(),
        2,
        "only the first and genuinely new settings were sent"
    );
    assert_eq!(
        posts[1].1,
        json!({"MedMaxSetpointTemperature": {"old":75.0, "new":70.0}})
    );
    if matches!(case, Case::Quota) {
        let reply = control.reply_at.lock().unwrap().unwrap();
        assert!(
            control
                .reads
                .lock()
                .unwrap()
                .iter()
                .all(|at| *at <= reply || *at >= reply + PAUSE),
            "quota pause must remain effective"
        );
    }
}

async fn exercise(case: Case) {
    let hub = Hub::new(HubOptions {
        command_timeout: Duration::from_millis(2500),
        ..HubOptions::default()
    })
    .unwrap();
    let status = match case {
        Case::Quota => Some(429),
        Case::Other => Some(500),
        Case::Lost => Some(599),
        Case::External | Case::StaleRead => None,
    };
    let fake = Arc::new(Fake {
        post_status_once: Mutex::new(status),
        ..Fake::default()
    });
    let control = Arc::new(Control::default());
    let cancel = CancellationToken::new();
    let driver = spawn_driver(
        &hub,
        InstanceId::from("eau-chaude"),
        Arc::new(RecoveryDriver {
            fake: Arc::clone(&fake),
            control: Arc::clone(&control),
            case,
        }),
        cancel.clone(),
    );
    let mut assertions = tokio::spawn(body(hub, fake, Arc::clone(&control), case));
    let result =
        if let Ok(result) = tokio::time::timeout(Duration::from_secs(10), &mut assertions).await {
            result
        } else {
            assertions.abort();
            assertions.await
        };
    control.read_release.notify_one();
    control.stop.cancel();
    let drained = tokio::time::timeout(Duration::from_secs(3), control.finished.cancelled())
        .await
        .is_ok();
    cancel.cancel();
    let joined = tokio::time::timeout(Duration::from_secs(3), driver).await;
    assert!(
        drained && matches!(joined, Ok(Ok(()))),
        "recovery fixture did not join its resources"
    );
    let failures = control.failures.lock().unwrap().clone();
    assert!(
        failures.is_empty(),
        "recovery fixture failures: {failures:?}"
    );
    if let Err(error) = result {
        if error.is_panic() {
            std::panic::resume_unwind(error.into_panic());
        }
        panic!("recovery assertion task did not finish: {error}");
    }
}

#[tokio::test]
async fn quota_refusal_recovers_on_fresh_divergent_settings() {
    exercise(Case::Quota).await;
}
#[tokio::test]
async fn server_refusal_recovers_on_fresh_divergent_settings() {
    exercise(Case::Other).await;
}
#[tokio::test]
async fn lost_reply_recovers_on_fresh_divergent_settings() {
    exercise(Case::Lost).await;
}
#[tokio::test]
async fn external_change_after_ack_recovers_on_fresh_divergent_settings() {
    exercise(Case::External).await;
}

#[tokio::test]
async fn prelimit_matching_read_cannot_replace_the_recovery_read() {
    exercise(Case::StaleRead).await;
}
