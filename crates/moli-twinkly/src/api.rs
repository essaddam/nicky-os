//! One bounded HTTP exchange at a time; session material never leaves RAM.
use std::collections::VecDeque;
use std::net::Ipv4Addr;
use std::time::Duration;

use http::{HeaderValue, Method, Request, StatusCode};
use ring::rand::{SecureRandom as _, SystemRandom};
use serde_json::{Value as Json, json};
use tokio::time::Instant;
use zeroize::Zeroizing;

use crate::model::{self, Capabilities, Info, MAX_JSON, Mode, Step};

const RECOVERY_WINDOW: Duration = Duration::from_secs(60 * 60);
const MAX_RECOVERIES: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    NotSent,
    Transport,
    Identity,
    Session,
    Reconnect,
    Expired,
    Refused,
    Unavailable,
    /// A prior step may have acted, but a later step was not confirmed.
    Partial,
    Protocol(&'static str),
}

pub(crate) struct Snapshot {
    pub info: Info,
    pub caps: Capabilities,
    pub mode: Mode,
    pub brightness: u8,
    pub color: Option<[u8; 3]>,
}

// No Debug: even an incidental diagnostic must not expose credentials.
struct Session {
    token: Zeroizing<String>,
    expires: Instant,
    /// A complete useful state was observed with this token.
    confirmed: bool,
}

pub(crate) struct Client {
    host: Ipv4Addr,
    port: u16,
    native: String,
    session: Option<Session>,
    parked: Option<Failure>,
    caps: Capabilities,
    recoveries: VecDeque<Instant>,
}

fn protocol(key: &'static str) -> Failure {
    if key == "pilotes.twinkly.refus" {
        Failure::Refused
    } else {
        Failure::Protocol(key)
    }
}

fn check(body: &[u8]) -> Result<Json, Failure> {
    model::object(body).map_err(protocol)
}

impl Client {
    pub fn new(host: Ipv4Addr, port: u16, native: String) -> Self {
        Self {
            host,
            port,
            native,
            session: None,
            parked: None,
            caps: Capabilities::default(),
            recoveries: VecDeque::new(),
        }
    }

    /// Invalidate only this serialized session. Recover a previously useful
    /// token on the next read; never repeat the request that was refused.
    fn session_refused(&mut self, authenticated: bool) -> Failure {
        let confirmed = self
            .session
            .take()
            .is_some_and(|session| authenticated && session.confirmed);
        let now = Instant::now();
        self.recoveries
            .retain(|at| now.duration_since(*at) < RECOVERY_WINDOW);
        if confirmed && self.recoveries.len() < MAX_RECOVERIES {
            self.recoveries.push_back(now);
            Failure::Reconnect
        } else {
            self.parked = Some(Failure::Session);
            Failure::Session
        }
    }

