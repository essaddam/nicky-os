//! Cold, private synthetic catalogue. Commands select validated IDs only.

use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tempfile::TempDir;

use crate::decode::Decoder;

pub const SAMPLE_RATE: u32 = 48_000;
pub const CHANNELS: u16 = 2;
pub const SYNTHETIC_FRAMES: u64 = 4_800;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SourceId(String);

impl SourceId {
    pub fn parse(value: &str) -> Result<Self> {
        if value.is_empty()
            || value.len() > 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            bail!("source ID must contain 1 to 64 ASCII letters, digits, underscores or hyphens");
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for SourceId {
    type Error = anyhow::Error;
    fn try_from(value: String) -> Result<Self> {
        Self::parse(&value)
    }
}

impl From<SourceId> for String {
    fn from(value: SourceId) -> Self {
        value.0
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Descriptor {
    pub id: SourceId,
    pub label: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub total_frames: u64,
}

pub struct Catalog {
    root: TempDir,
}

impl Catalog {
    pub fn synthetic() -> Result<Self> {
        let root = tempfile::Builder::new()
            .prefix("moli-engine-")
            .tempdir()
            .context("cannot create private catalogue")?;
        let mut descriptors = Vec::new();
        for (id, label) in [
            ("tone-a", "Synthetic stereo A"),
            ("tone-b", "Synthetic stereo B"),
        ] {
            let id = SourceId::parse(id)?;
            write_wave(&root.path().join(format!("{}.wav", id.as_str())), &id)?;
            descriptors.push(Descriptor {
                id,
                label: label.to_owned(),
                sample_rate: SAMPLE_RATE,
                channels: CHANNELS,
                total_frames: SYNTHETIC_FRAMES,
            });
        }
        let manifest = File::create(root.path().join("catalog.json"))
            .context("cannot create catalogue manifest")?;
        serde_json::to_writer(manifest, &descriptors).context("cannot write catalogue manifest")?;
        Ok(Self { root })
    }

    pub fn list(&self) -> Result<Vec<Descriptor>> {
        let manifest = File::open(self.root.path().join("catalog.json"))
            .context("cannot open catalogue manifest")?;
        if manifest.metadata()?.len() > 16 * 1024 {
            bail!("catalogue manifest exceeds limit");
        }
        let mut bytes = Vec::new();
        manifest
            .take(16 * 1024 + 1)
            .read_to_end(&mut bytes)
            .context("cannot read catalogue manifest")?;
        if bytes.len() > 16 * 1024 {
            bail!("catalogue manifest exceeds limit");
        }
        let descriptors: Vec<Descriptor> =
            serde_json::from_slice(&bytes).context("invalid catalogue manifest")?;
        if descriptors.len() != 2 {
            bail!("unexpected synthetic catalogue length");
        }
        for (index, descriptor) in descriptors.iter().enumerate() {
            if descriptor.sample_rate != SAMPLE_RATE
                || descriptor.channels != CHANNELS
                || descriptor.total_frames != SYNTHETIC_FRAMES
                || descriptor.label.len() > 80
            {
                bail!("invalid catalogue descriptor");
            }
            if descriptors[..index]
                .iter()
                .any(|previous| previous.id == descriptor.id)
            {
                bail!("duplicate source ID");
            }
        }
        Ok(descriptors)
    }

    pub fn contains(&self, id: &SourceId) -> Result<bool> {
        Ok(self.list()?.iter().any(|descriptor| descriptor.id == *id))
    }

    pub fn open(&self, id: &SourceId) -> Result<Decoder> {
        let descriptor = self
            .list()?
            .into_iter()
            .find(|descriptor| descriptor.id == *id)
            .context("source ID is not present in catalogue")?;
        let file = File::open(self.root.path().join(format!("{}.wav", id.as_str())))
            .context("cannot open catalogue source")?;
        let decoder = Decoder::open(id.clone(), file)?;
        if decoder.total_frames() != descriptor.total_frames {
            bail!("source duration differs from catalogue");
        }
        Ok(decoder)
    }
}

/// Distinct channels and sources; each frame can be checked without a PCM cache.
pub fn expected_sample(source: &SourceId, frame: u64, channel: u16) -> i16 {
    let seed = if source.as_str() == "tone-a" {
        73_u64
    } else {
        991_u64
    };
    let value = ((frame % 20_000) * 17 + u64::from(channel) * 3_001 + seed) % 20_000;
    value as i16 - 10_000
}

fn write_wave(path: &Path, id: &SourceId) -> Result<()> {
    let mut file = File::create(path).context("cannot create synthetic WAV")?;
    let bytes = u32::try_from(SYNTHETIC_FRAMES * u64::from(CHANNELS) * 2)?;
    file.write_all(b"RIFF")?;
    file.write_all(&(bytes + 36).to_le_bytes())?;
    file.write_all(b"WAVEfmt ")?;
    file.write_all(&16_u32.to_le_bytes())?;
    file.write_all(&1_u16.to_le_bytes())?;
    file.write_all(&CHANNELS.to_le_bytes())?;
    file.write_all(&SAMPLE_RATE.to_le_bytes())?;
    file.write_all(&(SAMPLE_RATE * u32::from(CHANNELS) * 2).to_le_bytes())?;
    file.write_all(&(CHANNELS * 2).to_le_bytes())?;
    file.write_all(&16_u16.to_le_bytes())?;
    file.write_all(b"data")?;
    file.write_all(&bytes.to_le_bytes())?;
    for first in (0..SYNTHETIC_FRAMES).step_by(256) {
        let mut block = Vec::with_capacity(256 * usize::from(CHANNELS) * 2);
        for frame in first..(first + 256).min(SYNTHETIC_FRAMES) {
            for channel in 0..CHANNELS {
                block.extend_from_slice(&expected_sample(id, frame, channel).to_le_bytes());
            }
        }
        file.write_all(&block)?;
    }
    file.sync_all().context("cannot finish synthetic WAV")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_ids_cannot_be_paths_or_uris_including_after_deserialization() {
        for value in [
            "",
            "../tone-a",
            "/tmp/a",
            "file:a",
            "https://example.com/a",
            "a b",
            "é",
            "a\\b",
            "a.b",
        ] {
            assert!(SourceId::parse(value).is_err(), "{value}");
            assert!(
                serde_json::from_str::<SourceId>(&serde_json::to_string(value).unwrap()).is_err()
            );
        }
        assert!(SourceId::parse(&"a".repeat(65)).is_err());
        assert!(SourceId::parse("Valid_01-a").is_ok());
    }

    #[test]
    fn cold_catalogue_lists_two_sources_and_rejects_unknown_ids() {
        let catalog = Catalog::synthetic().unwrap();
        let list = catalog.list().unwrap();
        assert_eq!(list.len(), 2);
        assert!(catalog.contains(&list[0].id).unwrap());
        let unknown = SourceId::parse("missing").unwrap();
        assert!(!catalog.contains(&unknown).unwrap());
        assert!(catalog.open(&unknown).is_err());
        assert_ne!(
            expected_sample(&list[0].id, 0, 0),
            expected_sample(&list[0].id, 0, 1)
        );
        assert_ne!(
            expected_sample(&list[0].id, 0, 0),
            expected_sample(&list[1].id, 0, 0)
        );
    }
}
