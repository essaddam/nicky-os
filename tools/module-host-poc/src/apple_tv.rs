//! Apple TV normalized-input fixture, not a physical pyatv driver.
//! No network, pairing, credentials or command execution exists in this module.
use crate::adapter::{Adapter, Observation};
use anyhow::{Context, Result, bail, ensure};
use moli_core::{
    Access, Device, DeviceId, InstanceId, Kind, PointId, PointSpec, Semantic, Unit, Value,
};
use serde::{Deserialize, Deserializer};
use serde_json::{Value as Json, json};
use std::collections::BTreeMap;
use std::sync::Arc;

const MAX_INPUT_BYTES: usize = 128 * 1024;
const MAX_TEXT_BYTES: usize = 4096;
const MAX_ID_BYTES: usize = 128;
const MAX_LABEL_BYTES: usize = 256;
const MAX_SECONDS: f64 = 7.0 * 24.0 * 3600.0;
const MAX_TIMESTAMP_MS: u64 = 4_102_444_800_000;
const FIXTURE_TIMESTAMP_MS: u64 = 1_767_225_600_000;
const DEVICE_NATIVE: &str = "apple-tv-fixture";
const APPS: &[(&str, &str)] = &[
    ("fixture.video", "Vidéo de démonstration"),
    ("fixture.music", "Musique de démonstration"),
    ("fixture.settings", "Réglages de démonstration"),
];
const SOURCES: &[(&str, &str)] = &[
    ("fixture.home", "Accueil fictif"),
    ("fixture.input", "Entrée fictive"),
];
const STATES: &[&str] = &["playing", "paused", "idle", "unavailable"];
const OBSERVABLE_KEYS: &[&str] = &[
    "state",
    "app_id",
    "source",
    "volume",
    "muted",
    "title",
    "artist",
    "album",
    "duration",
    "position",
    "position_updated_at",
];

/// Absent preserves the previous value; JSON null explicitly makes it unknown.
#[derive(Default, Debug)]
enum Field<T> {
    #[default]
    Absent,
    Unknown,
    Known(T),
}
impl<'de, T: Deserialize<'de>> Deserialize<'de> for Field<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        Ok(match Option::<T>::deserialize(deserializer)? {
            Some(value) => Self::Known(value),
            None => Self::Unknown,
        })
    }
}

/// The fixture's normalized transport format. Time is Unix epoch milliseconds;
/// duration/position are seconds. Display names never identify applications.
#[derive(Default, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Snapshot {
    state: Field<String>,
    app_id: Field<String>,
    app_name: Field<String>,
    source: Field<String>,
    volume: Field<f64>,
    muted: Field<bool>,
    title: Field<String>,
    artist: Field<String>,
    album: Field<String>,
    duration: Field<f64>,
    position: Field<f64>,
    position_updated_at: Field<u64>,
}
impl Snapshot {
    fn parse(payload: &[u8]) -> Result<Self> {
        ensure!(
            payload.len() <= MAX_INPUT_BYTES,
            "normalized snapshot exceeds byte bound"
        );
        let snapshot: Self =
            serde_json::from_slice(payload).context("invalid normalized snapshot")?;
        snapshot.validate()?;
        Ok(snapshot)
    }
    fn validate(&self) -> Result<()> {
        if let Field::Known(state) = &self.state {
            ensure!(
                STATES.contains(&state.as_str()),
                "unsupported playback state"
            );
        }
        if let Field::Known(native) = &self.app_id {
            ensure!(
                !native.is_empty()
                    && native.len() <= MAX_ID_BYTES
                    && APPS.iter().any(|(id, _)| id == native),
                "unknown application ID"
            );
        }
        if let Field::Known(native) = &self.source {
            ensure!(
                native.len() <= MAX_ID_BYTES && SOURCES.iter().any(|(id, _)| id == native),
                "unknown source ID"
            );
        }
        if let Field::Known(name) = &self.app_name {
            ensure!(
                name.len() <= MAX_LABEL_BYTES,
                "application name exceeds UTF-8 byte bound"
            );
        }
        for text in [&self.title, &self.artist, &self.album] {
            if let Field::Known(text) = text {
                ensure!(
                    text.len() <= MAX_TEXT_BYTES,
                    "free text exceeds UTF-8 byte bound"
                );
            }
        }
        if let Field::Known(volume) = self.volume {
            ensure!(
                volume.is_finite() && (0.0..=1.0).contains(&volume),
                "volume outside declared range"
            );
        }
        for number in [&self.duration, &self.position] {
            if let Field::Known(value) = number {
                ensure!(
                    value.is_finite() && (0.0..=MAX_SECONDS).contains(value),
                    "media time outside declared range"
                );
            }
        }
        if let (Field::Known(duration), Field::Known(position)) = (&self.duration, &self.position) {
            ensure!(
                position <= duration,
                "position exceeds duration in this snapshot"
            );
        }
        if let Field::Known(timestamp) = self.position_updated_at {
            ensure!(
                timestamp <= MAX_TIMESTAMP_MS,
                "position timestamp outside declared range"
            );
        }
        Ok(())
    }
}

