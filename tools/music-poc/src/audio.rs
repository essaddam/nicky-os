//! A deterministic FLAC source and a bounded Rust decoder, with no sound device.
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::io::Cursor;
use symphonia::core::{
    codecs::audio::{AudioDecoder as _, AudioDecoderOptions},
    formats::{FormatOptions, FormatReader as _, TrackType},
    io::{MediaSourceStream, MediaSourceStreamOptions},
};
use symphonia::default::{codecs::FlacDecoder, formats::FlacReader};

#[path = "../../../crates/moli-audio/src/flac.rs"]
mod fixture_writer;

const RATE: u32 = 48_000;
const SAMPLES: usize = 48_000;
const MAX_INPUT: usize = 8 * 1024 * 1024;
const MAX_SAMPLES: usize = 480_000;

pub fn fixture() -> Vec<u8> {
    fixture_writer::encode(&expected_samples(), RATE)
}

fn expected_samples() -> Vec<i16> {
    // Generated test data, no recordings or third-party music.
    (0..SAMPLES)
        .map(|i| ((i % 1000) as i16 - 500) * 20)
        .collect()
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct Decoded {
    pub sample_rate: u32,
    pub samples: usize,
    pub pcm_sha256: String,
    pub fixture_exact: bool,
    pub sound_device_opened: bool,
}

pub fn decode_fixture(data: Vec<u8>) -> Result<Decoded> {
    ensure!(data.len() <= MAX_INPUT, "encoded input exceeds POC bound");
    let source = MediaSourceStream::new(
        Box::new(Cursor::new(data)),
        MediaSourceStreamOptions::default(),
    );
    let mut reader =
        FlacReader::try_new(source, FormatOptions::default()).context("invalid FLAC source")?;
    let parameters = reader
        .default_track(TrackType::Audio)
        .and_then(|t| t.codec_params.as_ref())
        .and_then(|p| p.audio())
        .context("missing audio parameters")?
        .clone();
    let rate = parameters.sample_rate.context("missing sample rate")?;
    ensure!(rate == RATE, "fixture rate changed");
    let mut decoder = FlacDecoder::try_new(&parameters, &AudioDecoderOptions::default())?;
    let mut samples = Vec::new();
    while let Some(packet) = reader.next_packet()? {
        let mut block: Vec<i16> = Vec::new();
        decoder.decode(&packet)?.copy_to_vec_interleaved(&mut block);
        ensure!(
            block.len() <= MAX_SAMPLES - samples.len(),
            "decoded sample budget exceeded"
        );
        samples.extend(block);
    }
    let mut digest = Sha256::new();
    for sample in &samples {
        digest.update(sample.to_le_bytes());
    }
    Ok(Decoded {
        sample_rate: rate,
        samples: samples.len(),
        pcm_sha256: format!("{:x}", digest.finalize()),
        fixture_exact: samples == expected_samples(),
        sound_device_opened: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_generated_sample_is_recovered_without_a_sound_device() {
        let out = decode_fixture(fixture()).unwrap();
        assert!(out.fixture_exact);
        assert_eq!(out.samples, SAMPLES);
        assert!(!out.sound_device_opened);
    }
    #[test]
    fn malformed_and_oversized_inputs_fail() {
        assert!(decode_fixture(vec![0; 50]).is_err());
        assert!(decode_fixture(vec![0; MAX_INPUT + 1]).is_err());
    }
}
