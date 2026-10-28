//! Read-only, transitional Music Assistant bridge; no native service authentication.

use std::{collections::BTreeMap, env, fmt, net::IpAddr, time::Duration};

use anyhow::Result;
use reqwest::{
    Client, Url,
    header::{AUTHORIZATION, HeaderValue},
};
use serde_json::{Value, json};

const MAX_RESPONSE: usize = 1024 * 1024;
const MAX_ROWS: usize = 4096;

#[derive(Debug)]
enum BridgeError {
    Configuration,
    Transport,
    Authentication,
    Rejected,
    InvalidResponse,
    ResponseTooLarge,
    InvalidQuery,
}

impl fmt::Display for BridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Configuration => "Configuration Music Assistant absente ou refusée",
            Self::Transport => "Music Assistant injoignable ou délai dépassé",
            Self::Authentication => "Authentification Music Assistant refusée",
            Self::Rejected => "Commande de lecture Music Assistant refusée",
            Self::InvalidResponse => "Réponse Music Assistant non reconnue",
            Self::ResponseTooLarge => "Réponse Music Assistant trop volumineuse",
            Self::InvalidQuery => "Fournisseur ou recherche invalide",
        })
    }
}

impl std::error::Error for BridgeError {}

// No string-command entry point: callers cannot accidentally send playback commands.
#[derive(Clone, Copy)]
enum ReadCommand {
    Info,
    Providers,
    Players,
    ConfigProviders,
    Search,
}

impl ReadCommand {
    const fn name(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Providers => "providers",
            Self::Players => "players/all",
            Self::ConfigProviders => "config/providers",
            Self::Search => "music/search",
        }
    }
}

struct Bridge {
    client: Client,
    endpoint: Url,
    token: String,
    authorization: HeaderValue,
}

fn endpoint(raw: &str) -> std::result::Result<Url, BridgeError> {
    if raw.contains("{{secret:") || raw.len() > 2048 {
        return Err(BridgeError::Configuration);
    }
    // Check the original authority/path as URL parsing normalizes dot segments,
    // empty userinfo and some browser-style alternative spellings.
    let (_, authority_and_path) = raw.split_once("://").ok_or(BridgeError::Configuration)?;
    let authority = authority_and_path.split('/').next().unwrap_or_default();
    let raw_path = authority_and_path
        .strip_prefix(authority)
        .unwrap_or_default();
    if authority.contains('@')
        || raw.contains('\\')
        || raw.chars().any(char::is_whitespace)
        || !matches!(raw_path, "" | "/")
    {
        return Err(BridgeError::Configuration);
    }
    let mut url = Url::parse(raw).map_err(|_| BridgeError::Configuration)?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "" | "/")
    {
        return Err(BridgeError::Configuration);
    }
    let host = url.host_str().ok_or(BridgeError::Configuration)?;
    let literal = host.trim_matches(['[', ']']).parse::<IpAddr>().ok();
    let local = literal.is_some_and(|ip| match ip {
        IpAddr::V4(ip) => ip.is_private() || ip.is_loopback(),
        IpAddr::V6(ip) => ip.is_loopback() || (ip.segments()[0] & 0xfe00 == 0xfc00),
    });
    let dns = literal.is_none()
        && host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        });
    if !(url.scheme() == "http" && local || url.scheme() == "https" && dns) {
        return Err(BridgeError::Configuration);
    }
    url.set_path("/api");
    Ok(url)
}

impl Bridge {
    fn from_env() -> Result<Self> {
        let raw = env::var("MUSIC_ASSISTANT_URL").map_err(|_| BridgeError::Configuration)?;
        let token = env::var("MUSIC_ASSISTANT_TOKEN").map_err(|_| BridgeError::Configuration)?;
        Self::configured(&raw, token)
    }

