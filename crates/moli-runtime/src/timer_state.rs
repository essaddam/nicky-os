//! Persistent timer records and request receipts; no live clock or task.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use moli_core::{
    DeviceId, MAX_OCCURRENCE_SEQUENCE, MAX_TIMER_SECONDS, OccurrenceId, RequestId, TimerOperation,
    TimerStatus,
};
use serde::{Deserialize, Serialize};

use super::TimerError;

pub(super) const MAX_TIMERS: usize = 128;
pub(super) const MAX_CLIENTS: usize = 64;
pub(super) const WINDOW: u64 = 32;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Record {
    pub source: Arc<str>,
    pub generation: u64,
    pub status: TimerStatus,
    pub duration_s: u32,
    pub deadline_ms: Option<u64>,
    pub pending: bool,
    pub correlation: Option<String>,
    pub audience: Vec<moli_core::TimerRecipient>,
}

impl Record {
    pub fn identity(&self) -> OccurrenceId {
        OccurrenceId {
            source: self.source.clone(),
            sequence: self.generation,
        }
    }

    fn valid(&self) -> bool {
        OccurrenceId {
            source: self.source.clone(),
            sequence: 1,
        }
        .is_valid()
            && self.generation <= MAX_OCCURRENCE_SEQUENCE
            && (1..=MAX_TIMER_SECONDS).contains(&self.duration_s)
            && self
                .deadline_ms
                .is_none_or(|at| at <= MAX_OCCURRENCE_SEQUENCE)
            && self
                .correlation
                .as_ref()
                .is_none_or(|value| value.len() <= 160 && !value.chars().any(char::is_control))
            && (!self.pending || self.status == TimerStatus::Finished)
            && self.audience.len() <= 1024
            && self
                .audience
                .iter()
                .all(moli_core::TimerRecipient::is_valid)
            && self
                .audience
                .iter()
                .map(|target| (&target.automation, &target.trigger))
                .collect::<BTreeSet<_>>()
                .len()
                == self.audience.len()
            && match self.status {
                TimerStatus::Idle => self.generation == 0 && self.deadline_ms.is_none(),
                TimerStatus::Cancelled => self.deadline_ms.is_none(),
                TimerStatus::Running | TimerStatus::Finished => {
                    self.generation > 0 && self.deadline_ms.is_some()
                }
            }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Outcome {
    Applied,
    Generation,
    Clock,
}

impl Outcome {
    pub fn result(self) -> Result<(), TimerError> {
        match self {
            Self::Applied => Ok(()),
            Self::Generation => Err(TimerError::Generation),
            Self::Clock => Err(TimerError::Clock),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Receipt {
    pub device: DeviceId,
    pub operation: TimerOperation,
    pub seconds: Option<u32>,
    pub outcome: Outcome,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Client {
    pub highest: u64,
    pub requests: BTreeMap<u64, Receipt>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct State {
    pub version: u32,
    pub values: BTreeMap<DeviceId, Record>,
    pub clients: BTreeMap<String, Client>,
}

impl State {
    pub fn empty() -> Self {
        Self {
            version: 1,
            values: BTreeMap::new(),
            clients: BTreeMap::new(),
        }
    }

    pub fn valid(&self) -> bool {
        self.version == 1
            && self.values.len() <= MAX_TIMERS
            && self.clients.len() <= MAX_CLIENTS
            && self
                .values
                .iter()
                .all(|(device, record)| crate::counters::valid_device(device) && record.valid())
            && self
                .values
                .values()
                .map(|record| &record.source)
                .collect::<BTreeSet<_>>()
                .len()
                == self.values.len()
            && self.clients.iter().all(|(name, client)| {
                format!("{name}:1")
                    .parse::<RequestId>()
                    .is_ok_and(|id| id.client() == name)
                    && client.highest > 0
                    && client.requests.contains_key(&client.highest)
                    && client.requests.len() <= 32
                    && client.requests.iter().all(|(sequence, receipt)| {
                        *sequence > client.highest.saturating_sub(WINDOW)
                            && *sequence <= client.highest
                            && self.values.contains_key(&receipt.device)
                            && match receipt.operation {
                                TimerOperation::Start => receipt.seconds.is_some_and(|seconds| {
                                    (1..=MAX_TIMER_SECONDS).contains(&seconds)
                                }),
                                TimerOperation::Cancel => receipt.seconds.is_none(),
                            }
                    })
            })
    }
}
