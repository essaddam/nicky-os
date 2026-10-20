//! Bounded business state: one writer, temporary/fsync/rename/directory fsync.

use std::path::{Path, PathBuf};

use serde::{Serialize, de::DeserializeOwned};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Debug, thiserror::Error)]
#[error("{error}")]
pub struct StateFileError {
    pub error: std::io::Error,
    /// Rename happened: the caller must not roll its in-memory state back and retry.
    pub commit_unknown: bool,
}

#[derive(Clone, Debug)]
pub struct StateFile {
    path: Option<PathBuf>,
    max_bytes: usize,
    #[cfg(test)]
    failure: Option<WritePhase>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg(test)]
pub(crate) enum WritePhase {
    BeforeWrite,
    BeforeRename,
    AfterRename,
}

impl StateFile {
    #[cfg(test)]
    pub(crate) fn interrupt_at(&mut self, phase: Option<WritePhase>) {
        self.failure = phase;
    }

    #[must_use]
    pub fn new(path: Option<PathBuf>, max_bytes: usize) -> Self {
        Self {
            path,
            max_bytes,
            #[cfg(test)]
            failure: None,
        }
    }

    #[must_use]
    pub fn is_persistent(&self) -> bool {
        self.path.is_some()
    }

    pub async fn load<T: DeserializeOwned>(&self) -> std::io::Result<Option<T>> {
        let Some(path) = &self.path else {
            return Ok(None);
        };
        let metadata = match tokio::fs::symlink_metadata(path).await {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        if !metadata.is_file() || metadata.len() > self.max_bytes as u64 {
            return Err(std::io::Error::other("invalid business state file"));
        }
        let mut bytes = Vec::new();
        tokio::fs::File::open(path)
            .await?
            .take(self.max_bytes as u64 + 1)
            .read_to_end(&mut bytes)
            .await?;
        if bytes.len() > self.max_bytes {
            return Err(std::io::Error::other("business state too large"));
        }
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(std::io::Error::other)
    }

    pub async fn write<T: Serialize>(&self, state: &T) -> Result<(), StateFileError> {
        let mut renamed = false;
        let result = async {
            let bytes = serde_json::to_vec(state).map_err(std::io::Error::other)?;
            if bytes.len() > self.max_bytes {
                return Err(std::io::Error::other("business state too large"));
            }
            let Some(path) = &self.path else {
                return Ok(());
            };
            let parent = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            tokio::fs::create_dir_all(parent).await?;
            let temporary = path.with_extension("business-tmp");
            if let Ok(metadata) = tokio::fs::symlink_metadata(&temporary).await
                && !metadata.is_file()
            {
                return Err(std::io::Error::other("invalid temporary business state"));
            }
            #[cfg(test)]
            self.checkpoint(WritePhase::BeforeWrite)?;
            let mut output = tokio::fs::File::create(&temporary).await?;
            output.write_all(&bytes).await?;
            output.sync_all().await?;
            drop(output);
            #[cfg(test)]
            self.checkpoint(WritePhase::BeforeRename)?;
            tokio::fs::rename(&temporary, path).await?;
            renamed = true;
            #[cfg(test)]
            self.checkpoint(WritePhase::AfterRename)?;
            tokio::fs::File::open(parent).await?.sync_all().await
        }
        .await;
        result.map_err(|error| StateFileError {
            error,
            commit_unknown: renamed,
        })
    }

    #[cfg(test)]
    fn checkpoint(&self, phase: WritePhase) -> std::io::Result<()> {
        if self.failure == Some(phase) {
            return Err(std::io::Error::other(
                "simulated business-state interruption",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn auto_h4_atomic_state_cut_before_and_after_rename_is_explicit() {
        let parent = std::env::temp_dir().join(format!(
            "moli-h4-state-{}",
            crate::occurrences::random_id().unwrap()
        ));
        let path = parent.join("state.json");
        let mut file = StateFile::new(Some(path), 1024);
        file.write(&1_u64).await.unwrap();
        file.failure = Some(WritePhase::BeforeRename);
        assert!(!file.write(&2_u64).await.unwrap_err().commit_unknown);
        assert_eq!(file.load::<u64>().await.unwrap(), Some(1));
        file.failure = Some(WritePhase::AfterRename);
        assert!(file.write(&2_u64).await.unwrap_err().commit_unknown);
        assert_eq!(file.load::<u64>().await.unwrap(), Some(2));
        file.failure = None;
        file.write(&3_u64).await.unwrap();
        assert_eq!(file.load::<u64>().await.unwrap(), Some(3));
        tokio::fs::remove_dir_all(parent).await.unwrap();
    }

    #[tokio::test]
    async fn auto_h4_corrupt_or_oversized_business_state_is_never_reset() {
        let parent = std::env::temp_dir().join(format!(
            "moli-h4-invalid-{}",
            crate::occurrences::random_id().unwrap()
        ));
        tokio::fs::create_dir_all(&parent).await.unwrap();
        let path = parent.join("state.json");
        tokio::fs::write(&path, b"broken").await.unwrap();
        let file = StateFile::new(Some(path.clone()), 8);
        assert!(file.load::<u64>().await.is_err());
        assert_eq!(tokio::fs::read(&path).await.unwrap(), b"broken");
        assert!(file.write(&"a value beyond the bound").await.is_err());
        assert_eq!(tokio::fs::read(&path).await.unwrap(), b"broken");
        tokio::fs::remove_dir_all(parent).await.unwrap();
    }
}
