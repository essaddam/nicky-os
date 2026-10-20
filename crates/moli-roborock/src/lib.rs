//! Roborock vacuums paired in Mi Home, over the local miIO protocol (UDP
//! 54321, the device's token): state, battery, error, and the usual orders
//! (start, pause, stop, back to the dock). No cloud: the token is given by
//! a person, once.
//!
//! One exchange at a time with the vacuum: a `hello` (its id and clock),
//! then `miIO.info` (it must be a Roborock vacuum), then `get_status` every
//! 30 s (5 s while it works or just after an order). Orders leave **one at
//! a time, in the order received**, from a queue of [`QUEUE`] at most;
//! each is checked right before it leaves (caller still there, before its
//! deadline) and sent **once**: an order the vacuum does not answer in
//! time is « maybe sent, not confirmed », never sent again.

pub mod miio;
pub mod model;

use std::collections::VecDeque;
use std::time::Duration;

use moli_core::{Device, DeviceId, Value};
use moli_runtime::{BoxFuture, CommandRequest, Driver, DriverCtx};
use serde::Deserialize;
use serde_json::{Value as Json, json};
use tokio::net::UdpSocket;
use tokio::time::{Instant, sleep_until};

use crate::miio::Keys;

/// The vault's name for the device token.
pub const TOKEN: &str = "token";
/// Orders waiting behind the one in flight; more are refused at once.
pub const QUEUE: usize = 4;
const IDLE_POLL: Duration = Duration::from_secs(30);
const ACTIVE_POLL: Duration = Duration::from_secs(5);
/// After an order, the state is read often for this long.
const FOLLOW_FOR: Duration = Duration::from_secs(60);
/// How long a hello or a reading may take.
const READ_WAIT: Duration = Duration::from_secs(2);
/// The answer is given this long before the hub stops waiting.
const MARGIN: Duration = Duration::from_millis(400);
const HELLO_RETRY_MIN: Duration = Duration::from_secs(5);
const HELLO_RETRY_MAX: Duration = Duration::from_secs(60);
/// Readings unanswered in a row before the vacuum is offline.
const MISSES: u32 = 3;
const MAX_NAME: usize = 64;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    /// Shown until a person names it (miIO has no name).
    #[serde(default)]
    pub name: Option<String>,
}

fn default_port() -> u16 {
    miio::PORT
}

#[derive(Debug)]
pub struct Roborock {
    config: Config,
}

impl Roborock {
    #[must_use]
    pub fn new(config: Config) -> Self {
        Self { config }
    }
}

impl Driver for Roborock {
    fn kind(&self) -> &'static str {
        "roborock"
    }

    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(run(&self.config, ctx))
    }
}

/// Refuses orders until the system stops: a person must act first.
async fn park(ctx: &mut DriverCtx, why: &str) {
    while let Some(command) = ctx.next_command().await {
        command.reply(Err(why.to_owned()));
    }
}

async fn run(config: &Config, ctx: &mut DriverCtx) -> anyhow::Result<()> {
    let Some(token) = ctx.secret(TOKEN).as_deref().and_then(miio::token) else {
        let why = moli_i18n::tr!("pilotes.roborock.jeton_manquant", instance = ctx.instance());
        ctx.wait_for(why.clone());
        park(ctx, &why).await;
        return Ok(());
    };
    for id in ctx.known_devices() {
        ctx.set_availability(&id, false);
    }
    let socket = UdpSocket::bind(("0.0.0.0", 0)).await?;
    // Connected: only the vacuum's own address and port are heard.
    socket.connect((config.host.as_str(), config.port)).await?;
    let name: String = config
        .name
        .as_deref()
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME)
        .collect();
    let name = if name.trim().is_empty() {
        moli_i18n::tr!("pilotes.roborock.aspirateur")
    } else {
        name.trim().to_owned()
    };
    serve(ctx, &socket, &Keys::new(token), &name).await;
    Ok(())
}

