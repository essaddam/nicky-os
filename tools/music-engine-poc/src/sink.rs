//! Bounded, local-only destinations for decoded PCM16 blocks.

use std::{
    collections::VecDeque,
    fs::File,
    io::{Seek, SeekFrom, Write},
};

use anyhow::{Result, bail};
use sha2::{Digest, Sha256};

use crate::decode::PcmBlock;

const MAX_BLOCK_FRAMES: usize = 1024;
const MAX_BLOCK_SAMPLES: usize = 2048;
const MAX_BLOCK_BYTES: usize = MAX_BLOCK_SAMPLES * 2;
const MAX_QUEUE_CAPACITY: usize = 4;
const MAX_SAMPLE_RATE: u32 = 48_000;
const MAX_WAV_SECONDS: u64 = 60;

/// A full sink refuses a block before retaining or consuming it.
pub trait PcmSink {
    fn has_capacity(&self) -> bool;
    fn push(&mut self, block: &PcmBlock) -> Result<()>;

    /// Drops pending samples only; samples already consumed cannot be retracted.
    fn reset(&mut self) {}

    fn is_closed(&self) -> bool {
        false
    }

    fn ready_to_finish(&self) -> bool {
        true
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        Ok(())
    }
}

fn validate_format(sample_rate: u32, channels: u16) -> Result<()> {
    if sample_rate == 0 || sample_rate > MAX_SAMPLE_RATE || !(1..=2).contains(&channels) {
        bail!("Unsupported PCM format");
    }
    Ok(())
}

fn validate_block(block: &PcmBlock) -> Result<()> {
    validate_format(block.sample_rate, block.channels)?;
    let channels = usize::from(block.channels);
    if block.samples.is_empty()
        || block.samples.len() > MAX_BLOCK_SAMPLES
        || !block.samples.len().is_multiple_of(channels)
        || block.samples.len() / channels > MAX_BLOCK_FRAMES
    {
        bail!("PCM block exceeds its bounds or contains an incomplete frame");
    }
    Ok(())
}

/// Counts and hashes PCM bytes without retaining samples or enforcing source order.
#[derive(Default)]
pub struct NullSink {
    pub frames: u64,
    pub total_samples: u64,
    hasher: Sha256,
}

impl NullSink {
    pub fn new() -> Self {
        Self::default()
    }

    /// Hashes exactly the little-endian PCM16 sample bytes, independent of block boundaries.
    pub fn checksum(&self) -> String {
        format!("{:x}", self.hasher.clone().finalize())
    }
}

impl PcmSink for NullSink {
    fn has_capacity(&self) -> bool {
        true
    }

    fn push(&mut self, block: &PcmBlock) -> Result<()> {
        validate_block(block)?;
        let frames = self
            .frames
            .checked_add((block.samples.len() / usize::from(block.channels)) as u64);
        let samples = self.total_samples.checked_add(block.samples.len() as u64);
        let (Some(frames), Some(samples)) = (frames, samples) else {
            bail!("PCM statistics overflow");
        };
        for sample in &block.samples {
            self.hasher.update(sample.to_le_bytes());
        }
        self.frames = frames;
        self.total_samples = samples;
        Ok(())
    }
}

/// A fixed-format WAV writer. The caller owns the choice and opening of its file.
pub struct WavSink {
    file: File,
    sample_rate: u32,
    channels: u16,
    frames: u64,
    data_bytes: u32,
    finished: bool,
    failed: bool,
}

impl WavSink {
    pub fn new(mut file: File, sample_rate: u32, channels: u16) -> Result<Self> {
        validate_format(sample_rate, channels)?;
        if !file.metadata()?.is_file() {
            bail!("WAV output must be a regular file");
        }
        file.seek(SeekFrom::Start(0))?;
        file.set_len(0)?;
        file.write_all(&wav_header(sample_rate, channels, 0))?;
        Ok(Self {
            file,
            sample_rate,
            channels,
            frames: 0,
            data_bytes: 0,
            finished: false,
            failed: false,
        })
    }

    pub const fn frames(&self) -> u64 {
        self.frames
    }
}

fn wav_header(sample_rate: u32, channels: u16, data_bytes: u32) -> [u8; 44] {
    let mut header = [0_u8; 44];
    header[0..4].copy_from_slice(b"RIFF");
    header[4..8].copy_from_slice(&(36 + data_bytes).to_le_bytes());
    header[8..16].copy_from_slice(b"WAVEfmt ");
    header[16..20].copy_from_slice(&16_u32.to_le_bytes());
    header[20..22].copy_from_slice(&1_u16.to_le_bytes());
    header[22..24].copy_from_slice(&channels.to_le_bytes());
    header[24..28].copy_from_slice(&sample_rate.to_le_bytes());
    header[28..32].copy_from_slice(&(sample_rate * u32::from(channels) * 2).to_le_bytes());
    header[32..34].copy_from_slice(&(channels * 2).to_le_bytes());
    header[34..36].copy_from_slice(&16_u16.to_le_bytes());
    header[36..40].copy_from_slice(b"data");
    header[40..44].copy_from_slice(&data_bytes.to_le_bytes());
    header
}

