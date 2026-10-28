//! Versioned bounded messages; transport authentication is a separate boundary.
use std::collections::BTreeSet;

use moli_migration::{Binding, BindingKey, Proof};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

pub const VERSION: u8 = 1;
pub const MAX_CATALOGUE_BYTES: usize = 1024 * 1024;
pub const MAX_COMMAND_BYTES: usize = 1024;
pub const MAX_DEVICES: usize = 1024;
pub const MAX_CHANNELS: usize = 4096;
pub const MAX_TEXT_BYTES: usize = 4096;
pub const MAX_ID_BYTES: usize = 128;
pub const MAX_COMMAND_TTL_MS: u64 = 5000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProtocolError {
    #[error("message exceeds its byte bound")]
    Size,
    #[error("message is not valid UTF-8 JSON")]
    Decode,
    #[error("unsupported protocol version")]
    Version,
    #[error("invalid opaque identity")]
    Identity,
    #[error("message belongs to another installation")]
    Installation,
    #[error("message belongs to another consumer session")]
    Session,
    #[error("command expired or has an excessive lifetime")]
    Expired,
    #[error("invalid catalogue or binding revision")]
    Revision,
    #[error("duplicate or excessive catalogue entries")]
    Catalogue,
    #[error("value does not match the declared channel")]
    Value,
    #[error("unsupported or duplicated actions")]
    Actions,
    #[error("feedback cannot be inferred from a command acknowledgement")]
    Feedback,
    #[error("migration envelope does not match its binding")]
    Migration,
}

/// Check the wire byte bound and UTF-8 before asking serde to allocate a model.
pub fn decode<T: DeserializeOwned>(bytes: &[u8], max: usize) -> Result<T, ProtocolError> {
    if max == 0 || max > MAX_CATALOGUE_BYTES || bytes.len() > max {
        return Err(ProtocolError::Size);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| ProtocolError::Decode)?;
    serde_json::from_str(text).map_err(|_| ProtocolError::Decode)
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_BYTES
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'))
}

