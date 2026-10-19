//! Bounded weather decoding, independent of the network and the hub.

use std::collections::{BTreeMap, BTreeSet};

use moli_core::{Access, Device, DeviceId, InstanceId, Kind, PointSpec, Semantic, Unit, Value};
use serde_json::Value as Json;

use crate::Error;

pub(crate) const MAX_BODY: usize = 512 * 1024;
const MAX_STATIONS: usize = 16;
const MAX_MODULES: usize = 16;
const MAX_DEVICES: usize = 128;

#[cfg(test)]
mod tests {
    use moli_core::{InstanceId, Value};
    use moli_history::Family;
    use serde_json::{Value as Json, json};

    use super::*;

    const FIXTURE: &[u8] = include_bytes!("../../../integrations/netatmo/fixtures/weather.json");
    const NOW: u64 = 1_700_000_300;

    fn input() -> Json {
        serde_json::from_slice(FIXTURE).unwrap()
    }

    fn parsed(value: &Json) -> Weather {
        decode(&serde_json::to_vec(value).unwrap(), NOW, 1800).unwrap()
    }

    #[test]
    fn station_and_modules_keep_identity() {
        let mut value = input();
        let before = parsed(&value);
        value["body"]["devices"][0]["station_name"] = json!("New name");
        value["body"]["devices"][0]["modules"]
            .as_array_mut()
            .unwrap()
            .reverse();
        let after = parsed(&value);
        let ids = |w: &Weather| {
            let mut ids: Vec<_> = w.readings.iter().map(|r| r.native.clone()).collect();
            ids.sort();
            ids
        };
        assert_eq!(before.readings.len(), 3);
        assert_eq!(ids(&before), ids(&after));
        let station = &before.readings[0];
        assert_eq!(station.values["temperature"].as_f64(), Some(21.5));
        assert!(station.online);
    }

    #[test]
    fn old_http_success_is_offline() {
        let mut value = input();
        value["body"]["devices"][0]["dashboard_data"]["time_utc"] = json!(NOW - 1801);
        let weather = parsed(&value);
        assert!(!weather.readings[0].online);
        assert_eq!(weather.readings[0].values["temperature"], Value::Null);
    }

    #[test]
    fn missing_field_is_null() {
        let mut value = input();
        value["body"]["devices"][0]["dashboard_data"]
            .as_object_mut()
            .unwrap()
            .remove("Temperature");
        let weather = parsed(&value);
        assert_eq!(weather.readings[0].values["temperature"], Value::Null);
        assert_eq!(weather.readings[0].values["humidity"].as_f64(), Some(45.0));
    }

    #[test]
    fn duplicate_identity_is_rejected() {
        let mut value = input();
        value["body"]["devices"][0]["modules"][0]["_id"] =
            value["body"]["devices"][0]["_id"].clone();
        assert!(decode(&serde_json::to_vec(&value).unwrap(), NOW, 1800).is_err());
    }

    #[test]
    fn unknown_types_are_ignored() {
        let mut value = input();
        value["body"]["devices"][0]["modules"][0]["type"] = json!("NACamera");
        assert_eq!(parsed(&value).readings.len(), 2);
    }

    #[test]
    fn real_point_specs_have_b1_families() {
        let weather = parsed(&input());
        let mut count = 0;
        for reading in weather.readings {
            for point in reading.device(&InstanceId::from("weather")).points {
                let expected = match point.key.as_ref() {
                    "temperature" | "humidity" | "pressure" | "co2" => Family::Climate,
                    "battery" => Family::Diagnostic,
                    "noise" | "rain" | "rain_1h" | "rain_24h" => Family::Other,
                    other => panic!("unexpected weather point {other}"),
                };
                assert_eq!(Family::of(&point), expected, "{}", point.key);
                assert!(!point.access.write);
                count += 1;
            }
        }
        assert!(count >= 10);
    }

