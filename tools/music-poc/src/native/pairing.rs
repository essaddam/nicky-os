//! Opt-in, temporary, read-only YouTube association for the isolated POC.
//! CORS and loopback binding are containment, not production authentication.
//! Credentials are retained only in RAM and are never serialized or logged.

use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use axum::{
    Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, Request, State, rejection::BytesRejection},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::{net::TcpListener, sync::Semaphore};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroize;

use super::youtube::BrowserBootstrap;

const ORIGIN: &str = "https://music.youtube.com";
const LIFETIME: Duration = Duration::from_secs(10 * 60);
const BODY_LIMIT: usize = 17 * 1024;
const COOKIE_LIMIT: usize = 16 * 1024;
const NETWORK_TIMEOUT: Duration = Duration::from_secs(10);
const ATTEMPT_LIMIT: usize = 5;

static ASSOCIATED: AtomicBool = AtomicBool::new(false);
static ASSOCIATION_ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
static SEARCH_ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
static NETWORK: Semaphore = Semaphore::const_new(1);

// Deliberately no Debug, Serialize or public credential accessor.
struct TemporaryCookie(String, Option<BrowserBootstrap>);

impl Drop for TemporaryCookie {
    fn drop(&mut self) {
        self.0.zeroize();
        if let Some(bootstrap) = &mut self.1
            && let Some(visitor_data) = &mut bootstrap.visitor_data
        {
            visitor_data.zeroize();
        }
    }
}

type ProviderFuture = Pin<Box<dyn Future<Output = Result<Value>> + Send>>;

fn native_account(cookie: String, bootstrap: Option<BrowserBootstrap>) -> ProviderFuture {
    Box::pin(super::youtube::account_with_cookie(cookie, bootstrap))
}

fn native_search(
    cookie: String,
    query: String,
    bootstrap: Option<BrowserBootstrap>,
) -> ProviderFuture {
    Box::pin(super::youtube::search_with_cookie(cookie, query, bootstrap))
}

struct Pairing {
    host: String,
    expires: Instant,
    closed: AtomicBool,
    cookie: Mutex<Option<TemporaryCookie>>,
    stop: CancellationToken,
    account: fn(String, Option<BrowserBootstrap>) -> ProviderFuture,
    search: fn(String, String, Option<BrowserBootstrap>) -> ProviderFuture,
}

impl Pairing {
    fn new(port: u16) -> Arc<Self> {
        Arc::new(Self {
            host: format!("127.0.0.1:{port}"),
            expires: Instant::now() + LIFETIME,
            closed: AtomicBool::new(false),
            cookie: Mutex::new(None),
            stop: CancellationToken::new(),
            account: native_account,
            search: native_search,
        })
    }

    fn live(&self) -> bool {
        !self.closed.load(Ordering::Acquire) && Instant::now() < self.expires
    }

    fn clear(&self) {
        self.closed.store(true, Ordering::Release);
        self.stop.cancel();
        *self
            .cookie
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = None;
    }

    fn timeout(&self) -> Duration {
        NETWORK_TIMEOUT.min(self.expires.saturating_duration_since(Instant::now()))
    }
}

fn router(state: Arc<Pairing>) -> Router {
    Router::new()
        .route("/pair/youtube", post(pair).options(preflight))
        .route("/pair/youtube/search", post(search).options(preflight))
        .route("/pair/status", get(status).options(preflight))
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .layer(middleware::from_fn_with_state(state.clone(), boundary))
        .with_state(state)
}

/// Only the caller's explicit opt-in should start this listener.
/// The listener must be bound to IPv4 loopback; no port or credential is logged.
pub async fn serve(listener: TcpListener, cancel: CancellationToken) -> Result<()> {
    let local = listener
        .local_addr()
        .context("cannot inspect pairing listener")?;
    if local.ip() != std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST) {
        bail!("pairing listener must use IPv4 loopback");
    }
    let state = Pairing::new(local.port());
    struct ClearOnDrop(Arc<Pairing>);
    impl Drop for ClearOnDrop {
        fn drop(&mut self) {
            self.0.clear();
        }
    }
    let _clear = ClearOnDrop(state.clone());
    let shutdown = state.stop.clone();
    let server = axum::serve(listener, router(state.clone()))
        .with_graceful_shutdown(async move { shutdown.cancelled().await });
    let server = std::future::IntoFuture::into_future(server);
    tokio::pin!(server);
    tokio::select! {
        result = &mut server => return result.context("pairing listener stopped"),
        _ = cancel.cancelled() => {},
        _ = tokio::time::sleep(LIFETIME) => {},
    }
    state.clear();
    // Request-level cancellation drops upstream credential-bearing futures.
    // Axum's graceful connection tasks still belong to the enclosing runtime.
    if let Ok(result) = tokio::time::timeout(Duration::from_secs(2), &mut server).await {
        result.context("pairing listener shutdown failed")?;
    }
    Ok(())
}

