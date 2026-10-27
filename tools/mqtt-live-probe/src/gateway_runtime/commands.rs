//! Pure virtual-command contracts. Native routing never leaves this module's bindings.
use anyhow::{Result, bail, ensure};
use serde_json::{Map, Value as Json, json};
use std::collections::BTreeMap;

#[derive(Clone)]
pub struct Binding {
    pub source: usize,
    pub native: Json,
}

#[derive(Debug, PartialEq, Eq)]
pub struct NativePacket {
    pub topic: String,
    pub payload: Vec<u8>,
}

fn segment(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn topic(value: &Json) -> Result<&str> {
    let value = value.as_str().ok_or_else(|| anyhow::anyhow!("topic"))?;
    ensure!(
        !value.is_empty()
            && value.len() <= 65_535
            && !value
                .chars()
                .any(|c| c.is_control() || c == '+' || c == '#'),
        "topic"
    );
    Ok(value)
}

fn native_contract(native: &Json) -> Result<()> {
    ensure!(
        matches!(native["schema"].as_str(), Some("json" | "default")),
        "schema"
    );
    topic(&native["topic"])?;
    ensure!(
        native
            .get("retain")
            .is_none_or(|retain| retain == &Json::Bool(false)),
        "retain"
    );
    if let Some(parameters) = native.get("parametres") {
        let parameters = parameters
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("parameter"))?;
        for spec in parameters.values() {
            if let Some(value) = spec.get("topic") {
                topic(value)?;
            }
        }
    }
    Ok(())
}

fn scalar(value: &Json) -> bool {
    value.is_string() || value.is_boolean() || value.as_f64().is_some_and(f64::is_finite)
}

fn state_mapping(native: &Json) -> bool {
    let state = &native["parametres"]["state"];
    state["type"] == "enum"
        && state["valeurs"]
            .as_array()
            .is_some_and(|values| values.contains(&json!("ON")) && values.contains(&json!("OFF")))
        && native.get("on").is_some_and(scalar)
        && native.get("off").is_some_and(scalar)
}

fn components(mode: &str) -> Option<&'static [&'static str]> {
    match mode {
        "xy" => Some(&["x", "y"]),
        "hs" => Some(&["h", "s"]),
        "rgb" => Some(&["r", "g", "b"]),
        _ => None,
    }
}

fn feedback(channel: &Json, channels: &[Json], parameters: &mut Map<String, Json>, native: &Json) {
    if !matches!(channel["domaine"].as_str(), Some("light" | "switch")) {
        return;
    }
    if state_mapping(native)
        && let Some(state) = parameters.get_mut("state").and_then(Json::as_object_mut)
    {
        state.insert("source_channel".into(), channel["id"].clone());
        state.insert(
            "source_values".into(),
            json!([
                {"source":"on","value":"ON"},{"source":"off","value":"OFF"},
                {"source":true,"value":"ON"},{"source":false,"value":"OFF"},
                {"source":"ON","value":"ON"},{"source":"OFF","value":"OFF"}
            ]),
        );
    }
    if channel["domaine"] != "light" {
        return;
    }
    if let Some(color) = parameters.get_mut("color").and_then(Json::as_object_mut)
        && color.get("type").and_then(Json::as_str) == Some("color")
        && let Some(modes) = color.get("modes").and_then(Json::as_array)
    {
        let mut groups = Map::new();
        for mode in modes.iter().filter_map(Json::as_str) {
            let Some(keys) = components(mode) else {
                continue;
            };
            let mut group = Map::new();
            for key in keys {
                let semantic = format!("color_{key}");
                let source = format!("{}/{semantic}", channel["source"].as_str().unwrap_or(""));
                let matches: Vec<_> = channels
                    .iter()
                    .filter(|c| {
                        c["source"] == source
                            && c["semantique"] == semantic
                            && c["domaine"] == "light"
                    })
                    .collect();
                if matches.len() == 1 {
                    group.insert((*key).into(), matches[0]["id"].clone());
                }
            }
            if group.len() == keys.len() {
                groups.insert(mode.into(), Json::Object(group));
            }
        }
        color.insert("source_channels".into(), Json::Object(groups));
    }
    if let Some(source) = channel["source"].as_str()
        && let Some(brightness) = parameters
            .get_mut("brightness")
            .and_then(Json::as_object_mut)
    {
        let expected = format!("{source}/brightness");
        let matches: Vec<_> = channels
            .iter()
            .filter(|c| c["source"] == expected)
            .collect();
        if matches.len() == 1 {
            brightness.insert("source_channel".into(), matches[0]["id"].clone());
            if matches[0]["unite"] == "/255" && matches[0]["semantique"] == "intensite" {
                brightness.insert("source_scale".into(), json!({"min":0,"max":255}));
            }
        }
    }
}

