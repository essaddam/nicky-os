use super::*;
use moli_history::Family;
use ring::rand::{SecureRandom as _, SystemRandom};
use serde_json::json;

fn data(value: &serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(value).unwrap()
}

fn random_hex() -> String {
    let mut raw = [0u8; 20];
    SystemRandom::new().fill(&mut raw).unwrap();
    {
        use std::fmt::Write as _;
        raw.iter().fold(String::with_capacity(40), |mut text, b| {
            write!(text, "{b:02x}").unwrap();
            text
        })
    }
}

#[test]
fn twinkly_identity_is_stable_and_hostile_names_are_refused() {
    let mut raw = json!({"mac":"aa:bb:cc:dd:ee:ff","device_name":"String lights",
        "product_code":"RGB-Demo","led_profile":"RGB","code":1000});
    let got = info(&data(&raw)).unwrap();
    assert_eq!(got.native, "aabbccddeeff");
    assert_eq!(got.profile, "RGB");
    raw["device_name"] = json!("x".repeat(65));
    assert!(info(&data(&raw)).is_err());
    raw["device_name"] = json!("String lights");
    raw["mac"] = json!("not an identity");
    assert!(info(&data(&raw)).is_err());
}

#[test]
fn twinkly_color_needs_the_direct_rgb_contract() {
    assert!(static_color("2.7.1", "RGB"));
    assert!(static_color("3.0.7", "RGB"));
    assert!(!static_color("2.7.0", "RGB"));
    assert!(!static_color("3.0.7", "RGBW"));
    assert!(!static_color("unknown", "RGB"));
}

#[test]
fn twinkly_application_errors_are_errors_even_on_http_success() {
    assert_eq!(
        mode(br#"{"mode":"color","code":1000}"#).unwrap(),
        Mode::Color
    );
    assert!(mode(br#"{"mode":"color","code":1101}"#).is_err());
    assert!(mode(br#"{"mode":"color"}"#).is_err());
    assert!(mode(br#"{"mode":"unknown","code":1000}"#).is_err());
}

#[test]
fn twinkly_disabled_dimming_is_full_brightness() {
    assert_eq!(
        brightness(br#"{"value":17,"mode":"disabled","code":1000}"#).unwrap(),
        100
    );
    assert_eq!(
        brightness(br#"{"value":17,"mode":"enabled","code":1000}"#).unwrap(),
        17
    );
    assert!(brightness(br#"{"value":101,"mode":"enabled","code":1000}"#).is_err());
}

#[test]
fn twinkly_login_uses_the_bounded_returned_lifetime_without_echoing_secrets() {
    let token = random_hex()[..24].to_owned();
    let response = random_hex();
    let mut raw = json!({"authentication_token":token,"challenge-response":response,
        "authentication_token_expires_in":7,"code":1000});
    let got = login(&data(&raw)).unwrap();
    // Compare without an equality macro that would expose session values.
    assert!(got.token.eq(&token));
    assert!(got.response.eq(&response));
    assert_eq!(got.lifetime, Duration::from_secs(7));
    for invalid in [0, 86_401] {
        raw["authentication_token_expires_in"] = json!(invalid);
        let error = login(&data(&raw)).err().unwrap();
        assert!(!error.contains(&token) && !error.contains(&response));
    }
}

#[test]
fn twinkly_turn_on_does_not_choose_an_arbitrary_scene() {
    let caps = Capabilities {
        brightness: true,
        color: true,
    };
    assert!(plan("on", &Value::Bool(true), Mode::Off, None, caps).is_err());
    assert_eq!(
        plan("on", &Value::Bool(true), Mode::Off, Some(Mode::Movie), caps).unwrap(),
        vec![Step::Mode(Mode::Movie)]
    );
    assert!(
        plan("on", &Value::Bool(true), Mode::Color, None, caps)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn twinkly_zero_turns_off_and_a_color_uses_no_movie_upload() {
    let caps = Capabilities {
        brightness: true,
        color: true,
    };
    assert_eq!(
        plan("brightness", &Value::Int(0), Mode::Movie, None, caps).unwrap(),
        vec![Step::Mode(Mode::Off)]
    );
    assert_eq!(
        plan("color", &Value::from("#1234ab"), Mode::Movie, None, caps).unwrap(),
        vec![Step::Color([0x12, 0x34, 0xab]), Step::Mode(Mode::Color)]
    );
    assert!(
        plan(
            "color",
            &Value::from("#1234ab"),
            Mode::Movie,
            None,
            Capabilities::default()
        )
        .is_err()
    );
}

#[test]
fn twinkly_real_points_belong_to_the_states_family() {
    let points = points(Capabilities {
        brightness: true,
        color: true,
    });
    assert_eq!(points.len(), 3);
    for point in points {
        assert_eq!(Family::of(&point), Family::States);
    }
}

#[test]
fn twinkly_packaged_synthetic_fixtures_use_the_real_decoders() {
    let device = info(include_bytes!(
        "../../../integrations/twinkly/fixtures/gestalt.json"
    ))
    .unwrap();
    assert_eq!(device.native, "aabbccddeeff");
    let state: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../integrations/twinkly/fixtures/state.json"
    ))
    .unwrap();
    assert_eq!(mode(&data(&state["mode"])).unwrap(), Mode::Color);
    assert_eq!(brightness(&data(&state["brightness"])).unwrap(), 23);
    assert!(static_color(
        state["firmware"]["version"].as_str().unwrap(),
        &device.profile
    ));
}
