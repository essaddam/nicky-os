//! Private cold catalogue snapshots. No current values and no resident binding map.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::enum_catalogue::{CatalogRef, Pool};

const MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_BINDINGS: usize = 16_384;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

pub(super) struct Store {
    directory: PathBuf,
    path: PathBuf,
    owner: String,
    seen: AtomicBool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    owner: String,
    pool: Json,
    #[serde(deserialize_with = "read_bindings")]
    bindings: BTreeMap<String, CatalogRef>,
}

fn read_bindings<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<BTreeMap<String, CatalogRef>, D::Error> {
    struct Bindings;
    impl<'de> serde::de::Visitor<'de> for Bindings {
        type Value = BTreeMap<String, CatalogRef>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("at most 16384 unique channel references")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            let mut bindings = BTreeMap::new();
            while let Some(channel) = map.next_key::<String>()? {
                if bindings.len() == MAX_BINDINGS
                    || channel.is_empty()
                    || channel.len() > 4096
                    || channel.chars().any(char::is_control)
                {
                    return Err(serde::de::Error::custom("invalid enum binding bounds"));
                }
                let reference = map.next_value()?;
                if bindings.insert(channel, reference).is_some() {
                    return Err(serde::de::Error::custom("duplicate enum channel identity"));
                }
            }
            Ok(bindings)
        }
    }
    deserializer.deserialize_map(Bindings)
}