fn one_header(headers: &HeaderMap, name: header::HeaderName, expected: &str) -> bool {
    let mut values = headers.get_all(name).iter();
    matches!(values.next(), Some(value) if value.as_bytes() == expected.as_bytes())
        && values.next().is_none()
}

async fn boundary(State(state): State<Arc<Pairing>>, request: Request, next: Next) -> Response {
    if !one_header(request.headers(), header::HOST, &state.host)
        || !one_header(request.headers(), header::ORIGIN, ORIGIN)
    {
        return reply(StatusCode::FORBIDDEN, json!({"status":"forbidden"}));
    }
    let mut response = if state.live() {
        next.run(request).await
    } else {
        state.clear();
        reply(StatusCode::GONE, json!({"status":"expired"}))
    };
    let headers = response.headers_mut();
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static(ORIGIN),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, OPTIONS"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("Content-Type"),
    );
    headers.insert(
        "access-control-allow-private-network",
        HeaderValue::from_static("true"),
    );
    headers.insert(header::VARY, HeaderValue::from_static("Origin"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

async fn preflight(headers: HeaderMap) -> Response {
    if let Some(method) = headers.get(header::ACCESS_CONTROL_REQUEST_METHOD)
        && !matches!(method.as_bytes(), b"GET" | b"POST" | b"OPTIONS")
    {
        return reply(StatusCode::FORBIDDEN, json!({"status":"forbidden"}));
    }
    if let Some(requested) = headers.get(header::ACCESS_CONTROL_REQUEST_HEADERS) {
        let valid = requested.to_str().ok().is_some_and(|value| {
            value
                .split(',')
                .all(|name| name.trim().eq_ignore_ascii_case("content-type"))
        });
        if !valid {
            return reply(StatusCode::FORBIDDEN, json!({"status":"forbidden"}));
        }
    }
    reply(StatusCode::NO_CONTENT, Value::Null)
}

async fn status(State(state): State<Arc<Pairing>>) -> Response {
    let paired = state
        .cookie
        .lock()
        .ok()
        .is_some_and(|cookie| cookie.is_some());
    reply(
        StatusCode::OK,
        json!({"paired":paired,"native":paired,"persistent":false}),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PairBody {
    cookie: String,
    bootstrap: Option<BrowserBootstrap>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchBody {
    query: String,
}

fn parse_body<T: serde::de::DeserializeOwned>(
    headers: &HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> std::result::Result<T, (StatusCode, &'static str)> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok());
    if !content_type.is_some_and(|value| {
        value
            .split(';')
            .next()
            .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/json"))
    }) {
        return Err((StatusCode::UNSUPPORTED_MEDIA_TYPE, "json_required"));
    }
    let body = body.map_err(|_| (StatusCode::PAYLOAD_TOO_LARGE, "body_rejected"))?;
    serde_json::from_slice(&body).map_err(|_| (StatusCode::BAD_REQUEST, "invalid_body"))
}

fn consume_attempt(counter: &AtomicUsize) -> bool {
    counter
        .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
            (used < ATTEMPT_LIMIT).then_some(used + 1)
        })
        .is_ok()
}

fn authorized_account(value: &Value) -> bool {
    value.get("status").and_then(Value::as_str) == Some("authorized")
        && value.get("authenticated").and_then(Value::as_bool) == Some(true)
}

fn failure_reason(error: &anyhow::Error) -> &'static str {
    for cause in error.chain() {
        let text = cause.to_string();
        if text.contains("HTTP 302") {
            return "consent_redirect";
        }
        if text.contains("authorization refused") {
            return "provider_authorization_refused";
        }
        if text.contains("bootstrap schema changed") {
            return "bootstrap_schema_changed";
        }
        if text.contains("invalid YouTube Music credential")
            || text.contains("lacks a session identifier")
        {
            return "credential_format_rejected";
        }
    }
    "provider_request_failed"
}

async fn pair(
    State(state): State<Arc<Pairing>>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    let request: PairBody = match parse_body(&headers, body) {
        Ok(body) => body,
        Err((status, message)) => return reply(status, json!({"status":message})),
    };
    if request
        .bootstrap
        .as_ref()
        .is_some_and(|bootstrap| !bootstrap.is_valid())
    {
        return reply(
            StatusCode::BAD_REQUEST,
            json!({"status":"invalid_bootstrap"}),
        );
    }
    if request.cookie.trim().is_empty()
        || request.cookie.len() > COOKIE_LIMIT
        || !request
            .cookie
            .bytes()
            .all(|byte| (32..=126).contains(&byte))
    {
        return reply(StatusCode::BAD_REQUEST, json!({"status":"invalid_cookie"}));
    }
    if ASSOCIATED.load(Ordering::Acquire) {
        return reply(StatusCode::CONFLICT, json!({"status":"already_paired"}));
    }
    let Ok(_permit) = NETWORK.try_acquire() else {
        return reply(StatusCode::TOO_MANY_REQUESTS, json!({"status":"busy"}));
    };
    if !consume_attempt(&ASSOCIATION_ATTEMPTS) {
        return reply(
            StatusCode::TOO_MANY_REQUESTS,
            json!({"status":"attempt_limit"}),
        );
    }
    let checked = tokio::select! {
        _ = state.stop.cancelled() => return reply(StatusCode::GONE, json!({"status":"expired"})),
        result = tokio::time::timeout(state.timeout(), (state.account)(request.cookie.clone(), request.bootstrap.clone())) => result,
    };
    let failure = match checked {
        Ok(Ok(value)) if authorized_account(&value) => None,
        Ok(Ok(_)) => Some("account_response_unconfirmed"),
        Ok(Err(error)) => Some(failure_reason(&error)),
        Err(_) => Some("provider_timeout"),
    };
    if let Some(reason) = failure {
        return reply(
            StatusCode::UNAUTHORIZED,
            json!({"status":"authorization_failed","reason":reason}),
        );
    }
    let Ok(mut cookie) = state.cookie.lock() else {
        return reply(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"status":"unavailable"}),
        );
    };
    if !state.live() {
        return reply(StatusCode::GONE, json!({"status":"expired"}));
    }
    if ASSOCIATED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return reply(StatusCode::CONFLICT, json!({"status":"already_paired"}));
    }
    *cookie = Some(TemporaryCookie(request.cookie, request.bootstrap));
    reply(
        StatusCode::OK,
        json!({"paired":true,"native":true,"persistent":false}),
    )
}

