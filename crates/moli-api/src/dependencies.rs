//! Executable definitions. Every client edits a draft; only a verified PIN
//! session publishes the exact preview, including affected graph identities.
use crate::{caller::Caller, session::Sessions};
use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use moli_core::{DependencyKind, DependencyRef};
use moli_runtime::{DependencyDefinition, DependencyError, DependencyPreview, Hub};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

pub(crate) fn routes() -> Router<Hub> {
    Router::new()
        .route("/api/dependencies", get(list))
        .route("/api/dependencies/{kind}/{id}", get(read).put(stage))
        .route("/api/dependencies/{kind}/{id}/preview", get(preview))
        .route("/api/dependencies/{kind}/{id}/approve", post(approve))
}
fn reference((kind, id): (DependencyKind, String)) -> DependencyRef {
    DependencyRef {
        kind,
        id: id.into(),
    }
}
fn failure(error: DependencyError) -> Response {
    let code = match error {
        DependencyError::Unknown => StatusCode::NOT_FOUND,
        DependencyError::Definition => StatusCode::UNPROCESSABLE_ENTITY,
        DependencyError::Human => StatusCode::FORBIDDEN,
        DependencyError::Conflict => StatusCode::CONFLICT,
        DependencyError::Capacity => StatusCode::INSUFFICIENT_STORAGE,
        DependencyError::Corrupt
        | DependencyError::Uncertain
        | DependencyError::Consumers
        | DependencyError::Projection => StatusCode::SERVICE_UNAVAILABLE,
        DependencyError::Storage => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (code, Json(json!({"error": error.to_string()}))).into_response()
}
pub(crate) async fn list(State(hub): State<Hub>) -> Response {
    match hub.dependencies().await {
        Ok(views) => Json(json!({"dependencies": views})).into_response(),
        Err(e) => failure(e),
    }
}
async fn read(State(hub): State<Hub>, Path(path): Path<(DependencyKind, String)>) -> Response {
    match hub.dependency(&reference(path)).await {
        Ok(view) => Json(view).into_response(),
        Err(e) => failure(e),
    }
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Draft {
    expected: u64,
    definition: DependencyDefinition,
}
async fn stage(
    State(hub): State<Hub>,
    caller: Caller,
    Extension(humans): Extension<Arc<Sessions>>,
    Path(path): Path<(DependencyKind, String)>,
    headers: HeaderMap,
    Json(draft): Json<Draft>,
) -> Response {
    match hub
        .stage_dependency(
            reference(path),
            draft.expected,
            draft.definition,
            crate::rest::origin(&hub, &headers, &humans, &caller),
            Some(caller.describe()),
        )
        .await
    {
        Ok(view) => Json(view).into_response(),
        Err(e) => failure(e),
    }
}
async fn preview(State(hub): State<Hub>, Path(path): Path<(DependencyKind, String)>) -> Response {
    match hub.preview_dependency(&reference(path)).await {
        Ok(preview) => Json(preview).into_response(),
        Err(e) => failure(e),
    }
}
async fn approve(
    State(hub): State<Hub>,
    caller: Caller,
    Extension(humans): Extension<Arc<Sessions>>,
    Path(path): Path<(DependencyKind, String)>,
    headers: HeaderMap,
    Json(preview): Json<DependencyPreview>,
) -> Response {
    if preview.reference != reference(path) {
        return failure(DependencyError::Conflict);
    }
    let human = humans.is_human(&headers, &caller.key);
    match hub
        .publish_dependency(
            preview,
            human,
            crate::rest::origin(&hub, &headers, &humans, &caller),
            Some(caller.describe()),
        )
        .await
    {
        Ok(view) => Json(view).into_response(),
        Err(e) => failure(e),
    }
}
