//! Direct Apple Music REST access. Full-track streaming is not implemented.

use std::env;

use anyhow::{Result, anyhow};
use reqwest::{
    Client, Url,
    header::{AUTHORIZATION, HeaderValue},
};
use serde_json::{Value, json};

use super::http;

const ORIGIN: &str = "https://api.music.apple.com";
const USER_TOKEN_HEADER: &str = "Music-User-Token";

struct Credentials {
    developer: Option<HeaderValue>,
    user: Option<HeaderValue>,
}

fn token(raw: Option<String>, bearer: bool) -> Option<HeaderValue> {
    let raw = raw?;
    if raw.is_empty()
        || raw.len() > 8192
        || raw.contains("{{secret:")
        || raw
            .bytes()
            .any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
    {
        return None;
    }
    let value = if bearer { format!("Bearer {raw}") } else { raw };
    let mut header = HeaderValue::from_str(&value).ok()?;
    header.set_sensitive(true);
    Some(header)
}

impl Credentials {
    fn from_env() -> Result<Self> {
        let developer = match env::var("APPLE_MUSIC_DEVELOPER_TOKEN") {
            Ok(value) => Some(value),
            Err(env::VarError::NotPresent) => super::apple_token::from_env()?,
            Err(_) => return Err(anyhow!("Invalid Apple Music configuration")),
        };
        Ok(Self {
            developer: token(developer, true),
            user: token(env::var("APPLE_MUSIC_USER_TOKEN").ok(), false),
        })
    }

    fn status(&self, needs_user: bool) -> Value {
        json!({
            "provider": "apple_music", "native": true,
            "status": if self.developer.is_some() && (!needs_user || self.user.is_some()) {
                "credentials_present"
            } else { "credentials_required" },
            "developer_token_configured": self.developer.is_some(),
            "user_token_configured": self.user.is_some(),
            "full_stream": "not_implemented",
        })
    }

    fn contains_secret(&self, text: &str) -> bool {
        [&self.developer, &self.user]
            .into_iter()
            .flatten()
            .any(|value| {
                value.to_str().ok().is_some_and(|value| {
                    text.contains(value.strip_prefix("Bearer ").unwrap_or(value))
                })
            })
    }
}

fn valid_storefront(raw: &str) -> bool {
    raw.len() == 2 && raw.bytes().all(|b| b.is_ascii_lowercase())
}

fn storefront_from_env() -> Result<String> {
    let storefront = match env::var("APPLE_MUSIC_STOREFRONT") {
        Ok(value) => value,
        Err(env::VarError::NotPresent) => "fr".to_owned(),
        Err(env::VarError::NotUnicode(_)) => return Err(anyhow!("Invalid Apple Music storefront")),
    };
    if !valid_storefront(&storefront) {
        return Err(anyhow!("Invalid Apple Music storefront"));
    }
    Ok(storefront)
}

fn valid_query(query: &str) -> bool {
    !query.trim().is_empty()
        && query.len() <= 800
        && query.chars().count() <= 200
        && !query.chars().any(char::is_control)
        && !query.contains("{{secret:")
}

struct AppleClient {
    client: Client,
    origin: Url,
    credentials: Credentials,
}

impl AppleClient {
    fn new(credentials: Credentials) -> Result<Self> {
        Ok(Self {
            client: http::client()?,
            origin: Url::parse(ORIGIN).map_err(|_| anyhow!("Invalid Apple Music origin"))?,
            credentials,
        })
    }

    #[cfg(test)]
    fn with_test_origin(credentials: Credentials, origin: &str) -> Result<Self> {
        let mut client = Self::new(credentials)?;
        client.origin = Url::parse(origin).map_err(|_| anyhow!("Invalid fixture origin"))?;
        Ok(client)
    }

    fn request(&self, path: &str, personalized: bool) -> Result<reqwest::RequestBuilder> {
        let url = self
            .origin
            .join(path)
            .map_err(|_| anyhow!("Invalid Apple Music path"))?;
        let developer = self
            .credentials
            .developer
            .as_ref()
            .ok_or_else(|| anyhow!("Apple Music developer credentials required"))?;
        let mut request = self
            .client
            .get(url)
            .header(AUTHORIZATION, developer.clone());
        if personalized {
            let user = self
                .credentials
                .user
                .as_ref()
                .ok_or_else(|| anyhow!("Apple Music user credentials required"))?;
            request = request.header(USER_TOKEN_HEADER, user.clone());
        }
        Ok(request)
    }

