//! Symphonia streaming decoder for the canonical PCM16 WAV subset used by T0.
//! FLAC, arbitrary RIFF layouts and resampling are deliberately outside T0.

use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
};

use anyhow::{Context, Result, bail};
use symphonia::core::{
    codecs::audio::{AudioDecoder, AudioDecoderOptions},
    formats::{FormatReader, TrackType, probe::Hint},
    io::MediaSourceStream,
};

use crate::source::{CHANNELS, SAMPLE_RATE, SourceId};

pub const MAX_BLOCK_FRAMES: usize = 1024;
const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;
const MAX_DURATION_FRAMES: u64 = 120 * SAMPLE_RATE as u64;
// Symphonia 0.6.1's RIFF PCM packetizer emits at most 1152 frames.
const MAX_PACKET_FRAMES: usize = 1152;

#[derive(Clone, Debug)]
pub struct PcmBlock {
    pub source: SourceId,
    pub epoch: u64,
    pub first_frame: u64,
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Vec<i16>,
}

impl PcmBlock {
    pub fn frames(&self) -> usize {
        self.samples
            .len()
            .checked_div(usize::from(self.channels))
            .unwrap_or(0)
    }
}

struct Stream {
    format: Box<dyn FormatReader>,
    codec: Box<dyn AudioDecoder>,
    track_id: u32,
    pending: Vec<i16>,
    offset: usize,
    decoded_frames: u64,
}

pub struct Decoder {
    id: SourceId,
    file: File,
    stream: Stream,
    position: u64,
    total: u64,
    file_length: u64,
}

impl Decoder {
    pub(crate) fn open(id: SourceId, mut file: File) -> Result<Self> {
        let (total, file_length) = validate_wave(&mut file)?;
        let stream = open_stream(&mut file)?;
        Ok(Self {
            id,
            file,
            stream,
            position: 0,
            total,
            file_length,
        })
    }

    pub fn position(&self) -> u64 {
        self.position
    }
    pub fn total_frames(&self) -> u64 {
        self.total
    }

    pub fn read(&mut self, max_frames: usize) -> Result<Option<PcmBlock>> {
        if max_frames == 0 || max_frames > MAX_BLOCK_FRAMES {
            bail!("PCM block size must be 1 to 1024 frames");
        }
        if self.file.metadata()?.len() != self.file_length {
            bail!("source length changed or source was truncated");
        }
        if self.position == self.total {
            return Ok(None);
        }
        let first_frame = self.position;
        let target = max_frames.min((self.total - self.position) as usize) * usize::from(CHANNELS);
        let mut samples = Vec::with_capacity(target);
        while samples.len() < target {
            if self.stream.offset == self.stream.pending.len() {
                self.fill_packet()?;
            }
            let take = (target - samples.len()).min(self.stream.pending.len() - self.stream.offset);
            samples.extend_from_slice(
                &self.stream.pending[self.stream.offset..self.stream.offset + take],
            );
            self.stream.offset += take;
        }
        self.position += (samples.len() / usize::from(CHANNELS)) as u64;
        Ok(Some(PcmBlock {
            source: self.id.clone(),
            epoch: 0,
            first_frame,
            sample_rate: SAMPLE_RATE,
            channels: CHANNELS,
            samples,
        }))
    }

    fn fill_packet(&mut self) -> Result<()> {
        let packet = self
            .stream
            .format
            .next_packet()
            .context("cannot read WAV packet")?
            .context("WAV ended before its declared frame count")?;
        if packet.track_id != self.stream.track_id {
            bail!("unexpected WAV track");
        }
        let decoded = self
            .stream
            .codec
            .decode(&packet)
            .context("WAV PCM decoding failed")?;
        if decoded.spec().rate() != SAMPLE_RATE
            || decoded.spec().channels().count() != usize::from(CHANNELS)
        {
            bail!("PCM format changed");
        }
        let frames = decoded.frames();
        if frames == 0
            || frames > MAX_PACKET_FRAMES
            || self.stream.decoded_frames + frames as u64 > self.total
        {
            bail!("invalid or oversized WAV PCM packet");
        }
        self.stream
            .pending
            .resize(decoded.samples_interleaved(), 0_i16);
        decoded.copy_to_slice_interleaved(&mut self.stream.pending);
        self.stream.offset = 0;
        self.stream.decoded_frames += frames as u64;
        Ok(())
    }

