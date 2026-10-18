//! Voltalis: heaters on the pilot wire and a water heater relay, through
//! the Voltalis cloud with the household's account.
//!
//! Reads the appliances and the programs every 2 minutes (every 10 s while
//! an order is being confirmed), the reachability every 10 minutes, and
//! everything again every 30 minutes. A heater takes a regime (`mode`:
//! back to its planning, by hand, off), a preset and a setpoint; a program
//! is switched on or off. The water heater is shown, never driven: no
//! client documents an order for it.
//!
//! An order is handed to the cloud **once** (never retried, never after
//! its time), one at a time in the order received; the cloud's « OK » is
//! not taken for the heater's: the value must read back within 2 minutes,
//! or the household is told. Every wait on the network runs beside the
//! loop, so an answer is always on time.

mod api;
mod model;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::bail;
use moli_core::{Device, DeviceId, Value};
use moli_runtime::orders::{Confirmations, OrderLine};
use moli_runtime::{BoxFuture, CommandRequest, Driver, DriverCtx};
use serde::Deserialize;
use serde_json::Value as Json;
use tokio::task::JoinSet;

pub use api::{Cloud, Refusal, Refused, Sessions};
use model::{Appliance, Program};

const USERNAME: &str = "username";
const PASSWORD: &str = "password";

/// Between two reads when nothing is going on.
const IDLE: Duration = Duration::from_secs(120);
/// While an order is being confirmed.
const FOLLOW: Duration = Duration::from_secs(10);
/// Reachability, and everything (new appliances, manual settings).
const DIAG: Duration = Duration::from_secs(600);
const RELOAD: Duration = Duration::from_secs(1800);
/// A value the cloud took must read back within this.
const CONFIRM: Duration = Duration::from_secs(120);
/// Too many requests, or the service down: nothing goes to it before.
const BUSY_PAUSE: Duration = Duration::from_secs(600);
/// After a failed read; a few in a row and the link restarts.
const RETRY: Duration = Duration::from_secs(30);
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
pub struct Voltalis {
    /// The session outlives a restart of the driver.
    sessions: Arc<Sessions>,
}

impl Voltalis {
    #[must_use]
    pub fn new(_config: Config) -> Self {
        Self::default()
    }
}

impl Driver for Voltalis {
    fn kind(&self) -> &'static str {
        "voltalis"
    }

    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let (Some(user), Some(password)) = (ctx.secret(USERNAME), ctx.secret(PASSWORD)) else {
                let instance = ctx.instance().to_string();
                let reason =
                    moli_i18n::tr!("pilotes.voltalis.identifiants_absents", instance = instance);
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
    let Err(e) = result else { return Ok(()) };
    if api::refused(&e) == Some(Refusal::Denied) {
        link.cloud.forget();
        let instance = ctx.instance().to_string();
        let reason = moli_i18n::tr!(
            "pilotes.voltalis.identifiants_refuses",
            cause = e.to_string(),
            instance = instance
        );
        return park(ctx, reason).await;
    }
    Err(e)
}

/// A read of the cloud, done beside the loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Read {
    /// The site, the appliances and their manual settings, the programs,
    /// the reachability.
    Load,
    /// The appliances and the programs; the reachability when due.
    Poll { diag: bool },
}

/// What a read brings back.
#[derive(Debug)]
struct Snapshot {
    site: u64,
    appliances: Json,
    manual: Option<Json>,
    programs: Json,
    quick: Json,
    diag: Option<Json>,
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
struct NoAnswer;

impl std::fmt::Display for NoAnswer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("no answer in time")
    }
}

impl std::error::Error for NoAnswer {}

#[derive(Debug)]
struct Link {
    cloud: Arc<Cloud>,
    pace: Pace,
    site: Option<u64>,
    appliances: HashMap<DeviceId, Appliance>,
    /// Appliance id → its manual setting, what an order writes to.
    manual: HashMap<u64, u64>,
    programs: HashMap<DeviceId, Program>,
    line: OrderLine<Order>,
    confirm: Confirmations,
    /// Network work in flight: one read and one order at most.
    jobs: JoinSet<Done>,
    reading: bool,
    next_read: Instant,
    next_diag: Instant,
    next_load: Instant,
    busy_until: Option<Instant>,
    failures: u32,
    ready: bool,
}

impl Link {
    fn new(cloud: Cloud, pace: Pace) -> Self {
        let now = Instant::now();
        Self {
            cloud: Arc::new(cloud),
            pace,
            site: None,
            appliances: HashMap::new(),
            manual: HashMap::new(),
            programs: HashMap::new(),
            line: OrderLine::new(LINE),
            confirm: Confirmations::default(),
            jobs: JoinSet::new(),
            reading: false,
            next_read: now,
            next_diag: now + DIAG,
            next_load: now,
            busy_until: None,
            failures: 0,
            ready: false,
        }
    }

    fn busy(&self, now: Instant) -> Option<Duration> {
        self.busy_until
            .filter(|until| now < *until)
            .map(|until| until - now)
    }

