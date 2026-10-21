use moli_core::{Kind, Value, validate};
use serde_json::json;

fn kind(format: &str, timezone: &serde_json::Value) -> Kind {
    let parsed = serde_json::from_value(
        json!({"type":"temporal","spec":{"format":format,"timezone":timezone}}),
    );
    assert!(
        parsed.is_ok(),
        "temporal kinds must be explicit: {parsed:?}"
    );
    parsed.unwrap()
}
#[test]
fn auto_h3_calendar_and_clock_are_strict_civil_values() {
    let date = kind("date", &json!(null));
    for value in ["2000-02-29", "2024-02-29", "0001-01-01", "9999-12-31"] {
        assert!(validate(&date, &Value::Text(value.into())).is_ok());
    }
    for value in [
        "1900-02-29",
        "2100-02-29",
        "2026-04-31",
        "0000-01-01",
        "2026-2-01",
        "2026-01-01Z",
    ] {
        assert!(
            validate(&date, &Value::Text(value.into())).is_err(),
            "{value}"
        );
    }
    let time = kind("time", &json!(null));
    for value in ["00:00:00", "23:59:59"] {
        assert!(validate(&time, &Value::Text(value.into())).is_ok());
    }
    for value in ["24:00:00", "12:60:00", "12:00:60", "12:30", "12:30:00Z"] {
        assert!(validate(&time, &Value::Text(value.into())).is_err());
    }
    for value in [Value::Null, Value::Int(0)] {
        assert!(validate(&date, &value).is_err());
    }
}
