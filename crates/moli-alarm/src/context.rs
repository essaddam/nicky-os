//! Data needed by the remaining pure transition contracts, without I/O.
use std::collections::{BTreeMap, BTreeSet};

use crate::coverage::{ArmPeriod, Exclusions, ObservationState, SensorId, SensorState};
use crate::{Phase, TimerToken, TimestampMs};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AfterTrigger {
    #[default]
    RearmExcluding,
    RearmOrDisarm,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionPolicy {
    Notify,
    Trigger,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cause {
    Sensor(SensorId),
    Tamper(SensorId),
    Panic,
    EntryExpired,
    EntryExpiredDuringStop,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    Pending(crate::Mode),
    Triggered(Cause),
    Ended(Option<crate::Mode>),
    Disarmed,
    ArmFailed(Vec<SensorId>),
    Tamper(SensorId),
    Panic,
    Unavailable(SensorId),
    StateLost,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReturnFrame {
    pub phase: Phase,
    pub period: ArmPeriod,
    pub automatic: BTreeSet<SensorId>,
    pub forced: BTreeSet<SensorId>,
    pub unavailable_notified: BTreeSet<SensorId>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Details {
    pub startup_at: TimestampMs,
    pub return_to: Option<ReturnFrame>,
    pub entry_timer: Option<TimerToken>,
    pub trigger_timer: Option<TimerToken>,
    pub siren_timer: Option<TimerToken>,
    pub siren_on: bool,
    pub cause: Option<Cause>,
    pub seen: BTreeSet<SensorId>,
    pub period: ArmPeriod,
    pub exclusions: Option<Exclusions>,
    pub observations: BTreeMap<SensorId, ObservationState>,
    pub close_timers: BTreeMap<SensorId, TimerToken>,
    pub unavailable_notified: BTreeSet<SensorId>,
}

/// Plain snapshot data. File loading, serialization and fallback belong to lot 4.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SavedState {
    pub saved_at: TimestampMs,
    pub phase: Phase,
    pub generation: u64,
    pub exit_timer: Option<TimerToken>,
    pub entry_timer: Option<TimerToken>,
    pub trigger_timer: Option<TimerToken>,
    pub siren_timer: Option<TimerToken>,
    pub siren_on: bool,
    pub return_to: Option<ReturnFrame>,
    pub cause: Option<Cause>,
    pub seen: BTreeSet<SensorId>,
    pub period: ArmPeriod,
    pub automatic: BTreeSet<SensorId>,
    pub forced: BTreeSet<SensorId>,
    pub close_timers: BTreeMap<SensorId, TimerToken>,
    pub unavailable_notified: BTreeSet<SensorId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestoreRequest {
    pub configured: bool,
    pub saved: Option<SavedState>,
    pub cached: BTreeMap<SensorId, SensorState>,
}
