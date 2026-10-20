use moli_media_catalog::{
    CatalogError, ContentKey, ContentKind, Entitlement, Intent, Offer, OfferMode, Provider,
    ReaderCapabilities, SearchPage, SearchQuery, SelectionCheck, SelectionError, SourceOutcome,
    check_selection,
};

const NOW: u64 = 1_000;
const FIXTURE: &str = include_str!("fixtures/scenarios.json");

fn page() -> SearchPage {
    serde_json::from_str(FIXTURE).expect("the synthetic fixture must decode")
}

fn query() -> SearchQuery {
    SearchQuery {
        query: "Horizon de papier".into(),
        providers: vec![],
        kinds: vec![],
        country: "FR".into(),
        metadata_language: "fr-FR".into(),
        page_size: 20,
        cursor: None,
    }
}

fn reader(provider: Provider) -> ReaderCapabilities {
    ReaderCapabilities {
        providers: vec![provider],
        kinds: vec![
            ContentKind::Movie,
            ContentKind::Series,
            ContentKind::Episode,
        ],
        intents: vec![Intent::OpenDetails],
    }
}

fn offer(provider: Provider) -> Offer {
    page()
        .offers
        .into_iter()
        .find(|offer| offer.provider == provider)
        .expect("the fixture has an offer for each provider")
}

fn request(query: &str, page_size: usize) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "query": query,
        "country": "FR",
        "metadata_language": "fr-FR",
        "page_size": page_size
    }))
    .expect("the synthetic request must serialize")
}

#[test]
fn homonyms_keep_source_identity() {
    let page = page();
    assert_eq!(page.validate(&query(), NOW), Ok(()));
    let homonyms: Vec<_> = page
        .items
        .iter()
        .filter(|item| item.title == "Horizon de papier")
        .collect();
    assert_eq!(homonyms.len(), 3);
    assert_ne!(homonyms[0].key, homonyms[1].key);
    assert_ne!(homonyms[1].key, homonyms[2].key);
    assert_eq!(homonyms[0].year, Some(2001));
    assert_eq!(homonyms[1].year, Some(2021));
    assert_eq!(homonyms[2].kind, ContentKind::Series);

    let mut duplicate = page.clone();
    duplicate.items.push(duplicate.items[0].clone());
    assert_eq!(
        duplicate.validate(&query(), NOW),
        Err(CatalogError::DuplicateIdentity)
    );
}

#[test]
fn source_id_never_becomes_provider_id() {
    let page = page();
    let mut content = page.items[0].clone();
    content.key = ContentKey {
        source: "tmdb".into(),
        id: "fixture-metadata-only".into(),
    };
    content.provider_refs.clear();
    let mut selected = offer(Provider::Netflix);
    selected.content = content.key.clone();
    assert_eq!(
        check_selection(
            &content,
            &selected,
            Intent::OpenDetails,
            &reader(Provider::Netflix),
            NOW,
        ),
        Err(SelectionError::MissingProviderReference)
    );
    assert!(content.provider_refs.is_empty());
}

#[test]
fn series_season_episode_remain_distinct() {
    let page = page();
    assert_eq!(page.validate(&query(), NOW), Ok(()));
    let find = |kind| {
        page.items
            .iter()
            .find(|item| item.kind == kind)
            .expect("the fixture contains the requested kind")
    };
    let series = find(ContentKind::Series);
    let season = find(ContentKind::Season);
    let episode = find(ContentKind::Episode);
    assert_ne!(series.key, season.key);
    assert_eq!(season.series.as_ref(), Some(&series.key));
    assert_eq!(episode.series.as_ref(), Some(&series.key));
    assert_eq!(episode.season.as_ref(), Some(&season.key));
    assert_eq!(episode.season_number, Some(2));
    assert_eq!(episode.episode_number, Some(4));
    let special = page
        .items
        .iter()
        .find(|item| item.key.id == "special")
        .unwrap();
    assert_eq!(special.episode_number, Some(0));
    assert_ne!(special.key, episode.key);

    let mut offer = offer(Provider::Crunchyroll);
    offer.content = season.key.clone();
    assert_eq!(
        check_selection(
            season,
            &offer,
            Intent::OpenDetails,
            &reader(Provider::Crunchyroll),
            NOW,
        ),
        Err(SelectionError::UnsupportedKind)
    );

    let mut broken = page.clone();
    let item = broken
        .items
        .iter_mut()
        .find(|item| item.kind == ContentKind::Episode)
        .unwrap();
    item.season = Some(ContentKey {
        source: "fixture".into(),
        id: "missing-season".into(),
    });
    assert_eq!(
        broken.validate(&query(), NOW),
        Err(CatalogError::InvalidParent)
    );
}

