// Shared contracts through the actual TCP, TLS and Hyper client path.
use super::*;
use moli_core::{Action, Outcome};
use moli_net::contract_test::{Context, Stage};
use moli_runtime::CommandError;
use std::collections::BTreeMap;
use std::convert::Infallible;
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Default)]
struct Barrier {
    entered: Notify,
    release: Notify,
}

#[derive(Debug)]
struct NetworkDriver {
    context: Arc<Context>,
    stop: CancellationToken,
    drained: CancellationToken,
    settled: Arc<Notify>,
}

impl Driver for NetworkDriver {
    fn kind(&self) -> &'static str {
        "transport-contract"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let mut cloud = Cloud::new("user@example.com".into(), "x".into(), Arc::default());
            cloud.transport = Some(Arc::clone(&self.context));
            let mut link = Link::new(
                cloud,
                Pace {
                    idle: Duration::from_secs(600),
                    follow: Duration::from_millis(20),
                    confirm: Duration::from_millis(400),
                },
            );
            link.settled = Some(Arc::clone(&self.settled));
            let result = tokio::select! {
                result = link.serve(ctx) => result,
                () = self.stop.cancelled() => Ok(()),
            };
            link.jobs.abort_all();
            while let Some(result) = link.jobs.join_next().await {
                assert!(
                    !result.as_ref().is_err_and(tokio::task::JoinError::is_panic),
                    "worker panic: {result:?}"
                );
            }
            link.line.close();
            self.drained.cancel();
            assert!(result.is_ok(), "driver failed: {result:?}");
            self.stop.cancelled().await;
            result
        })
    }
}

async fn serve_peer(
    acceptor: tokio_rustls::TlsAcceptor,
    socket: tokio::net::TcpStream,
    fake: Arc<Fake>,
    barrier: Arc<Barrier>,
    after_emission: bool,
) {
    // A command cancelled after TCP intentionally closes before TLS.
    let stream = match acceptor.accept(socket).await {
        Ok(stream) => stream,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::ConnectionReset
            ) =>
        {
            return;
        }
        Err(error) => panic!("TLS fixture failure: {error}"),
    };
    let received = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let request_complete = Arc::clone(&received);
    let service =
        hyper::service::service_fn(move |request: http::Request<hyper::body::Incoming>| {
            let fake = Arc::clone(&fake);
            let received = Arc::clone(&received);
            let barrier = Arc::clone(&barrier);
            async move {
                let (parts, body) = request.into_parts();
                let body = http_body_util::Limited::new(body, 1024 * 1024)
                    .collect()
                    .await
                    .unwrap()
                    .to_bytes();
                received.store(true, SeqCst);
                let request = Request::from_parts(parts, Full::new(body));
                if after_emission && is_write(&request) {
                    barrier.entered.notify_one();
                    barrier.release.notified().await;
                }
                let (status, body) = fake.answer(request).await;
                Ok::<_, Infallible>(
                    http::Response::builder()
                        .status(status)
                        .body(Full::new(Body::from(body.to_string())))
                        .unwrap(),
                )
            }
        });
    let result = hyper::server::conn::http1::Builder::new()
        .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
        .await;
    if let Err(error) = result {
        assert!(
            !request_complete.load(SeqCst)
                && (error.is_incomplete_message() || error.is_shutdown()),
            "server error before request reception: {error}"
        );
    }
}

async fn serve_cloud(
    listener: TcpListener,
    config: rustls::ServerConfig,
    server_cancel: CancellationToken,
    peer: Arc<Fake>,
    response_barrier: Arc<Barrier>,
    after_emission: bool,
) {
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let mut connections = tokio::task::JoinSet::new();
    loop {
        let (socket, _) = tokio::select! {
            () = server_cancel.cancelled() => break,
            accepted = listener.accept() => accepted.unwrap(),
        };
        let (acceptor, fake) = (acceptor.clone(), Arc::clone(&peer));
        let barrier = Arc::clone(&response_barrier);
        connections.spawn(async move {
            serve_peer(acceptor, socket, fake, barrier, after_emission).await;
        });
    }
    while let Some(result) = connections.join_next().await {
        result.unwrap();
    }
}

