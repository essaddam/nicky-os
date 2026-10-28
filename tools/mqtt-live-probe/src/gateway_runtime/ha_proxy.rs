//! Fixed loopback proxy to the simulation module; never a generic network proxy.
use super::*;
use axum::http::{HeaderMap, Method};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper_util::rt::TokioIo;

const PORT: u16 = 1416;
const LIMIT: usize = 2 * 1024 * 1024;
static ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
fn admitted(app: &App, headers: &HeaderMap) -> bool {
    headers
        .get("x-moli-ticket")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|s| app.tickets.verify(s))
}
fn simulation_state(code: StatusCode, bytes: &[u8]) -> Option<JsonValue> {
    if code != StatusCode::OK {
        return None;
    }
    let state: JsonValue = serde_json::from_slice(bytes).ok()?;
    if state["v"] != 1
        || state["mode"] != "simulation"
        || !state["configured"].is_boolean()
        || state["metrics"]["physical_commands"] != 0
    {
        return None;
    }
    for (key, max) in [("houses", 2), ("nodes", 8), ("sources", 2)] {
        if !state[key].as_array().is_some_and(|v| v.len() <= max) {
            return None;
        }
    }
    Some(state)
}
async fn call(method: Method, path: &'static str, body: Vec<u8>) -> Result<(StatusCode, Vec<u8>)> {
    ensure!(
        matches!(path, "/api/status" | "/api/configure" | "/api/action") && body.len() <= 16384,
        "HA proxy bounds"
    );
    let stream = tokio::time::timeout(
        Duration::from_secs(2),
        tokio::net::TcpStream::connect(("127.0.0.1", PORT)),
    )
    .await??;
    let (mut sender, connection) =
        hyper::client::conn::http1::handshake(TokioIo::new(stream)).await?;
    let connection = tokio::spawn(connection);
    let operation = async {
        let request = hyper::Request::builder()
            .method(method)
            .uri(path)
            .header("host", format!("127.0.0.1:{PORT}"))
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from(body)))?;
        let response = sender.send_request(request).await?;
        let code = response.status();
        let mut body = response.into_body();
        let mut bytes = Vec::new();
        while let Some(frame) = body.frame().await {
            if let Ok(data) = frame?.into_data() {
                ensure!(bytes.len() + data.len() <= LIMIT, "HA response bound");
                bytes.extend_from_slice(&data);
            }
        }
        Ok::<_, anyhow::Error>((code, bytes))
    };
    let result = tokio::time::timeout(Duration::from_secs(45), operation).await;
    connection.abort();
    let _ = connection.await;
    result.context("HA proxy deadline")?
}
async fn proxy(
    app: Arc<App>,
    headers: HeaderMap,
    method: Method,
    path: &'static str,
    body: Vec<u8>,
) -> Response {
    if method == Method::POST {
        if !admitted(&app, &headers) {
            return json_response(StatusCode::UNAUTHORIZED, json!({"error":"ticket"}));
        }
        let valid = match call(Method::GET, "/api/status", Vec::new()).await {
            Ok((code, bytes)) => simulation_state(code, &bytes).is_some(),
            Err(_) => false,
        };
        if !valid {
            return json_response(
                StatusCode::SERVICE_UNAVAILABLE,
                json!({"error":"ha_module_unavailable"}),
            );
        }
    }
    ACTIVE.store(true, Ordering::Relaxed);
    match call(method, path, body).await {
        Ok((code, bytes)) => response(code, bytes, "application/json; charset=utf-8", "no-store"),
        Err(_) => json_response(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"error":"ha_module_unavailable"}),
        ),
    }
}
async fn status(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    proxy(app, headers, Method::GET, "/api/status", Vec::new()).await
}
async fn configure(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(body): Json<JsonValue>,
) -> Response {
    proxy(
        app,
        headers,
        Method::POST,
        "/api/configure",
        body.to_string().into_bytes(),
    )
    .await
}
async fn action(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(body): Json<JsonValue>,
) -> Response {
    proxy(
        app,
        headers,
        Method::POST,
        "/api/action",
        body.to_string().into_bytes(),
    )
    .await
}
pub(super) fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/ha/api/status", get(status))
        .route("/ha/api/configure", post(configure))
        .route("/ha/api/action", post(action))
        .layer(DefaultBodyLimit::max(16384))
}
pub(super) async fn monitor(app: Arc<App>) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    let mut previous_config = Vec::new();
    let mut previous_state = Vec::new();
    loop {
        tokio::select! {()=app.cancel.cancelled()=>break,_=interval.tick()=>{}}
        if !ACTIVE.load(Ordering::Relaxed) {
            continue;
        }
        let state = call(Method::GET, "/api/status", Vec::new())
            .await
            .ok()
            .and_then(|(code, bytes)| simulation_state(code, &bytes));
        let online = state.is_some();
        let ready = state.as_ref().is_some_and(|s| s["configured"] == true);
        let _ = app.broker().publish(
            "moli-probe/ha/availability".into(),
            json!({"v":1,"online":online,"ready":ready,"observed_at_ms":now_ms()})
                .to_string()
                .into_bytes(),
            true,
        );
        let Some(mut state) = state else {
            continue;
        };
        let config = json!({"v":1,"type":"ha_description","pont":"moli-probe/ha","topology":state["topology"],"sources":state["sources"],"availability_topic":"moli-probe/ha/availability","state_topic":"moli-probe/ha/state","freshness_max_ms":5000,"units":{"rss_bytes":"B","allocator_live_bytes":"B","cpu_percent_one_core":"%","document_bytes":"B","sequence":"count"}});
        let bytes = config.to_string().into_bytes();
        if bytes != previous_config {
            let _ = app
                .broker()
                .publish("moli-probe/ha/config".into(), bytes.clone(), true);
            previous_config = bytes;
        }
        if let Some(map) = state.as_object_mut() {
            map.remove("topology");
            map.remove("sources");
        }
        let bytes = state.to_string().into_bytes();
        if bytes != previous_state {
            let _ = app
                .broker()
                .publish("moli-probe/ha/state".into(), bytes.clone(), true);
            previous_state = bytes;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_a_bounded_simulation_module_is_accepted() {
        let valid = json!({"v":1,"mode":"simulation","configured":true,"houses":[],"nodes":[],"sources":[],"metrics":{"physical_commands":0}});
        assert!(simulation_state(StatusCode::OK, valid.to_string().as_bytes()).is_some());
        let mut wrong = valid.clone();
        wrong["mode"] = json!("production");
        assert!(simulation_state(StatusCode::OK, wrong.to_string().as_bytes()).is_none());
        wrong = valid.clone();
        wrong["nodes"] = json!([0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert!(simulation_state(StatusCode::OK, wrong.to_string().as_bytes()).is_none());
        assert!(simulation_state(StatusCode::BAD_GATEWAY, valid.to_string().as_bytes()).is_none());
    }
    #[test]
    fn ha_actions_require_a_valid_gateway_ticket() {
        let (jobs, _) = mpsc::channel(1);
        let app = App {
            broker: OnceLock::new(),
            private: Mutex::new(Private::default()),
            tickets: Tickets([5; 32]),
            jobs,
            child: tokio::sync::Mutex::new(None),
            root: PathBuf::new(),
            port: 1414,
            description_storage: "memory".into(),
            cancel: CancellationToken::new(),
            overkiz: OnceLock::new(),
        };
        let mut headers = HeaderMap::new();
        assert!(!admitted(&app, &headers));
        headers.insert("x-moli-ticket", "wrong".parse().unwrap());
        assert!(!admitted(&app, &headers));
        let ticket = app.tickets.issue().unwrap();
        headers.insert(
            "x-moli-ticket",
            ticket["ticket"].as_str().unwrap().parse().unwrap(),
        );
        assert!(admitted(&app, &headers));
    }
}
