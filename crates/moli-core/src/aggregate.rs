//! Pure bounded aggregation. Input timestamps are observations, never execution times.
use crate::{PointId, Sample, Unit, Value};
use serde::{Deserialize, Serialize};

pub const MAX_AGGREGATE_SOURCES: usize = 32;
pub const MAX_FRESHNESS_S: u64 = 7 * 24 * 60 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregateOp {
    Min,
    Max,
    Mean,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AggregateSpec {
    pub operation: AggregateOp,
    pub sources: Vec<PointId>,
    pub unit: Option<Unit>,
    pub max_age_s: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidAggregate;

#[derive(Clone, Debug)]
pub struct AggregateSource {
    pub sample: Option<Sample>,
    pub unit: Option<Unit>,
    pub online: bool,
    pub numeric: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AggregateResult {
    pub sample: Option<Sample>,
    pub next: Option<u64>,
    pub valid: usize,
    /// Bit positions of contributing inputs (at most 32).
    pub used: u32,
}

/// Only explicit ratios among known units; unknown symbols require exact equality.
#[must_use]
pub fn unit_ratio(from: Option<&Unit>, to: Option<&Unit>) -> Option<f64> {
    if from == to {
        return Some(1.0);
    }
    match (from?, to?) {
        (Unit::WattHour, Unit::KiloWattHour) | (Unit::MilliVolt, Unit::Volt) => Some(0.001),
        (Unit::KiloWattHour, Unit::WattHour) | (Unit::Volt, Unit::MilliVolt) => Some(1000.0),
        (Unit::Second, Unit::Minute) | (Unit::Minute, Unit::Hour) => Some(1.0 / 60.0),
        (Unit::Minute, Unit::Second) | (Unit::Hour, Unit::Minute) => Some(60.0),
        (Unit::Second, Unit::Hour) => Some(1.0 / 3600.0),
        (Unit::Hour, Unit::Second) => Some(3600.0),
        _ => None,
    }
}

impl AggregateSpec {
    pub fn normalize(mut self) -> Result<Self, InvalidAggregate> {
        if self.sources.len() > MAX_AGGREGATE_SOURCES
            || !(1..=MAX_FRESHNESS_S).contains(&self.max_age_s)
            || self.unit.as_ref().is_some_and(|u| {
                u.symbol().is_empty()
                    || u.symbol().len() > 32
                    || u.symbol().chars().any(char::is_control)
            })
            || self.sources.iter().any(|p| {
                p.as_str().len() > 256
                    || p.as_str().chars().any(char::is_control)
                    || crate::privacy::code_point(p.as_str())
                    || p.split().is_none_or(|(d, k)| {
                        d.as_str().split_once(':').is_none_or(|(instance, native)| {
                            instance.is_empty() || native.is_empty() || native.contains(':')
                        }) || k.is_empty()
                    })
            })
        {
            return Err(InvalidAggregate);
        }
        self.sources.sort();
        if self.sources.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(InvalidAggregate);
        }
        Ok(self)
    }

    /// Inputs correspond to the normalized source order. Empty/invalid inputs
    /// produce no sample. Freshness is [observed, observed + max_age).
    pub fn evaluate(&self, sources: &[AggregateSource], now: u64) -> AggregateResult {
        let mut result = AggregateResult {
            sample: None,
            next: None,
            valid: 0,
            used: 0,
        };
        if sources.len() > MAX_AGGREGATE_SOURCES || !(1..=MAX_FRESHNESS_S).contains(&self.max_age_s)
        {
            return result;
        }
        let mut values = [0.0_f64; MAX_AGGREGATE_SOURCES];
        let mut oldest = u64::MAX;
        for (index, source) in sources.iter().enumerate() {
            if !source.numeric || !source.online {
                continue;
            }
            let Some(sample) = &source.sample else {
                continue;
            };
            let Some(ratio) = unit_ratio(source.unit.as_ref(), self.unit.as_ref()) else {
                continue;
            };
            let Some(value) = sample
                .value
                .as_f64()
                .filter(|n| n.is_finite())
                .map(|n| n * ratio)
                .filter(|n| n.is_finite())
            else {
                continue;
            };
            let Some(expires) = sample.ts.checked_add(self.max_age_s * 1000) else {
                continue;
            };
            if now >= expires {
                continue;
            }
            let wake = if sample.ts > now { sample.ts } else { expires };
            result.next = Some(result.next.map_or(wake, |previous| previous.min(wake)));
            if sample.ts > now {
                continue;
            }
            result.used |= 1_u32 << index;
            values[result.valid] = value;
            result.valid += 1;
            oldest = oldest.min(sample.ts);
        }
        if result.valid == 0 {
            return result;
        }
        let values = &values[..result.valid];
        let value = match self.operation {
            AggregateOp::Min => values.iter().copied().fold(f64::INFINITY, f64::min),
            AggregateOp::Max => values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            AggregateOp::Mean => scaled_mean(values),
        };
        result.sample = Some(Sample {
            value: Value::Float(value),
            ts: oldest,
        });
        result
    }
}

fn scaled_mean(values: &[f64]) -> f64 {
    let scale = values.iter().map(|n| n.abs()).fold(0.0_f64, f64::max);
    if scale == 0.0 {
        return 0.0;
    }
    let (mut sum, mut correction) = (0.0, 0.0);
    for value in values {
        let adjusted = value / scale - correction;
        let next = sum + adjusted;
        correction = (next - sum) - adjusted;
        sum = next;
    }
    let count = f64::from(u32::try_from(values.len()).expect("bounded aggregate sources"));
    scale * (sum / count).clamp(-1.0, 1.0)
}
