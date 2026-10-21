use super::*;

#[test]
fn auto_h1_cli_keeps_duration_operation_and_request_identity() {
    let cli = Cli::try_parse_from([
        "moli-os",
        "timer",
        "fixture:clock",
        "start",
        "5",
        "--request-id",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:7",
    ])
    .unwrap();
    let Some(Command::Timer {
        device,
        operation: TimerAction::Start { seconds },
        request_id,
    }) = cli.command
    else {
        panic!("not a start")
    };
    assert_eq!(device, "fixture:clock");
    assert_eq!(seconds, 5);
    assert_eq!(
        request_id.as_deref(),
        Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:7")
    );
    assert!(Cli::try_parse_from(["moli-os", "timer", "fixture:clock", "start", "1.0"]).is_err());
    assert!(Cli::try_parse_from(["moli-os", "timer", "fixture:clock", "start", "-1"]).is_err());
    let cli = Cli::try_parse_from(["moli-os", "timer", "fixture:clock", "cancel"]).unwrap();
    assert!(matches!(
        cli.command,
        Some(Command::Timer {
            operation: TimerAction::Cancel,
            ..
        })
    ));
}
