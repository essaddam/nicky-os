//! Small, bounded direct Innertube client. No Python, external extractor or MA.
use super::{http, query_is_valid, text};
use anyhow::{Context as _, Result, bail, ensure};
use reqwest::header::{AUTHORIZATION, COOKIE, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use sha1::{Digest, Sha1};
use std::{
    collections::BTreeSet,
    env,
    time::{SystemTime, UNIX_EPOCH},
};
use zeroize::Zeroize;

const ORIGIN: &str = "https://music.youtube.com";
const MAX_COOKIE: usize = 16 * 1024;
const MAX_TREE_NODES: usize = 30_000;

/// Non-secret client metadata from the already authorized browser. This does
/// not confirm an account: the Rust client must still verify it with YouTube.
#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserBootstrap {
    pub client_version: String,
    pub visitor_data: Option<String>,
}

impl BrowserBootstrap {
    pub(super) fn is_valid(&self) -> bool {
        !self.client_version.is_empty()
            && self.client_version.len() <= 64
            && self.client_version.as_bytes()[0].is_ascii_digit()
            && self
                .client_version
                .bytes()
                .all(|b| b.is_ascii_digit() || b == b'.')
            && self.visitor_data.as_ref().is_none_or(|data| {
                !data.is_empty()
                    && data.len() <= 1024
                    && data.bytes().all(|b| (32..=126).contains(&b))
            })
    }
}

// No Debug/Serialize: these values must not enter reports.
struct Session {
    cookie: Option<String>,
    sapisid: Option<String>,
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(cookie) = &mut self.cookie {
            cookie.zeroize();
        }
        if let Some(sapisid) = &mut self.sapisid {
            sapisid.zeroize();
        }
    }
}

impl Session {
    fn from_env() -> Result<Self> {
        match env::var("YTMUSIC_COOKIE") {
            Ok(raw) => Self::parse(Some(raw)),
            Err(env::VarError::NotPresent) => Self::parse(None),
            Err(_) => bail!("invalid YouTube Music credential"),
        }
    }
    fn parse(cookie: Option<String>) -> Result<Self> {
        let Some(raw) = cookie else {
            return Ok(Self {
                cookie: None,
                sapisid: None,
            });
        };
        ensure!(
            !raw.is_empty()
                && raw.len() <= MAX_COOKIE
                && !raw.contains("{{secret:")
                && !raw.chars().any(char::is_control),
            "invalid YouTube Music credential"
        );
        let parts: Vec<_> = raw
            .split(';')
            .filter_map(|p| p.trim().split_once('='))
            .collect();
        let sapisid = ["SAPISID", "__Secure-3PAPISID"]
            .into_iter()
            .find_map(|key| {
                parts
                    .iter()
                    .find(|(k, _)| *k == key)
                    .map(|(_, v)| (*v).to_owned())
            })
            .filter(|v| !v.is_empty() && v.len() <= 4096)
            .ok_or_else(|| {
                anyhow::anyhow!("YouTube Music credential lacks a session identifier")
            })?;
        Ok(Self {
            cookie: Some(raw),
            sapisid: Some(sapisid),
        })
    }
    fn headers(&self, timestamp: u64) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        headers.insert("origin", HeaderValue::from_static(ORIGIN));
        headers.insert("x-origin", HeaderValue::from_static(ORIGIN));
        headers.insert("x-goog-authuser", HeaderValue::from_static("0"));
        headers.insert("x-youtube-client-name", HeaderValue::from_static("67"));
        if let (Some(cookie), Some(sapisid)) = (&self.cookie, &self.sapisid) {
            let digest = Sha1::digest(format!("{timestamp} {sapisid} {ORIGIN}").as_bytes());
            let mut auth = HeaderValue::from_str(&format!("SAPISIDHASH {timestamp}_{digest:x}"))
                .map_err(|_| anyhow::anyhow!("invalid YouTube Music credential"))?;
            auth.set_sensitive(true);
            let mut cookie = HeaderValue::from_str(cookie)
                .map_err(|_| anyhow::anyhow!("invalid YouTube Music credential"))?;
            cookie.set_sensitive(true);
            headers.insert(AUTHORIZATION, auth);
            headers.insert(COOKIE, cookie);
        }
        Ok(headers)
    }
}

fn configured() -> bool {
    env::var_os("YTMUSIC_COOKIE").is_some()
}

pub fn status() -> Value {
    json!({"native":true,"provider":"ytmusic","api":"unofficial_innertube",
        "credential_configured":configured(),"authenticated":null,
        "catalogue":"implemented_unqualified","full_stream":"not_implemented"})
}