    fn configured(raw: &str, token: String) -> Result<Self> {
        if token.trim().is_empty() || token.len() > 8192 || token.contains("{{secret:") {
            return Err(BridgeError::Configuration.into());
        }
        let endpoint = endpoint(raw)?;
        let mut authorization = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| BridgeError::Configuration)?;
        authorization.set_sensitive(true);
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .map_err(|_| BridgeError::Configuration)?;
        Ok(Self {
            client,
            endpoint,
            token,
            authorization,
        })
    }

    async fn read(&self, command: ReadCommand, args: Value) -> Result<Value> {
        let mut response = self
            .client
            .post(self.endpoint.clone())
            .header(AUTHORIZATION, self.authorization.clone())
            .json(&json!({"message_id": "readonly-poc", "command": command.name(), "args": args}))
            .send()
            .await
            .map_err(|_| BridgeError::Transport)?;
        if matches!(response.status().as_u16(), 401 | 403) {
            return Err(BridgeError::Authentication.into());
        }
        if !response.status().is_success() {
            return Err(BridgeError::Rejected.into());
        }
        if response
            .content_length()
            .is_some_and(|n| n > MAX_RESPONSE as u64)
        {
            return Err(BridgeError::ResponseTooLarge.into());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| BridgeError::Transport)? {
            if chunk.len() > MAX_RESPONSE.saturating_sub(bytes.len()) {
                return Err(BridgeError::ResponseTooLarge.into());
            }
            bytes.extend_from_slice(&chunk);
        }
        decode_response(&bytes).map_err(Into::into)
    }
}

fn decode_response(bytes: &[u8]) -> std::result::Result<Value, BridgeError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| BridgeError::InvalidResponse)?;
    if value.get("error_code").is_some_and(|v| !v.is_null())
        || value
            .get("error")
            .is_some_and(|v| !v.is_null() && v != &Value::Bool(false))
    {
        return Err(BridgeError::Rejected);
    }
    Ok(value.get("result").cloned().unwrap_or(value))
}

fn rows(value: &Value) -> Vec<&Value> {
    match value {
        Value::Array(values) => values.iter().take(MAX_ROWS).collect(),
        Value::Object(values) => values.values().take(MAX_ROWS).collect(),
        _ => Vec::new(),
    }
}

fn identifier(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}

fn domain(value: &Value) -> Option<&str> {
    value["domain"].as_str().filter(|v| identifier(v, 64))
}

fn safe_text(value: &str, max: usize, token: &str) -> Option<String> {
    if value.contains(token) || value.contains("://") || value.contains("{{secret:") {
        return None;
    }
    let value: String = value
        .chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect();
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn service_status(provider: &str, active: &[&Value], configured: &[&Value]) -> &'static str {
    if active
        .iter()
        .any(|v| domain(v) == Some(provider) && v["available"] == true)
    {
        "available"
    } else if active
        .iter()
        .chain(configured)
        .any(|v| domain(v) == Some(provider))
    {
        "configured_unavailable"
    } else {
        "not_configured"
    }
}

fn inspection(
    info: &Value,
    providers: &Value,
    config: &Value,
    players: &Value,
    token: &str,
) -> Value {
    let active = rows(providers);
    let configured = rows(config);
    let mut domains: BTreeMap<&str, Value> = BTreeMap::new();
    for (is_configured, entries) in [(false, &active), (true, &configured)] {
        for entry in entries {
            let Some(name) = domain(entry).filter(|name| !name.contains(token)) else {
                continue;
            };
            let kind = match entry["type"].as_str() {
                Some("music") => "music",
                Some("player") => "player",
                _ => "unknown",
            };
            let row = domains.entry(name).or_insert_with(|| {
                json!({
                    "domain": name, "type": kind, "configured": false, "available": false,
                    "player_count": 0,
                })
            });
            if kind != "unknown" {
                row["type"] = json!(kind);
            }
            if is_configured {
                row["configured"] = json!(true);
            }
            if !is_configured && entry["available"] == true {
                row["available"] = json!(true);
            }
        }
    }
    let mut unknown_players = 0;
    for player in rows(players) {
        let provider = player["provider"].as_str().unwrap_or_default();
        let matched = active
            .iter()
            .find(|entry| {
                entry["instance_id"].as_str() == Some(provider) || domain(entry) == Some(provider)
            })
            .and_then(|entry| domain(entry));
        if let Some(row) = matched.and_then(|name| domains.get_mut(name)) {
            row["player_count"] = json!(row["player_count"].as_u64().unwrap_or(0) + 1);
        } else {
            unknown_players += 1;
        }
    }
    let version = info["server_version"]
        .as_str()
        .or_else(|| info["version"].as_str())
        .filter(|v| {
            v.len() <= 64
                && v.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-+".contains(&b))
        })
        .filter(|v| !v.contains(token));
    json!({
        "bridge": "music_assistant", "native_auth": false, "read_only": true,
        "version": version, "providers": domains.into_values().collect::<Vec<_>>(),
        "unattributed_player_count": unknown_players,
        "services": {
            "apple_music": service_status("apple_music", &active, &configured),
            "ytmusic": service_status("ytmusic", &active, &configured),
        },
    })
}

