//! Bounded, read-only MQTT source for isolated synthetic load tests.
use super::load_fixture::SourceFixture;
use anyhow::{Result, bail, ensure};
use serde_json::{Value as Json, json};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Semaphore, watch};
use tokio::task::{JoinHandle, JoinSet};
use tokio_util::sync::CancellationToken;

const MAX_PACKET: usize = 4 * 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(10);
const FILTERS: [&str; 3] = [
    "homeassistant/statestream/#",
    "homeassistant/light/#",
    "homeassistant/switch/#",
];

#[derive(Default)]
struct Counters {
    sessions: u64,
    active: HashSet<(usize, usize)>,
    ready: HashSet<(usize, usize)>,
    initial: u64,
    updates: u64,
    bytes: u64,
    commands_refused: u64,
    protocol_errors: u64,
    slow_writes: u64,
    session_waves: u64,
    house_waves: HashMap<(usize, usize), u64>,
    last_send_ms: Option<u128>,
    last_wave_ms: Option<u128>,
}

struct Shared {
    started: Instant,
    paused: AtomicBool,
    counters: Mutex<Counters>,
    houses: usize,
}

impl Shared {
    fn record(&self, update: impl FnOnce(&mut Counters)) {
        update(
            &mut self
                .counters
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
    }
}

pub(super) struct SourceServer {
    port: u16,
    shared: Arc<Shared>,
    begin: watch::Sender<Option<(usize, Instant)>>,
    cancel: CancellationToken,
    task: JoinHandle<()>,
}

impl SourceServer {
    pub(super) async fn start(fixtures: Vec<Arc<SourceFixture>>, copies: usize) -> Result<Self> {
        ensure!(
            !fixtures.is_empty() && copies > 0 && copies <= 20 / fixtures.len(),
            "synthetic source requires one to twenty houses"
        );
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let port = listener.local_addr()?.port();
        let shared = Arc::new(Shared {
            started: Instant::now(),
            paused: AtomicBool::new(false),
            counters: Mutex::new(Counters::default()),
            houses: fixtures.len() * copies,
        });
        let (begin, receiver) = watch::channel(None);
        let cancel = CancellationToken::new();
        let task = tokio::spawn(serve(
            listener,
            fixtures,
            copies,
            shared.clone(),
            receiver,
            cancel.clone(),
        ));
        Ok(Self {
            port,
            shared,
            begin,
            cancel,
            task,
        })
    }

    pub(super) fn port(&self) -> u16 {
        self.port
    }

    pub(super) fn begin(&self, changes_per_second: usize) {
        self.begin.send_if_modified(|state| {
            if state.is_some() {
                return false;
            }
            *state = Some((changes_per_second, Instant::now()));
            true
        });
    }

    /// Stop future update ticks while leaving MQTT sessions available for draining.
    pub(super) fn pause(&self) {
        self.shared.paused.store(true, Ordering::Release);
    }

    pub(super) fn snapshot(&self) -> Json {
        let c = self
            .shared
            .counters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let begin_ms = self
            .begin
            .borrow()
            .map(|(_, start)| start.duration_since(self.shared.started).as_millis());
        json!({
            "sessions": c.sessions, "active_sessions": c.active.len(), "ready_sessions": c.ready.len(),
            "initial_messages_sent": c.initial, "update_messages_sent": c.updates,
            "bytes_sent": c.bytes, "commands_refused": c.commands_refused,
            "protocol_errors": c.protocol_errors, "slow_writes": c.slow_writes,
            "waves_completed": if c.house_waves.len() == self.shared.houses {
                c.house_waves.values().copied().min().unwrap_or(0)
            } else { 0 },
            "session_waves_completed": c.session_waves,
            "elapsed_ms": self.shared.started.elapsed().as_millis(), "begin_ms": begin_ms,
            "last_send_ms": c.last_send_ms, "last_wave_ms": c.last_wave_ms,
            "paused": self.shared.paused.load(Ordering::Acquire),
        })
    }

