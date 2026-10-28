//! The two roads to an Overkiz box: its local API (developer mode, on the
//! home network) and the Somfy cloud (the only one a Connectivity Kit has).
//! Both speak the same API; they differ in where it lives and who vouches
//! for the caller (a token a human made, or a Somfy account session).
//!
//! Every address is fixed here or given by a human in the configuration:
//! nothing the box or an agent says ever chooses where Moli connects.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use http::{Method, Request, StatusCode};
use http_body_util::Full;
use moli_net::{Body, PinnedHttps};
use moli_runtime::SendPermit;
use serde_json::Value as Json;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering::SeqCst};

use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::model::{Order, Refusal, refusal};

const LOCAL_ROOT: &str = "/enduser-mobile-web/1/enduserAPI";
/// Somfy Europe's Overkiz server.
const CLOUD_HOST: &str = "ha101-1.overkiz.com";
const CLOUD_ROOT: &str = "/enduser-mobile-web/enduserAPI";
const ACCOUNTS_HOST: &str = "accounts.somfy.com";
const TOKEN_PATH: &str = "/oauth/oauth/v2/token/jwt";
/// The Somfy app's own OAuth client, public in every Overkiz client
/// (pyoverkiz, MIT): it names the application, not the household.
const CLIENT_ID: &str =
    "0d8e920c-1478-11e7-a377-02dd59bd3041_1ewvaqmclfogo4kcsoo0c8k4kso884owg08sg8c40sk4go4ksg";
const CLIENT_SECRET: &str = "12k73w1n540g8o4cokg0cw84cog840k84cwggscwg884004kgk";

const CLOUD_LIMIT: Duration = Duration::from_secs(10);
/// The whole setup of a house is a few dozen kilobytes.
const MAX_ANSWER: usize = 2 * 1024 * 1024;
const MAX_TOKEN_ANSWER: usize = 64 * 1024;
/// A session is renewed this long before Somfy would end it.
const RENEW_EARLY: Duration = Duration::from_secs(120);
/// Identifiers the box hands back and Moli puts in a path or matches.
const MAX_ID: usize = 64;

/// An order whose time ran out before it left: never sent.
#[derive(Debug)]
pub struct NotSent;

impl std::fmt::Display for NotSent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("not sent: its time ran out")
    }
}

impl std::error::Error for NotSent {}

fn in_time(deadline: Option<std::time::Instant>) -> anyhow::Result<()> {
    match deadline {
        Some(deadline) if std::time::Instant::now() >= deadline => Err(NotSent.into()),
        _ => Ok(()),
    }
}

/// The box (or the cloud) said no.
#[derive(Debug)]
pub struct Refused {
    pub kind: Refusal,
    pub message: String,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Refused {}

/// What kind of refusal an error is, if it is one.
#[must_use]
pub fn refused(e: &anyhow::Error) -> Option<Refusal> {
    e.downcast_ref::<Refused>().map(|r| r.kind)
}

fn answer(status: StatusCode, body: &[u8]) -> anyhow::Result<Json> {
    if !status.is_success() {
        let (kind, message) = refusal(status.as_u16(), body);
        return Err(Refused { kind, message }.into());
    }
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(Json::Null);
    }
    serde_json::from_slice(body).context("overkiz: not JSON")
}

/// An identifier from the box: short, plain, safe in a path.
pub fn identifier(value: &Json) -> anyhow::Result<String> {
    let id = value.as_str().context("overkiz: no identifier")?;
    anyhow::ensure!(
        !id.is_empty()
            && id.len() <= MAX_ID
            && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'),
        "overkiz: unexpected identifier"
    );
    Ok(id.to_owned())
}

/// The cloud session, kept across restarts of the driver: a restart is not
/// a reason to log in again (Somfy bans accounts that log in too often).
#[derive(Debug, Default)]
pub struct Sessions {
    /// Held only to read or write it, never across the network: an answer
    /// already received is never kept waiting behind a login.
    slot: std::sync::Mutex<Option<Session>>,
    /// One renewal at a time (the login itself, over the network).
    renewal: Mutex<()>,
    /// Each session gets the next number: a refusal names the session it
    /// refused, and only that one is forgotten (never its replacement).
    generation: AtomicU64,
    /// Sessions refused in a row — sessions, not answers: several answers
    /// refusing one session count once.
    refused: AtomicU32,
}

