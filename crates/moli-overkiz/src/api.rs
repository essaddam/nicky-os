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
use serde_json::Value as Json;
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
pub type SharedSession = Arc<Mutex<Option<Session>>>;

pub struct Session {
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
        self.call(Method::GET, path, None, false).await
    }

    /// A `POST` without body, sent **once**: a fetch takes the events out
    /// of the box's buffer, so a repeat after a lost answer would come back
    /// empty and an execution's failure would vanish unseen.
    pub async fn post(&self, path: &str) -> anyhow::Result<Json> {
        self.call(Method::POST, path, None, true).await
    }

    /// Hands an order to the box, **once**: never retried, whatever
    /// happens (a gate opened twice is not the same as opened once).
    /// Returns the execution's id, to follow it.
    pub async fn apply(&self, order: &Order) -> anyhow::Result<String> {
        let answer = self
            .call(Method::POST, "/exec/apply", Some(&order.body()), true)
            .await?;
        identifier(&answer["execId"])
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<&Json>,
        once: bool,
    ) -> anyhow::Result<Json> {
        match self {
            Self::Local(local) => local.call(method, path, body, once).await,
            Self::Cloud(cloud) => cloud.call(method, path, body).await,
            #[cfg(test)]
            Self::Fake(fake) => fake.call(path, once).await,
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
        }
    }

    /// Ends the session: the next call logs in again.
    pub async fn forget(&self) {
        *self.session.lock().await = None;
    }

    /// A valid access token: the current one, renewed before it ends (by
    /// its refresh token, else by logging in again).
    async fn access(&self) -> anyhow::Result<String> {
        let mut slot = self.session.lock().await;
        if let Some(session) = slot.as_ref()
            && Instant::now() < session.renew_at
        {
            return Ok(session.access.clone());
        }
        let refresh = slot.as_ref().and_then(|s| s.refresh.clone());
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
        let session = if let Some(session) = renewed {
            session
        } else {
            *slot = None;
            self.token(&[
                ("grant_type", "password"),
                ("username", &self.user),
                ("password", &self.password),
            ])
            .await?
        };
        let access = session.access.clone();
        *slot = Some(session);
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

    async fn call(&self, method: Method, path: &str, body: Option<&Json>) -> anyhow::Result<Json> {
        let access = self.access().await?;
        let builder = Request::builder()
            .method(method)
            .uri(format!("{CLOUD_ROOT}{path}"))
            .header("authorization", format!("Bearer {access}"));
        let request = build(builder, body)?;
        // One connection per request, never retried by `web`.
        let (status, body) =
            moli_net::web(CLOUD_HOST, 443, true, request, CLOUD_LIMIT, MAX_ANSWER).await?;
        let result = answer(status, &body);
        if let Err(e) = &result
            && refused(e) == Some(Refusal::Session)
        {
            self.forget().await;
        }
        result
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
}
