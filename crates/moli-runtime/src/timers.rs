//! One durable timer owner; live deadlines never depend on wall-clock updates.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use moli_core::{
    DeviceId, MAX_OCCURRENCE_SEQUENCE, MAX_TIMER_SECONDS, Occurrence, PointId, RequestId,
    TimerOperation, TimerStatus, Value,
};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::StateFile;

#[path = "timer_state.rs"]
mod state;
use state::{MAX_CLIENTS, MAX_TIMERS, Outcome, Receipt, Record, State, WINDOW};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimerError {
    Value,
    Definition,
    Generation,
    Clock,
    Storage,
    Uncertain,
    CorruptState,
    Conflict,
    Stale,
    Capacity,
    Abandoned,
}

impl std::fmt::Display for TimerError {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use moli_i18n::tr;
        out.write_str(&match self {
            Self::Value => tr!("serveur.minuteur.valeur"),
            Self::Definition => tr!("serveur.minuteur.definition"),
            Self::Generation => tr!("serveur.minuteur.generation"),
            Self::Clock => tr!("serveur.minuteur.horloge"),
            Self::Storage => tr!("serveur.minuteur.persistance"),
            Self::Uncertain => tr!("serveur.minuteur.incertain"),
            Self::CorruptState => tr!("serveur.minuteur.corrompu"),
            Self::Conflict => tr!("serveur.minuteur.conflit"),
            Self::Stale => tr!("serveur.minuteur.ancien"),
            Self::Capacity => tr!("serveur.minuteur.capacite"),
            Self::Abandoned => tr!("serveur.minuteur.abandonne"),
        })
    }
}
impl std::error::Error for TimerError {}

pub(crate) struct Input {
    pub operation: TimerOperation,
    pub seconds: Option<u32>,
    pub request: Option<RequestId>,
    pub correlation: Option<String>,
    pub audience: Vec<moli_core::TimerRecipient>,
}

pub(crate) struct Change {
    pub changed: bool,
    pub result: Result<(), TimerError>,
}

#[derive(Clone, Debug)]
pub(crate) struct Projection {
    pub device: DeviceId,
    pub status: TimerStatus,
    pub generation: u64,
    pub remaining: Option<u64>,
    pub deadline: Option<u64>,
    pub correlation: Option<String>,
}

#[derive(Debug)]
struct Live {
    deadline: Option<Instant>,
    token: CancellationToken,
    event: Option<Occurrence>,
    clock_invalid: bool,
}

impl Live {
    fn restore(record: &Record, wall: u64) -> Self {
        let remaining = record.deadline_ms.unwrap_or(wall).saturating_sub(wall);
        let clock_invalid = record.status == TimerStatus::Running
            && remaining > u64::from(record.duration_s) * 1000;
        Self {
            deadline: (record.status == TimerStatus::Running && !clock_invalid)
                .then(|| Instant::now() + Duration::from_millis(remaining)),
            token: CancellationToken::new(),
            event: None,
            clock_invalid,
        }
    }
}

#[derive(Debug)]
pub(crate) struct Timers {
    pub(super) file: StateFile,
    state: Option<State>,
    live: BTreeMap<DeviceId, Live>,
    fenced: bool,
}

impl Timers {
    pub(crate) fn new(path: Option<PathBuf>) -> Self {
        Self {
            file: StateFile::new(path, 1024 * 1024),
            state: None,
            live: BTreeMap::new(),
            fenced: false,
        }
    }

    async fn ready(&mut self) -> Result<(), TimerError> {
        if self.fenced {
            return Err(TimerError::Uncertain);
        }
        if self.state.is_none() {
            let state = self
                .file
                .load::<State>()
                .await
                .map_err(|_| TimerError::CorruptState)?
                .unwrap_or_else(State::empty);
            if !state.valid() {
                return Err(TimerError::CorruptState);
            }
            self.state = Some(state);
        }
        Ok(())
    }

    async fn persist(&mut self, state: State) -> Result<(), TimerError> {
        match self.file.write(&state).await {
            Ok(()) => {
                self.state = Some(state);
                Ok(())
            }
            Err(error) => {
                if error.commit_unknown {
                    self.fenced = true;
                    for live in self.live.values() {
                        live.token.cancel();
                    }
                }
                tracing::warn!(%error, "timer state was not acknowledged");
                Err(if error.commit_unknown {
                    TimerError::Uncertain
                } else {
                    TimerError::Storage
                })
            }
        }
    }

    pub(crate) async fn initialize(
        &mut self,
        device: &DeviceId,
        default_seconds: u32,
        wall: u64,
    ) -> Result<(), TimerError> {
        if !crate::counters::valid_device(device)
            || !(1..=MAX_TIMER_SECONDS).contains(&default_seconds)
        {
            return Err(TimerError::Definition);
        }
        self.ready().await?;
        let mut state = self.state.clone().ok_or(TimerError::CorruptState)?;
        if !state.values.contains_key(device) {
            if state.values.len() >= MAX_TIMERS {
                return Err(TimerError::Capacity);
            }
            let record = Record {
                source: crate::occurrences::random_id().map_err(|_| TimerError::Generation)?,
                generation: 0,
                status: TimerStatus::Idle,
                duration_s: default_seconds,
                deadline_ms: None,
                pending: false,
                correlation: None,
                audience: Vec::new(),
            };
            state.values.insert(device.clone(), record);
            self.persist(state).await?;
        }
        let record = &self.state.as_ref().ok_or(TimerError::CorruptState)?.values[device];
        self.live
            .entry(device.clone())
            .or_insert_with(|| Live::restore(record, wall));
        Ok(())
    }

