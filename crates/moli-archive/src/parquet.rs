//! Small Parquet row groups; no Arrow, analytical engine or unbounded column buffers.
use crate::{protocol::Operation, receipt::sha256};
use anyhow::{Context, Result, ensure};
use parquet::{
    basic::Compression,
    data_type::{ByteArray, ByteArrayType, DoubleType, Int64Type},
    file::{
        properties::{EnabledStatistics, WriterProperties},
        reader::{FileReader, SerializedFileReader},
        writer::SerializedFileWriter,
    },
    record::{Field, Row, RowAccessor},
    schema::{
        parser::parse_message_type,
        types::{ColumnPath, Type},
    },
};
use ring::digest::{Context as Digest, SHA256};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
    sync::Arc,
};

const SCHEMA: &str = "message archive_v1 { REQUIRED BINARY entity (UTF8); REQUIRED INT64 ts; REQUIRED INT64 seq; REQUIRED BINARY kind (UTF8); REQUIRED BINARY payload (UTF8); OPTIONAL DOUBLE num; OPTIONAL DOUBLE cost; REQUIRED INT64 covered_ms; }";
const GROUP_ROWS: usize = 256;
const GROUP_BYTES: usize = 256 * 1024;
pub const PART_ROWS: u64 = 65536;
pub const PART_BYTES: usize = 32 * 1024 * 1024;