/// Returns only bindings for this canonical bridge; caller replaces its prior prefix atomically.
pub fn expose_description(
    description: &mut Json,
    source_count: usize,
) -> Result<BTreeMap<String, Binding>> {
    let source_id = description["source_id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("source"))?;
    let source = source_id
        .strip_prefix("z2m-")
        .and_then(|s| s.parse::<usize>().ok())
        .ok_or_else(|| anyhow::anyhow!("source"))?;
    ensure!(
        source < source_count && source_id == format!("z2m-{source}"),
        "source"
    );
    let root = description["pont"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("namespace"))?;
    ensure!(
        root.strip_prefix("moli-probe/maison/")
            .is_some_and(|tail| tail.split('/').all(segment))
            && root.len() < 65_000,
        "namespace"
    );
    let prefix = format!("{root}/canal/");
    let Some(raw_channels) = description.get("canaux") else {
        return Ok(BTreeMap::new());
    };
    let channels: Vec<Json> = raw_channels
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("channels"))?
        .values()
        .cloned()
        .collect();
    let mut replacement = raw_channels.clone();
    let mut bindings = BTreeMap::new();
    for channel in replacement
        .as_object_mut()
        .expect("checked channels")
        .values_mut()
    {
        let native = channel["commande"].clone();
        if !matches!(native["schema"].as_str(), Some("json" | "default")) {
            continue;
        }
        native_contract(&native)?;
        let id = channel["id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("channel"))?;
        ensure!(segment(id), "channel");
        let virtual_topic = format!("{prefix}{id}/set");
        topic(&json!(virtual_topic))?;
        let mut parameters = native
            .get("parametres")
            .and_then(Json::as_object)
            .cloned()
            .unwrap_or_default();
        for spec in parameters.values_mut().filter_map(Json::as_object_mut) {
            spec.remove("topic");
        }
        feedback(channel, &channels, &mut parameters, &native);
        let mut command = json!({"schema":"virtual","topic":virtual_topic,"retain":false,"parametres":parameters});
        if state_mapping(&native) {
            command["on"] = json!("ON");
            command["off"] = json!("OFF");
        }
        channel["commande"] = command;
        ensure!(
            bindings
                .insert(virtual_topic, Binding { source, native })
                .is_none(),
            "channel"
        );
    }
    description["canaux"] = replacement;
    Ok(bindings)
}

fn number(value: &Json) -> Result<f64> {
    value
        .as_f64()
        .filter(|n| n.is_finite())
        .ok_or_else(|| anyhow::anyhow!("number"))
}

fn validate_parameter(spec: &Json, value: &Json) -> Result<()> {
    ensure!(spec.is_object(), "parameter");
    match spec["type"].as_str() {
        Some("color") => {
            let modes = spec["modes"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("color"))?;
            ensure!(!modes.is_empty() && modes.len() <= 3, "color");
            for (index, mode) in modes.iter().enumerate() {
                ensure!(
                    mode.as_str().and_then(components).is_some() && !modes[..index].contains(mode),
                    "color"
                );
            }
            let values = value.as_object().ok_or_else(|| anyhow::anyhow!("color"))?;
            let mode = modes
                .iter()
                .filter_map(Json::as_str)
                .find(|mode| {
                    components(mode).is_some_and(|keys| {
                        keys.len() == values.len()
                            && keys.iter().all(|key| values.contains_key(*key))
                    })
                })
                .ok_or_else(|| anyhow::anyhow!("color"))?;
            for value in values.values() {
                number(value)?;
            }
            match mode {
                "xy" => {
                    let x = number(&value["x"])?;
                    let y = number(&value["y"])?;
                    ensure!(
                        (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y) && x + y <= 1.0 + 1e-9,
                        "color"
                    );
                }
                "hs" => ensure!(
                    (0.0..=360.0).contains(&number(&value["h"])?)
                        && (0.0..=100.0).contains(&number(&value["s"])?),
                    "color"
                ),
                "rgb" => ensure!(
                    values.values().all(|v| v
                        .as_f64()
                        .is_some_and(|n| n.fract() == 0.0 && (0.0..=255.0).contains(&n))),
                    "color"
                ),
                _ => bail!("color"),
            }
        }
        Some("enum") => {
            let values = spec["valeurs"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("enum"))?;
            ensure!(
                !values.is_empty()
                    && values.len() <= 128
                    && values.iter().all(scalar)
                    && values.iter().any(|v| v == value
                        || (v.is_number() && value.is_number() && v.as_f64() == value.as_f64())),
                "enum"
            );
        }
        Some("boolean") => ensure!(value.is_boolean(), "boolean"),
        Some("string" | "text") => {
            let max = number(&spec["maxLength"])?;
            let min = spec
                .get("minLength")
                .map(number)
                .transpose()?
                .unwrap_or(0.0);
            ensure!(
                max.fract() == 0.0
                    && (1.0..=256.0).contains(&max)
                    && min.fract() == 0.0
                    && min >= 0.0
                    && min <= max,
                "string"
            );
            let text = value.as_str().ok_or_else(|| anyhow::anyhow!("string"))?;
            let length = text.encode_utf16().count() as f64;
            ensure!(length >= min && length <= max, "string");
        }
        Some(kind @ ("integer" | "number")) => {
            let value = number(value)?;
            let min = number(&spec["min"])?;
            let max = number(&spec["max"])?;
            ensure!(
                min <= max
                    && value >= min
                    && value <= max
                    && (kind != "integer" || value.fract() == 0.0),
                "range"
            );
            if let Some(step) = spec.get("step") {
                let step = number(step)?;
                ensure!(step > 0.0, "step");
                let steps = (value - min) / step;
                ensure!(
                    steps.is_finite() && (steps - steps.round()).abs() <= 1e-6,
                    "step"
                );
            }
        }
        _ => bail!("type"),
    }
    Ok(())
}

fn state_value(native: &Json, value: &Json) -> Result<Json> {
    if state_mapping(native) && matches!(value.as_str(), Some("ON" | "OFF")) {
        return Ok(native[if value == "ON" { "on" } else { "off" }].clone());
    }
    ensure!(native["schema"] == "json", "state mapping");
    Ok(value.clone())
}

pub fn native_packets(binding: &Binding, body: &Json) -> Result<Vec<NativePacket>> {
    let values = body.as_object().ok_or_else(|| anyhow::anyhow!("payload"))?;
    ensure!(
        !values.is_empty() && serde_json::to_vec(body)?.len() <= 256,
        "payload"
    );
    let native = &binding.native;
    native_contract(native)?;
    let parameters = native["parametres"]
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("parameter"))?;
    for (key, value) in values {
        let spec = parameters
            .get(key)
            .ok_or_else(|| anyhow::anyhow!("parameter"))?;
        ensure!(
            spec["type"] != "color" || native["schema"] == "json",
            "color schema"
        );
        validate_parameter(spec, value)?;
    }
    if native["schema"] == "json" {
        let mut payload = body.clone();
        if let Some(value) = values.get("state") {
            payload["state"] = state_value(native, value)?;
        }
        let payload = serde_json::to_vec(&payload)?;
        ensure!(payload.len() <= 256, "payload");
        return Ok(vec![NativePacket {
            topic: topic(&native["topic"])?.into(),
            payload,
        }]);
    }
    values
        .iter()
        .map(|(key, value)| {
            let value = if key == "state" {
                state_value(native, value)?
            } else {
                value.clone()
            };
            let payload = value
                .as_str()
                .map_or_else(|| value.to_string(), str::to_owned)
                .into_bytes();
            ensure!(payload.len() <= 256, "payload");
            let target = parameters[key].get("topic").unwrap_or(&native["topic"]);
            Ok(NativePacket {
                topic: topic(target)?.into(),
                payload,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn native() -> Json {
        json!({"schema":"json","topic":"fixture/set","on":"ON","off":"OFF","parametres":{
            "state":{"type":"enum","valeurs":["ON","OFF"]},"brightness":{"type":"integer","min":0,"max":254}
        }})
    }
    fn description(command: Json) -> Json {
        json!({"source_id":"z2m-1","pont":"moli-probe/maison/fixture","canaux":{
            "light":{"id":"light","domaine":"light","source":"fixture/light","commande":command},
            "brightness":{"id":"brightness","source":"fixture/light/brightness"},
            "other":{"id":"other","source":"fixture/other/brightness"}
        }})
    }
    fn packets(command: Json, body: Json) -> Result<Vec<NativePacket>> {
        native_packets(
            &Binding {
                source: 0,
                native: command,
            },
            &body,
        )
    }
    fn parameter(spec: Json, value: Json) -> Result<Vec<NativePacket>> {
        packets(
            json!({"schema":"json","topic":"fixture/set","parametres":{"parameter":spec}}),
            json!({"parameter":value}),
        )
    }
    fn payload(command: Json, body: Json) -> Json {
        serde_json::from_slice(&packets(command, body).unwrap()[0].payload).unwrap()
    }

    #[test]
    fn public_commands_hide_native_routing() {
        let mut d = description(native());
        let bindings = expose_description(&mut d, 2).unwrap();
        let c = &d["canaux"]["light"]["commande"];
        assert_eq!(c["schema"], "virtual");
        assert_eq!(c["retain"], false);
        let binding = &bindings[c["topic"].as_str().unwrap()];
        assert_eq!(binding.source, 1);
        assert_eq!(binding.native["topic"], "fixture/set");
        assert_eq!(c["on"], "ON");
        assert_eq!(c["off"], "OFF");
    }
    #[test]
    fn feedback_matches_exact_source_without_inventing_scale() {
        for ambiguous in [false, true] {
            let mut d = description(native());
            if ambiguous {
                d["canaux"]["duplicate"] =
                    json!({"id":"duplicate","source":"fixture/light/brightness"});
            }
            expose_description(&mut d, 2).unwrap();
            let p = &d["canaux"]["light"]["commande"]["parametres"];
            assert_eq!(p["state"]["source_channel"], "light");
            assert_eq!(p["state"]["source_values"].as_array().unwrap().len(), 6);
            assert_eq!(
                p["brightness"]["source_channel"],
                if ambiguous {
                    Json::Null
                } else {
                    json!("brightness")
                }
            );
            assert!(p["brightness"]["source_scale"].is_null());
        }
    }
    #[test]
    fn unknown_values_and_parameters_are_rejected() {
        for body in [
            json!({"brightness":255}),
            json!({"brightness":-1}),
            json!({"brightness":1.5}),
            json!({"brightness":"12"}),
            json!({"state":"toggle"}),
            json!({"other":true}),
            json!({}),
            json!([]),
            json!({"toString":true}),
        ] {
            assert!(packets(native(), body).is_err());
        }
        assert_eq!(
            payload(native(), json!({"state":"ON","brightness":128})),
            json!({"state":"ON","brightness":128})
        );
    }
    #[test]
    fn scalar_json_types_are_preserved() {
        for (spec, value) in [
            (json!({"type":"boolean"}), json!(false)),
            (json!({"type":"number","min":1,"max":2}), json!(1.5)),
            (
                json!({"type":"string","minLength":1,"maxLength":3}),
                json!("abc"),
            ),
            (json!({"type":"text","maxLength":3}), json!("")),
            (
                json!({"type":"enum","valeurs":[false,2,"mode"]}),
                json!(false),
            ),
        ] {
            let result = parameter(spec, value.clone()).unwrap();
            assert_eq!(
                serde_json::from_slice::<Json>(&result[0].payload).unwrap(),
                json!({"parameter":value})
            );
        }
    }
    #[test]
    fn incomplete_contracts_and_incorrect_types_fail_closed() {
        for (spec, value) in [
            (json!({"type":"boolean"}), json!("false")),
            (json!({"type":"unknown","min":0,"max":10}), json!(1)),
            (json!({"type":"number","max":10}), json!(1)),
            (json!({"type":"number","min":10,"max":0}), json!(1)),
            (json!({"type":"enum","valeurs":[]}), json!("x")),
            (json!({"type":"enum","valeurs":[null]}), Json::Null),
            (json!({"type":"string"}), json!("x")),
            (json!({"type":"string","maxLength":3}), json!("long")),
            (
                json!({"type":"string","minLength":2,"maxLength":3}),
                json!("x"),
            ),
            (
                json!({"type":"string","maxLength":3,"minLength":-1}),
                json!("x"),
            ),
            (
                json!({"type":"string","maxLength":3,"minLength":"1"}),
                json!("x"),
            ),
        ] {
            assert!(parameter(spec, value).is_err());
        }
    }
    #[test]
    fn numeric_steps_start_at_minimum_and_allow_roundoff() {
        assert!(
            parameter(
                json!({"type":"number","min":10,"max":20,"step":0.5}),
                json!(10.5)
            )
            .is_ok()
        );
        assert!(
            parameter(
                json!({"type":"number","min":0,"max":1,"step":0.1}),
                json!(0.30000000000000004)
            )
            .is_ok()
        );
        assert!(
            parameter(
                json!({"type":"number","min":10,"max":20,"step":0.5}),
                json!(10.3)
            )
            .is_err()
        );
        for step in [json!(0), json!(-1), json!("1"), Json::Null] {
            assert!(
                parameter(
                    json!({"type":"number","min":0,"max":10,"step":step}),
                    json!(1)
                )
                .is_err()
            );
        }
    }
    #[test]
    fn payload_limit_counts_utf8_before_and_after_mapping() {
        assert!(
            parameter(
                json!({"type":"string","maxLength":256}),
                json!("é".repeat(100))
            )
            .is_ok()
        );
        assert!(
            parameter(
                json!({"type":"string","maxLength":256}),
                json!("é".repeat(125))
            )
            .is_err()
        );
        for schema in ["json", "default"] {
            let mut command = native();
            command["schema"] = json!(schema);
            command["on"] = json!("é".repeat(130));
            assert!(packets(command, json!({"state":"ON"})).is_err());
        }
        assert!(parameter(json!({"type":"string","maxLength":1}), json!("😀")).is_err());
    }
    #[test]
    fn unmapped_enums_are_preserved_without_false_off_conversion() {
        let modes = json!({"schema":"json","topic":"fixture/set","parametres":{"state":{"type":"enum","valeurs":["heat","off"]}}});
        assert_eq!(
            payload(modes.clone(), json!({"state":"heat"})),
            json!({"state":"heat"})
        );
        let mut d = description(modes);
        expose_description(&mut d, 2).unwrap();
        assert!(d["canaux"]["light"]["commande"]["on"].is_null());
        assert!(d["canaux"]["light"]["commande"]["parametres"]["state"]["source_values"].is_null());
        let mut command = native();
        command["on"] = json!(true);
        command["off"] = json!(false);
        assert_eq!(
            payload(command, json!({"state":"OFF"})),
            json!({"state":false})
        );
    }
    #[test]
    fn default_schema_requires_mapping_and_keeps_parameter_topics_private() {
        let mut command = native();
        command["schema"] = json!("default");
        command["on"] = json!("enable");
        command["off"] = json!("disable");
        command["parametres"]["brightness"]["topic"] = json!("fixture/brightness/set");
        let packets = packets(command.clone(), json!({"state":"ON","brightness":12})).unwrap();
        assert!(packets.contains(&NativePacket {
            topic: "fixture/set".into(),
            payload: b"enable".to_vec()
        }));
        assert!(packets.contains(&NativePacket {
            topic: "fixture/brightness/set".into(),
            payload: b"12".to_vec()
        }));
        let mut d = description(command.clone());
        expose_description(&mut d, 2).unwrap();
        assert!(d["canaux"]["light"]["commande"]["parametres"]["brightness"]["topic"].is_null());
        command.as_object_mut().unwrap().remove("off");
        assert!(
            super::native_packets(
                &Binding {
                    source: 0,
                    native: command
                },
                &json!({"state":"ON"})
            )
            .is_err()
        );
    }
    #[test]
    fn brightness_scale_requires_exact_explicit_metadata() {
        for mutation in 0..6 {
            let mut d = description(native());
            d["canaux"]["brightness"]["unite"] = json!("/255");
            d["canaux"]["brightness"]["semantique"] = json!("intensite");
            match mutation {
                1 => d["canaux"]["brightness"]["unite"] = Json::Null,
                2 => d["canaux"]["brightness"]["semantique"] = json!("autre"),
                3 => d["canaux"]["light"]["domaine"] = json!("cover"),
                4 => d["canaux"]["brightness"]["source"] = json!("fixture/other/brightness"),
                5 => d["canaux"]["duplicate"] = d["canaux"]["brightness"].clone(),
                _ => {}
            }
            expose_description(&mut d, 2).unwrap();
            assert_eq!(
                d["canaux"]["light"]["commande"]["parametres"]["brightness"]["source_scale"],
                if mutation == 0 {
                    json!({"min":0,"max":255})
                } else {
                    Json::Null
                }
            );
            assert_eq!(
                d["canaux"]["light"]["commande"]["parametres"]["brightness"]["max"],
                254
            );
        }
    }
    #[test]
    fn independent_outlets_keep_exact_routing_and_feedback() {
        let mut d = description(native());
        d["canaux"] = json!({});
        for id in ["one", "two"] {
            let mut command = native();
            command["schema"] = json!("default");
            command["topic"] = json!(format!("fixture/{id}/set"));
            command["off"] = json!(json!({id:"OFF"}).to_string());
            d["canaux"][id] = json!({"id":id,"domaine":"switch","commande":command});
        }
        let bindings = expose_description(&mut d, 2).unwrap();
        for id in ["one", "two"] {
            let command = &d["canaux"][id]["commande"];
            assert_eq!(command["parametres"]["state"]["source_channel"], id);
            let packet = native_packets(
                &bindings[command["topic"].as_str().unwrap()],
                &json!({"state":"OFF"}),
            )
            .unwrap()
            .remove(0);
            assert_eq!(packet.topic, format!("fixture/{id}/set"));
            assert_eq!(
                serde_json::from_slice::<Json>(&packet.payload).unwrap(),
                json!({id:"OFF"})
            );
        }
    }
    #[test]
    fn colors_require_exact_declared_model_and_bounded_coordinates() {
        let spec = json!({"type":"color","modes":["xy","hs","rgb"]});
        for value in [
            json!({"x":0.4,"y":0.3}),
            json!({"h":360,"s":100}),
            json!({"r":255,"g":0,"b":128}),
        ] {
            assert!(parameter(spec.clone(), value).is_ok());
        }
        for value in [
            Json::Null,
            json!([]),
            json!({}),
            json!({"x":0.2}),
            json!({"x":0.8,"y":0.8}),
            json!({"x":-1,"y":0.2}),
            json!({"x":0.2,"y":"0.3"}),
            json!({"h":361,"s":0}),
            json!({"h":20,"s":101}),
            json!({"r":1.5,"g":0,"b":0}),
            json!({"r":256,"g":0,"b":0}),
            json!({"x":0.2,"y":0.3,"r":12}),
            json!({"r":1,"g":2,"b":3,"w":4}),
        ] {
            assert!(parameter(spec.clone(), value).is_err());
        }
        for modes in [
            Json::Null,
            json!([]),
            json!(["unknown"]),
            json!(["xy", "xy"]),
        ] {
            assert!(
                parameter(
                    json!({"type":"color","modes":modes}),
                    json!({"x":0.2,"y":0.3})
                )
                .is_err()
            );
        }
        assert!(
            packets(
                json!({"schema":"default","topic":"fixture/set","parametres":{"color":spec}}),
                json!({"color":{"x":0.2,"y":0.3}})
            )
            .is_err()
        );
    }
    #[test]
    fn color_feedback_requires_unambiguous_source_semantic_and_domain() {
        for mutation in 0..4 {
            let mut d = description(native());
            d["canaux"]["light"]["commande"]["parametres"]["color"] =
                json!({"type":"color","modes":["xy"]});
            for component in ["x", "y"] {
                d["canaux"][component] = json!({"id":component,"source":format!("fixture/light/color_{component}"),"semantique":format!("color_{component}"),"domaine":"light"});
            }
            match mutation {
                1 => d["canaux"]["duplicate"] = d["canaux"]["x"].clone(),
                2 => d["canaux"]["x"]["domaine"] = json!("sensor"),
                3 => d["canaux"]["x"]["semantique"] = json!("other"),
                _ => {}
            }
            expose_description(&mut d, 2).unwrap();
            assert_eq!(
                d["canaux"]["light"]["commande"]["parametres"]["color"]["source_channels"],
                if mutation == 0 {
                    json!({"xy":{"x":"x","y":"y"}})
                } else {
                    json!({})
                }
            );
        }
    }
    #[test]
    fn routing_validation_is_canonical_and_atomic() {
        for source in ["1", "z2m-01", "z2m-+1", "z2m-2", "z2m--1"] {
            let mut d = description(native());
            d["source_id"] = json!(source);
            assert!(expose_description(&mut d, 2).is_err());
        }
        for root in [
            "moli-probe/maison/",
            "moli-probe/maison/../fixture",
            "moli-probe/maison/fixture/",
            "moli-probe/maison/fixture/#",
            "other/fixture",
        ] {
            let mut d = description(native());
            d["pont"] = json!(root);
            assert!(expose_description(&mut d, 2).is_err());
        }
        for target in ["", "fixture/#", "fixture/+", "fixture/\u{0}"] {
            let mut command = native();
            command["topic"] = json!(target);
            let mut d = description(command.clone());
            let original = d.clone();
            assert!(expose_description(&mut d, 2).is_err());
            assert_eq!(d, original);
            assert!(packets(command, json!({"state":"ON"})).is_err());
        }
        let mut command = native();
        command["retain"] = json!(true);
        assert!(packets(command, json!({"state":"ON"})).is_err());
    }
}