    fn retry(&self, device: &DeviceId, input: &Input) -> Result<Option<Change>, TimerError> {
        let Some(request) = &input.request else {
            return Ok(None);
        };
        let state = self.state.as_ref().ok_or(TimerError::CorruptState)?;
        if let Some(client) = state.clients.get(request.client()) {
            if let Some(receipt) = client.requests.get(&request.sequence()) {
                if &receipt.device != device
                    || receipt.operation != input.operation
                    || receipt.seconds != input.seconds
                {
                    return Err(TimerError::Conflict);
                }
                return Ok(Some(Change {
                    changed: false,
                    result: receipt.outcome.result(),
                }));
            }
            if request.sequence() <= client.highest.saturating_sub(WINDOW) {
                return Err(TimerError::Stale);
            }
        } else if state.clients.len() >= MAX_CLIENTS {
            return Err(TimerError::Capacity);
        }
        Ok(None)
    }

    pub(crate) async fn command(
        &mut self,
        device: &DeviceId,
        input: Input,
        wall: u64,
        abandoned: impl Fn() -> bool,
    ) -> Result<Change, TimerError> {
        self.ready().await?;
        if abandoned() {
            return Err(TimerError::Abandoned);
        }
        if !self.live.contains_key(device) {
            return Err(TimerError::Definition);
        }
        if input.audience.len() > 1024
            || !input
                .audience
                .iter()
                .all(moli_core::TimerRecipient::is_valid)
        {
            return Err(TimerError::Capacity);
        }
        match input.operation {
            TimerOperation::Start
                if input
                    .seconds
                    .is_some_and(|seconds| (1..=MAX_TIMER_SECONDS).contains(&seconds)) => {}
            TimerOperation::Cancel if input.seconds.is_none() => {}
            _ => return Err(TimerError::Value),
        }
        if let Some(retry) = self.retry(device, &input)? {
            return Ok(retry);
        }
        let mut state = self.state.clone().ok_or(TimerError::CorruptState)?;
        let record = state.values.get_mut(device).ok_or(TimerError::Definition)?;
        let outcome = match input.operation {
            TimerOperation::Start => start(record, input.seconds.ok_or(TimerError::Value)?, wall),
            TimerOperation::Cancel => {
                record.status = TimerStatus::Cancelled;
                record.deadline_ms = None;
                Outcome::Applied
            }
        };
        if outcome.result().is_ok() {
            record.pending = false;
            record.correlation.clone_from(&input.correlation);
            record.audience = if input.operation == TimerOperation::Start {
                input.audience
            } else {
                Vec::new()
            };
        }
        // Anchor the live deadline before I/O; a slow commit must not extend it.
        let next_live = outcome
            .result()
            .is_ok()
            .then(|| Live::restore(record, wall));
        if let Some(request) = input.request {
            let client = state.clients.entry(request.client().into()).or_default();
            client.highest = client.highest.max(request.sequence());
            client
                .requests
                .retain(|seq, _| *seq > client.highest.saturating_sub(WINDOW));
            client.requests.insert(
                request.sequence(),
                Receipt {
                    device: device.clone(),
                    operation: input.operation,
                    seconds: input.seconds,
                    outcome,
                },
            );
        }
        if abandoned() {
            return Err(TimerError::Abandoned);
        }
        self.persist(state).await?;
        let changed = outcome.result().is_ok();
        if changed {
            if let Some(old) = self.live.remove(device) {
                old.token.cancel();
            }
            self.live
                .insert(device.clone(), next_live.ok_or(TimerError::Definition)?);
        }
        Ok(Change {
            changed,
            result: outcome.result(),
        })
    }

    pub(crate) fn projection(&self, device: &DeviceId) -> Option<Projection> {
        let record = self.state.as_ref()?.values.get(device)?;
        let live = self.live.get(device)?;
        let remaining = if live.clock_invalid {
            None
        } else {
            Some(live.deadline.map_or(0, |at| {
                let remaining = at.saturating_duration_since(Instant::now());
                remaining
                    .as_secs()
                    .saturating_add(u64::from(remaining.subsec_nanos() > 0))
            }))
        };
        Some(Projection {
            device: device.clone(),
            status: record.status,
            generation: record.generation,
            remaining,
            deadline: record.deadline_ms,
            correlation: record.correlation.clone(),
        })
    }

    pub(crate) fn projections(&self) -> Vec<Projection> {
        self.live
            .keys()
            .filter_map(|id| self.projection(id))
            .collect()
    }