fn schema() -> Result<Arc<Type>> {
    Ok(Arc::new(parse_message_type(SCHEMA)?))
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub entity: String,
    pub ts: i64,
    pub seq: i64,
    pub kind: String,
    pub payload: String,
    pub num: Option<f64>,
    pub cost: Option<f64>,
    pub covered_ms: i64,
}
impl Record {
    pub fn key(&self) -> (&str, i64, i64) {
        (&self.entity, self.ts, self.seq)
    }
    pub fn bytes(&self) -> usize {
        self.entity.len() + self.payload.len() + 64
    }
    pub fn from_operation(op: &Operation) -> Result<Self> {
        let (entity, ts, kind, num, cost) = match op {
            Operation::Sample { point, ts, num, .. } => (
                point.clone(),
                *ts,
                "s",
                num.as_ref().and_then(serde_json::Number::as_f64),
                None,
            ),
            Operation::EnergyHour {
                meter,
                hour,
                kwh,
                cost,
                ..
            } => (meter.clone(), *hour, "e", Some(*kwh), *cost),
        };
        Ok(Self {
            entity,
            ts,
            seq: op.seq(),
            kind: kind.into(),
            payload: serde_json::to_string(op)?,
            num,
            cost,
            covered_ms: 0,
        })
    }
    fn validate(&self) -> Result<()> {
        crate::protocol::identifier(&self.entity)?;
        ensure!(
            self.ts >= 0 && self.seq > 0 && self.payload.len() <= 64 * 1024 && self.covered_ms >= 0,
            "invalid_parquet_record"
        );
        ensure!(
            self.num.is_none_or(f64::is_finite) && self.cost.is_none_or(f64::is_finite),
            "nonfinite_parquet_value"
        );
        if matches!(self.kind.as_str(), "s" | "e") {
            let op: Operation = serde_json::from_str(&self.payload)?;
            op.validate()?;
            ensure!(
                Self::from_operation(&op)? == *self,
                "parquet_payload_mismatch"
            );
        } else {
            ensure!(
                ["hour", "day", "month"].contains(&self.kind.as_str()),
                "invalid_record_kind"
            );
            serde_json::from_str::<serde_json::Value>(&self.payload)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub rows: u64,
    pub start: i64,
    pub end: i64,
    pub first: String,
    pub last: String,
    pub min_seq: i64,
    pub max_seq: i64,
    pub energy_sum: f64,
    pub known_cost_sum: f64,
    pub unknown_costs: u64,
    pub covered_ms: i64,
}
impl Default for Bounds {
    fn default() -> Self {
        Self {
            rows: 0,
            start: i64::MAX,
            end: 0,
            first: String::new(),
            last: String::new(),
            min_seq: i64::MAX,
            max_seq: 0,
            energy_sum: 0.0,
            known_cost_sum: 0.0,
            unknown_costs: 0,
            covered_ms: 0,
        }
    }
}
impl Bounds {
    fn add(&mut self, r: &Record) -> Result<()> {
        if self.rows == 0 {
            self.first.clone_from(&r.entity);
        }
        self.last.clone_from(&r.entity);
        self.rows += 1;
        self.start = self.start.min(r.ts);
        self.end = self
            .end
            .max(r.ts.checked_add(1).context("timestamp_overflow")?);
        self.min_seq = self.min_seq.min(r.seq);
        self.max_seq = self.max_seq.max(r.seq);
        if r.kind == "e" {
            self.energy_sum += r.num.context("missing_energy")?;
            if let Some(cost) = r.cost {
                self.known_cost_sum += cost;
            } else {
                self.unknown_costs += 1;
            }
        }
        self.covered_ms = self
            .covered_ms
            .checked_add(r.covered_ms)
            .context("coverage_overflow")?;
        ensure!(
            self.energy_sum.is_finite() && self.known_cost_sum.is_finite(),
            "energy_sum_overflow"
        );
        Ok(())
    }
}

pub struct PartWriter {
    writer: SerializedFileWriter<File>,
    chunk: Vec<Record>,
    chunk_bytes: usize,
    bounds: Bounds,
    previous: Option<Record>,
}
impl std::fmt::Debug for PartWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PartWriter")
            .field("bounds", &self.bounds)
            .finish_non_exhaustive()
    }
}
impl PartWriter {
    pub fn new(path: &Path) -> Result<Self> {
        let props = WriterProperties::builder()
            .set_compression(Compression::SNAPPY)
            .set_dictionary_enabled(false)
            .set_column_statistics_enabled(ColumnPath::from("payload"), EnabledStatistics::None)
            .set_data_page_size_limit(64 * 1024)
            .set_write_batch_size(GROUP_ROWS)
            .build();
        Ok(Self {
            writer: SerializedFileWriter::new(File::create(path)?, schema()?, Arc::new(props))?,
            chunk: Vec::new(),
            chunk_bytes: 0,
            bounds: Bounds::default(),
            previous: None,
        })
    }
    pub fn push(&mut self, r: Record) -> Result<()> {
        r.validate()?;
        ensure!(
            self.previous.as_ref().is_none_or(|p| p.key() < r.key()),
            "unsorted_or_duplicate_parquet_row"
        );
        self.bounds.add(&r)?;
        self.previous = Some(r.clone());
        self.chunk_bytes += r.bytes();
        self.chunk.push(r);
        if self.chunk.len() >= GROUP_ROWS || self.chunk_bytes >= GROUP_BYTES {
            self.flush()?;
        }
        Ok(())
    }
    fn flush(&mut self) -> Result<()> {
        if self.chunk.is_empty() {
            return Ok(());
        }
        let mut group = self.writer.next_row_group()?;
        macro_rules! required {
            ($typ:ty,$values:expr) => {{
                let mut c = group.next_column()?.context("missing_column")?;
                c.typed::<$typ>().write_batch(&$values, None, None)?;
                c.close()?;
            }};
        }
        macro_rules! optional {
            ($field:ident) => {{
                let defs: Vec<i16> = self
                    .chunk
                    .iter()
                    .map(|r| i16::from(r.$field.is_some()))
                    .collect();
                let values: Vec<f64> = self.chunk.iter().filter_map(|r| r.$field).collect();
                let mut c = group.next_column()?.context("missing_column")?;
                c.typed::<DoubleType>()
                    .write_batch(&values, Some(&defs), None)?;
                c.close()?;
            }};
        }
        required!(
            ByteArrayType,
            self.chunk
                .iter()
                .map(|r| ByteArray::from(r.entity.as_str()))
                .collect::<Vec<_>>()
        );
        required!(
            Int64Type,
            self.chunk.iter().map(|r| r.ts).collect::<Vec<_>>()
        );
        required!(
            Int64Type,
            self.chunk.iter().map(|r| r.seq).collect::<Vec<_>>()
        );
        required!(
            ByteArrayType,
            self.chunk
                .iter()
                .map(|r| ByteArray::from(r.kind.as_str()))
                .collect::<Vec<_>>()
        );
        required!(
            ByteArrayType,
            self.chunk
                .iter()
                .map(|r| ByteArray::from(r.payload.as_str()))
                .collect::<Vec<_>>()
        );
        optional!(num);
        optional!(cost);
        required!(
            Int64Type,
            self.chunk.iter().map(|r| r.covered_ms).collect::<Vec<_>>()
        );
        group.close()?;
        self.chunk.clear();
        self.chunk_bytes = 0;
        Ok(())
    }
    pub fn finish(mut self) -> Result<Bounds> {
        self.flush()?;
        ensure!(self.bounds.rows > 0, "empty_part");
        let f = self.writer.into_inner()?;
        f.sync_all()?;
        Ok(self.bounds)
    }
}

