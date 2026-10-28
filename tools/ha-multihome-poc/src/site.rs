//! Two declared sites, synthetic updates only, with no physical executor.
//! VPN addresses are mandatory unless an explicit all-loopback fixture is selected.
use crate::{
    authority::{Authority, Effect, Scope},
    config,
    engine::Bundle,
    site_transport::{Rpc, ServerConnection, SiteBroker},
    transport::{Operation, Replica},
};
use anyhow::{Context, Result, bail, ensure};
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{ConnectInfo, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{net::TcpListener, task::JoinHandle};
use tokio_util::sync::CancellationToken;

const RING_LENGTH: usize = 256;
const RING_BYTES: usize = 1024 * 1024;
const SNAPSHOT_THRESHOLD: u64 = 16;
const TICK_SECONDS: u64 = 2;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    node: String,
    house: String,
    bind_ip: Ipv4Addr,
    peer_ip: Ipv4Addr,
    observer_ip: Ipv4Addr,
    #[serde(default)]
    loopback_fixture: bool,
    #[serde(default = "mqtt_port")]
    mqtt_port: u16,
    #[serde(default)]
    peer_mqtt_port: Option<u16>,
    #[serde(default = "control_port")]
    control_port: u16,
    prefix: String,
    bundle: Bundle,
    #[serde(default = "ttl_seconds")]
    ttl_seconds: u64,
}
fn mqtt_port() -> u16 {
    53141
}
fn control_port() -> u16 {
    53142
}
fn ttl_seconds() -> u64 {
    270
}

