use moli_core::Unit;
use serde_json::{Map, Value as Json, json};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug, Default)]
pub struct Discovery {
    configs: u64,
    entities: u64,
    tombstones: u64,
    migrations: u64,
    invalid: u64,
    unknown_components: u64,
    missing_unique_id: u64,
    missing_device_identity: u64,
    disabled: u64,
    components: BTreeMap<&'static str, u64>,
    device_classes: BTreeMap<&'static str, u64>,
    state_classes: BTreeMap<&'static str, u64>,
    units: BTreeMap<&'static str, u64>,
    templates: BTreeMap<&'static str, u64>,
    availability_modes: BTreeMap<&'static str, u64>,
    availability_topics: HashSet<String>,
    state_topics: BTreeMap<String, u64>,
    shared_state_topics: u64,
    attributes_topics: u64,
    command_topics: u64,
    command_templates: u64,
    retained_configs: u64,
    qos_configs: u64,
    expire_after: u64,
    force_update: u64,
    display_precision: u64,
    json_attributes: u64,
    last_reset: u64,
    enum_options: u64,
    binary_encoding: u64,
    diagnostic: u64,
    custom_availability_payloads: u64,
    unsafe_topics: u64,
    home_online: u64,
    home_offline: u64,
    state_payloads: u64,
    payload_forms: BTreeMap<&'static str, u64>,
    other_payloads: u64,
    other_payload_forms: BTreeMap<&'static str, u64>,
    other_text_lengths: BTreeMap<&'static str, u64>,
    families: BTreeMap<&'static str, u64>,
    ha_domains: BTreeMap<&'static str, u64>,
    ha_suffixes: BTreeMap<&'static str, u64>,
    ha_depths: BTreeMap<u8, u64>,
    ha_second_shapes: BTreeMap<&'static str, u64>,
    device_groups: HashMap<String, u64>,
    alarm: crate::alarm::Alarm,
}

fn bump(map: &mut BTreeMap<&'static str, u64>, key: &'static str) {
    *map.entry(key).or_default() += 1;
}

fn field<'a>(object: &'a Map<String, Json>, long: &str, short: &str) -> Option<&'a Json> {
    object.get(long).or_else(|| object.get(short))
}

fn text<'a>(object: &'a Map<String, Json>, long: &str, short: &str) -> Option<&'a str> {
    field(object, long, short)?.as_str()
}

fn component(name: &str) -> &'static str {
    match name {
        "sensor" => "sensor",
        "binary_sensor" => "binary_sensor",
        "switch" => "switch",
        "number" => "number",
        "select" => "select",
        "button" => "button",
        "light" => "light",
        "climate" => "climate",
        "cover" => "cover",
        "fan" => "fan",
        "lock" => "lock",
        "vacuum" => "vacuum",
        "device_automation" => "device_automation",
        "device" => "device",
        "event" => "event",
        "text" => "text",
        "alarm_control_panel" => "alarm_control_panel",
        "camera" => "camera",
        "humidifier" => "humidifier",
        "image" => "image",
        "lawn_mower" => "lawn_mower",
        "notify" => "notify",
        "scene" => "scene",
        "siren" => "siren",
        "update" => "update",
        "valve" => "valve",
        "water_heater" => "water_heater",
        _ => "other",
    }
}

fn device_class(name: &str) -> &'static str {
    match name {
        "temperature" => "temperature",
        "humidity" => "humidity",
        "power" => "power",
        "energy" => "energy",
        "battery" => "battery",
        "illuminance" => "illuminance",
        "voltage" => "voltage",
        "current" => "current",
        "pressure" => "pressure",
        "timestamp" => "timestamp",
        "date" => "date",
        "duration" => "duration",
        "signal_strength" => "signal_strength",
        "enum" => "enum",
        "moisture" => "moisture",
        "pm1" => "pm1",
        "pm25" => "pm25",
        "pm10" => "pm10",
        "carbon_dioxide" => "carbon_dioxide",
        "carbon_monoxide" => "carbon_monoxide",
        _ => "other",
    }
}

