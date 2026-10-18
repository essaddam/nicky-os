//! The household's Worx account: an OAuth session (`id.worx.com`) and the
//! list of its mowers (`api.worxlandroid.com`). Both addresses are fixed
//! here (D5); the broker address the list gives is checked by
//! [`crate::model::broker`] before Moli connects to it.
//!
//! Worx limits how often an account may call its API: the session survives
//! restarts of the driver, is renewed by its refresh token, and the list is
//! read only when Moli connects (see the driver).

use std::sync::Arc;
use std::time::Duration;

use http::{Method, Request, StatusCode};
use http_body_util::Full;
use moli_net::Body;
use serde_json::Value as Json;
use tokio::time::Instant;

use crate::model::{self, Mower};

const AUTH_HOST: &str = "id.worx.com";
const API_HOST: &str = "api.worxlandroid.com";
/// The Worx app's own OAuth client, public in every Landroid client: it
/// names the application, not the household.
const CLIENT_ID: &str = "150da4d2-bb44-433b-9429-3773adc70a2a";
const LIMIT: Duration = Duration::from_secs(10);
const MAX_TOKEN_ANSWER: usize = 64 * 1024;
const MAX_LIST: usize = 1024 * 1024;
/// A session is renewed this long before Worx would end it.
const RENEW_EARLY: Duration = Duration::from_secs(300);

/// What kind of refusal stopped a call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The account's user name or password: a person must look.
    Credentials,
    /// The session ended: log in again.
    Session,
    /// Too many calls: wait (long) before the next one.
    TooMany,
    Other,
}

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
pub fn refusal(e: &anyhow::Error) -> Option<Refusal> {
    e.downcast_ref::<Refused>().map(|r| r.kind)
}

