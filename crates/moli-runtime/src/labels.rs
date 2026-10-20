//! Label persistence: one human-editable TOML file, written atomically.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use tokio::io::AsyncWriteExt as _;

use moli_core::{DeviceId, Label};

pub(crate) fn load(path: &Path) -> io::Result<BTreeMap<DeviceId, Label>> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            toml::from_str(&text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(e) => Err(e),
    }
}

pub(crate) async fn save(path: &Path, labels: &BTreeMap<DeviceId, Label>) -> io::Result<()> {
    let text = toml::to_string_pretty(labels).map_err(io::Error::other)?;
    write_atomic(path, text.as_bytes()).await
}

/// Sync the complete temporary file before rename, then sync its directory.
/// An error (including directory sync after rename) must prevent activation.
pub(crate) async fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let dir = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    tokio::fs::create_dir_all(dir).await?;
    let mut file = tokio::fs::File::create(&tmp).await?;
    file.write_all(bytes).await?;
    file.flush().await?;
    #[cfg(test)]
    fail_sync(path, SyncPoint::File)?;
    file.sync_all().await?;
    drop(file);
    tokio::fs::rename(&tmp, path).await?;
    let directory = tokio::fs::File::open(dir).await?;
    #[cfg(test)]
    fail_sync(path, SyncPoint::Directory)?;
    directory.sync_all().await
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum SyncPoint {
    File,
    Directory,
}

#[cfg(test)]
thread_local! {
    static SYNC_FAILURE: std::cell::RefCell<Option<(std::path::PathBuf, SyncPoint)>> = const {
        std::cell::RefCell::new(None)
    };
}

#[cfg(test)]
fn fail_sync(path: &Path, point: SyncPoint) -> io::Result<()> {
    SYNC_FAILURE.with_borrow(|failure| {
        if failure
            .as_ref()
            .is_some_and(|(target, at)| target == path && *at == point)
        {
            Err(io::Error::other("injected sync failure"))
        } else {
            Ok(())
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::{MasterKey, SecretStore, master_key_bytes};

    struct Vault(std::path::PathBuf);

    impl Drop for Vault {
        fn drop(&mut self) {
            SYNC_FAILURE.with_borrow_mut(|failure| *failure = None);
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    // A failed assertion must never print the generated credential values.
    fn stored_is(store: &SecretStore, expected: &MasterKey) -> bool {
        store.get("weather/tokens").as_deref() == Some(expected.expose_for_file())
    }

    async fn rejected_sync_does_not_activate(point: SyncPoint) {
        let key = MasterKey::generate().unwrap();
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let vault =
            Vault(std::env::temp_dir().join(format!("moli-sync-{}-{nonce}", std::process::id())));
        let path = vault.0.join("secrets.enc");
        let store = SecretStore::open(path.clone(), &master_key_bytes(&key).unwrap()).unwrap();
        let old = MasterKey::generate().unwrap();
        let new = MasterKey::generate().unwrap();
        store
            .set("weather/tokens", old.expose_for_file())
            .await
            .unwrap();

        SYNC_FAILURE.with_borrow_mut(|failure| *failure = Some((path.clone(), point)));
        assert!(
            store
                .set("weather/tokens", new.expose_for_file())
                .await
                .is_err()
        );
        assert!(stored_is(&store, &old));
        if point == SyncPoint::File {
            let reopened =
                SecretStore::open(path.clone(), &master_key_bytes(&key).unwrap()).unwrap();
            assert!(stored_is(&reopened, &old));
        }
        // After rename, a failed directory sync leaves disk durability unknown.
        // The error must still prevent activating the new value in this owner.
        SYNC_FAILURE.with_borrow_mut(|failure| *failure = None);
        store
            .set("weather/tokens", new.expose_for_file())
            .await
            .unwrap();
        let reopened = SecretStore::open(path, &master_key_bytes(&key).unwrap()).unwrap();
        assert!(stored_is(&reopened, &new));
    }

    #[tokio::test]
    async fn file_sync_failure_does_not_activate_secret() {
        rejected_sync_does_not_activate(SyncPoint::File).await;
    }

    #[tokio::test]
    async fn directory_sync_failure_does_not_activate_secret() {
        rejected_sync_does_not_activate(SyncPoint::Directory).await;
    }
}
