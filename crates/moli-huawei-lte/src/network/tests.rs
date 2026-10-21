use super::*;
use crate::{Auth, Transport, model::Action, xml};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
};
pub(crate) struct Server {
    pub port: u16,
    pub calls: Arc<Mutex<Vec<Call>>>,
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
        Config {
            host: "127.0.0.1".into(),
            port: self.port,
            transport: Transport::Http,
            auth: Auth::Sha256V4,
            session: crate::Session::Sestok,
            allow_plain_auth: true,
            every: 60,
            info_every: 3600,
        }
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
                tokio::select! {socket=listener.accept()=>{let(socket,_)=socket.unwrap();tasks.spawn(serve(socket,seen.clone(),control.clone()));},Some(result)=tasks.join_next()=>result.unwrap(),}
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
async fn serve(socket: TcpStream, seen: Arc<Mutex<Vec<Call>>>, control: Arc<AtomicUsize>) {
    let mut reader = BufReader::new(socket);
    let mut first = String::new();
    if reader.read_line(&mut first).await.unwrap_or(0) == 0 {
        return;
    }
    let parts = first.split_whitespace().collect::<Vec<_>>();
    let call = match parts[1] {
        "/api/webserver/SesTokInfo" => Call::Session,
        "/api/webserver/token" => Call::Token,
        "/api/user/state-login" => Call::State,
        "/api/user/login" => Call::Login,
        "/api/device/information" => Call::Read(Action::Info),
        "/api/monitoring/status" => Call::Read(Action::Status),
        "/api/device/signal" => Call::Read(Action::Signal),
        "/api/monitoring/traffic-statistics" => Call::Read(Action::Traffic),
        _ => panic!("unexpected route"),
    };
    assert_eq!(parts[0], if call == Call::Login { "POST" } else { "GET" });
    let mut size = 0;
    let mut cookie = false;
    let mut token = false;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        if line == "\r\n" {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            match key.to_ascii_lowercase().as_str() {
                "content-length" => size = value.trim().parse().unwrap(),
                "cookie" => {
                    cookie = true;
                    assert!(value.trim().starts_with("SessionID="));
                }
                "__requestverificationtoken" => token = true,
                _ => {}
            }
        }
    }
    assert!(size < 16384);
    let mut bytes = vec![0; size];
    reader.read_exact(&mut bytes).await.unwrap();
    if call == Call::Login {
        assert!(
            String::from_utf8(bytes)
                .unwrap()
                .contains("<password_type>4</password_type>")
        );
    } else {
        assert_eq!(size, 0);
    }
    if !matches!(call, Call::Session | Call::Token) {
        assert!(cookie);
        if matches!(call, Call::State | Call::Login) {
            assert!(token);
        }
    }
    seen.lock().unwrap().push(call);
    let mode = control.load(Ordering::SeqCst);
    if mode == 3 {
        tokio::time::sleep(Duration::from_secs(20)).await;
        return;
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let body = if mode == 2 {
        "x".repeat(xml::MAX_BODY + 1)
    } else {
        response(call, mode, &nonce.to_string(), &seen)
    };
    let headers = if matches!(call, Call::Session | Call::Token | Call::Login) {
        format!(
            "Set-Cookie: SessionID={nonce}; Path=/\r\n__RequestVerificationTokenone: synthetic-{nonce}-one\r\n__RequestVerificationTokentwo: synthetic-{nonce}-two\r\n"
        )
    } else {
        String::new()
    };
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n",
        body.len()
    );
    let socket = reader.get_mut();
    let _ = socket.write_all(head.as_bytes()).await;
    let _ = socket.write_all(body.as_bytes()).await;
}
fn response(call: Call, mode: usize, nonce: &str, seen: &Mutex<Vec<Call>>) -> String {
    match call{
  Call::Session=>format!("<response><SesInfo>SessionID={nonce}</SesInfo><TokInfo>synthetic-{nonce}</TokInfo></response>"),
  Call::Token=>format!("<response><token>synthetic-{nonce}</token></response>"),
  Call::State=>"<response><State>-1</State><password_type>4</password_type></response>".into(),
  Call::Login if mode==6=>"<error><code>108007</code></error>".into(),Call::Login=>"<response>OK</response>".into(),
  Call::Read(Action::Info)=>"<response><SerialNumber>synthetic-modem-01</SerialNumber></response>".into(),
  Call::Read(Action::Status)=>format!("<response><SimStatus>{}</SimStatus><ConnectionStatus>{}</ConnectionStatus><CurrentNetworkType>7</CurrentNetworkType></response>",i32::from(mode!=4),if mode==5{902}else{901}),
  Call::Read(Action::Signal) if mode==7=>"<error><code>100002</code></error>".into(),
  Call::Read(Action::Signal)=>"<response><rssi>-75dBm</rssi><rsrp>-95dBm</rsrp><rsrq>-9.5dB</rsrq><sinr>20dB</sinr></response>".into(),
  Call::Read(Action::Traffic) if mode==1=>"<response><CurrentDownload>broken</CurrentDownload></response>".into(),
  Call::Read(Action::Traffic) if mode==9&&seen.lock().unwrap().iter().filter(|c|**c==Call::Read(Action::Traffic)).count()==1=>"<error><code>125003</code></error>".into(),
  Call::Read(Action::Traffic)=>"<response><CurrentDownload>1000</CurrentDownload><CurrentUpload>2000</CurrentUpload><CurrentDownloadRate>10</CurrentDownloadRate><CurrentUploadRate>20</CurrentUploadRate><CurrentConnectTime>300</CurrentConnectTime></response>".into(),
 }
}
#[tokio::test]
async fn fixed_routes_and_no_retry_with_a_body_larger_than_one_mib() {
    let server = Server::start().await;
    let (network, _) = Network::open(&server.config(), None).await.unwrap();
    let answer = network.send(Call::Session, None, None, None).await.unwrap();
    assert!(xml::parse(&answer.body).is_ok());
    assert_eq!(*server.calls.lock().unwrap(), vec![Call::Session]);
    server.mode.store(2, Ordering::SeqCst);
    assert_eq!(
        network.send(Call::Session, None, None, None).await.err(),
        Some(Failure::Transport)
    );
    assert_eq!(server.calls.lock().unwrap().len(), 2);
}
#[tokio::test]
async fn an_inflight_session_request_is_cancellable_without_an_implicit_retry() {
    let server = Server::start().await;
    server.mode.store(3, Ordering::SeqCst);
    let (network, _) = Network::open(&server.config(), None).await.unwrap();
    let result = tokio::time::timeout(
        Duration::from_millis(100),
        network.send(Call::Session, None, None, None),
    )
    .await;
    assert!(result.is_err());
    assert_eq!(server.calls.lock().unwrap().len(), 1);
}
