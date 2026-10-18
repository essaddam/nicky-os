//! Partitioned writers, leveled compaction and collection of unreferenced generations.
use crate::{
    Archive, PublicationFailure, Snapshot,
    manifest::{Manifest, Part},
    merge::ExternalMerge,
    parquet::{self, PartWriter, Record},
    protocol::Resolution,
    receipt,
};
use anyhow::{Context, Result, ensure};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
};

struct Spool {
    writer: std::io::BufWriter<std::fs::File>,
    path: PathBuf,
    month: String,
    rows: u64,
    hash: ring::digest::Context,
}
impl std::fmt::Debug for Spool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Spool")
            .field("month", &self.month)
            .field("rows", &self.rows)
            .finish_non_exhaustive()
    }
}
#[derive(Debug)]
struct Output {
    writer: PartWriter,
    stage: PathBuf,
    file: String,
    month: String,
    shard: u32,
    rows: u64,
    bytes: usize,
}
#[derive(Debug)]
pub(crate) struct RecordSink<'a> {
    archive: &'a Archive,
    group: &'static str,
    tag: &'static str,
    resolution: Option<Resolution>,
    revision: i64,
    level: u8,
    spools: BTreeMap<String, Spool>,
    parts: Vec<Part>,
}
impl<'a> RecordSink<'a> {
    pub fn new(
        archive: &'a Archive,
        group: &'static str,
        tag: &'static str,
        resolution: Option<Resolution>,
        revision: i64,
        level: u8,
    ) -> Self {
        Self {
            archive,
            group,
            tag,
            resolution,
            revision,
            level,
            spools: BTreeMap::new(),
            parts: Vec::new(),
        }
    }
    fn suffix(&self) -> String {
        self.resolution
            .map_or_else(|| "raw".into(), |r| format!("{r:?}").to_ascii_lowercase())
    }
    pub fn push(&mut self, row: &Record) -> Result<()> {
        use std::io::Write;
        let month = jiff::Timestamp::from_millisecond(row.ts)?
            .to_zoned(jiff::tz::TimeZone::UTC)
            .strftime("%Y-%m")
            .to_string();
        if !self.spools.contains_key(&month) {
            ensure!(self.spools.len() < 128, "open_partition_limit");
            let path = self.archive.root.join(".staging").join(format!(
                "spool-{}-{}-{}-{month}.ndjson",
                self.group,
                self.suffix(),
                self.tag
            ));
            let writer =
                std::io::BufWriter::with_capacity(16 * 1024, std::fs::File::create(&path)?);
            self.spools.insert(
                month.clone(),
                Spool {
                    writer,
                    path,
                    month: month.clone(),
                    rows: 0,
                    hash: ring::digest::Context::new(&ring::digest::SHA256),
                },
            );
        }
        let bytes = serde_json::to_vec(row)?;
        ensure!(bytes.len() < 1024 * 1024, "spool_record_limit");
        let spool = self.spools.get_mut(&month).expect("spool created");
        spool.writer.write_all(&bytes)?;
        spool.writer.write_all(b"\n")?;
        spool.hash.update(&bytes);
        spool.hash.update(b"\n");
        spool.rows += 1;
        Ok(())
    }
    fn output(&self, month: &str, shard: u32) -> Result<Output> {
        let (year, mon) = month.split_once('-').context("partition_month")?;
        let suffix = self.suffix();
        let tag = self.tag;
        let file = format!(
            "{}/year={year}/month={mon}/part-{}-{}-{suffix}-{tag}-{shard:06}.parquet",
            self.group, self.revision, self.level
        );
        let stage = self.archive.root.join(".staging").join(format!(
            "part-{}-{suffix}-{tag}-{month}-{shard}.parquet",
            self.group
        ));
        let writer = PartWriter::new(&stage)?;
        Ok(Output {
            writer,
            stage,
            file,
            month: month.into(),
            shard,
            rows: 0,
            bytes: 0,
        })
    }
    fn seal(&mut self, output: Output) -> Result<()> {
        let bounds = output.writer.finish()?;
        self.archive.checkpoint(PublicationFailure::PartSynced)?;
        let hash = parquet::verify(&output.stage, &bounds)?;
        let bytes = fs::metadata(&output.stage)?.len();
        let target = self.archive.root.join(&output.file);
        let parent = target.parent().context("part_parent")?;
        fs::create_dir_all(parent)?;
        for dir in parent
            .ancestors()
            .take_while(|p| *p != self.archive.root.as_path())
        {
            receipt::sync_dir(dir)?;
            if let Some(p) = dir.parent() {
                receipt::sync_dir(p)?;
            }
        }
        fs::rename(&output.stage, &target)?;
        receipt::sync_dir(parent)?;
        receipt::sync_dir(&self.archive.root.join(".staging"))?;
        self.archive.checkpoint(PublicationFailure::PartRenamed)?;
        self.archive.verified.remember(&target, hash)?;
        self.parts.push(Part {
            file: output.file,
            group: self.group.into(),
            month: output.month,
            level: self.level,
            run: self.revision,
            shard: output.shard,
            resolution: self.resolution,
            bounds,
            bytes,
            hash,
        });
        Ok(())
    }
    fn finish_partition(&mut self, spool: Spool) -> Result<()> {
        use std::io::{BufRead, Read, Write};
        let Spool {
            mut writer,
            path,
            month,
            rows,
            hash,
        } = spool;
        writer.flush()?;
        drop(writer);
        let expected = hash.finish();
        let mut actual = ring::digest::Context::new(&ring::digest::SHA256);
        let mut count = 0u64;
        let mut reader = std::io::BufReader::new(std::fs::File::open(&path)?);
        let mut line = Vec::new();
        let mut output: Option<Output> = None;
        let mut shard = 0;
        let mut previous: Option<(String, i64, i64)> = None;
        loop {
            line.clear();
            let n = (&mut reader)
                .take(1024 * 1024 + 1)
                .read_until(b'\n', &mut line)?;
            if n == 0 {
                break;
            }
            ensure!(
                n <= 1024 * 1024 && line.last() == Some(&b'\n'),
                "spool_record_limit"
            );
            actual.update(&line);
            let row: Record = serde_json::from_slice(&line)?;
            let key = (row.entity.clone(), row.ts, row.seq);
            ensure!(
                previous.as_ref().is_none_or(|old| old < &key),
                "invalid_spool_order"
            );
            previous = Some(key);
            if output.is_none() {
                output = Some(self.output(&month, shard)?);
            }
            let active = output.as_mut().expect("output created");
            active.rows += 1;
            active.bytes += row.bytes();
            active.writer.push(row)?;
            count += 1;
            if active.rows >= parquet::PART_ROWS || active.bytes >= parquet::PART_BYTES {
                self.seal(output.take().expect("active output"))?;
                shard += 1;
            }
        }
        if let Some(output) = output {
            self.seal(output)?;
        }
        ensure!(
            count == rows && actual.finish().as_ref() == expected.as_ref(),
            "corrupt_partition_spool"
        );
        fs::remove_file(path)?;
        Ok(())
    }
    pub fn finish(mut self) -> Result<Vec<Part>> {
        for (_, spool) in std::mem::take(&mut self.spools) {
            self.finish_partition(spool)?;
        }
        Ok(self.parts)
    }
}
impl Archive {
    pub(crate) fn verify_parts(&self, parts: &[Part]) -> Result<()> {
        for p in parts {
            self.verified.verify(&self.root.join(&p.file), p.hash)?;
        }
        Ok(())
    }
    pub(crate) fn compact_manifest(&self, m: &mut Manifest, force: bool) -> Result<()> {
        loop {
            let mut groups: CompactionRuns = BTreeMap::new();
            for p in &m.parts {
                groups
                    .entry((
                        p.group.clone(),
                        p.month.clone(),
                        p.resolution,
                        if force { 0 } else { p.level },
                    ))
                    .or_default()
                    .insert((p.run, p.level));
            }
            let Some(((group, month, resolution, level), runs)) = groups
                .into_iter()
                .find(|(_, runs)| runs.len() >= if force { 2 } else { 4 })
            else {
                break;
            };
            let selected: BTreeSet<_> = runs
                .into_iter()
                .take(if force { usize::MAX } else { 4 })
                .collect();
            let inputs: Vec<_> = m
                .parts
                .iter()
                .filter(|p| {
                    p.group == group
                        && p.month == month
                        && p.resolution == resolution
                        && (force || p.level == level)
                        && selected.contains(&(p.run, p.level))
                })
                .cloned()
                .collect();
            let next_level = inputs
                .iter()
                .map(|p| p.level)
                .max()
                .unwrap_or(0)
                .checked_add(1)
                .context("compaction_level_limit")?;
            ensure!(next_level <= 32, "compaction_level_limit");
            let resolve = matches!(group.as_str(), "energy" | "corrections" | "stats");
            self.verify_parts(&inputs)?;
            let merge = ExternalMerge::new(&self.root, &inputs, None, resolve, "compact")?;
            let group_static = match group.as_str() {
                "numeric" => "numeric",
                "events" => "events",
                "energy" => "energy",
                "corrections" => "corrections",
                "stats" => "stats",
                _ => "other",
            };
            let mut sink = RecordSink::new(
                self,
                group_static,
                "merged",
                resolution,
                m.revision,
                next_level,
            );
            for row in merge {
                let row = row?;
                if group == "stats" {
                    let payload: serde_json::Value = serde_json::from_str(&row.payload)?;
                    if payload
                        .get("catalog_revision")
                        .and_then(serde_json::Value::as_i64)
                        != Some(m.catalog_revision)
                    {
                        continue;
                    }
                }
                sink.push(&row)?;
            }
            let outputs = sink.finish()?;
            let files: BTreeSet<_> = inputs.iter().map(|p| &p.file).collect();
            m.parts.retain(|p| !files.contains(&p.file));
            m.parts.extend(outputs);
        }
        Ok(())
    }
    pub fn compact(&mut self) -> Result<crate::protocol::Status> {
        ensure!(!self.poisoned, "archive_requires_reopen");
        let result = (|| {
            let mut next = (*self.current).clone();
            next.revision = next.revision.checked_add(1).context("revision_overflow")?;
            self.compact_manifest(&mut next, true)?;
            self.publish_manifest(next)?;
            self.log
                .published(self.current.published_upto, self.current.revision)?;
            Ok(self.status())
        })();
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
    /// Future backup callers persist a pin before copying a generation, then release it.
    pub fn pin_current(&self, label: &str) -> Result<Snapshot> {
        pin_label(label)?;
        let dir = self.root.join("pins");
        fs::create_dir_all(&dir)?;
        receipt::sync_dir(&self.root)?;
        ensure!(fs::read_dir(&dir)?.count() < 16, "backup_pin_limit");
        let path = dir.join(format!("{label}.json"));
        ensure!(!path.exists(), "backup_pin_exists");
        receipt::atomic_write(&path, &serde_json::to_vec(self.current.as_ref())?)?;
        Ok(self.snapshot())
    }
    pub fn release_pin(&self, label: &str) -> Result<()> {
        pin_label(label)?;
        let dir = self.root.join("pins");
        fs::remove_file(dir.join(format!("{label}.json")))?;
        receipt::sync_dir(&dir)
    }
    pub fn collect(&mut self) -> Result<()> {
        ensure!(!self.poisoned, "archive_requires_reopen");
        let result = (|| {
            self.readers.retain(|w| w.strong_count() > 0);
            let mut files = BTreeSet::new();
            let mut manifests = BTreeSet::new();
            let mut mark = |m: &Manifest| {
                manifests.insert(m.revision);
                for p in &m.parts {
                    files.insert(p.file.clone());
                }
            };
            mark(&self.current);
            for reader in &self.readers {
                if let Some(m) = reader.upgrade() {
                    mark(&m);
                }
            }
            let pins = self.root.join("pins");
            if pins.exists() {
                for entry in fs::read_dir(pins)? {
                    let m: Manifest =
                        crate::manifest::read_json(&entry?.path(), crate::manifest::MAX_MANIFEST)?;
                    m.validate(&self.root)?;
                    mark(&m);
                }
            }
            for group in [
                "numeric",
                "events",
                "energy",
                "stats",
                "corrections",
                "other",
            ] {
                let dir = self.root.join(group);
                if !dir.exists() {
                    continue;
                }
                for year in fs::read_dir(&dir)? {
                    for month in fs::read_dir(year?.path())? {
                        let month = month?.path();
                        for entry in fs::read_dir(&month)? {
                            let path = entry?.path();
                            let relative = path
                                .strip_prefix(self.root.as_path())?
                                .to_str()
                                .context("invalid_part_name")?;
                            if !files.contains(relative) {
                                fs::remove_file(path)?;
                            }
                        }
                        receipt::sync_dir(&month)?;
                    }
                }
            }
            for entry in fs::read_dir(self.root.as_path())? {
                let entry = entry?;
                let name = entry.file_name();
                let Some(revision) = name
                    .to_str()
                    .and_then(|s| s.strip_prefix("manifest-"))
                    .and_then(|s| s.strip_suffix(".json"))
                    .and_then(|s| s.parse::<i64>().ok())
                else {
                    continue;
                };
                if !manifests.contains(&revision) {
                    fs::remove_file(entry.path())?;
                }
            }
            receipt::sync_dir(&self.root)
        })();
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
}
fn pin_label(label: &str) -> Result<()> {
    ensure!(
        !label.is_empty()
            && label.len() <= 64
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
        "invalid_backup_pin"
    );
    Ok(())
}

type Partition = (String, String, Option<Resolution>, u8);
type CompactionRuns = BTreeMap<Partition, BTreeSet<(i64, u8)>>;
