//! Synthetic two-site bench transport. No discovery, physical driver or live credential.
use crate::transport::{
    CHUNK, Connection, DEADLINE, LIMIT, Operation, PASSWORD, Received, Reply, Request, USER, WIRE,
    broker, normalized,
};
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::{self, Read},
    net::{IpAddr, Ipv4Addr, SocketAddr},
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    task::{Context as PollContext, Poll, Waker},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::TcpListener,
    task::{JoinHandle, JoinSet},
    time::timeout,
};
use tokio_util::sync::CancellationToken;

const PEER_LIMIT: usize = 16;

fn valid_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => {
            ip.is_loopback()
                || ip.is_private()
                || (ip.octets()[0] == 100 && (64..=127).contains(&ip.octets()[1]))
        }
        IpAddr::V6(ip) => ip.is_loopback() || ip.is_unique_local(),
    }
}

fn valid_port(port: u16, ephemeral: bool) -> bool {
    (ephemeral && port == 0)
        || (port >= 1024 && ![1414, 1415, 1416, 1883, 1884, 8790].contains(&port))
}

fn valid_prefix(prefix: &str) -> Result<()> {
    ensure!(
        prefix.starts_with("moli-probe/")
            && prefix.len() <= 128
            && !prefix.ends_with('/')
            && prefix.split('/').all(|part| !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')),
        "invalid site bench namespace"
    );
    Ok(())
}

fn valid_node(node: &str) -> Result<()> {
    ensure!(
        !node.is_empty()
            && node.len() <= 64
            && node
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
        "invalid site node ID"
    );
    Ok(())
}

fn random_id() -> Result<String> {
    let mut bytes = [0_u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// One outstanding request per client, with a random namespace and bounded pump.
pub(crate) struct Rpc {
    connection: Connection,
    target: String,
    prefix: String,
    namespace: String,
    sequence: u64,
}

impl Rpc {
    pub(crate) async fn open(host: IpAddr, port: u16, target: &str, prefix: &str) -> Result<Self> {
        ensure!(
            valid_ip(host) && valid_port(port, false),
            "RPC endpoint must be a declared private bench address"
        );
        valid_node(target)?;
        valid_prefix(prefix)?;
        let namespace = random_id()?;
        let connection = Connection::open_at(
            host,
            port,
            &format!("site-rpc-{namespace}"),
            format!("{prefix}/reply/{target}"),
            true,
        )
        .await?;
        Ok(Self {
            connection,
            target: target.into(),
            prefix: prefix.into(),
            namespace,
            sequence: 0,
        })
    }

    pub(crate) async fn request(&mut self, command: Operation) -> Result<Value> {
        self.sequence = self
            .sequence
            .checked_add(1)
            .context("site RPC counter exhausted")?;
        let id = format!("{}-{}", self.namespace, self.sequence);
        timeout(DEADLINE, async {
            self.connection
                .publish(
                    format!("{}/command/{}", self.prefix, self.target),
                    &Request {
                        id: id.clone(),
                        command,
                    },
                )
                .await?;
            loop {
                match self
                    .connection
                    .next()
                    .await
                    .context("site RPC connection closed")?
                {
                    Received::Subscribed => {}
                    Received::Payload(bytes) => {
                        let reply: Reply =
                            serde_json::from_slice(&bytes).context("invalid site reply")?;
                        if reply.id != id || reply.node != self.target {
                            continue;
                        }
                        if let Some(error) = reply.error {
                            bail!("site request rejected: {error}");
                        }
                        return reply.result.context("site reply has no result");
                    }
                }
            }
        })
        .await
        .context("site RPC deadline")?
    }

    pub(crate) async fn seed(
        &mut self,
        house: &str,
        epoch: u64,
        sequence: u64,
        document: &Value,
        role: &str,
    ) -> Result<Value> {
        let ordered = normalized(document)?;
        ensure!(
            ["active", "passive", "offline"].contains(&role),
            "invalid display role"
        );
        let data = serde_json::to_vec(&ordered)?;
        ensure!(data.len() <= LIMIT, "snapshot exceeds 8 MiB");
        let stream = random_id()?;
        self.request(Operation::SeedBegin {
            house: house.into(),
            stream: stream.clone(),
            epoch,
            sequence,
            bytes: data.len(),
            parts: data.len().div_ceil(CHUNK),
            sha256: format!("{:x}", Sha256::digest(&data)),
        })
        .await?;
        for (index, bytes) in data.chunks(CHUNK).enumerate() {
            self.request(Operation::SeedChunk {
                house: house.into(),
                stream: stream.clone(),
                index,
                data: STANDARD.encode(bytes),
            })
            .await?;
        }
        self.request(Operation::SeedCommit {
            house: house.into(),
            stream,
        })
        .await?;
        self.request(Operation::SetRole {
            house: house.into(),
            epoch,
            role: role.into(),
        })
        .await
    }
}

/// Local consumer of the site's broker. Its owner decides which operations are allowed.
pub(crate) struct ServerConnection {
    connection: Connection,
    node: String,
    reply_topic: String,
}

impl ServerConnection {
    pub(crate) async fn open(loopback_port: u16, node: &str, prefix: &str) -> Result<Self> {
        valid_node(node)?;
        valid_prefix(prefix)?;
        ensure!(
            valid_port(loopback_port, false),
            "invalid local broker port"
        );
        let id = format!("site-server-{}", random_id()?);
        let connection = Connection::open_at(
            Ipv4Addr::LOCALHOST.into(),
            loopback_port,
            &id,
            format!("{prefix}/command/{node}"),
            false,
        )
        .await?;
        Ok(Self {
            connection,
            node: node.into(),
            reply_topic: format!("{prefix}/reply/{node}"),
        })
    }

    pub(crate) async fn next(&mut self) -> Option<(String, Operation)> {
        while let Some(event) = self.connection.next().await {
            if let Received::Payload(payload) = event
                && let Ok(request) = serde_json::from_slice::<Request>(&payload)
                && valid_node(&request.id).is_ok()
            {
                return Some((request.id, request.command));
            }
        }
        None
    }

    pub(crate) async fn reply(&self, id: String, result: Result<Value>) -> Result<()> {
        let (result, error) = match result {
            Ok(value) => (Some(value), None),
            Err(error) => (None, Some(error.to_string())),
        };
        self.connection
            .publish(
                self.reply_topic.clone(),
                &Reply {
                    id,
                    node: self.node.clone(),
                    result,
                    error,
                },
            )
            .await
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Up,
    Cut,
    Blackhole,
}

impl Mode {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "up" => Ok(Self::Up),
            "cut" => Ok(Self::Cut),
            "blackhole" => Ok(Self::Blackhole),
            _ => bail!("unknown peer fault mode"),
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Cut => "cut",
            Self::Blackhole => "blackhole",
        }
    }
}

#[derive(Default)]
struct WakePair {
    read: Option<Waker>,
    write: Option<Waker>,
}

struct Control {
    mode: Mode,
    generation: u64,
    next: u64,
    closed: bool,
    waiters: BTreeMap<u64, WakePair>,
}

impl Default for Control {
    fn default() -> Self {
        Self {
            mode: Mode::Up,
            generation: 0,
            next: 0,
            closed: false,
            waiters: BTreeMap::new(),
        }
    }
}

#[derive(Default)]
struct Counters {
    accepted: AtomicU64,
    rejected: AtomicU64,
    forced: AtomicU64,
    forwarded: AtomicU64,
    active: AtomicUsize,
    maximum: AtomicUsize,
}

#[derive(Default)]
struct FaultGate {
    control: Mutex<Control>,
    counters: Arc<Counters>,
}

impl FaultGate {
    fn set(&self, mode: Mode) -> Result<()> {
        let wakes = {
            let mut state = self
                .control
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if mode == Mode::Cut {
                state.generation = state
                    .generation
                    .checked_add(1)
                    .context("fault generation exhausted")?;
            }
            state.mode = mode;
            state
                .waiters
                .values()
                .flat_map(|wake| [wake.read.clone(), wake.write.clone()])
                .flatten()
                .collect::<Vec<_>>()
        };
        for wake in wakes {
            wake.wake();
        }
        Ok(())
    }

    fn close(&self) {
        let wakes = {
            let mut state = self
                .control
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.closed = true;
            state
                .waiters
                .values()
                .flat_map(|wake| [wake.read.clone(), wake.write.clone()])
                .flatten()
                .collect::<Vec<_>>()
        };
        for wake in wakes {
            wake.wake();
        }
    }

    fn stats(&self) -> Value {
        let state = self
            .control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        json!({"mode":state.mode.name(),"generation":state.generation,
            "accepted_connections":self.counters.accepted.load(Ordering::Relaxed),
            "active_connections":self.counters.active.load(Ordering::Relaxed),
            "max_active_seen":self.counters.maximum.load(Ordering::Relaxed),
            "rejected_connections":self.counters.rejected.load(Ordering::Relaxed),
            "forced_closures":self.counters.forced.load(Ordering::Relaxed),
            "forwarded_bytes":self.counters.forwarded.load(Ordering::Relaxed),
            "gate_buffer_bytes_per_direction":0,"peer_connection_limit":PEER_LIMIT})
    }
}

/// No userspace relay buffer: pending I/O stays pending during a silence.
/// A cut invalidates the accepted generation and wakes both directions.
struct Gated<S> {
    stream: S,
    gate: Arc<FaultGate>,
    id: u64,
    generation: u64,
    failed: bool,
}

impl<S> Gated<S> {
    fn new(stream: S, gate: Arc<FaultGate>) -> Result<Self> {
        let (id, generation) = {
            let mut state = gate
                .control
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            ensure!(
                !state.closed && state.mode != Mode::Cut && state.waiters.len() < PEER_LIMIT,
                "peer gate unavailable"
            );
            state.next = state
                .next
                .checked_add(1)
                .context("peer gate counter exhausted")?;
            let id = state.next;
            let generation = state.generation;
            state.waiters.insert(id, WakePair::default());
            (id, generation)
        };
        let active = gate.counters.active.fetch_add(1, Ordering::Relaxed) + 1;
        gate.counters.maximum.fetch_max(active, Ordering::Relaxed);
        Ok(Self {
            stream,
            gate,
            id,
            generation,
            failed: false,
        })
    }

    fn blocked(&mut self, cx: &PollContext<'_>, writing: bool) -> io::Result<bool> {
        let mut state = self
            .gate
            .control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed || state.mode == Mode::Cut || state.generation != self.generation {
            if !self.failed {
                self.gate.counters.forced.fetch_add(1, Ordering::Relaxed);
                self.failed = true;
            }
            return Err(io::Error::from(io::ErrorKind::ConnectionAborted));
        }
        if let Some(wake) = state.waiters.get_mut(&self.id) {
            let slot = if writing {
                &mut wake.write
            } else {
                &mut wake.read
            };
            if slot.as_ref().is_none_or(|old| !old.will_wake(cx.waker())) {
                *slot = Some(cx.waker().clone());
            }
        }
        Ok(state.mode == Mode::Blackhole)
    }
}

impl<S> Drop for Gated<S> {
    fn drop(&mut self) {
        self.gate
            .control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .waiters
            .remove(&self.id);
        self.gate.counters.active.fetch_sub(1, Ordering::Relaxed);
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Gated<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut PollContext<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.blocked(cx, false) {
            Ok(false) => {}
            Ok(true) => return Poll::Pending,
            Err(error) => return Poll::Ready(Err(error)),
        }
        let before = buffer.filled().len();
        let result = Pin::new(&mut self.stream).poll_read(cx, buffer);
        if matches!(result, Poll::Ready(Ok(()))) {
            self.gate
                .counters
                .forwarded
                .fetch_add((buffer.filled().len() - before) as u64, Ordering::Relaxed);
        }
        result
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Gated<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut PollContext<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.blocked(cx, true) {
            Ok(false) => {}
            Ok(true) => return Poll::Pending,
            Err(error) => return Poll::Ready(Err(error)),
        }
        let result = Pin::new(&mut self.stream).poll_write(cx, bytes);
        if let Poll::Ready(Ok(size)) = result {
            self.gate
                .counters
                .forwarded
                .fetch_add(size as u64, Ordering::Relaxed);
        }
        result
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut PollContext<'_>) -> Poll<io::Result<()>> {
        match self.blocked(cx, true) {
            Ok(false) => Pin::new(&mut self.stream).poll_flush(cx),
            Ok(true) => Poll::Pending,
            Err(error) => Poll::Ready(Err(error)),
        }
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut PollContext<'_>) -> Poll<io::Result<()>> {
        // Explicit owner shutdown always releases the stream, including a silence.
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

pub(crate) struct SiteBroker {
    local_port: u16,
    peer_addr: SocketAddr,
    gate: Arc<FaultGate>,
    cancel: CancellationToken,
    task: Option<JoinHandle<()>>,
}

impl Drop for SiteBroker {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.gate.close();
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

impl SiteBroker {
    pub(crate) async fn start(bind: SocketAddr, peer: IpAddr, prefix: &str) -> Result<Self> {
        ensure!(
            valid_ip(bind.ip()) && valid_ip(peer) && valid_port(bind.port(), true),
            "site broker requires explicit private addresses and a test port"
        );
        valid_prefix(prefix)?;
        let local = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let local_port = local.local_addr()?.port();
        ensure!(valid_port(local_port, false), "reserved local broker port");
        let listener = TcpListener::bind(bind).await?;
        let peer_addr = listener.local_addr()?;
        ensure!(
            valid_port(peer_addr.port(), false),
            "reserved peer broker port"
        );
        let allowed = format!("{prefix}/");
        let broker = broker::Broker::new(Arc::new(move |_, topic, payload, retained| {
            ensure!(
                topic.starts_with(&allowed) && payload.len() <= WIRE && !retained,
                "invalid site bench publication"
            );
            Ok(payload.to_vec())
        }));
        broker.set_credentials(USER.into(), PASSWORD.into());
        let gate = Arc::new(FaultGate::default());
        let fault = gate.clone();
        let cancel = CancellationToken::new();
        let stopped = cancel.clone();
        let task = tokio::spawn(async move {
            let mut local_connections = JoinSet::new();
            let mut peer_connections = JoinSet::new();
            loop {
                tokio::select! {
                    biased;
                    () = stopped.cancelled() => break,
                    Some(_) = local_connections.join_next(), if !local_connections.is_empty() => {},
                    Some(_) = peer_connections.join_next(), if !peer_connections.is_empty() => {},
                    accepted = local.accept() => {
                        let Ok((stream, address)) = accepted else { break; };
                        if !address.ip().is_loopback() || local_connections.len() >= 16 { continue; }
                        let _ = stream.set_nodelay(true);
                        let broker = broker.clone();
                        local_connections.spawn(async move { let _ = broker.handle(stream, broker::Role::Bridge).await; });
                    },
                    accepted = listener.accept() => {
                        let Ok((stream, address)) = accepted else { break; };
                        fault.counters.accepted.fetch_add(1, Ordering::Relaxed);
                        if address.ip() != peer || peer_connections.len() >= PEER_LIMIT {
                            fault.counters.rejected.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                        let _ = stream.set_nodelay(true);
                        let Ok(stream) = Gated::new(stream, fault.clone()) else {
                            fault.counters.rejected.fetch_add(1, Ordering::Relaxed);
                            continue;
                        };
                        let broker = broker.clone();
                        peer_connections.spawn(async move { let _ = broker.handle(stream, broker::Role::Bridge).await; });
                    },
                }
            }
            local_connections.abort_all();
            peer_connections.abort_all();
            while local_connections.join_next().await.is_some() {}
            while peer_connections.join_next().await.is_some() {}
        });
        Ok(Self {
            local_port,
            peer_addr,
            gate,
            cancel,
            task: Some(task),
        })
    }

    pub(crate) fn port(&self) -> u16 {
        self.local_port
    }

    pub(crate) fn set_fault(&self, mode: &str) -> Result<()> {
        self.gate.set(Mode::parse(mode)?)
    }

    pub(crate) fn fault_stats(&self) -> Value {
        let mut stats = self.gate.stats();
        stats["peer_port"] = json!(self.peer_addr.port());
        stats
    }

    pub(crate) async fn stop(&mut self) {
        self.cancel.cancel();
        self.gate.close();
        if let Some(mut task) = self.task.take()
            && timeout(DEADLINE, &mut task).await.is_err()
        {
            task.abort();
            let _ = task.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::Replica;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn silence_resumes_the_same_stream_in_both_directions() {
        let (stream, mut wire) = tokio::io::duplex(32);
        let state = Arc::new(FaultGate::default());
        let mut gated = Gated::new(stream, state.clone()).unwrap();
        state.set(Mode::Blackhole).unwrap();
        wire.write_all(b"a").await.unwrap();
        let mut data = [0_u8; 1];
        assert!(
            timeout(Duration::from_millis(20), gated.read_exact(&mut data))
                .await
                .is_err()
        );
        assert!(
            timeout(Duration::from_millis(20), gated.write_all(b"b"))
                .await
                .is_err()
        );
        state.set(Mode::Up).unwrap();
        gated.read_exact(&mut data).await.unwrap();
        assert_eq!(&data, b"a");
        gated.write_all(b"b").await.unwrap();
        wire.read_exact(&mut data).await.unwrap();
        assert_eq!(&data, b"b");
        assert_eq!(state.stats()["active_connections"], 1);
        assert_eq!(state.stats()["generation"], 0);
    }

    #[tokio::test]
    async fn cut_wakes_existing_stream_and_old_generation_never_heals() {
        let (stream, _wire) = tokio::io::duplex(32);
        let state = Arc::new(FaultGate::default());
        let mut gated = Gated::new(stream, state.clone()).unwrap();
        state.set(Mode::Blackhole).unwrap();
        let waiting = tokio::spawn(async move { gated.read_u8().await });
        tokio::task::yield_now().await;
        state.set(Mode::Cut).unwrap();
        state.set(Mode::Up).unwrap();
        assert!(
            timeout(Duration::from_millis(250), waiting)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        assert_eq!(state.stats()["forced_closures"], 1);
        assert_eq!(state.stats()["active_connections"], 0);
    }

    #[tokio::test]
    async fn site_listener_rejects_unlisted_source_and_public_configuration() {
        let prefix = "moli-probe/site-test/allowlist";
        let mut broker = SiteBroker::start(
            "127.0.0.1:0".parse().unwrap(),
            "127.0.0.2".parse().unwrap(),
            prefix,
        )
        .await
        .unwrap();
        let mut stream = tokio::net::TcpStream::connect(broker.peer_addr)
            .await
            .unwrap();
        let mut byte = [0];
        assert_eq!(
            timeout(Duration::from_millis(250), stream.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        assert_eq!(broker.fault_stats()["rejected_connections"], 1);
        broker.stop().await;
        for bind in [
            "0.0.0.0:0",
            "8.8.8.8:30000",
            "127.0.0.1:1883",
            "127.0.0.1:1414",
        ] {
            assert!(
                SiteBroker::start(bind.parse().unwrap(), Ipv4Addr::LOCALHOST.into(), prefix)
                    .await
                    .is_err()
            );
        }
        assert!(
            SiteBroker::start(
                "127.0.0.1:0".parse().unwrap(),
                "1.1.1.1".parse().unwrap(),
                prefix
            )
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn real_mqtt_roundtrip_preserves_the_replica_contract() {
        let prefix = "moli-probe/site-test/roundtrip";
        let local = Ipv4Addr::LOCALHOST.into();
        let mut broker = SiteBroker::start("127.0.0.1:0".parse().unwrap(), local, prefix)
            .await
            .unwrap();
        let mut server = ServerConnection::open(broker.port(), "node-a", prefix)
            .await
            .unwrap();
        let observed = Arc::new(Mutex::new(Replica::default()));
        let shared = observed.clone();
        let service = tokio::spawn(async move {
            while let Some((id, operation)) = server.next().await {
                let result = {
                    let mut replica = shared.lock().unwrap();
                    match operation {
                        Operation::Status => Ok(json!({"houses":replica.summaries()})),
                        operation => replica.apply(operation),
                    }
                };
                server.reply(id, result).await.unwrap();
            }
        });
        let mut rpc = Rpc::open(local, broker.peer_addr.port(), "node-a", prefix)
            .await
            .unwrap();
        let document = json!({"long":"x".repeat(CHUNK * 3),"value":1});
        rpc.seed("house-a", 1, 0, &document, "passive")
            .await
            .unwrap();
        rpc.request(Operation::Update {
            house: "house-a".into(),
            epoch: 1,
            sequence: 1,
            key: "value".into(),
            value: json!(2),
        })
        .await
        .unwrap();
        let mut expected = Replica::default();
        expected
            .seed("house-a", 1, 0, &document, "passive")
            .unwrap();
        expected
            .apply(Operation::Update {
                house: "house-a".into(),
                epoch: 1,
                sequence: 1,
                key: "value".into(),
                value: json!(2),
            })
            .unwrap();
        let status = rpc.request(Operation::Status).await.unwrap();
        assert_eq!(status["houses"], json!(expected.summaries()));

        let accepted = broker.fault_stats()["accepted_connections"].clone();
        broker.set_fault("blackhole").unwrap();
        let (resumed, ()) = tokio::join!(rpc.request(Operation::Status), async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            broker.set_fault("up").unwrap();
        });
        assert_eq!(resumed.unwrap(), status);
        assert_eq!(broker.fault_stats()["accepted_connections"], accepted);
        broker.set_fault("cut").unwrap();
        assert!(rpc.request(Operation::Status).await.is_err());
        broker.set_fault("up").unwrap();
        let mut reconnected = Rpc::open(local, broker.peer_addr.port(), "node-a", prefix)
            .await
            .unwrap();
        assert_eq!(
            reconnected.request(Operation::Status).await.unwrap(),
            status
        );
        {
            let actual = observed.lock().unwrap();
            assert_eq!(
                actual.document("house-a").unwrap(),
                expected.document("house-a").unwrap()
            );
            assert_eq!(
                actual.summary("house-a").unwrap(),
                expected.summary("house-a").unwrap()
            );
        }
        service.abort();
        let _ = service.await;
        broker.stop().await;
    }
}
