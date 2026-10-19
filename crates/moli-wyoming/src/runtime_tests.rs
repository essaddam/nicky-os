//! Real hub and supervisor against a synthetic loopback Wyoming service.
use super::*;
use moli_core::{DeviceId, InstanceId, Origin, PointId, Semantic, Value};
use moli_runtime::{CommandError, Hub, HubOptions, spawn_driver};
use serde_json::Value as Json;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio::task::{JoinHandle, JoinSet};
use tokio_util::sync::CancellationToken;

struct Server {
    port: u16,
    mode: Arc<AtomicU8>,
    requests: Arc<AtomicUsize>,
    closed: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

impl Server {
    async fn start(mode: u8) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let mode = Arc::new(AtomicU8::new(mode));
        let requests = Arc::new(AtomicUsize::new(0));
        let closed = Arc::new(AtomicUsize::new(0));
        let (m, r, c) = (mode.clone(), requests.clone(), closed.clone());
        let task = tokio::spawn(async move {
            let mut tasks = JoinSet::new();
            loop {
                tokio::select! {
                    socket = listener.accept() => {
                        let (socket, _) = socket.unwrap();
                        let (mode, requests, closed) = (m.clone(), r.clone(), c.clone());
                        tasks.spawn(async move {
                            let (read, mut write) = socket.into_split();
                            let mut reader = BufReader::new(read);
                            let mut line = String::new();
                            reader.read_line(&mut line).await.unwrap();
                            let header: Json = serde_json::from_str(&line).unwrap();
                            assert_eq!(header["type"], "describe");
                            assert_eq!(header.get("payload_length").and_then(Json::as_u64).unwrap_or(0), 0);
                            let len = usize::try_from(header["data_length"].as_u64().unwrap()).unwrap();
                            assert!(len <= 512);
                            let mut data = vec![0; len];
                            reader.read_exact(&mut data).await.unwrap();
                            assert_eq!(serde_json::from_slice::<Json>(&data).unwrap(), serde_json::json!({}));
                            requests.fetch_add(1, Ordering::SeqCst);
                            let response: &[u8] = match mode.load(Ordering::SeqCst) {
                                0 => include_bytes!("../../../integrations/wyoming/fixtures/piper-info.wyoming"),
                                1 => b"{\"type\":\"info\",\"data\":{\"asr\":null}}\n",
                                2 => b"{\"type\":\"error\",\"data\":{\"text\":\"PRIVATE_TOKEN user@example.com\"}}\n",
                                _ => b"", // Stalls until the caller times out or cancels.
                            };
                            write.write_all(response).await.unwrap();
                            let mut extra = [0];
                            match reader.read(&mut extra).await {
                                Ok(0) => {}
                                Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
                                other => panic!("only describe followed by closure is allowed: {other:?}"),
                            }
                            closed.fetch_add(1, Ordering::SeqCst);
                        });
                    }
                    Some(result) = tasks.join_next() => result.unwrap(),
                }
            }
        });
        Self {
            port,
            mode,
            requests,
            closed,
            task,
        }
    }

    fn count(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn point(key: &str) -> PointId {
    PointId::from(format!("voice:service/{key}"))
}

fn device() -> DeviceId {
    DeviceId::from("voice:service")
}

fn driver(server: &Server) -> Wyoming {
    Wyoming::new(Config {
        host: "127.0.0.1".into(),
        port: server.port,
        every: 10,
        timeout: 1,
    })
    .unwrap()
}

fn start(server: &Server) -> (Hub, CancellationToken, JoinHandle<()>) {
    let mut options = HubOptions::default();
    options.guard.protect_unassigned = true;
    let hub = Hub::new(options).unwrap();
    let cancel = CancellationToken::new();
    let task = spawn_driver(
        &hub,
        InstanceId::from("voice"),
        Arc::new(driver(server)),
        cancel.clone(),
    );
    (hub, cancel, task)
}

async fn until(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("expected observable state from synthetic service");
}

async fn stop(cancel: CancellationToken, task: JoinHandle<()>) {
    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
}

async fn advance(seconds: u64) {
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(seconds)).await;
    tokio::time::resume();
}

fn state(hub: &Hub, key: &str) -> Option<Value> {
    hub.state(&point(key)).map(|sample| sample.value)
}

#[tokio::test]
async fn one_stable_service_starts_offline_with_six_read_only_points() {
    let server = Server::start(3).await;
    let (hub, cancel, task) = start(&server);
    until(|| hub.device(&device()).is_some()).await;
    let view = hub.device(&device()).unwrap();
    assert_eq!(hub.snapshot().devices.len(), 1);
    assert_eq!(view.online, Some(false));
    assert_eq!(state(&hub, "available"), Some(Value::Bool(false)));
    let keys: Vec<_> = view.device.points.iter().map(|p| p.key.as_ref()).collect();
    assert_eq!(
        keys,
        [
            "available",
            "last_error",
            "models",
            "program",
            "version",
            "voices"
        ]
    );
    assert!(
        view.device
            .points
            .iter()
            .all(|p| p.access.read && !p.access.write)
    );
    assert_eq!(
        view.device.point("available").unwrap().semantic,
        Semantic::OnOff
    );
    stop(cancel, task).await;
}

#[tokio::test]
async fn valid_probe_failure_and_recovery_follow_the_period_without_bursts() {
    let server = Server::start(0).await;
    let (hub, cancel, task) = start(&server);
    until(|| state(&hub, "available") == Some(Value::Bool(true))).await;
    assert_eq!(hub.device(&device()).unwrap().online, Some(true));
    assert_eq!(state(&hub, "program"), Some(Value::from("piper")));
    assert_eq!(state(&hub, "voices"), Some(Value::from("example-fr-voice")));
    until(|| server.closed.load(Ordering::SeqCst) == 1).await;
    server.mode.store(1, Ordering::SeqCst);
    advance(9).await;
    assert_eq!(server.count(), 1);
    advance(1).await;
    until(|| server.count() == 2 && state(&hub, "available") == Some(Value::Bool(false))).await;
    assert_eq!(hub.device(&device()).unwrap().online, Some(false));
    for key in ["models", "program", "version", "voices"] {
        assert_eq!(state(&hub, key), Some(Value::from("")));
    }
    assert!(matches!(state(&hub, "last_error"), Some(Value::Text(text)) if !text.is_empty()));
    server.mode.store(0, Ordering::SeqCst);
    advance(60).await;
    until(|| server.count() >= 3 && state(&hub, "available") == Some(Value::Bool(true))).await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        server.count(),
        3,
        "missed periods must never become a burst"
    );
    assert_eq!(state(&hub, "last_error"), Some(Value::from("")));
    assert_eq!(hub.snapshot().devices.len(), 1);
    stop(cancel, task).await;
    assert_eq!(hub.device(&device()).unwrap().online, Some(false));
    assert_eq!(state(&hub, "available"), Some(Value::Bool(false)));
}

