//! Generic Android TV driver. Configuration construction performs no network I/O.

mod commands;
mod config;
mod driver;
mod model;
pub mod pairing;
mod transport;
mod vault;

pub use config::Config;

pub struct AndroidTv {
    config: Config,
    gateway: pairing::PairingGateway,
    connector: std::sync::Arc<dyn transport::Connector>,
}

impl AndroidTv {
    /// # Errors
    /// Rejects invalid hosts before any task or connection is created.
    pub fn new(config: Config) -> anyhow::Result<Self> {
        config.check()?;
        Ok(Self {
            config,
            gateway: pairing::PairingGateway::new(),
            connector: std::sync::Arc::new(transport::Network),
        })
    }
}

#[cfg(test)]
mod tests;

impl std::fmt::Debug for AndroidTv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AndroidTv")
            .field("config", &self.config)
            .field("gateway", &self.gateway)
            .finish_non_exhaustive()
    }
}
impl AndroidTv {
    #[must_use]
    pub fn gateway(&self) -> pairing::PairingGateway {
        self.gateway.clone()
    }
}
impl moli_runtime::Driver for AndroidTv {
    fn kind(&self) -> &'static str {
        "androidtv"
    }
    fn run<'a>(
        &'a self,
        ctx: &'a mut moli_runtime::DriverCtx,
    ) -> moli_runtime::BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(driver::run(self, ctx))
    }
}
#[cfg(test)]
mod driver_tests;
