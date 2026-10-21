use moli_core::{AggregateOp, AggregateSource, AggregateSpec, Sample, Unit, Value};

fn spec(operation: AggregateOp) -> AggregateSpec {
    AggregateSpec {
        operation,
        sources: vec!["fixture:a/value".into(), "fixture:b/value".into()],
        unit: Some(Unit::Watt),
        max_age_s: 1,
    }
}
fn source(value: Value, ts: u64, online: bool) -> AggregateSource {
    AggregateSource {
        sample: Some(Sample { value, ts }),
        unit: Some(Unit::Watt),
        online,
        numeric: true,
    }
}

#[test]
fn auto_k1_only_fresh_finite_numeric_online_sources_contribute() {
    let mut sources = vec![
        source(Value::Float(10.0), 1000, true),
        source(Value::Float(20.0), 1500, true),
    ];
    let result = spec(AggregateOp::Mean).evaluate(&sources, 1999);
    assert_eq!(
        result.sample.unwrap(),
        Sample {
            value: Value::Float(15.0),
            ts: 1000
        }
    );
    assert_eq!(result.next, Some(2000));
    assert_eq!(
        spec(AggregateOp::Mean)
            .evaluate(&sources, 2000)
            .sample
            .unwrap()
            .value,
        Value::Float(20.0)
    );
    assert!(
        spec(AggregateOp::Mean)
            .evaluate(&sources, 2500)
            .sample
            .is_none()
    );
    for invalid in [
        source(Value::Null, 1500, true),
        source(Value::Float(f64::NAN), 1500, true),
        source(Value::Float(f64::INFINITY), 1500, true),
        source(Value::Float(20.0), 1500, false),
        source(Value::Float(20.0), 2100, true),
    ] {
        sources[1] = invalid;
        let result = spec(AggregateOp::Mean).evaluate(&sources, 1999);
        assert_eq!(result.sample.unwrap().value, Value::Float(10.0));
    }
}

#[test]
fn auto_k1_mean_does_not_overflow_and_min_max_use_valid_values() {
    let values = [
        source(Value::Float(f64::MAX), 1000, true),
        source(Value::Float(-f64::MAX), 1000, true),
    ];
    assert_eq!(
        spec(AggregateOp::Mean)
            .evaluate(&values, 1000)
            .sample
            .unwrap()
            .value,
        Value::Float(0.0)
    );
    assert_eq!(
        spec(AggregateOp::Min)
            .evaluate(&values, 1000)
            .sample
            .unwrap()
            .value,
        Value::Float(-f64::MAX)
    );
    assert_eq!(
        spec(AggregateOp::Max)
            .evaluate(&values, 1000)
            .sample
            .unwrap()
            .value,
        Value::Float(f64::MAX)
    );
    assert!(spec(AggregateOp::Mean).evaluate(&[], 1000).sample.is_none());
}

#[test]
fn auto_k1_unit_conversion_is_explicit_and_incompatible_units_are_excluded() {
    let mut spec = spec(AggregateOp::Mean);
    spec.unit = Some(Unit::KiloWattHour);
    let values = [
        AggregateSource {
            unit: Some(Unit::WattHour),
            ..source(Value::Float(1000.0), 1000, true)
        },
        AggregateSource {
            unit: Some(Unit::KiloWattHour),
            ..source(Value::Float(3.0), 1000, true)
        },
    ];
    assert_eq!(
        spec.evaluate(&values, 1000).sample.unwrap().value,
        Value::Float(2.0)
    );
    let wrong = [source(Value::Float(10.0), 1000, true)];
    assert!(spec.evaluate(&wrong, 1000).sample.is_none());
}

#[test]
fn auto_k1_definition_limits_and_source_shapes_are_checked() {
    let base = spec(AggregateOp::Mean);
    for age in [0, 604_801, u64::MAX] {
        assert!(
            AggregateSpec {
                max_age_s: age,
                ..base.clone()
            }
            .normalize()
            .is_err()
        );
    }
    let duplicate = AggregateSpec {
        sources: vec!["fixture:a/value".into(); 2],
        ..base.clone()
    };
    assert!(duplicate.normalize().is_err());
    assert!(
        AggregateSpec {
            sources: vec!["fixture:a/value".into(); 33],
            ..base.clone()
        }
        .normalize()
        .is_err()
    );
    for point in [
        "missing_separator",
        "no_device/value",
        "fixture:panel/action_code",
    ] {
        assert!(
            AggregateSpec {
                sources: vec![point.into()],
                ..base.clone()
            }
            .normalize()
            .is_err()
        );
    }
    assert!(
        AggregateSpec {
            sources: vec![],
            ..base
        }
        .normalize()
        .is_ok()
    );
}

#[test]
fn auto_k1_state_provenance_is_optional_on_the_existing_wire_format() {
    let raw = serde_json::json!({"type":"state","point":"fixture:x/value","value":10.0,"ts":42});
    let event: moli_core::Event = serde_json::from_value(raw.clone()).unwrap();
    assert_eq!(serde_json::to_value(event).unwrap(), raw);
    let derived = serde_json::json!({"type":"state","point":"fixture:a/value","value":10.0,"ts":42,"source_ts":1});
    let event: moli_core::Event = serde_json::from_value(derived.clone()).unwrap();
    assert_eq!(serde_json::to_value(event).unwrap(), derived);
}