fn config_string(html: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\"");
    for (_, tail) in html
        .match_indices(&marker)
        .map(|(at, _)| (at, &html[at + marker.len()..]))
    {
        let Some(tail) = tail.trim_start().strip_prefix(':') else {
            continue;
        };
        let mut decoder =
            serde_json::Deserializer::from_str(tail.trim_start()).into_iter::<Value>();
        if let Some(Ok(Value::String(value))) = decoder.next()
            && !value.is_empty()
            && value.len() <= 4096
            && !value.chars().any(char::is_control)
        {
            return Some(value);
        }
    }
    None
}

struct Api {
    client: reqwest::Client,
    origin: String,
    session: Session,
    version: String,
    visitor: Option<String>,
}

impl Api {
    async fn new(session: Session) -> Result<Self> {
        Self::connect(session, ORIGIN).await
    }
    fn from_browser(session: Session, bootstrap: BrowserBootstrap) -> Result<Self> {
        ensure!(bootstrap.is_valid(), "invalid browser client metadata");
        Ok(Self {
            client: http::client()?,
            origin: ORIGIN.to_owned(),
            session,
            version: bootstrap.client_version,
            visitor: bootstrap.visitor_data,
        })
    }

    async fn with_bootstrap(session: Session, bootstrap: Option<BrowserBootstrap>) -> Result<Self> {
        match bootstrap {
            Some(bootstrap) => Self::from_browser(session, bootstrap),
            None => Self::new(session).await,
        }
    }
    // Production always supplies ORIGIN; tests use an isolated loopback server.
    async fn connect(session: Session, origin: &str) -> Result<Self> {
        let client = http::client()?;
        let mut request = client.get(origin);
        if let Some(cookie) = &session.cookie {
            let mut cookie = HeaderValue::from_str(cookie)
                .map_err(|_| anyhow::anyhow!("invalid YouTube Music credential"))?;
            cookie.set_sensitive(true);
            request = request.header(COOKIE, cookie);
        }
        let page = http::bytes(request, 2 * 1024 * 1024)
            .await
            .context("YouTube Music bootstrap failed")?;
        let page = std::str::from_utf8(&page)
            .map_err(|_| anyhow::anyhow!("YouTube Music bootstrap is invalid"))?;
        let version = config_string(page, "INNERTUBE_CLIENT_VERSION")
            .filter(|v| v.len() <= 64 && v.bytes().all(|b| b.is_ascii_digit() || b == b'.'))
            .ok_or_else(|| anyhow::anyhow!("YouTube Music bootstrap schema changed"))?;
        let visitor = config_string(page, "VISITOR_DATA");
        Ok(Self {
            client,
            origin: origin.to_owned(),
            session,
            version,
            visitor,
        })
    }
    async fn post(&self, operation: &str, mut body: Value) -> Result<Value> {
        ensure!(
            ["search", "account/account_menu", "player"].contains(&operation),
            "unsupported provider operation"
        );
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| anyhow::anyhow!("system clock unavailable"))?
            .as_secs();
        let mut headers = self.session.headers(timestamp)?;
        headers.insert(
            "x-youtube-client-version",
            HeaderValue::from_str(&self.version)?,
        );
        let mut context = json!({"client":{"clientName":"WEB_REMIX","clientVersion":self.version,"hl":"en","gl":"FR"}});
        if let Some(visitor) = &self.visitor {
            context["client"]["visitorData"] = json!(visitor);
        }
        body["context"] = context;
        http::json(
            self.client
                .post(format!(
                    "{}/youtubei/v1/{operation}?prettyPrint=false",
                    self.origin
                ))
                .headers(headers)
                .json(&body),
        )
        .await
        .context("YouTube Music API request failed")
    }
}

fn video_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn walk<'a>(root: &'a Value, key: &str) -> Result<Vec<&'a Value>> {
    let mut pending = vec![(root, 0)];
    let mut found = Vec::new();
    let mut visited = 0;
    while let Some((node, depth)) = pending.pop() {
        visited += 1;
        ensure!(
            visited <= MAX_TREE_NODES && depth <= 48,
            "YouTube Music response structure exceeds bound"
        );
        match node {
            Value::Object(fields) => {
                if let Some(value) = fields.get(key) {
                    found.push(value);
                }
                ensure!(
                    pending.len() + fields.len() <= MAX_TREE_NODES,
                    "YouTube Music response structure exceeds bound"
                );
                pending.extend(fields.values().rev().map(|v| (v, depth + 1)));
            }
            Value::Array(items) => {
                ensure!(
                    pending.len() + items.len() <= MAX_TREE_NODES,
                    "YouTube Music response structure exceeds bound"
                );
                pending.extend(items.iter().rev().map(|v| (v, depth + 1)));
            }
            _ => {}
        }
    }
    Ok(found)
}

