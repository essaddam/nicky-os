//! Synthetic API v1 server. No device, stored token or authentication capture.
use super::*;
use ring::rand::SystemRandom;
use serde_json::{Value as Json, json};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;
use tokio::task::JoinHandle;

struct Wire {
    token: String,
    response: String,
    requests: Vec<(String, String, Json)>,
    lifetime: u64,
    firmware: String,
    mac: String,
    mode: String,
    fail_path: Option<String>,
    failure: Vec<u8>,
    status: u16,
    hold_path: Option<String>,
    hold_method: Option<String>,
    lose_write: bool,
}

pub(crate) struct Server {
    pub port: u16,
    wire: Arc<Mutex<Wire>>,
    pub received: Arc<Notify>,
    pub release: Arc<Notify>,
    task: JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn secret() -> String {
    let mut bytes = [0; 20];
    SystemRandom::new().fill(&mut bytes).unwrap();
    {
        use std::fmt::Write as _;
        bytes.iter().fold(String::with_capacity(40), |mut text, b| {
            write!(text, "{b:02x}").unwrap();
            text
        })
    }
}

impl Server {
    pub async fn start() -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let wire = Arc::new(Mutex::new(Wire {
            token: secret()[..24].to_owned(),
            response: secret(),
            requests: Vec::new(),
            lifetime: 60,
            firmware: "3.0.7".into(),
            mac: "aa:bb:cc:dd:ee:ff".into(),
            mode: "movie".into(),
            fail_path: None,
            failure: Vec::new(),
            status: 200,
            hold_path: None,
            hold_method: None,
            lose_write: false,
        }));
        let received = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let (w, r, g) = (
            Arc::clone(&wire),
            Arc::clone(&received),
            Arc::clone(&release),
        );
        let task = tokio::spawn(async move {
            let mut jobs = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    socket=listener.accept()=>{
                        let Ok((socket,_))=socket else {break};
                        let (w,r,g)=(Arc::clone(&w),Arc::clone(&r),Arc::clone(&g));
                        jobs.spawn(async move {serve(socket,&w,&r,&g).await;});
                    },
                    Some(done)=jobs.join_next()=>done.unwrap(),
                }
            }
        });
        Self {
            port,
            wire,
            received,
            release,
            task,
        }
    }
    pub fn client(&self) -> Client {
        Client::new(Ipv4Addr::LOCALHOST, self.port, "aabbccddeeff".into())
    }
    pub(crate) fn count(&self, method: &str, path: &str) -> usize {
        self.wire
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|(m, p, _)| m == method && p == path)
            .count()
    }
    pub(crate) fn fail(&self, path: &str, status: u16, body: Vec<u8>) {
        let mut w = self.wire.lock().unwrap();
        w.fail_path = Some(path.into());
        w.status = status;
        w.failure = body;
    }
    pub(crate) fn hold(&self, path: &str) {
        let mut w = self.wire.lock().unwrap();
        w.hold_path = Some(path.into());
        w.hold_method = None;
    }
    pub(crate) fn hold_post(&self, path: &str) {
        let mut w = self.wire.lock().unwrap();
        w.hold_path = Some(path.into());
        w.hold_method = Some("POST".into());
    }
    pub(crate) fn unhold(&self) {
        let mut w = self.wire.lock().unwrap();
        w.hold_path = None;
        w.hold_method = None;
    }
    pub(crate) fn set_mode(&self, mode: &str) {
        self.wire.lock().unwrap().mode = mode.into();
    }
    pub fn writes(&self) -> Vec<Json> {
        self.wire
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|(m, p, _)| m == "POST" && p.starts_with("/xled/v1/led/"))
            .map(|(_, _, j)| j.clone())
            .collect()
    }
}

async fn serve(mut socket: TcpStream, wire: &Mutex<Wire>, received: &Notify, release: &Notify) {
    let mut bytes = Vec::new();
    let mut buf = [0; 1024];
    let (headers, body) = loop {
        let Ok(n) = socket.read(&mut buf).await else {
            return;
        };
        if n == 0 || bytes.len() + n > 16 * 1024 {
            return;
        }
        bytes.extend_from_slice(&buf[..n]);
        if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
            let len = headers
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|n| n.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            if bytes.len() >= end + 4 + len {
                break (headers, bytes[end + 4..end + 4 + len].to_vec());
            }
        }
    };
    let mut line = headers.lines().next().unwrap().split_whitespace();
    let method = line.next().unwrap();
    let path = line.next().unwrap();
    let json: Json = if body.is_empty() {
        Json::Null
    } else {
        serde_json::from_slice(&body).unwrap()
    };
    let (status, body, hold, lose) = {
        let mut w = wire.lock().unwrap();
        w.requests.push((method.into(), path.into(), json.clone()));
        if path != "/xled/v1/gestalt" && path != "/xled/v1/login" {
            assert!(
                headers
                    .to_ascii_lowercase()
                    .contains(&format!("x-auth-token: {}", w.token))
            );
        }
        let reply = payload(&w, &headers, method, path, &json);
        let failed = w.fail_path.as_deref() == Some(path);
        (
            if failed { w.status } else { 200 },
            if failed {
                w.failure.clone()
            } else {
                serde_json::to_vec(&reply).unwrap()
            },
            w.hold_path.as_deref() == Some(path)
                && w.hold_method.as_deref().is_none_or(|m| m == method),
            w.lose_write && method == "POST" && path.contains("/led/"),
        )
    };
    if hold {
        received.notify_one();
        release.notified().await;
    }
    if lose {
        return;
    }
    let header = format!(
        "HTTP/1.1 {status} Test\r\nLocation: /xled/v1/login\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = socket.write_all(header.as_bytes()).await;
    let _ = socket.write_all(&body).await;
}

