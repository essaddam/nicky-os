//! One business-state owner for every helper counter in a hub.

use std::collections::BTreeMap;
use std::path::PathBuf;

use moli_core::{CounterOperation, CounterSpec, DeviceId, MAX_COUNTER_VALUE, PointId, RequestId};
use serde::{Deserialize, Serialize};

use crate::StateFile;

const MAX_COUNTERS: usize = 128;
const MAX_CLIENTS: usize = 64;
const WINDOW: u64 = 32;
const MAX_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CounterError {
    Value,
    ReadOnly,
    Definition,
    Bounds,
    Overflow,
    CorruptState,
    Storage,
    Uncertain,
    Conflict,
    Stale,
    Capacity,
    Abandoned,
}

impl std::fmt::Display for CounterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use moli_i18n::tr;
        f.write_str(&match self {
            Self::Value => tr!("serveur.compteur.valeur"),
            Self::ReadOnly => tr!("serveur.compteur.lecture_seule"),
            Self::Definition => tr!("serveur.compteur.definition"),
            Self::Bounds => tr!("serveur.compteur.bornes"),
            Self::Overflow => tr!("serveur.compteur.debordement"),
            Self::CorruptState => tr!("serveur.compteur.corrompu"),
            Self::Storage => tr!("serveur.compteur.persistance"),
            Self::Uncertain => tr!("serveur.compteur.incertain"),
            Self::Conflict => tr!("serveur.compteur.conflit"),
            Self::Stale => tr!("serveur.compteur.ancien"),
            Self::Capacity => tr!("serveur.compteur.capacite"),
            Self::Abandoned => tr!("serveur.compteur.abandonne"),
        })
    }
}

impl std::error::Error for CounterError {}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Applied,
    Bounds,
    Overflow,
}

