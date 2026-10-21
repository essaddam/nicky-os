//! Scene runs keep the caller's origin. Approval of definitions uses dependencies.
use crate::{caller::Caller, session::Sessions};
use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use moli_runtime::{Hub, SceneError, ScenePreview};
use serde_json::json;
use std::sync::Arc;
pub(crate) fn routes() -> Router<Hub> {
    Router::new()
        .route("/api/scenes/{id}/preview", get(preview))
        .route("/api/scenes/{id}/run", post(run))
        .route("/api/scene-runs/{id}", get(read))
        .route("/api/scene-runs/{id}/cancel", post(cancel))
}
fn failure(error: SceneError) -> Response {
    let code = match error {
        SceneError::Unknown => StatusCode::NOT_FOUND,
        SceneError::Definition => StatusCode::UNPROCESSABLE_ENTITY,
        SceneError::Conflict => StatusCode::CONFLICT,
        SceneError::Busy => StatusCode::TOO_MANY_REQUESTS,
        SceneError::Storage => StatusCode::SERVICE_UNAVAILABLE,
    };
    (code, Json(json!({"error":error.to_string()}))).into_response()
}
async fn preview(State(hub): State<Hub>, Path(id): Path<String>) -> Response {
    match hub.preview_scene(&id.into()).await {
        Ok(p) => Json(p).into_response(),
        Err(e) => failure(e),
    }
}
async fn run(
    State(hub): State<Hub>,
    caller: Caller,
    Extension(humans): Extension<Arc<Sessions>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(preview): Json<ScenePreview>,
) -> Response {
    if preview.id.as_str() != id {
        return failure(SceneError::Conflict);
    }
    match hub
        .start_scene(
            preview,
            crate::rest::origin(&hub, &headers, &humans, &caller),
            Some(caller.describe()),
            None,
        )
        .await
    {
        Ok(run) => (StatusCode::ACCEPTED, Json(run)).into_response(),
        Err(e) => failure(e),
    }
}
async fn read(State(hub): State<Hub>, Path(id): Path<String>) -> Response {
    match hub.scene_run(&id) {
        Some(r) => Json(r).into_response(),
        None => failure(SceneError::Unknown),
    }
}
async fn cancel(
    State(hub): State<Hub>,
    caller: Caller,
    Extension(humans): Extension<Arc<Sessions>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    match hub.cancel_scene(
        &id,
        crate::rest::origin(&hub, &headers, &humans, &caller),
        Some(caller.describe()),
    ) {
        Ok(()) => Json(json!({"cancelled": true})).into_response(),
        Err(e) => failure(e),
    }
}
