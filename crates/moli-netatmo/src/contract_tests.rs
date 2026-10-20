//! Contracts through the provider exchange seam; never a real account.

use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use http::{HeaderMap, StatusCode, header};
use http_body_util::BodyExt as _;
use moli_core::InstanceId;
use moli_net::{Body, OnceError};
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::json;

use crate::{Error, api::Api, config::Config, oauth::TokenSet};

fn generated() -> String {
    let mut bytes = [0_u8; 32];
    SystemRandom::new().fill(&mut bytes).unwrap();
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(encoded, "{byte:02x}").unwrap();
    }
    encoded
}

fn config(origin: &str) -> Config {
    serde_json::from_value(json!({"public_origin":origin})).unwrap()
}

fn client() -> Api {
    Api::new(
        generated(),
        generated(),
        "https://example.com/api/integrations/netatmo/weather/callback".into(),
    )
}

fn token_body(access: &str, refresh: &str) -> Body {
    Body::from(
        json!({"access_token":access,"refresh_token":refresh,"expires_in":3600,"scope":["read_station"]})
            .to_string(),
    )
}

fn responder(
    status: StatusCode,
    headers: HeaderMap,
    body: Body,
) -> (crate::api::Exchange, Arc<AtomicUsize>) {
    let count = Arc::new(AtomicUsize::new(0));
    let calls = count.clone();
    let exchange: crate::api::Exchange = Arc::new(move |_, _, _| {
        calls.fetch_add(1, Ordering::SeqCst);
        let answer = (status, headers.clone(), body.clone());
        Box::pin(async move { Ok(answer) })
    });
    (exchange, count)
}

#[test]
fn configuration_has_bounded_defaults_and_fixed_callback() {
    let cfg = config("https://example.com");
    assert_eq!((cfg.poll_seconds, cfg.stale_seconds), (300, 1800));
    assert_eq!(
        cfg.callback(&InstanceId::from("weather")).unwrap(),
        "https://example.com/api/integrations/netatmo/weather/callback"
    );
    for origin in [
        "http://localhost:8790",
        "http://127.0.0.1:8790",
        "http://[::1]:8790",
    ] {
        assert!(
            config(origin)
                .callback(&InstanceId::from("weather"))
                .is_ok()
        );
    }
}

#[test]
fn callback_uses_browser_canonical_origins() {
    for (configured, canonical) in [
        ("https://EXAMPLE.COM:443", "https://example.com"),
        ("https://Example.COM:8443", "https://example.com:8443"),
        ("http://LOCALHOST:80", "http://localhost"),
        ("http://127.0.0.1:80", "http://127.0.0.1"),
        ("http://[0:0:0:0:0:0:0:1]:80", "http://[::1]"),
        (
            "https://[::ffff:192.0.2.1]:443",
            "https://[::ffff:c000:201]",
        ),
        ("https://[::FFFF:C000:0201]", "https://[::ffff:c000:201]"),
    ] {
        assert_eq!(
            config(configured)
                .callback(&InstanceId::from("weather"))
                .unwrap(),
            format!("{canonical}/api/integrations/netatmo/weather/callback")
        );
    }
    for ambiguous in ["https://127.1", "https://0177.0.0.1", "https://0x7f000001"] {
        assert_eq!(
            config(ambiguous).callback(&InstanceId::from("weather")),
            Err(Error::Invalid)
        );
    }
}

#[test]
fn configuration_rejects_hostile_origins_and_poll_bounds() {
    for origin in [
        "",
        "http://example.com",
        "https://user@example.com",
        "https://example.com/",
        "https://example.com/path",
        "https://example.com?next=other",
        "https://example.com#x",
        "ftp://example.com",
        "//example.com",
        "https://example.com:0",
        "https://example.com:70000",
        "http://127.0.0.1.example.com",
        "http://127.1",
        "https://example.com\r\nX: x",
    ] {
        assert_eq!(
            config(origin).callback(&InstanceId::from("weather")),
            Err(Error::Invalid)
        );
    }
    for (poll, stale) in [
        (59, 1800),
        (3601, 1800),
        (300, 299),
        (300, 599),
        (300, 86401),
        (u64::MAX, u64::MAX),
    ] {
        let mut cfg = config("https://example.com");
        cfg.poll_seconds = poll;
        cfg.stale_seconds = stale;
        assert_eq!(
            cfg.callback(&InstanceId::from("weather")),
            Err(Error::Invalid)
        );
    }
    assert!(
        serde_json::from_value::<Config>(
            json!({"public_origin":"https://example.com","provider_url":"https://example.org"})
        )
        .is_err()
    );
    assert_eq!(
        config("https://example.com").callback(&InstanceId::from("../other")),
        Err(Error::Invalid)
    );
}

