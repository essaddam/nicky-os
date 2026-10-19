use super::{Config, info};
use serde_json::{Value, json};

fn configured() -> Config {
    serde_json::from_value(json!({"host":"127.0.0.1", "port":10700})).unwrap()
}

fn program(name: &str) -> Value {
    json!({"name":name, "version":"1.2.3", "models":[]})
}

fn fixture(name: &str) -> Value {
    let frame = match name {
        "piper" => include_str!("../../../integrations/wyoming/fixtures/piper-info.wyoming"),
        "whisper" => include_str!("../../../integrations/wyoming/fixtures/whisper-info.wyoming"),
        "wake" => include_str!("../../../integrations/wyoming/fixtures/openwakeword-info.wyoming"),
        _ => panic!("unknown synthetic fixture"),
    };
    serde_json::from_str(frame.split_once('\n').unwrap().1).unwrap()
}

#[test]
fn endpoint_is_required_and_options_are_closed() {
    for config in [
        json!({}),
        json!({"host":"127.0.0.1"}),
        json!({"port":10700}),
        json!({"host":"127.0.0.1", "port":10700, "voice":"example"}),
    ] {
        assert!(serde_json::from_value::<Config>(config).is_err());
    }
    let config = configured();
    assert_eq!(config.every, 60);
    assert_eq!(config.timeout, 4);
}

#[test]
fn human_selected_local_addresses_and_dns_names_are_valid() {
    for host in [
        "127.0.0.1",
        "192.168.1.2",
        "::1",
        "fd00::1",
        "service.example.invalid",
    ] {
        let mut config = configured();
        config.host = host.into();
        assert!(config.validate().is_ok(), "rejected {host}");
    }
}

#[test]
fn malformed_or_nonlocal_literal_endpoints_are_refused_without_echo() {
    for host in [
        "",
        " service.local",
        "service.local\n",
        "tcp://service.local",
        "user:PRIVATE_TOKEN@example.com",
        "service.local/path",
        "service.local:10700",
        "bad..local",
        "-bad.local",
        "0.0.0.0",
        "224.0.0.1",
        "203.0.113.1",
        "::",
        "ff02::1",
    ] {
        let mut config = configured();
        config.host = host.into();
        let error = config.validate().expect_err("invalid endpoint accepted");
        let text = format!("{error:#}");
        assert!(!text.contains("PRIVATE_TOKEN"));
        assert!(!text.contains("service.local"));
    }
}

#[test]
fn port_poll_and_timeout_have_explicit_bounds() {
    let mut invalid = Vec::new();
    let mut config = configured();
    config.port = 0;
    invalid.push(config);
    for every in [0, 9, 3601, u64::MAX] {
        let mut config = configured();
        config.every = every;
        invalid.push(config);
    }
    for timeout in [0, 11, u64::MAX] {
        let mut config = configured();
        config.timeout = timeout;
        invalid.push(config);
    }
    for config in invalid {
        assert!(config.validate().is_err());
    }
    for (every, timeout) in [(10, 1), (3600, 10)] {
        let mut config = configured();
        config.every = every;
        config.timeout = timeout;
        assert!(config.validate().is_ok());
    }
}

#[test]
fn each_synthetic_service_exposes_only_its_announced_metadata() {
    for (name, expected_program, models, voices) in [
        ("piper", "piper", "", "example-fr-voice"),
        ("whisper", "faster-whisper", "example-small", ""),
        ("wake", "openWakeWord", "example-wake", ""),
    ] {
        let info = info::parse(&fixture(name)).unwrap();
        assert_eq!(info.program, expected_program);
        assert_eq!(info.version, format!("{expected_program}: 0.0.0"));
        assert_eq!(info.models, models);
        assert_eq!(info.voices, voices);
    }
}

#[test]
fn lists_are_sorted_deduplicated_and_versions_keep_their_program() {
    let input = json!({
        "asr":[{"name":"zeta", "version":"2", "models":[{"name":"small"},{"name":"base"},{"name":"small"}]}],
        "tts":[{"name":"alpha", "version":"1", "voices":[{"name":"voice-b"},{"name":"voice-a"}]}],
        "wake":[{"name":"zeta", "version":"2", "models":[{"name":"base"}]}],
        "satellite":{"name":"PRIVATE_TOKEN"},
        "unrelated":"https://example.org/private"
    });
    let info = info::parse(&input).unwrap();
    assert_eq!(info.program, "alpha, zeta");
    assert_eq!(info.version, "alpha: 1, zeta: 2");
    assert_eq!(info.models, "base, small");
    assert_eq!(info.voices, "voice-a, voice-b");
}

#[test]
fn displayed_names_strip_controls_and_bound_unicode_text() {
    let long = "é".repeat(200);
    let info = info::parse(&json!({"asr":[{
        "name":"test\n\u{202e}\u{2066}",
        "version":null,
        "models":[{"name":long}]
    }]}))
    .unwrap();
    assert_eq!(info.program, "test");
    assert!(info.version.is_empty());
    assert_eq!(info.models, "é".repeat(128));
}

#[test]
fn published_lists_remain_bounded_even_with_maximum_valid_arrays() {
    let programs: Vec<Value> = (0..16)
        .map(|p| {
            let models: Vec<Value> = (0..64)
                .map(|m| json!({"name":format!("{p:02}-{m:02}-{}", "é".repeat(120))}))
                .collect();
            json!({"name":format!("program-{p:02}"), "version":"1", "models":models})
        })
        .collect();
    let info = info::parse(&json!({"asr":programs})).unwrap();
    assert!(!info.models.is_empty());
    for text in [&info.models, &info.program, &info.version, &info.voices] {
        assert!(text.chars().count() <= 2048);
        assert!(!text.chars().any(char::is_control));
    }
}

#[test]
fn malformed_known_fields_and_empty_descriptions_fail_closed() {
    for input in [
        json!(null),
        json!([]),
        json!({}),
        json!({"asr":[], "tts":[], "wake":[]}),
        json!({"asr":null}),
        json!({"asr":{}}),
        json!({"asr":[null]}),
        json!({"asr":[{"name":"test"}]}),
        json!({"asr":[{"name":42, "models":[]}]}),
        json!({"asr":[{"name":"\n", "models":[]}]}),
        json!({"asr":[{"name":"test", "models":null}]}),
        json!({"asr":[{"name":"test", "models":[{}]}]}),
        json!({"asr":[{"name":"test", "version":42, "models":[]}]}),
        json!({"asr":[program("test")], "tts":null}),
    ] {
        assert!(info::parse(&input).is_err(), "accepted invalid description");
    }
}

#[test]
fn program_and_model_array_limits_are_enforced() {
    let too_many_programs = json!({"asr":vec![program("test");17]});
    let too_many_models =
        json!({"wake":[{"name":"test", "models":vec![json!({"name":"wake"});65]}]});
    assert!(info::parse(&too_many_programs).is_err());
    assert!(info::parse(&too_many_models).is_err());
}

#[test]
fn absent_or_null_versions_are_allowed() {
    for version in [None, Some(json!(null))] {
        let mut entry = program("test");
        match version {
            Some(version) => entry["version"] = version,
            None => {
                entry.as_object_mut().unwrap().remove("version");
            }
        }
        let info = info::parse(&json!({"asr":[entry]})).unwrap();
        assert_eq!(info.program, "test");
        assert!(info.version.is_empty());
    }
}
