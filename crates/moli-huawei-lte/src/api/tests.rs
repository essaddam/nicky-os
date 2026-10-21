use super::*;
use crate::{cookies::Cookies, protocol, tokens::Tokens};
use http::{HeaderMap, StatusCode};
use std::{collections::VecDeque, sync::Mutex};
const USER: &str = "synthetic-user";
const PASS: &str = "synthetic-password";
struct Script {
    answers: Mutex<VecDeque<(Call, Answer)>>,
    sent: Mutex<Vec<Call>>,
}
impl Script {
    fn new(replies: Vec<(Call, Answer)>) -> Self {
        Self {
            answers: Mutex::new(replies.into()),
            sent: Mutex::new(vec![]),
        }
    }
    fn sent(&self) -> Vec<Call> {
        self.sent.lock().unwrap().clone()
    }
}
impl Wire for Script {
    fn host(&self) -> &'static str {
        "modem.example.invalid"
    }
    fn https(&self) -> bool {
        true
    }
    fn send(
        &self,
        call: Call,
        body: Option<String>,
        token: Option<String>,
        cookie: Option<String>,
    ) -> impl Future<Output = Result<Answer, Failure>> + Send {
        assert_eq!(body.is_some(), call == Call::Login);
        if call == Call::Login {
            // Predicate assertion: never dump the authentication body on failure.
            let expected = protocol::login(USER, PASS, token.as_deref().unwrap()).unwrap();
            assert!(
                body.as_deref()
                    .is_some_and(|actual| actual == expected.as_str())
            );
        }
        if matches!(call, Call::Session | Call::Token) {
            assert!(cookie.is_none());
        } else {
            assert!(cookie.is_some());
            if matches!(call, Call::State | Call::Login) {
                assert!(token.is_some());
            }
        }
        self.sent.lock().unwrap().push(call);
        let (expected, answer) = self
            .answers
            .lock()
            .unwrap()
            .pop_front()
            .expect("no extra request permitted");
        assert_eq!(call, expected);
        std::future::ready(Ok(answer))
    }
}
fn answer(call: Call, raw: &str) -> (Call, Answer) {
    (
        call,
        Answer {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            body: raw.as_bytes().to_vec(),
        },
    )
}
fn bootstrap() -> Vec<(Call, Answer)> {
    let random = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let token = format!("synthetic-{random}");
    let mut login = answer(Call::Login, "<response>OK</response>");
    login.1.headers.insert(
        "__requestverificationtokenone",
        format!("{token}-one").parse().unwrap(),
    );
    login.1.headers.insert(
        "__requestverificationtokentwo",
        format!("{token}-two").parse().unwrap(),
    );
    vec![
        answer(
            Call::Session,
            &format!(
                "<response><SesInfo>SessionID={random}</SesInfo><TokInfo>{token}</TokInfo></response>"
            ),
        ),
        answer(
            Call::State,
            "<response><State>-1</State><password_type>4</password_type></response>",
        ),
        login,
        info(),
    ]
}
fn info() -> (Call, Answer) {
    answer(
        Call::Read(Action::Info),
        "<response><SerialNumber>synthetic-modem-01</SerialNumber></response>",
    )
}
fn client() -> Client {
    Client::new(Auth::Sha256V4, Session::Sestok, None)
}
#[test]
fn token_rotation_replaces_the_bounded_queue_and_rejects_ambiguous_headers() {
    let mut tokens = Tokens::default();
    tokens.seed("synthetic").unwrap();
    assert!(tokens.for_read().is_some_and(|t| t == "synthetic"));
    let mut headers = HeaderMap::new();
    headers.insert("__requestverificationtokenone", "first".parse().unwrap());
    headers.insert("__requestverificationtokentwo", "second".parse().unwrap());
    tokens.replace(&headers).unwrap();
    assert!(
        tokens.for_read().is_none(),
        "a pair is retained for POST, not consumed by GET"
    );
    assert!(tokens.take().is_some_and(|v| v == "first"));
    assert!(tokens.take().is_some_and(|v| v == "second"));
    headers.clear();
    headers.insert("__requestverificationtoken", "replacement".parse().unwrap());
    tokens.replace(&headers).unwrap();
    assert!(tokens.take().is_some_and(|v| v == "replacement"));
    headers.insert(
        "__requestverificationtokenone",
        "ambiguous".parse().unwrap(),
    );
    assert!(tokens.replace(&headers).is_err());
    assert!(tokens.take().is_some_and(|v| v == "replacement"));
    for bad in [String::new(), "x\r\ny".into(), "x".repeat(1025)] {
        assert!(tokens.seed(&bad).is_err());
    }
}
#[test]
fn cookies_are_instance_scoped_and_bounded_without_url_adoption() {
    let mut cookies = Cookies::default();
    cookies
        .apply(
            &["SessionID=synthetic; Path=/; Secure".into()],
            "modem.example.invalid",
            true,
        )
        .unwrap();
    assert!(cookies.header().is_some_and(|v| v == "SessionID=synthetic"));
    for headers in [
        vec!["SessionID=x; Domain=other.example.invalid".into()],
        vec!["SessionID=x\r\ny:z".into()],
        (0..9).map(|i| format!("k{i}=x")).collect(),
        vec![format!("SessionID={}", "x".repeat(4096))],
    ] {
        assert!(
            cookies
                .apply(&headers, "modem.example.invalid", true)
                .is_err()
        );
    }
    assert!(cookies.header().is_some_and(|v| v == "SessionID=synthetic"));
}
#[tokio::test]
async fn only_the_expired_read_is_replayed_once_and_identity_is_rechecked() {
    let mut replies = bootstrap();
    replies.push(answer(
        Call::Read(Action::Signal),
        "<error><code>125003</code></error>",
    ));
    replies.extend(bootstrap());
    replies.push(answer(
        Call::Read(Action::Signal),
        "<response><rssi>-75dBm</rssi></response>",
    ));
    replies.push(answer(
        Call::Read(Action::Traffic),
        "<error><code>100003</code></error>",
    ));
    let wire = Script::new(replies);
    let mut client = client();
    let mut budget = Budget::default();
    assert!(
        client
            .read(&wire, Action::Signal, Some((USER, PASS)), &mut budget)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(client.epoch, 2);
    assert_eq!(
        client
            .read(&wire, Action::Traffic, Some((USER, PASS)), &mut budget)
            .await
            .unwrap_err(),
        Failure::Refused
    );
    let sent = wire.sent();
    assert_eq!(sent.iter().filter(|c| **c == Call::Login).count(), 2);
    assert_eq!(
        sent.iter()
            .filter(|c| **c == Call::Read(Action::Signal))
            .count(),
        2
    );
    assert!(
        client
            .read(
                &wire,
                Action::Info,
                Some((USER, PASS)),
                &mut Budget::default()
            )
            .await
            .is_err()
    );
    assert_eq!(wire.sent(), sent);
}
#[tokio::test]
async fn credential_lock_under_http_200_401_500_never_relogs() {
    for status in [
        StatusCode::OK,
        StatusCode::UNAUTHORIZED,
        StatusCode::INTERNAL_SERVER_ERROR,
    ] {
        let mut replies = bootstrap();
        let mut refusal = answer(
            Call::Read(Action::Traffic),
            "<error><code>108007</code><message>private</message></error>",
        );
        refusal.1.status = status;
        replies.push(refusal);
        let wire = Script::new(replies);
        let mut client = client();
        client
            .read(
                &wire,
                Action::Info,
                Some((USER, PASS)),
                &mut Budget::default(),
            )
            .await
            .unwrap();
        assert_eq!(
            client
                .read(
                    &wire,
                    Action::Traffic,
                    Some((USER, PASS)),
                    &mut Budget::default()
                )
                .await
                .unwrap_err(),
            Failure::Refused
        );
        assert_eq!(wire.sent().iter().filter(|c| **c == Call::Login).count(), 1);
        assert!(
            client
                .read(
                    &wire,
                    Action::Info,
                    Some((USER, PASS)),
                    &mut Budget::default()
                )
                .await
                .is_err()
        );
        assert_eq!(wire.sent().len(), 5);
    }
}
#[tokio::test]
async fn unknown_login_type_and_existing_session_do_not_force_login() {
    let mut replies = bootstrap();
    replies.truncate(2);
    replies[1] = answer(
        Call::State,
        "<response><State>-1</State><password_type>3</password_type></response>",
    );
    let wire = Script::new(replies);
    assert_eq!(
        client()
            .read(
                &wire,
                Action::Info,
                Some((USER, PASS)),
                &mut Budget::default()
            )
            .await
            .unwrap_err(),
        Failure::Variant
    );
    assert_eq!(wire.sent(), vec![Call::Session, Call::State]);
    let mut replies = bootstrap();
    replies.truncate(2);
    replies[1] = answer(
        Call::State,
        "<response><State>0</State><password_type>4</password_type></response>",
    );
    replies.push(info());
    let wire = Script::new(replies);
    assert!(
        client()
            .read(
                &wire,
                Action::Info,
                Some((USER, PASS)),
                &mut Budget::default()
            )
            .await
            .is_ok()
    );
    assert_eq!(
        wire.sent(),
        vec![Call::Session, Call::State, Call::Read(Action::Info)]
    );
}
#[tokio::test]
async fn explicit_token_variant_without_auth_uses_no_alternate_route_or_post() {
    let mut first = answer(Call::Token, "<response><token>synthetic</token></response>");
    first
        .1
        .headers
        .insert("set-cookie", "SessionID=synthetic; Path=/".parse().unwrap());
    let wire = Script::new(vec![first, info()]);
    let mut client = Client::new(Auth::None, Session::Token, None);
    assert!(
        client
            .read(&wire, Action::Info, None, &mut Budget::default())
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(wire.sent(), vec![Call::Token, Call::Read(Action::Info)]);
}
#[tokio::test]
async fn missing_identity_changed_identity_capability_and_budget_are_fail_closed() {
    let mut replies = bootstrap();
    replies[3] = answer(
        Call::Read(Action::Info),
        "<response><DeviceName>ignored</DeviceName></response>",
    );
    let wire = Script::new(replies);
    assert_eq!(
        client()
            .read(
                &wire,
                Action::Info,
                Some((USER, PASS)),
                &mut Budget::default()
            )
            .await
            .unwrap_err(),
        Failure::Identity
    );
    let mut replies = bootstrap();
    replies.push(answer(
        Call::Read(Action::Signal),
        "<error><code>100002</code></error>",
    ));
    let wire = Script::new(replies);
    let mut client = client();
    assert!(
        client
            .read(
                &wire,
                Action::Signal,
                Some((USER, PASS)),
                &mut Budget::default()
            )
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        client
            .read(
                &wire,
                Action::Signal,
                Some((USER, PASS)),
                &mut Budget::default()
            )
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(wire.sent().len(), 5);
    let wire = Script::new(vec![]);
    assert_eq!(
        client
            .read(
                &wire,
                Action::Status,
                Some((USER, PASS)),
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
}

#[tokio::test]
async fn an_unsupported_login_variant_parks_instead_of_resending_credentials() {
    let mut replies = bootstrap();
    replies.truncate(3);
    let mut unsupported = answer(Call::Login, "<error><code>100002</code></error>");
    unsupported.1.status = StatusCode::NOT_FOUND;
    replies[2] = unsupported;
    let wire = Script::new(replies);
    let mut client = client();
    assert_eq!(
        client
            .read(
                &wire,
                Action::Info,
                Some((USER, PASS)),
                &mut Budget::default()
            )
            .await
            .unwrap_err(),
        Failure::Variant
    );
    assert_eq!(
        client
            .read(
                &wire,
                Action::Info,
                Some((USER, PASS)),
                &mut Budget::default()
            )
            .await
            .unwrap_err(),
        Failure::Variant
    );
    assert_eq!(wire.sent(), vec![Call::Session, Call::State, Call::Login]);
}

#[tokio::test]
async fn the_login_token_pair_is_not_sent_or_consumed_by_get() {
    struct PairWire(Script);
    impl Wire for PairWire {
        fn host(&self) -> &str {
            self.0.host()
        }
        fn https(&self) -> bool {
            self.0.https()
        }
        async fn send(
            &self,
            call: Call,
            body: Option<String>,
            token: Option<String>,
            cookie: Option<String>,
        ) -> Result<Answer, Failure> {
            if matches!(call, Call::Read(_)) {
                assert!(
                    token.is_none(),
                    "GET must not consume a queued POST token pair"
                );
            }
            self.0.send(call, body, token, cookie).await
        }
    }
    let wire = PairWire(Script::new(bootstrap()));
    let mut client = client();
    client
        .read(
            &wire,
            Action::Info,
            Some((USER, PASS)),
            &mut Budget::default(),
        )
        .await
        .unwrap();
    assert!(client.tokens.take().is_some_and(|t| t.ends_with("-one")));
    assert!(client.tokens.take().is_some_and(|t| t.ends_with("-two")));
}