pub type Selection = std::collections::BTreeMap<String, (i64, i64)>;
pub struct PartReader {
    reader: SerializedFileReader<File>,
    group: usize,
    rows: std::vec::IntoIter<Record>,
    selection: Option<Arc<Selection>>,
}
impl std::fmt::Debug for PartReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PartReader")
            .field("group", &self.group)
            .finish_non_exhaustive()
    }
}
fn record(row: &Row) -> Result<Record> {
    let optional = |i| -> Result<Option<f64>> {
        if matches!(
            row.get_column_iter().nth(i).map(|(_, f)| f),
            Some(Field::Null)
        ) {
            Ok(None)
        } else {
            Ok(Some(row.get_double(i)?))
        }
    };
    Ok(Record {
        entity: row.get_string(0)?.clone(),
        ts: row.get_long(1)?,
        seq: row.get_long(2)?,
        kind: row.get_string(3)?.clone(),
        payload: row.get_string(4)?.clone(),
        num: optional(5)?,
        cost: optional(6)?,
        covered_ms: row.get_long(7)?,
    })
}
fn open_reader(path: &Path) -> Result<SerializedFileReader<File>> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    ensure!(len >= 12, "invalid_parquet_file");
    file.seek(SeekFrom::End(-8))?;
    let mut footer = [0; 8];
    file.read_exact(&mut footer)?;
    let bytes = u64::from(u32::from_le_bytes(footer[..4].try_into()?));
    ensure!(
        &footer[4..] == b"PAR1" && bytes <= 1024 * 1024 && bytes + 8 < len,
        "parquet_footer_limit"
    );
    let reader = SerializedFileReader::new(file)?;
    ensure!(
        reader
            .metadata()
            .file_metadata()
            .schema_descr()
            .root_schema()
            == schema()?.as_ref(),
        "parquet_schema_mismatch"
    );
    for g in reader.metadata().row_groups() {
        ensure!(
            g.num_rows() > 0
                && g.num_rows() <= i64::try_from(GROUP_ROWS)?
                && g.total_byte_size() <= 1024 * 1024,
            "parquet_group_limit"
        );
    }
    Ok(reader)
}
fn group_bounds(
    reader: &SerializedFileReader<File>,
    g: usize,
) -> Option<(String, String, i64, i64)> {
    use parquet::file::statistics::Statistics;
    let m = reader.metadata().row_group(g);
    let Statistics::ByteArray(ids) = m.column(0).statistics()? else {
        return None;
    };
    let Statistics::Int64(times) = m.column(1).statistics()? else {
        return None;
    };
    Some((
        ids.min_opt()?.as_utf8().ok()?.into(),
        ids.max_opt()?.as_utf8().ok()?.into(),
        *times.min_opt()?,
        *times.max_opt()?,
    ))
}
impl PartReader {
    pub fn new(path: &Path) -> Result<Self> {
        Self::filtered(path, None)
    }
    pub fn filtered(path: &Path, selection: Option<Arc<Selection>>) -> Result<Self> {
        Ok(Self {
            reader: open_reader(path)?,
            group: 0,
            rows: Vec::new().into_iter(),
            selection,
        })
    }
    pub fn next_record(&mut self) -> Result<Option<Record>> {
        loop {
            if let Some(row) = self.rows.next() {
                if self.selection.as_ref().is_none_or(|s| {
                    s.get(&row.entity)
                        .is_some_and(|(a, b)| row.ts >= *a && row.ts < *b)
                }) {
                    return Ok(Some(row));
                }
                continue;
            }
            if self.group >= self.reader.num_row_groups() {
                return Ok(None);
            }
            let g = self.group;
            self.group += 1;
            if let (Some(selection), Some((first, last, lo, hi))) =
                (&self.selection, group_bounds(&self.reader, g))
                && !selection
                    .range(first..=last)
                    .any(|(_, (a, b))| lo < *b && hi >= *a)
            {
                continue;
            }
            let group = self.reader.get_row_group(g)?;
            let rows = group
                .get_row_iter(None)?
                .map(|r| record(&r?))
                .collect::<Result<Vec<_>>>()?;
            self.rows = rows.into_iter();
        }
    }
}
#[derive(Debug)]
pub struct Neighbors {
    pub start: i64,
    pub last_new: i64,
    pub anchor: Option<Record>,
    pub next: Option<i64>,
}
/// Looks up all affected points together; row-group bounds avoid N separate scans.
pub fn neighbors(
    paths: &[std::path::PathBuf],
    wanted: &mut std::collections::BTreeMap<String, Neighbors>,
) -> Result<()> {
    for path in paths {
        let reader = open_reader(path)?;
        for g in (0..reader.num_row_groups()).rev() {
            if let Some((first, last, lo, hi)) = group_bounds(&reader, g) {
                let needed = wanted.range(first..=last).any(|(_, w)| {
                    (lo < w.start && w.anchor.as_ref().is_none_or(|a| hi >= a.ts))
                        || (hi > w.last_new && w.next.is_none_or(|n| lo <= n))
                });
                if !needed {
                    continue;
                }
            }
            let group = reader.get_row_group(g)?;
            for r in group.get_row_iter(None)? {
                let row = record(&r?)?;
                let Some(w) = wanted.get_mut(&row.entity) else {
                    continue;
                };
                if row.ts < w.start
                    && w.anchor
                        .as_ref()
                        .is_none_or(|a| (row.ts, row.seq) > (a.ts, a.seq))
                {
                    w.anchor = Some(row.clone());
                }
                if row.ts > w.last_new && w.next.is_none_or(|n| row.ts < n) {
                    w.next = Some(row.ts);
                }
            }
        }
    }
    Ok(())
}

pub fn hash_file(path: &Path) -> Result<[u8; 32]> {
    let mut f = File::open(path)?;
    let mut hash = Digest::new(&SHA256);
    let mut buf = vec![0; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    Ok(hash.finish().as_ref().try_into()?)
}
pub fn verify(path: &Path, expected: &Bounds) -> Result<[u8; 32]> {
    let mut reader = PartReader::new(path)?;
    let mut bounds = Bounds::default();
    let mut prev: Option<Record> = None;
    while let Some(r) = reader.next_record()? {
        r.validate()?;
        ensure!(
            prev.as_ref().is_none_or(|p| p.key() < r.key()),
            "invalid_part_order"
        );
        bounds.add(&r)?;
        prev = Some(r);
    }
    ensure!(bounds == *expected, "part_bounds_mismatch");
    let hash = hash_file(path)?;
    ensure!(hash != sha256(&[]), "empty_part_hash");
    Ok(hash)
}
