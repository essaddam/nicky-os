//! Declarative command metadata only. This module never sends a command.
use serde_json::{Value as Json, json};

pub(super) fn string<'a>(data: &'a Json, long: &str, short: &str) -> Option<&'a str> {
    data[long].as_str().or_else(|| data[short].as_str())
}
pub(super) fn topic(data: &Json, long: &str, short: &str) -> Option<String> {
    let raw = string(data, long, short)?;
    let expanded = if let Some(tail) = raw.strip_prefix('~') {
        format!("{}{tail}", data["~"].as_str()?)
    } else {
        raw.into()
    };
    (!expanded.is_empty()
        && expanded.len() <= 512
        && !expanded
            .chars()
            .any(|c| c.is_control() || matches!(c, '#' | '+'))
        && !expanded.starts_with("moli-probe/"))
    .then_some(expanded)
}

pub(super) fn description(data: &Json) -> Option<(String, Json)> {
    for (long, short) in [
        ("schema", "schema"),
        ("payload_on", "pl_on"),
        ("payload_off", "pl_off"),
    ] {
        if [long, short]
            .iter()
            .any(|key| data.get(*key).is_some_and(|v| !v.is_string()))
        {
            return None;
        }
    }
    if ["brightness_scale", "bri_scl"]
        .iter()
        .any(|key| data.get(*key).is_some_and(|v| v.as_u64().is_none()))
    {
        return None;
    }
    for key in ["color_temp_kelvin", "clr_temp_k", "color_temp"] {
        if data.get(key).is_some_and(|value| !value.is_boolean()) {
            return None;
        }
    }
    for key in ["supported_color_modes", "sup_clrm"] {
        if data.get(key).is_some_and(|value| {
            !value.as_array().is_some_and(|modes| {
                modes.len() <= 16
                    && modes
                        .iter()
                        .all(|mode| mode.as_str().is_some_and(|mode| mode.len() <= 32))
            })
        }) {
            return None;
        }
    }
    let unique = string(data, "unique_id", "uniq_id")?;
    if unique.is_empty() || unique.len() > 256 {
        return None;
    }
    if [
        "command_template",
        "cmd_tpl",
        "state_command_template",
        "stat_cmd_tpl",
        "brightness_command_template",
        "bri_cmd_tpl",
        "color_temp_command_template",
        "clr_temp_cmd_tpl",
        "rgb_command_template",
        "rgb_cmd_tpl",
        "xy_command_template",
        "xy_cmd_tpl",
        "hs_command_template",
        "hs_cmd_tpl",
    ]
    .iter()
    .any(|k| !data[*k].is_null())
    {
        return None;
    }
    let command = topic(data, "command_topic", "cmd_t")?;
    let schema = string(data, "schema", "schema").unwrap_or("default");
    if !matches!(schema, "json" | "default") {
        return None;
    }
    let on = string(data, "payload_on", "pl_on").unwrap_or("ON");
    let off = string(data, "payload_off", "pl_off").unwrap_or("OFF");
    if on.is_empty() || off.is_empty() || on.len() > 64 || off.len() > 64 {
        return None;
    }
    let mut out = json!({"schema":schema,"topic":command,"on":on,"off":off,"retain":false,"parametres":{"state":{"type":"enum","valeurs":["ON","OFF"]}}});
    let brightness_topic = topic(data, "brightness_command_topic", "bri_cmd_t");
    let brightness = schema == "json"
        && data["brightness"]
            .as_bool()
            .or_else(|| data["bri"].as_bool())
            .unwrap_or(false)
        || brightness_topic.is_some();
    if brightness {
        let max = data["brightness_scale"]
            .as_u64()
            .or_else(|| data["bri_scl"].as_u64())
            .unwrap_or(255);
        if (1..=65535).contains(&max) {
            out["parametres"]["brightness"] = json!({"type":"integer","min":0,"max":max,"topic":brightness_topic.unwrap_or(command.clone())});
        }
    }
    if schema == "json" {
        let modes = data
            .get("supported_color_modes")
            .or_else(|| data.get("sup_clrm"))
            .and_then(Json::as_array);
        let colors: Vec<_> = ["xy", "hs", "rgb"]
            .into_iter()
            .filter(|mode| {
                modes.is_some_and(|modes| modes.iter().any(|value| value.as_str() == Some(mode)))
            })
            .collect();
        if !colors.is_empty() {
            out["parametres"]["color"] = json!({"type":"color","modes":colors});
        }
        let temperature = modes.is_some_and(|modes| modes.iter().any(|mode| mode == "color_temp"))
            || data["color_temp"].as_bool() == Some(true);
        let kelvin = data["color_temp_kelvin"]
            .as_bool()
            .or_else(|| data["clr_temp_k"].as_bool())
            .unwrap_or(false);
        let min = data["min_mireds"]
            .as_u64()
            .or_else(|| data["min_mirs"].as_u64());
        let max = data["max_mireds"]
            .as_u64()
            .or_else(|| data["max_mirs"].as_u64());
        if let (Some(min), Some(max)) = (min, max)
            && temperature
            && !kelvin
            && min > 0
            && max >= min
            && max <= 10000
        {
            out["parametres"]["color_temp"] = json!({"type":"integer","min":min,"max":max});
        }
    }
    Some((unique.into(), out))
}