/// The vacuum's id and clock, from its hello.
struct Peer {
    device: [u8; 4],
    stamp: u32,
    at: Instant,
}

impl Peer {
    /// The vacuum's clock now, a second ahead (it refuses old stamps).
    fn stamp(&self) -> u32 {
        let elapsed = u32::try_from(self.at.elapsed().as_secs()).unwrap_or(u32::MAX);
        self.stamp.wrapping_add(elapsed).wrapping_add(1)
    }
}

/// What the request in flight asked.
enum Asked {
    Hello,
    Info,
    Poll,
    Order(CommandRequest),
}

struct Pending {
    id: u64,
    asked: Asked,
    until: Instant,
}

/// The vacuum as published.
struct Published {
    id: DeviceId,
    device: [u8; 4],
    values: Vec<(&'static str, Value)>,
    online: bool,
}

struct Link<'a> {
    socket: &'a UdpSocket,
    keys: &'a Keys,
    peer: Option<Peer>,
    last_id: u64,
}

impl Link<'_> {
    /// Sends `method` once; its id, or why it did not leave.
    fn send(&mut self, method: &str) -> Result<u64, String> {
        let Some(peer) = &self.peer else {
            return Err(moli_i18n::tr!("pilotes.roborock.injoignable"));
        };
        self.last_id = self.last_id % 999_999 + 1;
        let request = json!({ "id": self.last_id, "method": method, "params": [] });
        let packet = self
            .keys
            .encode(peer.device, peer.stamp(), request.to_string().as_bytes());
        self.socket
            // Never suspend between checking an order and emitting its datagram.
            // A full socket refuses the order instead of sending it later.
            .try_send(&packet)
            .map_err(|e| moli_i18n::tr!("pilotes.roborock.non_envoye", cause = e))?;
        Ok(self.last_id)
    }
}

