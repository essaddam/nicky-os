//! Loopback-only process bench; none of these nodes owns a physical driver.
use crate::link_fault::{Fault, LinkFault};
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::Read,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant as StdInstant},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    process::{Child, Command},
    sync::mpsc,
    task::{JoinHandle, JoinSet},
    time::{Instant, timeout},
};
use tokio_util::sync::CancellationToken;

#[allow(dead_code)]
#[path = "../../mqtt-live-probe/src/gateway_runtime/broker.rs"]
pub(crate) mod broker;

pub(crate) const LIMIT: usize = 8 * 1024 * 1024;
pub(crate) const CHUNK: usize = 2048;
pub(crate) const WIRE: usize = 4096;
pub(crate) const DEADLINE: Duration = Duration::from_secs(5);
const HEARTBEAT: Duration = Duration::from_millis(500);
// Deliberately public test credentials; unrelated to any installation account.
pub(crate) const USER: &str = "ha-bench-fixture";
pub(crate) const PASSWORD: &str = "loopback-fixture-only";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NodeSpec {
    node: String,
    prefix: String,
    port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    local_house: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    replica_port: Option<u16>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    pub(crate) id: String,
    pub(crate) command: Operation,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Operation {
    SeedBegin {
        house: String,
        stream: String,
        epoch: u64,
        sequence: u64,
        bytes: usize,
        parts: usize,
        sha256: String,
    },
    SeedChunk {
        house: String,
        stream: String,
        index: usize,
        data: String,
    },
    SeedCommit {
        house: String,
        stream: String,
    },
    Update {
        house: String,
        epoch: u64,
        sequence: u64,
        key: String,
        value: Value,
    },
    SetRole {
        house: String,
        epoch: u64,
        role: String,
    },
    Status,
    Stop,
}

impl Operation {
    fn house(&self) -> Option<&str> {
        match self {
            Self::SeedBegin { house, .. }
            | Self::SeedChunk { house, .. }
            | Self::SeedCommit { house, .. }
            | Self::Update { house, .. }
            | Self::SetRole { house, .. } => Some(house),
            Self::Status | Self::Stop => None,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reply {
    pub(crate) id: String,
    pub(crate) node: String,
    pub(crate) result: Option<Value>,
    pub(crate) error: Option<String>,
}

pub(crate) enum Received {
    Subscribed,
    Payload(Vec<u8>),
}

// Update presence in the transport pump, independently of the coordinator's lock.
// Heartbeats never occupy the bounded request/reply queue.
fn route_reply(
    payload: &[u8],
    heartbeats: &std::sync::Mutex<BTreeMap<String, StdInstant>>,
) -> bool {
    let Ok(reply) = serde_json::from_slice::<Reply>(payload) else {
        return false;
    };
    if identifier(&reply.node).is_err()
        || reply.id.is_empty()
        || reply.id.len() > 128
        || reply.result.is_some() == reply.error.is_some()
    {
        return false;
    }
    let mut seen = heartbeats
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if seen.contains_key(&reply.node) || seen.len() < 8 {
        seen.insert(reply.node, StdInstant::now());
    }
    reply.id != "heartbeat"
}

pub(crate) struct Connection {
    client: AsyncClient,
    incoming: mpsc::Receiver<Received>,
    pump: JoinHandle<()>,
    heartbeats: Arc<std::sync::Mutex<BTreeMap<String, StdInstant>>>,
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.pump.abort();
    }
}

impl Connection {
    async fn open(port: u16, id: &str, subscription: String, track_replies: bool) -> Result<Self> {
        Self::open_at(
            std::net::Ipv4Addr::LOCALHOST.into(),
            port,
            id,
            subscription,
            track_replies,
        )
        .await
    }

    pub(crate) async fn open_at(
        host: std::net::IpAddr,
        port: u16,
        id: &str,
        subscription: String,
        track_replies: bool,
    ) -> Result<Self> {
        let mut options = MqttOptions::new(id, host.to_string(), port);
        options.set_credentials(USER, PASSWORD);
        options.set_clean_session(true);
        options.set_keep_alive(Duration::from_secs(10));
        options.set_max_packet_size(WIRE + 1024, WIRE + 1024);
        let (client, mut events) = AsyncClient::new(options, 16);
        let (sender, incoming) = mpsc::channel(32);
        let heartbeats = Arc::new(std::sync::Mutex::new(BTreeMap::new()));
        let activity = heartbeats.clone();
        // Poll forever in a separate task, including all outgoing/PUBACK events.
        let pump = tokio::spawn(async move {
            loop {
                let message = match events.poll().await {
                    Ok(Event::Incoming(Packet::SubAck(_))) => Some(Received::Subscribed),
                    Ok(Event::Incoming(Packet::Publish(p))) if p.payload.len() <= WIRE => {
                        if !track_replies || route_reply(&p.payload, &activity) {
                            Some(Received::Payload(p.payload.to_vec()))
                        } else {
                            None
                        }
                    }
                    Ok(_) => None,
                    Err(_) => break,
                };
                if let Some(message) = message
                    && sender.try_send(message).is_err()
                {
                    break;
                }
            }
        });
        let mut connection = Self {
            client,
            incoming,
            pump,
            heartbeats,
        };
        timeout(DEADLINE, async {
            connection
                .client
                .subscribe(subscription, QoS::AtLeastOnce)
                .await?;
            match connection.incoming.recv().await {
                Some(Received::Subscribed) => Ok(()),
                _ => bail!("MQTT subscription not acknowledged"),
            }
        })
        .await
        .context("MQTT subscription deadline")??;
        Ok(connection)
    }

    pub(crate) async fn publish<T: Serialize>(&self, topic: String, value: &T) -> Result<()> {
        let bytes = serde_json::to_vec(value).context("cannot encode bench message")?;
        ensure!(bytes.len() <= WIRE, "bench message exceeds 4096 bytes");
        timeout(
            DEADLINE,
            self.client.publish(topic, QoS::AtLeastOnce, false, bytes),
        )
        .await
        .context("MQTT publish deadline")?
        .context("MQTT publish failed")
    }

    pub(crate) async fn next(&mut self) -> Option<Received> {
        self.incoming.recv().await
    }
}

struct Process {
    child: Child,
    readers: Vec<JoinHandle<()>>,
    site: Option<String>,
}

pub struct Cluster {
    prefix: String,
    port: u16,
    serial: u64,
    connection: Connection,
    nodes: BTreeMap<String, Process>,
    cancel: CancellationToken,
    listener: JoinHandle<()>,
    relay: Option<LinkFault>,
}

impl Drop for Cluster {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.listener.abort();
        for process in self.nodes.values_mut() {
            let _ = process.child.start_kill();
            for reader in &process.readers {
                reader.abort();
            }
        }
    }
}

impl Cluster {
    pub async fn new() -> Result<Self> {
        let mut random = [0_u8; 16];
        std::fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
        let run: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let prefix = format!("moli-probe/ha-bench/{run}");
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let port = listener.local_addr()?.port();
        ensure!(
            ![1414, 1415, 1883, 1884].contains(&port),
            "reserved bench port"
        );
        let allowed = format!("{prefix}/");
        let broker = broker::Broker::new(Arc::new(move |_, topic, payload, retained| {
            ensure!(
                topic.starts_with(&allowed) && !retained && payload.len() <= WIRE,
                "invalid bench publication"
            );
            Ok(payload.to_vec())
        }));
        broker.set_credentials(USER.into(), PASSWORD.into());
        let cancel = CancellationToken::new();
        let stop = cancel.clone();
        let listener = tokio::spawn(async move {
            let mut peers = JoinSet::new();
            loop {
                tokio::select! {
                    () = stop.cancelled() => break,
                    Some(_) = peers.join_next(), if !peers.is_empty() => {},
                    accepted = listener.accept() => {
                        let Ok((stream, address)) = accepted else { break; };
                        if !address.ip().is_loopback() || peers.len() >= 32 { continue; }
                        let _ = stream.set_nodelay(true);
                        let broker = broker.clone();
                        peers.spawn(async move { let _ = broker.handle(stream, broker::Role::Bridge).await; });
                    }
                }
            }
            peers.abort_all();
            while peers.join_next().await.is_some() {}
        });
        let connection = match Connection::open(
            port,
            &format!("control-{run}"),
            format!("{prefix}/reply/+"),
            true,
        )
        .await
        {
            Ok(connection) => connection,
            Err(error) => {
                cancel.cancel();
                listener.abort();
                return Err(error);
            }
        };
        Ok(Self {
            prefix,
            port,
            serial: 0,
            connection,
            nodes: BTreeMap::new(),
            cancel,
            listener,
            relay: None,
        })
    }

    /// Dedicated CLI bench; the ordinary service never creates a fault relay.
    pub async fn new_network_test() -> Result<Self> {
        let mut cluster = Self::new().await?;
        cluster.relay = Some(LinkFault::start(cluster.port).await?);
        Ok(cluster)
    }

    pub fn set_link_fault(&self, fault: Fault) -> Result<()> {
        self.relay
            .as_ref()
            .context("network fault bench is not enabled")?
            .set(fault);
        Ok(())
    }

    pub fn link_stats(&self) -> Value {
        self.relay
            .as_ref()
            .map(LinkFault::stats)
            .unwrap_or(Value::Null)
    }

    pub fn heartbeats(&self) -> Arc<std::sync::Mutex<BTreeMap<String, StdInstant>>> {
        self.connection.heartbeats.clone()
    }

    pub async fn spawn_node(&mut self, node: &str) -> Result<()> {
        self.spawn(node, None).await
    }

    pub async fn spawn_site_node(&mut self, node: &str, site: &str) -> Result<()> {
        ensure!(["house-a", "house-b"].contains(&site), "invalid node site");
        ensure!(self.relay.is_some(), "network fault bench is not enabled");
        self.spawn(node, Some(site)).await
    }

    async fn spawn(&mut self, node: &str, site: Option<&str>) -> Result<()> {
        identifier(node)?;
        ensure!(
            self.nodes.len() < 8 && !self.nodes.contains_key(node),
            "node unavailable or limit reached"
        );
        let spec = NodeSpec {
            node: node.into(),
            prefix: self.prefix.clone(),
            port: self.port,
            local_house: site.map(str::to_owned),
            replica_port: site.and_then(|_| self.relay.as_ref().map(LinkFault::port)),
        };
        let mut command = Command::new(std::env::current_exe()?);
        command
            .arg("--worker")
            .env_clear()
            .env("LANG", "C")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().context("cannot spawn bench worker")?;
        let mut readers = Vec::new();
        if let Some(stdout) = child.stdout.take() {
            readers.push(tokio::spawn(drain(stdout)));
        }
        if let Some(stderr) = child.stderr.take() {
            readers.push(tokio::spawn(drain(stderr)));
        }
        self.nodes.insert(
            node.into(),
            Process {
                child,
                readers,
                site: site.map(str::to_owned),
            },
        );
        let result = timeout(DEADLINE, async {
            let mut input = self
                .nodes
                .get_mut(node)
                .context("node missing")?
                .child
                .stdin
                .take()
                .context("worker stdin missing")?;
            let mut line = serde_json::to_vec(&spec)?;
            line.push(b'\n');
            ensure!(line.len() <= WIRE, "worker specification too large");
            timeout(DEADLINE, input.write_all(&line))
                .await
                .context("worker input deadline")??;
            drop(input);
            self.receive(node, &format!("ready-{node}")).await?;
            Ok(())
        })
        .await
        .context("worker readiness deadline")
        .and_then(|result| result);
        if result.is_err() {
            let _ = self.kill(node).await;
        }
        result
    }

    async fn receive(&mut self, node: &str, id: &str) -> Result<Value> {
        timeout(DEADLINE, async {
            loop {
                match self
                    .connection
                    .incoming
                    .recv()
                    .await
                    .context("MQTT control connection closed")?
                {
                    Received::Payload(bytes) => {
                        let reply: Reply =
                            serde_json::from_slice(&bytes).context("invalid bench reply")?;
                        if reply.node != node || reply.id != id {
                            continue;
                        }
                        if let Some(error) = reply.error {
                            bail!("worker rejected request: {error}");
                        }
                        return reply.result.context("worker reply missing result");
                    }
                    Received::Subscribed => {}
                }
            }
        })
        .await
        .context("worker response deadline")?
    }

    async fn request(&mut self, node: &str, command: Operation) -> Result<Value> {
        self.request_route(node, command, false, DEADLINE).await
    }

    async fn request_route(
        &mut self,
        node: &str,
        command: Operation,
        force_direct: bool,
        deadline: Duration,
    ) -> Result<Value> {
        let process = self.nodes.get(node).context("node is not running")?;
        let remote = !force_direct
            && process
                .site
                .as_deref()
                .zip(command.house())
                .is_some_and(|(site, house)| site != house);
        let lane = if remote { "replica" } else { "command" };
        self.serial = self
            .serial
            .checked_add(1)
            .context("request counter exhausted")?;
        let id = self.serial.to_string();
        timeout(deadline, async {
            self.connection
                .publish(
                    format!("{}/{lane}/{node}", self.prefix),
                    &Request {
                        id: id.clone(),
                        command,
                    },
                )
                .await?;
            self.receive(node, &id).await
        })
        .await
        .context("worker request deadline")?
    }

    pub async fn seed(
        &mut self,
        node: &str,
        house: &str,
        epoch: u64,
        sequence: u64,
        document: &Value,
    ) -> Result<Value> {
        identifier(house)?;
        let document = normalized(document)?;
        let data = serde_json::to_vec(&document)?;
        ensure!(data.len() <= LIMIT, "snapshot exceeds 8 MiB");
        let stream = format!("seed-{}", self.serial + 1);
        self.request(
            node,
            Operation::SeedBegin {
                house: house.into(),
                stream: stream.clone(),
                epoch,
                sequence,
                bytes: data.len(),
                parts: data.len().div_ceil(CHUNK),
                sha256: digest(&data),
            },
        )
        .await?;
        for (index, chunk) in data.chunks(CHUNK).enumerate() {
            self.request(
                node,
                Operation::SeedChunk {
                    house: house.into(),
                    stream: stream.clone(),
                    index,
                    data: STANDARD.encode(chunk),
                },
            )
            .await?;
        }
        self.request(
            node,
            Operation::SeedCommit {
                house: house.into(),
                stream,
            },
        )
        .await
    }

    pub async fn update(
        &mut self,
        node: &str,
        house: &str,
        epoch: u64,
        sequence: u64,
        key: &str,
        value: &Value,
    ) -> Result<Value> {
        self.request(
            node,
            Operation::Update {
                house: house.into(),
                epoch,
                sequence,
                key: key.into(),
                value: value.clone(),
            },
        )
        .await
    }

    /// Short timeout for loss probes; callers resnapshot after restoring the link.
    pub async fn probe_update(
        &mut self,
        node: &str,
        house: &str,
        epoch: u64,
        sequence: u64,
        key: &str,
        value: &Value,
    ) -> Result<Value> {
        self.request_route(
            node,
            Operation::Update {
                house: house.into(),
                epoch,
                sequence,
                key: key.into(),
                value: value.clone(),
            },
            false,
            Duration::from_millis(300),
        )
        .await
    }

    /// Negative API probe: foreign-house mutations must fail on the direct lane.
    pub async fn direct_update_for_test(
        &mut self,
        node: &str,
        house: &str,
        epoch: u64,
        sequence: u64,
        key: &str,
        value: &Value,
    ) -> Result<Value> {
        self.request_route(
            node,
            Operation::Update {
                house: house.into(),
                epoch,
                sequence,
                key: key.into(),
                value: value.clone(),
            },
            true,
            DEADLINE,
        )
        .await
    }

    pub async fn set_role(
        &mut self,
        node: &str,
        house: &str,
        epoch: u64,
        role: &str,
    ) -> Result<Value> {
        self.request(
            node,
            Operation::SetRole {
                house: house.into(),
                epoch,
                role: role.into(),
            },
        )
        .await
    }

    pub async fn status(&mut self, node: &str) -> Result<Value> {
        self.request(node, Operation::Status).await
    }

    pub async fn kill(&mut self, node: &str) -> Result<()> {
        let mut process = self.nodes.remove(node).context("node is not running")?;
        let killed = process.child.start_kill();
        let waited = timeout(DEADLINE, process.child.wait()).await;
        for mut reader in process.readers.drain(..) {
            if timeout(Duration::from_millis(500), &mut reader)
                .await
                .is_err()
            {
                reader.abort();
            }
        }
        if let Err(error) = killed {
            ensure!(
                process.child.try_wait()?.is_some(),
                "worker kill failed: {error}"
            );
        }
        waited.context("worker exit deadline")??;
        self.connection
            .heartbeats
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(node);
        Ok(())
    }

    pub async fn shutdown(&mut self) {
        let nodes: Vec<_> = self.nodes.keys().cloned().collect();
        for node in nodes {
            let _ = self.kill(&node).await;
        }
        if let Some(mut relay) = self.relay.take() {
            relay.stop().await;
        }
        self.cancel.cancel();
        self.connection.pump.abort();
        let _ = timeout(DEADLINE, &mut self.listener).await;
        self.listener.abort();
    }
}

async fn drain(mut reader: impl AsyncRead + Unpin) {
    let mut buffer = [0_u8; 1024];
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
    }
}

fn identifier(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 64
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
        "invalid bench identifier"
    );
    Ok(())
}