/// Inventory only: never returns device identifiers, names, credentials or host URLs.
pub async fn inspect_from_env() -> Result<Value> {
    let bridge = Bridge::from_env()?;
    let info = bridge.read(ReadCommand::Info, json!({})).await?;
    let providers = bridge.read(ReadCommand::Providers, json!({})).await?;
    let config = bridge.read(ReadCommand::ConfigProviders, json!({})).await?;
    let players = bridge.read(ReadCommand::Players, json!({})).await?;
    Ok(inspection(
        &info,
        &providers,
        &config,
        &players,
        &bridge.token,
    ))
}

fn valid_query(provider: &str, query: &str) -> bool {
    matches!(provider, "apple_music" | "ytmusic")
        && !query.trim().is_empty()
        && query.len() <= 800
        && query.chars().count() <= 200
        && !query.chars().any(char::is_control)
        && !query.contains("{{secret:")
}

fn item_id(raw: &str) -> bool {
    !raw.is_empty()
        && !matches!(raw, "." | "..")
        && raw.len() <= 256
        && raw
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}

fn library_uri<'a>(raw: &'a str, provider: &str, instance: &str) -> Option<&'a str> {
    if raw.len() > 512 {
        return None;
    }
    let (scheme, path) = raw.split_once("://")?;
    if !["library", provider, instance].contains(&scheme) {
        return None;
    }
    let id = path.strip_prefix("track/")?;
    item_id(id).then_some(raw)
}

fn search_output(answer: &Value, provider: &str, instance: &str, token: &str) -> Value {
    let mut tracks = Vec::new();
    for track in answer["tracks"].as_array().into_iter().flatten().take(100) {
        if tracks.len() == 5 {
            break;
        }
        // A library hit is attributable only through its original provider mapping.
        let direct = track["provider"]
            .as_str()
            .filter(|p| *p == instance || *p == provider)
            .and_then(|p| track["item_id"].as_str().map(|id| (p, id)));
        let mapped = track["provider_mappings"]
            .as_array()
            .into_iter()
            .flatten()
            .find_map(|m| {
                let origin = m["provider_instance"].as_str()?;
                (origin == instance && m["provider_domain"].as_str() == Some(provider))
                    .then(|| m["item_id"].as_str().map(|id| (origin, id)))
                    .flatten()
            });
        let Some((origin, id)) = mapped.or(direct) else {
            continue;
        };
        if !item_id(id) || id.contains(token) || origin.contains(token) {
            continue;
        }
        let Some(uri) = track["uri"]
            .as_str()
            .and_then(|v| library_uri(v, provider, instance))
            .filter(|v| !v.contains(token))
        else {
            continue;
        };
        let Some((scheme, uri_id)) = uri.split_once("://track/") else {
            continue;
        };
        let identity_matches = if scheme == "library" {
            track["provider"] == "library" && track["item_id"].as_str() == Some(uri_id)
        } else {
            uri_id == id
        };
        if !identity_matches {
            continue;
        }
        let Some(title) = track["name"]
            .as_str()
            .and_then(|v| safe_text(v, 160, token))
        else {
            continue;
        };
        let artists: Vec<String> = track["artists"]
            .as_array()
            .into_iter()
            .flatten()
            .take(3)
            .filter_map(|a| a["name"].as_str().and_then(|v| safe_text(v, 80, token)))
            .collect();
        tracks.push(json!({"provider": origin, "item_id": id, "title": title,
            "artists": artists, "uri": uri}));
    }
    json!({"bridge": "music_assistant", "native_auth": false, "provider": provider,
        "status": if tracks.is_empty() { "no_match" } else { "ok" }, "tracks": tracks})
}

