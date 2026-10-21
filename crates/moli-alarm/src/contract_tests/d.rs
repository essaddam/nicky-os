use super::fixtures::*;
use crate::context::ReturnFrame;
use crate::coverage::{ArmPeriod, SensorState};
use crate::{
    AfterTrigger, ArmRequest, Authorization, AuthorizationFailure, Cause, Event, Mode, Notice,
    Permission, Phase, RequestId, Source, Verdict,
};

#[test]
fn coverage_blocks_arming_and_mode_changes_roll_back() {
    let mut blocked = state(Phase::Disarmed);
    value(&mut blocked, "sensor-a", SensorState::Open, true);
    let mut expect = Expect::phase(Phase::Disarmed)
        .verdict(Verdict::OpenSensors)
        .same();
    expect.blocking = Some(ids(&["sensor-a"]));
    let mut failed = Expect::phase(Phase::Disarmed)
        .generation(8)
        .messages(vec![Message::Other(Notice::ArmFailed(vec![id(
            "sensor-a",
        )]))]);
    failed.exit = Some(None);
    failed.blocking = Some(ids(&["sensor-a"]));
    let mut exiting = state(Phase::Arming(Mode::Away));
    value(&mut exiting, "sensor-a", SensorState::Open, true);
    let mut change = state(Phase::Arming(Mode::Home));
    change.exit_timer = Some(token(7, 61_000));
    change.details.return_to = Some(ReturnFrame {
        phase: Phase::Armed(Mode::Away),
        period: ArmPeriod(8),
        forced: ids(&["sensor-a"]),
        ..ReturnFrame::default()
    });
    value(&mut change, "sensor-a", SensorState::Open, true);
    let mut rollback = Expect::phase(Phase::Armed(Mode::Away))
        .generation(8)
        .messages(vec![Message::Other(Notice::ArmFailed(vec![id(
            "sensor-a",
        )]))]);
    rollback.exit = Some(None);
    rollback.forced = Some(ids(&["sensor-a"]));
    let mut accepted = Expect::phase(Phase::Arming(Mode::Home))
        .verdict(Verdict::AcceptedArming(Mode::Home))
        .generation(8)
        .permission(Permission::ChangeMode);
    accepted.exit = Some(Some(token(8, 61_000)));
    let mut automatic_config = config();
    automatic_config.sensors[0].auto_bypass.insert(Mode::Away);
    let mut automatic = Expect::phase(Phase::Armed(Mode::Away))
        .verdict(Verdict::Armed(Mode::Away))
        .generation(8)
        .messages(vec![Message::Armed(Mode::Away)]);
    automatic.automatic = Some(ids(&["sensor-a"]));
    let force_event = Event::Arm(ArmRequest {
        id: RequestId(3),
        mode: Mode::Away,
        source: Source::Ui,
        skip_exit: false,
        force: ids(&["sensor-a"]),
    });
    let mut forced = Expect::phase(Phase::Armed(Mode::Away))
        .verdict(Verdict::Armed(Mode::Away))
        .generation(8)
        .messages(vec![Message::Armed(Mode::Away)])
        .permission(Permission::Exclude);
    forced.forced = Some(ids(&["sensor-a"]));
    let skip = Event::Arm(ArmRequest {
        id: RequestId(4),
        mode: Mode::Home,
        source: Source::Ui,
        skip_exit: true,
        force: std::collections::BTreeSet::new(),
    });
    let mut absent_mode = config();
    absent_mode.modes.remove(&Mode::Night);
    let rows = evaluate(vec![
        Case::new(
            "blocked-immediate",
            blocked.clone(),
            arm(Mode::Away, Source::Household),
            expect,
        )
        .auth(Authorization::Missing),
        Case::new(
            "blocked-at-exit",
            exiting,
            Event::ExitExpired(token(7, 31_000)),
            failed,
        )
        .at(31_000),
        Case::new(
            "mode-change-rollback",
            change,
            Event::ExitExpired(token(7, 61_000)),
            rollback,
        )
        .at(61_000),
        Case::new(
            "mode-change-code-required",
            state(Phase::Armed(Mode::Away)),
            arm(Mode::Home, Source::Ui),
            Expect::phase(Phase::Armed(Mode::Away))
                .verdict(Verdict::CodeRequired)
                .same()
                .permission(Permission::ChangeMode),
        )
        .auth(Authorization::Missing),
        Case::new(
            "mode-change-accepted",
            state(Phase::Armed(Mode::Away)),
            arm(Mode::Home, Source::Ui),
            accepted,
        ),
        Case::new(
            "automatic-exclusion",
            blocked.clone(),
            arm(Mode::Away, Source::Household),
            automatic,
        )
        .config(automatic_config)
        .auth(Authorization::Missing),
        Case::new("named-force", blocked, force_event, forced),
        Case::new(
            "skip-with-code",
            state(Phase::Disarmed),
            skip.clone(),
            Expect::phase(Phase::Armed(Mode::Home))
                .verdict(Verdict::Armed(Mode::Home))
                .generation(8)
                .messages(vec![Message::Armed(Mode::Home)])
                .permission(Permission::SkipDelay),
        ),
        Case::new(
            "skip-needs-code",
            state(Phase::Disarmed),
            skip,
            Expect::phase(Phase::Disarmed)
                .verdict(Verdict::CodeRequired)
                .same()
                .permission(Permission::SkipDelay),
        )
        .auth(Authorization::Missing),
        Case::new(
            "inactive-mode",
            state(Phase::Disarmed),
            arm(Mode::Night, Source::Household),
            Expect::phase(Phase::Disarmed)
                .verdict(Verdict::NotAllowed)
                .same(),
        )
        .config(absent_mode)
        .auth(Authorization::Missing),
    ]);
    assert!(finish("D1", rows));
}

