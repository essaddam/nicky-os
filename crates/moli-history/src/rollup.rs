//! Hourly summaries of one point's history, then days and months in local
//! time. Pure computation: no I/O, no clock, no configuration.
//!
//! The history keeps changes only, so a value **holds** from its sample
//! until the next one. From that:
//!
//! - an hour starts with the value carried from before it (the anchor, then
//!   each hour's [`Stat::last`]), so rolling `[a, c)` at once or as `[a, b)`
//!   then `[b, c)` gives the same hours;
//! - `Null` (the point is offline or unknown) holds too, as a period that is
//!   not covered;
//! - a value is trusted for at most [`Rollup::freshness_ms`] after its
//!   sample, then the time is not covered either, so an old value does not
//!   fill weeks. Unbounded is right for a point whose driver only reports
//!   changes (a stable temperature legitimately has one old sample); a
//!   bound fits a point known to re-report;
//! - an hour that is not covered at all has no aggregate (`None`), which is
//!   not an hour at zero;
//! - every sample is one change, two identical ones at the same instant
//!   included.
//!
//! Which [`Aggregation`] a point gets is decided by the caller from what it
//! means (power is time-weighted, energy is summed…), not by its value type.

use std::collections::BTreeMap;

use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan as _, Zoned};
use moli_core::Value;
use serde::Serialize;

pub const HOUR_MS: u64 = 3_600_000;

/// How the samples of one point are summarised.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Aggregation {
    /// A level (temperature, power): mean weighted by how long each value
    /// held, with the min and max of the values that held. Numbers only;
    /// any other value is not covered.
    TimeWeighted,
    /// Quantities (energy consumed per hour): each numeric sample is added
    /// to the hour it falls in. Samples are consumptions, never cumulative
    /// meter readings, so a meter reset is already absorbed upstream.
    Sum,
    /// Events (a pulse, a button press): every non-`Null` sample counts.
    Count,
    /// States (on / off, a mode): how long each state held.
    Durations,
}

/// Where the samples come from, copied into every summary. Ordered from the
/// most to the least reliable: a merged period keeps the least reliable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// Recorded live by Moli.
    Recorded,
    /// Imported from another system.
    Imported,
    /// Computed, not measured.
    Estimated,
}

/// How much of the period the aggregate covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    Complete,
    Partial,
    Empty,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Aggregate {
    TimeWeighted {
        mean: f64,
        min: f64,
        max: f64,
    },
    Sum {
        total: f64,
    },
    Count {
        events: u64,
    },
    /// State (as text) → milliseconds held.
    Durations {
        ms: BTreeMap<String, u64>,
    },
}

/// The summary of one period: an hour from [`hourly`], a local day or month
/// from [`group`].
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Stat {
    /// `[start, end)`, ms since the Unix epoch.
    pub start: u64,
    pub end: u64,
    /// `None` when nothing is known of the period (not the same as zero).
    pub aggregate: Option<Aggregate>,
    /// Time the aggregate is computed over: a fresh, known value held
    /// (a number, for [`Aggregation::TimeWeighted`] and [`Aggregation::Sum`]).
    pub covered_ms: u64,
    /// Samples inside the period, `Null` ones included.
    pub changes: u64,
    /// Latest sample before `end`, maybe older than the period: the anchor
    /// of the next one.
    pub last: Option<(u64, Value)>,
    pub quality: Quality,
    pub origin: Origin,
}

