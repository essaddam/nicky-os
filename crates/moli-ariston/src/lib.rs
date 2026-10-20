//! Ariston: Velis water heaters (Velis Evo 2, Andris 2) through the
//! Ariston NET cloud, with the household's account.
//!
//! A water heater is not a relay: Moli reads its tank temperature, its
//! setpoint and the one it really heats to, its mode (by hand or on its own
//! schedule), eco mode, showers ready, time left, the anti-legionella cycle
//! and its errors; it switches it on or off and sets the setpoint, the
//! mode, eco, the setpoint's ceiling and the anti-legionella safeguard.
//!
//! Reads every 3 minutes (every 20 s while an order is being confirmed),
//! its settings and errors every 30 minutes. An order is handed to the
//! cloud **once** (never retried, never after its time), one at a time in
//! the order received; the cloud's « OK » is not the heater's: the value
//! must read back within 3 minutes, or the household is told. Every wait on
//! the network runs beside the loop, so an answer is always on time.

mod api;
mod model;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::bail;
use moli_core::{Device, DeviceId, Value};
use moli_runtime::orders::{Confirmations, OrderLine};
use moli_runtime::{BoxFuture, CommandRequest, Driver, DriverCtx, SendPermit};
use serde::Deserialize;
use serde_json::Value as Json;
use tokio::task::JoinSet;

pub use api::{Cloud, Refusal, Refused, Sessions};
use model::Heater;

const USERNAME: &str = "username";
const PASSWORD: &str = "password";

/// Between two reads when nothing is going on: the cloud limits callers.
const IDLE: Duration = Duration::from_secs(180);
/// While an order is being confirmed.
const FOLLOW: Duration = Duration::from_secs(20);
/// Settings and errors; the list of heaters.
const SLOW: Duration = Duration::from_secs(1800);
const RELOAD: Duration = Duration::from_secs(6 * 3600);
/// A value the cloud took must read back within this (the heater hears
/// its gateway, the gateway the cloud).
const CONFIRM: Duration = Duration::from_secs(180);
/// Too many requests, or the service down: nothing goes to it before.
const BUSY_PAUSE: Duration = Duration::from_secs(900);
/// After a failed read; a few in a row and the link restarts.
const RETRY: Duration = Duration::from_secs(60);
const MAX_READ_FAILURES: u32 = 3;
/// Refused sessions in a row before a person is asked.
const MAX_SESSION_REFUSALS: u32 = 3;
/// Callers waiting for the cloud at once, orders sent or not.
const LINE: usize = 4;

/// How often it reads, and how long a value may take to read back.
#[derive(Clone, Copy, Debug)]
struct Pace {
    idle: Duration,
    follow: Duration,
    confirm: Duration,
}

const PACE: Pace = Pace {
    idle: IDLE,
    follow: FOLLOW,
    confirm: CONFIRM,
};

/// Driver configuration: nothing, the cloud's address is fixed.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {}

#[derive(Debug, Default)]
pub struct Ariston {
    /// The session outlives a restart of the driver.
    sessions: Arc<Sessions>,
}

impl Ariston {
    #[must_use]
    pub fn new(_config: Config) -> Self {
        Self::default()
    }
}

impl Driver for Ariston {
    fn kind(&self) -> &'static str {
        "ariston"
    }

    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let (Some(user), Some(password)) = (ctx.secret(USERNAME), ctx.secret(PASSWORD)) else {
                let instance = ctx.instance().to_string();
                let reason =
                    moli_i18n::tr!("pilotes.ariston.identifiants_absents", instance = instance);
                return park(ctx, reason).await;
            };
            let cloud = Cloud::new(user, password, Arc::clone(&self.sessions));
            drive(cloud, ctx, PACE).await
        })
    }
}

/// Shows `reason` on every surface and idles until shutdown: the devices
/// are offline until a person acts and Moli restarts.
async fn park(ctx: &DriverCtx, reason: String) -> anyhow::Result<()> {
    for id in ctx.devices() {
        ctx.set_availability(&id, false);
    }
    ctx.wait_for(reason);
    ctx.cancelled().await;
    Ok(())
}

/// Runs the link until shutdown, a failure for the supervisor, or a
/// refusal only a person can lift (then parked).
async fn drive(cloud: Cloud, ctx: &mut DriverCtx, pace: Pace) -> anyhow::Result<()> {
    let mut link = Link::new(cloud, pace);
    let result = link.serve(ctx).await;
    // Whatever is still on its way is dropped here, never sent later.
    link.jobs.abort_all();
    link.line.close();
    link.confirm.close();
    let Err(e) = result else { return Ok(()) };
    if api::refused(&e) == Some(Refusal::Denied) {
        link.cloud.forget();
        let instance = ctx.instance().to_string();
        let reason = moli_i18n::tr!(
            "pilotes.ariston.identifiants_refuses",
            cause = e.to_string(),
            instance = instance
        );
        return park(ctx, reason).await;
    }
    Err(e)
}