#[test]
fn entry_delays_only_shorten_and_refusals_never_extend_them() {
    let mut entered = Expect::phase(Phase::Pending(Mode::Away))
        .generation(8)
        .messages(vec![Message::Other(Notice::Pending(Mode::Away))]);
    entered.entry = Some(Some(token(8, 31_000)));
    entered.cause = Some(Cause::Sensor(id("sensor-a")));
    let mut shorter = Expect::phase(Phase::Pending(Mode::Away)).generation(8);
    shorter.entry = Some(Some(token(8, 10_000)));
    let mut no_longer = Expect::phase(Phase::Pending(Mode::Away)).generation(7);
    no_longer.entry = Some(Some(token(7, 31_000)));
    let mut close = Expect::phase(Phase::Pending(Mode::Away)).generation(7);
    close.entry = Some(Some(token(7, 31_000)));
    let mut denied = Expect::phase(Phase::Pending(Mode::Away))
        .verdict(Verdict::InvalidCode)
        .same()
        .permission(Permission::Disarm);
    denied.entry = Some(Some(token(7, 31_000)));
    let mut immediate = Expect::phase(Phase::Triggered(Some(Mode::Away)))
        .generation(8)
        .commands(true)
        .messages(vec![Message::Other(Notice::Triggered(Cause::Sensor(id(
            "sensor-b",
        ))))]);
    immediate.entry = Some(None);
    immediate.siren = Some(Some(token(8, 181_000)));
    immediate.trigger = Some(Some(token(8, 601_000)));
    immediate.siren_on = Some(true);
    let mut expired = Expect::phase(Phase::Triggered(Some(Mode::Away)))
        .generation(8)
        .commands(true)
        .messages(vec![Message::Other(Notice::Triggered(Cause::Sensor(id(
            "sensor-a",
        ))))]);
    expired.entry = Some(None);
    expired.siren = Some(Some(token(8, 211_000)));
    expired.trigger = Some(Some(token(8, 631_000)));
    let mut disarmed = Expect::phase(Phase::Disarmed)
        .verdict(Verdict::Disarmed)
        .generation(8)
        .permission(Permission::Disarm);
    disarmed.entry = Some(None);
    let rows = evaluate(vec![
        Case::new(
            "begin-entry",
            state(Phase::Armed(Mode::Away)),
            Event::Open(id("sensor-a")),
            entered,
        ),
        Case::new(
            "shorter-sensor",
            state(Phase::Pending(Mode::Away)),
            Event::Open(id("short")),
            shorter,
        )
        .at(5_000),
        Case::new(
            "never-lengthen",
            state(Phase::Pending(Mode::Away)),
            Event::Open(id("short")),
            no_longer,
        )
        .at(29_000),
        Case::new(
            "close-does-not-cancel",
            state(Phase::Pending(Mode::Away)),
            Event::Close(id("sensor-a")),
            close,
        )
        .at(5_000),
        Case::new(
            "bad-code-keeps-deadline",
            state(Phase::Pending(Mode::Away)),
            disarm(Source::Ui),
            denied,
        )
        .at(5_000)
        .auth(Authorization::Invalid(AuthorizationFailure::Revoked)),
        Case::new(
            "immediate-sensor",
            state(Phase::Pending(Mode::Away)),
            Event::Open(id("sensor-b")),
            immediate,
        ),
        Case::new(
            "entry-expired",
            state(Phase::Pending(Mode::Away)),
            Event::EntryExpired(token(7, 31_000)),
            expired,
        )
        .at(31_000),
        Case::new(
            "stale-entry-timer",
            state(Phase::Pending(Mode::Away)),
            Event::EntryExpired(token(6, 31_000)),
            Expect::phase(Phase::Pending(Mode::Away)).ignored(),
        )
        .at(31_000),
        Case::new(
            "valid-disarm",
            state(Phase::Pending(Mode::Away)),
            disarm(Source::Ui),
            disarmed,
        )
        .at(5_000),
        Case::new(
            "uncovered-point",
            state(Phase::Armed(Mode::Away)),
            Event::Open(id("unknown")),
            Expect::phase(Phase::Armed(Mode::Away)).ignored(),
        ),
    ]);
    assert!(finish("D2", rows));
}

