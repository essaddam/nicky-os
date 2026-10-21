use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use crate::configuration::{PointCatalog, PointDescriptor};
use crate::coverage::{
    ArmPeriod, Exclusions, ObservationState, PointSemantic, SensorId, SensorPolicy, SensorState,
};
use crate::{
    ArmRequest, Authorization, Cause, Config, DisarmRequest, Effect, Event, Mode, ModeConfig,
    Notice, Permission, Phase, ProfileId, Profiles, RequestId, RestoreRequest, ReturnFrame,
    SavedState, Source, State, TimerToken, TimestampMs, Transition, Verdict, step,
};

pub(super) fn id(name: &str) -> SensorId {
    SensorId(name.into())
}
pub(super) fn ids(names: &[&str]) -> BTreeSet<SensorId> {
    names.iter().map(|name| id(name)).collect()
}
pub(super) fn token(generation: u64, deadline: u64) -> TimerToken {
    TimerToken {
        generation,
        deadline: TimestampMs(deadline),
    }
}
pub(super) fn allowed() -> Authorization {
    Authorization::Allowed(ProfileId("test-profile".into()))
}

pub(super) fn config() -> Config {
    let modes = BTreeMap::from([
        (
            Mode::Away,
            ModeConfig {
                exit_s: 0,
                entry_s: 30,
                trigger_s: 600,
            },
        ),
        (
            Mode::Home,
            ModeConfig {
                exit_s: 60,
                entry_s: 0,
                trigger_s: 120,
            },
        ),
        (
            Mode::Night,
            ModeConfig {
                exit_s: 30,
                entry_s: 30,
                trigger_s: 600,
            },
        ),
        (
            Mode::Vacation,
            ModeConfig {
                exit_s: 30,
                entry_s: 30,
                trigger_s: 600,
            },
        ),
    ]);
    let all_modes = BTreeSet::from([Mode::Away, Mode::Home, Mode::Night, Mode::Vacation]);
    let a = SensorPolicy {
        id: id("sensor-a"),
        modes: all_modes.clone(),
        arm_on_close: true,
        ..SensorPolicy::default()
    };
    let b = SensorPolicy {
        id: id("sensor-b"),
        modes: all_modes,
        entry: false,
        exit: false,
        ..SensorPolicy::default()
    };
    let short = SensorPolicy {
        id: id("short"),
        modes: BTreeSet::from([Mode::Away]),
        entry_s: Some(5),
        ..SensorPolicy::default()
    };
    Config {
        modes,
        sensors: vec![a, b, short],
        sirens: vec!["siren-a".into(), "siren-b".into()],
        keypads: vec!["keypad-a".into()],
        ..Config::default()
    }
}

pub(super) fn catalog() -> PointCatalog {
    let sensor = PointDescriptor {
        semantic: PointSemantic::Contact,
        readable: true,
        writable: false,
        binary: true,
        private_keypad_input: false,
    };
    let siren = PointDescriptor {
        semantic: PointSemantic::Other,
        readable: true,
        writable: true,
        binary: true,
        private_keypad_input: false,
    };
    let keypad = PointDescriptor {
        semantic: PointSemantic::Other,
        readable: false,
        writable: false,
        binary: false,
        private_keypad_input: true,
    };
    BTreeMap::from([
        ("sensor-a".into(), sensor.clone()),
        ("sensor-b".into(), sensor.clone()),
        ("short".into(), sensor),
        ("siren-a".into(), siren.clone()),
        ("siren-b".into(), siren),
        ("keypad-a".into(), keypad),
    ])
}

pub(super) fn state(phase: Phase) -> State {
    let mut result = State {
        phase,
        generation: 7,
        ..State::default()
    };
    result.details.startup_at = TimestampMs(0);
    result.details.period = ArmPeriod(9);
    for name in ["sensor-a", "sensor-b", "short"] {
        result.details.observations.insert(
            id(name),
            ObservationState {
                started_at: TimestampMs(0),
                last: SensorState::Closed,
                confirmed: true,
            },
        );
    }
    if let Some(mode) = phase.mode() {
        result.details.exclusions = Some(Exclusions::new(ArmPeriod(9), mode));
    }
    match phase {
        Phase::Arming(_) => {
            result.exit_timer = Some(token(7, 31_000));
            result.details.return_to = Some(ReturnFrame {
                phase: Phase::Disarmed,
                period: ArmPeriod(8),
                ..ReturnFrame::default()
            });
        }
        Phase::Pending(_) => {
            result.details.entry_timer = Some(token(7, 31_000));
            result.details.cause = Some(Cause::Sensor(id("sensor-a")));
        }
        Phase::Triggered(mode) => {
            result.details.siren_on = true;
            result.details.siren_timer = Some(token(7, 181_000));
            result.details.trigger_timer = mode.map(|_| token(7, 601_000));
            result.details.seen = ids(&["sensor-a"]);
            result.details.cause = Some(Cause::Sensor(id("sensor-a")));
        }
        _ => {}
    }
    result
}