    /// Exact seek by reopening the bounded stream and discarding small blocks.
    /// Work is bounded by 10 MiB / 120 seconds; no whole-source PCM cache exists.
    pub fn seek(&mut self, frame: u64) -> Result<()> {
        if frame > self.total {
            bail!("seek exceeds source frame count");
        }
        let (total, file_length) = validate_wave(&mut self.file)?;
        if total != self.total || file_length != self.file_length {
            bail!("source changed during seek");
        }
        self.stream = open_stream(&mut self.file)?;
        self.position = 0;
        while self.position < frame {
            let count = MAX_BLOCK_FRAMES.min((frame - self.position) as usize);
            self.read(count)?.context("WAV ended while seeking")?;
        }
        Ok(())
    }
}

fn open_stream(file: &mut File) -> Result<Stream> {
    file.seek(SeekFrom::Start(0))
        .context("cannot rewind WAV source")?;
    // This duplicate shares a cursor with the anchor File. Only the format
    // reader accesses it between seeks; &mut Decoder excludes concurrent seeks.
    let source = file.try_clone().context("cannot duplicate WAV source")?;
    let mut hint = Hint::new();
    hint.with_extension("wav");
    let format = symphonia::default::get_probe()
        .probe(
            &hint,
            MediaSourceStream::new(Box::new(source), Default::default()),
            Default::default(),
            Default::default(),
        )
        .context("unsupported WAV container")?;
    let track = format
        .default_track(TrackType::Audio)
        .context("WAV has no audio track")?;
    let parameters = track
        .codec_params
        .as_ref()
        .context("WAV codec parameters missing")?
        .audio()
        .context("WAV audio parameters missing")?;
    let codec = symphonia::default::get_codecs()
        .make_audio_decoder(parameters, &AudioDecoderOptions::default())
        .context("unsupported WAV PCM codec")?;
    let track_id = track.id;
    Ok(Stream {
        format,
        codec,
        track_id,
        pending: Vec::new(),
        offset: 0,
        decoded_frames: 0,
    })
}