pub(crate) fn check_id(id: &str) -> Result<(), ProtocolError> {
    if valid_id(id) {
        Ok(())
    } else {
        Err(ProtocolError::Identity)
    }
}
pub(crate) fn text(value: &str) -> Result<(), ProtocolError> {
    if value.len() <= MAX_TEXT_BYTES {
        Ok(())
    } else {
        Err(ProtocolError::Size)
    }
}
pub(crate) fn version(version: u8) -> Result<(), ProtocolError> {
    if version == VERSION {
        Ok(())
    } else {
        Err(ProtocolError::Version)
    }
}
pub(crate) fn installation(actual: &str, expected: &str) -> Result<(), ProtocolError> {
    check_id(actual)?;
    check_id(expected)?;
    if actual == expected {
        Ok(())
    } else {
        Err(ProtocolError::Installation)
    }
}
fn session(actual: &str, expected: &str) -> Result<(), ProtocolError> {
    check_id(actual)?;
    check_id(expected)?;
    if actual == expected {
        Ok(())
    } else {
        Err(ProtocolError::Session)
    }
}
fn expires(expires_at_ms: u64, now_ms: u64) -> Result<(), ProtocolError> {
    match expires_at_ms.checked_sub(now_ms) {
        Some(1..=MAX_COMMAND_TTL_MS) => Ok(()),
        _ => Err(ProtocolError::Expired),
    }
}
fn revision(revision: u64, generation: u64) -> Result<(), ProtocolError> {
    if revision > 0 && generation > 0 {
        Ok(())
    } else {
        Err(ProtocolError::Revision)
    }
}
pub(crate) fn encoded_bound<T: Serialize>(value: &T, max: usize) -> Result<(), ProtocolError> {
    let bytes = serde_json::to_vec(value).map_err(|_| ProtocolError::Decode)?;
    if bytes.len() <= max {
        Ok(())
    } else {
        Err(ProtocolError::Size)
    }
}
pub fn validate_binding(binding: &BindingKey, expected: &str) -> Result<(), ProtocolError> {
    installation(&binding.ha_installation, expected)?;
    check_id(&binding.registry_entity_id)?;
    check_id(&binding.writer_scope)?;
    let Some((device, point)) = binding.target.split() else {
        return Err(ProtocolError::Identity);
    };
    check_id(device.as_str())?;
    check_id(point)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverAction {
    Open,
    Close,
    Stop,
    My,
}
impl CoverAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Close => "close",
            Self::Stop => "stop",
            Self::My => "my",
        }
    }
    pub fn from_core_str(value: &str) -> Option<Self> {
        match value {
            "open" => Some(Self::Open),
            "close" => Some(Self::Close),
            "stop" => Some(Self::Stop),
            "my" => Some(Self::My),
            _ => None,
        }
    }
}
pub(crate) fn actions(actions: &[CoverAction]) -> Result<(), ProtocolError> {
    let unique: BTreeSet<_> = actions.iter().collect();
    if actions.len() <= 4 && unique.len() == actions.len() {
        Ok(())
    } else {
        Err(ProtocolError::Actions)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Exports {
    pub v: u8,
    pub installation: String,
    pub session: String,
    pub items: Vec<ExportCover>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExportCover {
    pub binding: BindingKey,
    pub label: String,
    pub revision: u64,
    pub generation: u64,
    pub actions: Vec<CoverAction>,
    pub feedback: bool,
}
impl ExportCover {
    pub fn validate(&self, expected: &str) -> Result<(), ProtocolError> {
        validate_binding(&self.binding, expected)?;
        text(&self.label)?;
        revision(self.revision, self.generation)?;
        actions(&self.actions)
    }
}
impl Exports {
    pub fn validate(&self, expected: &str, current_session: &str) -> Result<(), ProtocolError> {
        version(self.v)?;
        installation(&self.installation, expected)?;
        session(&self.session, current_session)?;
        if self.items.len() > MAX_DEVICES {
            return Err(ProtocolError::Catalogue);
        }
        let mut identities = BTreeSet::new();
        for item in &self.items {
            item.validate(expected)?;
            if !identities.insert(&item.binding) {
                return Err(ProtocolError::Catalogue);
            }
        }
        encoded_bound(self, MAX_CATALOGUE_BYTES)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeCommand {
    pub v: u8,
    pub installation: String,
    pub session: String,
    pub request_id: String,
    pub binding: BindingKey,
    pub revision: u64,
    pub generation: u64,
    pub expires_at_ms: u64,
    pub action: CoverAction,
}
impl NativeCommand {
    pub fn validate(
        &self,
        expected: &str,
        current_session: &str,
        now_ms: u64,
    ) -> Result<(), ProtocolError> {
        version(self.v)?;
        installation(&self.installation, expected)?;
        session(&self.session, current_session)?;
        check_id(&self.request_id)?;
        validate_binding(&self.binding, expected)?;
        revision(self.revision, self.generation)?;
        expires(self.expires_at_ms, now_ms)?;
        encoded_bound(self, MAX_COMMAND_BYTES)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HaCommand {
    pub v: u8,
    pub installation: String,
    pub session: String,
    pub request_id: String,
    pub entity_registry_id: String,
    #[serde(default)]
    pub generation: u64,
    pub expires_at_ms: u64,
    pub action: CoverAction,
}
impl HaCommand {
    pub fn validate(
        &self,
        expected: &str,
        current_session: &str,
        now_ms: u64,
    ) -> Result<(), ProtocolError> {
        version(self.v)?;
        installation(&self.installation, expected)?;
        session(&self.session, current_session)?;
        check_id(&self.request_id)?;
        check_id(&self.entity_registry_id)?;
        expires(self.expires_at_ms, now_ms)?;
        encoded_bound(self, MAX_COMMAND_BYTES)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CoverState {
    pub v: u8,
    pub installation: String,
    pub binding: BindingKey,
    pub revision: u64,
    pub generation: u64,
    pub available: bool,
    pub feedback: bool,
    pub position: Option<u8>,
    pub closed: Option<bool>,
}
impl CoverState {
    pub fn validate(&self, expected: &str) -> Result<(), ProtocolError> {
        version(self.v)?;
        installation(&self.installation, expected)?;
        validate_binding(&self.binding, expected)?;
        revision(self.revision, self.generation)?;
        if !self.feedback && (self.position.is_some() || self.closed.is_some()) {
            return Err(ProtocolError::Feedback);
        }
        if self.position.is_some_and(|position| position > 100) {
            return Err(ProtocolError::Value);
        }
        encoded_bound(self, MAX_COMMAND_BYTES)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultCode {
    Accepted,
    ApprovalRequired,
    Rejected,
    Expired,
    Uncertain,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResultMessage {
    pub v: u8,
    pub installation: String,
    pub request_id: String,
    pub code: ResultCode,
}
impl ResultMessage {
    pub fn validate(&self, expected: &str) -> Result<(), ProtocolError> {
        version(self.v)?;
        installation(&self.installation, expected)?;
        check_id(&self.request_id)?;
        encoded_bound(self, MAX_COMMAND_BYTES)
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Availability {
    pub v: u8,
    pub installation: String,
    pub session: String,
    pub online: bool,
}
impl Availability {
    pub fn validate(&self, expected: &str, current_session: &str) -> Result<(), ProtocolError> {
        version(self.v)?;
        installation(&self.installation, expected)?;
        session(&self.session, current_session)?;
        encoded_bound(self, MAX_COMMAND_BYTES)
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProofMessage {
    pub v: u8,
    pub installation: String,
    pub session: String,
    pub binding: BindingKey,
    pub proof: Proof,
}
impl ProofMessage {
    pub fn validate(&self, expected: &str, current_session: &str) -> Result<(), ProtocolError> {
        version(self.v)?;
        installation(&self.installation, expected)?;
        session(&self.session, current_session)?;
        validate_binding(&self.binding, expected)?;
        if self.proof.revision == 0
            || self.proof.writer_scope != self.binding.writer_scope
            || self.proof.operation.len() > 64
            || !valid_id(&self.proof.operation)
            || !valid_hash(&self.proof.plan_hash)
        {
            return Err(ProtocolError::Migration);
        }
        encoded_bound(self, MAX_CATALOGUE_BYTES)
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationRequest {
    pub v: u8,
    pub installation: String,
    pub session: String,
    pub expires_at_ms: u64,
    pub binding: Binding,
}
impl MigrationRequest {
    pub fn validate(
        &self,
        expected: &str,
        current_session: &str,
        now_ms: u64,
    ) -> Result<(), ProtocolError> {
        version(self.v)?;
        installation(&self.installation, expected)?;
        session(&self.session, current_session)?;
        expires(self.expires_at_ms, now_ms)?;
        validate_binding(&self.binding.key, expected)?;
        revision(self.binding.revision, self.binding.generation)?;
        if self.binding.key != self.binding.plan.binding
            || self.binding.native_device != self.binding.plan.native_device
            || !valid_id(&self.binding.operation_id)
            || self.binding.operation_id.len() > 64
            || self
                .binding
                .plan
                .hash()
                .map_err(|_| ProtocolError::Migration)?
                != self.binding.plan_hash
        {
            return Err(ProtocolError::Migration);
        }
        encoded_bound(self, MAX_CATALOGUE_BYTES)
    }
}
fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;
    use moli_core::PointId;
    use serde_json::json;
    fn binding(installation: &str) -> BindingKey {
        BindingKey {
            ha_installation: installation.into(),
            registry_entity_id: "entity-fixture".into(),
            target: PointId::from("fixture:cover/command"),
            writer_scope: "cover-fixture".into(),
        }
    }
    fn command(installation: &str) -> NativeCommand {
        NativeCommand {
            v: VERSION,
            installation: installation.into(),
            session: "consumer-nonce".into(),
            request_id: "request-fixture".into(),
            binding: binding(installation),
            revision: 1,
            generation: 2,
            expires_at_ms: 1100,
            action: CoverAction::Open,
        }
    }
    #[test]
    fn bounded_decode_checks_bytes_utf8_unknown_fields_and_origin_spoofing() {
        let message = command("fixture-a");
        let mut value = serde_json::to_value(&message).unwrap();
        value["origin"] = json!("automation");
        assert_eq!(
            decode::<NativeCommand>(&serde_json::to_vec(&value).unwrap(), MAX_COMMAND_BYTES)
                .unwrap_err(),
            ProtocolError::Decode
        );
        assert_eq!(
            decode::<NativeCommand>(&vec![b' '; MAX_COMMAND_BYTES + 1], MAX_COMMAND_BYTES)
                .unwrap_err(),
            ProtocolError::Size
        );
        assert_eq!(
            decode::<NativeCommand>(&[0xff], MAX_COMMAND_BYTES).unwrap_err(),
            ProtocolError::Decode
        );
        assert_eq!(
            decode::<NativeCommand>(b"{}", MAX_CATALOGUE_BYTES + 1).unwrap_err(),
            ProtocolError::Size
        );
    }
    #[test]
    fn identical_registry_ids_remain_scoped_to_installation_and_current_consumer() {
        let a = command("fixture-a");
        let b = command("fixture-b");
        assert_ne!(a.binding, b.binding);
        assert!(a.validate("fixture-a", "consumer-nonce", 1000).is_ok());
        assert_eq!(
            b.validate("fixture-a", "consumer-nonce", 1000),
            Err(ProtocolError::Installation)
        );
        assert_eq!(
            a.validate("fixture-a", "old-consumer", 1000),
            Err(ProtocolError::Session)
        );
    }
    #[test]
    fn expiration_and_identity_bounds_are_strict() {
        let mut message = command("fixture-a");
        for expires in [0, 1000, 6001, u64::MAX] {
            message.expires_at_ms = expires;
            assert_eq!(
                message.validate("fixture-a", "consumer-nonce", 1000),
                Err(ProtocolError::Expired)
            );
        }
        message.expires_at_ms = 6000;
        assert!(
            message
                .validate("fixture-a", "consumer-nonce", 1000)
                .is_ok()
        );
        for id in [
            "",
            "entity/alias",
            "bad+filter",
            "bad#filter",
            "é",
            &"x".repeat(MAX_ID_BYTES + 1),
        ] {
            assert!(!valid_id(id));
        }
        assert!(valid_id("fixture.namespace:registry_entity-1"));
    }
    #[test]
    fn acknowledgements_do_not_invent_feedback() {
        let mut state = CoverState {
            v: VERSION,
            installation: "fixture-a".into(),
            binding: binding("fixture-a"),
            revision: 1,
            generation: 1,
            available: true,
            feedback: false,
            position: None,
            closed: None,
        };
        assert!(state.validate("fixture-a").is_ok());
        state.position = Some(100);
        assert_eq!(state.validate("fixture-a"), Err(ProtocolError::Feedback));
        state.feedback = true;
        assert!(state.validate("fixture-a").is_ok());
        state.position = Some(101);
        assert_eq!(state.validate("fixture-a"), Err(ProtocolError::Value));
        state.position = None;
        state.feedback = false;
        state.closed = Some(true);
        assert_eq!(state.validate("fixture-a"), Err(ProtocolError::Feedback));
    }
    #[test]
    fn ha_command_generation_defaults_to_unmanaged_and_roundtrips_explicit_epochs() {
        let mut body = json!({"v":VERSION,"installation":"fixture-a","session":"consumer-nonce","request_id":"request-fixture","entity_registry_id":"registry-fixture","expires_at_ms":1100,"action":"open"});
        let legacy: HaCommand =
            decode(&serde_json::to_vec(&body).unwrap(), MAX_COMMAND_BYTES).unwrap();
        assert_eq!(legacy.generation, 0);
        assert!(legacy.validate("fixture-a", "consumer-nonce", 1000).is_ok());
        body["generation"] = json!(7);
        let managed: HaCommand =
            decode(&serde_json::to_vec(&body).unwrap(), MAX_COMMAND_BYTES).unwrap();
        assert_eq!(managed.generation, 7);
        assert_eq!(serde_json::to_value(managed).unwrap()["generation"], 7);
    }
}
