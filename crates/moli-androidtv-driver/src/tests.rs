use moli_androidtv::{cast, remote};
use moli_core::{InstanceId, Value};

use crate::commands::{Plan, plan};
use crate::model::{Observation, Volume, device};
use crate::{AndroidTv, Config};

#[test]
fn construction_rejects_urls_ports_and_unknown_options_without_network() {
    for host in [
        "",
        "https://tv.invalid",
        "tv.invalid:6466",
        " tv.invalid",
        "tv.invalid/path",
    ] {
        assert!(AndroidTv::new(Config { host: host.into() }).is_err());
    }
    for host in ["tv.invalid", "192.0.2.10", "2001:db8::1"] {
        assert!(AndroidTv::new(Config { host: host.into() }).is_ok());
    }
    assert!(serde_json::from_str::<Config>(r#"{"host":"tv.invalid","port":6466}"#).is_err());
}

#[test]
fn television_identity_and_capabilities_are_stable_and_have_no_ambilight() {
    let tv = device(&InstanceId::from("example"), None);
    assert_eq!(tv.id.as_str(), "example:tv");
    for key in [
        "power",
        "volume",
        "mute",
        "key",
        "app",
        "text",
        "media_control",
        "play_media",
    ] {
        assert!(tv.point(key).unwrap().access.write);
    }
    assert!(!tv.point("play_media").unwrap().access.read);
    assert!(tv.point("ambilight").is_none());
    assert!(tv.point("media_title").unwrap().access.read);
    assert!(!tv.point("media_title").unwrap().access.write);
}

#[test]
fn waking_and_sleeping_use_distinct_android_keys() {
    let observed = Observation::default();
    assert!(matches!(
        plan("power", &Value::Bool(true), &observed),
        Ok(Plan::Remote(remote::Order::Key(224)))
    ));
    assert!(matches!(
        plan("power", &Value::Bool(false), &observed),
        Ok(Plan::Remote(remote::Order::Key(223)))
    ));
}

#[test]
fn mute_never_toggles_an_unknown_or_already_matching_state() {
    let mut observed = Observation::default();
    assert!(plan("mute", &Value::Bool(true), &observed).is_err());
    observed.volume = Some(Volume {
        level: 10,
        max: 60,
        muted: true,
    });
    assert!(matches!(
        plan("mute", &Value::Bool(true), &observed),
        Ok(Plan::Noop)
    ));
    assert!(matches!(
        plan("mute", &Value::Bool(false), &observed),
        Ok(Plan::Remote(remote::Order::Key(164)))
    ));
}

#[test]
fn absolute_volume_requires_a_valid_observed_scale() {
    let mut observed = Observation::default();
    assert!(plan("volume", &Value::Int(50), &observed).is_err());
    observed.volume = Some(Volume {
        level: 42,
        max: 60,
        muted: false,
    });
    assert!(matches!(
        plan("volume", &Value::Int(50), &observed),
        Ok(Plan::Volume(50))
    ));
    assert!(plan("volume", &Value::Int(61), &observed).is_err());
    assert!(plan("volume", &Value::Float(f64::NAN), &observed).is_err());
}

#[test]
fn precise_content_uses_the_shared_resolver_and_never_falls_back_home() {
    let observed = Observation::default();
    let value = Value::from(r#"{"service":"netflix","kind":"episode","target":"12345678"}"#);
    let Plan::Content(target) = plan("play_media", &value, &observed).unwrap() else {
        panic!("content route missing");
    };
    assert_eq!(
        target.uri,
        "https://www.netflix.com/title/12345678?source=30"
    );
    for text in [
        r#"{"service":"molotov","kind":"channel","target":"1"}"#,
        r#"{"service":"freetv","kind":"replay","target":"https://oq.ee/replay/123"}"#,
    ] {
        assert!(plan("play_media", &Value::from(text), &observed).is_err());
    }
}

#[test]
fn remote_loss_clears_observations_instead_of_claiming_power_off() {
    let mut observed = Observation {
        online: true,
        power: Some(true),
        app: Some("com.example.player".into()),
        volume: Some(Volume {
            level: 10,
            max: 60,
            muted: false,
        }),
        ..Observation::default()
    };
    observed.remote(remote::Event::Lost);
    assert!(!observed.online);
    assert_eq!(observed.power, None);
    assert_eq!(observed.volume, None);
    assert_eq!(observed.app, None);
    assert_eq!(observed.epoch, 1);
}

#[test]
fn cast_loss_clears_metadata_but_not_remote_availability() {
    let mut observed = Observation {
        online: true,
        playing: cast::Playing {
            title: Some("Example".into()),
            ..cast::Playing::default()
        },
        ..Observation::default()
    };
    observed.cast(cast::Event::Lost);
    assert!(observed.online);
    assert_eq!(observed.playing, cast::Playing::default());
    assert_eq!(observed.cast_epoch, 1);
}

#[test]
fn service_app_requests_launch_native_provider_packages_without_a_browser_fallback() {
    for (service, package) in [
        ("netflix", "com.netflix.ninja"),
        ("crunchyroll", "com.crunchyroll.crunchyroid"),
        ("freetv", "net.oqee.androidtv"),
        ("primevideo", "com.amazon.amazonvideo.livingroom"),
        ("disneyplus", "com.disney.disneyplus"),
        ("molotov", "tv.molotov.app"),
        ("appletv", "com.apple.atve.androidtv.appletv"),
    ] {
        let request = serde_json::json!({"service":service,"kind":"app"}).to_string();
        let Plan::App { links } = plan(
            "play_media",
            &Value::from(request.as_str()),
            &Observation::default(),
        )
        .unwrap() else {
            panic!("{service}: opening an app must use native package launch semantics");
        };
        assert!(links.contains(&format!("market://launch?id={package}")));
        assert!(
            links
                .iter()
                .all(|link| link.starts_with("market://launch?id="))
        );
        if service == "appletv" {
            assert!(links.contains(&"market://launch?id=com.apple.atve.sony.appletv".into()));
        }
    }
}
