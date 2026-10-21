use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use crate::coverage::{
    ArmPeriod, CoverageEvent, Exclusions, ForceRequest, ForceVerdict, Forced, Observation,
    ObservationOrigin, ObservationState, PointSemantic, SampleValue, SensorId, SensorPolicy,
    SensorState, SensorStatus, begin_period, end_period, force, interpret, observe, snapshot,
};
use crate::{
    Authorization, AuthorizationFailure, Mode, Permission, ProfileId, Profiles, RequestId, Source,
    TimestampMs,
};

#[test]
fn sensor_semantics_keep_unknown_and_tamper_distinct() {
    use PointSemantic::{Contact, Lock, Occupancy, Other as OtherPoint, Tamper as TamperPoint};
    use SensorState::{Closed, Open, Tamper, Unavailable, Unknown};
    let cases = [
        (Contact, true, SampleValue::Bool(false), Open),
        (Contact, true, SampleValue::Bool(true), Closed),
        (Occupancy, true, SampleValue::Bool(true), Open),
        (Occupancy, true, SampleValue::Bool(false), Closed),
        (Lock, true, SampleValue::Bool(false), Open),
        (Lock, true, SampleValue::Bool(true), Closed),
        (Lock, false, SampleValue::Bool(false), Unknown),
        (TamperPoint, true, SampleValue::Bool(true), Tamper),
        (TamperPoint, true, SampleValue::Bool(false), Closed),
        (OtherPoint, true, SampleValue::Bool(true), Unknown),
        (Contact, true, SampleValue::Null, Unknown),
        (Contact, true, SampleValue::Text("unexpected"), Unknown),
        (Contact, true, SampleValue::Other, Unknown),
        (Contact, true, SampleValue::Unavailable, Unavailable),
        (Lock, false, SampleValue::Unavailable, Unavailable),
    ];
    // Evaluate every row before an assertion: no interpretation installs another case.
    let results: Vec<_> = cases
        .into_iter()
        .map(|(semantic, readable, value, expected)| {
            (interpret(semantic, readable, value), expected)
        })
        .collect();
    for (actual, expected) in results {
        assert_eq!(actual, expected);
    }
}

fn observed_fixture(last: SensorState, confirmed: bool) -> ObservationState {
    ObservationState {
        started_at: TimestampMs(1_000),
        last,
        confirmed,
    }
}

fn reading(state: SensorState, origin: ObservationOrigin, at: u64) -> Observation {
    Observation {
        state,
        origin,
        observed_at: TimestampMs(at),
    }
}

fn fresh(state: SensorState, at: u64) -> Observation {
    reading(state, ObservationOrigin::Fresh, at)
}

#[test]
fn fresh_observations_emit_each_real_opening() {
    use SensorState::{Closed, Open, Tamper, Unavailable, Unknown};
    // C2 takes interpreted values directly, independently of C1.
    let cold = ObservationState::new(TimestampMs(1_000));
    let cached = observe(
        cold.clone(),
        reading(Open, ObservationOrigin::Cached, 2_000),
        true,
    );
    let too_old = observe(cold.clone(), fresh(Open, 999), true);
    let at_start = observe(cold.clone(), fresh(Open, 1_000), true);
    let first = observe(cold.clone(), fresh(Open, 2_000), true);
    let repeated = observe(observed_fixture(Open, true), fresh(Open, 2_100), true);
    let closed = observe(observed_fixture(Open, true), fresh(Closed, 2_200), true);
    let reopened = observe(observed_fixture(Closed, true), fresh(Open, 2_300), true);
    let cycle = observe(closed.state.clone(), fresh(Open, 2_300), true);
    let unknown = observe(observed_fixture(Open, true), fresh(Unknown, 2_400), true);
    let recovered = observe(observed_fixture(Unknown, false), fresh(Open, 2_500), true);
    let recovery_cycle = observe(unknown.state.clone(), fresh(Open, 2_500), true);
    let offline = observe(
        observed_fixture(Open, true),
        fresh(Unavailable, 2_401),
        true,
    );
    let online = observe(offline.state.clone(), fresh(Open, 2_501), true);
    let passive = observe(cold.clone(), fresh(Open, 2_600), false);
    let tamper = observe(cold, fresh(Tamper, 2_700), true);
    assert_eq!(first.event, Some(CoverageEvent::Open));
    for ignored in [cached, too_old, at_start] {
        assert!(!ignored.state.confirmed());
        assert_eq!(ignored.event, None);
    }
    for opening in [first, reopened, cycle, recovered, recovery_cycle, online] {
        assert_eq!(opening.event, Some(CoverageEvent::Open));
        assert_eq!(opening.state.last(), Open);
        assert!(opening.state.confirmed());
    }
    assert_eq!(repeated.event, None);
    assert_eq!(closed.state.last(), Closed);
    assert!(closed.state.confirmed());
    assert_eq!(closed.event, None);
    assert_eq!(unknown.state.last(), Unknown);
    assert!(!unknown.state.confirmed());
    assert_eq!(unknown.event, None);
    assert_eq!(offline.state.last(), Unavailable);
    assert!(!offline.state.confirmed());
    assert_eq!(offline.event, None);
    assert!(passive.state.confirmed());
    assert_eq!(passive.state.last(), Open);
    assert_eq!(passive.event, None);
    assert_eq!(tamper.state.last(), Tamper);
    assert!(tamper.state.confirmed());
    assert_eq!(tamper.event, Some(CoverageEvent::Tamper));
}

