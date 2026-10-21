use super::*;
use crate::computed::Computed;
use moli_core::{AggregateOp, MAX_CALCULATIONS, Value};
use std::cell::Cell;

#[derive(Default)]
struct Model {
    sources: BTreeMap<PointId, AggregateSource>,
    reads: Cell<usize>,
}
impl Projection for Model {
    fn source(&self, point: &PointId) -> AggregateSource {
        self.reads.set(self.reads.get() + 1);
        self.sources.get(point).cloned().unwrap_or(AggregateSource {
            sample: None,
            unit: None,
            online: false,
            numeric: false,
        })
    }
    fn project(&mut self, evaluation: &Evaluation) {
        self.sources.insert(
            PointId::new(&evaluation.id, "value"),
            AggregateSource {
                sample: evaluation.sample.clone(),
                unit: evaluation.unit.clone(),
                online: true,
                numeric: true,
            },
        );
    }
}
fn input(value: f64, ts: u64) -> AggregateSource {
    AggregateSource {
        sample: Some(Sample {
            value: Value::Float(value),
            ts,
        }),
        unit: Some(Unit::Watt),
        online: true,
        numeric: true,
    }
}
fn spec(sources: &[&str], age: u64) -> AggregateSpec {
    AggregateSpec {
        operation: AggregateOp::Mean,
        sources: sources.iter().map(|p| (*p).into()).collect(),
        unit: Some(Unit::Watt),
        max_age_s: age,
    }
    .normalize()
    .unwrap()
}
fn fixture(definitions: BTreeMap<DeviceId, AggregateSpec>) -> (Computed, Instant) {
    let computed = Computed::default();
    let at = {
        let mut engine = computed.engine.lock().unwrap();
        let mut signals = computed.signals.lock().unwrap();
        computed
            .install(&mut engine, &mut signals, definitions)
            .unwrap();
        engine.started
    };
    (computed, at)
}
fn wave(computed: &Computed, model: &mut Model, at: Instant, wall: u64) -> Vec<Evaluation> {
    computed
        .engine
        .lock()
        .unwrap()
        .propagate(&mut computed.signals.lock().unwrap(), at, wall, model)
        .unwrap()
}

#[test]
fn auto_k1_monotonic_expiration_and_identical_observations_do_not_follow_wall_jumps() {
    let (computed, at) = fixture(BTreeMap::from([(
        "calc:a".into(),
        spec(&["fixture:x/value"], 1),
    )]));
    let point = PointId::from("fixture:x/value");
    let mut model = Model::default();
    model.sources.insert(point.clone(), input(10.0, 1000));
    assert!(computed.sample(point.clone(), 1000, at, Some(Unit::Watt)));
    assert_eq!(
        wave(&computed, &mut model, at, 1000)[0]
            .sample
            .as_ref()
            .unwrap()
            .ts,
        1000
    );
    assert!(
        wave(
            &computed,
            &mut model,
            at + Duration::from_millis(999),
            u64::MAX
        )
        .is_empty()
    );
    assert!(
        wave(&computed, &mut model, at + Duration::from_millis(1000), 1)[0]
            .sample
            .is_none()
    );
    assert!(
        wave(
            &computed,
            &mut model,
            at + Duration::from_millis(1100),
            1000
        )
        .is_empty()
    );
    // A genuinely new observation resets age, even with identical value/stamp.
    assert!(computed.sample(
        point,
        1000,
        at + Duration::from_millis(1200),
        Some(Unit::Watt)
    ));
    assert_eq!(
        wave(
            &computed,
            &mut model,
            at + Duration::from_millis(1200),
            1000
        )[0]
        .sample
        .as_ref()
        .unwrap()
        .value,
        Value::Float(10.0)
    );
}

#[test]
fn auto_k1_chain_keeps_source_age_and_propagates_expiration_in_stable_order() {
    let definitions = BTreeMap::from([
        ("calc:c".into(), spec(&["calc:b/value"], 86400)),
        ("calc:b".into(), spec(&["calc:a/value"], 86400)),
        (
            "calc:a".into(),
            spec(&["fixture:x/value", "fixture:y/value"], 1),
        ),
    ]);
    let (computed, at) = fixture(definitions);
    let mut model = Model::default();
    model
        .sources
        .insert("fixture:x/value".into(), input(10.0, 1000));
    model
        .sources
        .insert("fixture:y/value".into(), input(20.0, 1500));
    let first = wave(&computed, &mut model, at, 1500);
    assert_eq!(
        first.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
        vec!["calc:a", "calc:b", "calc:c"]
    );
    assert!(first.iter().all(|e| e.sample.as_ref().unwrap().ts == 1000));
    let boundary = wave(&computed, &mut model, at + Duration::from_millis(500), 2000);
    assert_eq!(boundary.len(), 3);
    assert!(boundary.iter().all(|e| e.sample
        == Some(Sample {
            value: Value::Float(20.0),
            ts: 1500
        })));
    let expired = wave(
        &computed,
        &mut model,
        at + Duration::from_millis(1000),
        2500,
    );
    assert_eq!(expired.len(), 3);
    assert!(expired.iter().all(|e| e.sample.is_none()));
}

#[test]
fn auto_k1_restored_timestamp_is_not_renewed_and_future_cache_waits_for_its_instant() {
    let (computed, at) = fixture(BTreeMap::from([(
        "calc:a".into(),
        spec(&["fixture:x/value"], 1),
    )]));
    let mut model = Model::default();
    model
        .sources
        .insert("fixture:x/value".into(), input(10.0, 1000));
    assert!(wave(&computed, &mut model, at, 5000)[0].sample.is_none());
    computed.device(&DeviceId::from("fixture:x"), false);
    assert!(
        wave(&computed, &mut model, at + Duration::from_secs(1), 1000)[0]
            .sample
            .is_none(),
        "a backwards clock cannot revive an observed stale cache entry"
    );
    model
        .sources
        .insert("fixture:x/value".into(), input(10.0, 10000));
    computed.device(&DeviceId::from("fixture:x"), false);
    assert!(
        wave(&computed, &mut model, at + Duration::from_secs(2), 9000)[0]
            .sample
            .is_none()
    );
    assert_eq!(
        wave(&computed, &mut model, at + Duration::from_secs(3), 10000)[0]
            .sample
            .as_ref()
            .unwrap()
            .ts,
        10000
    );
}

#[test]
fn auto_k1_maximum_fanout_stays_within_budget_and_unindexed_events_do_no_work() {
    let sources: Vec<_> = (0..32).map(|n| format!("fixture:s{n:02}/value")).collect();
    let definitions = (0..MAX_CALCULATIONS)
        .map(|n| {
            (
                format!("calc:c{n:03}").into(),
                spec(&sources.iter().map(String::as_str).collect::<Vec<_>>(), 10),
            )
        })
        .collect();
    let (computed, at) = fixture(definitions);
    let mut model = Model::default();
    for point in sources {
        model.sources.insert(point.into(), input(1.0, 1000));
    }
    let started = Instant::now();
    assert_eq!(wave(&computed, &mut model, at, 1000).len(), 128);
    assert_eq!(computed.engine.lock().unwrap().last_work, 4096);
    assert!(!computed.sample("fixture:unindexed/value".into(), 1000, at, Some(Unit::Watt)));
    model.reads.set(0);
    assert!(wave(&computed, &mut model, at, 1000).is_empty());
    assert_eq!(model.reads.get(), 0);
    eprintln!(
        "AUTO-K1 mesure native debug : 128 calculs × 32 sources, 4096 lectures, {:?}",
        started.elapsed()
    );
}
