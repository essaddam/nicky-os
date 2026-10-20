use super::*;
use crate::{model, protocol};
use http::StatusCode;
use std::collections::VecDeque;
use std::sync::Mutex;
struct Script {
    answers: Mutex<VecDeque<(Call, Answer)>>,
    sent: Mutex<Vec<Call>>,
}
impl Script {
    fn new(answers: Vec<(Call, Answer)>) -> Self {
        Self {
            answers: Mutex::new(answers.into()),
            sent: Mutex::new(vec![]),
        }
    }
    fn sent(&self) -> Vec<Call> {
        self.sent.lock().unwrap().clone()
    }
}
impl Wire for Script {
    fn send(
        &self,
        call: Call,
        body: String,
        token: Option<String>,
    ) -> impl Future<Output = Result<Answer, Failure>> + Send {
        assert_eq!(
            body,
            protocol::body(call, (call == Call::Login).then_some(CREDS)).unwrap()
        );
        if call == Call::Login {
            assert!(token.is_none());
        } else {
            assert!(token.is_some());
        }
        self.sent.lock().unwrap().push(call);
        let (expected, answer) = self
            .answers
            .lock()
            .unwrap()
            .pop_front()
            .expect("no extra request");
        assert_eq!(call, expected);
        std::future::ready(Ok(answer))
    }
}
fn reply(call: Call, status: StatusCode, json: &serde_json::Value) -> (Call, Answer) {
    (
        call,
        Answer {
            status,
            body: json.to_string().into_bytes(),
        },
    )
}
fn login() -> (Call, Answer) {
    let generated = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    reply(
        Call::Login,
        StatusCode::OK,
        &serde_json::json!({"asus_token":format!("synthetic-{generated}")}),
    )
}
fn info() -> (Call, Answer) {
    reply(
        Call::Read(Action::Info),
        StatusCode::OK,
        &serde_json::json!({"label_mac":"aa:bb:cc:dd:ee:01","firmver":"test"}),
    )
}
const CREDS: (&str, &str) = ("synthetic-user", "synthetic-password");
#[test]
fn wire_has_only_two_paths_and_closed_encoded_payloads() {
    assert_eq!(Call::Login.path(), "/login.cgi");
    assert_eq!(Call::Read(Action::Clients).path(), "/appGet.cgi");
    let login = protocol::body(Call::Login, Some(CREDS)).unwrap();
    assert!(login.starts_with("login_authorization%3D"));
    assert!(!login.contains(CREDS.1));
    for action in [Action::Info, Action::Telemetry, Action::Clients] {
        let body = protocol::body(Call::Read(action), None).unwrap();
        assert!(body.starts_with("hook%3D"));
        assert!(!body.contains("apply"));
        assert!(!body.contains("netool"));
    }
    assert!(protocol::body(Call::Login, Some(("bad:user", "value"))).is_err());
}
#[test]
fn tokens_controls_sizes_and_duplicate_json_are_refused() {
    assert!(protocol::token(&serde_json::json!({"asus_token":"synthetic"})).is_ok());
    for token in [
        String::new(),
        "a\r\nb".into(),
        "a;b".into(),
        "é".into(),
        "x".repeat(1025),
    ] {
        assert!(protocol::token(&serde_json::json!({"asus_token":token})).is_err());
    }
    for code in [3, 7, 10, 11] {
        assert_eq!(
            protocol::access(&serde_json::json!({"error_status":code}), false),
            Err(Failure::Refused)
        );
    }
    assert_eq!(
        protocol::access(&serde_json::json!({"error_status":1}), false),
        Err(Failure::Expired)
    );
}
#[tokio::test]
async fn session_refresh_is_unique_and_replays_only_the_expired_read() {
    let wire = Script::new(vec![
        login(),
        info(),
        reply(
            Call::Read(Action::Telemetry),
            StatusCode::OK,
            &serde_json::json!({"error_status":1}),
        ),
        login(),
        info(),
        reply(
            Call::Read(Action::Telemetry),
            StatusCode::OK,
            &serde_json::json!({"memory_usage":{"mem_total":"100","mem_used":"25"}}),
        ),
        reply(
            Call::Read(Action::Clients),
            StatusCode::UNAUTHORIZED,
            &serde_json::json!({}),
        ),
    ]);
    let mut client = Client::new(None);
    let mut budget = Budget::default();
    assert!(
        client
            .read(&wire, Action::Telemetry, CREDS, &mut budget)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(client.epoch, 2);
    assert_eq!(
        client
            .read(&wire, Action::Clients, CREDS, &mut budget)
            .await
            .unwrap_err(),
        Failure::Refused
    );
    let sent = wire.sent();
    assert_eq!(sent.len(), 7);
    assert_eq!(sent.iter().filter(|c| **c == Call::Login).count(), 2);
    assert!(
        client
            .read(&wire, Action::Info, CREDS, &mut Budget::default())
            .await
            .is_err()
    );
    assert_eq!(wire.sent(), sent);
}
#[tokio::test]
async fn locked_or_captcha_during_valid_session_never_resends_credentials() {
    for (status, code) in [
        (StatusCode::LOCKED, 0),
        (StatusCode::OK, 7),
        (StatusCode::INTERNAL_SERVER_ERROR, 10),
        (StatusCode::OK, 3),
    ] {
        let wire = Script::new(vec![
            login(),
            info(),
            reply(
                Call::Read(Action::Clients),
                status,
                &serde_json::json!({"error_status":code}),
            ),
        ]);
        let mut client = Client::new(None);
        client
            .read(&wire, Action::Info, CREDS, &mut Budget::default())
            .await
            .unwrap();
        assert_eq!(
            client
                .read(&wire, Action::Clients, CREDS, &mut Budget::default())
                .await
                .unwrap_err(),
            Failure::Refused
        );
        assert!(
            client
                .read(&wire, Action::Info, CREDS, &mut Budget::default())
                .await
                .is_err()
        );
        assert_eq!(
            wire.sent(),
            vec![
                Call::Login,
                Call::Read(Action::Info),
                Call::Read(Action::Clients)
            ]
        );
    }
}
#[tokio::test]
async fn identity_change_redirect_and_budget_fail_without_extra_requests() {
    let wire = Script::new(vec![
        login(),
        reply(
            Call::Read(Action::Info),
            StatusCode::OK,
            &serde_json::json!({"label_mac":"aa:bb:cc:dd:ee:02"}),
        ),
    ]);
    let mut client = Client::new(Some("aabbccddee01".into()));
    assert_eq!(
        client
            .read(&wire, Action::Info, CREDS, &mut Budget::default())
            .await
            .unwrap_err(),
        Failure::Identity
    );
    assert!(
        client
            .read(&wire, Action::Info, CREDS, &mut Budget::default())
            .await
            .is_err()
    );
    assert_eq!(wire.sent().len(), 2);
    let wire = Script::new(vec![]);
    let mut client = Client::new(None);
    assert_eq!(
        client
            .read(
                &wire,
                Action::Info,
                CREDS,
                &mut Budget {
                    sent: 16,
                    relogin: false
                }
            )
            .await
            .unwrap_err(),
        Failure::Budget
    );
    assert!(wire.sent().is_empty());
    let wire = Script::new(vec![reply(
        Call::Login,
        StatusCode::FOUND,
        &serde_json::json!({}),
    )]);
    let mut client = Client::new(None);
    assert!(
        client
            .read(&wire, Action::Info, CREDS, &mut Budget::default())
            .await
            .is_err()
    );
    assert_eq!(wire.sent(), vec![Call::Login]);
}
#[tokio::test]
async fn unsupported_optional_capability_is_cached_for_the_session() {
    let wire = Script::new(vec![
        login(),
        info(),
        reply(
            Call::Read(Action::Clients),
            StatusCode::NOT_FOUND,
            &serde_json::json!({}),
        ),
    ]);
    let mut client = Client::new(None);
    assert!(
        client
            .read(&wire, Action::Clients, CREDS, &mut Budget::default())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        client
            .read(&wire, Action::Clients, CREDS, &mut Budget::default())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(wire.sent().len(), 3);
    let _ = model::Samples::new();
}

#[tokio::test]
async fn interrupted_identity_verification_cannot_leave_an_authenticated_session() {
    let wire = Script::new(vec![
        login(),
        reply(
            Call::Read(Action::Info),
            StatusCode::INTERNAL_SERVER_ERROR,
            &serde_json::json!({}),
        ),
        login(),
        info(),
    ]);
    let mut client = Client::new(None);
    assert_eq!(
        client
            .read(&wire, Action::Info, CREDS, &mut Budget::default())
            .await
            .unwrap_err(),
        Failure::Transport
    );
    assert!(
        client
            .read(&wire, Action::Info, CREDS, &mut Budget::default())
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(client.epoch, 1);
    assert_eq!(
        wire.sent(),
        vec![
            Call::Login,
            Call::Read(Action::Info),
            Call::Login,
            Call::Read(Action::Info)
        ]
    );
}

#[tokio::test]
async fn http_auth_status_cannot_hide_an_explicit_json_refusal() {
    for (status, code) in [
        (StatusCode::UNAUTHORIZED, 3),
        (StatusCode::UNAUTHORIZED, 7),
        (StatusCode::FORBIDDEN, 10),
        (StatusCode::FORBIDDEN, 0),
    ] {
        let wire = Script::new(vec![
            login(),
            info(),
            reply(
                Call::Read(Action::Clients),
                status,
                &serde_json::json!({"error_status":code}),
            ),
        ]);
        let mut client = Client::new(None);
        client
            .read(&wire, Action::Info, CREDS, &mut Budget::default())
            .await
            .unwrap();
        assert_eq!(
            client
                .read(&wire, Action::Clients, CREDS, &mut Budget::default())
                .await
                .unwrap_err(),
            Failure::Refused
        );
        assert_eq!(
            wire.sent(),
            vec![
                Call::Login,
                Call::Read(Action::Info),
                Call::Read(Action::Clients)
            ]
        );
    }
}