#[test]
fn authorize_is_fixed_encoded_and_has_no_transport() {
    let mut api = client();
    let (exchange, count) = responder(StatusCode::OK, HeaderMap::new(), Body::new());
    api.exchange = Some(exchange);
    let state = generated();
    let url = api.authorization_url(&state).unwrap();
    assert!(url.starts_with("https://api.netatmo.com/oauth2/authorize?"));
    let query = url.split_once('?').unwrap().1;
    let fields: Vec<_> = query.split('&').collect();
    assert_eq!(fields.len(), 5);
    assert!(fields.contains(&"response_type=code"));
    assert!(fields.contains(&"scope=read_station"));
    assert!(fields.contains(&format!("state={state}").as_str()));
    assert!(fields.contains(&"redirect_uri=https%3A%2F%2Fexample.com%2Fapi%2Fintegrations%2Fnetatmo%2Fweather%2Fcallback"));
    assert_eq!(count.load(Ordering::SeqCst), 0);
    assert_eq!(api.authorization_url("x&scope=other"), Err(Error::Invalid));
}

#[tokio::test]
async fn code_exchange_uses_form_and_complete_token_pair() {
    let id = generated();
    let secret = format!("{}+&=%", generated());
    let code = format!("{}+&=%", generated());
    let access = generated();
    let refresh = generated();
    let mut api = Api::new(
        id.clone(),
        secret.clone(),
        "https://example.com/callback".into(),
    );
    let response = token_body(&access, &refresh);
    let expected_id = id.clone();
    let expected_secret = format!("{}%2B%26%3D%25", secret.trim_end_matches("+&=%"));
    let expected_code = format!("{}%2B%26%3D%25", code.trim_end_matches("+&=%"));
    api.exchange = Some(Arc::new(move |request, limit, _| {
        let id = expected_id.clone();
        let secret = expected_secret.clone();
        let code = expected_code.clone();
        let body = response.clone();
        Box::pin(async move {
            assert_eq!(limit, 16 * 1024);
            assert_eq!(request.method(), "POST");
            assert_eq!(request.uri().to_string(), "/oauth2/token");
            assert_eq!(request.headers()[header::HOST], "api.netatmo.com");
            assert_eq!(
                request.headers()[header::CONTENT_TYPE],
                "application/x-www-form-urlencoded"
            );
            let bytes = request.into_body().collect().await.unwrap().to_bytes();
            let text = std::str::from_utf8(&bytes).unwrap();
            let fields: Vec<_> = text.split('&').collect();
            assert_eq!(fields.len(), 5);
            for field in [
                "grant_type=authorization_code".to_owned(),
                format!("client_id={id}"),
                format!("client_secret={secret}"),
                format!("code={code}"),
                "redirect_uri=https%3A%2F%2Fexample.com%2Fcallback".to_owned(),
            ] {
                assert!(fields.contains(&field.as_str()));
            }
            Ok((StatusCode::OK, HeaderMap::new(), body))
        })
    }));
    let tokens = api
        .exchange_code(&code, 100, Arc::new(|| true))
        .await
        .unwrap();
    assert_eq!(tokens.access, access);
    assert_eq!(tokens.refresh, refresh);
    assert_eq!(tokens.expires_at, 3700);
    let debug = format!("{api:?} {tokens:?}");
    for private in [&id, &secret, &code, &access, &refresh] {
        assert!(!debug.contains(private));
    }
}