fn payload(w: &Wire, headers: &str, method: &str, path: &str, json: &Json) -> Json {
    match path {
        "/xled/v1/gestalt" => {
            json!({"code":1000,"mac":w.mac,"device_name":"String lights","product_code":"RGB-Demo","led_profile":"RGB"})
        }
        "/xled/v1/fw/version" => json!({"code":1000,"version":w.firmware}),
        "/xled/v1/login" => {
            let challenge = json["challenge"].as_str().unwrap();
            assert_eq!(challenge.len(), 44); // base64 of 32 random bytes
            assert!(challenge.ends_with('='));
            json!({"code":1000,"authentication_token":w.token,"challenge-response":w.response,"authentication_token_expires_in":w.lifetime})
        }
        "/xled/v1/verify" => {
            assert!(
                headers
                    .to_ascii_lowercase()
                    .contains(&format!("x-auth-token: {}", w.token))
            );
            // Assert the boolean: equality macros would print session material.
            assert!(
                json["challenge-response"]
                    .as_str()
                    .eq(&Some(w.response.as_str()))
            );
            json!({"code":1000})
        }
        "/xled/v1/led/mode" if method == "GET" => json!({"code":1000,"mode":w.mode}),
        "/xled/v1/led/out/brightness" if method == "GET" => {
            json!({"code":1000,"mode":"enabled","value":23})
        }
        "/xled/v1/led/color" if method == "GET" => {
            json!({"code":1000,"red":18,"green":52,"blue":171})
        }
        "/xled/v1/led/mode" | "/xled/v1/led/out/brightness" | "/xled/v1/led/color" => {
            json!({"code":1000})
        }
        _ => panic!("unexpected route"),
    }
}

#[tokio::test]
async fn twinkly_http_session_is_verified_once_and_state_is_observed() {
    let s = Server::start().await;
    let mut c = s.client();
    for _ in 0..2 {
        let state = c.read(|| true).await.expect("real synthetic HTTP state");
        assert_eq!(state.mode, Mode::Movie);
        assert_eq!(state.brightness, 23);
        assert_eq!(state.color, Some([18, 52, 171]));
        assert!(state.caps.color);
    }
    assert_eq!(s.count("POST", "/xled/v1/login"), 1);
    assert_eq!(s.count("POST", "/xled/v1/verify"), 1);
}

#[tokio::test]
async fn twinkly_http_changed_identity_prevents_authentication() {
    let s = Server::start().await;
    s.wire.lock().unwrap().mac = "02:00:00:00:00:01".into();
    assert!(matches!(
        s.client().read(|| true).await,
        Err(Failure::Identity)
    ));
    assert_eq!(s.count("POST", "/xled/v1/login"), 0);
}

#[tokio::test]
async fn twinkly_http_malformed_login_never_verifies_or_echoes_secrets() {
    let s = Server::start().await;
    let sensitive = secret();
    s.fail("/xled/v1/login",200,serde_json::to_vec(&json!({"code":1000,"authentication_token":sensitive,"challenge-response":sensitive,"authentication_token_expires_in":0})).unwrap());
    let error = s.client().read(|| true).await.err().unwrap();
    assert!(matches!(error, Failure::Protocol(_) | Failure::Session));
    assert!(!format!("{error:?}").contains(&sensitive));
    assert_eq!(s.count("POST", "/xled/v1/verify"), 0);
}

#[tokio::test]
async fn twinkly_http_cancel_before_login_or_verify_prevents_the_next_send() {
    for path in ["/xled/v1/gestalt", "/xled/v1/login"] {
        let s = Server::start().await;
        s.hold(path);
        let wanted = Arc::new(AtomicBool::new(true));
        let flag = Arc::clone(&wanted);
        let mut c = s.client();
        let task =
            tokio::spawn(async move { c.read(|| flag.load(Ordering::SeqCst)).await.map(|_| ()) });
        tokio::time::timeout(Duration::from_secs(2), s.received.notified())
            .await
            .unwrap();
        wanted.store(false, Ordering::SeqCst);
        s.release.notify_one();
        assert_eq!(task.await.unwrap(), Err(Failure::NotSent));
        assert_eq!(s.count("POST", "/xled/v1/verify"), 0);
        if path.ends_with("gestalt") {
            assert_eq!(s.count("POST", "/xled/v1/login"), 0);
        }
    }
}