/// How to summarise one point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rollup {
    pub aggregation: Aggregation,
    /// How long a sample is trusted; `None`: until the next one.
    pub freshness_ms: Option<u64>,
    pub origin: Origin,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RollupError {
    #[error("bounds must be whole UTC hours with start <= end")]
    Unaligned,
    #[error("samples must be ordered, with the anchor before the window")]
    Unordered,
    #[error("periods must be contiguous summaries of the same aggregation")]
    Mismatch,
    #[error("local {0} does not start on a whole UTC hour in this time zone")]
    OffHour(&'static str),
    #[error("a sum exceeds the range of a 64-bit float")]
    Overflow,
    #[error("time: {0}")]
    Time(String),
}

/// One [`Stat`] per UTC hour of `[start, end)`. `anchor` is the latest
/// sample before `start` (it may be far older); `samples` are ordered by
/// time. Samples before `start` only update the anchor, those at or after
/// `end` are ignored.
pub fn hourly(
    rule: &Rollup,
    anchor: Option<&(u64, Value)>,
    samples: &[(u64, Value)],
    start: u64,
    end: u64,
) -> Result<Vec<Stat>, RollupError> {
    if !start.is_multiple_of(HOUR_MS) || !end.is_multiple_of(HOUR_MS) || start > end {
        return Err(RollupError::Unaligned);
    }
    if anchor.is_some_and(|a| a.0 >= start || samples.first().is_some_and(|s| s.0 < a.0))
        || samples.windows(2).any(|w| w[1].0 < w[0].0)
    {
        return Err(RollupError::Unordered);
    }
    let mut carry = anchor;
    let mut rest = samples;
    while let Some((first, tail)) = rest.split_first().filter(|(s, _)| s.0 < start) {
        carry = Some(first);
        rest = tail;
    }
    let mut stats = Vec::new();
    for hour in (start..end).step_by(usize::try_from(HOUR_MS).unwrap_or(usize::MAX)) {
        let hour_end = hour + HOUR_MS;
        let mut acc = Acc::new(rule.aggregation);
        let mut cursor = hour;
        loop {
            let next = rest.first().filter(|s| s.0 < hour_end);
            if let Some(held) = carry {
                acc.hold(
                    held,
                    cursor,
                    next.map_or(hour_end, |s| s.0),
                    rule.freshness_ms,
                );
            }
            let Some(sample) = next else { break };
            acc.event(&sample.1);
            carry = Some(sample);
            cursor = sample.0;
            rest = &rest[1..];
        }
        stats.push(acc.finish(hour, hour_end, carry.cloned(), rule.origin)?);
    }
    Ok(stats)
}

/// Accumulates one hour.
struct Acc {
    aggregation: Aggregation,
    covered: u64,
    changes: u64,
    /// Contributing events (sum, count).
    events: u64,
    total: f64,
    /// Running mean, so value × duration never overflows.
    mean: f64,
    min: f64,
    max: f64,
    durations: BTreeMap<String, u64>,
}

impl Acc {
    fn new(aggregation: Aggregation) -> Self {
        Self {
            aggregation,
            covered: 0,
            changes: 0,
            events: 0,
            total: 0.0,
            mean: 0.0,
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
            durations: BTreeMap::new(),
        }
    }

    /// `sample` held over `[from, to)`, as long as it was fresh.
    fn hold(&mut self, sample: &(u64, Value), from: u64, to: u64, freshness: Option<u64>) {
        let (ts, value) = sample;
        let to = freshness.map_or(to, |f| to.min(ts.saturating_add(f)));
        let held = to.saturating_sub(from);
        if held == 0 {
            return;
        }
        match self.aggregation {
            Aggregation::Sum if number(value).is_none() => return,
            Aggregation::TimeWeighted => {
                let Some(x) = number(value) else { return };
                #[allow(clippy::cast_precision_loss)] // ms in an hour: exact
                let (w, held) = (self.covered as f64, held as f64);
                self.mean = blend(self.mean, w, x, held);
                self.min = self.min.min(x);
                self.max = self.max.max(x);
            }
            Aggregation::Durations => {
                let Some(state) = state(value) else { return };
                *self.durations.entry(state).or_default() += held;
            }
            Aggregation::Count if matches!(value, Value::Null) => return,
            Aggregation::Sum | Aggregation::Count => {}
        }
        self.covered += held;
    }

    fn event(&mut self, value: &Value) {
        self.changes += 1;
        match self.aggregation {
            Aggregation::Sum => {
                if let Some(x) = number(value) {
                    self.total += x;
                    self.events += 1;
                }
            }
            Aggregation::Count if !matches!(value, Value::Null) => self.events += 1,
            _ => {}
        }
    }

    fn finish(
        self,
        start: u64,
        end: u64,
        last: Option<(u64, Value)>,
        origin: Origin,
    ) -> Result<Stat, RollupError> {
        if !self.total.is_finite() {
            return Err(RollupError::Overflow);
        }
        let known = self.covered > 0;
        let aggregate = match self.aggregation {
            Aggregation::TimeWeighted if known => Some(Aggregate::TimeWeighted {
                mean: self.mean.clamp(self.min, self.max),
                min: self.min,
                max: self.max,
            }),
            Aggregation::Durations if known => Some(Aggregate::Durations { ms: self.durations }),
            Aggregation::Sum if known || self.events > 0 => {
                Some(Aggregate::Sum { total: self.total })
            }
            Aggregation::Count if known || self.events > 0 => Some(Aggregate::Count {
                events: self.events,
            }),
            _ => None,
        };
        Ok(Stat {
            start,
            end,
            aggregate,
            covered_ms: self.covered,
            changes: self.changes,
            last,
            quality: quality(self.covered, end - start),
            origin,
        })
    }
}

/// Mean of `a` weighing `wa` and `b` weighing `wb` (`wa + wb > 0`), as a
/// convex combination: no product of a value by a weight, so finite values
/// near the f64 limits neither overflow nor give NaN. Rounding may land an
/// ulp outside the inputs: callers clamp to [min, max].
fn blend(a: f64, wa: f64, b: f64, wb: f64) -> f64 {
    let total = wa + wb;
    a * (wa / total) + b * (wb / total)
}

fn number(value: &Value) -> Option<f64> {
    value.as_f64().filter(|x| x.is_finite())
}

fn state(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::Bool(b) => Some(b.to_string()),
        Value::Int(i) => Some(i.to_string()),
        Value::Float(f) => Some(f.to_string()),
        Value::Text(t) => Some(t.to_string()),
    }
}