fn validate_wave(file: &mut File) -> Result<(u64, u64)> {
    let metadata = file.metadata().context("cannot inspect WAV source")?;
    let length = metadata.len();
    if !metadata.is_file() || !(44..=MAX_FILE_BYTES).contains(&length) {
        bail!("WAV source must be a regular file of at most 10 MiB");
    }
    file.seek(SeekFrom::Start(0))?;
    let mut header = [0_u8; 44];
    file.read_exact(&mut header)
        .context("truncated WAV header")?;
    let u16_at = |offset| u16::from_le_bytes([header[offset], header[offset + 1]]);
    let u32_at = |offset| {
        u32::from_le_bytes(
            header[offset..offset + 4]
                .try_into()
                .expect("fixed WAV header offsets"),
        )
    };
    if &header[0..4] != b"RIFF"
        || &header[8..16] != b"WAVEfmt "
        || &header[36..40] != b"data"
        || u32_at(16) != 16
    {
        bail!("T0 requires canonical PCM WAV layout");
    }
    if u16_at(20) != 1
        || u16_at(22) != CHANNELS
        || u32_at(24) != SAMPLE_RATE
        || u32_at(28) != SAMPLE_RATE * u32::from(CHANNELS) * 2
        || u16_at(32) != CHANNELS * 2
        || u16_at(34) != 16
    {
        bail!("T0 supports PCM16 stereo WAV at 48 kHz only");
    }
    let data_bytes = u64::from(u32_at(40));
    if u64::from(u32_at(4)) + 8 != length
        || data_bytes + 44 != length
        || data_bytes == 0
        || data_bytes % (u64::from(CHANNELS) * 2) != 0
    {
        bail!("WAV declared size differs from complete frame data");
    }
    let frames = data_bytes / (u64::from(CHANNELS) * 2);
    if frames > MAX_DURATION_FRAMES {
        bail!("WAV duration exceeds 120 seconds");
    }
    Ok((frames, length))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{Catalog, SYNTHETIC_FRAMES, expected_sample};
    use std::io::Write;

    #[test]
    fn streaming_pcm_matches_each_channel_and_frame_exactly() {
        let catalog = Catalog::synthetic().unwrap();
        for descriptor in catalog.list().unwrap() {
            let mut decoder = catalog.open(&descriptor.id).unwrap();
            let mut frames = 0;
            while let Some(block) = decoder.read(997).unwrap() {
                assert_eq!(block.first_frame, frames);
                assert_eq!(block.sample_rate, SAMPLE_RATE);
                assert_eq!(block.channels, CHANNELS);
                assert!(block.frames() <= 997);
                for (offset, samples) in block.samples.as_chunks::<2>().0.iter().enumerate() {
                    for channel in 0..CHANNELS {
                        assert_eq!(
                            samples[usize::from(channel)],
                            expected_sample(&descriptor.id, frames + offset as u64, channel)
                        );
                    }
                }
                frames += block.frames() as u64;
            }
            assert_eq!(frames, SYNTHETIC_FRAMES);
            assert_eq!(decoder.position(), frames);
            assert!(decoder.read(1).unwrap().is_none());
        }
    }

    #[test]
    fn exact_seek_restarts_at_requested_frame_and_rejects_past_end() {
        let catalog = Catalog::synthetic().unwrap();
        let id = catalog.list().unwrap()[0].id.clone();
        let mut decoder = catalog.open(&id).unwrap();
        assert!(decoder.read(0).is_err());
        assert!(decoder.read(MAX_BLOCK_FRAMES + 1).is_err());
        for frame in [1153, 3, 4799, 0] {
            decoder.seek(frame).unwrap();
            assert_eq!(decoder.position(), frame);
            let block = decoder.read(1).unwrap().unwrap();
            assert_eq!(
                block.samples,
                vec![
                    expected_sample(&id, frame, 0),
                    expected_sample(&id, frame, 1)
                ]
            );
        }
        decoder.seek(SYNTHETIC_FRAMES).unwrap();
        assert!(decoder.read(1).unwrap().is_none());
        assert!(decoder.seek(SYNTHETIC_FRAMES + 1).is_err());
    }

    #[test]
    fn invalid_unknown_codec_and_truncated_sources_are_errors() {
        let id = SourceId::parse("invalid").unwrap();
        for bytes in [&b"fLaCinvalid"[..], &b"RIFF"[..], &[0_u8; 44][..]] {
            let mut file = tempfile::tempfile().unwrap();
            file.write_all(bytes).unwrap();
            assert!(Decoder::open(id.clone(), file).is_err());
        }
        let catalog = Catalog::synthetic().unwrap();
        let id = catalog.list().unwrap()[0].id.clone();
        let mut decoder = catalog.open(&id).unwrap();
        // The private catalogue uses a read-only anchor. A second handle to its
        // backing file is unnecessary: exercise declared-size truncation with
        // a separate writable synthetic header.
        let mut file = tempfile::tempfile().unwrap();
        decoder.file.seek(SeekFrom::Start(0)).unwrap();
        let mut header = [0; 44];
        decoder.file.read_exact(&mut header).unwrap();
        file.write_all(&header).unwrap();
        assert!(Decoder::open(id.clone(), file).is_err());
        for (offset, replacement) in [
            (20, 99_u16.to_le_bytes().to_vec()),
            (22, 1_u16.to_le_bytes().to_vec()),
            (24, 44_100_u32.to_le_bytes().to_vec()),
            (34, 24_u16.to_le_bytes().to_vec()),
        ] {
            let mut changed = header;
            changed[offset..offset + replacement.len()].copy_from_slice(&replacement);
            let mut file = tempfile::tempfile().unwrap();
            file.write_all(&changed).unwrap();
            file.set_len(44 + SYNTHETIC_FRAMES * 4).unwrap();
            assert!(Decoder::open(id.clone(), file).is_err());
        }
    }

    #[test]
    fn truncation_after_open_is_not_clean_eof() {
        let catalog = Catalog::synthetic().unwrap();
        let id = catalog.list().unwrap()[0].id.clone();
        let mut source = catalog.open(&id).unwrap();
        source.file.seek(SeekFrom::Start(0)).unwrap();
        let mut file = tempfile::tempfile().unwrap();
        std::io::copy(&mut source.file, &mut file).unwrap();
        let writer = file.try_clone().unwrap();
        let mut decoder = Decoder::open(id, file).unwrap();
        assert!(decoder.read(10).unwrap().is_some());
        writer.set_len(44).unwrap();
        assert!(decoder.read(10).is_err());
    }
}
