//! Read-only RAM projection into the real moli-runtime Hub. Timestamps are
//! reception timestamps in this POC, not evidence of fresh upstream measures.
use super::*;
use moli_core::{Access, Device, DeviceId, EnumCodes, Kind, PointSpec, Semantic, Unit, Value};

fn scalar(value: &Json, domain: &str) -> Value {
    match value {
        Json::Null => Value::Null,
        Json::Bool(v) => Value::Bool(*v),
        Json::Number(v) => v
            .as_i64()
            .map_or_else(|| Value::Float(v.as_f64().unwrap_or(0.0)), Value::Int),
        Json::String(v) => {
            if matches!(
                domain,
                "light" | "binary_sensor" | "switch" | "input_boolean"
            ) {
                match v.as_str() {
                    "on" | "ON" => return Value::Bool(true),
                    "off" | "OFF" => return Value::Bool(false),
                    _ => {}
                }
            }
            Value::Text(v.as_str().into())
        }
        _ => Value::Null,
    }
}
pub(super) fn channel_value(channel: &Channel) -> Value {
    if channel.enum_expected {
        channel
            .valeur
            .as_u64()
            .and_then(|code| u8::try_from(code).ok())
            .filter(|_| channel.enum_ref.is_some())
            .map_or(Value::Null, Value::EnumCode)
    } else {
        scalar(&channel.valeur, &channel.domaine)
    }
}
fn channel_kind(house: &House, channel: &Channel, value: &Value, unit: bool) -> Kind {
    if let Some(reference) = channel.enum_ref {
        let mut codes = EnumCodes::empty();
        for code in house.enum_pool.active_codes(reference).unwrap_or_default() {
            codes.insert(code);
        }
        Kind::EnumCode {
            catalogue: reference.id,
            version: reference.version,
            codes,
        }
    } else {
        kind(value, unit, &channel.domaine)
    }
}
fn meaning(s: &str) -> Semantic {
    match s {
        "temperature" => Semantic::Temperature,
        "humidite" => Semantic::Humidity,
        "lumiere" | "interrupteur" => Semantic::OnOff,
        "ouverture" => Semantic::Contact,
        "presence" => Semantic::Occupancy,
        "puissance" => Semantic::Power,
        "energie" => Semantic::Energy,
        "batterie" => Semantic::Battery,
        "intensite" => Semantic::Brightness,
        "diagnostic" => Semantic::Diagnostic,
        "configuration" | "consigne_chauffage" => Semantic::Config,
        _ => Semantic::Other,
    }
}
fn kind(value: &Value, unit: bool, domain: &str) -> Kind {
    match value {
        Value::Bool(_) => Kind::Binary,
        Value::Int(_) | Value::Float(_) => Kind::Numeric {
            min: None,
            max: None,
            step: None,
        },
        Value::Null if unit => Kind::Numeric {
            min: None,
            max: None,
            step: None,
        },
        Value::Null
            if matches!(
                domain,
                "light" | "binary_sensor" | "switch" | "input_boolean"
            ) =>
        {
            Kind::Binary
        }
        _ => Kind::Text,
    }
}
impl House {
    pub(super) fn hub_schema(&self, ctx: &moli_runtime::DriverCtx, description: &Json) {
        for d in description["appareils"].as_array().into_iter().flatten() {
            let Some(native) = d["id"].as_str() else {
                continue;
            };
            let points = d["canaux"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|id| {
                    let id = id.as_str()?;
                    let info = &description["canaux"][id];
                    let runtime = self.channels.get(id)?;
                    let unit = info["unite"].as_str().and_then(Unit::parse);
                    let value = channel_value(runtime);
                    let point_kind = channel_kind(self, runtime, &value, unit.is_some());
                    Some(PointSpec {
                        key: id.into(),
                        label: info["nom"].as_str().unwrap_or(id).into(),
                        kind: point_kind,
                        access: Access {
                            read: true,
                            write: false,
                        },
                        unit,
                        semantic: meaning(&runtime.semantique),
                    })
                })
                .collect();
            ctx.upsert_device(Device {
                id: DeviceId::new(ctx.instance(), native),
                instance: ctx.instance().clone(),
                native_name: d["nom"].as_str().unwrap_or("MQTT").into(),
                manufacturer: None,
                model: None,
                description: None,
                native_room: None,
                members: Vec::new(),
                points,
            });
        }
        for n in description["noeuds"].as_array().into_iter().flatten() {
            let Some(native) = n["id"].as_str() else {
                continue;
            };
            let points = description["generiques"][native]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|g| {
                    Some(PointSpec {
                        key: g["id"].as_str()?.into(),
                        label: g["nom"].as_str().unwrap_or("").into(),
                        kind: Kind::Numeric {
                            min: None,
                            max: None,
                            step: None,
                        },
                        access: Access {
                            read: true,
                            write: false,
                        },
                        unit: g["unite"].as_str().and_then(Unit::parse),
                        semantic: meaning(g["semantique"].as_str().unwrap_or("")),
                    })
                })
                .collect();
            let members = n["appareils"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Json::as_str)
                .map(|id| DeviceId::new(ctx.instance(), id))
                .collect();
            ctx.upsert_device(Device {
                id: DeviceId::new(ctx.instance(), native),
                instance: ctx.instance().clone(),
                native_name: n["nom"].as_str().unwrap_or("MQTT").into(),
                manufacturer: None,
                model: None,
                description: None,
                native_room: None,
                members,
                points,
            });
        }
    }
    pub(super) fn hub_publication(&self, ctx: &moli_runtime::DriverCtx, payload: &Json) {
        if payload["type"] == "canal"
            && let Some(c) = payload["id"].as_str().and_then(|id| self.channels.get(id))
        {
            ctx.set_state(
                &DeviceId::new(ctx.instance(), &c.device),
                &c.id,
                channel_value(c),
            );
        } else if payload["type"] == "noeud"
            && let Some(native) = payload["id"].as_str()
        {
            for g in payload["generiques"].as_array().into_iter().flatten() {
                if let Some(key) = g["id"].as_str() {
                    ctx.set_state(
                        &DeviceId::new(ctx.instance(), native),
                        key,
                        scalar(&g["valeur"], "sensor"),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod enum_tests {
    use super::*;
    #[test]
    fn real_hub_schema_uses_enum_code_and_a_membership_mask_without_labels() {
        let mut house = House::new("moli-probe/pont/fixture".into());
        house.registry(&serde_json::to_vec(&json!({"floors":[],"areas":[],"devices":[],"entities":[{"id":"fixture","entity_id":"select.fixture","entity_category":"config","original_device_class":"enum"}]})).unwrap()).unwrap();
        assert!(house.ingest(
            "homeassistant/statestream/select/fixture/options",
            br#"["1","2"]"#,
            true
        ));
        assert!(house.ingest("homeassistant/statestream/select/fixture/state", b"2", true));
        let channel = &house.channels[&id("canal", "fixture")];
        let value = channel_value(channel);
        assert_eq!(value, Value::EnumCode(1));
        let reference = channel.enum_ref.unwrap();
        let Kind::EnumCode {
            catalogue,
            version,
            codes,
        } = channel_kind(&house, channel, &value, false)
        else {
            panic!("enum schema required");
        };
        assert_eq!(catalogue, reference.id);
        assert_eq!(version, reference.version);
        assert!(codes.contains(0) && codes.contains(1));
        assert!(!codes.contains(2));
        assert!(house.ingest(
            "homeassistant/statestream/select/fixture/state",
            b"not listed",
            false
        ));
        assert_eq!(
            channel_value(&house.channels[&id("canal", "fixture")]),
            Value::Null
        );
    }
}