#[derive(Clone)]
struct Settings {
    node: String,
    house: String,
    peer_node: String,
    peer_house: String,
    bind_ip: Ipv4Addr,
    peer_ip: Ipv4Addr,
    observer_ip: Ipv4Addr,
    mqtt_port: u16,
    peer_mqtt_port: u16,
    control_port: u16,
    prefix: String,
    ttl_seconds: u64,
}
impl Input {
    fn validate(&self) -> Result<Settings> {
        self.bundle.validate()?;
        ensure!(
            matches!(
                (self.node.as_str(), self.house.as_str()),
                ("node-a", "house-a") | ("node-b", "house-b")
            ),
            "invalid synthetic site identity"
        );
        let addresses = [self.bind_ip, self.peer_ip, self.observer_ip];
        if self.loopback_fixture {
            ensure!(
                addresses.into_iter().all(|ip| ip.is_loopback()),
                "loopback fixture requires only loopback addresses"
            );
        } else {
            ensure!(
                addresses.into_iter().all(vpn_ip),
                "declared VPN addresses required"
            );
        }
        let peer_mqtt_port = self.peer_mqtt_port.unwrap_or(self.mqtt_port);
        ensure!(
            self.bind_ip != self.peer_ip
                || (self.loopback_fixture && peer_mqtt_port != self.mqtt_port),
            "site MQTT endpoints must be distinct"
        );
        ensure!(
            safe_port(self.mqtt_port)
                && safe_port(peer_mqtt_port)
                && safe_port(self.control_port)
                && self.mqtt_port != self.control_port,
            "invalid site bench ports"
        );
        ensure!(
            (1..=480).contains(&self.ttl_seconds),
            "site lifetime exceeds effect budget"
        );
        ensure!(
            self.prefix.starts_with("moli-probe/")
                && self.prefix.len() <= 128
                && self.prefix.split('/').all(|part| !part.is_empty()
                    && part
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))),
            "invalid synthetic namespace"
        );
        Ok(Settings {
            node: self.node.clone(),
            house: self.house.clone(),
            peer_node: if self.node == "node-a" {
                "node-b"
            } else {
                "node-a"
            }
            .into(),
            peer_house: if self.house == "house-a" {
                "house-b"
            } else {
                "house-a"
            }
            .into(),
            bind_ip: self.bind_ip,
            peer_ip: self.peer_ip,
            observer_ip: self.observer_ip,
            mqtt_port: self.mqtt_port,
            peer_mqtt_port,
            control_port: self.control_port,
            prefix: self.prefix.clone(),
            ttl_seconds: self.ttl_seconds,
        })
    }
}
fn vpn_ip(ip: Ipv4Addr) -> bool {
    ip.octets()[0] == 100 && (64..=127).contains(&ip.octets()[1])
}
fn safe_port(port: u16) -> bool {
    port >= 1024 && ![1414, 1415, 1416, 1883, 1884, 8790].contains(&port)
}
fn now(origin: Instant) -> u64 {
    origin.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

#[derive(Clone)]
struct Delta {
    sequence: u64,
    value: Value,
    hash: String,
    bytes: usize,
}
struct Snapshot {
    sequence: u64,
    document: Value,
    hash: String,
}
#[derive(Default)]
struct Network {
    successes: u64,
    errors: u64,
    connections: u64,
    delta_recoveries: u64,
    snapshot_recoveries: u64,
    last_success: Option<Instant>,
    last_peer_status: Option<(Instant, Value)>,
}
struct Shared {
    settings: Arc<Settings>,
    origin: Instant,
    replica: Replica,
    initial_own_hash: String,
    authority: Authority,
    ring: VecDeque<Delta>,
    frozen: bool,
    effects: u64,
    network: Network,
    last_error: Option<&'static str>,
}
impl Shared {
    fn new(settings: Arc<Settings>, bundle: &Bundle) -> Result<Self> {
        let origin = Instant::now();
        let mut replica = Replica::default();
        for seed in &bundle.houses {
            replica.seed(
                &seed.id,
                1,
                0,
                &seed.document,
                if seed.id == settings.house {
                    "active"
                } else {
                    "passive"
                },
            )?;
        }
        let initial_own_hash = replica.summary(&settings.house)?["sha256"]
            .as_str()
            .context("initial own hash absent")?
            .to_owned();
        let authority = Authority::new(config::preset("cross")?, now(origin))?;
        Ok(Self {
            settings,
            origin,
            replica,
            initial_own_hash,
            authority,
            ring: VecDeque::new(),
            frozen: false,
            effects: 0,
            network: Network::default(),
            last_error: None,
        })
    }
    fn advance(&mut self) -> Result<()> {
        if self.frozen {
            return Ok(());
        }
        let house = &self.settings.house;
        let node = &self.settings.node;
        let sequence = self
            .authority
            .lease(house)
            .context("own lease absent")?
            .sequence
            .checked_add(1)
            .context("sequence exhausted")?;
        let at = now(self.origin);
        let value =
            json!({"value":sequence,"quality":"simulation","observed_ms":at,"changed_ms":at});
        let mut preview = self.authority.clone();
        preview.committed(house, node, 1, at, sequence)?;
        ensure!(
            preview.accept_effect(
                &Effect {
                    house: house.clone(),
                    node: node.clone(),
                    epoch: 1,
                    id: format!("site-{sequence}"),
                    scope: Scope::Local
                },
                at
            )?,
            "duplicate synthetic effect"
        );
        let summary = self.replica.apply(Operation::Update {
            house: house.clone(),
            epoch: 1,
            sequence,
            key: "site-bench/tick".into(),
            value: value.clone(),
        })?;
        let bytes = serde_json::to_vec(&value)?.len() + "site-bench/tick".len() + 64;
        let hash = summary["sha256"]
            .as_str()
            .context("own hash absent")?
            .to_owned();
        self.authority = preview;
        self.effects += 1;
        self.ring.push_back(Delta {
            sequence,
            value,
            hash,
            bytes,
        });
        self.trim_ring();
        Ok(())
    }
    fn trim_ring(&mut self) {
        while self.ring.len() > RING_LENGTH || self.ring_bytes() > RING_BYTES {
            self.ring.pop_front();
        }
    }
    fn ring_bytes(&self) -> usize {
        self.ring.iter().map(|delta| delta.bytes).sum()
    }
    fn expected_hash(&self, sequence: u64) -> Option<String> {
        if sequence == 0 {
            return Some(self.initial_own_hash.clone());
        }
        self.ring
            .iter()
            .find(|delta| delta.sequence == sequence)
            .map(|delta| delta.hash.clone())
            .or_else(|| {
                let own = self.replica.summary(&self.settings.house).ok()?;
                (own["sequence"] == sequence)
                    .then(|| own["sha256"].as_str().unwrap_or_default().to_owned())
            })
    }
    fn applied_rpc(&mut self) {
        self.network.successes += 1;
        self.network.last_success = Some(Instant::now());
        if matches!(
            self.last_error,
            Some("peer_unavailable" | "replication_retry")
        ) {
            self.last_error = None;
        }
    }
    fn status(&self) -> Value {
        let own = self
            .replica
            .summary(&self.settings.house)
            .unwrap_or(Value::Null);
        let time = now(self.origin);
        let lease_valid = self
            .authority
            .lease(&self.settings.house)
            .is_some_and(|lease| {
                lease.owner == self.settings.node
                    && lease.epoch == 1
                    && lease.sequence == own["sequence"].as_u64().unwrap_or(u64::MAX)
                    && time < lease.expires_ms
            });
        let age = self.network.last_success.map(|at| at.elapsed().as_millis());
        let peer = self.network.last_peer_status.as_ref().map(|(at, status)| json!({"age_ms":at.elapsed().as_millis(),"own_sequence":status["own_sequence"],"own_sha256":status["own_sha256"]}));
        json!({"mode":"site_bench_simulation","network_scope":if self.settings.bind_ip.is_loopback(){"loopback_fixture"}else{"declared_vpn"},"node":self.settings.node,"house":self.settings.house,"uptime_ms":time,"own_sequence":own["sequence"],"own_sha256":own["sha256"],"copies":self.replica.summaries(),"own_lease_valid":lease_valid,"effects":self.effects,"physical_commands":0,"frozen":self.frozen,"ring_len":self.ring.len(),"ring_bytes":self.ring_bytes(),"ring_max_entries":RING_LENGTH,"ring_max_bytes":RING_BYTES,"snapshot_threshold":SNAPSHOT_THRESHOLD,"replica_rpc":{"successes":self.network.successes,"errors":self.network.errors,"connections":self.network.connections},"delta_recoveries":self.network.delta_recoveries,"snapshot_recoveries":self.network.snapshot_recoveries,"remote_connected":age.is_some_and(|age| age<5000),"remote_last_success_age_ms":age,"peer_freshness":peer,"allocator_live_bytes":crate::memory::live(),"allocator_peak_bytes":crate::memory::peak(),"error":self.last_error,"limits":{"ttl_seconds":self.settings.ttl_seconds,"tick_seconds":TICK_SECONDS,"effect_receipts":256},"limitations":["synthetic_updates_only","independent_local_authority_no_cross_site_promotion","vpn_ip_allowlist_not_production_authentication"]})
    }
}
type StateCell = Arc<Mutex<Shared>>;
fn lock(cell: &StateCell) -> std::sync::MutexGuard<'_, Shared> {
    cell.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn validate_incoming(operation: &Operation, settings: &Settings) -> Result<()> {
    let house = match operation {
        Operation::SeedBegin { house, epoch, .. } | Operation::Update { house, epoch, .. } => {
            ensure!(*epoch == 1, "foreign epoch changes are disabled");
            house
        }
        Operation::SeedChunk { house, .. } | Operation::SeedCommit { house, .. } => house,
        Operation::SetRole { house, epoch, role } => {
            ensure!(
                *epoch == 1 && role == "passive",
                "foreign promotion is disabled"
            );
            house
        }
        Operation::Status => return Ok(()),
        Operation::Stop => bail!("peer cannot stop a site"),
    };
    ensure!(
        house == &settings.peer_house,
        "peer cannot mutate the local primary"
    );
    Ok(())
}
async fn incoming(mut connection: ServerConnection, state: StateCell, cancel: CancellationToken) {
    loop {
        let next = tokio::select! { ()=cancel.cancelled()=>break, next=connection.next()=>next };
        let Some((id, operation)) = next else {
            cancel.cancel();
            break;
        };
        let result = {
            let mut shared = lock(&state);
            validate_incoming(&operation, &shared.settings).and_then(|()| {
                if matches!(operation, Operation::Status) {
                    let own = shared.replica.summary(&shared.settings.house)?;
                    Ok(json!({"node":shared.settings.node,"house":shared.settings.house,"own_sequence":own["sequence"],"own_sha256":own["sha256"],"copies":shared.replica.summaries()}))
                } else { shared.replica.apply(operation) }
            })
        };
        if connection.reply(id, result).await.is_err() {
            lock(&state).last_error = Some("local_reply_failed");
        }
    }
}
async fn local_ticker(state: StateCell, cancel: CancellationToken) {
    let mut renew = tokio::time::interval(Duration::from_millis(200));
    let mut updates = tokio::time::interval_at(
        tokio::time::Instant::now() + Duration::from_secs(TICK_SECONDS),
        Duration::from_secs(TICK_SECONDS),
    );
    renew.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    updates.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            ()=cancel.cancelled()=>break,
            _=renew.tick()=>{
                let mut shared = lock(&state);
                let house = shared.settings.house.clone(); let node = shared.settings.node.clone(); let time = now(shared.origin);
                if shared.authority.renew(&house,&node,1,time).is_err() { shared.last_error=Some("local_lease_expired"); }
            }
            _=updates.tick()=>{
                let result = { lock(&state).advance() };
                if result.is_err() { lock(&state).last_error=Some("local_update_refused"); }
            }
        }
    }
}