    pub(super) async fn stop(self) {
        self.cancel.cancel();
        let _ = self.task.await;
    }
}

async fn serve(
    listener: TcpListener,
    fixtures: Vec<Arc<SourceFixture>>,
    copies: usize,
    shared: Arc<Shared>,
    begin: watch::Receiver<Option<(usize, Instant)>>,
    cancel: CancellationToken,
) {
    let permits = Arc::new(Semaphore::new(shared.houses));
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            () = cancel.cancelled() => break,
            Some(_) = tasks.join_next(), if !tasks.is_empty() => {},
            accepted = listener.accept() => {
                let Ok((stream, peer)) = accepted else { break; };
                if !peer.ip().is_loopback() { continue; }
                let Ok(permit) = permits.clone().try_acquire_owned() else { continue; };
                let fixtures = fixtures.clone();
                let shared = shared.clone();
                let begin = begin.clone();
                let cancel = cancel.clone();
                tasks.spawn(async move {
                    let _permit = permit;
                    let mut identity = None;
                    let result = tokio::select! {
                        biased;
                        () = cancel.cancelled() => Ok(()),
                        result = session(stream, &fixtures, copies, &shared, begin, &mut identity) => result,
                    };
                    shared.record(|c| {
                        if let Some(id) = identity { c.active.remove(&id); c.ready.remove(&id); }
                        if result.is_err() { c.protocol_errors += 1; }
                    });
                });
            }
        }
    }
    cancel.cancel();
    while tasks.join_next().await.is_some() {}
}

async fn session(
    mut stream: TcpStream,
    fixtures: &[Arc<SourceFixture>],
    copies: usize,
    shared: &Shared,
    mut begin: watch::Receiver<Option<(usize, Instant)>>,
    identity: &mut Option<(usize, usize)>,
) -> Result<()> {
    stream.set_nodelay(true)?;
    let (header, body) = tokio::time::timeout(DEADLINE, read_first(&mut stream)).await??;
    ensure!(header == 0x10, "CONNECT required");
    let id = connect_identity(&body, fixtures.len(), copies)?;
    {
        let mut c = shared
            .counters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        ensure!(c.active.insert(id), "duplicate synthetic identity");
        c.sessions += 1;
    }
    *identity = Some(id);
    write_frame(&mut stream, 0x20, &[0, 0], shared).await?;
    let mut input = Vec::with_capacity(8192);
    let mut chunk = [0_u8; 8192];
    let mut subscriptions = HashSet::new();
    let mut ready = false;
    let mut step = 0_u64;
    let mut next_tick = tokio::time::Instant::now() + Duration::from_secs(86400);
    let mut rate = 0;
    let mut begun = false;
    loop {
        if !begun
            && ready
            && let Some((changes, start)) = *begin.borrow()
        {
            begun = true;
            rate = changes;
            next_tick = tokio::time::Instant::from_std(start + Duration::from_secs(1));
        }
        tokio::select! {
            changed = begin.changed(), if !begun => { changed?; },
            () = tokio::time::sleep_until(next_tick), if begun => {
                next_tick += Duration::from_secs(1);
                if shared.paused.load(Ordering::Acquire) { continue; }
                step += 1;
                send_messages(&mut stream, fixtures[id.0].messages(id.1, step, rate), false, shared).await?;
                if step.is_multiple_of(6) {
                    let wave = step / 6 - 1;
                    send_messages(&mut stream, workflow_messages(id.0, id.1, Some(wave))?, false, shared).await?;
                    shared.record(|c| {
                        c.session_waves += 1;
                        c.house_waves.insert(id, wave + 1);
                        c.last_wave_ms = Some(shared.started.elapsed().as_millis());
                    });
                }
            },
            count = stream.read(&mut chunk) => {
                let count = count?;
                if count == 0 { return Ok(()); }
                input.extend_from_slice(&chunk[..count]);
                ensure!(input.len() <= MAX_PACKET + 8197, "input buffer exceeded");
                while let Some((header, body, consumed)) = decode_frame(&input)? {
                    match header {
                        0x82 => {
                            let (packet_id, filters) = parse_subscribe(&body)?;
                            let mut ack = packet_id.to_vec();
                            for filter in filters {
                                let accepted = FILTERS.contains(&filter.as_str());
                                ack.push(if accepted { 0 } else { 0x80 });
                                if accepted { subscriptions.insert(filter); }
                            }
                            write_frame(&mut stream, 0x90, &ack, shared).await?;
                            if subscriptions.len() == 3 && !ready {
                                send_messages(&mut stream, fixtures[id.0].initial_messages(id.1), true, shared).await?;
                                send_messages(&mut stream, workflow_messages(id.0, id.1, None)?, true, shared).await?;
                                ready = true;
                                shared.record(|c| { c.ready.insert(id); });
                            }
                        },
                        0xc0 if body.is_empty() => write_frame(&mut stream, 0xd0, &[], shared).await?,
                        0x40 if body.len() == 2 => {},
                        0xe0 if body.is_empty() => return Ok(()),
                        value if value >> 4 == 3 => {
                            // Never acknowledge or forward an attempted command/publication.
                            shared.record(|c| c.commands_refused += 1);
                            return Ok(());
                        },
                        _ => bail!("unsupported source MQTT packet"),
                    }
                    input.drain(..consumed);
                }
            }
        }
    }
}