    /// Fixed local destination, fixed caller-owned path, no redirect/retry.
    /// `wanted` controls emission only; a sent answer is always collected.
    async fn request(
        &mut self,
        method: Method,
        path: &'static str,
        body: Option<&Json>,
        authenticated: bool,
        wanted: &(impl Fn() -> bool + Sync),
    ) -> Result<Vec<u8>, Failure> {
        if !wanted() {
            return Err(Failure::NotSent);
        }
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header("host", format!("{}:{}", self.host, self.port));
        if authenticated {
            let session = self
                .session
                .as_ref()
                .filter(|s| Instant::now() < s.expires)
                .ok_or(Failure::Expired)?;
            let mut token =
                HeaderValue::try_from(session.token.as_str()).map_err(|_| Failure::Session)?;
            token.set_sensitive(true);
            builder = builder.header("X-Auth-Token", token);
        }
        let request = match body {
            Some(json) => moli_net::json_body(builder, json),
            None => moli_net::empty(builder),
        }
        .map_err(|_| Failure::Protocol("pilotes.twinkly.reponse_invalide"))?;
        let host = self.host.to_string();
        let (status, body) =
            moli_net::plain_limited_if(&host, self.port, request, MAX_JSON, || {
                wanted()
                    && (!authenticated
                        || self
                            .session
                            .as_ref()
                            .is_some_and(|s| Instant::now() < s.expires))
            })
            .await
            .map_err(|e| {
                if e.is::<moli_net::NotSent>() {
                    Failure::NotSent
                } else if e.is::<moli_net::BodyTooLarge>() {
                    Failure::Protocol("pilotes.twinkly.reponse_invalide")
                } else {
                    Failure::Transport
                }
            })?;
        match status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                Err(self.session_refused(authenticated))
            }
            StatusCode::NOT_FOUND | StatusCode::NOT_IMPLEMENTED => Err(Failure::Unavailable),
            s if s.is_success() => Ok(body.to_vec()),
            _ => Err(Failure::Refused),
        }
    }

    async fn authenticate(&mut self, wanted: &(impl Fn() -> bool + Sync)) -> Result<(), Failure> {
        if self
            .session
            .as_ref()
            .is_some_and(|s| Instant::now() < s.expires)
        {
            return Ok(());
        }
        self.session = None;
        let started = Instant::now();
        let challenge = challenge()?;
        let body = self
            .request(
                Method::POST,
                "/xled/v1/login",
                Some(&json!({"challenge":challenge})),
                false,
                wanted,
            )
            .await?;
        let login = model::login(&body).map_err(protocol)?;
        let expires = started + login.lifetime;
        if Instant::now() >= expires {
            return Err(Failure::Expired);
        }
        self.session = Some(Session {
            token: Zeroizing::new(login.token),
            expires,
            confirmed: false,
        });
        let verified = self
            .request(
                Method::POST,
                "/xled/v1/verify",
                Some(&json!({"challenge-response":Zeroizing::new(login.response).as_str()})),
                true,
                wanted,
            )
            .await
            .and_then(|body| check(&body).map(|_| ()));
        if verified.is_err() {
            self.session = None;
            if matches!(verified, Err(Failure::Refused | Failure::Session)) {
                self.parked = Some(Failure::Session);
            }
        }
        verified
    }

    /// Revalidate identity before authentication and before any command's
    /// plan. A complete useful read is the only source of published state.
    pub async fn read(&mut self, wanted: impl Fn() -> bool + Sync) -> Result<Snapshot, Failure> {
        if let Some(reason) = self.parked {
            return Err(reason);
        }
        let body = self
            .request(Method::GET, "/xled/v1/gestalt", None, false, &wanted)
            .await?;
        let info = model::info(&body).map_err(protocol)?;
        if info.native != self.native {
            self.session = None;
            self.parked = Some(Failure::Identity);
            return Err(Failure::Identity);
        }
        self.authenticate(&wanted).await?;
        let version = check(
            &self
                .request(Method::GET, "/xled/v1/fw/version", None, true, &wanted)
                .await?,
        )?;
        let version = model::text(&version, "version", 24).map_err(protocol)?;
        let mode = model::mode(
            &self
                .request(Method::GET, "/xled/v1/led/mode", None, true, &wanted)
                .await?,
        )
        .map_err(protocol)?;
        let brightness = model::brightness(
            &self
                .request(
                    Method::GET,
                    "/xled/v1/led/out/brightness",
                    None,
                    true,
                    &wanted,
                )
                .await?,
        )
        .map_err(protocol)?;
        let mut caps = Capabilities {
            brightness: true,
            color: model::static_color(version, &info.profile),
        };
        let color = if caps.color {
            match self
                .request(Method::GET, "/xled/v1/led/color", None, true, &wanted)
                .await
            {
                Ok(body) => {
                    let raw = check(&body)?;
                    let component = |key| {
                        raw.get(key)
                            .and_then(Json::as_u64)
                            .and_then(|n| u8::try_from(n).ok())
                            .ok_or(Failure::Protocol("pilotes.twinkly.reponse_invalide"))
                    };
                    Some([component("red")?, component("green")?, component("blue")?])
                }
                Err(Failure::Unavailable) => {
                    caps.color = false;
                    None
                }
                Err(error) => return Err(error),
            }
        } else {
            None
        };
        self.caps = caps;
        if let Some(session) = &mut self.session {
            session.confirmed = true;
        }
        Ok(Snapshot {
            info,
            caps,
            mode,
            brightness,
            color,
        })
    }

    /// No authentication renewal or automatic retry on this path. A caller
    /// must read/plan first. A partial multi-step command stays uncertain.
    pub async fn apply(
        &mut self,
        steps: &[Step],
        wanted: impl Fn() -> bool + Sync,
    ) -> Result<(), Failure> {
        for (index, step) in steps.iter().enumerate() {
            let (path, body) = match step {
                Step::Mode(mode) => {
                    let mode = match mode {
                        Mode::Off => "off",
                        Mode::Movie => "movie",
                        Mode::Playlist => "playlist",
                        Mode::Color if self.caps.color => "color",
                        _ => return Err(Failure::Protocol("pilotes.twinkly.capacite_absente")),
                    };
                    ("/xled/v1/led/mode", json!({"mode":mode}))
                }
                Step::Brightness(value) if self.caps.brightness && (1..=100).contains(value) => (
                    "/xled/v1/led/out/brightness",
                    json!({"mode":"enabled","type":"A","value":value}),
                ),
                Step::Color([red, green, blue]) if self.caps.color => (
                    "/xled/v1/led/color",
                    json!({"red":red,"green":green,"blue":blue}),
                ),
                _ => return Err(Failure::Protocol("pilotes.twinkly.capacite_absente")),
            };
            let result = self
                .request(Method::POST, path, Some(&body), true, &wanted)
                .await
                .and_then(|body| check(&body).map(|_| ()));
            if let Err(error) = result {
                return Err(if index > 0 { Failure::Partial } else { error });
            }
        }
        Ok(())
    }
}

/// RFC 4648 standard base64, with padding, for a fresh 32-byte nonce.
/// The algorithm is also used by moli-net's WebSocket handshake.
fn challenge() -> Result<String, Failure> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut raw = [0u8; 32];
    SystemRandom::new()
        .fill(&mut raw)
        .map_err(|_| Failure::Session)?;
    let mut out = String::with_capacity(44);
    for chunk in raw.chunks(3) {
        let word = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            out.push(if i <= chunk.len() {
                char::from(TABLE[usize::try_from((word >> (18 - 6 * i)) & 63).unwrap_or(0)])
            } else {
                '='
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
#[path = "api_tests.rs"]
pub(crate) mod tests;