fn quality(covered: u64, length: u64) -> Quality {
    match covered {
        0 => Quality::Empty,
        c if c >= length => Quality::Complete,
        _ => Quality::Partial,
    }
}

/// Calendar periods in local time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Period {
    Day,
    Month,
}

impl Period {
    fn name(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Month => "month",
        }
    }
}

/// Contiguous hours (from [`hourly`]) merged into local days or months.
/// Each result spans its whole calendar period (a day lasts 23 or 25 hours
/// across a DST change); hours outside the input simply are not covered, so
/// a partly-known day is [`Quality::Partial`]. All hours must summarise the
/// same point under the same rule: aggregates of different kinds are refused.
///
/// ponytail: hours are the base slice, so a zone whose midnight is not a
/// whole UTC hour (UTC+05:30…) is refused rather than approximated; finer
/// base slices if such a zone matters.
pub fn group(hours: &[Stat], period: Period, tz: &TimeZone) -> Result<Vec<Stat>, RollupError> {
    let mut groups: Vec<Stat> = Vec::new();
    let mut kinds = hours
        .iter()
        .filter_map(|h| h.aggregate.as_ref().map(std::mem::discriminant));
    if let Some(kind) = kinds.next()
        && kinds.any(|k| k != kind)
    {
        return Err(RollupError::Mismatch);
    }
    for (i, hour) in hours.iter().enumerate() {
        if hour.end != hour.start + HOUR_MS || (i > 0 && hours[i - 1].end != hour.start) {
            return Err(RollupError::Mismatch);
        }
        if let Some(g) = groups.last_mut().filter(|g| hour.start < g.end) {
            merge(g, hour)?;
        } else {
            let start = floor(hour.start, period, tz)?;
            let end = next(start, period, tz)?;
            let mut g = hour.clone();
            (g.start, g.end) = (start, end);
            groups.push(g);
        }
    }
    for g in &mut groups {
        g.quality = quality(g.covered_ms, g.end - g.start);
    }
    Ok(groups)
}