fn search_output(answer: &Value, cookie: Option<&str>) -> Result<Value> {
    let mut results = Vec::new();
    let mut ids = BTreeSet::new();
    for item in walk(answer, "musicResponsiveListItemRenderer")? {
        let id = item.pointer("/overlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint/watchEndpoint/videoId")
            .or_else(|| item.pointer("/navigationEndpoint/watchEndpoint/videoId"))
            .and_then(Value::as_str);
        let id = match id {
            Some(id) => id.to_owned(),
            None => walk(item, "watchEndpoint")?
                .iter()
                .find_map(|p| p["videoId"].as_str().map(str::to_owned))
                .unwrap_or_default(),
        };
        if !video_id(&id) || !ids.insert(id.clone()) {
            continue;
        }
        let columns = item["flexColumns"].as_array();
        let label = |index: usize| {
            columns
                .and_then(|c| c.get(index))
                .and_then(|c| c.pointer("/musicResponsiveListItemFlexColumnRenderer/text/runs"))
                .and_then(Value::as_array)
                .map(|runs| {
                    runs.iter()
                        .take(8)
                        .filter_map(|r| r["text"].as_str())
                        .collect::<String>()
                })
                .unwrap_or_default()
        };
        let title = text(&label(0), 160);
        let subtitle = text(&label(1), 160);
        if title.is_empty()
            || cookie.is_some_and(|secret| title.contains(secret) || subtitle.contains(secret))
        {
            continue;
        }
        results.push(json!({"id":id,"title":title,"subtitle":subtitle,"kind":"track_candidate"}));
        if results.len() == 5 {
            break;
        }
    }
    Ok(
        json!({"native":true,"provider":"ytmusic","status":if results.is_empty(){"no_match"}else{"ok"},
        "authenticated":null,"results":results,"full_stream":"not_implemented"}),
    )
}

pub async fn search(query: &str) -> Result<Value> {
    ensure!(query_is_valid(query), "invalid provider search query");
    let session = Session::from_env()?;
    search_session(session, query, None).await
}

pub(super) async fn search_with_cookie(
    cookie: String,
    query: String,
    bootstrap: Option<BrowserBootstrap>,
) -> Result<Value> {
    ensure!(query_is_valid(&query), "invalid provider search query");
    search_session(Session::parse(Some(cookie))?, &query, bootstrap).await
}

async fn search_session(
    session: Session,
    query: &str,
    bootstrap: Option<BrowserBootstrap>,
) -> Result<Value> {
    let api = Api::with_bootstrap(session, bootstrap).await?;
    let answer = api.post("search", json!({"query":query.trim()})).await?;
    search_output(&answer, api.session.cookie.as_deref())
}

pub async fn account() -> Result<Value> {
    let session = Session::from_env()?;
    account_session(session, None).await
}

pub(super) async fn account_with_cookie(
    cookie: String,
    bootstrap: Option<BrowserBootstrap>,
) -> Result<Value> {
    account_session(Session::parse(Some(cookie))?, bootstrap).await
}

