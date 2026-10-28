//! Explicit human-confirmed import of Chrome's JSON-shaped "fetch (Node.js)".
//! Clipboard text is parsed as data, never evaluated. No clipboard process is
//! launched here. Only the fixed loopback pairing endpoint can receive data.

use std::io::Read;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use serde_json::{Map, Value, json};
use zeroize::{Zeroize, Zeroizing};

use super::youtube::BrowserBootstrap;

const INPUT_LIMIT: usize = 64 * 1024;
const COOKIE_LIMIT: usize = 16 * 1024;
const RESPONSE_LIMIT: usize = 2048;
const ORIGIN: &str = "https://music.youtube.com";

// Deliberately no Debug, Serialize, credential getter or filesystem API.
pub struct BrowserImport {
    cookie: String,
    bootstrap: BrowserBootstrap,
}

impl Drop for BrowserImport {
    fn drop(&mut self) {
        self.cookie.zeroize();
        if let Some(visitor) = &mut self.bootstrap.visitor_data {
            visitor.zeroize();
        }
    }
}

struct PrivateJson(Value);

impl Drop for PrivateJson {
    fn drop(&mut self) {
        wipe_value(&mut self.0);
    }
}

fn wipe_value(value: &mut Value) {
    match value {
        Value::String(text) => text.zeroize(),
        Value::Array(values) => values.iter_mut().for_each(wipe_value),
        Value::Object(values) => {
            for (mut key, mut value) in std::mem::take(values) {
                key.zeroize();
                wipe_value(&mut value);
            }
        }
        _ => {}
    }
}

fn json_prefix(input: &str) -> Result<(PrivateJson, usize)> {
    let mut stream = serde_json::Deserializer::from_str(input).into_iter::<Value>();
    let value = stream
        .next()
        .ok_or_else(|| anyhow!("copied request JSON is missing"))?
        .map_err(|_| anyhow!("copied request JSON is invalid"))?;
    Ok((PrivateJson(value), stream.byte_offset()))
}

fn header<'a>(headers: &'a Map<String, Value>, name: &str) -> Result<Option<&'a str>> {
    let mut found = None;
    for (key, value) in headers {
        if key.eq_ignore_ascii_case(name) {
            if found.is_some() {
                bail!("copied request contains duplicate headers");
            }
            found = Some(
                value
                    .as_str()
                    .ok_or_else(|| anyhow!("copied request header is invalid"))?,
            );
        }
    }
    Ok(found)
}

fn accepted_url(url: &str) -> bool {
    // Match the original spelling before URL normalization, rejecting encoded
    // separators, dot segments, userinfo, alternate ports and host spellings.
    let path = url.split_once('?').map_or(url, |(path, _)| path);
    if !matches!(
        path,
        "https://music.youtube.com/youtubei/v1/browse"
            | "https://music.youtube.com/youtubei/v1/search"
            | "https://music.youtube.com/youtubei/v1/account/account_menu"
    ) || url.contains('#')
        || url.chars().any(char::is_control)
    {
        return false;
    }
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    parsed.scheme() == "https"
        && parsed.host_str() == Some("music.youtube.com")
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.port().is_none()
        && parsed.fragment().is_none()
}

fn valid_cookie(cookie: &str) -> bool {
    !cookie.is_empty()
        && cookie.len() <= COOKIE_LIMIT
        && cookie.bytes().all(|byte| (32..=126).contains(&byte))
        && !cookie.contains("{{")
        && !cookie.contains("}}")
        && cookie.split(';').any(|part| {
            part.trim().split_once('=').is_some_and(|(name, value)| {
                matches!(name, "SAPISID" | "__Secure-3PAPISID") && !value.trim().is_empty()
            })
        })
}

