//! Supervised driver contracts against the real hub and an encrypted temp vault.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::future::Future as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::task::Poll;
use std::time::Duration;

use http::{HeaderMap, StatusCode, header};
use moli_core::{DeviceId, Event, InstanceId, Origin, PointId, Value};
use moli_net::{Body, OnceError};
use moli_runtime::{Hub, HubOptions, MasterKey, spawn_driver};
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::{Value as Json, json};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::{
    Config, Error, Gateway, Netatmo, Status,
    actor::{Clock, SaveGate},
};

const EPOCH: u64 = 1_700_000_000;
const STATION: &str = "aa:bb:cc:dd:ee:01";
const OUTDOOR: &str = "aa:bb:cc:dd:ee:02";
type Reply = Result<(StatusCode, Body), OnceError>;
type WeatherCheck = Arc<dyn Fn(&str) + Send + Sync>;

fn generated() -> String {
    let mut bytes = [0_u8; 32];
    SystemRandom::new().fill(&mut bytes).unwrap();
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(encoded, "{byte:02x}").unwrap();
    }
    encoded
}

fn instance() -> InstanceId {
    InstanceId::from("weather")
}
fn device(native: &str) -> DeviceId {
    DeviceId::new(&instance(), native)
}
fn point(native: &str) -> PointId {
    PointId::new(&device(native), "temperature")
}

fn bundle(access: &str, refresh: &str, expires: u64) -> String {
    json!({"version":1,"access_token":access,"refresh_token":refresh,"expires_at":expires})
        .to_string()
}

fn options(path: PathBuf, key: MasterKey) -> HubOptions {
    HubOptions {
        secrets: Some((path, key)),
        ..HubOptions::default()
    }
}

struct Provider {
    clock: Clock,
    access: String,
    refresh: String,
    token_count: AtomicUsize,
    weather_count: AtomicUsize,
    token_answers: Mutex<VecDeque<Reply>>,
    fixed_time: AtomicBool,
    changed_value: AtomicBool,
    omit_outdoor: AtomicBool,
    hold_weather: AtomicBool,
    hold_token: AtomicBool,
    check_weather: Mutex<Option<WeatherCheck>>,
}

impl Provider {
    fn new(clock: Clock) -> Arc<Self> {
        Arc::new(Self {
            clock,
            access: generated(),
            refresh: generated(),
            token_count: AtomicUsize::new(0),
            weather_count: AtomicUsize::new(0),
            token_answers: Mutex::new(VecDeque::new()),
            fixed_time: AtomicBool::new(false),
            changed_value: AtomicBool::new(false),
            omit_outdoor: AtomicBool::new(false),
            hold_weather: AtomicBool::new(false),
            hold_token: AtomicBool::new(false),
            check_weather: Mutex::new(None),
        })
    }

    fn exchange(self: &Arc<Self>) -> crate::api::Exchange {
        let provider = self.clone();
        Arc::new(move |request, _| {
            let provider = provider.clone();
            Box::pin(async move {
                let (status, body) =
                    if request.uri().path() == "/oauth2/token" {
                        provider.token_count.fetch_add(1, SeqCst);
                        if provider.hold_token.load(SeqCst) {
                            std::future::pending::<()>().await;
                        }
                        let answer = { provider.token_answers.lock().unwrap().pop_front() };
                        answer.unwrap_or_else(|| {
                            Ok((StatusCode::OK,Body::from(json!({
                        "access_token":provider.access,"refresh_token":provider.refresh,
                        "expires_in":3600,"scope":["read_station"]
                    }).to_string())))
                        })?
                    } else {
                        assert_eq!(
                            request.uri().to_string(),
                            "/api/getstationsdata?get_favorites=false"
                        );
                        provider.weather_count.fetch_add(1, SeqCst);
                        let check = provider.check_weather.lock().unwrap().clone();
                        if let Some(check) = check {
                            check(request.headers()[header::AUTHORIZATION].to_str().unwrap());
                        }
                        if provider.hold_weather.load(SeqCst) {
                            std::future::pending::<()>().await;
                        }
                        (StatusCode::OK, provider.weather())
                    };
                Ok((status, HeaderMap::new(), body))
            })
        })
    }