async fn account_session(session: Session, bootstrap: Option<BrowserBootstrap>) -> Result<Value> {
    if session.cookie.is_none() {
        return Ok(
            json!({"native":true,"provider":"ytmusic","status":"credentials_required","authenticated":false}),
        );
    }
    let api = Api::with_bootstrap(session, bootstrap).await?;
    let answer = api.post("account/account_menu", json!({})).await?;
    let recognized = !walk(&answer, "activeAccountHeaderRenderer")?.is_empty();
    Ok(
        json!({"native":true,"provider":"ytmusic","status":if recognized{"authorized"}else{"authorization_unconfirmed"},
        "authenticated":recognized,"subscription_verified":false,"full_stream":"not_implemented"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::Bytes,
        routing::{get, post},
    };

    #[tokio::test]
    async fn authorized_browser_metadata_allows_direct_native_api_without_homepage() {
        let router = Router::new().route(
            "/youtubei/v1/account/account_menu",
            post(|headers: HeaderMap, body: Bytes| async move {
                assert_eq!(headers[COOKIE], "SAPISID=synthetic-session");
                let request: Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(
                    request["context"]["client"]["clientVersion"],
                    "1.20261025.01.00"
                );
                (
                    [("content-type", "application/json")],
                    r#"{"header":{"activeAccountHeaderRenderer":{}}}"#,
                )
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let mut api = Api::from_browser(
            Session::parse(Some("SAPISID=synthetic-session".into())).unwrap(),
            BrowserBootstrap {
                client_version: "1.20261025.01.00".into(),
                visitor_data: None,
            },
        )
        .unwrap();
        api.origin = origin;
        let answer = api.post("account/account_menu", json!({})).await.unwrap();
        assert_eq!(
            walk(&answer, "activeAccountHeaderRenderer").unwrap().len(),
            1
        );
        server.abort();
    }

    #[test]
    fn browser_metadata_is_bounded_and_cannot_inject_headers() {
        for version in ["", "bad", "1.2\r\nInjected: yes"] {
            assert!(
                !BrowserBootstrap {
                    client_version: version.into(),
                    visitor_data: None
                }
                .is_valid()
            );
        }
        assert!(
            !BrowserBootstrap {
                client_version: "1.2".into(),
                visitor_data: Some("x".repeat(1025))
            }
            .is_valid()
        );
    }

    #[tokio::test]
    async fn direct_http_bootstrap_and_search_use_the_same_session() {
        let router = Router::new()
            .route("/", get(|headers: HeaderMap| async move {
                assert_eq!(headers[COOKIE], "SAPISID=synthetic-session");
                r#"ytcfg.set({"INNERTUBE_CLIENT_VERSION":"1.20261025.01.00","VISITOR_DATA":"synthetic-visitor"});"#
            }))
            .route("/youtubei/v1/search", post(|headers: HeaderMap, body: Bytes| async move {
                assert_eq!(headers[COOKIE], "SAPISID=synthetic-session");
                assert_eq!(headers["origin"], ORIGIN);
                assert!(headers[AUTHORIZATION].to_str().unwrap().starts_with("SAPISIDHASH "));
                let request: Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(request["query"], "synthetic search");
                assert_eq!(request["context"]["client"]["clientName"], "WEB_REMIX");
                assert_eq!(request["context"]["client"]["clientVersion"], "1.20261025.01.00");
                assert_eq!(request["context"]["client"]["visitorData"], "synthetic-visitor");
                ([("content-type", "application/json")], r#"{"contents":[]}"#)
            }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let api = Api::connect(
            Session::parse(Some("SAPISID=synthetic-session".into())).unwrap(),
            &origin,
        )
        .await
        .unwrap();
        let response = api
            .post("search", json!({"query":"synthetic search"}))
            .await
            .unwrap();
        assert_eq!(response, json!({"contents":[]}));
        server.abort();
    }

    #[tokio::test]
    async fn bootstrap_consent_redirect_remains_a_refusal() {
        let router = Router::new().route(
            "/",
            get(|| async {
                (
                    axum::http::StatusCode::FOUND,
                    [("location", "https://consent.youtube.com/m")],
                )
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let result = Api::connect(Session::parse(None).unwrap(), &origin).await;
        let error = match result {
            Err(error) => format!("{error:#}"),
            Ok(_) => panic!("redirect accepted"),
        };
        assert!(error.contains("HTTP 302"));
        assert!(!error.contains("https://"));
        server.abort();
    }
    #[test]
    fn credentials_and_authorization_headers_remain_private() {
        assert!(Session::parse(Some("bad".into())).is_err());
        assert!(Session::parse(Some("SAPISID={{secret:example}}".into())).is_err());
        assert!(Session::parse(Some("SAPISID=a\r\nother".into())).is_err());
        let session = Session::parse(Some("SAPISID=synthetic-session; OTHER=fake".into())).unwrap();
        let headers = session.headers(123).unwrap();
        assert!(headers[AUTHORIZATION].is_sensitive());
        assert!(headers[COOKIE].is_sensitive());
        let digest = Sha1::digest(b"123 synthetic-session https://music.youtube.com");
        assert_eq!(
            headers[AUTHORIZATION].to_str().unwrap(),
            format!("SAPISIDHASH 123_{digest:x}")
        );
        assert!(
            Session::parse(None)
                .unwrap()
                .headers(123)
                .unwrap()
                .get(AUTHORIZATION)
                .is_none()
        );
    }
    #[test]
    fn bootstrap_extracts_json_without_accepting_invalid_versions() {
        let page = r#"ytcfg.set({"INNERTUBE_CLIENT_VERSION":"1.20261025.01.00","VISITOR_DATA":"fake-data"});"#;
        assert_eq!(
            config_string(page, "INNERTUBE_CLIENT_VERSION").as_deref(),
            Some("1.20261025.01.00")
        );
        assert!(config_string(page, "OTHER").is_none());
    }
    #[test]
    fn only_bounded_music_candidates_are_returned() {
        let row = json!({"musicResponsiveListItemRenderer":{"navigationEndpoint":{"watchEndpoint":{"videoId":"aaaaaaaaaaa"}},
            "flexColumns":[{"musicResponsiveListItemFlexColumnRenderer":{"text":{"runs":[{"text":"Synthetic title"}]}}}]}});
        let out = search_output(&json!({"items":[row.clone(),row]}), None).unwrap();
        assert_eq!(out["results"].as_array().unwrap().len(), 1);
        assert_eq!(out["results"][0]["title"], "Synthetic title");
        assert!(!out.to_string().contains("cookie"));
        assert_eq!(
            search_output(&json!({}), None).unwrap()["status"],
            "no_match"
        );
    }
}