#[test]
fn sirens_stop_at_the_bound_and_on_every_triggered_exit() {
    let mut stopped = Expect::phase(Phase::Triggered(Some(Mode::Away)))
        .generation(8)
        .commands(false);
    stopped.siren = Some(None);
    stopped.siren_on = Some(false);
    stopped.trigger = Some(Some(token(8, 601_000)));
    let mut disarmed = Expect::phase(Phase::Disarmed)
        .verdict(Verdict::Disarmed)
        .generation(8)
        .commands(false)
        .messages(vec![Message::Other(Notice::Ended(None))])
        .permission(Permission::Disarm);
    disarmed.siren_on = Some(false);
    disarmed.siren = Some(None);
    disarmed.trigger = Some(None);
    let mut rearmed = Expect::phase(Phase::Armed(Mode::Away))
        .generation(8)
        .commands(false)
        .messages(vec![Message::Other(Notice::Ended(Some(Mode::Away)))]);
    rearmed.siren_on = Some(false);
    rearmed.trigger = Some(None);
    rearmed.siren = Some(None);
    let saved = crate::SavedState {
        phase: Phase::Armed(Mode::Home),
        generation: 7,
        ..crate::SavedState::default()
    };
    let mut short_config = config();
    short_config.siren_max_s = 5;
    let mut short_siren = Expect::phase(Phase::Triggered(Some(Mode::Away)))
        .generation(8)
        .commands(true)
        .messages(vec![Message::Other(Notice::Triggered(Cause::Sensor(id(
            "sensor-b",
        ))))]);
    short_siren.siren = Some(Some(token(8, 6_000)));
    short_siren.siren_on = Some(true);
    let rows = evaluate(vec![
        Case::new(
            "configured-five-second-bound",
            state(Phase::Armed(Mode::Away)),
            Event::Open(id("sensor-b")),
            short_siren,
        )
        .config(short_config),
        Case::new(
            "siren-180s",
            state(Phase::Triggered(Some(Mode::Away))),
            Event::SirenExpired(token(7, 181_000)),
            stopped,
        )
        .at(181_000),
        Case::new(
            "disarm-stops",
            state(Phase::Triggered(Some(Mode::Away))),
            disarm(Source::Ui),
            disarmed,
        )
        .at(5_000),
        Case::new(
            "duration-stops",
            state(Phase::Triggered(Some(Mode::Away))),
            Event::TriggerExpired(token(7, 601_000)),
            rearmed,
        )
        .at(601_000),
        Case::new(
            "restore-exit-stops",
            state(Phase::Triggered(Some(Mode::Away))),
            restore(Some(saved), true),
            Expect::phase(Phase::Armed(Mode::Home))
                .generation(8)
                .commands(false)
                .messages(vec![Message::Other(Notice::Ended(Some(Mode::Home)))]),
        )
        .at(10_000),
        Case::new(
            "stale-siren-timer",
            state(Phase::Triggered(Some(Mode::Away))),
            Event::SirenExpired(token(6, 181_000)),
            Expect::phase(Phase::Triggered(Some(Mode::Away))).ignored(),
        )
        .at(181_000),
    ]);
    assert!(finish("D3", rows));
}

