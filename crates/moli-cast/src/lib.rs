//! Explicitly activated Cast video receivers.
mod config;
pub mod discovery;
mod dns;
pub use config::Config;

pub mod gateway;
pub mod ownership;

mod vault;

mod driver;
mod model;
mod transport;
use moli_runtime::{BoxFuture, Driver, DriverCtx};
use std::sync::Arc;

pub struct Cast {
    config: Config,
    gateway: gateway::Gateway,
    connector: Arc<dyn transport::Connector>,
    discover: Arc<dyn gateway::Discover>,
}
impl Cast {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        Self::with_ownership(config, ownership::Ownership::default())
    }
    pub fn with_ownership(config: Config, owners: ownership::Ownership) -> anyhow::Result<Self> {
        config.check()?;
        Ok(Self {
            gateway: gateway::Gateway::new(config.clone(), owners),
            config,
            connector: Arc::new(transport::Network::default()),
            discover: Arc::new(gateway::NetworkDiscovery),
        })
    }
    pub fn gateway(&self) -> gateway::Gateway {
        self.gateway.clone()
    }
}
impl std::fmt::Debug for Cast {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cast")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}
impl Driver for Cast {
    fn kind(&self) -> &'static str {
        "cast"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(driver::run(self, ctx))
    }
}
#[cfg(test)]
mod driver_tests;
