use std::collections::BTreeMap;

use crate::{
    ArmRequest, Authorization, Config, Effect, Event, Mode, ModeConfig, Permission, Phase,
    Profiles, RequestId, Source, State, TimestampMs, Verdict, step,
};

struct NoCode;

impl Profiles for NoCode {
    fn authorize(
        &self,
        _request: RequestId,
        _source: &Source,
        _permission: Permission,
        _mode: Option<Mode>,
        _now: TimestampMs,
    ) -> Authorization {
        Authorization::Missing
    }
}

#[test]
fn arming_without_exit_confirms_each_enabled_mode() {
    // Spec 4.3: zero exit delay, no blocking sensor, and no supplied code.
    // A successful command must report confirmed armed, not merely accepted.
    for mode in [Mode::Away, Mode::Home, Mode::Night, Mode::Vacation] {
        let config = Config {
            modes: BTreeMap::from([(
                mode,
                ModeConfig {
                    exit_s: 0,
                    entry_s: 30,
                    trigger_s: 1800,
                },
            )]),
            siren_max_s: 180,
            ..Config::default()
        };
        let result = step(
            State::default(),
            Event::Arm(ArmRequest {
                id: RequestId(1),
                mode,
                source: Source::Household,
                skip_exit: false,
                force: std::collections::BTreeSet::new(),
            }),
            TimestampMs(1_000),
            &config,
            &NoCode,
        );
        assert_eq!(result.state.phase(), Phase::Armed(mode));
        assert_eq!(result.state.generation(), 1);
        assert_eq!(result.verdict, Some(Verdict::Armed(mode)));
        assert_eq!(
            result.effects,
            [
                Effect::JournalTransition {
                    from: Phase::Disarmed,
                    to: Phase::Armed(mode),
                },
                Effect::NotifyArmed(mode),
            ]
        );
    }
}
