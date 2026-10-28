use moli_core::{DeviceId, PointId};
use ring::digest::{SHA256, digest};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::MigrationError;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingKey {
    pub ha_installation: String,
    pub registry_entity_id: String,
    pub target: PointId,
    pub writer_scope: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    ToMoli,
    ToHa,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Owner {
    Ha,
    Moli,
    None,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Ha,
    PreparedToMoli,
    HaStopped,
    Moli,
    PreparedToHa,
    MoliStopped,
    ReconcileRequired,
}

impl Phase {
    pub(crate) fn active(self) -> bool {
        !matches!(self, Self::Ha | Self::Moli)
    }
}

/// Server-owned metadata. This crate stores drafts; it never activates an engine graph.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutomationProvenance {
    pub ha_installation: String,
    pub ha_id: String,
    pub source_sha256: String,
    pub target_sha256: String,
    pub mapping_revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationPlan {
    pub version: u8,
    pub binding: BindingKey,
    pub native_device: DeviceId,
    pub direction: Direction,
    pub automation_provenance: Option<AutomationProvenance>,
}

impl MigrationPlan {
    pub fn hash(&self) -> Result<String, MigrationError> {
        self.validate()?;
        let value = serde_json::to_value(self).map_err(|_| MigrationError::Invalid)?;
        let mut canonical = Vec::new();
        encode(&value, &mut canonical)?;
        Ok(sha256(&canonical))
    }

    pub(crate) fn validate(&self) -> Result<(), MigrationError> {
        let key = &self.binding;
        let Some((device, point)) = key.target.split() else {
            return Err(MigrationError::Unresolved);
        };
        if self.version != 1
            || device != self.native_device
            || key.target.as_str().len() > 512
            || !opaque(&key.ha_installation)
            || !opaque(&key.registry_entity_id)
            || !opaque(&key.writer_scope)
            || self.native_device.as_str().len() > 384
            || point.len() > 128
            || point.contains('/')
            || point.chars().any(char::is_control)
        {
            return Err(MigrationError::Invalid);
        }
        if let Some(provenance) = &self.automation_provenance
            && (provenance.ha_installation != key.ha_installation
                || !opaque(&provenance.ha_id)
                || !hash_value(&provenance.source_sha256)
                || !hash_value(&provenance.target_sha256)
                || provenance.mapping_revision == 0)
        {
            return Err(MigrationError::Invalid);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proof {
    pub operation: String,
    pub plan_hash: String,
    pub writer_scope: String,
    pub revision: u64,
    /// Logical exclusive owner observed by the authenticated companion.
    pub observed_owner: Owner,
    /// The previous writer has stopped and has no running or queued work.
    pub quiescent: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProofChallenge {
    pub operation: String,
    pub plan_hash: String,
    pub writer_scope: String,
    pub revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub key: BindingKey,
    pub native_device: DeviceId,
    pub phase: Phase,
    pub revision: u64,
    pub generation: u64,
    pub operation_id: String,
    pub plan_hash: String,
    pub plan: MigrationPlan,
    /// Runtime suspension; the serialized store always contains false here.
    pub reconcile_required: bool,
}

impl Binding {
    /// No engine activation is supplied by this first migration building block.
    #[allow(clippy::unused_self)]
    pub fn automation_draft_enabled(&self) -> bool {
        false
    }

    pub fn native_owner_moli(&self) -> bool {
        self.phase == Phase::Moli && !self.reconcile_required
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Snapshot {
    pub schema: u8,
    pub bindings: Vec<Binding>,
    pub active_operations: usize,
    pub suspended: bool,
}

pub(crate) fn opaque(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}
pub(crate) fn operation(value: &str) -> bool {
    opaque(value) && value.len() <= 64
}
pub(crate) fn hash_value(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
pub(crate) fn sha256(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    digest(&SHA256, bytes)
        .as_ref()
        .iter()
        .fold(String::with_capacity(64), |mut value, byte| {
            let _ = write!(value, "{byte:02x}");
            value
        })
}
fn encode(value: &Value, output: &mut Vec<u8>) -> Result<(), MigrationError> {
    match value {
        Value::Object(values) => {
            output.push(b'{');
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                serde_json::to_writer(&mut *output, key).map_err(|_| MigrationError::Invalid)?;
                output.push(b':');
                encode(&values[key], output)?;
            }
            output.push(b'}');
        }
        Value::Array(values) => {
            output.push(b'[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                encode(value, output)?;
            }
            output.push(b']');
        }
        _ => serde_json::to_writer(output, value).map_err(|_| MigrationError::Invalid)?,
    }
    Ok(())
}
