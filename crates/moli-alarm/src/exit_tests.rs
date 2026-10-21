use std::cell::RefCell;
use std::collections::BTreeMap;

use crate::{
    ArmRequest, Authorization, AuthorizationFailure, Config, DisarmRequest, Effect, Event, Mode,
    ModeConfig, Permission, Phase, ProfileId, Profiles, RequestId, Source, State, TimerToken,
    TimestampMs, Transition, Verdict, step,
};

#[derive(Debug, PartialEq, Eq)]
struct AuthorizationCall {
    request: RequestId,
    source: Source,
    permission: Permission,
    mode: Option<Mode>,
    now: TimestampMs,
}

struct TestProfiles {
    result: Authorization,
    calls: RefCell<Vec<AuthorizationCall>>,
}

impl TestProfiles {
    fn new(result: Authorization) -> Self {
        Self {
            result,
            calls: RefCell::new(Vec::new()),
        }
    }
}

impl Profiles for TestProfiles {
    fn authorize(
        &self,
        request: RequestId,
        source: &Source,
        permission: Permission,
        mode: Option<Mode>,
        now: TimestampMs,
    ) -> Authorization {
        self.calls.borrow_mut().push(AuthorizationCall {
            request,
            source: source.clone(),
            permission,
            mode,
            now,
        });
        self.result.clone()
    }
}

fn no_code() -> TestProfiles {
    TestProfiles::new(Authorization::Missing)
}

fn configuration() -> Config {
    let mode = ModeConfig {
        exit_s: 30,
        entry_s: 30,
        trigger_s: 1800,
    };
    Config {
        modes: BTreeMap::from([(Mode::Away, mode), (Mode::Home, mode)]),
        siren_max_s: 180,
        ..Config::default()
    }
}

fn exit_timer() -> TimerToken {
    TimerToken {
        generation: 7,
        deadline: TimestampMs(31_000),
    }
}

fn arming_fixture() -> State {
    // Direct fixture: no scenario depends on delayed arming already working.
    State {
        phase: Phase::Arming(Mode::Away),
        generation: 7,
        exit_timer: Some(exit_timer()),
        ..State::default()
    }
}

fn arm(mode: Mode, id: RequestId) -> Event {
    Event::Arm(ArmRequest {
        id,
        mode,
        source: Source::Household,
        skip_exit: false,
        force: std::collections::BTreeSet::new(),
    })
}

#[test]
fn exit_delay_accepts_with_absolute_deadline() {
    // B1, spec 4.1/4.3: accepted arming is distinct from confirmed armed.
    let result = step(
        State::default(),
        arm(Mode::Away, RequestId(2)),
        TimestampMs(1_000),
        &configuration(),
        &no_code(),
    );
    let timer = TimerToken {
        generation: 1,
        deadline: TimestampMs(31_000),
    };
    assert_eq!(result.state.phase(), Phase::Arming(Mode::Away));
    assert_eq!(result.state.generation(), 1);
    assert_eq!(result.state.exit_timer(), Some(timer));
    assert_eq!(result.verdict, Some(Verdict::AcceptedArming(Mode::Away)));
    assert_eq!(
        result.effects,
        [
            Effect::JournalTransition {
                from: Phase::Disarmed,
                to: Phase::Arming(Mode::Away),
            },
            Effect::ScheduleExit(timer),
        ]
    );
}

#[test]
fn exit_delay_repeated_requests_keep_the_original_deadline() {
    // B2, spec 4.3/4.4: exercise both requests before checking either result.
    let initial = arming_fixture();
    let repeated = step(
        initial.clone(),
        arm(Mode::Away, RequestId(3)),
        TimestampMs(5_000),
        &configuration(),
        &no_code(),
    );
    let other = step(
        initial.clone(),
        arm(Mode::Home, RequestId(4)),
        TimestampMs(6_000),
        &configuration(),
        &no_code(),
    );
    assert_eq!(repeated.state, initial);
    assert_eq!(other.state, initial);
    assert_eq!(repeated.verdict, Some(Verdict::AlreadyArming(Mode::Away)));
    assert!(repeated.effects.is_empty());
    assert_eq!(other.verdict, Some(Verdict::Busy));
    assert_eq!(other.effects, [Effect::JournalRefusal(Verdict::Busy)]);
}

