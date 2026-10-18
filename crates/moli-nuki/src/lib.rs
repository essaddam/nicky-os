//! Nuki smart locks behind a Nuki Bridge, over the bridge's local HTTP API
//! (hashed token: the token itself never travels).
//!
//! Reads `/list` (the bridge's cached state: asking a lock directly drains
//! its batteries) every 30 s, every 3 s for a while after an order. Writes:
//! `locked` (lock, unlock) and `unlatch`, with the `lock` semantic: opening
//! is held by the guard for a person (an agent asks, so does the dashboard
//! without a code), and every attempt is in the journal.
//!
//! The bridge serves one request at a time: this loop never sends two.

pub mod model;

use std::collections::HashSet;
use std::hash::Hash;
use std::time::Duration;

use http::{Method, Request, StatusCode};
use moli_core::{Device, DeviceId, PointSpec, Value};
use moli_runtime::{BoxFuture, CommandRequest, Driver, DriverCtx};
use ring::rand::{SecureRandom, SystemRandom};
use serde::Deserialize;
use tokio::time::{Instant, sleep_until};

use crate::model::{Last, Lock};

/// The vault's name for the bridge's API token.
pub const TOKEN: &str = "token";
const IDLE: Duration = Duration::from_secs(30);
/// After an order: the bolt moves for a few seconds.
const FOLLOW: Duration = Duration::from_secs(3);
const FOLLOW_FOR: Duration = Duration::from_secs(30);
const RETRY: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_port() -> u16 {
    8080
}

#[derive(Debug)]
pub struct Nuki {
    config: Config,
}

impl Nuki {
    #[must_use]
    pub fn new(config: Config) -> Self {
        Self { config }
    }
}

impl Driver for Nuki {
    fn kind(&self) -> &'static str {
        "nuki"
    }

    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(run(&self.config, ctx))
    }
}

/// Why a call failed, as far as the driver cares.
#[derive(Debug)]
enum Failure {
    /// 401 or 403: the token, the bridge's clock, or the API switched off.
    Refused(StatusCode),
    Other(anyhow::Error),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(status) => write!(f, "Nuki bridge {}", status.as_u16()),
            Self::Other(e) => write!(f, "{}", e.root_cause()),
        }
    }
}

/// Whether a failed order may still have reached the bridge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sent {
    /// Refused before leaving: nothing can have happened.
    No,
    /// Left (or may have): no answer is not a proof it did nothing.
    Maybe,
}

struct Bridge<'a> {
    config: &'a Config,
    token: String,
    rng: SystemRandom,
}

impl Bridge<'_> {
    /// One call; `query` without the authentication. `wanted` is asked
    /// before each step up to the request leaving (see
    /// [`moli_net::plain_if`]).
    async fn get(
        &self,
        path: &str,
        query: &str,
        wanted: impl Fn() -> bool,
    ) -> Result<Vec<u8>, Failure> {
        let mut rnr = [0u8; 2];
        self.rng
            .fill(&mut rnr)
            .map_err(|_| Failure::Other(anyhow::anyhow!("no randomness")))?;
        let ts = jiff::Timestamp::now()
            .strftime("%Y-%m-%dT%H:%M:%SZ")
            .to_string();
        let auth = model::auth(&self.token, &ts, u16::from_be_bytes(rnr));
        let sep = if query.is_empty() { "" } else { "&" };
        let request = Request::builder()
            .method(Method::GET)
            .uri(format!("{path}?{query}{sep}{auth}"));
        let (status, body) = moli_net::plain_if(
            &self.config.host,
            self.config.port,
            moli_net::empty(request).map_err(Failure::Other)?,
            wanted,
        )
        .await
        .map_err(Failure::Other)?;
        match status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(Failure::Refused(status)),
            s if s.is_success() => Ok(body.to_vec()),
            s => Err(Failure::Other(anyhow::anyhow!(
                "Nuki bridge {}",
                s.as_u16()
            ))),
        }
    }

    async fn list(&self) -> Result<Vec<(Lock, Option<Last>)>, Failure> {
        let body = self.get("/list", "", || true).await?;
        model::locks(&body).map_err(|e| Failure::Other(e.context("Nuki bridge: not a list")))
    }

    /// Sends a lock action while `wanted`. The error says whether it may
    /// have left (then its effect is followed) or surely did not.
    async fn act(
        &self,
        lock: &Lock,
        action: u8,
        wanted: impl Fn() -> bool,
    ) -> Result<(), (String, Sent)> {
        let query = format!(
            "nukiId={}&deviceType={}&action={action}&nowait=1",
            lock.nuki_id, lock.device_type
        );
        match self.get("/lockAction", &query, wanted).await {
            Err(Failure::Other(e)) if e.is::<moli_net::NotSent>() => {
                Err((moli_i18n::tr!("pilotes.nuki.trop_tard"), Sent::No))
            }
            Err(e) => Err((e.to_string(), Sent::Maybe)),
            Ok(body) if model::succeeded(&body) => Ok(()),
            Ok(_) => Err((moli_i18n::tr!("pilotes.nuki.refuse"), Sent::Maybe)),
        }
    }
}

