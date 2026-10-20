//! The driver's loop against a cloud in memory and the real hub: answers
//! on time whatever the network does, orders sent once and confirmed by
//! what reads back, refusals that a person must lift.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering::SeqCst};

use http::{Request, StatusCode};
use http_body_util::{BodyExt as _, Full};
use moli_core::{Event, InstanceId, Origin, PointId};
use moli_net::Body;
use moli_runtime::{Hub, HubOptions, spawn_driver};
use serde_json::json;
use tokio::sync::broadcast;

use super::*;

mod p1;

const TEST_PACE: Pace = Pace {
    idle: Duration::from_millis(200),
    follow: Duration::from_millis(30),
    confirm: Duration::from_millis(400),
};
const GATEWAY: &str = "A1B2C3D4E5F6";

fn fixture(name: &str) -> Json {
    let path = format!(
        "{}/../../integrations/ariston/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

/// The Ariston NET cloud in memory.
#[derive(Debug)]
struct Fake {
    data: Mutex<Json>,
    settings: Mutex<Json>,
    posts: Mutex<Vec<(String, Json)>>,
    logins: AtomicU32,
    /// An order changes what reads back (a heater that obeys).
    obeys: bool,
    slow_post: Option<Duration>,
    post_status: Option<u16>,
    login_status: Option<u16>,
    read_status: Mutex<Option<u16>>,
}

impl Default for Fake {
    fn default() -> Self {
        Self {
            data: Mutex::new(fixture("med-plant-data.json")),
            settings: Mutex::new(fixture("plant-settings.json")),
            posts: Mutex::default(),
            logins: AtomicU32::default(),
            obeys: true,
            slow_post: None,
            post_status: None,
            login_status: None,
            read_status: Mutex::default(),
        }
    }
}

impl Fake {
    async fn answer(&self, request: Request<Full<Body>>) -> (u16, Json) {
        let method = request.method().clone();
        let uri = request.uri().clone();
        assert_eq!(request.headers()["user-agent"], "RestSharp/106.11.7.0");
        let token = request.headers().get("ar.authtoken").cloned();
        let body = request.into_body().collect().await.unwrap().to_bytes();
        let body: Json = serde_json::from_slice(&body).unwrap_or_default();
        let path = uri.path().strip_prefix("/api/v2/").expect("under /api/v2/");
        if path == "accounts/login" {
            self.logins.fetch_add(1, SeqCst);
            assert_eq!(body, json!({ "usr": "user@example.com", "pwd": "x" }));
            return match self.login_status {
                Some(status) => (status, json!({ "message": "bad user@example.com" })),
                None => (200, json!({ "token": "SYNTH-T" })),
            };
        }
        assert_eq!(token.unwrap(), "SYNTH-T");
        if method == http::Method::POST {
            self.posts
                .lock()
                .unwrap()
                .push((path.to_owned(), body.clone()));
            if let Some(slow) = self.slow_post {
                tokio::time::sleep(slow).await;
            }
            if let Some(status) = self.post_status {
                return (status, Json::Null);
            }
            if self.obeys {
                self.obey(path, &body);
            }
            return (200, Json::Null);
        }
        if let Some(status) = *self.read_status.lock().unwrap() {
            return (status, Json::Null);
        }
        let base = format!("velis/medPlantData/{GATEWAY}");
        let answer = match path {
            "velis/plants" => fixture("velis-plants.json"),
            "busErrors" => {
                assert_eq!(
                    uri.query(),
                    Some(format!("gatewayId={GATEWAY}&blockingOnly=False&culture=en-US").as_str())
                );
                fixture("bus-errors.json")
            }
            p if p == base => self.data.lock().unwrap().clone(),
            p if p == format!("{base}/plantSettings") => self.settings.lock().unwrap().clone(),
            other => panic!("unexpected read {other}"),
        };
        (200, answer)
    }

    /// What the heater does with an order.
    fn obey(&self, path: &str, body: &Json) {
        let action = path.rsplit('/').next().unwrap();
        let mut data = self.data.lock().unwrap();
        match action {
            "switch" => data["on"] = body.clone(),
            "switchEco" => data["eco"] = body.clone(),
            "mode" => data["mode"] = body["new"].clone(),
            "temperature" => data["reqTemp"] = body["new"].clone(),
            "plantSettings" => {
                let mut settings = self.settings.lock().unwrap();
                for (key, change) in body.as_object().unwrap() {
                    settings[key] = change["new"].clone();
                }
            }
            other => panic!("unexpected order {other}"),
        }
    }

    fn posts(&self) -> Vec<(String, Json)> {
        self.posts.lock().unwrap().clone()
    }
}

#[derive(Debug)]
struct Test(Arc<Fake>);

impl Driver for Test {
    fn kind(&self) -> &'static str {
        "ariston"
    }

    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        let mut cloud = Cloud::new("user@example.com".into(), "x".into(), Arc::default());
        let fake = Arc::clone(&self.0);
        cloud.exchange = Some(Arc::new(move |request| {
            let fake = Arc::clone(&fake);
            Box::pin(async move {
                let (status, body) = fake.answer(request).await;
                let body = if body.is_null() {
                    Body::new()
                } else {
                    Body::from(body.to_string())
                };
                Ok((StatusCode::from_u16(status)?, body))
            })
        }));
        Box::pin(drive(cloud, ctx, TEST_PACE))
    }
}

