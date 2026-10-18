//! What the Ariston NET cloud says about a Velis water heater, as Moli's
//! points, and the bodies of the orders: pure, tested on the fixtures.
//!
//! A water heater is not a relay: it has a tank temperature, a setpoint,
//! a mode (by hand or on its own schedule), an eco mode, an
//! anti-legionella cycle and a ceiling on its setpoint. Everything here
//! comes from the network and is treated as hostile: the list is bounded,
//! names trimmed and bounded, gateway ids checked before they enter a
//! path, an unknown value shown as it came, never guessed.

use moli_core::{Access, Device, DeviceId, InstanceId, Kind, PointSpec, Semantic, Unit, Value};
use serde_json::{Value as Json, json};

/// More water heaters than a house has: the rest is ignored.
pub const MAX_HEATERS: usize = 4;
const MAX_NAME: usize = 64;
const MAX_GATEWAY: usize = 32;

/// `sys` of a Velis water heater.
const VELIS: u64 = 4;
/// The `wheType`s served by `medPlantData` (Velis Evo 2 and Andris 2, the
/// same family in the Ariston NET API); others answer elsewhere and are
/// left out until seen.
const MED: [(u64, &str); 2] = [(6, "Velis Evo 2"), (5, "Andris 2")];

/// Modes (`mode`), Moli value ↔ Ariston number.
pub const MODES: [(&str, u64); 2] = [("manuel", 1), ("programme", 5)];

/// The setpoint's range when the heater has not said its own ceiling.
pub const MIN_SETPOINT: f64 = 40.0;
pub const MAX_SETPOINT: f64 = 80.0;
/// The ceiling's own range when the heater has not said it.
const MAX_SETPOINT_RANGE: (f64, f64) = (65.0, 80.0);

/// Settings names (`plantSettings`).
pub const SETTING_MAX: &str = "MedMaxSetpointTemperature";
const SETTING_MAX_MIN: &str = "MedMaxSetpointTemperatureMin";
const SETTING_MAX_MAX: &str = "MedMaxSetpointTemperatureMax";
pub const SETTING_ANTILEG: &str = "MedAntilegionellaOnOff";

/// One water heater of the account (`velis/plants`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heater {
    /// Its gateway: stable, and what every path names.
    pub gateway: String,
    pub name: String,
    pub model: &'static str,
}

/// The water heaters of a `velis/plants` answer Moli drives, at most
/// [`MAX_HEATERS`]; anything else is left out (and logged by the caller).
#[must_use]
pub fn heaters(answer: &Json) -> Vec<Heater> {
    answer
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|plant| {
            let gateway = plant["gw"].as_str()?;
            if gateway.is_empty()
                || gateway.len() > MAX_GATEWAY
                || !gateway.bytes().all(|b| b.is_ascii_alphanumeric())
            {
                return None;
            }
            if plant["sys"].as_u64() != Some(VELIS) {
                return None;
            }
            let kind = plant["wheType"].as_u64()?;
            let model = MED.iter().find(|(t, _)| *t == kind).map(|(_, m)| *m)?;
            let name: String = plant["name"]
                .as_str()
                .unwrap_or_default()
                .chars()
                .filter(|c| !c.is_control())
                .take(MAX_NAME)
                .collect();
            let name = name.trim();
            Some(Heater {
                gateway: gateway.to_owned(),
                name: if name.is_empty() {
                    format!("Ariston {gateway}")
                } else {
                    name.to_owned()
                },
                model,
            })
        })
        .take(MAX_HEATERS)
        .collect()
}

/// A number from the cloud, finite.
fn number(value: &Json) -> Option<f64> {
    value.as_f64().filter(|n| n.is_finite())
}

/// A setting that is on or off (`1.0`, `0.0`, or a boolean).
fn flag(value: &Json) -> Option<bool> {
    value.as_bool().or_else(|| number(value).map(|n| n != 0.0))
}

/// The ceiling the heater accepts for its setpoint, and the range of that
/// ceiling, from its settings.
#[must_use]
pub fn ceiling(settings: &Json) -> (Option<f64>, (f64, f64)) {
    let range = match (
        number(&settings[SETTING_MAX_MIN]),
        number(&settings[SETTING_MAX_MAX]),
    ) {
        (Some(min), Some(max)) if min <= max => (min, max),
        _ => MAX_SETPOINT_RANGE,
    };
    (number(&settings[SETTING_MAX]), range)
}

