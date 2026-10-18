//! Durable contiguous receipts; unpublished payloads are kept until publication.
use crate::protocol::{self, Status};
use anyhow::{Context, Result, bail, ensure};
use ring::digest::{SHA256, digest};
use rustix::fs::{FlockOperation, flock};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

const RECORD_BYTES: u64 = 64;
const HEADER_BYTES: u64 = 64;
const MAX_INDEX_BYTES: u64 = 8 * 1024 * 1024;
const RECEIPT_TTL: u64 = 7 * 24 * 3600;
const MAX_PENDING_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    PayloadSynced,
    PayloadRenamed,
    IndexWritten,
    IndexSynced,
}

#[derive(Debug)]
pub struct Pending {
    pub first: i64,
    pub last: i64,
    pub bytes: Vec<u8>,
}

#[derive(Debug)]
struct Receipt {
    first: i64,
    last: i64,
    hash: [u8; 32],
    published_at: u64,
}
impl Receipt {
    fn encode(&self) -> [u8; 64] {
        let mut b = [0; 64];
        b[..8].copy_from_slice(&self.first.to_le_bytes());
        b[8..16].copy_from_slice(&self.last.to_le_bytes());
        b[16..48].copy_from_slice(&self.hash);
        b[48..56].copy_from_slice(&self.published_at.to_le_bytes());
        let crc = crc32fast::hash(&b[..60]);
        b[60..].copy_from_slice(&crc.to_le_bytes());
        b
    }
    fn decode(b: &[u8; 64]) -> Result<Self> {
        ensure!(
            crc32fast::hash(&b[..60]).to_le_bytes() == b[60..],
            "corrupt_receipt_checksum"
        );
        let integer =
            |i: usize| i64::from_le_bytes(b[i..i + 8].try_into().expect("fixed receipt field"));
        let r = Self {
            first: integer(0),
            last: integer(8),
            hash: b[16..48].try_into()?,
            published_at: u64::from_le_bytes(b[48..56].try_into()?),
        };
        ensure!(
            r.first > 0
                && r.last >= r.first
                && r.last - r.first < protocol::MAX_OPERATIONS
                && b[56..60] == [0; 4],
            "corrupt_receipt_bounds"
        );
        Ok(r)
    }
}

fn header(floor: i64) -> [u8; 64] {
    let mut b = [0; 64];
    b[..8].copy_from_slice(b"MOLIRCP1");
    b[8..16].copy_from_slice(&floor.to_le_bytes());
    let hash = sha256(&b[..32]);
    b[32..].copy_from_slice(&hash);
    b
}
fn read_header(index: &mut File) -> Result<i64> {
    index.rewind()?;
    let mut b = [0; 64];
    index.read_exact(&mut b)?;
    ensure!(
        &b[..8] == b"MOLIRCP1" && sha256(&b[..32]) == b[32..] && b[16..32] == [0; 16],
        "corrupt_receipt_header"
    );
    let floor = i64::from_le_bytes(b[8..16].try_into()?);
    ensure!(floor >= 0, "invalid_receipt_floor");
    Ok(floor)
}

pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    digest(&SHA256, bytes)
        .as_ref()
        .try_into()
        .expect("SHA256 length")
}

pub fn sync_dir(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

/// Atomic replacement inside one filesystem, including both rename directories.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("missing_parent")?;
    fs::create_dir_all(parent)?;
    let staged = path.with_extension("staged");
    let mut f = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&staged)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    fs::rename(staged, path)?;
    sync_dir(parent)
}

