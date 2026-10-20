use super::{ContentError, ContentKind, ContentRequest, Qualification, Service, resolve};

fn request(service: Service, kind: ContentKind, target: &str) -> ContentRequest {
    ContentRequest {
        service,
        kind,
        target: target.to_owned(),
    }
}

#[test]
fn netflix_preserves_the_selected_title_instead_of_resuming_the_last_one() {
    for target in [
        "12345678",
        "https://www.netflix.com/title/12345678?trackingId=999",
        "https://www.netflix.com/watch/12345678?source=other",
    ] {
        let launch = resolve(&request(Service::Netflix, ContentKind::Title, target)).unwrap();
        assert_eq!(
            launch.uri,
            "https://www.netflix.com/title/12345678?source=30"
        );
        assert_eq!(launch.qualification, Qualification::Documented);
    }
}

#[test]
fn crunchyroll_distinguishes_series_from_an_episode_and_accepts_locale_links() {
    for (kind, target, expected) in [
        (
            ContentKind::Title,
            "GY000TEST1",
            "https://www.crunchyroll.com/series/GY000TEST1",
        ),
        (
            ContentKind::Episode,
            "GE000TEST1",
            "https://www.crunchyroll.com/watch/GE000TEST1",
        ),
        (
            ContentKind::Episode,
            "https://www.crunchyroll.com/fr/watch/GE000TEST1/example?utm_source=example",
            "https://www.crunchyroll.com/watch/GE000TEST1",
        ),
    ] {
        let launch = resolve(&request(Service::Crunchyroll, kind, target)).unwrap();
        assert_eq!(launch.uri, expected);
        assert_eq!(launch.qualification, Qualification::Unqualified);
    }
    assert_eq!(
        resolve(&request(
            Service::Crunchyroll,
            ContentKind::Episode,
            "https://www.crunchyroll.com/series/GY000TEST1/example"
        )),
        Err(ContentError::InvalidTarget)
    );
}

#[test]
fn freetv_channel_uses_a_provider_id_and_never_a_guessed_channel_name() {
    for target in [
        "123",
        "https://oq.ee/channel/123/play",
        "https://tv.free.fr/home/channels/123/play",
    ] {
        assert_eq!(
            resolve(&request(Service::Freetv, ContentKind::Channel, target))
                .unwrap()
                .uri,
            "https://oq.ee/channel/123/play"
        );
    }
    assert_eq!(
        resolve(&request(
            Service::Freetv,
            ContentKind::Channel,
            "Example channel"
        )),
        Err(ContentError::InvalidTarget)
    );
}

#[test]
fn freetv_replay_portal_is_not_a_programme_and_no_android_route_is_invented() {
    for target in [
        "123",
        "https://oq.ee/replay/123",
        "https://oq.ee/home/replay",
    ] {
        assert_eq!(
            resolve(&request(Service::Freetv, ContentKind::Replay, target)),
            Err(ContentError::UnsupportedTarget)
        );
    }
    let target = resolve(&request(
        Service::Freetv,
        ContentKind::Replay,
        "https://tv.free.fr/replay/123/detail",
    ))
    .unwrap();
    assert_eq!(target.uri, "https://tv.free.fr/replay/123/detail");
    assert_eq!(target.qualification, Qualification::Unqualified);
}

#[test]
fn primevideo_accepts_explicit_content_ids_and_removes_tracking() {
    for target in [
        "B000TEST00",
        "https://www.primevideo.com/detail/B000TEST00?ref_=example",
        "https://app.primevideo.com/detail?gti=B000TEST00",
    ] {
        assert_eq!(
            resolve(&request(Service::PrimeVideo, ContentKind::Episode, target))
                .unwrap()
                .uri,
            "https://app.primevideo.com/detail?gti=B000TEST00"
        );
    }
}

#[test]
fn disneyplus_never_turns_a_series_entity_into_an_episode_id() {
    let entity = "00000000-0000-4000-8000-000000000001";
    assert_eq!(
        resolve(&request(Service::DisneyPlus, ContentKind::Title, entity))
            .unwrap()
            .uri,
        "https://www.disneyplus.com/browse/entity-00000000-0000-4000-8000-000000000001"
    );
    assert_eq!(resolve(&request(Service::DisneyPlus, ContentKind::Episode, "https://www.disneyplus.com/fr-fr/play/00000000-0000-4000-8000-000000000002?utm_source=example")).unwrap().uri, "https://www.disneyplus.com/play/00000000-0000-4000-8000-000000000002");
    assert_eq!(
        resolve(&request(Service::DisneyPlus, ContentKind::Episode, entity)),
        Err(ContentError::InvalidTarget)
    );
    assert_eq!(
        resolve(&request(
            Service::DisneyPlus,
            ContentKind::Episode,
            "https://www.disneyplus.com/browse/entity-00000000-0000-4000-8000-000000000001"
        )),
        Err(ContentError::InvalidTarget)
    );
}

