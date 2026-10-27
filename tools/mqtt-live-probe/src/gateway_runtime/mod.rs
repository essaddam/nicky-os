//! HTTP, authenticated WebSocket and bounded MQTT transport for the POC.
mod broker;
mod commands;

use anyhow::{Context, Result, bail, ensure};
use axum::{
    Json, Router,
    extract::{
        ConnectInfo, DefaultBodyLimit, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use broker::{Broker, Role};
use commands::{Binding, expose_description, native_packets};
use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::{Value as JsonValue, json};
use sha2::Sha256;
use std::{
    collections::BTreeMap,
    io::Read,
    net::{IpAddr, SocketAddr},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, UnixListener, UnixStream},
    process::{Child, Command},
    sync::{Semaphore, mpsc},
};
use tokio_util::sync::CancellationToken;

const MAX_BODY: usize = 4 * 1024 * 1024;
const TICKET_MS: u64 = 300_000;
static NEXT_COMMAND: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}
fn random<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0; N];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes)
}

struct Tickets([u8; 32]);
impl Tickets {
    fn issue(&self) -> Result<JsonValue> {
        let expires = now_ms().saturating_add(TICKET_MS);
        let mut data = expires.to_be_bytes().to_vec();
        data.extend(random::<16>()?);
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.0).expect("fixed key size");
        mac.update(&data);
        data.extend(mac.finalize().into_bytes());
        Ok(json!({"ticket":URL_SAFE_NO_PAD.encode(data),"expiresAt":expires}))
    }
    fn verify(&self, encoded: &str) -> bool {
        if encoded.len() > 128 {
            return false;
        }
        let Ok(data) = URL_SAFE_NO_PAD.decode(encoded) else {
            return false;
        };
        if data.len() != 56 {
            return false;
        }
        let expires = u64::from_be_bytes(data[..8].try_into().expect("ticket prefix"));
        let now = now_ms();
        if expires <= now || expires > now.saturating_add(TICKET_MS + 1_000) {
            return false;
        }
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.0).expect("fixed key size");
        mac.update(&data[..24]);
        mac.verify_slice(&data[24..]).is_ok()
    }
}

#[derive(Clone, Deserialize)]
struct Config {
    hosts: Vec<String>,
    username: String,
    password: String,
}
impl Config {
    fn validate(&self) -> Result<()> {
        ensure!(
            !self.hosts.is_empty() && self.hosts.len() <= 4,
            "source count"
        );
        ensure!(
            self.hosts.iter().all(|host| !host.is_empty()
                && host.len() <= 253
                && host
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".:-".contains(&b))),
            "host"
        );
        ensure!(
            !self.username.is_empty()
                && self.username.len() <= 256
                && !self.password.is_empty()
                && self.password.len() <= 1024
                && !self.password.starts_with("{{secret:"),
            "credentials"
        );
        Ok(())
    }
}
#[derive(Default)]
struct Private {
    config: Option<Config>,
    bindings: BTreeMap<String, Binding>,
    runtime: JsonValue,
    rust_rss: u64,
    gateway_rss: u64,
    child_id: Option<u32>,
}
struct App {
    broker: OnceLock<Arc<Broker>>,
    private: Mutex<Private>,
    tickets: Tickets,
    jobs: mpsc::Sender<Job>,
    child: tokio::sync::Mutex<Option<Child>>,
    root: PathBuf,
    port: u16,
    description_storage: String,
    cancel: CancellationToken,
}
#[derive(Clone)]
struct Job {
    topic: String,
    body: JsonValue,
    binding: Binding,
    expires: Instant,
}