#[test]
fn episodes_restart_once_per_new_sensor_and_rearm_explicitly() {
    let mut silent = state(Phase::Triggered(Some(Mode::Away)));
    silent.details.siren_on = false;
    silent.details.siren_timer = None;
    let mut fresh = Expect::phase(Phase::Triggered(Some(Mode::Away)))
        .generation(8)
        .commands(true);
    fresh.siren_on = Some(true);
    fresh.siren = Some(Some(token(8, 380_000)));
    fresh.trigger = Some(Some(token(8, 601_000)));
    fresh.seen = Some(ids(&["sensor-a", "sensor-b"]));
    let mut repeat = Expect::phase(Phase::Triggered(Some(Mode::Away))).generation(7);
    repeat.siren_on = Some(false);
    repeat.siren = Some(None);
    repeat.seen = Some(ids(&["sensor-a"]));
    let mut recorded = Expect::phase(Phase::Triggered(Some(Mode::Away))).generation(8);
    recorded.seen = Some(ids(&["sensor-a", "sensor-b"]));
    recorded.siren = Some(Some(token(8, 181_000)));
    recorded.trigger = Some(Some(token(8, 601_000)));
    let mut opened = state(Phase::Triggered(Some(Mode::Away)));
    value(&mut opened, "sensor-a", SensorState::Open, true);
    value(&mut opened, "sensor-b", SensorState::Open, true);
    let mut excluding = Expect::phase(Phase::Armed(Mode::Away))
        .generation(8)
        .commands(false)
        .messages(vec![Message::Other(Notice::Ended(Some(Mode::Away)))]);
    excluding.automatic = Some(ids(&["sensor-a", "sensor-b"]));
    let mut fallback_config = config();
    fallback_config.after_trigger = AfterTrigger::RearmOrDisarm;
    let mut indefinite_config = config();
    indefinite_config
        .modes
        .get_mut(&Mode::Away)
        .expect("fixture mode")
        .trigger_s = 0;
    let mut indefinite = state(Phase::Triggered(Some(Mode::Away)));
    indefinite.details.trigger_timer = None;
    let mut limited = Expect::phase(Phase::Triggered(Some(Mode::Away)))
        .generation(8)
        .commands(false);
    limited.siren_on = Some(false);
    limited.siren = Some(None);
    limited.trigger = Some(None);
    let rows = evaluate(vec![
        Case::new(
            "new-sensor-one-cycle",
            silent.clone(),
            Event::Open(id("sensor-b")),
            fresh,
        )
        .at(200_000),
        Case::new(
            "seen-sensor-no-cycle",
            silent,
            Event::Open(id("sensor-a")),
            repeat,
        )
        .at(200_000),
        Case::new(
            "seen-while-sounding",
            state(Phase::Triggered(Some(Mode::Away))),
            Event::Open(id("sensor-b")),
            recorded,
        )
        .at(10_000),
        Case::new(
            "rearm-excludes-open",
            opened.clone(),
            Event::TriggerExpired(token(7, 601_000)),
            excluding,
        )
        .at(601_000),
        Case::new(
            "rearm-or-disarm",
            opened,
            Event::TriggerExpired(token(7, 601_000)),
            Expect::phase(Phase::Disarmed)
                .generation(8)
                .commands(false)
                .messages(vec![Message::Other(Notice::Ended(None))]),
        )
        .at(601_000)
        .config(fallback_config),
        Case::new(
            "indefinite-still-stops-siren",
            indefinite.clone(),
            Event::SirenExpired(token(7, 181_000)),
            limited,
        )
        .at(181_000)
        .config(indefinite_config.clone()),
        Case::new(
            "indefinite-no-trigger-expiry",
            indefinite,
            Event::TriggerExpired(token(7, 601_000)),
            Expect::phase(Phase::Triggered(Some(Mode::Away))).ignored(),
        )
        .config(indefinite_config),
    ]);
    assert!(finish("D4", rows));
}