    async fn serve(&mut self, ctx: &mut DriverCtx) -> anyhow::Result<()> {
        loop {
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
            let wake = answer_at.into_iter().chain(read_at).min();
            tokio::select! {
                () = sleep(wake) => {}
                Some(done) = self.jobs.join_next() => {
                    let done = done.map_err(|e| anyhow::anyhow!("voltalis task: {e}"))?;
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
        let kind = if self.site.is_none() || now >= self.next_load {
            Read::Load
        } else {
            Read::Poll {
                diag: now >= self.next_diag,
            }
        };
        let cloud = Arc::clone(&self.cloud);
        let site = self.site;
        self.reading = true;
        self.jobs
            .spawn(async move { Done::Read(kind, fetch(&cloud, site, kind).await) });
    }

    fn done(&mut self, ctx: &DriverCtx, done: Done) -> anyhow::Result<()> {
        let now = Instant::now();
        match done {
            Done::Read(kind, Ok(snapshot)) => {
                self.reading = false;
                self.failures = 0;
                if kind == Read::Load {
                    self.next_load = now + RELOAD;
                }
                if snapshot.diag.is_some() {
                    self.next_diag = now + DIAG;
                }
                let unknown = self.publish(ctx, &snapshot);
                self.confirm.check(ctx);
                if !self.ready {
                    self.ready = true;
                    ctx.ready();
                }
                // A new appliance: its manual setting comes with a load.
                if unknown {
                    self.next_load = now;
                }
                self.next_read = now
                    + if self.confirm.watching() || unknown {
                        self.pace.follow
                    } else {
                        self.pace.idle
                    };
            }
            Done::Read(_, Err(e)) => {
                self.reading = false;
                self.next_read = now + RETRY;
                self.refused(&e)?;
                if api::refused(&e).is_none() {
                    self.failures += 1;
                    tracing::warn!(instance = %ctx.instance(), error = %e, failures = self.failures, "voltalis read failed");
                    if self.failures >= MAX_READ_FAILURES {
                        return Err(e);
                    }
                }
            }
            Done::Sent(order, result) => {
                let caller = self.line.sent();
                let listening = caller.as_ref().is_some_and(|c| !c.abandoned());
                match result {
                    Ok(()) => {
                        self.expect(&order);
                        if let Some(caller) = caller {
                            caller.reply(Ok(()));
                        }
                    }
                    Err(e) => {
                        let message = describe(&e);
                        // Maybe taken: what reads back will tell.
                        if e.is::<NoAnswer>() {
                            self.expect(&order);
                        }
                        let unsent = e.is::<api::NotSent>();
                        match caller {
                            Some(caller) if listening => caller.reply(Err(message)),
                            _ if unsent || e.is::<NoAnswer>() => {}
                            _ => ctx.media_publisher().tell(
                                ctx.instance().as_str(),
                                moli_i18n::tr!(
                                    "pilotes.voltalis.echec_tardif",
                                    device = order.device.native_name,
                                    cause = message
                                ),
                            ),
                        }
                        self.refused(&e)?;
                    }
                }
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
                tracing::warn!(error = %e, "voltalis busy, pausing");
                self.busy_until = Some(Instant::now() + BUSY_PAUSE);
            }
            // The session ended (forgotten by the transport): the next call
            // logs in again, a few times at most.
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
            // Something vanished: everything is read again.
            Some(Refusal::Gone) => self.next_load = Instant::now(),
            _ => {}
        }
        Ok(())
    }

    /// Publishes a read. `true` when an appliance has no known manual
    /// setting yet (a load brings it).
    fn publish(&mut self, ctx: &DriverCtx, snapshot: &Snapshot) -> bool {
        let instance = ctx.instance();
        self.site = Some(snapshot.site);
        if let Some(manual) = &snapshot.manual {
            self.manual = model::manual_settings(manual);
        }
        let diag = snapshot.diag.as_ref().map(model::diagnostics);
        let mut appliances = HashMap::new();
        for appliance in model::appliances(&snapshot.appliances) {
            let device = appliance.device(instance);
            let id = device.id.clone();
            ctx.upsert_device(device);
            for (key, value) in appliance.values() {
                ctx.set_state(&id, key, value);
            }
            if let Some(reachable) = diag.as_ref().map(|d| d.get(&appliance.id) != Some(&false)) {
                ctx.set_availability(&id, reachable);
            } else if !self.appliances.contains_key(&id) {
                ctx.set_availability(&id, true);
            }
            appliances.insert(id, appliance);
        }
        let mut programs = HashMap::new();
        for program in model::programs(&snapshot.programs, true)
            .into_iter()
            .chain(model::programs(&snapshot.quick, false))
        {
            let device = model::program_device(instance, &program);
            let id = device.id.clone();
            ctx.upsert_device(device);
            ctx.set_state(&id, "enabled", Value::Bool(program.enabled));
            ctx.set_availability(&id, true);
            programs.insert(id, program);
        }
        for id in ctx.devices() {
            if !appliances.contains_key(&id) && !programs.contains_key(&id) {
                ctx.remove_device(&id);
            }
        }
        let unknown = appliances
            .values()
            .any(|a| a.role == model::Role::Heater && !self.manual.contains_key(&a.id));
        self.appliances = appliances;
        self.programs = programs;
        unknown
    }

    /// Takes an order from the hub: refused at once, or queued.
    fn order(&mut self, command: CommandRequest) {
        let now = Instant::now();
        let Some(site) = self.site.filter(|_| self.ready) else {
            command.reply(Err(moli_i18n::tr!("pilotes.voltalis.pas_pret")));
            return;
        };
        if let Some(left) = self.busy(now) {
            command.reply(Err(moli_i18n::tr!(
                "pilotes.voltalis.en_pause",
                seconds = left.as_secs().max(1)
            )));
            return;
        }
        let id = &command.device.id;
        let encoded = if let Some(appliance) = self.appliances.get(id) {
            let Some(manual) = self.manual.get(&appliance.id) else {
                command.reply(Err(moli_i18n::tr!("pilotes.voltalis.sans_reglage")));
                return;
            };
            appliance
                .order(*manual, &command.key, &command.value)
                .map(|body| (format!("/api/site/{site}/manualsetting/{manual}"), body))
        } else if let Some(program) = self.programs.get(id) {
            match (&*command.key, &command.value) {
                ("enabled", Value::Bool(on)) => Some((
                    if program.user {
                        format!("/api/site/{site}/programming/program/{}", program.id)
                    } else {
                        format!("/api/site/{site}/quicksettings/{}/enable", program.id)
                    },
                    model::program_order(program, *on),
                )),
                _ => None,
            }
        } else {
            None
        };
        let Some((path, body)) = encoded else {
            command.reply(Err(moli_i18n::tr!("pilotes.voltalis.point_non_ecrivable")));
            return;
        };
        let order = Order {
            path,
            body,
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
        let Some((order, reply_by)) = self.line.to_send() else {
            return;
        };
        let cloud = Arc::clone(&self.cloud);
        self.jobs.spawn(async move {
            let result = emit(&cloud, &order, reply_by).await;
            Done::Sent(order, result)
        });
    }
}

/// Every read of one kind, in turn (one connection each).
async fn fetch(cloud: &Cloud, site: Option<u64>, kind: Read) -> anyhow::Result<Snapshot> {
    let site = match (kind, site) {
        (Read::Poll { .. }, Some(site)) => site,
        _ => model::site(&cloud.get("/api/account/me").await?)?,
    };
    let base = format!("/api/site/{site}");
    let appliances = cloud.get(&format!("{base}/managed-appliance")).await?;
    // An answer that is not a list must never empty the house.
    anyhow::ensure!(appliances.is_array(), "voltalis: appliances are not a list");
    let manual = match kind {
        Read::Load => Some(cloud.get(&format!("{base}/manualsetting")).await?),
        Read::Poll { .. } => None,
    };
    let programs = cloud.get(&format!("{base}/programming/program")).await?;
    let quick = cloud.get(&format!("{base}/quicksettings")).await?;
    anyhow::ensure!(
        programs.is_array() && quick.is_array(),
        "voltalis: programs are not a list"
    );
    let diag = match kind {
        Read::Load | Read::Poll { diag: true } => {
            Some(cloud.get(&format!("{base}/autodiag")).await?)
        }
        Read::Poll { diag: false } => None,
    };
    Ok(Snapshot {
        site,
        appliances,
        manual,
        programs,
        quick,
        diag,
    })
}

/// One order to the cloud, never after `reply_by`: refused unsent if the
/// task starts late, its answer given up at that instant (maybe taken
/// then, never sent again).
async fn emit(cloud: &Cloud, order: &Order, reply_by: Instant) -> anyhow::Result<()> {
    if Instant::now() >= reply_by {
        return Err(api::NotSent.into());
    }
    tokio::time::timeout_at(
        reply_by.into(),
        cloud.put(&order.path, &order.body, reply_by),
    )
    .await
    .unwrap_or_else(|_| Err(NoAnswer.into()))
}

/// What an order's failure means to its caller.
fn describe(e: &anyhow::Error) -> String {
    if e.is::<NoAnswer>() {
        return moli_i18n::tr!("pilotes.ordres.sans_reponse");
    }
    if e.is::<api::NotSent>() {
        return moli_i18n::tr!("pilotes.ordres.non_envoye");
    }
    match api::refused(e) {
        Some(Refusal::Busy) => moli_i18n::tr!("pilotes.voltalis.occupe", cause = e.to_string()),
        Some(Refusal::Session) => moli_i18n::tr!("pilotes.voltalis.session"),
        Some(_) => moli_i18n::tr!("pilotes.voltalis.refus", cause = e.to_string()),
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