fn connect_identity(body: &[u8], sources: usize, copies: usize) -> Result<(usize, usize)> {
    let mut offset = 0;
    ensure!(field(body, &mut offset)? == b"MQTT", "MQTT 3.1.1 required");
    ensure!(body.get(offset) == Some(&4), "unsupported protocol level");
    offset += 1;
    let flags = *body
        .get(offset)
        .ok_or_else(|| anyhow::anyhow!("missing CONNECT flags"))?;
    ensure!(
        flags & 0x3f == 2 && (flags & 0x40 == 0 || flags & 0x80 != 0),
        "clean session without will required"
    );
    offset += 3; // flags and keepalive
    let client = std::str::from_utf8(field(body, &mut offset)?)?;
    let parts: Vec<_> = client.split('-').collect();
    ensure!(
        parts.len() == 3 && parts[0] == "load",
        "synthetic client identity required"
    );
    let source: usize = parts[1].parse()?;
    let copy: usize = parts[2].parse()?;
    ensure!(
        source < sources && copy < copies && client == format!("load-{source}-{copy}"),
        "invalid synthetic identity"
    );
    if flags & 0x80 != 0 {
        ensure!(
            field(body, &mut offset)? == b"load-only",
            "synthetic username required"
        );
    }
    if flags & 0x40 != 0 {
        ensure!(
            field(body, &mut offset)? == b"load-only",
            "synthetic password required"
        );
    }
    ensure!(offset == body.len(), "trailing CONNECT data");
    Ok((source, copy))
}

fn field<'a>(body: &'a [u8], offset: &mut usize) -> Result<&'a [u8]> {
    let size = body
        .get(*offset..*offset + 2)
        .ok_or_else(|| anyhow::anyhow!("missing MQTT field length"))?;
    let size = usize::from(u16::from_be_bytes([size[0], size[1]]));
    *offset += 2;
    let value = body
        .get(*offset..*offset + size)
        .ok_or_else(|| anyhow::anyhow!("truncated MQTT field"))?;
    *offset += size;
    Ok(value)
}

fn parse_subscribe(body: &[u8]) -> Result<([u8; 2], Vec<String>)> {
    ensure!(
        body.len() >= 2 && body[..2] != [0, 0],
        "invalid subscription identifier"
    );
    let mut offset = 2;
    let mut filters = Vec::new();
    while offset < body.len() {
        ensure!(filters.len() < 8, "too many subscription filters");
        filters.push(std::str::from_utf8(field(body, &mut offset)?)?.to_owned());
        ensure!(
            body.get(offset).is_some_and(|qos| *qos <= 1),
            "unsupported subscription QoS"
        );
        offset += 1;
    }
    ensure!(!filters.is_empty(), "empty subscription");
    Ok(([body[0], body[1]], filters))
}

fn workflow_messages(
    source: usize,
    copy: usize,
    wave: Option<u64>,
) -> Result<Vec<(String, Vec<u8>)>> {
    let points = super::load_workflows::input_points(source, copy)?;
    let comfort = wave.is_some_and(|value| value % 2 == 0);
    Ok([
        (
            points.mode_entity_id,
            if comfort { "comfort" } else { "eco" },
        ),
        (
            points.temperature_entity_id,
            if wave.is_none() {
                "20"
            } else if comfort {
                "24"
            } else {
                "18"
            },
        ),
        (points.opening_entity_id, if comfort { "on" } else { "off" }),
    ]
    .into_iter()
    .map(|(entity, value)| {
        (
            format!(
                "homeassistant/statestream/{}/state",
                entity.replace('.', "/")
            ),
            value.as_bytes().to_vec(),
        )
    })
    .collect())
}

async fn send_messages(
    stream: &mut TcpStream,
    messages: Vec<(String, Vec<u8>)>,
    retained: bool,
    shared: &Shared,
) -> Result<()> {
    for (topic, payload) in messages {
        ensure!(
            !topic.split('/').any(|part| part == "set"),
            "command topic forbidden"
        );
        ensure!(topic.len() <= usize::from(u16::MAX), "topic too long");
        let mut body = u16::try_from(topic.len())?.to_be_bytes().to_vec();
        body.extend_from_slice(topic.as_bytes());
        body.extend_from_slice(&payload);
        write_frame(stream, if retained { 0x31 } else { 0x30 }, &body, shared).await?;
        shared.record(|c| {
            if retained {
                c.initial += 1;
            } else {
                c.updates += 1;
            }
            c.last_send_ms = Some(shared.started.elapsed().as_millis());
        });
    }
    Ok(())
}