fn ids(names: &[&str]) -> BTreeSet<SensorId> {
    names.iter().map(|name| SensorId((*name).into())).collect()
}

fn policy(name: &str, mode: Mode) -> SensorPolicy {
    SensorPolicy {
        id: SensorId(name.into()),
        enabled: true,
        modes: BTreeSet::from([mode]),
        allow_open: false,
        auto_bypass: BTreeSet::new(),
        ..SensorPolicy::default()
    }
}

fn policies() -> Vec<SensorPolicy> {
    let mut allowed = policy("allowed-motion", Mode::Away);
    allowed.allow_open = true;
    let mut automatic_open = policy("auto-open", Mode::Away);
    automatic_open.auto_bypass.insert(Mode::Away);
    let mut automatic_closed = policy("auto-closed", Mode::Away);
    automatic_closed.auto_bypass.insert(Mode::Away);
    let mut disabled = policy("disabled", Mode::Away);
    disabled.enabled = false;
    vec![
        policy("blocked-a", Mode::Away),
        policy("blocked-b", Mode::Away),
        allowed,
        automatic_open,
        automatic_closed,
        policy("unconfirmed", Mode::Away),
        policy("offline", Mode::Away),
        disabled,
        policy("night-only", Mode::Night),
    ]
}

fn statuses() -> BTreeMap<SensorId, SensorStatus> {
    [
        ("blocked-a", SensorState::Open, true),
        ("blocked-b", SensorState::Open, true),
        ("allowed-motion", SensorState::Open, true),
        ("auto-open", SensorState::Open, true),
        ("auto-closed", SensorState::Closed, true),
        ("unconfirmed", SensorState::Unknown, false),
        ("offline", SensorState::Unavailable, false),
        ("disabled", SensorState::Open, true),
        ("night-only", SensorState::Open, true),
    ]
    .into_iter()
    .map(|(name, state, confirmed)| (SensorId(name.into()), SensorStatus { state, confirmed }))
    .collect()
}

fn exclusions_fixture() -> Exclusions {
    // C3/C4 never depend on begin_period already computing automatic exclusions.
    Exclusions {
        period: ArmPeriod(9),
        mode: Mode::Away,
        automatic: ids(&["auto-open"]),
        forced: BTreeSet::new(),
    }
}

#[test]
fn coverage_is_derived_per_mode_and_arm_period() {
    let config = policies();
    let readings = statuses();
    let exclusions = exclusions_fixture();
    let view = snapshot(ArmPeriod(9), Mode::Away, &config, &readings, &exclusions);
    let night = snapshot(ArmPeriod(9), Mode::Night, &config, &readings, &exclusions);
    let next = snapshot(ArmPeriod(10), Mode::Away, &config, &readings, &exclusions);
    let automatic = begin_period(ArmPeriod(9), Mode::Away, &config, &readings);
    let night_automatic = begin_period(ArmPeriod(9), Mode::Night, &config, &readings);
    let mut cached = readings.clone();
    let _ = cached.insert(
        SensorId("auto-open".into()),
        SensorStatus {
            state: SensorState::Open,
            confirmed: false,
        },
    );
    let unconfirmed_automatic = begin_period(ArmPeriod(9), Mode::Away, &config, &cached);
    let mut closing = exclusions.clone();
    closing.forced = ids(&["blocked-b"]);
    let finished = end_period(closing);
    assert_eq!(view.covered.len(), 7);
    assert_eq!(
        view.covered,
        ids(&[
            "blocked-a",
            "blocked-b",
            "allowed-motion",
            "auto-open",
            "auto-closed",
            "unconfirmed",
            "offline",
        ])
    );
    assert_eq!(
        view.monitored,
        ids(&[
            "blocked-a",
            "blocked-b",
            "allowed-motion",
            "auto-closed",
            "unconfirmed",
            "offline",
        ])
    );
    assert_eq!(view.blocking, ids(&["blocked-a", "blocked-b"]));
    assert_eq!(view.degraded, ids(&["unconfirmed", "offline"]));
    assert_eq!(view.bypassed, ids(&["auto-open"]));
    assert_eq!(night.covered, ids(&["night-only"]));
    assert_eq!(night.monitored, ids(&["night-only"]));
    assert_eq!(night.blocking, ids(&["night-only"]));
    assert!(night.bypassed.is_empty());
    assert!(night.degraded.is_empty());
    assert!(next.bypassed.is_empty());
    assert!(next.blocking.contains(&SensorId("auto-open".into())));
    assert_eq!(automatic.automatic(), &ids(&["auto-open"]));
    assert!(automatic.forced().is_empty());
    assert!(night_automatic.automatic().is_empty());
    assert!(unconfirmed_automatic.automatic().is_empty());
    assert!(finished.automatic().is_empty());
    assert!(finished.forced().is_empty());
}

