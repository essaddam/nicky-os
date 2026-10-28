//! Cold HA catalogue metadata and schema-bound values; labels never become keys.
use std::collections::BTreeSet;

use moli_core::Value;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::protocol::{
    self, CoverAction, MAX_CATALOGUE_BYTES, MAX_CHANNELS, MAX_DEVICES, MAX_TEXT_BYTES,
    ProtocolError,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub v: u8,
    pub installation: String,
    pub revision: u64,
    pub devices: Vec<HaDevice>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HaDevice {
    pub id: String,
    pub name: String,
    pub channels: Vec<HaChannel>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HaChannel {
    pub id: String,
    pub entity_id: String,
    pub registry_entity_id: String,
    pub name: String,
    pub unit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic: Option<moli_core::Semantic>,
    pub kind: HaKind,
    pub readable: bool,
    pub actions: Vec<CoverAction>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    deny_unknown_fields,
    try_from = "KindWire"
)]
pub enum HaKind {
    Binary,
    Numeric,
    Text,
    EnumCode {
        catalogue: u32,
        version: u32,
        choices: Vec<HaChoice>,
    },
}

// Internally tagged unit variants otherwise ignore extra object members.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KindWire {
    #[serde(rename = "type")]
    kind: String,
    catalogue: Option<u32>,
    version: Option<u32>,
    choices: Option<Vec<HaChoice>>,
}
impl TryFrom<KindWire> for HaKind {
    type Error = ProtocolError;
    fn try_from(wire: KindWire) -> Result<Self, Self::Error> {
        if wire.kind == "enum_code" {
            return Ok(Self::EnumCode {
                catalogue: wire.catalogue.ok_or(ProtocolError::Catalogue)?,
                version: wire.version.ok_or(ProtocolError::Catalogue)?,
                choices: wire.choices.ok_or(ProtocolError::Catalogue)?,
            });
        }
        if wire.catalogue.is_some() || wire.version.is_some() || wire.choices.is_some() {
            return Err(ProtocolError::Catalogue);
        }
        match wire.kind.as_str() {
            "binary" => Ok(Self::Binary),
            "numeric" => Ok(Self::Numeric),
            "text" => Ok(Self::Text),
            _ => Err(ProtocolError::Value),
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HaChoice {
    pub code: u8,
    pub native: String,
    pub label: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HaState {
    pub v: u8,
    pub installation: String,
    pub catalogue_revision: u64,
    pub device: String,
    pub channel: String,
    pub value: Json,
    pub available: bool,
}

impl Catalog {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        protocol::version(self.v)?;
        protocol::check_id(&self.installation)?;
        if self.revision == 0 {
            return Err(ProtocolError::Revision);
        }
        if self.devices.len() > MAX_DEVICES {
            return Err(ProtocolError::Catalogue);
        }
        let mut devices = BTreeSet::new();
        let mut channels = 0_usize;
        for device in &self.devices {
            protocol::check_id(&device.id)?;
            protocol::text(&device.name)?;
            if !devices.insert(&device.id) {
                return Err(ProtocolError::Catalogue);
            }
            channels = channels
                .checked_add(device.channels.len())
                .ok_or(ProtocolError::Catalogue)?;
            if channels > MAX_CHANNELS {
                return Err(ProtocolError::Catalogue);
            }
            let mut keys = BTreeSet::new();
            for channel in &device.channels {
                channel.validate()?;
                if !keys.insert(&channel.id) {
                    return Err(ProtocolError::Catalogue);
                }
            }
        }
        protocol::encoded_bound(self, MAX_CATALOGUE_BYTES)
    }
    pub fn validate_installation(&self, expected: &str) -> Result<(), ProtocolError> {
        protocol::installation(&self.installation, expected)?;
        self.validate()
    }
    pub fn channel(&self, device: &str, channel: &str) -> Option<&HaChannel> {
        self.devices
            .iter()
            .find(|item| item.id == device)?
            .channels
            .iter()
            .find(|item| item.id == channel)
    }
}
impl HaChannel {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        protocol::check_id(&self.id)?;
        protocol::check_id(&self.entity_id)?;
        protocol::check_id(&self.registry_entity_id)?;
        protocol::text(&self.name)?;
        if let Some(unit) = &self.unit {
            protocol::text(unit)?;
        }
        protocol::actions(&self.actions)?;
        self.kind.validate()
    }
    pub fn writable(&self) -> bool {
        !self.actions.is_empty()
    }
    pub fn convert_value(&self, value: &Json) -> Result<Value, ProtocolError> {
        self.kind.convert_value(value)
    }
}
impl HaKind {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if let Self::EnumCode {
            catalogue,
            version,
            choices,
        } = self
        {
            if *catalogue == 0 || *version == 0 || choices.is_empty() || choices.len() > 256 {
                return Err(ProtocolError::Catalogue);
            }
            let mut codes = BTreeSet::new();
            let mut natives = BTreeSet::new();
            for choice in choices {
                protocol::text(&choice.native)?;
                protocol::text(&choice.label)?;
                if !codes.insert(choice.code) || !natives.insert(&choice.native) {
                    return Err(ProtocolError::Catalogue);
                }
            }
        }
        Ok(())
    }
    /// Catalogue validation belongs to the cold path. Value conversion never
    /// allocates a label dictionary or infers an enum from its display text.
    pub fn convert_value(&self, value: &Json) -> Result<Value, ProtocolError> {
        if value.is_null() {
            return Ok(Value::Null);
        }
        match (self, value) {
            (Self::Binary, Json::Bool(value)) => Ok(Value::Bool(*value)),
            (Self::Numeric, Json::Number(value)) => {
                if let Some(integer) = value.as_i64() {
                    return Ok(Value::Int(integer));
                }
                value
                    .as_f64()
                    .filter(|value| value.is_finite())
                    .map(Value::Float)
                    .ok_or(ProtocolError::Value)
            }
            (Self::Text, Json::String(value)) if value.len() <= MAX_TEXT_BYTES => {
                Ok(Value::Text(value.as_str().into()))
            }
            (Self::EnumCode { choices, .. }, Json::Number(value)) if choices.len() <= 256 => {
                let code = value
                    .as_u64()
                    .and_then(|code| u8::try_from(code).ok())
                    .ok_or(ProtocolError::Value)?;
                if choices.iter().any(|choice| choice.code == code) {
                    Ok(Value::EnumCode(code))
                } else {
                    Err(ProtocolError::Value)
                }
            }
            _ => Err(ProtocolError::Value),
        }
    }
}
impl HaState {
    pub fn validate(&self, catalog: &Catalog) -> Result<Value, ProtocolError> {
        protocol::version(self.v)?;
        protocol::installation(&self.installation, &catalog.installation)?;
        protocol::check_id(&self.device)?;
        protocol::check_id(&self.channel)?;
        if self.catalogue_revision != catalog.revision || self.catalogue_revision == 0 {
            return Err(ProtocolError::Revision);
        }
        let channel = catalog
            .channel(&self.device, &self.channel)
            .ok_or(ProtocolError::Identity)?;
        if !channel.readable {
            return Err(ProtocolError::Value);
        }
        channel.convert_value(&self.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{MAX_COMMAND_BYTES, VERSION, decode};
    use serde_json::json;
    fn channel(id: &str, kind: HaKind) -> HaChannel {
        HaChannel {
            id: id.into(),
            entity_id: format!("sensor.{id}"),
            registry_entity_id: format!("registry-{id}"),
            name: "Cold fixture label".into(),
            unit: None,
            semantic: None,
            kind,
            readable: true,
            actions: vec![],
        }
    }
    fn fixture_catalog() -> Catalog {
        Catalog {
            v: VERSION,
            installation: "fixture-a".into(),
            revision: 1,
            devices: vec![HaDevice {
                id: "device-fixture".into(),
                name: "Fixture".into(),
                channels: vec![
                    channel("temperature", HaKind::Numeric),
                    channel("humidity", HaKind::Numeric),
                ],
            }],
        }
    }
    #[test]
    fn multiple_channels_on_one_device_keep_independent_identity_and_values() {
        let catalog = fixture_catalog();
        catalog.validate().unwrap();
        let mut state = HaState {
            v: VERSION,
            installation: "fixture-a".into(),
            catalogue_revision: 1,
            device: "device-fixture".into(),
            channel: "temperature".into(),
            value: json!(21.5),
            available: true,
        };
        assert_eq!(state.validate(&catalog), Ok(Value::Float(21.5)));
        state.channel = "humidity".into();
        state.value = json!(43);
        assert_eq!(state.validate(&catalog), Ok(Value::Int(43)));
        state.channel = "Cold fixture label".into();
        assert!(state.validate(&catalog).is_err());
        state.channel = "humidity".into();
        state.installation = "fixture-b".into();
        assert_eq!(state.validate(&catalog), Err(ProtocolError::Installation));
        let mut second = catalog.clone();
        second.installation = "fixture-b".into();
        second.validate().unwrap();
        assert_eq!(state.validate(&second), Ok(Value::Int(43)));
        state.catalogue_revision = 2;
        assert_eq!(state.validate(&second), Err(ProtocolError::Revision));
    }
    #[test]
    fn enums_are_codes_and_utf8_text_stays_free_text() {
        let kind = HaKind::EnumCode {
            catalogue: 7,
            version: 1,
            choices: vec![
                HaChoice {
                    code: 0,
                    native: "eco".into(),
                    label: "Économie".into(),
                },
                HaChoice {
                    code: 9,
                    native: "comfort".into(),
                    label: "Confort".into(),
                },
            ],
        };
        kind.validate().unwrap();
        assert_eq!(kind.convert_value(&json!(9)), Ok(Value::EnumCode(9)));
        for value in [
            json!("comfort"),
            json!("9"),
            json!(8),
            json!(256),
            json!(-1),
            json!(9.0),
        ] {
            assert_eq!(kind.convert_value(&value), Err(ProtocolError::Value));
        }
        assert_eq!(kind.convert_value(&Json::Null), Ok(Value::Null));
        let text = "État libre : été 🌡";
        assert_eq!(
            HaKind::Text.convert_value(&json!(text)),
            Ok(Value::Text(text.into()))
        );
        assert!(HaKind::Text.convert_value(&json!("é".repeat(2048))).is_ok());
        assert_eq!(
            HaKind::Text.convert_value(&json!("é".repeat(2049))),
            Err(ProtocolError::Value)
        );
        assert_eq!(
            HaKind::Numeric.convert_value(&json!(true)),
            Err(ProtocolError::Value)
        );
        assert_eq!(
            HaKind::Binary.convert_value(&json!(1)),
            Err(ProtocolError::Value)
        );
        assert_eq!(
            HaKind::Text.convert_value(&json!({"nested":"text"})),
            Err(ProtocolError::Value)
        );
    }
    #[test]
    fn duplicate_ids_actions_enum_codes_and_catalogue_bounds_are_rejected() {
        let mut catalog = fixture_catalog();
        catalog.devices[0].channels[1].id = "temperature".into();
        assert_eq!(catalog.validate(), Err(ProtocolError::Catalogue));
        let mut catalog = fixture_catalog();
        catalog.devices.push(catalog.devices[0].clone());
        assert_eq!(catalog.validate(), Err(ProtocolError::Catalogue));
        let mut cover = channel("cover", HaKind::Binary);
        cover.actions = vec![CoverAction::Open, CoverAction::Open];
        assert_eq!(cover.validate(), Err(ProtocolError::Actions));
        assert!(cover.writable());
        cover.actions.clear();
        assert!(!cover.writable());
        let kind = HaKind::EnumCode {
            catalogue: 1,
            version: 1,
            choices: vec![
                HaChoice {
                    code: 1,
                    native: "one".into(),
                    label: "One".into(),
                },
                HaChoice {
                    code: 1,
                    native: "two".into(),
                    label: "Two".into(),
                },
            ],
        };
        assert_eq!(kind.validate(), Err(ProtocolError::Catalogue));
        let mut catalog = fixture_catalog();
        catalog.devices[0].channels[0].name = "x".repeat(MAX_TEXT_BYTES + 1);
        assert_eq!(catalog.validate(), Err(ProtocolError::Size));
        let mut catalog = fixture_catalog();
        catalog.devices[0].channels = vec![channel("fixture", HaKind::Numeric); MAX_CHANNELS + 1];
        assert_eq!(catalog.validate(), Err(ProtocolError::Catalogue));
        let mut catalog = fixture_catalog();
        catalog.devices = vec![catalog.devices[0].clone(); MAX_DEVICES + 1];
        assert_eq!(catalog.validate(), Err(ProtocolError::Catalogue));
    }
    #[test]
    fn unknown_schema_fields_and_raw_oversized_catalogues_fail_before_use() {
        let mut value = serde_json::to_value(fixture_catalog()).unwrap();
        value["devices"][0]["channels"][0]["origin"] = json!("system");
        assert!(
            decode::<Catalog>(&serde_json::to_vec(&value).unwrap(), MAX_CATALOGUE_BYTES).is_err()
        );
        let oversized = vec![b' '; MAX_CATALOGUE_BYTES + 1];
        assert_eq!(
            decode::<Catalog>(&oversized, MAX_CATALOGUE_BYTES).unwrap_err(),
            ProtocolError::Size
        );
        let bytes = serde_json::to_vec(&json!({"type":"numeric","scale":1})).unwrap();
        assert!(decode::<HaKind>(&bytes, MAX_COMMAND_BYTES).is_err());
    }
    #[test]
    fn semantic_is_optional_cold_metadata_and_preserves_role_without_names() {
        let mut descriptor = channel("opaque-channel", HaKind::Numeric);
        let legacy = serde_json::to_value(&descriptor).unwrap();
        assert!(legacy.get("semantic").is_none());
        let restored: HaChannel = serde_json::from_value(legacy).unwrap();
        assert_eq!(restored.semantic, None);
        descriptor.name = "Fixture".into();
        descriptor.unit = Some("%".into());
        descriptor.semantic = Some(moli_core::Semantic::Humidity);
        let encoded = serde_json::to_value(&descriptor).unwrap();
        assert_eq!(encoded["semantic"], "humidity");
        let restored: HaChannel = serde_json::from_value(encoded).unwrap();
        assert_eq!(restored.semantic, Some(moli_core::Semantic::Humidity));
        assert_eq!(restored.convert_value(&json!(45)), Ok(Value::Int(45)));
    }
}
