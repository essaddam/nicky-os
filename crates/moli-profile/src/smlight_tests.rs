//! Public supervision and additive profile policies, using synthetic contracts.
use super::*;

const POLICY: &str = r#"
[profile]
id = "test-public"
name = "Test"
identity = "info.Info.MAC"
mac = "info.Info.MAC"
fixed_target = true
min_request_interval_ms = 1000
[[request]]
id = "info"
path = "/ha_info"
format = "json"
every = "1h"
[[request]]
id = "sensors"
path = "/ha_sensors"
format = "json"
every = "60s"
[[point]]
key = "uptime"
label = "Uptime"
from = "sensors.Sensors.uptime"
kind = "numeric"
unit = "s"
semantic = "diagnostic"
missing = "null"
"#;
fn policy(text: &str) -> Compiled {
    Profile::parse(text)
        .expect("new additive options must parse")
        .compile(Source::Builtin)
        .unwrap()
}
#[test]
fn null_missing_is_opt_in_and_other_profiles_keep_their_previous_value() {
    let p = policy(POLICY);
    let empty = Answer::parse(Format::Json, br#"{"Sensors":{}}"#).unwrap();
    assert_eq!(p.points[0].read(&empty), Some(Value::Null));
    let keep = policy(&POLICY.replace("missing = \"null\"", "missing = \"keep\""));
    assert_eq!(keep.points[0].read(&empty), None);
    let old = policy(
        &POLICY
            .replace("missing = \"null\"", "")
            .replace("fixed_target = true", "")
            .replace("min_request_interval_ms = 1000", ""),
    );
    assert_eq!(old.points[0].read(&empty), None);
}
#[test]
fn pacing_and_fixed_identity_options_are_bounded_and_explicit() {
    let _ = policy(POLICY);
    for text in [
        POLICY.replace("1000", "5001"),
        POLICY.replace("1000", "-1"),
        POLICY.replace("identity = \"info.Info.MAC\"", ""),
        POLICY.replace("missing = \"null\"", "missing = \"zero\""),
    ] {
        assert!(
            Profile::parse(&text)
                .and_then(|p| p.compile(Source::Builtin))
                .is_err()
        );
    }
    let cfg = Config {
        profile: "unused".into(),
        host: Some("127.0.0.1".into()),
        port: Some(1234),
        vars: std::collections::BTreeMap::default(),
    };
    let p = policy(POLICY);
    let lan = Lan {
        home: |_| true,
        ..Lan::REAL
    };
    assert!(
        !followable(&cfg, &p, lan),
        "fixed target never invokes LAN following"
    );
    for host in [
        "http://router.invalid/path",
        "user@example.com",
        "router.invalid:80",
        "router.invalid?x=1",
    ] {
        assert!(
            Target::of(
                &Config {
                    host: Some(host.into()),
                    ..cfg.clone()
                },
                &p
            )
            .is_err()
        );
    }
}
#[test]
fn smlight_builtin_declares_only_seven_readonly_diagnostic_bindings() {
    let text = builtin("smlight").expect("public SMLIGHT profile is registered");
    let p = policy(text);
    assert_eq!(p.requests.len(), 2);
    assert_eq!(p.points.len(), 7);
    assert!(p.writes.is_empty());
    assert!(p.meta.secret.is_none());
    for point in p.points {
        assert!(!point.spec.access.write);
        assert_eq!(point.spec.semantic, moli_core::Semantic::Diagnostic);
        assert!(
            ![
                "presence",
                "associated",
                "ip",
                "mac",
                "ssid",
                "reboot",
                "mode"
            ]
            .contains(&point.spec.key.as_ref())
        );
    }
    assert_eq!(
        p.requests
            .iter()
            .map(|r| r.path.as_str())
            .collect::<Vec<_>>(),
        vec!["/ha_info", "/ha_sensors"]
    );
    assert_eq!(p.requests[0].every, Duration::from_secs(3600));
    assert_eq!(p.requests[1].every, Duration::from_secs(60));
}

#[test]
fn previous_at_prefixed_labels_stay_literal_without_translation_keys() {
    let old = POLICY
        .replace("name = \"Test\"", "name = \"@literal.name\"")
        .replace("label = \"Uptime\"", "label = \"@literal.point\"");
    let p = policy(&old);
    assert_eq!(p.meta.name, "@literal.name");
    assert_eq!(p.points[0].spec.label.as_ref(), "@literal.point");
}

#[test]
fn localized_names_and_labels_are_explicit_and_the_legacy_defaults_stay_disabled() {
    let old = policy(
        &POLICY
            .replace("fixed_target = true", "")
            .replace("min_request_interval_ms = 1000", ""),
    );
    assert!(!old.meta.fixed_target);
    assert_eq!(old.meta.min_request_interval_ms, 0);
    let cfg = Config {
        profile: "unused".into(),
        host: Some("127.0.0.1".into()),
        port: Some(80),
        vars: std::collections::BTreeMap::default(),
    };
    assert!(followable(
        &cfg,
        &old,
        Lan {
            home: |_| true,
            ..Lan::REAL
        }
    ));
    let translated = policy(
        &POLICY
            .replace(
                "name = \"Test\"",
                "name = \"Test\"\nname_key = \"pilotes.smlight.name\"",
            )
            .replace(
                "label = \"Uptime\"",
                "label = \"Uptime\"\nlabel_key = \"pilotes.smlight.uptime\"",
            ),
    );
    assert_eq!(
        translated.meta.display_name(),
        moli_i18n::tr!("pilotes.smlight.name")
    );
    assert_eq!(
        translated.points[0].localized_spec().label.as_ref(),
        moli_i18n::tr!("pilotes.smlight.uptime")
    );
    assert!(
        Profile::parse(
            &POLICY.replace("label = \"Uptime\"", "label = \"Uptime\"\nlabel_key = \"\"")
        )
        .and_then(|p| p.compile(Source::Builtin))
        .is_err()
    );
    let p = policy(builtin("smlight").unwrap());
    for point in p.points {
        if point.spec.key.as_ref() == "ram_usage" {
            assert!(point.spec.unit.is_none());
        }
        if point.spec.key.as_ref() == "uptime" {
            assert_eq!(point.spec.unit, Some(moli_core::Unit::Second));
        }
    }
}
#[test]
fn fixed_mac_identity_does_not_accept_ambiguous_or_placeholder_addresses() {
    let p = policy(POLICY);
    for raw in [
        "",
        "00:00:00:00:00:00",
        "FF:FF:FF:FF:FF:FF",
        "aa:bb-cc:dd:ee:10",
        "aa:bb:cc:dd:ee:10 ",
        "aabbccddee100",
        "aa:bb:cc:dd:ee:1z",
    ] {
        assert!(policies::canonical(&p, raw).is_err());
    }
    for raw in ["AA-BB-CC-DD-EE-10", "aa:bb:cc:dd:ee:10", "aabbccddee10"] {
        assert_eq!(policies::canonical(&p, raw).unwrap(), "aabbccddee10");
    }
}