pub(super) fn value(state: &mut State, name: &str, reading: SensorState, confirmed: bool) {
    state.details.observations.insert(
        id(name),
        ObservationState {
            started_at: TimestampMs(0),
            last: reading,
            confirmed,
        },
    );
}

pub(super) fn arm(mode: Mode, source: Source) -> Event {
    Event::Arm(ArmRequest {
        id: RequestId(1),
        mode,
        source,
        skip_exit: false,
        force: BTreeSet::new(),
    })
}
pub(super) fn disarm(source: Source) -> Event {
    Event::Disarm(DisarmRequest {
        id: RequestId(2),
        source,
    })
}
pub(super) fn restore(saved: Option<SavedState>, configured: bool) -> Event {
    Event::Restore(Box::new(RestoreRequest {
        saved,
        configured,
        cached: BTreeMap::new(),
    }))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Message {
    Armed(Mode),
    Other(Notice),
}

#[derive(Default)]
// Three states: no assertion, timer absent, or an exact expected timer.
#[allow(clippy::option_option, clippy::struct_excessive_bools)]
pub(super) struct Expect {
    pub phase: Phase,
    pub verdict: Option<Verdict>,
    pub generation: Option<u64>,
    pub commands: Vec<(String, bool)>,
    pub messages: Vec<Message>,
    pub exit: Option<Option<TimerToken>>,
    pub entry: Option<Option<TimerToken>>,
    pub trigger: Option<Option<TimerToken>>,
    pub siren: Option<Option<TimerToken>>,
    pub siren_on: Option<bool>,
    pub cause: Option<Cause>,
    pub seen: Option<BTreeSet<SensorId>>,
    pub automatic: Option<BTreeSet<SensorId>>,
    pub forced: Option<BTreeSet<SensorId>>,
    pub blocking: Option<BTreeSet<SensorId>>,
    pub close: Option<(SensorId, Option<TimerToken>)>,
    pub retained: bool,
    pub same_state: bool,
    pub no_effects: bool,
    pub permission: Option<Permission>,
    pub no_auth: bool,
    pub startup_at: Option<u64>,
    pub cold_observations: bool,
}

impl Expect {
    pub(super) fn phase(phase: Phase) -> Self {
        Self {
            phase,
            ..Self::default()
        }
    }
    pub(super) fn verdict(mut self, verdict: Verdict) -> Self {
        self.verdict = Some(verdict);
        self
    }
    pub(super) fn generation(mut self, generation: u64) -> Self {
        self.generation = Some(generation);
        self
    }
    pub(super) fn messages(mut self, messages: Vec<Message>) -> Self {
        self.messages = messages;
        self
    }
    pub(super) fn commands(mut self, on: bool) -> Self {
        self.commands = commands(on);
        self
    }
    pub(super) fn permission(mut self, permission: Permission) -> Self {
        self.permission = Some(permission);
        self
    }
    pub(super) fn same(mut self) -> Self {
        self.same_state = true;
        self
    }
    pub(super) fn ignored(mut self) -> Self {
        self.same_state = true;
        self.no_effects = true;
        self
    }
}

pub(super) fn commands(on: bool) -> Vec<(String, bool)> {
    vec![("siren-a".into(), on), ("siren-b".into(), on)]
}

pub(super) struct Case {
    pub name: String,
    pub before: State,
    pub event: Event,
    pub now: u64,
    pub config: Config,
    pub auth: Authorization,
    pub permissions: BTreeMap<Permission, Authorization>,
    pub expected: Expect,
}
impl Case {
    pub(super) fn new(name: &str, before: State, event: Event, expected: Expect) -> Self {
        Self {
            name: name.into(),
            before,
            event,
            now: 1_000,
            config: config(),
            auth: allowed(),
            permissions: BTreeMap::new(),
            expected,
        }
    }
    pub(super) fn at(mut self, now: u64) -> Self {
        self.now = now;
        self
    }
    pub(super) fn auth(mut self, auth: Authorization) -> Self {
        self.auth = auth;
        self
    }
    pub(super) fn config(mut self, config: Config) -> Self {
        self.config = config;
        self
    }
    pub(super) fn override_permission(
        mut self,
        permission: Permission,
        answer: Authorization,
    ) -> Self {
        self.permissions.insert(permission, answer);
        self
    }
}

#[derive(Debug)]
struct Call {
    permission: Permission,
    now: TimestampMs,
}
struct TestProfiles {
    answer: Authorization,
    permissions: BTreeMap<Permission, Authorization>,
    calls: RefCell<Vec<Call>>,
}
impl Profiles for TestProfiles {
    fn authorize(
        &self,
        _request: RequestId,
        _source: &Source,
        permission: Permission,
        _mode: Option<Mode>,
        now: TimestampMs,
    ) -> Authorization {
        self.calls.borrow_mut().push(Call { permission, now });
        self.permissions
            .get(&permission)
            .unwrap_or(&self.answer)
            .clone()
    }
}

pub(super) struct Row {
    pub name: String,
    pub checks: Vec<(&'static str, bool)>,
}
impl Row {
    pub(super) fn check(name: &str, field: &'static str, passed: bool) -> Self {
        Self {
            name: name.into(),
            checks: vec![(field, passed)],
        }
    }
}

pub(super) fn evaluate(cases: Vec<Case>) -> Vec<Row> {
    cases.into_iter().map(evaluate_case).collect()
}
fn evaluate_case(case: Case) -> Row {
    let profiles = TestProfiles {
        answer: case.auth,
        permissions: case.permissions,
        calls: RefCell::new(Vec::new()),
    };
    let result = step(
        case.before.clone(),
        case.event,
        TimestampMs(case.now),
        &case.config,
        &profiles,
    );
    let mut row = compare(&case.name, &case.before, &result, &case.expected);
    row.checks.extend(timer_invariants(
        &case.before,
        &result,
        case.now,
        &case.config,
    ));
    let calls = profiles.calls.into_inner();
    row.checks.push((
        "authorization_time",
        calls.iter().all(|call| call.now == TimestampMs(case.now)),
    ));
    if let Some(permission) = case.expected.permission {
        row.checks.push((
            "permission",
            calls.iter().any(|call| call.permission == permission),
        ));
    }
    if case.expected.no_auth {
        row.checks.push(("no_auth", calls.is_empty()));
    }
    row
}

fn compare(name: &str, before: &State, result: &Transition, expected: &Expect) -> Row {
    let mut actual_commands: Vec<_> = result
        .effects
        .iter()
        .filter_map(|effect| {
            if let Effect::Siren { point, on } = effect {
                Some((point.clone(), *on))
            } else {
                None
            }
        })
        .collect();
    actual_commands.sort();
    let mut wanted_commands = expected.commands.clone();
    wanted_commands.sort();
    let actual_messages: Vec<_> = result
        .effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::NotifyArmed(mode) => Some(Message::Armed(*mode)),
            Effect::Notify(notice) => Some(Message::Other(notice.clone())),
            _ => None,
        })
        .collect();
    let scheduled: Vec<_> = result
        .effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::ScheduleExit(token)
            | Effect::ScheduleEntry(token)
            | Effect::ScheduleTrigger(token)
            | Effect::ScheduleSiren(token)
            | Effect::ScheduleArmOnClose { token, .. } => Some(*token),
            _ => None,
        })
        .collect();
    let mut checks = vec![
        ("phase", result.state.phase() == expected.phase),
        ("verdict", result.verdict == expected.verdict),
        ("siren_commands", actual_commands == wanted_commands),
        ("messages", actual_messages == expected.messages),
        (
            "retained",
            result
                .effects
                .iter()
                .any(|effect| matches!(effect, Effect::RetainArm { .. }))
                == expected.retained,
        ),
        (
            "scheduled_generation",
            scheduled
                .iter()
                .all(|token| token.generation == result.state.generation()),
        ),
    ];
    if let Some(generation) = expected.generation {
        checks.push(("generation", result.state.generation() == generation));
    }
    for (name, actual, wanted) in [
        ("exit_timer", result.state.exit_timer(), expected.exit),
        ("entry_timer", result.state.entry_timer(), expected.entry),
        (
            "trigger_timer",
            result.state.trigger_timer(),
            expected.trigger,
        ),
        ("siren_timer", result.state.siren_timer(), expected.siren),
    ] {
        if let Some(wanted) = wanted {
            checks.push((name, actual == wanted));
        }
    }
    if let Some(on) = expected.siren_on {
        checks.push(("siren_on", result.state.siren_on() == on));
    }
    if let Some(cause) = &expected.cause {
        checks.push(("cause", result.state.cause() == Some(cause)));
    }
    if let Some(seen) = &expected.seen {
        checks.push(("seen", &result.state.details.seen == seen));
    }
    if let Some(automatic) = &expected.automatic {
        checks.push((
            "automatic",
            result
                .state
                .details
                .exclusions
                .as_ref()
                .is_some_and(|value| value.automatic() == automatic),
        ));
    }
    if let Some(forced) = &expected.forced {
        checks.push((
            "forced",
            result
                .state
                .details
                .exclusions
                .as_ref()
                .is_some_and(|value| value.forced() == forced),
        ));
    }
    if let Some(blocking) = &expected.blocking {
        checks.push(("blocking", result.effects.iter().any(|effect| matches!(effect, Effect::Blocking(points) if points.iter().cloned().collect::<BTreeSet<_>>() == *blocking))));
    }
    if let Some((sensor, timer)) = &expected.close {
        checks.push((
            "close_timer",
            result.state.details.close_timers.get(sensor).copied() == *timer,
        ));
    }
    if let Some(at) = expected.startup_at {
        checks.push((
            "startup_at",
            result.state.details.startup_at == TimestampMs(at),
        ));
    }
    if expected.cold_observations {
        checks.push((
            "cold_observations",
            result
                .state
                .details
                .observations
                .values()
                .all(|value| !value.confirmed()),
        ));
    }
    if expected.same_state {
        checks.push(("state_unchanged", &result.state == before));
    }
    if expected.no_effects {
        checks.push(("no_effects", result.effects.is_empty()));
    }
    Row {
        name: name.into(),
        checks,
    }
}