impl BrowserImport {
    pub fn read(reader: impl Read) -> Result<Self> {
        let mut raw = Zeroizing::new(Vec::with_capacity(INPUT_LIMIT + 1));
        reader
            .take((INPUT_LIMIT + 1) as u64)
            .read_to_end(&mut raw)
            .map_err(|_| anyhow!("cannot read copied browser request"))?;
        if raw.len() > INPUT_LIMIT {
            bail!("copied browser request exceeds 64 KiB");
        }
        let text = std::str::from_utf8(&raw)
            .map_err(|_| anyhow!("copied browser request is not UTF-8"))?
            .trim();
        let input = text
            .strip_prefix("fetch(")
            .ok_or_else(|| anyhow!("expected Chrome fetch Node.js format"))?
            .trim_start();
        let (url, consumed) = json_prefix(input)?;
        let url = url
            .0
            .as_str()
            .ok_or_else(|| anyhow!("copied request URL must be a JSON string"))?;
        if !accepted_url(url) {
            bail!("copied request URL is not an allowed read-only YouTube endpoint");
        }
        let rest = input[consumed..]
            .trim_start()
            .strip_prefix(',')
            .ok_or_else(|| anyhow!("copied fetch options are missing"))?
            .trim_start();
        let (options, consumed) = json_prefix(rest)?;
        let trailing = rest[consumed..].trim();
        if !matches!(trailing, ")" | ");") {
            bail!("additional code after copied request is forbidden");
        }
        let options = options
            .0
            .as_object()
            .ok_or_else(|| anyhow!("copied fetch options must be a JSON object"))?;
        if options.get("method").and_then(Value::as_str) != Some("POST") {
            bail!("copied request must use POST");
        }
        let headers = options
            .get("headers")
            .and_then(Value::as_object)
            .ok_or_else(|| anyhow!("copied request headers are missing"))?;
        let cookie = header(headers, "cookie")?
            .ok_or_else(|| anyhow!("copied Node.js request lacks its Cookie header"))?;
        if !valid_cookie(cookie) {
            bail!("copied Cookie header is invalid or lacks a session identifier");
        }
        if header(headers, "x-goog-authuser")?.is_some_and(|user| user != "0") {
            bail!("only the confirmed primary account is supported");
        }
        let body = match options.get("body") {
            None => None,
            Some(Value::String(body)) => {
                Some(PrivateJson(serde_json::from_str(body).map_err(|_| {
                    anyhow!("copied request body is not valid JSON")
                })?))
            }
            _ => bail!("copied request body must be a JSON string"),
        };
        let client = body
            .as_ref()
            .and_then(|body| body.0.get("context"))
            .and_then(|context| context.get("client"));
        let version = header(headers, "x-youtube-client-version")?
            .or_else(|| {
                client
                    .and_then(|client| client.get("clientVersion"))
                    .and_then(Value::as_str)
            })
            .ok_or_else(|| anyhow!("copied request lacks its browser client version"))?;
        let visitor = header(headers, "x-goog-visitor-id")?.or_else(|| {
            client
                .and_then(|client| client.get("visitorData"))
                .and_then(Value::as_str)
        });
        let bootstrap = BrowserBootstrap {
            client_version: version.to_owned(),
            visitor_data: visitor.map(str::to_owned),
        };
        if !bootstrap.is_valid() {
            let mut bootstrap = bootstrap;
            if let Some(visitor) = &mut bootstrap.visitor_data {
                visitor.zeroize();
            }
            bail!("copied browser bootstrap is invalid");
        }
        Ok(Self {
            cookie: cookie.to_owned(),
            bootstrap,
        })
    }

    pub async fn submit(self, port: u16) -> Result<Value> {
        if port == 0 {
            bail!("local pairing port is invalid");
        }
        let payload = PrivateJson(json!({"cookie":self.cookie.as_str(),"bootstrap":{
            "client_version":self.bootstrap.client_version.as_str(),
            "visitor_data":self.bootstrap.visitor_data.as_deref()
        }}));
        let serialized = Zeroizing::new(
            serde_json::to_vec(&payload.0)
                .map_err(|_| anyhow!("cannot encode local pairing request"))?,
        );
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|_| anyhow!("cannot create local pairing client"))?;
        // Bytes owns the zeroizing allocation until the last request-body
        // reference is released. Network stacks may maintain their own copies.
        let mut response = client
            .post(format!("http://127.0.0.1:{port}/pair/youtube"))
            .header(reqwest::header::ORIGIN, ORIGIN)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(bytes::Bytes::from_owner(serialized))
            .send()
            .await
            .map_err(|_| anyhow!("local pairing request failed"))?;
        let http_status = response.status().as_u16();
        if response
            .content_length()
            .is_some_and(|length| length > RESPONSE_LIMIT as u64)
        {
            bail!("local pairing response exceeds limit");
        }
        let mut raw = Zeroizing::new(Vec::with_capacity(RESPONSE_LIMIT));
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| anyhow!("cannot read local pairing response"))?
        {
            if raw.len() + chunk.len() > RESPONSE_LIMIT {
                bail!("local pairing response exceeds limit");
            }
            raw.extend_from_slice(&chunk);
        }
        let response = PrivateJson(
            serde_json::from_slice(&raw)
                .map_err(|_| anyhow!("local pairing response is invalid JSON"))?,
        );
        sanitize_response(http_status, &response.0)
    }
}

