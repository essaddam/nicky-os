//! Immutable manifest snapshots. The durable current pointer is the publication boundary.
use crate::{
    parquet::{self, Bounds},
    protocol::{Catalog, Query, QueryResponse, QuerySeries, Resolution, Source},
    receipt::sha256,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

pub const MAX_MANIFEST: usize = 4 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Part {
    pub file: String,
    pub group: String,
    pub month: String,
    pub level: u8,
    pub run: i64,
    pub shard: u32,
    pub resolution: Option<Resolution>,
    pub bounds: Bounds,
    pub bytes: u64,
    pub hash: [u8; 32],
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: u32,
    pub installation: String,
    pub generation: i64,
    pub revision: i64,
    pub published_upto: i64,
    pub catalog: Catalog,
    pub catalog_revision: i64,
    pub parts: Vec<Part>,
}
impl Manifest {
    pub fn validate(&self, root: &Path) -> Result<()> {
        ensure!(
            self.version == 1
                && self.generation > 0
                && self.revision >= 0
                && self.published_upto >= 0,
            "invalid_manifest"
        );
        crate::protocol::installation(&self.installation)?;
        self.catalog.validate()?;
        let mut files = BTreeSet::new();
        for p in &self.parts {
            ensure!(
                files.insert(&p.file)
                    && p.run > 0
                    && p.run <= self.revision
                    && p.bounds.rows > 0
                    && p.bounds.start >= 0
                    && p.bounds.end > p.bounds.start
                    && p.bounds.min_seq > 0
                    && p.bounds.max_seq > 0
                    && p.level <= 32,
                "invalid_manifest_part"
            );
            ensure!(
                p.file
                    .split('/')
                    .all(|s| !s.is_empty() && s != "." && s != "..")
                    && !p.file.starts_with('/')
                    && p.file.ends_with(".parquet"),
                "invalid_part_path"
            );
            ensure!(
                [
                    "numeric",
                    "events",
                    "energy",
                    "stats",
                    "corrections",
                    "other"
                ]
                .contains(&p.group.as_str())
                    && p.file.starts_with(&format!("{}/", p.group)),
                "invalid_part_group"
            );
            ensure!(
                p.bytes > 0
                    && fs::metadata(root.join(&p.file))
                        .context("missing_published_part")?
                        .len()
                        == p.bytes,
                "part_size_mismatch"
            );
        }
        Ok(())
    }
    pub fn path(&self, root: &Path) -> PathBuf {
        root.join(format!("manifest-{}.json", self.revision))
    }
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pointer {
    pub revision: i64,
    pub hash: [u8; 32],
}

pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path, limit: usize) -> Result<T> {
    use std::io::Read;
    let f = fs::File::open(path)?;
    ensure!(
        f.metadata()?.len() <= u64::try_from(limit)?,
        "metadata_limit"
    );
    let mut bytes = Vec::new();
    f.take(u64::try_from(limit)? + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= limit, "metadata_limit");
    Ok(serde_json::from_slice(&bytes)?)
}
pub fn load(root: &Path) -> Result<Manifest> {
    let pointer: Pointer = read_json(&root.join("current.json"), 256)?;
    ensure!(pointer.revision >= 0, "invalid_revision");
    let path = root.join(format!("manifest-{}.json", pointer.revision));
    ensure!(
        fs::metadata(&path)?.len() <= u64::try_from(MAX_MANIFEST)?,
        "manifest_limit"
    );
    let bytes = fs::read(path)?;
    ensure!(sha256(&bytes) == pointer.hash, "corrupt_manifest");
    let m: Manifest = serde_json::from_slice(&bytes)?;
    ensure!(m.revision == pointer.revision, "manifest_revision_mismatch");
    m.validate(root)?;
    Ok(m)
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub(crate) root: Arc<PathBuf>,
    pub(crate) manifest: Arc<Manifest>,
    pub(crate) verified: Arc<crate::verified::Verified>,
}
impl Snapshot {
    pub fn manifest_path(&self) -> PathBuf {
        self.manifest.path(&self.root)
    }
    pub fn part_paths(&self) -> Vec<PathBuf> {
        self.manifest
            .parts
            .iter()
            .map(|p| self.root.join(&p.file))
            .collect()
    }
    pub fn revision(&self) -> i64 {
        self.manifest.revision
    }
    pub fn catalog(&self) -> &Catalog {
        &self.manifest.catalog
    }
    pub fn query(&self, q: &Query) -> Result<QueryResponse> {
        q.validate()?;
        validate_resolution(q, &self.manifest.catalog)?;
        let wanted: Vec<String> = q
            .points
            .iter()
            .filter(|id| {
                q.resolution == Resolution::Raw
                    || q.source == Source::Energy
                    || self
                        .manifest
                        .catalog
                        .points
                        .iter()
                        .any(|p| p.id == **id && p.aggregation.is_some())
            })
            .map(|id| {
                if q.resolution == Resolution::Raw {
                    id.clone()
                } else {
                    format!(
                        "{}{id}",
                        match q.source {
                            Source::Samples => "s:",
                            Source::Energy => "e:",
                        }
                    )
                }
            })
            .collect();
        let values = self.query_records(q, &wanted)?;
        let mut series = Vec::new();
        for id in &q.points {
            let mut rows = Vec::new();
            for ((point, _, _), row) in &values {
                if point == id {
                    rows.push(serde_json::from_str(&row.payload)?);
                }
            }
            let coverage = if rows.is_empty() {
                "unknown"
            } else {
                match q.resolution {
                    Resolution::Raw => "observed",
                    _ => "per_value",
                }
            };
            series.push(QuerySeries {
                point: id.clone(),
                coverage: coverage.into(),
                values: rows,
            });
        }
        let response = QueryResponse {
            published_upto: self.manifest.published_upto,
            generation: self.manifest.generation,
            revision: self.manifest.revision,
            from: q.from,
            to: q.to,
            series,
        };
        ensure!(
            serde_json::to_vec(&response)?.len() <= 1024 * 1024,
            "query_response_limit"
        );
        Ok(response)
    }
    fn query_records(&self, q: &Query, wanted: &[String]) -> Result<QueryRows> {
        let selection: Arc<parquet::Selection> = Arc::new(
            wanted
                .iter()
                .map(|id| (id.clone(), (q.from, q.to)))
                .collect(),
        );
        let started = Instant::now();
        let mut values = BTreeMap::new();
        let mut result_bytes = 0usize;
        let mut scanned = 0u64;
        let mut input_bytes = 0u64;
        for part in self.manifest.parts.iter().filter(|p| {
            let group = match q.resolution {
                Resolution::Raw => match q.source {
                    Source::Samples => ["numeric", "events", "other"].contains(&p.group.as_str()),
                    Source::Energy => ["energy", "corrections"].contains(&p.group.as_str()),
                },
                r => p.group == "stats" && p.resolution == Some(r),
            };
            group
                && p.bounds.start < q.to
                && p.bounds.end > q.from
                && wanted
                    .iter()
                    .any(|id| id >= &p.bounds.first && id <= &p.bounds.last)
        }) {
            input_bytes = input_bytes
                .checked_add(part.bytes)
                .context("query_work_limit")?;
            ensure!(input_bytes <= 64 * 1024 * 1024, "query_work_limit");
            let path = self.root.join(&part.file);
            self.verified.verify(&path, part.hash)?;
            let mut reader = parquet::PartReader::filtered(&path, Some(selection.clone()))?;
            while let Some(row) = reader.next_record()? {
                scanned += 1;
                ensure!(
                    scanned <= 1_000_000 && started.elapsed() < Duration::from_secs(2),
                    "query_work_limit"
                );
                if row.ts < q.from || row.ts >= q.to || !wanted.contains(&row.entity) {
                    continue;
                }
                save_row(
                    row,
                    q,
                    self.manifest.catalog_revision,
                    &mut values,
                    &mut result_bytes,
                )?;
            }
        }

        Ok(values)
    }
}

type QueryRows = BTreeMap<(String, i64, i64), parquet::Record>;
fn save_row(
    row: parquet::Record,
    q: &Query,
    catalog_revision: i64,
    values: &mut QueryRows,
    result_bytes: &mut usize,
) -> Result<()> {
    if q.resolution != Resolution::Raw {
        // Stats for samples and energy are disjoint within their payload namespace.
        let payload: serde_json::Value = serde_json::from_str(&row.payload)?;
        let source = payload.get("source").and_then(serde_json::Value::as_str);
        if payload
            .get("catalog_revision")
            .and_then(serde_json::Value::as_i64)
            != Some(catalog_revision)
        {
            return Ok(());
        }
        if source
            != Some(match q.source {
                Source::Samples => "samples",
                Source::Energy => "energy",
            })
        {
            return Ok(());
        }
    }
    let version = if q.source == Source::Samples && q.resolution == Resolution::Raw {
        row.seq
    } else {
        0
    };
    let id = if q.resolution == Resolution::Raw {
        row.entity.clone()
    } else {
        row.entity
            .get(2..)
            .context("invalid_summary_namespace")?
            .to_owned()
    };
    let key = (id, row.ts, version);
    let replace = values
        .get(&key)
        .is_none_or(|old: &parquet::Record| old.seq < row.seq);
    if replace {
        let previous = values.get(&key).map_or(0, parquet::Record::bytes);
        let next_bytes = result_bytes
            .saturating_sub(previous)
            .checked_add(row.bytes())
            .context("query_response_limit")?;
        ensure!(next_bytes <= 1024 * 1024, "query_response_limit");
        *result_bytes = next_bytes;
        values.insert(key, row);
    }
    ensure!(values.len() <= q.max_points, "query_result_limit");
    Ok(())
}

fn validate_resolution(q: &Query, catalog: &Catalog) -> Result<()> {
    match q.resolution {
        Resolution::Raw => {}
        Resolution::Hour => ensure!(
            q.from % crate::protocol::HOUR_MS == 0 && q.to % crate::protocol::HOUR_MS == 0,
            "unaligned_query_range"
        ),
        Resolution::Day | Resolution::Month => {
            let tz = jiff::tz::TimeZone::get(&catalog.timezone)?;
            let period = if q.resolution == Resolution::Day {
                moli_rollup::Period::Day
            } else {
                moli_rollup::Period::Month
            };
            for edge in [q.from, q.to] {
                ensure!(
                    moli_rollup::period_bounds(u64::try_from(edge)?, period, &tz)?.0
                        == u64::try_from(edge)?,
                    "unaligned_query_range"
                );
            }
        }
    }
    Ok(())
}
