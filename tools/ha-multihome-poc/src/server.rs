//! Read-only snapshots and explicitly scoped simulation actions; no device executor.
use crate::{
    config::{self, Topology},
    engine::{Action, Engine},
};
use anyhow::Result;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{path::Path, sync::Arc};
use tokio::{net::TcpListener, sync::Mutex};
use tokio_util::sync::CancellationToken;
#[derive(Clone)]
struct App {
    engine: Arc<Mutex<Engine>>,
    snapshot: Arc<std::sync::Mutex<Value>>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Configure {
    topology: Topology,
}
fn response(code: StatusCode, value: Value) -> Response {
    (
        code,
        [
            ("cache-control", "no-store"),
            ("x-content-type-options", "nosniff"),
        ],
        Json(value),
    )
        .into_response()
}
fn allowed(headers: &HeaderMap) -> bool {
    let host = headers
        .get("host")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if !host.starts_with("127.0.0.1:") && !host.starts_with("localhost:") {
        return false;
    }
    match headers.get("origin") {
        None => true,
        Some(origin) => origin
            .to_str()
            .ok()
            .zip(headers.get("host").and_then(|h| h.to_str().ok()))
            .is_some_and(|(origin, host)| {
                origin == format!("http://{host}") || origin == format!("https://{host}")
            }),
    }
}
async fn status(State(app): State<App>) -> Response {
    response(
        StatusCode::OK,
        app.snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone(),
    )
}
async fn configure(
    State(app): State<App>,
    headers: HeaderMap,
    Json(body): Json<Configure>,
) -> Response {
    if !allowed(&headers) {
        return response(StatusCode::FORBIDDEN, json!({"error":"origin"}));
    }
    let Ok(mut engine) =
        tokio::time::timeout(std::time::Duration::from_secs(3), app.engine.lock()).await
    else {
        return response(StatusCode::CONFLICT, json!({"error":"busy"}));
    };
    let result = engine.configure(body.topology).await;
    let snapshot = engine.status();
    *app.snapshot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = snapshot.clone();
    match result {
        Ok(()) => response(StatusCode::OK, snapshot),
        Err(_) => response(
            StatusCode::BAD_REQUEST,
            json!({"error":"configuration_refused"}),
        ),
    }
}

async fn action(State(app): State<App>, headers: HeaderMap, Json(body): Json<Action>) -> Response {
    if !allowed(&headers) {
        return response(StatusCode::FORBIDDEN, json!({"error":"origin"}));
    }
    let Ok(mut engine) =
        tokio::time::timeout(std::time::Duration::from_secs(3), app.engine.lock()).await
    else {
        return response(StatusCode::CONFLICT, json!({"error":"busy"}));
    };
    match engine.action(body).await {
        Ok(()) => {
            let status = engine.status();
            *app.snapshot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = status.clone();
            response(StatusCode::OK, status)
        }
        Err(_) => response(StatusCode::BAD_REQUEST, json!({"error":"action_refused"})),
    }
}
async fn index() -> Response {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    match tokio::fs::read(root.join("docs/proposition/analyse/poc-ha-multimaison.html")).await {
        Ok(bytes) => (
            [
                ("content-type", "text/html; charset=utf-8"),
                ("cache-control", "no-store"),
                ("x-content-type-options", "nosniff"),
            ],
            bytes,
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}
pub async fn serve(mut engine: Engine, port: u16) -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", port)).await?;
    engine.configure(config::preset("cross")?).await?;
    let snapshot = Arc::new(std::sync::Mutex::new(engine.status()));
    let app = App {
        engine: Arc::new(Mutex::new(engine)),
        snapshot,
    };
    let cancel = CancellationToken::new();
    let stop = cancel.clone();
    let state = app.clone();
    let background = tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(700));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {()=stop.cancelled()=>break,_=tick.tick()=>{if let Ok(mut engine)=state.engine.try_lock(){if let Err(e)=engine.tick().await{engine.last_error=Some(e.to_string());}*state.snapshot.lock().unwrap_or_else(std::sync::PoisonError::into_inner)=engine.status();}}}
        }
    });
    let routes = Router::new()
        .route("/", get(index))
        .route("/api/status", get(status))
        .route("/api/configure", post(configure))
        .route("/api/action", post(action))
        .layer(DefaultBodyLimit::max(16384))
        .with_state(app.clone());

    println!(
        "HA_READY {}",
        json!({"port":port,"mode":"simulation","physical_commands":0})
    );
    let result = axum::serve(listener, routes)
        .with_graceful_shutdown(async {
            shutdown_signal().await;
        })
        .await;
    cancel.cancel();
    background.abort();
    let _ = background.await;
    app.engine.lock().await.shutdown().await;
    result?;
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        if let Ok(mut term) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            tokio::select! {_ = tokio::signal::ctrl_c()=>{}, _=term.recv()=>{}}
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}
