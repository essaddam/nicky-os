use anyhow::{Result, ensure};
use moli_music_engine_poc::{
    decode::PcmBlock,
    engine::{Command, Engine, Render, State},
    sink::{NullSink, PcmSink, WavSink},
    source::{CHANNELS, Catalog, SAMPLE_RATE, SYNTHETIC_FRAMES, SourceId, expected_sample},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
};

struct Checked<S> {
    destination: S,
    digest: NullSink,
    source: Option<SourceId>,
    next_frame: u64,
    discontinuity: bool,
    max_block: usize,
}

impl<S: PcmSink> Checked<S> {
    fn new(destination: S) -> Self {
        Self {
            destination,
            digest: NullSink::new(),
            source: None,
            next_frame: 0,
            discontinuity: true,
            max_block: 0,
        }
    }
}

impl<S: PcmSink> PcmSink for Checked<S> {
    fn has_capacity(&self) -> bool {
        self.destination.has_capacity()
    }
    fn push(&mut self, block: &PcmBlock) -> Result<()> {
        if self.discontinuity {
            self.source = Some(block.source.clone());
            self.next_frame = block.first_frame;
            self.discontinuity = false;
        } else if self.source.as_ref() != Some(&block.source) {
            ensure!(
                block.first_frame == 0,
                "new track did not start at its first frame"
            );
            self.source = Some(block.source.clone());
            self.next_frame = 0;
        }
        ensure!(
            block.first_frame == self.next_frame,
            "PCM frames are out of order"
        );
        for (offset, frame) in block
            .samples
            .chunks_exact(usize::from(CHANNELS))
            .enumerate()
        {
            for channel in 0..CHANNELS {
                ensure!(
                    frame[usize::from(channel)]
                        == expected_sample(
                            &block.source,
                            block.first_frame + offset as u64,
                            channel
                        ),
                    "decoded PCM differs from its source"
                );
            }
        }
        self.destination.push(block)?;
        self.digest.push(block)?;
        self.max_block = self.max_block.max(block.frames());
        self.next_frame += block.frames() as u64;
        Ok(())
    }
    fn reset(&mut self) {
        self.destination.reset();
        self.discontinuity = true;
    }
    fn is_closed(&self) -> bool {
        self.destination.is_closed()
    }
    fn ready_to_finish(&self) -> bool {
        self.destination.ready_to_finish()
    }
    fn flush(&mut self) -> Result<()> {
        self.destination.flush()
    }
    fn finish(&mut self) -> Result<()> {
        self.destination.finish()
    }
}

fn play<S: PcmSink>(destination: S) -> Result<(Engine<Checked<S>>, Value)> {
    let mut engine = Engine::new(Catalog::synthetic()?, Checked::new(destination))?;
    let sources = engine
        .catalog()
        .list()?
        .into_iter()
        .map(|entry| entry.id)
        .collect();
    engine.apply(0, Command::ReplaceQueue { sources })?;
    ensure!(
        engine.snapshot().state == State::Stopped,
        "queue replacement started playback"
    );
    engine.apply(1, Command::Play { index: 0 })?;
    let mut ended = false;
    for _ in 0..1000 {
        if engine.render_next(997)? == Render::QueueEnd {
            ended = true;
            break;
        }
    }
    ensure!(ended, "demo render step budget exceeded");
    ensure!(
        engine.sink().digest.frames == 2 * SYNTHETIC_FRAMES,
        "incomplete queue output"
    );
    let receipt = json!({"queue_revision":engine.queue().revision,
        "frames":engine.sink().digest.frames,"samples":engine.sink().digest.total_samples,
        "sample_rate":SAMPLE_RATE,"channels":CHANNELS,"pcm_sha256":engine.sink().digest.checksum(),
        "max_block_frames":engine.sink().max_block,"state":"ended","pcm_exact":true,
        "physical_outputs":0,"network_clients":0,"master_linked":false});
    Ok((engine, receipt))
}

fn verify_wave(file: &mut File, checksum: &str) -> Result<u64> {
    file.seek(SeekFrom::Start(0))?;
    let mut header = [0; 44];
    file.read_exact(&mut header)?;
    ensure!(
        &header[..4] == b"RIFF" && &header[8..16] == b"WAVEfmt ",
        "invalid WAV output"
    );
    let data_size = u32::from_le_bytes(header[40..44].try_into()?);
    ensure!(
        u64::from(data_size) == 2 * SYNTHETIC_FRAMES * u64::from(CHANNELS) * 2,
        "invalid WAV frame count"
    );
    ensure!(
        file.metadata()?.len() == u64::from(data_size) + 44,
        "invalid WAV file size"
    );
    let mut hash = Sha256::new();
    let mut buffer = [0; 4096];
    loop {
        let size = file.read(&mut buffer)?;
        if size == 0 {
            break;
        }
        hash.update(&buffer[..size]);
    }
    ensure!(
        format!("{:x}", hash.finalize()) == checksum,
        "WAV PCM checksum mismatch"
    );
    Ok(file.metadata()?.len())
}

fn main() -> Result<()> {
    ensure!(
        std::env::args().skip(1).all(|arg| arg == "demo"),
        "usage: moli-music-engine-poc [demo]"
    );
    let (_, null_receipt) = play(NullSink::new())?;
    let mut file = tempfile::tempfile()?;
    let (wave_engine, mut wave) = play(WavSink::new(file.try_clone()?, SAMPLE_RATE, CHANNELS)?)?;
    let checksum = wave_engine.sink().digest.checksum();
    drop(wave_engine);
    wave["wav_file_bytes"] = json!(verify_wave(&mut file, &checksum)?);
    ensure!(
        null_receipt["pcm_sha256"] == wave["pcm_sha256"],
        "null and WAV output differ"
    );
    println!(
        "{}",
        json!({"null":null_receipt,"wav":wave,"automatic_playback_on_create":false,
        "full_music_engine":false,"control_transport":"local typed API; MQTT adapter pending"})
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifier_accepts_exact_seek_and_replay_without_erasing_written_history() {
        let mut engine =
            Engine::new(Catalog::synthetic().unwrap(), Checked::new(NullSink::new())).unwrap();
        let source = engine.catalog().list().unwrap().remove(0).id;
        engine
            .apply(
                0,
                Command::ReplaceQueue {
                    sources: vec![source],
                },
            )
            .unwrap();
        engine.apply(1, Command::Play { index: 0 }).unwrap();
        engine.render_next(100).unwrap();
        engine.apply(1, Command::Seek { frame: 37 }).unwrap();
        engine.render_next(100).unwrap();
        engine.apply(1, Command::Stop).unwrap();
        engine.apply(1, Command::Play { index: 0 }).unwrap();
        engine.render_next(50).unwrap();
        assert_eq!(engine.snapshot().state, State::Playing);
        assert_eq!(engine.sink().digest.frames, 250);
        assert_eq!(engine.sink().destination.frames, 250);
        assert_eq!(
            engine.sink().digest.checksum(),
            engine.sink().destination.checksum()
        );
    }
}
