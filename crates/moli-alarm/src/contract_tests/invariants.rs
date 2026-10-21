//! Integration contracts for the caller that executes absolute timer effects.
use crate::contract_tests::fixtures::{config, state, token};
use crate::{Authorization, Mode, Permission, Profiles, RequestId, Source};
use crate::{Effect, Event, Phase, State, TimestampMs, step};
struct NoCode;
impl Profiles for NoCode {
    fn authorize(
        &self,
        _: RequestId,
        _: &Source,
        _: Permission,
        _: Option<Mode>,
        _: TimestampMs,
    ) -> Authorization {
        Authorization::Missing
    }
}
#[test]
fn stopping_a_siren_reschedules_the_surviving_episode_without_extending_it() {
    let config = config();
    let stopped = step(
        state(Phase::Triggered(Some(Mode::Away))),
        Event::SirenExpired(token(7, 181_000)),
        TimestampMs(181_000),
        &config,
        &NoCode,
    );
    let surviving = stopped.state.trigger_timer().unwrap();
    assert_eq!(surviving, token(8, 601_000));
    assert!(
        stopped
            .effects
            .contains(&Effect::ScheduleTrigger(surviving))
    );
    let stale = step(
        stopped.state.clone(),
        Event::TriggerExpired(token(7, 601_000)),
        TimestampMs(601_000),
        &config,
        &NoCode,
    );
    assert_eq!(stale.state, stopped.state);
    assert!(stale.effects.is_empty());
    let elapsed = step(
        stopped.state,
        Event::TriggerExpired(surviving),
        TimestampMs(601_000),
        &config,
        &NoCode,
    );
    assert_eq!(elapsed.state.phase(), Phase::Armed(Mode::Away));
}
#[test]
fn generation_exhaustion_cannot_leave_a_siren_running_past_its_deadline() {
    let config = config();
    let mut initial = state(Phase::Triggered(Some(Mode::Away)));
    initial.generation = u64::MAX;
    initial.details.siren_timer = Some(token(u64::MAX, 181_000));
    initial.details.trigger_timer = Some(token(u64::MAX, 601_000));
    let stopped = step(
        initial,
        Event::SirenExpired(token(u64::MAX, 181_000)),
        TimestampMs(181_000),
        &config,
        &NoCode,
    );
    assert!(!stopped.state.siren_on());
    assert_eq!(stopped.state.siren_timer(), None);
    assert!(stopped.effects.contains(&Effect::Siren {
        point: "siren-a".into(),
        on: false
    }));
}
#[test]
fn overflowing_a_new_deadline_cannot_issue_an_unbounded_on_command() {
    let config = config();
    let initial = state(Phase::Armed(Mode::Away));
    let result = step(
        initial.clone(),
        Event::Open(crate::coverage::SensorId("sensor-b".into())),
        TimestampMs(u64::MAX),
        &config,
        &NoCode,
    );
    assert_eq!(result.state.phase(), initial.phase());
    assert!(
        !result
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::Siren { on: true, .. }))
    );
}
#[test]
fn an_empty_installation_has_no_current_timer() {
    let restored = step(
        State::default(),
        Event::Restore(Box::new(crate::RestoreRequest {
            configured: false,
            saved: None,
            cached: std::collections::BTreeMap::new(),
        })),
        TimestampMs(50),
        &config(),
        &NoCode,
    );
    assert_eq!(restored.state.phase(), Phase::Disarmed);
    assert_eq!(restored.state.generation(), 0);
    assert!(restored.effects.is_empty());
}

#[test]
fn forced_exclusions_from_the_old_period_do_not_authorize_rearming_a_blocker() {
    use super::fixtures::{id, ids, value};
    let mut config = config();
    config.after_trigger = crate::AfterTrigger::RearmOrDisarm;
    let mut initial = state(Phase::Triggered(Some(Mode::Away)));
    value(
        &mut initial,
        "sensor-b",
        crate::coverage::SensorState::Open,
        true,
    );
    initial.details.exclusions.as_mut().unwrap().forced = ids(&["sensor-b"]);
    let result = step(
        initial,
        Event::TriggerExpired(token(7, 601_000)),
        TimestampMs(601_000),
        &config,
        &NoCode,
    );
    assert_eq!(result.state.phase(), Phase::Disarmed);
    assert!(result.state.details.exclusions.is_none());
    assert!(
        result
            .effects
            .contains(&Effect::Notify(crate::Notice::Ended(None)))
    );
    assert!(result.effects.contains(&Effect::Siren {
        point: "siren-a".into(),
        on: false
    }));
    assert_eq!(
        result.state.coverage(Mode::Away, &config).blocking,
        std::collections::BTreeSet::from([id("sensor-b")])
    );
}