impl PcmSink for WavSink {
    fn is_closed(&self) -> bool {
        self.finished || self.failed
    }
    fn has_capacity(&self) -> bool {
        !self.finished
            && !self.failed
            && self.frames < u64::from(self.sample_rate) * MAX_WAV_SECONDS
    }

    fn push(&mut self, block: &PcmBlock) -> Result<()> {
        validate_block(block)?;
        if self.finished || self.failed {
            bail!("WAV sink is closed or failed");
        }
        if block.sample_rate != self.sample_rate || block.channels != self.channels {
            bail!("WAV format cannot change");
        }
        let added_frames = (block.samples.len() / usize::from(block.channels)) as u64;
        if self.frames + added_frames > u64::from(self.sample_rate) * MAX_WAV_SECONDS {
            bail!("WAV duration quota exceeded");
        }
        let mut bytes = [0_u8; MAX_BLOCK_BYTES];
        for (sample, output) in block
            .samples
            .iter()
            .zip(bytes.as_chunks_mut::<2>().0.iter_mut())
        {
            output.copy_from_slice(&sample.to_le_bytes());
        }
        let byte_count = block.samples.len() * 2;
        if let Err(error) = self.file.write_all(&bytes[..byte_count]) {
            self.failed = true;
            return Err(error.into());
        }
        self.frames += added_frames;
        self.data_bytes += byte_count as u32;
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        if self.failed {
            bail!("WAV sink failed");
        }
        if self.finished {
            return Ok(());
        }
        let result = (|| -> std::io::Result<()> {
            self.file.seek(SeekFrom::Start(0))?;
            self.file.write_all(&wav_header(
                self.sample_rate,
                self.channels,
                self.data_bytes,
            ))?;
            self.file.set_len(44 + u64::from(self.data_bytes))?;
            self.file.flush()?;
            self.file.seek(SeekFrom::End(0))?;
            Ok(())
        })();
        if let Err(error) = result {
            self.failed = true;
            return Err(error.into());
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        self.flush()?;
        self.finished = true;
        Ok(())
    }
}

/// Backpressure queue: at most four blocks, each at most 4096 sample bytes.
pub struct BoundedSink {
    queue: VecDeque<PcmBlock>,
    capacity: usize,
    queued_bytes: usize,
    peak_bytes: usize,
}

impl BoundedSink {
    pub fn new(capacity: usize) -> Result<Self> {
        if !(1..=MAX_QUEUE_CAPACITY).contains(&capacity) {
            bail!("PCM queue capacity must be between one and four");
        }
        Ok(Self {
            queue: VecDeque::with_capacity(capacity),
            capacity,
            queued_bytes: 0,
            peak_bytes: 0,
        })
    }

    pub fn pop(&mut self) -> Option<PcmBlock> {
        let block = self.queue.pop_front()?;
        self.queued_bytes -= block.samples.len() * 2;
        Some(block)
    }

    pub const fn queued_bytes(&self) -> usize {
        self.queued_bytes
    }
    pub const fn peak_bytes(&self) -> usize {
        self.peak_bytes
    }
}

impl PcmSink for BoundedSink {
    fn ready_to_finish(&self) -> bool {
        self.queue.is_empty()
    }
    fn has_capacity(&self) -> bool {
        self.queue.len() < self.capacity
    }

    fn push(&mut self, block: &PcmBlock) -> Result<()> {
        validate_block(block)?;
        if !self.has_capacity() {
            bail!("PCM queue is full");
        }
        let samples = block.samples.clone();
        self.queued_bytes += samples.len() * 2;
        self.peak_bytes = self.peak_bytes.max(self.queued_bytes);
        self.queue.push_back(PcmBlock {
            source: block.source.clone(),
            first_frame: block.first_frame,
            epoch: block.epoch,
            sample_rate: block.sample_rate,
            channels: block.channels,
            samples,
        });
        Ok(())
    }

