//! Offline adapters around public production functions, without a hub or transport.
use std::collections::HashMap;
use std::sync::Arc;

use moli_core::{DeviceId, InstanceId, PointId, Value, validate};
use moli_profile::{Answer, Compiled, Format, Profile, Source};
use moli_z2m::{Catalog, Entry};
use serde::Deserialize;
use serde_json::{Value as Json, json};

pub const BASE: &str = "synthetic";
pub const PROFILE: &str = include_str!("../fixtures/profile.toml");
pub const PROFILE_JSON: &[u8] =
    br#"{"temperature":2154,"power":12345,"on":"ON","mode":2,"brightness":127}"#;
pub const CONTROL_KV: &[u8] = b"ret=OK,pow=0,mode=3,stemp=21.0,shum=0,f_rate=A,f_dir=0";

pub fn compiled() -> Compiled {
    Profile::parse(PROFILE)
        .unwrap()
        .compile(Source::File)
        .unwrap()
}

pub fn catalog(devices: usize) -> Catalog {
    let devices: Vec<_> = (0..devices).map(|i| json!({
        "ieee_address": format!("synthetic-{i:08}"),
        "friendly_name": format!("lamp-{i}"), "type": "Router",
        "definition": {"vendor":"Example", "model":"Synthetic", "exposes":[
            {"type":"binary","property":"state","access":7,"value_on":"ON","value_off":"OFF"},
            {"type":"numeric","property":"brightness","access":7,"value_min":0,"value_max":254,"value_step":1},
            {"type":"numeric","property":"temperature","access":1,"unit":"°C"},
            {"type":"numeric","property":"power","access":1,"unit":"W"},
            {"type":"enum","property":"mode","access":7,"values":[0,1,2]},
            {"type":"text","property":"label","access":7},
            {"type":"composite","property":"color","features":[
                {"type":"numeric","property":"x","access":7,"value_min":0,"value_max":1},
                {"type":"numeric","property":"y","access":7,"value_min":0,"value_max":1}
            ]}
        ]}
    })).collect();
    Catalog::from_bridge_devices(
        &InstanceId::from("synthetic"),
        &serde_json::to_vec(&devices).unwrap(),
    )
    .unwrap()
}

pub fn payload(i: usize) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "state": if i.is_multiple_of(2) {"ON"} else {"OFF"},
        "brightness": i % 255, "temperature": 18.0 + (i % 100) as f64 / 10.0,
        "power": i % 1000, "mode": i % 3, "label": format!("sample-{i}"),
        "color":{"x":0.3,"y":0.4}, "ignored":{"firmware":"synthetic"}
    }))
    .unwrap()
}

/// Mirrors only state-topic routing in private `moli_z2m::on_publish`.
/// Catalog routing, JSON parse and codec calls themselves are production functions.
/// Does not include keypad handling, hub state, MQTT framing, availability or catalog reload.
pub fn ingress(catalog: &Catalog, topic: &str, bytes: &[u8]) -> Option<Vec<(Arc<str>, Value)>> {
    let name = topic.strip_prefix(BASE)?.strip_prefix('/')?;
    if name.starts_with("bridge/")
        || name.ends_with("/set")
        || name.ends_with("/get")
        || name.contains("/set/")
        || name.contains("/get/")
        || name.ends_with("/availability")
    {
        return None;
    }
    let entry = catalog.by_name(name)?;
    let Json::Object(object) = serde_json::from_slice(bytes).ok()? else {
        return None;
    };
    Some(entry.decode(&object))
}

#[derive(Debug, Deserialize)]
pub struct Command {
    pub point: PointId,
    pub value: Value,
}

/// Minimal offline command adapter: production identity lookup, spec validation,
/// codec and JSON serialization. Access check mirrors Hub::prepare; there is no
/// guard, journal, deadline, approval or delivery. Never use this to issue commands.
pub fn action(catalog: &Catalog, bytes: &[u8]) -> Result<Vec<u8>, &'static str> {
    let command: Command = serde_json::from_slice(bytes).map_err(|_| "json")?;
    let (id, key) = command.point.split().ok_or("identity")?;
    let entry = catalog.by_id(&id).ok_or("device")?;
    encode_command(entry, key, &command.value)
}

/// EXPERIMENT ONLY. Rebuild after every production catalog replacement;
/// borrows entries so it cannot outlive the catalog or survive its replacement.
pub type ActionIndex<'a> = HashMap<DeviceId, &'a Entry>;

pub fn action_index(catalog: &Catalog) -> ActionIndex<'_> {
    catalog
        .entries()
        .map(|entry| (entry.device.id.clone(), entry))
        .collect()
}