fn merge(into: &mut Stat, hour: &Stat) -> Result<(), RollupError> {
    let (a, b) = (into.aggregate.take(), hour.aggregate.clone());
    into.aggregate = match (a, b) {
        (x, None) | (None, x) => x,
        (
            Some(Aggregate::TimeWeighted { mean, min, max }),
            Some(Aggregate::TimeWeighted {
                mean: m2,
                min: lo,
                max: hi,
            }),
        ) => {
            #[allow(clippy::cast_precision_loss)] // ms over a month: exact
            let (w1, w2) = (into.covered_ms as f64, hour.covered_ms as f64);
            let (min, max) = (min.min(lo), max.max(hi));
            Some(Aggregate::TimeWeighted {
                mean: blend(mean, w1, m2, w2).clamp(min, max),
                min,
                max,
            })
        }
        (Some(Aggregate::Sum { total }), Some(Aggregate::Sum { total: t2 })) => {
            let total = total + t2;
            if !total.is_finite() {
                return Err(RollupError::Overflow);
            }
            Some(Aggregate::Sum { total })
        }
        (Some(Aggregate::Count { events }), Some(Aggregate::Count { events: e2 })) => {
            Some(Aggregate::Count {
                events: events + e2,
            })
        }
        (Some(Aggregate::Durations { mut ms }), Some(Aggregate::Durations { ms: m2 })) => {
            for (state, held) in m2 {
                *ms.entry(state).or_default() += held;
            }
            Some(Aggregate::Durations { ms })
        }
        _ => return Err(RollupError::Mismatch),
    };
    into.covered_ms += hour.covered_ms;
    into.changes += hour.changes;
    if hour.last.is_some() {
        into.last.clone_from(&hour.last);
    }
    into.origin = into.origin.max(hour.origin);
    Ok(())
}

fn zoned(ms: u64, tz: &TimeZone) -> Result<Zoned, RollupError> {
    let ms = i64::try_from(ms).map_err(|e| RollupError::Time(e.to_string()))?;
    Timestamp::from_millisecond(ms)
        .map(|t| t.to_zoned(tz.clone()))
        .map_err(|e| RollupError::Time(e.to_string()))
}

/// Refused rather than clamped: a local period may start before the epoch
/// (a day of UTC+01:00 holding 1970-01-01T00:00Z), which `u64` cannot carry.
fn millis(z: &Zoned) -> Result<u64, RollupError> {
    u64::try_from(z.timestamp().as_millisecond())
        .map_err(|_| RollupError::Time("period starts before the Unix epoch".into()))
}

/// Start of the local period holding `ms` (same rule as the energy
/// reports: where DST skips midnight, the day starts at the first instant).
fn floor(ms: u64, period: Period, tz: &TimeZone) -> Result<u64, RollupError> {
    let z = zoned(ms, tz)?;
    let start = match period {
        Period::Day => z.start_of_day(),
        Period::Month => z.first_of_month().and_then(|z| z.start_of_day()),
    }
    .map_err(|e| RollupError::Time(e.to_string()))?;
    let start = millis(&start)?;
    if start.is_multiple_of(HOUR_MS) {
        Ok(start)
    } else {
        Err(RollupError::OffHour(period.name()))
    }
}

