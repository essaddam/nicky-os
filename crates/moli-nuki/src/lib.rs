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

use std::time::Duration;

use anyhow::bail;
use http::{Method, Request, StatusCode};
use moli_core::{Device, DeviceId, PointSpec};
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

struct Bridge<'a> {
    config: &'a Config,
    token: String,
    rng: SystemRandom,
}

impl Bridge<'_> {
    /// One call; `query` without the authentication.
    async fn get(&self, path: &str, query: &str) -> Result<Vec<u8>, Failure> {
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
        let (status, body) = moli_net::plain(
            &self.config.host,
            self.config.port,
            moli_net::empty(request).map_err(Failure::Other)?,
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
        let body = self.get("/list", "").await?;
        model::locks(&body).map_err(|e| Failure::Other(e.context("Nuki bridge: not a list")))
    }

    async fn act(&self, lock: &Lock, action: u8) -> anyhow::Result<()> {
        let query = format!(
            "nukiId={}&deviceType={}&action={action}&nowait=1",
            lock.nuki_id, lock.device_type
        );
        let body = self
            .get("/lockAction", &query)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        if !model::succeeded(&body) {
            bail!(moli_i18n::tr!("pilotes.nuki.refuse"));
        }
        Ok(())
    }
}

/// A lock as published: its points, to republish only when they change.
struct Known {
    lock: Lock,
    id: DeviceId,
    points: Vec<PointSpec>,
    online: bool,
}

/// Publishes what `/list` said: new locks, their values, who went quiet.
fn publish(ctx: &DriverCtx, known: &mut Vec<Known>, listed: Vec<(Lock, Option<Last>)>) {
    let mut seen = Vec::with_capacity(listed.len());
    for (lock, last) in listed {
        let id = ctx.device_id(&lock.native);
        let points = model::points(last.as_ref());
        let index = known.iter().position(|k| k.id == id).unwrap_or_else(|| {
            known.push(Known {
                lock: lock.clone(),
                id: id.clone(),
                points: Vec::new(),
                online: false,
            });
            known.len() - 1
        });
        let entry = &mut known[index];
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
            for (key, value) in model::values(last) {
                ctx.set_state(&id, key, value);
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

async fn obey(bridge: &Bridge<'_>, known: &[Known], command: CommandRequest) -> bool {
    let Some(entry) = known.iter().find(|k| k.id == command.device.id) else {
        command.reply(Err(moli_i18n::tr!("pilotes.nuki.inconnue")));
        return false;
    };
    let result = match model::action(&command.key, &command.value) {
        Err(why) => Err(why),
        Ok(action) => bridge
            .act(&entry.lock, action)
            .await
            .map_err(|e| e.to_string()),
    };
    let done = result.is_ok();
    command.reply(result);
    done
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
    for id in ctx.known_devices() {
        ctx.set_availability(&id, false);
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
                        publish(ctx, &mut known, listed);
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

    #[test]
    fn options_are_bounded_to_what_is_known() {
        let config: Config = serde_json::from_str(r#"{"host":"192.168.1.x"}"#).unwrap();
        assert_eq!(config.port, 8080);
        assert!(serde_json::from_str::<Config>(r#"{"host":"h","token":"x"}"#).is_err());
        let refused = Failure::Refused(StatusCode::UNAUTHORIZED).to_string();
        assert_eq!(refused, "Nuki bridge 401");
    }
}
