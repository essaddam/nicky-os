//! Synthetic load material derived from export shapes. No output contains source identities.
//! This module reads files and prepares bytes; it never opens a broker or sends a command.
use anyhow::{Context, Result, ensure};
use serde_json::{Value as Json, json};
use std::collections::BTreeMap;
use std::path::Path;

const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_ENTITIES: usize = 8192;
const MAX_DEVICES: usize = 2048;
const MAX_INITIAL_MESSAGES: usize = 131072;
const MAX_SCHEMA_BYTES: usize = 256 * 1024;
const MAX_TEXT_BYTES: usize = 4096;

const ATTRIBUTES: &[&str] = &[
    "options",
    "operation_list",
    "hvac_modes",
    "source_list",
    "applications",
    "application_list",
    "apps",
    "friendly_name",
    "unit_of_measurement",
    "device_class",
    "app_id",
    "app_name",
    "current_temperature",
    "current_humidity",
    "temperature",
    "target_temp_low",
    "target_temp_high",
    "hvac_action",
    "brightness",
    "current_position",
    "power",
    "battery_level",
    "volume_level",
    "media_title",
    "source",
    "operation_mode",
    "media_artist",
    "media_album_name",
    "media_duration",
    "media_position",
    "media_position_updated_at",
    "is_volume_muted",
    "supported_features",
    "hs_color",
    "xy_color",
    "rgb_color",
    "color_temp_kelvin",
    "color_temp",
    "color_mode",
];

