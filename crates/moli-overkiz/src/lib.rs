//! Somfy TaHoma and Connectivity Kit boxes (Overkiz API): RTS shutters and
//! gate openers, OGP gate, light and outlet.
//!
//! Two roads to the same API: the box's **local** API (developer mode, its
//! certificate pinned at first contact, a token a human made), or the
//! **Somfy cloud** (a Connectivity Kit has no local API). Either way: the
//! devices are loaded, an event listener is registered and fetched (states,
//! availability, executions), and orders go out as executions.
//!
//! An order is handed to the box **once** (never retried, never replayed
//! after a reconnection) and followed: the caller hears « done » when the
//! box says the execution completed, its failure when it failed, and, if
//! the box is still at work when the hub stops waiting, « accepted »; a
//! failure after that is told to the household. Every wait on the network
//! runs beside the driver's loop, so no slow fetch or slow order ever
//! delays an answer past its deadline. An RTS motor never says where it is:
//! Moli never shows a position it does not know.

mod api;
#[cfg(test)]
mod fake;
mod follow;
mod model;

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};
use moli_core::DeviceId;
use moli_runtime::{BoxFuture, CommandRequest, Driver, DriverCtx};
use serde::Deserialize;
use serde_json::Value as Json;
use tokio::task::JoinSet;
use tokio::time::sleep_until;

use crate::follow::{About, Due, Outcome, Tracker};

pub use api::{Api, Cloud, Local, Refused, SharedSession};
pub use model::{Event, Execution, Index, Load, MOVES, Order, Refusal, SUPPORTED};

const TOKEN: &str = "token";
const CERT: &str = "cert_sha256";
const USERNAME: &str = "username";
const PASSWORD: &str = "password";

/// Between two fetches when nothing is going on. The cloud counts calls.
const IDLE_LOCAL: Duration = Duration::from_secs(10);
const IDLE_CLOUD: Duration = Duration::from_secs(30);
/// While an execution is followed. Somfy asks for one fetch a second at most.
const FOLLOW_LOCAL: Duration = Duration::from_secs(1);
const FOLLOW_CLOUD: Duration = Duration::from_secs(2);
/// Quota reached, queue full, maintenance: leave the box alone this long,
/// reads and orders alike.
const BUSY_PAUSE: Duration = Duration::from_secs(60);
/// The caller is answered this long before the hub stops waiting.
const REPLY_MARGIN: Duration = Duration::from_millis(400);
/// Refused sessions in a row before a human is asked (the cloud bans
/// accounts that log in too often).
const MAX_SESSION_REFUSALS: u32 = 3;
/// Orders waiting for the box behind the one being sent; more are refused
/// at once (the hub's own queue must not turn into unbounded work here).
const MAX_WAITING: usize = 8;

/// Which road leads to the box.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    /// The box's local API (TaHoma in developer mode).
    Local,
    /// The Somfy cloud (Connectivity Kit, or a box without developer mode).
    Cloud,
}

/// Driver configuration (`[driver.options]` in `moli.toml`).
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub transport: Transport,
    /// The box's address on the home network (local only).
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_port() -> u16 {
    8443
}

#[derive(Debug)]
pub struct Overkiz {
    config: Config,
    /// The cloud session outlives a restart of the driver.
    session: SharedSession,
}

impl Overkiz {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        match (config.transport, config.host.as_deref().map(str::trim)) {
            (Transport::Local, None | Some("")) => {
                bail!("transport \"local\" needs the box's address: host")
            }
            (Transport::Cloud, Some(_)) => {
                bail!("transport \"cloud\" takes no host: the Somfy cloud's address is fixed")
            }
            _ => Ok(Self {
                config,
                session: SharedSession::default(),
            }),
        }
    }
}

impl Driver for Overkiz {
    fn kind(&self) -> &'static str {
        "overkiz"
    }

    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(run(self, ctx))
    }
}

/// Shows `reason` on every surface and idles until shutdown. Nothing is
/// read any more: the devices are offline until a human acts and Moli
/// restarts (the supervisor only does it when `run` returns).
async fn park(ctx: &DriverCtx, reason: String) -> anyhow::Result<()> {
    for id in ctx.devices() {
        ctx.set_availability(&id, false);
    }
    ctx.wait_for(reason);
    ctx.cancelled().await;
    Ok(())
}

