//! The Voltalis cloud: a session of the household's account (a token from
//! a login), every request on its own connection, never retried here.
//!
//! The address is fixed here: nothing the cloud or an agent says ever
//! chooses where Moli connects.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering::SeqCst};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use http::{Method, Request, StatusCode};
use http_body_util::Full;
use moli_net::Body;
use serde_json::{Value as Json, json};

const HOST: &str = "api.myvoltalis.com";
const LOGIN: &str = "/auth/login";
const LIMIT: Duration = Duration::from_secs(10);
/// A house's appliances and programs are a few kilobytes.
const MAX_ANSWER: usize = 1024 * 1024;
const MAX_TOKEN: usize = 4096;

/// Why the cloud said no, as far as the driver cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The account's identifiers: a person must give them again.
    Denied,
    /// The session ended: the next call logs in again (bounded).
    Session,
    /// Too many requests, or the service is down: leave it alone a while.
    Busy,
    /// What was asked for does not exist (any more).
    Gone,
    Other,
}

/// The cloud's refusal. Its message never quotes the cloud (it may repeat
/// what was sent).
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

#[must_use]
pub fn refused(e: &anyhow::Error) -> Option<Refusal> {
    e.downcast_ref::<Refused>().map(|r| r.kind)
}

/// An order whose time ran out before it left: never sent.
#[derive(Debug)]
pub struct NotSent;

impl std::fmt::Display for NotSent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("not sent: its time ran out")
    }
}

impl std::error::Error for NotSent {}

fn in_time(deadline: Option<Instant>) -> anyhow::Result<()> {
    match deadline {
        Some(deadline) if Instant::now() >= deadline => Err(NotSent.into()),
        _ => Ok(()),
    }
}

fn answer(status: StatusCode, body: &[u8]) -> anyhow::Result<Json> {
    if !status.is_success() {
        let kind = match status.as_u16() {
            401 => Refusal::Session,
            403 => Refusal::Denied,
            404 => Refusal::Gone,
            429 | 502..=504 => Refusal::Busy,
            _ => Refusal::Other,
        };
        let message = format!("Voltalis HTTP {}", status.as_u16());
        return Err(Refused { kind, message }.into());
    }
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(Json::Null);
    }
    serde_json::from_slice(body).context("voltalis: not JSON")
}

/// The session, kept across restarts of the driver: a restart is not a
/// reason to log in again.
#[derive(Debug, Default)]
pub struct Sessions {
    /// Held only to read or write it, never across the network.
    slot: std::sync::Mutex<Option<Session>>,
    /// One login at a time.
    renewal: tokio::sync::Mutex<()>,
    /// Each session gets the next number: a refusal names the session it
    /// refused, and only that one is forgotten (never its replacement).
    generation: AtomicU64,
    /// Sessions refused in a row (sessions, not answers).
    refused: AtomicU32,
}

struct Session {
    generation: u64,
    token: String,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the token: it drives every heater of the house.
        f.debug_struct("Session")
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

impl Sessions {
    fn slot(&self) -> std::sync::MutexGuard<'_, Option<Session>> {
        self.slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn valid(&self) -> Option<(String, u64)> {
        self.slot()
            .as_ref()
            .map(|s| (s.token.clone(), s.generation))
    }
}

/// The HTTP exchange, replaced in tests (nothing else is).
#[cfg(test)]
pub type Exchange = Arc<
    dyn Fn(
            Request<Full<Body>>,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = anyhow::Result<(StatusCode, Body)>> + Send>,
        > + Send
        + Sync,
>;

/// The household's Voltalis account.
pub struct Cloud {
    user: String,
    password: String,
    sessions: Arc<Sessions>,
    #[cfg(test)]
    pub(crate) exchange: Option<Exchange>,
}

impl std::fmt::Debug for Cloud {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cloud").finish_non_exhaustive()
    }
}

impl Cloud {
    #[must_use]
    pub fn new(user: String, password: String, sessions: Arc<Sessions>) -> Self {
        Self {
            user,
            password,
            sessions,
            #[cfg(test)]
            exchange: None,
        }
    }

    /// Sessions refused in a row.
    #[must_use]
    pub fn refused_sessions(&self) -> u32 {
        self.sessions.refused.load(SeqCst)
    }

    /// Ends the session: the next call logs in again.
    pub fn forget(&self) {
        *self.sessions.slot() = None;
    }

    pub async fn get(&self, path: &str) -> anyhow::Result<Json> {
        self.call(Method::GET, path, None, None).await
    }

    /// Hands an order to the cloud, **once** (never retried, whatever
    /// happens), never after `deadline` (checked again after a login).
    pub async fn put(&self, path: &str, body: &Json, deadline: Instant) -> anyhow::Result<()> {
        self.call(Method::PUT, path, Some(body), Some(deadline))
            .await
            .map(drop)
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<&Json>,
        deadline: Option<Instant>,
    ) -> anyhow::Result<Json> {
        in_time(deadline)?;
        let (token, generation) = self.access().await?;
        // Logging in may have taken a while: an order out of time stays home.
        in_time(deadline)?;
        let builder = Request::builder()
            .method(method)
            .uri(path)
            .header("accept", "application/json")
            .header("authorization", format!("Bearer {token}"));
        let request = match body {
            Some(body) => moli_net::json_body(builder, body)?,
            None => moli_net::empty(builder)?,
        };
        let (status, body) = self.exchange(request).await?;
        let result = answer(status, &body);
        match &result {
            Err(e) if refused(e) == Some(Refusal::Session) => self.settle(generation, true),
            Ok(_) => self.settle(generation, false),
            Err(_) => {}
        }
        result
    }

