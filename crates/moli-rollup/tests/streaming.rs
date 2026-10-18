use jiff::tz::TimeZone;
use moli_core::Value;
use moli_rollup::{
    Aggregation, HOUR_MS, Hour, Origin, Period, PeriodAccumulator, Rollup, group, hourly,
};

#[test]
fn streaming_matches_sliced_hourly_and_calendar_grouping() {
    for aggregation in [
        Aggregation::TimeWeighted,
        Aggregation::Sum,
        Aggregation::Count,
        Aggregation::Durations,
    ] {
        let rule = Rollup {
            aggregation,
            freshness_ms: Some(HOUR_MS / 2),
            origin: Origin::Imported,
        };
        let samples = vec![
            (1000, Value::Int(10)),
            (1000, Value::Int(20)),
            (HOUR_MS / 2, Value::Null),
            (HOUR_MS + 1000, Value::Int(30)),
        ];
        let expected = hourly(&rule, None, &samples, 0, 2 * HOUR_MS).unwrap();
        let mut actual = Vec::new();
        let mut anchor = None;
        for start in [0, HOUR_MS] {
            let mut hour = Hour::new(&rule, start, anchor, 64).unwrap();
            for (ts, v) in samples
                .iter()
                .filter(|(ts, _)| *ts >= start && *ts < start + HOUR_MS)
            {
                hour.push(*ts, v.clone()).unwrap();
            }
            let stat = hour.finish().unwrap();
            anchor = stat.last.clone();
            actual.push(stat);
        }
        assert_eq!(actual, expected);
        let tz = TimeZone::UTC;
        for period in [Period::Day, Period::Month] {
            let mut acc = PeriodAccumulator::new(period, tz.clone(), 64);
            let mut grouped = Vec::new();
            for h in &actual {
                if let Some(s) = acc.push(h).unwrap() {
                    grouped.push(s);
                }
            }
            if let Some(s) = acc.finish() {
                grouped.push(s);
            }
            assert_eq!(grouped, group(&actual, period, &tz).unwrap());
        }
    }
}

#[test]
fn streaming_rejects_unsorted_out_of_hour_and_excess_state_cardinality() {
    let rule = Rollup {
        aggregation: Aggregation::Durations,
        freshness_ms: None,
        origin: Origin::Recorded,
    };
    let mut h = Hour::new(&rule, 0, None, 1).unwrap();
    h.push(1000, Value::Int(1)).unwrap();
    assert!(h.push(999, Value::Int(1)).is_err());
    assert!(h.push(HOUR_MS, Value::Int(1)).is_err());
    h.push(2000, Value::Int(2)).unwrap();
    assert!(h.push(3000, Value::Int(3)).is_err());
}

#[test]
fn daily_spans_merge_to_same_month_as_the_original_hours() {
    let rule = Rollup {
        aggregation: Aggregation::TimeWeighted,
        freshness_ms: None,
        origin: Origin::Recorded,
    };
    let samples = vec![(0, Value::Int(10)), (25 * HOUR_MS, Value::Int(20))];
    let hours = hourly(&rule, None, &samples, 0, 48 * HOUR_MS).unwrap();
    let days = group(&hours, Period::Day, &TimeZone::UTC).unwrap();
    let mut accumulator = PeriodAccumulator::new(Period::Month, TimeZone::UTC, 64);
    for day in &days {
        assert!(accumulator.push_span(day).unwrap().is_none());
    }
    let mut actual = accumulator.finish().unwrap();
    let expected = group(&hours, Period::Month, &TimeZone::UTC)
        .unwrap()
        .remove(0);
    if let (
        Some(moli_rollup::Aggregate::TimeWeighted { mean: a, .. }),
        Some(moli_rollup::Aggregate::TimeWeighted { mean: b, .. }),
    ) = (&mut actual.aggregate, &expected.aggregate)
    {
        assert!((*a - *b).abs() < 1e-12);
        *a = *b;
    }
    assert_eq!(actual, expected);
}

#[test]
fn a_base_span_crossing_the_calendar_end_is_refused() {
    let rule = Rollup {
        aggregation: Aggregation::Sum,
        freshness_ms: None,
        origin: Origin::Recorded,
    };
    let hours = hourly(&rule, None, &[(0, Value::Int(1))], 0, HOUR_MS).unwrap();
    let mut acc = PeriodAccumulator::new(Period::Day, TimeZone::UTC, 64);
    acc.push(&hours[0]).unwrap();
    let mut invalid = hours[0].clone();
    invalid.start = HOUR_MS;
    invalid.end = 25 * HOUR_MS;
    assert!(acc.push_span(&invalid).is_err());
}