struct AppleTv {
    device: DeviceId,
    description: Vec<Device>,
    values: BTreeMap<&'static str, Value>,
    last_step: Option<u64>,
}
fn enumeration(values: impl IntoIterator<Item = &'static str>) -> Kind {
    Kind::Enum {
        values: values.into_iter().map(Arc::from).collect(),
    }
}
fn point(
    key: &str,
    label: &str,
    kind: Kind,
    unit: Option<Unit>,
    write: bool,
    read: bool,
) -> PointSpec {
    PointSpec {
        key: key.into(),
        label: label.into(),
        kind,
        unit,
        access: Access { read, write },
        semantic: if write {
            Semantic::Control
        } else {
            Semantic::Other
        },
    }
}
fn numeric(min: f64, max: f64) -> Kind {
    Kind::Numeric {
        min: Some(min),
        max: Some(max),
        step: None,
    }
}
impl AppleTv {
    fn new(instance: &InstanceId) -> Self {
        let device = DeviceId::new(instance, DEVICE_NATIVE);
        let description = Device {
            id: device.clone(),
            instance: instance.clone(),
            native_name: "Lecteur fictif Apple TV".into(),
            manufacturer: Some("Fixture".into()),
            model: Some("Entrée normalisée POC".into()),
            description: None,
            native_room: None,
            members: Vec::new(),
            points: vec![
                point(
                    "state",
                    "Lecture",
                    enumeration(STATES.iter().copied()),
                    None,
                    false,
                    true,
                ),
                point(
                    "app_id",
                    "Application",
                    enumeration(APPS.iter().map(|(id, _)| *id)),
                    None,
                    false,
                    true,
                ),
                point(
                    "source",
                    "Source",
                    enumeration(SOURCES.iter().map(|(id, _)| *id)),
                    None,
                    false,
                    true,
                ),
                point(
                    "volume",
                    "Volume",
                    numeric(0.0, 1.0),
                    Some(Unit::Other("ratio".into())),
                    true,
                    true,
                ),
                point("muted", "Muet", Kind::Binary, None, false, true),
                point("title", "Titre", Kind::Text, None, false, true),
                point("artist", "Artiste", Kind::Text, None, false, true),
                point("album", "Album", Kind::Text, None, false, true),
                point(
                    "duration",
                    "Durée",
                    numeric(0.0, MAX_SECONDS),
                    Some(Unit::Second),
                    false,
                    true,
                ),
                point(
                    "position",
                    "Position",
                    numeric(0.0, MAX_SECONDS),
                    Some(Unit::Second),
                    false,
                    true,
                ),
                point(
                    "position_updated_at",
                    "Date de position",
                    numeric(0.0, MAX_TIMESTAMP_MS as f64),
                    Some(Unit::Other("unix_ms".into())),
                    false,
                    true,
                ),
                point(
                    "playback",
                    "Action de lecture",
                    enumeration(["play", "pause"]),
                    None,
                    true,
                    false,
                ),
                point(
                    "select_app",
                    "Choisir une application",
                    enumeration(APPS.iter().map(|(id, _)| *id)),
                    None,
                    true,
                    false,
                ),
            ],
        };
        Self {
            device,
            description: vec![description],
            values: OBSERVABLE_KEYS
                .iter()
                .map(|&key| (key, Value::Null))
                .collect(),
            last_step: None,
        }
    }