pub type SharedSession = Arc<Sessions>;

impl Sessions {
    fn slot(&self) -> std::sync::MutexGuard<'_, Option<Session>> {
        self.slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The current session, if still valid: its token and generation.
    fn valid(&self) -> Option<(String, u64)> {
        self.slot()
            .as_ref()
            .filter(|s| Instant::now() < s.renew_at)
            .map(|s| (s.access.clone(), s.generation))
    }
}

/// The HTTP exchange of the cloud, replaced in tests (nothing else is).
#[cfg(test)]
pub type Exchange = Arc<
    dyn Fn(
            Request<Full<Body>>,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = anyhow::Result<(StatusCode, Body)>> + Send>,
        > + Send
        + Sync,
>;

pub struct Session {
    generation: u64,
    access: String,
    refresh: Option<String>,
    renew_at: Instant,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("renew_at", &self.renew_at)
            .finish_non_exhaustive()
    }
}

/// One box, by one road.
#[derive(Debug)]
pub enum Api {
    Local(Box<Local>),
    Cloud(Cloud),
    /// A box simulated in memory, for the driver's own tests.
    #[cfg(test)]
    Fake(std::sync::Arc<crate::fake::Fake>),
}

impl Api {
    pub async fn get(&self, path: &str) -> anyhow::Result<Json> {
        self.call(Method::GET, path, None, false, None).await
    }

    /// A `POST` without body, sent **once**: a fetch takes the events out
    /// of the box's buffer, so a repeat after a lost answer would come back
    /// empty and an execution's failure would vanish unseen.
    pub async fn post(&self, path: &str) -> anyhow::Result<Json> {
        self.call(Method::POST, path, None, true, None).await
    }

    /// Hands an order to the box, **once**: never retried, whatever
    /// happens (a gate opened twice is not the same as opened once), and
    /// never after `deadline` (checked again after any wait for a session).
    /// Returns the execution's id, to follow it.
    pub async fn apply(
        &self,
        order: &Order,
        deadline: std::time::Instant,
    ) -> anyhow::Result<String> {
        let answer = self
            .call(
                Method::POST,
                "/exec/apply",
                Some(&order.body()),
                true,
                Some(deadline),
            )
            .await?;
        identifier(&answer["execId"])
    }

    /// The driver's live authorization survives OAuth, TCP and TLS waits.
    /// Revocation before HTTP handoff is unsent; after it the answer is retained.
    pub async fn apply_with_permit(
        &self,
        order: &Order,
        permit: &SendPermit,
    ) -> anyhow::Result<String> {
        let wanted = || permit.wanted();
        let begin_send = || permit.begin_send();
        let guard = moli_net::WebGuard {
            wanted: &wanted,
            begin_send: &begin_send,
        };
        let deadline = permit.deadline();
        let apply = async {
            if !(guard.wanted)() {
                return Err(NotSent.into());
            }
            let answer = match self {
                Self::Local(local) => local.apply_guarded(order, deadline, &guard).await?,
                Self::Cloud(cloud) => {
                    cloud
                        .call_guarded(
                            Method::POST,
                            "/exec/apply",
                            Some(&order.body()),
                            Some(deadline),
                            Some(&guard),
                        )
                        .await?
                }
                #[cfg(test)]
                Self::Fake(fake) => {
                    fake.call_guarded("/exec/apply", Some(&order.body()), true, Some(&guard))
                        .await?
                }
            };
            identifier(&answer["execId"])
        };
        tokio::time::timeout_at(deadline.into(), apply)
            .await
            .unwrap_or_else(|_| {
                Err(if permit.emitted() {
                    crate::NoAnswer.into()
                } else {
                    NotSent.into()
                })
            })
    }

    /// Cloud sessions refused in a row (0 for a local box).
    #[must_use]
    pub fn refused_sessions(&self) -> u32 {
        match self {
            Self::Cloud(cloud) => cloud.session.refused.load(SeqCst),
            Self::Local(_) => 0,
            #[cfg(test)]
            Self::Fake(_) => 0,
        }
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<&Json>,
        once: bool,
        deadline: Option<std::time::Instant>,
    ) -> anyhow::Result<Json> {
        in_time(deadline)?;
        match self {
            Self::Local(local) => local.call(method, path, body, once).await,
            Self::Cloud(cloud) => cloud.call(method, path, body, deadline).await,
            #[cfg(test)]
            Self::Fake(fake) => fake.call(path, body, once).await,
        }
    }
}

