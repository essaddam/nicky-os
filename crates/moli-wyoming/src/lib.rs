//! Read-only supervision of explicitly configured local Wyoming services.

mod info;
mod runtime;

use moli_runtime::{BoxFuture, Driver, DriverCtx};
use serde::Deserialize;
use std::net::{IpAddr, Ipv4Addr};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub host: String,
    pub port: u16,
    #[serde(default = "default_every")]
    pub every: u64,
    #[serde(default = "default_timeout")]
    pub timeout: u64,
}

fn default_every() -> u64 {
    60
}

fn default_timeout() -> u64 {
    4
}

impl Config {
    fn validate(&self) -> anyhow::Result<()> {
        if !local_endpoint(&self.host)
            || self.port == 0
            || !(10..=3600).contains(&self.every)
            || !(1..=10).contains(&self.timeout)
        {
            anyhow::bail!(moli_i18n::tr!("pilotes.wyoming.configuration_invalide"));
        }
        Ok(())
    }
}

fn local_ipv4(ip: Ipv4Addr) -> bool {
    ip.is_private() || ip.is_loopback() || ip.is_link_local()
}

fn local_endpoint(host: &str) -> bool {
    if host.is_empty() || host.len() > 253 || !host.is_ascii() {
        return false;
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(ip) => local_ipv4(ip),
            IpAddr::V6(ip) => {
                ip.is_loopback()
                    || ip.is_unique_local()
                    || ip.is_unicast_link_local()
                    || ip.to_ipv4_mapped().is_some_and(local_ipv4)
            }
        };
    }
    host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
    })
}

/// One explicitly configured Wyoming service, supervised without audio.
#[derive(Debug)]
pub struct Wyoming {
    config: Config,
}

impl Wyoming {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        config.validate()?;
        Ok(Self { config })
    }
}

impl Driver for Wyoming {
    fn kind(&self) -> &'static str {
        "wyoming"
    }

    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(runtime::run(&self.config, ctx))
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod runtime_tests;
