//! A pull-driven queue actor. Control events never contain PCM or source paths.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use crate::{
    decode::{Decoder, MAX_BLOCK_FRAMES},
    sink::PcmSink,
    source::{Catalog, SourceId},
};

const MAX_QUEUE_ENTRIES: usize = 16;
const MAX_COMMAND_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Stopped,
    Playing,
    Paused,
    Ended,
    Faulted,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueueSnapshot {
    pub revision: u64,
    pub sources: Vec<SourceId>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    ReplaceQueue { sources: Vec<SourceId> },
    Play { index: usize },
    Pause,
    Resume,
    Seek { frame: u64 },
    Stop,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    session: String,
    expected_revision: u64,
    command: Command,
}

#[derive(Debug, Serialize)]
pub struct Snapshot {
    pub session: String,
    pub epoch: u64,
    pub queue_revision: u64,
    pub state: State,
    pub entry: Option<usize>,
    pub source: Option<SourceId>,
    /// Frames admitted to the sink, not a claim about a physical DAC.
    pub frame: u64,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Render {
    Idle,
    Backpressure,
    Block {
        source: SourceId,
        epoch: u64,
        first_frame: u64,
        frames: usize,
    },
    TrackEnd {
        source: SourceId,
    },
    QueueEnd,
}

pub struct Engine<S: PcmSink> {
    catalog: Catalog,
    queue: QueueSnapshot,
    sink: S,
    decoder: Option<Decoder>,
    state: State,
    entry: Option<usize>,
    frame: u64,
    epoch: u64,
    session: String,
}

impl<S: PcmSink> Engine<S> {
    pub fn new(catalog: Catalog, sink: S) -> Result<Self> {
        let mut random = [0; 16];
        getrandom::fill(&mut random)
            .map_err(|_| anyhow::anyhow!("session identity unavailable"))?;
        Ok(Self {
            catalog,
            sink,
            queue: QueueSnapshot {
                revision: 0,
                sources: Vec::new(),
            },
            decoder: None,
            state: State::Stopped,
            entry: None,
            frame: 0,
            epoch: 0,
            session: random.iter().map(|byte| format!("{byte:02x}")).collect(),
        })
    }

    pub fn restore(catalog: Catalog, sink: S, queue: QueueSnapshot) -> Result<Self> {
        let mut engine = Self::new(catalog, sink)?;
        engine.validate_sources(&queue.sources)?;
        engine.queue = queue;
        Ok(engine)
    }

