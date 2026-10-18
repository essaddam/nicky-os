//! Receipt/publication orchestration, without network or device access.
use crate::{
    manifest::{self, Manifest, Part, Pointer, Snapshot},
    parquet::{self, PartWriter, Record},
    protocol::{self, Catalog, Status},
    receipt::{self, ReceiptLog},
};
use anyhow::{Context, Result, bail, ensure};
use ring::rand::{SecureRandom, SystemRandom};
use rustix::fs::{FlockOperation, flock};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Weak},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicationFailure {
    PartSynced,
    PartRenamed,
    ManifestSynced,
    ManifestRenamed,
    PointerSynced,
    PointerRenamed,
    ReceiptsUpdated,
    EpochIntentSynced,
    EpochManifestPublished,
    EpochIdentityPublished,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    installation: String,
    generation: i64,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EpochIntent {
    identity: Identity,
    manifest: Manifest,
}
fn fresh_generation(previous: i64) -> Result<i64> {
    loop {
        let mut bytes = [0; 8];
        SystemRandom::new()
            .fill(&mut bytes)
            .map_err(|_| anyhow::anyhow!("random_unavailable"))?;
        let value = (i64::from_le_bytes(bytes) & i64::MAX).max(1);
        if value != previous {
            return Ok(value);
        }
    }
}
fn recover_epoch(root: &Path, installation: &str) -> Result<()> {
    let path = root.join("epoch-change.json");
    if !path.exists() {
        return Ok(());
    }
    let intent: EpochIntent = manifest::read_json(&path, manifest::MAX_MANIFEST + 1024)?;
    ensure!(
        intent.identity.installation == installation
            && intent.manifest.installation == installation
            && intent.identity.generation == intent.manifest.generation,
        "invalid_epoch_intent"
    );
    intent.manifest.validate(root)?;
    let bytes = serde_json::to_vec(&intent.manifest)?;
    receipt::atomic_write(&intent.manifest.path(root), &bytes)?;
    receipt::atomic_write(
        &root.join("current.json"),
        &serde_json::to_vec(&Pointer {
            revision: intent.manifest.revision,
            hash: receipt::sha256(&bytes),
        })?,
    )?;
    receipt::atomic_write(
        &root.join("identity.json"),
        &serde_json::to_vec(&intent.identity)?,
    )?;
    fs::remove_file(path)?;
    receipt::sync_dir(root)
}
#[derive(Debug)]
pub struct Archive {
    pub(crate) root: Arc<PathBuf>,
    pub(crate) log: ReceiptLog,
    pub(crate) current: Arc<Manifest>,
    pub(crate) readers: Vec<Weak<Manifest>>,
    failure: Option<PublicationFailure>,
    pub(crate) poisoned: bool,
    pub(crate) verified: Arc<crate::verified::Verified>,
}

fn ensure_dir(path: &Path) -> Result<()> {
    if path.is_dir() {
        return Ok(());
    }
    let parent = path.parent().context("missing_directory_parent")?;
    ensure_dir(parent)?;
    fs::create_dir(path)?;
    receipt::sync_dir(parent)
}
impl Archive {
    pub fn open(path: &Path, installation: &str) -> Result<Self> {
        protocol::installation(installation)?;
        let root = std::path::absolute(path)?;
        ensure_dir(&root)?;
        ensure_dir(&root.join(".staging"))?;
        ensure_dir(&root.join("journal"))?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join("writer.lock"))?;
        flock(&lock, FlockOperation::NonBlockingLockExclusive).context("archive_already_open")?;
        for entry in fs::read_dir(root.join(".staging"))? {
            fs::remove_file(entry?.path())?;
        }
        recover_epoch(&root, installation)?;
        let identity_path = root.join("identity.json");
        let identity: Identity = if identity_path.exists() {
            manifest::read_json(&identity_path, 512)?
        } else {
            ensure!(
                !root.join("current.json").exists()
                    && !root.join("receipts.bin").exists()
                    && !has_data_partitions(&root),
                "missing_archive_identity"
            );
            let generation = fresh_generation(0)?;
            let identity = Identity {
                installation: installation.into(),
                generation,
            };
            receipt::atomic_write(&identity_path, &serde_json::to_vec(&identity)?)?;
            identity
        };
        ensure!(
            identity.installation == installation && identity.generation > 0,
            "installation_mismatch"
        );
        let current = if root.join("current.json").exists() {
            manifest::load(&root)?
        } else {
            ensure!(
                !root.join("receipts.bin").exists() && !has_data_partitions(&root),
                "missing_current_manifest"
            );
            let m = Manifest {
                version: 1,
                installation: installation.into(),
                generation: identity.generation,
                revision: 0,
                published_upto: 0,
                catalog: Catalog::default(),
                catalog_revision: 0,
                parts: Vec::new(),
            };
            let bytes = serde_json::to_vec(&m)?;
            receipt::atomic_write(&m.path(&root), &bytes)?;
            receipt::atomic_write(
                &root.join("current.json"),
                &serde_json::to_vec(&Pointer {
                    revision: 0,
                    hash: receipt::sha256(&bytes),
                })?,
            )?;
            m
        };
        ensure!(
            current.installation == installation && current.generation == identity.generation,
            "manifest_identity_mismatch"
        );
        let mut log = ReceiptLog::open_locked(
            &root,
            identity.generation,
            current.published_upto,
            current.revision,
            lock,
        )?;
        log.published(current.published_upto, current.revision)?;
        let current = Arc::new(current);
        let readers = vec![Arc::downgrade(&current)];
        Ok(Self {
            root: Arc::new(root),
            log,
            current,
            readers,
            failure: None,
            poisoned: false,
            verified: Arc::default(),
        })
    }
    /// Only for a restored archive while the server is stopped; an ordinary move preserves the epoch.
    pub fn rotate_epoch(&mut self) -> Result<Status> {
        ensure!(!self.poisoned, "archive_requires_reopen");
        let result = (|| {
            let mut next = (*self.current).clone();
            next.generation = fresh_generation(next.generation)?;
            next.revision = next.revision.checked_add(1).context("revision_overflow")?;
            let identity = Identity {
                installation: next.installation.clone(),
                generation: next.generation,
            };
            let intent = EpochIntent {
                identity,
                manifest: next,
            };
            let path = self.root.join("epoch-change.json");
            receipt::atomic_write(&path, &serde_json::to_vec(&intent)?)?;
            self.checkpoint(PublicationFailure::EpochIntentSynced)?;
            self.publish_manifest(intent.manifest)?;
            self.checkpoint(PublicationFailure::EpochManifestPublished)?;
            receipt::atomic_write(
                &self.root.join("identity.json"),
                &serde_json::to_vec(&intent.identity)?,
            )?;
            self.checkpoint(PublicationFailure::EpochIdentityPublished)?;
            fs::remove_file(path)?;
            receipt::sync_dir(&self.root)?;
            self.log.set_generation(intent.identity.generation)?;
            self.log
                .published(self.current.published_upto, self.current.revision)?;
            Ok(self.status())
        })();
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
    pub fn status(&self) -> Status {
        self.log.status()
    }
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            root: self.root.clone(),
            manifest: self.current.clone(),
            verified: self.verified.clone(),
        }
    }
    pub fn fail_at(&mut self, at: PublicationFailure) {
        self.failure = Some(at);
    }
    pub(crate) fn checkpoint(&self, at: PublicationFailure) -> Result<()> {
        if self.failure == Some(at) {
            bail!("injected_publication_interruption");
        }
        Ok(())
    }
    pub fn receive(&mut self, first: i64, last: i64, bytes: &[u8]) -> Result<Status> {
        ensure!(!self.poisoned, "archive_requires_reopen");
        let result = self.log.receive(first, last, bytes);
        if self.log.is_poisoned() {
            self.poisoned = true;
        }
        result
    }
    pub fn publish(&mut self) -> Result<Status> {
        ensure!(!self.poisoned, "archive_requires_reopen");
        let Some(pending) = self.log.pending(self.current.published_upto)? else {
            return Ok(self.status());
        };
        let result = (|| {
            let operations = protocol::parse(pending.first, pending.last, &pending.bytes)?;
            let revision = self
                .current
                .revision
                .checked_add(1)
                .context("revision_overflow")?;
            let mut next = (*self.current).clone();
            next.revision = revision;
            next.published_upto = pending.last;
            let mut groups: BTreeMap<(String, String), Vec<Record>> = BTreeMap::new();
            for op in &operations {
                let row = Record::from_operation(op)?;
                let group = match op {
                    protocol::Operation::EnergyHour { .. } => {
                        if self.current.parts.iter().any(|p| {
                            ["energy", "corrections"].contains(&p.group.as_str())
                                && row.ts >= p.bounds.start
                                && row.ts < p.bounds.end
                                && row.entity >= p.bounds.first
                                && row.entity <= p.bounds.last
                        }) {
                            "corrections"
                        } else {
                            "energy"
                        }
                    }
                    protocol::Operation::Sample { point, kind, .. } => {
                        if !self.current.catalog.points.iter().any(|p| p.id == *point) {
                            "other"
                        } else if matches!(kind.as_str(), "b" | "t" | "z") {
                            "events"
                        } else {
                            "numeric"
                        }
                    }
                };
                let month = jiff::Timestamp::from_millisecond(row.ts)?
                    .to_zoned(jiff::tz::TimeZone::UTC)
                    .strftime("%Y-%m")
                    .to_string();
                groups.entry((group.into(), month)).or_default().push(row);
            }
            for ((group, month), mut rows) in groups {
                rows.sort_by(|a, b| a.key().cmp(&b.key()));
                // Absolute hour snapshots within the same batch also resolve to the newest sequence.
                if group == "energy" || group == "corrections" {
                    let mut resolved: Vec<Record> = Vec::new();
                    for row in rows {
                        if resolved
                            .last()
                            .is_some_and(|p| p.entity == row.entity && p.ts == row.ts)
                        {
                            *resolved.last_mut().expect("last checked") = row;
                        } else {
                            resolved.push(row);
                        }
                    }
                    rows = resolved;
                }
                let part = self.write_part(&group, &month, revision, 0, None, rows.into_iter())?;
                next.parts.push(part);
            }
            self.update_summaries(&mut next, Some(&operations))?;
            self.compact_manifest(&mut next, false)?;
            self.publish_manifest(next)?;
            self.log.published(pending.last, revision)?;
            self.checkpoint(PublicationFailure::ReceiptsUpdated)?;
            self.collect()?;
            Ok(self.status())
        })();
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
    pub(crate) fn write_part(
        &self,
        group: &str,
        month: &str,
        revision: i64,
        level: u8,
        resolution: Option<protocol::Resolution>,
        rows: impl Iterator<Item = Record>,
    ) -> Result<Part> {
        let (year, mon) = month.split_once('-').context("invalid_partition_month")?;
        let suffix =
            resolution.map_or_else(|| "raw".into(), |r| format!("{r:?}").to_ascii_lowercase());
        let file = format!(
            "{group}/year={year}/month={mon}/part-{revision}-{level}-{suffix}-000000.parquet"
        );
        let target = self.root.join(&file);
        ensure_dir(target.parent().context("missing_part_parent")?)?;
        let stage = self.root.join(".staging/part.parquet");
        let mut writer = PartWriter::new(&stage)?;
        for row in rows {
            writer.push(row)?;
        }
        let bounds = writer.finish()?;
        self.checkpoint(PublicationFailure::PartSynced)?;
        let hash = parquet::verify(&stage, &bounds)?;
        let bytes = fs::metadata(&stage)?.len();
        fs::rename(stage, &target)?;
        receipt::sync_dir(target.parent().context("missing_part_parent")?)?;
        receipt::sync_dir(&self.root.join(".staging"))?;
        self.checkpoint(PublicationFailure::PartRenamed)?;
        self.verified.remember(&target, hash)?;
        Ok(Part {
            file,
            group: group.into(),
            month: month.into(),
            level,
            run: revision,
            shard: 0,
            resolution,
            bounds,
            bytes,
            hash,
        })
    }
    fn stage_metadata(
        &self,
        path: &Path,
        bytes: &[u8],
        synced: PublicationFailure,
        renamed: PublicationFailure,
    ) -> Result<()> {
        let stage = self.root.join(".staging/metadata.json");
        let mut f = File::create(&stage)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        self.checkpoint(synced)?;
        fs::rename(stage, path)?;
        receipt::sync_dir(&self.root)?;
        receipt::sync_dir(&self.root.join(".staging"))?;
        self.checkpoint(renamed)
    }
    pub(crate) fn publish_manifest(&mut self, next: Manifest) -> Result<()> {
        next.validate(&self.root)?;
        let bytes = serde_json::to_vec(&next)?;
        ensure!(bytes.len() <= manifest::MAX_MANIFEST, "manifest_limit");
        self.stage_metadata(
            &next.path(&self.root),
            &bytes,
            PublicationFailure::ManifestSynced,
            PublicationFailure::ManifestRenamed,
        )?;
        let pointer = Pointer {
            revision: next.revision,
            hash: receipt::sha256(&bytes),
        };
        self.stage_metadata(
            &self.root.join("current.json"),
            &serde_json::to_vec(&pointer)?,
            PublicationFailure::PointerSynced,
            PublicationFailure::PointerRenamed,
        )?;
        self.current = Arc::new(next);
        self.readers.retain(|w| w.strong_count() > 0);
        self.readers.push(Arc::downgrade(&self.current));
        receipt::atomic_write(
            &self.root.join("catalog.json"),
            &serde_json::to_vec(&self.current.catalog)?,
        )?;
        Ok(())
    }
}

fn has_data_partitions(root: &Path) -> bool {
    [
        "numeric",
        "events",
        "energy",
        "stats",
        "corrections",
        "other",
    ]
    .iter()
    .any(|name| root.join(name).exists())
}
