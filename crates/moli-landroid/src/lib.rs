//! Worx Landroid robotic mowers (wire models, protocol 0), through the Worx
//! cloud: the account's OAuth session and mower list over HTTPS, then the
//! live status and the orders over the account's MQTT broker (AWS IoT, on
//! a websocket, the access token as credential).
//!
//! Orders (`control`: start, pause, home; `rain_delay`; `schedule`) leave
//! **one at a time, in the order received**, from a queue of
//! [`QUEUE`] at most, each published **once** (QoS 1, never again, never
//! after a reconnection). The answer is the broker's acknowledgement, given
//! before the hub stops waiting; the mower's next status confirms it, and a
//! mower that stays silent is told to the house. An order whose caller
//! left, or whose time ran out, before it left never leaves.

pub mod cloud;
pub mod model;
pub mod mqtt;

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use moli_core::{Device, DeviceId, Value};
use moli_runtime::{BoxFuture, CommandRequest, Driver, DriverCtx, MediaPublisher};
use ring::rand::{SecureRandom, SystemRandom};
use serde::Deserialize;
use serde_json::Value as Json;
use tokio::time::{Instant, sleep_until, timeout, timeout_at};

use crate::cloud::{Account, Refusal, Sessions, refusal};
use crate::model::Mower;
use crate::mqtt::{Frames, Incoming, Mqtt};

/// The vault's names for the Worx account.
pub const USERNAME: &str = "username";
pub const PASSWORD: &str = "password";

/// Orders waiting behind the one in flight; more are refused at once.
pub const QUEUE: usize = 4;
const KEEP_ALIVE: Duration = Duration::from_secs(60);
const PING_EVERY: Duration = Duration::from_secs(25);
/// The answer is given this long before the hub stops waiting.
const MARGIN: Duration = Duration::from_millis(400);
/// How long the mower has to send a status after an order.
const FOLLOW: Duration = Duration::from_secs(60);
const RETRY_MIN: Duration = Duration::from_secs(10);
const RETRY_MAX: Duration = Duration::from_secs(600);
/// A connection that lived this long resets the retry delay.
const STABLE: Duration = Duration::from_secs(600);
/// After « too many requests »: Worx bans accounts that insist.
const LIMITED: Duration = Duration::from_secs(3600);
/// The mower list is read again only when older than this.
const LIST_FRESH: Duration = Duration::from_secs(1800);
const NO_MOWER_RETRY: Duration = Duration::from_secs(1800);

/// No option: everything comes from the account.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {}

#[derive(Debug)]
pub struct Landroid {
    /// The cloud session, kept across restarts of the driver.
    sessions: Arc<Sessions>,
}

impl Landroid {
    #[must_use]
    pub fn new(_config: Config) -> Self {
        Self {
            sessions: Arc::default(),
        }
    }
}

impl Driver for Landroid {
    fn kind(&self) -> &'static str {
        "landroid"
    }

    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(run(Arc::clone(&self.sessions), ctx))
    }
}

/// A mower as published.
struct Known {
    mower: Mower,
    id: DeviceId,
    /// The last schedule seen (`cfg.sc`): a `schedule` order sends it back.
    schedule: Option<Json>,
    published: Vec<(&'static str, Value)>,
    online: bool,
}

/// What survives a reconnection: the mowers, and the orders whose effect
/// is still awaited.
#[derive(Default)]
struct House {
    known: Vec<Known>,
    /// Device, name, until when its status is awaited.
    follows: Vec<(DeviceId, String, Instant)>,
}

impl House {
    fn set_online(&mut self, ctx: &DriverCtx, index: usize, online: bool) {
        let known = &mut self.known[index];
        if known.online != online {
            known.online = online;
            ctx.set_availability(&known.id, online);
        }
    }

    fn offline(&mut self, ctx: &DriverCtx) {
        for i in 0..self.known.len() {
            self.set_online(ctx, i, false);
        }
    }