pub(super) fn color_components(attribute: &str, value: &Json) -> Option<Vec<(&'static str, Json)>> {
    let components: &[(&str, f64)] = match attribute {
        "hs_color" => &[("color_h", 360.0), ("color_s", 100.0)],
        "xy_color" => &[("color_x", 1.0), ("color_y", 1.0)],
        "rgb_color" => &[("color_r", 255.0), ("color_g", 255.0), ("color_b", 255.0)],
        _ => return None,
    };
    if value.is_null()
        || value
            .as_str()
            .is_some_and(|value| matches!(value, "unknown" | "unavailable" | "None" | "null"))
    {
        return Some(
            components
                .iter()
                .map(|(key, _)| (*key, Json::Null))
                .collect(),
        );
    }
    let values = value.as_array()?;
    if values.len() != components.len() {
        return None;
    }
    if attribute == "xy_color" && values[0].as_f64()? + values[1].as_f64()? > 1.0 + 1e-9 {
        return None;
    }
    values
        .iter()
        .zip(components)
        .map(|(value, &(key, max))| {
            let number = value.as_f64()?;
            if !number.is_finite()
                || !(0.0..=max).contains(&number)
                || attribute == "rgb_color" && value.as_u64().is_none()
            {
                return None;
            }
            Some((key, value.clone()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn json_light_preserves_native_scale_and_declared_parameter_contract() {
        let(_,c)=description(&json!({"unique_id":"fixture-light","command_topic":"zigbee2mqtt/fixture/set","schema":"json","brightness":true,"brightness_scale":254})).unwrap();
        assert_eq!(c["parametres"]["brightness"]["max"], 254);
        assert_eq!(c["retain"], false);
        assert_eq!(c["parametres"]["state"]["valeurs"], json!(["ON", "OFF"]));
    }
    #[test]
    fn unsupported_templates_wildcards_and_self_loops_never_become_actions() {
        for topic in ["zigbee2mqtt/+/set", "moli-probe/control/set", ""] {
            assert!(description(&json!({"unique_id":"fixture","command_topic":topic})).is_none());
        }
        assert!(description(&json!({"unique_id":"fixture","command_topic":"zigbee2mqtt/fixture/set","command_template":"{{ unsupported }}"})).is_none());
    }
    #[test]
    fn compact_discovery_and_base_topic_resolve_without_using_a_name() {
        let(id,c)=description(&json!({"uniq_id":"stable","~":"zigbee2mqtt/fixture","cmd_t":"~/set","schema":"json","bri":true,"bri_scl":254})).unwrap();
        assert_eq!(id, "stable");
        assert_eq!(c["topic"], "zigbee2mqtt/fixture/set");
    }
    #[test]
    fn json_colors_require_explicit_modes_and_temperature_units() {
        let fixture = json!({"unique_id":"fixture","command_topic":"fixture/lamp/set","schema":"json","min_mireds":153,"max_mireds":500});
        let (_, base) = description(&fixture).unwrap();
        assert!(base["parametres"]["color"].is_null());
        assert!(base["parametres"]["color_temp"].is_null());
        let mut input = fixture.clone();
        input["supported_color_modes"] = json!(["xy", "hs", "rgb", "rgbw", "color_temp"]);
        let (_, contract) = description(&input).unwrap();
        assert_eq!(
            contract["parametres"]["color"],
            json!({"type":"color","modes":["xy","hs","rgb"]})
        );
        assert_eq!(
            contract["parametres"]["color_temp"],
            json!({"type":"integer","min":153,"max":500})
        );
        input["color_temp_kelvin"] = json!(true);
        assert!(description(&input).unwrap().1["parametres"]["color_temp"].is_null());
        input["schema"] = json!("default");
        assert!(description(&input).unwrap().1["parametres"]["color"].is_null());
        input = fixture;
        input["sup_clrm"] = json!(["xy", "xy", "color_temp"]);
        assert_eq!(
            description(&input).unwrap().1["parametres"]["color"]["modes"],
            json!(["xy"])
        );
        for modes in [json!(["xy", false]), json!("xy"), json!([])] {
            input["supported_color_modes"] = modes;
            if input["supported_color_modes"]
                .as_array()
                .is_some_and(Vec::is_empty)
            {
                assert!(description(&input).unwrap().1["parametres"]["color"].is_null());
            } else {
                assert!(description(&input).is_none());
            }
        }
    }

    #[test]
    fn malformed_declared_parameters_do_not_silently_become_defaults() {
        for extra in [
            json!({"schema":true}),
            json!({"brightness_scale":"254"}),
            json!({"payload_on":false}),
        ] {
            let mut data = json!({"unique_id":"fixture","command_topic":"zigbee2mqtt/fixture/set"});
            data.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            assert!(description(&data).is_none());
        }
    }
}
