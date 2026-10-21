use super::{advance, blocking, deadline, refused, unchanged};
use crate::coverage::{self, ArmPeriod, ForceRequest, ForceVerdict};
use crate::{
    ArmRequest, Authorization, Config, DisarmRequest, Effect, Notice, Permission, Phase, Profiles,
    ReturnFrame, Source, State, TimerToken, TimestampMs, Transition, Verdict,
};

fn authorization(
    request: crate::RequestId,
    source: &Source,
    permission: Permission,
    mode: Option<crate::Mode>,
    now: TimestampMs,
    missing_allowed: bool,
    profiles: &impl Profiles,
) -> Result<(), Verdict> {
    match profiles.authorize(request, source, permission, mode, now) {
        Authorization::Allowed(_) => Ok(()),
        Authorization::Missing if missing_allowed => Ok(()),
        Authorization::Missing => Err(Verdict::CodeRequired),
        Authorization::Invalid(_) => Err(Verdict::InvalidCode),
    }
}
pub(super) fn arm(
    state: State,
    request: &ArmRequest,
    now: TimestampMs,
    config: &Config,
    profiles: &impl Profiles,
) -> Transition {
    if !config.modes.contains_key(&request.mode) {
        return refused(state, Verdict::NotAllowed);
    }
    match state.phase {
        Phase::Arming(current) => {
            return if current == request.mode {
                Transition {
                    state,
                    effects: Vec::new(),
                    verdict: Some(Verdict::AlreadyArming(current)),
                }
            } else {
                refused(state, Verdict::Busy)
            };
        }
        Phase::Pending(_) | Phase::Triggered(_) => return refused(state, Verdict::Busy),
        _ => {}
    }
    if request.source == Source::Agent {
        return if state.phase == Phase::Disarmed && !request.skip_exit && request.force.is_empty() {
            Transition {
                state,
                effects: vec![
                    Effect::JournalRefusal(Verdict::HumanApprovalRequired),
                    Effect::RetainArm {
                        request: request.id,
                        mode: request.mode,
                    },
                ],
                verdict: Some(Verdict::HumanApprovalRequired),
            }
        } else {
            refused(state, Verdict::SourceDenied)
        };
    }
    let changing = matches!(state.phase, Phase::Armed(old) if old != request.mode);
    if changing && request.source == Source::Automation {
        return refused(state, Verdict::SourceDenied);
    }
    let permission = if changing {
        Permission::ChangeMode
    } else {
        Permission::Arm
    };
    if let Err(verdict) = authorization(
        request.id,
        &request.source,
        permission,
        Some(request.mode),
        now,
        !changing,
        profiles,
    ) {
        return refused(state, verdict);
    }
    if matches!(state.phase, Phase::Armed(current) if current == request.mode) {
        return Transition {
            state,
            effects: Vec::new(),
            verdict: Some(Verdict::AlreadyArmed(request.mode)),
        };
    }
    if request.skip_exit {
        if request.source != Source::Ui {
            return refused(state, Verdict::SourceDenied);
        }
        if let Err(verdict) = authorization(
            request.id,
            &request.source,
            Permission::SkipDelay,
            Some(request.mode),
            now,
            false,
            profiles,
        ) {
            return refused(state, verdict);
        }
    }
    begin_arming(state, request, now, config, profiles)
}