pub fn action_indexed(index: &ActionIndex<'_>, bytes: &[u8]) -> Result<Vec<u8>, &'static str> {
    let command: Command = serde_json::from_slice(bytes).map_err(|_| "json")?;
    let (id, key) = command.point.split().ok_or("identity")?;
    let entry = index.get(&id).ok_or("device")?;
    encode_command(entry, key, &command.value)
}

fn encode_command(entry: &Entry, key: &str, value: &Value) -> Result<Vec<u8>, &'static str> {
    let spec = entry.device.point(key).ok_or("point")?;
    if !spec.access.write {
        return Err("read_only");
    }
    let value = validate(&spec.kind, value).map_err(|_| "value")?;
    serde_json::to_vec(&entry.encode(key, &value)).map_err(|_| "encode")
}

pub fn read_profile(profile: &Compiled, bytes: &[u8]) -> Vec<Option<Value>> {
    let answer = Answer::parse(Format::Json, bytes).unwrap();
    profile.points.iter().map(|p| p.read(&answer)).collect()
}

pub fn control() -> HashMap<String, Answer> {
    HashMap::from([(
        "control".into(),
        Answer::parse(Format::Kv, CONTROL_KV).unwrap(),
    )])
}

/// EXPERIMENT ONLY: a proposed bounded bidirectional affine descriptor.
/// Not called by Moli's production drivers. Rejects outside either domain;
/// rounds the native command after conversion when the device needs integers.
#[derive(Debug, Clone, Copy)]
pub struct Affine {
    native_min: f64,
    native_max: f64,
    ratio: f64,
    inverse: bool,
    integer_native: bool,
}