#[test]
fn channel_number_and_replay_identity() {
    let mut page = page();
    assert_eq!(page.validate(&query(), NOW), Ok(()));
    let channel = page
        .items
        .iter_mut()
        .find(|item| item.kind == ContentKind::Channel)
        .unwrap();
    let identity = channel.key.clone();
    channel.channel_number = Some("27".into());
    assert_eq!(channel.key, identity);
    assert_eq!(page.validate(&query(), NOW), Ok(()));

    for kind in [
        ContentKind::Broadcast,
        ContentKind::ReplayPortal,
        ContentKind::ReplayCollection,
    ] {
        let content = page.items.iter().find(|item| item.kind == kind).unwrap();
        let mut selected = offer(Provider::Freetv);
        selected.content = content.key.clone();
        let capabilities = ReaderCapabilities {
            providers: vec![Provider::Freetv],
            kinds: vec![ContentKind::Channel, ContentKind::ReplayProgram],
            intents: vec![Intent::PlayExact],
        };
        assert_eq!(
            check_selection(content, &selected, Intent::PlayExact, &capabilities, NOW),
            Err(SelectionError::UnsupportedKind)
        );
    }
}

#[test]
fn offer_unknown_is_not_entitlement() {
    let mut page = page();
    let subscription = offer(Provider::PrimeVideo);
    assert_eq!(subscription.entitlement, Entitlement::Unknown);
    let mut rental = subscription.clone();
    rental.id = "fixture-prime-rental".into();
    rental.mode = OfferMode::Rent;
    page.offers.push(rental.clone());
    assert_eq!(page.validate(&query(), NOW), Ok(()));
    assert_eq!(subscription.mode, OfferMode::Subscription);
    assert_eq!(rental.mode, OfferMode::Rent);

    let content = page
        .items
        .iter()
        .find(|item| item.key == subscription.content)
        .unwrap();
    assert_eq!(
        check_selection(
            content,
            &subscription,
            Intent::OpenDetails,
            &reader(Provider::PrimeVideo),
            NOW,
        ),
        Ok(SelectionCheck::RequiresAdapterEvidence)
    );
    assert_eq!(subscription.entitlement, Entitlement::Unknown);

    let mut wrong = subscription;
    wrong.content.id = "different-content".into();
    assert_eq!(
        check_selection(
            content,
            &wrong,
            Intent::OpenDetails,
            &reader(Provider::PrimeVideo),
            NOW,
        ),
        Err(SelectionError::OfferMismatch)
    );
}

#[test]
fn expired_or_future_offer_is_not_current() {
    let page = page();
    let content = &page.items[0];
    let mut selected = offer(Provider::Netflix);
    let capabilities = reader(Provider::Netflix);
    selected.expires_at = Some(NOW);
    assert_eq!(
        check_selection(content, &selected, Intent::OpenDetails, &capabilities, NOW),
        Err(SelectionError::OfferExpired)
    );
    selected.expires_at = None;
    selected.starts_at = Some(NOW + 1);
    assert_eq!(
        check_selection(content, &selected, Intent::OpenDetails, &capabilities, NOW),
        Err(SelectionError::OfferNotStarted)
    );
    selected.starts_at = None;
    assert_eq!(
        check_selection(content, &selected, Intent::OpenDetails, &capabilities, NOW),
        Ok(SelectionCheck::RequiresAdapterEvidence)
    );
    assert_eq!(selected.expires_at, None);
    selected.provenance.freshness.freshness_until = NOW;
    assert_eq!(
        check_selection(content, &selected, Intent::OpenDetails, &capabilities, NOW),
        Err(SelectionError::StaleEvidence)
    );

    let replay = page
        .items
        .iter()
        .find(|item| item.kind == ContentKind::ReplayProgram)
        .unwrap();
    let mut expired_replay = offer(Provider::Freetv);
    expired_replay.content = replay.key.clone();
    expired_replay.expires_at = Some(NOW - 1);
    let replay_reader = ReaderCapabilities {
        providers: vec![Provider::Freetv],
        kinds: vec![ContentKind::ReplayProgram],
        intents: vec![Intent::OpenDetails],
    };
    assert_eq!(
        check_selection(
            replay,
            &expired_replay,
            Intent::OpenDetails,
            &replay_reader,
            NOW,
        ),
        Err(SelectionError::OfferExpired)
    );
}