fn begin_arming(
    state: State,
    request: &ArmRequest,
    now: TimestampMs,
    config: &Config,
    profiles: &impl Profiles,
) -> Transition {
    let Some(period) = state.details.period.0.checked_add(1).map(ArmPeriod) else {
        return unchanged(state);
    };
    let mut exclusions = coverage::begin_period(
        period,
        request.mode,
        &config.sensors,
        &super::statuses(&state),
    );
    if !request.force.is_empty() {
        let forced = coverage::force(
            exclusions,
            &ForceRequest {
                id: request.id,
                source: request.source.clone(),
                targets: request.force.clone(),
            },
            now,
            &config.sensors,
            profiles,
        );
        match forced.verdict {
            Some(ForceVerdict::Applied) => exclusions = forced.exclusions,
            Some(ForceVerdict::CodeRequired) => return refused(state, Verdict::CodeRequired),
            Some(ForceVerdict::InvalidCode) => return refused(state, Verdict::InvalidCode),
            _ => return refused(state, Verdict::SourceDenied),
        }
    }
    let exit = if request.skip_exit {
        0
    } else {
        config.modes[&request.mode].exit_s
    };
    let Some(until) = deadline(now, exit) else {
        return unchanged(state);
    };
    let mut candidate = state.clone();
    candidate.details.return_to = Some(ReturnFrame {
        phase: state.phase,
        period: state.details.period,
        automatic: state
            .details
            .exclusions
            .as_ref()
            .map_or_else(Default::default, |e| e.automatic().clone()),
        forced: state
            .details
            .exclusions
            .as_ref()
            .map_or_else(Default::default, |e| e.forced().clone()),
        unavailable_notified: state.details.unavailable_notified.clone(),
    });
    candidate.details.period = period;
    candidate.details.exclusions = Some(exclusions);
    candidate.details.unavailable_notified.clear();
    if exit == 0 {
        let blocked = blocking(&candidate, request.mode, config, false);
        if !blocked.is_empty() {
            return super::transitions::refusal_with_blocking(state, blocked);
        }
    }
    if !advance(&mut candidate) {
        return unchanged(state);
    }
    if exit == 0 {
        let mut result = super::transitions::armed(candidate, request.mode);
        result.verdict = Some(Verdict::Armed(request.mode));
        return result;
    }
    candidate.phase = Phase::Arming(request.mode);
    candidate.exit_timer = Some(TimerToken {
        generation: candidate.generation,
        deadline: until,
    });
    Transition {
        state: candidate,
        effects: Vec::new(),
        verdict: Some(Verdict::AcceptedArming(request.mode)),
    }
}

pub(super) fn disarm(
    mut state: State,
    request: &DisarmRequest,
    now: TimestampMs,
    profiles: &impl Profiles,
) -> Transition {
    if matches!(request.source, Source::Agent | Source::Automation) {
        return refused(state, Verdict::SourceDenied);
    }
    if let Err(verdict) = authorization(
        request.id,
        &request.source,
        Permission::Disarm,
        state.phase.mode(),
        now,
        false,
        profiles,
    ) {
        return refused(state, verdict);
    }
    if state.phase == Phase::Disarmed {
        return Transition {
            state,
            effects: Vec::new(),
            verdict: Some(Verdict::AlreadyDisarmed),
        };
    }
    let prior = state.phase;
    let exit = state.exit_timer;
    if !advance(&mut state) {
        return unchanged(state);
    }
    state.phase = Phase::Disarmed;
    state.exit_timer = None;
    state.details.entry_timer = None;
    state.details.trigger_timer = None;
    state.details.siren_timer = None;
    state.details.siren_on = false;
    state.details.cause = None;
    state.details.seen.clear();
    state.details.return_to = None;
    state.details.close_timers.clear();
    state.details.exclusions = None;
    state.details.unavailable_notified.clear();
    let mut effects = Vec::new();
    if let Some(timer) = exit {
        effects.push(Effect::CancelExit(timer));
    }
    if matches!(prior, Phase::Armed(_)) {
        effects.push(Effect::Notify(Notice::Disarmed));
    }
    Transition {
        state,
        effects,
        verdict: Some(Verdict::Disarmed),
    }
}
pub(super) fn skip(
    state: State,
    request: &DisarmRequest,
    now: TimestampMs,
    config: &Config,
    profiles: &impl Profiles,
) -> Transition {
    if !matches!(state.phase, Phase::Arming(_) | Phase::Pending(_)) {
        return unchanged(state);
    }
    if request.source != Source::Ui {
        return refused(state, Verdict::SourceDenied);
    }
    if let Err(verdict) = authorization(
        request.id,
        &request.source,
        Permission::SkipDelay,
        state.phase.mode(),
        now,
        false,
        profiles,
    ) {
        return refused(state, verdict);
    }
    let mut result = if matches!(state.phase, Phase::Arming(_)) {
        super::transitions::exit(state, now, config, false)
    } else {
        let cause = state
            .details
            .cause
            .clone()
            .unwrap_or(crate::Cause::EntryExpired);
        super::transitions::trigger(state, cause, now, config)
    };
    result.verdict = Some(Verdict::SkippedDelay);
    result
}
