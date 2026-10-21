//! Explicit Cast activation; discovery does not grant command authority.
use crate::{caller::Caller, session::Sessions};
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, Path, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use moli_cast::gateway::{Error, Gateway, View};
use moli_runtime::Hub;
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
static NEXT: AtomicU64 = AtomicU64::new(1);
fn operation() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}
#[derive(Clone)]
struct Gate {
    instances: Arc<BTreeMap<String, Gateway>>,
    demo: bool,
}
pub(crate) fn routes(instances: BTreeMap<String, Gateway>, demo: bool) -> Router<Hub> {
    Router::new()
        .route("/api/integrations/cast/{instance}", get(state))
        .route(
            "/api/integrations/cast/{instance}/discovery",
            post(discover),
        )
        .route(
            "/api/integrations/cast/{instance}/activation",
            put(approve).delete(revoke),
        )
        .layer(DefaultBodyLimit::max(1024))
        .layer(Extension(Gate {
            instances: Arc::new(instances),
            demo,
        }))
}
struct Problem(StatusCode, &'static str);
impl IntoResponse for Problem {
    fn into_response(self) -> Response {
        (
            self.0,
            Json(json!({"code":self.1,"error":moli_i18n::tr(self.1)})),
        )
            .into_response()
    }
}
fn household(caller: &Caller) -> bool {
    caller.is_household()
        && !caller
            .person
            .as_ref()
            .is_some_and(crate::people::PersonRef::is_guest)
}
fn lookup<'a>(gate: &'a Gate, id: &str) -> Result<&'a Gateway, Problem> {
    gate.instances
        .get(id)
        .ok_or(Problem(StatusCode::NOT_FOUND, "serveur.cast.unknown"))
}
fn permitted<'a>(
    gate: &'a Gate,
    humans: &Sessions,
    caller: &Caller,
    headers: &HeaderMap,
    id: &str,
) -> Result<&'a Gateway, Problem> {
    if gate.demo || !household(caller) || !humans.is_human(headers, &caller.key) {
        return Err(Problem(StatusCode::FORBIDDEN, "serveur.cast.human"));
    }
    lookup(gate, id)
}
fn body<T>(payload: Result<Json<T>, JsonRejection>) -> Result<T, Problem> {
    payload.map(|Json(body)| body).map_err(|error| {
        Problem(
            if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
                StatusCode::PAYLOAD_TOO_LARGE
            } else {
                StatusCode::UNPROCESSABLE_ENTITY
            },
            "serveur.cast.body",
        )
    })
}
fn text(error: Error) -> String {
    moli_i18n::tr(match error {
        Error::Unavailable => "serveur.cast.unavailable",
        Error::Busy => "serveur.cast.busy",
        Error::Stale => "serveur.cast.stale",
        Error::Invalid => "serveur.cast.invalid",
        Error::Network => "serveur.cast.network",
        Error::Vault => "serveur.cast.vault",
        Error::OwnedElsewhere => "serveur.cast.owned",
    })
}
fn public(view: View) -> Json<serde_json::Value> {
    let message = view.error.map(text);
    let mut view = serde_json::to_value(view).expect("public activation state serializes");
    view["message"] = json!(message);
    Json(view)
}
fn answer(result: Result<View, Error>) -> Response {
    match result {
        Ok(view) => public(view).into_response(),
        Err(error) => {
            let status = match error {
                Error::Unavailable | Error::Vault => StatusCode::SERVICE_UNAVAILABLE,
                Error::Busy | Error::Stale | Error::OwnedElsewhere => StatusCode::CONFLICT,
                Error::Invalid => StatusCode::UNPROCESSABLE_ENTITY,
                Error::Network => StatusCode::BAD_GATEWAY,
            };
            (status, Json(json!({"code":error,"error":text(error)}))).into_response()
        }
    }
}
fn audit(caller: &Caller, id: &str, action: &str, response: Response) -> Response {
    tracing::info!(actor=%caller.describe(),instance=id,action,status=response.status().as_u16(),"Cast activation");
    response
}
async fn state(
    Extension(gate): Extension<Gate>,
    caller: Caller,
    Path(id): Path<String>,
) -> Response {
    if !household(&caller) {
        return Problem(StatusCode::FORBIDDEN, "serveur.cast.human").into_response();
    }
    match lookup(&gate, &id) {
        Ok(gateway) => public(gateway.state()).into_response(),
        Err(error) => error.into_response(),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Discover {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Approve {
    generation: u64,
    candidate: String,
    exclusive: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Revoke {
    generation: u64,
}
#[tracing::instrument(skip_all,fields(operation_id=operation()))]
async fn discover(
    Extension(gate): Extension<Gate>,
    Extension(humans): Extension<Arc<Sessions>>,
    caller: Caller,
    headers: HeaderMap,
    Path(id): Path<String>,
    payload: Result<Json<Discover>, JsonRejection>,
) -> Response {
    let response = match permitted(&gate, &humans, &caller, &headers, &id) {
        Err(error) => error.into_response(),
        Ok(gateway) => match body(payload) {
            Err(error) => error.into_response(),
            Ok(_) => answer(gateway.discover().await),
        },
    };
    audit(&caller, &id, "discover", response)
}
#[tracing::instrument(skip_all,fields(operation_id=operation()))]
async fn approve(
    Extension(gate): Extension<Gate>,
    Extension(humans): Extension<Arc<Sessions>>,
    caller: Caller,
    headers: HeaderMap,
    Path(id): Path<String>,
    payload: Result<Json<Approve>, JsonRejection>,
) -> Response {
    let response = match permitted(&gate, &humans, &caller, &headers, &id) {
        Err(error) => error.into_response(),
        Ok(gateway) => match body(payload) {
            Err(error) => error.into_response(),
            Ok(payload) => answer(
                gateway
                    .approve(payload.generation, payload.candidate, payload.exclusive)
                    .await,
            ),
        },
    };
    audit(&caller, &id, "approve", response)
}
#[tracing::instrument(skip_all,fields(operation_id=operation()))]
async fn revoke(
    Extension(gate): Extension<Gate>,
    Extension(humans): Extension<Arc<Sessions>>,
    caller: Caller,
    headers: HeaderMap,
    Path(id): Path<String>,
    payload: Result<Json<Revoke>, JsonRejection>,
) -> Response {
    let response = match permitted(&gate, &humans, &caller, &headers, &id) {
        Err(error) => error.into_response(),
        Ok(gateway) => match body(payload) {
            Err(error) => error.into_response(),
            Ok(payload) => answer(gateway.revoke(payload.generation).await),
        },
    };
    audit(&caller, &id, "revoke", response)
}
