//! AUTO-H4 domain contracts, independent of clocks and hardware.

use moli_core::{Event, Kind, Value, validate};

#[test]
fn auto_h4_occurrences_keep_distinct_identity_at_the_same_millisecond() {
    let event = |sequence| {
        serde_json::json!({
            "type": "occurrence",
            "occurrence": {
                "id": { "source": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "sequence": sequence },
                "stream": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "position": sequence,
                "point": "simulated:button/press",
                "value": null,
                "ts": 42,
                "durable": true
            }
        })
    };
    let first = serde_json::from_value::<Event>(event(1));
    assert!(first.is_ok(), "typed occurrence is supported: {first:?}");
    let first = first.unwrap();
    let second: Event = serde_json::from_value(event(2)).expect("second press is supported");
    assert_ne!(
        first, second,
        "equal contents and timestamps are not equal occurrences"
    );
    assert_eq!(serde_json::to_value(first).unwrap(), event(1));
    assert_eq!(serde_json::to_value(second).unwrap(), event(2));
}

#[test]
fn auto_h4_button_accepts_an_appui_without_a_synthetic_state_value() {
    let kind = serde_json::from_value::<Kind>(serde_json::json!({"type": "button"}));
    assert!(kind.is_ok(), "a button has an explicit kind: {kind:?}");
    let kind = kind.unwrap();
    assert_eq!(validate(&kind, &Value::Null), Ok(Value::Null));
    assert!(validate(&kind, &Value::Bool(true)).is_err());
    assert!(validate(&kind, &Value::from("2026-01-01T00:00:00Z")).is_err());
}