struct Entity {
    domain: String,
    object: String,
}
struct Message {
    entity: usize,
    attribute: &'static str,
    payload: Vec<u8>,
}
enum Pattern {
    Number {
        min: f64,
        span: f64,
        integer: bool,
    },
    Choices {
        payloads: Vec<Vec<u8>>,
        start: usize,
    },
    Text {
        even: Vec<u8>,
        odd: Vec<u8>,
    },
    Color(&'static str),
    Timestamp,
}
impl Pattern {
    fn payload(&self, phase: u64) -> Vec<u8> {
        match self {
            Self::Number { min, span, integer } => {
                let value = min + span * (phase % 101) as f64 / 100.0;
                if *integer {
                    format!("{:.0}", value).into_bytes()
                } else {
                    format!("{value:.2}").into_bytes()
                }
            }
            Self::Choices { payloads, start } => {
                payloads[(start + phase as usize % payloads.len()) % payloads.len()].clone()
            }
            Self::Text { even, odd } => {
                if phase.is_multiple_of(2) {
                    even.clone()
                } else {
                    odd.clone()
                }
            }
            Self::Color(kind) => match *kind {
                "hs_color" => json!([(phase % 360) as u32, 50]).to_string().into_bytes(),
                "xy_color" => json!([0.2 + (phase % 30) as f64 / 100.0, 0.3])
                    .to_string()
                    .into_bytes(),
                _ => json!([phase % 256, (phase + 71) % 256, (phase + 137) % 256])
                    .to_string()
                    .into_bytes(),
            },
            Self::Timestamp => format!(
                "\"2026-01-01T{:02}:{:02}:{:02}Z\"",
                phase / 3600 % 24,
                phase / 60 % 60,
                phase % 60
            )
            .into_bytes(),
        }
    }
}
struct Update {
    entity: usize,
    attribute: &'static str,
    pattern: Pattern,
}

pub(super) struct SourceFixture {
    template: Json,
    entities: Vec<Entity>,
    initial: Vec<Message>,
    updates: Vec<Update>,
    states: usize,
    matched_states: usize,
    shape: String,
}

fn text<'a>(row: &'a Json, key: &str) -> &'a str {
    row[key].as_str().unwrap_or("")
}
fn read(path: &Path) -> Result<Json> {
    ensure!(
        path.metadata().context("fixture input unavailable")?.len() <= MAX_FILE_BYTES,
        "fixture file exceeds byte bound"
    );
    serde_json::from_slice(&std::fs::read(path).context("fixture input read failed")?)
        .context("invalid fixture JSON")
}
fn rows(data: &Json, key: &str, max: usize) -> Result<Vec<Json>> {
    let rows = data
        .get(key)
        .or_else(|| data.get("data").and_then(|data| data.get(key)))
        .and_then(Json::as_array)
        .context("missing fixture registry rows")?;
    ensure!(rows.len() <= max, "fixture registry exceeds row bound");
    Ok(rows.clone())
}
fn entity(source: &str, index: usize) -> Result<Entity> {
    let (domain, _) = source
        .split_once('.')
        .context("invalid fixture entity shape")?;
    ensure!(
        !domain.is_empty()
            && domain.len() <= 64
            && domain
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'),
        "invalid fixture entity domain"
    );
    let domain = if matches!(
        domain,
        "alarm_control_panel"
            | "automation"
            | "binary_sensor"
            | "button"
            | "calendar"
            | "camera"
            | "climate"
            | "conversation"
            | "counter"
            | "cover"
            | "date"
            | "datetime"
            | "device_tracker"
            | "event"
            | "fan"
            | "geo_location"
            | "group"
            | "humidifier"
            | "image"
            | "input_boolean"
            | "input_button"
            | "input_datetime"
            | "input_number"
            | "input_select"
            | "input_text"
            | "lawn_mower"
            | "light"
            | "lock"
            | "media_player"
            | "number"
            | "person"
            | "remote"
            | "scene"
            | "script"
            | "select"
            | "sensor"
            | "siren"
            | "stt"
            | "sun"
            | "switch"
            | "tag"
            | "text"
            | "time"
            | "timer"
            | "todo"
            | "tts"
            | "update"
            | "vacuum"
            | "valve"
            | "wake_word"
            | "water_heater"
            | "weather"
            | "zone"
            | "notify"
    ) {
        domain
    } else {
        "fixture"
    };
    Ok(Entity {
        domain: domain.into(),
        object: format!("entity_{index:05}"),
    })
}
fn topic(entity: &Entity, copy: usize, attribute: &str) -> String {
    format!(
        "homeassistant/statestream/{}/{}_copy_{copy}/{attribute}",
        entity.domain, entity.object
    )
}
fn disabled(value: &Json) -> Json {
    if value.is_null() {
        Json::Null
    } else {
        json!("fixture_disabled")
    }
}
fn mapped(map: &BTreeMap<String, String>, value: &Json) -> Json {
    value
        .as_str()
        .filter(|value| !value.is_empty())
        .map_or(Json::Null, |value| {
            json!(
                map.get(value)
                    .map_or("fixture_missing_reference", String::as_str)
            )
        })
}
fn schema_class(value: &Json) -> Json {
    let Some(value) = value.as_str() else {
        return Json::Null;
    };
    json!(if matches!(
        value,
        "enum"
            | "temperature"
            | "humidity"
            | "power"
            | "energy"
            | "battery"
            | "illuminance"
            | "door"
            | "window"
            | "opening"
            | "garage_door"
            | "motion"
            | "occupancy"
            | "presence"
            | "current"
            | "voltage"
            | "frequency"
            | "pressure"
            | "duration"
            | "timestamp"
            | "date"
            | "signal_strength"
            | "data_size"
            | "distance"
            | "water"
            | "gas"
            | "monetary"
            | "volume"
            | "weight"
            | "carbon_dioxide"
            | "carbon_monoxide"
            | "pm1"
            | "pm25"
            | "pm10"
            | "aqi"
            | "moisture"
            | "safety"
            | "smoke"
            | "problem"
            | "connectivity"
            | "tamper"
            | "heat"
            | "cold"
            | "running"
            | "update"
            | "sound"
            | "plug"
            | "light"
            | "lock"
            | "battery_charging"
    ) {
        value
    } else {
        "fixture_class"
    })
}
fn unit(value: &Json) -> Json {
    value
        .as_str()
        .filter(|value| value.len() <= 64 && !value.chars().any(char::is_control))
        .map_or(Json::Null, |value| json!(value))
}
fn anonymized(value: &str, variant: bool) -> String {
    value
        .chars()
        .map(|character| match (character.len_utf8(), variant) {
            (1, false) => 'x',
            (1, true) => 'y',
            (2, false) => 'é',
            (2, true) => 'à',
            (3, false) => '中',
            (3, true) => '文',
            (_, false) => '🟢',
            (_, true) => '🔵',
        })
        .collect()
}
fn safe_literal(value: &str) -> bool {
    matches!(
        value,
        "unknown"
            | "unavailable"
            | "None"
            | "null"
            | "on"
            | "off"
            | "ON"
            | "OFF"
            | "heat"
            | "cool"
            | "auto"
            | "dry"
            | "fan_only"
            | "idle"
            | "heating"
            | "cooling"
            | "playing"
            | "paused"
            | "standby"
            | "open"
            | "closed"
            | "opening"
            | "closing"
            | "locked"
            | "unlocked"
            | "xy"
            | "hs"
            | "rgb"
            | "color_temp"
            | "brightness"
    )
}
fn missing(value: &str) -> bool {
    matches!(value, "unknown" | "unavailable" | "None" | "null")
}
fn choice(map: &mut BTreeMap<String, String>, native: &str) -> Result<String> {
    ensure!(
        native.len() <= 256,
        "fixture enum choice exceeds byte bound"
    );
    if let Some(value) = map.get(native) {
        return Ok(value.clone());
    }
    ensure!(map.len() < 1024, "too many fixture choices per entity");
    let code = map.len();
    let value = if native.is_empty() || safe_literal(native) {
        native.to_owned()
    } else if native.parse::<f64>().is_ok_and(f64::is_finite) {
        if native.contains(['e', 'E']) {
            format!("{}e1", 100 + code)
        } else if native.contains('.') {
            format!("{}.0", 100 + code)
        } else {
            format!("{}", 100 + code)
        }
    } else {
        format!("fixture_option_{code:03}")
    };
    map.insert(native.into(), value.clone());
    Ok(value)
}
fn list(value: &Json, map: &mut BTreeMap<String, String>) -> Result<Json> {
    let values = value
        .as_array()
        .context("fixture options must be an array")?;
    ensure!(values.len() <= 256, "too many fixture options");
    Ok(Json::Array(
        values
            .iter()
            .map(|value| {
                choice(
                    map,
                    value.as_str().context("fixture option must be a string")?,
                )
                .map(Json::String)
            })
            .collect::<Result<_>>()?,
    ))
}
fn number_pattern(attribute: &str, unit: &str, class: &str) -> Pattern {
    let (min, span, integer) = match attribute {
        "current_humidity" | "battery_level" => (30.0, 40.0, true),
        "brightness" => (0.0, 254.0, true),
        "current_position" => (0.0, 100.0, true),
        "volume_level" => (0.0, 1.0, false),
        "media_duration" => (180.0, 0.0, false),
        "media_position" => (0.0, 180.0, false),
        "supported_features" => (1.0, 0.0, true),
        "color_temp_kelvin" => (2700.0, 3800.0, true),
        "color_temp" => (153.0, 347.0, true),
        "current_temperature" | "temperature" | "target_temp_low" | "target_temp_high" => {
            (18.0, 10.0, false)
        }
        _ if class == "humidity" || unit == "%" => (30.0, 40.0, true),
        _ if class == "temperature" || matches!(unit, "°C" | "°F") => (18.0, 10.0, false),
        _ => (1.0, 99.0, true),
    };
    Pattern::Number { min, span, integer }
}