async fn search(
    State(state): State<Arc<Pairing>>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    let request: SearchBody = match parse_body(&headers, body) {
        Ok(body) => body,
        Err((status, message)) => return reply(status, json!({"status":message})),
    };
    let query = request.query.trim();
    if !(2..=200).contains(&query.chars().count())
        || query.len() > 800
        || query.chars().any(char::is_control)
    {
        return reply(StatusCode::BAD_REQUEST, json!({"status":"invalid_query"}));
    }
    let cookie = match state.cookie.lock() {
        Ok(cookie) => cookie
            .as_ref()
            .map(|cookie| (cookie.0.clone(), cookie.1.clone())),
        Err(_) => {
            return reply(
                StatusCode::SERVICE_UNAVAILABLE,
                json!({"status":"unavailable"}),
            );
        }
    };
    let Some((cookie, bootstrap)) = cookie else {
        return reply(
            StatusCode::UNAUTHORIZED,
            json!({"status":"association_required"}),
        );
    };
    let Ok(_permit) = NETWORK.try_acquire() else {
        return reply(StatusCode::TOO_MANY_REQUESTS, json!({"status":"busy"}));
    };
    if !consume_attempt(&SEARCH_ATTEMPTS) {
        return reply(
            StatusCode::TOO_MANY_REQUESTS,
            json!({"status":"attempt_limit"}),
        );
    }
    let result = tokio::select! {
        _ = state.stop.cancelled() => return reply(StatusCode::GONE, json!({"status":"expired"})),
        result = tokio::time::timeout(state.timeout(), (state.search)(cookie, query.to_owned(), bootstrap)) => result,
    };
    if !state.live() {
        return reply(StatusCode::GONE, json!({"status":"expired"}));
    }
    let value = match result {
        Ok(Ok(value)) => value,
        _ => return reply(StatusCode::BAD_GATEWAY, json!({"status":"search_failed"})),
    };
    let status = match value.get("status").and_then(Value::as_str) {
        Some("ok") => "ok",
        Some("no_match") => "no_match",
        _ => return reply(StatusCode::BAD_GATEWAY, json!({"status":"search_failed"})),
    };
    let Some(results) = value.get("results").and_then(Value::as_array) else {
        return reply(StatusCode::BAD_GATEWAY, json!({"status":"search_failed"}));
    };
    reply(
        StatusCode::OK,
        json!({"status":status,"result_count":results.len(),"native":true}),
    )
}