pub(crate) fn normalized(document: &Value) -> Result<BTreeMap<String, Value>> {
    let object = document.as_object().context("snapshot must be an object")?;
    ensure!(
        object.len() <= 20_000 && object.keys().all(|key| !key.is_empty() && key.len() <= 256),
        "snapshot record bounds exceeded"
    );
    Ok(object
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect())
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

struct House {
    epoch: u64,
    sequence: u64,
    role: String,
    document: BTreeMap<String, Value>,
    bytes: usize,
    sha256: String,
    last_update: Option<(String, Value)>,
}

impl House {
    fn summary(&self, house: &str) -> Value {
        json!({"house":house,"epoch":self.epoch,"sequence":self.sequence,"role":self.role,
            "records":self.document.len(),"document_bytes":self.bytes,"sha256":self.sha256})
    }
}

struct Stage {
    stream: String,
    epoch: u64,
    sequence: u64,
    bytes: usize,
    parts: usize,
    next: usize,
    sha256: String,
    data: Vec<u8>,
    touched: Instant,
}

#[derive(Default)]
pub(crate) struct Replica {
    houses: BTreeMap<String, House>,
    stages: BTreeMap<String, Stage>,
    // Worker-level evidence: snapshots must not erase an accepted loss probe.
    applied_probe_updates: u64,
}

impl Replica {
    pub(crate) fn summary(&self, house: &str) -> Result<Value> {
        Ok(self
            .houses
            .get(house)
            .context("house not seeded")?
            .summary(house))
    }

    pub(crate) fn summaries(&self) -> Vec<Value> {
        self.houses
            .iter()
            .map(|(id, house)| house.summary(id))
            .collect()
    }

    pub(crate) fn document(&self, house: &str) -> Result<Value> {
        serde_json::to_value(&self.houses.get(house).context("house not seeded")?.document)
            .context("cannot encode replica document")
    }

    /// Uses the same staging, digest and sequence validation as MQTT snapshots.
    pub(crate) fn seed(
        &mut self,
        house: &str,
        epoch: u64,
        sequence: u64,
        document: &Value,
        role: &str,
    ) -> Result<Value> {
        ensure!(
            ["active", "passive", "offline"].contains(&role),
            "invalid display role"
        );
        let document = normalized(document)?;
        let data = serde_json::to_vec(&document)?;
        ensure!(data.len() <= LIMIT, "snapshot exceeds 8 MiB");
        let stream = "local-adapter".to_owned();
        self.apply(Operation::SeedBegin {
            house: house.into(),
            stream: stream.clone(),
            epoch,
            sequence,
            bytes: data.len(),
            parts: data.len().div_ceil(CHUNK),
            sha256: digest(&data),
        })?;
        for (index, part) in data.chunks(CHUNK).enumerate() {
            self.apply(Operation::SeedChunk {
                house: house.into(),
                stream: stream.clone(),
                index,
                data: STANDARD.encode(part),
            })?;
        }
        self.apply(Operation::SeedCommit {
            house: house.into(),
            stream,
        })?;
        self.apply(Operation::SetRole {
            house: house.into(),
            epoch,
            role: role.into(),
        })
    }

    pub(crate) fn apply(&mut self, operation: Operation) -> Result<Value> {
        self.stages
            .retain(|_, stage| stage.touched.elapsed() < DEADLINE);
        match operation {
            Operation::SeedBegin {
                house,
                stream,
                epoch,
                sequence,
                bytes,
                parts,
                sha256,
            } => {
                identifier(&house)?;
                identifier(&stream)?;
                ensure!(
                    bytes > 0 && bytes <= LIMIT && parts == bytes.div_ceil(CHUNK),
                    "invalid snapshot bounds"
                );
                ensure!(
                    sha256.len() == 64 && sha256.bytes().all(|b| b.is_ascii_hexdigit()),
                    "invalid snapshot digest"
                );
                ensure!(
                    !self.stages.contains_key(&house),
                    "snapshot already in progress"
                );
                let mut identities: std::collections::BTreeSet<_> =
                    self.houses.keys().chain(self.stages.keys()).collect();
                identities.insert(&house);
                ensure!(identities.len() <= 2, "two houses per node maximum");
                if let Some(current) = self.houses.get(&house) {
                    fresh(current, epoch, sequence)?;
                }
                self.stages.insert(
                    house,
                    Stage {
                        stream,
                        epoch,
                        sequence,
                        bytes,
                        parts,
                        next: 0,
                        sha256,
                        data: Vec::with_capacity(bytes),
                        touched: Instant::now(),
                    },
                );
                Ok(json!({"accepted":true}))
            }
            Operation::SeedChunk {
                house,
                stream,
                index,
                data,
            } => {
                let stage = self
                    .stages
                    .get_mut(&house)
                    .context("snapshot not started")?;
                ensure!(
                    stage.stream == stream && stage.next == index && index < stage.parts,
                    "snapshot stream or chunk order mismatch"
                );
                ensure!(
                    data.len() <= CHUNK.div_ceil(3) * 4,
                    "snapshot chunk too large"
                );
                let decoded = STANDARD.decode(data).context("invalid snapshot encoding")?;
                let expected = (stage.bytes - stage.data.len()).min(CHUNK);
                ensure!(decoded.len() == expected, "snapshot chunk length mismatch");
                stage.data.extend_from_slice(&decoded);
                stage.next += 1;
                stage.touched = Instant::now();
                Ok(json!({"accepted":true,"part":index}))
            }
            Operation::SeedCommit { house, stream } => {
                ensure!(
                    self.stages
                        .get(&house)
                        .is_some_and(|stage| stage.stream == stream),
                    "snapshot stream mismatch"
                );
                let stage = self.stages.remove(&house).context("snapshot not started")?;
                ensure!(
                    stage.next == stage.parts && stage.data.len() == stage.bytes,
                    "truncated snapshot"
                );
                ensure!(
                    digest(&stage.data) == stage.sha256,
                    "snapshot SHA-256 mismatch"
                );
                let value: Value = serde_json::from_slice(&stage.data)
                    .map_err(|_| anyhow::anyhow!("invalid snapshot JSON"))?;
                let document = normalized(&value)?;
                let canonical = serde_json::to_vec(&document)?;
                ensure!(canonical == stage.data, "snapshot is not canonical");
                if let Some(current) = self.houses.get(&house) {
                    fresh(current, stage.epoch, stage.sequence)?;
                    ensure!(
                        current.epoch != stage.epoch
                            || current.sequence != stage.sequence
                            || current.sha256 == stage.sha256,
                        "conflicting snapshot at same sequence"
                    );
                }
                let current = House {
                    epoch: stage.epoch,
                    sequence: stage.sequence,
                    role: "passive".into(),
                    document,
                    bytes: stage.bytes,
                    sha256: stage.sha256,
                    last_update: None,
                };
                let summary = current.summary(&house);
                self.houses.insert(house, current);
                Ok(summary)
            }
            Operation::Update {
                house,
                epoch,
                sequence,
                key,
                value,
            } => {
                ensure!(!self.stages.contains_key(&house), "snapshot in progress");
                ensure!(!key.is_empty() && key.len() <= 256, "invalid record key");
                let current = self.houses.get_mut(&house).context("house not seeded")?;
                ensure!(epoch == current.epoch, "update epoch mismatch");
                if sequence == current.sequence {
                    ensure!(
                        current
                            .last_update
                            .as_ref()
                            .is_some_and(|(previous_key, previous_value)| previous_key == &key
                                && previous_value == &value),
                        "conflicting duplicate update"
                    );
                    return Ok(current.summary(&house));
                }
                ensure!(
                    current.sequence.checked_add(1) == Some(sequence),
                    "update sequence gap or stale sequence"
                );
                ensure!(
                    current.document.contains_key(&key) || current.document.len() < 20_000,
                    "record limit reached"
                );
                let next_probe_updates =
                    if value.get("must_not_arrive").and_then(Value::as_bool) == Some(true) {
                        self.applied_probe_updates
                            .checked_add(1)
                            .context("probe evidence counter exhausted")?
                    } else {
                        self.applied_probe_updates
                    };
                let previous = current.document.insert(key.clone(), value.clone());
                let data = serde_json::to_vec(&current.document)?;
                if data.len() > LIMIT {
                    match previous {
                        Some(previous) => {
                            current.document.insert(key, previous);
                        }
                        None => {
                            current.document.remove(&key);
                        }
                    }
                    bail!("updated document exceeds 8 MiB");
                }
                current.sequence = sequence;
                current.bytes = data.len();
                current.sha256 = digest(&data);
                current.last_update = Some((key, value));
                self.applied_probe_updates = next_probe_updates;
                Ok(current.summary(&house))
            }
            Operation::SetRole { house, epoch, role } => {
                ensure!(
                    ["active", "passive", "offline"].contains(&role.as_str()),
                    "invalid display role"
                );
                let current = self.houses.get_mut(&house).context("house not seeded")?;
                ensure!(epoch >= current.epoch, "stale role epoch");
                if epoch != current.epoch {
                    current.last_update = None;
                }
                current.epoch = epoch;
                current.role = role;
                Ok(current.summary(&house))
            }
            Operation::Status | Operation::Stop => bail!("operation needs worker context"),
        }
    }
}

fn fresh(current: &House, epoch: u64, sequence: u64) -> Result<()> {
    ensure!(
        epoch > current.epoch || (epoch == current.epoch && sequence >= current.sequence),
        "stale snapshot epoch or sequence"
    );
    Ok(())
}

/// The remote connection may spend seconds opening or draining without delaying
/// the direct worker loop. Each reconnect creates a clean MQTT session.
struct Remote {
    incoming: mpsc::Receiver<Received>,
    outgoing: mpsc::Sender<Reply>,
    connected: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    task: JoinHandle<()>,
}

impl Drop for Remote {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Remote {
    fn start(port: u16, node: &str, prefix: &str) -> Self {
        let (incoming_tx, incoming) = mpsc::channel(32);
        let (outgoing, mut outgoing_rx) = mpsc::channel::<Reply>(16);
        let connected = Arc::new(AtomicBool::new(false));
        let state = connected.clone();
        let generation = Arc::new(AtomicU64::new(0));
        let incarnation = generation.clone();
        let client_id = format!("replica-{node}-{port}");
        let subscription = format!("{prefix}/replica/{node}");
        let reply_topic = format!("{prefix}/reply/{node}");
        let task = tokio::spawn(async move {
            loop {
                if incoming_tx.is_closed() {
                    break;
                }
                if let Ok(mut connection) =
                    Connection::open(port, &client_id, subscription.clone(), false).await
                {
                    // Increment only after CONNECT and SUBACK have completed.
                    if incarnation
                        .try_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                            value.checked_add(1)
                        })
                        .is_err()
                    {
                        break;
                    }
                    state.store(true, Ordering::Release);
                    loop {
                        tokio::select! {
                            event = connection.incoming.recv() => match event {
                                Some(event) => if incoming_tx.try_send(event).is_err() { break; },
                                None => break,
                            },
                            reply = outgoing_rx.recv() => match reply {
                                Some(reply) => if connection.publish(reply_topic.clone(), &reply).await.is_err() { break; },
                                None => return,
                            },
                        }
                    }
                }
                state.store(false, Ordering::Release);
                // A reply from a dead TCP incarnation must not cross into the next.
                while outgoing_rx.try_recv().is_ok() {}
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        });
        Self {
            incoming,
            outgoing,
            connected,
            generation,
            task,
        }
    }
}

async fn receive_remote(remote: &mut Option<Remote>) -> Option<Received> {
    match remote {
        Some(remote) => remote.incoming.recv().await,
        None => std::future::pending().await,
    }
}

fn validate_lane(operation: &Operation, local_house: Option<&str>, remote: bool) -> Result<()> {
    let Some(local_house) = local_house else {
        ensure!(!remote, "legacy worker has no replica lane");
        return Ok(());
    };
    if let Some(house) = operation.house() {
        ensure!(
            ["house-a", "house-b"].contains(&house),
            "unknown network-test house"
        );
        ensure!(
            (house != local_house) == remote,
            "house operation received on the wrong transport lane"
        );
    } else {
        ensure!(!remote, "worker control requires the direct lane");
    }
    Ok(())
}

pub async fn worker() -> Result<()> {
    let mut input = Vec::new();
    timeout(
        DEADLINE,
        tokio::io::stdin()
            .take((WIRE + 1) as u64)
            .read_to_end(&mut input),
    )
    .await
    .context("worker input deadline")??;
    ensure!(input.len() <= WIRE, "worker specification too large");
    let spec: NodeSpec = serde_json::from_slice(&input).context("invalid worker specification")?;
    identifier(&spec.node)?;
    ensure!(
        spec.prefix.starts_with("moli-probe/ha-bench/")
            && spec.prefix.len() <= 96
            && !spec.prefix.contains(['+', '#'])
            && ![0, 1414, 1415, 1883, 1884].contains(&spec.port),
        "invalid worker transport"
    );
    ensure!(
        spec.local_house.is_some() == spec.replica_port.is_some(),
        "incomplete site transport specification"
    );
    if let Some(house) = &spec.local_house {
        ensure!(
            ["house-a", "house-b"].contains(&house.as_str()),
            "invalid worker site"
        );
    }
    if let Some(port) = spec.replica_port {
        ensure!(
            ![0, 1414, 1415, 1883, 1884].contains(&port) && port != spec.port,
            "invalid replica port"
        );
    }
    let mut connection = Connection::open(
        spec.port,
        &format!("worker-{}-{}", spec.node, spec.port),
        format!("{}/command/{}", spec.prefix, spec.node),
        false,
    )
    .await?;
    let reply_topic = format!("{}/reply/{}", spec.prefix, spec.node);
    connection
        .publish(
            reply_topic.clone(),
            &Reply {
                id: format!("ready-{}", spec.node),
                node: spec.node.clone(),
                result: Some(json!({"ready":true})),
                error: None,
            },
        )
        .await?;
    let mut replica = Replica::default();
    let mut remote = spec
        .replica_port
        .map(|port| Remote::start(port, &spec.node, &spec.prefix));
    let mut heartbeat = tokio::time::interval(HEARTBEAT);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        let has_remote = remote.is_some();
        let (event, remote_lane) = tokio::select! {
            event = connection.incoming.recv() => match event {
                Some(event) => (event, false),
                None => break,
            },
            event = receive_remote(&mut remote), if has_remote => match event {
                Some(event) => (event, true),
                None => {
                    remote = spec.replica_port.map(|port| Remote::start(port, &spec.node, &spec.prefix));
                    continue;
                },
            },
            _ = heartbeat.tick() => {
                connection.publish(reply_topic.clone(), &Reply {
                    id: "heartbeat".into(),
                    node: spec.node.clone(),
                    result: Some(json!({"alive":true})),
                    error: None,
                }).await?;
                continue;
            },
        };
        let Received::Payload(payload) = event else {
            continue;
        };
        let request: Request = match serde_json::from_slice(&payload) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if request.id.len() > 64 {
            continue;
        }
        let valid_lane = validate_lane(&request.command, spec.local_house.as_deref(), remote_lane);
        let stopping = valid_lane.is_ok() && matches!(request.command, Operation::Stop);
        let result = match valid_lane {
            Err(error) => Err(error),
            Ok(()) => match request.command {
                Operation::Status => {
                    let (rss_bytes, cpu_seconds) = process_metrics().await.unwrap_or((None, None));
                    Ok(
                        json!({"node":spec.node,"houses":replica.houses.iter().map(|(id, house)| house.summary(id)).collect::<Vec<_>>(),
                    "allocator_live_bytes":crate::memory::live(),"allocator_peak_bytes":crate::memory::peak(),
                    "rss_bytes":rss_bytes,"cpu_seconds":cpu_seconds,
                    "replica_connected":remote.as_ref().is_some_and(|link| link.connected.load(Ordering::Acquire)),
                    "replica_generation":remote.as_ref().map_or(0,|link|link.generation.load(Ordering::Acquire)),
                    "applied_probe_updates":replica.applied_probe_updates}),
                    )
                }
                Operation::Stop => Ok(json!({"stopped":true})),
                operation => replica.apply(operation),
            },
        };
        let (result, error) = match result {
            Ok(value) => (Some(value), None),
            Err(error) => (None, Some(error.to_string())),
        };
        let reply = Reply {
            id: request.id,
            node: spec.node.clone(),
            result,
            error,
        };
        if remote_lane {
            if let Some(link) = &remote {
                // A blackholed remote writer cannot hold the direct loop or HB.
                let _ = link.outgoing.try_send(reply);
            }
        } else {
            connection.publish(reply_topic.clone(), &reply).await?;
        }
        if stopping {
            break;
        }
    }
    Ok(())
}