impl Store {
    pub(super) fn new(directory: PathBuf, owner: String) -> Result<Self> {
        ensure!(
            !owner.is_empty() && owner.len() <= 4096,
            "invalid enum snapshot owner"
        );
        fs::create_dir_all(&directory).context("enum directory creation failed")?;
        let metadata = fs::symlink_metadata(&directory).context("enum directory unavailable")?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "invalid enum directory"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            ensure!(
                metadata.uid() == rustix::process::geteuid().as_raw(),
                "enum directory owner mismatch"
            );
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        }
        let digest = Sha256::digest(owner.as_bytes());
        let filename: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
        Ok(Self {
            path: directory.join(format!("{filename}.json")),
            directory,
            owner,
            seen: AtomicBool::new(false),
        })
    }

    /// Only a missing first snapshot means an empty pool. Corrupt or unreadable
    /// history must never allocate fresh identities for an existing catalogue.
    pub(super) fn load_pool(&self) -> Result<Pool> {
        Ok(self.read()?.map_or_else(Pool::default, |(_, pool)| pool))
    }

    /// Called on cold point/catalogue creation, never on an observation.
    pub(super) fn reference(&self, channel_id: &str) -> Result<Option<CatalogRef>> {
        Ok(self
            .read()?
            .and_then(|(saved, _)| saved.bindings.get(channel_id).copied()))
    }

    fn read(&self) -> Result<Option<(Snapshot, Pool)>> {
        let metadata = match fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                ensure!(
                    !self.seen.load(Ordering::Relaxed),
                    "enum snapshot disappeared"
                );
                return Ok(None);
            }
            Err(error) => return Err(error).context("enum snapshot unavailable"),
        };
        self.seen.store(true, Ordering::Relaxed);
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "invalid enum snapshot file"
        );
        ensure!(
            metadata.len() <= MAX_BYTES as u64,
            "enum snapshot byte bound exceeded"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            ensure!(
                metadata.uid() == rustix::process::geteuid().as_raw(),
                "enum snapshot file owner mismatch"
            );
            ensure!(
                metadata.permissions().mode() & 0o077 == 0,
                "enum snapshot permissions are not private"
            );
        }
        let mut bytes = Vec::new();
        fs::File::open(&self.path)?
            .take(MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= MAX_BYTES,
            "enum snapshot byte bound exceeded"
        );
        let saved: Snapshot =
            serde_json::from_slice(&bytes).context("invalid enum store snapshot")?;
        ensure!(saved.owner == self.owner, "enum snapshot owner mismatch");
        ensure!(
            saved.bindings.len() <= MAX_BINDINGS,
            "enum binding count bound exceeded"
        );
        let pool = Pool::load(&serde_json::to_vec(&saved.pool)?)?;
        for (channel, reference) in &saved.bindings {
            ensure!(
                !channel.is_empty()
                    && channel.len() <= 4096
                    && !channel.chars().any(char::is_control),
                "invalid enum channel identity"
            );
            ensure!(
                pool.active_codes(*reference).is_some(),
                "unknown saved enum catalogue reference"
            );
        }
        Ok(Some((saved, pool)))
    }

    /// Merge persisted references with current ones; dormant surviving points
    /// keep their history without any permanent channel-to-reference map in RAM.
    pub(super) fn save(
        &self,
        pool: &mut Pool,
        known_base_ids: &BTreeSet<String>,
        current: impl IntoIterator<Item = (String, CatalogRef)>,
    ) -> Result<()> {
        let previous = self.read()?;
        let mut bindings = previous
            .as_ref()
            .map_or_else(BTreeMap::new, |(saved, _)| saved.bindings.clone());
        let survives = |channel: &str| {
            known_base_ids.contains(channel)
                || channel
                    .match_indices("-attribut-")
                    .any(|(offset, _)| known_base_ids.contains(&channel[..offset]))
        };
        bindings.retain(|channel, _| survives(channel));
        for (channel, reference) in current {
            ensure!(
                !channel.is_empty()
                    && channel.len() <= 4096
                    && !channel.chars().any(char::is_control),
                "invalid enum channel identity"
            );
            if !survives(&channel) {
                continue;
            }
            bindings.insert(channel, reference);
            ensure!(
                bindings.len() <= MAX_BINDINGS,
                "enum binding count bound exceeded"
            );
        }
        for reference in bindings.values() {
            ensure!(
                pool.active_codes(*reference).is_some(),
                "unknown enum catalogue reference"
            );
            if let Some((_, old_pool)) = &previous
                && let Some(old) = old_pool.descriptor(*reference)
            {
                ensure!(
                    pool.descriptor(*reference).as_ref() == Some(&old),
                    "enum catalogue identity was reassigned"
                );
            }
        }
        let original = pool.export()?;
        let mut committed = false;
        let result = (|| {
            pool.retain_refs(bindings.values().copied());
            let snapshot = Snapshot {
                owner: self.owner.clone(),
                pool: serde_json::from_slice(&pool.export()?)?,
                bindings,
            };
            let bytes = serde_json::to_vec(&snapshot)?;
            ensure!(
                bytes.len() <= MAX_BYTES,
                "enum snapshot byte bound exceeded"
            );
            if previous.as_ref().is_some_and(|(saved, _)| {
                saved.owner == snapshot.owner
                    && saved.pool == snapshot.pool
                    && saved.bindings == snapshot.bindings
            }) {
                return Ok(());
            }
            self.write_atomic(&bytes, &mut committed)
        })();
        if result.is_err() && !committed {
            *pool = Pool::load(&original).context("enum pool rollback failed")?;
        }
        result
    }

    fn write_atomic(&self, bytes: &[u8], committed: &mut bool) -> Result<()> {
        let temp = self.directory.join(format!(
            ".enum-{}-{}.pending",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        let mut staged = false;
        let result = (|| {
            let mut options = OpenOptions::new();
            options.create_new(true).write(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut output = options
                .open(&temp)
                .context("enum snapshot staging failed")?;
            staged = true;
            output
                .write_all(bytes)
                .context("enum snapshot write failed")?;
            output.sync_all().context("enum snapshot flush failed")?;
            fs::rename(&temp, &self.path).context("enum snapshot replacement failed")?;
            // A later durability error must not roll RAM back behind the file
            // that was already atomically installed.
            *committed = true;
            self.seen.store(true, Ordering::Relaxed);
            fs::File::open(&self.directory)?
                .sync_all()
                .context("enum directory flush failed")?;
            Ok(())
        })();
        if result.is_err() && staged {
            let _ = fs::remove_file(&temp);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enum_catalogue::Choice;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!(
                "moli-enum-store-test-{}-{}",
                std::process::id(),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            )))
        }
        fn store(&self, owner: &str) -> Store {
            Store::new(self.0.clone(), owner.into()).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn choices(names: &[&str]) -> Vec<Choice> {
        names
            .iter()
            .map(|name| Choice {
                native: (*name).into(),
                label: (*name).into(),
            })
            .collect()
    }
    fn bases() -> BTreeSet<String> {
        ["canal-fixture".into()].into()
    }

    #[test]
    fn identical_catalogue_and_bindings_never_replace_the_initial_file() {
        let fixture = Fixture::new();
        let store = fixture.store("synthetic-owner");
        let mut pool = store.load_pool().unwrap();
        let reference = pool.update(None, &choices(&["manual", "program"])).unwrap();
        store
            .save(&mut pool, &bases(), [("canal-fixture".into(), reference)])
            .unwrap();
        let open = fs::File::open(&store.path).unwrap();
        let before = open.metadata().unwrap();
        store
            .save(&mut pool, &bases(), [("canal-fixture".into(), reference)])
            .unwrap();
        let after = fs::metadata(&store.path).unwrap();
        assert_eq!(before.modified().unwrap(), after.modified().unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(before.ino(), after.ino());
        }
    }

    #[test]
    fn restart_and_label_rename_preserve_codes_and_immutable_history() {
        let fixture = Fixture::new();
        let store = fixture.store("synthetic-owner");
        let mut pool = store.load_pool().unwrap();
        assert_eq!(store.reference("canal-fixture").unwrap(), None);
        let first = pool.update(None, &choices(&["off", "heat"])).unwrap();
        store
            .save(&mut pool, &bases(), [("canal-fixture".into(), first)])
            .unwrap();
        let restarted = fixture.store("synthetic-owner");
        let mut pool = restarted.load_pool().unwrap();
        assert_eq!(restarted.reference("canal-fixture").unwrap(), Some(first));
        let changed = pool
            .update(
                Some(first),
                &[
                    Choice {
                        native: "heat".into(),
                        label: "Chauffer".into(),
                    },
                    Choice {
                        native: "cool".into(),
                        label: "Refroidir".into(),
                    },
                ],
            )
            .unwrap();
        assert_eq!(pool.encode(changed, "heat"), Some(1));
        assert_eq!(pool.encode(changed, "cool"), Some(2));
        restarted
            .save(&mut pool, &bases(), [("canal-fixture".into(), changed)])
            .unwrap();
        let restored = restarted.load_pool().unwrap();
        assert_eq!(restored.decode(changed, 1), Some("heat"));
        assert_eq!(restored.decode(changed, 0), None);
        assert_eq!(restarted.reference("canal-fixture").unwrap(), Some(changed));
        assert!(restored.descriptor(first).is_none());
        let snapshot: Json = serde_json::from_slice(&fs::read(&store.path).unwrap()).unwrap();
        assert_eq!(snapshot.as_object().unwrap().len(), 3);
        assert!(snapshot.get("state").is_none());
    }

    #[test]
    fn dormant_bindings_survive_and_removed_bases_drop_only_exact_descendants() {
        let fixture = Fixture::new();
        let store = fixture.store("synthetic-owner");
        let mut pool = Pool::default();
        let first = pool.update(None, &choices(&["off", "heat"])).unwrap();
        let second = pool.update(None, &choices(&["a", "b"])).unwrap();
        let known = ["canal-fixture".into(), "canal-other".into()].into();
        store
            .save(
                &mut pool,
                &known,
                [
                    ("canal-fixture".into(), first),
                    ("canal-fixture-attribut-mode".into(), first),
                    ("canal-other".into(), second),
                ],
            )
            .unwrap();
        store.save(&mut pool, &bases(), []).unwrap();
        assert_eq!(store.reference("canal-fixture").unwrap(), Some(first));
        assert_eq!(
            store.reference("canal-fixture-attribut-mode").unwrap(),
            Some(first)
        );
        assert_eq!(store.reference("canal-other").unwrap(), None);
        assert!(store.load_pool().unwrap().descriptor(second).is_none());
        let no_match = ["canal-fixtur".into()].into();
        store.save(&mut pool, &no_match, []).unwrap();
        assert_eq!(pool.stats()["catalogues"], 0);
        let next = pool.update(None, &choices(&["new"])).unwrap();
        assert!(next.id > second.id);
    }

    #[test]
    fn corruption_owner_mismatch_and_unknown_refs_are_errors_without_rewriting() {
        let fixture = Fixture::new();
        let store = fixture.store("synthetic-owner");
        let mut pool = Pool::default();
        let reference = pool.update(None, &choices(&["off"])).unwrap();
        store
            .save(&mut pool, &bases(), [("canal-fixture".into(), reference)])
            .unwrap();
        let original = fs::read(&store.path).unwrap();
        for mutation in 0..4 {
            let mut saved: Json = serde_json::from_slice(&original).unwrap();
            match mutation {
                0 => saved["owner"] = "other-owner".into(),
                1 => saved["bindings"]["canal-fixture"]["id"] = 999.into(),
                2 => saved["pool"]["next_id"] = 0.into(),
                _ => saved["state"] = 7.into(),
            }
            let bad = serde_json::to_vec(&saved).unwrap();
            fs::write(&store.path, &bad).unwrap();
            assert!(store.load_pool().is_err());
            assert!(store.reference("canal-fixture").is_err());
            assert!(store.save(&mut pool, &bases(), []).is_err());
            assert_eq!(fs::read(&store.path).unwrap(), bad);
        }
        fs::write(&store.path, b"invalid").unwrap();
        assert!(store.load_pool().is_err());
        fs::write(&store.path, vec![b' '; MAX_BYTES + 1]).unwrap();
        assert!(store.load_pool().is_err());
    }

    #[test]
    fn filenames_are_hashed_and_permissions_are_private() {
        let fixture = Fixture::new();
        let store = fixture.store("synthetic private owner");
        assert_eq!(store.path.file_stem().unwrap().len(), 64);
        assert!(
            !store
                .path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .contains("synthetic")
        );
        assert_eq!(store.path, fixture.store("synthetic private owner").path);
        assert_ne!(store.path, fixture.store("another synthetic owner").path);
        let mut pool = Pool::default();
        store.save(&mut pool, &BTreeSet::new(), []).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&fixture.0).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(&store.path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 1);
    }

    #[test]
    fn failed_staging_preserves_the_previous_file_and_unpruned_pool() {
        let fixture = Fixture::new();
        let mut store = fixture.store("synthetic-owner");
        let mut pool = Pool::default();
        let first = pool.update(None, &choices(&["off"])).unwrap();
        let second = pool.update(None, &choices(&["auto"])).unwrap();
        let known = ["canal-fixture".into(), "canal-other".into()].into();
        store
            .save(
                &mut pool,
                &known,
                [
                    ("canal-fixture".into(), first),
                    ("canal-other".into(), second),
                ],
            )
            .unwrap();
        let before_file = fs::read(&store.path).unwrap();
        let before_pool = pool.export().unwrap();
        store.directory = fixture.0.join("missing-staging-directory");
        assert!(store.save(&mut pool, &bases(), []).is_err());
        assert_eq!(fs::read(&store.path).unwrap(), before_file);
        assert_eq!(pool.export().unwrap(), before_pool);
    }

    #[test]
    fn duplicate_bindings_and_a_disappeared_snapshot_never_reset_history() {
        let fixture = Fixture::new();
        let store = fixture.store("synthetic-owner");
        let mut pool = Pool::default();
        let reference = pool.update(None, &choices(&["off"])).unwrap();
        store
            .save(&mut pool, &bases(), [("canal-fixture".into(), reference)])
            .unwrap();
        let original: Json = serde_json::from_slice(&fs::read(&store.path).unwrap()).unwrap();
        let duplicate = format!(
            r#"{{"owner":"synthetic-owner","pool":{},"bindings":{{"canal-fixture":{},"canal-fixture":{}}}}}"#,
            original["pool"],
            serde_json::to_string(&reference).unwrap(),
            serde_json::to_string(&reference).unwrap()
        );
        fs::write(&store.path, duplicate).unwrap();
        assert!(store.load_pool().is_err());
        fs::remove_file(&store.path).unwrap();
        assert!(store.load_pool().is_err());
        assert!(store.reference("canal-fixture").is_err());
        assert!(store.save(&mut pool, &bases(), []).is_err());
    }
}
