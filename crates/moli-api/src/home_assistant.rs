//! Household-only view and human-session mutations for Home Assistant migration.

use std::{collections::BTreeMap, sync::Arc};

use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, Path, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use moli_home_assistant::Gateway;
use moli_migration::{BindingKey, MigrationPlan};
use moli_runtime::Hub;
use serde::Deserialize;
use serde_json::json;

use crate::{caller::Caller, session::Sessions};

const BODY_LIMIT: usize = 8 * 1024;
type Humans = Extension<Arc<Sessions>>;

#[derive(Clone)]
struct Gate {
    instances: Arc<BTreeMap<String, Gateway>>,
    demo: bool,
}

pub(crate) fn routes(instances: BTreeMap<String, Gateway>, demo: bool) -> Router<Hub> {
    Router::new()
        .route("/api/integrations/home-assistant/{instance}", get(snapshot))
        .route(
            "/api/integrations/home-assistant/{instance}/prepare",
            post(prepare).layer(DefaultBodyLimit::max(BODY_LIMIT)),
        )
        .route(
            "/api/integrations/home-assistant/{instance}/advance",
            post(advance).layer(DefaultBodyLimit::max(BODY_LIMIT)),
        )
        .route(
            "/api/integrations/home-assistant/{instance}/reconcile",
            post(reconcile).layer(DefaultBodyLimit::max(BODY_LIMIT)),
        )
        .route(
            "/api/integrations/home-assistant/{instance}/reload",
            post(reload).layer(DefaultBodyLimit::max(BODY_LIMIT)),
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
        (status, Json(json!({"code": code, "error": message}))).into_response()
    }
}

fn problem(status: StatusCode, code: &'static str, message: String) -> Problem {
    Problem(status, code, message)
}

fn household(caller: &Caller) -> bool {
    caller.is_household()
        && !caller
            .person
            .as_ref()
            .is_some_and(crate::people::PersonRef::is_guest)
}

fn lookup<'a>(gate: &'a Gate, instance: &str) -> Result<&'a Gateway, Problem> {
    gate.instances.get(instance).ok_or_else(|| {
        problem(
            StatusCode::NOT_FOUND,
            "unknown_instance",
            moli_i18n::tr!("home_assistant.api.unknown"),
        )
    })
}

fn human_write(
    gate: &Gate,
    humans: &Sessions,
    caller: &Caller,
    headers: &HeaderMap,
    instance: &str,
    operation: &str,
) -> Result<(), Problem> {
    if gate.demo || !household(caller) || !humans.is_human(headers, &caller.key) {
        tracing::info!(actor=%caller.describe(), instance, operation, "Home Assistant migration denied");
        return Err(problem(
            StatusCode::FORBIDDEN,
            if gate.demo { "demo" } else { "human_required" },
            moli_i18n::tr!(if gate.demo {
                "home_assistant.api.demo"
            } else {
                "home_assistant.api.human"
            }),
        ));
    }
    Ok(())
}

fn bridge_problem(
    instance: &str,
    operation: &str,
    error: &moli_home_assistant::BridgeError,
) -> Problem {
    let code = error.code();
    let status = match code {
        "conflict" | "busy" | "reconcile_required" => StatusCode::CONFLICT,
        "unsupported" | "invalid" => StatusCode::UNPROCESSABLE_ENTITY,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    };
    let key = match code {
        "unavailable" => "home_assistant.api.unavailable",
        "conflict" => "home_assistant.api.conflict",
        "busy" => "home_assistant.api.busy",
        "unsupported" => "home_assistant.api.unsupported",
        "reconcile_required" => "home_assistant.api.reconcile",
        _ => "home_assistant.api.invalid",
    };
    tracing::info!(
        instance,
        operation,
        code,
        status = status.as_u16(),
        "Home Assistant migration operation"
    );
    problem(status, code, moli_i18n::tr!(key))
}

fn invalid_body(caller: &Caller, instance: &str, operation: &str) -> Problem {
    tracing::info!(actor=%caller.describe(), instance, operation, "Home Assistant migration request rejected");
    problem(
        StatusCode::UNPROCESSABLE_ENTITY,
        "invalid_request",
        moli_i18n::tr!("home_assistant.api.body"),
    )
}

fn decode_body<T: serde::de::DeserializeOwned>(
    payload: Result<Json<T>, JsonRejection>,
    caller: &Caller,
    instance: &str,
    operation: &str,
) -> Result<T, Problem> {
    payload.map(|Json(value)| value).map_err(|error| {
        let mut problem = invalid_body(caller, instance, operation);
        if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
            problem.0 = StatusCode::PAYLOAD_TOO_LARGE;
        }
        problem
    })
}

