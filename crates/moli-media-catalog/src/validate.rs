//! Bounded, deterministic validation; references are never fetched or rendered.

use std::io::{self, Write};

use crate::{
    CatalogError, ContentKey, ContentKind, ContentRef, Freshness, MAX_KEY_BYTES, MAX_PAGE_SIZE,
    MAX_PROVIDERS, MAX_QUERY_BYTES, MAX_QUERY_CHARS, MAX_REFERENCE_BYTES, MAX_RESPONSE_BYTES,
    MAX_SUMMARY_BYTES, MAX_TITLE_BYTES, Provenance, SearchPage, SearchQuery, SourceOutcome,
};

pub(crate) fn query(query: &SearchQuery) -> Result<(), CatalogError> {
    let length = query.query.chars().count();
    if length < 2 {
        return Err(CatalogError::QueryTooShort);
    }
    if length > MAX_QUERY_CHARS || query.query.len() > MAX_QUERY_BYTES {
        return Err(CatalogError::QueryTooLong);
    }
    if query.page_size == 0 || query.page_size > MAX_PAGE_SIZE {
        return Err(CatalogError::TooManyResults);
    }
    if query.country != "FR" || query.metadata_language != "fr-FR" {
        return Err(CatalogError::UnsupportedFilter);
    }
    if query.providers.len() > MAX_PROVIDERS
        || query.kinds.len() > 9
        || query.query.chars().any(char::is_control)
        || query
            .cursor
            .as_ref()
            .is_some_and(|v| !text(v, MAX_KEY_BYTES))
        || query
            .providers
            .iter()
            .enumerate()
            .any(|(i, v)| query.providers[..i].contains(v))
        || query
            .kinds
            .iter()
            .enumerate()
            .any(|(i, v)| query.kinds[..i].contains(v))
    {
        return Err(CatalogError::InvalidRequest);
    }
    Ok(())
}

pub(crate) fn text(value: &str, limit: usize) -> bool {
    !value.trim().is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
}

fn key(value: &ContentKey) -> bool {
    text(&value.source, MAX_KEY_BYTES) && text(&value.id, MAX_KEY_BYTES)
}

pub(crate) fn fresh(value: &Freshness, now_ms: u64) -> bool {
    value.fetched_at <= now_ms
        && now_ms < value.freshness_until
        && value
            .upstream_updated_at
            .is_none_or(|v| v <= value.fetched_at)
}

/// Syntax restrictions only: DNS spelling proves neither public reachability
/// nor an official provider. No URL is an operational target in this crate.
fn reference(value: &str) -> bool {
    if value.len() > MAX_REFERENCE_BYTES
        || value.chars().any(|c| c.is_whitespace() || c.is_control())
        || value.contains('\\')
    {
        return false;
    }
    let Some(rest) = value.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let labels: Vec<_> = host.split('.').collect();
    let Some(suffix) = labels.last() else {
        return false;
    };
    labels.len() >= 2
        && host.len() <= 253
        && suffix.bytes().all(|c| c.is_ascii_alphabetic())
        && ![
            "invalid",
            "localhost",
            "local",
            "internal",
            "test",
            "example",
        ]
        .iter()
        .any(|v| suffix.eq_ignore_ascii_case(v))
        && labels.iter().all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        })
}

pub(crate) fn provenance(value: &Provenance, now_ms: u64) -> bool {
    text(&value.source, MAX_KEY_BYTES)
        && text(&value.record_id, MAX_KEY_BYTES)
        && fresh(&value.freshness, now_ms)
        && value.evidence_url.as_ref().is_none_or(|v| reference(v))
}

fn parents(item: &ContentRef, page: &SearchPage) -> bool {
    let find = |key: &ContentKey, kind| {
        page.items
            .iter()
            .find(|v| &v.key == key && v.kind == kind && v.key != item.key)
    };
    if item
        .series
        .as_ref()
        .is_some_and(|k| find(k, ContentKind::Series).is_none())
        || item
            .season
            .as_ref()
            .is_some_and(|k| find(k, ContentKind::Season).is_none())
        || item
            .channel
            .as_ref()
            .is_some_and(|k| find(k, ContentKind::Channel).is_none())
    {
        return false;
    }
    if let Some(season) = item
        .season
        .as_ref()
        .and_then(|k| find(k, ContentKind::Season))
        && (item.series != season.series || item.season_number != season.season_number)
    {
        return false;
    }
    let episodic = matches!(item.kind, ContentKind::Season | ContentKind::Episode);
    let televised = matches!(
        item.kind,
        ContentKind::Broadcast | ContentKind::ReplayProgram
    );
    (episodic || (item.series.is_none() && item.season_number.is_none()))
        && (item.kind == ContentKind::Episode
            || (item.season.is_none() && item.episode_number.is_none()))
        && (televised || item.channel.is_none())
        && (item.kind == ContentKind::Channel || item.channel_number.is_none())
}

