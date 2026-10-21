use super::fixtures::*;
use crate::configuration::{AutomationAction, ConfigError, validate, validate_automation};
use crate::coverage::{PointSemantic, SensorState};
use crate::{
    ArmRequest, Authorization, AuthorizationFailure, Event, Mode, Permission, Phase, RequestId,
    Source, Verdict,
};

#[test]
fn configuration_rejects_invalid_points_modes_and_limits() {
    let base = config();
    let known = catalog();
    let mut rows = vec![Row::check(
        "valid",
        "validation",
        validate(&base, &known) == Ok(()),
    )];
    let mut unknown = base.clone();
    unknown.sensors[0].id = id("unknown");
    let mut disabled = base.clone();
    disabled.modes.remove(&Mode::Home);
    let mut too_long = base.clone();
    too_long.sensors[0].entry_s = Some(31);
    let mut siren_limit = base.clone();
    siren_limit.siren_max_s = 181;
    let mut zero_limit = base.clone();
    zero_limit.siren_max_s = 0;
    let mut duplicate = base.clone();
    duplicate.sensors.push(duplicate.sensors[0].clone());
    let configs = [
        ("unknown-point", unknown, ConfigError::UnknownPoint),
        ("disabled-mode", disabled, ConfigError::DisabledMode),
        ("entry-too-long", too_long, ConfigError::EntryTooLong),
        (
            "siren-over-180",
            siren_limit,
            ConfigError::InvalidSirenLimit,
        ),
        ("siren-zero", zero_limit, ConfigError::InvalidSirenLimit),
        ("duplicate-point", duplicate, ConfigError::DuplicatePoint),
    ];
    for (name, input, error) in configs {
        rows.push(Row::check(
            name,
            "validation",
            validate(&input, &known) == Err(error),
        ));
    }
    let mut unsupported = known.clone();
    unsupported.get_mut("sensor-a").expect("fixture").semantic = PointSemantic::Other;
    let mut unreadable = known.clone();
    let point = unreadable.get_mut("sensor-a").expect("fixture");
    point.semantic = PointSemantic::Lock;
    point.readable = false;
    let mut readonly = known.clone();
    readonly.get_mut("siren-a").expect("fixture").writable = false;
    let mut composite = known.clone();
    composite.get_mut("siren-a").expect("fixture").binary = false;
    let mut no_private_input = known.clone();
    no_private_input
        .get_mut("keypad-a")
        .expect("fixture")
        .private_keypad_input = false;
    for (name, points, error) in [
        (
            "unsupported-sensor",
            unsupported,
            ConfigError::UnsupportedSensor,
        ),
        (
            "unreadable-lock",
            unreadable,
            ConfigError::UnsupportedSensor,
        ),
        ("readonly-siren", readonly, ConfigError::InvalidSiren),
        ("composite-siren", composite, ConfigError::InvalidSiren),
        (
            "missing-private-keypad-input",
            no_private_input,
            ConfigError::MissingPrivateInput,
        ),
    ] {
        rows.push(Row::check(
            name,
            "validation",
            validate(&base, &points) == Err(error),
        ));
    }
    for (name, action, result) in [
        ("automation-arm", AutomationAction::Arm(Mode::Away), Ok(())),
        (
            "automation-disarm",
            AutomationAction::Disarm,
            Err(ConfigError::ForbiddenAutomation),
        ),
        (
            "automation-skip",
            AutomationAction::SkipDelay,
            Err(ConfigError::ForbiddenAutomation),
        ),
        (
            "automation-force",
            AutomationAction::Force,
            Err(ConfigError::ForbiddenAutomation),
        ),
    ] {
        rows.push(Row::check(
            name,
            "automation_validation",
            validate_automation(action) == result,
        ));
    }
    assert!(finish("E1", rows));
}

