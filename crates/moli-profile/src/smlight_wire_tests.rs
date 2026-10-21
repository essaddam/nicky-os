//! Loopback-only public profile contracts. No real coordinator.
use super::*;
use moli_core::{DriverStatus, InstanceId, Origin, PointId};
use moli_runtime::{CommandError, Hub, HubOptions, spawn_driver};
use std::sync::atomic::{AtomicU8, Ordering};
use tokio_util::sync::CancellationToken;
const INFO: &[u8] = include_bytes!("../../../integrations/smlight/fixtures/info.json");
const SENSORS: &[u8] = include_bytes!("../../../integrations/smlight/fixtures/sensors.json");
struct Call {
    at: Instant,
    path: String,
    method: http::Method,
    authenticated: bool,
}
struct Server {
    port: u16,
    mode: Arc<AtomicU8>,
    calls: Arc<Mutex<Vec<Call>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    async fn new(mode: u8) -> Self {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let mode = Arc::new(AtomicU8::new(mode));
        let calls = Arc::new(Mutex::new(vec![]));
        let (flag, seen) = (mode.clone(), calls.clone());
        let task = tokio::spawn(async move {
            if flag.load(Ordering::SeqCst) == 16 {
                tokio::time::sleep(Duration::from_millis(1800)).await;
            }
            loop {
                let Ok((tcp, _)) = listener.accept().await else {
                    break;
                };
                let (flag, seen) = (flag.clone(), seen.clone());
                tokio::spawn(async move {
                    let service = hyper::service::service_fn(
                        move |req: http::Request<hyper::body::Incoming>| {
                            let mode = flag.load(Ordering::SeqCst);
                            seen.lock().unwrap().push(Call {
                                at: Instant::now(),
                                path: req.uri().to_string(),
                                method: req.method().clone(),
                                authenticated: req.headers().contains_key("authorization")
                                    || req.headers().contains_key("cookie"),
                            });
                            async move {
                                let mut status = http::StatusCode::OK;
                                let mut body = match req.uri().path() {
                                    "/ha_info" => INFO.to_vec(),
                                    "/ha_sensors" => SENSORS.to_vec(),
                                    _ => {
                                        status = http::StatusCode::NOT_FOUND;
                                        vec![]
                                    }
                                };
                                if req.uri().path() == "/ha_info" {
                                    body = match mode {
                                    1 => include_bytes!("../../../integrations/smlight/fixtures/partial-info.json").to_vec(),
                                    2 => include_bytes!("../../../integrations/smlight/fixtures/missing-identity.json").to_vec(),
                                    3 => br#"{"Info":{"MAC":"00:00:00:00:00:00"}}"#.to_vec(),
                                    4 => br#"{"Info":{"MAC":"aa:bb:cc:dd:ee:11"}}"#.to_vec(),
                                    5 => { status=http::StatusCode::UNAUTHORIZED; vec![] },
                                    6 => { status=http::StatusCode::FORBIDDEN; vec![] },
                                    7 => include_bytes!("../../../integrations/smlight/fixtures/login.html").to_vec(),
                                    8 => { status=http::StatusCode::FOUND; vec![] },
                                    12 => vec![b' '; MAX_BODY+1],
                                    13 => { tokio::time::sleep(Duration::from_secs(8)).await; INFO.to_vec() },
                                    14 => br#"{"Info":{"MAC":123456789012}}"#.to_vec(),
                                    _ => body,
                                };
                                } else if req.uri().path() == "/ha_sensors" {
                                    body = match mode {
                                    1 => include_bytes!("../../../integrations/smlight/fixtures/partial-sensors.json").to_vec(),
                                    10 => {status=http::StatusCode::UNAUTHORIZED; vec![]},
                                    11 => include_bytes!("../../../integrations/smlight/fixtures/login.html").to_vec(),
                                    _ => body,
                                };
                                }
                                let mut response = http::Response::builder().status(status);
                                if status == http::StatusCode::FOUND {
                                    response = response
                                        .header("location", "http://elsewhere.example.invalid/api");
                                }
                                Ok::<_, std::convert::Infallible>(
                                    response.body(Full::new(Bytes::from(body))).unwrap(),
                                )
                            }
                        },
                    );
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(tcp), service)
                        .await;
                });
            }
        });
        Self {
            port,
            mode,
            calls,
            task,
        }
    }
    fn config(&self) -> Config {
        Config {
            profile: "smlight".into(),
            host: Some("127.0.0.1".into()),
            port: Some(self.port),
            vars: std::collections::BTreeMap::default(),
        }
    }
}
async fn until(mut predicate: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(8), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}
fn forbidden_table() -> BoxFuture<'static, Vec<(Ipv4Addr, String)>> {
    panic!("fixed target must not consult neighbours");
}
fn forbidden_ask(_: String, _: Ipv4Addr, _: Vec<Ipv4Addr>) -> BoxFuture<'static, Vec<Ipv4Addr>> {
    panic!("fixed target must not probe addresses");
}
fn launch(
    server: &Server,
    hub: &Hub,
    fast: bool,
) -> (CancellationToken, tokio::task::JoinHandle<()>) {
    let mut driver = ProfileDriver::new(server.config(), PathBuf::new()).unwrap();
    if fast {
        let mut p = (*driver.profile).clone();
        for r in &mut p.requests {
            r.every = Duration::from_millis(10);
        }
        driver.profile = Arc::new(p);
    }
    driver.lan = Lan {
        home: |_| true,
        table: forbidden_table,
        ask: forbidden_ask,
    };
    let cancel = CancellationToken::new();
    let task = spawn_driver(
        hub,
        InstanceId::from("monitor"),
        Arc::new(driver),
        cancel.clone(),
    );
    (cancel, task)
}
async fn stop(cancel: CancellationToken, task: tokio::task::JoinHandle<()>) {
    cancel.cancel();
    tokio::time::timeout(Duration::from_millis(500), task)
        .await
        .unwrap()
        .unwrap();
}
fn running(hub: &Hub) -> bool {
    hub.snapshot()
        .drivers
        .iter()
        .any(|d| matches!(d.status, DriverStatus::Running))
}
fn failed(hub: &Hub) -> bool {
    hub.snapshot()
        .drivers
        .iter()
        .any(|d| matches!(d.status, DriverStatus::Backoff { .. }))
}
fn point(key: &str) -> PointId {
    PointId::from(format!("monitor:aabbccddee10/{key}"))
}
fn assert_partial(hub: &Hub) {
    let s = hub.snapshot();
    assert_eq!(s.devices.len(), 1);
    assert_eq!(s.devices[0].device.points.len(), 7);
    for key in ["firmware", "zb_temp", "ram_usage", "ethernet"] {
        assert_eq!(hub.state(&point(key)).unwrap().value, Value::Null, "{key}");
    }
    assert_eq!(
        hub.state(&point("esp32_temp")).unwrap().value,
        Value::Float(46.5)
    );
}
#[tokio::test]
async fn initial_and_periodic_reads_are_paced_readonly_and_missing_values_become_null() {
    let server = Server::new(0).await;
    let hub = Hub::new(HubOptions::default()).unwrap();
    let (c, t) = launch(&server, &hub, true);
    until(|| running(&hub)).await;
    let count = server.calls.lock().unwrap().len();
    assert_eq!(
        hub.command(&point("uptime"), Value::Float(0.0), Origin::Mcp, None)
            .await,
        Err(CommandError::ReadOnly)
    );
    assert_eq!(server.calls.lock().unwrap().len(), count);
    server.mode.store(1, Ordering::SeqCst);
    until(|| {
        hub.state(&point("esp32_temp"))
            .is_some_and(|v| v.value == Value::Float(46.5))
    })
    .await;
    assert_partial(&hub);
    {
        let calls = server.calls.lock().unwrap();
        assert!(calls.len() >= 4);
        for pair in calls.windows(2) {
            assert!(pair[1].at.duration_since(pair[0].at) >= Duration::from_millis(990));
        }
        for call in calls.iter() {
            assert_eq!(call.method, http::Method::GET);
            assert!(["/ha_info", "/ha_sensors"].contains(&call.path.as_str()));
            assert!(!call.authenticated);
        }
    }
    stop(c, t).await;
    assert_eq!(hub.state(&point("uptime")).unwrap().value, Value::Null);
}
#[tokio::test]
async fn cold_cache_and_restarts_keep_all_seven_bindings_without_fresh_stale_values() {
    let server = Server::new(1).await;
    let path =
        std::env::temp_dir().join(format!("moli-smlight-cache-{}.json", moli_core::now_ms()));
    let data: serde_json::Map<String, serde_json::Value> = [
        "firmware",
        "esp32_temp",
        "zb_temp",
        "uptime",
        "ram_usage",
        "ethernet",
        "wifi_connected",
    ]
    .into_iter()
    .map(|k| (k.to_owned(), serde_json::json!({"value":999,"ts":1})))
    .collect();
    std::fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({"monitor:aabbccddee10":data})).unwrap(),
    )
    .unwrap();
    let hub = Hub::new(HubOptions {
        state_path: Some(path.clone()),
        ..HubOptions::default()
    })
    .unwrap();
    let (c, t) = launch(&server, &hub, false);
    until(|| running(&hub)).await;
    assert_partial(&hub);
    stop(c, t).await;
    server.mode.store(0, Ordering::SeqCst);
    let (c, t) = launch(&server, &hub, false);
    until(|| running(&hub)).await;
    assert_eq!(hub.snapshot().devices.len(), 1);
    assert_eq!(hub.snapshot().devices[0].device.points.len(), 7);
    assert_eq!(
        hub.state(&point("ram_usage")).unwrap().value,
        Value::Float(96.0)
    );
    stop(c, t).await;
    for pair in server.calls.lock().unwrap().windows(2) {
        assert!(
            pair[1].at.duration_since(pair[0].at) >= Duration::from_millis(990),
            "restart must not burst requests"
        );
    }
    std::fs::remove_file(path).unwrap();
}
#[tokio::test]
async fn protected_html_redirect_and_invalid_identity_variants_never_publish() {
    for mode in [2, 3, 5, 6, 7, 8, 10, 11, 12, 14] {
        let server = Server::new(mode).await;
        let hub = Hub::new(HubOptions::default()).unwrap();
        let (c, t) = launch(&server, &hub, false);
        until(|| failed(&hub) || running(&hub)).await;
        assert!(
            hub.snapshot().devices.is_empty(),
            "unsupported variant {mode} must not be published"
        );
        stop(c, t).await;
    }
}
#[tokio::test]
async fn identity_substitution_is_rejected_before_any_new_value_is_published() {
    let server = Server::new(0).await;
    let hub = Hub::new(HubOptions::default()).unwrap();
    let (c, t) = launch(&server, &hub, true);
    until(|| running(&hub)).await;
    server.mode.store(4, Ordering::SeqCst);
    until(|| failed(&hub)).await;
    assert_eq!(hub.snapshot().devices.len(), 1);
    assert_eq!(hub.state(&point("esp32_temp")).unwrap().value, Value::Null);
    stop(c, t).await;
    let (c, t) = launch(&server, &hub, false);
    until(|| failed(&hub)).await;
    assert_eq!(hub.snapshot().devices.len(), 1);
    stop(c, t).await;
}
#[tokio::test]
async fn spacing_and_a_slow_request_are_cancelled_promptly_and_timeout_stays_five_seconds() {
    for mode in [0, 13] {
        let server = Server::new(mode).await;
        let hub = Hub::new(HubOptions::default()).unwrap();
        let (c, t) = launch(&server, &hub, false);
        until(|| !server.calls.lock().unwrap().is_empty()).await;
        stop(c, t).await;
    }
    let server = Server::new(13).await;
    let hub = Hub::new(HubOptions::default()).unwrap();
    let start = Instant::now();
    let (c, t) = launch(&server, &hub, false);
    until(|| failed(&hub)).await;
    assert!(start.elapsed() >= Duration::from_secs(5));
    assert!(hub.snapshot().devices.is_empty());
    stop(c, t).await;
}

