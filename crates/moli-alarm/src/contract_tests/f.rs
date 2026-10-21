use super::fixtures::*;
use crate::coverage::{Observation, ObservationOrigin, SensorState};
use crate::{ActionPolicy, Cause, Event, Mode, Notice, Phase, Source, TimestampMs, Verdict};

#[test]
fn restore_clamps_deadlines_and_never_replays_requests() {
    let mut cfg = config();
    cfg.modes.get_mut(&Mode::Away).expect("fixture").exit_s = 30;
    let mut cases = Vec::new();
    let mut initial = Expect::phase(Phase::Disarmed).generation(0);
    initial.startup_at = Some(10_000);
    cases.push(
        Case::new(
            "first-install",
            crate::State::default(),
            restore(None, false),
            initial,
        )
        .at(10_000)
        .config(cfg.clone()),
    );
    cases.push(
        Case::new(
            "configured-state-lost",
            crate::State::default(),
            restore(None, true),
            Expect::phase(Phase::Disarmed)
                .generation(1)
                .messages(vec![Message::Other(Notice::StateLost)]),
        )
        .at(10_000)
        .config(cfg.clone()),
    );
    for (name, phase) in [
        ("disarmed", Phase::Disarmed),
        ("arming-future", Phase::Arming(Mode::Away)),
        ("armed", Phase::Armed(Mode::Away)),
        ("pending-future", Phase::Pending(Mode::Away)),
        ("triggered-future", Phase::Triggered(Some(Mode::Away))),
    ] {
        let mut expected = Expect::phase(phase).generation(8);
        expected.startup_at = Some(10_000);
        expected.cold_observations = true;
        match phase {
            Phase::Arming(_) => expected.exit = Some(Some(token(8, 31_000))),
            Phase::Pending(_) => expected.entry = Some(Some(token(8, 31_000))),
            Phase::Triggered(_) => {
                expected.commands = commands(true);
                expected.siren_on = Some(true);
                expected.siren = Some(Some(token(8, 181_000)));
                expected.trigger = Some(Some(token(8, 601_000)));
                expected.seen = Some(ids(&["sensor-a"]));
            }
            _ => {}
        }
        cases.push(
            Case::new(
                name,
                crate::State::default(),
                restore(Some(saved(phase)), true),
                expected,
            )
            .at(10_000)
            .config(cfg.clone()),
        );
    }
    let mut entry_elapsed = Expect::phase(Phase::Triggered(Some(Mode::Away)))
        .generation(8)
        .commands(true)
        .messages(vec![Message::Other(Notice::Triggered(
            Cause::EntryExpiredDuringStop,
        ))]);
    entry_elapsed.entry = Some(None);
    entry_elapsed.siren = Some(Some(token(8, 220_000)));
    entry_elapsed.trigger = Some(Some(token(8, 640_000)));
    entry_elapsed.cause = Some(Cause::EntryExpiredDuringStop);
    cases.push(
        Case::new(
            "pending-expired-during-stop",
            crate::State::default(),
            restore(Some(saved(Phase::Pending(Mode::Away))), true),
            entry_elapsed,
        )
        .at(40_000)
        .config(cfg.clone()),
    );
    let mut exit_request = restore(Some(saved(Phase::Arming(Mode::Away))), true);
    if let Event::Restore(request) = &mut exit_request {
        for name in ["sensor-a", "sensor-b", "short"] {
            request.cached.insert(id(name), SensorState::Closed);
        }
    }
    let mut exit_elapsed =
        Expect::phase(Phase::Disarmed)
            .generation(8)
            .messages(vec![Message::Other(Notice::ArmFailed(vec![
                id("sensor-a"),
                id("sensor-b"),
                id("short"),
            ]))]);
    exit_elapsed.exit = Some(None);
    exit_elapsed.blocking = Some(ids(&["sensor-a", "sensor-b", "short"]));
    cases.push(
        Case::new(
            "expired-exit-cache-is-not-proof",
            crate::State::default(),
            exit_request,
            exit_elapsed,
        )
        .at(40_000)
        .config(cfg.clone()),
    );
    let mut siren_elapsed = Expect::phase(Phase::Triggered(Some(Mode::Away)))
        .generation(8)
        .commands(false);
    siren_elapsed.siren_on = Some(false);
    siren_elapsed.siren = Some(None);
    siren_elapsed.trigger = Some(Some(token(8, 601_000)));
    cases.push(
        Case::new(
            "siren-expired-stop-once",
            crate::State::default(),
            restore(Some(saved(Phase::Triggered(Some(Mode::Away)))), true),
            siren_elapsed,
        )
        .at(200_000)
        .config(cfg.clone()),
    );
    cases.push(
        Case::new(
            "episode-expired",
            crate::State::default(),
            restore(Some(saved(Phase::Triggered(Some(Mode::Away)))), true),
            Expect::phase(Phase::Armed(Mode::Away))
                .generation(8)
                .commands(false)
                .messages(vec![Message::Other(Notice::Ended(Some(Mode::Away)))]),
        )
        .at(700_000)
        .config(cfg.clone()),
    );
    let mut backwards = saved(Phase::Pending(Mode::Away));
    backwards.entry_timer = Some(token(7, 1_000_000));
    backwards.saved_at = TimestampMs(999_000);
    let mut clamped = Expect::phase(Phase::Pending(Mode::Away)).generation(8);
    clamped.entry = Some(Some(token(8, 40_000)));
    cases.push(
        Case::new(
            "backward-clock-bounded",
            crate::State::default(),
            restore(Some(backwards), true),
            clamped,
        )
        .at(10_000)
        .config(cfg),
    );
    let mut cached = state(Phase::Armed(Mode::Away));
    cached.details.startup_at = TimestampMs(10_000);
    value(&mut cached, "sensor-a", SensorState::Open, false);
    let fresh = Event::Observe {
        sensor: id("sensor-a"),
        observation: Observation {
            state: SensorState::Open,
            origin: ObservationOrigin::Fresh,
            observed_at: TimestampMs(10_001),
        },
    };
    let mut first_open = Expect::phase(Phase::Pending(Mode::Away))
        .generation(8)
        .messages(vec![Message::Other(Notice::Pending(Mode::Away))]);
    first_open.entry = Some(Some(token(8, 40_001)));
    cases.push(Case::new("first-fresh-after-restore", cached, fresh, first_open).at(10_001));
    let mut rows = evaluate(cases);
    let mut input = state(Phase::Triggered(Some(Mode::Away)));
    input.details.seen = ids(&["sensor-a", "sensor-b"]);
    input.details.exclusions.as_mut().expect("fixture").forced = ids(&["sensor-b"]);
    let exported = input.saved(TimestampMs(12_000));
    rows.push(Row {
        name: "export-no-request-replay".into(),
        checks: vec![
            (
                "phase",
                exported.phase == Phase::Triggered(Some(Mode::Away)),
            ),
            ("generation", exported.generation == 7),
            ("seen", exported.seen == ids(&["sensor-a", "sensor-b"])),
            ("forced", exported.forced == ids(&["sensor-b"])),
            ("siren", exported.siren_timer == Some(token(7, 181_000))),
            ("saved_at", exported.saved_at == TimestampMs(12_000)),
        ],
    });
    assert!(finish("F1", rows));
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    Arm,
    Disarm,
    Exit,
    Entry,
    Siren,
    Trigger,
    Open,
    Close,
    Observe,
    Unavailable,
    Available,
    Tamper,
    Panic,
    Skip,
    StableClose,
    Restore,
}
const KINDS: [Kind; 16] = [
    Kind::Arm,
    Kind::Disarm,
    Kind::Exit,
    Kind::Entry,
    Kind::Siren,
    Kind::Trigger,
    Kind::Open,
    Kind::Close,
    Kind::Observe,
    Kind::Unavailable,
    Kind::Available,
    Kind::Tamper,
    Kind::Panic,
    Kind::Skip,
    Kind::StableClose,
    Kind::Restore,
];