async fn write_frame(
    stream: &mut TcpStream,
    header: u8,
    body: &[u8],
    shared: &Shared,
) -> Result<()> {
    ensure!(body.len() <= MAX_PACKET, "MQTT packet exceeds bound");
    let mut frame = vec![header];
    let mut remaining = body.len();
    loop {
        let mut byte = u8::try_from(remaining % 128)?;
        remaining /= 128;
        if remaining != 0 {
            byte |= 128;
        }
        frame.push(byte);
        if remaining == 0 {
            break;
        }
    }
    frame.extend_from_slice(body);
    match tokio::time::timeout(DEADLINE, stream.write_all(&frame)).await {
        Ok(result) => result?,
        Err(error) => {
            shared.record(|c| c.slow_writes += 1);
            return Err(error.into());
        }
    }
    shared.record(|c| c.bytes += frame.len() as u64);
    Ok(())
}

fn decode_frame(input: &[u8]) -> Result<Option<(u8, Vec<u8>, usize)>> {
    if input.len() < 2 {
        return Ok(None);
    }
    let mut length = 0_usize;
    for index in 0..4 {
        let Some(&byte) = input.get(index + 1) else {
            return Ok(None);
        };
        length += usize::from(byte & 127) << (index * 7);
        ensure!(length <= MAX_PACKET, "MQTT packet exceeds bound");
        if byte & 128 == 0 {
            let end = index + 2 + length;
            return Ok(input
                .get(index + 2..end)
                .map(|body| (input[0], body.to_vec(), end)));
        }
    }
    bail!("malformed MQTT remaining length")
}

async fn read_first(stream: &mut TcpStream) -> Result<(u8, Vec<u8>)> {
    let mut input = vec![stream.read_u8().await?];
    for _ in 0..4 {
        let byte = stream.read_u8().await?;
        input.push(byte);
        if byte & 128 == 0 {
            let mut length = 0;
            for (index, byte) in input[1..].iter().enumerate() {
                length += usize::from(byte & 127) << (index * 7);
            }
            ensure!(length <= 1024, "CONNECT exceeds bound");
            let offset = input.len();
            input.resize(offset + length, 0);
            stream.read_exact(&mut input[offset..]).await?;
            let (header, body, _) =
                decode_frame(&input)?.ok_or_else(|| anyhow::anyhow!("incomplete CONNECT"))?;
            return Ok((header, body));
        }
    }
    bail!("malformed CONNECT length")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mqtt_decoder_preserves_partial_packets_and_rejects_oversize() {
        assert!(decode_frame(&[0x30, 0x80]).unwrap().is_none());
        assert!(decode_frame(&[0x30, 3, 0, 1]).unwrap().is_none());
        assert_eq!(
            decode_frame(&[0xc0, 0, 0xd0, 0]).unwrap().unwrap(),
            (0xc0, vec![], 2)
        );
        assert!(decode_frame(&[0x30, 0xff, 0xff, 0xff, 0x7f]).is_err());
        assert!(decode_frame(&[0x30, 0x80, 0x80, 0x80, 0x80]).is_err());
    }

    #[test]
    fn connect_requires_exact_bounded_synthetic_identity() {
        let make = |client: &str| {
            let mut body = b"\x00\x04MQTT\x04\x02\x00\x3c".to_vec();
            body.extend_from_slice(&u16::try_from(client.len()).unwrap().to_be_bytes());
            body.extend_from_slice(client.as_bytes());
            body
        };
        assert_eq!(connect_identity(&make("load-1-9"), 2, 10).unwrap(), (1, 9));
        for client in ["load-2-0", "load-0-10", "load-00-1", "other-0-0"] {
            assert!(connect_identity(&make(client), 2, 10).is_err());
        }
        assert!(parse_subscribe(&[0, 1, 0, 4, b'x']).is_err());
    }

    #[test]
    fn workflow_waves_are_ordered_and_never_target_commands() {
        let initial = workflow_messages(1, 9, None).unwrap();
        assert!(initial[0].0.ends_with("load_1_009_mode/state"));
        assert_eq!(initial[1].1, b"20");
        assert_eq!(
            initial[1].0,
            "homeassistant/statestream/sensor/load_1_009_temperature/state"
        );
        let comfort = workflow_messages(1, 9, Some(0)).unwrap();
        assert_eq!(comfort[0].1, b"comfort");
        assert_eq!(comfort[1].1, b"24");
        let high = std::str::from_utf8(&comfort[1].1)
            .unwrap()
            .parse::<f64>()
            .unwrap();
        assert!(high > crate::load_workflows::TEMP_ABOVE_C);
        assert_eq!(comfort[2].1, b"on");
        let eco = workflow_messages(1, 9, Some(1)).unwrap();
        assert_eq!(eco[0].1, b"eco");
        assert_eq!(eco[1].1, b"18");
        assert!(eco.iter().all(|(topic, _)| !topic.ends_with("/set")));
    }
}