    /// Publishes a status: only what changed (the cloud's memory read again
    /// is not a new observation).
    fn status(&mut self, ctx: &DriverCtx, index: usize, status: &Json) {
        let known = &mut self.known[index];
        if status["cfg"]["sc"].is_object() {
            known.schedule = Some(status["cfg"]["sc"].clone());
        }
        let values = model::values(status);
        if values != known.published {
            for (key, value) in &values {
                ctx.set_state(&known.id, key, value.clone());
            }
            known.published = values;
        }
    }
}

/// Publishes the account's mowers: new ones, renamed ones, their last
/// status kept by the cloud, who is online. A mower gone from the account
/// goes offline (its device and history stay).
fn publish(ctx: &DriverCtx, house: &mut House, mowers: Vec<Mower>) {
    let mut seen = Vec::new();
    for mower in mowers {
        let id = ctx.device_id(&mower.serial);
        let index = house
            .known
            .iter()
            .position(|k| k.id == id)
            .unwrap_or_else(|| {
                house.known.push(Known {
                    mower: mower.clone(),
                    id: id.clone(),
                    schedule: None,
                    published: Vec::new(),
                    online: false,
                });
                ctx.upsert_device(device(ctx, &id, &mower));
                house.known.len() - 1
            });
        let known = &house.known[index];
        if known.mower.name != mower.name || known.mower.firmware != mower.firmware {
            ctx.upsert_device(device(ctx, &id, &mower));
        }
        if let Some(last) = &mower.last {
            house.status(ctx, index, last);
        }
        let online = mower.online;
        house.known[index].mower = mower;
        house.set_online(ctx, index, online);
        seen.push(id);
    }
    for i in 0..house.known.len() {
        if !seen.contains(&house.known[i].id) {
            house.set_online(ctx, i, false);
        }
    }
}

fn device(ctx: &DriverCtx, id: &DeviceId, mower: &Mower) -> Device {
    Device {
        id: id.clone(),
        instance: ctx.instance().clone(),
        native_name: mower.name.as_str().into(),
        manufacturer: Some("Worx".into()),
        model: Some(
            mower
                .firmware
                .as_ref()
                .map_or_else(|| "Landroid".to_owned(), |f| format!("Landroid · {f}"))
                .into(),
        ),
        description: Some(moli_i18n::tr!("pilotes.landroid.tondeuse").into()),
        native_room: None,
        members: Vec::new(),
        points: model::points(),
    }
}

/// An order accepted, not sent yet.
struct Order {
    command: CommandRequest,
    topic: String,
    serial: String,
    body: Json,
    name: String,
}

/// The order published, waiting for the broker's acknowledgement.
struct InFlight {
    command: CommandRequest,
    pkid: u16,
    id: DeviceId,
    name: String,
    /// When to answer without it: just before the hub stops waiting.
    until: Instant,
}

/// Why [`serve`] ended.
#[derive(Debug)]
enum End {
    Stopped,
    /// The session is to be renewed: reconnect with the new token.
    Renew,
    Lost(anyhow::Error),
}

/// Checks a command and queues it, or refuses it at once.
fn accept(house: &House, queue: &mut VecDeque<Order>, command: CommandRequest) {
    let Some(known) = house.known.iter().find(|k| k.id == command.device.id) else {
        command.reply(Err(moli_i18n::tr!("pilotes.landroid.inconnue")));
        return;
    };
    if !known.online {
        command.reply(Err(moli_i18n::tr!(
            "pilotes.landroid.hors_ligne",
            name = known.mower.name
        )));
        return;
    }
    let body = match model::order(&command.key, &command.value, known.schedule.as_ref()) {
        Ok(body) => body,
        Err(why) => {
            command.reply(Err(why));
            return;
        }
    };
    if queue.len() >= QUEUE {
        command.reply(Err(moli_i18n::tr!("pilotes.landroid.occupee")));
        return;
    }
    queue.push_back(Order {
        command,
        topic: known.mower.command_in.clone(),
        serial: known.mower.serial.clone(),
        body,
        name: known.mower.name.clone(),
    });
}

/// Refuses the queued orders whose caller left or whose time ran out:
/// they never leave.
fn prune(queue: &mut VecDeque<Order>) {
    let mut kept = VecDeque::with_capacity(queue.len());
    for order in queue.drain(..) {
        if order.command.abandoned() {
            order
                .command
                .reply(Err(moli_i18n::tr!("pilotes.landroid.trop_tard")));
        } else {
            kept.push_back(order);
        }
    }
    *queue = kept;
}

/// The link went down: the order in flight may have left, the queued ones
/// did not. None is ever sent again.
fn fail(in_flight: Option<InFlight>, queue: &mut VecDeque<Order>, house: &mut House) {
    if let Some(f) = in_flight {
        f.command
            .reply(Err(moli_i18n::tr!("pilotes.landroid.peut_etre_parti")));
        house.follows.push((f.id, f.name, Instant::now() + FOLLOW));
    }
    for order in queue.drain(..) {
        order
            .command
            .reply(Err(moli_i18n::tr!("pilotes.landroid.non_envoye")));
    }
}

fn message_id() -> u16 {
    let mut raw = [0u8; 2];
    let _ = SystemRandom::new().fill(&mut raw);
    1024 + u16::from_be_bytes(raw) % (u16::MAX - 1024)
}

/// The mower's clock: Moli's local time (the house's).
fn local_now() -> jiff::civil::DateTime {
    jiff::Zoned::now().datetime()
}

/// Publishes the next order still wanted, if any. An error: the link is
/// lost (the order is then answered « maybe sent »).
async fn send_next<F: Frames>(
    mqtt: &mut Mqtt<F>,
    queue: &mut VecDeque<Order>,
    house: &mut House,
) -> Result<Option<InFlight>, anyhow::Error> {
    while let Some(order) = queue.pop_front() {
        // Checked right before it leaves: never late, never unwanted.
        if order.command.abandoned() {
            order
                .command
                .reply(Err(moli_i18n::tr!("pilotes.landroid.trop_tard")));
            continue;
        }
        let until = Instant::from_std(order.command.deadline)
            .checked_sub(MARGIN)
            .unwrap_or_else(Instant::now);
        let payload = model::envelope(&order.body, &order.serial, message_id(), &local_now());
        let sent = timeout_at(until, mqtt.publish(&order.topic, payload))
            .await
            .unwrap_or_else(|_| Err(anyhow::anyhow!("mqtt: publish too slow")));
        let flight = InFlight {
            id: order.command.device.id.clone(),
            command: order.command,
            pkid: 0,
            name: order.name,
            until,
        };
        // Written (or partly): from here it may have left.
        return match sent {
            Ok(pkid) => Ok(Some(InFlight { pkid, ..flight })),
            Err(e) => {
                fail(Some(flight), queue, house);
                Err(e)
            }
        };
    }
    Ok(None)
}

/// A status from the broker, published on the mower it belongs to.
fn heard(ctx: &DriverCtx, house: &mut House, topic: &str, payload: &[u8]) {
    let Some(index) = house
        .known
        .iter()
        .position(|k| k.mower.command_out == topic)
    else {
        return;
    };
    let Ok(status) = serde_json::from_slice::<Json>(payload) else {
        tracing::debug!(topic, "landroid: not JSON");
        return;
    };
    if !model::belongs(&status, &house.known[index].mower.serial) {
        tracing::debug!(topic, "landroid: not this mower's status");
        return;
    }
    house.status(ctx, index, &status);
    house.set_online(ctx, index, true);
    let id = house.known[index].id.clone();
    house.follows.retain(|(f, _, _)| *f != id);
}

/// Serves one connection: orders out, statuses in, until it ends.
async fn serve<F: Frames>(
    ctx: &mut DriverCtx,
    house: &mut House,
    mqtt: &mut Mqtt<F>,
    renew_at: Instant,
    media: &MediaPublisher,
) -> End {
    let mut queue: VecDeque<Order> = VecDeque::with_capacity(QUEUE);
    let mut in_flight: Option<InFlight> = None;
    let mut ping_at = Instant::now() + PING_EVERY;
    let mut awaiting_pong = false;
    let far = || Instant::now() + Duration::from_secs(86_400);
    loop {
        prune(&mut queue);
        if in_flight.is_none() {
            match send_next(mqtt, &mut queue, house).await {
                Ok(next) => in_flight = next,
                Err(e) => return End::Lost(e),
            }
        }
        let ack_until = in_flight.as_ref().map(|f| f.until);
        let follow_due = house.follows.iter().map(|(_, _, until)| *until).min();
        let idle = in_flight.is_none() && queue.is_empty();
        tokio::select! {
            command = ctx.next_command() => {
                let Some(command) = command else {
                    fail(in_flight.take(), &mut queue, house);
                    return End::Stopped;
                };
                accept(house, &mut queue, command);
            }
            incoming = mqtt.next() => match incoming {
                Ok(Incoming::Message { topic, payload }) => heard(ctx, house, &topic, &payload),
                Ok(Incoming::PubAck(pkid)) => {
                    if in_flight.as_ref().is_some_and(|f| f.pkid == pkid)
                        && let Some(f) = in_flight.take()
                    {
                        // The broker has it: answered (even to a caller who
                        // left, for the journal), its effect followed.
                        f.command.reply(Ok(()));
                        house.follows.push((f.id, f.name, Instant::now() + FOLLOW));
                    }
                }
                Ok(Incoming::Pong) => awaiting_pong = false,
                Ok(Incoming::Other) => {}
                Err(e) => {
                    fail(in_flight.take(), &mut queue, house);
                    return End::Lost(e);
                }
            },
            () = sleep_until(ack_until.unwrap_or_else(far)), if ack_until.is_some() => {
                // No acknowledgement in time: it may have left, never again.
                if let Some(f) = in_flight.take() {
                    f.command.reply(Err(moli_i18n::tr!("pilotes.landroid.non_confirme")));
                    house.follows.push((f.id, f.name, Instant::now() + FOLLOW));
                }
            }
            () = sleep_until(ping_at) => {
                if awaiting_pong {
                    fail(in_flight.take(), &mut queue, house);
                    return End::Lost(anyhow::anyhow!("mqtt: no answer to the keep-alive"));
                }
                if let Ok(Ok(())) = timeout(moli_net::TIMEOUT, mqtt.ping()).await {
                    awaiting_pong = true;
                    ping_at = Instant::now() + PING_EVERY;
                } else {
                    fail(in_flight.take(), &mut queue, house);
                    return End::Lost(anyhow::anyhow!("mqtt: keep-alive not sent"));
                }
            }
            () = sleep_until(follow_due.unwrap_or_else(far)), if follow_due.is_some() => {
                let now = Instant::now();
                let (due, kept): (Vec<_>, Vec<_>) =
                    house.follows.drain(..).partition(|(_, _, until)| *until <= now);
                house.follows = kept;
                for (_, name, _) in due {
                    media.tell(
                        ctx.instance().as_str(),
                        moli_i18n::tr!("pilotes.landroid.sans_reponse", name = name),
                    );
                }
            }
            () = sleep_until(renew_at), if idle => return End::Renew,
        }
    }
}

/// Runs `work`, refusing orders with `why` meanwhile (nothing is queued
/// while there is no link). `None`: the system stops.
async fn refusing<T>(
    ctx: &mut DriverCtx,
    work: impl std::future::Future<Output = T>,
    why: &str,
) -> Option<T> {
    tokio::pin!(work);
    loop {
        tokio::select! {
            out = &mut work => return Some(out),
            command = ctx.next_command() => match command {
                Some(command) => command.reply(Err(why.to_owned())),
                None => return None,
            },
        }
    }
}

/// Waits `delay`, refusing orders. `false`: the system stops.
async fn pause(ctx: &mut DriverCtx, delay: Duration, why: &str) -> bool {
    refusing(ctx, tokio::time::sleep(delay), why)
        .await
        .is_some()
}

/// Refuses orders until the system stops: a person must act first.
async fn park(ctx: &mut DriverCtx, why: &str) {
    while let Some(command) = ctx.next_command().await {
        command.reply(Err(why.to_owned()));
    }
}

/// The client id: the account and a fresh random suffix (two clients with
/// one id push each other off the broker).
fn client_id(user_id: &str) -> String {
    let mut raw = [0u8; 8];
    let _ = SystemRandom::new().fill(&mut raw);
    format!("WX/USER/{user_id}/moli/{:016x}", u64::from_be_bytes(raw))
}

type Link = Mqtt<moli_net::ws::WebSocket>;

/// The session, the mowers (read again when stale), the broker.
async fn connect(
    account: &Account,
    listed: &mut Option<(Instant, Vec<Mower>)>,
) -> anyhow::Result<(Link, Instant, Vec<Mower>)> {
    let (access, renew_at) = account.access().await?;
    let mowers = match listed {
        Some((at, mowers)) if at.elapsed() < LIST_FRESH => mowers.clone(),
        _ => {
            let mowers = account.mowers().await?;
            *listed = Some((Instant::now(), mowers.clone()));
            mowers
        }
    };
    let Some(first) = mowers.first() else {
        *listed = None;
        anyhow::bail!(NoMower);
    };
    // One broker per account; a mower elsewhere is not followed.
    let endpoint = first.endpoint.clone();
    let user_id = first.user_id.clone();
    let username = model::username(&access).ok_or_else(|| anyhow::anyhow!("Worx: odd token"))?;
    let ws = moli_net::ws::WebSocket::connect_public(
        &endpoint,
        443,
        "/mqtt",
        &[("Sec-WebSocket-Protocol", "mqtt")],
    )
    .await?;
    let mut mqtt = Mqtt::open(ws, &client_id(&user_id), &username, KEEP_ALIVE).await?;
    let topics: Vec<String> = mowers
        .iter()
        .filter(|m| m.endpoint == endpoint)
        .map(|m| m.command_out.clone())
        .collect();
    mqtt.subscribe(&topics).await?;
    let mowers = mowers
        .into_iter()
        .filter(|m| {
            let same = m.endpoint == endpoint;
            if !same {
                tracing::warn!(mower = %m.name, "landroid: on another broker, not followed");
            }
            same
        })
        .collect();
    Ok((mqtt, renew_at, mowers))
}

#[derive(Debug)]
struct NoMower;

impl std::fmt::Display for NoMower {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("no Landroid on the account")
    }
}