fn event_kind(event: &Event) -> Kind {
    match event {
        Event::Arm(_) => Kind::Arm,
        Event::Disarm(_) => Kind::Disarm,
        Event::ExitExpired(_) => Kind::Exit,
        Event::EntryExpired(_) => Kind::Entry,
        Event::SirenExpired(_) => Kind::Siren,
        Event::TriggerExpired(_) => Kind::Trigger,
        Event::Open(_) => Kind::Open,
        Event::Close(_) => Kind::Close,
        Event::Observe { .. } => Kind::Observe,
        Event::Unavailable(_) => Kind::Unavailable,
        Event::Available(_) => Kind::Available,
        Event::Tamper(_) => Kind::Tamper,
        Event::Panic(_) => Kind::Panic,
        Event::SkipDelay(_) => Kind::Skip,
        Event::ArmOnClose { .. } => Kind::StableClose,
        Event::Restore(_) => Kind::Restore,
    }
}

fn matrix_config() -> crate::Config {
    let mut result = config();
    for mode in result.modes.values_mut() {
        mode.exit_s = 30;
        mode.entry_s = 30;
        mode.trigger_s = 600;
    }
    result
}

fn matrix_input(phase: Phase, kind: Kind) -> (crate::State, Event, u64, crate::Config) {
    let mut initial = state(phase);
    let mut cfg = matrix_config();
    if phase == Phase::Triggered(None) {
        cfg.sensors[0].always_on = true;
    }
    if matches!(phase, Phase::Arming(_)) {
        initial
            .details
            .close_timers
            .insert(id("sensor-a"), token(7, 6_000));
    }
    let (event, now) = match kind {
        Kind::Arm => (arm(Mode::Away, Source::Ui), 10_000),
        Kind::Disarm => (disarm(Source::Ui), 10_000),
        Kind::Exit => (Event::ExitExpired(token(7, 31_000)), 31_000),
        Kind::Entry => (Event::EntryExpired(token(7, 31_000)), 31_000),
        Kind::Siren => (Event::SirenExpired(token(7, 181_000)), 181_000),
        Kind::Trigger => (Event::TriggerExpired(token(7, 601_000)), 601_000),
        Kind::Open => (Event::Open(id("sensor-a")), 10_000),
        Kind::Close => (Event::Close(id("sensor-a")), 10_000),
        Kind::Observe => (
            Event::Observe {
                sensor: id("sensor-a"),
                observation: Observation {
                    state: SensorState::Open,
                    origin: ObservationOrigin::Fresh,
                    observed_at: TimestampMs(10_000),
                },
            },
            10_000,
        ),
        Kind::Unavailable => (Event::Unavailable(id("sensor-a")), 10_000),
        Kind::Available => (Event::Available(id("sensor-a")), 10_000),
        Kind::Tamper => (Event::Tamper(id("sensor-a")), 10_000),
        Kind::Panic => (Event::Panic(Source::Keypad("keypad-a".into())), 10_000),
        Kind::Skip => (
            Event::SkipDelay(crate::DisarmRequest {
                id: crate::RequestId(9),
                source: Source::Ui,
            }),
            10_000,
        ),
        Kind::StableClose => (
            Event::ArmOnClose {
                sensor: id("sensor-a"),
                token: token(7, 6_000),
            },
            6_000,
        ),
        Kind::Restore => (restore(Some(saved(phase)), true), 10_000),
    };
    (initial, event, now, cfg)
}