    #[test]
    fn hostile_sizes_types_and_times_are_bounded() {
        assert!(decode(&vec![b' '; 512 * 1024 + 1], NOW, 1800).is_err());
        let mut value = input();
        value["body"]["devices"][0]["_id"] = json!("x".repeat(65));
        assert!(decode(&serde_json::to_vec(&value).unwrap(), NOW, 1800).is_err());
        value = input();
        value["body"]["devices"][0]["dashboard_data"]["time_utc"] = json!(NOW + 301);
        assert!(!parsed(&value).readings[0].online);
        value = input();
        value["body"]["devices"][0]["dashboard_data"]["Humidity"] = json!(101);
        assert_eq!(parsed(&value).readings[0].values["humidity"], Value::Null);
        value = input();
        value["body"]["devices"][0]["reachable"] = json!(false);
        assert!(!parsed(&value).readings[0].online);
    }
}

#[derive(Debug)]
pub(crate) struct Weather {
    pub readings: Vec<Reading>,
}

#[derive(Debug)]
pub(crate) struct Reading {
    pub native: String,
    pub values: BTreeMap<String, Value>,
    pub online: bool,
    pub measured_at: Option<u64>,
    pub valid_until: u64,
    model: &'static str,
    name: String,
}

impl Reading {
    pub fn device(&self, instance: &InstanceId) -> Device {
        Device {
            id: DeviceId::new(instance, &self.native),
            instance: instance.clone(),
            native_name: self.name.clone().into(),
            manufacturer: Some("Netatmo".into()),
            model: Some(self.model.into()),
            description: None,
            native_room: None,
            members: Vec::new(),
            points: measures(self.model).iter().map(Measure::spec).collect(),
        }
    }
}

struct Measure {
    field: &'static str,
    key: &'static str,
    unit: &'static str,
    semantic: Semantic,
    min: f64,
    max: f64,
}

impl Measure {
    fn spec(&self) -> PointSpec {
        PointSpec {
            key: self.key.into(),
            label: moli_i18n::tr!(&format!("netatmo.point.{}", self.key)).into(),
            kind: Kind::Numeric {
                min: Some(self.min),
                max: Some(self.max),
                step: None,
            },
            access: Access {
                read: true,
                write: false,
            },
            unit: Unit::parse(self.unit),
            semantic: self.semantic,
        }
    }

    fn value(&self, json: &Json) -> Value {
        json.get(self.field)
            .and_then(Json::as_f64)
            .filter(|n| n.is_finite() && *n >= self.min && *n <= self.max)
            .map_or(Value::Null, Value::Float)
    }
}

const TEMP: Measure = Measure {
    field: "Temperature",
    key: "temperature",
    unit: "°C",
    semantic: Semantic::Temperature,
    min: -100.0,
    max: 100.0,
};
const HUMIDITY: Measure = Measure {
    field: "Humidity",
    key: "humidity",
    unit: "%",
    semantic: Semantic::Humidity,
    min: 0.0,
    max: 100.0,
};
const BATTERY: Measure = Measure {
    field: "battery_percent",
    key: "battery",
    unit: "%",
    semantic: Semantic::Battery,
    min: 0.0,
    max: 100.0,
};
const STATION: &[Measure] = &[
    TEMP,
    HUMIDITY,
    Measure {
        field: "CO2",
        key: "co2",
        unit: "ppm",
        semantic: Semantic::Other,
        min: 0.0,
        max: 20000.0,
    },
    Measure {
        field: "Pressure",
        key: "pressure",
        unit: "hPa",
        semantic: Semantic::Pressure,
        min: 100.0,
        max: 1500.0,
    },
    Measure {
        field: "Noise",
        key: "noise",
        unit: "dB",
        semantic: Semantic::Other,
        min: 0.0,
        max: 200.0,
    },
];
const OUTDOOR: &[Measure] = &[TEMP, HUMIDITY, BATTERY];
const RAIN: &[Measure] = &[
    Measure {
        field: "Rain",
        key: "rain",
        unit: "mm",
        semantic: Semantic::Other,
        min: 0.0,
        max: 100_000.0,
    },
    Measure {
        field: "sum_rain_1",
        key: "rain_1h",
        unit: "mm",
        semantic: Semantic::Other,
        min: 0.0,
        max: 100_000.0,
    },
    Measure {
        field: "sum_rain_24",
        key: "rain_24h",
        unit: "mm",
        semantic: Semantic::Other,
        min: 0.0,
        max: 100_000.0,
    },
    BATTERY,
];

