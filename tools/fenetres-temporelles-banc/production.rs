//! Evidence from public production APIs; independent from structure prototypes.
use moli_core::{
    AggregateOp, AggregateSource, AggregateSpec, DurationLedger, PointId, Sample, Unit, Value,
};
use moli_history::Store;
use moli_rollup::{Aggregate, Aggregation, HOUR_MS, Origin, Period, Rollup, group, hourly};
use serde_json::json;
use std::{hint::black_box, time::Instant};
fn mean(stats: &[moli_rollup::Stat]) -> Option<f64> {
    match stats[0].aggregate {
        Some(Aggregate::TimeWeighted { mean, .. }) => Some(mean),
        _ => None,
    }
}
fn rule(freshness_ms: Option<u64>) -> Rollup {
    Rollup {
        aggregation: Aggregation::TimeWeighted,
        freshness_ms,
        origin: Origin::Recorded,
    }
}
fn evidence() -> serde_json::Value {
    let path = std::env::temp_dir().join(format!(
        "moli-synthetic-window-{}-{}.db",
        std::process::id(),
        moli_core::now_ms()
    ));
    let store = Store::open(&path).unwrap();
    let point: PointId = "fixture:temporal/power".into();
    // Three actual polls: [0,0,100]. Only two changes are recorded.
    store
        .insert(
            &[
                (point.clone(), Value::Float(0.0), 0),
                (point.clone(), Value::Float(100.0), 9 * HOUR_MS / 10),
            ],
            false,
        )
        .unwrap();
    let series = store.series(point.as_str(), 0, HOUR_MS, 60).unwrap();
    let sample_avg = series
        .raw
        .iter()
        .filter_map(|(_, v)| v.as_f64())
        .sum::<f64>()
        / series.raw.len() as f64;
    let stats = hourly(&rule(None), None, &series.raw, 0, HOUR_MS).unwrap();
    assert_eq!(sample_avg, 50.0);
    assert_eq!(mean(&stats), Some(10.0));
    let bucketed = store.series(point.as_str(), 0, HOUR_MS, 1).unwrap();
    let bucket_average = bucketed
        .buckets
        .iter()
        .map(|b| b.avg * b.count as f64)
        .sum::<f64>()
        / bucketed.buckets.iter().map(|b| b.count).sum::<u64>() as f64;
    assert_eq!(bucket_average, 50.0);
    let before = store
        .series(point.as_str(), HOUR_MS, 2 * HOUR_MS, 60)
        .unwrap();
    assert!(before.raw.is_empty());
    assert!(before.before.is_some());
    let carry = hourly(
        &rule(None),
        before.before.as_ref(),
        &before.raw,
        HOUR_MS,
        2 * HOUR_MS,
    )
    .unwrap();
    assert_eq!(mean(&carry), Some(100.0));
    let boundary = store
        .series(point.as_str(), 0, 9 * HOUR_MS / 10, 60)
        .unwrap();
    assert_eq!(boundary.raw.len(), 2); // History's inclusive BETWEEN upper boundary.
    let missing = hourly(
        &rule(Some(10_000)),
        None,
        &[(0, Value::Float(0.0)), (20_000, Value::Null)],
        0,
        HOUR_MS,
    )
    .unwrap();
    assert_eq!(missing[0].covered_ms, 10_000);
    assert_eq!(mean(&missing), Some(0.0));
    let duplicate = hourly(
        &rule(None),
        None,
        &[(0, Value::Float(1.0)), (0, Value::Float(3.0))],
        0,
        HOUR_MS,
    )
    .unwrap();
    assert_eq!(duplicate[0].changes, 2);
    assert_eq!(mean(&duplicate), Some(3.0));
    assert!(
        hourly(
            &rule(None),
            None,
            &[(2, Value::Float(1.0)), (1, Value::Float(2.0))],
            0,
            HOUR_MS
        )
        .is_err()
    );
    assert!(hourly(&rule(None), None, &[], 0, 60_000).is_err()); // Not arbitrary sliding windows.
    let definition = AggregateSpec {
        operation: AggregateOp::Mean,
        sources: vec!["fixture:a/value".into(), "fixture:b/value".into()],
        unit: Some(Unit::KiloWattHour),
        max_age_s: 60,
    };
    let source = |value, unit| AggregateSource {
        sample: Some(Sample {
            value: Value::Float(value),
            ts: 0,
        }),
        unit: Some(unit),
        online: true,
        numeric: true,
    };
    let converted = definition.evaluate(
        &[
            source(1000.0, Unit::WattHour),
            source(3.0, Unit::KiloWattHour),
        ],
        1,
    );
    assert_eq!(converted.sample.unwrap().value, Value::Float(2.0));
    assert!(
        definition
            .evaluate(&[source(2.0, Unit::Watt)], 1)
            .sample
            .is_none()
    );
    let mut ledger = DurationLedger::new(0);
    ledger.observe(0, Some(true), 10_000);
    ledger.advance(10_000);
    ledger.resume();
    ledger.advance(60_000);
    let duration = ledger.stats(60_000, 60_000);
    assert_eq!(duration.known_ms, 10_000);
    assert_eq!(duration.unknown_ms, 50_000);
    let tz = jiff::tz::TimeZone::get("Europe/Paris").unwrap();
    let mut civil = Vec::new();
    for day in [
        "2026-03-29T00:00:00+01:00[Europe/Paris]",
        "2026-10-25T00:00:00+02:00[Europe/Paris]",
    ] {
        let from: jiff::Zoned = day.parse().unwrap();
        let to = from.checked_add(jiff::Span::new().days(1)).unwrap();
        let start = from.timestamp().as_millisecond() as u64;
        let end = to.timestamp().as_millisecond() as u64;
        let hours = hourly(&rule(None), None, &[(start, Value::Float(7.0))], start, end).unwrap();
        let days = group(&hours, Period::Day, &tz).unwrap();
        assert_eq!(days.len(), 1);
        assert_eq!(days[0].covered_ms, end - start);
        civil.push(json!({"local_day":day,"elapsed_hours":(end-start)/HOUR_MS,"mean":mean(&days)}));
    }
    assert_eq!(civil[0]["elapsed_hours"], 23);
    assert_eq!(civil[1]["elapsed_hours"], 25);
    let data = (0..86_400u64)
        .map(|i| (i * 1000, Value::Float((i % 100) as f64)))
        .collect::<Vec<_>>();
    let mut timings = Vec::new();
    for round in 0..8 {
        let start = Instant::now();
        let value = hourly(&rule(Some(60_000)), None, black_box(&data), 0, 24 * HOUR_MS).unwrap();
        black_box(value);
        if round > 0 {
            timings.push(start.elapsed().as_nanos());
        }
    }
    timings.sort();
    drop(store);
    for suffix in ["", "-wal", "-shm"] {
        let file = std::path::PathBuf::from(format!("{}{suffix}", path.display()));
        if file.exists() {
            std::fs::remove_file(file).unwrap();
        }
    }
    json!({"status":"production_public_apis","input_poll_mean":100.0/3.0,"recorded_changes":series.raw.len(),"recorded_change_mean":sample_avg,"bucket_count_weighted_mean":bucket_average,"duration_weighted_mean":mean(&stats),"covered_ms":stats[0].covered_ms,"anchor_next_hour_mean":mean(&carry),"history_upper_bound_inclusive":true,"rollup_bounds":"whole_utc_hours_only","rollup_duplicates":"count_each_change_last_value_holds","prototype_duplicates":"replacement_one_sample","production_unit_conversion_1000wh_3kwh_mean_kwh":2.0,"duration_restart_known_ms":duration.known_ms,"duration_restart_unknown_ms":duration.unknown_ms,"civil_days":civil,"rolling_24h_elapsed_hours":24,"hourly_86400_changes_24h_ns_rounds":timings,"hourly_86400_changes_24h_ns_median":timings[3]})
}
fn main() {
    println!("{}", evidence());
}
#[cfg(test)]
mod tests {
    #[test]
    fn production_contracts() {
        super::evidence();
    }
}
