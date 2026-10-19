//! Read-only Netatmo weather stations, with human OAuth and durable rotation.

mod actor;
mod api;
mod config;
mod gateway;
mod model;
mod oauth;
mod observed;

pub use config::Config;
pub use gateway::{Authorization, Gateway, Status};

use moli_core::InstanceId;
use moli_runtime::{BoxFuture, Driver, DriverCtx};
use tokio::sync::{Mutex, mpsc, watch};

#[cfg(test)]
mod actor_tests;

#[cfg(test)]
mod contract_tests;

#[cfg(test)]
mod gateway_tests;

/// Safe error categories: never carry provider text, credentials or codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Unauthorized,
    Expired,
    Busy,
    Unavailable,
    Storage,
    BeforeSend,
    Ambiguous,
    RateLimited(u64),
}

/// One weather instance. The supervised actor alone owns its credentials.
pub struct Netatmo {
    config: Config,
    gateway: Gateway,
    controls: Mutex<mpsc::Receiver<gateway::Control>>,
    status: watch::Sender<Status>,
    clock: actor::Clock,
    #[cfg(test)]
    exchange: Option<api::Exchange>,
    #[cfg(test)]
    persist_gate: Option<std::sync::Arc<actor::SaveGate>>,
}

impl std::fmt::Debug for Netatmo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Netatmo").finish_non_exhaustive()
    }
}

impl Netatmo {
    /// Validates installation options without connecting to any service.
    ///
    /// # Errors
    /// Returns `Invalid` for an invalid origin or polling/freshness bounds.
    pub fn new(config: Config) -> Result<Self, Error> {
        config.callback(&InstanceId::from("validation"))?;
        let (gateway, controls, status) = gateway::channel();
        Ok(Self {
            config,
            gateway,
            controls: Mutex::new(controls),
            status,
            clock: actor::Clock::at(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            ),
            #[cfg(test)]
            exchange: None,
            #[cfg(test)]
            persist_gate: None,
        })
    }

    #[must_use]
    pub fn gateway(&self) -> Gateway {
        self.gateway.clone()
    }
}

impl Driver for Netatmo {
    fn kind(&self) -> &'static str {
        "netatmo"
    }

    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(actor::run(self, ctx))
    }
}