/// The road to the box, or `None` when a human must act first (parked).
async fn connect(driver: &Overkiz, ctx: &DriverCtx) -> anyhow::Result<Option<Api>> {
    let instance = ctx.instance().to_string();
    if driver.config.transport == Transport::Cloud {
        let (Some(user), Some(password)) = (ctx.secret(USERNAME), ctx.secret(PASSWORD)) else {
            let reason =
                moli_i18n::tr!("pilotes.overkiz.identifiants_absents", instance = instance);
            park(ctx, reason).await?;
            return Ok(None);
        };
        return Ok(Some(Api::Cloud(Cloud::new(
            user,
            password,
            Arc::clone(&driver.session),
        ))));
    }
    let host = driver.config.host.as_deref().unwrap_or_default().trim();
    let port = driver.config.port;
    let Some(token) = ctx.secret(TOKEN) else {
        let reason = moli_i18n::tr!("pilotes.overkiz.jeton_absent", instance = instance);
        park(ctx, reason).await?;
        return Ok(None);
    };
    let pin = match ctx.secret(CERT) {
        Some(pin) => pin,
        None if !ctx.can_store_secrets() => {
            let problem = ctx.secrets_problem().map_or_else(
                || moli_i18n::tr!("pilotes.overkiz.coffre_indisponible"),
                str::to_owned,
            );
            let reason = moli_i18n::tr!("pilotes.overkiz.epinglage_impossible", problem = problem);
            park(ctx, reason).await?;
            return Ok(None);
        }
        // Pinned and stored before the token ever leaves.
        None => {
            let seen = Local::first_contact(host, port).await?;
            ctx.store_secret(CERT, &seen).await?;
            tracing::info!(instance = %ctx.instance(), "overkiz box certificate pinned");
            seen
        }
    };
    Ok(Some(Api::Local(Box::new(Local::new(
        host, port, pin, token,
    )?))))
}

async fn run(driver: &Overkiz, ctx: &mut DriverCtx) -> anyhow::Result<()> {
    let Some(api) = connect(driver, ctx).await? else {
        return Ok(());
    };
    let pace = if matches!(api, Api::Cloud(_)) {
        (IDLE_CLOUD, FOLLOW_CLOUD)
    } else {
        (IDLE_LOCAL, FOLLOW_LOCAL)
    };
    drive(
        api,
        ctx,
        pace,
        driver.config.host.as_deref().unwrap_or_default(),
    )
    .await
}

/// Runs the link to the box until shutdown, a failure for the supervisor,
/// or a refusal only a human can lift (then parked, devices offline).
async fn drive(
    api: Api,
    ctx: &mut DriverCtx,
    (idle, follow): (Duration, Duration),
    host: &str,
) -> anyhow::Result<()> {
    let cloud = matches!(api, Api::Cloud(_));
    let mut link = Link::new(api, idle, follow);
    let result = link.serve(ctx).await;
    // Whatever is still on its way is dropped here, never sent later.
    link.jobs.abort_all();
    for queued in link.queue.drain(..) {
        if let Some(command) = link.tracker.refused(queued.key) {
            command.reply(Err(moli_i18n::tr!("pilotes.overkiz.non_envoye")));
        }
    }
    // Sent, never confirmed: say so rather than let the hub call it a crash.
    for command in link.tracker.abandon() {
        command.reply(Err(moli_i18n::tr!("pilotes.overkiz.confirmation_perdue")));
    }
    let Err(e) = result else { return Ok(()) };
    let instance = ctx.instance().to_string();
    // A local box has no session to renew: a refused bearer means its token
    // was revoked, a human's call like a refused account.
    let denied = e
        .downcast_ref::<Refused>()
        .filter(|r| r.kind == Refusal::Denied || (r.kind == Refusal::Session && !cloud));
    if let Some(Refused { message, .. }) = denied {
        let reason = if let Api::Cloud(cloud) = &*link.api {
            cloud.forget().await;
            moli_i18n::tr!(
                "pilotes.overkiz.identifiants_refuses",
                cause = message,
                instance = instance
            )
        } else {
            moli_i18n::tr!(
                "pilotes.overkiz.jeton_refuse",
                cause = message,
                instance = instance
            )
        };
        return park(ctx, reason).await;
    }
    if format!("{e:#}").contains(moli_net::PIN_MISMATCH) {
        let reason = moli_i18n::tr!(
            "pilotes.overkiz.certificat_change",
            host = host,
            instance = instance,
            key = CERT
        );
        return park(ctx, reason).await;
    }
    Err(e)
}

/// A read of the box, done beside the loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Read {
    /// A new event listener, then every device (start, listener expired).
    Sync,
    /// Every device again (added, removed, renamed; a box back).
    Load,
    /// The events since the last fetch.
    Fetch,
}