    fn weather(&self) -> Body {
        let mut json: Json = serde_json::from_slice(include_bytes!(
            "../../../integrations/netatmo/fixtures/weather.json"
        ))
        .unwrap();
        let stamp = if self.fixed_time.load(SeqCst) {
            EPOCH
        } else {
            self.clock.now()
        };
        let station = &mut json["body"]["devices"][0];
        station["dashboard_data"]["time_utc"] = json!(stamp);
        if self.changed_value.load(SeqCst) {
            station["dashboard_data"]["Temperature"] = json!(22.5);
        }
        for module in station["modules"].as_array_mut().unwrap() {
            module["dashboard_data"]["time_utc"] = json!(stamp);
        }
        if self.omit_outdoor.load(SeqCst) {
            station["modules"]
                .as_array_mut()
                .unwrap()
                .retain(|m| m["_id"] != OUTDOOR);
        }
        Body::from(json.to_string())
    }
}

struct Rig {
    root: PathBuf,
    vault: PathBuf,
    key: MasterKey,
    hub: Hub,
    clock: Clock,
    provider: Arc<Provider>,
    driver: Option<Netatmo>,
    gateway: Gateway,
    cancel: CancellationToken,
    task: Option<JoinHandle<()>>,
}

impl Rig {
    async fn new(expires: Option<u64>) -> Self {
        let root = std::env::temp_dir().join(format!("moli-netatmo-{}", generated()));
        std::fs::create_dir(&root).unwrap();
        let vault = root.join("store/vault");
        let key = MasterKey::generate().unwrap();
        let hub = Hub::new(options(vault.clone(), key.clone())).unwrap();
        hub.store_secret(&instance(), "client_id", &generated())
            .await
            .unwrap();
        hub.store_secret(&instance(), "client_secret", &generated())
            .await
            .unwrap();
        if let Some(expires) = expires {
            hub.store_secret(
                &instance(),
                "oauth_tokens",
                &bundle(&generated(), &generated(), expires),
            )
            .await
            .unwrap();
        }
        let clock = Clock::at(EPOCH);
        let provider = Provider::new(clock);
        let driver = Self::driver(clock, &provider);
        let gateway = driver.gateway();
        Self {
            root,
            vault,
            key,
            hub,
            clock,
            provider,
            driver: Some(driver),
            gateway,
            cancel: CancellationToken::new(),
            task: None,
        }
    }

    fn driver(clock: Clock, provider: &Arc<Provider>) -> Netatmo {
        let config: Config = serde_json::from_value(
            json!({"public_origin":"https://example.com","poll_seconds":60,"stale_seconds":300}),
        )
        .unwrap();
        let mut driver = Netatmo::new(config).unwrap();
        driver.clock = clock;
        driver.exchange = Some(provider.exchange());
        driver
    }

    fn start(&mut self) {
        self.task = Some(spawn_driver(
            &self.hub,
            instance(),
            Arc::new(self.driver.take().unwrap()),
            self.cancel.clone(),
        ));
    }

    async fn stop(&mut self) {
        self.cancel.cancel();
        if let Some(task) = self.task.take() {
            task.await.unwrap();
        }
    }

    async fn restart(&mut self) {
        self.stop().await;
        self.hub = Hub::new(options(self.vault.clone(), self.key.clone())).unwrap();
        self.provider = Provider::new(self.clock);
        let driver = Self::driver(Clock::at(self.clock.now()), &self.provider);
        self.gateway = driver.gateway();
        self.driver = Some(driver);
        self.cancel = CancellationToken::new();
    }

    fn online(&self, native: &str) -> bool {
        self.hub
            .device(&device(native))
            .is_some_and(|d| d.online == Some(true))
    }