fn reply(status: StatusCode, value: Value) -> Response {
    let body = if status == StatusCode::NO_CONTENT {
        Body::empty()
    } else {
        Body::from(value.to_string())
    };
    let mut response = Response::new(body);
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::to_bytes,
        http::{Method, Request as HttpRequest},
    };
    use tower::ServiceExt;

    #[test]
    fn failure_diagnostics_are_fixed_codes_and_never_echo_values() {
        assert_eq!(
            failure_reason(&anyhow::anyhow!(
                "native provider request rejected (HTTP 302)"
            )),
            "consent_redirect"
        );
        assert_eq!(
            failure_reason(&anyhow::anyhow!("native provider authorization refused")),
            "provider_authorization_refused"
        );
        assert_eq!(
            failure_reason(&anyhow::anyhow!("synthetic-cookie-and-url-must-not-escape")),
            "provider_request_failed"
        );
    }

    #[test]
    fn account_confirmation_requires_both_fields_and_limits_are_exact() {
        assert!(authorized_account(
            &json!({"status":"authorized","authenticated":true})
        ));
        for value in [
            json!({"status":"authorized","authenticated":false}),
            json!({"status":"authorization_unconfirmed","authenticated":true}),
            json!({"authenticated":true}),
            json!({"status":"authorized"}),
        ] {
            assert!(!authorized_account(&value));
        }
        let attempts = AtomicUsize::new(0);
        for _ in 0..ATTEMPT_LIMIT {
            assert!(consume_attempt(&attempts));
        }
        assert!(!consume_attempt(&attempts));
        assert_eq!(attempts.load(Ordering::Acquire), ATTEMPT_LIMIT);
    }

    #[test]
    fn cookie_only_json_remains_compatible_without_browser_bootstrap() {
        let request: PairBody =
            serde_json::from_str("{\"cookie\":\"synthetic-only-cookie\"}").unwrap();
        assert_eq!(request.cookie, "synthetic-only-cookie");
        assert!(request.bootstrap.is_none());
    }

    #[tokio::test]
    async fn malformed_browser_bootstrap_is_rejected_before_any_provider_request() {
        for body in [
            "{\"cookie\":\"synthetic-only-cookie\",\"bootstrap\":{}}",
            "{\"cookie\":\"synthetic-only-cookie\",\"bootstrap\":{\"client_version\":\"\"}}",
            "{\"cookie\":\"synthetic-only-cookie\",\"bootstrap\":{\"client_version\":17}}",
            "{\"cookie\":\"synthetic-only-cookie\",\"bootstrap\":{\"client_version\":\"1.20261027.01.00\",\"visitor_data\":[]}}",
            "{\"cookie\":\"synthetic-only-cookie\",\"bootstrap\":{\"client_version\":\"1.20261027.01.00\",\"unknown\":true}}",
        ] {
            let response = send(
                Method::POST,
                "/pair/youtube",
                "127.0.0.1:17990",
                Some(ORIGIN),
                body,
            )
            .await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            let value: Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 1024).await.unwrap())
                    .unwrap();
            assert!(matches!(
                value.get("status").and_then(Value::as_str),
                Some("invalid_body" | "invalid_bootstrap")
            ));
        }
    }

    async fn send(
        method: Method,
        path: &str,
        host: &str,
        origin: Option<&str>,
        body: &str,
    ) -> Response {
        let mut request = HttpRequest::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, host)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(origin) = origin {
            request = request.header(header::ORIGIN, origin);
        }
        router(Pairing::new(17990))
            .oneshot(request.body(Body::from(body.to_owned())).unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn rejects_wrong_or_missing_origin_and_nonliteral_host() {
        for host in [
            "localhost:17990",
            "127.0.0.1:17991",
            "attacker@127.0.0.1:17990",
            "127.0.0.1:17990.example.com",
        ] {
            assert_eq!(
                send(Method::GET, "/pair/status", host, Some(ORIGIN), "")
                    .await
                    .status(),
                StatusCode::FORBIDDEN
            );
        }
        for origin in [
            None,
            Some("null"),
            Some("https://youtube.com"),
            Some("https://music.youtube.com.attacker.invalid"),
        ] {
            let response = send(Method::GET, "/pair/status", "127.0.0.1:17990", origin, "").await;
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            assert!(
                !response
                    .headers()
                    .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            );
        }
    }

    #[tokio::test]
    async fn preflight_has_exact_origin_and_no_credentials_or_wildcard() {
        let response = send(
            Method::OPTIONS,
            "/pair/youtube",
            "127.0.0.1:17990",
            Some(ORIGIN),
            "",
        )
        .await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
            ORIGIN
        );
        assert_eq!(
            response.headers()[header::ACCESS_CONTROL_ALLOW_METHODS],
            "GET, POST, OPTIONS"
        );
        assert_eq!(
            response.headers()[header::ACCESS_CONTROL_ALLOW_HEADERS],
            "Content-Type"
        );
        assert_eq!(
            response.headers()["access-control-allow-private-network"],
            "true"
        );
        assert!(
            !response
                .headers()
                .contains_key(header::ACCESS_CONTROL_ALLOW_CREDENTIALS)
        );
        for (_, value) in response.headers() {
            assert_ne!(value, "*");
        }
    }

    #[tokio::test]
    async fn status_is_private_and_invalid_input_never_reaches_upstream() {
        let response = send(
            Method::GET,
            "/pair/status",
            "127.0.0.1:17990",
            Some(ORIGIN),
            "",
        )
        .await;
        let value: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 512).await.unwrap()).unwrap();
        assert_eq!(
            value,
            json!({"paired":false,"native":false,"persistent":false})
        );
        for body in [
            "{}",
            "{\"cookie\":\"\"}",
            "{\"cookie\":\"a\\nb\"}",
            "{\"cookie\":\"x\",\"unknown\":true}",
        ] {
            assert_eq!(
                send(
                    Method::POST,
                    "/pair/youtube",
                    "127.0.0.1:17990",
                    Some(ORIGIN),
                    body
                )
                .await
                .status(),
                StatusCode::BAD_REQUEST
            );
        }
        let oversized = format!("{{\"cookie\":\"{}\"}}", "x".repeat(BODY_LIMIT));
        assert_eq!(
            send(
                Method::POST,
                "/pair/youtube",
                "127.0.0.1:17990",
                Some(ORIGIN),
                &oversized
            )
            .await
            .status(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
        assert_eq!(
            send(
                Method::POST,
                "/pair/youtube/search",
                "127.0.0.1:17990",
                Some(ORIGIN),
                "{\"query\":\"example\"}"
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn expired_state_clears_credentials_without_serializing_them() {
        let state = Arc::new(Pairing {
            host: "127.0.0.1:17990".to_owned(),
            expires: Instant::now() - Duration::from_secs(1),
            closed: AtomicBool::new(false),
            cookie: Mutex::new(Some(TemporaryCookie("private-test-value".to_owned(), None))),
            stop: CancellationToken::new(),
            account: native_account,
            search: native_search,
        });
        let request = HttpRequest::builder()
            .uri("/pair/status")
            .header(header::HOST, "127.0.0.1:17990")
            .header(header::ORIGIN, ORIGIN)
            .body(Body::empty())
            .unwrap();
        let response = router(state.clone()).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::GONE);
        assert!(state.cookie.lock().unwrap().is_none());
        assert!(state.stop.is_cancelled());
        assert_eq!(
            to_bytes(response.into_body(), 512).await.unwrap().as_ref(),
            b"{\"status\":\"expired\"}"
        );
    }

    fn mock_account(cookie: String, bootstrap: Option<BrowserBootstrap>) -> ProviderFuture {
        Box::pin(async move {
            assert_eq!(cookie, "synthetic-only-cookie");
            let bootstrap = bootstrap.expect("mock account requires browser bootstrap");
            assert_eq!(bootstrap.client_version, "1.20261027.01.00");
            assert_eq!(bootstrap.visitor_data.as_deref(), Some("synthetic-visitor"));
            Ok(
                json!({"status":"authorized","authenticated":true,"account_name":"private-test-account"}),
            )
        })
    }

    fn mock_search(
        cookie: String,
        query: String,
        bootstrap: Option<BrowserBootstrap>,
    ) -> ProviderFuture {
        Box::pin(async move {
            assert_eq!(cookie, "synthetic-only-cookie");
            assert_eq!(query, "synthetic query");
            let bootstrap = bootstrap.expect("mock search requires retained browser bootstrap");
            assert_eq!(bootstrap.client_version, "1.20261027.01.00");
            assert_eq!(bootstrap.visitor_data.as_deref(), Some("synthetic-visitor"));
            Ok(
                json!({"status":"ok","results":[{"title":"private-test-title","id":"private-test-result"}]}),
            )
        })
    }

    async fn mock_request(app: &Router, method: Method, path: &str, body: &str) -> Response {
        let request = HttpRequest::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "127.0.0.1:17990")
            .header(header::ORIGIN, ORIGIN)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_owned()))
            .unwrap();
        app.clone().oneshot(request).await.unwrap()
    }

    async fn response_value(response: Response, expected_status: StatusCode) -> Value {
        assert_eq!(response.status(), expected_status);
        serde_json::from_slice(&to_bytes(response.into_body(), 1024).await.unwrap()).unwrap()
    }

    #[tokio::test]
    async fn complete_mock_association_search_and_clear_never_expose_credentials_or_metadata() {
        // This is the only test committing the process-wide one-shot success.
        // No production globals are reset and no real provider future is built.
        let mut state = Pairing::new(17990);
        let configuration = Arc::get_mut(&mut state).unwrap();
        configuration.account = mock_account;
        configuration.search = mock_search;
        let app = router(state.clone());
        let paired = mock_request(
            &app,
            Method::POST,
            "/pair/youtube",
            "{\"cookie\":\"synthetic-only-cookie\",\"bootstrap\":{\"client_version\":\"1.20261027.01.00\",\"visitor_data\":\"synthetic-visitor\"}}",
        )
        .await;
        assert_eq!(
            response_value(paired, StatusCode::OK).await,
            json!({"paired":true,"native":true,"persistent":false})
        );
        assert!(state.cookie.lock().unwrap().is_some());
        let status = mock_request(&app, Method::GET, "/pair/status", "").await;
        assert_eq!(
            response_value(status, StatusCode::OK).await,
            json!({"paired":true,"native":true,"persistent":false})
        );
        let repeated = mock_request(
            &app,
            Method::POST,
            "/pair/youtube",
            "{\"cookie\":\"synthetic-only-cookie\"}",
        )
        .await;
        assert_eq!(
            response_value(repeated, StatusCode::CONFLICT).await,
            json!({"status":"already_paired"})
        );
        let searched = mock_request(
            &app,
            Method::POST,
            "/pair/youtube/search",
            "{\"query\":\"synthetic query\"}",
        )
        .await;
        assert_eq!(
            response_value(searched, StatusCode::OK).await,
            json!({"status":"ok","result_count":1,"native":true})
        );
        state.clear();
        assert!(state.cookie.lock().unwrap().is_none());
        assert!(state.stop.is_cancelled());
        for path in ["/pair/status", "/pair/youtube/search"] {
            let expired = mock_request(&app, Method::GET, path, "").await;
            assert_eq!(
                response_value(expired, StatusCode::GONE).await,
                json!({"status":"expired"})
            );
        }
    }
}