/// What a read covers, done beside the loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Read {
    /// The account's heaters again.
    list: bool,
    /// Their settings (ceiling, anti-legionella).
    settings: bool,
    errors: bool,
}

/// One heater, as read.
#[derive(Debug)]
struct Reading {
    heater: Heater,
    data: Json,
    settings: Option<Json>,
    errors: Option<Json>,
}

#[derive(Debug)]
struct Snapshot {
    /// The account's heaters, when the list was read.
    listed: bool,
    readings: Vec<Reading>,
}

/// An order for the cloud, and what it should read back.
#[derive(Clone, Debug)]
struct Order {
    path: String,
    body: Json,
    device: Arc<Device>,
    key: String,
    value: Value,
}

#[derive(Debug)]
enum Done {
    Read(Read, anyhow::Result<Snapshot>),
    Sent(Order, anyhow::Result<()>),
}

/// The cloud did not answer an order within its time: maybe taken, never
/// resent.
#[derive(Debug)]
struct NoAnswer(String);

impl std::fmt::Display for NoAnswer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for NoAnswer {}

#[derive(Debug)]
struct Link {
    cloud: Arc<Cloud>,
    pace: Pace,
    /// The heaters, and their settings as last read.
    heaters: HashMap<DeviceId, (Heater, Json)>,
    line: OrderLine<Order>,
    confirm: Confirmations,
    /// Network work in flight: one read and one order at most.
    jobs: JoinSet<Done>,
    reading: bool,
    read_started: Instant,
    next_read: Instant,
    next_slow: Instant,
    next_list: Instant,
    busy_until: Option<Instant>,
    failures: u32,
    ready: bool,
    #[cfg(test)]
    settled: Option<Arc<tokio::sync::Notify>>,
}

impl Link {
    fn new(cloud: Cloud, pace: Pace) -> Self {
        let now = Instant::now();
        Self {
            cloud: Arc::new(cloud),
            pace,
            heaters: HashMap::new(),
            line: OrderLine::new(LINE),
            confirm: Confirmations::default(),
            jobs: JoinSet::new(),
            reading: false,
            read_started: now,
            next_read: now,
            next_slow: now,
            next_list: now,
            busy_until: None,
            failures: 0,
            ready: false,
            #[cfg(test)]
            settled: None,
        }
    }

    fn busy(&self, now: Instant) -> Option<Duration> {
        self.busy_until
            .filter(|until| now < *until)
            .map(|until| until - now)
    }

    async fn serve(&mut self, ctx: &mut DriverCtx) -> anyhow::Result<()> {
        self.confirm.bind(ctx);
        loop {
            self.confirm.expire();
            let now = Instant::now();
            // Answers first: due callers never wait on anything.
            let answer_at = self.line.due();
            self.pump(now);
            if !self.reading && now >= self.next_read && self.busy(now).is_none() {
                self.read(now);
            }
            let read_at = (!self.reading).then(|| {
                self.next_read
                    .max(self.busy_until.unwrap_or(self.next_read))
            });
            let wake = answer_at
                .into_iter()
                .chain(read_at)
                .chain(self.confirm.next_deadline())
                .min();
            tokio::select! {
                () = sleep(wake) => {}
                Some(done) = self.jobs.join_next() => {
                    let done = done.map_err(|e| anyhow::anyhow!("ariston task: {e}"))?;
                    self.done(ctx, done)?;
                }
                command = ctx.next_command() => match command {
                    Some(command) => self.order(command),
                    None => return Ok(()),
                },
            }
        }
    }

    fn read(&mut self, now: Instant) {
        let slow = now >= self.next_slow;
        let kind = Read {
            list: self.heaters.is_empty() || now >= self.next_list,
            // Read back with the rest while an order is being confirmed.
            settings: slow || self.confirm.watching(),
            errors: slow,
        };
        let cloud = Arc::clone(&self.cloud);
        let known: Vec<Heater> = self.heaters.values().map(|(h, _)| h.clone()).collect();
        self.reading = true;
        self.read_started = now;
        self.jobs
            .spawn(async move { Done::Read(kind, fetch(&cloud, known, kind).await) });
    }

