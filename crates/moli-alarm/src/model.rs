use std::collections::BTreeMap;

use crate::context::{
    ActionPolicy, AfterTrigger, Cause, Details, Notice, RestoreRequest, SavedState,
};
use crate::coverage::{Observation, SensorId, SensorPolicy};

/// Absolute milliseconds since the Unix epoch, supplied by the caller.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct TimestampMs(pub u64);

/// Created internally by the serial consumer, not chosen by a public client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestId(pub u64);

/// Internally scheduled timer identity; an old generation cannot authorize it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimerToken {
    pub generation: u64,
    pub deadline: TimestampMs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mode {
    Away,
    Home,
    Night,
    Vacation,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Disarmed,
    Arming(Mode),
    Armed(Mode),
    Pending(Mode),
    Triggered(Option<Mode>),
}

impl Phase {
    pub fn mode(self) -> Option<Mode> {
        match self {
            Self::Arming(mode) | Self::Armed(mode) | Self::Pending(mode) => Some(mode),
            Self::Triggered(mode) => mode,
            Self::Disarmed => None,
        }
    }
}

/// Live state of the pure engine. Only typed observations and internal timer
/// identities are retained; credentials and execution adapters stay outside.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub(crate) phase: Phase,
    pub(crate) generation: u64,
    pub(crate) exit_timer: Option<TimerToken>,
    pub(crate) details: Details,
}

impl State {
    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn exit_timer(&self) -> Option<TimerToken> {
        self.exit_timer
    }

    pub fn entry_timer(&self) -> Option<TimerToken> {
        self.details.entry_timer
    }
    pub fn trigger_timer(&self) -> Option<TimerToken> {
        self.details.trigger_timer
    }
    pub fn siren_timer(&self) -> Option<TimerToken> {
        self.details.siren_timer
    }
    pub fn siren_on(&self) -> bool {
        self.details.siren_on
    }
    pub fn cause(&self) -> Option<&Cause> {
        self.details.cause.as_ref()
    }
    /// Derived coverage, outside the transition table, for any enabled mode.
    pub fn coverage(&self, mode: Mode, config: &Config) -> crate::coverage::Coverage {
        let empty = crate::coverage::Exclusions::new(self.details.period, mode);
        crate::coverage::snapshot(
            self.details.period,
            mode,
            &config.sensors,
            &crate::engine::statuses(self),
            self.details.exclusions.as_ref().unwrap_or(&empty),
        )
    }
    pub fn saved(&self, at: TimestampMs) -> SavedState {
        crate::restore::save(self, at)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Ui,
    Household,
    Keypad(String),
    Automation,
    Agent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArmRequest {
    pub id: RequestId,
    pub mode: Mode,
    pub source: Source,
    pub skip_exit: bool,
    pub force: std::collections::BTreeSet<SensorId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisarmRequest {
    pub id: RequestId,
    pub source: Source,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Arm(ArmRequest),
    Disarm(DisarmRequest),
    ExitExpired(TimerToken),
    EntryExpired(TimerToken),
    TriggerExpired(TimerToken),
    SirenExpired(TimerToken),
    Open(SensorId),
    Close(SensorId),
    Observe {
        sensor: SensorId,
        observation: Observation,
    },
    Unavailable(SensorId),
    Available(SensorId),
    Tamper(SensorId),
    Panic(Source),
    SkipDelay(DisarmRequest),
    ArmOnClose {
        sensor: SensorId,
        token: TimerToken,
    },
    Restore(Box<RestoreRequest>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModeConfig {
    pub exit_s: u32,
    pub entry_s: u32,
    pub trigger_s: u32,
}

/// Configuration validated against descriptors before the caller starts the engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub modes: BTreeMap<Mode, ModeConfig>,
    pub siren_max_s: u32,
    pub sensors: Vec<SensorPolicy>,
    pub sirens: Vec<String>,
    pub keypads: Vec<String>,
    pub after_trigger: AfterTrigger,
    pub panic: ActionPolicy,
    pub tamper_armed: ActionPolicy,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            modes: BTreeMap::new(),
            siren_max_s: 180,
            sensors: Vec::new(),
            sirens: Vec::new(),
            keypads: Vec::new(),
            after_trigger: AfterTrigger::RearmExcluding,
            panic: ActionPolicy::Notify,
            tamper_armed: ActionPolicy::Trigger,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    NotifyArmed(Mode),
    JournalTransition { from: Phase, to: Phase },
    JournalRefusal(Verdict),
    ScheduleExit(TimerToken),
    CancelExit(TimerToken),
    ScheduleEntry(TimerToken),
    ScheduleTrigger(TimerToken),
    ScheduleSiren(TimerToken),
    ScheduleArmOnClose { sensor: SensorId, token: TimerToken },
    CancelArmOnClose(SensorId),
    Siren { point: String, on: bool },
    Notify(Notice),
    Blocking(Vec<SensorId>),
    RetainArm { request: RequestId, mode: Mode },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Armed(Mode),
    AcceptedArming(Mode),
    AlreadyArming(Mode),
    Busy,
    Disarmed,
    CodeRequired,
    InvalidCode,
    AlreadyArmed(Mode),
    AlreadyDisarmed,
    NotAllowed,
    OpenSensors,
    HumanApprovalRequired,
    SourceDenied,
    SkippedDelay,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transition {
    pub state: State,
    pub effects: Vec<Effect>,
    pub verdict: Option<Verdict>,
}
