//! Actual HTTP router, sessions and optional native actor; no provider request.

use std::fmt::Write as _;
use std::net::SocketAddr;
use std::path::PathBuf;

use axum::http::{HeaderMap, header};
use moli_runtime::{Hub, HubOptions, MasterKey};
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use crate::caller::{Caller, Via};
use crate::session::{Sessions, UiPin};

fn generated() -> String {
    let mut random = [0_u8; 32];
    SystemRandom::new().fill(&mut random).unwrap();
    let mut encoded = String::with_capacity(random.len() * 2);
    for byte in random {
        write!(encoded, "{byte:02x}").unwrap();
    }
    encoded
}

#[test]
fn endpoint_uses_canonical_origin_and_keeps_hostile_origins_out() {
    for (configured, host, origin) in [
        (
            "https://EXAMPLE.COM:443",
            "example.com",
            "https://example.com",
        ),
        (
            "https://Example.com:8443",
            "example.com:8443",
            "https://example.com:8443",
        ),
        ("http://LOCALHOST:80", "localhost", "http://localhost"),
        ("http://127.0.0.1:80", "127.0.0.1", "http://127.0.0.1"),
        ("http://[0:0:0:0:0:0:0:1]:80", "[::1]", "http://[::1]"),
        (
            "https://[::ffff:192.0.2.1]:443",
            "[::ffff:c000:201]",
            "https://[::ffff:c000:201]",
        ),
        (
            "https://[::FFFF:C000:0201]",
            "[::ffff:c000:201]",
            "https://[::ffff:c000:201]",
        ),
    ] {
        let driver = moli_netatmo::Netatmo::new(moli_netatmo::Config {
            public_origin: configured.into(),
            poll_seconds: 300,
            stale_seconds: 1800,
        })
        .unwrap();
        let endpoint = super::Endpoint::new(driver.gateway(), configured.into()).unwrap();
        assert_eq!(endpoint.origin, origin);
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, host.parse().unwrap());
        headers.insert(header::ORIGIN, origin.parse().unwrap());
        assert!(endpoint.same_origin(&headers));
        for rejected in [
            "https://other.example.com",
            "https://example.com:444",
            "https://user@example.com",
            "https://example.com/?query=1",
            "https://example.com/#fragment",
            "https://example.com/other",
            "https://192.0.2.1",
        ] {
            headers.insert(header::ORIGIN, rejected.parse().unwrap());
            assert!(!endpoint.same_origin(&headers));
        }
        for suffix in ["#fragment", "/#fragment", "/#"] {
            headers.insert(header::ORIGIN, format!("{origin}{suffix}").parse().unwrap());
            assert!(!endpoint.same_origin(&headers));
        }
        headers.insert(header::ORIGIN, origin.parse().unwrap());
        headers.insert(header::HOST, "other.example.com".parse().unwrap());
        assert!(!endpoint.same_origin(&headers));
    }
}

fn pin() -> String {
    let mut random = [0_u8; 6];
    SystemRandom::new().fill(&mut random).unwrap();
    random
        .iter()
        .map(|byte| char::from(b'0' + byte % 10))
        .collect()
}

struct Server {
    addr: SocketAddr,
    pin: String,
    task: tokio::task::JoinHandle<()>,
    cancel: CancellationToken,
    driver: Option<tokio::task::JoinHandle<()>>,
    _vault: Option<Vault>,
}

impl Server {
    async fn new() -> Self {
        Self::start(false).await
    }

    async fn active() -> Self {
        Self::start(true).await
    }