    fn reset(&mut self) {
        self.queue.clear();
        self.queued_bytes = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceId;
    use std::io::Read;

    fn block(frames: usize) -> PcmBlock {
        PcmBlock {
            source: SourceId::parse("fixture").unwrap(),
            first_frame: 0,
            epoch: 1,
            sample_rate: 48_000,
            channels: 2,
            samples: vec![123; frames * 2],
        }
    }

    #[test]
    fn null_hashes_sample_bytes_without_retaining_pcm() {
        let mut sink = NullSink::new();
        let block = block(3);
        sink.push(&block).unwrap();
        let bytes: Vec<u8> = block.samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        assert_eq!(sink.checksum(), format!("{:x}", Sha256::digest(&bytes)));
        assert_eq!(sink.frames, 3);
        assert_eq!(sink.total_samples, 6);
        let before = sink.checksum();
        assert!(sink.push(&self::block(1025)).is_err());
        assert_eq!(sink.checksum(), before);
        assert_eq!(sink.frames, 3);
    }

    #[test]
    fn bounded_queue_has_exact_capacity_and_byte_limits() {
        assert!(BoundedSink::new(0).is_err());
        assert!(BoundedSink::new(5).is_err());
        let mut sink = BoundedSink::new(4).unwrap();
        for frame in 0..4 {
            let mut block = block(1024);
            block.first_frame = frame * 1024;
            sink.push(&block).unwrap();
        }
        assert!(!sink.has_capacity());
        assert_eq!(sink.queued_bytes(), 16_384);
        assert_eq!(sink.peak_bytes(), 16_384);
        assert!(sink.push(&block(1)).is_err());
        for frame in 0..4 {
            let block = sink.pop().unwrap();
            assert_eq!(block.first_frame, frame * 1024);
            assert_eq!(block.epoch, 1);
        }
        assert!(sink.pop().is_none());
        assert!(sink.has_capacity());
        assert_eq!(sink.queued_bytes(), 0);
        assert_eq!(sink.peak_bytes(), 16_384);
        assert!(sink.push(&block(1025)).is_err());
        assert_eq!(sink.queued_bytes(), 0);
        sink.push(&block(1024)).unwrap();
        sink.reset();
        assert!(sink.pop().is_none());
        assert_eq!(sink.queued_bytes(), 0);
        assert_eq!(sink.peak_bytes(), 16_384);
    }

    #[test]
    fn wav_header_sizes_samples_and_format_are_correct() {
        let file = tempfile::tempfile().unwrap();
        let mut reader = file.try_clone().unwrap();
        let mut sink = WavSink::new(file, 48_000, 2).unwrap();
        let block = block(3);
        sink.push(&block).unwrap();
        let mut changed = self::block(1);
        changed.sample_rate = 44_100;
        assert!(sink.push(&changed).is_err());
        sink.finish().unwrap();
        sink.finish().unwrap();
        assert!(!sink.has_capacity());
        assert!(sink.push(&block).is_err());
        reader.seek(SeekFrom::Start(0)).unwrap();
        let mut output = Vec::new();
        reader.read_to_end(&mut output).unwrap();
        assert_eq!(&output[..4], b"RIFF");
        assert_eq!(&output[8..16], b"WAVEfmt ");
        assert_eq!(u32::from_le_bytes(output[4..8].try_into().unwrap()), 48);
        assert_eq!(u16::from_le_bytes(output[22..24].try_into().unwrap()), 2);
        assert_eq!(
            u32::from_le_bytes(output[24..28].try_into().unwrap()),
            48_000
        );
        assert_eq!(u16::from_le_bytes(output[34..36].try_into().unwrap()), 16);
        assert_eq!(u32::from_le_bytes(output[40..44].try_into().unwrap()), 12);
        assert_eq!(output.len(), 56);
        assert_eq!(&output[44..46], &123_i16.to_le_bytes());
    }

    #[test]
    fn wav_refuses_more_than_sixty_seconds_before_writing() {
        let file = tempfile::tempfile().unwrap();
        let mut sink = WavSink::new(file, 1, 1).unwrap();
        let mut block = block(30);
        block.sample_rate = 1;
        block.channels = 1;
        // Sixty mono samples are exactly sixty seconds at this test rate.
        sink.push(&block).unwrap();
        assert_eq!(sink.frames(), 60);
        assert!(!sink.has_capacity());
        block.samples.truncate(1);
        assert!(sink.push(&block).is_err());
        assert_eq!(sink.frames(), 60);
        sink.finish().unwrap();
    }

    #[test]
    fn rejects_invalid_formats_and_partial_frames() {
        let directory = tempfile::tempdir().unwrap();
        assert!(WavSink::new(File::open(directory.path()).unwrap(), 48_000, 2).is_err());
        assert!(WavSink::new(tempfile::tempfile().unwrap(), 0, 2).is_err());
        assert!(WavSink::new(tempfile::tempfile().unwrap(), 48_001, 2).is_err());
        let mut bad = block(1);
        bad.samples.pop();
        assert!(NullSink::new().push(&bad).is_err());
        assert!(BoundedSink::new(1).unwrap().push(&bad).is_err());
    }
}
