//! Human weather authorization, same-origin callback and secret-free status.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::{Path, rejection::JsonRejection};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use moli_core::InstanceId;
use moli_netatmo::{Error as DriverError, Gateway};
use moli_runtime::Hub;
use serde::Deserialize;
use serde_json::json;

use crate::{Options, caller::Caller, session::Sessions};

const PREFIX: &str = "/api/integrations/netatmo/";
const SCRIPT_PATH: &str = "/api/integrations/netatmo/callback.js";
const STYLE_PATH: &str = "/api/integrations/netatmo/callback.css";
const SCRIPT: &str = include_str!("netatmo/callback.js");
const STYLE: &str = include_str!("netatmo/callback.css");
const BODY: usize = 8192;

/// An enabled instance, reached from the origin configured by its operator.
#[derive(Clone)]
pub struct Endpoint {
    gateway: Gateway,
    origin: String,
}

impl std::fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NetatmoEndpoint").finish_non_exhaustive()
    }
}

impl Endpoint {
    /// Validates the same origin rules as the native driver's configuration.
    /// This constructs no request to that origin.
    ///
    /// # Errors
    /// Returns `Invalid` if the configured origin is not allowed.
    pub fn new(gateway: Gateway, origin: String) -> Result<Self, DriverError> {
        moli_netatmo::Config {
            public_origin: origin.clone(),
            poll_seconds: 300,
            stale_seconds: 1800,
        }
        .callback(&InstanceId::from("validation"))?;
        Ok(Self { gateway, origin })
    }

    fn same_origin(&self, headers: &HeaderMap) -> bool {
        let Ok(expected) = self.origin.parse::<Uri>() else {
            return false;
        };
        let Some(host) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) else {
            return false;
        };
        if !authority_matches(&expected, host) {
            return false;
        }
        headers.get(header::ORIGIN).is_none_or(|value| {
            value
                .to_str()
                .ok()
                .and_then(|origin| origin.parse::<Uri>().ok())
                .is_some_and(|origin| {
                    origin.scheme_str() == expected.scheme_str()
                        && origin.path() == "/"
                        && origin.query().is_none()
                        && origin
                            .authority()
                            .is_some_and(|a| authority_matches(&expected, a.as_str()))
                })
        })
    }
}

fn authority_matches(expected: &Uri, actual: &str) -> bool {
    if actual.contains('@') {
        return false;
    }
    let (Some(expected_host), Ok(actual)) =
        (expected.authority(), actual.parse::<http::uri::Authority>())
    else {
        return false;
    };
    let default = if expected.scheme_str() == Some("https") {
        443
    } else {
        80
    };
    expected_host.host().eq_ignore_ascii_case(actual.host())
        && expected_host.port_u16().unwrap_or(default) == actual.port_u16().unwrap_or(default)
}

#[derive(Clone)]
struct Gates(Arc<BTreeMap<InstanceId, Endpoint>>);

