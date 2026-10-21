//! Certificates and private keys are generated in memory for this test only.
use crate::{
    Config, Failure, HuaweiLte, api::Wire, model::Action, network::Network, protocol::Call,
};
use moli_core::InstanceId;
use moli_runtime::{Hub, spawn_driver};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct Seen {
    requests: AtomicUsize,
    auth: AtomicUsize,
    missing_pin: AtomicBool,
}
struct TlsServer {
    port: u16,
    seen: Arc<Seen>,
    task: tokio::task::JoinHandle<()>,
    mode: Arc<AtomicUsize>,
}
impl Drop for TlsServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}
fn configuration(port: u16) -> Config {
    serde_json::from_value(
        serde_json::json!({"host":"127.0.0.1","port":port,"transport":"https","auth":"sha256-v4"}),
    )
    .unwrap()
}
impl TlsServer {
    async fn start(hub: Hub) -> Self {
        let generated = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let key =
            rustls::pki_types::PrivatePkcs8KeyDer::from(generated.signing_key.serialize_der());
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![generated.cert.der().clone()], key.into())
        .unwrap();
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Seen::default());
        let observed = seen.clone();
        let mode = Arc::new(AtomicUsize::new(0));
        let control = mode.clone();
        let task = tokio::spawn(async move {
            let mut tasks = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    socket=listener.accept()=>{let (socket,_)=socket.unwrap();let acceptor=acceptor.clone();let seen=observed.clone();let hub=hub.clone();let mode=control.clone();tasks.spawn(async move{if let Ok(stream)=acceptor.accept(socket).await{serve(stream,&hub,&seen,&mode).await;}});},
                    Some(result)=tasks.join_next()=>result.unwrap(),
                }
            }
        });
        Self {
            port,
            seen,
            task,
            mode,
        }
    }
}
async fn serve(
    stream: tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
    hub: &Hub,
    seen: &Seen,
    mode: &AtomicUsize,
) {
    let (read, mut write) = tokio::io::split(stream);
    let mut reader = BufReader::new(read);
    let mut first = String::new();
    if reader.read_line(&mut first).await.unwrap_or(0) == 0 {
        return;
    }
    seen.requests.fetch_add(1, Ordering::SeqCst);
    let route = first.split_whitespace().nth(1).unwrap();
    let login = route == "/api/user/login";
    assert_eq!(
        first.split_whitespace().next().unwrap(),
        if login { "POST" } else { "GET" }
    );
    let session = route == "/api/webserver/SesTokInfo";
    let mut size = 0;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
            return;
        }
        if line == "\r\n" {
            break;
        }
        if let Some((key, value)) = line.split_once(':')
            && key.eq_ignore_ascii_case("content-length")
        {
            size = value.trim().parse().unwrap();
        }
    }
    assert!(size < 16384);
    let mut bytes = vec![0; size];
    if reader.read_exact(&mut bytes).await.is_err() {
        return;
    }
    if login || session {
        if hub
            .secret(&InstanceId::from("network"), "cert_sha256")
            .is_none()
        {
            seen.missing_pin.store(true, Ordering::SeqCst);
        }
        if login {
            seen.auth.fetch_add(1, Ordering::SeqCst);
        }
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let body = if mode.load(Ordering::SeqCst) == 2 {
        "x".repeat(crate::xml::MAX_BODY + 1)
    } else {
        match route {
            "/api/webserver/SesTokInfo" => format!(
                "<response><SesInfo>SessionID={nonce}</SesInfo><TokInfo>synthetic-{nonce}</TokInfo></response>"
            ),
            "/api/user/state-login" => {
                "<response><State>-1</State><password_type>4</password_type></response>".into()
            }
            "/api/user/login" => "<response>OK</response>".into(),
            "/api/device/information" => {
                "<response><SerialNumber>synthetic-modem-01</SerialNumber></response>".into()
            }
            _ => "<error><code>100002</code></error>".into(),
        }
    };
    let extra = if login {
        format!(
            "Set-Cookie: SessionID={nonce}; Path=/; Secure\r\n__RequestVerificationTokenone: synthetic-{nonce}-one\r\n__RequestVerificationTokentwo: synthetic-{nonce}-two\r\n"
        )
    } else {
        String::new()
    };
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n",
        body.len()
    );
    let _ = write.write_all(header.as_bytes()).await;
    let _ = write.write_all(body.as_bytes()).await;
}

#[tokio::test]
async fn tls_pin_precedes_session_and_credentials_with_bounded_response_and_no_http_on_mismatch() {
    let vault = crate::runtime_tests::vault().await;
    let server = TlsServer::start(vault.hub.clone()).await;
    let config = configuration(server.port);
    let (network, pin) = Network::open(&config, None).await.unwrap();
    assert_eq!(pin.unwrap().len(), 64);
    assert_eq!(server.seen.requests.load(Ordering::SeqCst), 0);
    network
        .send(
            Call::Read(Action::Info),
            None,
            None,
            Some("SessionID=synthetic".into()),
        )
        .await
        .unwrap();
    assert_eq!(server.seen.requests.load(Ordering::SeqCst), 1);
    server.mode.store(2, Ordering::SeqCst);
    assert_eq!(
        network
            .send(
                Call::Read(Action::Info),
                None,
                None,
                Some("SessionID=synthetic".into())
            )
            .await
            .err(),
        Some(Failure::Transport)
    );
    assert_eq!(server.seen.requests.load(Ordering::SeqCst), 2);
    server.mode.store(0, Ordering::SeqCst);
    assert!(matches!(
        Network::open(&config, Some("00".repeat(32))).await,
        Err(Failure::Certificate)
    ));
    assert_eq!(server.seen.requests.load(Ordering::SeqCst), 2);
    let cancel = CancellationToken::new();
    let task = spawn_driver(
        &vault.hub,
        InstanceId::from("network"),
        Arc::new(HuaweiLte::new(config).unwrap()),
        cancel.clone(),
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while vault
            .hub
            .secret(&InstanceId::from("network"), "device_identity")
            .is_none()
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(!server.seen.missing_pin.load(Ordering::SeqCst));
    assert_eq!(server.seen.auth.load(Ordering::SeqCst), 1);
    cancel.cancel();
    task.await.unwrap();
}