impl Outcome {
    fn result(self) -> Result<(), CounterError> {
        match self {
            Self::Applied => Ok(()),
            Self::Bounds => Err(CounterError::Bounds),
            Self::Overflow => Err(CounterError::Overflow),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    point: PointId,
    outcome: Outcome,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Client {
    highest: u64,
    requests: BTreeMap<u64, Receipt>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    version: u32,
    values: BTreeMap<DeviceId, i64>,
    clients: BTreeMap<String, Client>,
}

pub(crate) fn valid_device(device: &DeviceId) -> bool {
    device.as_str().len() <= 240
        && device
            .as_str()
            .split_once(':')
            .is_some_and(|(instance, native)| {
                !instance.is_empty() && !native.is_empty() && !native.contains(':')
            })
        && !device.as_str().contains('/')
        && !device.as_str().chars().any(char::is_control)
}

impl State {
    fn empty() -> Self {
        Self {
            version: 1,
            values: BTreeMap::new(),
            clients: BTreeMap::new(),
        }
    }

    fn valid(&self) -> bool {
        self.version == 1
            && self.values.len() <= MAX_COUNTERS
            && self.clients.len() <= MAX_CLIENTS
            && self.values.iter().all(|(id, value)| {
                valid_device(id) && (-MAX_COUNTER_VALUE..=MAX_COUNTER_VALUE).contains(value)
            })
            && self.clients.iter().all(|(client, window)| {
                format!("{client}:1")
                    .parse::<RequestId>()
                    .is_ok_and(|id| id.client() == client)
                    && window.highest > 0
                    && window.requests.contains_key(&window.highest)
                    && window.requests.len() <= usize::try_from(WINDOW).unwrap_or(32)
                    && window.requests.iter().all(|(sequence, receipt)| {
                        *sequence > window.highest.saturating_sub(WINDOW)
                            && *sequence <= window.highest
                            && receipt.point.split().is_some_and(|(device, key)| {
                                self.values.contains_key(&device)
                                    && matches!(key, "increment" | "reset")
                            })
                    })
            })
    }
}

#[derive(Debug)]
pub(crate) struct Counters {
    pub(super) file: StateFile,
    state: Option<State>,
    definitions: BTreeMap<DeviceId, CounterSpec>,
    fenced: bool,
}

pub(crate) struct Commit {
    pub change: Option<i64>,
    pub result: Result<(), CounterError>,
}

impl Counters {
    pub(crate) fn new(path: Option<PathBuf>) -> Self {
        Self {
            file: StateFile::new(path, MAX_BYTES),
            state: None,
            definitions: BTreeMap::new(),
            fenced: false,
        }
    }

    async fn ready(&mut self) -> Result<(), CounterError> {
        if self.fenced {
            return Err(CounterError::Uncertain);
        }
        if self.state.is_none() {
            let state: State = self
                .file
                .load()
                .await
                .map_err(|error| {
                    tracing::warn!(%error, "counter business state could not be loaded");
                    CounterError::CorruptState
                })?
                .unwrap_or_else(State::empty);
            if !state.valid() {
                return Err(CounterError::CorruptState);
            }
            self.state = Some(state);
        }
        Ok(())
    }

    async fn persist(&mut self, state: State) -> Result<(), CounterError> {
        match self.file.write(&state).await {
            Ok(()) => {
                self.state = Some(state);
                Ok(())
            }
            Err(error) => {
                if error.commit_unknown {
                    self.fenced = true;
                }
                tracing::warn!(%error, commit_unknown = error.commit_unknown, "counter business state was not acknowledged");
                Err(if error.commit_unknown {
                    CounterError::Uncertain
                } else {
                    CounterError::Storage
                })
            }
        }
    }

    pub(crate) async fn initialize(
        &mut self,
        device: &DeviceId,
        spec: CounterSpec,
    ) -> Result<i64, CounterError> {
        if !valid_device(device) || !spec.is_valid() {
            return Err(CounterError::Definition);
        }
        self.ready().await?;
        let mut state = self.state.clone().ok_or(CounterError::CorruptState)?;
        let value = if let Some(value) = state.values.get(device) {
            if !spec.contains(*value) {
                return Err(CounterError::Definition);
            }
            *value
        } else {
            if state.values.len() >= MAX_COUNTERS {
                return Err(CounterError::Capacity);
            }
            state.values.insert(device.clone(), spec.initial);
            self.persist(state).await?;
            spec.initial
        };
        self.definitions.insert(device.clone(), spec);
        Ok(value)
    }

    pub(crate) async fn execute(
        &mut self,
        device: &DeviceId,
        spec: CounterSpec,
        operation: CounterOperation,
        request: Option<&RequestId>,
        abandoned: impl Fn() -> bool,
    ) -> Result<Commit, CounterError> {
        self.ready().await?;
        if abandoned() {
            return Err(CounterError::Abandoned);
        }
        if self.definitions.get(device) != Some(&spec) {
            return Err(CounterError::Definition);
        }
        let mut state = self.state.clone().ok_or(CounterError::CorruptState)?;
        let key = match operation {
            CounterOperation::Increment => "increment",
            CounterOperation::Reset => "reset",
        };
        let point = PointId::new(device, key);
        if let Some(request) = request {
            if let Some(client) = state.clients.get(request.client()) {
                if let Some(receipt) = client.requests.get(&request.sequence()) {
                    if receipt.point != point {
                        return Err(CounterError::Conflict);
                    }
                    return Ok(Commit {
                        change: None,
                        result: receipt.outcome.result(),
                    });
                }
                if request.sequence() <= client.highest.saturating_sub(WINDOW) {
                    return Err(CounterError::Stale);
                }
            } else if state.clients.len() >= MAX_CLIENTS {
                return Err(CounterError::Capacity);
            }
        }
        let current = *state.values.get(device).ok_or(CounterError::Definition)?;
        let next = match operation {
            CounterOperation::Increment => current.checked_add(spec.step),
            CounterOperation::Reset => Some(spec.initial),
        };
        let (change, outcome) = match next {
            Some(value) if spec.contains(value) => (Some(value), Outcome::Applied),
            Some(_) => (None, Outcome::Bounds),
            None => (None, Outcome::Overflow),
        };
        if let Some(value) = change {
            state.values.insert(device.clone(), value);
        }
        if let Some(request) = request {
            let client = state.clients.entry(request.client().into()).or_default();
            client.highest = client.highest.max(request.sequence());
            client
                .requests
                .retain(|sequence, _| *sequence > client.highest.saturating_sub(WINDOW));
            client
                .requests
                .insert(request.sequence(), Receipt { point, outcome });
        }
        // The commit follows to completion independently of the HTTP waiter or driver.
        if abandoned() {
            return Err(CounterError::Abandoned);
        }
        self.persist(state).await?;
        Ok(Commit {
            change,
            result: outcome.result(),
        })
    }
}

#[cfg(test)]
#[path = "counter_store_tests.rs"]
mod tests;
