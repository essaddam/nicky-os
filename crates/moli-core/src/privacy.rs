//! Credential namespaces shared by discovery, publication and persisted-data repair.
//! These are private inputs, never public points. Ordinary diagnostic `error_code`
//! and `date_code` remain public. Compare decoded property names, not wire bytes.
use crate::Value;
use serde_json::Value as Json;

mod json_text;

/// Maximum JSON text inspected or restored as a public value.
pub const MAX_JSON_TEXT_BYTES: usize = 4 * 1024 * 1024;

/// Closed credential namespaces, including equivalent separator/case spellings.
/// A whole `pin_code` composite is private, including its non-code members.
const CODE_NAMES: &[&str] = &[
    "actioncode",
    "actionpin",
    "pincode",
    "pincodevalue",
    "keypadcode",
    "usercode",
    "mastercode",
    "accesscode",
    "securitycode",
    "alarmcode",
    "lockcode",
    "inputcode",
    "codeinput",
    "passcode",
    "password",
    "unlockdoor",
];

#[must_use]
pub fn code_property(key: &str) -> bool {
    credential_name(key) || key.split('.').any(credential_name)
}

fn credential_name(key: &str) -> bool {
    let normalized: String = key
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect();
    matches!(normalized.as_str(), "code" | "pin" | "keycode")
        || CODE_NAMES.iter().any(|name| normalized.contains(name))
}

#[must_use]
pub fn code_point(point: &str) -> bool {
    point
        .split_once('/')
        .is_some_and(|(_, key)| code_property(key))
}

/// Objects can otherwise be converted to JSON text by a driver's scalar decoder.
#[must_use]
pub fn private_json(value: &Json) -> bool {
    nested(value, 0)
}

fn nested(value: &Json, depth: usize) -> bool {
    if depth >= 16 {
        return true;
    }
    match value {
        Json::Object(map) => map.iter().any(|(key, value)| {
            code_property(key)
                || (key == "point" && value.as_str().is_some_and(code_point))
                || nested(value, depth + 1)
        }),
        Json::Array(values) => values.iter().any(|value| nested(value, depth + 1)),
        Json::String(text) => inspect_text(text, depth + 1),
        _ => false,
    }
}

/// Inspects borrowed text without constructing an owned JSON tree.
/// JSON-like text that cannot be fully inspected is refused.
#[must_use]
pub fn private_text(text: &str) -> bool {
    inspect_text(text, 0)
}

fn inspect_text(text: &str, depth: usize) -> bool {
    let text = text.trim_start();
    if !text.starts_with(['{', '[', '"']) {
        return false;
    }
    if text.len() > MAX_JSON_TEXT_BYTES {
        return true;
    }
    json_text::inspect(text, depth)
}

#[must_use]
pub fn private_value(value: &Value) -> bool {
    matches!(value, Value::Text(text) if private_text(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn equivalent_credential_names_and_composites_are_private() {
        for name in [
            "action_code",
            "ActionCode",
            "action-code",
            "action.code",
            "pin_code.user",
            "pin.code.value",
            "ephemeral_pin_code.pincode",
            "unlock_door_with_timeout.pin_code",
            "code",
            "PIN",
            "access_code",
            "settings.pin",
            "settings.code",
            "settings.key_code",
        ] {
            assert!(code_property(name), "credential alias unclassified");
        }
        for name in [
            "battery",
            "error_code",
            "date_code",
            "action_transaction",
            "lock_state",
            "color.x",
            "contact",
            "settings.error_code",
            "settings.date_code",
        ] {
            assert!(!code_property(name), "ordinary property classified private");
        }
        assert!(private_json(
            &serde_json::json!({"outer":[{"action_code":"generated elsewhere"}]})
        ));
        assert!(!private_json(
            &serde_json::json!({"state":"ON","error_code":7})
        ));
    }

    #[test]
    fn borrowed_text_scanner_matches_structural_classification() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let code = format!("0{stamp}");
        for value in [
            serde_json::json!({"nested":[{"settings.pin":code}]}),
            serde_json::json!({"point":"fixture:panel/settings.pin","value":code}),
            serde_json::json!({"value":serde_json::json!({"pin":code}).to_string()}),
            serde_json::json!({"ordinary":[null,true,false,1,-2,1.5,"text"]}),
            serde_json::json!({"settings.error_code":7,"settings.date_code":"ordinary"}),
        ] {
            assert!(
                private_text(&value.to_string()) == private_json(&value),
                "text scanner classification differed"
            );
        }
        assert!(
            private_text(&format!("{{\"p\\u0069n\":\"{code}\"}}")),
            "escaped property was missed"
        );
        let dense = format!("[{}null]", "null,".repeat(400_000));
        assert!(!private_text(&dense), "ordinary dense array rejected");
    }

    #[test]
    fn suspicious_json_text_never_becomes_public_after_a_parse_error() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let code = format!("0{stamp}");
        for text in [
            format!(r#"{{"pin":"{code}","tail":}}"#),
            format!(r#"{{"tail":,"pin":"{code}"}}"#),
            format!(r#"{{"pin":"{code}","n":1e400}}"#),
            format!(r#"{{"n":1e400,"pin":"{code}"}}"#),
            format!(r#"{{"pin":"{code}"}} trailing"#),
        ] {
            assert!(private_text(&text), "uninspectable JSON text was public");
            assert!(
                private_json(&Json::String(text)),
                "outer JSON string lost its privacy guard"
            );
        }
        assert!(
            private_text(r#"{"ordinary":}"#),
            "invalid JSON text must not certify absence of a code"
        );
        for ordinary in ["disarm", r#"{"ordinary":[true,81,"ready"]}"#] {
            assert!(!private_text(ordinary), "valid ordinary text was hidden");
        }
    }
}
