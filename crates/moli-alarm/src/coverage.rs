//! Pure projections of sensor coverage, separate from the alarm transition engine.
//! The adapter supplies point semantics, observation origin, time and policy.

use std::collections::{BTreeMap, BTreeSet};

use crate::{Authorization, Mode, Permission, Profiles, RequestId, Source, TimestampMs};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointSemantic {
    Contact,
    Occupancy,
    Lock,
    Tamper,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleValue<'a> {
    Bool(bool),
    Null,
    Text(&'a str),
    Other,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SensorState {
    #[default]
    Unknown,
    Unavailable,
    Closed,
    Open,
    Tamper,
}

/// Interpret typed values by semantics; unsupported values remain unknown.
pub fn interpret(semantic: PointSemantic, readable: bool, value: SampleValue<'_>) -> SensorState {
    match (value, semantic) {
        (SampleValue::Unavailable, _) => SensorState::Unavailable,
        (SampleValue::Bool(value), PointSemantic::Contact) => {
            if value {
                SensorState::Closed
            } else {
                SensorState::Open
            }
        }
        (SampleValue::Bool(value), PointSemantic::Occupancy) => {
            if value {
                SensorState::Open
            } else {
                SensorState::Closed
            }
        }
        (SampleValue::Bool(value), PointSemantic::Lock) if readable => {
            if value {
                SensorState::Closed
            } else {
                SensorState::Open
            }
        }
        (SampleValue::Bool(value), PointSemantic::Tamper) => {
            if value {
                SensorState::Tamper
            } else {
                SensorState::Closed
            }
        }
        _ => SensorState::Unknown,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservationOrigin {
    Cached,
    Fresh,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Observation {
    pub state: SensorState,
    pub origin: ObservationOrigin,
    pub observed_at: TimestampMs,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservationState {
    pub(crate) started_at: TimestampMs,
    pub(crate) last: SensorState,
    pub(crate) confirmed: bool,
}

impl ObservationState {
    pub fn new(started_at: TimestampMs) -> Self {
        Self {
            started_at,
            last: SensorState::Unknown,
            confirmed: false,
        }
    }

    pub fn last(&self) -> SensorState {
        self.last
    }

    pub fn confirmed(&self) -> bool {
        self.confirmed
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoverageEvent {
    Open,
    Tamper,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observed {
    pub state: ObservationState,
    pub event: Option<CoverageEvent>,
}

/// Record post-start observations, keeping freshness separate from value.
pub fn observe(mut state: ObservationState, observation: Observation, watching: bool) -> Observed {
    if observation.origin == ObservationOrigin::Cached
        || observation.observed_at <= state.started_at
    {
        return Observed { state, event: None };
    }
    let edge = !state.confirmed || state.last != observation.state;
    state.last = observation.state;
    state.confirmed = !matches!(
        observation.state,
        SensorState::Unknown | SensorState::Unavailable
    );
    let event = match observation.state {
        SensorState::Open if watching && edge => Some(CoverageEvent::Open),
        SensorState::Tamper if edge => Some(CoverageEvent::Tamper),
        _ => None,
    };
    Observed { state, event }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SensorId(pub String);

/// An arm period survives phase changes; it is not a timer generation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ArmPeriod(pub u64);

#[derive(Clone, Debug, PartialEq, Eq)]
// Independent descriptor/configuration switches, not alternative machine states.
#[allow(clippy::struct_excessive_bools)]
pub struct SensorPolicy {
    pub id: SensorId,
    pub enabled: bool,
    pub modes: BTreeSet<Mode>,
    pub allow_open: bool,
    pub auto_bypass: BTreeSet<Mode>,
    pub exit: bool,
    pub entry: bool,
    pub entry_s: Option<u32>,
    pub arm_on_close: bool,
    pub trigger_unavailable: bool,
    pub always_on: bool,
}

impl Default for SensorPolicy {
    fn default() -> Self {
        Self {
            id: SensorId(String::new()),
            enabled: true,
            modes: BTreeSet::new(),
            allow_open: false,
            auto_bypass: BTreeSet::new(),
            exit: true,
            entry: true,
            entry_s: None,
            arm_on_close: false,
            trigger_unavailable: false,
            always_on: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SensorStatus {
    pub state: SensorState,
    pub confirmed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exclusions {
    pub(crate) period: ArmPeriod,
    pub(crate) mode: Mode,
    pub(crate) automatic: BTreeSet<SensorId>,
    pub(crate) forced: BTreeSet<SensorId>,
}

impl Exclusions {
    pub fn new(period: ArmPeriod, mode: Mode) -> Self {
        Self {
            period,
            mode,
            automatic: BTreeSet::new(),
            forced: BTreeSet::new(),
        }
    }

    pub fn automatic(&self) -> &BTreeSet<SensorId> {
        &self.automatic
    }

    pub fn forced(&self) -> &BTreeSet<SensorId> {
        &self.forced
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Coverage {
    pub covered: BTreeSet<SensorId>,
    pub monitored: BTreeSet<SensorId>,
    pub blocking: BTreeSet<SensorId>,
    pub degraded: BTreeSet<SensorId>,
    pub bypassed: BTreeSet<SensorId>,
}

pub fn begin_period(
    period: ArmPeriod,
    mode: Mode,
    policies: &[SensorPolicy],
    states: &BTreeMap<SensorId, SensorStatus>,
) -> Exclusions {
    let automatic = policies
        .iter()
        .filter(|policy| {
            covers(policy, mode)
                && policy.auto_bypass.contains(&mode)
                && states
                    .get(&policy.id)
                    .is_some_and(|value| value.confirmed && value.state == SensorState::Open)
        })
        .map(|policy| policy.id.clone())
        .collect();
    Exclusions {
        period,
        mode,
        automatic,
        forced: BTreeSet::new(),
    }
}

pub fn snapshot(
    period: ArmPeriod,
    mode: Mode,
    policies: &[SensorPolicy],
    states: &BTreeMap<SensorId, SensorStatus>,
    exclusions: &Exclusions,
) -> Coverage {
    let mut result = Coverage::default();
    let current = exclusions.period == period && exclusions.mode == mode;
    for policy in policies.iter().filter(|policy| covers(policy, mode)) {
        result.covered.insert(policy.id.clone());
        if current
            && (exclusions.automatic.contains(&policy.id) || exclusions.forced.contains(&policy.id))
        {
            result.bypassed.insert(policy.id.clone());
            continue;
        }
        result.monitored.insert(policy.id.clone());
        let value = states.get(&policy.id).copied().unwrap_or(SensorStatus {
            state: SensorState::Unknown,
            confirmed: false,
        });
        if !value.confirmed
            || matches!(value.state, SensorState::Unknown | SensorState::Unavailable)
        {
            result.degraded.insert(policy.id.clone());
        }
        if value.confirmed && value.state == SensorState::Open && !policy.allow_open {
            result.blocking.insert(policy.id.clone());
        }
    }
    result
}

pub fn end_period(mut exclusions: Exclusions) -> Exclusions {
    exclusions.automatic.clear();
    exclusions.forced.clear();
    exclusions
}

fn covers(policy: &SensorPolicy, mode: Mode) -> bool {
    policy.enabled && policy.modes.contains(&mode)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForceRequest {
    pub id: RequestId,
    pub source: Source,
    pub targets: BTreeSet<SensorId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForceVerdict {
    Applied,
    CodeRequired,
    InvalidCode,
    SourceDenied,
    TargetsRequired,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Forced {
    pub exclusions: Exclusions,
    pub verdict: Option<ForceVerdict>,
}

/// Only an authenticated UI request can add explicitly named covered sensors.
pub fn force(
    mut exclusions: Exclusions,
    request: &ForceRequest,
    now: TimestampMs,
    policies: &[SensorPolicy],
    profiles: &impl Profiles,
) -> Forced {
    if request.source != Source::Ui {
        return force_refusal(exclusions, ForceVerdict::SourceDenied);
    }
    let eligible: BTreeSet<_> = policies
        .iter()
        .filter(|policy| covers(policy, exclusions.mode))
        .map(|policy| policy.id.clone())
        .collect();
    if request.targets.is_empty() || !request.targets.is_subset(&eligible) {
        return force_refusal(exclusions, ForceVerdict::TargetsRequired);
    }
    match profiles.authorize(
        request.id,
        &request.source,
        Permission::Exclude,
        Some(exclusions.mode),
        now,
    ) {
        Authorization::Missing => return force_refusal(exclusions, ForceVerdict::CodeRequired),
        Authorization::Invalid(_) => return force_refusal(exclusions, ForceVerdict::InvalidCode),
        Authorization::Allowed(_) => {}
    }
    exclusions.forced.extend(request.targets.iter().cloned());
    Forced {
        exclusions,
        verdict: Some(ForceVerdict::Applied),
    }
}

fn force_refusal(exclusions: Exclusions, verdict: ForceVerdict) -> Forced {
    Forced {
        exclusions,
        verdict: Some(verdict),
    }
}