/// Checks a command and queues it, or refuses it at once.
fn accept(
    published: Option<&Published>,
    queue: &mut VecDeque<(CommandRequest, &'static str)>,
    command: CommandRequest,
) {
    if !published.is_some_and(|p| p.online && p.id == command.device.id) {
        command.reply(Err(moli_i18n::tr!("pilotes.roborock.injoignable")));
        return;
    }
    match model::order(&command.key, &command.value) {
        Err(why) => command.reply(Err(why)),
        Ok(_) if queue.len() >= QUEUE => {
            command.reply(Err(moli_i18n::tr!("pilotes.roborock.occupe")));
        }
        Ok(method) => queue.push_back((command, method)),
    }
}

/// Refuses the queued orders whose caller left or whose time ran out:
/// they never leave.
fn prune(queue: &mut VecDeque<(CommandRequest, &'static str)>) {
    let mut kept = VecDeque::with_capacity(queue.len());
    for (command, method) in queue.drain(..) {
        if command.abandoned() {
            command.reply(Err(moli_i18n::tr!("pilotes.roborock.trop_tard")));
        } else {
            kept.push_back((command, method));
        }
    }
    *queue = kept;
}

fn set_online(ctx: &DriverCtx, published: Option<&mut Published>, online: bool) {
    if let Some(p) = published
        && p.online != online
    {
        p.online = online;
        ctx.set_availability(&p.id, online);
    }
}

/// Serves the vacuum until the system stops.
#[allow(clippy::too_many_lines)] // one loop, its cases side by side
async fn serve(ctx: &mut DriverCtx, socket: &UdpSocket, keys: &Keys, name: &str) {
    let mut link = Link {
        socket,
        keys,
        peer: None,
        last_id: 0,
    };
    let mut published: Option<Published> = None;
    let mut pending: Option<Pending> = None;
    let mut queue: VecDeque<(CommandRequest, &'static str)> = VecDeque::with_capacity(QUEUE);
    let mut hello_at = Instant::now();
    let mut hello_retry = HELLO_RETRY_MIN;
    let mut poll_at = Instant::now();
    let mut fast_until = Instant::now();
    let mut misses = 0u32;
    let mut known_info = false;
    let mut buf = vec![0u8; miio::MAX_PACKET + 1];
    loop {
        prune(&mut queue);
        let now = Instant::now();
        if pending.is_none() {
            if link.peer.is_none() {
                if now >= hello_at {
                    match socket.send(&miio::hello()).await {
                        Ok(_) => {
                            pending = Some(Pending {
                                id: 0,
                                asked: Asked::Hello,
                                until: now + READ_WAIT,
                            });
                        }
                        Err(e) => {
                            tracing::debug!(error = %e, "roborock: hello not sent");
                            hello_at = now + hello_retry;
                        }
                    }
                }
            } else if let Some((command, method)) = queue.pop_front() {
                if !published.as_ref().is_some_and(|p| p.online) {
                    command.reply(Err(moli_i18n::tr!("pilotes.roborock.injoignable")));
                    continue;
                }
                // Checked right before it leaves: never late, never unwanted.
                if command.abandoned() {
                    command.reply(Err(moli_i18n::tr!("pilotes.roborock.trop_tard")));
                    continue;
                }
                let until = Instant::from_std(command.deadline)
                    .checked_sub(MARGIN)
                    .unwrap_or(now);
                if until <= now {
                    // No time left to hear its answer: it stays home.
                    command.reply(Err(moli_i18n::tr!("pilotes.roborock.trop_tard")));
                    continue;
                }
                match link.send(method) {
                    Ok(id) => {
                        pending = Some(Pending {
                            id,
                            asked: Asked::Order(command),
                            until,
                        });
                    }
                    Err(why) => command.reply(Err(why)),
                }
            } else if !known_info || now >= poll_at {
                let (method, asked) = if known_info {
                    ("get_status", Asked::Poll)
                } else {
                    ("miIO.info", Asked::Info)
                };
                match link.send(method) {
                    Ok(id) => {
                        pending = Some(Pending {
                            id,
                            asked,
                            until: now + READ_WAIT,
                        });
                    }
                    Err(_) => link.peer = None,
                }
            }
        }
        let wake = match &pending {
            Some(p) => p.until,
            None if link.peer.is_none() => hello_at,
            None => poll_at,
        };
        tokio::select! {
            command = ctx.next_command() => {
                let Some(command) = command else {
                    if let Some(Pending { asked: Asked::Order(c), .. }) = pending.take() {
                        c.reply(Err(moli_i18n::tr!("pilotes.roborock.non_confirme")));
                    }
                    for (c, _) in queue.drain(..) {
                        c.reply(Err(moli_i18n::tr!("pilotes.roborock.trop_tard")));
                    }
                    return;
                };
                accept(published.as_ref(), &mut queue, command);
            }
            received = socket.recv(&mut buf) => {
                let Ok(n) = received else { continue };
                let packet = &buf[..n];
                if matches!(pending, Some(Pending { asked: Asked::Hello, .. })) {
                    let Some((device, stamp)) = miio::parse_hello(packet) else { continue };
                    pending = None;
                    if published.as_ref().is_some_and(|p| p.device != device) {
                        // Another device answers at this address now.
                        let why = moli_i18n::tr!("pilotes.roborock.autre_appareil");
                        ctx.wait_for(why);
                        set_online(ctx, published.as_mut(), false);
                        hello_at = Instant::now() + HELLO_RETRY_MAX;
                        continue;
                    }
                    link.peer = Some(Peer { device, stamp, at: Instant::now() });
                    hello_retry = HELLO_RETRY_MIN;
                    continue;
                }
                let Some(peer) = &mut link.peer else { continue };
                let Ok((stamp, plain)) = keys.decode(peer.device, packet) else {
                    tracing::debug!("roborock: packet refused");
                    continue;
                };
                peer.stamp = stamp;
                peer.at = Instant::now();
                let Ok(answer) = serde_json::from_slice::<Json>(&plain) else { continue };
                let matches = pending
                    .as_ref()
                    .is_some_and(|p| answer["id"].as_u64() == Some(p.id));
                if !matches {
                    // A late answer to something already given up.
                    continue;
                }
                let Some(Pending { asked, .. }) = pending.take() else { continue };
                let now = Instant::now();
                match asked {
                    Asked::Hello => {}
                    Asked::Info => {
                        let info = &answer["result"];
                        let Some(model) = model::model(info) else {
                            let why = moli_i18n::tr!("pilotes.roborock.pas_un_aspirateur");
                            ctx.wait_for(why.clone());
                            park(ctx, &why).await;
                            return;
                        };
                        let device = peer.device;
                        let id = ctx.device_id(&u32::from_be_bytes(device).to_string());
                        ctx.upsert_device(Device {
                            id: id.clone(),
                            instance: ctx.instance().clone(),
                            native_name: name.into(),
                            manufacturer: Some("Roborock".into()),
                            model: Some(
                                model::firmware(info)
                                    .map_or_else(|| model.clone(), |f| format!("{model} · {f}"))
                                    .into(),
                            ),
                            description: Some(moli_i18n::tr!("pilotes.roborock.aspirateur").into()),
                            native_room: None,
                            members: Vec::new(),
                            points: model::points(),
                        });
                        published = Some(Published {
                            id,
                            device,
                            values: Vec::new(),
                            online: false,
                        });
                        known_info = true;
                        poll_at = now;
                    }
                    Asked::Poll => {
                        let Some(status) = model::status(&answer["result"]) else {
                            // A reply without a status proves no usable state.
                            set_online(ctx, published.as_mut(), false);
                            ctx.wait_for(moli_i18n::tr!("pilotes.roborock.injoignable"));
                            poll_at = now + IDLE_POLL;
                            continue;
                        };
                        misses = 0;
                        let Some(p) = published.as_mut() else { continue };
                        {
                            let values = model::values(status);
                            if values != p.values {
                                for (key, value) in &values {
                                    ctx.set_state(&p.id, key, value.clone());
                                }
                                p.values = values;
                            }
                            let pace = if model::active(status) || now < fast_until {
                                ACTIVE_POLL
                            } else {
                                IDLE_POLL
                            };
                            poll_at = now + pace;
                        }
                        set_online(ctx, published.as_mut(), true);
                        ctx.ready();
                    }
                    Asked::Order(command) => {
                        command.reply(model::outcome(&answer));
                        fast_until = now + FOLLOW_FOR;
                        poll_at = now + Duration::from_secs(1);
                    }
                }
            }
            () = sleep_until(wake) => {
                let now = Instant::now();
                let Some(p) = pending.take_if(|p| p.until <= now) else { continue };
                match p.asked {
                    Asked::Hello => {
                        hello_at = now + hello_retry;
                        hello_retry = (hello_retry * 2).min(HELLO_RETRY_MAX);
                        set_online(ctx, published.as_mut(), false);
                        ctx.wait_for(moli_i18n::tr!("pilotes.roborock.injoignable"));
                    }
                    Asked::Info | Asked::Poll => {
                        // Its clock may have moved (a restart): hello again.
                        misses += 1;
                        link.peer = None;
                        hello_at = now;
                        if misses >= MISSES {
                            set_online(ctx, published.as_mut(), false);
                            ctx.wait_for(moli_i18n::tr!("pilotes.roborock.injoignable"));
                        }
                    }
                    Asked::Order(command) => {
                        // It may have left: never again.
                        command.reply(Err(moli_i18n::tr!("pilotes.roborock.non_confirme")));
                        link.peer = None;
                        hello_at = now;
                        fast_until = now + FOLLOW_FOR;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
