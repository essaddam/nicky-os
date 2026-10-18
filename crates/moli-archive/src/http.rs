//! Authenticated, bounded HTTP. Admission happens before allocating request bodies.
use crate::{
    Archive, Snapshot,
    protocol::{self, CatalogUpdate, Query, Status},
};
use anyhow::{Result, bail, ensure};
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{Path, Request, State},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use hyper_util::{
    rt::{TokioIo, TokioTimer},
    service::TowerToHyperService,
};
use moli_i18n::tr;
use ring::hmac;
use std::{
    io::Read,
    net::SocketAddr,
    path::Path as FsPath,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    net::TcpListener,
    sync::{Semaphore, watch},
};

struct Auth {
    key: hmac::Key,
    tag: hmac::Tag,
}
impl std::fmt::Debug for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Auth").finish_non_exhaustive()
    }
}
impl Auth {
    fn from_file(path: &FsPath) -> Result<Self> {
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(4097)
            .read_to_end(&mut bytes)?;
        ensure!((16..=4096).contains(&bytes.len()), "invalid_token_file");
        let token = std::str::from_utf8(&bytes)?.trim();
        ensure!(
            token.len() >= 16 && !token.chars().any(char::is_whitespace),
            "invalid_token_file"
        );
        let key = hmac::Key::new(hmac::HMAC_SHA256, b"moli-archive bearer verification v1");
        let tag = hmac::sign(&key, token.as_bytes());
        Ok(Self { key, tag })
    }
    fn accepts(&self, headers: &axum::http::HeaderMap) -> bool {
        headers
            .get(header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.strip_prefix("Bearer "))
            .is_some_and(|token| {
                token.len() <= 4096
                    && hmac::verify(&self.key, token.as_bytes(), self.tag.as_ref()).is_ok()
            })
    }
}
#[derive(Debug)]
struct App {
    installation: String,
    archive: Mutex<Archive>,
    status: RwLock<Status>,
    snapshot: RwLock<Snapshot>,
    auth: Auth,
    jobs: Arc<Semaphore>,
    reads: Arc<Semaphore>,
    fatal: watch::Sender<bool>,
    publishing: AtomicBool,
}
type Shared = Arc<App>;
impl App {
    fn failure(&self, archive: &Archive, error: &anyhow::Error) {
        eprintln!("{}: {error:#}", tr!("archive.storage_error"));
        if archive.poisoned {
            self.fatal.send_replace(true);
        }
    }
    fn worker_failed(&self, error: &tokio::task::JoinError) {
        eprintln!("{}: {error:#}", tr!("archive.storage_error"));
        self.fatal.send_replace(true);
    }
    fn refresh(&self, archive: &Archive) {
        *self
            .status
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = archive.status();
        *self
            .snapshot
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = archive.snapshot();
    }
    fn status(&self) -> Status {
        *self
            .status
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}
fn error(state: Option<&App>, status: StatusCode, code: &str) -> Response {
    if status.is_server_error() {
        eprintln!(
            "{}: HTTP {} {code}",
            tr!("archive.storage_error"),
            status.as_u16()
        );
    }
    let key = match status {
        StatusCode::UNAUTHORIZED => "archive.unauthorized",
        StatusCode::CONFLICT => "archive.conflict",
        StatusCode::PAYLOAD_TOO_LARGE => "archive.limit",
        StatusCode::SERVICE_UNAVAILABLE => "archive.busy",
        StatusCode::INTERNAL_SERVER_ERROR => "archive.storage_error",
        _ => "archive.invalid_request",
    };
    let mut body = serde_json::json!({"error":code,"message":tr!(key)});
    if let Some(state) = state {
        let s = state.status();
        body["durable_upto"] = s.durable_upto.into();
        body["published_upto"] = s.published_upto.into();
        body["generation"] = s.generation.into();
    }
    (status, Json(body)).into_response()
}
async fn authenticate(State(state): State<Shared>, request: Request, next: Next) -> Response {
    if !state.auth.accepts(request.headers()) {
        return error(None, StatusCode::UNAUTHORIZED, "unauthorized");
    }
    next.run(request).await
}
fn identity(state: &App, installation: &str) -> Result<()> {
    protocol::installation(installation)?;
    ensure!(installation == state.installation, "installation_mismatch");
    Ok(())
}
async fn status(State(state): State<Shared>, Path(installation): Path<String>) -> Response {
    if identity(&state, &installation).is_err() {
        return error(None, StatusCode::NOT_FOUND, "installation_mismatch");
    }
    Json(state.status()).into_response()
}
async fn catalog(State(state): State<Shared>, Path(installation): Path<String>) -> Response {
    if identity(&state, &installation).is_err() {
        return error(None, StatusCode::NOT_FOUND, "installation_mismatch");
    }
    let snapshot = state
        .snapshot
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Json(serde_json::json!({"revision":snapshot.revision(),"catalog":snapshot.catalog()}))
        .into_response()
}
async fn batches(
    State(state): State<Shared>,
    Path((installation, range)): Path<(String, String)>,
    request: Request<Body>,
) -> Response {
    if identity(&state, &installation).is_err() {
        return error(None, StatusCode::NOT_FOUND, "installation_mismatch");
    }
    if request
        .headers()
        .get(header::CONTENT_ENCODING)
        .and_then(|v| v.to_str().ok())
        != Some("gzip")
    {
        return error(
            Some(&state),
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "gzip_required",
        );
    }
    let Some((first, last)) = range
        .split_once('-')
        .and_then(|(a, b)| Some((a.parse::<i64>().ok()?, b.parse::<i64>().ok()?)))
    else {
        return error(Some(&state), StatusCode::BAD_REQUEST, "invalid_batch_range");
    };
    let Ok(permit) = state.jobs.clone().try_acquire_owned() else {
        return error(Some(&state), StatusCode::SERVICE_UNAVAILABLE, "busy");
    };
    let Ok(Ok(body)) = tokio::time::timeout(
        Duration::from_secs(30),
        to_bytes(request.into_body(), protocol::MAX_COMPRESSED),
    )
    .await
    else {
        return error(
            Some(&state),
            StatusCode::PAYLOAD_TOO_LARGE,
            "compressed_limit",
        );
    };
    let work = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let batch = protocol::decode(first, last, &body).map_err(|e| {
            if ["compressed_limit", "decompressed_limit"].contains(&e.to_string().as_str()) {
                ("batch_limit", StatusCode::PAYLOAD_TOO_LARGE)
            } else {
                ("invalid_batch", StatusCode::BAD_REQUEST)
            }
        })?;
        drop(batch.operations);
        let mut archive = work
            .archive
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let result = archive.receive(first, last, &batch.bytes);
        work.refresh(&archive);
        result.map_err(|e| {
            let code = e.to_string();
            if ["sequence_gap", "batch_conflict", "receipt_expired"].contains(&code.as_str()) {
                ("batch_conflict", StatusCode::CONFLICT)
            } else {
                work.failure(&archive, &e);
                ("storage_unavailable", StatusCode::SERVICE_UNAVAILABLE)
            }
        })
    })
    .await;
    match result {
        Ok(Ok(ack)) => {
            schedule_publication(&state);
            Json(ack).into_response()
        }
        Ok(Err((code, status))) => error(Some(&state), status, code),
        Err(e) => {
            state.worker_failed(&e);
            error(
                Some(&state),
                StatusCode::INTERNAL_SERVER_ERROR,
                "worker_failed",
            )
        }
    }
}
async fn query(
    State(state): State<Shared>,
    Path(installation): Path<String>,
    request: Request<Body>,
) -> Response {
    if identity(&state, &installation).is_err() {
        return error(None, StatusCode::NOT_FOUND, "installation_mismatch");
    }
    let Ok(permit) = state.reads.clone().try_acquire_owned() else {
        return error(Some(&state), StatusCode::SERVICE_UNAVAILABLE, "busy");
    };
    let snapshot = state
        .snapshot
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let Ok(Ok(body)) = tokio::time::timeout(
        Duration::from_secs(2),
        to_bytes(request.into_body(), 64 * 1024),
    )
    .await
    else {
        return error(
            Some(&state),
            StatusCode::PAYLOAD_TOO_LARGE,
            "query_body_limit",
        );
    };
    let query: Query = match serde_json::from_slice(&body) {
        Ok(q) => q,
        Err(_) => return error(Some(&state), StatusCode::BAD_REQUEST, "invalid_query"),
    };
    match tokio::task::spawn_blocking(move || {
        let _permit = permit;
        snapshot.query(&query)
    })
    .await
    {
        Ok(Ok(response)) => Json(response).into_response(),
        Ok(Err(_)) => error(Some(&state), StatusCode::BAD_REQUEST, "query_rejected"),
        Err(e) => {
            eprintln!("{}: {e:#}", tr!("archive.storage_error"));
            error(
                Some(&state),
                StatusCode::INTERNAL_SERVER_ERROR,
                "worker_failed",
            )
        }
    }
}
async fn update_catalog(
    State(state): State<Shared>,
    Path(installation): Path<String>,
    request: Request<Body>,
) -> Response {
    if identity(&state, &installation).is_err() {
        return error(None, StatusCode::NOT_FOUND, "installation_mismatch");
    }
    let Ok(permit) = state.jobs.clone().try_acquire_owned() else {
        return error(Some(&state), StatusCode::SERVICE_UNAVAILABLE, "busy");
    };
    let Ok(Ok(body)) = tokio::time::timeout(
        Duration::from_secs(2),
        to_bytes(request.into_body(), protocol::MAX_CATALOG_BYTES + 64),
    )
    .await
    else {
        return error(
            Some(&state),
            StatusCode::PAYLOAD_TOO_LARGE,
            "catalog_body_limit",
        );
    };
    let update: CatalogUpdate = match serde_json::from_slice(&body) {
        Ok(u) => u,
        Err(_) => return error(Some(&state), StatusCode::BAD_REQUEST, "invalid_catalog"),
    };
    if update.catalog.validate().is_err() {
        return error(Some(&state), StatusCode::BAD_REQUEST, "invalid_catalog");
    }
    let work = state.clone();
    match tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let mut archive = work
            .archive
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let result = archive.set_catalog(update);
        work.refresh(&archive);
        result.map_err(|e| {
            if archive.poisoned {
                work.failure(&archive, &e);
                ("storage_unavailable", StatusCode::SERVICE_UNAVAILABLE)
            } else {
                ("catalog_rejected", StatusCode::CONFLICT)
            }
        })
    })
    .await
    {
        Ok(Ok(status)) => Json(status).into_response(),
        Ok(Err((code, status))) => error(Some(&state), status, code),
        Err(e) => {
            state.worker_failed(&e);
            error(
                Some(&state),
                StatusCode::INTERNAL_SERVER_ERROR,
                "worker_failed",
            )
        }
    }
}
fn schedule_publication(state: &Shared) {
    if state.status().published_upto >= state.status().durable_upto {
        return;
    }
    if state
        .publishing
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    let state = state.clone();
    tokio::spawn(async move {
        loop {
            let Ok(permit) = state.jobs.clone().acquire_owned().await else {
                break;
            };
            let work = state.clone();
            let result = tokio::task::spawn_blocking(move || {
                let _permit = permit;
                let mut archive = work
                    .archive
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let result = archive.publish();
                work.refresh(&archive);
                if let Err(e) = &result {
                    work.failure(&archive, e);
                }
                result
            })
            .await;
            match result {
                Ok(Ok(_)) => {}
                Ok(Err(_)) => {
                    state.fatal.send_replace(true);
                    break;
                }
                Err(e) => {
                    state.worker_failed(&e);
                    break;
                }
            }
            if state.status().published_upto < state.status().durable_upto {
                tokio::task::yield_now().await;
                continue;
            }
            state.publishing.store(false, Ordering::Release);
            if state.status().published_upto < state.status().durable_upto
                && state
                    .publishing
                    .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
            {
                continue;
            }
            return;
        }
        state.publishing.store(false, Ordering::Release);
    });
}
/// Dedicated HTTP/1 listener with finite connection/header/body admission.
pub async fn serve(
    archive: Archive,
    installation: String,
    address: SocketAddr,
    token_file: &FsPath,
) -> Result<()> {
    ensure!(!archive.poisoned, "archive_requires_reopen");
    let snapshot = archive.snapshot();
    let initial_status = archive.status();
    let state = Arc::new(App {
        installation,
        archive: Mutex::new(archive),
        status: RwLock::new(initial_status),
        snapshot: RwLock::new(snapshot),
        auth: Auth::from_file(token_file)?,
        jobs: Arc::new(Semaphore::new(1)),
        reads: Arc::new(Semaphore::new(1)),
        fatal: watch::channel(false).0,
        publishing: AtomicBool::new(false),
    });
    let listener = TcpListener::bind(address).await?;
    serve_state(state, listener).await
}
async fn serve_state(state: Shared, listener: TcpListener) -> Result<()> {
    let router = Router::new()
        .route("/v1/installations/{installation}/status", get(status))
        .route(
            "/v1/installations/{installation}/batches/{range}",
            post(batches),
        )
        .route(
            "/v1/installations/{installation}/catalog",
            get(catalog).put(update_catalog),
        )
        .route("/v1/installations/{installation}/query", post(query))
        .layer(middleware::from_fn_with_state(state.clone(), authenticate))
        .with_state(state.clone());
    let connections = Arc::new(Semaphore::new(16));
    let mut fatal = state.fatal.subscribe();
    ensure!(!*fatal.borrow(), "archive_requires_reopen");
    schedule_publication(&state);
    loop {
        let (socket, _) = tokio::select! {
            _ = fatal.changed() => bail!("archive_requires_reopen"),
            accepted = listener.accept() => accepted?,
        };
        let Ok(permit) = connections.clone().try_acquire_owned() else {
            drop(socket);
            continue;
        };
        let service = TowerToHyperService::new(router.clone());
        tokio::spawn(async move {
            let _permit = permit;
            let mut builder = hyper::server::conn::http1::Builder::new();
            builder
                .timer(TokioTimer::new())
                .header_read_timeout(Duration::from_secs(5))
                .keep_alive(false)
                .max_buf_size(16 * 1024);
            let _ = builder
                .serve_connection(TokioIo::new(socket), service)
                .await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn http_reads_the_previous_snapshot_during_a_paused_publication() {
        let root =
            std::env::temp_dir().join(format!("moli-http-publication-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let token = root.join("token");
        std::fs::write(&token, "fixture-archive-token-for-tests").unwrap();
        let installation = "00000000000000000000000000000002";
        let mut archive = Archive::open(&root.join("data"), installation).unwrap();
        archive.receive(1, 1, b"{\"type\":\"energy_hour\",\"seq\":1,\"meter\":\"demo.meter\",\"hour\":0,\"kwh\":1.0,\"cost\":null}\n").unwrap();
        let state = Arc::new(App {
            installation: installation.into(),
            status: RwLock::new(archive.status()),
            snapshot: RwLock::new(archive.snapshot()),
            archive: Mutex::new(archive),
            auth: Auth::from_file(&token).unwrap(),
            jobs: Arc::new(Semaphore::new(1)),
            reads: Arc::new(Semaphore::new(1)),
            fatal: watch::channel(false).0,
            publishing: AtomicBool::new(false),
        });
        // Pause the publisher after admission, before any manifest can change.
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let writer = state.clone();
        let paused = std::thread::spawn(move || {
            let _guard = writer.archive.lock().unwrap();
            ready_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        ready_rx.recv().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(serve_state(state.clone(), listener));
        tokio::time::timeout(Duration::from_secs(2), async {
            while state.jobs.available_permits() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(state.publishing.load(Ordering::Acquire));
        let response = tokio::task::spawn_blocking(move || {
            use std::io::{Read, Write};
            let mut socket = std::net::TcpStream::connect(address).unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let body = r#"{"source":"energy","points":["demo.meter"],"from":0,"to":3600000,"resolution":"raw","max_points":200}"#;
            let request = format!("POST /v1/installations/{installation}/query HTTP/1.1\r\nHost: archive\r\nAuthorization: Bearer fixture-archive-token-for-tests\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            socket.write_all(request.as_bytes()).unwrap();
            let mut response = Vec::new();
            socket.read_to_end(&mut response).unwrap();
            response
        }).await.unwrap();
        release_tx.send(()).unwrap();
        paused.join().unwrap();
        let response = String::from_utf8(response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        let (_, body) = response.split_once("\r\n\r\n").unwrap();
        let result: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(result["published_upto"], 0);
        assert!(result["series"][0]["values"].as_array().unwrap().is_empty());
        tokio::time::timeout(Duration::from_secs(5), async {
            while state.status().published_upto != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        server.abort();
        let _ = server.await;
        drop(state);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn a_failed_background_publication_stops_the_listener() {
        let root = std::env::temp_dir().join(format!("moli-http-fatal-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let token = root.join("token");
        std::fs::write(&token, "fixture-archive-token-for-tests").unwrap();
        let installation = "00000000000000000000000000000002";
        let mut archive = Archive::open(&root.join("data"), installation).unwrap();
        archive.receive(1, 1, b"{\"type\":\"energy_hour\",\"seq\":1,\"meter\":\"demo.meter\",\"hour\":0,\"kwh\":1.0,\"cost\":null}\n").unwrap();
        archive.fail_at(crate::PublicationFailure::PartSynced);
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            serve(
                archive,
                installation.into(),
                "127.0.0.1:0".parse().unwrap(),
                &token,
            ),
        )
        .await
        .unwrap();
        assert_eq!(result.unwrap_err().to_string(), "archive_requires_reopen");
        std::fs::remove_dir_all(root).unwrap();
    }
}