pub(crate) fn routes(humans: &Arc<Sessions>, options: &Options) -> Router<Hub> {
    Router::new()
        .route(
            "/api/integrations/netatmo",
            crate::upload(get(status), BODY, humans),
        )
        .route(
            "/api/integrations/netatmo/{instance}/authorize",
            crate::upload(post(authorize), BODY, humans),
        )
        .route(
            "/api/integrations/netatmo/{instance}/callback",
            get(callback).merge(crate::upload(post(complete), BODY, humans)),
        )
        .route(SCRIPT_PATH, get(script))
        .route(STYLE_PATH, get(style))
        .layer(Extension(Gates(Arc::new(options.netatmo.clone()))))
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

/// Static, public return resources contain no session or instance data.
pub(crate) fn public_return(method: &Method, path: &str) -> bool {
    if !matches!(*method, Method::GET | Method::HEAD) {
        return false;
    }
    matches!(path, SCRIPT_PATH | STYLE_PATH)
        || path
            .strip_prefix(PREFIX)
            .and_then(|s| s.strip_suffix("/callback"))
            .is_some_and(valid_id)
}

fn error(status: StatusCode, key: &str) -> Response {
    let message = moli_i18n::tr!(&format!("netatmo.http.{key}"));
    (status, Json(json!({"error": message}))).into_response()
}

fn driver_error(failure: DriverError) -> Response {
    let (status, key) = match failure {
        DriverError::Invalid => (StatusCode::UNPROCESSABLE_ENTITY, "invalid"),
        DriverError::Unauthorized => (StatusCode::FORBIDDEN, "refused"),
        DriverError::Expired => (StatusCode::GONE, "expired"),
        DriverError::Busy => (StatusCode::CONFLICT, "busy"),
        DriverError::Ambiguous => (StatusCode::BAD_GATEWAY, "ambiguous"),
        DriverError::Storage => (StatusCode::SERVICE_UNAVAILABLE, "storage"),
        DriverError::RateLimited(_) => (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
        DriverError::BeforeSend | DriverError::Unavailable => {
            (StatusCode::SERVICE_UNAVAILABLE, "unavailable")
        }
    };
    let mut response = error(status, key);
    if let DriverError::RateLimited(seconds) = failure
        && let Ok(value) = HeaderValue::from_str(&seconds.clamp(60, 900).to_string())
    {
        response.headers_mut().insert(header::RETRY_AFTER, value);
    }
    response
}

fn rejected_body(rejection: &JsonRejection) -> Response {
    if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
        error(StatusCode::PAYLOAD_TOO_LARGE, "too_large")
    } else {
        error(StatusCode::UNPROCESSABLE_ENTITY, "invalid")
    }
}

async fn status(Extension(gates): Extension<Gates>) -> Response {
    let instances: Vec<_> = gates
        .0
        .iter()
        .map(|(id, entry)| json!({"instance":id,"status":entry.gateway.status()}))
        .collect();
    Json(json!({"instances":instances})).into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

async fn authorize(
    Extension(gates): Extension<Gates>,
    Extension(humans): Extension<Arc<Sessions>>,
    caller: Caller,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Result<Json<Empty>, JsonRejection>,
) -> Response {
    let Some(binding) = humans.binding(&headers, &caller.key) else {
        return error(StatusCode::FORBIDDEN, "human");
    };
    if let Err(rejection) = body {
        return rejected_body(&rejection);
    }
    if !valid_id(&id) {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "invalid");
    }
    let Some(entry) = gates.0.get(&InstanceId::from(id)) else {
        return error(StatusCode::NOT_FOUND, "absent");
    };
    if !entry.same_origin(&headers) {
        return error(StatusCode::FORBIDDEN, "origin");
    }
    match entry.gateway.authorize(binding).await {
        Ok(authorization) => {
            Json(json!({"url":authorization.url,"expires_in":authorization.expires_in}))
                .into_response()
        }
        Err(failure) => driver_error(failure),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Callback {
    state: String,
    code: String,
}

async fn complete(
    Extension(gates): Extension<Gates>,
    Extension(humans): Extension<Arc<Sessions>>,
    caller: Caller,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Result<Json<Callback>, JsonRejection>,
) -> Response {
    let Some(binding) = humans.binding(&headers, &caller.key) else {
        return error(StatusCode::FORBIDDEN, "human");
    };
    let Json(body) = match body {
        Ok(body) => body,
        Err(rejection) => return rejected_body(&rejection),
    };
    if !valid_id(&id)
        || body.state.len() != 64
        || !body.state.bytes().all(|b| b.is_ascii_hexdigit())
        || body.code.is_empty()
        || body.code.len() > 2048
        || !body.code.bytes().all(|b| b.is_ascii_graphic())
    {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "invalid");
    }
    let Some(entry) = gates.0.get(&InstanceId::from(id)) else {
        return error(StatusCode::NOT_FOUND, "absent");
    };
    if !entry.same_origin(&headers) {
        return error(StatusCode::FORBIDDEN, "origin");
    }
    match entry.gateway.complete(binding, body.state, body.code).await {
        Ok(()) => Json(json!({"connected":true})).into_response(),
        Err(failure) => driver_error(failure),
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

async fn callback(Path(id): Path<String>, uri: Uri) -> Response {
    if !valid_id(&id) {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "invalid");
    }
    // Routes are merged at the root. Revisit this bound if they become nested:
    // the extracted Uri would then have its routing prefix removed.
    if uri.to_string().len() > 8192 {
        return error(StatusCode::URI_TOO_LONG, "too_large");
    }
    let hash =
        STANDARD.encode(ring::digest::digest(&ring::digest::SHA256, SCRIPT.as_bytes()).as_ref());
    let text = |key: &str| escape(&moli_i18n::tr!(&format!("netatmo.callback.{key}")));
    let body = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{title}</title><link rel=\"stylesheet\" href=\"{STYLE_PATH}\"><script defer src=\"{SCRIPT_PATH}\" integrity=\"sha256-{hash}\"></script></head><body data-success=\"{success}\" data-failed=\"{failed}\" data-human=\"{human}\" data-denied=\"{denied}\"><main><h1>{title}</h1><p role=\"status\" id=\"status\">{waiting}</p><a href=\"/\">{back}</a></main></body></html>",
        title = text("title"),
        success = text("success"),
        failed = text("failed"),
        human = text("human"),
        denied = text("denied"),
        waiting = text("waiting"),
        back = text("back"),
    );
    let mut response = Html(body).into_response();
    let headers = response.headers_mut();
    let policy = format!(
        "default-src 'none'; script-src 'self' 'sha256-{hash}' 'strict-dynamic'; connect-src 'self'; style-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"
    );
    if let Ok(value) = HeaderValue::from_str(&policy) {
        headers.insert(header::CONTENT_SECURITY_POLICY, value);
    }
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

async fn script() -> Response {
    (
        [
            (
                header::CONTENT_TYPE,
                "application/javascript; charset=utf-8",
            ),
            (header::CACHE_CONTROL, "no-store"),
            (header::REFERRER_POLICY, "no-referrer"),
        ],
        SCRIPT,
    )
        .into_response()
}

async fn style() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        STYLE,
    )
        .into_response()
}

#[cfg(test)]
mod tests;