#[tokio::test]
async fn refresh_is_one_request_and_returns_rotated_pair() {
    let old = TokenSet {
        access: generated(),
        refresh: generated(),
        expires_at: 100,
    };
    let access = generated();
    let refresh = generated();
    let response = token_body(&access, &refresh);
    let expected_refresh = old.refresh.clone();
    let count = Arc::new(AtomicUsize::new(0));
    let calls = count.clone();
    let mut api = client();
    api.exchange = Some(Arc::new(move |request, limit, _| {
        calls.fetch_add(1, Ordering::SeqCst);
        let expected = expected_refresh.clone();
        let response = response.clone();
        Box::pin(async move {
            assert_eq!(limit, 16 * 1024);
            assert_eq!(request.uri().to_string(), "/oauth2/token");
            let bytes = request.into_body().collect().await.unwrap().to_bytes();
            let form = std::str::from_utf8(&bytes).unwrap();
            assert!(form.split('&').any(|s| s == "grant_type=refresh_token"));
            assert!(
                form.split('&')
                    .any(|s| s == format!("refresh_token={expected}"))
            );
            assert_eq!(form.split('&').count(), 4);
            Ok((StatusCode::OK, HeaderMap::new(), response))
        })
    }));
    let tokens = api.refresh(&old, 200).await.unwrap();
    assert_eq!(
        (tokens.access, tokens.refresh, tokens.expires_at),
        (access, refresh, 3800)
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn failed_exchanges_preserve_stage_and_never_retry() {
    for (stage, expected) in [
        (OnceError::BeforeSend, Error::BeforeSend),
        (OnceError::AfterSend, Error::Ambiguous),
    ] {
        let count = Arc::new(AtomicUsize::new(0));
        let calls = count.clone();
        let mut api = client();
        api.exchange = Some(Arc::new(move |_, _, _| {
            calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move { Err(stage) })
        }));
        assert_eq!(
            api.exchange_code(&generated(), 100, Arc::new(|| true))
                .await
                .unwrap_err(),
            expected
        );
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn weather_uses_fixed_path_and_bearer_without_query_credentials() {
    let access = generated();
    let expected = access.clone();
    let mut api = client();
    api.exchange = Some(Arc::new(move |request, limit, _| {
        assert_eq!(limit, 512 * 1024);
        assert_eq!(request.method(), "GET");
        assert_eq!(
            request.uri().to_string(),
            "/api/getstationsdata?get_favorites=false"
        );
        assert_eq!(request.headers()[header::HOST], "api.netatmo.com");
        assert_eq!(
            request.headers()[header::AUTHORIZATION],
            format!("Bearer {expected}")
        );
        Box::pin(async { Ok((StatusCode::OK, HeaderMap::new(), Body::from("{}"))) })
    }));
    assert_eq!(api.weather(&access).await.unwrap().as_ref(), b"{}");
}

#[tokio::test]
async fn hostile_responses_are_bounded_refused_and_not_retried() {
    let mut compressed = HeaderMap::new();
    compressed.insert(header::CONTENT_ENCODING, "gzip".parse().unwrap());
    let mut redirected = HeaderMap::new();
    redirected.insert(header::LOCATION, "https://example.org/".parse().unwrap());
    for (status, headers, body) in [
        (StatusCode::FOUND, redirected, Body::new()),
        (StatusCode::OK, compressed, Body::from("{}")),
        (
            StatusCode::OK,
            HeaderMap::new(),
            Body::from(vec![b'x'; 512 * 1024 + 1]),
        ),
    ] {
        let mut api = client();
        let (exchange, count) = responder(status, headers, body);
        api.exchange = Some(exchange);
        assert!(api.weather(&generated()).await.is_err());
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
    let mut api = client();
    let (exchange, count) = responder(StatusCode::OK, HeaderMap::new(), Body::from("not JSON"));
    api.exchange = Some(exchange);
    assert_eq!(
        api.exchange_code(&generated(), 100, Arc::new(|| true))
            .await
            .unwrap_err(),
        Error::Ambiguous
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn oauth_refusal_and_numeric_retry_after_are_safe() {
    let sensitive = generated();
    let mut api = client();
    let (exchange, count) = responder(
        StatusCode::BAD_REQUEST,
        HeaderMap::new(),
        Body::from(json!({"error":"invalid_grant","error_description":sensitive}).to_string()),
    );
    api.exchange = Some(exchange);
    let error = api
        .exchange_code(&generated(), 100, Arc::new(|| true))
        .await
        .unwrap_err();
    assert_eq!(error, Error::Unauthorized);
    assert!(!format!("{error:?}").contains(&sensitive));
    assert_eq!(count.load(Ordering::SeqCst), 1);
    for (value, expected) in [
        ("1", 60),
        ("999999", 900),
        ("invalid", 60),
        ("18446744073709551616", 60),
    ] {
        let mut headers = HeaderMap::new();
        headers.insert(header::RETRY_AFTER, value.parse().unwrap());
        let (exchange, count) = responder(StatusCode::TOO_MANY_REQUESTS, headers, Body::new());
        api.exchange = Some(exchange);
        assert_eq!(
            api.weather(&generated()).await.unwrap_err(),
            Error::RateLimited(expected)
        );
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn invalid_credentials_and_codes_never_reach_transport() {
    let mut api = client();
    let (exchange, count) = responder(StatusCode::OK, HeaderMap::new(), Body::new());
    api.exchange = Some(exchange.clone());
    for code in [String::new(), "x".repeat(2049), "x\r\ny".into()] {
        assert_eq!(
            api.exchange_code(&code, 100, Arc::new(|| true))
                .await
                .unwrap_err(),
            Error::Invalid
        );
    }
    for access in [String::new(), "x".repeat(4097), "x\r\ny".into()] {
        assert_eq!(api.weather(&access).await.unwrap_err(), Error::Invalid);
    }
    for (id, secret) in [
        (String::new(), generated()),
        (generated(), String::new()),
        ("x".repeat(257), generated()),
        (generated(), "x".repeat(4097)),
    ] {
        let mut api = Api::new(id, secret, "https://example.com/callback".into());
        api.exchange = Some(exchange.clone());
        assert_eq!(
            api.exchange_code(&generated(), 100, Arc::new(|| true))
                .await
                .unwrap_err(),
            Error::Invalid
        );
    }
    assert_eq!(count.load(Ordering::SeqCst), 0);
}
