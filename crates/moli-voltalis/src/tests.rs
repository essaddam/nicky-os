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

fn fixture(name: &str) -> Json {
    let path = format!(
        "{}/../../integrations/voltalis/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

/// The Voltalis cloud in memory.
#[derive(Debug)]
struct Fake {
    appliances: Mutex<Json>,
    programs: Mutex<Json>,
    puts: Mutex<Vec<(String, Json)>>,
    logins: AtomicU32,
    /// An order changes what reads back (a heater that obeys).
    obeys: bool,
    slow_put: Option<Duration>,
    put_status: Option<u16>,
    login_status: Option<u16>,
    read_status: Mutex<Option<u16>>,
}

impl Default for Fake {
    fn default() -> Self {
        Self {
            appliances: Mutex::new(fixture("managed-appliance.json")),
            programs: Mutex::new(fixture("programs.json")),
            puts: Mutex::default(),
            logins: AtomicU32::default(),
            obeys: true,
            slow_put: None,
            put_status: None,
            login_status: None,
            read_status: Mutex::default(),
        }
    }
}

impl Fake {
    async fn answer(&self, request: Request<Full<Body>>) -> (u16, Json) {
        let method = request.method().clone();
        let path = request.uri().path().to_owned();
        let body = request.into_body().collect().await.unwrap().to_bytes();
        let body: Json = serde_json::from_slice(&body).unwrap_or_default();
        if path == "/auth/login" {
            self.logins.fetch_add(1, SeqCst);
            assert_eq!(body["login"], "user@example.com");
            return match self.login_status {
                Some(status) => (status, json!({ "message": "bad user@example.com" })),
                None => (200, json!({ "token": "SYNTH-T" })),
            };
        }
        if method == http::Method::PUT {
            self.puts.lock().unwrap().push((path.clone(), body.clone()));
            if let Some(slow) = self.slow_put {
                tokio::time::sleep(slow).await;
            }
            if let Some(status) = self.put_status {
                return (status, Json::Null);
            }
            if self.obeys {
                self.obey(&path, &body);
            }
            return (200, Json::Null);
        }
        if let Some(status) = *self.read_status.lock().unwrap() {
            return (status, Json::Null);
        }
        let answer = match path.rsplit('/').next().unwrap() {
            "me" => fixture("account-me.json"),
            "managed-appliance" => self.appliances.lock().unwrap().clone(),
            "manualsetting" => fixture("manualsetting.json"),
            "program" => self.programs.lock().unwrap().clone(),
            "quicksettings" => fixture("quicksettings.json"),
            "autodiag" => fixture("autodiag.json"),
            other => panic!("unexpected read {other}"),
        };
        assert!(path.starts_with("/api/"), "{path}");
        (200, answer)
    }

    /// What the cloud does with a manual setting or a program.
    fn obey(&self, path: &str, body: &Json) {
        if path.contains("/manualsetting/") {
            let mut list = self.appliances.lock().unwrap();
            let appliance = list
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|a| a["id"] == body["idAppliance"])
                .unwrap();
            let p = &mut appliance["programming"];
            if body["enabled"] == true {
                p["progType"] = "MANUAL".into();
                p["isOn"] = body["isOn"].clone();
                p["mode"] = body["mode"].clone();
                p["temperatureTarget"] = body["temperatureTarget"].clone();
            } else {
                p["progType"] = "USER".into();
                p["isOn"] = true.into();
            }
        } else if path.contains("/programming/program/") {
            let id: u64 = path.rsplit('/').next().unwrap().parse().unwrap();
            let mut list = self.programs.lock().unwrap();
            for p in list.as_array_mut().unwrap() {
                if p["id"] == id {
                    p["enabled"] = body["enabled"].clone();
                }
            }
        }
    }

    fn puts(&self) -> Vec<(String, Json)> {
        self.puts.lock().unwrap().clone()
    }
}

#[derive(Debug)]
struct Test(Arc<Fake>);

impl Driver for Test {
    fn kind(&self) -> &'static str {
        "voltalis"
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

fn device(native: &str) -> DeviceId {
    DeviceId::new(&InstanceId::from("chauffage"), native)
}

fn heater() -> DeviceId {
    device("appliance-200001")
}

fn start(fake: Fake) -> (Hub, Arc<Fake>, broadcast::Receiver<Event>) {
    let hub = Hub::new(HubOptions::default()).unwrap();
    let events = hub.subscribe();
    let fake = Arc::new(fake);
    spawn_driver(
        &hub,
        InstanceId::from("chauffage"),
        Arc::new(Test(Arc::clone(&fake))),
        tokio_util::sync::CancellationToken::new(),
    );
    (hub, fake, events)
}

async fn loaded(fake: Fake) -> (Hub, Arc<Fake>, broadcast::Receiver<Event>) {
    let started = start(fake);
    let hub = started.0.clone();
    until(move || hub.device(&heater()).is_some()).await;
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

async fn command(hub: &Hub, id: &DeviceId, key: &str, value: Value) -> Result<(), String> {
    hub.command(&PointId::new(id, key), value, Origin::Ui, None)
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
async fn the_house_as_the_cloud_sees_it() {
    let (hub, fake, _) = loaded(Fake::default()).await;
    let bureau = device("appliance-200002");
    let water = device("appliance-200003");
    until(|| hub.device(&device("quick-2")).is_some()).await;
    assert_eq!(
        value(&hub, &heater(), "mode"),
        Some(Value::from("programme"))
    );
    assert_eq!(value(&hub, &bureau, "mode"), Some(Value::from("arret")));
    assert_eq!(
        hub.device(&bureau).unwrap().online,
        Some(false),
        "autodiag NOK"
    );
    assert_eq!(value(&hub, &water, "setpoint"), Some(Value::Float(55.0)));
    assert!(
        hub.device(&device("appliance-200004")).is_none(),
        "unknown type"
    );
    assert_eq!(
        value(&hub, &device("program-400001"), "enabled"),
        Some(Value::Bool(true))
    );
    assert_eq!(fake.logins.load(SeqCst), 1);
    // The water heater takes no order: the hub refuses before the driver.
    assert!(
        command(&hub, &water, "setpoint", Value::Float(50.0))
            .await
            .is_err()
    );
    assert!(fake.puts().is_empty());
}

#[tokio::test]
async fn an_order_leaves_once_and_reads_back() {
    let (hub, fake, mut events) = loaded(Fake::default()).await;
    assert_eq!(
        command(&hub, &heater(), "setpoint", Value::Float(20.5)).await,
        Ok(())
    );
    let puts = fake.puts();
    assert_eq!(puts.len(), 1);
    assert_eq!(puts[0].0, "/api/site/100001/manualsetting/300001");
    assert_eq!(puts[0].1["temperatureTarget"], 20.5);
    until(|| value(&hub, &heater(), "setpoint") == Some(Value::Float(20.5))).await;
    assert_eq!(value(&hub, &heater(), "mode"), Some(Value::from("manuel")));
    tokio::time::sleep(TEST_PACE.confirm * 2).await;
    assert!(notices(&mut events).is_empty(), "read back: nothing to say");
    // Back to its planning.
    assert_eq!(
        command(&hub, &heater(), "mode", Value::from("programme")).await,
        Ok(())
    );
    until(|| value(&hub, &heater(), "mode") == Some(Value::from("programme"))).await;
    assert_eq!(fake.puts()[1].1["enabled"], false);
}

#[tokio::test]
async fn an_order_the_heater_never_shows_is_told() {
    let (hub, fake, mut events) = loaded(Fake {
        obeys: false,
        ..Fake::default()
    })
    .await;
    assert_eq!(
        command(&hub, &heater(), "preset", Value::from("confort")).await,
        Ok(())
    );
    tokio::time::sleep(TEST_PACE.confirm + Duration::from_millis(300)).await;
    let told = notices(&mut events);
    assert_eq!(told.len(), 1, "{told:?}");
    assert!(
        told[0].contains("SEJOUR") && told[0].contains("confort"),
        "{told:?}"
    );
    assert_eq!(fake.puts().len(), 1, "never resent");
}

#[tokio::test]
async fn a_slow_cloud_never_delays_the_answer_and_the_next_never_leaves_late() {
    let (hub, fake, _) = loaded(Fake {
        slow_put: Some(Duration::from_secs(8)),
        ..Fake::default()
    })
    .await;
    let at = Instant::now();
    let first = {
        let hub = hub.clone();
        tokio::spawn(async move { command(&hub, &heater(), "setpoint", Value::Float(20.0)).await })
    };
    until(|| fake.puts().len() == 1).await;
    let second = command(&hub, &heater(), "preset", Value::from("eco")).await;
    let first = first.await.unwrap();
    assert!(at.elapsed() < Duration::from_secs(5), "{:?}", at.elapsed());
    assert!(first.unwrap_err().contains("jamais renvoyé"));
    assert!(second.unwrap_err().contains("non envoyé"));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(fake.puts().len(), 1, "the second never left late");
}

#[tokio::test]
async fn a_refused_session_is_never_replayed() {
    let (hub, fake, _) = loaded(Fake {
        put_status: Some(401),
        ..Fake::default()
    })
    .await;
    let error = command(&hub, &heater(), "setpoint", Value::Float(20.0))
        .await
        .unwrap_err();
    assert!(error.contains("session"), "{error}");
    assert_eq!(fake.puts().len(), 1, "not replayed after a new login");
    // The next read logs in again.
    until(|| fake.logins.load(SeqCst) == 2).await;
}

#[tokio::test]
async fn refused_identifiers_wait_for_a_person() {
    let (hub, fake, _) = start(Fake {
        login_status: Some(401),
        ..Fake::default()
    });
    until(|| {
        hub.snapshot()
            .drivers
            .iter()
            .any(|d| matches!(&d.status, moli_core::DriverStatus::Waiting { reason } if reason.contains("username")))
    })
    .await;
    let reason = format!("{:?}", hub.snapshot().drivers);
    assert!(!reason.contains("example.com"), "the cloud is never quoted");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        fake.logins.load(SeqCst),
        1,
        "no more tries before a person acts"
    );
}

#[tokio::test]
async fn a_busy_cloud_is_left_alone() {
    let (hub, fake, _) = loaded(Fake::default()).await;
    *fake.read_status.lock().unwrap() = Some(429);
    // The next read meets the quota: orders are refused at once.
    let refused = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            tokio::time::sleep(Duration::from_millis(50)).await;
            if let Err(e) = command(&hub, &heater(), "setpoint", Value::Float(20.0)).await {
                break e;
            }
        }
    })
    .await
    .expect("paused in time");
    assert!(refused.contains("saturé"), "{refused}");
}

#[tokio::test]
async fn programs_switch() {
    let (hub, fake, _) = loaded(Fake::default()).await;
    let holidays = device("program-400002");
    until(|| hub.device(&holidays).is_some()).await;
    assert_eq!(
        command(&hub, &holidays, "enabled", Value::Bool(true)).await,
        Ok(())
    );
    let puts = fake.puts();
    assert_eq!(puts[0].0, "/api/site/100001/programming/program/400002");
    assert_eq!(
        puts[0].1,
        json!({ "name": "Vacances scolaires", "enabled": true })
    );
    until(|| value(&hub, &holidays, "enabled") == Some(Value::Bool(true))).await;
}
