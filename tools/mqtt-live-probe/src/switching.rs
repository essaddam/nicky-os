//! Bounded MQTT switch command metadata. No commands are sent here.
use crate::lighting::{string, topic};
use serde_json::{Value as Json, json};

pub(super) fn description(data: &Json) -> Option<(String, Json)> {
    for key in [
        "unique_id",
        "uniq_id",
        "command_topic",
        "cmd_t",
        "schema",
        "payload_on",
        "pl_on",
        "payload_off",
        "pl_off",
        "entity_category",
        "ent_cat",
    ] {
        if data.get(key).is_some_and(|value| !value.is_string()) {
            return None;
        }
    }
    if [
        "command_template",
        "cmd_tpl",
        "state_command_template",
        "stat_cmd_tpl",
    ]
    .iter()
    .any(|key| !data[*key].is_null())
        || matches!(
            string(data, "entity_category", "ent_cat"),
            Some("config" | "diagnostic")
        )
        || string(data, "schema", "schema").is_some_and(|schema| schema != "default")
    {
        return None;
    }
    let unique = string(data, "unique_id", "uniq_id")?;
    let on = string(data, "payload_on", "pl_on").unwrap_or("ON");
    let off = string(data, "payload_off", "pl_off").unwrap_or("OFF");
    if unique.is_empty()
        || unique.len() > 256
        || on.is_empty()
        || off.is_empty()
        || on.len() > 256
        || off.len() > 256
    {
        return None;
    }
    let command = topic(data, "command_topic", "cmd_t")?;
    Some((
        unique.into(),
        json!({"schema":"default","topic":command,"on":on,"off":off,"retain":false,"parametres":{"state":{"type":"enum","valeurs":["ON","OFF"]}}}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switches_keep_exact_native_payloads_and_compact_topics() {
        let (_,contract)=description(&json!({"uniq_id":"fixture","~":"fixture/strip","cmd_t":"~/set","pl_on":"{\"state_l1\":\"ON\"}","pl_off":"{\"state_l1\":\"OFF\"}"})).unwrap();
        assert_eq!(contract["topic"], "fixture/strip/set");
        assert_eq!(contract["on"], "{\"state_l1\":\"ON\"}");
        assert_eq!(contract["off"], "{\"state_l1\":\"OFF\"}");
        assert_eq!(contract["schema"], "default");
        assert_eq!(contract["retain"], false);
        assert_eq!(
            contract["parametres"],
            json!({"state":{"type":"enum","valeurs":["ON","OFF"]}})
        );
    }

    #[test]
    fn switch_payloads_are_bounded_in_bytes_and_never_accept_command_templates() {
        let fixture = json!({"unique_id":"fixture","command_topic":"fixture/strip/set"});
        for extra in [
            json!({"payload_on":false}),
            json!({"pl_off":12}),
            json!({"payload_on":""}),
            json!({"payload_off":"é".repeat(129)}),
            json!({"unique_id":""}),
            json!({"command_topic":"fixture/+/set"}),
            json!({"command_topic":"fixture/#"}),
            json!({"command_topic":"moli-probe/control/set"}),
            json!({"command_topic":"fixture/\nset"}),
            json!({"command_topic":""}),
            json!({"command_template":"{{ value }}"}),
            json!({"cmd_tpl":"{{ value }}"}),
            json!({"entity_category":"config"}),
            json!({"ent_cat":"diagnostic"}),
            json!({"schema":"json"}),
        ] {
            let mut input = fixture.clone();
            input
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            assert!(description(&input).is_none(), "{extra}");
        }
        for payload in ["x".repeat(256), "é".repeat(128)] {
            let mut input = fixture.clone();
            input["payload_on"] = json!(payload);
            assert!(description(&input).is_some());
        }
    }
}