#[tracing::instrument(skip_all)]
async fn snapshot(
    Extension(gate): Extension<Gate>,
    caller: Caller,
    Path(instance): Path<String>,
) -> Response {
    if !household(&caller) {
        return problem(
            StatusCode::FORBIDDEN,
            "household_required",
            moli_i18n::tr!("home_assistant.api.human"),
        )
        .into_response();
    }
    match lookup(&gate, &instance) {
        Ok(gateway) => Json(gateway.snapshot().await).into_response(),
        Err(error) => error.into_response(),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Prepare {
    plan: MigrationPlan,
    expected_revision: u64,
    operation_id: String,
    plan_hash: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Advance {
    binding: BindingKey,
    expected_revision: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reconcile {
    binding: BindingKey,
}

#[tracing::instrument(skip_all)]
async fn prepare(
    Extension(gate): Extension<Gate>,
    Extension(humans): Humans,
    caller: Caller,
    headers: HeaderMap,
    Path(instance): Path<String>,
    payload: Result<Json<Prepare>, JsonRejection>,
) -> Response {
    if let Err(error) = human_write(&gate, &humans, &caller, &headers, &instance, "prepare") {
        return error.into_response();
    }
    let body = match decode_body(payload, &caller, &instance, "prepare") {
        Ok(body) => body,
        Err(error) => return error.into_response(),
    };
    let gateway = match lookup(&gate, &instance) {
        Ok(gateway) => gateway,
        Err(error) => return error.into_response(),
    };
    match gateway
        .prepare(
            body.plan,
            body.expected_revision,
            &body.operation_id,
            &body.plan_hash,
        )
        .await
    {
        Ok(binding) => (StatusCode::ACCEPTED, Json(json!({"binding": binding}))).into_response(),
        Err(error) => bridge_problem(&instance, "prepare", &error).into_response(),
    }
}

#[tracing::instrument(skip_all)]
async fn advance(
    Extension(gate): Extension<Gate>,
    Extension(humans): Humans,
    caller: Caller,
    headers: HeaderMap,
    Path(instance): Path<String>,
    payload: Result<Json<Advance>, JsonRejection>,
) -> Response {
    if let Err(error) = human_write(&gate, &humans, &caller, &headers, &instance, "advance") {
        return error.into_response();
    }
    let body = match decode_body(payload, &caller, &instance, "advance") {
        Ok(body) => body,
        Err(error) => return error.into_response(),
    };
    let gateway = match lookup(&gate, &instance) {
        Ok(gateway) => gateway,
        Err(error) => return error.into_response(),
    };
    match gateway
        .request_step(&body.binding, body.expected_revision)
        .await
    {
        Ok(binding) => (StatusCode::ACCEPTED, Json(json!({"binding": binding}))).into_response(),
        Err(error) => bridge_problem(&instance, "advance", &error).into_response(),
    }
}

#[tracing::instrument(skip_all)]
async fn reconcile(
    Extension(gate): Extension<Gate>,
    Extension(humans): Humans,
    caller: Caller,
    headers: HeaderMap,
    Path(instance): Path<String>,
    payload: Result<Json<Reconcile>, JsonRejection>,
) -> Response {
    if let Err(error) = human_write(&gate, &humans, &caller, &headers, &instance, "reconcile") {
        return error.into_response();
    }
    let body = match decode_body(payload, &caller, &instance, "reconcile") {
        Ok(body) => body,
        Err(error) => return error.into_response(),
    };
    let gateway = match lookup(&gate, &instance) {
        Ok(gateway) => gateway,
        Err(error) => return error.into_response(),
    };
    match gateway.request_reconcile(&body.binding).await {
        Ok(challenge) => {
            (StatusCode::ACCEPTED, Json(json!({"challenge": challenge}))).into_response()
        }
        Err(error) => bridge_problem(&instance, "reconcile", &error).into_response(),
    }
}

#[tracing::instrument(skip_all)]
async fn reload(
    Extension(gate): Extension<Gate>,
    Extension(humans): Humans,
    caller: Caller,
    headers: HeaderMap,
    Path(instance): Path<String>,
) -> Response {
    if let Err(error) = human_write(&gate, &humans, &caller, &headers, &instance, "reload") {
        return error.into_response();
    }
    let gateway = match lookup(&gate, &instance) {
        Ok(gateway) => gateway,
        Err(error) => return error.into_response(),
    };
    match gateway.reload().await {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(error) => bridge_problem(&instance, "reload", &error).into_response(),
    }
}