impl SourceFixture {
    pub(super) fn load(root: &Path) -> Result<Self> {
        let base = if root.join("sanitized/runtime/states.json").is_file() {
            root.join("sanitized")
        } else {
            root.to_path_buf()
        };
        let states = read(&base.join("runtime/states.json"))?;
        let devices = read(&base.join("registries/core.device_registry.json"))?;
        let entities = read(&base.join("registries/core.entity_registry.json"))?;
        let areas = read(&base.join("registries/core.area_registry.json"))?;
        let floors = read(&base.join("registries/core.floor_registry.json"))?;
        Self::from_data(states, devices, entities, areas, floors)
    }

    fn from_data(
        states: Json,
        devices: Json,
        entities: Json,
        areas: Json,
        floors: Json,
    ) -> Result<Self> {
        let states = states
            .as_array()
            .context("fixture states must be an array")?;
        ensure!(states.len() <= MAX_ENTITIES, "too many fixture states");
        let devices = rows(&devices, "devices", MAX_DEVICES)?;
        let entity_rows = rows(&entities, "entities", MAX_ENTITIES)?;
        let areas = rows(&areas, "areas", 256)?;
        let floors = rows(&floors, "floors", 64)?;
        let shape = format!(
            "fixture-{}-{}-{}-{}",
            devices.len(),
            entity_rows.len(),
            areas.len(),
            floors.len()
        );
        for (values, key) in [(&devices, "id"), (&areas, "id"), (&floors, "floor_id")] {
            let mut seen = std::collections::BTreeSet::new();
            for row in values {
                let native = text(row, key);
                ensure!(
                    !native.is_empty() && seen.insert(native),
                    "invalid fixture registry identity"
                );
            }
        }
        let device_ids: BTreeMap<_, _> = devices
            .iter()
            .enumerate()
            .map(|(index, row)| (text(row, "id").to_owned(), format!("device-{index:05}")))
            .collect();
        let area_ids: BTreeMap<_, _> = areas
            .iter()
            .enumerate()
            .map(|(index, row)| (text(row, "id").to_owned(), format!("area-{index:03}")))
            .collect();
        let floor_ids: BTreeMap<_, _> = floors
            .iter()
            .enumerate()
            .map(|(index, row)| {
                (
                    text(row, "floor_id").to_owned(),
                    format!("floor-{index:03}"),
                )
            })
            .collect();
        let mut entity_ids = BTreeMap::new();
        let mut synthetic_entities = Vec::new();
        for row in &entity_rows {
            let native = text(row, "entity_id");
            ensure!(
                !native.is_empty() && !entity_ids.contains_key(native),
                "invalid fixture entity identity"
            );
            let index = synthetic_entities.len();
            synthetic_entities.push(entity(native, index)?);
            entity_ids.insert(native.to_owned(), index);
        }
        let matched_states = states
            .iter()
            .filter(|row| entity_ids.contains_key(text(row, "entity_id")))
            .count();
        for row in states {
            let native = text(row, "entity_id");
            if !entity_ids.contains_key(native) {
                let index = synthetic_entities.len();
                synthetic_entities.push(entity(native, index)?);
                entity_ids.insert(native.to_owned(), index);
            }
        }
        let mut mappings = vec![BTreeMap::<String, String>::new(); synthetic_entities.len()];
        for row in &entity_rows {
            let index = entity_ids[text(row, "entity_id")];
            if let Some(options) = row["capabilities"].get("options") {
                list(options, &mut mappings[index])?;
            }
            for key in ["options", "operation_list", "hvac_modes", "source_list"] {
                if let Some(value) = row.get(key) {
                    list(value, &mut mappings[index])?;
                }
            }
        }
        for row in states {
            let index = entity_ids[text(row, "entity_id")];
            for key in ["options", "operation_list", "hvac_modes", "source_list"] {
                if let Some(value) = row["attributes"].get(key) {
                    list(value, &mut mappings[index])?;
                }
            }
        }
        let mut output_entities = Vec::with_capacity(entity_rows.len());
        for (index, row) in entity_rows.iter().enumerate() {
            let mut output = json!({"id":format!("entity-{index:05}"),"entity_id":format!("{}.{}",synthetic_entities[index].domain,synthetic_entities[index].object),"device_id":mapped(&device_ids,&row["device_id"]),"area_id":mapped(&area_ids,&row["area_id"]),"name":format!("Fixture entity {index:05}"),"disabled_by":disabled(&row["disabled_by"])});
            output["unit_of_measurement"] = unit(&row["unit_of_measurement"]);
            for key in ["device_class", "original_device_class"] {
                output[key] = schema_class(&row[key]);
            }
            output["entity_category"] =
                if matches!(text(row, "entity_category"), "config" | "diagnostic") {
                    row["entity_category"].clone()
                } else {
                    Json::Null
                };
            output["platform"] = json!(if text(row, "platform") == "mqtt" {
                "mqtt"
            } else {
                "fixture"
            });
            if let Some(value) = row["capabilities"].get("options") {
                output["options"] = list(value, &mut mappings[index])?;
            }
            for key in ["options", "operation_list", "hvac_modes", "source_list"] {
                if let Some(value) = row.get(key) {
                    output[key] = list(value, &mut mappings[index])?;
                }
            }
            output_entities.push(output);
        }
        let output_devices:Vec<_>=devices.iter().enumerate().map(|(index,row)|json!({"id":device_ids[text(row,"id")],"name":format!("Fixture device {index:05}"),"area_id":mapped(&area_ids,&row["area_id"]),"disabled_by":disabled(&row["disabled_by"]),"via_device_id":mapped(&device_ids,&row["via_device_id"])})).collect();
        let output_areas:Vec<_>=areas.iter().enumerate().map(|(index,row)|json!({"id":area_ids[text(row,"id")],"name":format!("Fixture room {index:03}"),"floor_id":mapped(&floor_ids,&row["floor_id"])})).collect();
        let output_floors:Vec<_>=floors.iter().enumerate().map(|(index,row)|json!({"floor_id":floor_ids[text(row,"floor_id")],"name":format!("Fixture floor {index:03}"),"level":row["level"].as_i64()})).collect();
        let template = json!({"floors":output_floors,"areas":output_areas,"devices":output_devices,"entities":output_entities,"temperature_unit":"°C"});
        let mut fixture = Self {
            template,
            entities: synthetic_entities,
            initial: Vec::new(),
            updates: Vec::new(),
            states: states.len(),
            matched_states,
            shape,
        };
        for row in states {
            let index = entity_ids[text(row, "entity_id")];
            let attributes = &row["attributes"];
            let unit = attributes["unit_of_measurement"].as_str().unwrap_or("");
            let class = attributes["device_class"].as_str().unwrap_or("");
            for &attribute in ATTRIBUTES {
                let Some(value) = attributes.get(attribute) else {
                    continue;
                };
                if (value.is_array() || value.is_object())
                    && !matches!(
                        attribute,
                        "options"
                            | "operation_list"
                            | "hvac_modes"
                            | "source_list"
                            | "applications"
                            | "application_list"
                            | "apps"
                            | "hs_color"
                            | "xy_color"
                            | "rgb_color"
                    )
                {
                    continue;
                }

                let (value, pattern) =
                    fixture.attribute(index, attribute, value, attributes, &mut mappings[index])?;
                fixture.add(index, attribute, value, pattern)?;
            }
            let source = row["state"]
                .as_str()
                .map_or_else(|| row["state"].to_string(), str::to_owned);
            let domain = fixture.entities[index].domain.as_str();
            let expected = matches!(
                domain,
                "select" | "input_select" | "climate" | "water_heater"
            ) || domain == "sensor" && class == "enum"
                || attributes.get("options").is_some();
            let base_choices = ["hvac_modes", "operation_list", "options"]
                .into_iter()
                .find_map(|key| attributes[key].as_array())
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Json::as_str)
                        .filter_map(|native| mappings[index].get(native).cloned())
                        .collect::<Vec<_>>()
                })
                .or_else(|| {
                    fixture.template["entities"]
                        .get(index)
                        .and_then(|row| row["options"].as_array())
                        .map(|values| {
                            values
                                .iter()
                                .filter_map(Json::as_str)
                                .map(str::to_owned)
                                .collect()
                        })
                });
            let (payload, pattern) = if missing(&source) {
                (
                    source.as_bytes().to_vec(),
                    Self::state_pattern(domain, base_choices.as_deref(), &source, unit, class),
                )
            } else if expected {
                let value = mappings[index]
                    .get(&source)
                    .cloned()
                    .unwrap_or_else(|| "fixture_unrecognized".into());
                (
                    value.as_bytes().to_vec(),
                    base_choices
                        .as_deref()
                        .and_then(|values| Self::choices_pattern(values, &value, false)),
                )
            } else if matches!(
                domain,
                "light" | "switch" | "binary_sensor" | "input_boolean"
            ) {
                (
                    source.as_bytes().to_vec(),
                    Self::choices_pattern(&["off".into(), "on".into()], &source, false),
                )
            } else if matches!(source.as_str(), "true" | "false") {
                (
                    source.as_bytes().to_vec(),
                    Self::choices_pattern(&["false".into(), "true".into()], &source, false),
                )
            } else if source.parse::<f64>().is_ok_and(f64::is_finite) {
                let pattern = number_pattern("state", unit, class);
                (pattern.payload(0), Some(pattern))
            } else {
                let value = if safe_literal(&source) {
                    source.clone()
                } else {
                    anonymized(&source, false)
                };
                let pattern =
                    Self::state_pattern(domain, None, &value, unit, class).or_else(|| {
                        Some(Pattern::Text {
                            even: value.as_bytes().to_vec(),
                            odd: anonymized(&source, true).into_bytes(),
                        })
                    });
                (value.into_bytes(), pattern)
            };
            fixture.add(index, "state", payload, pattern)?;
        }
        ensure!(
            fixture.initial.len() <= MAX_INITIAL_MESSAGES,
            "fixture message count exceeded"
        );
        Ok(fixture)
    }

    fn choices_pattern(values: &[String], selected: &str, json_payload: bool) -> Option<Pattern> {
        if values.len() < 2 {
            return None;
        }
        Some(Pattern::Choices {
            payloads: values
                .iter()
                .map(|value| {
                    if json_payload {
                        json!(value).to_string().into_bytes()
                    } else {
                        value.as_bytes().to_vec()
                    }
                })
                .collect(),
            start: values
                .iter()
                .position(|value| value == selected)
                .unwrap_or(0),
        })
    }
    fn state_pattern(
        domain: &str,
        values: Option<&[String]>,
        selected: &str,
        unit: &str,
        class: &str,
    ) -> Option<Pattern> {
        if let Some(values) = values {
            return Self::choices_pattern(values, selected, false);
        }
        if domain == "media_player" {
            return Self::choices_pattern(
                &["idle".into(), "playing".into(), "paused".into()],
                selected,
                false,
            );
        }
        if matches!(
            domain,
            "light" | "switch" | "binary_sensor" | "input_boolean"
        ) {
            return Self::choices_pattern(&["off".into(), "on".into()], selected, false);
        }
        if !unit.is_empty()
            || matches!(
                class,
                "temperature" | "humidity" | "power" | "energy" | "battery"
            )
        {
            return Some(number_pattern("state", unit, class));
        }
        None
    }
    fn attribute(
        &self,
        index: usize,
        attribute: &'static str,
        value: &Json,
        attributes: &Json,
        map: &mut BTreeMap<String, String>,
    ) -> Result<(Vec<u8>, Option<Pattern>)> {
        if matches!(
            attribute,
            "options" | "operation_list" | "hvac_modes" | "source_list"
        ) {
            return Ok((list(value, map)?.to_string().into_bytes(), None));
        }
        if matches!(attribute, "applications" | "application_list" | "apps") {
            let mut apps = Vec::new();
            if let Some(values) = value.as_array() {
                ensure!(values.len() <= 256, "too many fixture applications");
                for row in values {
                    let native = row
                        .as_str()
                        .or_else(|| row["id"].as_str())
                        .context("invalid fixture application")?;
                    apps.push(json!({"id":choice(map,native)?,"label":format!("Fixture application {}",apps.len())}));
                }
            } else if let Some(values) = value.as_object() {
                ensure!(values.len() <= 256, "too many fixture applications");
                for native in values.keys() {
                    apps.push(json!({"id":choice(map,native)?,"label":format!("Fixture application {}",apps.len())}));
                }
            } else {
                anyhow::bail!("invalid fixture application list");
            }
            return Ok((json!(apps).to_string().into_bytes(), None));
        }
        if attribute == "friendly_name" {
            return Ok((
                json!(format!("Fixture entity {index:05}"))
                    .to_string()
                    .into_bytes(),
                None,
            ));
        }
        if attribute == "unit_of_measurement" {
            return Ok((unit(value).to_string().into_bytes(), None));
        }
        if attribute == "device_class" {
            return Ok((schema_class(value).to_string().into_bytes(), None));
        }
        if attribute == "app_id" {
            let native = value.as_str().unwrap_or("");
            let mapped = if native.is_empty() || missing(native) {
                native.into()
            } else {
                choice(map, native)?
            };
            return Ok((json!(mapped).to_string().into_bytes(), None));
        }
        if attribute == "app_name" {
            return Ok((
                json!(if missing(value.as_str().unwrap_or("null")) {
                    "unknown"
                } else {
                    "Fixture application"
                })
                .to_string()
                .into_bytes(),
                None,
            ));
        }
        if matches!(attribute, "source" | "operation_mode") {
            let native = value.as_str().unwrap_or("unknown");
            let selected = map.get(native).cloned().unwrap_or_else(|| {
                if missing(native) {
                    native.into()
                } else {
                    "fixture_unrecognized".into()
                }
            });
            let list_key = if attribute == "source" {
                "source_list"
            } else {
                "operation_list"
            };
            let values: Vec<_> = attributes[list_key]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Json::as_str)
                .filter_map(|native| map.get(native).cloned())
                .collect();
            return Ok((
                json!(selected).to_string().into_bytes(),
                Self::choices_pattern(&values, &selected, true),
            ));
        }
        if matches!(attribute, "hs_color" | "xy_color" | "rgb_color") {
            let pattern = Pattern::Color(attribute);
            return Ok((pattern.payload(0), Some(pattern)));
        }
        if attribute == "media_position_updated_at" {
            let pattern = Pattern::Timestamp;
            return Ok((pattern.payload(0), Some(pattern)));
        }
        if attribute == "is_volume_muted" {
            return Ok((
                json!(false).to_string().into_bytes(),
                Some(Pattern::Choices {
                    payloads: vec![b"false".to_vec(), b"true".to_vec()],
                    start: 0,
                }),
            ));
        }
        if value.is_number() {
            let pattern = number_pattern(
                attribute,
                text(attributes, "unit_of_measurement"),
                text(attributes, "device_class"),
            );
            return Ok((pattern.payload(0), Some(pattern)));
        }
        if let Some(value) = value.as_str() {
            let initial = if safe_literal(value) {
                value.into()
            } else {
                anonymized(value, false)
            };
            let alternate = if missing(value) {
                initial.clone()
            } else {
                anonymized(value, true)
            };
            let pattern = if attribute == "hvac_action" {
                Self::choices_pattern(&["idle".into(), "heating".into()], &initial, true)
            } else if attribute == "color_mode" || missing(value) {
                None
            } else {
                Some(Pattern::Text {
                    even: json!(initial).to_string().into_bytes(),
                    odd: json!(alternate).to_string().into_bytes(),
                })
            };
            return Ok((json!(initial).to_string().into_bytes(), pattern));
        }
        Ok((value.to_string().into_bytes(), None))
    }
    fn add(
        &mut self,
        entity: usize,
        attribute: &'static str,
        payload: Vec<u8>,
        pattern: Option<Pattern>,
    ) -> Result<()> {
        let schema = matches!(
            attribute,
            "options"
                | "operation_list"
                | "hvac_modes"
                | "source_list"
                | "applications"
                | "application_list"
                | "apps"
        );
        ensure!(
            payload.len()
                <= if schema {
                    MAX_SCHEMA_BYTES
                } else {
                    MAX_TEXT_BYTES
                },
            "fixture payload exceeds bound"
        );
        ensure!(
            self.initial.len() < MAX_INITIAL_MESSAGES,
            "fixture message count exceeded"
        );
        self.initial.push(Message {
            entity,
            attribute,
            payload,
        });
        if let Some(pattern) = pattern {
            self.updates.push(Update {
                entity,
                attribute,
                pattern,
            });
        }
        Ok(())
    }

    pub(super) fn registry(&self, copy: usize) -> Json {
        let mut registry = self.template.clone();
        registry["house_id"] = json!(format!("{}-copy-{copy}", self.shape));
        for row in registry["floors"].as_array_mut().into_iter().flatten() {
            row["floor_id"] = json!(format!("{}_copy_{copy}", text(row, "floor_id")));
        }
        for row in registry["areas"].as_array_mut().into_iter().flatten() {
            for key in ["id", "floor_id"] {
                if let Some(value) = row[key].as_str() {
                    row[key] = json!(format!("{value}_copy_{copy}"));
                }
            }
        }
        for row in registry["devices"].as_array_mut().into_iter().flatten() {
            for key in ["id", "area_id", "via_device_id"] {
                if let Some(value) = row[key].as_str() {
                    row[key] = json!(format!("{value}_copy_{copy}"));
                }
            }
        }
        for row in registry["entities"].as_array_mut().into_iter().flatten() {
            for key in ["id", "device_id", "area_id"] {
                if let Some(value) = row[key].as_str() {
                    row[key] = json!(format!("{value}_copy_{copy}"));
                }
            }
            row["entity_id"] = json!(format!("{}_copy_{copy}", text(row, "entity_id")));
        }
        registry
    }
    pub(super) fn initial_messages(&self, copy: usize) -> Vec<(String, Vec<u8>)> {
        self.initial
            .iter()
            .map(|message| {
                (
                    topic(&self.entities[message.entity], copy, message.attribute),
                    message.payload.clone(),
                )
            })
            .collect()
    }
    pub(super) fn messages(
        &self,
        copy: usize,
        step: u64,
        changes: usize,
    ) -> Vec<(String, Vec<u8>)> {
        let count = changes.min(self.updates.len());
        if count == 0 {
            return Vec::new();
        }
        let start = u128::from(step) * count as u128;
        let total = self.updates.len() as u128;
        (0..count)
            .map(|offset| {
                let sequence = start + offset as u128;
                let update = &self.updates[(sequence % total) as usize];
                let phase = ((sequence / total) as u64).wrapping_add(1);
                (
                    topic(&self.entities[update.entity], copy, update.attribute),
                    update.pattern.payload(phase),
                )
            })
            .collect()
    }
    pub(super) fn stats(&self) -> Json {
        json!({"registry_devices":self.template["devices"].as_array().map_or(0,Vec::len),"registry_entities":self.template["entities"].as_array().map_or(0,Vec::len),"registry_rooms":self.template["areas"].as_array().map_or(0,Vec::len),"registry_floors":self.template["floors"].as_array().map_or(0,Vec::len),"state_entities":self.states,"states_matching_registry":self.matched_states,"states_without_registry":self.states-self.matched_states,"initial_messages":self.initial.len(),"updatable_points":self.updates.len(),"initial_payload_bytes":self.initial.iter().map(|message|message.payload.len()).sum::<usize>(),"registry_template_bytes":self.template.to_string().len(),"synthetic_identities":true,"physical_publications":0})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> SourceFixture {
        SourceFixture::from_data(json!([
            {"entity_id":"sensor.private_temperature","state":"21.5","attributes":{"unit_of_measurement":"°C","device_class":"temperature"}},
            {"entity_id":"select.private_mode","state":"1e2","attributes":{"options":["1e2","private_option"]}},
            {"entity_id":"media_player.private_player","state":"playing","attributes":{"media_title":"private title","source_list":["private input"],"source":"private input","app_id":"private.application","app_name":"private name","xy_color":[0.3,0.4]}},
            {"entity_id":"switch.unregistered","state":"off","attributes":{}}
        ]),json!({"devices":[{"id":"private_device","name":"private device name","area_id":"private_area"}]}),json!({"entities":[
            {"entity_id":"sensor.private_temperature","device_id":"private_device"},
            {"entity_id":"select.private_mode","device_id":"private_device","capabilities":{"options":["1e2","private_option"]}},
            {"entity_id":"media_player.private_player","device_id":"private_device"}
        ]}),json!({"areas":[{"id":"private_area","name":"private room","floor_id":"private_floor"}]}),json!({"floors":[{"floor_id":"private_floor","name":"private floor","level":1}]})).unwrap()
    }
    #[test]
    fn counts_topology_and_copy_namespaces_survive_without_original_identities() {
        let fixture = fixture();
        assert_eq!(fixture.stats()["registry_entities"], 3);
        assert_eq!(fixture.stats()["state_entities"], 4);
        assert_eq!(fixture.stats()["states_without_registry"], 1);
        let first = fixture.registry(0);
        let second = fixture.registry(1);
        assert_ne!(first["house_id"], second["house_id"]);
        assert_eq!(first["entities"][0]["device_id"], first["devices"][0]["id"]);
        assert_eq!(first["devices"][0]["area_id"], first["areas"][0]["id"]);
        assert_eq!(
            first["areas"][0]["floor_id"],
            first["floors"][0]["floor_id"]
        );
        for registry in [first, second] {
            assert!(!registry.to_string().contains("private"));
        }
        for (topic, payload) in fixture
            .initial_messages(3)
            .into_iter()
            .chain(fixture.messages(3, 7, 100))
        {
            assert!(!topic.contains("private"));
            assert!(!String::from_utf8(payload).unwrap().contains("private"));
            assert!(topic.contains("copy_3"));
        }
    }
    #[test]
    fn initial_enums_keep_numeric_lexeme_shape_and_lists_precede_state() {
        let fixture = fixture();
        let messages = fixture.initial_messages(0);
        let option = messages
            .iter()
            .position(|(topic, _)| topic.ends_with("/options") && topic.contains("/select/"))
            .unwrap();
        let state = messages
            .iter()
            .position(|(topic, _)| topic.ends_with("/state") && topic.contains("/select/"))
            .unwrap();
        assert!(option < state);
        let options: Json = serde_json::from_slice(&messages[option].1).unwrap();
        let native = std::str::from_utf8(&messages[state].1).unwrap();
        assert!(native.contains('e'));
        assert_eq!(options[0], native);
        assert!(
            messages
                .iter()
                .any(|(topic, payload)| topic.ends_with("/app_id")
                    && serde_json::from_slice::<Json>(payload).unwrap().is_string())
        );
    }
    #[test]
    fn updates_are_bounded_deterministic_and_cycle_per_channel_visit() {
        let fixture = fixture();
        assert_eq!(fixture.messages(0, 7, 3), fixture.messages(0, 7, 3));
        assert!(fixture.messages(0, 0, 0).is_empty());
        let count = fixture.updates.len();
        assert_eq!(fixture.messages(0, 1, usize::MAX).len(), count);
        let first = fixture.messages(0, 0, count);
        let next = fixture.messages(0, 1, count);
        assert!(
            first
                .iter()
                .zip(&next)
                .any(|((topic, left), (other, right))| topic == other && left != right)
        );
        for (topic, payload) in next {
            if topic.ends_with("/xy_color") {
                let value: Json = serde_json::from_slice(&payload).unwrap();
                assert!(value[0].as_f64().unwrap() + value[1].as_f64().unwrap() <= 1.0);
            }
            assert!(payload.len() <= MAX_SCHEMA_BYTES);
        }
    }
}