#[test]
fn appletv_keeps_the_supplied_movie_show_or_episode_target() {
    let link = "https://tv.apple.com/fr/episode/example/umc.cmc.example000000000000000001";
    let target = resolve(&request(
        Service::AppleTv,
        ContentKind::Episode,
        &format!("{link}?itsct=example"),
    ))
    .unwrap();
    assert_eq!(target.uri, link);
    assert_eq!(target.qualification, Qualification::Unqualified);
    assert_eq!(
        resolve(&request(
            Service::AppleTv,
            ContentKind::Episode,
            "https://tv.apple.com/fr/show/example/umc.cmc.example000000000000000001"
        )),
        Err(ContentError::InvalidTarget)
    );
}

#[test]
fn every_requested_service_has_an_explicit_app_launch() {
    for (service, expected) in [
        (Service::Netflix, "https://www.netflix.com/title"),
        (Service::Crunchyroll, "crunchyroll://"),
        (Service::Freetv, "https://oq.ee/home/"),
        (Service::PrimeVideo, "https://app.primevideo.com"),
        (Service::DisneyPlus, "https://www.disneyplus.com"),
        (Service::Molotov, "market://launch?id=tv.molotov.app"),
        (Service::AppleTv, "https://tv.apple.com"),
    ] {
        assert_eq!(
            resolve(&request(service, ContentKind::App, ""))
                .unwrap()
                .uri,
            expected
        );
    }
}

#[test]
fn unsupported_content_never_falls_back_to_opening_an_app() {
    for service in [
        Service::Netflix,
        Service::Crunchyroll,
        Service::PrimeVideo,
        Service::DisneyPlus,
        Service::AppleTv,
    ] {
        assert_eq!(
            resolve(&request(service, ContentKind::Channel, "123")),
            Err(ContentError::UnsupportedTarget)
        );
    }
    for kind in [
        ContentKind::Title,
        ContentKind::Episode,
        ContentKind::Channel,
        ContentKind::Replay,
    ] {
        assert_eq!(
            resolve(&request(Service::Molotov, kind, "example")),
            Err(ContentError::UnsupportedTarget)
        );
    }
}

#[test]
fn links_cannot_hide_another_host_or_launch_an_arbitrary_intent() {
    for target in [
        "https://www.netflix.com.example.com/title/12345678",
        "https://www.netflix.com@example.com/title/12345678",
        // Synthetic userinfo, kept separate from a literal e-mail address.
        concat!("https://user:password@", "www.netflix.com/title/12345678"),
        "https://www.netflix.com:8443/title/12345678",
        "http://www.netflix.com/title/12345678",
        "intent://www.netflix.com/title/12345678#Intent;scheme=https;end",
        "javascript:alert(1)",
        "file:///title/12345678",
        "https://www.netflix.com/title/12%2F345678",
        "https://www.netflix.com/title/12345678\n",
    ] {
        assert_eq!(
            resolve(&request(Service::Netflix, ContentKind::Title, target)),
            Err(ContentError::InvalidTarget),
            "target: {target:?}"
        );
    }
}

#[test]
fn malformed_ids_and_extra_path_components_are_not_interpreted_as_content() {
    for target in [
        "",
        "not-a-number",
        "https://www.netflix.com/",
        "https://www.netflix.com/title/12345678/extra",
        "https://www.netflix.com/title/12345678/../87654321",
        "https://www.netflix.com/title/12345678#token",
    ] {
        assert_eq!(
            resolve(&request(Service::Netflix, ContentKind::Title, target)),
            Err(ContentError::InvalidTarget)
        );
    }
}

#[test]
fn target_bounds_are_enforced_before_parsing_or_allocation() {
    assert_eq!(
        resolve(&request(
            Service::Netflix,
            ContentKind::Title,
            &"1".repeat(2049)
        )),
        Err(ContentError::TooLong)
    );
    assert_eq!(
        ContentRequest::parse(&" ".repeat(4097)),
        Err(ContentError::TooLong)
    );
}

#[test]
fn request_schema_refuses_unknown_fields_services_and_kinds() {
    for input in [
        r#"{"service":"netflix","kind":"title","target":"12345678","unknown":true}"#,
        r#"{"service":"unknown","kind":"title","target":"12345678"}"#,
        r#"{"service":"netflix","kind":"unknown","target":"12345678"}"#,
        r#"{"service":"netflix","service":"freetv","kind":"title","target":"12345678"}"#,
        r#"{"service":"netflix","kind":"title","target":12345678}"#,
    ] {
        assert_eq!(
            ContentRequest::parse(input),
            Err(ContentError::InvalidRequest)
        );
    }
    assert_eq!(
        ContentRequest::parse(r#"{"service":"netflix","kind":"title","target":"12345678"}"#)
            .unwrap(),
        request(Service::Netflix, ContentKind::Title, "12345678")
    );
}