#[test]
fn exit_delay_expires_only_for_the_current_generation() {
    // B3: every input starts from its own direct fixture, not from a failed setup.
    let initial = arming_fixture();
    let token = exit_timer();
    let early = step(
        initial.clone(),
        Event::ExitExpired(token),
        TimestampMs(30_999),
        &configuration(),
        &no_code(),
    );
    let stale = step(
        initial.clone(),
        Event::ExitExpired(TimerToken {
            generation: 6,
            ..token
        }),
        TimestampMs(31_000),
        &configuration(),
        &no_code(),
    );
    let expired = step(
        initial.clone(),
        Event::ExitExpired(token),
        TimestampMs(31_000),
        &configuration(),
        &no_code(),
    );
    let armed = State {
        phase: Phase::Armed(Mode::Away),
        generation: 8,
        exit_timer: None,
        ..State::default()
    };
    let replay = step(
        armed.clone(),
        Event::ExitExpired(token),
        TimestampMs(32_000),
        &configuration(),
        &no_code(),
    );
    for ignored in [early, stale] {
        assert_eq!(ignored.state, initial);
        assert!(ignored.effects.is_empty());
        assert_eq!(ignored.verdict, None);
    }
    assert_eq!(replay.state, armed);
    assert!(replay.effects.is_empty());
    assert_eq!(replay.verdict, None);
    assert_eq!(expired.state.phase(), Phase::Armed(Mode::Away));
    assert_eq!(expired.state.generation(), 8);
    assert_eq!(expired.state.exit_timer(), None);
    assert_eq!(expired.verdict, None);
    assert_eq!(
        expired.effects,
        [
            Effect::JournalTransition {
                from: Phase::Arming(Mode::Away),
                to: Phase::Armed(Mode::Away),
            },
            Effect::NotifyArmed(Mode::Away),
        ]
    );
}

struct DisarmOutcome {
    transition: Transition,
    calls: Vec<AuthorizationCall>,
}

fn disarm_case(result: Authorization, id: RequestId) -> DisarmOutcome {
    let profiles = TestProfiles::new(result);
    let transition = step(
        arming_fixture(),
        Event::Disarm(DisarmRequest {
            id,
            source: Source::Ui,
        }),
        TimestampMs(5_000),
        &configuration(),
        &profiles,
    );
    DisarmOutcome {
        transition,
        calls: profiles.calls.into_inner(),
    }
}

#[test]
fn exit_delay_disarm_invalidates_the_previous_timer() {
    // B4, spec 4.3/6/7: no code value, credential handling, or external service.
    let missing = disarm_case(Authorization::Missing, RequestId(10));
    let invalid = disarm_case(
        Authorization::Invalid(AuthorizationFailure::Revoked),
        RequestId(11),
    );
    let valid = disarm_case(
        Authorization::Allowed(ProfileId("test-profile".into())),
        RequestId(12),
    );
    let late_timer = step(
        valid.transition.state.clone(),
        Event::ExitExpired(exit_timer()),
        TimestampMs(31_000),
        &configuration(),
        &no_code(),
    );
    // All three disarm requests and the old timer have reached step already.
    assert_eq!(valid.transition.state.phase(), Phase::Disarmed);
    assert_eq!(valid.transition.state.generation(), 8);
    assert_eq!(valid.transition.state.exit_timer(), None);
    assert_eq!(valid.transition.verdict, Some(Verdict::Disarmed));
    assert_eq!(
        valid.transition.effects,
        [
            Effect::JournalTransition {
                from: Phase::Arming(Mode::Away),
                to: Phase::Disarmed,
            },
            Effect::CancelExit(exit_timer()),
        ]
    );
    for (outcome, verdict) in [
        (&missing, Verdict::CodeRequired),
        (&invalid, Verdict::InvalidCode),
    ] {
        assert_eq!(outcome.transition.state, arming_fixture());
        assert_eq!(outcome.transition.verdict, Some(verdict));
        assert_eq!(
            outcome.transition.effects,
            [Effect::JournalRefusal(verdict)]
        );
    }
    for (outcome, id) in [
        (&missing, RequestId(10)),
        (&invalid, RequestId(11)),
        (&valid, RequestId(12)),
    ] {
        assert_eq!(
            outcome.calls,
            [AuthorizationCall {
                request: id,
                source: Source::Ui,
                permission: Permission::Disarm,
                mode: Some(Mode::Away),
                now: TimestampMs(5_000),
            }]
        );
    }
    assert_eq!(late_timer.state, valid.transition.state);
    assert!(late_timer.effects.is_empty());
    assert_eq!(late_timer.verdict, None);
}