#[tokio::test]
async fn a_cached_other_identity_blocks_first_publication_without_deleting_the_cache() {
    let server = Server::new(0).await;
    let path = std::env::temp_dir().join(format!(
        "moli-smlight-other-cache-{}.json",
        moli_core::now_ms()
    ));
    std::fs::write(
        &path,
        br#"{"monitor:aabbccddee11":{"uptime":{"value":999,"ts":1}}}"#,
    )
    .unwrap();
    let hub = Hub::new(HubOptions {
        state_path: Some(path.clone()),
        ..HubOptions::default()
    })
    .unwrap();
    let before = std::fs::read(&path).unwrap();
    let (c, t) = launch(&server, &hub, false);
    until(|| failed(&hub)).await;
    assert!(hub.snapshot().devices.is_empty());
    stop(c, t).await;
    assert_eq!(std::fs::read(&path).unwrap(), before);
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn a_delayed_connection_does_not_compress_the_spacing_seen_by_the_device() {
    let server = Server::new(16).await;
    let hub = Hub::new(HubOptions::default()).unwrap();
    let (c, t) = launch(&server, &hub, false);
    until(|| running(&hub)).await;
    let times = server
        .calls
        .lock()
        .unwrap()
        .iter()
        .map(|c| c.at)
        .collect::<Vec<_>>();
    stop(c, t).await;
    assert_eq!(times.len(), 2);
    assert!(
        times[1].duration_since(times[0]) >= Duration::from_millis(990),
        "slow connect compressed the device's request interval"
    );
}