fn remote_copy<'a>(status: &'a Value, house: &str) -> Result<&'a Value> {
    status["copies"]
        .as_array()
        .and_then(|copies| copies.iter().find(|copy| copy["house"] == house))
        .context("peer copy absent")
}
fn check_ack(ack: &Value, sequence: u64, hash: &str) -> Result<()> {
    ensure!(
        ack["epoch"] == 1
            && ack["sequence"] == sequence
            && ack["sha256"] == hash
            && ack["records"].as_u64().unwrap_or(0) > 0,
        "replica acknowledgement mismatch"
    );
    Ok(())
}
async fn replicate_once(
    rpc: &mut Rpc,
    state: &StateCell,
    snapshot: &mut Option<Snapshot>,
    catching_up: &mut bool,
) -> Result<()> {
    let peer = rpc.request(Operation::Status).await?;
    let (house, local_sequence, remote_sequence, remote_hash) = {
        let mut shared = lock(state);
        shared.applied_rpc();
        shared.network.last_peer_status = Some((Instant::now(), peer.clone()));
        let own = shared.replica.summary(&shared.settings.house)?;
        let copy = remote_copy(&peer, &shared.settings.house)?;
        ensure!(copy["epoch"] == 1, "peer epoch mismatch");
        let local = own["sequence"].as_u64().context("own sequence absent")?;
        let remote = copy["sequence"]
            .as_u64()
            .context("remote sequence absent")?;
        ensure!(remote <= local, "peer copy is ahead of this primary");
        (
            shared.settings.house.clone(),
            local,
            remote,
            copy["sha256"]
                .as_str()
                .context("remote hash absent")?
                .to_owned(),
        )
    };
    if let Some(pending) = snapshot
        && remote_sequence >= pending.sequence
    {
        let expected = if remote_sequence == pending.sequence {
            Some(pending.hash.clone())
        } else {
            lock(state).expected_hash(remote_sequence)
        };
        ensure!(
            expected.as_deref() == Some(remote_hash.as_str()),
            "peer snapshot lineage mismatch"
        );
        lock(state).network.snapshot_recoveries += 1;
        *snapshot = None;
        *catching_up = true;
    }
    if remote_sequence == local_sequence {
        ensure!(
            lock(state).expected_hash(local_sequence).as_deref() == Some(remote_hash.as_str()),
            "equal sequences have different hashes"
        );
        *snapshot = None;
        *catching_up = false;
        return Ok(());
    }
    let deltas = {
        let shared = lock(state);
        shared
            .ring
            .iter()
            .filter(|delta| delta.sequence > remote_sequence && delta.sequence <= local_sequence)
            .cloned()
            .collect::<Vec<_>>()
    };
    let contiguous = deltas
        .first()
        .is_some_and(|delta| Some(delta.sequence) == remote_sequence.checked_add(1))
        && deltas
            .last()
            .is_some_and(|delta| delta.sequence == local_sequence);
    if snapshot.is_none()
        && contiguous
        && (*catching_up || local_sequence - remote_sequence <= SNAPSHOT_THRESHOLD)
    {
        for delta in deltas {
            let ack = rpc
                .request(Operation::Update {
                    house: house.clone(),
                    epoch: 1,
                    sequence: delta.sequence,
                    key: "site-bench/tick".into(),
                    value: delta.value,
                })
                .await?;
            check_ack(&ack, delta.sequence, &delta.hash)?;
            lock(state).applied_rpc();
        }
        lock(state).network.delta_recoveries += 1;
        return Ok(());
    }
    if snapshot.is_none() {
        let shared = lock(state);
        let own = shared.replica.summary(&house)?;
        *snapshot = Some(Snapshot {
            sequence: own["sequence"]
                .as_u64()
                .context("snapshot sequence absent")?,
            document: shared.replica.document(&house)?,
            hash: own["sha256"]
                .as_str()
                .context("snapshot hash absent")?
                .into(),
        });
    }
    let pending = snapshot.as_ref().context("snapshot absent")?;
    let earliest = lock(state).ring.front().map(|delta| delta.sequence);
    if earliest.is_some_and(|first| first > pending.sequence.saturating_add(1)) {
        *snapshot = None;
        bail!("snapshot rank exceeded bounded history");
    }
    let ack = rpc
        .seed(&house, 1, pending.sequence, &pending.document, "passive")
        .await?;
    check_ack(&ack, pending.sequence, &pending.hash)?;
    {
        let mut shared = lock(state);
        shared.applied_rpc();
        shared.network.snapshot_recoveries += 1;
    }
    *snapshot = None;
    *catching_up = true;
    Ok(())
}
async fn outgoing(settings: Arc<Settings>, state: StateCell, cancel: CancellationToken) {
    let mut interval = tokio::time::interval(Duration::from_millis(500));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut rpc = None;
    let mut snapshot = None;
    let mut catching_up = false;
    loop {
        tokio::select! {()=cancel.cancelled()=>break,_=interval.tick()=>{}}
        if rpc.is_none() {
            match Rpc::open(
                IpAddr::V4(settings.peer_ip),
                settings.peer_mqtt_port,
                &settings.peer_node,
                &settings.prefix,
            )
            .await
            {
                Ok(connection) => {
                    rpc = Some(connection);
                    lock(&state).network.connections += 1;
                }
                Err(_) => {
                    let mut shared = lock(&state);
                    shared.network.errors += 1;
                    shared.last_error = Some("peer_unavailable");
                    continue;
                }
            }
        }
        if replicate_once(
            rpc.as_mut().unwrap(),
            &state,
            &mut snapshot,
            &mut catching_up,
        )
        .await
        .is_err()
        {
            rpc = None;
            let mut shared = lock(&state);
            shared.network.errors += 1;
            shared.last_error = Some("replication_retry");
        }
    }
}

