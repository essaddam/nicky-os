//! Durable civil settings and UTC instants; never a clock or scheduler.
use crate::StateFile;
use moli_core::{DeviceId, Kind, TemporalFormat, TemporalSpec, Value};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemporalError {
    Value,
    Definition,
    Gap,
    Fold,
    Offset,
    Storage,
    Uncertain,
    Corrupt,
    Capacity,
    Abandoned,
}
impl std::fmt::Display for TemporalError {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use moli_i18n::tr;
        out.write_str(&match self {
            Self::Value => tr!("serveur.date.valeur"),
            Self::Definition => tr!("serveur.date.definition"),
            Self::Gap => tr!("serveur.date.inexistante"),
            Self::Fold => tr!("serveur.date.ambigue"),
            Self::Offset => tr!("serveur.date.decalage"),
            Self::Storage => tr!("serveur.date.persistance"),
            Self::Uncertain => tr!("serveur.date.incertain"),
            Self::Corrupt => tr!("serveur.date.corrompu"),
            Self::Capacity => tr!("serveur.date.capacite"),
            Self::Abandoned => tr!("serveur.date.abandonne"),
        })
    }
}
impl std::error::Error for TemporalError {}

/// One validator shared by configuration, commands and graph checks.
pub fn normalize_temporal(spec: &TemporalSpec, value: &Value) -> Result<Value, TemporalError> {
    let Value::Text(text) = value else {
        return Err(TemporalError::Value);
    };
    if !spec.is_valid() {
        return Err(TemporalError::Definition);
    }
    if !spec.accepts_shape(text) {
        return Err(TemporalError::Value);
    }
    if spec.format != TemporalFormat::Datetime {
        return Ok(value.clone());
    }
    let zone = jiff::tz::TimeZone::get(spec.timezone.as_deref().ok_or(TemporalError::Definition)?)
        .map_err(|_| TemporalError::Definition)?;
    let local: jiff::civil::DateTime = text[..19].parse().map_err(|_| TemporalError::Value)?;
    let timestamp = if text.ends_with('Z') {
        text.parse::<jiff::Timestamp>()
            .map_err(|_| TemporalError::Value)?
    } else if text.len() == 25 {
        let ts = text
            .parse::<jiff::Timestamp>()
            .map_err(|_| TemporalError::Value)?;
        if ts.to_zoned(zone).datetime() != local {
            return Err(TemporalError::Offset);
        }
        ts
    } else {
        let candidate = zone.to_ambiguous_timestamp(local);
        match candidate.offset() {
            jiff::tz::AmbiguousOffset::Gap { .. } => return Err(TemporalError::Gap),
            jiff::tz::AmbiguousOffset::Fold { .. } => return Err(TemporalError::Fold),
            jiff::tz::AmbiguousOffset::Unambiguous { .. } => {
                candidate.unambiguous().map_err(|_| TemporalError::Value)?
            }
        }
    };
    let canonical = timestamp.to_string();
    if canonical.len() != 20 || !spec.accepts_shape(&canonical) {
        return Err(TemporalError::Value);
    }
    Ok(Value::Text(canonical.into()))
}

/// Complete point validation, including time-zone resolution for temporal points.
pub fn validate_point_value(kind: &Kind, value: &Value) -> Result<Value, crate::CommandError> {
    if let Kind::Temporal { spec } = kind {
        normalize_temporal(spec, value).map_err(Into::into)
    } else {
        moli_core::validate(kind, value).map_err(Into::into)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    spec: TemporalSpec,
    value: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    version: u32,
    values: BTreeMap<DeviceId, Record>,
}
#[derive(Debug)]
pub(crate) struct Temporals {
    pub(super) file: StateFile,
    state: Option<State>,
    initialized: BTreeSet<DeviceId>,
    fenced: bool,
}
impl Temporals {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self {
            file: StateFile::new(path, 1024 * 1024),
            state: None,
            initialized: BTreeSet::new(),
            fenced: false,
        }
    }
    async fn ready(&mut self) -> Result<(), TemporalError> {
        if self.fenced {
            return Err(TemporalError::Uncertain);
        }
        if self.state.is_none() {
            let state = self
                .file
                .load::<State>()
                .await
                .map_err(|_| TemporalError::Corrupt)?
                .unwrap_or_else(|| State {
                    version: 1,
                    values: BTreeMap::new(),
                });
            if state.version != 1
                || state.values.len() > 128
                || state.values.iter().any(|(id, record)| {
                    !crate::counters::valid_device(id)
                        || normalize_temporal(&record.spec, &record.value)
                            .ok()
                            .as_ref()
                            != Some(&record.value)
                })
            {
                return Err(TemporalError::Corrupt);
            }
            self.state = Some(state);
        }
        Ok(())
    }
    async fn persist(&mut self, state: State) -> Result<(), TemporalError> {
        match self.file.write(&state).await {
            Ok(()) => {
                self.state = Some(state);
                Ok(())
            }
            Err(error) => {
                tracing::warn!(%error,"temporal setting was not acknowledged");
                if error.commit_unknown {
                    self.fenced = true;
                    Err(TemporalError::Uncertain)
                } else {
                    Err(TemporalError::Storage)
                }
            }
        }
    }
    pub async fn initialize(
        &mut self,
        id: &DeviceId,
        spec: TemporalSpec,
        initial: Value,
    ) -> Result<Value, TemporalError> {
        if !crate::counters::valid_device(id) {
            return Err(TemporalError::Definition);
        }
        let value = normalize_temporal(&spec, &initial)?;
        self.ready().await?;
        let mut state = self.state.clone().ok_or(TemporalError::Corrupt)?;
        if let Some(record) = state.values.get(id) {
            if record.spec != spec {
                return Err(TemporalError::Definition);
            }
        } else {
            if state.values.len() >= 128 {
                return Err(TemporalError::Capacity);
            }
            state.values.insert(id.clone(), Record { spec, value });
            self.persist(state).await?;
        }
        self.initialized.insert(id.clone());
        self.value(id).ok_or(TemporalError::Definition)
    }
    pub fn value(&self, id: &DeviceId) -> Option<Value> {
        self.state.as_ref()?.values.get(id).map(|r| r.value.clone())
    }
    pub fn devices(&self) -> impl Iterator<Item = &DeviceId> {
        self.initialized.iter()
    }
    pub async fn assign(
        &mut self,
        id: &DeviceId,
        spec: &TemporalSpec,
        value: &Value,
        abandoned: impl Fn() -> bool,
    ) -> Result<Value, TemporalError> {
        self.ready().await?;
        if abandoned() {
            return Err(TemporalError::Abandoned);
        }
        if !self.initialized.contains(id) {
            return Err(TemporalError::Definition);
        }
        let value = normalize_temporal(spec, value)?;
        let mut state = self.state.clone().ok_or(TemporalError::Corrupt)?;
        let record = state.values.get_mut(id).ok_or(TemporalError::Definition)?;
        if &record.spec != spec {
            return Err(TemporalError::Definition);
        }
        if record.value == value {
            return Ok(value);
        }
        record.value = value.clone();
        if abandoned() {
            return Err(TemporalError::Abandoned);
        }
        self.persist(state).await?;
        Ok(value)
    }
}
#[cfg(test)]
#[path = "temporal_tests.rs"]
mod tests;