    pub fn queue(&self) -> QueueSnapshot {
        self.queue.clone()
    }
    pub fn sink(&self) -> &S {
        &self.sink
    }
    pub fn sink_mut(&mut self) -> &mut S {
        &mut self.sink
    }
    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            session: self.session.clone(),
            epoch: self.epoch,
            queue_revision: self.queue.revision,
            state: self.state,
            entry: self.entry,
            source: self
                .entry
                .and_then(|index| self.queue.sources.get(index))
                .cloned(),
            frame: self.frame,
        }
    }

    pub fn command_bytes(&mut self, bytes: &[u8]) -> Result<Snapshot> {
        ensure!(
            bytes.len() <= MAX_COMMAND_BYTES,
            "control envelope exceeds bound"
        );
        let envelope: Envelope =
            serde_json::from_slice(bytes).context("invalid control envelope")?;
        ensure!(envelope.session == self.session, "stale process session");
        self.apply(envelope.expected_revision, envelope.command)
    }

    pub fn apply(&mut self, revision: u64, command: Command) -> Result<Snapshot> {
        ensure!(revision == self.queue.revision, "queue revision changed");
        match command {
            Command::ReplaceQueue { sources } => self.replace_queue(sources)?,
            Command::Play { index } => {
                ensure!(!self.sink.is_closed(), "output sink is closed");
                let source = self
                    .queue
                    .sources
                    .get(index)
                    .context("queue is empty or entry does not exist")?;
                let decoder = self.catalog.open(source)?;
                let epoch = self.next_epoch()?;
                self.sink.reset();
                self.decoder = Some(decoder);
                self.entry = Some(index);
                self.epoch = epoch;
                self.frame = 0;
                self.state = State::Playing;
            }
            Command::Pause => {
                ensure!(
                    self.state == State::Playing,
                    "pause requires active playback"
                );
                self.state = State::Paused;
            }
            Command::Resume => {
                ensure!(
                    self.state == State::Paused,
                    "resume requires paused playback"
                );
                self.state = State::Playing;
            }
            Command::Seek { frame } => {
                ensure!(
                    matches!(self.state, State::Playing | State::Paused),
                    "seek requires a current decoder"
                );
                let epoch = self.next_epoch()?;
                let decoder = self.decoder.as_mut().context("decoder is absent")?;
                ensure!(
                    frame <= decoder.total_frames(),
                    "seek exceeds source duration"
                );
                if let Err(error) = decoder.seek(frame) {
                    self.fault();
                    return Err(error);
                }
                self.sink.reset();
                self.epoch = epoch;
                self.frame = frame;
            }
            Command::Stop => self.stop()?,
        }
        Ok(self.snapshot())
    }

    fn next_epoch(&self) -> Result<u64> {
        self.epoch.checked_add(1).context("session epoch exhausted")
    }

    fn validate_sources(&self, sources: &[SourceId]) -> Result<()> {
        ensure!(
            sources.len() <= MAX_QUEUE_ENTRIES,
            "queue entry bound exceeded"
        );
        for source in sources {
            ensure!(
                self.catalog.contains(source)?,
                "source is not in the approved catalogue"
            );
        }
        Ok(())
    }

    fn replace_queue(&mut self, sources: Vec<SourceId>) -> Result<()> {
        self.validate_sources(&sources)?;
        let revision = self
            .queue
            .revision
            .checked_add(1)
            .context("queue revision exhausted")?;
        self.stop()?;
        self.queue = QueueSnapshot { revision, sources };
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        let epoch = self.next_epoch()?;
        if let Err(error) = self.sink.flush() {
            self.fault();
            return Err(error);
        }
        self.decoder = None;
        self.sink.reset();
        self.entry = None;
        self.frame = 0;
        self.epoch = epoch;
        self.state = State::Stopped;
        Ok(())
    }

    fn fault(&mut self) {
        self.decoder = None;
        self.sink.reset();
        self.state = State::Faulted;
    }

    /// Returns without decoding when paused/stopped or when the sink is full.
    /// The call performs at most one bounded source read and one sink push.
    pub fn render_next(&mut self, max_frames: usize) -> Result<Render> {
        ensure!(
            (1..=MAX_BLOCK_FRAMES).contains(&max_frames),
            "render block bound exceeded"
        );
        if self.state != State::Playing {
            return Ok(Render::Idle);
        }
        if self.sink.is_closed() {
            self.fault();
            anyhow::bail!("output sink is closed");
        }
        if !self.sink.has_capacity() {
            return Ok(Render::Backpressure);
        }
        let block = match self
            .decoder
            .as_mut()
            .context("decoder is absent")?
            .read(max_frames)
        {
            Ok(block) => block,
            Err(error) => {
                self.fault();
                return Err(error);
            }
        };
        if let Some(mut block) = block {
            block.epoch = self.epoch;
            let result = Render::Block {
                source: block.source.clone(),
                epoch: self.epoch,
                first_frame: block.first_frame,
                frames: block.frames(),
            };
            if let Err(error) = self.sink.push(&block) {
                self.fault();
                return Err(error);
            }
            self.frame = block.first_frame + block.frames() as u64;
            return Ok(result);
        }
        self.end_track()
    }

    fn end_track(&mut self) -> Result<Render> {
        let index = self.entry.context("queue entry is absent")?;
        if index + 1 < self.queue.sources.len() {
            let source = self.queue.sources[index].clone();
            match self.catalog.open(&self.queue.sources[index + 1]) {
                Ok(decoder) => {
                    self.decoder = Some(decoder);
                    self.entry = Some(index + 1);
                    self.frame = 0;
                }
                Err(error) => {
                    self.fault();
                    return Err(error);
                }
            }
            // Normal track progression preserves queued tail blocks and its epoch.
            Ok(Render::TrackEnd { source })
        } else {
            if !self.sink.ready_to_finish() {
                return Ok(Render::Backpressure);
            }
            if let Err(error) = self.sink.finish() {
                self.fault();
                return Err(error);
            }
            self.decoder = None;
            self.state = State::Ended;
            Ok(Render::QueueEnd)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        decode::PcmBlock,
        sink::{BoundedSink, NullSink, WavSink},
        source::{SYNTHETIC_FRAMES, expected_sample},
    };

    fn engine<S: PcmSink>(sink: S) -> Engine<S> {
        let mut engine = Engine::new(Catalog::synthetic().unwrap(), sink).unwrap();
        let sources = engine
            .catalog()
            .list()
            .unwrap()
            .into_iter()
            .map(|d| d.id)
            .collect();
        engine.apply(0, Command::ReplaceQueue { sources }).unwrap();
        engine
    }

    #[test]
    fn construction_restore_and_replacement_never_start_audio() {
        let mut current = engine(NullSink::new());
        assert_eq!(current.snapshot().state, State::Stopped);
        assert_eq!(current.render_next(100).unwrap(), Render::Idle);
        let saved = current.queue();
        let mut restored =
            Engine::restore(Catalog::synthetic().unwrap(), NullSink::new(), saved).unwrap();
        assert_eq!(restored.snapshot().state, State::Stopped);
        assert_ne!(current.snapshot().session, restored.snapshot().session);
        assert_eq!(restored.render_next(100).unwrap(), Render::Idle);
        current.apply(1, Command::Play { index: 0 }).unwrap();
        current
            .apply(
                1,
                Command::ReplaceQueue {
                    sources: Vec::new(),
                },
            )
            .unwrap();
        assert_eq!(current.snapshot().queue_revision, 2);
        assert_eq!(current.snapshot().state, State::Stopped);
        assert!(current.apply(2, Command::Play { index: 0 }).is_err());
    }

    #[test]
    fn revisions_unknown_sources_and_malformed_envelopes_do_not_mutate_state() {
        let mut engine = engine(NullSink::new());
        assert!(engine.apply(0, Command::Play { index: 0 }).is_err());
        assert!(
            engine
                .apply(
                    1,
                    Command::ReplaceQueue {
                        sources: vec![SourceId::parse("not-registered").unwrap()]
                    }
                )
                .is_err()
        );
        let session = engine.snapshot().session;
        for bytes in [
            b"{}".as_slice(),
            b"{\"path\":\"/tmp/test\"}",
            &vec![0; MAX_COMMAND_BYTES + 1],
        ] {
            assert!(engine.command_bytes(bytes).is_err());
        }
        let command = serde_json::json!({"session":"old-session","expected_revision":1,"command":{"operation":"play","index":0}});
        assert!(
            engine
                .command_bytes(&serde_json::to_vec(&command).unwrap())
                .is_err()
        );
        let command = serde_json::json!({"session":session,"expected_revision":1,"command":{"operation":"play","index":0}});
        assert_eq!(
            engine
                .command_bytes(&serde_json::to_vec(&command).unwrap())
                .unwrap()
                .state,
            State::Playing
        );
    }

    #[test]
    fn pause_resume_seek_stop_and_backpressure_keep_exact_positions() {
        let mut engine = engine(BoundedSink::new(1).unwrap());
        engine.apply(1, Command::Play { index: 0 }).unwrap();
        engine.render_next(100).unwrap();
        assert_eq!(engine.snapshot().frame, 100);
        for _ in 0..1000 {
            assert_eq!(engine.render_next(100).unwrap(), Render::Backpressure);
        }
        assert_eq!(engine.snapshot().frame, 100);
        assert_eq!(engine.sink().queued_bytes(), 400);
        engine.apply(1, Command::Pause).unwrap();
        assert_eq!(engine.render_next(100).unwrap(), Render::Idle);
        engine.apply(1, Command::Resume).unwrap();
        let old_epoch = engine.sink_mut().pop().unwrap().epoch;
        engine.render_next(100).unwrap();
        engine.apply(1, Command::Pause).unwrap();
        engine.apply(1, Command::Seek { frame: 37 }).unwrap();
        assert!(engine.sink_mut().pop().is_none());
        assert_eq!(engine.snapshot().state, State::Paused);
        engine.apply(1, Command::Resume).unwrap();
        engine.render_next(100).unwrap();
        let block = engine.sink_mut().pop().unwrap();
        assert_eq!(block.first_frame, 37);
        assert!(block.epoch > old_epoch);
        assert_eq!(block.samples[0], expected_sample(&block.source, 37, 0));
        engine
            .apply(
                1,
                Command::Seek {
                    frame: SYNTHETIC_FRAMES + 1,
                },
            )
            .unwrap_err();
        engine.render_next(100).unwrap();
        engine.apply(1, Command::Stop).unwrap();
        assert!(engine.sink_mut().pop().is_none());
        assert_eq!(engine.render_next(100).unwrap(), Render::Idle);
        assert!(engine.apply(1, Command::Resume).is_err());
    }

    struct FailingSink;
    impl PcmSink for FailingSink {
        fn has_capacity(&self) -> bool {
            true
        }
        fn push(&mut self, _: &PcmBlock) -> Result<()> {
            anyhow::bail!("synthetic consumer failure")
        }
    }
    #[test]
    fn consumer_failure_faults_without_a_hidden_retry() {
        let mut engine = engine(FailingSink);
        engine.apply(1, Command::Play { index: 0 }).unwrap();
        assert!(engine.render_next(100).is_err());
        assert_eq!(engine.snapshot().state, State::Faulted);
        assert_eq!(engine.render_next(100).unwrap(), Render::Idle);
    }

    #[test]
    fn complete_queue_has_exact_frames_and_a_stable_eof() {
        let mut engine = engine(NullSink::new());
        engine.apply(1, Command::Play { index: 0 }).unwrap();
        let mut ends = 0;
        for _ in 0..100 {
            match engine.render_next(511).unwrap() {
                Render::TrackEnd { .. } => ends += 1,
                Render::QueueEnd => break,
                _ => {}
            }
        }
        assert_eq!(ends, 1);
        assert_eq!(engine.snapshot().state, State::Ended);
        assert_eq!(engine.sink().frames, 2 * SYNTHETIC_FRAMES);
        assert_eq!(engine.render_next(1).unwrap(), Render::Idle);
    }

    #[test]
    fn stop_flushes_a_valid_partial_wav_and_closed_output_rejects_play() {
        use std::io::{Read, Seek, SeekFrom};
        let file = tempfile::tempfile().unwrap();
        let mut reader = file.try_clone().unwrap();
        let mut engine = engine(WavSink::new(file, 48_000, 2).unwrap());
        engine.apply(1, Command::Play { index: 0 }).unwrap();
        engine.render_next(25).unwrap();
        engine.apply(1, Command::Stop).unwrap();
        reader.seek(SeekFrom::Start(0)).unwrap();
        let mut header = [0; 44];
        reader.read_exact(&mut header).unwrap();
        assert_eq!(u32::from_le_bytes(header[40..44].try_into().unwrap()), 100);
        assert_eq!(reader.metadata().unwrap().len(), 144);
        reader.seek(SeekFrom::End(0)).unwrap();
        engine.apply(1, Command::Play { index: 0 }).unwrap();
        for _ in 0..100 {
            if engine.render_next(1024).unwrap() == Render::QueueEnd {
                break;
            }
        }
        assert_eq!(engine.snapshot().state, State::Ended);
        assert_eq!(engine.sink().frames(), 25 + 2 * SYNTHETIC_FRAMES);
        assert!(engine.apply(1, Command::Play { index: 0 }).is_err());
        assert_eq!(engine.snapshot().state, State::Ended);
    }

    #[test]
    fn eof_waits_for_the_last_queued_block_to_be_consumed() {
        let mut engine = engine(BoundedSink::new(2).unwrap());
        let source = engine.queue().sources.remove(0);
        engine
            .apply(
                1,
                Command::ReplaceQueue {
                    sources: vec![source],
                },
            )
            .unwrap();
        engine.apply(2, Command::Play { index: 0 }).unwrap();
        engine
            .apply(
                2,
                Command::Seek {
                    frame: SYNTHETIC_FRAMES - 1,
                },
            )
            .unwrap();
        engine.render_next(1).unwrap();
        assert_eq!(engine.render_next(1).unwrap(), Render::Backpressure);
        assert_eq!(engine.snapshot().state, State::Playing);
        assert_eq!(
            engine.sink_mut().pop().unwrap().first_frame,
            SYNTHETIC_FRAMES - 1
        );
        assert_eq!(engine.render_next(1).unwrap(), Render::QueueEnd);
        assert_eq!(engine.snapshot().state, State::Ended);
    }
}
