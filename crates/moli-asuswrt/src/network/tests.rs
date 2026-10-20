use super::*;
use crate::{json, protocol};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
};
pub(crate) struct Server {
    pub port: u16,
    pub calls: Arc<Mutex<Vec<String>>>,
    pub mode: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    pub fn config(&self) -> Config {
        serde_json::from_value(serde_json::json!({"host":"127.0.0.1","port":self.port,"transport":"http","allow_plain_auth":true})).unwrap()
    }
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let calls = Arc::new(Mutex::new(vec![]));
        let mode = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let control = mode.clone();
        let task = tokio::spawn(async move {
            let mut tasks = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                 socket=listener.accept()=>{let(socket,_)=socket.unwrap();let seen=seen.clone();let control=control.clone();tasks.spawn(serve(socket,seen,control));},Some(result)=tasks.join_next()=>result.unwrap(),
                }
            }
        });
        Self {
            port,
            calls,
            mode,
            task,
        }
    }
}
async fn serve(
    socket: tokio::net::TcpStream,
    seen: Arc<Mutex<Vec<String>>>,
    control: Arc<AtomicUsize>,
) {
    let mut reader = BufReader::new(socket);
    let mut first = String::new();
    if reader.read_line(&mut first).await.unwrap_or(0) == 0 {
        return;
    }
    assert!(first == "POST /login.cgi HTTP/1.1\r\n" || first == "POST /appGet.cgi HTTP/1.1\r\n");
    let login = first.contains("login.cgi");
    let mut size = 0;
    let mut cookie = false;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        if line == "\r\n" {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            if key.eq_ignore_ascii_case("content-length") {
                size = value.trim().parse().unwrap();
            }
            if key.eq_ignore_ascii_case("cookie") {
                cookie = true;
                assert!(value.trim().starts_with("asus_token="));
            }
        }
    }
    assert!(size < 16384);
    assert_eq!(cookie, !login);
    let mut bytes = vec![0; size];
    reader.read_exact(&mut bytes).await.unwrap();
    let body = String::from_utf8(bytes).unwrap();
    let action = if login {
        "login"
    } else if body.contains("label_mac") {
        "info"
    } else if body.contains("cpu_usage") {
        "telemetry"
    } else {
        assert_eq!(body, "hook%3Dget_clientlist%28%29");
        "clients"
    };
    seen.lock().unwrap().push(action.into());
    let mode = control.load(Ordering::SeqCst);
    if mode == 3 {
        tokio::time::sleep(Duration::from_secs(20)).await;
        return;
    }
    let response = if mode == 2 {
        "x".repeat(json::MAX_BODY + 1)
    } else if login {
        serde_json::json!({"asus_token":format!("synthetic-{}",std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos())}).to_string()
    } else {
        response(action, mode, seen.lock().unwrap().len())
    };
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\nSet-Cookie: ignored=synthetic\r\n\r\n",
        response.len()
    );
    let socket = reader.get_mut();
    let _ = socket.write_all(header.as_bytes()).await;
    let _ = socket.write_all(response.as_bytes()).await;
}
fn response(action: &str, mode: usize, n: usize) -> String {
    match (action, mode) {
        ("info", _) => include_str!("../../../../integrations/asuswrt/fixtures/info.json").into(),
        ("telemetry", 5) => r#"{"error_status":7}"#.into(),
        ("telemetry", 6) => serde_json::json!({
          "cpu_usage":{"cpu1_total":n*100,"cpu1_usage":n*20},
          "memory_usage":{"mem_total":"100","mem_used":"25","mem_free":"75"},
          "netdev":{"INTERNET_rx":format!("0x{:x}",n*1000),"INTERNET_tx":format!("0x{:x}",n*2000)}
        })
        .to_string(),
        ("telemetry", _) => {
            r#"{"memory_usage":{"mem_total":"100","mem_used":"25","mem_free":"75"}}"#.into()
        }
        ("clients", 1) => r#"{"get_clientlist":{"broken":{}}}"#.into(),
        ("clients", 4) => {
            include_str!("../../../../integrations/asuswrt/fixtures/clients-empty.json").into()
        }
        ("clients", 6) => {
            let clients=(0..512).map(|i|(
     format!("aa:bb:cc:dd:{:02x}:{:02x}",i/256,i%256),
     serde_json::json!({"isOnline":"1","isWL":"1","rssi":"-50","curRx":"10","curTx":"20"})
   )).collect::<serde_json::Map<_,_>>();
            serde_json::json!({"get_clientlist":clients}).to_string()
        }
        ("clients", _) => {
            include_str!("../../../../integrations/asuswrt/fixtures/clients.json").into()
        }
        _ => unreachable!("closed route assertion precedes response"),
    }
}
#[tokio::test]
async fn only_fixed_post_routes_send_cookie_and_oversize_is_rejected() {
    let server = Server::start().await;
    let (net, _) = Network::open(&server.config(), None).await.unwrap();
    let answer = net
        .send(
            Call::Login,
            protocol::body(Call::Login, Some(("synthetic-user", "synthetic-password"))).unwrap(),
            None,
        )
        .await
        .unwrap();
    assert!(protocol::token(&json::parse(&answer.body).unwrap()).is_ok());
    net.send(
        Call::Read(crate::model::Action::Info),
        protocol::body(Call::Read(crate::model::Action::Info), None).unwrap(),
        Some("synthetic".into()),
    )
    .await
    .unwrap();
    assert_eq!(*server.calls.lock().unwrap(), vec!["login", "info"]);
    server.mode.store(2, Ordering::SeqCst);
    assert!(
        net.send(
            Call::Login,
            protocol::body(Call::Login, Some(("synthetic-user", "synthetic-password"))).unwrap(),
            None
        )
        .await
        .is_err()
    );
}
#[tokio::test]
async fn transport_can_be_cancelled_and_never_retries() {
    let server = Server::start().await;
    server.mode.store(3, Ordering::SeqCst);
    let (net, _) = Network::open(&server.config(), None).await.unwrap();
    let start = tokio::time::Instant::now();
    let result = tokio::time::timeout(
        Duration::from_millis(100),
        net.send(Call::Login, "synthetic".into(), None),
    )
    .await;
    assert!(result.is_err());
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_eq!(server.calls.lock().unwrap().len(), 1);
}
