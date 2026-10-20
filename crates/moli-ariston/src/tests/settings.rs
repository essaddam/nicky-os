//! Deterministic admission/ACK/read/dispatch interleavings using the real
//! order queue, model, cloud API and hub. Only the cloud peer is simulated.
use super::*;

#[derive(Clone, Copy, Debug)]
enum Scenario {
    AckCeiling,
    ReadCeiling,
    SettingsAgain,
}

impl Scenario {
    fn pair(self) -> [(&'static str, Value); 2] {
        match self {
            Self::AckCeiling => [
                ("max_setpoint", Value::Float(65.0)),
                ("setpoint", Value::Float(75.0)),
            ],
            Self::ReadCeiling => [("on", Value::Bool(false)), ("setpoint", Value::Float(75.0))],
            Self::SettingsAgain => [
                ("max_setpoint", Value::Float(75.0)),
                ("max_setpoint", Value::Float(70.0)),
            ],
        }
    }
}

#[derive(Debug)]
struct Script {
    fake: Arc<Fake>,
    scenario: Scenario,
}

fn cloud(fake: Arc<Fake>) -> Cloud {
    let mut cloud = Cloud::new("user@example.com".into(), "x".into(), Arc::default());
    cloud.exchange = Some(Arc::new(move |request| {
        let fake = Arc::clone(&fake);
        Box::pin(async move {
            let (status, body) = fake.answer(request).await;
            Ok((StatusCode::from_u16(status)?, Body::from(body.to_string())))
        })
    }));
    cloud
}

async fn refresh(link: &mut Link, ctx: &DriverCtx) -> anyhow::Result<()> {
    link.read_started = Instant::now();
    let kind = Read {
        list: true,
        settings: true,
        errors: true,
    };
    let snapshot = fetch(&link.cloud, Vec::new(), kind).await?;
    link.done(ctx, Done::Read(kind, Ok(snapshot)))
}

async fn finish_send(link: &mut Link, ctx: &DriverCtx) -> anyhow::Result<()> {
    let done = tokio::time::timeout(Duration::from_secs(3), link.jobs.join_next())
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing send worker"))??;
    link.done(ctx, done)
}

impl Driver for Script {
    fn kind(&self) -> &'static str {
        "settings-contract"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let mut link = Link::new(cloud(Arc::clone(&self.fake)), TEST_PACE);
            refresh(&mut link, ctx).await?;
            let first = ctx.next_command().await.unwrap();
            link.order(first);
            link.pump(Instant::now());
            // The second command is admitted before handling the first ACK.
            let second = ctx.next_command().await.unwrap();
            link.order(second);
            finish_send(&mut link, ctx).await?;
            assert_eq!(
                ctx.current(&heater(), "max_setpoint"),
                Some(Value::Float(80.0)),
                "an ACK is not telemetry"
            );
            if matches!(self.scenario, Scenario::ReadCeiling) {
                self.fake.settings.lock().unwrap()["MedMaxSetpointTemperature"] = json!(65.0);
                refresh(&mut link, ctx).await?;
            }
            link.pump(Instant::now());
            // A setting-dependent intention can wait for a post-ACK read.
            if link.jobs.is_empty() && !link.line.is_empty() {
                refresh(&mut link, ctx).await?;
                link.pump(Instant::now());
            }
            if !link.jobs.is_empty() {
                finish_send(&mut link, ctx).await?;
            }
            ctx.wait_for("script finished");
            ctx.cancelled().await;
            Ok(())
        })
    }
}

async fn run(scenario: Scenario) -> (Arc<Fake>, Result<(), String>, Result<(), String>) {
    let hub = Hub::new(HubOptions::default()).unwrap();
    let fake = Arc::new(Fake::default());
    let cancel = tokio_util::sync::CancellationToken::new();
    let task = spawn_driver(
        &hub,
        InstanceId::from("eau-chaude"),
        Arc::new(Script {
            fake: Arc::clone(&fake),
            scenario,
        }),
        cancel.clone(),
    );
    until(|| value(&hub, &heater(), "max_setpoint").is_some()).await;
    let [(first_key, first_value), (second_key, second_value)] = scenario.pair();
    let caller = hub.clone();
    let first = tokio::spawn(async move { command(&caller, first_key, first_value).await });
    until(|| fake.posts().len() == 1).await;
    let caller = hub.clone();
    let second = tokio::spawn(async move { command(&caller, second_key, second_value).await });
    let results = tokio::time::timeout(Duration::from_secs(3), async {
        (first.await.unwrap(), second.await.unwrap())
    })
    .await;
    cancel.cancel();
    task.await.unwrap();
    let (first, second) = results.expect("both commands settle");
    (fake, first, second)
}

#[tokio::test]
async fn acknowledged_lower_ceiling_rejects_a_queued_hotter_setpoint() {
    let (fake, first, second) = run(Scenario::AckCeiling).await;
    assert_eq!(first, Ok(()));
    assert!(
        second.is_err(),
        "75 must not follow an acknowledged ceiling of 65"
    );
    assert_eq!(fake.posts().len(), 1, "no hotter setpoint emitted");
}

#[tokio::test]
async fn a_new_settings_read_revalidates_the_queued_intention() {
    let (fake, first, second) = run(Scenario::ReadCeiling).await;
    assert_eq!(first, Ok(()));
    assert!(
        second.is_err(),
        "queued 75 must be rechecked against newly read 65"
    );
    assert_eq!(fake.posts().len(), 1);
}

#[tokio::test]
async fn successive_setting_mutations_encode_the_newly_read_old_value() {
    let (fake, first, second) = run(Scenario::SettingsAgain).await;
    assert_eq!(first, Ok(()));
    assert_eq!(second, Ok(()));
    let posts = fake.posts();
    assert_eq!(posts.len(), 2);
    assert_eq!(
        posts[1].1,
        json!({ "MedMaxSetpointTemperature": { "old": 75.0, "new": 70.0 } })
    );
}