    fn done(&mut self, ctx: &DriverCtx, done: Done) -> anyhow::Result<()> {
        let now = Instant::now();
        match done {
            Done::Read(kind, Ok(snapshot)) => {
                self.reading = false;
                self.failures = 0;
                if kind.list {
                    self.next_list = now + RELOAD;
                }
                if kind.errors {
                    self.next_slow = now + SLOW;
                }
                self.publish(ctx, snapshot);
                self.confirm.observe(ctx, self.read_started);
                if !self.ready {
                    self.ready = true;
                    ctx.ready();
                }
                self.next_read = now
                    + if self.confirm.watching() {
                        self.pace.follow
                    } else {
                        self.pace.idle
                    };
            }
            Done::Read(_, Err(e)) => {
                self.reading = false;
                self.next_read = now + RETRY.min(self.pace.idle);
                self.refused(&e)?;
                if matches!(api::refused(&e), None | Some(Refusal::Other)) {
                    self.failures += 1;
                    tracing::warn!(instance = %ctx.instance(), error = %e, failures = self.failures, "ariston read failed");
                    if self.failures >= MAX_READ_FAILURES {
                        return Err(e);
                    }
                }
            }
            Done::Sent(order, result) => self.sent(ctx, &order, result)?,
        }
        Ok(())
    }

    fn sent(
        &mut self,
        ctx: &DriverCtx,
        order: &Order,
        result: anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        #[cfg(test)]
        if let Some(settled) = &self.settled {
            settled.notify_one();
        }
        let caller = self.line.sent();
        let listening = caller.as_ref().is_some_and(|c| !c.abandoned());
        match result {
            Ok(()) => {
                self.expect(order);
                if let Some(caller) = caller {
                    caller.reply(Ok(()));
                }
            }
            Err(e) => {
                let message = describe(&e);
                // Maybe taken: what reads back will tell.
                if e.is::<NoAnswer>()
                    || matches!(api::refused(&e), Some(Refusal::Busy | Refusal::Other))
                {
                    self.expect(order);
                }
                let unsent = e.is::<api::NotSent>();
                match caller {
                    Some(caller) => {
                        if !listening && !unsent && !e.is::<NoAnswer>() {
                            ctx.media_publisher().tell(
                                ctx.instance().as_str(),
                                moli_i18n::tr!(
                                    "pilotes.ariston.echec_tardif",
                                    device = order.device.native_name,
                                    cause = message
                                ),
                            );
                        }
                        caller.reply(Err(message));
                    }
                    _ if unsent || e.is::<NoAnswer>() => {}
                    _ => ctx.media_publisher().tell(
                        ctx.instance().as_str(),
                        moli_i18n::tr!(
                            "pilotes.ariston.echec_tardif",
                            device = order.device.native_name,
                            cause = message
                        ),
                    ),
                }
                self.refused(&e)?;
            }
        }

        Ok(())
    }

    fn expect(&mut self, order: &Order) {
        self.confirm.expect(
            &order.device,
            &order.key,
            order.value.clone(),
            self.pace.confirm,
        );
        self.next_read = self.next_read.min(Instant::now() + self.pace.follow);
    }

    /// A refusal the link survives (a pause, a new session, bounded), or
    /// the error that ends it.
    fn refused(&mut self, e: &anyhow::Error) -> anyhow::Result<()> {
        match api::refused(e) {
            Some(Refusal::Busy) => {
                tracing::warn!(error = %e, "ariston busy, pausing");
                self.busy_until = Some(Instant::now() + BUSY_PAUSE);
            }
            Some(Refusal::Session) if self.cloud.refused_sessions() >= MAX_SESSION_REFUSALS => {
                bail!(Refused {
                    kind: Refusal::Denied,
                    message: e.to_string(),
                });
            }
            Some(Refusal::Denied) => {
                bail!(Refused {
                    kind: Refusal::Denied,
                    message: e.to_string(),
                });
            }
            // A heater gone from the account: the list is read again.
            Some(Refusal::Gone) => self.next_list = Instant::now(),
            _ => {}
        }
        Ok(())
    }

    fn publish(&mut self, ctx: &DriverCtx, snapshot: Snapshot) {
        let instance = ctx.instance();
        let mut heaters = HashMap::new();
        for reading in snapshot.readings {
            let id = DeviceId::new(instance, &reading.heater.gateway);
            let settings = reading
                .settings
                .or_else(|| self.heaters.get(&id).map(|(_, s)| s.clone()))
                .unwrap_or(Json::Null);
            let (_, range) = model::ceiling(&settings);
            ctx.upsert_device(reading.heater.device(instance, range));
            for (key, value) in model::state(&reading.data) {
                ctx.set_state(&id, key, value);
            }
            if !settings.is_null() {
                for (key, value) in model::settings(&settings) {
                    ctx.set_state(&id, key, value);
                }
            }
            if let Some(errors) = &reading.errors {
                ctx.set_state(&id, "errors", model::errors(errors));
            }
            ctx.set_availability(&id, true);
            heaters.insert(id, (reading.heater, settings));
        }
        if snapshot.listed {
            for id in ctx.devices() {
                if !heaters.contains_key(&id) {
                    ctx.remove_device(&id);
                }
            }
        }
        self.heaters = heaters;
    }