    async fn start(active: bool) -> Self {
        let (vault, hub) = if active {
            let (vault, hub) = Vault::hub();
            (Some(vault), hub)
        } else {
            (None, Hub::new(HubOptions::default()).unwrap())
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let pin = pin();
        let mut options = crate::Options {
            ui_pin: Some(UiPin::new(&pin)),
            ..crate::Options::default()
        };
        let cancel = CancellationToken::new();
        let driver = if active {
            let instance = moli_core::InstanceId::from("weather");
            for key in ["client_id", "client_secret"] {
                hub.store_secret(&instance, key, &generated())
                    .await
                    .unwrap();
            }
            let origin = format!("http://{addr}");
            let native = moli_netatmo::Netatmo::new(moli_netatmo::Config {
                public_origin: origin.clone(),
                poll_seconds: 300,
                stale_seconds: 1800,
            })
            .unwrap();
            let gateway = native.gateway();
            options.netatmo.insert(
                instance.clone(),
                super::Endpoint::new(gateway.clone(), origin).unwrap(),
            );
            options.allowed_hosts = vec!["localhost".into()];
            let driver = moli_runtime::spawn_driver(
                &hub,
                instance,
                std::sync::Arc::new(native),
                cancel.clone(),
            );
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                while gateway.status() != moli_netatmo::Status::NeedsAuthorization {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("synthetic keys loaded without provider request");
            Some(driver)
        } else {
            None
        };
        let app = crate::router(hub, cancel.clone(), &options);
        let shutdown = cancel.clone();
        let task = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(shutdown.cancelled_owned())
            .await
            .unwrap();
        });
        Self {
            addr,
            pin,
            task,
            cancel,
            driver,
            _vault: vault,
        }
    }

    async fn request(
        &self,
        method: &str,
        path: &str,
        body: &str,
        extra: &str,
    ) -> (u16, String, String) {
        self.request_host(method, path, body, extra, &self.addr.to_string())
            .await
    }

    async fn request_host(
        &self,
        method: &str,
        path: &str,
        body: &str,
        extra: &str,
        host: &str,
    ) -> (u16, String, String) {
        tokio::time::timeout(std::time::Duration::from_secs(2),async {
            let mut stream=tokio::net::TcpStream::connect(self.addr).await.unwrap();
            let request=format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n{body}",body.len());
            stream.write_all(request.as_bytes()).await.unwrap();
            let mut response=Vec::new();
            stream.take(128*1024).read_to_end(&mut response).await.unwrap();
            let response=String::from_utf8(response).unwrap();
            let status=response[9..12].parse().unwrap();
            let (headers,body)=response.split_once("\r\n\r\n").unwrap();
            (status,headers.to_owned(),body.to_owned())
        }).await.expect("HTTP contract within two seconds")
    }

    async fn login(&self) -> String {
        let (status, headers, _) = self
            .request(
                "POST",
                "/api/session",
                &json!({"pin":self.pin}).to_string(),
                "",
            )
            .await;
        assert_eq!(status, 200);
        let cookie = headers
            .lines()
            .find_map(|line| {
                let (key, value) = line.split_once(':')?;
                key.eq_ignore_ascii_case("set-cookie")
                    .then(|| value.trim().split(';').next().unwrap().to_owned())
            })
            .unwrap();
        assert!(headers.contains("SameSite=Strict"));
        format!("Cookie: {cookie}\r\n")
    }

    async fn authorization(&self, cookie: &str) -> String {
        let (status, _, body) = self
            .request(
                "POST",
                "/api/integrations/netatmo/weather/authorize",
                "{}",
                cookie,
            )
            .await;
        assert_eq!(status, 200);
        let body: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["expires_in"], 600);
        assert_eq!(body.as_object().unwrap().len(), 2);
        body["url"].as_str().unwrap().to_owned()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.task.abort();
        if let Some(driver) = &self.driver {
            driver.abort();
        }
    }
}

#[tokio::test]
async fn active_status_lists_only_the_enabled_instance_without_credentials() {
    let server = Server::active().await;
    assert_eq!(
        server
            .request("GET", "/api/integrations/netatmo", "", "")
            .await
            .0,
        403
    );
    let cookie = server.login().await;
    let (status, _, body) = server
        .request("GET", "/api/integrations/netatmo", "", &cookie)
        .await;
    assert_eq!(status, 200);
    let body: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        body,
        json!({"instances":[{"instance":"weather","status":"needs_authorization"}]})
    );
}

