//! Bounded D13 transport, independent of the Parquet representation.
use anyhow::{Result, ensure};
use flate2::bufread::GzDecoder;
use serde::{Deserialize, Serialize};
use serde_json::Number;
use std::io::Read;

pub const MAX_CATALOG_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_COMPRESSED: usize = 1024 * 1024;
pub const MAX_DECOMPRESSED: usize = 8 * 1024 * 1024;
pub const MAX_OPERATIONS: i64 = 5000;
pub const HOUR_MS: i64 = 3_600_000;
/// Upper end of the supported civil-calendar domain (year 9999).
pub const MAX_TIMESTAMP: i64 = 253_402_300_799_999;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Status {
    pub durable_upto: i64,
    pub published_upto: i64,
    pub generation: i64,
    #[serde(skip)]
    pub revision: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Sample {
        seq: i64,
        point: String,
        ts: i64,
        kind: String,
        num: Option<Number>,
        txt: Option<String>,
    },
    EnergyHour {
        seq: i64,
        meter: String,
        hour: i64,
        kwh: f64,
        cost: Option<f64>,
    },
}
impl Operation {
    pub fn seq(&self) -> i64 {
        match self {
            Self::Sample { seq, .. } | Self::EnergyHour { seq, .. } => *seq,
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(self.seq() > 0, "invalid_sequence");
        match self {
            Self::Sample {
                point,
                ts,
                kind,
                num,
                txt,
                ..
            } => {
                identifier(point)?;
                ensure!((0..=MAX_TIMESTAMP).contains(ts), "invalid_timestamp");
                let scalar = num.as_ref().and_then(Number::as_f64);
                let valid = match kind.as_str() {
                    "b" => txt.is_none() && matches!(scalar, Some(0.0 | 1.0)),
                    "i" => {
                        txt.is_none()
                            && num.as_ref().is_some_and(|n| {
                                n.as_i64().is_some()
                                    || n.as_f64().is_some_and(|f| {
                                        f.is_finite()
                                            && f.fract() == 0.0
                                            && f.abs() <= 9_007_199_254_740_992.0
                                    })
                            })
                    }
                    "f" => txt.is_none() && scalar.is_some_and(f64::is_finite),
                    "t" => num.is_none() && txt.as_ref().is_some_and(|s| s.len() <= 4096),
                    "z" => num.is_none() && txt.is_none(),
                    _ => false,
                };
                ensure!(valid, "invalid_scalar");
            }
            Self::EnergyHour {
                meter,
                hour,
                kwh,
                cost,
                ..
            } => {
                identifier(meter)?;
                ensure!(
                    (0..=MAX_TIMESTAMP).contains(hour) && hour % HOUR_MS == 0,
                    "invalid_energy_hour"
                );
                ensure!(
                    kwh.is_finite() && cost.is_none_or(f64::is_finite),
                    "invalid_energy_value"
                );
            }
        }
        Ok(())
    }
}

pub fn identifier(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control),
        "invalid_identifier"
    );
    Ok(())
}

pub fn installation(id: &str) -> Result<()> {
    ensure!(
        id.len() == 32
            && id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "invalid_installation"
    );
    Ok(())
}

#[derive(Debug)]
pub struct Batch {
    pub bytes: Vec<u8>,
    pub operations: Vec<Operation>,
}

pub fn parse(first: i64, last: i64, bytes: &[u8]) -> Result<Vec<Operation>> {
    ensure!(
        first > 0 && last >= first && last - first < MAX_OPERATIONS,
        "invalid_batch_range"
    );
    ensure!(
        bytes.len() <= MAX_DECOMPRESSED && bytes.last() == Some(&b'\n'),
        "invalid_ndjson_size"
    );
    let mut operations = Vec::new();
    for line in bytes[..bytes.len() - 1].split(|b| *b == b'\n') {
        ensure!(
            operations.len() < usize::try_from(MAX_OPERATIONS)?,
            "too_many_operations"
        );
        let op: Operation = serde_json::from_slice(line)?;
        op.validate()?;
        ensure!(
            op.seq() == first + i64::try_from(operations.len())?,
            "noncontiguous_batch"
        );
        operations.push(op);
    }
    ensure!(
        i64::try_from(operations.len())? == last - first + 1,
        "batch_range_mismatch"
    );
    Ok(operations)
}