fn family_of(topic: &str) -> &'static str {
    match topic.split('/').next().unwrap_or_default() {
        "zigbee2mqtt" => "zigbee2mqtt",
        "homeassistant" => "homeassistant",
        "frigate" => "frigate",
        "esphome" => "esphome",
        "tele" => "tele",
        "stat" => "stat",
        "cmnd" => "cmnd",
        "macosmqtt" => "macosmqtt",
        "alarmo" => "alarmo",
        "bambu" => "bambu",
        "shellies" => "shellies",
        "owntracks" => "owntracks",
        "music_assistant" => "music_assistant",
        "$SYS" => "sys",
        _ => "other",
    }
}

fn ha_domain(segment: &str) -> &'static str {
    match segment {
        "sensor" => "sensor",
        "binary_sensor" => "binary_sensor",
        "switch" => "switch",
        "light" => "light",
        "number" => "number",
        "select" => "select",
        "button" => "button",
        "update" => "update",
        "text" => "text",
        "climate" => "climate",
        "cover" => "cover",
        "fan" => "fan",
        "lock" => "lock",
        "alarm_control_panel" => "alarm_control_panel",
        "device_automation" => "device_automation",
        "device" => "device",
        "event" => "event",
        "camera" => "camera",
        "image" => "image",
        "notify" => "notify",
        "scene" => "scene",
        "siren" => "siren",
        "valve" => "valve",
        "water_heater" => "water_heater",
        "humidifier" => "humidifier",
        "lawn_mower" => "lawn_mower",
        "vacuum" => "vacuum",
        "device_tracker" => "device_tracker",
        "person" => "person",
        "zone" => "zone",
        "sun" => "sun",
        "weather" => "weather",
        "media_player" => "media_player",
        "remote" => "remote",
        "automation" => "automation",
        "script" => "script",
        "input_boolean" => "input_boolean",
        "input_button" => "input_button",
        "input_number" => "input_number",
        "input_select" => "input_select",
        "input_text" => "input_text",
        "input_datetime" => "input_datetime",
        "counter" => "counter",
        "timer" => "timer",
        "schedule" => "schedule",
        "group" => "group",
        "air_quality" => "air_quality",
        "assist_satellite" => "assist_satellite",
        "conversation" => "conversation",
        "stt" => "stt",
        "tts" => "tts",
        "todo" => "todo",
        "plant" => "plant",
        "wake_word" => "wake_word",
        "statestream" => "statestream",
        "status" => "status",
        "tag" => "tag",
        _ => "other",
    }
}

fn ha_suffix(segment: &str) -> &'static str {
    match segment {
        "config" => "config",
        "state" => "state",
        "availability" => "availability",
        "attributes" => "attributes",
        "set" => "set",
        "get" => "get",
        "press" => "press",
        "action" => "action",
        "event" => "event",
        _ => "other",
    }
}

fn shape_of(segment: &str) -> &'static str {
    if segment.is_empty() {
        return "empty";
    }
    if segment.len() == 18
        && segment.starts_with("0x")
        && segment[2..].chars().all(|c| c.is_ascii_hexdigit())
    {
        return "ieee_like";
    }
    if segment.chars().all(|c| c.is_ascii_digit()) {
        return "numeric";
    }
    if segment.len() == 36 && segment.chars().filter(|c| *c == '-').count() == 4 {
        return "uuid_like";
    }
    if segment.split(':').count() == 6
        && segment
            .split(':')
            .all(|part| part.len() == 2 && part.chars().all(|c| c.is_ascii_hexdigit()))
    {
        return "mac_like";
    }
    if segment
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    {
        return "word";
    }
    "mixed"
}

fn group_key(device: &Map<String, Json>) -> Option<String> {
    if let Some(ids) = field(device, "identifiers", "ids") {
        if let Some(id) = ids.as_str() {
            return Some(id.to_owned());
        }
        if let Some(id) = ids
            .as_array()
            .and_then(|ids| ids.first())
            .and_then(Json::as_str)
        {
            return Some(id.to_owned());
        }
    }
    field(device, "connections", "cns")
        .and_then(Json::as_array)
        .and_then(|connections| connections.first())
        .map(ToString::to_string)
}