fn build(
    builder: http::request::Builder,
    body: Option<&Json>,
) -> anyhow::Result<Request<Full<Body>>> {
    let builder = builder.header("accept", "application/json");
    match body {
        Some(body) => moli_net::json_body(builder, body),
        // The box answers 411 to a bodiless POST without a length
        // (`/events/register`, `/events/{id}/fetch`, seen on 2026.4.5-7).
        None if builder.method_ref() == Some(&Method::POST) => {
            moli_net::empty(builder.header("content-length", "0"))
        }
        None => moli_net::empty(builder),
    }
}

/// The box's local API: HTTPS on the LAN, its certificate pinned at first
/// contact, a bearer token a human made (developer mode).
pub struct Local {
    /// Fetches and loads, on their own connection…
    reads: PinnedHttps,
    /// … so an order never waits behind a slow fetch.
    writes: PinnedHttps,
    token: String,
}

impl std::fmt::Debug for Local {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the token: it opens every shutter of the house.
        f.debug_struct("Local")
            .field("http", &self.reads)
            .finish_non_exhaustive()
    }
}

impl Local {
    /// `pin`: the box's certificate fingerprint, known before the token
    /// ever leaves (see [`Local::first_contact`]).
    pub fn new(host: &str, port: u16, pin: String, token: String) -> anyhow::Result<Self> {
        Ok(Self {
            reads: PinnedHttps::new(host, port, Some(pin.clone()))?,
            writes: PinnedHttps::new(host, port, Some(pin))?,
            token,
        })
    }

    /// A bare handshake, nothing sent: the fingerprint to store first.
    pub async fn first_contact(host: &str, port: u16) -> anyhow::Result<String> {
        let probe = PinnedHttps::new(host, port, None)?;
        probe.handshake().await.context("first contact")?;
        probe.pinned_fingerprint().context("no certificate seen")
    }

    async fn apply_guarded(
        &self,
        order: &Order,
        deadline: std::time::Instant,
        guard: &moli_net::WebGuard<'_>,
    ) -> anyhow::Result<Json> {
        let request = build(
            self.writes
                .request(Method::POST, &format!("{LOCAL_ROOT}/exec/apply"))
                .header("authorization", format!("Bearer {}", self.token)),
            Some(&order.body()),
        )?;
        let (status, body) = self
            .writes
            .send_once_guarded(request, deadline, || (guard.begin_send)())
            .await
            .map_err(guarded_error)?;
        answer(status, &body)
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<&Json>,
        once: bool,
    ) -> anyhow::Result<Json> {
        let http = if path == "/exec/apply" {
            &self.writes
        } else {
            &self.reads
        };
        let builder = http
            .request(method, &format!("{LOCAL_ROOT}{path}"))
            .header("authorization", format!("Bearer {}", self.token));
        let request = build(builder, body)?;
        let (status, body) = if once {
            http.send_once(request).await?
        } else {
            http.send(request).await?
        };
        answer(status, &body)
    }
}

/// The Somfy cloud, with the household's Somfy account.
pub struct Cloud {
    user: String,
    password: String,
    session: SharedSession,
    #[cfg(test)]
    exchange: Option<Exchange>,
}

impl std::fmt::Debug for Cloud {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cloud").finish_non_exhaustive()
    }
}