#[derive(Clone)]
struct App {
    settings: Arc<Settings>,
    state: StateCell,
    broker: Arc<tokio::sync::Mutex<SiteBroker>>,
    cancel: CancellationToken,
}
fn response(code: StatusCode, body: Value) -> Response {
    (
        code,
        [
            ("cache-control", "no-store"),
            ("x-content-type-options", "nosniff"),
        ],
        Json(body),
    )
        .into_response()
}
async fn guard(State(app): State<App>, request: Request, next: Next) -> Response {
    let allowed = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .is_some_and(|info| info.0.ip() == IpAddr::V4(app.settings.observer_ip));
    let host = format!("{}:{}", app.settings.bind_ip, app.settings.control_port);
    let headers = request.headers();
    if !allowed
        || headers.get("host").and_then(|value| value.to_str().ok()) != Some(host.as_str())
        || ["origin", "referer", "sec-fetch-site"]
            .iter()
            .any(|name| headers.contains_key(*name))
    {
        return response(StatusCode::FORBIDDEN, json!({"error":"observer_required"}));
    }
    let (parts, body) = request.into_parts();
    let bytes = match tokio::time::timeout(Duration::from_secs(2), to_bytes(body, 1024)).await {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(_)) => {
            return response(StatusCode::PAYLOAD_TOO_LARGE, json!({"error":"body_limit"}));
        }
        Err(_) => return response(StatusCode::REQUEST_TIMEOUT, json!({"error":"body_timeout"})),
    };
    next.run(Request::from_parts(parts, Body::from(bytes)))
        .await
}
async fn status(State(app): State<App>) -> Response {
    let mut status = lock(&app.state).status();
    status["fault"] = app.broker.lock().await.fault_stats();
    status["process"] = process_metrics().await;
    response(StatusCode::OK, status)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FaultBody {
    mode: FaultMode,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum FaultMode {
    Up,
    Cut,
    Blackhole,
}
impl FaultMode {
    fn name(&self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Cut => "cut",
            Self::Blackhole => "blackhole",
        }
    }
}
async fn fault(State(app): State<App>, Json(body): Json<FaultBody>) -> Response {
    match app.broker.lock().await.set_fault(body.mode.name()) {
        Ok(()) => response(
            StatusCode::OK,
            json!({"mode":body.mode.name(),"physical_commands":0}),
        ),
        Err(_) => response(StatusCode::BAD_REQUEST, json!({"error":"fault_refused"})),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FreezeBody {
    enabled: bool,
}
async fn freeze(State(app): State<App>, Json(body): Json<FreezeBody>) -> Response {
    lock(&app.state).frozen = body.enabled;
    response(
        StatusCode::OK,
        json!({"frozen":body.enabled,"lease_renewal_continues":true}),
    )
}
async fn stop(State(app): State<App>) -> Response {
    app.cancel.cancel();
    response(StatusCode::OK, json!({"stopping":true}))
}

async fn process_metrics() -> Value {
    #[cfg(target_os = "linux")]
    {
        let status = tokio::fs::read_to_string("/proc/self/status")
            .await
            .unwrap_or_default();
        let rss = status.lines().find_map(|line| {
            line.strip_prefix("VmRSS:")?
                .split_whitespace()
                .next()?
                .parse::<u64>()
                .ok()?
                .checked_mul(1024)
        });
        let stat = tokio::fs::read_to_string("/proc/self/stat")
            .await
            .unwrap_or_default();
        let fields = stat
            .rsplit_once(')')
            .map(|(_, rest)| rest.split_whitespace().collect::<Vec<_>>())
            .unwrap_or_default();
        let ticks = fields
            .get(11)
            .and_then(|value| value.parse::<u64>().ok())
            .zip(fields.get(12).and_then(|value| value.parse::<u64>().ok()))
            .and_then(|(user, system)| user.checked_add(system));
        return json!({"rss_bytes":rss,"cpu_ticks":ticks,"cpu_source":"proc_self_stat"});
    }
    #[cfg(not(target_os = "linux"))]
    json!({"rss_bytes":null,"cpu_ticks":null,"cpu_source":"unavailable"})
}
pub async fn run(value: Value) -> Result<()> {
    let input: Input = serde_json::from_value(value).context("invalid site bench configuration")?;
    let settings = Arc::new(input.validate()?);
    let state = Arc::new(Mutex::new(Shared::new(settings.clone(), &input.bundle)?));
    drop(input);
    let mut broker = SiteBroker::start(
        SocketAddr::new(IpAddr::V4(settings.bind_ip), settings.mqtt_port),
        IpAddr::V4(settings.peer_ip),
        &settings.prefix,
    )
    .await?;
    let local = match ServerConnection::open(broker.port(), &settings.node, &settings.prefix).await
    {
        Ok(connection) => connection,
        Err(error) => {
            broker.stop().await;
            return Err(error);
        }
    };
    let listener = match TcpListener::bind((settings.bind_ip, settings.control_port)).await {
        Ok(listener) => listener,
        Err(error) => {
            broker.stop().await;
            return Err(error.into());
        }
    };
    let broker = Arc::new(tokio::sync::Mutex::new(broker));
    let cancel = CancellationToken::new();
    let app = App {
        settings: settings.clone(),
        state: state.clone(),
        broker: broker.clone(),
        cancel: cancel.clone(),
    };
    let tasks: Vec<JoinHandle<()>> = vec![
        tokio::spawn(local_ticker(state.clone(), cancel.clone())),
        tokio::spawn(incoming(local, state.clone(), cancel.clone())),
        tokio::spawn(outgoing(settings.clone(), state.clone(), cancel.clone())),
    ];
    let routes = Router::new()
        .route("/status", get(status))
        .route("/fault", post(fault))
        .route("/freeze", post(freeze))
        .route("/stop", post(stop))
        .layer(middleware::from_fn_with_state(app.clone(), guard))
        .with_state(app);
    let shutdown = cancel.clone();
    let ended = cancel.clone();
    let mut http = tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            routes.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async move { shutdown.cancelled().await })
        .await;
        ended.cancel();
    });
    println!(
        "SITE_READY {}",
        json!({"node":settings.node,"house":settings.house,"mode":"simulation","ttl_seconds":settings.ttl_seconds,"physical_commands":0})
    );
    tokio::select! {()=cancel.cancelled()=>{},_=tokio::time::sleep(Duration::from_secs(settings.ttl_seconds))=>{},_=shutdown_signal()=>{}}
    cancel.cancel();
    for task in tasks {
        task.abort();
        let _ = task.await;
    }
    if tokio::time::timeout(Duration::from_secs(2), &mut http)
        .await
        .is_err()
    {
        http.abort();
        let _ = http.await;
    }
    broker.lock().await.stop().await;
    Ok(())
}
async fn shutdown_signal() {
    #[cfg(unix)]
    if let Ok(mut term) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    {
        tokio::select! {_=tokio::signal::ctrl_c()=>{},_=term.recv()=>{}};
        return;
    }
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    fn settings() -> Settings {
        Settings {
            node: "node-a".into(),
            house: "house-a".into(),
            peer_node: "node-b".into(),
            peer_house: "house-b".into(),
            bind_ip: Ipv4Addr::new(100, 64, 1, 1),
            peer_ip: Ipv4Addr::new(100, 64, 1, 2),
            observer_ip: Ipv4Addr::new(100, 64, 1, 3),
            mqtt_port: mqtt_port(),
            peer_mqtt_port: mqtt_port(),
            control_port: control_port(),
            prefix: "moli-probe/synthetic-site-test".into(),
            ttl_seconds: 270,
        }
    }
    #[test]
    fn foreign_rpc_cannot_mutate_or_promote_the_local_house() {
        let settings = settings();
        for house in ["house-a", "unknown"] {
            assert!(
                validate_incoming(
                    &Operation::Update {
                        house: house.into(),
                        epoch: 1,
                        sequence: 1,
                        key: "test".into(),
                        value: 1.into()
                    },
                    &settings
                )
                .is_err()
            );
        }
        assert!(
            validate_incoming(
                &Operation::Update {
                    house: "house-b".into(),
                    epoch: 1,
                    sequence: 1,
                    key: "test".into(),
                    value: 1.into()
                },
                &settings
            )
            .is_ok()
        );
        assert!(
            validate_incoming(
                &Operation::SetRole {
                    house: "house-b".into(),
                    epoch: 1,
                    role: "active".into()
                },
                &settings
            )
            .is_err()
        );
        assert!(validate_incoming(&Operation::Stop, &settings).is_err());
    }
    #[test]
    fn local_progress_and_fake_effects_are_independent_of_the_foreign_house() {
        let mut shared = Shared::new(Arc::new(settings()), &Bundle::fixture()).unwrap();
        let foreign = shared.replica.summary("house-b").unwrap();
        shared.advance().unwrap();
        shared.advance().unwrap();
        assert_eq!(shared.effects, 2);
        assert_eq!(shared.replica.summary("house-a").unwrap()["sequence"], 2);
        assert_eq!(shared.replica.summary("house-b").unwrap(), foreign);
        assert_eq!(shared.authority.lease("house-b").unwrap().sequence, 0);
        shared.frozen = true;
        shared.advance().unwrap();
        assert_eq!(shared.effects, 2);
    }
    #[test]
    fn history_is_bounded_and_external_bindings_are_rejected() {
        let declared = settings();
        let mut input = Input {
            node: declared.node,
            house: declared.house,
            bind_ip: declared.bind_ip,
            peer_ip: declared.peer_ip,
            observer_ip: declared.observer_ip,
            loopback_fixture: false,
            mqtt_port: declared.mqtt_port,
            peer_mqtt_port: None,
            control_port: declared.control_port,
            prefix: declared.prefix,
            bundle: Bundle::fixture(),
            ttl_seconds: 480,
        };
        assert!(input.validate().unwrap().ttl_seconds / TICK_SECONDS < 256);
        assert_eq!(input.validate().unwrap().peer_mqtt_port, input.mqtt_port);
        input.ttl_seconds = 481;
        assert!(input.validate().is_err());
        let mut shared = Shared::new(Arc::new(settings()), &Bundle::fixture()).unwrap();
        for sequence in 0..300 {
            shared.ring.push_back(Delta {
                sequence,
                value: Value::Null,
                hash: String::new(),
                bytes: 8192,
            });
            shared.trim_ring();
        }
        assert!(shared.ring.len() <= RING_LENGTH && shared.ring_bytes() <= RING_BYTES);
        assert!(!vpn_ip(Ipv4Addr::UNSPECIFIED));
        assert!(!vpn_ip(Ipv4Addr::LOCALHOST));
        assert!(!vpn_ip(Ipv4Addr::new(203, 0, 113, 1)));
        for port in [0, 1414, 1415, 1416, 1883, 1884, 8790] {
            assert!(!safe_port(port));
        }
    }
    #[test]
    fn loopback_fixture_requires_explicit_opt_in_and_never_mixes_network_scopes() {
        let mut input = Input {
            node: "node-a".into(),
            house: "house-a".into(),
            bind_ip: Ipv4Addr::LOCALHOST,
            peer_ip: Ipv4Addr::new(127, 0, 0, 2),
            observer_ip: Ipv4Addr::LOCALHOST,
            loopback_fixture: false,
            mqtt_port: mqtt_port(),
            peer_mqtt_port: None,
            control_port: control_port(),
            prefix: "moli-probe/loopback-site-fixture".into(),
            bundle: Bundle::fixture(),
            ttl_seconds: 10,
        };
        assert!(input.validate().is_err());
        input.loopback_fixture = true;
        let fixture = Shared::new(Arc::new(input.validate().unwrap()), &input.bundle).unwrap();
        assert_eq!(fixture.status()["network_scope"], "loopback_fixture");
        let vpn = Ipv4Addr::new(100, 64, 1, 1);
        input.observer_ip = vpn;
        assert!(input.validate().is_err());
        input.observer_ip = Ipv4Addr::LOCALHOST;
        input.peer_ip = vpn;
        assert!(input.validate().is_err());
        input.peer_ip = Ipv4Addr::new(127, 0, 0, 2);
        input.bind_ip = vpn;
        assert!(input.validate().is_err());
        input.bind_ip = Ipv4Addr::LOCALHOST;
        input.peer_ip = Ipv4Addr::LOCALHOST;
        assert!(input.validate().is_err());
        input.peer_mqtt_port = Some(input.mqtt_port);
        assert!(input.validate().is_err());
        input.peer_mqtt_port = Some(53143);
        let first = input.validate().unwrap();
        input.node = "node-b".into();
        input.house = "house-b".into();
        input.mqtt_port = 53143;
        input.peer_mqtt_port = Some(first.mqtt_port);
        input.control_port = 53144;
        let second = input.validate().unwrap();
        assert_eq!(first.peer_mqtt_port, second.mqtt_port);
        assert_eq!(second.peer_mqtt_port, first.mqtt_port);
        for port in [0, 1414, 1415, 1416, 1883, 1884, 8790] {
            input.peer_mqtt_port = Some(port);
            assert!(input.validate().is_err());
        }
    }
}