fn transport(
    address: std::net::SocketAddr,
    barrier: &Arc<Barrier>,
    stage: Stage,
    after_emission: bool,
) -> (rustls::ServerConfig, Arc<Context>) {
    let identity = rcgen::generate_simple_self_signed(vec![HOST.into()]).unwrap();
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![identity.cert.der().clone()],
        rustls::pki_types::PrivatePkcs8KeyDer::from(identity.key_pair.serialize_der()).into(),
    )
    .unwrap();
    let observer = Arc::clone(barrier);
    let context = Arc::new(
        Context::new(
            1,
            BTreeMap::from([(HOST.into(), address)]),
            identity.cert.der().to_vec(),
            Arc::new(move |event| {
                let barrier = Arc::clone(&observer);
                Box::pin(async move {
                    let mutation =
                        event.method != http::Method::GET && !event.path.ends_with("login");
                    if !after_emission && mutation && event.stage == stage {
                        barrier.entered.notify_one();
                        barrier.release.notified().await;
                    }
                })
            }),
        )
        .unwrap(),
    );
    (config, context)
}

async fn start_caller(hub: &Hub, approval: bool) -> tokio::task::JoinHandle<()> {
    let point = PointId::new(&heater(), CONTROL_KEY);
    let held = if approval {
        let result = hub
            .command(&point, control_value(), Origin::Mcp, None)
            .await;
        let Err(CommandError::NeedsApproval { id, .. }) = result else {
            panic!("approval missing: {result:?}")
        };
        Some(id)
    } else {
        None
    };
    let caller_hub = hub.clone();
    tokio::spawn(async move {
        if let Some(id) = held {
            let _ = caller_hub
                .resolve_approval(id, &point, true, Origin::Ui, None)
                .await;
        } else {
            let _ = caller_hub
                .command(&point, control_value(), Origin::Ui, None)
                .await;
        }
    })
}

#[derive(Clone, Copy, Debug)]
enum Departure {
    Stays,
    Abandons,
    Expires,
}

async fn depart(call: tokio::task::JoinHandle<()>, barrier: &Barrier, abandon: bool, expire: bool) {
    if abandon {
        call.abort();
        assert!(call.await.unwrap_err().is_cancelled());
    } else if expire {
        let _ = tokio::time::timeout(Duration::from_secs(3), call)
            .await
            .unwrap();
    } else {
        barrier.release.notify_one();
        call.await.unwrap();
    }
    if abandon || expire {
        barrier.release.notify_one();
    }
}

async fn case(stage: Stage, approval: bool, departure: Departure, after_emission: bool) -> bool {
    let abandon = matches!(departure, Departure::Abandons);
    let expire = matches!(departure, Departure::Expires);
    let fake = Arc::new(Fake::default());
    let barrier = Arc::new(Barrier::default());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (config, context) = transport(address, &barrier, stage, after_emission);
    let server_stop = CancellationToken::new();
    let server_cancel = server_stop.clone();
    let peer = Arc::clone(&fake);
    let response_barrier = Arc::clone(&barrier);
    let server = tokio::spawn(serve_cloud(
        listener,
        config,
        server_cancel,
        peer,
        response_barrier,
        after_emission,
    ));
    let hub = Hub::new(HubOptions {
        command_timeout: if expire {
            Duration::from_millis(1800)
        } else {
            Duration::from_secs(5)
        },
        ..HubOptions::default()
    })
    .unwrap();
    let stop = CancellationToken::new();
    let drained = CancellationToken::new();
    let cancel = CancellationToken::new();
    let settled = Arc::new(Notify::new());
    let driver = spawn_driver(
        &hub,
        InstanceId::from(INSTANCE),
        Arc::new(NetworkDriver {
            context: Arc::clone(&context),
            stop: stop.clone(),
            drained: drained.clone(),
            settled: Arc::clone(&settled),
        }),
        cancel.clone(),
    );
    until(|| ready(&hub)).await;
    let call = start_caller(&hub, approval).await;
    tokio::time::timeout(Duration::from_secs(3), barrier.entered.notified())
        .await
        .unwrap();
    depart(call, &barrier, abandon, expire).await;
    tokio::time::timeout(Duration::from_secs(3), settled.notified())
        .await
        .unwrap();
    stop.cancel();
    tokio::time::timeout(Duration::from_secs(3), drained.cancelled())
        .await
        .unwrap();
    cancel.cancel();
    driver.await.unwrap();
    let report = context
        .finish_connections(Duration::from_secs(3))
        .await
        .unwrap();
    server_stop.cancel();
    tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap();
    assert!(
        report.healthy(&[]),
        "client connection fixture failure: {report:?}"
    );
    let correct = mutations(&fake) == usize::from(after_emission || (!abandon && !expire));
    if after_emission {
        assert!(
            hub.journal(100)
                .iter()
                .any(|entry| matches!(entry.outcome, Outcome::Ok)
                    && matches!(
                        entry.action,
                        Action::Command { .. } | Action::Approval { .. }
                    )),
            "received response must be journaled after caller leaves"
        );
    }
    if !correct {
        eprintln!(
            "unexpected mutation at {stage:?}, approval={approval}, abandon={abandon}, expire={expire}"
        );
    }
    correct
}

