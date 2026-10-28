use moli_core::InstanceId;
use serde::Deserialize;

use crate::{BridgeError, protocol};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "host")]
    pub host: String,
    #[serde(default = "port")]
    pub port: u16,
    pub installation: String,
    #[serde(default = "base")]
    pub base_topic: String,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub tls: bool,
    #[serde(default)]
    pub native_instances: Vec<InstanceId>,
}
fn host() -> String {
    "127.0.0.1".into()
}
fn port() -> u16 {
    1883
}
fn base() -> String {
    "moli/ha-bridge".into()
}

impl Config {
    pub fn validate(mut self) -> Result<Self, BridgeError> {
        if self.host.is_empty()
            || self.host.len() > 253
            || self.port == 0
            || self
                .host
                .chars()
                .any(|c| c.is_control() || matches!(c, '/' | '@' | '#' | '?' | ' '))
            || !protocol::valid_id(&self.installation)
            || self.native_instances.len() > 32
            || self
                .native_instances
                .iter()
                .any(|id| !protocol::valid_id(id.as_str()))
            || self
                .client_id
                .as_ref()
                .is_some_and(|id| !protocol::valid_id(id))
        {
            return Err(BridgeError::Configuration);
        }
        let loopback = self.host == "localhost"
            || self
                .host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback());
        if !self.tls && !loopback {
            return Err(BridgeError::Configuration);
        }
        if self.base_topic == base() {
            self.base_topic = format!("{}/{}", self.base_topic, self.installation);
        }
        if !(self.base_topic.starts_with("moli/") || self.base_topic.starts_with("moli-probe/"))
            || self.base_topic.len() > 160
            || self
                .base_topic
                .split('/')
                .any(|part| !protocol::valid_id(part))
        {
            return Err(BridgeError::Configuration);
        }
        self.native_instances.sort();
        self.native_instances.dedup();
        Ok(self)
    }
}