/// What a task beside the loop brings back.
#[derive(Debug)]
enum Done {
    Read(Read, anyhow::Result<(Option<String>, Json)>),
    /// The box's answer to order `key`: its execution, or why not.
    Applied(u64, anyhow::Result<String>),
}

/// The box did not answer an order within its time: maybe sent, never resent.
#[derive(Debug)]
struct NoAnswer;

impl std::fmt::Display for NoAnswer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("no answer in time")
    }
}

impl std::error::Error for NoAnswer {}

#[derive(Debug)]
struct Link {
    api: Arc<Api>,
    idle: Duration,
    follow: Duration,
    index: Index,
    listener: String,
    tracker: Tracker<CommandRequest>,
    /// Network work in flight: one read at most, and the orders.
    jobs: JoinSet<Done>,
    reading: bool,
    /// The next read and when (a failed one is tried again).
    next: Read,
    next_at: Instant,
    /// Quota, full queue or maintenance: nothing goes to the box before.
    busy_until: Option<Instant>,
    synced: bool,
    /// Orders not sent yet, in the order the hub gave them: they leave one
    /// at a time, so they reach the box in that order too.
    queue: VecDeque<Queued>,
    /// An order is being handed to the box.
    writing: bool,
    orders: u64,
}

/// An order waiting for its turn.
#[derive(Debug)]
struct Queued {
    key: u64,
    order: Order,
    /// The absolute instant past which it must not leave.
    reply_by: Instant,
}

impl Link {
    fn new(api: Api, idle: Duration, follow: Duration) -> Self {
        Self {
            api: Arc::new(api),
            idle,
            follow,
            index: Index::default(),
            listener: String::new(),
            tracker: Tracker::default(),
            jobs: JoinSet::new(),
            reading: false,
            next: Read::Sync,
            next_at: Instant::now(),
            busy_until: None,
            synced: false,
            queue: VecDeque::new(),
            writing: false,
            orders: 0,
        }
    }

    fn busy(&self, now: Instant) -> bool {
        self.busy_until.is_some_and(|until| now < until)
    }

    async fn serve(&mut self, ctx: &mut DriverCtx) -> anyhow::Result<()> {
        loop {
            let now = Instant::now();
            // Answers first: due callers never wait on anything.
            for due in self.tracker.due(now) {
                match due {
                    Due::Accepted(command) => command.reply(Ok(())),
                    Due::Unanswered(command) => {
                        command.reply(Err(moli_i18n::tr!("pilotes.overkiz.box_muette")));
                    }
                    Due::NotSent(command) => {
                        command.reply(Err(moli_i18n::tr!("pilotes.overkiz.non_envoye")));
                    }
                }
            }
            self.pump(now);
            if !self.reading && now >= self.next_at && !self.busy(now) {
                self.read();
            }
            let read_at =
                (!self.reading).then(|| self.next_at.max(self.busy_until.unwrap_or(self.next_at)));
            let wake = self.tracker.next_reply().into_iter().chain(read_at).min();
            tokio::select! {
                () = sleep(wake) => {}
                Some(done) = self.jobs.join_next() => {
                    let done = done.context("overkiz task")?;
                    self.done(ctx, done)?;
                }
                command = ctx.next_command() => match command {
                    Some(command) => self.order(command),
                    None => return Ok(()),
                },
            }
        }
    }

    /// Starts the next read beside the loop.
    fn read(&mut self) {
        let api = Arc::clone(&self.api);
        let kind = self.next;
        let listener = self.listener.clone();
        self.reading = true;
        self.jobs.spawn(async move {
            let result: anyhow::Result<(Option<String>, Json)> = async {
                match kind {
                    Read::Sync => {
                        let answer = api.post("/events/register").await?;
                        let listener = api::identifier(&answer["id"])?;
                        Ok((Some(listener), api.get("/setup/devices").await?))
                    }
                    Read::Load => Ok((None, api.get("/setup/devices").await?)),
                    Read::Fetch => {
                        Ok((None, api.post(&format!("/events/{listener}/fetch")).await?))
                    }
                }
            }
            .await;
            Done::Read(kind, result)
        });
    }

