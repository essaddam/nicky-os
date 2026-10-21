use super::{advance, blocking, deadline, unchanged};
use crate::coverage::{ArmPeriod, Exclusions, SensorState};
use crate::{
    AfterTrigger, Cause, Config, Effect, Mode, Notice, Phase, State, TimerToken, TimestampMs,
    Transition, Verdict,
};

pub(super) fn armed(mut state: State, mode: Mode) -> Transition {
    state.phase = Phase::Armed(mode);
    state.exit_timer = None;
    state.details.entry_timer = None;
    state.details.return_to = None;
    state.details.close_timers.clear();
    Transition {
        state,
        effects: vec![Effect::NotifyArmed(mode)],
        verdict: None,
    }
}
pub(crate) fn exit(mut state: State, _now: TimestampMs, config: &Config, cold: bool) -> Transition {
    let Phase::Arming(mode) = state.phase else {
        return unchanged(state);
    };
    let blocked = blocking(&state, mode, config, cold);
    if !advance(&mut state) {
        return unchanged(state);
    }
    if blocked.is_empty() {
        return armed(state, mode);
    }
    state.exit_timer = None;
    state.details.close_timers.clear();
    if let Some(frame) = state.details.return_to.take() {
        state.phase = frame.phase;
        state.details.period = frame.period;
        state.details.exclusions = frame.phase.mode().map(|mode| Exclusions {
            period: frame.period,
            mode,
            automatic: frame.automatic,
            forced: frame.forced,
        });
        state.details.unavailable_notified = frame.unavailable_notified;
    } else {
        state.phase = Phase::Disarmed;
        state.details.exclusions = None;
    }
    Transition {
        state,
        effects: vec![
            Effect::Blocking(blocked.clone()),
            Effect::Notify(Notice::ArmFailed(blocked)),
        ],
        verdict: None,
    }
}
pub(crate) fn trigger(
    mut state: State,
    cause: Cause,
    now: TimestampMs,
    config: &Config,
) -> Transition {
    let Some(siren_deadline) = deadline(now, config.siren_max_s.clamp(1, 180)) else {
        return unchanged(state);
    };
    let mode = state.phase.mode();
    let duration = mode
        .and_then(|m| config.modes.get(&m))
        .map_or(0, |m| m.trigger_s);
    let trigger_deadline = if duration == 0 {
        None
    } else {
        let Some(value) = deadline(now, duration) else {
            return unchanged(state);
        };
        Some(value)
    };
    if !advance(&mut state) {
        return unchanged(state);
    }
    state.phase = Phase::Triggered(mode);
    state.exit_timer = None;
    state.details.entry_timer = None;
    state.details.close_timers.clear();
    state.details.return_to = None;
    state.details.cause = Some(cause.clone());
    state.details.seen.clear();
    if let Cause::Sensor(id) | Cause::Tamper(id) = &cause {
        state.details.seen.insert(id.clone());
    }
    state.details.siren_on = true;
    state.details.siren_timer = Some(TimerToken {
        generation: state.generation,
        deadline: siren_deadline,
    });
    state.details.trigger_timer = trigger_deadline.map(|deadline| TimerToken {
        generation: state.generation,
        deadline,
    });
    let mut effects: Vec<_> = config
        .sirens
        .iter()
        .map(|point| Effect::Siren {
            point: point.clone(),
            on: true,
        })
        .collect();
    effects.push(Effect::Notify(Notice::Triggered(cause)));
    Transition {
        state,
        effects,
        verdict: None,
    }
}
pub(crate) fn stop_siren(mut state: State, config: &Config) -> Transition {
    // A due stop is mandatory even when no new generation can be allocated.
    advance(&mut state);
    state.details.siren_on = false;
    state.details.siren_timer = None;
    Transition {
        state,
        effects: config
            .sirens
            .iter()
            .map(|point| Effect::Siren {
                point: point.clone(),
                on: false,
            })
            .collect(),
        verdict: None,
    }
}
pub(crate) fn rearm(mut state: State, config: &Config) -> Transition {
    let Some(mode) = state.phase.mode() else {
        return unchanged(state);
    };
    // Evaluate the new period with its own automatic policy, never an old force.
    let next_period = ArmPeriod(state.details.period.0.saturating_add(1));
    let readings = super::statuses(&state);
    let exclusions = crate::coverage::begin_period(next_period, mode, &config.sensors, &readings);
    let blocked =
        crate::coverage::snapshot(next_period, mode, &config.sensors, &readings, &exclusions)
            .blocking;
    if !advance(&mut state) {
        return unchanged(state);
    }
    state.details.siren_on = false;
    state.details.siren_timer = None;
    state.details.trigger_timer = None;
    state.details.cause = None;
    state.details.seen.clear();
    if config.after_trigger == AfterTrigger::RearmOrDisarm && !blocked.is_empty() {
        state.phase = Phase::Disarmed;
        state.details.exclusions = None;
        state.details.unavailable_notified.clear();
    } else {
        state.phase = Phase::Armed(mode);
        // Each rearm starts a fresh period, with all still-open covered inputs excluded.
        state.details.period = ArmPeriod(state.details.period.0.saturating_add(1));
        let automatic = config
            .sensors
            .iter()
            .filter(|p| {
                p.enabled
                    && p.modes.contains(&mode)
                    && state
                        .details
                        .observations
                        .get(&p.id)
                        .is_some_and(|o| o.confirmed() && o.last() == SensorState::Open)
            })
            .map(|p| p.id.clone())
            .collect();
        state.details.exclusions = Some(Exclusions {
            period: state.details.period,
            mode,
            automatic,
            forced: std::collections::BTreeSet::new(),
        });
        state.details.unavailable_notified.clear();
    }
    let next = state.phase.mode();
    let mut effects: Vec<_> = config
        .sirens
        .iter()
        .map(|point| Effect::Siren {
            point: point.clone(),
            on: false,
        })
        .collect();
    effects.push(Effect::Notify(Notice::Ended(next)));
    Transition {
        state,
        effects,
        verdict: None,
    }
}
pub(super) fn refusal_with_blocking(
    state: State,
    points: Vec<crate::coverage::SensorId>,
) -> Transition {
    Transition {
        state,
        effects: vec![
            Effect::JournalRefusal(Verdict::OpenSensors),
            Effect::Blocking(points),
        ],
        verdict: Some(Verdict::OpenSensors),
    }
}
