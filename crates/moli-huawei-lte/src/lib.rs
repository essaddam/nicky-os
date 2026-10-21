//! Huawei HiLink read-only telemetry. No administration, SMS or presence.
mod config;
mod model;
mod tracker;
mod xml;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Failure {
    Invalid,
    Expired,
    Refused,
    Unsupported,
    Variant,
    Identity,
    Transport,
    Certificate,
    Secrets,
    Vault,
    Budget,
}
pub use config::{Auth, Config, Session, Transport};
#[cfg(test)]
mod model_tests;

mod api;
mod cookies;
mod protocol;
mod tokens;

mod network;

#[derive(Debug)]
pub struct HuaweiLte {
    config: Config,
}
impl HuaweiLte {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        Ok(Self {
            config: config.checked()?,
        })
    }
}
impl moli_runtime::Driver for HuaweiLte {
    fn kind(&self) -> &'static str {
        "huawei-lte"
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

#[cfg(test)]
mod publication_tests;