#[derive(Debug, PartialEq, Eq)]
struct ProfileCall {
    request: RequestId,
    source: Source,
    permission: Permission,
    mode: Option<Mode>,
    now: TimestampMs,
}

struct TestProfiles {
    result: Authorization,
    calls: RefCell<Vec<ProfileCall>>,
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
        self.calls.borrow_mut().push(ProfileCall {
            request,
            source: source.clone(),
            permission,
            mode,
            now,
        });
        self.result.clone()
    }
}

struct ForceOutcome {
    result: Forced,
    calls: Vec<ProfileCall>,
}

fn force_case(
    source: Source,
    result: Authorization,
    names: &[&str],
    id: RequestId,
) -> ForceOutcome {
    let profiles = TestProfiles {
        result,
        calls: RefCell::new(Vec::new()),
    };
    let request = ForceRequest {
        id,
        source,
        targets: ids(names),
    };
    let result = force(
        exclusions_fixture(),
        &request,
        TimestampMs(5_000),
        &policies(),
        &profiles,
    );
    ForceOutcome {
        result,
        calls: profiles.calls.into_inner(),
    }
}

fn allowed_profile() -> Authorization {
    Authorization::Allowed(ProfileId("test-profile".into()))
}

#[test]
fn forced_exclusions_require_named_authorized_ui() {
    use Authorization::{Invalid, Missing};
    use AuthorizationFailure::Revoked;
    use Source::{Agent, Automation, Ui};
    let valid = force_case(Ui, allowed_profile(), &["blocked-a"], RequestId(20));
    let missing = force_case(Ui, Missing, &["blocked-a"], RequestId(21));
    let invalid = force_case(Ui, Invalid(Revoked), &["blocked-a"], RequestId(22));
    let agent = force_case(Agent, allowed_profile(), &["blocked-a"], RequestId(23));
    let automation = force_case(Automation, allowed_profile(), &["blocked-a"], RequestId(24));
    let empty = force_case(Ui, allowed_profile(), &[], RequestId(25));
    let mut closing = exclusions_fixture();
    closing.forced = ids(&["blocked-a"]);
    let disarmed = end_period(closing);
    assert_eq!(valid.result.exclusions.forced().len(), 1);
    assert_eq!(valid.result.exclusions.forced(), &ids(&["blocked-a"]));
    assert_eq!(valid.result.exclusions.automatic(), &ids(&["auto-open"]));
    assert_eq!(valid.result.verdict, Some(ForceVerdict::Applied));
    for (outcome, verdict) in [
        (&missing, ForceVerdict::CodeRequired),
        (&invalid, ForceVerdict::InvalidCode),
        (&agent, ForceVerdict::SourceDenied),
        (&automation, ForceVerdict::SourceDenied),
        (&empty, ForceVerdict::TargetsRequired),
    ] {
        assert_eq!(outcome.result.exclusions, exclusions_fixture());
        assert_eq!(outcome.result.verdict, Some(verdict));
    }
    for (outcome, id) in [
        (&valid, RequestId(20)),
        (&missing, RequestId(21)),
        (&invalid, RequestId(22)),
    ] {
        assert_eq!(
            outcome.calls,
            [ProfileCall {
                request: id,
                source: Ui,
                permission: Permission::Exclude,
                mode: Some(Mode::Away),
                now: TimestampMs(5_000),
            }]
        );
    }
    assert!(agent.calls.is_empty());
    assert!(automation.calls.is_empty());
    assert!(disarmed.automatic().is_empty());
    assert!(disarmed.forced().is_empty());
}