/// Whether a lock may be published: one already admitted always, a new one
/// while fewer than [`model::MAX_LOCKS`] are. Bounds the instance over time,
/// not only one answer: locks that come and go never pile up.
fn admit<T: Eq + Hash + Clone>(admitted: &mut HashSet<T>, id: &T) -> bool {
    admitted.contains(id) || (admitted.len() < model::MAX_LOCKS && admitted.insert(id.clone()))
}

/// A lock as published: its points and values, to republish only what
/// changed.
struct Known {
    lock: Lock,
    id: DeviceId,
    points: Vec<PointSpec>,
    /// The values last published: the same memory of the bridge read again
    /// is not a new observation (its time would lie about freshness).
    published: Vec<(&'static str, Value)>,
    online: bool,
}

/// Publishes what `/list` said: new locks, what the bridge heard since,
/// who went quiet.
fn publish(
    ctx: &DriverCtx,
    known: &mut Vec<Known>,
    admitted: &mut HashSet<DeviceId>,
    listed: Vec<(Lock, Option<Last>)>,
) {
    let mut seen = Vec::with_capacity(listed.len());
    for (lock, last) in listed {
        let id = ctx.device_id(&lock.native);
        if !admit(admitted, &id) {
            tracing::warn!(
                "{}",
                moli_i18n::tr!(
                    "pilotes.nuki.trop_de_serrures",
                    max = model::MAX_LOCKS,
                    name = lock.name
                )
            );
            continue;
        }
        let points = model::points(last.as_ref());
        let index = known.iter().position(|k| k.id == id).unwrap_or_else(|| {
            known.push(Known {
                lock: lock.clone(),
                id: id.clone(),
                points: Vec::new(),
                published: Vec::new(),
                online: false,
            });
            known.len() - 1
        });
        let entry = &mut known[index];
        if entry.points != points {
            // Points gone and back (a lock that said nothing for a while)
            // come back empty: everything is published again.
            entry.published.clear();
        }
        if entry.points != points || entry.lock != lock {
            ctx.upsert_device(Device {
                id: id.clone(),
                instance: ctx.instance().clone(),
                native_name: lock.name.as_str().into(),
                manufacturer: Some("Nuki".into()),
                model: Some(lock.model.into()),
                description: Some(moli_i18n::tr!("pilotes.nuki.serrure").into()),
                native_room: None,
                members: Vec::new(),
                points: points.clone(),
            });
            entry.points = points;
            entry.lock = lock;
        }
        let online = last.is_some();
        if let Some(last) = &last {
            let values = model::values(last);
            if values != entry.published {
                for (key, value) in &values {
                    ctx.set_state(&id, key, value.clone());
                }
                entry.published = values;
            }
        }
        if entry.online != online {
            entry.online = online;
            ctx.set_availability(&id, online);
        }
        seen.push(id);
    }
    for entry in known.iter_mut().filter(|k| !seen.contains(&k.id)) {
        if entry.online {
            entry.online = false;
            ctx.set_availability(&entry.id, false);
        }
    }
}

fn offline(ctx: &DriverCtx, known: &mut [Known]) {
    for entry in known.iter_mut().filter(|k| k.online) {
        entry.online = false;
        ctx.set_availability(&entry.id, false);
    }
}

/// Carries out an order. Returns whether it may have reached the bridge:
/// its effect is then followed closely, answered or not.
async fn obey(bridge: &Bridge<'_>, known: &[Known], command: CommandRequest) -> bool {
    let Some(entry) = known.iter().find(|k| k.id == command.device.id) else {
        command.reply(Err(moli_i18n::tr!("pilotes.nuki.inconnue")));
        return false;
    };
    let (result, sent) = match model::action(&command.key, &command.value) {
        Err(why) => (Err(why), Sent::No),
        // Its caller gone or its deadline past before it left: it never
        // leaves. Once left, its answer is awaited and kept.
        Ok(action) => match bridge
            .act(&entry.lock, action, || !command.abandoned())
            .await
        {
            Ok(()) => (Ok(()), Sent::Maybe),
            Err((why, sent)) => (Err(why), sent),
        },
    };
    command.reply(result);
    sent == Sent::Maybe
}

async fn run(config: &Config, ctx: &mut DriverCtx) -> anyhow::Result<()> {
    let Some(token) = ctx.secret(TOKEN) else {
        ctx.wait_for(moli_i18n::tr!(
            "pilotes.nuki.jeton_manquant",
            instance = ctx.instance(),
            key = TOKEN
        ));
        ctx.cancelled().await;
        return Ok(());
    };
    let bridge = Bridge {
        config,
        token,
        rng: SystemRandom::new(),
    };
    let mut known: Vec<Known> = Vec::new();
    // Locks published before a restart count against the bound.
    let mut admitted = HashSet::new();
    for id in ctx.known_devices() {
        ctx.set_availability(&id, false);
        admit(&mut admitted, &id);
    }
    let mut next = Instant::now();
    let mut follow_until = Instant::now();
    loop {
        tokio::select! {
            command = ctx.next_command() => match command {
                Some(command) => {
                    if obey(&bridge, &known, command).await {
                        follow_until = Instant::now() + FOLLOW_FOR;
                        next = Instant::now() + FOLLOW;
                    }
                }
                None => return Ok(()),
            },
            () = sleep_until(next) => {
                match bridge.list().await {
                    Ok(listed) => {
                        publish(ctx, &mut known, &mut admitted, listed);
                        ctx.ready();
                        let pace = if Instant::now() < follow_until { FOLLOW } else { IDLE };
                        next = Instant::now() + pace;
                    }
                    Err(Failure::Refused(status)) => {
                        offline(ctx, &mut known);
                        ctx.wait_for(moli_i18n::tr!(
                            "pilotes.nuki.jeton_refuse",
                            status = status.as_u16(),
                            instance = ctx.instance(),
                            key = TOKEN
                        ));
                        next = Instant::now() + RETRY;
                    }
                    Err(Failure::Other(e)) => {
                        offline(ctx, &mut known);
                        ctx.wait_for(moli_i18n::tr!(
                            "pilotes.nuki.injoignable",
                            cause = e.root_cause()
                        ));
                        next = Instant::now() + RETRY;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn options_are_bounded_to_what_is_known() {
        let config: Config = serde_json::from_str(r#"{"host":"192.168.1.x"}"#).unwrap();
        assert_eq!(config.port, 8080);
        assert!(serde_json::from_str::<Config>(r#"{"host":"h","token":"x"}"#).is_err());
        let refused = Failure::Refused(StatusCode::UNAUTHORIZED).to_string();
        assert_eq!(refused, "Nuki bridge 401");
    }

    /// Runs the real `publish` against a real hub: what reaches the hub.
    struct Replay {
        hub: moli_runtime::Hub,
        report: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<(u64, u64, usize)>>>,
    }

    fn list(range: std::ops::Range<u32>, timestamp: &str) -> Vec<(Lock, Option<Last>)> {
        let entries: Vec<String> = range
            .map(|i| {
                format!(
                    r#"{{"nukiId":{i},"deviceType":4,"name":"n{i}","lastKnownState":{{"state":1,"timestamp":"{timestamp}"}}}}"#
                )
            })
            .collect();
        model::locks(format!("[{}]", entries.join(",")).as_bytes()).unwrap()
    }

    impl Driver for Replay {
        fn kind(&self) -> &'static str {
            "nuki"
        }

        fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
            Box::pin(async move {
                let (mut known, mut admitted) = (Vec::new(), HashSet::new());
                let point = moli_core::PointId::from("serrure:0/locked");
                let max = u32::try_from(model::MAX_LOCKS).unwrap();
                // An old state, read twice: the same observation, not two.
                publish(
                    ctx,
                    &mut known,
                    &mut admitted,
                    list(0..max, "2000-01-01T00:00:00+00:00"),
                );
                let first = self.hub.state(&point).map_or(0, |s| s.ts);
                tokio::time::sleep(Duration::from_millis(20)).await;
                publish(
                    ctx,
                    &mut known,
                    &mut admitted,
                    list(0..max, "2000-01-01T00:00:00+00:00"),
                );
                let again = self.hub.state(&point).map_or(0, |s| s.ts);
                // Then as many other locks: none admitted.
                publish(
                    ctx,
                    &mut known,
                    &mut admitted,
                    list(1000..1000 + max, "2000-01-01T00:00:00+00:00"),
                );
                let devices = self
                    .hub
                    .snapshot()
                    .devices
                    .iter()
                    .filter(|d| d.device.id.as_str().starts_with("serrure:"))
                    .count();
                if let Some(tx) = self.report.lock().unwrap().take() {
                    let _ = tx.send((first, again, devices));
                }
                ctx.cancelled().await;
                Ok(())
            })
        }
    }

    /// A lock that reports its battery and door, then nothing, then the
    /// same again: its points come back with their values.
    struct Silence {
        hub: moli_runtime::Hub,
        report: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<Vec<Option<Value>>>>>,
    }

    impl Driver for Silence {
        fn kind(&self) -> &'static str {
            "nuki"
        }

        fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
            Box::pin(async move {
                let (mut known, mut admitted) = (Vec::new(), HashSet::new());
                let full = br#"[{"nukiId":7,"deviceType":4,"lastKnownState":{"state":1,"batteryChargeState":80,"doorsensorState":2,"timestamp":"2026-01-01T08:00:00+00:00"}}]"#;
                let silent = br#"[{"nukiId":7,"deviceType":4}]"#;
                for body in [&full[..], &silent[..], &full[..], &full[..]] {
                    publish(ctx, &mut known, &mut admitted, model::locks(body).unwrap());
                }
                let values = ["battery", "door", "observed"]
                    .iter()
                    .map(|k| {
                        self.hub
                            .state(&moli_core::PointId::from(format!("serrure:7/{k}")))
                            .map(|s| s.value)
                    })
                    .collect();
                if let Some(tx) = self.report.lock().unwrap().take() {
                    let _ = tx.send(values);
                }
                ctx.cancelled().await;
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn points_that_come_back_come_back_with_their_values() {
        let hub = moli_runtime::Hub::new(moli_runtime::HubOptions::default()).unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let cancel = tokio_util::sync::CancellationToken::new();
        moli_runtime::spawn_driver(
            &hub,
            moli_core::InstanceId::from("serrure"),
            std::sync::Arc::new(Silence {
                hub: hub.clone(),
                report: std::sync::Mutex::new(Some(tx)),
            }),
            cancel.clone(),
        );
        let values = rx.await.unwrap();
        cancel.cancel();
        assert_eq!(
            values,
            [
                Some(Value::Int(80)),
                Some(Value::Bool(true)),
                Some(Value::from("2026-01-01T08:00:00Z"))
            ]
        );
    }

    #[tokio::test]
    async fn the_bridge_memory_read_again_is_not_fresh_and_locks_stay_bounded() {
        let hub = moli_runtime::Hub::new(moli_runtime::HubOptions::default()).unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let cancel = tokio_util::sync::CancellationToken::new();
        moli_runtime::spawn_driver(
            &hub,
            moli_core::InstanceId::from("serrure"),
            std::sync::Arc::new(Replay {
                hub: hub.clone(),
                report: std::sync::Mutex::new(Some(tx)),
            }),
            cancel.clone(),
        );
        let (first, again, devices) = rx.await.unwrap();
        cancel.cancel();
        assert!(first > 0);
        assert_eq!(first, again, "an unchanged observation must keep its time");
        assert_eq!(
            hub.state(&moli_core::PointId::from("serrure:0/observed"))
                .unwrap()
                .value,
            Value::from("2000-01-01T00:00:00Z")
        );
        assert_eq!(devices, model::MAX_LOCKS);
    }

    #[test]
    fn locks_that_come_and_go_never_pile_up() {
        let mut admitted = HashSet::new();
        // Two answers of MAX_LOCKS locks each, all different.
        let first = (0..model::MAX_LOCKS)
            .filter(|i| admit(&mut admitted, i))
            .count();
        let second = (1000..1000 + model::MAX_LOCKS)
            .filter(|i| admit(&mut admitted, i))
            .count();
        assert_eq!((first, second), (model::MAX_LOCKS, 0));
        assert_eq!(admitted.len(), model::MAX_LOCKS);
        // One already admitted stays so.
        assert!(admit(&mut admitted, &3));
    }

    /// A bridge on the loopback: says when an order arrived, answers it
    /// `{"success":true}` when told to.
    async fn bridge_on_loopback() -> (u16, Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let got = Arc::new(tokio::sync::Notify::new());
        let go = Arc::new(tokio::sync::Notify::new());
        let (g, o) = (Arc::clone(&got), Arc::clone(&go));
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let (g, o) = (Arc::clone(&g), Arc::clone(&o));
                tokio::spawn(async move {
                    let mut seen = Vec::new();
                    let mut buf = [0u8; 2048];
                    while let Ok(n) = socket.read(&mut buf).await {
                        if n == 0 {
                            return;
                        }
                        seen.extend_from_slice(&buf[..n]);
                        if seen.windows(4).any(|w| w == b"\r\n\r\n") {
                            g.notify_one();
                            o.notified().await;
                            let _ = socket
                                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 16\r\n\r\n{\"success\":true}")
                                .await;
                            return;
                        }
                    }
                });
            }
        });
        (port, got, go)
    }

    /// The real `obey`, on the orders the hub hands it, against `port`.
    struct Obeys {
        port: u16,
        followed: tokio::sync::mpsc::UnboundedSender<bool>,
    }

    impl Driver for Obeys {
        fn kind(&self) -> &'static str {
            "nuki"
        }

        fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
            Box::pin(async move {
                let config = Config {
                    host: "127.0.0.1".into(),
                    port: self.port,
                };
                let bridge = Bridge {
                    config: &config,
                    token: "123456".into(),
                    rng: SystemRandom::new(),
                };
                let (mut known, mut admitted) = (Vec::new(), HashSet::new());
                let list = br#"[{"nukiId":7,"deviceType":4,"lastKnownState":{"state":1}}]"#;
                publish(ctx, &mut known, &mut admitted, model::locks(list).unwrap());
                ctx.ready();
                while let Some(command) = ctx.next_command().await {
                    let followed = obey(&bridge, &known, command).await;
                    let _ = self.followed.send(followed);
                }
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn an_answer_after_the_order_left_is_kept_even_if_the_caller_left() {
        for approval in [false, true] {
            for leaves in [false, true] {
                let (port, got, go) = bridge_on_loopback().await;
                let hub = moli_runtime::Hub::new(moli_runtime::HubOptions::default()).unwrap();
                let (tx, mut followed) = tokio::sync::mpsc::unbounded_channel();
                let cancel = tokio_util::sync::CancellationToken::new();
                moli_runtime::spawn_driver(
                    &hub,
                    moli_core::InstanceId::from("serrure"),
                    Arc::new(Obeys { port, followed: tx }),
                    cancel.clone(),
                );
                let door = moli_core::PointId::from("serrure:7/locked");
                while hub.state(&door).is_none() {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                let call = if approval {
                    let held = hub
                        .command(&door, Value::Bool(false), moli_core::Origin::Mcp, None)
                        .await;
                    let Err(moli_runtime::CommandError::NeedsApproval { id, .. }) = held else {
                        panic!("held: {held:?}");
                    };
                    let (hub, door) = (hub.clone(), door.clone());
                    tokio::spawn(async move {
                        let _ = hub
                            .resolve_approval(id, &door, true, moli_core::Origin::Ui, None)
                            .await;
                    })
                } else {
                    let (hub, door) = (hub.clone(), door.clone());
                    tokio::spawn(async move {
                        let _ = hub
                            .command(&door, Value::Bool(false), moli_core::Origin::Ui, None)
                            .await;
                    })
                };
                // The bridge has the whole request; then its caller may leave.
                got.notified().await;
                if leaves {
                    call.abort();
                    let _ = call.await;
                }
                go.notify_one();
                let case = format!("approval={approval} leaves={leaves}");
                assert!(followed.recv().await.unwrap(), "followed: {case}");
                tokio::time::sleep(Duration::from_millis(100)).await;
                let lines: Vec<moli_core::Outcome> = hub
                    .journal(50)
                    .iter()
                    .filter(|e| match &e.action {
                        moli_core::Action::Command { .. } => !approval,
                        moli_core::Action::Approval { .. } => approval,
                        _ => false,
                    })
                    .map(|e| e.outcome.clone())
                    .collect();
                assert_eq!(lines, [moli_core::Outcome::Ok], "{case}");
                cancel.cancel();
            }
        }
    }
}