    fn done(&mut self, ctx: &DriverCtx, done: Done) -> anyhow::Result<()> {
        let now = Instant::now();
        match done {
            Done::Read(kind, Ok((listener, answer))) => {
                self.reading = false;
                if let Some(listener) = listener {
                    self.listener = listener;
                }
                let reload = match kind {
                    Read::Sync | Read::Load => {
                        self.publish(ctx, &answer);
                        if !self.synced {
                            self.synced = true;
                            ctx.ready();
                        }
                        false
                    }
                    Read::Fetch => self.events(ctx, &answer),
                };
                self.next = if reload { Read::Load } else { Read::Fetch };
                self.next_at = if reload {
                    now
                } else if self.tracker.watching() {
                    now + self.follow
                } else {
                    now + self.idle
                };
            }
            Done::Read(kind, Err(e)) => {
                self.reading = false;
                self.next = kind;
                self.next_at = now + self.follow;
                match api::refused(&e) {
                    Some(Refusal::Listener) => {
                        tracing::info!(instance = %ctx.instance(), "overkiz listener expired, registering again");
                        self.next = Read::Sync;
                        self.next_at = now;
                    }
                    Some(Refusal::Busy | Refusal::Session) => self.refused(ctx, &e)?,
                    // A fetch whose answer was lost may have taken events
                    // with it: the link is no longer trusted (the callers
                    // are told, nothing is resent).
                    _ => return Err(e),
                }
            }
            Done::Applied(key, Ok(execution)) => {
                self.writing = false;
                if let Some(outcome) = self.tracker.taken(key, execution) {
                    outcome_of(ctx, outcome);
                }
                // Follow it closely.
                self.next_at = self.next_at.min(now + self.follow);
            }
            Done::Applied(key, Err(e)) => {
                self.writing = false;
                let message = if e.is::<NoAnswer>() {
                    moli_i18n::tr!("pilotes.overkiz.box_muette")
                } else if e.is::<api::NotSent>() {
                    moli_i18n::tr!("pilotes.overkiz.non_envoye")
                } else {
                    let cause = e.to_string();
                    match api::refused(&e) {
                        Some(Refusal::Busy) => {
                            moli_i18n::tr!("pilotes.overkiz.occupee", cause = cause)
                        }
                        Some(_) => moli_i18n::tr!("pilotes.overkiz.refus", cause = cause),
                        None => format!("{e:#}"),
                    }
                };
                if let Some(command) = self.tracker.refused(key) {
                    command.reply(Err(message));
                }
                match api::refused(&e) {
                    Some(Refusal::Busy | Refusal::Session) => self.refused(ctx, &e)?,
                    Some(Refusal::Denied) => return Err(e),
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// A refusal the link survives: a pause, or a new session (bounded).
    fn refused(&mut self, ctx: &DriverCtx, e: &anyhow::Error) -> anyhow::Result<()> {
        match api::refused(e) {
            Some(Refusal::Busy) => {
                tracing::warn!(instance = %ctx.instance(), error = %e, "overkiz busy, pausing");
                self.busy_until = Some(Instant::now() + BUSY_PAUSE);
            }
            // The cloud ended the session (forgotten by the transport): the
            // next call logs in again, a few times at most.
            Some(Refusal::Session) if matches!(*self.api, Api::Cloud(_)) => {
                if self.api.refused_sessions() >= MAX_SESSION_REFUSALS {
                    return Err(Refused {
                        kind: Refusal::Denied,
                        message: e.to_string(),
                    }
                    .into());
                }
            }
            _ => {
                return Err(Refused {
                    kind: Refusal::Session,
                    message: e.to_string(),
                }
                .into());
            }
        }
        Ok(())
    }

    /// Publishes every device of a `/setup/devices` answer.
    fn publish(&mut self, ctx: &DriverCtx, answer: &Json) {
        let load = model::load(ctx.instance(), answer);
        let keep: Vec<&DeviceId> = load.devices.iter().map(|f| &f.device.id).collect();
        for id in ctx.devices() {
            if !keep.contains(&&id) {
                ctx.remove_device(&id);
            }
        }
        if !load.skipped.is_empty() {
            tracing::info!(instance = %ctx.instance(), skipped = ?load.skipped, "overkiz classes not published");
        }
        let count = load.devices.len();
        for model::Found {
            device,
            online,
            values,
        } in load.devices
        {
            let id = device.id.clone();
            ctx.upsert_device(device);
            for (key, value) in values {
                ctx.set_state(&id, key, value);
            }
            ctx.set_availability(&id, online);
        }
        self.index = load.index;
        tracing::info!(instance = %ctx.instance(), devices = count, "overkiz devices loaded");
    }

    /// Applies fetched events. `true` when the devices must be loaded again.
    fn events(&mut self, ctx: &DriverCtx, answer: &Json) -> bool {
        let mut reload = false;
        for event in model::events(answer) {
            match event {
                Event::States { url, values } => {
                    if let Some(id) = self.index.device(&url) {
                        for (key, value) in values {
                            ctx.set_state(id, key, value);
                        }
                    }
                }
                Event::Available { url, online } => {
                    if let Some(id) = self.index.device(&url) {
                        ctx.set_availability(id, online);
                    }
                }
                Event::GatewayDown { gateway } => {
                    for id in self.index.devices_on(&gateway) {
                        ctx.set_availability(id, false);
                    }
                }
                Event::Execution { id, state, cause } => {
                    if let Some(outcome) = self.tracker.moved(&id, state, cause) {
                        outcome_of(ctx, outcome);
                    }
                }
                Event::Reload => reload = true,
            }
        }
        reload
    }

    /// Takes an order from the hub: refused at once, or queued for the box.
    fn order(&mut self, command: CommandRequest) {
        let now = Instant::now();
        if !self.synced {
            command.reply(Err(moli_i18n::tr!("pilotes.overkiz.pas_pret")));
            return;
        }
        if let Some(refusal) = self.paused(now) {
            command.reply(Err(refusal));
            return;
        }
        let Some(order) = self
            .index
            .encode(&command.device.id, &command.key, &command.value)
        else {
            command.reply(Err(moli_i18n::tr!("pilotes.overkiz.point_non_ecrivable")));
            return;
        };
        if self.queue.len() >= MAX_WAITING {
            command.reply(Err(moli_i18n::tr!("pilotes.overkiz.trop_d_ordres")));
            return;
        }
        let reply_by = command
            .deadline
            .checked_sub(REPLY_MARGIN)
            .unwrap_or(command.deadline);
        if now >= reply_by {
            command.reply(Err("expired".into()));
            return;
        }
        self.orders += 1;
        let key = self.orders;
        let about = About {
            device: Arc::clone(&command.device.native_name),
            order: order.command,
        };
        self.tracker.track(key, command, about, reply_by);
        self.queue.push_back(Queued {
            key,
            order,
            reply_by,
        });
    }

    /// Why no order may leave now, if so.
    fn paused(&self, now: Instant) -> Option<String> {
        let until = self.busy_until.filter(|until| now < *until)?;
        let seconds = until.saturating_duration_since(now).as_secs().max(1);
        Some(moli_i18n::tr!(
            "pilotes.overkiz.en_pause",
            seconds = seconds
        ))
    }

    /// Sends the next order if none is on its way, oldest first. Each is
    /// checked just before it leaves: still awaited, box not paused, still
    /// in time (an order past its instant stays home, never sent late).
    fn pump(&mut self, now: Instant) {
        while !self.writing {
            let Some(next) = self.queue.pop_front() else {
                return;
            };
            if !self.tracker.waiting(next.key) {
                continue; // its caller already heard « not sent »
            }
            if let Some(refusal) = self.paused(now) {
                if let Some(command) = self.tracker.refused(next.key) {
                    command.reply(Err(refusal));
                }
                continue;
            }
            if now >= next.reply_by {
                if let Some(command) = self.tracker.refused(next.key) {
                    command.reply(Err(moli_i18n::tr!("pilotes.overkiz.non_envoye")));
                }
                continue;
            }
            self.tracker.sending(next.key);
            self.writing = true;
            let api = Arc::clone(&self.api);
            self.jobs.spawn(async move {
                Done::Applied(next.key, emit(&api, &next.order, next.reply_by).await)
            });
        }
    }
}

/// One order to the box, never after `deadline`: refused unsent if the
/// task starts late, abandoned at that instant if the box is slow (maybe
/// sent then, never sent again).
async fn emit(api: &Api, order: &Order, deadline: Instant) -> anyhow::Result<String> {
    if Instant::now() >= deadline {
        return Err(api::NotSent.into());
    }
    tokio::time::timeout_at(deadline.into(), api.apply(order, deadline))
        .await
        .unwrap_or_else(|_| Err(NoAnswer.into()))
}

/// What an execution's end means for its caller or the household.
fn outcome_of(ctx: &DriverCtx, outcome: Outcome<CommandRequest>) {
    match outcome {
        Outcome::Answer(command, result) => command
            .reply(result.map_err(|cause| moli_i18n::tr!("pilotes.overkiz.echec", cause = cause))),
        Outcome::Tell(about, cause) => ctx.media_publisher().tell(
            ctx.instance().as_str(),
            moli_i18n::tr!(
                "pilotes.overkiz.echec_tardif",
                device = about.device,
                order = about.order,
                cause = cause
            ),
        ),
    }
}

async fn sleep(until: Option<Instant>) {
    match until {
        Some(until) => sleep_until(until.into()).await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests;
