use moli_core::{Kind, Value, validate};
use serde_json::json;

#[test]
fn auto_h1_start_is_an_exact_bounded_duration_and_cancel_is_null() {
    let start = serde_json::from_value::<Kind>(json!({
        "type": "timer_command", "operation": "start", "default_seconds": 300
    }));
    assert!(start.is_ok(), "the timer command kind must be supported");
    let start = start.unwrap();
    assert!(start.is_operation());
    for seconds in [1, 604_800] {
        assert_eq!(
            validate(&start, &Value::Int(seconds)).unwrap(),
            Value::Int(seconds)
        );
    }
    for value in [
        Value::Int(0),
        Value::Int(-1),
        Value::Int(604_801),
        Value::Float(1.0),
        Value::Null,
    ] {
        assert!(validate(&start, &value).is_err(), "{value:?}");
    }
    let cancel: Kind = serde_json::from_value(json!({
        "type": "timer_command", "operation": "cancel", "default_seconds": 300
    }))
    .unwrap();
    assert_eq!(validate(&cancel, &Value::Null).unwrap(), Value::Null);
    assert!(validate(&cancel, &Value::Int(1)).is_err());
}