// Declarative table for the canonical fixtures above. D/E rows cover the guards
// (immediate points, short delays, refusal, policy overrides) independently.
fn matrix_expected(phase: Phase, kind: Kind) -> Expect {
    let mut result = Expect::phase(phase);
    match (phase, kind) {
        (Phase::Disarmed | Phase::Armed(Mode::Home | Mode::Night | Mode::Vacation), Kind::Arm) => {
            result = Expect::phase(Phase::Arming(Mode::Away))
                .verdict(Verdict::AcceptedArming(Mode::Away))
                .generation(8);
            result.exit = Some(Some(token(8, 40_000)));
        }
        (Phase::Arming(Mode::Away), Kind::Arm) => {
            result.verdict = Some(Verdict::AlreadyArming(Mode::Away));
        }
        (Phase::Armed(Mode::Away), Kind::Arm) => {
            result.verdict = Some(Verdict::AlreadyArmed(Mode::Away));
        }
        (Phase::Arming(_) | Phase::Pending(_) | Phase::Triggered(_), Kind::Arm) => {
            result.verdict = Some(Verdict::Busy);
        }
        (Phase::Disarmed, Kind::Disarm) => result.verdict = Some(Verdict::AlreadyDisarmed),
        (
            Phase::Arming(_) | Phase::Armed(_) | Phase::Pending(_) | Phase::Triggered(_),
            Kind::Disarm,
        ) => {
            result = Expect::phase(Phase::Disarmed)
                .verdict(Verdict::Disarmed)
                .generation(8);
            match phase {
                Phase::Armed(_) => result.messages = vec![Message::Other(Notice::Disarmed)],
                Phase::Triggered(_) => {
                    result.commands = commands(false);
                    result.messages = vec![Message::Other(Notice::Ended(None))];
                }
                _ => {}
            }
        }
        (Phase::Arming(mode), Kind::Exit | Kind::Skip | Kind::StableClose) => {
            result = Expect::phase(Phase::Armed(mode))
                .generation(8)
                .messages(vec![Message::Armed(mode)]);
            if matches!(kind, Kind::Skip) {
                result.verdict = Some(Verdict::SkippedDelay);
            }
        }
        (Phase::Armed(mode), Kind::Open | Kind::Observe) => {
            result = Expect::phase(Phase::Pending(mode))
                .generation(8)
                .messages(vec![Message::Other(Notice::Pending(mode))]);
            result.entry = Some(Some(token(8, 40_000)));
        }
        (Phase::Pending(mode), Kind::Entry | Kind::Skip) => {
            result = Expect::phase(Phase::Triggered(Some(mode)))
                .generation(8)
                .commands(true)
                .messages(vec![Message::Other(Notice::Triggered(Cause::Sensor(id(
                    "sensor-a",
                ))))]);
            if matches!(kind, Kind::Skip) {
                result.verdict = Some(Verdict::SkippedDelay);
            }
        }
        (Phase::Disarmed, Kind::Tamper) => {
            result.messages = vec![Message::Other(Notice::Tamper(id("sensor-a")))];
        }
        (Phase::Arming(mode) | Phase::Armed(mode) | Phase::Pending(mode), Kind::Tamper) => {
            result = Expect::phase(Phase::Triggered(Some(mode)))
                .generation(8)
                .commands(true)
                .messages(vec![Message::Other(Notice::Triggered(Cause::Tamper(id(
                    "sensor-a",
                ))))]);
        }
        (Phase::Disarmed | Phase::Arming(_) | Phase::Armed(_) | Phase::Pending(_), Kind::Panic) => {
            result.messages = vec![Message::Other(Notice::Panic)];
        }
        (Phase::Arming(_) | Phase::Armed(_) | Phase::Pending(_), Kind::Unavailable) => {
            result.messages = vec![Message::Other(Notice::Unavailable(id("sensor-a")))];
        }
        (Phase::Triggered(_), Kind::Siren) => {
            result.commands = commands(false);
            result.siren_on = Some(false);
            result.siren = Some(None);
        }
        (Phase::Triggered(Some(mode)), Kind::Trigger) => {
            result = Expect::phase(Phase::Armed(mode))
                .generation(8)
                .commands(false)
                .messages(vec![Message::Other(Notice::Ended(Some(mode)))]);
        }
        (_, Kind::Restore) => {
            result.generation = Some(8);
            if matches!(phase, Phase::Triggered(_)) {
                result.commands = commands(true);
                result.siren_on = Some(true);
            }
        }
        _ => {}
    }
    result
}