impl std::error::Error for NoMower {}

async fn run(sessions: Arc<Sessions>, ctx: &mut DriverCtx) -> anyhow::Result<()> {
    let (Some(user), Some(password)) = (ctx.secret(USERNAME), ctx.secret(PASSWORD)) else {
        let why = moli_i18n::tr!(
            "pilotes.landroid.identifiants_manquants",
            instance = ctx.instance()
        );
        ctx.wait_for(why.clone());
        park(ctx, &why).await;
        return Ok(());
    };
    let account = Account::new(user, password, sessions);
    let media = ctx.media_publisher();
    let mut house = House::default();
    for id in ctx.known_devices() {
        ctx.set_availability(&id, false);
    }
    let mut listed = None;
    let mut retry = RETRY_MIN;
    let offline_why = moli_i18n::tr!("pilotes.landroid.cloud_injoignable");
    loop {
        let Some(attempt) =
            refusing(ctx, Box::pin(connect(&account, &mut listed)), &offline_why).await
        else {
            return Ok(());
        };
        let (mut mqtt, renew_at, mowers) = match attempt {
            Ok(connected) => connected,
            Err(e) => {
                house.offline(ctx);
                let (why, delay) = match refusal(&e) {
                    Some(Refusal::Credentials) => {
                        let why = moli_i18n::tr!(
                            "pilotes.landroid.identifiants_refuses",
                            instance = ctx.instance()
                        );
                        ctx.wait_for(why.clone());
                        park(ctx, &why).await;
                        return Ok(());
                    }
                    Some(Refusal::TooMany) => {
                        (moli_i18n::tr!("pilotes.landroid.trop_d_appels"), LIMITED)
                    }
                    Some(Refusal::Session) => (offline_why.clone(), RETRY_MIN),
                    _ if e.is::<NoMower>() => {
                        (moli_i18n::tr!("pilotes.landroid.aucune"), NO_MOWER_RETRY)
                    }
                    _ => {
                        if e.to_string().contains("connection refused") {
                            // The broker turned the token down: a fresh session next time.
                            account.forget();
                        }
                        let delay = retry;
                        retry = (retry * 2).min(RETRY_MAX);
                        (
                            moli_i18n::tr!("pilotes.landroid.injoignable", cause = e.root_cause()),
                            delay,
                        )
                    }
                };
                ctx.wait_for(why.clone());
                if !pause(ctx, delay, &why).await {
                    return Ok(());
                }
                continue;
            }
        };
        publish(ctx, &mut house, mowers);
        ctx.ready();
        let started = Instant::now();
        let end = serve(ctx, &mut house, &mut mqtt, renew_at, &media).await;
        if started.elapsed() >= STABLE {
            retry = RETRY_MIN;
        }
        match end {
            End::Stopped => return Ok(()),
            End::Renew => {}
            End::Lost(e) => {
                house.offline(ctx);
                let why = moli_i18n::tr!("pilotes.landroid.injoignable", cause = e.root_cause());
                ctx.wait_for(why.clone());
                let delay = retry;
                retry = (retry * 2).min(RETRY_MAX);
                if !pause(ctx, delay, &why).await {
                    return Ok(());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