    fn ingest(&mut self, snapshot: Snapshot) -> Result<Vec<Observation>> {
        // Validate everything before any mutation, including snapshots constructed internally.
        snapshot.validate()?;
        let mut candidates = Vec::with_capacity(OBSERVABLE_KEYS.len());
        fn text(out: &mut Vec<(&'static str, Value)>, key: &'static str, field: Field<String>) {
            match field {
                Field::Absent => {}
                Field::Unknown => out.push((key, Value::Null)),
                Field::Known(text) => out.push((key, Value::Text(text.into()))),
            }
        }
        fn number(out: &mut Vec<(&'static str, Value)>, key: &'static str, field: Field<f64>) {
            match field {
                Field::Absent => {}
                Field::Unknown => out.push((key, Value::Null)),
                Field::Known(value) => out.push((key, Value::Float(value))),
            }
        }
        text(&mut candidates, "state", snapshot.state);
        text(&mut candidates, "app_id", snapshot.app_id);
        text(&mut candidates, "source", snapshot.source);
        text(&mut candidates, "title", snapshot.title);
        text(&mut candidates, "artist", snapshot.artist);
        text(&mut candidates, "album", snapshot.album);
        number(&mut candidates, "volume", snapshot.volume);
        number(&mut candidates, "duration", snapshot.duration);
        number(&mut candidates, "position", snapshot.position);
        match snapshot.muted {
            Field::Absent => {}
            Field::Unknown => candidates.push(("muted", Value::Null)),
            Field::Known(value) => candidates.push(("muted", Value::Bool(value))),
        }
        match snapshot.position_updated_at {
            Field::Absent => {}
            Field::Unknown => candidates.push(("position_updated_at", Value::Null)),
            Field::Known(value) => {
                candidates.push(("position_updated_at", Value::Int(value as i64)))
            }
        }
        // app_name is cold input metadata only. The declared catalogue is authoritative;
        // a name-only observation never invents or changes the selected native ID.
        let mut observations = Vec::with_capacity(candidates.len());
        for (key, value) in candidates {
            if self.values.get(key) == Some(&value) {
                continue;
            }
            self.values.insert(key, value.clone());
            observations.push(Observation {
                point: PointId::new(&self.device, key),
                value,
            });
        }
        Ok(observations)
    }
    fn ingest_bytes(&mut self, payload: &[u8]) -> Result<Vec<Observation>> {
        self.ingest(Snapshot::parse(payload)?)
    }
}
impl Adapter for AppleTv {
    fn take_description(&mut self) -> Vec<Device> {
        std::mem::take(&mut self.description)
    }
    fn observe(&mut self, step: u64) -> Result<Vec<Observation>> {
        if self.last_step == Some(step) {
            return Ok(Vec::new());
        }
        let snapshot = if self.last_step.is_none() {
            json!({"state":"playing","app_id":"fixture.video","source":"fixture.home","volume":0.25,"muted":false,"title":"Séquence fictive","artist":"Artiste fictif","album":"Album fictif","duration":180.0,"position":0.0,"position_updated_at":FIXTURE_TIMESTAMP_MS})
        } else if step % 64 == 63 {
            json!({"state":"unavailable","app_id":null,"source":null,"volume":null,"muted":null,"title":null,"artist":null,"album":null,"duration":null,"position":null,"position_updated_at":null})
        } else if step.is_multiple_of(64) {
            json!({"state":"playing","app_id":"fixture.video","source":"fixture.home","volume":0.25,"muted":false,"title":"Séquence fictive","artist":"Artiste fictif","album":"Album fictif","duration":180.0,"position":0.0,"position_updated_at":FIXTURE_TIMESTAMP_MS+(step%86400)*1000})
        } else {
            let mut snapshot = json!({"position":(step%180) as f64,"position_updated_at":FIXTURE_TIMESTAMP_MS+(step%86400)*1000});
            if step.is_multiple_of(8) {
                snapshot["state"] = json!(if (step / 8).is_multiple_of(2) {
                    "playing"
                } else {
                    "paused"
                });
            }
            if step.is_multiple_of(4) {
                snapshot["volume"] = json!(0.2 + (step % 6) as f64 / 10.0);
                snapshot["muted"] = json!(step / 4 % 2 == 1);
            }
            if step.is_multiple_of(16) {
                snapshot["app_id"] = json!(if (step / 16).is_multiple_of(2) {
                    "fixture.video"
                } else {
                    "fixture.music"
                });
                snapshot["title"] = json!(if (step / 16).is_multiple_of(2) {
                    "Séquence fictive"
                } else {
                    "Morceau fictif"
                });
            }
            snapshot
        };
        let observations = self.ingest_bytes(&serde_json::to_vec(&snapshot)?)?;
        self.last_step = Some(step);
        Ok(observations)
    }
    fn simulate_command(&self, point: &PointId, value: &Value) -> Result<Json> {
        let (device, key) = point.split().context("invalid point identity")?;
        ensure!(device == self.device, "command belongs to another device");
        let request = match key {
            "playback" => {
                let Value::Text(native) = value else {
                    bail!("playback requires a native enum value");
                };
                ensure!(
                    matches!(native.as_ref(), "play" | "pause"),
                    "undeclared playback operation"
                );
                json!({"operation":native.as_ref()})
            }
            "volume" => {
                let volume = value.as_f64().context("volume must be numeric")?;
                ensure!(
                    volume.is_finite() && (0.0..=1.0).contains(&volume),
                    "volume outside declared range"
                );
                json!({"operation":"set_volume","volume":volume})
            }
            "select_app" => {
                let Value::Text(native) = value else {
                    bail!("application selection requires a native enum value");
                };
                ensure!(
                    !native.is_empty()
                        && native.len() <= MAX_ID_BYTES
                        && APPS.iter().any(|(id, _)| id == &native.as_ref()),
                    "application is not declared"
                );
                json!({"operation":"select_app","app_id":native.as_ref()})
            }
            _ => bail!("point has no declared command capability"),
        };
        ensure!(
            serde_json::to_vec(&request)?.len() <= 256,
            "simulated request exceeds byte bound"
        );
        Ok(request)
    }
    fn enum_label(&self, point: &PointId, native: &str) -> String {
        let Some((device, key)) = point.split() else {
            return native.into();
        };
        if device != self.device {
            return native.into();
        }
        let labels = match key {
            "app_id" | "select_app" => APPS,
            "source" => SOURCES,
            _ => return native.into(),
        };
        labels
            .iter()
            .find(|(id, _)| *id == native)
            .map_or_else(|| native.to_owned(), |(_, label)| (*label).into())
    }
}

pub fn build(instance: &InstanceId) -> Result<Box<dyn Adapter>> {
    Ok(Box::new(AppleTv::new(instance)))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> AppleTv {
        AppleTv::new(&InstanceId::from("fixture-apple-tv"))
    }
    fn point(adapter: &AppleTv, key: &str) -> PointId {
        PointId::new(&adapter.device, key)
    }

    #[test]
    fn partial_snapshots_keep_secondary_channels_and_null_is_explicit_unknown() {
        let mut adapter = fixture();
        let first = adapter.observe(0).unwrap();
        assert_eq!(first.len(), OBSERVABLE_KEYS.len());
        let title = adapter.values["title"].clone();
        let position = adapter.values["position"].clone();
        let observations = adapter.ingest_bytes(br#"{"volume":0.6}"#).unwrap();
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].point, point(&adapter, "volume"));
        assert_eq!(adapter.values["title"], title);
        assert_eq!(adapter.values["position"], position);
        let observations = adapter.ingest_bytes(br#"{"artist":null}"#).unwrap();
        assert_eq!(observations.len(), 1);
        assert_eq!(adapter.values["artist"], Value::Null);
        assert_eq!(adapter.values["title"], title);
        assert!(adapter.observe(0).unwrap().is_empty());
    }
    #[test]
    fn catalogue_identity_is_native_and_names_never_select_an_application() {
        let mut adapter = fixture();
        let snapshot = json!({"app_name":"Vidéo de démonstration"});
        assert!(
            adapter
                .ingest_bytes(&serde_json::to_vec(&snapshot).unwrap())
                .unwrap()
                .is_empty()
        );
        assert_eq!(adapter.values["app_id"], Value::Null);
        assert!(
            adapter
                .ingest_bytes(
                    &serde_json::to_vec(&json!({"app_id":"Vidéo de démonstration"})).unwrap()
                )
                .is_err()
        );
        adapter
            .ingest_bytes(br#"{"app_id":"fixture.video"}"#)
            .unwrap();
        assert_eq!(adapter.values["app_id"], Value::from("fixture.video"));
        assert_eq!(
            adapter.enum_label(&point(&adapter, "app_id"), "fixture.video"),
            "Vidéo de démonstration"
        );
        assert_ne!(
            adapter.values["app_id"],
            Value::from("Vidéo de démonstration")
        );
    }
    #[test]
    fn commands_validate_capabilities_and_never_change_observations() {
        let mut adapter = fixture();
        adapter.observe(0).unwrap();
        let before = adapter.values.clone();
        assert_eq!(
            adapter
                .simulate_command(&point(&adapter, "playback"), &Value::from("pause"))
                .unwrap(),
            json!({"operation":"pause"})
        );
        assert_eq!(
            adapter
                .simulate_command(
                    &point(&adapter, "select_app"),
                    &Value::from("fixture.music")
                )
                .unwrap()["app_id"],
            "fixture.music"
        );
        assert!(
            adapter
                .simulate_command(&point(&adapter, "volume"), &Value::Float(0.75))
                .is_ok()
        );
        for volume in [f64::NAN, f64::INFINITY, -0.01, 1.01] {
            assert!(
                adapter
                    .simulate_command(&point(&adapter, "volume"), &Value::Float(volume))
                    .is_err()
            );
        }
        for (key, value) in [
            ("playback", Value::from("stop")),
            ("select_app", Value::from("Application libre")),
            ("muted", Value::Bool(true)),
            ("volume", Value::from("0.5")),
        ] {
            assert!(
                adapter
                    .simulate_command(&point(&adapter, key), &value)
                    .is_err()
            );
        }
        let foreign = PointId::new(
            &DeviceId::new(&InstanceId::from("other-fixture"), DEVICE_NATIVE),
            "volume",
        );
        assert!(
            adapter
                .simulate_command(&foreign, &Value::Float(0.5))
                .is_err()
        );
        assert_eq!(adapter.values, before);
    }
    #[test]
    fn long_utf8_titles_are_kept_whole_or_rejected_atomically() {
        let mut adapter = fixture();
        let title = "é".repeat(MAX_TEXT_BYTES / 2);
        adapter
            .ingest_bytes(&serde_json::to_vec(&json!({"title":title,"volume":0.25})).unwrap())
            .unwrap();
        let Value::Text(stored) = &adapter.values["title"] else {
            panic!("free text expected");
        };
        assert_eq!(stored.len(), MAX_TEXT_BYTES);
        assert_eq!(stored.as_ref(), title);
        let before = adapter.values.clone();
        assert!(
            adapter
                .ingest_bytes(
                    &serde_json::to_vec(
                        &json!({"title":"é".repeat(MAX_TEXT_BYTES/2+1),"volume":0.75})
                    )
                    .unwrap()
                )
                .is_err()
        );
        assert_eq!(adapter.values, before);
        assert!(
            adapter
                .ingest_bytes(&vec![b' '; MAX_INPUT_BYTES + 1])
                .is_err()
        );
        assert!(
            adapter
                .ingest_bytes(br#"{"position":3,"duration":2}"#)
                .is_err()
        );
        assert!(adapter.ingest_bytes(br#"{"state":"buffering"}"#).is_err());
    }
    #[test]
    fn schema_moves_once_and_fixture_changes_are_deterministic() {
        let mut left = fixture();
        let mut right = fixture();
        let schema = left.take_description();
        assert_eq!(schema.len(), 1);
        assert_eq!(schema[0].points.len(), OBSERVABLE_KEYS.len() + 2);
        assert!(left.take_description().is_empty());
        for key in ["state", "app_id", "source", "select_app", "playback"] {
            assert!(matches!(
                schema[0].point(key).unwrap().kind,
                Kind::Enum { .. }
            ));
        }
        for step in [0, 1, 4, 8, 16, 63, 64] {
            let a = left.observe(step).unwrap();
            let b = right.observe(step).unwrap();
            assert_eq!(a.len(), b.len());
            for (a, b) in a.iter().zip(b) {
                assert_eq!(a.point, b.point);
                assert_eq!(a.value, b.value);
            }
        }
    }
}
