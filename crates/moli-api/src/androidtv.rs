//! Human Android TV pairing. Request bodies and TLS errors are never logged.
use crate::{caller::Caller, session::Sessions};
use axum::extract::{DefaultBodyLimit, Path, rejection::JsonRejection};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Json, Router};
use moli_androidtv_driver::pairing::{Error, PairingGateway, View};
use moli_runtime::Hub;
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_OPERATION: AtomicU64 = AtomicU64::new(1);
fn next_operation_id() -> u64 {
    NEXT_OPERATION.fetch_add(1, Ordering::Relaxed)
}

#[derive(Clone)]
struct Gate {
    instances: Arc<BTreeMap<String, PairingGateway>>,
    demo: bool,
}

pub(crate) fn routes(instances: BTreeMap<String, PairingGateway>, demo: bool) -> Router<Hub> {
    Router::new()
        .route(
            "/api/integrations/androidtv/{instance}/pairing",
            get(state)
                .post(start)
                .put(submit)
                .delete(cancel)
                .layer(DefaultBodyLimit::max(1024)),
        )
        .layer(Extension(Gate {
            instances: Arc::new(instances),
            demo,
        }))
}
struct Problem(StatusCode, &'static str, String);
impl IntoResponse for Problem {
    fn into_response(self) -> Response {
        let Self(status, code, message) = self;
        (status, Json(json!({"code":code,"error":message}))).into_response()
    }
}
fn problem(status: StatusCode, code: &'static str, message: String) -> Problem {
    Problem(status, code, message)
}
fn text(error: Error) -> String {
    moli_i18n::tr(match error {
        Error::Unavailable => "serveur.androidtv.unavailable",
        Error::Busy => "serveur.androidtv.busy",
        Error::Stale => "serveur.androidtv.stale",
        Error::InvalidCode => "serveur.androidtv.invalid_code",
        Error::Refused => "serveur.androidtv.refused",
        Error::Expired => "serveur.androidtv.expired",
        Error::Network => "serveur.androidtv.network",
        Error::Vault => "serveur.androidtv.vault",
    })
}
fn public(view: View) -> Json<serde_json::Value> {
    let message = view.error.map(text);
    let mut body = serde_json::to_value(view).expect("public pairing state is serializable");
    body["message"] = json!(message);
    Json(body)
}
fn household(caller: &Caller) -> bool {
    caller.is_household()
        && !caller
            .person
            .as_ref()
            .is_some_and(crate::people::PersonRef::is_guest)
}
fn lookup<'a>(gate: &'a Gate, instance: &str) -> Result<&'a PairingGateway, Problem> {
    gate.instances.get(instance).ok_or_else(|| {
        problem(
            StatusCode::NOT_FOUND,
            "unknown_instance",
            moli_i18n::tr!("serveur.androidtv.unknown"),
        )
    })
}
async fn state(
    Extension(gate): Extension<Gate>,
    caller: Caller,
    Path(instance): Path<String>,
) -> Response {
    if !household(&caller) {
        return problem(
            StatusCode::FORBIDDEN,
            "human_required",
            moli_i18n::tr!("serveur.androidtv.human"),
        )
        .into_response();
    }
    match lookup(&gate, &instance) {
        Ok(gateway) => public(gateway.state()).into_response(),
        Err(error) => error.into_response(),
    }
}
fn audit(caller: &Caller, instance: &str, operation: &str, status: StatusCode) {
    tracing::info!(actor=%caller.describe(),instance,operation,status=status.as_u16(),"Android TV pairing");
}
fn enabled<'a>(
    gate: &'a Gate,
    humans: &Sessions,
    caller: &Caller,
    headers: &HeaderMap,
    instance: &str,
    operation: &str,
) -> Result<&'a PairingGateway, Problem> {
    if gate.demo || !household(caller) || !humans.is_human(headers, &caller.key) {
        audit(caller, instance, operation, StatusCode::FORBIDDEN);
        return Err(problem(
            StatusCode::FORBIDDEN,
            "human_required",
            moli_i18n::tr!("serveur.androidtv.human"),
        ));
    }
    lookup(gate, instance)
        .inspect_err(|_| audit(caller, instance, operation, StatusCode::NOT_FOUND))
}
fn body<T>(
    payload: Result<Json<T>, JsonRejection>,
    caller: &Caller,
    instance: &str,
    operation: &str,
) -> Result<T, Problem> {
    payload.map(|Json(value)| value).map_err(|error| {
        let status = if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
            StatusCode::PAYLOAD_TOO_LARGE
        } else {
            StatusCode::UNPROCESSABLE_ENTITY
        };
        audit(caller, instance, operation, status);
        problem(
            status,
            "invalid_request",
            moli_i18n::tr!("serveur.androidtv.body"),
        )
    })
}
fn answer(
    caller: &Caller,
    instance: &str,
    operation: &str,
    result: Result<View, Error>,
    success: StatusCode,
) -> Response {
    match result {
        Ok(view) => {
            audit(caller, instance, operation, success);
            if success == StatusCode::NO_CONTENT {
                success.into_response()
            } else {
                (success, public(view)).into_response()
            }
        }
        Err(error) => {
            let status = match error {
                Error::Busy | Error::Stale => StatusCode::CONFLICT,
                Error::InvalidCode | Error::Refused => StatusCode::UNPROCESSABLE_ENTITY,
                Error::Expired => StatusCode::GATEWAY_TIMEOUT,
                Error::Network => StatusCode::BAD_GATEWAY,
                Error::Unavailable | Error::Vault => StatusCode::SERVICE_UNAVAILABLE,
            };
            audit(caller, instance, operation, status);
            (status, Json(json!({"code":error,"error":text(error)}))).into_response()
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Start {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Submit {
    generation: u64,
    code: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cancel {
    generation: u64,
}

#[tracing::instrument(skip_all, fields(operation_id=next_operation_id()))]
async fn start(
    Extension(gate): Extension<Gate>,
    Extension(humans): Extension<Arc<Sessions>>,
    caller: Caller,
    headers: HeaderMap,
    Path(instance): Path<String>,
    payload: Result<Json<Start>, JsonRejection>,
) -> Response {
    let gateway = match enabled(&gate, &humans, &caller, &headers, &instance, "start") {
        Ok(gateway) => gateway,
        Err(error) => return error.into_response(),
    };
    if let Err(error) = body(payload, &caller, &instance, "start") {
        return error.into_response();
    }
    answer(
        &caller,
        &instance,
        "start",
        gateway.start().await,
        StatusCode::ACCEPTED,
    )
}
#[tracing::instrument(skip_all, fields(operation_id=next_operation_id()))]
async fn submit(
    Extension(gate): Extension<Gate>,
    Extension(humans): Extension<Arc<Sessions>>,
    caller: Caller,
    headers: HeaderMap,
    Path(instance): Path<String>,
    payload: Result<Json<Submit>, JsonRejection>,
) -> Response {
    let gateway = match enabled(&gate, &humans, &caller, &headers, &instance, "submit") {
        Ok(gateway) => gateway,
        Err(error) => return error.into_response(),
    };
    let payload = match body(payload, &caller, &instance, "submit") {
        Ok(payload) => payload,
        Err(error) => return error.into_response(),
    };
    answer(
        &caller,
        &instance,
        "submit",
        gateway.submit(payload.generation, payload.code).await,
        StatusCode::OK,
    )
}
#[tracing::instrument(skip_all, fields(operation_id=next_operation_id()))]
async fn cancel(
    Extension(gate): Extension<Gate>,
    Extension(humans): Extension<Arc<Sessions>>,
    caller: Caller,
    headers: HeaderMap,
    Path(instance): Path<String>,
    payload: Result<Json<Cancel>, JsonRejection>,
) -> Response {
    let gateway = match enabled(&gate, &humans, &caller, &headers, &instance, "cancel") {
        Ok(gateway) => gateway,
        Err(error) => return error.into_response(),
    };
    let payload = match body(payload, &caller, &instance, "cancel") {
        Ok(payload) => payload,
        Err(error) => return error.into_response(),
    };
    answer(
        &caller,
        &instance,
        "cancel",
        gateway.cancel(payload.generation).await,
        StatusCode::NO_CONTENT,
    )
}
