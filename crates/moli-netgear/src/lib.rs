//! Bounded read-only Netgear SOAP; no discovery, administration or presence.
mod api;
mod cookies;
mod model;
mod network;
mod protocol;
mod publication;
mod runtime;
mod xml;

use serde::Deserialize;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    Http,
    Https,
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Version {
    V1,
    V2,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub transport: Transport,
    pub login: Version,
    pub inventory: Version,
    #[serde(default)]
    pub allow_plain_auth: bool,
    #[serde(default = "every")]
    pub every: u64,
    #[serde(default = "inventory_every")]
    pub inventory_every: u64,
    #[serde(default = "info_every")]
    pub info_every: u64,
    #[serde(default)]
    pub selected: Vec<String>,
}
fn every() -> u64 {
    60
}
fn inventory_every() -> u64 {
    300
}
fn info_every() -> u64 {
    3600
}
#[derive(Debug)]
pub struct Netgear {
    config: Config,
}
impl Netgear {
    pub fn new(mut config: Config) -> anyhow::Result<Self> {
        let host = &config.host;
        let valid_host = if let Ok(ip) = host.parse::<std::net::Ipv4Addr>() {
            ip.is_private() || ip.is_loopback()
        } else {
            !host.is_empty()
                && host.len() <= 253
                && host.split('.').all(|label| {
                    !label.is_empty()
                        && label.len() <= 63
                        && !label.starts_with('-')
                        && !label.ends_with('-')
                        && label
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                })
        };
        let periods = [config.every, config.inventory_every, config.info_every];
        if !valid_host
            || config.port == 0
            || config.selected.len() > 32
            || periods
                .iter()
                .any(|seconds| !(30..=86400).contains(seconds))
            || config.inventory_every < config.every
            || config.info_every < config.every
            || config.transport == Transport::Http && !config.allow_plain_auth
        {
            anyhow::bail!(moli_i18n::tr!("pilotes.netgear.configuration_invalide"));
        }
        let mut selected = std::collections::BTreeSet::new();
        for identity in &config.selected {
            let native = model::mac(identity).ok_or_else(|| {
                anyhow::anyhow!(moli_i18n::tr!("pilotes.netgear.configuration_invalide"))
            })?;
            if !selected.insert(native) {
                anyhow::bail!(moli_i18n::tr!("pilotes.netgear.configuration_invalide"));
            }
        }
        config.selected = selected.into_iter().collect();
        Ok(Self { config })
    }
}
#[cfg(test)]
mod config_tests;

impl moli_runtime::Driver for Netgear {
    fn kind(&self) -> &'static str {
        "netgear"
    }
    fn run<'a>(
        &'a self,
        ctx: &'a mut moli_runtime::DriverCtx,
    ) -> moli_runtime::BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(runtime::run(&self.config, ctx))
    }
}
#[cfg(test)]
mod runtime_tests;

#[cfg(test)]
mod tls_tests;

#[cfg(test)]
mod publication_tests;