    /// What an answer says of the session it used: refused, that session
    /// (only) is forgotten and counted; accepted, the count starts again.
    fn settle(&self, generation: u64, refused: bool) {
        let mut slot = self.sessions.slot();
        if slot.as_ref().is_none_or(|s| s.generation != generation) {
            return;
        }
        if refused {
            *slot = None;
            self.sessions.refused.fetch_add(1, SeqCst);
        } else {
            self.sessions.refused.store(0, SeqCst);
        }
    }

    /// The current session, or a new one (one login at a time).
    async fn access(&self) -> anyhow::Result<(String, u64)> {
        if let Some(valid) = self.sessions.valid() {
            return Ok(valid);
        }
        let _renewal = self.sessions.renewal.lock().await;
        if let Some(valid) = self.sessions.valid() {
            return Ok(valid);
        }
        let token = self.login().await?;
        let generation = self.sessions.generation.fetch_add(1, SeqCst) + 1;
        *self.sessions.slot() = Some(Session {
            generation,
            token: token.clone(),
        });
        Ok((token, generation))
    }

    async fn login(&self) -> anyhow::Result<String> {
        let builder = Request::builder()
            .method(Method::POST)
            .uri(LOGIN)
            .header("accept", "application/json");
        let request = moli_net::json_body(
            builder,
            &json!({ "login": self.user, "password": self.password }),
        )?;
        let (status, body) = self.exchange(request).await?;
        login_answer(status, &body)
    }

    async fn exchange(&self, request: Request<Full<Body>>) -> anyhow::Result<(StatusCode, Body)> {
        #[cfg(test)]
        if let Some(exchange) = &self.exchange {
            return exchange(request).await;
        }
        // One connection per request, never retried by `web`.
        moli_net::web(HOST, 443, true, request, LIMIT, MAX_ANSWER).await
    }
}

/// The login's answer: a token, or why not. Wrong identifiers are named,
/// never quoted.
fn login_answer(status: StatusCode, body: &[u8]) -> anyhow::Result<String> {
    if status.is_success() {
        let json: Json = serde_json::from_slice(body).unwrap_or_default();
        let token = json["token"]
            .as_str()
            .filter(|t| !t.is_empty() && t.len() <= MAX_TOKEN)
            .filter(|t| t.bytes().all(|b| b.is_ascii_graphic()))
            .context("voltalis: no session in the login answer")?;
        return Ok(token.to_owned());
    }
    let kind = match status.as_u16() {
        400 | 401 | 403 => Refusal::Denied,
        429 | 502..=504 => Refusal::Busy,
        _ => Refusal::Other,
    };
    Err(Refused {
        kind,
        message: format!("Voltalis login HTTP {}", status.as_u16()),
    }
    .into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logins() {
        assert_eq!(
            login_answer(StatusCode::OK, br#"{"token":"SYNTH-T"}"#).unwrap(),
            "SYNTH-T"
        );
        assert!(login_answer(StatusCode::OK, br#"{"token":"a b"}"#).is_err());
        assert!(login_answer(StatusCode::OK, b"{}").is_err());
        let denied = login_answer(
            StatusCode::UNAUTHORIZED,
            b"bad password for user@example.com",
        )
        .unwrap_err();
        assert_eq!(refused(&denied), Some(Refusal::Denied));
        assert!(!denied.to_string().contains("example"), "never quoted");
        let busy = login_answer(StatusCode::TOO_MANY_REQUESTS, b"").unwrap_err();
        assert_eq!(refused(&busy), Some(Refusal::Busy));
    }

    #[test]
    fn answers() {
        assert_eq!(answer(StatusCode::OK, b"").unwrap(), Json::Null);
        assert_eq!(answer(StatusCode::OK, b"[1]").unwrap(), json!([1]));
        assert!(answer(StatusCode::OK, b"<html>").is_err());
        for (status, kind) in [
            (401, Refusal::Session),
            (403, Refusal::Denied),
            (404, Refusal::Gone),
            (429, Refusal::Busy),
            (503, Refusal::Busy),
            (500, Refusal::Other),
        ] {
            let e = answer(StatusCode::from_u16(status).unwrap(), b"x").unwrap_err();
            assert_eq!(refused(&e), Some(kind), "{status}");
        }
    }

    fn installed(cloud: &Cloud, generation: u64) {
        *cloud.sessions.slot() = Some(Session {
            generation,
            token: format!("SYNTH-T{generation}"),
        });
        cloud.sessions.generation.store(generation, SeqCst);
    }

    /// A late refusal about an older session never forgets the new one,
    /// and several answers refusing one session count once.
    #[test]
    fn refusals_name_their_session() {
        let cloud = Cloud::new("u".into(), "p".into(), Arc::default());
        installed(&cloud, 1);
        cloud.settle(1, true);
        assert!(cloud.sessions.valid().is_none());
        installed(&cloud, 2);
        cloud.settle(1, true);
        assert_eq!(cloud.sessions.valid().map(|(_, g)| g), Some(2));
        assert_eq!(cloud.refused_sessions(), 1);
        cloud.settle(2, false);
        assert_eq!(cloud.refused_sessions(), 0);
        assert!(
            !format!("{:?}", cloud.sessions).contains("SYNTH"),
            "never in logs"
        );
    }
}
