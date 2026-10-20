//! Session tests use a scripted transport; no packet reaches a household device.
use super::*;
use std::collections::VecDeque;
use std::sync::Mutex;
struct Fake {
    answers: Mutex<VecDeque<(Action, Answer)>>,
    sent: Mutex<Vec<Action>>,
}
impl Fake {
    fn new(answers: Vec<(Action, Answer)>) -> Self {
        Self {
            answers: Mutex::new(answers.into()),
            sent: Mutex::new(vec![]),
        }
    }
    fn sent(&self) -> Vec<Action> {
        self.sent.lock().unwrap().clone()
    }
}
impl Wire for Fake {
    fn host(&self) -> &'static str {
        "router.example"
    }
    fn https(&self) -> bool {
        true
    }

    fn send(
        &self,
        action: Action,
        body: String,
        cookie: Option<String>,
    ) -> impl std::future::Future<Output = Result<Answer, Failure>> + Send {
        assert!(body.contains(action.name()));
        if action == Action::LoginV2 {
            assert!(cookie.is_none());
            assert!(body.contains("<Username>"));
            assert!(!body.contains("NewUsername"));
        }
        if action == Action::LoginV1 {
            assert!(cookie.is_none());
            assert!(body.contains("<NewUsername>"));
        }
        self.sent.lock().unwrap().push(action);
        let (expected, answer) = self
            .answers
            .lock()
            .unwrap()
            .pop_front()
            .expect("no extra request permitted");
        assert_eq!(action, expected);
        std::future::ready(Ok(answer))
    }
}
fn answer(action: Action, fields: &str, code: u16) -> Answer {
    Answer {
        status: StatusCode::OK,
        headers: http::HeaderMap::new(),
        body: protocol::response(action, fields, code).into_bytes(),
    }
}
fn login(version: Version) -> (Action, Answer) {
    let action = if version == Version::V1 {
        Action::LoginV1
    } else {
        Action::LoginV2
    };
    let mut a = answer(action, "", 0);
    if version == Version::V2 {
        let value = format!(
            "sid={:x}; Path=/; HttpOnly",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        a.headers.insert("set-cookie", value.parse().unwrap());
    }
    (action, a)
}
fn info() -> (Action, Answer) {
    (
        Action::Info,
        answer(
            Action::Info,
            "<SerialNumber>synthetic-router-01</SerialNumber>",
            0,
        ),
    )
}
fn credentials() -> (String, String) {
    let s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        .to_string();
    (format!("u{s}"), format!("p{s}"))
}
#[tokio::test]
async fn login_variants_are_explicit_and_unsupported_capabilities_are_cached() {
    for version in [Version::V1, Version::V2] {
        let wire = Fake::new(vec![
            login(version),
            info(),
            (Action::System, answer(Action::System, "", 501)),
        ]);
        let mut client = Client::new(version, None);
        let mut budget = Budget::default();
        let (u, p) = credentials();
        assert!(
            client
                .read(&wire, Action::System, (&u, &p), &mut budget)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            client
                .read(&wire, Action::System, (&u, &p), &mut budget)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(wire.sent().len(), 3);
    }
}
#[tokio::test]
async fn only_one_relogin_and_only_the_refused_read_is_replayed() {
    let wire = Fake::new(vec![
        login(Version::V2),
        info(),
        (Action::System, answer(Action::System, "", 401)),
        login(Version::V2),
        info(),
        (
            Action::System,
            answer(
                Action::System,
                "<NewCPUUtilization>12</NewCPUUtilization>",
                0,
            ),
        ),
        (Action::Traffic, answer(Action::Traffic, "", 401)),
    ]);
    let mut client = Client::new(Version::V2, None);
    let mut budget = Budget::default();
    let (u, p) = credentials();
    assert!(
        client
            .read(&wire, Action::System, (&u, &p), &mut budget)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        client
            .read(&wire, Action::Traffic, (&u, &p), &mut budget)
            .await
            .unwrap_err(),
        Failure::Refused
    );
    assert_eq!(
        wire.sent(),
        vec![
            Action::LoginV2,
            Action::Info,
            Action::System,
            Action::LoginV2,
            Action::Info,
            Action::System,
            Action::Traffic
        ]
    );
    assert_eq!(
        client
            .read(&wire, Action::System, (&u, &p), &mut Budget::default())
            .await
            .unwrap_err(),
        Failure::Refused
    );
    assert_eq!(wire.sent().len(), 7);
}
#[tokio::test]
async fn bad_login_identity_change_redirect_and_exchange_budget_never_retry() {
    let (u, p) = credentials();
    let wire = Fake::new(vec![(Action::LoginV2, answer(Action::LoginV2, "", 401))]);
    let mut client = Client::new(Version::V2, None);
    for _ in 0..2 {
        assert_eq!(
            client
                .read(&wire, Action::System, (&u, &p), &mut Budget::default())
                .await
                .unwrap_err(),
            Failure::Refused
        );
    }
    assert_eq!(wire.sent().len(), 1);
    let wire = Fake::new(vec![login(Version::V2), info()]);
    let mut client = Client::new(Version::V2, Some("different-router".into()));
    assert_eq!(
        client
            .read(&wire, Action::System, (&u, &p), &mut Budget::default())
            .await
            .unwrap_err(),
        Failure::Identity
    );
    assert_eq!(wire.sent().len(), 2);
    let wire = Fake::new(vec![]);
    let mut client = Client::new(Version::V2, None);
    let mut b = Budget {
        sent: 16,
        relogin: false,
    };
    assert_eq!(
        client
            .read(&wire, Action::Info, (&u, &p), &mut b)
            .await
            .unwrap_err(),
        Failure::Budget
    );
    assert!(wire.sent().is_empty());
    let (a, mut response) = login(Version::V2);
    response.status = StatusCode::FOUND;
    response
        .headers
        .insert("location", "http://192.168.1.9/".parse().unwrap());
    let wire = Fake::new(vec![(a, response)]);
    let mut client = Client::new(Version::V2, None);
    assert!(
        client
            .read(&wire, Action::Info, (&u, &p), &mut Budget::default())
            .await
            .is_err()
    );
    assert_eq!(wire.sent().len(), 1);
}
#[tokio::test]
async fn missing_cookie_and_required_info_failure_park_the_session() {
    let (u, p) = credentials();
    let wire = Fake::new(vec![(Action::LoginV2, answer(Action::LoginV2, "", 0))]);
    let mut client = Client::new(Version::V2, None);
    assert_eq!(
        client
            .read(&wire, Action::Info, (&u, &p), &mut Budget::default())
            .await
            .unwrap_err(),
        Failure::Refused
    );
    assert_eq!(wire.sent().len(), 1);
    let wire = Fake::new(vec![
        login(Version::V2),
        info(),
        (Action::Info, answer(Action::Info, "", 501)),
    ]);
    let mut client = Client::new(Version::V2, None);
    client
        .read(&wire, Action::Info, (&u, &p), &mut Budget::default())
        .await
        .unwrap();
    assert_eq!(
        client
            .read(&wire, Action::Info, (&u, &p), &mut Budget::default())
            .await
            .unwrap_err(),
        Failure::Identity
    );
    assert_eq!(
        client
            .read(&wire, Action::System, (&u, &p), &mut Budget::default())
            .await
            .unwrap_err(),
        Failure::Identity
    );
    assert_eq!(wire.sent().len(), 3);
}
#[tokio::test]
async fn unavailable_authentication_service_backs_off_without_parking_or_retrying() {
    let (u, p) = credentials();
    for http_status in [
        StatusCode::SERVICE_UNAVAILABLE,
        StatusCode::INTERNAL_SERVER_ERROR,
    ] {
        let mut unavailable = answer(Action::LoginV2, "", 503);
        unavailable.status = http_status;
        unavailable.body = b"<html>temporarily unavailable</html>".to_vec();
        let wire = Fake::new(vec![
            (Action::LoginV2, unavailable),
            login(Version::V2),
            info(),
        ]);
        let mut client = Client::new(Version::V2, None);
        assert_eq!(
            client
                .read(&wire, Action::Info, (&u, &p), &mut Budget::default())
                .await
                .unwrap_err(),
            Failure::Transport
        );
        assert_eq!(wire.sent().len(), 1);
        assert!(
            client
                .read(&wire, Action::Info, (&u, &p), &mut Budget::default())
                .await
                .unwrap()
                .is_some()
        );
        assert_eq!(wire.sent().len(), 3);
    }
    let wire = Fake::new(vec![(Action::LoginV2, answer(Action::LoginV2, "", 503))]);
    let mut client = Client::new(Version::V2, None);
    assert_eq!(
        client
            .read(&wire, Action::Info, (&u, &p), &mut Budget::default())
            .await
            .unwrap_err(),
        Failure::Transport
    );
    assert_eq!(wire.sent().len(), 1);
}