fn next(start: u64, period: Period, tz: &TimeZone) -> Result<u64, RollupError> {
    let z = zoned(start, tz)?;
    let later = match period {
        Period::Day => z.checked_add(1.day()),
        Period::Month => z.checked_add(1.month()),
    }
    .map_err(|e| RollupError::Time(e.to_string()))?;
    floor(millis(&later)?, period, tz)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60_000;
    /// 2026-01-05T00:00:00Z, a Monday.
    const T0: u64 = 1_767_571_200_000;

    fn rule(aggregation: Aggregation, freshness_ms: Option<u64>) -> Rollup {
        Rollup {
            aggregation,
            freshness_ms,
            origin: Origin::Recorded,
        }
    }

    fn tw(stat: &Stat) -> (f64, f64, f64) {
        match stat.aggregate {
            Some(Aggregate::TimeWeighted { mean, min, max }) => (mean, min, max),
            ref other => panic!("not time-weighted: {other:?}"),
        }
    }

    fn s(ts: u64, v: impl Into<Value>) -> (u64, Value) {
        (ts, v.into())
    }

    #[test]
    fn irregular_intervals_are_weighted_by_duration() {
        let anchor = s(T0 - 10 * MIN, 20.0);
        let samples = [s(T0 + 15 * MIN, 22.0), s(T0 + 45 * MIN, 18.0)];
        let r = rule(Aggregation::TimeWeighted, None);
        let h = &hourly(&r, Some(&anchor), &samples, T0, T0 + HOUR_MS).unwrap()[0];
        // 15 min at 20, 30 at 22, 15 at 18.
        assert_eq!(tw(h), (20.5, 18.0, 22.0));
        assert_eq!((h.covered_ms, h.changes), (HOUR_MS, 2));
        assert_eq!(h.quality, Quality::Complete);
        assert_eq!(h.last, Some(s(T0 + 45 * MIN, 18.0)));
    }

    #[test]
    fn a_stable_point_holds_its_anchor_for_weeks_unless_bounded() {
        let anchor = s(T0 - 21 * 24 * HOUR_MS, 19.5);
        let end = T0 + 48 * HOUR_MS;
        let r = rule(Aggregation::TimeWeighted, None);
        let hours = hourly(&r, Some(&anchor), &[], T0, end).unwrap();
        assert_eq!(hours.len(), 48);
        for h in &hours {
            assert_eq!(tw(h), (19.5, 19.5, 19.5));
            assert_eq!((h.covered_ms, h.changes), (HOUR_MS, 0));
            assert_eq!(h.last, Some(anchor.clone()));
        }
        // Trusted for one day only: three weeks later, nothing is known.
        let bounded = rule(Aggregation::TimeWeighted, Some(24 * HOUR_MS));
        for h in hourly(&bounded, Some(&anchor), &[], T0, end).unwrap() {
            assert_eq!((h.aggregate, h.covered_ms), (None, 0));
            assert_eq!(h.quality, Quality::Empty);
            assert_eq!(h.last, Some(anchor.clone()), "still the next anchor");
        }
    }

    #[test]
    fn a_measurement_gap_is_not_filled() {
        let samples = [s(T0, 10.0), s(T0 + 2 * HOUR_MS, 12.0)];
        let r = rule(Aggregation::TimeWeighted, Some(30 * MIN));
        let hours = hourly(&r, None, &samples, T0, T0 + 3 * HOUR_MS).unwrap();
        assert_eq!(hours[0].covered_ms, 30 * MIN);
        assert_eq!(hours[0].quality, Quality::Partial);
        assert_eq!(tw(&hours[0]), (10.0, 10.0, 10.0));
        assert_eq!((hours[1].aggregate.clone(), hours[1].covered_ms), (None, 0));
        assert_eq!(hours[2].covered_ms, 30 * MIN);
        assert_eq!(tw(&hours[2]), (12.0, 12.0, 12.0));
    }

    #[test]
    fn offline_then_back_leaves_the_gap_uncovered() {
        let samples = [
            s(T0, 100.0),
            (T0 + 20 * MIN, Value::Null),
            s(T0 + 90 * MIN, 300.0),
        ];
        let r = rule(Aggregation::TimeWeighted, None);
        let hours = hourly(&r, None, &samples, T0, T0 + 2 * HOUR_MS).unwrap();
        assert_eq!((hours[0].covered_ms, hours[0].changes), (20 * MIN, 2));
        assert_eq!(tw(&hours[0]), (100.0, 100.0, 100.0));
        assert_eq!((hours[1].covered_ms, hours[1].changes), (30 * MIN, 1));
        assert_eq!(tw(&hours[1]), (300.0, 300.0, 300.0));
        // A state point goes offline the same way.
        let states = [s(T0, true), (T0 + 20 * MIN, Value::Null)];
        let d = rule(Aggregation::Durations, None);
        let h = &hourly(&d, None, &states, T0, T0 + HOUR_MS).unwrap()[0];
        let ms = BTreeMap::from([("true".to_owned(), 20 * MIN)]);
        assert_eq!(h.aggregate, Some(Aggregate::Durations { ms }));
    }

    #[test]
    fn durations_split_an_hour_between_states() {
        let anchor = s(T0 - HOUR_MS, false);
        let samples = [s(T0 + 10 * MIN, true), s(T0 + 40 * MIN, false)];
        let r = rule(Aggregation::Durations, None);
        let h = &hourly(&r, Some(&anchor), &samples, T0, T0 + HOUR_MS).unwrap()[0];
        let ms = BTreeMap::from([
            ("false".to_owned(), 30 * MIN),
            ("true".to_owned(), 30 * MIN),
        ]);
        assert_eq!(h.aggregate, Some(Aggregate::Durations { ms }));
    }

    #[test]
    fn days_follow_dst_in_local_time() {
        let paris = TimeZone::get("Europe/Paris").unwrap();
        // 2026-03-29 (23 h) and 2026-10-25 (25 h), local midnights in UTC.
        for (midnight, length) in [(1_774_738_800_000, 23), (1_792_879_200_000, 25)] {
            let end = midnight + length * HOUR_MS;
            let r = rule(Aggregation::TimeWeighted, None);
            let anchor = s(midnight - HOUR_MS, 5.0);
            let hours = hourly(&r, Some(&anchor), &[], midnight, end).unwrap();
            let days = group(&hours, Period::Day, &paris).unwrap();
            assert_eq!(days.len(), 1);
            assert_eq!((days[0].start, days[0].end), (midnight, end));
            assert_eq!(days[0].covered_ms, length * HOUR_MS);
            assert_eq!(days[0].quality, Quality::Complete);
        }
    }

    #[test]
    fn months_and_partial_days() {
        let utc = TimeZone::UTC;
        let r = rule(Aggregation::Count, None);
        // Jan 31 12:00 → Feb 1 12:00: half of each day, two months.
        let start = T0 + 26 * 24 * HOUR_MS + 12 * HOUR_MS;
        let samples = [s(start, 1), s(start + 13 * HOUR_MS, 1)];
        let hours = hourly(&r, None, &samples, start, start + 24 * HOUR_MS).unwrap();
        let days = group(&hours, Period::Day, &utc).unwrap();
        assert_eq!(days.len(), 2);
        assert!(days.iter().all(|d| d.quality == Quality::Partial));
        assert!(days.iter().all(|d| d.covered_ms == 12 * HOUR_MS));
        let months = group(&hours, Period::Month, &utc).unwrap();
        assert_eq!(months.len(), 2);
        assert_eq!(months[1].start, T0 + 27 * 24 * HOUR_MS);
        assert_eq!(months[1].aggregate, Some(Aggregate::Count { events: 1 }));
        assert_eq!(months[1].last, Some(s(start + 13 * HOUR_MS, 1)));
    }

    #[test]
    fn a_meter_reset_is_absorbed_by_hourly_consumptions() {
        // Readings 1000 → 1001.2 → reset → 0.8 → 1.3 become consumptions
        // 1.2, 0.8, 0.5 upstream; their sum is the energy, the reading
        // difference (−998.7) is not.
        let samples = [s(T0, 1.2), s(T0 + HOUR_MS, 0.8), s(T0 + 2 * HOUR_MS, 0.5)];
        let r = rule(Aggregation::Sum, Some(HOUR_MS));
        let hours = hourly(&r, None, &samples, T0, T0 + 4 * HOUR_MS).unwrap();
        let day = &group(&hours, Period::Day, &TimeZone::UTC).unwrap()[0];
        let Some(Aggregate::Sum { total }) = day.aggregate else {
            panic!()
        };
        assert!((total - 2.5).abs() < 1e-12);
        // The fourth hour has no consumption row: unknown, not zero.
        assert_eq!(hours[3].aggregate, None);
    }

    #[test]
    fn identical_events_at_the_same_instant_count_twice() {
        let samples = [s(T0 + MIN, "single"), s(T0 + MIN, "single")];
        let r = rule(Aggregation::Count, None);
        let h = &hourly(&r, None, &samples, T0, T0 + HOUR_MS).unwrap()[0];
        assert_eq!(h.aggregate, Some(Aggregate::Count { events: 2 }));
        assert_eq!(h.changes, 2);
    }

    #[test]
    fn an_empty_hour_is_not_an_hour_at_zero() {
        let r = rule(Aggregation::Count, None);
        let empty = &hourly(&r, None, &[], T0, T0 + HOUR_MS).unwrap()[0];
        assert_eq!((empty.aggregate.clone(), empty.covered_ms), (None, 0));
        assert_eq!((empty.quality, empty.last.clone()), (Quality::Empty, None));
        let anchor = s(T0 - MIN, "single");
        let zero = &hourly(&r, Some(&anchor), &[], T0, T0 + HOUR_MS).unwrap()[0];
        assert_eq!(zero.aggregate, Some(Aggregate::Count { events: 0 }));
        assert_eq!(zero.quality, Quality::Complete);
    }

    #[test]
    fn bounds_are_start_inclusive_end_exclusive() {
        let samples = [s(T0, 1.0), s(T0 + HOUR_MS, 2.0), s(T0 + 2 * HOUR_MS, 4.0)];
        let r = rule(Aggregation::Sum, None);
        let hours = hourly(&r, None, &samples, T0, T0 + 2 * HOUR_MS).unwrap();
        let totals: Vec<_> = hours.iter().map(|h| h.aggregate.clone()).collect();
        assert_eq!(
            totals,
            [
                Some(Aggregate::Sum { total: 1.0 }),
                Some(Aggregate::Sum { total: 2.0 })
            ]
        );
        assert_eq!(hours[1].last, Some(s(T0 + HOUR_MS, 2.0)), "end excluded");
        // A sample before the window is only the anchor.
        let hours = hourly(&r, None, &samples, T0 + HOUR_MS, T0 + 2 * HOUR_MS).unwrap();
        assert_eq!(hours[0].aggregate, Some(Aggregate::Sum { total: 2.0 }));
        assert_eq!(hours[0].changes, 1);
        assert!(hourly(&r, None, &samples, T0, T0).unwrap().is_empty());
    }

    #[test]
    fn invalid_input_is_refused() {
        let r = rule(Aggregation::Sum, None);
        let unordered = [s(T0 + 2 * MIN, 1.0), s(T0 + MIN, 1.0)];
        let late_anchor = s(T0, 1.0);
        assert_eq!(
            hourly(&r, None, &[], T0 + 1, T0 + HOUR_MS),
            Err(RollupError::Unaligned)
        );
        assert_eq!(
            hourly(&r, None, &[], T0 + HOUR_MS, T0),
            Err(RollupError::Unaligned)
        );
        assert_eq!(
            hourly(&r, None, &unordered, T0, T0 + HOUR_MS),
            Err(RollupError::Unordered)
        );
        assert_eq!(
            hourly(&r, Some(&late_anchor), &[], T0, T0 + HOUR_MS),
            Err(RollupError::Unordered)
        );
        let kolkata = TimeZone::get("Asia/Kolkata").unwrap();
        let hours = hourly(&r, None, &[], T0, T0 + HOUR_MS).unwrap();
        assert_eq!(
            group(&hours, Period::Day, &kolkata),
            Err(RollupError::OffHour("day"))
        );
        let gap = [
            hours[0].clone(),
            hourly(&r, None, &[], T0 + 2 * HOUR_MS, T0 + 3 * HOUR_MS).unwrap()[0].clone(),
        ];
        assert_eq!(
            group(&gap, Period::Day, &TimeZone::UTC),
            Err(RollupError::Mismatch)
        );
    }

    #[test]
    fn means_near_the_float_limits_neither_overflow_nor_turn_nan() {
        let r = rule(Aggregation::TimeWeighted, None);
        let close = |a: f64, b: f64| (a - b).abs() <= b.abs() * 1e-12;
        let half = |a: f64, b: f64| [s(T0, a), s(T0 + 30 * MIN, b)];
        let h = &hourly(&r, None, &half(1e308, 2e307), T0, T0 + HOUR_MS).unwrap()[0];
        assert!(close(tw(h).0, 6e307), "{:?}", tw(h));
        let h = &hourly(&r, None, &half(1e308, -1e308), T0, T0 + HOUR_MS).unwrap()[0];
        assert_eq!(tw(h), (0.0, -1e308, 1e308));
        // Each hour is fine alone; the day must not multiply by its weight.
        let samples = [s(T0, 1e301), s(T0 + 12 * HOUR_MS, 2e301)];
        let hours = hourly(&r, None, &samples, T0, T0 + 24 * HOUR_MS).unwrap();
        let day = &group(&hours, Period::Day, &TimeZone::UTC).unwrap()[0];
        assert!(close(tw(day).0, 1.5e301), "{:?}", tw(day));
    }

    #[test]
    fn an_unrepresentable_sum_is_an_error_not_a_complete_hour() {
        let r = rule(Aggregation::Sum, None);
        let samples = [s(T0, 1e308), s(T0 + 1, 1e308)];
        let hour = hourly(&r, None, &samples, T0, T0 + HOUR_MS);
        assert_eq!(hour, Err(RollupError::Overflow));
        let samples = [s(T0, 1e308), s(T0 + HOUR_MS, 1e308)];
        let hours = hourly(&r, None, &samples, T0, T0 + 2 * HOUR_MS).unwrap();
        let day = group(&hours, Period::Day, &TimeZone::UTC);
        assert_eq!(day, Err(RollupError::Overflow));
    }

    #[test]
    fn a_period_starting_before_the_epoch_is_refused_not_truncated() {
        // 1970-01-01 in Paris (UTC+01:00) starts at -1 h.
        let paris = TimeZone::get("Europe/Paris").unwrap();
        let r = rule(Aggregation::TimeWeighted, None);
        let hours = hourly(&r, None, &[s(0, 1.0)], 0, 23 * HOUR_MS).unwrap();
        for period in [Period::Day, Period::Month] {
            let refused = group(&hours, period, &paris);
            assert!(matches!(refused, Err(RollupError::Time(_))), "{refused:?}");
        }
    }

    #[test]
    fn hours_of_different_rules_are_not_grouped() {
        let count = rule(Aggregation::Count, None);
        let sum = rule(Aggregation::Sum, None);
        let before = hourly(&count, None, &[s(T0 - HOUR_MS, 1)], T0 - HOUR_MS, T0).unwrap();
        let after = hourly(&sum, None, &[s(T0, 1.0)], T0, T0 + HOUR_MS).unwrap();
        let mixed = [before, after].concat();
        assert_eq!(
            group(&mixed, Period::Day, &TimeZone::UTC),
            Err(RollupError::Mismatch)
        );
    }

    /// Tiny deterministic generator: no dependency for one test.
    struct Lcg(u64);

    impl Lcg {
        fn below(&mut self, n: u64) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 33) % n
        }
    }

    #[test]
    fn property_coverage_bounded_mean_between_min_and_max_and_chaining_exact() {
        let mut rng = Lcg(7);
        let end = T0 + 6 * HOUR_MS;
        for _ in 0..500 {
            let mut ts = T0 - HOUR_MS;
            let samples: Vec<_> = (0..rng.below(40))
                .map(|_| {
                    ts += rng.below(40 * MIN);
                    let v = match rng.below(5) {
                        0 => Value::Null,
                        1 => Value::Int(i64::try_from(rng.below(10)).unwrap()),
                        #[allow(clippy::cast_precision_loss)]
                        _ => Value::Float(rng.below(10_000) as f64 / 7.0 - 500.0),
                    };
                    (ts, v)
                })
                .collect();
            let freshness =
                [None, Some(10 * MIN), Some(2 * HOUR_MS)][usize::try_from(rng.below(3)).unwrap()];
            for aggregation in [
                Aggregation::TimeWeighted,
                Aggregation::Sum,
                Aggregation::Count,
                Aggregation::Durations,
            ] {
                let r = rule(aggregation, freshness);
                let hours = hourly(&r, None, &samples, T0, end).unwrap();
                let covered: u64 = hours.iter().map(|h| h.covered_ms).sum();
                assert!(covered <= end - T0);
                let mut groups = group(&hours, Period::Day, &TimeZone::UTC).unwrap();
                groups.extend(hours.iter().cloned());
                for h in &groups {
                    assert!(h.covered_ms <= h.end - h.start);
                    if let Some(Aggregate::TimeWeighted { mean, min, max }) = h.aggregate {
                        assert!(min <= mean && mean <= max, "{min} {mean} {max}");
                    }
                    if let Some(Aggregate::Durations { ms }) = &h.aggregate {
                        assert_eq!(ms.values().sum::<u64>(), h.covered_ms);
                    }
                }
                // Rolling in two steps, anchored on the first step's last
                // sample, gives the same hours.
                let mid = T0 + 3 * HOUR_MS;
                let first = hourly(&r, None, &samples, T0, mid).unwrap();
                let anchor = first.last().unwrap().last.clone();
                let split = samples.partition_point(|s| s.0 < mid);
                let second = hourly(&r, anchor.as_ref(), &samples[split..], mid, end).unwrap();
                assert_eq!([first, second].concat(), hours);
            }
        }
    }
}
