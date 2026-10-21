//! ASUSWRT JSON telemetry; no administration or presence.
mod config;
mod json;
mod model;
mod tracker;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Failure {
    Invalid,
    Identity,
    Expired,
    Refused,
    Unsupported,
    Transport,
    Certificate,
    Secrets,
    Vault,
    Budget,
    Capacity,
}
pub use config::{Config, Transport};
mod api;
#[cfg(test)]
mod model_tests;
mod protocol;

mod network;

#[derive(Debug)]
pub struct Asuswrt {
    config: Config,
}
impl Asuswrt {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        Ok(Self {
            config: config.checked()?,
        })
    }
}
impl moli_runtime::Driver for Asuswrt {
    fn kind(&self) -> &'static str {
        "asuswrt"
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

mod publication;
mod runtime;

#[cfg(test)]
mod tls_tests;