pub(super) fn finish(group: &str, rows: Vec<Row>) -> bool {
    let mut passed = true;
    for row in rows {
        let failures: Vec<_> = row
            .checks
            .iter()
            .filter_map(|(field, pass)| if *pass { None } else { Some(*field) })
            .collect();
        let row_passed = failures.is_empty();
        println!(
            "CASE {group}::{} {} {:?}",
            row.name,
            if row_passed { "PASS" } else { "FAIL" },
            failures
        );
        passed &= row_passed;
    }
    passed
}

pub(super) fn saved(phase: Phase) -> SavedState {
    // Direct input data, never produced by the save/restore implementation.
    let input = state(phase);
    SavedState {
        saved_at: TimestampMs(1_000),
        phase,
        generation: 7,
        exit_timer: input.exit_timer,
        entry_timer: input.details.entry_timer,
        trigger_timer: input.details.trigger_timer,
        siren_timer: input.details.siren_timer,
        siren_on: input.details.siren_on,
        return_to: input.details.return_to,
        cause: input.details.cause,
        seen: input.details.seen,
        period: input.details.period,
        automatic: std::collections::BTreeSet::new(),
        forced: std::collections::BTreeSet::new(),
        close_timers: input.details.close_timers,
        unavailable_notified: input.details.unavailable_notified,
    }
}