#[test]
fn sources_and_current_profile_permissions_gate_every_request() {
    let mut cases = Vec::new();
    for (name, source) in [
        ("ui-no-code", Source::Ui),
        ("household-no-code", Source::Household),
        ("keypad-no-code", Source::Keypad("keypad-a".into())),
        ("automation-arm", Source::Automation),
    ] {
        cases.push(
            Case::new(
                name,
                state(Phase::Disarmed),
                arm(Mode::Away, source),
                Expect::phase(Phase::Armed(Mode::Away))
                    .verdict(Verdict::Armed(Mode::Away))
                    .generation(8)
                    .messages(vec![Message::Armed(Mode::Away)])
                    .permission(Permission::Arm),
            )
            .auth(Authorization::Missing),
        );
    }
    for (name, reason) in [
        ("unknown-code", AuthorizationFailure::Unknown),
        ("revoked", AuthorizationFailure::Revoked),
        ("outside-validity", AuthorizationFailure::OutsideValidity),
        ("permission-denied", AuthorizationFailure::PermissionDenied),
        ("mode-denied", AuthorizationFailure::ModeDenied),
        ("source-locked", AuthorizationFailure::SourceLocked),
    ] {
        cases.push(
            Case::new(
                name,
                state(Phase::Disarmed),
                arm(Mode::Away, Source::Ui),
                Expect::phase(Phase::Disarmed)
                    .verdict(Verdict::InvalidCode)
                    .same()
                    .permission(Permission::Arm),
            )
            .auth(Authorization::Invalid(reason)),
        );
    }
    let mut retained = Expect::phase(Phase::Disarmed)
        .verdict(Verdict::HumanApprovalRequired)
        .same();
    retained.retained = true;
    retained.no_auth = true;
    cases.push(Case::new(
        "agent-arm-held",
        state(Phase::Disarmed),
        arm(Mode::Away, Source::Agent),
        retained,
    ));
    for (name, source) in [
        ("agent-never-disarms", Source::Agent),
        ("automation-never-disarms", Source::Automation),
    ] {
        let mut expected = Expect::phase(Phase::Armed(Mode::Away))
            .verdict(Verdict::SourceDenied)
            .same();
        expected.no_auth = true;
        cases.push(Case::new(
            name,
            state(Phase::Armed(Mode::Away)),
            disarm(source),
            expected,
        ));
    }
    for (name, source) in [
        ("ui-disarm", Source::Ui),
        ("household-disarm", Source::Household),
        ("keypad-disarm", Source::Keypad("keypad-a".into())),
    ] {
        cases.push(Case::new(
            name,
            state(Phase::Armed(Mode::Away)),
            disarm(source),
            Expect::phase(Phase::Disarmed)
                .verdict(Verdict::Disarmed)
                .generation(8)
                .messages(vec![Message::Other(crate::Notice::Disarmed)])
                .permission(Permission::Disarm),
        ));
    }
    cases.push(
        Case::new(
            "human-session-still-needs-code",
            state(Phase::Armed(Mode::Away)),
            disarm(Source::Ui),
            Expect::phase(Phase::Armed(Mode::Away))
                .verdict(Verdict::CodeRequired)
                .same()
                .permission(Permission::Disarm),
        )
        .auth(Authorization::Missing),
    );
    cases.push(
        Case::new(
            "revoked-at-processing-time",
            state(Phase::Armed(Mode::Away)),
            disarm(Source::Ui),
            Expect::phase(Phase::Armed(Mode::Away))
                .verdict(Verdict::InvalidCode)
                .same()
                .permission(Permission::Disarm),
        )
        .at(12_000)
        .override_permission(
            Permission::Disarm,
            Authorization::Invalid(AuthorizationFailure::Revoked),
        ),
    );
    let mut blocked = state(Phase::Disarmed);
    value(&mut blocked, "sensor-a", SensorState::Open, true);
    let force = Event::Arm(ArmRequest {
        id: RequestId(3),
        source: Source::Ui,
        mode: Mode::Away,
        skip_exit: false,
        force: ids(&["sensor-a"]),
    });
    cases.push(
        Case::new(
            "exclude-right-required",
            blocked,
            force,
            Expect::phase(Phase::Disarmed)
                .verdict(Verdict::InvalidCode)
                .same()
                .permission(Permission::Exclude),
        )
        .override_permission(
            Permission::Exclude,
            Authorization::Invalid(AuthorizationFailure::PermissionDenied),
        ),
    );
    let skip = Event::Arm(ArmRequest {
        id: RequestId(4),
        source: Source::Ui,
        mode: Mode::Home,
        skip_exit: true,
        force: std::collections::BTreeSet::new(),
    });
    cases.push(
        Case::new(
            "skip-right-required",
            state(Phase::Disarmed),
            skip,
            Expect::phase(Phase::Disarmed)
                .verdict(Verdict::InvalidCode)
                .same()
                .permission(Permission::SkipDelay),
        )
        .override_permission(
            Permission::SkipDelay,
            Authorization::Invalid(AuthorizationFailure::PermissionDenied),
        ),
    );
    cases.push(
        Case::new(
            "already-armed",
            state(Phase::Armed(Mode::Away)),
            arm(Mode::Away, Source::Household),
            Expect::phase(Phase::Armed(Mode::Away))
                .verdict(Verdict::AlreadyArmed(Mode::Away))
                .same(),
        )
        .auth(Authorization::Missing),
    );
    cases.push(Case::new(
        "already-disarmed-with-code",
        state(Phase::Disarmed),
        disarm(Source::Ui),
        Expect::phase(Phase::Disarmed)
            .verdict(Verdict::AlreadyDisarmed)
            .same()
            .permission(Permission::Disarm),
    ));
    let rows = evaluate(cases);
    assert!(finish("E2", rows));
}