impl Heater {
    /// The device Moli shows; `range`: its setpoint ceiling's range.
    #[must_use]
    pub fn device(&self, instance: &InstanceId, range: (f64, f64)) -> Device {
        use Semantic::{Config, Control, Other, Temperature};
        use Shape::{Binary, Celsius, Count, Minutes, Modes};
        // key, label, shape, writable, meaning. Switching the hot water off
        // and the anti-legionella safeguard off are a person's call when an
        // agent asks (`control`).
        let table: [(&str, &str, Shape, bool, Semantic); 13] = [
            ("on", "pilotes.ariston.marche", Binary, true, Control),
            ("mode", "pilotes.ariston.mode", Modes, true, Config),
            (
                "setpoint",
                "pilotes.ariston.consigne",
                Celsius(Some((MIN_SETPOINT, MAX_SETPOINT))),
                true,
                Config,
            ),
            ("eco", "pilotes.ariston.eco", Binary, true, Config),
            (
                "temperature",
                "pilotes.ariston.temperature",
                Celsius(None),
                false,
                Temperature,
            ),
            (
                "effective_setpoint",
                "pilotes.ariston.consigne_effective",
                Celsius(None),
                false,
                Other,
            ),
            ("heating", "pilotes.ariston.chauffe", Binary, false, Other),
            ("showers", "pilotes.ariston.douches", Count, false, Other),
            ("remaining", "pilotes.ariston.reste", Minutes, false, Other),
            (
                "antilegionella_cycle",
                "pilotes.ariston.cycle_antilegionelle",
                Binary,
                false,
                Other,
            ),
            (
                "antilegionella",
                "pilotes.ariston.antilegionelle",
                Binary,
                true,
                Control,
            ),
            (
                "max_setpoint",
                "pilotes.ariston.consigne_max",
                Celsius(Some(range)),
                true,
                Config,
            ),
            ("errors", "pilotes.ariston.erreurs", Count, false, Other),
        ];
        let points = table
            .into_iter()
            .map(|(key, label, shape, write, semantic)| {
                let (kind, unit) = shape.kind();
                PointSpec {
                    key: key.into(),
                    label: moli_i18n::tr!(label).into(),
                    kind,
                    access: Access { read: true, write },
                    unit,
                    semantic,
                }
            })
            .collect();
        Device {
            id: DeviceId::new(instance, &self.gateway),
            instance: instance.clone(),
            native_name: self.name.as_str().into(),
            manufacturer: Some("Ariston".into()),
            model: Some(self.model.into()),
            description: None,
            native_room: None,
            members: Vec::new(),
            points,
        }
    }
}

/// The shape of a point's values.
#[derive(Clone, Copy)]
enum Shape {
    Binary,
    Modes,
    /// Degrees, bounded for the orders when writable.
    Celsius(Option<(f64, f64)>),
    Minutes,
    Count,
}

impl Shape {
    fn kind(self) -> (Kind, Option<Unit>) {
        let free = Kind::Numeric {
            min: None,
            max: None,
            step: None,
        };
        match self {
            Self::Binary => (Kind::Binary, None),
            Self::Modes => (
                Kind::Enum {
                    values: MODES.iter().map(|(m, _)| (*m).into()).collect(),
                },
                None,
            ),
            Self::Celsius(None) => (free, Some(Unit::Celsius)),
            Self::Celsius(Some((min, max))) => (
                Kind::Numeric {
                    min: Some(min),
                    max: Some(max),
                    step: Some(1.0),
                },
                Some(Unit::Celsius),
            ),
            Self::Minutes => (free, Some(Unit::Minute)),
            Self::Count => (free, None),
        }
    }
}

