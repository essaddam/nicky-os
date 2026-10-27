use serde_json::{Map, Value as Json, json};
use std::collections::{BTreeMap, HashSet};

#[derive(Debug, Default)]
pub struct Alarm {
    configs: u64,
    local_code: u64,
    remote_numeric_code: u64,
    remote_text_code: u64,
    code_arm_required: BTreeMap<&'static str, u64>,
    code_disarm_required: BTreeMap<&'static str, u64>,
    code_trigger_required: BTreeMap<&'static str, u64>,
    command_templates: u64,
    supported_features: BTreeMap<&'static str, u64>,
    state_topics: HashSet<String>,
    state_payloads: u64,
    states: BTreeMap<&'static str, u64>,
}

fn bump(map: &mut BTreeMap<&'static str, u64>, key: &'static str) {
    *map.entry(key).or_default() += 1;
}

fn state_name(value: &str) -> &'static str {
    match value {
        "disarmed" => "disarmed",
        "armed_home" => "armed_home",
        "armed_away" => "armed_away",
        "armed_night" => "armed_night",
        "armed_vacation" => "armed_vacation",
        "armed_custom_bypass" => "armed_custom_bypass",
        "pending" => "pending",
        "triggered" => "triggered",
        "arming" => "arming",
        "disarming" => "disarming",
        "None" => "unknown",
        "" => "empty",
        _ => "other",
    }
}

impl Alarm {
    pub fn inspect(&mut self, object: &Map<String, Json>) {
        self.configs += 1;
        if let Some(code) = object.get("code") {
            match code.as_str() {
                Some("REMOTE_CODE") => self.remote_numeric_code += 1,
                Some("REMOTE_CODE_TEXT") => self.remote_text_code += 1,
                Some(_) => self.local_code += 1,
                None => self.local_code += 1,
            }
        }
        for (field, counts) in [
            ("code_arm_required", &mut self.code_arm_required),
            ("code_disarm_required", &mut self.code_disarm_required),
            ("code_trigger_required", &mut self.code_trigger_required),
        ] {
            if let Some(required) = object.get(field).and_then(Json::as_bool) {
                bump(counts, if required { "required" } else { "not_required" });
            }
        }
        if object.get("command_template").is_some() {
            self.command_templates += 1;
        }
        if let Some(features) = object.get("supported_features").and_then(Json::as_array) {
            for feature in features.iter().filter_map(Json::as_str) {
                let Some(feature) = (match feature {
                    "arm_home" => Some("arm_home"),
                    "arm_away" => Some("arm_away"),
                    "arm_night" => Some("arm_night"),
                    "arm_vacation" => Some("arm_vacation"),
                    "arm_custom_bypass" => Some("arm_custom_bypass"),
                    "trigger" => Some("trigger"),
                    _ => None,
                }) else {
                    continue;
                };
                bump(&mut self.supported_features, feature);
            }
        }
    }

    pub fn watch(&mut self, topic: &str) {
        self.state_topics.insert(topic.to_owned());
    }

    pub fn observe_state(&mut self, topic: &str, payload: &[u8]) {
        if !self.state_topics.contains(topic) {
            return;
        }
        self.state_payloads += 1;
        let Ok(text) = std::str::from_utf8(payload) else {
            bump(&mut self.states, "non_utf8");
            return;
        };
        match serde_json::from_str::<Json>(text) {
            Ok(Json::Object(object)) => {
                let state = object
                    .get("state")
                    .and_then(Json::as_str)
                    .map_or("other", state_name);
                bump(&mut self.states, state);
            }
            Ok(Json::String(state)) => bump(&mut self.states, state_name(&state)),
            Ok(Json::Null) => bump(&mut self.states, "unknown"),
            Ok(_) => bump(&mut self.states, "other"),
            Err(_) => bump(&mut self.states, state_name(text.trim())),
        }
    }

    pub fn public(&self) -> Json {
        json!({
            "configs": self.configs,
            "local_code": self.local_code,
            "remote_numeric_code": self.remote_numeric_code,
            "remote_text_code": self.remote_text_code,
            "code_arm_required": self.code_arm_required,
            "code_disarm_required": self.code_disarm_required,
            "code_trigger_required": self.code_trigger_required,
            "command_templates": self.command_templates,
            "supported_features": self.supported_features,
            "state_topics": self.state_topics.len(),
            "state_payloads": self.state_payloads,
            "states": self.states,
            "privacy": "codes_pin_topics_and_raw_payloads_are_not_exported",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alarm_metadata_and_states_never_expose_codes() {
        let marker = "NE_DOIT_PAS_SORTIR";
        let object = json!({
            "code": marker,
            "code_arm_required": false,
            "code_disarm_required": true,
            "code_trigger_required": false,
            "command_template": "{\"action\":\"{{action}}\",\"code\":\"{{code}}\"}",
            "supported_features": ["arm_home", "arm_away"]
        });
        let object = object.as_object().unwrap();
        let mut alarm = Alarm::default();
        alarm.inspect(object);
        alarm.watch("synthetic/alarm/state");
        alarm.observe_state("synthetic/alarm/state", b"disarmed");
        alarm.observe_state("synthetic/alarm/state", br#"{"state":"armed_home"}"#);
        alarm.observe_state("synthetic/alarm/state", b"None");
        let public = alarm.public();
        assert_eq!(public["configs"], 1);
        assert_eq!(public["local_code"], 1);
        assert_eq!(public["code_arm_required"]["not_required"], 1);
        assert_eq!(public["code_disarm_required"]["required"], 1);
        assert_eq!(public["code_trigger_required"]["not_required"], 1);
        assert_eq!(public["supported_features"]["arm_home"], 1);
        assert_eq!(public["supported_features"]["arm_away"], 1);
        assert_eq!(public["states"]["disarmed"], 1);
        assert_eq!(public["states"]["armed_home"], 1);
        assert_eq!(public["states"]["unknown"], 1);
        assert!(!public.to_string().contains(marker));
    }

    #[test]
    fn remote_code_modes_are_metadata_not_credentials() {
        for (code, key) in [
            ("REMOTE_CODE", "remote_numeric_code"),
            ("REMOTE_CODE_TEXT", "remote_text_code"),
        ] {
            let mut alarm = Alarm::default();
            alarm.inspect(json!({"code": code}).as_object().unwrap());
            assert_eq!(alarm.public()[key], 1);
        }
    }
}