#[tokio::test]
async fn every_write_is_refused_before_guard_and_never_reaches_the_service() {
    let server = Server::start(0).await;
    let (hub, cancel, task) = start(&server);
    until(|| state(&hub, "available") == Some(Value::Bool(true))).await;
    for key in [
        "available",
        "last_error",
        "models",
        "program",
        "version",
        "voices",
    ] {
        assert_eq!(
            hub.command(&point(key), Value::Bool(true), Origin::Mcp, None)
                .await,
            Err(CommandError::ReadOnly)
        );
    }
    assert!(hub.approvals().is_empty());
    assert_eq!(server.count(), 1);
    stop(cancel, task).await;
}

#[tokio::test]
async fn remote_errors_are_expurgated_and_a_timed_out_probe_waits_a_full_period() {
    let server = Server::start(2).await;
    let (hub, cancel, task) = start(&server);
    until(|| matches!(state(&hub,"last_error"), Some(Value::Text(text)) if !text.is_empty())).await;
    let error = state(&hub, "last_error").unwrap().to_json().to_string();
    assert!(!error.contains("PRIVATE_TOKEN") && !error.contains("user@example.com"));
    assert_eq!(hub.device(&device()).unwrap().online, Some(false));
    server.mode.store(3, Ordering::SeqCst);
    advance(10).await;
    until(|| server.count() == 2).await;
    until(|| server.closed.load(Ordering::SeqCst) == 2).await;
    assert_eq!(state(&hub, "available"), Some(Value::Bool(false)));
    advance(9).await;
    assert_eq!(server.count(), 2);
    advance(1).await;
    until(|| server.count() == 3).await;
    stop(cancel, task).await;
    until(|| server.closed.load(Ordering::SeqCst) == 3).await;
}

struct DropProbe {
    driver: Wyoming,
    abort: Arc<Notify>,
    ended: Arc<Notify>,
}

impl Driver for DropProbe {
    fn kind(&self) -> &'static str {
        "wyoming"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            tokio::select! {
                () = self.abort.notified() => {}
                result = self.driver.run(ctx) => { result?; }
            }
            self.ended.notify_one();
            std::future::pending().await
        })
    }
}

#[tokio::test]
async fn dropping_the_driver_future_closes_an_inflight_probe_and_marks_it_offline() {
    let server = Server::start(3).await;
    let hub = Hub::new(HubOptions::default()).unwrap();
    let cancel = CancellationToken::new();
    let abort = Arc::new(Notify::new());
    let ended = Arc::new(Notify::new());
    let task = spawn_driver(
        &hub,
        InstanceId::from("voice"),
        Arc::new(DropProbe {
            driver: driver(&server),
            abort: abort.clone(),
            ended: ended.clone(),
        }),
        cancel.clone(),
    );
    until(|| server.count() == 1).await;
    abort.notify_one();
    tokio::time::timeout(Duration::from_secs(2), ended.notified())
        .await
        .unwrap();
    until(|| server.closed.load(Ordering::SeqCst) == 1).await;
    assert_eq!(hub.device(&device()).unwrap().online, Some(false));
    assert_eq!(state(&hub, "available"), Some(Value::Bool(false)));
    stop(cancel, task).await;
}
