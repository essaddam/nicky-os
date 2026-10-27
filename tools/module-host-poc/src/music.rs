//! The native local audio engine as a module. Its only output is NullSink.
use crate::adapter::{Adapter, Observation};
use anyhow::{Context, Result, ensure};
use moli_core::{
    Access, Device, DeviceId, InstanceId, Kind, PointId, PointSpec, Semantic, Unit, Value,
};
use moli_music_engine_poc::{
    engine::{Command, Engine, State},
    sink::NullSink,
    source::{Catalog, SAMPLE_RATE},
};
use serde_json::{Value as Json, json};
use std::sync::{Mutex, PoisonError};

pub fn build(instance: &InstanceId) -> Result<Box<dyn Adapter>> {
    let id = DeviceId::new(instance, "native-null-player");
    let catalog = Catalog::synthetic()?;
    let entries = catalog.list()?;
    let mut engine = Engine::new(catalog, NullSink::new())?;
    engine.apply(
        0,
        Command::ReplaceQueue {
            sources: entries.iter().map(|e| e.id.clone()).collect(),
        },
    )?;
    let point = |key: &str, kind: Kind, read: bool, write: bool, unit: Option<Unit>| PointSpec {
        key: key.into(),
        label: key.into(),
        kind,
        access: Access { read, write },
        unit,
        semantic: Semantic::Other,
    };
    let numeric = || Kind::Numeric {
        min: Some(0.0),
        max: None,
        step: Some(1.0),
    };
    let description = vec![Device {
        id: id.clone(),
        instance: instance.clone(),
        native_name: "Native music engine · null fixture".into(),
        manufacturer: Some("Moli".into()),
        model: Some("Rust audio T0".into()),
        description: Some("Synthetic local WAV; null output; no audio device".into()),
        native_room: None,
        members: vec![],
        points: vec![
            point(
                "state",
                Kind::Enum {
                    values: ["stopped", "playing", "paused", "ended", "faulted"]
                        .into_iter()
                        .map(Into::into)
                        .collect(),
                },
                true,
                false,
                None,
            ),
            point(
                "control",
                Kind::Enum {
                    values: ["play", "pause", "resume", "stop", "seek_start"]
                        .into_iter()
                        .map(Into::into)
                        .collect(),
                },
                false,
                true,
                None,
            ),
            point(
                "play_index",
                Kind::Numeric {
                    min: Some(0.0),
                    max: Some(1.0),
                    step: Some(1.0),
                },
                false,
                true,
                None,
            ),
            point("position_frames", numeric(), true, false, None),
            point("rendered_frames", numeric(), true, false, None),
            point("queue_revision", numeric(), true, false, None),
            point("queue_entries", numeric(), true, false, None),
            point("track", Kind::Text, true, false, None),
            point("epoch", numeric(), true, false, None),
            point(
                "sample_rate",
                numeric(),
                true,
                false,
                Some(Unit::Other("Hz".into())),
            ),
        ],
    }];
    Ok(Box::new(Music {
        id,
        description,
        engine: Mutex::new(engine),
        metadata: json!({"output":"null","source":"generated_local_pcm16_stereo_48000","catalogue":entries,"queue_is_fixed_in_this_poc":true,"pcm_on_mqtt":false,"physical_executor":false}),
    }))
}
struct Music {
    id: DeviceId,
    description: Vec<Device>,
    engine: Mutex<Engine<NullSink>>,
    metadata: Json,
}
impl Adapter for Music {
    fn take_description(&mut self) -> Vec<Device> {
        std::mem::take(&mut self.description)
    }
    fn cold_metadata(&self) -> Json {
        self.metadata.clone()
    }
    fn runtime_metrics(&self) -> Json {
        let engine = self.engine.lock().unwrap_or_else(PoisonError::into_inner);
        json!({"rendered_frames":engine.sink().frames,"samples":engine.sink().total_samples,
            "output":"null","physical_outputs":0,"pcm_on_mqtt":false})
    }
    fn observe(&mut self, _step: u64) -> Result<Vec<Observation>> {
        let mut engine = self.engine.lock().unwrap_or_else(PoisonError::into_inner);
        if engine.snapshot().state == State::Playing {
            let _ = engine.render_next(1024)?;
        }
        let snapshot = engine.snapshot();
        let state = match snapshot.state {
            State::Stopped => "stopped",
            State::Playing => "playing",
            State::Paused => "paused",
            State::Ended => "ended",
            State::Faulted => "faulted",
        };
        Ok(vec![
            ("state", Value::from(state)),
            ("position_frames", Value::Int(snapshot.frame as i64)),
            ("rendered_frames", Value::Int(engine.sink().frames as i64)),
            ("queue_revision", Value::Int(snapshot.queue_revision as i64)),
            (
                "queue_entries",
                Value::Int(engine.queue().sources.len() as i64),
            ),
            (
                "track",
                snapshot
                    .source
                    .map_or(Value::Null, |id| Value::from(id.as_str())),
            ),
            ("epoch", Value::Int(snapshot.epoch as i64)),
            ("sample_rate", Value::Int(i64::from(SAMPLE_RATE))),
        ]
        .into_iter()
        .map(|(key, value)| Observation {
            point: PointId::new(&self.id, key),
            value,
        })
        .collect())
    }
    fn simulate_command(&self, point: &PointId, value: &Value) -> Result<Json> {
        let (device, key) = point.split().context("invalid music point")?;
        ensure!(device == self.id, "unknown music device");
        let mut engine = self.engine.lock().unwrap_or_else(PoisonError::into_inner);
        let command = match (key, value) {
            ("control", Value::Text(s)) => match s.as_ref() {
                "play" => Command::Play { index: 0 },
                "pause" => Command::Pause,
                "resume" => Command::Resume,
                "stop" => Command::Stop,
                "seek_start" => Command::Seek { frame: 0 },
                _ => anyhow::bail!("unknown music control"),
            },
            ("play_index", Value::Int(index)) if (0..=1).contains(index) => Command::Play {
                index: *index as usize,
            },
            ("play_index", Value::Float(index)) if *index == 0.0 || *index == 1.0 => {
                Command::Play {
                    index: *index as usize,
                }
            }
            _ => anyhow::bail!("read-only or invalid music command"),
        };
        let operation = serde_json::to_value(&command)?;
        let revision = engine.snapshot().queue_revision;
        let _ = engine.apply(revision, command)?;
        Ok(json!({"operation":operation,"output":"null","physical_sent":false}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moli_music_engine_poc::source::SYNTHETIC_FRAMES;
    #[test]
    fn native_music_module_starts_stopped_and_only_renders_after_explicit_play() {
        let mut module = build(&InstanceId::from("music")).unwrap();
        let devices = module.take_description();
        let id = &devices[0].id;
        let initial = module.observe(0).unwrap();
        assert!(initial.iter().any(|v|v.point==PointId::new(id,"rendered_frames")&&v.value==Value::Int(0)));
        module
            .simulate_command(&PointId::new(id, "control"), &Value::from("play"))
            .unwrap();
        let played = module.observe(1).unwrap();
        assert!(
            played
                .iter()
                .any(|v| v.point == PointId::new(id, "rendered_frames")
                    && v.value.as_f64().is_some_and(|n| n > 0.0))
        );
        module
            .simulate_command(&PointId::new(id, "control"), &Value::from("pause"))
            .unwrap();
        let paused = module.observe(2).unwrap();
        assert!(
            paused
                .iter()
                .any(|v| v.point == PointId::new(id, "state") && v.value == Value::from("paused"))
        );
        assert!(
            module
                .simulate_command(
                    &PointId::new(id, "position_frames"),
                    &Value::Int(SYNTHETIC_FRAMES as i64)
                )
                .is_err()
        );
        assert_eq!(module.cold_metadata()["pcm_on_mqtt"], false);
    }
}