fn heater() -> DeviceId {
    DeviceId::new(&InstanceId::from("eau-chaude"), GATEWAY)
}

fn start(fake: Fake) -> (Hub, Arc<Fake>, broadcast::Receiver<Event>) {
    let hub = Hub::new(HubOptions::default()).unwrap();
    let events = hub.subscribe();
    let fake = Arc::new(fake);
    spawn_driver(
        &hub,
        InstanceId::from("eau-chaude"),
        Arc::new(Test(Arc::clone(&fake))),
        tokio_util::sync::CancellationToken::new(),
    );
    (hub, fake, events)
}

async fn loaded(fake: Fake) -> (Hub, Arc<Fake>, broadcast::Receiver<Event>) {
    let started = start(fake);
    let hub = started.0.clone();
    until(move || value(&hub, &heater(), "max_setpoint").is_some()).await;
    started
}

async fn until(check: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !check() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("in time");
}

fn value(hub: &Hub, id: &DeviceId, key: &str) -> Option<Value> {
    hub.state(&PointId::new(id, key)).map(|s| s.value)
}

async fn command(hub: &Hub, key: &str, value: Value) -> Result<(), String> {
    hub.command(&PointId::new(&heater(), key), value, Origin::Ui, None)
        .await
        .map_err(|e| e.to_string())
}

fn notices(events: &mut broadcast::Receiver<Event>) -> Vec<String> {
    let mut out = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let Event::Notice { notice } = event {
            out.push(notice.message);
        }
    }
    out
}

#[tokio::test]
async fn the_water_heater_as_the_cloud_sees_it() {
    let (hub, fake, _) = loaded(Fake::default()).await;
    assert_eq!(
        value(&hub, &heater(), "temperature"),
        Some(Value::Float(50.0))
    );
    assert_eq!(value(&hub, &heater(), "setpoint"), Some(Value::Float(55.0)));
    assert_eq!(value(&hub, &heater(), "mode"), Some(Value::from("manuel")));
    assert_eq!(
        value(&hub, &heater(), "antilegionella"),
        Some(Value::Bool(true))
    );
    assert_eq!(value(&hub, &heater(), "errors"), Some(Value::Int(0)));
    assert_eq!(hub.snapshot().devices.len(), 1, "one Velis Evo 2 driven");
    assert_eq!(fake.logins.load(SeqCst), 1);
}