#[tokio::test]
async fn twinkly_http_cancel_after_verify_prevents_physical_write() {
    let s = Server::start().await;
    let mut c = s.client();
    c.read(|| true).await.expect("initial read");
    assert_eq!(
        c.apply(&[Step::Mode(Mode::Off)], || false).await,
        Err(Failure::NotSent)
    );
    assert!(s.writes().is_empty());
}

#[tokio::test]
async fn twinkly_http_application_refusal_is_not_a_successful_write_or_retry() {
    let s = Server::start().await;
    let mut c = s.client();
    c.read(|| true).await.unwrap();
    s.fail("/xled/v1/led/mode", 200, br#"{"code":1101}"#.to_vec());
    assert!(matches!(
        c.apply(&[Step::Mode(Mode::Off)], || true).await,
        Err(Failure::Refused | Failure::Protocol(_))
    ));
    assert_eq!(s.writes(), vec![json!({"mode":"off"})]);
}

#[tokio::test]
async fn twinkly_http_lost_write_is_never_replayed_on_the_next_read() {
    let s = Server::start().await;
    let mut c = s.client();
    c.read(|| true).await.unwrap();
    s.wire.lock().unwrap().lose_write = true;
    assert!(c.apply(&[Step::Mode(Mode::Off)], || true).await.is_err());
    c.read(|| true).await.unwrap();
    assert_eq!(s.writes(), vec![json!({"mode":"off"})]);
}

#[tokio::test]
async fn twinkly_http_reply_after_emission_is_kept_when_caller_leaves() {
    let s = Server::start().await;
    let mut c = s.client();
    c.read(|| true).await.unwrap();
    s.hold("/xled/v1/led/mode");
    let wanted = Arc::new(AtomicBool::new(true));
    let flag = Arc::clone(&wanted);
    let task = tokio::spawn(async move {
        c.apply(&[Step::Mode(Mode::Off)], || flag.load(Ordering::SeqCst))
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), s.received.notified())
        .await
        .unwrap();
    wanted.store(false, Ordering::SeqCst);
    s.release.notify_one();
    assert_eq!(task.await.unwrap(), Ok(()));
    assert_eq!(s.writes().len(), 1);
}

#[tokio::test]
async fn twinkly_http_expired_session_renews_before_state_read_only() {
    let s = Server::start().await;
    s.wire.lock().unwrap().lifetime = 1;
    let mut c = s.client();
    c.read(|| true).await.unwrap();
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(
        c.apply(&[Step::Mode(Mode::Off)], || true).await,
        Err(Failure::Expired)
    );
    c.read(|| true).await.unwrap();
    assert_eq!(s.count("POST", "/xled/v1/login"), 2);
    assert!(s.writes().is_empty());
    let w = s.wire.lock().unwrap();
    let challenges: Vec<_> = w
        .requests
        .iter()
        .filter(|(_, p, _)| p.ends_with("/login"))
        .map(|(_, _, b)| b["challenge"].clone())
        .collect();
    // Do not include generated challenges in a failing assertion's output.
    assert!(challenges[0].ne(&challenges[1]));
}

#[tokio::test]
async fn twinkly_http_refused_session_parks_without_authentication_duel() {
    let s = Server::start().await;
    let mut c = s.client();
    c.read(|| true).await.unwrap();
    s.fail("/xled/v1/led/mode", 401, br#"{"code":1101}"#.to_vec());
    assert_eq!(
        c.apply(&[Step::Mode(Mode::Off)], || true).await,
        Err(Failure::Session)
    );
    assert_eq!(c.read(|| true).await.err(), Some(Failure::Session));
    assert_eq!(s.count("POST", "/xled/v1/login"), 1);
}

#[tokio::test]
async fn twinkly_http_old_firmware_never_queries_or_writes_static_color() {
    let s = Server::start().await;
    s.wire.lock().unwrap().firmware = "2.7.0".into();
    let state = s.client().read(|| true).await.unwrap();
    assert!(!state.caps.color);
    assert_eq!(state.color, None);
    assert_eq!(s.count("GET", "/xled/v1/led/color"), 0);
}

#[tokio::test]
async fn twinkly_http_oversized_body_is_rejected_and_redirect_never_followed() {
    for (status, body) in [(200, vec![b' '; 8193]), (302, b"redirect".to_vec())] {
        let s = Server::start().await;
        s.fail("/xled/v1/gestalt", status, body);
        assert!(matches!(
            s.client().read(|| true).await,
            Err(Failure::Protocol(_) | Failure::Refused)
        ));
        assert_eq!(s.count("GET", "/xled/v1/gestalt"), 1);
        assert_eq!(s.count("POST", "/xled/v1/login"), 0);
    }
}