fn timer_invariants(
    before: &State,
    result: &Transition,
    now: u64,
    config: &Config,
) -> Vec<(&'static str, bool)> {
    let state = &result.state;
    let current = [
        state.exit_timer(),
        state.entry_timer(),
        state.trigger_timer(),
        state.siren_timer(),
    ];
    let prior = [
        before.exit_timer(),
        before.entry_timer(),
        before.trigger_timer(),
        before.siren_timer(),
    ];
    let generations = current
        .iter()
        .flatten()
        .chain(state.details.close_timers.values())
        .all(|timer| timer.generation == state.generation());
    let no_extension = prior
        .into_iter()
        .zip(current)
        .all(|(old, new)| match (old, new) {
            (Some(old), Some(new)) => new.deadline <= old.deadline,
            _ => true,
        });
    let placement = (state.exit_timer().is_none() || matches!(state.phase(), Phase::Arming(_)))
        && (state.entry_timer().is_none() || matches!(state.phase(), Phase::Pending(_)))
        && (state.trigger_timer().is_none() || matches!(state.phase(), Phase::Triggered(Some(_))))
        && (state.siren_timer().is_none() || matches!(state.phase(), Phase::Triggered(_)))
        && (state.details.close_timers.is_empty() || matches!(state.phase(), Phase::Arming(_)));
    let on_command = result
        .effects
        .iter()
        .any(|effect| matches!(effect, Effect::Siren { on: true, .. }));
    let on_state = !state.siren_on()
        || (matches!(state.phase(), Phase::Triggered(_)) && state.siren_timer().is_some());
    let bounded = state.siren_timer().is_none_or(|timer| {
        timer.deadline.0 <= now.saturating_add(u64::from(config.siren_max_s.min(180)) * 1_000)
    });
    vec![
        ("all_timer_generations", generations),
        ("surviving_deadlines", no_extension),
        ("timer_placement", placement),
        ("siren_state", on_state),
        ("on_command_has_timer", !on_command || state.siren_on()),
        ("siren_bound", bounded),
    ]
}
