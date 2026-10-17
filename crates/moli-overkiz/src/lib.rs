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
//! failure after that is told to the household. An RTS motor never says
//! where it is: Moli never shows a position it does not know.

mod api;
mod follow;
mod model;

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::bail;
use moli_core::DeviceId;
use moli_runtime::{BoxFuture, CommandRequest, Driver, DriverCtx};
use serde::Deserialize;
use tokio::time::sleep_until;

use crate::follow::{About, Outcome, Tracker};

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
/// Quota reached, queue full, maintenance: leave the box alone this long.
const BUSY_PAUSE: Duration = Duration::from_secs(60);
/// The caller is answered this long before the hub stops waiting.
const REPLY_MARGIN: Duration = Duration::from_millis(400);

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

/// Shows `reason` on every surface and idles until shutdown.
async fn park(ctx: &DriverCtx, reason: String) -> anyhow::Result<()> {
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
    Ok(Some(Api::Local(Local::new(host, port, pin, token)?)))
}

async fn run(driver: &Overkiz, ctx: &mut DriverCtx) -> anyhow::Result<()> {
    let Some(api) = connect(driver, ctx).await? else {
        return Ok(());
    };
    let cloud = matches!(api, Api::Cloud(_));
    let (idle, follow) = if cloud {
        (IDLE_CLOUD, FOLLOW_CLOUD)
    } else {
        (IDLE_LOCAL, FOLLOW_LOCAL)
    };
    let mut link = Link {
        api,
        index: Index::default(),
        listener: String::new(),
        tracker: Tracker::default(),
    };
    let result = link.serve(ctx, idle, follow).await;
    // Sent, never confirmed: say so rather than let the hub call it a crash.
    for command in link.tracker.abandon() {
        command.reply(Err(moli_i18n::tr!("pilotes.overkiz.confirmation_perdue")));
    }
    let Err(e) = result else { return Ok(()) };
    let instance = ctx.instance().to_string();
    // A local box has no session to renew: « not authenticated » means its
    // token was revoked, a human's call like a refused one.
    let denied = e
        .downcast_ref::<Refused>()
        .filter(|r| r.kind == Refusal::Denied || (r.kind == Refusal::Session && !cloud));
    if let Some(Refused { message, .. }) = denied {
        let reason = if let Api::Cloud(cloud) = &link.api {
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
        let host = driver.config.host.clone().unwrap_or_default();
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

#[derive(Debug)]
struct Link {
    api: Api,
    index: Index,
    listener: String,
    tracker: Tracker<CommandRequest>,
}

impl Link {
    async fn serve(
        &mut self,
        ctx: &mut DriverCtx,
        idle: Duration,
        follow: Duration,
    ) -> anyhow::Result<()> {
        // Listen first, then load: nothing changes unseen in between.
        self.register().await?;
        self.reload(ctx).await?;
        ctx.ready();
        let mut next = Instant::now() + idle;
        let mut sessions_refused = 0;
        loop {
            let wake = self.tracker.next_reply().map_or(next, |r| r.min(next));
            tokio::select! {
                () = sleep_until(wake.into()) => {
                    if Instant::now() >= next {
                        next = Instant::now() + match self.poll(ctx).await {
                            Ok(()) => {
                                sessions_refused = 0;
                                if self.tracker.watching() { follow } else { idle }
                            }
                            Err(e) => match api::refused(&e) {
                                Some(Refusal::Listener) => {
                                    tracing::info!(instance = %ctx.instance(), "overkiz listener expired, registering again");
                                    self.register().await?;
                                    self.reload(ctx).await?;
                                    idle
                                }
                                Some(Refusal::Busy) => {
                                    tracing::warn!(instance = %ctx.instance(), error = %e, "overkiz busy, pausing");
                                    BUSY_PAUSE
                                }
                                // The cloud ended the session: the next call logs in again,
                                // once; a second refusal in a row is the supervisor's.
                                Some(Refusal::Session) if matches!(self.api, Api::Cloud(_)) && sessions_refused == 0 => {
                                    sessions_refused += 1;
                                    follow
                                }
                                Some(Refusal::Session) => {
                                    return Err(Refused { kind: Refusal::Denied, message: e.to_string() }.into());
                                }
                                _ => return Err(e),
                            },
                        };
                    }
                    // The box still at work when the hub stops waiting: « accepted ».
                    for command in self.tracker.due(Instant::now()) {
                        command.reply(Ok(()));
                    }
                }
                command = ctx.next_command() => match command {
                    Some(command) => {
                        if self.order(command).await {
                            next = next.min(Instant::now() + follow);
                        }
                    }
                    None => return Ok(()),
                },
            }
        }
    }

    async fn register(&mut self) -> anyhow::Result<()> {
        let answer = self.api.post("/events/register").await?;
        self.listener = api::identifier(&answer["id"])?;
        Ok(())
    }

    /// Loads every device; publishes them, their values and availability.
    async fn reload(&mut self, ctx: &DriverCtx) -> anyhow::Result<()> {
        let answer = self.api.get("/setup/devices").await?;
        let load = model::load(ctx.instance(), &answer);
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
        Ok(())
    }

    async fn poll(&mut self, ctx: &DriverCtx) -> anyhow::Result<()> {
        let answer = self
            .api
            .post(&format!("/events/{}/fetch", self.listener))
            .await?;
        let mut reload = false;
        for event in model::events(&answer) {
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
                Event::GatewayDown => {
                    for id in self.index.devices() {
                        ctx.set_availability(id, false);
                    }
                }
                Event::Execution { id, state, cause } => self.executed(ctx, &id, state, cause),
                Event::Reload => reload = true,
            }
        }
        if reload {
            self.reload(ctx).await?;
        }
        Ok(())
    }

    /// An execution moved: answer its caller, or tell the household.
    fn executed(&mut self, ctx: &DriverCtx, id: &str, state: Execution, cause: Option<String>) {
        match self.tracker.moved(id, state, cause) {
            Some(Outcome::Answer(command, result)) => command.reply(
                result.map_err(|cause| moli_i18n::tr!("pilotes.overkiz.echec", cause = cause)),
            ),
            Some(Outcome::Tell(about, cause)) => ctx.media_publisher().tell(
                ctx.instance().as_str(),
                moli_i18n::tr!(
                    "pilotes.overkiz.echec_tardif",
                    device = about.device,
                    order = about.order,
                    cause = cause
                ),
            ),
            None => {}
        }
    }

    /// Hands one order to the box. `true` when it is now followed.
    async fn order(&mut self, command: CommandRequest) -> bool {
        let Some(order) = self
            .index
            .encode(&command.device.id, &command.key, &command.value)
        else {
            command.reply(Err(moli_i18n::tr!("pilotes.overkiz.point_non_ecrivable")));
            return false;
        };
        let reply_by = command
            .deadline
            .checked_sub(REPLY_MARGIN)
            .unwrap_or(command.deadline);
        let budget = reply_by.saturating_duration_since(Instant::now());
        if budget.is_zero() {
            command.reply(Err("expired".into()));
            return false;
        }
        match tokio::time::timeout(budget, self.api.apply(&order)).await {
            Ok(Ok(execution)) => {
                let about = About {
                    device: Arc::clone(&command.device.native_name),
                    order: order.command,
                };
                self.tracker.track(execution, command, about, reply_by);
                true
            }
            Ok(Err(e)) => {
                let cause = e.to_string();
                command.reply(Err(match api::refused(&e) {
                    Some(Refusal::Busy) => moli_i18n::tr!("pilotes.overkiz.occupee", cause = cause),
                    Some(_) => moli_i18n::tr!("pilotes.overkiz.refus", cause = cause),
                    None => format!("{e:#}"),
                }));
                false
            }
            // Maybe sent: never sent again, the caller is told the truth.
            Err(_) => {
                command.reply(Err(moli_i18n::tr!("pilotes.overkiz.box_muette")));
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(text: &str) -> anyhow::Result<Overkiz> {
        Overkiz::new(toml::from_str(text)?)
    }

    #[test]
    fn configurations() {
        assert!(config("transport = \"local\"\nhost = \"192.168.1.x\"").is_ok());
        assert_eq!(
            config("transport = \"local\"\nhost = \"192.168.1.x\"")
                .unwrap()
                .config
                .port,
            8443
        );
        assert!(config("transport = \"cloud\"").is_ok());
        assert!(
            config("transport = \"local\"").is_err(),
            "a box needs its address"
        );
        assert!(
            config("transport = \"cloud\"\nhost = \"example.org\"").is_err(),
            "the cloud's address is never configurable"
        );
        assert!(config("transport = \"radio\"").is_err());
        assert!(config("transport = \"cloud\"\nurl = \"x\"").is_err());
    }
}