/// `application/x-www-form-urlencoded`.
fn form(fields: &[(&str, &str)]) -> String {
    let encode = |s: &str| {
        s.bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                    char::from(b).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect::<String>()
    };
    fields
        .iter()
        .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

impl Cloud {
    #[must_use]
    pub fn new(user: String, password: String, session: SharedSession) -> Self {
        Self {
            user,
            password,
            session,
            #[cfg(test)]
            exchange: None,
        }
    }

    /// Ends the session: the next call logs in again.
    pub fn forget(&self) {
        *self.session.slot() = None;
    }

    /// What an answer says of the session generation `generation` it used:
    /// refused, that session (only) is forgotten and counted; accepted, the
    /// count starts again. A late answer about an older session changes
    /// nothing. Never waits for a login in progress.
    fn settle(&self, generation: u64, refused: bool) {
        let mut slot = self.session.slot();
        if slot.as_ref().is_none_or(|s| s.generation != generation) {
            return;
        }
        if refused {
            *slot = None;
            self.session.refused.fetch_add(1, SeqCst);
        } else {
            self.session.refused.store(0, SeqCst);
        }
    }

    /// A valid access token: the current one, renewed before it ends (by
    /// its refresh token, else by logging in again), one renewal at a time.
    async fn access(&self) -> anyhow::Result<(String, u64)> {
        if let Some(valid) = self.session.valid() {
            return Ok(valid);
        }
        let _renewal = self.session.renewal.lock().await;
        // Renewed by another request meanwhile.
        if let Some(valid) = self.session.valid() {
            return Ok(valid);
        }
        let refresh = self.session.slot().as_ref().and_then(|s| s.refresh.clone());
        let renewed = match refresh {
            Some(refresh) => self
                .token(&[("grant_type", "refresh_token"), ("refresh_token", &refresh)])
                .await
                .inspect_err(
                    |e| tracing::debug!(error = %e, "overkiz session not renewed, logging in"),
                )
                .ok(),
            None => None,
        };
        let mut session = if let Some(session) = renewed {
            session
        } else {
            self.forget();
            self.token(&[
                ("grant_type", "password"),
                ("username", &self.user),
                ("password", &self.password),
            ])
            .await?
        };
        session.generation = self.session.generation.fetch_add(1, SeqCst) + 1;
        let access = (session.access.clone(), session.generation);
        *self.session.slot() = Some(session);
        Ok(access)
    }

    async fn token(&self, grant: &[(&str, &str)]) -> anyhow::Result<Session> {
        let mut fields = grant.to_vec();
        fields.extend([("client_id", CLIENT_ID), ("client_secret", CLIENT_SECRET)]);
        let request = Request::builder()
            .method(Method::POST)
            .uri(TOKEN_PATH)
            .header("content-type", "application/x-www-form-urlencoded")
            .header("accept", "application/json")
            .body(Full::new(Body::from(form(&fields))))?;
        let (status, body) = moli_net::web(
            ACCOUNTS_HOST,
            443,
            true,
            request,
            CLOUD_LIMIT,
            MAX_TOKEN_ANSWER,
        )
        .await?;
        session(status, &body)
    }

    async fn exchange(&self, request: Request<Full<Body>>) -> anyhow::Result<(StatusCode, Body)> {
        #[cfg(test)]
        if let Some(exchange) = &self.exchange {
            return exchange(request).await;
        }
        // One connection per request, never retried by `web`.
        moli_net::web(CLOUD_HOST, 443, true, request, CLOUD_LIMIT, MAX_ANSWER).await
    }

    async fn exchange_guarded(
        &self,
        request: Request<Full<Body>>,
        deadline: std::time::Instant,
        guard: &moli_net::WebGuard<'_>,
    ) -> anyhow::Result<(StatusCode, Body)> {
        #[cfg(test)]
        if let Some(exchange) = &self.exchange {
            if !(guard.begin_send)() {
                return Err(NotSent.into());
            }
            return exchange(request).await;
        }
        let remaining = deadline
            .saturating_duration_since(std::time::Instant::now())
            .min(CLOUD_LIMIT);
        let (status, _, body) = moli_net::web_once_guarded(
            CLOUD_HOST,
            443,
            true,
            request,
            remaining,
            MAX_ANSWER,
            || (guard.begin_send)(),
        )
        .await
        .map_err(guarded_error)?;
        Ok((status, body))
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<&Json>,
        deadline: Option<std::time::Instant>,
    ) -> anyhow::Result<Json> {
        self.call_guarded(method, path, body, deadline, None).await
    }

    async fn call_guarded(
        &self,
        method: Method,
        path: &str,
        body: Option<&Json>,
        deadline: Option<std::time::Instant>,
        guard: Option<&moli_net::WebGuard<'_>>,
    ) -> anyhow::Result<Json> {
        if guard.is_some_and(|guard| !(guard.wanted)()) {
            return Err(NotSent.into());
        }
        let (access, generation) = self.access().await?;
        // Logging in may have taken a while: an order out of time stays home.
        in_time(deadline)?;
        if guard.is_some_and(|guard| !(guard.wanted)()) {
            return Err(NotSent.into());
        }
        let builder = Request::builder()
            .method(method)
            .uri(format!("{CLOUD_ROOT}{path}"))
            .header("authorization", format!("Bearer {access}"));
        let request = build(builder, body)?;
        let (status, body) = if let Some(guard) = guard {
            self.exchange_guarded(
                request,
                deadline.context("guarded order needs an absolute deadline")?,
                guard,
            )
            .await?
        } else {
            self.exchange(request).await?
        };
        let result = answer(status, &body);
        match &result {
            Err(e) if refused(e) == Some(Refusal::Session) => self.settle(generation, true),
            Ok(_) => self.settle(generation, false),
            Err(_) => {}
        }
        result
    }
}

fn guarded_error(error: moli_net::OnceError) -> anyhow::Error {
    match error {
        moli_net::OnceError::BeforeSend => NotSent.into(),
        moli_net::OnceError::AfterSend => crate::NoAnswer.into(),
    }
}

/// Somfy's token answer: a session, or why not.
fn session(status: StatusCode, body: &[u8]) -> anyhow::Result<Session> {
    let json: Json = serde_json::from_slice(body).unwrap_or_default();
    if let Some(access) = json["access_token"].as_str().filter(|a| !a.is_empty()) {
        let life = json["expires_in"].as_u64().unwrap_or(3600);
        let renew_in = Duration::from_secs(life)
            .saturating_sub(RENEW_EARLY)
            .max(Duration::from_secs(30));
        return Ok(Session {
            generation: 0,
            access: access.to_owned(),
            refresh: json["refresh_token"].as_str().map(str::to_owned),
            renew_at: Instant::now() + renew_in,
        });
    }
    let raw = json["message"]
        .as_str()
        .or_else(|| json["error_description"].as_str())
        .or_else(|| json["error"].as_str())
        .unwrap_or_default();
    let refused = raw.to_lowercase().contains("invalid") || status == StatusCode::UNAUTHORIZED;
    let kind = if refused {
        Refusal::Denied
    } else if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
        Refusal::Busy
    } else {
        Refusal::Other
    };
    // An account refusal is named, never quoted (it may repeat what was sent).
    let message = if refused {
        format!("invalid_grant (HTTP {status})")
    } else {
        Some(crate::model::redact(raw))
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| format!("HTTP {status}"))
    };
    Err(Refused { kind, message }.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bodiless_post_says_its_length() {
        let post = build(Request::builder().method(Method::POST).uri("/x"), None).unwrap();
        assert_eq!(post.headers()["content-length"], "0");
        let get = build(Request::builder().method(Method::GET).uri("/x"), None).unwrap();
        assert!(get.headers().get("content-length").is_none());
    }

    #[test]
    fn forms_are_encoded() {
        assert_eq!(
            form(&[("username", "user@example.com"), ("password", "a b&c=d/é")]),
            "username=user%40example.com&password=a%20b%26c%3Dd%2F%C3%A9"
        );
    }

    #[test]
    fn identifiers_from_the_box_are_checked() {
        assert_eq!(identifier(&Json::from("0a1b-2c3d")).unwrap(), "0a1b-2c3d");
        for bad in ["", "../setup", "a b", &"a".repeat(65)] {
            assert!(identifier(&Json::from(bad)).is_err(), "{bad:?}");
        }
        assert!(identifier(&Json::Null).is_err());
    }

    #[test]
    fn somfy_sessions() {
        let ok = session(
            StatusCode::OK,
            br#"{"access_token":"SECRET-A","refresh_token":"SECRET-R","expires_in":3600}"#,
        )
        .unwrap();
        assert_eq!(
            (ok.access.as_str(), ok.refresh.as_deref()),
            ("SECRET-A", Some("SECRET-R"))
        );
        assert!(ok.renew_at > Instant::now() + Duration::from_secs(3000));
        assert!(!format!("{ok:?}").contains("SECRET"), "never in logs");

        let short = session(StatusCode::OK, br#"{"access_token":"a","expires_in":10}"#).unwrap();
        assert!(short.renew_at >= Instant::now() + Duration::from_secs(29));

        let denied = session(
            StatusCode::BAD_REQUEST,
            br#"{"message":"error.invalid.grant","data":[]}"#,
        )
        .unwrap_err();
        assert_eq!(refused(&denied), Some(Refusal::Denied));
        let busy = session(StatusCode::SERVICE_UNAVAILABLE, b"<html>").unwrap_err();
        assert_eq!(refused(&busy), Some(Refusal::Busy));
    }

    #[test]
    fn answers() {
        assert_eq!(answer(StatusCode::OK, b"").unwrap(), Json::Null);
        assert_eq!(answer(StatusCode::OK, br#"{"id":"x"}"#).unwrap()["id"], "x");
        let e = answer(
            StatusCode::BAD_REQUEST,
            br#"{"errorCode":"UNSPECIFIED_ERROR","error":"Invalid event listener id"}"#,
        )
        .unwrap_err();
        assert_eq!(refused(&e), Some(Refusal::Listener));
        assert_eq!(e.to_string(), "Invalid event listener id");
    }

    fn installed(cloud: &Cloud, generation: u64) {
        *cloud.session.slot() = Some(Session {
            generation,
            access: format!("SYNTH-T{generation}"),
            refresh: None,
            renew_at: Instant::now() + Duration::from_secs(3600),
        });
        cloud.session.generation.store(generation, SeqCst);
    }

    fn current(cloud: &Cloud) -> Option<u64> {
        cloud.session.slot().as_ref().map(|s| s.generation)
    }

    /// A cloud whose HTTP exchange answers `answer(path)`, each request
    /// held until the test releases one permit.
    fn held(
        answer: fn(&str) -> (StatusCode, &'static [u8]),
    ) -> (Cloud, Arc<tokio::sync::Semaphore>, Arc<AtomicU32>) {
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let entered = Arc::new(AtomicU32::new(0));
        let mut cloud = Cloud::new(
            "user@example.com".into(),
            "x".into(),
            SharedSession::default(),
        );
        let (g, e) = (Arc::clone(&gate), Arc::clone(&entered));
        cloud.exchange = Some(Arc::new(move |request: Request<Full<Body>>| {
            let (gate, entered) = (Arc::clone(&g), Arc::clone(&e));
            Box::pin(async move {
                assert_eq!(request.headers()["authorization"], "Bearer SYNTH-T1");
                entered.fetch_add(1, SeqCst);
                gate.acquire().await?.forget();
                let (status, body) = answer(request.uri().path());
                Ok((status, Body::from_static(body)))
            })
        }));
        installed(&cloud, 1);
        (cloud, gate, entered)
    }

    async fn until(check: impl Fn() -> bool) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while !check() {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .expect("in time");
    }

    #[tokio::test]
    async fn revocation_while_waiting_for_oauth_never_hands_off_apply() {
        use std::sync::atomic::AtomicBool;
        let (cloud, _response, exchanged) = held(|_| (StatusCode::OK, br#"{"execId":"exec-1"}"#));
        cloud.forget();
        let cloud = Arc::new(cloud);
        let renewing = cloud.session.renewal.lock().await;
        let allowed = Arc::new(AtomicBool::new(true));
        let waiting = Arc::new(tokio::sync::Notify::new());
        let (client, live, started) = (cloud.clone(), allowed.clone(), waiting.clone());
        let applying = tokio::spawn(async move {
            let wanted = || {
                started.notify_one();
                live.load(SeqCst)
            };
            let begin_send = || live.load(SeqCst);
            let guard = moli_net::WebGuard {
                wanted: &wanted,
                begin_send: &begin_send,
            };
            client
                .call_guarded(
                    Method::POST,
                    "/exec/apply",
                    Some(&serde_json::json!({})),
                    Some(std::time::Instant::now() + Duration::from_secs(2)),
                    Some(&guard),
                )
                .await
        });
        waiting.notified().await;
        allowed.store(false, SeqCst);
        installed(&cloud, 2);
        drop(renewing);
        let error = tokio::time::timeout(Duration::from_secs(1), applying)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert!(error.is::<NotSent>());
        assert_eq!(exchanged.load(SeqCst), 0, "no /exec/apply after OAuth wait");
    }

    #[tokio::test]
    async fn a_guarded_cloud_answer_is_kept_after_post_emission_revocation() {
        use std::sync::atomic::AtomicBool;
        let (cloud, response, exchanged) = held(|_| (StatusCode::OK, br#"{"execId":"exec-1"}"#));
        let allowed = Arc::new(AtomicBool::new(true));
        let emitted = Arc::new(AtomicBool::new(false));
        let (live, sent) = (allowed.clone(), emitted.clone());
        let applying = tokio::spawn(async move {
            let wanted = || live.load(SeqCst);
            let begin_send = || {
                sent.store(true, SeqCst);
                live.load(SeqCst)
            };
            let guard = moli_net::WebGuard {
                wanted: &wanted,
                begin_send: &begin_send,
            };
            cloud
                .call_guarded(
                    Method::POST,
                    "/exec/apply",
                    Some(&serde_json::json!({})),
                    Some(std::time::Instant::now() + Duration::from_secs(2)),
                    Some(&guard),
                )
                .await
        });
        until(|| exchanged.load(SeqCst) == 1).await;
        assert!(emitted.load(SeqCst));
        allowed.store(false, SeqCst);
        response.add_permits(1);
        assert_eq!(applying.await.unwrap().unwrap()["execId"], "exec-1");
        assert_eq!(exchanged.load(SeqCst), 1, "no retry");
    }

    /// Two requests leave with session 1 (through the real `call`); the
    /// first refusal forgets it, session 2 is made; the second refusal,
    /// about 1, must not touch 2, and counts as the same refused session.
    #[tokio::test]
    async fn a_late_refusal_never_forgets_the_new_session() {
        let (cloud, gate, entered) = held(|_| {
            (
                StatusCode::UNAUTHORIZED,
                br#"{"errorCode":"UNSPECIFIED_ERROR","error":"Invalid token"}"#,
            )
        });
        let cloud = Arc::new(cloud);
        let mut calls = tokio::task::JoinSet::new();
        for path in ["/first", "/late"] {
            let cloud = Arc::clone(&cloud);
            calls.spawn(async move { cloud.call(Method::GET, path, None, None).await });
        }
        until(|| entered.load(SeqCst) == 2).await;
        gate.add_permits(1);
        let one = calls.join_next().await.unwrap().unwrap();
        assert_eq!(refused(&one.unwrap_err()), Some(Refusal::Session));
        assert_eq!(current(&cloud), None, "session 1 forgotten");
        installed(&cloud, 2);
        gate.add_permits(1);
        let other = calls.join_next().await.unwrap().unwrap();
        assert_eq!(refused(&other.unwrap_err()), Some(Refusal::Session));
        assert_eq!(current(&cloud), Some(2), "the new session is kept");
        let api = Api::Cloud(Arc::try_unwrap(cloud).ok().unwrap());
        assert_eq!(
            api.refused_sessions(),
            1,
            "one session refused, not two answers"
        );
        let Api::Cloud(cloud) = &api else {
            unreachable!()
        };
        cloud.settle(2, false);
        assert_eq!(api.refused_sessions(), 0, "an accepted answer starts again");
        cloud.settle(2, true);
        installed(cloud, 3);
        cloud.settle(3, true);
        assert_eq!(api.refused_sessions(), 2, "two sessions refused in a row");
    }

    /// The box answered the order in time: a login in progress elsewhere
    /// never holds that answer back.
    #[tokio::test]
    async fn an_answer_never_waits_for_a_login() {
        let (cloud, gate, entered) = held(|_| (StatusCode::OK, br#"{"execId":"exec-1"}"#));
        let api = Arc::new(Api::Cloud(cloud));
        let Api::Cloud(inner) = &*api else {
            unreachable!()
        };
        // Another request is renewing the session over the network.
        let renewing = inner.session.renewal.lock().await;
        let order = Order {
            url: "ogp://1234-5678-9012/00000003".into(),
            command: "on",
            parameters: Vec::new(),
        };
        let deadline = std::time::Instant::now() + Duration::from_millis(300);
        let sender = Arc::clone(&api);
        let sent = tokio::spawn(async move { sender.apply(&order, deadline).await });
        until(|| entered.load(SeqCst) == 1).await;
        gate.add_permits(1);
        let execution = tokio::time::timeout(Duration::from_millis(250), sent)
            .await
            .expect("answered before the deadline")
            .unwrap()
            .unwrap();
        assert_eq!(execution, "exec-1");
        drop(renewing);
    }
}