struct Session {
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

/// The session, kept across restarts of the driver (a restart is not a
/// reason to log in again). One caller at a time: the driver's own loop.
// ponytail: a plain mutex, never held across the network; a generation
// count (as Overkiz has) when two tasks may call at once.
#[derive(Debug, Default)]
pub struct Sessions(std::sync::Mutex<Option<Session>>);

impl Sessions {
    fn slot(&self) -> std::sync::MutexGuard<'_, Option<Session>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// The HTTP exchange, replaced in tests (nothing else is).
#[cfg(test)]
pub type Exchange = Arc<
    dyn Fn(
            &'static str,
            Request<Full<Body>>,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = anyhow::Result<(StatusCode, Body)>> + Send>,
        > + Send
        + Sync,
>;

pub struct Account {
    user: String,
    password: String,
    sessions: Arc<Sessions>,
    #[cfg(test)]
    pub exchange: Option<Exchange>,
}

impl std::fmt::Debug for Account {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Account").finish_non_exhaustive()
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

impl Account {
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

    /// Ends the session: the next call logs in again.
    pub fn forget(&self) {
        *self.sessions.slot() = None;
    }

    /// A valid access token and when to renew it: the current one, renewed
    /// before it ends (by its refresh token, else by logging in again).
    pub async fn access(&self) -> anyhow::Result<(String, Instant)> {
        let (current, refresh) = {
            let slot = self.sessions.slot();
            let current = slot
                .as_ref()
                .filter(|s| Instant::now() < s.renew_at)
                .map(|s| (s.access.clone(), s.renew_at));
            (current, slot.as_ref().and_then(|s| s.refresh.clone()))
        };
        if let Some(current) = current {
            return Ok(current);
        }
        let renewed = match refresh {
            Some(refresh) => self
                .token(&[("grant_type", "refresh_token"), ("refresh_token", &refresh)])
                .await
                .inspect_err(|e| tracing::debug!(error = %e, "landroid session not renewed"))
                .ok(),
            None => None,
        };
        let session = if let Some(session) = renewed {
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
        let access = (session.access.clone(), session.renew_at);
        *self.sessions.slot() = Some(session);
        Ok(access)
    }

    async fn token(&self, grant: &[(&str, &str)]) -> anyhow::Result<Session> {
        let mut fields = grant.to_vec();
        fields.extend([("client_id", CLIENT_ID), ("scope", "*")]);
        let request = Request::builder()
            .method(Method::POST)
            .uri("/oauth/token")
            .header("content-type", "application/x-www-form-urlencoded")
            .header("accept", "application/json")
            .body(Full::new(Body::from(form(&fields))))?;
        let (status, body) = self.exchange(AUTH_HOST, request, MAX_TOKEN_ANSWER).await?;
        session(status, &body)
    }

    /// The mowers of the account Moli can drive. A refused session is
    /// forgotten (the next call logs in again).
    pub async fn mowers(&self) -> anyhow::Result<Vec<Mower>> {
        let (access, _) = self.access().await?;
        let request = Request::builder()
            .method(Method::GET)
            .uri("/api/v2/product-items?status=1")
            .header("authorization", format!("Bearer {access}"))
            .header("accept", "application/json")
            .body(Full::new(Body::new()))?;
        let (status, body) = self.exchange(API_HOST, request, MAX_LIST).await?;
        if !status.is_success() {
            let kind = match status {
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                    self.forget();
                    Refusal::Session
                }
                StatusCode::TOO_MANY_REQUESTS => Refusal::TooMany,
                _ => Refusal::Other,
            };
            return Err(Refused {
                kind,
                message: format!("Worx HTTP {}", status.as_u16()),
            }
            .into());
        }
        model::mowers(&body)
    }

    async fn exchange(
        &self,
        host: &'static str,
        request: Request<Full<Body>>,
        max: usize,
    ) -> anyhow::Result<(StatusCode, Body)> {
        #[cfg(test)]
        if let Some(exchange) = &self.exchange {
            let _ = max;
            return exchange(host, request).await;
        }
        // One connection per request, never retried by `web`.
        moli_net::web(host, 443, true, request, LIMIT, max).await
    }
}

/// The token answer: a session, or why not. A refusal is named, never
/// quoted (it may repeat what was sent).
fn session(status: StatusCode, body: &[u8]) -> anyhow::Result<Session> {
    let json: Json = serde_json::from_slice(body).unwrap_or_default();
    if status.is_success()
        && let Some(access) = json["access_token"].as_str().filter(|a| !a.is_empty())
    {
        let life = json["expires_in"].as_u64().unwrap_or(3600);
        let renew_in = Duration::from_secs(life)
            .saturating_sub(RENEW_EARLY)
            .max(Duration::from_secs(60));
        return Ok(Session {
            access: access.to_owned(),
            refresh: json["refresh_token"]
                .as_str()
                .filter(|r| !r.is_empty())
                .map(str::to_owned),
            renew_at: Instant::now() + renew_in,
        });
    }
    let code = json["error"].as_str().unwrap_or_default();
    let kind = if code == "invalid_grant"
        || code == "invalid_credentials"
        || status == StatusCode::UNAUTHORIZED
    {
        Refusal::Credentials
    } else if status == StatusCode::TOO_MANY_REQUESTS {
        Refusal::TooMany
    } else {
        Refusal::Other
    };
    let code: String = code
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .take(32)
        .collect();
    Err(Refused {
        kind,
        message: format!("Worx {} {code}", status.as_u16())
            .trim_end()
            .to_owned(),
    }
    .into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn forms_are_encoded() {
        assert_eq!(
            form(&[("username", "user@example.com"), ("password", "a b&c=d/é")]),
            "username=user%40example.com&password=a%20b%26c%3Dd%2F%C3%A9"
        );
    }

    #[test]
    fn sessions_and_refusals() {
        let ok = session(
            StatusCode::OK,
            br#"{"access_token":"SECRET-A","refresh_token":"SECRET-R","expires_in":3600,"token_type":"Bearer"}"#,
        )
        .unwrap();
        assert_eq!(ok.access, "SECRET-A");
        assert_eq!(ok.refresh.as_deref(), Some("SECRET-R"));
        assert!(ok.renew_at > Instant::now() + Duration::from_secs(3000));
        assert!(!format!("{ok:?}").contains("SECRET"), "never in logs");

        let denied = session(
            StatusCode::BAD_REQUEST,
            br#"{"error":"invalid_grant","error_description":"user@example.com wrong","message":"pw"}"#,
        )
        .unwrap_err();
        assert_eq!(refusal(&denied), Some(Refusal::Credentials));
        assert_eq!(denied.to_string(), "Worx 400 invalid_grant");
        let limited = session(StatusCode::TOO_MANY_REQUESTS, b"").unwrap_err();
        assert_eq!(refusal(&limited), Some(Refusal::TooMany));
        let odd = session(
            StatusCode::OK,
            br#"{"error":"<script>very long junk</script>"}"#,
        )
        .unwrap_err();
        assert_eq!(refusal(&odd), Some(Refusal::Other));
        assert!(!odd.to_string().contains('<'));
    }

    /// A Worx in memory: answers by host and path, counts the calls.
    fn account(
        answers: Vec<(&'static str, StatusCode, &'static str)>,
    ) -> (Account, Arc<Mutex<Vec<String>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&calls);
        let answers = Arc::new(Mutex::new(answers));
        let mut account = Account::new("user@example.com".into(), "pw".into(), Arc::default());
        account.exchange = Some(Arc::new(move |host, request: Request<Full<Body>>| {
            let call = format!("{host}{}", request.uri());
            seen.lock().unwrap().push(call.clone());
            let next = {
                let mut answers = answers.lock().unwrap();
                let i = answers
                    .iter()
                    .position(|(p, _, _)| call.contains(p))
                    .expect("an answer");
                answers.remove(i)
            };
            Box::pin(async move { Ok((next.1, Body::from(next.2))) })
        }));
        (account, calls)
    }

    #[tokio::test]
    async fn the_session_is_reused_then_a_refused_one_logs_in_again() {
        let (account, calls) = account(vec![
            (
                "/oauth/token",
                StatusCode::OK,
                r#"{"access_token":"a.b.c","refresh_token":"r","expires_in":3600}"#,
            ),
            ("product-items", StatusCode::OK, "[]"),
            ("product-items", StatusCode::UNAUTHORIZED, ""),
            (
                "/oauth/token",
                StatusCode::OK,
                r#"{"access_token":"d.e.f","expires_in":3600}"#,
            ),
            ("product-items", StatusCode::OK, "[]"),
        ]);
        assert!(account.mowers().await.unwrap().is_empty());
        let refused = account.mowers().await.unwrap_err();
        assert_eq!(refusal(&refused), Some(Refusal::Session));
        assert!(account.mowers().await.unwrap().is_empty());
        let calls = calls.lock().unwrap().clone();
        assert_eq!(
            calls,
            [
                "id.worx.com/oauth/token",
                "api.worxlandroid.com/api/v2/product-items?status=1",
                "api.worxlandroid.com/api/v2/product-items?status=1",
                "id.worx.com/oauth/token",
                "api.worxlandroid.com/api/v2/product-items?status=1",
            ]
        );
    }
}