/// The points read from `medPlantData/{gw}`. A missing field is `Null`,
/// never a guess.
#[must_use]
pub fn state(data: &Json) -> Vec<(&'static str, Value)> {
    let num = |key: &str| number(&data[key]).map_or(Value::Null, Value::Float);
    let boolean = |key: &str| data[key].as_bool().map_or(Value::Null, Value::Bool);
    let mode = data["mode"].as_u64().map_or(Value::Null, |m| {
        MODES.iter().find(|(_, n)| *n == m).map_or_else(
            || Value::from(m.to_string().as_str()),
            |(name, _)| Value::from(*name),
        )
    });
    vec![
        ("on", boolean("on")),
        ("mode", mode),
        ("setpoint", num("reqTemp")),
        ("eco", boolean("eco")),
        ("temperature", num("temp")),
        ("effective_setpoint", num("procReqTemp")),
        ("heating", boolean("heatReq")),
        ("showers", num("avShw")),
        ("remaining", remaining(&data["rmTm"])),
        ("antilegionella_cycle", boolean("antiLeg")),
    ]
}

/// `rmTm` (« HH:MM:SS ») in minutes.
fn remaining(value: &Json) -> Value {
    let Some(text) = value.as_str().filter(|t| t.len() <= 12) else {
        return Value::Null;
    };
    let parts: Vec<u64> = text.split(':').filter_map(|p| p.parse().ok()).collect();
    match parts[..] {
        [h, m, _] if h < 1000 && m < 60 => Value::Int(i64::try_from(h * 60 + m).unwrap_or(0)),
        _ => Value::Null,
    }
}

/// The points read from `medPlantData/{gw}/plantSettings`.
#[must_use]
pub fn settings(settings: &Json) -> Vec<(&'static str, Value)> {
    vec![
        (
            "antilegionella",
            flag(&settings[SETTING_ANTILEG]).map_or(Value::Null, Value::Bool),
        ),
        (
            "max_setpoint",
            number(&settings[SETTING_MAX]).map_or(Value::Null, Value::Float),
        ),
    ]
}

/// The errors the heater reports (`busErrors`): how many.
#[must_use]
pub fn errors(answer: &Json) -> Value {
    answer.as_array().map_or(Value::Null, |list| {
        Value::Int(i64::try_from(list.len()).unwrap_or(i64::MAX))
    })
}

/// An order: where it goes (below `velis/medPlantData/{gw}/`) and its body.
#[derive(Clone, Debug, PartialEq)]
pub struct Order {
    pub action: &'static str,
    pub body: Json,
}

