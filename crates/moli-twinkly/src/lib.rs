//! Local Twinkly API v1; no device discovery or real-device qualification.
mod api;
mod model;
mod runtime;

use moli_runtime::{BoxFuture, Driver, DriverCtx};
use serde::Deserialize;
use std::net::Ipv4Addr;

/// A human-selected local endpoint and stable device identity.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub host: Ipv4Addr,
    #[serde(default = "default_port")]
    pub port: u16,
    pub mac: String,
    #[serde(default)]
    pub resume_mode: Option<ResumeMode>,
}

fn default_port() -> u16 {
    80
}

/// An explicitly selected compatible mode; never an arbitrary default.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResumeMode {
    Color,
    Movie,
    Playlist,
}

#[derive(Debug)]
pub struct Twinkly {
    config: Config,
    native: String,
}

impl Twinkly {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        let native = model::identity(&config.mac).map_err(|_| {
            anyhow::anyhow!(moli_i18n::tr!("pilotes.twinkly.configuration_invalide"))
        })?;
        if !(config.host.is_private() || config.host.is_loopback()) || config.port == 0 {
            anyhow::bail!(moli_i18n::tr!("pilotes.twinkly.configuration_invalide"));
        }
        Ok(Self { config, native })
    }
}

impl Driver for Twinkly {
    fn kind(&self) -> &'static str {
        "twinkly"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(runtime::run(&self.config, &self.native, ctx))
    }
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