#[tokio::test]
async fn an_order_leaves_once_and_reads_back() {
    let (hub, fake, mut events) = loaded(Fake::default()).await;
    assert_eq!(command(&hub, "setpoint", Value::Float(60.0)).await, Ok(()));
    let posts = fake.posts();
    assert_eq!(
        posts,
        vec![(
            format!("velis/medPlantData/{GATEWAY}/temperature"),
            json!({ "new": 60.0 })
        )]
    );
    until(|| value(&hub, &heater(), "setpoint") == Some(Value::Float(60.0))).await;
    assert_eq!(
        command(&hub, "antilegionella", Value::Bool(false)).await,
        Ok(())
    );
    until(|| value(&hub, &heater(), "antilegionella") == Some(Value::Bool(false))).await;
    assert_eq!(
        fake.posts()[1].1,
        json!({ "MedAntilegionellaOnOff": { "new": 0.0, "old": 1.0 } })
    );
    tokio::time::sleep(TEST_PACE.confirm * 2).await;
    assert!(notices(&mut events).is_empty(), "read back: nothing to say");
}

#[tokio::test]
async fn above_the_heaters_own_ceiling_is_refused_unsent() {
    let fake = Fake::default();
    fake.settings.lock().unwrap()["MedMaxSetpointTemperature"] = json!(65.0);
    let (hub, fake, _) = loaded(fake).await;
    let error = command(&hub, "setpoint", Value::Float(70.0))
        .await
        .unwrap_err();
    assert!(error.contains("65"), "{error}");
    assert!(fake.posts().is_empty());
}

#[tokio::test]
async fn an_order_the_heater_never_shows_is_told() {
    let (hub, fake, mut events) = loaded(Fake {
        obeys: false,
        ..Fake::default()
    })
    .await;
    assert_eq!(
        command(&hub, "mode", Value::from("programme")).await,
        Ok(())
    );
    tokio::time::sleep(TEST_PACE.confirm + Duration::from_millis(300)).await;
    let told = notices(&mut events);
    assert_eq!(told.len(), 1, "{told:?}");
    assert!(told[0].contains("programme"), "{told:?}");
    assert_eq!(fake.posts().len(), 1, "never resent");
}

#[tokio::test]
async fn a_slow_cloud_never_delays_the_answer_and_the_next_never_leaves_late() {
    let (hub, fake, _) = loaded(Fake {
        slow_post: Some(Duration::from_secs(8)),
        ..Fake::default()
    })
    .await;
    let at = Instant::now();
    let first = {
        let hub = hub.clone();
        tokio::spawn(async move { command(&hub, "on", Value::Bool(false)).await })
    };
    until(|| fake.posts().len() == 1).await;
    let second = command(&hub, "eco", Value::Bool(true)).await;
    let first = first.await.unwrap();
    assert!(at.elapsed() < Duration::from_secs(5), "{:?}", at.elapsed());
    assert!(first.unwrap_err().contains("jamais renvoyé"));
    assert!(second.unwrap_err().contains("non envoyé"));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(fake.posts().len(), 1, "the second never left late");
}

#[tokio::test]
async fn an_expired_session_is_never_replayed() {
    let (hub, fake, _) = loaded(Fake {
        post_status: Some(405),
        ..Fake::default()
    })
    .await;
    let error = command(&hub, "eco", Value::Bool(true)).await.unwrap_err();
    assert!(error.contains("session"), "{error}");
    assert_eq!(fake.posts().len(), 1, "not replayed after a new login");
    until(|| fake.logins.load(SeqCst) == 2).await;
}

#[tokio::test]
async fn refused_identifiers_wait_for_a_person() {
    let (hub, fake, _) = start(Fake {
        login_status: Some(401),
        ..Fake::default()
    });
    until(|| {
        hub.snapshot().drivers.iter().any(|d| {
            matches!(&d.status, moli_core::DriverStatus::Waiting { reason } if reason.contains("username"))
        })
    })
    .await;
    assert!(!format!("{:?}", hub.snapshot().drivers).contains("example.com"));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        fake.logins.load(SeqCst),
        1,
        "no more tries before a person acts"
    );
}