#[test]
fn intent_and_evidence_do_not_upgrade() {
    let page = page();
    let content = &page.items[0];
    let selected = offer(Provider::Netflix);
    assert_eq!(
        check_selection(
            content,
            &selected,
            Intent::PlayExact,
            &reader(Provider::Netflix),
            NOW,
        ),
        Err(SelectionError::UnsupportedIntent)
    );
    let mut capabilities = reader(Provider::Netflix);
    capabilities.intents.push(Intent::PlayExact);
    assert_eq!(
        check_selection(content, &selected, Intent::PlayExact, &capabilities, NOW),
        Err(SelectionError::IntentNotEstablished)
    );

    // A client-supplied label is neither evidence nor a new public field.
    let mut supplied = serde_json::to_value(content).unwrap();
    supplied["qualification"] = serde_json::json!("documented");
    assert!(serde_json::from_value::<moli_media_catalog::ContentRef>(supplied).is_err());
    let forged = br#"{"query":"papier","intent":"play_exact","official":true}"#;
    assert_eq!(
        SearchQuery::parse(forged),
        Err(CatalogError::InvalidRequest)
    );
}

#[test]
fn missing_source_differs_from_no_match() {
    let mut missing = page();
    missing.items.clear();
    missing.offers.clear();
    for source in &mut missing.sources {
        source.outcome = SourceOutcome::NotConfigured;
    }
    assert_eq!(
        missing.validate(&query(), NOW),
        Err(CatalogError::NoUsableSource)
    );
    for source in &mut missing.sources {
        source.outcome = SourceOutcome::NoMatch;
    }
    assert_eq!(missing.validate(&query(), NOW), Ok(()));

    let mut partial = page();
    partial.sources[0].outcome = SourceOutcome::Timeout;
    partial.truncated = true;
    assert_eq!(partial.validate(&query(), NOW), Ok(()));
    assert_eq!(partial.sources[0].outcome, SourceOutcome::Timeout);
    partial.sources[0].outcome = SourceOutcome::Quota;
    assert_eq!(partial.validate(&query(), NOW), Ok(()));
    assert_eq!(partial.sources[0].outcome, SourceOutcome::Quota);
}

#[test]
fn boundaries_and_untrusted_links() {
    for text in ["ab".to_owned(), "a".repeat(200), "\u{1f4da}".repeat(200)] {
        assert!(SearchQuery::parse(&request(&text, 20)).is_ok());
    }
    assert_eq!(
        SearchQuery::parse(&request(&"a".repeat(201), 20)),
        Err(CatalogError::QueryTooLong)
    );
    assert_eq!(
        SearchQuery::parse(&request("a", 20)),
        Err(CatalogError::QueryTooShort)
    );
    assert_eq!(
        SearchQuery::parse(&request("papier", 51)),
        Err(CatalogError::TooManyResults)
    );
    assert!(SearchQuery::parse(&request("papier", 50)).is_ok());
    let mut alias: serde_json::Value = serde_json::from_slice(&request("papier", 20)).unwrap();
    alias["metadata_language"] = serde_json::json!("fr");
    assert_eq!(
        SearchQuery::parse(&serde_json::to_vec(&alias).unwrap())
            .unwrap()
            .metadata_language,
        "fr-FR"
    );
    alias["country"] = serde_json::json!("US");
    assert_eq!(
        SearchQuery::parse(&serde_json::to_vec(&alias).unwrap()),
        Err(CatalogError::UnsupportedFilter)
    );
    assert_eq!(
        SearchQuery::parse(br#"{"query":"papier","query":"autre"}"#),
        Err(CatalogError::InvalidRequest)
    );
    let mut oversized = request("papier", 20);
    oversized.resize(4 * 1024 + 1, b' ');
    assert_eq!(
        SearchQuery::parse(&oversized),
        Err(CatalogError::RequestTooLarge)
    );
    let mut too_many = page();
    while too_many.items.len() <= 50 {
        let mut item = too_many.items[0].clone();
        item.key.id = format!("fixture-extra-{}", too_many.items.len());
        too_many.items.push(item);
    }
    assert_eq!(
        too_many.validate(&query(), NOW),
        Err(CatalogError::TooManyResults)
    );
    for url in [
        "javascript:fixture",
        "file:///fixture",
        "https://provider.example.com@127.0.0.1/fixture",
        "https://provider.example.com.evil.invalid/fixture",
    ] {
        let mut hostile = page();
        hostile.items[0].image = Some(url.into());
        assert_eq!(
            hostile.validate(&query(), NOW),
            Err(CatalogError::UnsafeReference)
        );
    }
}

#[test]
fn same_reference_two_readers() {
    let page = page();
    let content = &page.items[0];
    let selected = offer(Provider::Netflix);
    let before = content.clone();
    let compatible = reader(Provider::Netflix);
    let incompatible = reader(Provider::Molotov);
    assert_eq!(
        check_selection(content, &selected, Intent::OpenDetails, &compatible, NOW),
        Ok(SelectionCheck::RequiresAdapterEvidence)
    );
    assert_eq!(
        check_selection(content, &selected, Intent::OpenDetails, &incompatible, NOW),
        Err(SelectionError::UnsupportedProvider)
    );
    assert_eq!(content, &before);
}