#[tokio::test]
async fn active_authorization_uses_fixed_provider_scope_and_exact_callback() {
    let server = Server::active().await;
    assert_eq!(
        server
            .request(
                "POST",
                "/api/integrations/netatmo/weather/authorize",
                "{}",
                ""
            )
            .await
            .0,
        403
    );
    let cookie = server.login().await;
    let url: http::Uri = server.authorization(&cookie).await.parse().unwrap();
    assert_eq!(url.scheme_str(), Some("https"));
    assert_eq!(url.authority().unwrap().as_str(), "api.netatmo.com");
    assert_eq!(url.path(), "/oauth2/authorize");
    let query: std::collections::BTreeMap<_, _> = url
        .query()
        .unwrap()
        .split('&')
        .map(|part| part.split_once('=').unwrap())
        .collect();
    assert_eq!(query.len(), 5);
    assert_eq!(query["scope"], "read_station");
    assert_eq!(query["response_type"], "code");
    assert_eq!(query["state"].len(), 64);
    assert!(query["state"].bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_eq!(
        query["redirect_uri"],
        format!(
            "http%3A%2F%2F127.0.0.1%3A{}%2Fapi%2Fintegrations%2Fnetatmo%2Fweather%2Fcallback",
            server.addr.port()
        )
    );
    assert!(query.contains_key("client_id"));
    assert!(!query.contains_key("client_secret"));
}

#[tokio::test]
async fn active_callback_refuses_other_session_and_wrong_state_before_provider() {
    let server = Server::active().await;
    let first = server.login().await;
    let second = server.login().await;
    let uri: http::Uri = server.authorization(&first).await.parse().unwrap();
    let state = uri
        .query()
        .unwrap()
        .split('&')
        .find_map(|part| part.strip_prefix("state="))
        .unwrap();
    let body = json!({"state":state,"code":generated()}).to_string();
    let (status, _, response) = server
        .request(
            "POST",
            "/api/integrations/netatmo/weather/callback",
            &body,
            &second,
        )
        .await;
    assert_eq!(status, 403);
    assert!(!response.contains(state));
    let mut wrong = state.to_owned();
    wrong.replace_range(..1, if &state[..1] == "0" { "1" } else { "0" });
    let body = json!({"state":wrong,"code":generated()}).to_string();
    assert_eq!(
        server
            .request(
                "POST",
                "/api/integrations/netatmo/weather/callback",
                &body,
                &first
            )
            .await
            .0,
        403
    );
    let (status, _, body) = server
        .request("GET", "/api/integrations/netatmo", "", &first)
        .await;
    assert_eq!(status, 200);
    let body: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(body["instances"][0]["status"], "authorizing");
}

#[tokio::test]
async fn active_host_alias_is_refused_before_authorization_and_exchange() {
    let server = Server::active().await;
    let cookie = server.login().await;
    let alias = format!("localhost:{}", server.addr.port());
    for (path, body) in [
        ("authorize", "{}".to_owned()),
        (
            "callback",
            json!({"state":generated(),"code":generated()}).to_string(),
        ),
    ] {
        let path = format!("/api/integrations/netatmo/weather/{path}");
        let (status, _, body) = server
            .request_host("POST", &path, &body, &cookie, &alias)
            .await;
        assert_eq!(status, 403);
        let body: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["error"], moli_i18n::tr!("netatmo.http.origin"));
    }
}

#[tokio::test]
async fn authorize_requires_human_before_instance_lookup() {
    let server = Server::new().await;
    assert_eq!(
        server
            .request(
                "POST",
                "/api/integrations/netatmo/weather/authorize",
                "{}",
                ""
            )
            .await
            .0,
        403
    );
}

#[tokio::test]
async fn callback_without_cookie_does_not_exchange() {
    let server = Server::new().await;
    let (status, _, body) = server
        .request(
            "POST",
            "/api/integrations/netatmo/weather/callback",
            "{}",
            "",
        )
        .await;
    assert_eq!(status, 403);
    assert!(!body.contains("api.netatmo.com"));
}