fn sanitize_response(http_status: u16, response: &Value) -> Result<Value> {
    let source = response
        .as_object()
        .ok_or_else(|| anyhow!("local pairing response must be an object"))?;
    let mut output = Map::new();
    output.insert("http_status".to_owned(), json!(http_status));
    for field in ["paired", "native", "persistent"] {
        if let Some(value) = source.get(field).and_then(Value::as_bool) {
            output.insert(field.to_owned(), json!(value));
        }
    }
    if let Some(status) = source
        .get("status")
        .and_then(Value::as_str)
        .filter(|status| {
            matches!(
                *status,
                "forbidden"
                    | "expired"
                    | "json_required"
                    | "body_rejected"
                    | "invalid_body"
                    | "invalid_bootstrap"
                    | "invalid_cookie"
                    | "already_paired"
                    | "busy"
                    | "attempt_limit"
                    | "authorization_failed"
                    | "unavailable"
            )
        })
    {
        output.insert("status".to_owned(), json!(status));
    }
    if let Some(reason) = source
        .get("reason")
        .and_then(Value::as_str)
        .filter(|reason| {
            matches!(
                *reason,
                "consent_redirect"
                    | "provider_authorization_refused"
                    | "bootstrap_schema_changed"
                    | "credential_format_rejected"
                    | "provider_request_failed"
                    | "account_response_unconfirmed"
                    | "provider_timeout"
            )
        })
    {
        output.insert("reason".to_owned(), json!(reason));
    }
    Ok(Value::Object(output))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn submit_uses_private_body_and_never_returns_upstream_credentials() {
        use axum::{Router, body::Bytes, http::HeaderMap, routing::post};
        let router = Router::new().route("/pair/youtube", post(|headers: HeaderMap, body: Bytes| async move {
            assert_eq!(headers[reqwest::header::ORIGIN], ORIGIN);
            let request: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(request["cookie"], "SAPISID=synthetic; SID=fake");
            assert_eq!(request["bootstrap"]["client_version"], "1.20261027.01.00");
            ([("content-type", "application/json")], r#"{"paired":true,"native":true,"persistent":false,"cookie":"must-not-escape","status":"must-not-escape"}"#)
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let raw = copied(
            "https://music.youtube.com/youtubei/v1/browse?prettyPrint=false",
            json!({
                "method":"POST","headers":{"Cookie":"SAPISID=synthetic; SID=fake","x-youtube-client-version":"1.20261027.01.00"}
            }),
        );
        let result = BrowserImport::read(raw.as_bytes())
            .unwrap()
            .submit(port)
            .await
            .unwrap();
        assert_eq!(
            result,
            json!({"http_status":200,"paired":true,"native":true,"persistent":false})
        );
        server.abort();
    }

    fn copied(url: &str, options: Value) -> String {
        format!(
            "fetch({}, {});",
            serde_json::to_string(url).unwrap(),
            options
        )
    }

    fn options() -> Value {
        json!({"method":"POST","headers":{"Cookie":"SAPISID=synthetic-session; other=fake","Authorization":"ignored-sensitive-authorization","X-Goog-AuthUser":"0","x-youtube-client-version":"1.20261027.01.00","x-goog-visitor-id":"synthetic-visitor"},"body":json!({"context":{"client":{"clientVersion":"1.19990101.01.00","visitorData":"fallback-visitor"}},"query":"quoted \"text\" with {braces} and ); characters"}).to_string()})
    }

    #[test]
    fn parses_node_fetch_and_prefers_headers_without_executing_body_text() {
        let raw = copied(
            "https://music.youtube.com/youtubei/v1/browse?prettyPrint=false",
            options(),
        );
        let imported = BrowserImport::read(raw.as_bytes()).unwrap();
        assert_eq!(imported.cookie, "SAPISID=synthetic-session; other=fake");
        assert_eq!(imported.bootstrap.client_version, "1.20261027.01.00");
        assert_eq!(
            imported.bootstrap.visitor_data.as_deref(),
            Some("synthetic-visitor")
        );
    }

    #[test]
    fn uses_context_fallback_and_case_insensitive_cookie_header() {
        let mut options = options();
        let headers = options["headers"].as_object_mut().unwrap();
        headers.remove("x-youtube-client-version");
        headers.remove("x-goog-visitor-id");
        let cookie = headers.remove("Cookie").unwrap();
        headers.insert("cOoKiE".to_owned(), cookie);
        let raw = copied(
            "https://music.youtube.com/youtubei/v1/account/account_menu",
            options,
        );
        let imported = BrowserImport::read(raw.as_bytes()).unwrap();
        assert_eq!(imported.bootstrap.client_version, "1.19990101.01.00");
        assert_eq!(
            imported.bootstrap.visitor_data.as_deref(),
            Some("fallback-visitor")
        );
    }

    #[test]
    fn rejects_mutant_hosts_paths_and_additional_code_without_echoing_secrets() {
        for url in [
            "http://music.youtube.com/youtubei/v1/browse",
            "https://music.youtube.com:443/youtubei/v1/browse",
            "https://user@example.com/youtubei/v1/browse",
            "https://music.youtube.com.evil.invalid/youtubei/v1/browse",
            "https://music.youtube.com/youtubei/v1/../v1/browse",
            "https://music.youtube.com/youtubei/v1/%62rowse",
            "https://music.youtube.com/youtubei/v1/player",
            "https://music.youtube.com/youtubei/v1/browse#secret",
        ] {
            let raw = copied(url, options());
            let error = BrowserImport::read(raw.as_bytes())
                .err()
                .expect("must reject request");
            assert!(!format!("{error:#}").contains("synthetic-session"));
        }
        let valid = copied("https://music.youtube.com/youtubei/v1/search", options());
        for raw in [
            format!("{valid}console.log('synthetic-session')"),
            format!("alert('synthetic-session');{valid}"),
            valid.replace("fetch(", "eval("),
        ] {
            let error = BrowserImport::read(raw.as_bytes())
                .err()
                .expect("must reject code");
            assert!(!format!("{error:#}").contains("synthetic-session"));
        }
    }

    #[test]
    fn rejects_oversized_invalid_and_unconfirmed_credentials() {
        assert!(BrowserImport::read(vec![b'x'; INPUT_LIMIT + 1].as_slice()).is_err());
        assert!(BrowserImport::read(&[255_u8][..]).is_err());
        for (field, value) in [
            ("Cookie", "SID=synthetic-session"),
            ("Cookie", "SAPISID={{secret}}"),
            ("Cookie", "SAPISID=synthetic\nsecret"),
            ("X-Goog-AuthUser", "1"),
        ] {
            let mut options = options();
            options["headers"][field] = json!(value);
            let raw = copied("https://music.youtube.com/youtubei/v1/browse", options);
            let error = BrowserImport::read(raw.as_bytes())
                .err()
                .expect("must reject credentials");
            assert!(!format!("{error:#}").contains("synthetic"));
        }
        let mut options = options();
        options["method"] = json!("GET");
        assert!(
            BrowserImport::read(
                copied("https://music.youtube.com/youtubei/v1/browse", options).as_bytes()
            )
            .is_err()
        );
    }

    #[test]
    fn filters_every_upstream_field_and_unknown_error_code() {
        let output = sanitize_response(401, &json!({"paired":false,"native":false,"persistent":false,"status":"authorization_failed","reason":"consent_redirect","cookie":"synthetic-secret","account_name":"private-name","title":"private-title"})).unwrap();
        assert_eq!(
            output,
            json!({"http_status":401,"paired":false,"native":false,"persistent":false,"status":"authorization_failed","reason":"consent_redirect"})
        );
        let output = sanitize_response(
            500,
            &json!({"status":"synthetic-secret","reason":"private-name","paired":"private-title"}),
        )
        .unwrap();
        assert_eq!(output, json!({"http_status":500}));
    }
}