#[derive(Debug)]
pub struct ReceiptLog {
    dir: PathBuf,
    index: File,
    _lock: File,
    floor: i64,
    max_records: u64,
    records: u64,
    status: Status,
    pending_bytes: u64,
    poisoned: bool,
    failure: Option<Failure>,
}
impl ReceiptLog {
    pub fn open(dir: &Path, generation: i64) -> Result<Self> {
        Self::open_at(dir, generation, 0, 0)
    }
    pub fn open_at(dir: &Path, generation: i64, published: i64, revision: i64) -> Result<Self> {
        fs::create_dir_all(dir.join("journal"))?;
        fs::create_dir_all(dir.join(".staging"))?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(dir.join("writer.lock"))?;
        flock(&lock, FlockOperation::NonBlockingLockExclusive).context("archive_already_open")?;
        Self::open_locked(dir, generation, published, revision, lock)
    }
    pub(crate) fn open_locked(
        dir: &Path,
        generation: i64,
        published: i64,
        revision: i64,
        lock: File,
    ) -> Result<Self> {
        ensure!(
            generation > 0 && published >= 0 && revision >= 0,
            "invalid_status"
        );
        let index_path = dir.join("receipts.bin");
        if !index_path.exists() {
            atomic_write(&index_path, &header(0))?;
        }
        let mut index = OpenOptions::new().read(true).write(true).open(index_path)?;
        let len = index.metadata()?.len();
        ensure!(
            (HEADER_BYTES..=MAX_INDEX_BYTES).contains(&len),
            "receipt_index_limit"
        );
        let floor = read_header(&mut index)?;
        ensure!(floor <= published, "receipt_floor_ahead_of_manifest");
        let records = (len - HEADER_BYTES) / RECORD_BYTES;
        if !(len - HEADER_BYTES).is_multiple_of(RECORD_BYTES) {
            index.set_len(HEADER_BYTES + records * RECORD_BYTES)?;
            index.sync_all()?;
        }
        let mut log = Self {
            dir: dir.into(),
            index,
            _lock: lock,
            floor,
            max_records: (MAX_INDEX_BYTES - HEADER_BYTES) / RECORD_BYTES,
            records,
            status: Status {
                durable_upto: floor,
                published_upto: published,
                generation,
                revision,
            },
            pending_bytes: 0,
            poisoned: false,
            failure: None,
        };
        for offset in 0..records {
            let r = match log.read(offset) {
                Ok(r) => r,
                Err(_) if offset + 1 == records => {
                    log.index.set_len(HEADER_BYTES + offset * RECORD_BYTES)?;
                    log.index.sync_all()?;
                    log.records -= 1;
                    break;
                }
                Err(e) => return Err(e),
            };
            ensure!(
                r.first
                    == log
                        .status
                        .durable_upto
                        .checked_add(1)
                        .context("sequence_overflow")?,
                "receipt_gap"
            );
            if r.last > published {
                ensure!(r.first > published, "publication_not_batch_boundary");
                let bytes = log.payload(&r)?;
                log.pending_bytes += u64::try_from(bytes.len())?;
            }
            log.status.durable_upto = r.last;
        }
        ensure!(
            published <= log.status.durable_upto,
            "manifest_ahead_of_receipts"
        );
        ensure!(
            log.pending_bytes <= MAX_PENDING_BYTES,
            "pending_journal_full"
        );
        // Never adopt an unacknowledged file. The producer still owns its replay.
        for entry in fs::read_dir(dir.join("journal"))? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_str().context("invalid_journal_name")?;
            let Some((first, last)) = name
                .strip_suffix(".ndjson")
                .and_then(|s| s.split_once('-'))
                .and_then(|(a, b)| Some((a.parse::<i64>().ok()?, b.parse::<i64>().ok()?)))
            else {
                bail!("invalid_journal_name");
            };
            let retained = log
                .find(first)?
                .is_some_and(|r| r.last == last && r.last > published);
            if !retained {
                fs::remove_file(entry.path())?;
            }
        }
        sync_dir(&dir.join("journal"))?;
        Ok(log)
    }
    pub fn status(&self) -> Status {
        self.status
    }
    pub(crate) fn set_generation(&mut self, generation: i64) -> Result<()> {
        ensure!(generation > 0, "invalid_generation");
        self.status.generation = generation;
        Ok(())
    }
    pub(crate) fn is_poisoned(&self) -> bool {
        self.poisoned
    }
    /// Lower the receipt quota; never increases the compiled resource ceiling.
    pub fn restrict_receipts(&mut self, maximum: u64) -> Result<()> {
        ensure!(
            maximum > 0 && maximum <= self.max_records && self.records <= maximum,
            "invalid_receipt_quota"
        );
        self.max_records = maximum;
        Ok(())
    }
    pub fn durable_upto(&self) -> i64 {
        self.status.durable_upto
    }
    /// Test injection returns an I/O-like error and poisons further writes.
    pub fn fail_at(&mut self, failure: Failure) {
        self.failure = Some(failure);
    }
    fn checkpoint(&mut self, at: Failure) -> Result<()> {
        if self.failure == Some(at) {
            bail!("injected_interruption");
        }
        Ok(())
    }
    fn read(&mut self, n: u64) -> Result<Receipt> {
        self.index
            .seek(SeekFrom::Start(HEADER_BYTES + n * RECORD_BYTES))?;
        let mut b = [0; 64];
        self.index.read_exact(&mut b)?;
        Receipt::decode(&b)
    }
    fn find(&mut self, first: i64) -> Result<Option<Receipt>> {
        let (mut lo, mut hi) = (0, self.records);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let r = self.read(mid)?;
            if r.first < first {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        if lo == self.records {
            return Ok(None);
        }
        let r = self.read(lo)?;
        Ok((r.first == first).then_some(r))
    }
    fn path(&self, first: i64, last: i64) -> PathBuf {
        self.dir
            .join("journal")
            .join(format!("{first}-{last}.ndjson"))
    }
    fn payload(&self, r: &Receipt) -> Result<Vec<u8>> {
        let f = File::open(self.path(r.first, r.last)).context("missing_pending_payload")?;
        ensure!(
            f.metadata()?.len() <= u64::try_from(protocol::MAX_DECOMPRESSED)?,
            "pending_payload_limit"
        );
        let mut bytes = Vec::new();
        f.take(u64::try_from(protocol::MAX_DECOMPRESSED)? + 1)
            .read_to_end(&mut bytes)?;
        ensure!(sha256(&bytes) == r.hash, "corrupt_pending_payload");
        protocol::parse(r.first, r.last, &bytes)?;
        Ok(bytes)
    }
    pub fn receive(&mut self, first: i64, last: i64, bytes: &[u8]) -> Result<Status> {
        ensure!(!self.poisoned, "archive_requires_reopen");
        protocol::parse(first, last, bytes)?;
        let hash = sha256(bytes);
        let found = self.find(first);
        if found.is_err() {
            self.poisoned = true;
        }
        if let Some(r) = found? {
            ensure!(r.last == last && r.hash == hash, "batch_conflict");
            return Ok(self.status);
        }
        ensure!(first > self.floor, "receipt_expired");
        if self.records >= self.max_records {
            let oldest = self.read(0)?;
            ensure!(
                oldest.last <= self.status.published_upto,
                "unpublished_receipt_quota"
            );
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs();
            if let Err(e) = self.rewrite(self.status.published_upto, now, true) {
                self.poisoned = true;
                return Err(e);
            }
        }
        ensure!(self.records < self.max_records, "receipt_index_full");
        ensure!(
            first
                == self
                    .status
                    .durable_upto
                    .checked_add(1)
                    .context("sequence_overflow")?,
            "sequence_gap"
        );
        ensure!(
            self.pending_bytes + u64::try_from(bytes.len())? <= MAX_PENDING_BYTES,
            "pending_journal_full"
        );
        let result = self.append(first, last, bytes, hash);
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
    fn append(&mut self, first: i64, last: i64, bytes: &[u8], hash: [u8; 32]) -> Result<Status> {
        let stage = self.dir.join(".staging/receipt.ndjson");
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&stage)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        self.checkpoint(Failure::PayloadSynced)?;
        fs::rename(stage, self.path(first, last))?;
        sync_dir(&self.dir.join("journal"))?;
        sync_dir(&self.dir.join(".staging"))?;
        self.checkpoint(Failure::PayloadRenamed)?;
        let r = Receipt {
            first,
            last,
            hash,
            published_at: 0,
        };
        self.index
            .seek(SeekFrom::Start(HEADER_BYTES + self.records * RECORD_BYTES))?;
        self.index.write_all(&r.encode())?;
        self.checkpoint(Failure::IndexWritten)?;
        self.index.sync_all()?;
        self.checkpoint(Failure::IndexSynced)?;
        self.records += 1;
        self.pending_bytes += u64::try_from(bytes.len())?;
        self.status.durable_upto = last;
        Ok(self.status)
    }
    pub fn pending(&mut self, after: i64) -> Result<Option<Pending>> {
        if after == self.status.durable_upto {
            return Ok(None);
        }
        let r = self
            .find(after.checked_add(1).context("sequence_overflow")?)?
            .context("pending_boundary_missing")?;
        Ok(Some(Pending {
            first: r.first,
            last: r.last,
            bytes: self.payload(&r)?,
        }))
    }
    /// Call only after the matching manifest pointer is durable.
    pub fn published(&mut self, published: i64, revision: i64) -> Result<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();
        self.published_at(published, revision, now)
    }
    /// Explicit clock makes the seven-day replay window deterministic in tests.
    pub fn published_at(&mut self, published: i64, revision: i64, now: u64) -> Result<()> {
        ensure!(
            !self.poisoned
                && now > 0
                && published >= self.status.published_upto
                && published <= self.status.durable_upto
                && revision >= self.status.revision,
            "invalid_publication"
        );
        if published < self.status.durable_upto {
            ensure!(
                self.find(published + 1)?.is_some(),
                "publication_not_batch_boundary"
            );
        }
        let old = self.status.published_upto;
        let result = self.rewrite(published, now, false);
        if result.is_err() {
            self.poisoned = true;
            return result;
        }
        self.status.published_upto = published;
        self.status.revision = revision;
        let result = (|| {
            // Some old receipts may already have expired from the new index.
            for entry in fs::read_dir(self.dir.join("journal"))? {
                let entry = entry?;
                let name = entry.file_name();
                let last = name
                    .to_str()
                    .and_then(|s| s.strip_suffix(".ndjson"))
                    .and_then(|s| s.split_once('-'))
                    .and_then(|(_, s)| s.parse::<i64>().ok())
                    .context("invalid_journal_name")?;
                if last <= published {
                    let len = entry.metadata()?.len();
                    fs::remove_file(entry.path())?;
                    if last > old {
                        self.pending_bytes = self.pending_bytes.saturating_sub(len);
                    }
                }
            }
            sync_dir(&self.dir.join("journal"))
        })();
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
    fn rewrite(&mut self, published: i64, now: u64, make_room: bool) -> Result<()> {
        let stage = self.dir.join(".staging/receipts.bin");
        let mut out = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&stage)?;
        out.write_all(&header(self.floor))?;
        let (mut floor, mut retained) = (self.floor, 0);
        for n in 0..self.records {
            let mut r = self.read(n)?;
            if r.last <= published && r.published_at == 0 {
                r.published_at = now;
            }
            let expired = r.published_at > 0 && now.saturating_sub(r.published_at) >= RECEIPT_TTL;
            if retained == 0
                && (expired
                    || (make_room
                        && n + 1 + self.max_records <= self.records + 1
                        && r.last <= published))
            {
                floor = r.last;
            } else {
                out.write_all(&r.encode())?;
                retained += 1;
            }
        }
        out.rewind()?;
        out.write_all(&header(floor))?;
        out.sync_all()?;
        fs::rename(stage, self.dir.join("receipts.bin"))?;
        sync_dir(&self.dir)?;
        sync_dir(&self.dir.join(".staging"))?;
        self.index = OpenOptions::new()
            .read(true)
            .write(true)
            .open(self.dir.join("receipts.bin"))?;
        self.floor = floor;
        self.records = retained;
        Ok(())
    }
}