/// Search an already configured MA provider; no native Apple/YouTube login or playback.
pub async fn search_from_env(provider: &str, query: &str) -> Result<Value> {
    if !valid_query(provider, query) {
        return Err(BridgeError::InvalidQuery.into());
    }
    let bridge = Bridge::from_env()?;
    let providers = bridge.read(ReadCommand::Providers, json!({})).await?;
    let entries = rows(&providers);
    let matching: Vec<_> = entries
        .iter()
        .filter(|p| domain(p) == Some(provider))
        .collect();
    let instance = matching
        .iter()
        .filter(|p| p["available"] != false)
        .find_map(|p| p["instance_id"].as_str().filter(|id| identifier(id, 128)));
    let Some(instance) = instance else {
        return Ok(json!({"bridge": "music_assistant", "native_auth": false,
            "provider": provider, "status": if matching.is_empty() {
                "not_configured"
            } else { "unavailable" }, "tracks": []}));
    };
    let answer = bridge
        .read(
            ReadCommand::Search,
            json!({
                "search_query": query.trim(), "media_types": ["track"], "limit": 5,
                "provider_instance_or_domain": instance,
            }),
        )
        .await?;
    Ok(search_output(&answer, provider, instance, &bridge.token))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::{Body, Bytes},
        http::{HeaderMap, Response, StatusCode},
        routing::post,
    };
    use std::{
        convert::Infallible,
        pin::Pin,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        task::{Context, Poll},
    };

    async fn serve(router: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        (format!("http://{address}"), task)
    }

    #[tokio::test]
    async fn http_read_sends_authenticated_envelope_and_decodes_result() {
        let router = Router::new().route(
            "/api",
            post(|headers: HeaderMap, body: Bytes| async move {
                // Sensitivity is a local logging flag, not a property transmitted over HTTP.
                assert!(
                    headers
                        .get(AUTHORIZATION)
                        .is_some_and(|v| v == "Bearer fixture-token")
                );
                let request: Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(
                    request,
                    json!({
                        "message_id":"readonly-poc", "command":"music/search", "args":{
                            "search_query":"fixture song", "media_types":["track"], "limit":5,
                            "provider_instance_or_domain":"ytmusic--fixture"
                        }
                    })
                );
                (
                    [("content-type", "application/json")],
                    r#"{"message_id":"readonly-poc","result":{"tracks":[]}}"#,
                )
            }),
        );
        let (url, server) = serve(router).await;
        let bridge = Bridge::configured(&url, "fixture-token".to_owned()).unwrap();
        assert!(bridge.authorization.is_sensitive());
        assert!(!format!("{:?}", bridge.authorization).contains("fixture-token"));
        let answer = bridge
            .read(
                ReadCommand::Search,
                json!({
                    "search_query":"fixture song", "media_types":["track"], "limit":5,
                    "provider_instance_or_domain":"ytmusic--fixture"
                }),
            )
            .await
            .unwrap();
        assert_eq!(answer, json!({"tracks":[]}));
        server.abort();
    }

    #[tokio::test]
    async fn http_redirect_is_never_followed() {
        let followed = Arc::new(AtomicUsize::new(0));
        let calls = Arc::clone(&followed);
        let router = Router::new()
            .route(
                "/api",
                post(|| async { (StatusCode::TEMPORARY_REDIRECT, [("location", "/target")]) }),
            )
            .route(
                "/target",
                post(move || {
                    let calls = Arc::clone(&calls);
                    async move {
                        calls.fetch_add(1, Ordering::Relaxed);
                        "{}"
                    }
                }),
            );
        let (url, server) = serve(router).await;
        let bridge = Bridge::configured(&url, "fixture-token".to_owned()).unwrap();
        let error = bridge.read(ReadCommand::Info, json!({})).await.unwrap_err();
        assert!(matches!(
            error.downcast_ref::<BridgeError>(),
            Some(BridgeError::Rejected)
        ));
        assert_eq!(followed.load(Ordering::Relaxed), 0);
        server.abort();
    }

    // Unknown content length exercises the incremental limit, not just the header check.
    struct LargeBody {
        remaining: usize,
    }

    impl http_body::Body for LargeBody {
        type Data = Bytes;
        type Error = Infallible;

        fn poll_frame(
            mut self: Pin<&mut Self>,
            _: &mut Context<'_>,
        ) -> Poll<Option<std::result::Result<http_body::Frame<Bytes>, Infallible>>> {
            if self.remaining == 0 {
                return Poll::Ready(None);
            }
            let len = self.remaining.min(32 * 1024);
            self.remaining -= len;
            Poll::Ready(Some(Ok(http_body::Frame::data(Bytes::from(vec![
                b'x';
                len
            ])))))
        }
    }

    #[tokio::test]
    async fn http_response_is_bounded_without_content_length() {
        let router = Router::new().route(
            "/api",
            post(|| async {
                Response::new(Body::new(LargeBody {
                    remaining: MAX_RESPONSE + 1,
                }))
            }),
        );
        let (url, server) = serve(router).await;
        let bridge = Bridge::configured(&url, "fixture-token".to_owned()).unwrap();
        let error = bridge
            .read(ReadCommand::Providers, json!({}))
            .await
            .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<BridgeError>(),
            Some(BridgeError::ResponseTooLarge)
        ));
        assert!(!error.to_string().contains("fixture-token"));
        assert!(!error.to_string().contains(&url));
        server.abort();
    }

    #[test]
    fn endpoint_is_explicit_and_constrained() {
        for raw in [
            "http://192.168.1.2:8095",
            "http://127.0.0.1:8095/",
            "http://[::1]:8095",
            "https://music.example.com",
        ] {
            assert!(endpoint(raw).is_ok());
        }
        for raw in [
            "http://music.example.com",
            "http://8.8.8.8",
            "https://127.0.0.1",
            "http://user:pw@127.0.0.1",
            "http://@127.0.0.1",
            "http://127.0.0.1/api",
            "http://127.0.0.1/../",
            "http://127.0.0.1/?token=x",
            "http://127.0.0.1/#x",
            "{{secret:server}}",
        ] {
            assert!(endpoint(raw).is_err());
        }
    }

    #[test]
    fn inspection_omits_private_metadata() {
        let providers = json!([{"domain":"ytmusic", "instance_id":"private-provider-id", "type":"music", "available":true,
            "name":"Private Name", "token":"sensitive-token", "url":"http://192.168.1.5"}]);
        let result = inspection(
            &json!({"server_version":"2.7.0"}),
            &providers,
            &providers,
            &json!([{"provider":"private-provider-id", "player_id":"private-player-id", "name":"Bedroom"}]),
            "sensitive-token",
        );
        let text = result.to_string();
        for private in [
            "private-provider-id",
            "private-player-id",
            "Private Name",
            "sensitive-token",
            "192.168.1.5",
            "Bedroom",
        ] {
            assert!(!text.contains(private));
        }
        assert_eq!(result["services"]["ytmusic"], "available");
        assert_eq!(result["providers"][0]["player_count"], 1);
    }

    #[test]
    fn response_errors_never_repeat_upstream_text() {
        let err = decode_response(br#"{"error_code":7,"details":"private-token http://host"}"#)
            .unwrap_err()
            .to_string();
        assert!(!err.contains("private-token"));
        assert!(!err.contains("host"));
        assert_eq!(decode_response(br#"{"result":[1]}"#).unwrap(), json!([1]));
        assert_eq!(decode_response(br#"[1]"#).unwrap(), json!([1]));
    }

    #[test]
    fn query_and_library_references_are_bounded() {
        assert!(valid_query("apple_music", "a track"));
        assert!(!valid_query("spotify", "a track"));
        assert!(!valid_query("ytmusic", &"a".repeat(201)));
        assert!(!valid_query("ytmusic", "x\n"));
        for raw in [
            "https://host/track/1",
            "builtin://track/1",
            "ytmusic://track/http://host",
            "ytmusic://track/%2fetc",
            "ytmusic://track/../secret",
        ] {
            assert!(library_uri(raw, "ytmusic", "ytmusic--one").is_none());
        }
        assert_eq!(
            library_uri("library://track/12", "ytmusic", "ytmusic--one"),
            Some("library://track/12")
        );
    }

    #[test]
    fn search_keeps_actual_provider_and_drops_foreign_results() {
        let answer = json!({"tracks":[
            {"provider":"ytmusic--one","item_id":"video1","name":"Song","uri":"ytmusic--one://track/video1", "artists":[{"name":"Artist"}]},
            {"provider":"spotify","item_id":"other","name":"Foreign","uri":"spotify://track/other"},
            {"provider":"ytmusic--one","item_id":"video2","name":"sensitive-token","uri":"ytmusic--one://track/video2"}
        ]});
        let output = search_output(&answer, "ytmusic", "ytmusic--one", "sensitive-token");
        assert_eq!(output["tracks"].as_array().unwrap().len(), 1);
        assert_eq!(output["tracks"][0]["provider"], "ytmusic--one");
        assert!(!output.to_string().contains("sensitive-token"));
    }
}