impl App {
    fn broker(&self) -> &Arc<Broker> {
        self.broker
            .get()
            .expect("broker initialized before serving")
    }
    fn ready(&self) -> bool {
        self.private.lock().expect("state").config.is_some()
    }
    fn authorize(&self, role: Role, topic: &str, data: &[u8], retain: bool) -> Result<Vec<u8>> {
        if matches!(role, Role::Bridge) {
            ensure!(
                topic.starts_with("moli-probe/") && data.len() <= MAX_BODY,
                "bridge namespace"
            );
            if topic.ends_with("/config") && !data.is_empty() {
                let mut description: JsonValue = serde_json::from_slice(data)?;
                if description["type"] == "description" && description.get("canaux").is_some() {
                    let mut state = self.private.lock().expect("state");
                    let sources = state.config.as_ref().context("starting")?.hosts.len();
                    let bindings = expose_description(&mut description, sources)?;
                    let root = description["pont"].as_str().context("namespace")?;
                    ensure!(topic == format!("{root}/config"), "description topic");
                    let prefix = format!("{root}/canal/");
                    state.bindings.retain(|key, _| !key.starts_with(&prefix));
                    state.bindings.extend(bindings);
                    return Ok(serde_json::to_vec(&description)?);
                }
            }
            return Ok(data.to_vec());
        }
        ensure!(!retain && data.len() <= 256, "command bounds");
        let body: JsonValue = serde_json::from_slice(data)?;
        let binding = self
            .private
            .lock()
            .expect("state")
            .bindings
            .get(topic)
            .cloned()
            .context("unknown command")?;
        native_packets(&binding, &body)?;
        self.jobs
            .try_send(Job {
                topic: topic.into(),
                body,
                binding,
                expires: Instant::now() + Duration::from_secs(5),
            })
            .context("command queue full")?;
        Ok(data.to_vec())
    }
    fn status(&self) -> JsonValue {
        let state = self.private.lock().expect("state");
        let broker = self.broker().stats();
        let sources:Vec<_> = state.config.as_ref().map_or(&[][..], |config| config.hosts.as_slice()).iter().enumerate().map(|(index,_)| {
            let projection=&state.runtime["sources"][index];
            json!({"index":index,"registry":projection["projection"]["channels"].as_u64().unwrap_or(0)>0,"projection":projection})
        }).collect();
        json!({"ready":state.config.is_some(),"port":self.port,"mqtt_port":self.port+1,"sources":sources,"hub":state.runtime,"memory":{"gateway_rss":state.gateway_rss,"gateway_requested":crate::memory::live(),"rust_rss":state.rust_rss,"retained_topics":broker["retained_topics"],"retained_payload_bytes":broker["retained_payload_bytes"],"description_storage":self.description_storage,"description_ram_bytes":broker["description_ram_bytes"],"description_disk_bytes":broker["description_disk_bytes"]},"clients":broker["clients"],"gateway":"rust"})
    }
    async fn configure(self: &Arc<Self>, config: Config) -> Result<()> {
        config.validate()?;
        let mut child = {
            let mut state = self.private.lock().expect("state");
            ensure!(state.config.is_none(), "already configured");
            self.broker()
                .set_credentials(config.username.clone(), config.password.clone());
            let mut command = Command::new(
                self.root
                    .join("target/mqtt-live-probe/debug/mqtt-live-probe"),
            );
            for host in &config.hosts {
                command.args(["--host", host]);
            }
            command.args([
                "--port",
                "1883",
                "--username",
                &config.username,
                "--seconds",
                "0",
                "--bridges",
                &config.hosts.len().to_string(),
                "--republish",
                "moli-probe",
                "--republish-retain",
                "--virtual-tree",
                "--virtual-mirror",
                "--publish-config",
                "--metadata-memory",
                "--destination",
                &format!("127.0.0.1:{}", self.port + 1),
            ]);
            command
                .env("MQTT_PASSWORD", &config.password)
                .env(
                    "MOLI_ENUM_CATALOGUE_DIR",
                    PathBuf::from("/tmp")
                        .join(format!("moli-poc-{}", rustix::process::getuid().as_raw()))
                        .join("enum-catalogues"),
                )
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true);
            let child = command.spawn().context("Hub executable unavailable")?;
            state.child_id = child.id();
            state.config = Some(config);
            child
        };
        let stdout = child.stdout.take().context("child stdout")?;
        *self.child.lock().await = Some(child);
        let app = self.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(raw) = line.strip_prefix("POC_RUNTIME ")
                    && let Ok(value) = serde_json::from_str(raw)
                {
                    app.private.lock().expect("state").runtime = value;
                }
            }
        });
        self.publish_runtime_description()?;
        Ok(())
    }
    fn publish_runtime_description(&self) -> Result<()> {
        let fields = runtime_fields();
        let gauges: Vec<_> = fields
            .iter()
            .map(|(id, unit)| json!({"id":id,"nom":id,"unite":unit}))
            .collect();
        self.broker().publish("moli-probe/runtime/config".into(),serde_json::to_vec(&json!({"v":3,"type":"description","pont":"moli-probe/runtime","nom":"Hub","noeuds":[],"appareils":[],"systeme":gauges}))?,true)
    }
}

