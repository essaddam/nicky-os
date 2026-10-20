use crate::model;
use serde::Deserialize;
use std::collections::BTreeSet;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    Http,
    Https,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub transport: Transport,
    #[serde(default)]
    pub allow_plain_auth: bool,
    #[serde(default = "every")]
    pub every: u64,
    #[serde(default = "inventory")]
    pub inventory_every: u64,
    #[serde(default = "info")]
    pub info_every: u64,
    #[serde(default)]
    pub selected: Vec<String>,
}
fn every() -> u64 {
    60
}
fn inventory() -> u64 {
    300
}
fn info() -> u64 {
    3600
}
impl Config {
    pub fn checked(mut self) -> anyhow::Result<Self> {
        let host = &self.host;
        let valid = if let Ok(ip) = host.parse::<std::net::Ipv4Addr>() {
            ip.is_private() || ip.is_loopback()
        } else {
            !host.is_empty()
                && host.len() <= 253
                && host.split('.').all(|l| {
                    !l.is_empty()
                        && l.len() <= 63
                        && !l.starts_with('-')
                        && !l.ends_with('-')
                        && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                })
        };
        if !valid
            || self.port == 0
            || self.selected.len() > 32
            || [self.every, self.inventory_every, self.info_every]
                .iter()
                .any(|v| !(30..=86400).contains(v))
            || self.inventory_every < self.every
            || self.info_every < self.every
            || self.transport == Transport::Http && !self.allow_plain_auth
        {
            anyhow::bail!(moli_i18n::tr!("pilotes.asuswrt.configuration_invalide"));
        }
        let mut seen = BTreeSet::new();
        for raw in &self.selected {
            let native = model::mac(raw).ok_or_else(|| {
                anyhow::anyhow!(moli_i18n::tr!("pilotes.asuswrt.configuration_invalide"))
            })?;
            if !seen.insert(native) {
                anyhow::bail!(moli_i18n::tr!("pilotes.asuswrt.configuration_invalide"));
            }
        }
        self.selected = seen.into_iter().collect();
        Ok(self)
    }
}