pub fn decode(first: i64, last: i64, compressed: &[u8]) -> Result<Batch> {
    ensure!(compressed.len() <= MAX_COMPRESSED, "compressed_limit");
    let mut decoder = GzDecoder::new(compressed);
    let mut bytes = Vec::new();
    (&mut decoder)
        .take(u64::try_from(MAX_DECOMPRESSED)? + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= MAX_DECOMPRESSED, "decompressed_limit");
    ensure!(decoder.into_inner().is_empty(), "trailing_gzip_data");
    let operations = parse(first, last, &bytes)?;
    Ok(Batch { bytes, operations })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    #[default]
    Samples,
    Energy,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Resolution {
    Raw,
    Hour,
    Day,
    Month,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Query {
    #[serde(default)]
    pub source: Source,
    pub points: Vec<String>,
    pub from: i64,
    pub to: i64,
    pub resolution: Resolution,
    pub max_points: usize,
}
impl Query {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.points.is_empty()
                && self.points.len() <= 128
                && self.max_points > 0
                && self.max_points <= 5000,
            "invalid_query_limit"
        );
        ensure!(
            self.from >= 0 && self.to > self.from && self.to <= MAX_TIMESTAMP,
            "invalid_query_range"
        );
        let mut seen = std::collections::BTreeSet::new();
        for p in &self.points {
            identifier(p)?;
            ensure!(seen.insert(p), "duplicate_query_point");
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PointRule {
    pub id: String,
    pub family: String,
    pub unit: Option<String>,
    pub aggregation: Option<moli_rollup::Aggregation>,
    pub freshness_ms: Option<u64>,
    pub origin: moli_rollup::Origin,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Meter {
    pub id: String,
    pub origin: moli_rollup::Origin,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub timezone: String,
    pub points: Vec<PointRule>,
    #[serde(default)]
    pub meters: Vec<Meter>,
}
impl Default for Catalog {
    fn default() -> Self {
        Self {
            timezone: "UTC".into(),
            points: Vec::new(),
            meters: Vec::new(),
        }
    }
}
impl Catalog {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            serde_json::to_vec(self)?.len() <= MAX_CATALOG_BYTES
                && self.points.len() <= 4096
                && self.meters.len() <= 1024,
            "catalog_limit"
        );
        jiff::tz::TimeZone::get(&self.timezone)?;
        let mut seen = std::collections::BTreeSet::new();
        for p in &self.points {
            identifier(&p.id)?;
            ensure!(seen.insert(&p.id), "duplicate_catalog_point");
            ensure!(
                [
                    "power",
                    "climate",
                    "energy",
                    "states",
                    "diagnostic",
                    "other"
                ]
                .contains(&p.family.as_str()),
                "invalid_family"
            );
            ensure!(
                p.family != "energy" || p.aggregation != Some(moli_rollup::Aggregation::Sum),
                "cannot_sum_cumulative_index"
            );
            ensure!(p.unit.as_ref().is_none_or(|u| u.len() <= 64), "unit_limit");
        }
        seen.clear();
        for m in &self.meters {
            identifier(&m.id)?;
            ensure!(seen.insert(&m.id), "duplicate_catalog_meter");
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogUpdate {
    pub expected_revision: i64,
    pub catalog: Catalog,
}
#[derive(Debug, Serialize)]
pub struct QuerySeries {
    pub point: String,
    pub coverage: String,
    pub values: Vec<serde_json::Value>,
}
#[derive(Debug, Serialize)]
pub struct QueryResponse {
    pub published_upto: i64,
    pub generation: i64,
    pub revision: i64,
    pub from: i64,
    pub to: i64,
    pub series: Vec<QuerySeries>,
}
