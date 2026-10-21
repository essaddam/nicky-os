//! Plain snapshot data and bounded deadline reconciliation, without I/O.
use crate::coverage::{Exclusions, ObservationState, SensorState};
use crate::{
    Cause, Config, Effect, Notice, Phase, RestoreRequest, SavedState, State, TimerToken,
    TimestampMs, Transition,
};

pub fn save(state: &State, saved_at: TimestampMs) -> SavedState {
    SavedState {
        saved_at,
        phase: state.phase,
        generation: state.generation,
        exit_timer: state.exit_timer,
        entry_timer: state.details.entry_timer,
        trigger_timer: state.details.trigger_timer,
        siren_timer: state.details.siren_timer,
        siren_on: state.details.siren_on,
        return_to: state.details.return_to.clone(),
        cause: state.details.cause.clone(),
        seen: state.details.seen.clone(),
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
        close_timers: state.details.close_timers.clone(),
        unavailable_notified: state.details.unavailable_notified.clone(),
    }
}
fn clamp(timer: Option<TimerToken>, now: TimestampMs, maximum: u32) -> Option<TimerToken> {
    let upper = crate::engine::deadline(now, maximum)?;
    timer.map(|token| TimerToken {
        deadline: token.deadline.max(now).min(upper),
        ..token
    })
}
pub(crate) fn restore(
    previous: State,
    request: RestoreRequest,
    now: TimestampMs,
    config: &Config,
) -> Transition {
    let Some(saved) = request.saved else {
        let mut state = State::default();
        state.details.startup_at = now;
        if request.configured {
            state.generation = previous.generation.saturating_add(1);
            return Transition {
                state,
                effects: vec![Effect::Notify(Notice::StateLost)],
                verdict: None,
            };
        }
        return crate::engine::unchanged(state);
    };
    if saved
        .phase
        .mode()
        .is_some_and(|mode| !config.modes.contains_key(&mode))
    {
        return restore(
            previous,
            RestoreRequest {
                configured: true,
                saved: None,
                cached: request.cached,
            },
            now,
            config,
        );
    }
    let mut state = load(saved, previous.generation, &request.cached, now, config);
    // Stable-close observations must be confirmed again after a restart.
    if matches!(state.phase, Phase::Arming(_)) && state.exit_timer.is_none_or(|t| now >= t.deadline)
    {
        return crate::engine::transitions::exit(state, now, config, true);
    }
    if matches!(state.phase, Phase::Pending(_))
        && state.details.entry_timer.is_none_or(|t| now >= t.deadline)
    {
        return crate::engine::transitions::trigger(
            state,
            Cause::EntryExpiredDuringStop,
            now,
            config,
        );
    }
    if matches!(state.phase, Phase::Triggered(Some(_)))
        && state
            .details
            .trigger_timer
            .is_some_and(|t| now >= t.deadline)
    {
        return crate::engine::transitions::rearm(state, config);
    }
    if matches!(state.phase, Phase::Triggered(_))
        && state.details.siren_timer.is_none_or(|t| now >= t.deadline)
    {
        return crate::engine::transitions::stop_siren(state, config);
    }
    if !crate::engine::advance(&mut state) {
        return crate::engine::unchanged(previous);
    }
    let effects = if matches!(state.phase, Phase::Triggered(_)) && state.details.siren_on {
        config
            .sirens
            .iter()
            .map(|point| Effect::Siren {
                point: point.clone(),
                on: true,
            })
            .collect()
    } else {
        Vec::new()
    };
    crate::Transition {
        state,
        effects,
        verdict: None,
    }
}

fn load(
    saved: SavedState,
    previous_generation: u64,
    cached: &std::collections::BTreeMap<crate::coverage::SensorId, SensorState>,
    now: TimestampMs,
    config: &Config,
) -> State {
    let mut state = State {
        phase: saved.phase,
        generation: saved.generation.max(previous_generation),
        exit_timer: saved.exit_timer,
        ..State::default()
    };
    state.details.startup_at = now;
    state.details.period = saved.period;
    state.details.return_to = saved.return_to;
    state.details.cause = saved.cause;
    state.details.seen = saved.seen;
    state.details.unavailable_notified = saved.unavailable_notified;
    state.details.exclusions = state.phase.mode().map(|mode| Exclusions {
        period: saved.period,
        mode,
        automatic: saved.automatic,
        forced: saved.forced,
    });
    for policy in &config.sensors {
        state.details.observations.insert(
            policy.id.clone(),
            ObservationState {
                started_at: now,
                last: cached
                    .get(&policy.id)
                    .copied()
                    .unwrap_or(SensorState::Unknown),
                confirmed: false,
            },
        );
    }
    let timing = state.phase.mode().and_then(|mode| config.modes.get(&mode));
    state.exit_timer = if matches!(state.phase, Phase::Arming(_)) {
        clamp(state.exit_timer, now, timing.map_or(0, |m| m.exit_s))
    } else {
        None
    };
    state.details.entry_timer = if matches!(state.phase, Phase::Pending(_)) {
        clamp(saved.entry_timer, now, timing.map_or(0, |m| m.entry_s))
    } else {
        None
    };
    if matches!(state.phase, Phase::Triggered(_)) {
        state.details.siren_timer = clamp(saved.siren_timer, now, config.siren_max_s.clamp(1, 180));
        state.details.siren_on = saved.siren_on;
        if timing.is_some_and(|m| m.trigger_s > 0) {
            state.details.trigger_timer =
                clamp(saved.trigger_timer, now, timing.unwrap().trigger_s);
        }
    }
    state
}