#[tokio::test]
async fn callback_html_never_embeds_code() {
    let server = Server::new().await;
    let code = generated();
    let path = format!("/api/integrations/netatmo/weather/callback?code={code}&state={code}");
    let (status, headers, body) = server.request("GET", &path, "", "").await;
    assert_eq!(status, 200);
    assert!(!body.contains(&code));
    let headers = headers.to_ascii_lowercase();
    assert!(headers.contains("referrer-policy: no-referrer"));
    assert!(headers.contains("cache-control: no-store"));
    assert!(headers.contains("default-src 'none'"));
    assert!(headers.contains("script-src 'self'"));
    assert!(headers.contains("connect-src 'self'"));
    assert!(!headers.contains("unsafe-inline"));
    assert!(body.contains("/api/integrations/netatmo/callback.js"));
}

#[tokio::test]
async fn status_is_human_only_and_empty_without_enabled_instances() {
    let server = Server::new().await;
    assert_eq!(
        server
            .request("GET", "/api/integrations/netatmo", "", "")
            .await
            .0,
        403
    );
    let cookie = server.login().await;
    let (status, _, body) = server
        .request("GET", "/api/integrations/netatmo", "", &cookie)
        .await;
    assert_eq!(status, 200);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json, json!({"instances":[]}));
}

#[tokio::test]
async fn callback_limit_is_applied_after_human_proof() {
    let server = Server::new().await;
    let body = json!({"state":generated(),"code":"x".repeat(9000)}).to_string();
    assert_eq!(
        server
            .request(
                "POST",
                "/api/integrations/netatmo/weather/callback",
                &body,
                ""
            )
            .await
            .0,
        403
    );
    let cookie = server.login().await;
    assert_eq!(
        server
            .request(
                "POST",
                "/api/integrations/netatmo/weather/callback",
                &body,
                &cookie
            )
            .await
            .0,
        413
    );
}

#[tokio::test]
async fn cross_site_callback_is_refused_even_with_cookie() {
    let server = Server::new().await;
    let cookie = server.login().await;
    let extra = format!("{cookie}Origin: https://example.org\r\nSec-Fetch-Site: cross-site\r\n");
    assert_eq!(
        server
            .request(
                "POST",
                "/api/integrations/netatmo/weather/callback",
                "{}",
                &extra
            )
            .await
            .0,
        403
    );
}

#[tokio::test]
async fn callback_script_is_static_without_query_echo() {
    let server = Server::new().await;
    let sensitive = generated();
    let path = format!("/api/integrations/netatmo/callback.js?code={sensitive}");
    let (status, headers, body) = server.request("GET", &path, "", "").await;
    assert_eq!(status, 200);
    assert!(!body.contains(&sensitive));
    assert!(
        headers
            .to_ascii_lowercase()
            .contains("content-type: application/javascript")
    );
    assert!(
        headers
            .to_ascii_lowercase()
            .contains("cache-control: no-store")
    );
}

#[tokio::test]
async fn logout_invalidates_the_http_callback() {
    let server = Server::new().await;
    let cookie = server.login().await;
    assert_eq!(
        server
            .request("DELETE", "/api/session", "", &cookie)
            .await
            .0,
        200
    );
    let body = json!({"state":generated(),"code":generated()}).to_string();
    assert_eq!(
        server
            .request(
                "POST",
                "/api/integrations/netatmo/weather/callback",
                &body,
                &cookie
            )
            .await
            .0,
        403
    );
}

#[tokio::test]
async fn static_return_is_public_but_the_post_stays_protected() {
    let server = Server::new().await;
    let outside = "CF-Connecting-IP: 203.0.113.20\r\n";
    assert_eq!(
        server
            .request(
                "GET",
                "/api/integrations/netatmo/weather/callback",
                "",
                outside
            )
            .await
            .0,
        200
    );
    assert_eq!(
        server
            .request("GET", "/api/integrations/netatmo/callback.js", "", outside)
            .await
            .0,
        200
    );
    assert_eq!(
        server
            .request(
                "POST",
                "/api/integrations/netatmo/weather/callback",
                "{}",
                outside
            )
            .await
            .0,
        403
    );
}

