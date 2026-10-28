//! Versioned MQTT contracts and guarded Home Assistant bridge.
mod config;
mod driver;
mod gateway;
pub mod inventory;
pub mod protocol;
mod transport;

pub use config::Config;
pub use gateway::Gateway;
use moli_runtime::{BoxFuture, Driver, DriverCtx};
use ring::{
    digest::{SHA256, digest},
    rand::{SecureRandom, SystemRandom},
};
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    #[error("invalid bridge configuration")]
    Configuration,
    #[error("bridge unavailable")]
    Unavailable,
    #[error("bridge busy")]
    Busy,
    #[error("bridge revision conflict")]
    Conflict,
    #[error("bridge operation unsupported")]
    Unsupported,
    #[error("bridge protocol refused")]
    Protocol(#[from] protocol::ProtocolError),
    #[error("bridge message encoding failed")]
    Encoding(#[from] serde_json::Error),
    #[error("migration refused: {0}")]
    Migration(#[from] moli_migration::MigrationError),
    #[error("bridge session unavailable")]
    Random,
}
impl BridgeError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Configuration | Self::Protocol(_) | Self::Encoding(_) => "invalid",
            Self::Unavailable | Self::Random => "unavailable",
            Self::Busy => "busy",
            Self::Unsupported => "unsupported",
            Self::Conflict
            | Self::Migration(
                moli_migration::MigrationError::Revision | moli_migration::MigrationError::PlanHash,
            ) => "conflict",
            Self::Migration(_) => "reconcile_required",
        }
    }
}
#[derive(Debug)]
pub(crate) struct Frame {
    pub suffix: String,
    pub payload: Vec<u8>,
    pub retained: bool,
}

#[derive(Debug)]
pub struct HomeAssistant {
    config: Config,
    gateway: Option<Gateway>,
}
impl HomeAssistant {
    pub fn new(config: Config) -> Result<Self, BridgeError> {
        Ok(Self {
            config: config.validate()?,
            gateway: None,
        })
    }
    pub fn with_gateway(config: Config, gateway: Gateway) -> Result<Self, BridgeError> {
        Ok(Self {
            config: config.validate()?,
            gateway: Some(gateway),
        })
    }
    pub fn gateway(&self) -> Option<Gateway> {
        self.gateway.clone()
    }
}
impl Driver for HomeAssistant {
    fn kind(&self) -> &'static str {
        "home-assistant"
    }
    fn integration(&self) -> Arc<str> {
        Arc::from("home-assistant")
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let gateway = self.gateway.as_ref().ok_or_else(|| {
                anyhow::anyhow!(moli_i18n::tr!("home_assistant.status.unattached"))
            })?;
            driver::run(&self.config, gateway, ctx).await
        })
    }
}

pub(crate) fn now_ms() -> u64 {
    u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}
pub(crate) fn request_id() -> Result<String, BridgeError> {
    let mut bytes = [0u8; 16];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| BridgeError::Random)?;
    Ok(hex(&bytes))
}
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut text, byte| {
            let _ = write!(text, "{byte:02x}");
            text
        })
}
pub(crate) fn binding_id(binding: &moli_migration::BindingKey) -> String {
    let values = std::collections::BTreeMap::from([
        ("ha_installation", binding.ha_installation.as_str()),
        ("registry_entity_id", binding.registry_entity_id.as_str()),
        ("target", binding.target.as_str()),
        ("writer_scope", binding.writer_scope.as_str()),
    ]);
    let bytes = serde_json::to_vec(&values).expect("binding contains only strings");
    hex(digest(&SHA256, &bytes).as_ref())
}

#[cfg(test)]
mod interoperability_tests {
    use super::*;

    #[test]
    fn the_companion_fixture_has_the_same_id_and_wire_contract_in_rust() {
        let fixture: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../tools/home-assistant/tests_moli_companion/fixtures/v1.json"
        ))
        .unwrap();
        let binding: moli_migration::BindingKey =
            serde_json::from_value(fixture["binding"].clone()).unwrap();
        assert_eq!(
            binding_id(&binding),
            "e8d28c0eed2f6e987ff9875fd5504ad174b200ab5db276b38add81eafb742154"
        );
        let catalog: inventory::Catalog =
            serde_json::from_value(fixture["ha_catalog"].clone()).unwrap();
        catalog.validate().unwrap();
        let native: protocol::NativeCommand =
            serde_json::from_value(fixture["native_command"].clone()).unwrap();
        native
            .validate(
                &native.installation,
                &native.session,
                native.expires_at_ms - 1,
            )
            .unwrap();
        let command: protocol::HaCommand =
            serde_json::from_value(fixture["ha_command"].clone()).unwrap();
        command
            .validate(
                &command.installation,
                &command.session,
                command.expires_at_ms - 1,
            )
            .unwrap();
        let state: protocol::CoverState =
            serde_json::from_value(fixture["cover_state"].clone()).unwrap();
        state.validate(&state.installation).unwrap();
    }
}