struct ResponseBudget(usize);

impl Write for ResponseBudget {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_RESPONSE_BYTES.saturating_sub(self.0) {
            return Err(io::Error::other("media response budget exceeded"));
        }
        self.0 += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn page(
    page: &SearchPage,
    request: &SearchQuery,
    now_ms: u64,
) -> Result<(), CatalogError> {
    query(request)?;
    if page.items.len() > request.page_size
        || page.items.len() > MAX_PAGE_SIZE
        || page.offers.len() > MAX_PAGE_SIZE * MAX_PROVIDERS
        || page.sources.len() > MAX_PROVIDERS
    {
        return Err(CatalogError::TooManyResults);
    }
    serde_json::to_writer(&mut ResponseBudget(0), page)
        .map_err(|_| CatalogError::TooManyResults)?;
    if !page
        .sources
        .iter()
        .any(|s| matches!(s.outcome, SourceOutcome::Available | SourceOutcome::NoMatch))
    {
        return Err(CatalogError::NoUsableSource);
    }
    if page
        .next_cursor
        .as_ref()
        .is_some_and(|v| !text(v, MAX_KEY_BYTES))
    {
        return Err(CatalogError::Unvalidated);
    }
    for (index, item) in page.items.iter().enumerate() {
        if page.items[..index].iter().any(|v| v.key == item.key) {
            return Err(CatalogError::DuplicateIdentity);
        }
        if !key(&item.key)
            || !text(&item.title, MAX_TITLE_BYTES)
            || item
                .summary
                .as_ref()
                .is_some_and(|v| v.len() > MAX_SUMMARY_BYTES)
            || item
                .metadata_language
                .as_ref()
                .is_some_and(|v| v != "fr" && v != "fr-FR")
            || item
                .channel_number
                .as_ref()
                .is_some_and(|v| !text(v, MAX_KEY_BYTES))
            || item.provider_refs.len() > MAX_PROVIDERS
            || item.provenance.source != item.key.source
            || !provenance(&item.provenance, now_ms)
        {
            return Err(CatalogError::Unvalidated);
        }
        if item.image.as_ref().is_some_and(|v| !reference(v)) {
            return Err(CatalogError::UnsafeReference);
        }
        if !parents(item, page) {
            return Err(CatalogError::InvalidParent);
        }
        for provider_ref in &item.provider_refs {
            if !text(&provider_ref.namespace, MAX_KEY_BYTES)
                || !text(&provider_ref.id, MAX_KEY_BYTES)
                || !provenance(&provider_ref.provenance, now_ms)
            {
                return Err(CatalogError::Unvalidated);
            }
        }
    }
    for (index, offer) in page.offers.iter().enumerate() {
        if !text(&offer.id, MAX_KEY_BYTES)
            || offer.country != "FR"
            || !page.items.iter().any(|v| v.key == offer.content)
            || offer
                .plan
                .as_ref()
                .is_some_and(|v| !text(v, MAX_TITLE_BYTES))
            || !provenance(&offer.provenance, now_ms)
            || offer
                .starts_at
                .zip(offer.expires_at)
                .is_some_and(|(a, b)| a >= b)
            || page.offers[..index]
                .iter()
                .any(|v| v.id == offer.id && v.provider == offer.provider)
        {
            return Err(CatalogError::Unvalidated);
        }
    }
    for (index, source) in page.sources.iter().enumerate() {
        if !text(&source.source, MAX_KEY_BYTES)
            || page.sources[..index]
                .iter()
                .any(|v| v.source == source.source && v.provider == source.provider)
        {
            return Err(CatalogError::Unvalidated);
        }
    }
    Ok(())
}
