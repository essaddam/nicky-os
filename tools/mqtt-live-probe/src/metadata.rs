//! Cold, private descriptor backing. Never stores credentials. The temporary
//! file is outside the checkout, mode 0600, removed when the projection stops.
use anyhow::{Context, Result, ensure};
use serde_json::Value as Json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
pub(super) enum Bank {
    Memory { value: Json, bytes: usize },
    File(PathBuf),
}
impl Default for Bank {
    fn default() -> Self {
        Self::Memory {
            value: Json::Null,
            bytes: 4,
        }
    }
}
impl Bank {
    /// RAM compares borrowed fields. File mode deliberately reads/parses the cold
    /// descriptor per check instead of keeping a second permanent JSON in memory.
    pub(super) fn contains_patch(&self, channel: &str, patch: &Json) -> Result<bool> {
        let matches = |descriptor: &Json| {
            patch.as_object().is_some_and(|fields| {
                fields.iter().all(|(key, value)| {
                    descriptor
                        .get("canaux")
                        .and_then(|v| v.get(channel))
                        .and_then(|v| v.get(key))
                        == Some(value)
                })
            })
        };
        match self {
            Self::Memory { value, .. } => Ok(matches(value)),
            Self::File(_) => Ok(matches(&self.read()?)),
        }
    }

    pub(super) fn bytes(&self) -> usize {
        match self {
            Self::Memory { bytes, .. } => *bytes,
            Self::File(p) => p.metadata().map_or(0, |m| m.len() as usize),
        }
    }
    pub(super) fn replace(&mut self, value: Json, file: bool) -> Result<()> {
        if !file {
            let bytes = serde_json::to_vec(&value)?.len();
            ensure!(bytes <= 4 * 1024 * 1024, "metadata bound exceeded");
            *self = Self::Memory { value, bytes };
            return Ok(());
        }
        if let Self::File(path) = self {
            let data = serde_json::to_vec(&value)?;
            ensure!(data.len() <= 4 * 1024 * 1024, "metadata bound exceeded");
            std::fs::write(path, data).context("cold metadata update failed")?;
            return Ok(());
        }
        use std::io::Write;
        let path = std::env::temp_dir().join(format!(
            "moli-probe-metadata-{}-{}.json",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut output = opts.open(&path).context("cold metadata creation failed")?;
        let bytes = serde_json::to_vec(&value)?;
        ensure!(bytes.len() <= 4 * 1024 * 1024, "metadata bound exceeded");
        output.write_all(&bytes)?;
        *self = Self::File(path);
        Ok(())
    }
    pub(super) fn read(&self) -> Result<Json> {
        match self {
            Self::Memory { value, .. } => Ok(value.clone()),
            Self::File(p) => {
                ensure!(
                    p.metadata()?.len() <= 4 * 1024 * 1024,
                    "metadata bound exceeded"
                );
                Ok(serde_json::from_slice(&std::fs::read(p)?)?)
            }
        }
    }
}
impl Drop for Bank {
    fn drop(&mut self) {
        if let Self::File(p) = self {
            let _ = std::fs::remove_file(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn patch_comparison_is_exact_for_both_backings() {
        for file in [false, true] {
            let mut bank = Bank::default();
            bank.replace(
                serde_json::json!({"canaux":{"fixture":{"nom":"Capteur","unite":null}}}),
                file,
            )
            .unwrap();
            assert!(
                bank.contains_patch(
                    "fixture",
                    &serde_json::json!({"nom":"Capteur","unite":null})
                )
                .unwrap()
            );
            assert!(
                !bank
                    .contains_patch("fixture", &serde_json::json!({"nom":"Autre"}))
                    .unwrap()
            );
            assert!(
                !bank
                    .contains_patch("fixture", &serde_json::json!({"absent":null}))
                    .unwrap()
            );
            assert!(
                !bank
                    .contains_patch("missing", &serde_json::json!({"nom":null}))
                    .unwrap()
            );
        }
    }
    #[test]
    fn unreadable_cold_descriptor_is_an_error_instead_of_an_unchanged_patch() {
        let mut bank = Bank::default();
        bank.replace(serde_json::json!({"canaux":{}}), true)
            .unwrap();
        if let Bank::File(path) = &bank {
            std::fs::write(path, b"invalid JSON").unwrap();
        }
        assert!(
            bank.contains_patch("fixture", &serde_json::json!({"nom":"Capteur"}))
                .is_err()
        );
    }
    #[test]
    fn cold_and_memory_descriptors_have_the_same_contract() {
        let v = serde_json::json!({"nom":"Fixture","commandes":{"state":["ON","OFF"]}});
        let mut mem = Bank::default();
        mem.replace(v.clone(), false).unwrap();
        let mut file = Bank::default();
        file.replace(v.clone(), true).unwrap();
        assert_eq!(mem.read().unwrap(), file.read().unwrap());
        file.replace(serde_json::json!({"nom":"Renommé"}), true)
            .unwrap();
        assert_eq!(file.read().unwrap()["nom"], "Renommé");
    }
}