#[test]
fn every_state_event_pair_and_policy_guard_has_a_declared_outcome() {
    let mut phases = vec![Phase::Disarmed, Phase::Triggered(None)];
    for mode in [Mode::Away, Mode::Home, Mode::Night, Mode::Vacation] {
        phases.extend([
            Phase::Arming(mode),
            Phase::Armed(mode),
            Phase::Pending(mode),
            Phase::Triggered(Some(mode)),
        ]);
    }
    let mut cases = Vec::new();
    for phase in phases {
        for kind in KINDS {
            let (before, event, now, cfg) = matrix_input(phase, kind);
            let name = format!("{phase:?}.{:?}", event_kind(&event));
            cases.push(
                Case::new(&name, before, event, matrix_expected(phase, kind))
                    .at(now)
                    .config(cfg),
            );
        }
    }
    for phase in [
        Phase::Arming(Mode::Away),
        Phase::Armed(Mode::Away),
        Phase::Pending(Mode::Away),
    ] {
        let mut notify = config();
        notify.tamper_armed = ActionPolicy::Notify;
        cases.push(
            Case::new(
                &format!("{phase:?}.tamper-notify"),
                state(phase),
                Event::Tamper(id("sensor-a")),
                Expect::phase(phase).messages(vec![Message::Other(Notice::Tamper(id("sensor-a")))]),
            )
            .config(notify),
        );
        let mut panic = config();
        panic.panic = ActionPolicy::Trigger;
        cases.push(
            Case::new(
                &format!("{phase:?}.panic-trigger"),
                state(phase),
                Event::Panic(Source::Keypad("keypad-a".into())),
                Expect::phase(Phase::Triggered(Some(Mode::Away)))
                    .generation(8)
                    .commands(true)
                    .messages(vec![Message::Other(Notice::Triggered(Cause::Panic))]),
            )
            .config(panic),
        );
    }
    let mut always = config();
    always.sensors[0].always_on = true;
    cases.push(
        Case::new(
            "disarmed.always-on",
            state(Phase::Disarmed),
            Event::Open(id("sensor-a")),
            Expect::phase(Phase::Triggered(None))
                .generation(8)
                .commands(true)
                .messages(vec![Message::Other(Notice::Triggered(Cause::Sensor(id(
                    "sensor-a",
                ))))]),
        )
        .config(always),
    );
    let mut unavailable = config();
    unavailable.sensors[0].trigger_unavailable = true;
    unavailable.sensors[0].entry = false;
    cases.push(
        Case::new(
            "armed.unavailable-triggers",
            state(Phase::Armed(Mode::Away)),
            Event::Unavailable(id("sensor-a")),
            Expect::phase(Phase::Triggered(Some(Mode::Away)))
                .generation(8)
                .commands(true)
                .messages(vec![Message::Other(Notice::Triggered(Cause::Sensor(id(
                    "sensor-a",
                ))))]),
        )
        .config(unavailable),
    );
    let mut notified = state(Phase::Armed(Mode::Away));
    notified.details.unavailable_notified = ids(&["sensor-a"]);
    cases.push(Case::new(
        "armed.unavailable-once",
        notified,
        Event::Unavailable(id("sensor-a")),
        Expect::phase(Phase::Armed(Mode::Away)),
    ));
    let mut closing = state(Phase::Arming(Mode::Away));
    value(&mut closing, "sensor-a", SensorState::Open, true);
    let mut close_expected = Expect::phase(Phase::Arming(Mode::Away)).generation(8);
    close_expected.close = Some((id("sensor-a"), Some(token(8, 6_000))));
    close_expected.exit = Some(Some(token(8, 31_000)));
    cases.push(Case::new(
        "arming.close-starts-five-seconds",
        closing,
        Event::Close(id("sensor-a")),
        close_expected,
    ));
    let mut reopened = state(Phase::Arming(Mode::Away));
    reopened
        .details
        .close_timers
        .insert(id("sensor-a"), token(7, 6_000));
    let mut reopened_expected = Expect::phase(Phase::Arming(Mode::Away)).generation(8);
    reopened_expected.close = Some((id("sensor-a"), None));
    reopened_expected.exit = Some(Some(token(8, 31_000)));
    cases.push(
        Case::new(
            "arming.reopen-invalidates-close",
            reopened,
            Event::Open(id("sensor-a")),
            reopened_expected,
        )
        .at(3_000),
    );
    let mut stable = state(Phase::Arming(Mode::Away));
    stable
        .details
        .close_timers
        .insert(id("sensor-a"), token(7, 6_000));
    let mut duplicate_close = Expect::phase(Phase::Arming(Mode::Away)).ignored();
    duplicate_close.close = Some((id("sensor-a"), Some(token(7, 6_000))));
    cases.push(
        Case::new(
            "arming.duplicate-close-keeps-deadline",
            stable.clone(),
            Event::Close(id("sensor-a")),
            duplicate_close,
        )
        .at(3_000),
    );
    cases.push(
        Case::new(
            "arming.close-not-stable-yet",
            stable.clone(),
            Event::ArmOnClose {
                sensor: id("sensor-a"),
                token: token(7, 6_000),
            },
            Expect::phase(Phase::Arming(Mode::Away)).ignored(),
        )
        .at(5_999),
    );
    let mut outdated = stable;
    outdated.generation = 8;
    outdated.exit_timer = Some(token(8, 31_000));
    outdated
        .details
        .close_timers
        .insert(id("sensor-a"), token(8, 6_000));
    cases.push(
        Case::new(
            "arming.stale-close-generation",
            outdated,
            Event::ArmOnClose {
                sensor: id("sensor-a"),
                token: token(7, 6_000),
            },
            Expect::phase(Phase::Arming(Mode::Away)).ignored(),
        )
        .at(6_000),
    );
    cases.push(Case::new(
        "arming.non-exit-immediate",
        state(Phase::Arming(Mode::Away)),
        Event::Open(id("sensor-b")),
        Expect::phase(Phase::Triggered(Some(Mode::Away)))
            .generation(8)
            .commands(true)
            .messages(vec![Message::Other(Notice::Triggered(Cause::Sensor(id(
                "sensor-b",
            ))))]),
    ));
    let mut short_unavailable = matrix_config();
    short_unavailable.sensors[0].trigger_unavailable = true;
    short_unavailable.sensors[0].entry_s = Some(5);
    let mut short_expected = Expect::phase(Phase::Pending(Mode::Away)).generation(8);
    short_expected.entry = Some(Some(token(8, 10_000)));
    cases.push(
        Case::new(
            "pending.unavailable-shortens",
            state(Phase::Pending(Mode::Away)),
            Event::Unavailable(id("sensor-a")),
            short_expected,
        )
        .at(5_000)
        .config(short_unavailable),
    );
    cases.push(Case::new(
        "agent.panic-ignored",
        state(Phase::Armed(Mode::Away)),
        Event::Panic(Source::Agent),
        Expect::phase(Phase::Armed(Mode::Away)).ignored(),
    ));
    let mut not_covered = config();
    not_covered.sensors[0].modes = std::collections::BTreeSet::from([Mode::Away]);
    cases.push(
        Case::new(
            "uncovered.tamper-ignored",
            state(Phase::Armed(Mode::Home)),
            Event::Tamper(id("sensor-a")),
            Expect::phase(Phase::Armed(Mode::Home)).ignored(),
        )
        .config(not_covered),
    );
    cases.push(Case::new(
        "siren.tamper-triggers",
        state(Phase::Armed(Mode::Away)),
        Event::Tamper(id("siren-a")),
        Expect::phase(Phase::Triggered(Some(Mode::Away)))
            .generation(8)
            .commands(true)
            .messages(vec![Message::Other(Notice::Triggered(Cause::Tamper(id(
                "siren-a",
            ))))]),
    ));
    let rows = evaluate(cases);
    assert!(finish("F2", rows));
}
