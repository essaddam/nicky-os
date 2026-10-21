use super::{advance, current, deadline, set_reading, unchanged};
use crate::coverage::{self, CoverageEvent, ObservationState, SensorId, SensorPolicy, SensorState};
use crate::{
    ActionPolicy, Cause, Config, Effect, Event, Notice, Phase, Source, State, TimerToken,
    TimestampMs, Transition,
};

fn covered(policy: &SensorPolicy, phase: Phase) -> bool {
    policy.enabled
        && (policy.always_on
            || phase
                .mode()
                .is_some_and(|mode| policy.modes.contains(&mode)))
}
fn monitored(state: &State, policy: &SensorPolicy) -> bool {
    covered(policy, state.phase)
        && state.details.exclusions.as_ref().is_none_or(|e| {
            e.period != state.details.period
                || Some(e.mode) != state.phase.mode()
                || (!e.automatic.contains(&policy.id) && !e.forced.contains(&policy.id))
        })
}
pub(super) fn event(
    mut state: State,
    event: Event,
    now: TimestampMs,
    config: &Config,
) -> Transition {
    match event {
        Event::Observe {
            sensor,
            observation,
        } => {
            let Some(policy) = config.sensors.iter().find(|p| p.id == sensor && p.enabled) else {
                return unchanged(state);
            };
            let old = state
                .details
                .observations
                .get(&sensor)
                .cloned()
                .unwrap_or_else(|| ObservationState::new(state.details.startup_at));
            let closed = old.last() != SensorState::Closed;
            let observed = coverage::observe(old, observation, monitored(&state, policy));
            let closing = closed
                && observed.state.confirmed()
                && observed.state.last() == SensorState::Closed;
            state
                .details
                .observations
                .insert(sensor.clone(), observed.state);
            match observed.event {
                Some(CoverageEvent::Open) => open(state, &sensor, now, config),
                Some(CoverageEvent::Tamper) => tamper(state, &sensor, now, config),
                None if closing => close(state, &sensor, now, config, true),
                None => unchanged(state),
            }
        }
        Event::Open(sensor) => {
            if !config.sensors.iter().any(|p| p.id == sensor && p.enabled) {
                return unchanged(state);
            }
            set_reading(&mut state, &sensor, SensorState::Open, true);
            open(state, &sensor, now, config)
        }
        Event::Close(sensor) => {
            if !config.sensors.iter().any(|p| p.id == sensor && p.enabled) {
                return unchanged(state);
            }
            let changed = state
                .details
                .observations
                .get(&sensor)
                .is_none_or(|o| !o.confirmed() || o.last() != SensorState::Closed);
            set_reading(&mut state, &sensor, SensorState::Closed, true);
            close(state, &sensor, now, config, changed)
        }
        Event::Unavailable(sensor) => unavailable(state, &sensor, now, config),
        Event::Available(sensor) => {
            if config.sensors.iter().any(|p| p.id == sensor && p.enabled) {
                set_reading(&mut state, &sensor, SensorState::Unknown, false);
            }
            unchanged(state)
        }
        Event::Tamper(sensor) => tamper(state, &sensor, now, config),
        Event::Panic(Source::Keypad(keypad)) if config.keypads.contains(&keypad) => {
            panic(state, now, config)
        }
        Event::ArmOnClose { sensor, token }
            if matches!(state.phase, Phase::Arming(_))
                && current(
                    &state,
                    state.details.close_timers.get(&sensor).copied(),
                    token,
                    now,
                )
                && config
                    .sensors
                    .iter()
                    .any(|p| p.id == sensor && p.arm_on_close && monitored(&state, p))
                && state
                    .details
                    .observations
                    .get(&sensor)
                    .is_some_and(|o| o.confirmed() && o.last() == SensorState::Closed) =>
        {
            super::transitions::exit(state, now, config, false)
        }
        _ => unchanged(state),
    }
}
fn close(
    mut state: State,
    sensor: &SensorId,
    now: TimestampMs,
    config: &Config,
    changed: bool,
) -> Transition {
    if !changed
        || !matches!(state.phase, Phase::Arming(_))
        || !config
            .sensors
            .iter()
            .any(|p| &p.id == sensor && p.arm_on_close && monitored(&state, p))
    {
        return unchanged(state);
    }
    let Some(until) = deadline(now, 5) else {
        return unchanged(state);
    };
    if !advance(&mut state) {
        return unchanged(state);
    }
    state.details.close_timers.insert(
        sensor.clone(),
        TimerToken {
            generation: state.generation,
            deadline: until,
        },
    );
    unchanged(state)
}
fn open(mut state: State, sensor: &SensorId, now: TimestampMs, config: &Config) -> Transition {
    let Some(policy) = config
        .sensors
        .iter()
        .find(|p| &p.id == sensor && monitored(&state, p))
    else {
        return unchanged(state);
    };
    match state.phase {
        Phase::Disarmed if policy.always_on => {
            super::transitions::trigger(state, Cause::Sensor(sensor.clone()), now, config)
        }
        Phase::Arming(_) if policy.exit => {
            if state.details.close_timers.contains_key(sensor) && advance(&mut state) {
                state.details.close_timers.remove(sensor);
            }
            unchanged(state)
        }
        Phase::Arming(_) | Phase::Armed(_) | Phase::Pending(_) => entry(state, policy, now, config),
        Phase::Triggered(_) => episode_sensor(state, sensor, now, config),
        Phase::Disarmed => unchanged(state),
    }
}
fn entry(mut state: State, policy: &SensorPolicy, now: TimestampMs, config: &Config) -> Transition {
    let Some(mode) = state.phase.mode() else {
        return unchanged(state);
    };
    let Some(mode_config) = config.modes.get(&mode) else {
        return unchanged(state);
    };
    let seconds = if policy.entry {
        policy
            .entry_s
            .unwrap_or(mode_config.entry_s)
            .min(mode_config.entry_s)
    } else {
        0
    };
    if seconds == 0 {
        return super::transitions::trigger(state, Cause::Sensor(policy.id.clone()), now, config);
    }
    let Some(until) = deadline(now, seconds) else {
        return unchanged(state);
    };
    let pending = matches!(state.phase, Phase::Pending(_));
    if pending
        && state
            .details
            .entry_timer
            .is_some_and(|t| t.deadline <= until)
    {
        return unchanged(state);
    }
    if !advance(&mut state) {
        return unchanged(state);
    }
    state.phase = Phase::Pending(mode);
    state.exit_timer = None;
    state.details.close_timers.clear();
    state.details.entry_timer = Some(TimerToken {
        generation: state.generation,
        deadline: until,
    });
    if !pending {
        state.details.cause = Some(Cause::Sensor(policy.id.clone()));
    }
    Transition {
        state,
        effects: if pending {
            vec![]
        } else {
            vec![Effect::Notify(Notice::Pending(mode))]
        },
        verdict: None,
    }
}
fn episode_sensor(
    mut state: State,
    sensor: &SensorId,
    now: TimestampMs,
    config: &Config,
) -> Transition {
    if state.details.seen.contains(sensor) {
        return unchanged(state);
    }
    let until = if state.details.siren_on {
        None
    } else {
        let Some(until) = deadline(now, config.siren_max_s.clamp(1, 180)) else {
            return unchanged(state);
        };
        Some(until)
    };
    if !advance(&mut state) {
        return unchanged(state);
    }
    state.details.seen.insert(sensor.clone());
    let mut effects = Vec::new();
    if let Some(deadline) = until {
        state.details.siren_on = true;
        state.details.siren_timer = Some(TimerToken {
            generation: state.generation,
            deadline,
        });
        effects.extend(config.sirens.iter().map(|point| Effect::Siren {
            point: point.clone(),
            on: true,
        }));
    }
    Transition {
        state,
        effects,
        verdict: None,
    }
}
fn unavailable(
    mut state: State,
    sensor: &SensorId,
    now: TimestampMs,
    config: &Config,
) -> Transition {
    let Some(policy) = config.sensors.iter().find(|p| &p.id == sensor && p.enabled) else {
        return unchanged(state);
    };
    let watched = monitored(&state, policy);
    set_reading(&mut state, sensor, SensorState::Unavailable, false);
    if !watched {
        return unchanged(state);
    }
    if policy.trigger_unavailable && matches!(state.phase, Phase::Armed(_) | Phase::Pending(_)) {
        return open(state, sensor, now, config);
    }
    let notify = matches!(
        state.phase,
        Phase::Arming(_) | Phase::Armed(_) | Phase::Pending(_)
    ) && state.details.unavailable_notified.insert(sensor.clone());
    Transition {
        state,
        effects: if notify {
            vec![Effect::Notify(Notice::Unavailable(sensor.clone()))]
        } else {
            vec![]
        },
        verdict: None,
    }
}
fn tamper(state: State, sensor: &SensorId, now: TimestampMs, config: &Config) -> Transition {
    let configured = config.sirens.contains(&sensor.0)
        || config.keypads.contains(&sensor.0)
        || config.sensors.iter().any(|p| {
            &p.id == sensor
                && p.enabled
                && (state.phase == Phase::Disarmed || covered(p, state.phase))
        });
    if !configured {
        return unchanged(state);
    }
    match state.phase {
        Phase::Arming(_) | Phase::Armed(_) | Phase::Pending(_)
            if config.tamper_armed == ActionPolicy::Trigger =>
        {
            super::transitions::trigger(state, Cause::Tamper(sensor.clone()), now, config)
        }
        Phase::Disarmed | Phase::Arming(_) | Phase::Armed(_) | Phase::Pending(_) => Transition {
            state,
            effects: vec![Effect::Notify(Notice::Tamper(sensor.clone()))],
            verdict: None,
        },
        Phase::Triggered(_) => unchanged(state),
    }
}
fn panic(state: State, now: TimestampMs, config: &Config) -> Transition {
    match state.phase {
        Phase::Arming(_) | Phase::Armed(_) | Phase::Pending(_)
            if config.panic == ActionPolicy::Trigger =>
        {
            super::transitions::trigger(state, Cause::Panic, now, config)
        }
        Phase::Disarmed | Phase::Arming(_) | Phase::Armed(_) | Phase::Pending(_) => Transition {
            state,
            effects: vec![Effect::Notify(Notice::Panic)],
            verdict: None,
        },
        Phase::Triggered(_) => unchanged(state),
    }
}
