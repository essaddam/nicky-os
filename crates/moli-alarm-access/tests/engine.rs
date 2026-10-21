mod common;
use common::House;
use moli_alarm::{
    ArmRequest, Config, DisarmRequest, Event, Mode, ModeConfig, Permission, Phase, RequestId,
    Source, State, TimestampMs, Transition, Verdict, step,
};
use moli_alarm_access::{Code, RequestSource};

async fn command(
    h: &House,
    state: State,
    source: RequestSource,
    mode: Option<Mode>,
    code: Code<'_>,
) -> Transition {
    let permission = match mode {
        None => Permission::Disarm,
        Some(m) if state.phase().mode().is_some_and(|old| old != m) => Permission::ChangeMode,
        _ => Permission::Arm,
    };
    let receipt = h
        .profiles
        .verify(RequestId(42), source.clone(), code, permission, mode)
        .await
        .unwrap();
    let event = mode.map_or_else(
        || {
            Event::Disarm(DisarmRequest {
                id: RequestId(42),
                source: source.engine_source().clone(),
            })
        },
        |mode| {
            Event::Arm(ArmRequest {
                id: RequestId(42),
                mode,
                source: source.engine_source().clone(),
                skip_exit: false,
                force: std::collections::BTreeSet::default(),
            })
        },
    );
    let config = Config {
        modes: [Mode::Away, Mode::Home]
            .into_iter()
            .map(|mode| {
                (
                    mode,
                    ModeConfig {
                        exit_s: 0,
                        entry_s: 30,
                        trigger_s: 180,
                    },
                )
            })
            .collect(),
        ..Default::default()
    };
    step(state, event, TimestampMs(1_000), &config, &receipt)
}

#[tokio::test]
async fn a_pin_session_never_replaces_the_alarm_code_for_disarm_or_mode_change() {
    let h = House::new().await;
    h.profiles
        .create(&House::owner(), House::draft(), Code::provided("70710937"))
        .await
        .unwrap();
    let armed = command(
        &h,
        State::default(),
        House::source("ui"),
        Some(Mode::Away),
        Code::missing(),
    )
    .await;
    assert_eq!(armed.verdict, Some(Verdict::Armed(Mode::Away)));
    for human in [false, true] {
        let source = RequestSource::dashboard("ui", human).unwrap();
        let disarm = command(
            &h,
            armed.state.clone(),
            source.clone(),
            None,
            Code::missing(),
        )
        .await;
        assert_eq!(disarm.verdict, Some(Verdict::CodeRequired));
        assert_eq!(disarm.state, armed.state);
        let change = command(
            &h,
            armed.state.clone(),
            source,
            Some(Mode::Home),
            Code::missing(),
        )
        .await;
        assert_eq!(change.verdict, Some(Verdict::CodeRequired));
    }
    let changed = command(
        &h,
        armed.state,
        House::source("ui"),
        Some(Mode::Home),
        Code::provided("70710937"),
    )
    .await;
    assert_eq!(changed.verdict, Some(Verdict::Armed(Mode::Home)));
    let disarmed = command(
        &h,
        changed.state,
        House::source("ui"),
        None,
        Code::provided("70710937"),
    )
    .await;
    assert_eq!(disarmed.verdict, Some(Verdict::Disarmed));
    assert_eq!(disarmed.state.phase(), Phase::Disarmed);
}

#[tokio::test]
async fn a_supplied_code_must_have_arm_rights_even_when_absence_is_permitted() {
    let h = House::new().await;
    let mut draft = House::draft();
    draft.can_arm = false;
    h.profiles
        .create(&House::owner(), draft, Code::provided("70710937"))
        .await
        .unwrap();
    for code in ["70710937", "70710938", "invalid"] {
        let r = command(
            &h,
            State::default(),
            House::source("ui-code"),
            Some(Mode::Away),
            Code::provided(code),
        )
        .await;
        assert_eq!(r.verdict, Some(Verdict::InvalidCode));
        assert_eq!(r.state.phase(), Phase::Disarmed);
        assert!(!format!("{r:?}").contains(code));
    }
    let r = command(
        &h,
        State::default(),
        House::source("ui"),
        Some(Mode::Away),
        Code::missing(),
    )
    .await;
    assert_eq!(r.verdict, Some(Verdict::Armed(Mode::Away)));
}

#[tokio::test]
async fn profiles_preserve_agent_and_automation_source_policy() {
    let h = House::new().await;
    h.profiles
        .create(&House::owner(), House::draft(), Code::provided("70710937"))
        .await
        .unwrap();
    let agent = command(
        &h,
        State::default(),
        RequestSource::agent(),
        Some(Mode::Away),
        Code::missing(),
    )
    .await;
    assert_eq!(agent.verdict, Some(Verdict::HumanApprovalRequired));
    let automatic = command(
        &h,
        State::default(),
        RequestSource::automation(),
        Some(Mode::Away),
        Code::missing(),
    )
    .await;
    assert_eq!(automatic.verdict, Some(Verdict::Armed(Mode::Away)));
    for source in [RequestSource::agent(), RequestSource::automation()] {
        let denied = command(
            &h,
            automatic.state.clone(),
            source,
            None,
            Code::provided("70710937"),
        )
        .await;
        assert_eq!(denied.verdict, Some(Verdict::SourceDenied));
        assert_eq!(denied.state, automatic.state);
    }
    assert_eq!(House::source("ui").engine_source(), &Source::Ui);
}