fn caller() -> Caller {
    Caller {
        via: Via::Lan,
        key: "192.168.1.20".into(),
        person: None,
    }
}

fn cookie(token: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::COOKIE,
        format!("moli_session={token}").parse().unwrap(),
    );
    headers
}

#[tokio::test]
async fn same_address_different_pin_sessions_have_distinct_bindings() {
    let pin = pin();
    let hub = Hub::new(HubOptions::default()).unwrap();
    let sessions = Sessions::new(hub, Some(UiPin::new(&pin)), &[]);
    let caller = caller();
    let first = cookie(&sessions.login(&pin, &caller).await.unwrap());
    let second = cookie(&sessions.login(&pin, &caller).await.unwrap());
    let a = sessions
        .binding(&first, &caller.key)
        .expect("first human proof");
    let b = sessions
        .binding(&second, &caller.key)
        .expect("second human proof");
    assert_ne!(a, b);
    assert_eq!(sessions.binding(&first, "192.168.1.21"), None);
    assert_eq!(sessions.binding(&HeaderMap::new(), &caller.key), None);
}

#[tokio::test]
async fn logout_revokes_the_exact_binding() {
    let pin = pin();
    let hub = Hub::new(HubOptions::default()).unwrap();
    let sessions = Sessions::new(hub, Some(UiPin::new(&pin)), &[]);
    let caller = caller();
    let headers = cookie(&sessions.login(&pin, &caller).await.unwrap());
    assert!(sessions.binding(&headers, &caller.key).is_some());
    sessions.logout(&headers).await;
    assert_eq!(sessions.binding(&headers, &caller.key), None);
}

struct Vault(PathBuf);

impl Vault {
    fn hub() -> (Self, Hub) {
        let root = std::env::temp_dir().join(format!("moli-netatmo-http-{}", generated()));
        std::fs::create_dir(&root).unwrap();
        let hub = Hub::new(HubOptions {
            secrets: Some((root.join("vault"), MasterKey::generate().unwrap())),
            ..HubOptions::default()
        })
        .unwrap();
        (Self(root), hub)
    }
}

impl Drop for Vault {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn pin_change_revokes_existing_bindings() {
    let (_vault, hub) = Vault::hub();
    let original = pin();
    let sessions = Sessions::new(hub, Some(UiPin::new(&original)), &[]);
    let caller = caller();
    let headers = cookie(&sessions.login(&original, &caller).await.unwrap());
    assert!(sessions.binding(&headers, &caller.key).is_some());
    let mut replacement = pin();
    while replacement == original {
        replacement = pin();
    }
    sessions.set_pin(&replacement, "test").await.unwrap();
    assert_eq!(sessions.binding(&headers, &caller.key), None);
}

#[tokio::test]
async fn expired_persisted_session_has_no_binding() {
    let (_vault, hub) = Vault::hub();
    let pin = pin();
    let sessions = Sessions::new(hub.clone(), Some(UiPin::new(&pin)), &[]);
    let caller = caller();
    let headers = cookie(&sessions.login(&pin, &caller).await.unwrap());
    assert!(sessions.binding(&headers, &caller.key).is_some());
    let mut stored: serde_json::Value =
        serde_json::from_str(&hub.core_secret("ui_sessions").unwrap()).unwrap();
    for value in stored["sessions"].as_object_mut().unwrap().values_mut() {
        value["created"] = json!(0);
    }
    hub.store_core_secret("ui_sessions", &stored.to_string())
        .await
        .unwrap();
    let restored = Sessions::new(hub, Some(UiPin::new(&pin)), &[]);
    assert_eq!(restored.binding(&headers, &caller.key), None);
}