    async fn search(&self, storefront: &str, query: &str) -> Result<Value> {
        if self.credentials.developer.is_none() {
            return Ok(self.credentials.status(false));
        }
        let request = self
            .request(&format!("/v1/catalog/{storefront}/search"), false)?
            .query(&[("types", "songs"), ("limit", "5"), ("term", query.trim())]);
        let response = http::json(request).await?;
        Ok(search_response(&response, &self.credentials))
    }

    async fn account(&self) -> Result<Value> {
        if self.credentials.developer.is_none() || self.credentials.user.is_none() {
            return Ok(self.credentials.status(true));
        }
        let response = http::json(self.request("/v1/me/storefront", true)?).await?;
        account_response(&response)
    }
}

fn clean_text(raw: &str, credentials: &Credentials) -> Option<String> {
    if credentials.contains_secret(raw) || raw.contains("://") || raw.contains("{{secret:") {
        return None;
    }
    let text: String = raw.chars().filter(|c| !c.is_control()).take(160).collect();
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

fn search_response(response: &Value, credentials: &Credentials) -> Value {
    let tracks: Vec<Value> = response["results"]["songs"]["data"]
        .as_array()
        .into_iter()
        .flatten()
        .take(100)
        .filter_map(|song| {
            let id = song["id"].as_str()?;
            if id.is_empty()
                || id.len() > 64
                || !id.bytes().all(|b| b.is_ascii_digit())
                || credentials.contains_secret(id)
            {
                return None;
            }
            let attributes = &song["attributes"];
            let title = clean_text(attributes["name"].as_str()?, credentials)?;
            let artist = attributes["artistName"]
                .as_str()
                .and_then(|name| clean_text(name, credentials));
            let preview_available = attributes["previews"].as_array().is_some_and(|items| {
                items
                    .iter()
                    .take(10)
                    .any(|item| item["url"].as_str().is_some_and(|url| !url.is_empty()))
            });
            Some(
                json!({"provider":"apple_music", "item_id":id, "title":title,
                "artist":artist, "preview_available":preview_available}),
            )
        })
        .take(5)
        .collect();
    json!({"provider":"apple_music", "native":true,
        "status":if tracks.is_empty() { "no_match" } else { "ok" },
        "full_stream":"not_implemented", "tracks":tracks})
}

fn account_response(response: &Value) -> Result<Value> {
    let storefront = response["data"]
        .as_array()
        .and_then(|items| items.first())
        .and_then(|item| item["id"].as_str())
        .filter(|id| valid_storefront(id))
        .ok_or_else(|| anyhow!("Invalid Apple Music authorization response"))?;
    Ok(
        json!({"provider":"apple_music", "native":true, "status":"authorized",
        "storefront":storefront, "full_stream":"not_implemented"}),
    )
}

/// Describes available credentials; does not assert that Apple has accepted them.
pub fn status() -> Value {
    match Credentials::from_env() {
        Ok(credentials) => credentials.status(false),
        Err(_) => {
            json!({"provider":"apple_music","native":true,"status":"invalid_signing_configuration","full_stream":"not_implemented"})
        }
    }
}

/// Searches the official catalogue directly; returns metadata, not complete audio.
pub async fn search(query: &str) -> Result<Value> {
    if !valid_query(query) {
        return Err(anyhow!("Invalid Apple Music query"));
    }
    let credentials = Credentials::from_env()?;
    if credentials.developer.is_none() {
        return Ok(credentials.status(false));
    }
    AppleClient::new(credentials)?
        .search(&storefront_from_env()?, query)
        .await
}

/// Validates user authorization and returns only its storefront, not account details.
pub async fn account() -> Result<Value> {
    let credentials = Credentials::from_env()?;
    if credentials.developer.is_none() || credentials.user.is_none() {
        return Ok(credentials.status(true));
    }
    AppleClient::new(credentials)?.account().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        http::{HeaderMap, Uri},
        routing::get,
    };

    fn fixture_credentials() -> Credentials {
        Credentials {
            developer: token(Some("fixture-developer".to_owned()), true),
            user: token(Some("fixture-user".to_owned()), false),
        }
    }

    async fn serve(router: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        (format!("http://{address}"), task)
    }

    #[test]
    fn rejects_invalid_credentials_and_bounds_inputs() {
        for raw in ["", "{{secret:apple}}", "a\nb", "white space"] {
            assert!(token(Some(raw.to_owned()), false).is_none());
        }
        assert!(token(Some("a".repeat(8193)), true).is_none());
        assert!(valid_query("a song"));
        assert!(!valid_query(&"a".repeat(201)));
        assert!(!valid_query(&"🦀".repeat(201)));
        assert!(valid_storefront("fr"));
        assert!(!valid_storefront("FR"));
        assert!(!valid_storefront("../"));
        let credentials = fixture_credentials();
        assert!(credentials.developer.as_ref().unwrap().is_sensitive());
        assert!(credentials.user.as_ref().unwrap().is_sensitive());
    }

    #[tokio::test]
    async fn missing_credentials_never_require_an_environment_mutation_or_request() {
        let credentials = Credentials {
            developer: None,
            user: None,
        };
        assert_eq!(credentials.status(false)["status"], "credentials_required");
        let client = AppleClient::with_test_origin(credentials, "http://127.0.0.1:1").unwrap();
        assert_eq!(
            client.search("fr", "song").await.unwrap()["status"],
            "credentials_required"
        );
        assert_eq!(
            client.account().await.unwrap()["status"],
            "credentials_required"
        );
    }

    #[test]
    fn parses_only_bounded_metadata_and_never_exposes_urls_or_tokens() {
        let fixture = json!({"results":{"songs":{"data":[
            {"id":"123", "attributes":{"name":"Song", "artistName":"Artist", "url":"https://private.example/", "previews":[{"url":"https://preview.example/"}]}},
            {"id":"https://bad.example", "attributes":{"name":"Bad"}},
            {"id":"456", "attributes":{"name":"fixture-user"}}
        ]}}});
        let output = search_response(&fixture, &fixture_credentials());
        assert_eq!(output["tracks"].as_array().unwrap().len(), 1);
        assert_eq!(output["tracks"][0]["preview_available"], true);
        for forbidden in [
            "https://",
            "private.example",
            "fixture-user",
            "fixture-developer",
        ] {
            assert!(!output.to_string().contains(forbidden));
        }
        let account = account_response(
            &json!({"data":[{"id":"fr", "name":"private account", "userId":"private-id"}]}),
        )
        .unwrap();
        assert_eq!(account["storefront"], "fr");
        assert!(!account.to_string().contains("private"));
    }

    #[tokio::test]
    async fn direct_http_search_uses_developer_header_and_encoded_parameters() {
        let router = Router::new().route("/v1/catalog/fr/search", get(|headers: HeaderMap, uri: Uri| async move {
            assert!(headers.get(AUTHORIZATION).is_some_and(|v| v == "Bearer fixture-developer"));
            assert!(headers.get(USER_TOKEN_HEADER).is_none());
            let url = Url::parse(&format!("http://fixture.invalid{uri}")).unwrap();
            let params: std::collections::BTreeMap<_, _> = url.query_pairs().collect();
            assert_eq!(params.get("types").map(|v| v.as_ref()), Some("songs"));
            assert_eq!(params.get("limit").map(|v| v.as_ref()), Some("5"));
            assert_eq!(params.get("term").map(|v| v.as_ref()), Some("été & son"));
            ([("content-type", "application/json")], r#"{"results":{"songs":{"data":[{"id":"123","attributes":{"name":"Song","artistName":"Artist","previews":[]}}]}}}"#)
        }));
        let (origin, server) = serve(router).await;
        let client = AppleClient::with_test_origin(fixture_credentials(), &origin).unwrap();
        let output = client.search("fr", "été & son").await.unwrap();
        assert_eq!(output["tracks"][0]["item_id"], "123");
        assert_eq!(output["full_stream"], "not_implemented");
        server.abort();
    }

    #[tokio::test]
    async fn direct_http_account_uses_both_headers_and_returns_storefront_only() {
        let router = Router::new().route("/v1/me/storefront", get(|headers: HeaderMap| async move {
            assert!(headers.get(AUTHORIZATION).is_some_and(|v| v == "Bearer fixture-developer"));
            assert!(headers.get(USER_TOKEN_HEADER).is_some_and(|v| v == "fixture-user"));
            ([("content-type", "application/json")], r#"{"data":[{"id":"fr","attributes":{"name":"private user","account":"private-id"}}]}"#)
        }));
        let (origin, server) = serve(router).await;
        let client = AppleClient::with_test_origin(fixture_credentials(), &origin).unwrap();
        let output = client.account().await.unwrap();
        assert_eq!(output["storefront"], "fr");
        assert!(!output.to_string().contains("private"));
        server.abort();
    }
}