async fn process_metrics() -> Result<(Option<u64>, Option<f64>)> {
    let mut child = Command::new("/bin/ps")
        .args([
            "-p",
            &std::process::id().to_string(),
            "-o",
            "rss=",
            "-o",
            "time=",
        ])
        .env_clear()
        .env("LANG", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    timeout(Duration::from_millis(500), async {
        let mut output = Vec::new();
        child
            .stdout
            .take()
            .context("metrics stdout missing")?
            .take(4097)
            .read_to_end(&mut output)
            .await?;
        ensure!(
            output.len() <= 4096 && child.wait().await?.success(),
            "metrics command failed"
        );
        let text = std::str::from_utf8(&output)?;
        let mut fields = text.split_whitespace();
        let rss = fields
            .next()
            .context("missing RSS")?
            .parse::<u64>()?
            .checked_mul(1024)
            .context("RSS overflow")?;
        let cpu = parse_cpu(fields.next().context("missing CPU time")?)?;
        Ok((Some(rss), Some(cpu)))
    })
    .await
    .context("metrics deadline")?
}

fn parse_cpu(input: &str) -> Result<f64> {
    let (days, clock) = match input.split_once('-') {
        Some((days, clock)) => (days.parse::<f64>()?, clock),
        None => (0.0, input),
    };
    let mut total = 0.0;
    for field in clock.split(':') {
        total = total * 60.0 + field.parse::<f64>()?;
    }
    Ok(days * 86400.0 + total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepted_probe_evidence_survives_snapshot_replacement() {
        let mut replica = Replica::default();
        let original = br#"{"value":1}"#;
        commit(&mut replica, "house-a", original, digest(original)).unwrap();
        replica
            .apply(Operation::Update {
                house: "house-a".into(),
                epoch: 1,
                sequence: 11,
                key: "value".into(),
                value: json!({"must_not_arrive":true}),
            })
            .unwrap();
        assert_eq!(replica.applied_probe_updates, 1);

        replica
            .apply(Operation::SeedBegin {
                house: "house-a".into(),
                stream: "replacement".into(),
                epoch: 2,
                sequence: 20,
                bytes: original.len(),
                parts: 1,
                sha256: digest(original),
            })
            .unwrap();
        replica
            .apply(Operation::SeedChunk {
                house: "house-a".into(),
                stream: "replacement".into(),
                index: 0,
                data: STANDARD.encode(original),
            })
            .unwrap();
        replica
            .apply(Operation::SeedCommit {
                house: "house-a".into(),
                stream: "replacement".into(),
            })
            .unwrap();
        assert_eq!(replica.houses["house-a"].document["value"], 1);
        assert_eq!(replica.applied_probe_updates, 1);
    }

    #[test]
    fn probe_counter_overflow_rejects_update_before_document_mutation() {
        let mut replica = Replica::default();
        let original = br#"{"value":1}"#;
        commit(&mut replica, "house-a", original, digest(original)).unwrap();
        replica.applied_probe_updates = u64::MAX;
        assert!(
            replica
                .apply(Operation::Update {
                    house: "house-a".into(),
                    epoch: 1,
                    sequence: 11,
                    key: "value".into(),
                    value: json!({"must_not_arrive":true}),
                })
                .is_err()
        );
        assert_eq!(replica.houses["house-a"].sequence, 10);
        assert_eq!(replica.houses["house-a"].document["value"], 1);
        assert_eq!(replica.applied_probe_updates, u64::MAX);
    }

    #[test]
    fn network_house_mutations_cannot_bypass_the_replica_lane() {
        let mutation = |house: &str| Operation::Update {
            house: house.into(),
            epoch: 1,
            sequence: 1,
            key: "value".into(),
            value: json!(2),
        };
        assert!(validate_lane(&mutation("house-a"), Some("house-a"), false).is_ok());
        assert!(validate_lane(&mutation("house-b"), Some("house-a"), false).is_err());
        assert!(validate_lane(&mutation("house-a"), Some("house-a"), true).is_err());
        assert!(validate_lane(&mutation("house-b"), Some("house-a"), true).is_ok());
        let role = Operation::SetRole {
            house: "house-b".into(),
            epoch: 1,
            role: "passive".into(),
        };
        assert!(validate_lane(&role, Some("house-a"), false).is_err());
        assert!(validate_lane(&role, Some("house-a"), true).is_ok());
        assert!(validate_lane(&Operation::Status, Some("house-a"), false).is_ok());
        assert!(validate_lane(&Operation::Stop, Some("house-a"), true).is_err());
        assert!(validate_lane(&mutation("house-b"), None, false).is_ok());
    }

    #[test]
    fn legacy_worker_spec_keeps_optional_network_fields_absent() {
        let bytes = br#"{"node":"node-a","prefix":"moli-probe/ha-bench/test","port":32123}"#;
        let spec: NodeSpec = serde_json::from_slice(bytes).unwrap();
        assert!(spec.local_house.is_none() && spec.replica_port.is_none());
        let encoded = serde_json::to_value(spec).unwrap();
        assert!(encoded.get("local_house").is_none() && encoded.get("replica_port").is_none());
    }

    #[test]
    fn heartbeat_updates_presence_without_entering_request_queue() {
        let seen = std::sync::Mutex::new(BTreeMap::new());
        let encode = |id: &str, node: &str| {
            serde_json::to_vec(&Reply {
                id: id.into(),
                node: node.into(),
                result: Some(json!({"alive":true})),
                error: None,
            })
            .unwrap()
        };
        assert!(!route_reply(&encode("heartbeat", "node-a"), &seen));
        assert!(seen.lock().unwrap().contains_key("node-a"));
        assert!(route_reply(&encode("42", "node-b"), &seen));
        assert!(seen.lock().unwrap().contains_key("node-b"));
        assert!(!route_reply(&encode("heartbeat", "../invalid"), &seen));
        assert!(!route_reply(
            br#"{"node":"unknown","id":"heartbeat"}"#,
            &seen
        ));
        assert_eq!(seen.lock().unwrap().len(), 2);
    }

    #[test]
    fn presence_map_is_bounded_to_eight_valid_node_ids() {
        let seen = std::sync::Mutex::new(BTreeMap::new());
        for index in 0..10 {
            let payload = serde_json::to_vec(&Reply {
                id: "heartbeat".into(),
                node: format!("node-{index}"),
                result: Some(json!({"alive":true})),
                error: None,
            })
            .unwrap();
            assert!(!route_reply(&payload, &seen));
        }
        assert_eq!(seen.lock().unwrap().len(), 8);
        assert!(!seen.lock().unwrap().contains_key("node-8"));
    }

    fn begin(replica: &mut Replica, house: &str, bytes: &[u8], sha256: String) {
        replica
            .apply(Operation::SeedBegin {
                house: house.into(),
                stream: "stream".into(),
                epoch: 1,
                sequence: 10,
                bytes: bytes.len(),
                parts: bytes.len().div_ceil(CHUNK),
                sha256,
            })
            .unwrap();
    }

    fn commit(replica: &mut Replica, house: &str, bytes: &[u8], checksum: String) -> Result<Value> {
        begin(replica, house, bytes, checksum);
        for (index, chunk) in bytes.chunks(CHUNK).enumerate() {
            replica.apply(Operation::SeedChunk {
                house: house.into(),
                stream: "stream".into(),
                index,
                data: STANDARD.encode(chunk),
            })?;
        }
        replica.apply(Operation::SeedCommit {
            house: house.into(),
            stream: "stream".into(),
        })
    }

    #[test]
    fn fragmented_snapshot_is_atomic_and_bad_digest_preserves_previous_copy() {
        let mut replica = Replica::default();
        let bytes = serde_json::to_vec(&json!({"large":"x".repeat(CHUNK * 3)})).unwrap();
        begin(&mut replica, "house-a", &bytes, digest(&bytes));
        assert!(replica.houses.is_empty());
        assert!(
            replica
                .apply(Operation::SeedCommit {
                    house: "house-a".into(),
                    stream: "stream".into()
                })
                .is_err()
        );
        commit(&mut replica, "house-a", &bytes, digest(&bytes)).unwrap();
        let original = replica.houses["house-a"].sha256.clone();
        assert!(commit(&mut replica, "house-a", &bytes, "0".repeat(64)).is_err());
        assert_eq!(replica.houses["house-a"].sha256, original);
    }

    #[test]
    fn sequence_and_duplicate_checks_do_not_cross_houses() {
        let mut replica = Replica::default();
        let bytes = br#"{"value":1}"#;
        for house in ["house-a", "house-b"] {
            commit(&mut replica, house, bytes, digest(bytes)).unwrap();
        }
        let update = |house: &str, sequence, value| Operation::Update {
            house: house.into(),
            epoch: 1,
            sequence,
            key: "value".into(),
            value: json!(value),
        };
        assert!(replica.apply(update("house-a", 12, 2)).is_err());
        replica.apply(update("house-a", 11, 2)).unwrap();
        replica.apply(update("house-a", 11, 2)).unwrap();
        assert!(replica.apply(update("house-a", 11, 3)).is_err());
        assert_eq!(replica.houses["house-b"].sequence, 10);
        assert_eq!(replica.houses["house-b"].document["value"], 1);
    }

    #[test]
    fn snapshot_streams_cannot_mix_and_old_epochs_are_rejected() {
        let mut replica = Replica::default();
        let bytes = br#"{"value":1}"#;
        begin(&mut replica, "house-a", bytes, digest(bytes));
        assert!(
            replica
                .apply(Operation::SeedChunk {
                    house: "house-a".into(),
                    stream: "other".into(),
                    index: 0,
                    data: STANDARD.encode(bytes)
                })
                .is_err()
        );
        replica.stages.clear();
        commit(&mut replica, "house-a", bytes, digest(bytes)).unwrap();
        replica
            .apply(Operation::SetRole {
                house: "house-a".into(),
                epoch: 2,
                role: "active".into(),
            })
            .unwrap();
        assert!(
            replica
                .apply(Operation::SeedBegin {
                    house: "house-a".into(),
                    stream: "old".into(),
                    epoch: 1,
                    sequence: 99,
                    bytes: bytes.len(),
                    parts: 1,
                    sha256: digest(bytes)
                })
                .is_err()
        );
    }
}
