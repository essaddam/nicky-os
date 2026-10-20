//! Fixed-provider weather and OAuth requests. No redirects or implicit retries.

use std::fmt::Write as _;
use std::time::Duration;

use http::{HeaderMap, Request, StatusCode, header};
use http_body_util::Full;
use moli_net::{Body, OnceError};
use serde_json::Value as Json;

use crate::{Error, oauth::TokenSet};

const HOST: &str = "api.netatmo.com";
const TOKEN_LIMIT: usize = 16 * 1024;
const WEATHER_LIMIT: usize = 512 * 1024;
const TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) type SendGuard = std::sync::Arc<dyn Fn() -> bool + Send + Sync>;

#[cfg(test)]
pub(crate) type Exchange = std::sync::Arc<
    dyn Fn(
            Request<Full<Body>>,
            usize,
            SendGuard,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<(StatusCode, HeaderMap, Body), OnceError>>
                    + Send,
            >,
        > + Send
        + Sync,
>;

pub(crate) struct Api {
    client_id: String,
    client_secret: String,
    callback: String,
    #[cfg(test)]
    pub exchange: Option<Exchange>,
}

impl std::fmt::Debug for Api {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Api").finish_non_exhaustive()
    }
}

impl Api {
    pub fn new(client_id: String, client_secret: String, callback: String) -> Self {
        Self {
            client_id,
            client_secret,
            callback,
            #[cfg(test)]
            exchange: None,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), Error> {
        if !plain(&self.client_id, 256)
            || !plain(&self.client_secret, 4096)
            || !plain(&self.callback, 4096)
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }

    pub fn authorization_url(&self, state: &str) -> Result<String, Error> {
        self.validate()?;
        if state.len() != 64 || !state.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::Invalid);
        }
        let query = form(&[
            ("client_id", &self.client_id),
            ("redirect_uri", &self.callback),
            ("response_type", "code"),
            ("scope", "read_station"),
            ("state", state),
        ]);
        Ok(format!("https://{HOST}/oauth2/authorize?{query}"))
    }

    pub async fn exchange_code(
        &self,
        code: &str,
        now: u64,
        allowed: SendGuard,
    ) -> Result<TokenSet, Error> {
        self.validate()?;
        if !plain(code, 2048) {
            return Err(Error::Invalid);
        }
        let body = form(&[
            ("grant_type", "authorization_code"),
            ("client_id", &self.client_id),
            ("client_secret", &self.client_secret),
            ("code", code),
            ("redirect_uri", &self.callback),
        ]);
        self.token_request(body, now, allowed).await
    }

    pub async fn refresh(&self, tokens: &TokenSet, now: u64) -> Result<TokenSet, Error> {
        self.validate()?;
        if !plain(&tokens.refresh, 4096) {
            return Err(Error::Invalid);
        }
        let body = form(&[
            ("grant_type", "refresh_token"),
            ("client_id", &self.client_id),
            ("client_secret", &self.client_secret),
            ("refresh_token", &tokens.refresh),
        ]);
        self.token_request(body, now, std::sync::Arc::new(|| true))
            .await
    }

    async fn token_request(
        &self,
        body: String,
        now: u64,
        allowed: SendGuard,
    ) -> Result<TokenSet, Error> {
        let request = Request::builder()
            .method("POST")
            .uri("/oauth2/token")
            .header(header::HOST, HOST)
            .header(header::ACCEPT, "application/json")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Full::new(Body::from(body)))
            .map_err(|_| Error::Invalid)?;
        let (status, headers, body) = self.send(request, TOKEN_LIMIT, allowed).await?;
        // Once handed to HTTP, an unreadable token answer may already have
        // rotated the pair. Never treat a malformed answer as safe to replay.
        if !plain_response(&headers) {
            return Err(Error::Ambiguous);
        }
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Err(Error::RateLimited(retry_after(&headers)));
        }
        if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            return Err(Error::Unauthorized);
        }
        if status == StatusCode::BAD_REQUEST {
            let refused = serde_json::from_slice::<Json>(&body)
                .ok()
                .and_then(|json| json.get("error").and_then(Json::as_str).map(str::to_owned))
                .is_some_and(|error| {
                    matches!(
                        error.as_str(),
                        "invalid_grant" | "invalid_client" | "access_denied"
                    )
                });
            if refused {
                return Err(Error::Unauthorized);
            }
        }
        if status != StatusCode::OK {
            return Err(Error::Ambiguous);
        }
        TokenSet::parse(&body, now).map_err(|_| Error::Ambiguous)
    }

    pub async fn weather(&self, access: &str) -> Result<Body, Error> {
        if !plain(access, 4096) {
            return Err(Error::Invalid);
        }
        let request = Request::builder()
            .method("GET")
            .uri("/api/getstationsdata?get_favorites=false")
            .header(header::HOST, HOST)
            .header(header::ACCEPT, "application/json")
            .header(header::AUTHORIZATION, format!("Bearer {access}"))
            .body(Full::new(Body::new()))
            .map_err(|_| Error::Invalid)?;
        let (status, headers, body) = self
            .send(request, WEATHER_LIMIT, std::sync::Arc::new(|| true))
            .await?;
        if !plain_response(&headers) {
            return Err(Error::Invalid);
        }
        match status {
            StatusCode::OK => Ok(body),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(Error::Unauthorized),
            StatusCode::TOO_MANY_REQUESTS => Err(Error::RateLimited(retry_after(&headers))),
            status if status.is_server_error() => Err(Error::Unavailable),
            _ => Err(Error::Invalid),
        }
    }

    async fn send(
        &self,
        request: Request<Full<Body>>,
        max_body: usize,
        allowed: SendGuard,
    ) -> Result<(StatusCode, HeaderMap, Body), Error> {
        #[cfg(test)]
        let response = if let Some(exchange) = &self.exchange {
            exchange(request, max_body, allowed.clone()).await
        } else {
            moli_net::web_once_guarded(HOST, 443, true, request, TIMEOUT, max_body, || allowed())
                .await
        };
        #[cfg(not(test))]
        let response =
            moli_net::web_once_guarded(HOST, 443, true, request, TIMEOUT, max_body, || allowed())
                .await;
        let response = response.map_err(|error| match error {
            OnceError::BeforeSend if !allowed() => Error::Expired,
            OnceError::BeforeSend => Error::BeforeSend,
            OnceError::AfterSend => Error::Ambiguous,
        })?;
        // The real transport bounds this already. Keep the same boundary when
        // an exchange is injected, so no unbounded decoder exists in tests.
        if response.2.len() > max_body {
            return Err(Error::Ambiguous);
        }
        Ok(response)
    }
}

fn plain(text: &str, max: usize) -> bool {
    !text.is_empty() && text.len() <= max && text.bytes().all(|b| b.is_ascii_graphic())
}

fn plain_response(headers: &HeaderMap) -> bool {
    let uncompressed = headers
        .get_all(header::CONTENT_ENCODING)
        .iter()
        .all(|value| value.as_bytes().eq_ignore_ascii_case(b"identity"));
    let json = headers.get_all(header::CONTENT_TYPE).iter().all(|value| {
        value.to_str().is_ok_and(|text| {
            text.split(';')
                .next()
                .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("application/json"))
        })
    });
    uncompressed && json
}

fn retry_after(headers: &HeaderMap) -> u64 {
    headers
        .get(header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(60)
        .clamp(60, 900)
}

fn form(fields: &[(&str, &str)]) -> String {
    let mut result = String::new();
    for (index, (name, value)) in fields.iter().enumerate() {
        if index != 0 {
            result.push('&');
        }
        encode(&mut result, name);
        result.push('=');
        encode(&mut result, value);
    }
    result
}

fn encode(output: &mut String, text: &str) {
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            output.push(char::from(byte));
        } else {
            // Writing to a String cannot fail.
            let _ = write!(output, "%{byte:02X}");
        }
    }
}