fn template_kind(template: &str) -> &'static str {
    let template = template.trim();
    if !(template.starts_with("{{") && template.ends_with("}}")) {
        return "literal_or_unsupported";
    }
    let body = template[2..template.len() - 2].trim();
    if body == "value" {
        return "value";
    }
    if let Some(path) = body.strip_prefix("value_json.")
        && !path.is_empty()
        && path.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(character, '_' | '.' | '[' | ']' | '\'' | '"')
        })
    {
        return "value_json_path";
    }
    "jinja_unsupported"
}

fn classify(payload: &[u8], map: &mut BTreeMap<&'static str, u64>) -> &'static str {
    let form = if payload.is_empty() {
        "empty"
    } else if let Ok(text) = std::str::from_utf8(payload) {
        match serde_json::from_str::<Json>(text) {
            Ok(Json::Object(_)) => "json_object",
            Ok(Json::Array(_)) => "json_array",
            Ok(Json::Bool(_)) => "json_boolean",
            Ok(Json::Number(_)) => "json_number",
            Ok(Json::Null) => "json_null",
            Ok(Json::String(_)) => "json_string",
            Err(_) => {
                let text = text.trim();
                if text.eq_ignore_ascii_case("on") || text.eq_ignore_ascii_case("off") {
                    "boolean_word"
                } else if text.eq_ignore_ascii_case("none") || text.eq_ignore_ascii_case("null") {
                    "null_word"
                } else if text.parse::<f64>().is_ok_and(f64::is_finite) {
                    "number_text"
                } else {
                    "other_text"
                }
            }
        }
    } else {
        "non_utf8"
    };
    bump(map, form);
    form
}

pub fn safe_subscription(topic: &str) -> bool {
    !topic.is_empty()
        && topic.len() <= 2048
        && !topic
            .chars()
            .any(|character| character.is_control() || matches!(character, '#' | '+'))
        && !crate::is_request(topic)
}

impl Discovery {
    fn topic(&mut self, topic: &str, output: &mut Vec<String>, availability: bool) {
        if !safe_subscription(topic) {
            self.unsafe_topics += 1;
            return;
        }
        if availability {
            self.availability_topics.insert(topic.to_owned());
        } else {
            *self.state_topics.entry(topic.to_owned()).or_default() += 1;
        }
        output.push(topic.to_owned());
    }

    fn identity(&mut self, object: &Map<String, Json>, inherited: Option<&Map<String, Json>>) {
        if !text(object, "unique_id", "uniq_id").is_some_and(|id| !id.is_empty()) {
            self.missing_unique_id += 1;
        }
        let Some(device) = field(object, "device", "dev")
            .and_then(Json::as_object)
            .or(inherited)
        else {
            self.missing_device_identity += 1;
            return;
        };
        let identifiers = field(device, "identifiers", "ids").is_some_and(|value| {
            value.as_str().is_some_and(|id| !id.is_empty())
                || value.as_array().is_some_and(|ids| !ids.is_empty())
        });
        let connections = field(device, "connections", "cns")
            .and_then(Json::as_array)
            .is_some_and(|connections| !connections.is_empty());
        if !identifiers && !connections {
            self.missing_device_identity += 1;
            return;
        }
        if let Some(key) = group_key(device) {
            *self.device_groups.entry(key).or_default() += 1;
        }
    }

    fn entity(
        &mut self,
        object: &Map<String, Json>,
        platform: &str,
        inherited_device: Option<&Map<String, Json>>,
        output: &mut Vec<String>,
    ) {
        self.entities += 1;
        if platform == "alarm_control_panel" {
            self.alarm.inspect(object);
        }
        bump(&mut self.components, component(platform));
        self.identity(object, inherited_device);
        if field(object, "enabled_by_default", "enabled_by_default") == Some(&Json::Bool(false)) {
            self.disabled += 1;
        }
        if let Some(class) = text(object, "device_class", "dev_cla") {
            bump(&mut self.device_classes, device_class(class));
        }
        if let Some(class) = text(object, "state_class", "stat_cla") {
            bump(
                &mut self.state_classes,
                match class {
                    "measurement" => "measurement",
                    "total" => "total",
                    "total_increasing" => "total_increasing",
                    _ => "other",
                },
            );
        }
        if let Some(unit) = text(object, "unit_of_measurement", "unit_of_meas") {
            let parsed = Unit::parse(unit);
            bump(
                &mut self.units,
                parsed.as_ref().map_or("custom", crate::safe_unit),
            );
        }
        for (long, short) in [
            ("value_template", "val_tpl"),
            ("availability_template", "avty_tpl"),
            ("json_attributes_template", "json_attr_tpl"),
            ("last_reset_value_template", "lr_val_tpl"),
        ] {
            if let Some(template) = text(object, long, short) {
                bump(&mut self.templates, template_kind(template));
            }
        }
        if let Some(topic) = text(object, "state_topic", "stat_t") {
            self.topic(topic, output, false);
            if platform == "alarm_control_panel" && safe_subscription(topic) {
                self.alarm.watch(topic);
            }
        }
        if text(object, "json_attributes_topic", "json_attr_t").is_some() {
            self.json_attributes += 1;
            self.attributes_topics += 1;
            if let Some(topic) = text(object, "json_attributes_topic", "json_attr_t") {
                self.topic(topic, output, false);
            }
        }
        if let Some(topic) = text(object, "availability_topic", "avty_t") {
            self.topic(topic, output, true);
        }
        if let Some(mode) = text(object, "availability_mode", "avty_mode") {
            bump(
                &mut self.availability_modes,
                match mode {
                    "all" => "all",
                    "any" => "any",
                    "latest" => "latest",
                    _ => "other",
                },
            );
        }
        if let Some(availability) = object.get("availability").and_then(Json::as_array) {
            for item in availability {
                if let Some(topic) = item.get("topic").and_then(Json::as_str) {
                    self.topic(topic, output, true);
                }
                for (key, default) in [
                    ("payload_available", "online"),
                    ("payload_not_available", "offline"),
                ] {
                    if item
                        .get(key)
                        .and_then(Json::as_str)
                        .is_some_and(|payload| payload != default)
                    {
                        self.custom_availability_payloads += 1;
                    }
                }
            }
        }
        if field(object, "expire_after", "exp_aft")
            .and_then(Json::as_u64)
            .is_some_and(|seconds| seconds > 0)
        {
            self.expire_after += 1;
        }
        if object.get("force_update") == Some(&Json::Bool(true)) {
            self.force_update += 1;
        }
        if object.get("suggested_display_precision").is_some() {
            self.display_precision += 1;
        }
        if text(object, "command_topic", "cmd_t").is_some() {
            self.command_topics += 1;
        }
        if text(object, "command_template", "cmd_tpl").is_some() {
            self.command_templates += 1;
        }
        if object.get("retain") == Some(&Json::Bool(true)) {
            self.retained_configs += 1;
        }
        if object
            .get("qos")
            .and_then(Json::as_u64)
            .is_some_and(|qos| qos <= 2)
        {
            self.qos_configs += 1;
        }
        if field(object, "last_reset", "last_rst").is_some() {
            self.last_reset += 1;
            if let Some(topic) = text(object, "last_reset", "last_rst") {
                self.topic(topic, output, false);
            }
        }
        if object.get("options").and_then(Json::as_array).is_some() {
            self.enum_options += 1;
        }
        if object.get("encoding").and_then(Json::as_str) == Some("") {
            self.binary_encoding += 1;
        }
        if object.get("entity_category").and_then(Json::as_str) == Some("diagnostic") {
            self.diagnostic += 1;
        }
    }

    fn state_payload(&mut self, payload: &[u8]) {
        self.state_payloads += 1;
        classify(payload, &mut self.payload_forms);
    }

    fn other_payload(&mut self, payload: &[u8]) {
        self.other_payloads += 1;
        let form = classify(payload, &mut self.other_payload_forms);
        if matches!(form, "json_string" | "other_text") {
            let bucket = match payload.len() {
                0..=8 => "<=8",
                9..=16 => "9-16",
                17..=64 => "17-64",
                65..=256 => "65-256",
                257..=1024 => "257-1024",
                1025..=4000 => "1025-4000",
                _ => ">4000",
            };
            bump(&mut self.other_text_lengths, bucket);
        }
    }

    pub fn inspect(&mut self, topic: &str, payload: &[u8]) -> Vec<String> {
        let family = family_of(topic);
        bump(&mut self.families, family);
        if family == "homeassistant" {
            bump(
                &mut self.ha_domains,
                ha_domain(topic.split('/').nth(1).unwrap_or_default()),
            );
            bump(
                &mut self.ha_suffixes,
                ha_suffix(topic.rsplit('/').next().unwrap_or_default()),
            );
            *self
                .ha_depths
                .entry(u8::try_from(topic.split('/').count()).unwrap_or(u8::MAX))
                .or_default() += 1;
            bump(
                &mut self.ha_second_shapes,
                shape_of(topic.split('/').nth(1).unwrap_or_default()),
            );
        }
        if topic == "homeassistant/status" {
            match payload {
                b"online" => self.home_online += 1,
                b"offline" => self.home_offline += 1,
                _ => {}
            }
            return Vec::new();
        }
        if self.state_topics.contains_key(topic) || self.availability_topics.contains(topic) {
            self.state_payload(payload);
            self.alarm.observe_state(topic, payload);
            return Vec::new();
        }
        let Some(rest) = topic.strip_prefix("homeassistant/") else {
            self.other_payload(payload);
            return Vec::new();
        };
        let Some(path) = rest.strip_suffix("/config") else {
            self.other_payload(payload);
            return Vec::new();
        };
        let parts: Vec<_> = path.split('/').collect();
        if !(2..=3).contains(&parts.len()) {
            self.invalid += 1;
            return Vec::new();
        }
        let platform = parts[0];
        if component(platform) == "other" {
            self.unknown_components += 1;
            return Vec::new();
        }
        self.configs += 1;
        if payload.is_empty() {
            self.tombstones += 1;
            return Vec::new();
        }
        let Ok(Json::Object(object)) = serde_json::from_slice::<Json>(payload) else {
            self.invalid += 1;
            return Vec::new();
        };
        if object.get("migrate_discovery").and_then(Json::as_bool) == Some(true) {
            self.migrations += 1;
            return Vec::new();
        }
        let mut output = Vec::new();
        if platform == "device" {
            let inherited = field(&object, "device", "dev").and_then(Json::as_object);
            let Some(components) = object
                .get("components")
                .or_else(|| object.get("cmps"))
                .and_then(Json::as_object)
            else {
                self.invalid += 1;
                return output;
            };
            for (_, entity) in components {
                let Some(entity) = entity.as_object() else {
                    self.invalid += 1;
                    continue;
                };
                let Some(platform) = entity.get("p").and_then(Json::as_str) else {
                    self.invalid += 1;
                    continue;
                };
                self.entity(entity, platform, inherited, &mut output);
            }
        } else {
            self.entity(&object, platform, None, &mut output);
        }
        self.shared_state_topics = self
            .state_topics
            .values()
            .filter(|count| **count > 1)
            .count() as u64;
        output
    }

    pub fn public(&self) -> Json {
        let mut public = json!({
            "configs": self.configs,
            "entities": self.entities,
            "tombstones": self.tombstones,
            "migrations": self.migrations,
            "invalid": self.invalid,
            "unknown_components": self.unknown_components,
            "missing_unique_id": self.missing_unique_id,
            "missing_device_identity": self.missing_device_identity,
            "disabled": self.disabled,
            "components": self.components,
            "device_classes": self.device_classes,
            "state_classes": self.state_classes,
            "units": self.units,
            "templates": self.templates,
            "availability_modes": self.availability_modes,
            "availability_topics": self.availability_topics.len(),
            "state_topics": self.state_topics.len(),
            "shared_state_topics": self.shared_state_topics,
            "attributes_topics": self.attributes_topics,
            "command_topics": self.command_topics,
            "command_templates": self.command_templates,
            "retained_configs": self.retained_configs,
            "qos_configs": self.qos_configs,
            "expire_after": self.expire_after,
            "force_update": self.force_update,
            "display_precision": self.display_precision,
            "json_attributes": self.json_attributes,
            "last_reset": self.last_reset,
            "enum_options": self.enum_options,
            "binary_encoding": self.binary_encoding,
            "diagnostic": self.diagnostic,
            "custom_availability_payloads": self.custom_availability_payloads,
            "unsafe_topics": self.unsafe_topics,
            "home_online": self.home_online,
            "home_offline": self.home_offline,
            "state_payloads": self.state_payloads,
            "payload_forms": self.payload_forms,
            "alarm": self.alarm.public(),
            "privacy": "configuration_and_payload_values_are_not_exported",
        });
        let extra = json!({
            "devices": self.device_groups.len(),
            "max_entries_per_device": self.device_groups.values().max().copied().unwrap_or(0),
            "families": self.families,
            "ha_domains": self.ha_domains,
            "ha_suffixes": self.ha_suffixes,
            "ha_depths": self.ha_depths,
            "ha_second_shapes": self.ha_second_shapes,
            "other_payloads": self.other_payloads,
            "other_payload_forms": self.other_payload_forms,
            "other_text_lengths": self.other_text_lengths,
        });
        public
            .as_object_mut()
            .expect("discovery report is an object")
            .extend(
                extra
                    .as_object()
                    .expect("extra report is an object")
                    .clone(),
            );
        public
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensor_aliases_have_typed_metadata_without_command_subscription() {
        let mut discovery = Discovery::default();
        let payload = json!({
            "uniq_id": "NE_DOIT_PAS_SORTIR",
            "stat_t": "synthetic/state",
            "dev_cla": "temperature",
            "stat_cla": "measurement",
            "unit_of_meas": "°C",
            "val_tpl": "{{ value_json.temperature }}",
            "avty_t": "synthetic/availability",
            "avty_mode": "latest",
            "exp_aft": 60,
            "force_update": true,
            "cmd_t": "synthetic/set",
            "json_attr_t": "synthetic/attributes",
            "suggested_display_precision": 2,
            "entity_category": "diagnostic",
            "dev": {"ids": ["NE_DOIT_PAS_SORTIR"]}
        });
        let topics = discovery.inspect(
            "homeassistant/sensor/node/object/config",
            payload.to_string().as_bytes(),
        );
        assert_eq!(topics.len(), 3);
        assert!(!topics.contains(&"synthetic/set".to_owned()));
        let public = discovery.public();
        assert_eq!(public["components"]["sensor"], 1);
        assert_eq!(public["device_classes"]["temperature"], 1);
        assert_eq!(public["state_classes"]["measurement"], 1);
        assert_eq!(public["units"]["°C"], 1);
        assert_eq!(public["templates"]["value_json_path"], 1);
        assert_eq!(public["expire_after"], 1);
        assert_eq!(public["force_update"], 1);
        assert_eq!(public["command_topics"], 1);
        assert_eq!(public["diagnostic"], 1);
        assert!(!public.to_string().contains("NE_DOIT_PAS_SORTIR"));
    }

    #[test]
    fn device_discovery_counts_components_and_empty_tombstone() {
        let mut discovery = Discovery::default();
        let payload = json!({
            "dev": {"ids": ["synthetic-device"]},
            "cmps": {
                "one": {"p": "sensor", "uniq_id": "one", "stat_t": "synthetic/number"},
                "two": {"p": "binary_sensor", "uniq_id": "two", "stat_t": "synthetic/contact"}
            }
        });
        let topics = discovery.inspect(
            "homeassistant/device/object/config",
            payload.to_string().as_bytes(),
        );
        assert_eq!(topics.len(), 2);
        assert_eq!(discovery.public()["entities"], 2);
        discovery.inspect("homeassistant/device/object/config", b"");
        assert_eq!(discovery.public()["tombstones"], 1);
    }

    #[test]
    fn unsupported_templates_and_missing_identity_are_explicit() {
        let mut discovery = Discovery::default();
        discovery.inspect(
            "homeassistant/sensor/object/config",
            br#"{"stat_t":"synthetic/state","val_tpl":"{{ as_datetime(value) }}"}"#,
        );
        let public = discovery.public();
        assert_eq!(public["templates"]["jinja_unsupported"], 1);
        assert_eq!(public["missing_unique_id"], 1);
        assert!(public["missing_device_identity"].as_u64().unwrap() >= 1);
    }

    #[test]
    fn payload_shapes_are_counted_without_values() {
        let mut discovery = Discovery::default();
        discovery.inspect(
            "homeassistant/sensor/object/config",
            br#"{"uniq_id":"one","stat_t":"synthetic/state"}"#,
        );
        for payload in [
            b"12.5".as_slice(),
            b"+12",
            b"ON",
            b"{\"secret\":42}",
            b"null",
            b"text",
            &[0xff],
        ] {
            discovery.inspect("synthetic/state", payload);
        }
        let public = discovery.public().to_string();
        assert!(public.contains("json_number"));
        assert!(public.contains("number_text"));
        assert!(public.contains("boolean_word"));
        assert!(public.contains("json_object"));
        assert!(public.contains("non_utf8"));
        assert!(!public.contains("secret"));
        assert!(!public.contains("12.5"));
    }

    #[test]
    fn shared_topics_do_not_become_shared_identities() {
        let mut discovery = Discovery::default();
        for id in ["one", "two"] {
            let payload = json!({"uniq_id": id, "stat_t": "synthetic/shared"});
            discovery.inspect(
                "homeassistant/sensor/object/config",
                payload.to_string().as_bytes(),
            );
        }
        assert_eq!(discovery.public()["entities"], 2);
        assert_eq!(discovery.public()["shared_state_topics"], 1);
    }

    #[test]
    fn availability_lists_and_migration_have_separate_contracts() {
        let mut discovery = Discovery::default();
        let payload = json!({
            "uniq_id": "one",
            "stat_t": "synthetic/state",
            "availability": [
                {"topic": "synthetic/a", "payload_available": "READY"},
                {"topic": "synthetic/b"}
            ],
            "avty_mode": "any"
        });
        let topics = discovery.inspect(
            "homeassistant/sensor/object/config",
            payload.to_string().as_bytes(),
        );
        assert_eq!(topics.len(), 3);
        assert_eq!(discovery.public()["availability_modes"]["any"], 1);
        assert_eq!(discovery.public()["custom_availability_payloads"], 1);
        discovery.inspect(
            "homeassistant/sensor/object/config",
            br#"{"migrate_discovery":true}"#,
        );
        assert_eq!(discovery.public()["migrations"], 1);
    }

    #[test]
    fn subscriptions_refuse_commands_and_wildcards() {
        assert!(!safe_subscription("synthetic/set"));
        assert!(!safe_subscription("synthetic/+/state"));
        assert!(!safe_subscription("synthetic/#"));
        assert!(safe_subscription("synthetic/state"));
    }

    #[test]
    fn device_groups_count_distinct_identities_without_exporting_them() {
        let mut discovery = Discovery::default();
        for platform in ["sensor", "switch"] {
            let payload = json!({
                "uniq_id": format!("{platform}-private"),
                "stat_t": "synthetic/state",
                "dev": {"ids": ["PRIVATE-DEVICE-ONE"]}
            });
            discovery.inspect(
                &format!("homeassistant/{platform}/object/config"),
                payload.to_string().as_bytes(),
            );
        }
        let payload = json!({
            "uniq_id": "other-private",
            "stat_t": "synthetic/state",
            "dev": {"cns": [["mac", "PRIVATE-CONNECTION"]]}
        });
        discovery.inspect(
            "homeassistant/sensor/other/config",
            payload.to_string().as_bytes(),
        );
        let public = discovery.public();
        assert_eq!(public["devices"], 2);
        assert_eq!(public["max_entries_per_device"], 2);
        assert_eq!(public["entities"], 3);
        let text = public.to_string();
        assert!(!text.contains("PRIVATE-DEVICE-ONE"));
        assert!(!text.contains("PRIVATE-CONNECTION"));
        assert!(!text.contains("private"));
    }

    #[test]
    fn device_style_config_inherits_identity_and_groups_once_per_entity() {
        let mut discovery = Discovery::default();
        let payload = json!({
            "dev": {"ids": ["PRIVATE-GROUP"]},
            "cmps": {
                "one": {"p": "sensor", "uniq_id": "one", "stat_t": "synthetic/one"},
                "two": {"p": "switch", "uniq_id": "two", "stat_t": "synthetic/two"}
            }
        });
        discovery.inspect(
            "homeassistant/device/object/config",
            payload.to_string().as_bytes(),
        );
        let public = discovery.public();
        assert_eq!(public["devices"], 1);
        assert_eq!(public["max_entries_per_device"], 2);
        assert_eq!(public["missing_device_identity"], 0);
        assert!(!public.to_string().contains("PRIVATE-GROUP"));
    }

    #[test]
    fn families_and_other_payload_shapes_cover_the_whole_broker() {
        let mut discovery = Discovery::default();
        discovery.inspect("zigbee2mqtt/synthetic", br#"{"x":1}"#);
        discovery.inspect("frigate/synthetic", b"+12");
        discovery.inspect("tele/synthetic", b"ON");
        discovery.inspect("unknown-root/synthetic", b"text");
        let public = discovery.public();
        assert_eq!(public["families"]["zigbee2mqtt"], 1);
        assert_eq!(public["families"]["frigate"], 1);
        assert_eq!(public["families"]["tele"], 1);
        assert_eq!(public["families"]["other"], 1);
        assert_eq!(public["other_payloads"], 4);
        assert_eq!(public["other_payload_forms"]["json_object"], 1);
        assert_eq!(public["other_payload_forms"]["number_text"], 1);
        assert_eq!(public["other_payload_forms"]["boolean_word"], 1);
        assert_eq!(public["state_payloads"], 0);
    }

    #[test]
    fn homeassistant_traffic_is_split_by_domain_and_suffix() {
        let mut discovery = Discovery::default();
        discovery.inspect(
            "homeassistant/sensor/node/object/config",
            br#"{"uniq_id":"one","stat_t":"synthetic/state"}"#,
        );
        discovery.inspect("homeassistant/sensor/object/state", b"21.5");
        discovery.inspect("homeassistant/binary_sensor/object/state", b"ON");
        discovery.inspect("homeassistant/status", b"online");
        let public = discovery.public();
        assert_eq!(public["ha_domains"]["sensor"], 2);
        assert_eq!(public["ha_domains"]["binary_sensor"], 1);
        assert_eq!(public["ha_domains"]["status"], 1);
        assert_eq!(public["ha_suffixes"]["config"], 1);
        assert_eq!(public["ha_suffixes"]["state"], 2);
        assert_eq!(public["families"]["homeassistant"], 4);
    }

    #[test]
    fn topic_segments_are_classified_by_shape_not_value() {
        assert_eq!(shape_of("0x0000000000abcdef"), "ieee_like");
        assert_eq!(shape_of("aa:bb:cc:dd:ee:ff"), "mac_like");
        assert_eq!(shape_of("12345"), "numeric");
        assert_eq!(
            shape_of("550e8400-e29b-41d4-a716-446655440000"),
            "uuid_like"
        );
        assert_eq!(shape_of("sensor"), "word");
        assert_eq!(shape_of("a value with spaces"), "mixed");
        assert_eq!(shape_of(""), "empty");
    }

    #[test]
    fn homeassistant_depth_and_second_segment_shape_are_counted() {
        let mut discovery = Discovery::default();
        discovery.inspect("homeassistant/sensor/object/state", b"1");
        discovery.inspect("homeassistant/0x0000000000abcdef/thing/state", b"2");
        let public = discovery.public();
        assert_eq!(public["ha_depths"]["4"], 2);
        assert_eq!(public["ha_second_shapes"]["word"], 1);
        assert_eq!(public["ha_second_shapes"]["ieee_like"], 1);
        assert!(!public.to_string().contains("0000000000abcdef"));
    }
}
