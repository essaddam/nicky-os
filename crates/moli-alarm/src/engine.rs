//! Deterministic decisions; the caller owns authorization, time and effects.
mod commands;
mod sensors;
pub(crate) mod transitions;

use crate::coverage::{self, SensorId, SensorState, SensorStatus};
use crate::{
    Config, Effect, Event, Phase, Profiles, State, TimerToken, TimestampMs, Transition, Verdict,
};

pub fn step(
    state: State,
    event: Event,
    now: TimestampMs,
    config: &Config,
    profiles: &impl Profiles,
) -> Transition {
    let before = state.clone();
    let result = match event {
        Event::Arm(request) => commands::arm(state, &request, now, config, profiles),
        Event::Disarm(request) => commands::disarm(state, &request, now, profiles),
        Event::SkipDelay(request) => commands::skip(state, &request, now, config, profiles),
        Event::ExitExpired(token)
            if matches!(state.phase, Phase::Arming(_))
                && current(&state, state.exit_timer, token, now) =>
        {
            transitions::exit(state, now, config, false)
        }
        Event::EntryExpired(token)
            if matches!(state.phase, Phase::Pending(_))
                && current(&state, state.details.entry_timer, token, now) =>
        {
            let cause = state
                .details
                .cause
                .clone()
                .unwrap_or(crate::Cause::EntryExpired);
            transitions::trigger(state, cause, now, config)
        }
        Event::SirenExpired(token)
            if matches!(state.phase, Phase::Triggered(_))
                && current(&state, state.details.siren_timer, token, now) =>
        {
            transitions::stop_siren(state, config)
        }
        Event::TriggerExpired(token)
            if matches!(state.phase, Phase::Triggered(Some(_)))
                && current(&state, state.details.trigger_timer, token, now)
                && state
                    .phase
                    .mode()
                    .and_then(|m| config.modes.get(&m))
                    .is_some_and(|m| m.trigger_s > 0) =>
        {
            transitions::rearm(state, config)
        }
        Event::Restore(request) => crate::restore::restore(state, *request, now, config),
        event => sensors::event(state, event, now, config),
    };
    finalize(&before, result, config)
}
pub(super) fn current(
    state: &State,
    active: Option<TimerToken>,
    token: TimerToken,
    now: TimestampMs,
) -> bool {
    active == Some(token) && token.generation == state.generation && now >= token.deadline
}
pub(super) fn deadline(now: TimestampMs, seconds: u32) -> Option<TimestampMs> {
    now.0
        .checked_add(u64::from(seconds) * 1000)
        .map(TimestampMs)
}
pub(super) fn advance(state: &mut State) -> bool {
    let Some(next) = state.generation.checked_add(1) else {
        return false;
    };
    state.generation = next;
    for token in [
        &mut state.exit_timer,
        &mut state.details.entry_timer,
        &mut state.details.trigger_timer,
        &mut state.details.siren_timer,
    ]
    .into_iter()
    .flatten()
    {
        token.generation = next;
    }
    for token in state.details.close_timers.values_mut() {
        token.generation = next;
    }
    true
}
pub(super) fn unchanged(state: State) -> Transition {
    Transition {
        state,
        effects: Vec::new(),
        verdict: None,
    }
}
pub(super) fn refused(state: State, verdict: Verdict) -> Transition {
    Transition {
        state,
        effects: vec![Effect::JournalRefusal(verdict)],
        verdict: Some(verdict),
    }
}
pub(super) fn statuses(state: &State) -> std::collections::BTreeMap<SensorId, SensorStatus> {
    state
        .details
        .observations
        .iter()
        .map(|(id, value)| {
            (
                id.clone(),
                SensorStatus {
                    state: value.last(),
                    confirmed: value.confirmed(),
                },
            )
        })
        .collect()
}
pub(super) fn blocking(
    state: &State,
    mode: crate::Mode,
    config: &Config,
    cold: bool,
) -> Vec<SensorId> {
    let empty = coverage::Exclusions::new(state.details.period, mode);
    let coverage = coverage::snapshot(
        state.details.period,
        mode,
        &config.sensors,
        &statuses(state),
        state.details.exclusions.as_ref().unwrap_or(&empty),
    );
    coverage
        .blocking
        .into_iter()
        .chain(if cold {
            coverage.degraded
        } else {
            std::collections::BTreeSet::new()
        })
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}
pub(super) fn set_reading(state: &mut State, id: &SensorId, value: SensorState, confirmed: bool) {
    state.details.observations.insert(
        id.clone(),
        coverage::ObservationState {
            started_at: state.details.startup_at,
            last: value,
            confirmed,
        },
    );
}
fn finalize(before: &State, mut result: Transition, config: &Config) -> Transition {
    if matches!(before.phase, Phase::Triggered(_))
        && !matches!(result.state.phase, Phase::Triggered(_))
    {
        result.state.details.siren_on = false;
        result.state.details.siren_timer = None;
        result.state.details.trigger_timer = None;
        if !result
            .effects
            .iter()
            .any(|e| matches!(e, Effect::Siren { on: false, .. }))
        {
            result
                .effects
                .extend(config.sirens.iter().map(|point| Effect::Siren {
                    point: point.clone(),
                    on: false,
                }));
        }
        if !result
            .effects
            .iter()
            .any(|e| matches!(e, Effect::Notify(crate::Notice::Ended(_))))
        {
            result.effects.push(Effect::Notify(crate::Notice::Ended(
                result.state.phase.mode(),
            )));
        }
    }
    if result.state.generation != before.generation {
        result.effects.insert(
            0,
            Effect::JournalTransition {
                from: before.phase,
                to: result.state.phase,
            },
        );
    }
    for (old, new, effect) in [
        (
            before.exit_timer,
            result.state.exit_timer,
            Effect::ScheduleExit as fn(TimerToken) -> Effect,
        ),
        (
            before.details.entry_timer,
            result.state.details.entry_timer,
            Effect::ScheduleEntry,
        ),
        (
            before.details.trigger_timer,
            result.state.details.trigger_timer,
            Effect::ScheduleTrigger,
        ),
        (
            before.details.siren_timer,
            result.state.details.siren_timer,
            Effect::ScheduleSiren,
        ),
    ] {
        if new != old
            && let Some(token) = new
        {
            result.effects.push(effect(token));
        }
    }
    for (sensor, token) in &result.state.details.close_timers {
        if before.details.close_timers.get(sensor) != Some(token) {
            result.effects.push(Effect::ScheduleArmOnClose {
                sensor: sensor.clone(),
                token: *token,
            });
        }
    }
    for sensor in before.details.close_timers.keys() {
        if !result.state.details.close_timers.contains_key(sensor) {
            result
                .effects
                .push(Effect::CancelArmOnClose(sensor.clone()));
        }
    }
    result
}