    fn stored(&self) -> String {
        self.hub.secret(&instance(), "oauth_tokens").unwrap()
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(task) = &self.task {
            task.abort();
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Wall-clock bound, independent of the paused Tokio clock. Yielding keeps
/// filesystem completions runnable without advancing the simulated schedule.
async fn until(check: impl Fn() -> bool) {
    let end = std::time::Instant::now() + Duration::from_secs(1);
    while !check() && std::time::Instant::now() < end {
        tokio::task::yield_now().await;
    }
    assert!(check(), "condition within one second");
}

async fn pump() {
    for _ in 0..200 {
        tokio::task::yield_now().await;
    }
}
async fn step(seconds: u64) {
    tokio::time::advance(Duration::from_secs(seconds)).await;
    pump().await;
}

fn state(url: &str) -> String {
    url.split_once('?')
        .unwrap()
        .1
        .split('&')
        .find_map(|p| p.strip_prefix("state="))
        .unwrap()
        .to_owned()
}

#[tokio::test(start_paused = true)]
async fn authorize_never_calls_provider() {
    let mut rig = Rig::new(None).await;
    rig.start();
    let authorization = rig.gateway.authorize([1; 32]).await.unwrap();
    assert_eq!(state(&authorization.url).len(), 64);
    assert_eq!(rig.provider.token_count.load(SeqCst), 0);
    assert_eq!(rig.provider.weather_count.load(SeqCst), 0);
    rig.stop().await;
}

#[tokio::test(start_paused = true)]
async fn exact_session_and_single_use_gate_the_exchange() {
    let mut rig = Rig::new(None).await;
    rig.start();
    let pending = state(&rig.gateway.authorize([1; 32]).await.unwrap().url);
    assert_eq!(
        rig.gateway
            .complete([2; 32], pending.clone(), generated())
            .await,
        Err(Error::Unauthorized)
    );
    assert_eq!(rig.provider.token_count.load(SeqCst), 0);
    let (first, second) = tokio::join!(
        rig.gateway.complete([1; 32], pending.clone(), generated()),
        rig.gateway.complete([1; 32], pending, generated())
    );
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    assert_eq!(rig.provider.token_count.load(SeqCst), 1);
    until(|| rig.online(STATION)).await;
    rig.stop().await;
}

#[tokio::test(start_paused = true)]
async fn rotation_persists_before_any_use() {
    let mut rig = Rig::new(Some(EPOCH + 30)).await;
    let hub = rig.hub.clone();
    let access = rig.provider.access.clone();
    let refresh = rig.provider.refresh.clone();
    *rig.provider.check_weather.lock().unwrap() = Some(Arc::new(move |bearer| {
        let stored: Json =
            serde_json::from_str(&hub.secret(&instance(), "oauth_tokens").unwrap()).unwrap();
        assert_eq!(stored["version"], 1);
        assert!(
            stored["access_token"] == access && stored["refresh_token"] == refresh,
            "complete new pair durable before GET"
        );
        // Keep the generated bearer credential out of assertion diagnostics.
        assert!(bearer.eq(&format!("Bearer {access}")));
    }));
    rig.start();
    until(|| rig.online(STATION)).await;
    assert_eq!(rig.provider.token_count.load(SeqCst), 1);
    rig.stop().await;
}

#[tokio::test(start_paused = true)]
async fn storage_failure_never_activates_new_tokens() {
    let mut rig = Rig::new(Some(EPOCH + 30)).await;
    std::fs::rename(rig.root.join("store"), rig.root.join("previous")).unwrap();
    std::fs::write(rig.root.join("store"), b"obstruction").unwrap();
    rig.start();
    until(|| rig.gateway.status() == Status::NeedsAuthorization).await;
    assert_eq!(rig.provider.token_count.load(SeqCst), 1);
    assert_eq!(rig.provider.weather_count.load(SeqCst), 0);
    assert!(!rig.online(STATION));
    rig.stop().await;
}

#[tokio::test(start_paused = true)]
async fn restart_reads_the_durable_bundle() {
    let mut rig = Rig::new(Some(EPOCH + 10000)).await;
    let before = rig.stored();
    rig.start();
    until(|| rig.online(STATION)).await;
    rig.restart().await;
    assert_eq!(rig.stored(), before);
    rig.start();
    until(|| rig.online(STATION)).await;
    assert_eq!(rig.provider.token_count.load(SeqCst), 0);
    rig.stop().await;
}

#[tokio::test(start_paused = true)]
async fn lost_refresh_response_is_not_replayed() {
    let mut rig = Rig::new(Some(EPOCH + 30)).await;
    rig.provider
        .token_answers
        .lock()
        .unwrap()
        .push_back(Err(OnceError::AfterSend));
    rig.start();
    until(|| rig.gateway.status() == Status::NeedsAuthorization).await;
    assert_eq!(rig.provider.token_count.load(SeqCst), 1);
    step(1200).await;
    assert_eq!(rig.provider.token_count.load(SeqCst), 1);
    assert_eq!(rig.provider.weather_count.load(SeqCst), 0);
    rig.stop().await;
}

#[tokio::test(start_paused = true)]
async fn failure_before_send_keeps_bundle_and_retries_after_backoff() {
    let mut rig = Rig::new(Some(EPOCH + 30)).await;
    let before = rig.stored();
    rig.provider
        .token_answers
        .lock()
        .unwrap()
        .push_back(Err(OnceError::BeforeSend));
    rig.start();
    until(|| rig.gateway.status() == Status::Unavailable).await;
    assert_eq!(rig.stored(), before);
    step(59).await;
    assert_eq!(rig.provider.token_count.load(SeqCst), 1);
    step(1).await;
    until(|| rig.online(STATION)).await;
    assert_eq!(rig.provider.token_count.load(SeqCst), 2);
    rig.stop().await;
}

#[tokio::test(start_paused = true)]
async fn simultaneous_refresh_and_human_completion_are_single_flight() {
    let mut rig = Rig::new(Some(EPOCH + 30)).await;
    rig.provider.hold_token.store(true, SeqCst);
    rig.start();
    until(|| rig.provider.token_count.load(SeqCst) == 1).await;
    let pending = state(&rig.gateway.authorize([1; 32]).await.unwrap().url);
    assert_eq!(
        rig.gateway.complete([1; 32], pending, generated()).await,
        Err(Error::Busy)
    );
    step(600).await;
    assert_eq!(rig.provider.token_count.load(SeqCst), 1);
    rig.stop().await;
}

#[tokio::test(start_paused = true)]
async fn repeated_timestamp_expires_instead_of_refreshing_availability() {
    let mut rig = Rig::new(Some(EPOCH + 10000)).await;
    rig.provider.fixed_time.store(true, SeqCst);
    rig.start();
    until(|| rig.online(STATION)).await;
    for _ in 0..5 {
        step(60).await;
    }
    step(1).await;
    until(|| !rig.online(STATION)).await;
    assert_eq!(rig.hub.state(&point(STATION)).unwrap().value, Value::Null);
    rig.stop().await;
}

#[tokio::test(start_paused = true)]
async fn expiry_fires_while_network_is_pending_and_controls_still_answer() {
    let mut rig = Rig::new(Some(EPOCH + 10000)).await;
    rig.provider.fixed_time.store(true, SeqCst);
    rig.start();
    until(|| rig.online(STATION)).await;
    rig.provider.hold_weather.store(true, SeqCst);
    step(60).await;
    until(|| rig.provider.weather_count.load(SeqCst) == 2).await;
    step(241).await;
    until(|| !rig.online(STATION)).await;
    assert_eq!(rig.hub.state(&point(STATION)).unwrap().value, Value::Null);
    assert!(rig.gateway.authorize([1; 32]).await.is_ok());
    rig.stop().await;
}

#[tokio::test(start_paused = true)]
async fn repeated_source_timestamp_cannot_publish_a_changed_value() {
    let mut rig = Rig::new(Some(EPOCH + 10000)).await;
    rig.provider.fixed_time.store(true, SeqCst);
    rig.start();
    until(|| rig.online(STATION)).await;
    rig.provider.changed_value.store(true, SeqCst);
    step(60).await;
    until(|| rig.provider.weather_count.load(SeqCst) == 2).await;
    pump().await;
    assert_eq!(
        rig.hub.state(&point(STATION)).unwrap().value.as_f64(),
        Some(21.5)
    );
    rig.stop().await;
}

#[tokio::test(start_paused = true)]
async fn stable_value_at_new_timestamp_does_not_inflate_state_events() {
    let mut rig = Rig::new(Some(EPOCH + 10000)).await;
    rig.start();
    until(|| rig.online(STATION)).await;
    let mut events = rig.hub.subscribe();
    step(60).await;
    until(|| rig.provider.weather_count.load(SeqCst) == 2).await;
    pump().await;
    while let Ok(event) = events.try_recv() {
        assert!(!matches!(event,Event::State {point:ref changed,..} if *changed==point(STATION)));
    }
    assert!(rig.online(STATION));
    rig.stop().await;
}

#[tokio::test(start_paused = true)]
async fn missing_module_goes_offline_and_returns_with_the_same_identity() {
    let mut rig = Rig::new(Some(EPOCH + 10000)).await;
    rig.start();
    until(|| rig.online(OUTDOOR)).await;
    let id = rig.hub.device(&device(OUTDOOR)).unwrap().device.id.clone();
    rig.provider.omit_outdoor.store(true, SeqCst);
    step(60).await;
    until(|| !rig.online(OUTDOOR)).await;
    assert_eq!(rig.hub.state(&point(OUTDOOR)).unwrap().value, Value::Null);
    rig.provider.omit_outdoor.store(false, SeqCst);
    step(60).await;
    until(|| rig.online(OUTDOOR)).await;
    assert_eq!(rig.hub.device(&device(OUTDOOR)).unwrap().device.id, id);
    rig.stop().await;
}

#[tokio::test(start_paused = true)]
async fn commands_are_read_only() {
    let mut rig = Rig::new(Some(EPOCH + 10000)).await;
    rig.start();
    until(|| rig.online(STATION)).await;
    let error = rig
        .hub
        .command(&point(STATION), Value::Float(22.0), Origin::Ui, None)
        .await
        .unwrap_err();
    assert!(matches!(error, moli_runtime::CommandError::ReadOnly));
    assert_eq!(rig.provider.token_count.load(SeqCst), 0);
    assert_eq!(rig.provider.weather_count.load(SeqCst), 1);
    rig.stop().await;
}

#[tokio::test(start_paused = true)]
async fn cancellation_invalidates_availability_and_pending_authorization() {
    let mut rig = Rig::new(Some(EPOCH + 10000)).await;
    rig.start();
    until(|| rig.online(STATION)).await;
    let pending = state(&rig.gateway.authorize([1; 32]).await.unwrap().url);
    rig.stop().await;
    assert!(!rig.online(STATION));
    assert_eq!(
        rig.gateway.complete([1; 32], pending, generated()).await,
        Err(Error::Unavailable)
    );
    assert_eq!(rig.gateway.status(), Status::Unavailable);
}

#[tokio::test(start_paused = true)]
async fn cancelled_save_leaves_a_readable_pair_and_restart_never_loops() {
    let mut rig = Rig::new(Some(EPOCH + 30)).await;
    let before: Json = serde_json::from_str(&rig.stored()).unwrap();
    let expected = (rig.provider.access.clone(), rig.provider.refresh.clone());
    let gate = Arc::new(SaveGate::default());
    rig.driver.as_mut().unwrap().persist_gate = Some(gate.clone());
    rig.start();
    until(|| gate.entered.load(SeqCst)).await;
    rig.restart().await;
    let after: Json = serde_json::from_str(&rig.stored()).unwrap();
    assert!(
        after == before
            || (after["access_token"] == expected.0 && after["refresh_token"] == expected.1),
        "old or complete new pair only"
    );
    rig.provider.token_answers.lock().unwrap().push_back(Ok((
        StatusCode::BAD_REQUEST,
        Body::from(json!({"error":"invalid_grant"}).to_string()),
    )));
    rig.start();
    until(|| rig.online(STATION) || rig.gateway.status() == Status::NeedsAuthorization).await;
    assert!(rig.provider.token_count.load(SeqCst) <= 1);
    if rig.provider.token_count.load(SeqCst) == 1 {
        assert_eq!(rig.gateway.status(), Status::NeedsAuthorization);
        step(1200).await;
        assert_eq!(rig.provider.token_count.load(SeqCst), 1);
    }
    rig.stop().await;
}

#[tokio::test(start_paused = true)]
async fn abandoned_completion_is_not_sent_or_consumed() {
    let mut rig = Rig::new(None).await;
    rig.start();
    let pending = state(&rig.gateway.authorize([1; 32]).await.unwrap().url);
    let mut request = Box::pin(rig.gateway.complete([1; 32], pending.clone(), generated()));
    std::future::poll_fn(|cx| {
        assert!(request.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    drop(request);
    pump().await;
    assert_eq!(rig.provider.token_count.load(SeqCst), 0);
    assert_eq!(
        rig.gateway.complete([1; 32], pending, generated()).await,
        Ok(())
    );
    assert_eq!(rig.provider.token_count.load(SeqCst), 1);
    rig.stop().await;
}
