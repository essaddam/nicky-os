//! Synthetic regression triggers: never read a broker or a house's data.
use super::*;
use moli_core::{InstanceId, Value};
use serde_json::{Value as Json, json};

fn code() -> String {
    format!(
        "0{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )
}

fn entry(exposes: &[Json]) -> Entry {
    let payload = json!([{"ieee_address":"fixture-panel", "friendly_name":"panel", "type":"Router",
        "definition":{"exposes":exposes}}]);
    Catalog::from_bridge_devices(
        &InstanceId::from("fixture"),
        &serde_json::to_vec(&payload).unwrap(),
    )
    .unwrap()
    .entries()
    .next()
    .unwrap()
    .clone()
}

#[test]
fn ordinary_properties_keep_their_states_and_reads() {
    let entry = entry(&[
        json!({"property":"battery","type":"numeric","access":5}),
        json!({"property":"error_code","type":"numeric","access":1}),
        json!({"property":"action","type":"enum","values":["disarm"],"access":1}),
    ]);
    let states = entry.decode(
        json!({"battery":81, "error_code":7, "action":"disarm"})
            .as_object()
            .unwrap(),
    );
    assert!(
        states
            .iter()
            .any(|(key, value)| key.as_ref() == "battery" && *value == Value::Int(81))
    );
    assert!(
        states
            .iter()
            .any(|(key, value)| key.as_ref() == "error_code" && *value == Value::Int(7))
    );
    assert_eq!(entry.read_request(), Some(json!({"battery":""})));
}

#[test]
fn credential_properties_never_become_public_points_or_read_requests() {
    let aliases = [
        "action_code",
        "actionCode",
        "ACTION_CODE",
        "action-code",
        "action_pin",
        "pin_code",
        "pinCode",
        "pincode",
        "user_code",
        "keypad_code",
        "passcode",
        "password",
        "unlock_door",
        "code",
        "pin",
        "key_code",
        "access_code",
        "alarm_code",
        "input_code",
        "settings.pin",
        "settings.code",
        "settings.key_code",
    ];
    for alias in aliases {
        let entry = entry(&[json!({"property":alias,"type":"text","access":7})]);
        let secret = code();
        let payload = json!({alias:secret});
        let public = serde_json::to_string(&entry.decode(payload.as_object().unwrap())).unwrap();
        assert!(
            !public.contains(&secret),
            "credential value reached public decode"
        );
        assert!(entry.device.points.is_empty(), "credential point declared");
        assert!(entry.read_request().is_none(), "credential read scheduled");
    }
}

#[test]
fn pin_code_composites_and_nested_credentials_are_private() {
    let entry = entry(&[
        json!({"property":"pin_code","type":"composite","features":[
        {"property":"pin_code","type":"text","access":7},
        {"property":"user","type":"numeric","access":7}]}),
        json!({"property":"action","type":"text","access":1}),
    ]);
    let secret = code();
    let payload = json!({"pin_code":{"pin_code":secret,"user":1},"action":{"action_code":secret}});
    let public = serde_json::to_string(&entry.decode(payload.as_object().unwrap())).unwrap();
    assert!(
        !public.contains(&secret),
        "nested credential reached public decode"
    );
    assert!(
        !entry
            .device
            .points
            .iter()
            .any(|p| p.key.starts_with("pin_code"))
    );
    assert!(entry.read_request().is_none());
}

#[test]
fn credential_discovery_is_independent_of_the_exposed_scalar_type() {
    for kind in ["numeric", "binary", "enum", "text"] {
        let secret = code();
        let entry =
            entry(&[json!({"property":"action_code","type":kind,"access":7,"values":[secret]})]);
        assert!(
            entry.device.points.is_empty(),
            "credential scalar type declared publicly"
        );
        assert!(entry.read_request().is_none());
    }
}

#[test]
fn flattened_and_nested_composite_paths_cannot_publish_a_credential() {
    let secret = code();
    let entry = entry(&[
        json!({"property":"settings.pin","type":"text","access":7}),
        json!({"property":"settings","type":"composite","features":[
            {"property":"code","type":"text","access":7},
            {"property":"error_code","type":"numeric","access":1},
            {"property":"date_code","type":"text","access":1}]}),
    ]);
    let payload =
        json!({"settings":{"pin":secret,"code":secret,"error_code":7,"date_code":"ordinary"}});
    let states = entry.decode(payload.as_object().unwrap());
    assert_eq!(states.len(), 2);
    assert!(entry.read_request().is_none());
    assert!(
        !serde_json::to_string(&states).unwrap().contains(&secret),
        "composite credential reached public decode"
    );
    assert!(entry.device.points.iter().all(|point| matches!(
        point.key.as_ref(),
        "settings.error_code" | "settings.date_code"
    )));
}