impl Affine {
    pub fn compile(
        native_min: f64,
        native_max: f64,
        inverse: bool,
        integer_native: bool,
    ) -> Result<Self, &'static str> {
        let span = native_max - native_min;
        let ratio = 100.0 / span;
        if !native_min.is_finite()
            || !native_max.is_finite()
            || span <= 0.0
            || !span.is_finite()
            || !ratio.is_finite()
            || ratio <= 0.0
            || (integer_native && (native_min.fract() != 0.0 || native_max.fract() != 0.0))
        {
            return Err("domain");
        }
        Ok(Self {
            native_min,
            native_max,
            ratio,
            inverse,
            integer_native,
        })
    }

    pub fn inbound(&self, native: f64) -> Result<f64, &'static str> {
        if !native.is_finite() || native < self.native_min || native > self.native_max {
            return Err("native_range");
        }
        let percent = if native == self.native_min {
            0.0
        } else if native == self.native_max {
            100.0
        } else {
            (native - self.native_min) * self.ratio
        };
        Ok(if self.inverse {
            100.0 - percent
        } else {
            percent
        })
    }

    pub fn outbound(&self, percent: f64) -> Result<f64, &'static str> {
        if !percent.is_finite() || !(0.0..=100.0).contains(&percent) {
            return Err("percent_range");
        }
        let percent = if self.inverse {
            100.0 - percent
        } else {
            percent
        };
        // Explicit endpoints avoid a floating point residue outside the native domain.
        let native = if percent == 0.0 {
            self.native_min
        } else if percent == 100.0 {
            self.native_max
        } else {
            self.native_min + percent / self.ratio
        };
        Ok(if self.integer_native {
            native.round()
        } else {
            native
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moli_core::{Kind, Unit, unit_ratio};

    #[test]
    fn mqtt_real_binary_numeric_enum_and_nested_codecs_round_trip() {
        let cat = catalog(2);
        let values = ingress(&cat, "synthetic/lamp-1", &payload(127)).unwrap();
        assert_eq!(values.len(), 8);
        assert!(values.contains(&("state".into(), Value::Bool(false))));
        assert!(values.contains(&("mode".into(), Value::from("1"))));
        assert!(values.contains(&("color.x".into(), Value::Float(0.3))));
        assert_eq!(
            action(
                &cat,
                br#"{"point":"synthetic:synthetic-00000001/state","value":true}"#
            )
            .unwrap(),
            br#"{"state":"ON"}"#
        );
        assert_eq!(
            action(
                &cat,
                br#"{"point":"synthetic:synthetic-00000001/mode","value":"2"}"#
            )
            .unwrap(),
            br#"{"mode":2}"#
        );
    }

    #[test]
    fn native_brightness_keeps_254_scale_and_rejects_out_of_range_without_clamping() {
        let cat = catalog(1);
        for n in 0..=254 {
            let bytes = serde_json::to_vec(&json!({"brightness":n})).unwrap();
            assert_eq!(
                ingress(&cat, "synthetic/lamp-0", &bytes).unwrap(),
                vec![("brightness".into(), Value::Int(n))]
            );
            let command = serde_json::to_vec(
                &json!({"point":"synthetic:synthetic-00000000/brightness","value":n}),
            )
            .unwrap();
            assert_eq!(
                serde_json::from_slice::<Json>(&action(&cat, &command).unwrap()).unwrap(),
                json!({"brightness":n})
            );
        }
        for value in [json!(-1), json!(255), Json::Null, json!("50"), json!(true)] {
            let cmd = serde_json::to_vec(
                &json!({"point":"synthetic:synthetic-00000000/brightness","value":value}),
            )
            .unwrap();
            assert_eq!(action(&cat, &cmd), Err("value"));
        }
        // Production inbound codec intentionally does not apply command bounds.
        assert_eq!(
            ingress(&cat, "synthetic/lamp-0", br#"{"brightness":255}"#).unwrap()[0].1,
            Value::Int(255)
        );
    }

    #[test]
    fn profile_value_scale_rounding_missing_invalid_and_enum() {
        let p = compiled();
        assert_eq!(
            read_profile(&p, PROFILE_JSON),
            vec![
                Some(Value::Float(21.54)),
                Some(Value::Float(12.345)),
                Some(Value::Bool(true)),
                Some(Value::from("high")),
                Some(Value::Float(50.0))
            ]
        );
        let a = Answer::parse(
            Format::Json,
            br#"{"temperature":"NaN","power":"1e999","on":null,"mode":999,"brightness":254}"#,
        )
        .unwrap();
        assert_eq!(p.points[0].read(&a), Some(Value::Null));
        assert_eq!(p.points[1].read(&a), Some(Value::Null));
        assert_eq!(p.points[2].read(&a), Some(Value::Null));
        assert_eq!(p.points[3].read(&a), Some(Value::Null));
        assert_eq!(p.points[4].read(&a), Some(Value::Float(100.0)));
        let missing = Answer::parse(Format::Json, b"{}").unwrap();
        assert_eq!(p.points[0].read(&missing), None);
        assert_eq!(p.points[4].read(&missing), Some(Value::Null));
        // A percentage mapping is explicit in this synthetic profile; it is
        // not present in the production Z2M codec and has no implicit inverse.
        for raw in 0..=254 {
            let a = Answer::Json(json!({"brightness":raw}));
            let got = p.points[4].read(&a).unwrap().as_f64().unwrap();
            let expected = ((raw as f64 * (100.0 / 254.0)) * 1000.0).round() / 1000.0;
            assert!((got - expected).abs() < 1e-12);
        }
    }

    #[test]
    fn profile_action_uses_compiled_mapping_and_echoes_other_parameters() {
        let p = Profile::parse(moli_profile::builtin("daikin-brp069").unwrap())
            .unwrap()
            .compile(Source::Builtin)
            .unwrap();
        let on = p.writes.iter().find(|w| &*w.key == "on").unwrap();
        let path = on.render(&Value::Bool(true), &control(), |_| None).unwrap();
        assert_eq!(
            path,
            "/aircon/set_control_info?pow=1&mode=3&stemp=21.0&shum=0&f_rate=A&f_dir=0"
        );
        let target = p
            .writes
            .iter()
            .find(|w| &*w.key == "target_temperature")
            .unwrap();
        assert!(
            target
                .render(&Value::Float(22.0), &control(), |_| None)
                .unwrap()
                .contains("stemp=22.0")
        );
        assert!(
            on.render(&Value::Bool(true), &HashMap::new(), |_| None)
                .is_err()
        );
        assert!(
            validate(
                &p.points
                    .iter()
                    .find(|p| &*p.spec.key == "target_temperature")
                    .unwrap()
                    .spec
                    .kind,
                &Value::Float(31.0)
            )
            .is_err()
        );
    }

    #[test]
    fn typed_actions_buttons_parameters_identity_and_readonly_are_distinct() {
        let cat = catalog(1);
        assert_eq!(validate(&Kind::Button, &Value::Null).unwrap(), Value::Null);
        assert!(validate(&Kind::Button, &Value::Bool(true)).is_err());
        for point in [
            "bad",
            "synthetic:other/state",
            "synthetic:synthetic-00000000/no-such-point",
            "synthetic:synthetic-00000000/temperature",
            "synthetic:synthetic-00000000/color.x",
        ] {
            let cmd = serde_json::to_vec(&json!({"point":point,"value":21})).unwrap();
            assert!(action(&cat, &cmd).is_err());
        }
        // Current command Value is scalar; arbitrary named parameter objects
        // require a separate typed contract and are rejected by serde here.
        assert!(
            action(
                &cat,
                br#"{"point":"synthetic:synthetic-00000000/state","value":{"on":true}}"#
            )
            .is_err()
        );
        let k = Kind::Numeric {
            min: Some(0.0),
            max: Some(254.0),
            step: Some(1.0),
        };
        for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(validate(&k, &Value::Float(v)).is_err());
        }
        assert!(ingress(&cat, "synthetic/lamp-0/set", b"{}").is_none());
        assert!(ingress(&cat, "synthetic/lamp-0", b"[]").is_none());
        assert!(ingress(&cat, "synthetic/lamp-0", b"{").is_none());
    }

    #[test]
    fn production_unit_ratios_have_explicit_compatible_pairs() {
        assert_eq!(
            unit_ratio(Some(&Unit::WattHour), Some(&Unit::KiloWattHour)),
            Some(0.001)
        );
        assert_eq!(
            unit_ratio(Some(&Unit::KiloWattHour), Some(&Unit::WattHour)),
            Some(1000.0)
        );
        assert_eq!(
            unit_ratio(Some(&Unit::MilliVolt), Some(&Unit::Volt)),
            Some(0.001)
        );
        assert_eq!(
            unit_ratio(Some(&Unit::Second), Some(&Unit::Hour)),
            Some(1.0 / 3600.0)
        );
        assert_eq!(unit_ratio(Some(&Unit::Watt), Some(&Unit::Celsius)), None);
        assert_eq!(unit_ratio(Some(&Unit::Watt), Some(&Unit::Watt)), Some(1.0));
    }

    #[test]
    fn experimental_affine_inverse_domains_endpoints_and_integer_roundtrips() {
        for inverse in [false, true] {
            let a = Affine::compile(0.0, 254.0, inverse, true).unwrap();
            assert_eq!(a.outbound(0.0).unwrap(), if inverse { 254.0 } else { 0.0 });
            assert_eq!(
                a.outbound(100.0).unwrap(),
                if inverse { 0.0 } else { 254.0 }
            );
            for raw in 0..=254 {
                assert_eq!(
                    a.outbound(a.inbound(raw as f64).unwrap()).unwrap(),
                    raw as f64
                );
            }
            for percent in [0.0, 0.1, 25.0, 50.0, 99.9, 100.0] {
                let actual = a.inbound(a.outbound(percent).unwrap()).unwrap();
                assert!((actual - percent).abs() <= 50.0 / 254.0 + 1e-12);
            }
            for bad in [-1.0, 101.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
                assert!(a.outbound(bad).is_err());
            }
            for bad in [-1.0, 255.0, f64::NAN, f64::INFINITY] {
                assert!(a.inbound(bad).is_err());
            }
        }
        let offset = Affine::compile(10.0, 110.0, true, false).unwrap();
        assert_eq!(offset.inbound(35.0).unwrap(), 75.0);
        assert_eq!(offset.outbound(75.0).unwrap(), 35.0);
        for (min, max) in [(-3.7, 9.1), (0.1, 0.7), (12.2, 13.9)] {
            for inverse in [false, true] {
                let a = Affine::compile(min, max, inverse, false).unwrap();
                assert_eq!(a.inbound(min).unwrap(), if inverse { 100.0 } else { 0.0 });
                assert_eq!(a.inbound(max).unwrap(), if inverse { 0.0 } else { 100.0 });
                assert_eq!(a.outbound(a.inbound(min).unwrap()).unwrap(), min);
                assert_eq!(a.outbound(a.inbound(max).unwrap()).unwrap(), max);
            }
        }
        for (min, max) in [
            (1.0, 1.0),
            (2.0, 1.0),
            (f64::NAN, 254.0),
            (0.0, f64::INFINITY),
            (-f64::MAX, f64::MAX),
        ] {
            assert!(Affine::compile(min, max, false, false).is_err());
        }
        assert!(Affine::compile(0.5, 254.0, false, true).is_err());
    }

    #[test]
    fn experimental_action_index_matches_production_lookup_for_all_ids_and_failures() {
        let cat = catalog(100);
        let index = action_index(&cat);
        assert_eq!(index.len(), 100);
        for i in 0..100 {
            for (key, value) in [
                ("state", json!(true)),
                ("brightness", json!(127)),
                ("mode", json!("2")),
                ("brightness", json!(300)),
                ("temperature", json!(21.0)),
                ("color.x", json!(0.2)),
                ("absent", Json::Null),
            ] {
                let command = serde_json::to_vec(
                    &json!({"point":format!("synthetic:synthetic-{i:08}/{key}"),"value":value}),
                )
                .unwrap();
                assert_eq!(action(&cat, &command), action_indexed(&index, &command));
            }
        }
        for invalid in [
            b"bad".as_slice(),
            br#"{"point":"bad","value":true}"#,
            br#"{"point":"synthetic:absent/state","value":true}"#,
        ] {
            assert_eq!(action(&cat, invalid), action_indexed(&index, invalid));
        }
        // Replacement requires a fresh index: renamed topic routes to the
        // same ID only through the new catalog. No stale pointer is retained.
        drop(index);
        let replacement = catalog(1);
        let rebuilt = action_index(&replacement);
        assert_eq!(rebuilt.len(), 1);
    }
}