#[tokio::test]
async fn tcp_tls_abandonment_never_emits() {
    let mut correct = true;
    for stage in [
        Stage::BeforeTcp,
        Stage::AfterTcp,
        Stage::AfterTls,
        Stage::BeforeSend,
    ] {
        for approval in [false, true] {
            correct &= case(stage, approval, Departure::Abandons, false).await;
        }
    }
    assert!(correct, "abandoned mutations received by TLS peer");
}

#[tokio::test]
async fn tcp_tls_deadline_never_emits() {
    for stage in [Stage::AfterTcp, Stage::AfterTls, Stage::BeforeSend] {
        for approval in [false, true] {
            assert!(case(stage, approval, Departure::Expires, false).await);
        }
    }
}

#[tokio::test]
async fn tcp_tls_without_abandonment_emits_once() {
    for stage in [Stage::AfterTcp, Stage::AfterTls, Stage::BeforeSend] {
        for approval in [false, true] {
            assert!(case(stage, approval, Departure::Stays, false).await);
        }
    }
}

#[tokio::test]
async fn tls_response_is_retained_after_complete_request_and_caller_departure() {
    for approval in [false, true] {
        assert!(case(Stage::BeforeSend, approval, Departure::Abandons, true).await);
    }
}

#[tokio::test]
async fn tls_rejects_an_untrusted_certificate_before_any_http_request() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let identity = rcgen::generate_simple_self_signed(vec![HOST.into()]).unwrap();
    let untrusted = rcgen::generate_simple_self_signed(vec![HOST.into()]).unwrap();
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![identity.cert.der().clone()],
        rustls::pki_types::PrivatePkcs8KeyDer::from(identity.key_pair.serialize_der()).into(),
    )
    .unwrap();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let result = tokio_rustls::TlsAcceptor::from(Arc::new(config))
            .accept(socket)
            .await;
        assert!(result.is_err(), "no HTTP service may start on rejected TLS");
    });
    let context = Arc::new(
        Context::new(
            2,
            BTreeMap::from([(HOST.into(), address)]),
            untrusted.cert.der().to_vec(),
            Arc::new(|_| Box::pin(async {})),
        )
        .unwrap(),
    );
    let request = moli_net::empty(Request::builder().uri("/contract")).unwrap();
    let error = moli_net::contract_test::scope(
        Arc::clone(&context),
        moli_net::web(HOST, 443, true, request, Duration::from_secs(3), 1024),
    )
    .await
    .unwrap_err();
    let message = format!("{error:#}");
    assert!(
        message.contains("invalid peer certificate"),
        "wrong TLS failure: {message}"
    );
    server.await.unwrap();
    assert!(
        context
            .finish_connections(Duration::from_secs(3))
            .await
            .unwrap()
            .healthy(&[])
    );
}