/// What it takes to make `key` read `value`, from the heater as last read
/// (`settings`: its last settings, for the ones that send the old value
/// too). `Err`: why not, for the caller.
pub fn order(key: &str, value: &Value, settings: &Json) -> Result<Order, String> {
    let unknown = || moli_i18n::tr!("pilotes.ariston.point_non_ecrivable");
    let order = match (key, value) {
        ("on", Value::Bool(on)) => Order {
            action: "switch",
            body: Json::Bool(*on),
        },
        ("eco", Value::Bool(on)) => Order {
            action: "switchEco",
            body: Json::Bool(*on),
        },
        ("mode", Value::Text(m)) => {
            let (_, n) = MODES
                .iter()
                .find(|(name, _)| *name == &**m)
                .ok_or_else(unknown)?;
            Order {
                action: "mode",
                body: json!({ "new": n }),
            }
        }
        ("setpoint", v) => {
            let t = v.as_f64().ok_or_else(unknown)?;
            // The heater refuses above its own ceiling: say why, now.
            if let (Some(ceiling), _) = ceiling(settings)
                && t > ceiling
            {
                return Err(moli_i18n::tr!("pilotes.ariston.au_dessus", max = ceiling));
            }
            Order {
                action: "temperature",
                body: json!({ "new": t }),
            }
        }
        ("max_setpoint", v) => {
            let t = v.as_f64().ok_or_else(unknown)?;
            let old = number(&settings[SETTING_MAX])
                .ok_or_else(|| moli_i18n::tr!("pilotes.ariston.reglages_inconnus"))?;
            Order {
                action: "plantSettings",
                body: json!({ SETTING_MAX: { "new": t, "old": old } }),
            }
        }
        ("antilegionella", Value::Bool(on)) => {
            let old = flag(&settings[SETTING_ANTILEG])
                .ok_or_else(|| moli_i18n::tr!("pilotes.ariston.reglages_inconnus"))?;
            let as_number = |b: bool| if b { 1.0 } else { 0.0 };
            Order {
                action: "plantSettings",
                body: json!({ SETTING_ANTILEG: { "new": as_number(*on), "old": as_number(old) } }),
            }
        }
        _ => return Err(unknown()),
    };
    Ok(order)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Json {
        let path = format!(
            "{}/../../integrations/ariston/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    fn get(values: &[(&str, Value)], key: &str) -> Value {
        values.iter().find(|(k, _)| *k == key).unwrap().1.clone()
    }

    #[test]
    fn the_fixtures_read() {
        let list = heaters(&fixture("velis-plants.json"));
        assert_eq!(list.len(), 1, "a Lydos and a bad gateway are left out");
        assert_eq!(list[0].gateway, "A1B2C3D4E5F6");
        assert_eq!(list[0].model, "Velis Evo 2");
        let data = state(&fixture("med-plant-data.json"));
        assert_eq!(get(&data, "on"), Value::Bool(true));
        assert_eq!(get(&data, "mode"), Value::from("manuel"));
        assert_eq!(get(&data, "setpoint"), Value::Float(55.0));
        assert_eq!(get(&data, "temperature"), Value::Float(50.0));
        assert_eq!(get(&data, "remaining"), Value::Int(75));
        let settings = fixture("plant-settings.json");
        let read = super::settings(&settings);
        assert_eq!(get(&read, "antilegionella"), Value::Bool(true));
        assert_eq!(get(&read, "max_setpoint"), Value::Float(80.0));
        assert_eq!(ceiling(&settings), (Some(80.0), (65.0, 80.0)));
        assert_eq!(errors(&fixture("bus-errors.json")), Value::Int(0));
        let device = list[0].device(&InstanceId::from("eau-chaude"), (65.0, 80.0));
        assert_eq!(device.points.len(), 13);
    }

    #[test]
    fn orders() {
        let settings = fixture("plant-settings.json");
        let o = |k: &str, v: Value| order(k, &v, &settings);
        assert_eq!(o("on", Value::Bool(false)).unwrap().body, json!(false));
        assert_eq!(
            o("mode", Value::from("programme")).unwrap().body,
            json!({ "new": 5 })
        );
        assert_eq!(
            o("setpoint", Value::Float(60.0)).unwrap(),
            Order {
                action: "temperature",
                body: json!({ "new": 60.0 })
            }
        );
        let anti = o("antilegionella", Value::Bool(false)).unwrap();
        assert_eq!(
            anti.body,
            json!({ SETTING_ANTILEG: { "new": 0.0, "old": 1.0 } })
        );
        let max = o("max_setpoint", Value::Float(70.0)).unwrap();
        assert_eq!(
            max.body,
            json!({ SETTING_MAX: { "new": 70.0, "old": 80.0 } })
        );
        // Above the heater's own ceiling: refused, with why.
        let low = json!({ SETTING_MAX: 65.0 });
        assert!(
            order("setpoint", &Value::Float(70.0), &low)
                .unwrap_err()
                .contains("65")
        );
        // Without its settings, an order that sends the old value waits.
        assert!(order("antilegionella", &Value::Bool(true), &Json::Null).is_err());
        assert!(o("temperature", Value::Float(50.0)).is_err());
        assert!(o("mode", Value::from("boost")).is_err());
    }

    #[test]
    fn hostile_answers() {
        let many: Vec<Json> = (0..20)
            .map(|i| json!({ "gw": format!("GW{i}"), "sys": 4, "wheType": 6, "name": "x".repeat(300) }))
            .collect();
        let list = heaters(&Json::from(many));
        assert_eq!(list.len(), MAX_HEATERS);
        assert_eq!(list[0].name.chars().count(), MAX_NAME);
        for gw in ["", "../x", "a b", &"A".repeat(40)] {
            assert!(
                heaters(&json!([{ "gw": gw, "sys": 4, "wheType": 6 }])).is_empty(),
                "{gw:?}"
            );
        }
        let odd = state(&json!({ "mode": 8, "rmTm": "99:99:99", "temp": "hot" }));
        assert_eq!(get(&odd, "mode"), Value::from("8"), "shown, not guessed");
        assert_eq!(get(&odd, "remaining"), Value::Null);
        assert_eq!(get(&odd, "temperature"), Value::Null);
        assert_eq!(get(&odd, "on"), Value::Null);
        assert_eq!(
            ceiling(&json!({ SETTING_MAX_MIN: 90, SETTING_MAX_MAX: 10 })).1,
            (65.0, 80.0)
        );
        assert_eq!(errors(&json!({})), Value::Null);
    }
}
