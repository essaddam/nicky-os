//! Pure interpretation of the local TV protocol.
use moli_core::Value;
use serde_json::Value as Json;

#[derive(Debug, PartialEq)]
pub(crate) enum Action {
    Key(&'static str),
    Launch(String),
    Volume(u8),
    Mute(bool),
    Wake,
}

pub(crate) const KEYS: [(&str, &str); 22] = [
    ("CursorUp", "KEY_UP"),
    ("CursorDown", "KEY_DOWN"),
    ("CursorLeft", "KEY_LEFT"),
    ("CursorRight", "KEY_RIGHT"),
    ("Confirm", "KEY_ENTER"),
    ("Back", "KEY_RETURN"),
    ("Home", "KEY_HOME"),
    ("Options", "KEY_TOOLS"),
    ("Info", "KEY_INFO"),
    ("PlayPause", "KEY_PLAYPAUSE"),
    ("Play", "KEY_PLAY"),
    ("Pause", "KEY_PAUSE"),
    ("Stop", "KEY_STOP"),
    ("FastForward", "KEY_FF"),
    ("Rewind", "KEY_REWIND"),
    ("VolumeUp", "KEY_VOLUP"),
    ("VolumeDown", "KEY_VOLDOWN"),
    ("Mute", "KEY_MUTE"),
    ("ChannelStepUp", "KEY_CHUP"),
    ("ChannelStepDown", "KEY_CHDOWN"),
    ("Source", "KEY_SOURCE"),
    ("WatchTV", "KEY_TV"),
];

fn identifier(text: &str, max: usize) -> bool {
    !text.is_empty()
        && text.len() <= max
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        && !text.contains("..")
}

pub(crate) fn action(key: &str, value: &Value) -> Option<Action> {
    match (key, value) {
        ("key", Value::Text(text)) => KEYS
            .iter()
            .find(|(k, _)| *k == &**text)
            .map(|(_, k)| Action::Key(k)),
        ("power", Value::Bool(true)) => Some(Action::Wake),
        ("power", Value::Bool(false)) => Some(Action::Key("KEY_POWEROFF")),
        ("volume", Value::Int(v)) => u8::try_from(*v)
            .ok()
            .filter(|v| *v <= 100)
            .map(Action::Volume),
        ("volume", Value::Float(v))
            if v.is_finite() && (0.0..=100.0).contains(v) && v.fract() == 0.0 =>
        {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            // checked integral 0..=100
            Some(Action::Volume(*v as u8))
        }
        ("mute", Value::Bool(v)) => Some(Action::Mute(*v)),
        ("app", Value::Text(text)) if identifier(text, 128) => {
            Some(Action::Launch(text.to_string()))
        }
        _ => None,
    }
}

pub(crate) fn valid_token(text: &str) -> bool {
    !text.is_empty() && text.len() <= 256 && text.bytes().all(|b| b.is_ascii_digit())
}

pub(crate) fn token(message: &Json) -> Option<String> {
    (message["event"].as_str()? == "ms.channel.connect").then_some(())?;
    let text = message["data"]["token"].as_str()?;
    valid_token(text).then(|| text.to_owned())
}

pub(crate) fn native_id(message: &Json) -> Option<String> {
    let id = message["device"]["id"].as_str()?;
    let native = id.strip_prefix("uuid:").unwrap_or(id);
    identifier(native, 128).then(|| native.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn dashboard_keys_are_mapped_without_raw_remote_injection() {
        for (key, expected) in [
            ("CursorUp", "KEY_UP"),
            ("Confirm", "KEY_ENTER"),
            ("Back", "KEY_RETURN"),
            ("Home", "KEY_HOME"),
            ("Mute", "KEY_MUTE"),
            ("VolumeUp", "KEY_VOLUP"),
            ("VolumeDown", "KEY_VOLDOWN"),
            ("Source", "KEY_SOURCE"),
            ("WatchTV", "KEY_TV"),
        ] {
            assert_eq!(
                action("key", &Value::from(key)),
                Some(Action::Key(expected))
            );
        }
        for key in ["KEY_FACTORY", "KEY_POWER", "", "Home\n"] {
            assert_eq!(action("key", &Value::from(key)), None);
        }
    }

    #[test]
    fn absolute_audio_and_power_never_become_relative_toggle_sequences() {
        assert_eq!(action("volume", &Value::Int(42)), Some(Action::Volume(42)));
        assert_eq!(action("mute", &Value::Bool(true)), Some(Action::Mute(true)));
        assert_eq!(action("power", &Value::Bool(true)), Some(Action::Wake));
        assert_eq!(
            action("power", &Value::Bool(false)),
            Some(Action::Key("KEY_POWEROFF"))
        );
        for value in [
            Value::Int(-1),
            Value::Int(101),
            Value::from("42"),
            Value::Bool(true),
        ] {
            assert_eq!(action("volume", &value), None);
        }
    }

    #[test]
    fn hub_canonical_numeric_values_work_without_rounding_invalid_levels() {
        let kind = moli_core::Kind::Numeric {
            min: Some(0.0),
            max: Some(100.0),
            step: Some(1.0),
        };
        let canonical = moli_core::validate(&kind, &Value::Int(42)).unwrap();
        assert_eq!(action("volume", &canonical), Some(Action::Volume(42)));
        for v in [42.5, f64::NAN, f64::INFINITY, -1.0, 101.0] {
            assert_eq!(action("volume", &Value::Float(v)), None);
        }
    }

    #[test]
    fn applications_are_identifiers_never_urls_or_request_paths() {
        for id in ["11101200001", "org.tizen.browser"] {
            assert_eq!(
                action("app", &Value::from(id)),
                Some(Action::Launch(id.into()))
            );
        }
        for id in [
            "",
            "../settings",
            "https://example.com",
            "app?token=hidden",
            "app\r\n",
        ] {
            assert_eq!(action("app", &Value::from(id)), None);
        }
        assert_eq!(action("app", &Value::from("a".repeat(129).as_str())), None);
    }

    #[test]
    fn token_is_only_taken_from_a_successful_bounded_pairing_event() {
        assert_eq!(
            token(
                &serde_json::from_str(include_str!(
                    "../../../integrations/samsung/fixtures/paired.json"
                ))
                .unwrap()
            ),
            Some("12345678".into())
        );
        for message in [
            json!({"event":"ms.channel.unauthorized","data":{"token":"123"}}),
            json!({"event":"ms.channel.connect","data":{"token":"12\r\n"}}),
            json!({"event":"ms.channel.connect","data":{"token":"1".repeat(257)}}),
        ] {
            assert_eq!(token(&message), None);
        }
    }

    #[test]
    fn identity_comes_from_native_device_id_not_name_or_network_address() {
        let mut info: Json = serde_json::from_str(include_str!(
            "../../../integrations/samsung/fixtures/info.json"
        ))
        .unwrap();
        let native = native_id(&info).expect("native id");
        info["device"]["name"] = json!("Renamed");
        info["device"]["ip"] = json!("192.168.1.3");
        assert_eq!(native_id(&info), Some(native));
        assert_eq!(
            native_id(&json!({"device":{"name":"Example TV", "ip":"192.168.1.2"}})),
            None
        );
        assert_eq!(native_id(&json!({"device":{"id":"../foreign"}})), None);
    }
}
