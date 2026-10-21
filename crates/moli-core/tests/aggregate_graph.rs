use moli_core::{AggregateGraphError, AggregateOp, AggregateSpec, DeviceId, Unit, aggregate_order};
use std::collections::BTreeMap;
fn spec(sources: &[&str]) -> AggregateSpec {
    AggregateSpec {
        operation: AggregateOp::Mean,
        sources: sources.iter().map(|s| (*s).into()).collect(),
        unit: Some(Unit::Watt),
        max_age_s: 10,
    }
}
#[test]
fn auto_k1_graph_order_is_stable_and_cycles_are_rejected() {
    let mut defs = BTreeMap::from([
        (DeviceId::from("calc:b"), spec(&["calc:a/value"])),
        ("calc:a".into(), spec(&["fixture:x/value"])),
    ]);
    assert_eq!(
        aggregate_order(&defs).unwrap(),
        vec![DeviceId::from("calc:a"), DeviceId::from("calc:b")]
    );
    defs.insert("calc:a".into(), spec(&["calc:b/value"]));
    assert_eq!(aggregate_order(&defs), Err(AggregateGraphError::Cycle));
    defs.remove(&DeviceId::from("calc:b"));
    defs.insert("calc:a".into(), spec(&["calc:a/value"]));
    assert_eq!(aggregate_order(&defs), Err(AggregateGraphError::Cycle));
}
#[test]
fn auto_k1_depth_units_and_node_bounds_are_checked() {
    let mut defs = BTreeMap::new();
    for n in 0..17 {
        let source = if n == 0 {
            "fixture:x/value".into()
        } else {
            format!("calc:c{:02}/value", n - 1)
        };
        defs.insert(format!("calc:c{n:02}").into(), spec(&[&source]));
    }
    assert_eq!(aggregate_order(&defs), Err(AggregateGraphError::Depth));
    defs.clear();
    defs.insert("calc:a".into(), spec(&["fixture:x/value"]));
    let mut child = spec(&["calc:a/value"]);
    child.unit = Some(Unit::Celsius);
    defs.insert("calc:b".into(), child);
    assert_eq!(aggregate_order(&defs), Err(AggregateGraphError::Units));
    defs.clear();
    for n in 0..129 {
        defs.insert(format!("calc:c{n:03}").into(), spec(&["fixture:x/value"]));
    }
    assert_eq!(aggregate_order(&defs), Err(AggregateGraphError::Capacity));
}