    pub(crate) async fn expire(&mut self) -> Result<Vec<DeviceId>, TimerError> {
        if self.live.is_empty() {
            return Ok(Vec::new());
        }
        self.ready().await?;
        let due: Vec<_> = self
            .live
            .iter()
            .filter(|(_, live)| live.deadline.is_some_and(|at| at <= Instant::now()))
            .map(|(id, _)| id.clone())
            .collect();
        if due.is_empty() {
            return Ok(due);
        }
        let mut state = self.state.clone().ok_or(TimerError::CorruptState)?;
        for id in &due {
            let record = state.values.get_mut(id).ok_or(TimerError::Definition)?;
            record.status = TimerStatus::Finished;
            record.pending = true;
        }
        self.persist(state).await?;
        for id in &due {
            if let Some(live) = self.live.get_mut(id) {
                live.deadline = None;
            }
        }
        Ok(due)
    }

    pub(crate) fn events(
        &mut self,
        stream: &crate::occurrences::Stream,
    ) -> Result<Vec<Occurrence>, TimerError> {
        if self.fenced {
            return Err(TimerError::Uncertain);
        }
        let mut events = Vec::new();
        let Some(state) = &self.state else {
            return Ok(events);
        };
        for (id, record) in state.values.iter().filter(|(_, record)| record.pending) {
            let Some(live) = self.live.get_mut(id) else {
                continue;
            };
            if live.event.is_none() {
                live.event = Some(
                    stream
                        .event(
                            PointId::new(id, "finished"),
                            Value::Null,
                            record.deadline_ms.unwrap_or(0),
                            Some(record.identity()),
                            record.correlation.clone(),
                            true,
                        )
                        .map_err(|_| TimerError::Generation)?,
                );
            }
            events.extend(live.event.clone());
        }
        Ok(events)
    }

    pub(crate) fn delivery(&self, event: &Occurrence) -> Result<CancellationToken, TimerError> {
        if self.fenced {
            return Err(TimerError::Uncertain);
        }
        let device = event
            .point
            .split()
            .filter(|(_, key)| *key == "finished")
            .map(|(id, _)| id)
            .ok_or(TimerError::Definition)?;
        let record = self
            .state
            .as_ref()
            .and_then(|state| state.values.get(&device))
            .ok_or(TimerError::Stale)?;
        if record.status != TimerStatus::Finished || record.identity() != event.id {
            return Err(TimerError::Stale);
        }
        self.live
            .get(&device)
            .map(|live| live.token.clone())
            .ok_or(TimerError::Stale)
    }

    pub(crate) async fn ack(&mut self, event: &Occurrence) -> Result<bool, TimerError> {
        self.ready().await?;
        if self.delivery(event).is_err() {
            return Ok(false);
        }
        let device = event
            .point
            .split()
            .map(|(id, _)| id)
            .ok_or(TimerError::Definition)?;
        let mut state = self.state.clone().ok_or(TimerError::CorruptState)?;
        let record = state
            .values
            .get_mut(&device)
            .ok_or(TimerError::Definition)?;
        if !record.pending {
            return Ok(false);
        }
        record.pending = false;
        self.persist(state).await?;
        Ok(true)
    }

    pub(crate) fn audience(
        &self,
        event: &Occurrence,
    ) -> Result<Vec<moli_core::TimerRecipient>, TimerError> {
        self.delivery(event)?;
        let device = event
            .point
            .split()
            .map(|(id, _)| id)
            .ok_or(TimerError::Definition)?;
        self.state
            .as_ref()
            .and_then(|state| state.values.get(&device))
            .map(|record| record.audience.clone())
            .ok_or(TimerError::Stale)
    }

    pub(crate) fn wait(&self, consumer_ready: bool) -> Option<Duration> {
        if self.fenced {
            return None;
        }
        self.live
            .values()
            .filter_map(|live| live.deadline)
            .map(|at| {
                at.saturating_duration_since(Instant::now())
                    .min(Duration::from_secs(1))
            })
            .min()
            .or_else(|| {
                (consumer_ready
                    && self.state.as_ref().is_some_and(|state| {
                        self.live
                            .keys()
                            .any(|id| state.values.get(id).is_some_and(|record| record.pending))
                    }))
                .then_some(Duration::from_secs(1))
            })
    }
}

fn start(record: &mut Record, seconds: u32, wall: u64) -> Outcome {
    let Some(generation) = record
        .generation
        .checked_add(1)
        .filter(|generation| *generation <= MAX_OCCURRENCE_SEQUENCE)
    else {
        return Outcome::Generation;
    };
    let Some(deadline) = wall
        .checked_add(u64::from(seconds) * 1000)
        .filter(|at| *at <= MAX_OCCURRENCE_SEQUENCE)
    else {
        return Outcome::Clock;
    };
    record.generation = generation;
    record.duration_s = seconds;
    record.deadline_ms = Some(deadline);
    record.status = TimerStatus::Running;
    Outcome::Applied
}

#[cfg(test)]
#[path = "timer_tests.rs"]
mod tests;