    /// Takes an order from the hub: refused at once, or queued.
    fn order(&mut self, command: CommandRequest) {
        let now = Instant::now();
        if !self.ready {
            command.reply(Err(moli_i18n::tr!("pilotes.ariston.pas_pret")));
            return;
        }
        if let Some(left) = self.busy(now) {
            command.reply(Err(moli_i18n::tr!(
                "pilotes.ariston.en_pause",
                seconds = left.as_secs().max(1)
            )));
            return;
        }
        let Some((heater, settings)) = self.heaters.get(&command.device.id) else {
            command.reply(Err(moli_i18n::tr!("pilotes.ariston.point_non_ecrivable")));
            return;
        };
        let encoded = match model::order(&command.key, &command.value, settings) {
            Ok(encoded) => encoded,
            Err(why) => {
                command.reply(Err(why));
                return;
            }
        };
        let order = Order {
            path: format!("velis/medPlantData/{}/{}", heater.gateway, encoded.action),
            body: encoded.body,
            device: Arc::clone(&command.device),
            key: command.key.to_string(),
            value: command.value.clone(),
        };
        self.line.push(command, order);
    }

    /// Sends the next order if none is on its way and the cloud is not
    /// paused; the line has already checked it is awaited and in time.
    fn pump(&mut self, now: Instant) {
        if self.busy(now).is_some() {
            return;
        }
        let Some((order, permit)) = self.line.to_send() else {
            return;
        };
        let cloud = Arc::clone(&self.cloud);
        self.jobs.spawn(async move {
            let result = emit(&cloud, &order, &permit).await;
            Done::Sent(order, result)
        });
    }
}

/// Every read of one kind, in turn (one connection each).
async fn fetch(cloud: &Cloud, known: Vec<Heater>, kind: Read) -> anyhow::Result<Snapshot> {
    let heaters = if kind.list {
        let plants = cloud.get("velis/plants").await?;
        // An answer that is not a list must never empty the house.
        anyhow::ensure!(plants.is_array(), "ariston: the heaters are not a list");
        let heaters = model::heaters(&plants);
        if heaters.is_empty() {
            tracing::info!(
                plants = plants.as_array().map_or(0, Vec::len),
                "ariston: no Velis Evo 2 or Andris 2 on the account"
            );
        }
        heaters
    } else {
        known
    };
    let mut readings = Vec::with_capacity(heaters.len());
    for heater in heaters {
        let base = format!("velis/medPlantData/{}", heater.gateway);
        let data = cloud.get(&base).await?;
        anyhow::ensure!(data.is_object(), "ariston: no state for a heater");
        let settings = if kind.settings {
            Some(cloud.get(&format!("{base}/plantSettings")).await?).filter(Json::is_object)
        } else {
            None
        };
        let errors = if kind.errors {
            Some(
                cloud
                    .get(&format!(
                        "busErrors?gatewayId={}&blockingOnly=False&culture=en-US",
                        heater.gateway
                    ))
                    .await?,
            )
        } else {
            None
        };
        readings.push(Reading {
            heater,
            data,
            settings,
            errors,
        });
    }
    Ok(Snapshot {
        listed: kind.list,
        readings,
    })
}

/// One order to the cloud, never after `reply_by`: refused unsent if the
/// task starts late, its answer given up at that instant (maybe taken
/// then, never sent again).
async fn emit(cloud: &Cloud, order: &Order, permit: &SendPermit) -> anyhow::Result<()> {
    let result = cloud.post_if(&order.path, &order.body, permit).await;
    match result {
        Err(e) if permit.emitted() && api::refused(&e).is_none() => {
            Err(NoAnswer(format!("{e:#}")).into())
        }
        other => other,
    }
}

/// What an order's failure means to its caller.
fn describe(e: &anyhow::Error) -> String {
    if e.is::<NoAnswer>() {
        return format!("{}: {e}", moli_i18n::tr!("pilotes.ordres.sans_reponse"));
    }
    if e.is::<api::NotSent>() {
        return moli_i18n::tr!("pilotes.ordres.non_envoye");
    }
    match api::refused(e) {
        Some(Refusal::Busy) => moli_i18n::tr!("pilotes.ariston.occupe", cause = e.to_string()),
        Some(Refusal::Session) => moli_i18n::tr!("pilotes.ariston.session"),
        Some(_) => moli_i18n::tr!("pilotes.ariston.refus", cause = e.to_string()),
        None => format!("{e:#}"),
    }
}

async fn sleep(until: Option<Instant>) {
    match until {
        Some(until) => tokio::time::sleep_until(until.into()).await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests;