fn measures(model: &str) -> &'static [Measure] {
    match model {
        "NAMain" => STATION,
        "NAModule1" => OUTDOOR,
        "NAModule3" => RAIN,
        _ => &[],
    }
}

fn reading(
    json: &Json,
    now: u64,
    stale: u64,
    parent_reachable: bool,
) -> Result<Option<Reading>, Error> {
    let model = match json.get("type").and_then(Json::as_str) {
        Some("NAMain") => "NAMain",
        Some("NAModule1") => "NAModule1",
        Some("NAModule3") => "NAModule3",
        _ => return Ok(None),
    };
    let native = json
        .get("_id")
        .and_then(Json::as_str)
        .ok_or(Error::Invalid)?;
    if native.is_empty()
        || native.len() > 64
        || !native
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_:".contains(&b))
    {
        return Err(Error::Invalid);
    }
    let data = &json["dashboard_data"];
    let measured_at = data
        .get("time_utc")
        .and_then(Json::as_u64)
        .filter(|stamp| *stamp > 0 && *stamp <= now.saturating_add(300));
    let valid_until = measured_at.map_or(0, |stamp| stamp.saturating_add(stale));
    let online = parent_reachable
        && json.get("reachable").and_then(Json::as_bool) == Some(true)
        && measured_at.is_some()
        && now <= valid_until;
    let values = measures(model)
        .iter()
        .map(|measure| {
            let source = if measure.field == "battery_percent" {
                json
            } else {
                data
            };
            let value = if online {
                measure.value(source)
            } else {
                Value::Null
            };
            (measure.key.to_owned(), value)
        })
        .collect();
    let name = json
        .get(if model == "NAMain" {
            "station_name"
        } else {
            "module_name"
        })
        .and_then(Json::as_str)
        .unwrap_or_default();
    let name: String = name.chars().filter(|c| !c.is_control()).take(128).collect();
    Ok(Some(Reading {
        native: native.to_ascii_lowercase(),
        values,
        online,
        measured_at,
        valid_until,
        model,
        name: if name.trim().is_empty() {
            format!("Netatmo {model}")
        } else {
            name
        },
    }))
}

pub(crate) fn decode(bytes: &[u8], now_s: u64, stale_s: u64) -> Result<Weather, Error> {
    if bytes.len() > MAX_BODY {
        return Err(Error::Invalid);
    }
    let json: Json = serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?;
    if json.get("status").and_then(Json::as_str) != Some("ok") {
        return Err(Error::Invalid);
    }
    let stations = json["body"]["devices"].as_array().ok_or(Error::Invalid)?;
    if stations.len() > MAX_STATIONS {
        return Err(Error::Invalid);
    }
    let mut readings = Vec::new();
    let mut identities = BTreeSet::new();
    for station in stations {
        if station.get("type").and_then(Json::as_str) != Some("NAMain") {
            continue;
        }
        let Some(main) = reading(station, now_s, stale_s, true)? else {
            continue;
        };
        push(&mut readings, &mut identities, main)?;
        let Some(modules) = station.get("modules") else {
            continue;
        };
        let modules = modules.as_array().ok_or(Error::Invalid)?;
        if modules.len() > MAX_MODULES {
            return Err(Error::Invalid);
        }
        let reachable = station.get("reachable").and_then(Json::as_bool) == Some(true);
        for module in modules {
            if let Some(module) = reading(module, now_s, stale_s, reachable)? {
                push(&mut readings, &mut identities, module)?;
            }
        }
    }
    Ok(Weather { readings })
}

fn push(
    readings: &mut Vec<Reading>,
    identities: &mut BTreeSet<String>,
    reading: Reading,
) -> Result<(), Error> {
    if readings.len() >= MAX_DEVICES || !identities.insert(reading.native.clone()) {
        return Err(Error::Invalid);
    }
    readings.push(reading);
    Ok(())
}
