//! Finite cache of verified immutable files; a changed file stamp forces a fresh hash.
use anyhow::{Result, ensure};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Mutex,
    time::SystemTime,
};
#[derive(Debug, Clone, PartialEq, Eq)]
struct Stamp {
    bytes: u64,
    modified: SystemTime,
    hash: [u8; 32],
}
#[derive(Debug, Default)]
pub(crate) struct Verified(Mutex<BTreeMap<PathBuf, Stamp>>);
impl Verified {
    fn stamp(path: &Path, hash: [u8; 32]) -> Result<Stamp> {
        let m = std::fs::metadata(path)?;
        Ok(Stamp {
            bytes: m.len(),
            modified: m.modified()?,
            hash,
        })
    }
    pub fn verify(&self, path: &Path, hash: [u8; 32]) -> Result<()> {
        let stamp = Self::stamp(path, hash)?;
        if self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(path)
            == Some(&stamp)
        {
            return Ok(());
        }
        ensure!(crate::parquet::hash_file(path)? == hash, "corrupt_part");
        self.remember(path, hash)
    }
    pub fn remember(&self, path: &Path, hash: [u8; 32]) -> Result<()> {
        let stamp = Self::stamp(path, hash)?;
        let mut cache = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if cache.len() >= 16384 {
            cache.clear();
        }
        cache.insert(path.into(), stamp);
        Ok(())
    }
}