fn runtime_fields() -> Vec<(&'static str, &'static str)> {
    vec![
        ("ram_rust_rss", "B"),
        ("ram_rust_requested", "B"),
        ("ram_gateway_rss", "B"),
        ("ram_gateway_requested", "B"),
        ("retained_bytes", "B"),
        ("retained_topics", "topics"),
        ("hub_devices", "appareils"),
        ("hub_points", "canaux"),
        ("description_ram_bytes", "B"),
        ("description_disk_bytes", "B"),
    ]
}

fn response(
    status: StatusCode,
    body: impl Into<axum::body::Body>,
    kind: &str,
    cache: &str,
) -> Response {
    (
        status,
        [
            (header::CONTENT_TYPE, kind),
            (header::CACHE_CONTROL, cache),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        body.into(),
    )
        .into_response()
}
fn json_response(status: StatusCode, value: JsonValue) -> Response {
    response(
        status,
        value.to_string(),
        "application/json; charset=utf-8",
        "no-store",
    )
}
async fn public_status(State(app): State<Arc<App>>) -> Response {
    json_response(StatusCode::OK, json!({"ready":app.ready(),"port":app.port}))
}
async fn ticket(State(app): State<Arc<App>>) -> Response {
    if !app.ready() {
        return json_response(StatusCode::SERVICE_UNAVAILABLE, json!({"error":"starting"}));
    }
    match app.tickets.issue() {
        Ok(value) => json_response(StatusCode::OK, value),
        Err(_) => json_response(StatusCode::SERVICE_UNAVAILABLE, json!({"error":"ticket"})),
    }
}
async fn static_file(State(app): State<Arc<App>>, uri: axum::http::Uri) -> Response {
    let (path, kind, cache) = match uri.path() {
        "/" => (
            app.root
                .join("docs/proposition/analyse/poc-interface-mqtt.html"),
            "text/html; charset=utf-8",
            "no-cache",
        ),
        "/mqtt.min.js" => (
            app.root
                .join("tools/mqtt-live-probe/web-runtime/node_modules/mqtt/dist/mqtt.min.js"),
            "application/javascript",
            "public, max-age=86400",
        ),
        "/service-worker.js" => (
            app.root
                .join("tools/mqtt-live-probe/web-runtime/public/service-worker.js"),
            "application/javascript",
            "no-cache",
        ),
        "/manifest.webmanifest" => (
            app.root
                .join("tools/mqtt-live-probe/web-runtime/public/manifest.webmanifest"),
            "application/manifest+json",
            "no-cache",
        ),
        "/icons/moli.svg"
        | "/icons/moli-192.png"
        | "/icons/moli-512.png"
        | "/icons/apple-touch-icon.png" => (
            app.root
                .join("tools/mqtt-live-probe/web-runtime/public")
                .join(uri.path().trim_start_matches('/')),
            if uri.path().ends_with(".svg") {
                "image/svg+xml"
            } else {
                "image/png"
            },
            "public, max-age=86400",
        ),
        _ => return json_response(StatusCode::NOT_FOUND, json!({"error":"not_found"})),
    };
    match tokio::fs::read(path).await {
        Ok(bytes) => {
            let bytes = if uri.path() == "/" {
                String::from_utf8_lossy(&bytes)
                    .replace(
                        "https://unpkg.com/mqtt@5.10.3/dist/mqtt.min.js",
                        "/mqtt.min.js",
                    )
                    .into_bytes()
            } else {
                bytes
            };
            response(StatusCode::OK, bytes, kind, cache)
        }
        Err(_) => json_response(StatusCode::NOT_FOUND, json!({"error":"not_found"})),
    }
}
async fn websocket(
    State(app): State<Arc<App>>,
    Query(query): Query<BTreeMap<String, String>>,
    ws: WebSocketUpgrade,
) -> Response {
    if !app.ready()
        || !query
            .get("ticket")
            .is_some_and(|value| app.tickets.verify(value))
    {
        return json_response(StatusCode::UNAUTHORIZED, json!({"error":"socket_ticket"}));
    }
    ws.max_message_size(64 * 1024)
        .max_frame_size(64 * 1024)
        .on_upgrade(move |socket| websocket_session(app, socket))
}
async fn websocket_session(app: Arc<App>, socket: WebSocket) {
    let (stream, mut adapter) = tokio::io::duplex(64 * 1024);
    let broker = app.broker().clone();
    let session = tokio::spawn(async move {
        let _ = broker.handle(stream, Role::Browser).await;
    });
    let (mut sender, mut receiver) = socket.split();
    let (mut output, mut input) = tokio::io::split(&mut adapter);
    let incoming = async {
        while let Some(Ok(message)) = receiver.next().await {
            match message {
                Message::Binary(data) => input.write_all(&data).await?,
                Message::Close(_) => break,
                Message::Text(_) => bail!("binary MQTT required"),
                _ => {}
            }
        }
        Ok::<(), anyhow::Error>(())
    };
    let outgoing = async {
        let mut block = [0; 8192];
        loop {
            let length = output.read(&mut block).await?;
            if length == 0 {
                break;
            }
            sender
                .send(Message::Binary(block[..length].to_vec().into()))
                .await?;
        }
        Ok::<(), anyhow::Error>(())
    };
    tokio::select! {_ = incoming=>{},_ = outgoing=>{},_ = app.cancel.cancelled()=>{}}
    session.abort();
}

async fn source_registry(
    State(app): State<Arc<App>>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    Json(body): Json<JsonValue>,
) -> Response {
    let source = {
        let state = app.private.lock().expect("state");
        state.config.as_ref().and_then(|c| {
            c.hosts
                .iter()
                .position(|host| host.parse::<IpAddr>().ok() == Some(address.ip()))
        })
    };
    let Some(index) = source else {
        return json_response(StatusCode::FORBIDDEN, json!({"error":"source"}));
    };
    if body["source"].as_u64() != Some(index as u64)
        || !body["registry"]["entities"]
            .as_array()
            .is_some_and(|a| a.len() <= 8192)
    {
        return json_response(StatusCode::BAD_REQUEST, json!({"error":"registry"}));
    }
    let result = serde_json::to_vec(&body["registry"])
        .map_err(anyhow::Error::from)
        .and_then(|data| {
            app.broker().publish(
                format!("moli-probe/source/z2m-{index}/registry"),
                data,
                true,
            )
        });
    if result.is_err() {
        return json_response(StatusCode::BAD_REQUEST, json!({"error":"registry"}));
    }
    json_response(StatusCode::OK, json!({"accepted":true,"source":index}))
}

async fn rss(pid: u32) -> u64 {
    let Ok(output) = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .await
    else {
        return 0;
    };
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u64>()
        .unwrap_or(0)
        * 1024
}
async fn monitor(app: Arc<App>) {
    let mut interval = tokio::time::interval(Duration::from_secs(2));
    loop {
        interval.tick().await;
        if !app.ready() {
            continue;
        }
        let child = app.private.lock().expect("state").child_id;
        let gateway_rss = rss(std::process::id()).await;
        let rust_rss = if let Some(pid) = child {
            rss(pid).await
        } else {
            0
        };
        {
            let mut state = app.private.lock().expect("state");
            state.gateway_rss = gateway_rss;
            state.rust_rss = rust_rss;
        }
        let status = app.status();
        let values = json!({"ram_rust_rss":rust_rss,"ram_rust_requested":status["hub"]["allocator_live_bytes"],"ram_gateway_rss":gateway_rss,"ram_gateway_requested":crate::memory::live(),"retained_bytes":status["memory"]["retained_payload_bytes"],"retained_topics":status["memory"]["retained_topics"],"hub_devices":status["hub"]["hub_devices"],"hub_points":status["hub"]["hub_points"],"description_ram_bytes":status["memory"]["description_ram_bytes"],"description_disk_bytes":status["memory"]["description_disk_bytes"]});
        let gauges: Vec<_> = runtime_fields()
            .iter()
            .map(|(id, _)| json!({"id":id,"valeur":values[id]}))
            .collect();
        let _=app.broker().publish("moli-probe/runtime/state".into(),json!({"v":3,"type":"systeme","pont":"moli-probe/runtime","canaux":gauges,"ts":now_ms()}).to_string().into_bytes(),true);
    }
}
async fn execute(app: Arc<App>, job: Job) {
    let config = {
        let state = app.private.lock().expect("state");
        if !state.bindings.get(&job.topic).is_some_and(|binding| {
            binding.source == job.binding.source && binding.native == job.binding.native
        }) {
            return;
        }
        state.config.clone()
    };
    let Some(config) = config else {
        return;
    };
    let operation = async {
        ensure!(Instant::now() < job.expires, "expired");
        let host = config.hosts.get(job.binding.source).context("source")?;
        let mut options = rumqttc::MqttOptions::new(
            format!(
                "moli-human-{}-{}",
                std::process::id(),
                NEXT_COMMAND.fetch_add(1, Ordering::Relaxed)
            ),
            host,
            1883,
        );
        options.set_credentials(config.username, config.password);
        let (client, mut events) = rumqttc::AsyncClient::new(options, 8);
        loop {
            if matches!(
                events.poll().await?,
                rumqttc::Event::Incoming(rumqttc::Packet::ConnAck(_))
            ) {
                break;
            }
        }
        for packet in native_packets(&job.binding, &job.body)? {
            client.try_publish(
                packet.topic,
                rumqttc::QoS::AtLeastOnce,
                false,
                packet.payload,
            )?;
            loop {
                if matches!(
                    events.poll().await?,
                    rumqttc::Event::Incoming(rumqttc::Packet::PubAck(_))
                ) {
                    break;
                }
            }
        }
        Ok::<(), anyhow::Error>(())
    };
    let ok = tokio::time::timeout_at(job.expires.into(), operation)
        .await
        .is_ok_and(|r| r.is_ok());
    let result = json!({"v":3,"type":"commande_resultat","ok":ok,"livraison":if ok{"transmise_sans_confirmation_physique"}else{"non_confirmee"},"ts":now_ms()});
    let _ = app.broker().publish(
        job.topic.trim_end_matches("/set").to_owned() + "/result",
        result.to_string().into_bytes(),
        false,
    );
}
async fn commands_worker(app: Arc<App>, mut jobs: mpsc::Receiver<Job>) {
    let workers = Arc::new(Semaphore::new(4));
    while let Some(job) = jobs.recv().await {
        let Ok(permit) = workers.clone().acquire_owned().await else {
            break;
        };
        let app = app.clone();
        tokio::spawn(async move {
            let _permit = permit;
            execute(app, job).await;
        });
    }
}
async fn control_session(app: Arc<App>, mut stream: UnixStream) -> Result<()> {
    let mut bytes = Vec::new();
    (&mut stream).take(16385).read_to_end(&mut bytes).await?;
    ensure!(bytes.len() <= 16384, "control bounds");
    let body: JsonValue = serde_json::from_slice(&bytes)?;
    let reply = if body["op"] == "status" {
        app.status()
    } else {
        match serde_json::from_value::<Config>(body) {
            Ok(config) => {
                if app.configure(config).await.is_ok() {
                    json!({"ready":true,"port":app.port,"mqtt_port":app.port+1})
                } else {
                    json!({"error":"configuration_refused"})
                }
            }
            Err(_) => json!({"error":"configuration_refused"}),
        }
    };
    stream.write_all(reply.to_string().as_bytes()).await?;
    Ok(())
}

pub async fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let option = |key: &str| {
        args.iter()
            .position(|s| s == key)
            .and_then(|i| args.get(i + 1))
    };
    let port = option("--port").map_or(Ok(1414), |s| s.parse::<u16>())?;
    ensure!((1024..65534).contains(&port), "port");
    let storage = option("--description-storage").map_or("memory", String::as_str);
    ensure!(
        matches!(storage, "memory" | "disk"),
        "description storage must be memory or disk"
    );
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .context("repository root")?
        .to_owned();
    let (jobs, receiver) = mpsc::channel(128);
    let app = Arc::new(App {
        broker: OnceLock::new(),
        private: Mutex::new(Private::default()),
        tickets: Tickets(random()?),
        jobs,
        child: tokio::sync::Mutex::new(None),
        root,
        port,
        description_storage: storage.into(),
        cancel: CancellationToken::new(),
    });
    let weak = Arc::downgrade(&app);
    let broker = Broker::new(Arc::new(move |role, topic, data, retain| {
        weak.upgrade()
            .context("shutdown")?
            .authorize(role, topic, data, retain)
    }));
    // Reserve both ports before replacing any running instance's control socket.
    let tcp = TcpListener::bind(("127.0.0.1", port + 1)).await?;
    let listener = TcpListener::bind(("0.0.0.0", port)).await?;
    // Preserve the private control address used by the existing launcher.
    let directory =
        PathBuf::from("/tmp").join(format!("moli-poc-{}", rustix::process::getuid().as_raw()));
    std::fs::create_dir_all(&directory)?;
    ensure!(
        directory.metadata()?.uid() == rustix::process::getuid().as_raw(),
        "private directory owner"
    );
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
    if storage == "disk" {
        broker.set_description_directory(
            directory.join(format!("{port}-descriptions-{}", std::process::id())),
        )?;
    }
    app.broker
        .set(broker.clone())
        .map_err(|_| anyhow::anyhow!("broker initialization"))?;
    let socket = directory.join(format!("{port}.sock"));
    match std::fs::remove_file(&socket) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let control = UnixListener::bind(&socket)?;
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
    let control_app = app.clone();
    tokio::spawn(async move {
        while let Ok((stream, _)) = control.accept().await {
            let app = control_app.clone();
            tokio::spawn(async move {
                let _ = tokio::time::timeout(Duration::from_secs(5), control_session(app, stream))
                    .await;
            });
        }
    });
    tokio::spawn(async move {
        while let Ok((stream, _)) = tcp.accept().await {
            let broker = broker.clone();
            tokio::spawn(async move {
                let _ = broker.handle(stream, Role::Bridge).await;
            });
        }
    });
    tokio::spawn(monitor(app.clone()));
    tokio::spawn(commands_worker(app.clone(), receiver));
    let routes = Router::new()
        .route("/", get(static_file))
        .route("/mqtt.min.js", get(static_file))
        .route("/service-worker.js", get(static_file))
        .route("/manifest.webmanifest", get(static_file))
        .route("/icons/{name}", get(static_file))
        .route("/api/status", get(public_status))
        .route("/api/socket-ticket", get(ticket))
        .route("/mqtt", get(websocket))
        .route("/source/registry", post(source_registry))
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .with_state(app.clone());
    println!("Passerelle web Rust prête sur le port {port} ; configuration privée attendue.");
    let shutdown_app = app.clone();
    let serving = axum::serve(
        listener,
        routes.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        let _ = tokio::signal::ctrl_c().await;
        shutdown_app.cancel.cancel();
    });
    let result = serving.await;
    if let Some(mut child) = app.child.lock().await.take() {
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
    let _ = std::fs::remove_file(socket);
    result.context("HTTP server")
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BytesMut;
    use rumqttc::mqttbytes::v4::{Connect, ConnectReturnCode, Packet, Subscribe};
    use tokio_tungstenite::tungstenite::{Message as ClientMessage, protocol::WebSocketConfig};

    #[tokio::test]
    async fn real_websocket_requires_ticket_and_mqtt_auth_and_streams_large_retained() {
        let (jobs, _receiver) = mpsc::channel(128);
        let app = Arc::new(App {
            broker: OnceLock::new(),
            private: Mutex::new(Private {
                config: Some(Config {
                    hosts: vec!["127.0.0.1".into()],
                    username: "fixture".into(),
                    password: "fixture-password".into(),
                }),
                ..Private::default()
            }),
            tickets: Tickets([7; 32]),
            jobs,
            child: tokio::sync::Mutex::new(None),
            root: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .to_path_buf(),
            port: 1414,
            description_storage: "memory".into(),
            cancel: CancellationToken::new(),
        });
        let weak = Arc::downgrade(&app);
        let broker = Broker::new(Arc::new(move |role, topic, data, retain| {
            weak.upgrade()
                .context("shutdown")?
                .authorize(role, topic, data, retain)
        }));
        broker.set_credentials("fixture".into(), "fixture-password".into());
        app.broker.set(broker.clone()).ok().unwrap();
        let payload = vec![b'a'; 1_500_000];
        broker
            .publish(
                "moli-probe/maison/fixture/config".into(),
                payload.clone(),
                true,
            )
            .unwrap();
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let routes = Router::new()
            .route("/mqtt", get(websocket))
            .with_state(app.clone());
        let stop = app.cancel.clone();
        let task = tokio::spawn(async move {
            axum::serve(listener, routes)
                .with_graceful_shutdown(stop.cancelled_owned())
                .await
                .unwrap();
        });
        let refused = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/mqtt"))
            .await
            .unwrap_err();
        assert!(
            matches!(refused,tokio_tungstenite::tungstenite::Error::Http(response)if response.status()==StatusCode::UNAUTHORIZED)
        );
        for password in ["wrong", "fixture-password"] {
            let ticket = app.tickets.issue().unwrap();
            let url = format!(
                "ws://127.0.0.1:{port}/mqtt?ticket={}",
                ticket["ticket"].as_str().unwrap()
            );
            let mut limits = WebSocketConfig::default();
            limits.max_message_size = Some(32768);
            limits.max_frame_size = Some(32768);
            let (mut socket, _) =
                tokio_tungstenite::connect_async_with_config(url, Some(limits), false)
                    .await
                    .unwrap();
            let mut connect = Connect::new("fixture-browser");
            connect.set_login("fixture", password);
            let mut encoded = BytesMut::new();
            Packet::Connect(connect)
                .write(&mut encoded, MAX_BODY)
                .unwrap();
            for part in encoded.chunks(3) {
                socket
                    .send(ClientMessage::Binary(part.to_vec().into()))
                    .await
                    .unwrap();
            }
            let mut buffer = BytesMut::new();
            let ack = tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    if let Ok(packet) = Packet::read(&mut buffer, MAX_BODY) {
                        break packet;
                    }
                    match socket.next().await.unwrap().unwrap() {
                        ClientMessage::Binary(data) => buffer.extend_from_slice(&data),
                        other => panic!("unexpected frame {other:?}"),
                    }
                }
            })
            .await
            .unwrap();
            let Packet::ConnAck(ack) = ack else {
                panic!("CONNACK expected")
            };
            if password == "wrong" {
                assert_eq!(ack.code, ConnectReturnCode::BadUserNamePassword);
                continue;
            }
            assert_eq!(ack.code, ConnectReturnCode::Success);
            let mut subscribe = Subscribe::new("moli-probe/maison/#", rumqttc::QoS::AtLeastOnce);
            subscribe.pkid = 1;
            let mut encoded = BytesMut::new();
            Packet::Subscribe(subscribe)
                .write(&mut encoded, MAX_BODY)
                .unwrap();
            socket
                .send(ClientMessage::Binary(encoded.to_vec().into()))
                .await
                .unwrap();
            let received = tokio::time::timeout(Duration::from_secs(3), async {
                'received: loop {
                    while let Ok(packet) = Packet::read(&mut buffer, MAX_BODY) {
                        if let Packet::Publish(value) = packet {
                            break 'received value;
                        }
                    }
                    match socket.next().await.unwrap().unwrap() {
                        ClientMessage::Binary(data) => buffer.extend_from_slice(&data),
                        other => panic!("unexpected frame {other:?}"),
                    }
                }
            })
            .await;
            let received = received.unwrap();
            assert!(received.retain);
            assert_eq!(received.payload.as_ref(), payload);
            socket.close(None).await.unwrap();
        }
        app.cancel.cancel();
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
    }
    #[test]
    fn tickets_are_signed_expiring_and_scoped_to_this_server() {
        let tickets = Tickets([1; 32]);
        let issue = tickets.issue().unwrap();
        let value = issue["ticket"].as_str().unwrap();
        assert!(tickets.verify(value));
        assert!(!Tickets([2; 32]).verify(value));
        assert!(!tickets.verify("invalid"));
        let mut data = URL_SAFE_NO_PAD.decode(value).unwrap();
        data[0] ^= 1;
        assert!(!tickets.verify(&URL_SAFE_NO_PAD.encode(data)));
    }
    #[test]
    fn bootstrap_does_not_accept_unbounded_sources_or_literal_secrets() {
        let valid = Config {
            hosts: vec!["127.0.0.1".into()],
            username: "fixture".into(),
            password: "fixture-password".into(),
        };
        assert!(valid.validate().is_ok());
        let mut bad = valid.clone();
        bad.password = "{{secret:fixture}}".into();
        assert!(bad.validate().is_err());
        bad = valid;
        bad.hosts = vec!["fixture/path".into()];
        assert!(bad.validate().is_err());
    }
}
