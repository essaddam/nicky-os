//! Data and machine-readable contract errors. No field grants operational trust.

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Netflix,
    Crunchyroll,
    Freetv,
    #[serde(rename = "primevideo")]
    PrimeVideo,
    #[serde(rename = "disneyplus")]
    DisneyPlus,
    Molotov,
    #[serde(rename = "appletv")]
    AppleTv,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContentKind {
    Movie,
    Series,
    Season,
    Episode,
    Channel,
    Broadcast,
    ReplayProgram,
    ReplayCollection,
    ReplayPortal,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContentKey {
    pub source: String,
    pub id: String,
}

/// All timestamps are synthetic or observed UTC epoch milliseconds.
/// An unknown upstream timestamp must not be replaced with the fetch time.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Freshness {
    pub fetched_at: u64,
    pub upstream_updated_at: Option<u64>,
    pub freshness_until: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub source: String,
    pub record_id: String,
    /// Untrusted evidence location; never fetched by this crate.
    pub evidence_url: Option<String>,
    pub freshness: Freshness,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderRef {
    pub provider: Provider,
    pub namespace: String,
    pub id: String,
    pub provenance: Provenance,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContentRef {
    pub key: ContentKey,
    pub kind: ContentKind,
    pub title: String,
    pub year: Option<u16>,
    /// Language actually known for the metadata, not an audio-track promise.
    pub metadata_language: Option<String>,
    pub series: Option<ContentKey>,
    pub season: Option<ContentKey>,
    pub channel: Option<ContentKey>,
    pub season_number: Option<u16>,
    pub episode_number: Option<u16>,
    /// Display label only; never the stable channel identifier.
    pub channel_number: Option<String>,
    pub summary: Option<String>,
    /// Untrusted location; neither fetched nor rendered by this crate.
    pub image: Option<String>,
    #[serde(default)]
    pub provider_refs: Vec<ProviderRef>,
    pub provenance: Provenance,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OfferMode {
    Subscription,
    Rent,
    Buy,
    Free,
    Ads,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Entitlement {
    KnownAllowed,
    KnownDenied,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Offer {
    pub id: String,
    pub content: ContentKey,
    pub provider: Provider,
    pub country: String,
    pub mode: OfferMode,
    pub plan: Option<String>,
    pub starts_at: Option<u64>,
    pub expires_at: Option<u64>,
    pub entitlement: Entitlement,
    pub provenance: Provenance,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SearchQuery {
    pub query: String,
    #[serde(default)]
    pub providers: Vec<Provider>,
    #[serde(default)]
    pub kinds: Vec<ContentKind>,
    #[serde(default = "country")]
    pub country: String,
    #[serde(default = "language")]
    pub metadata_language: String,
    #[serde(default = "page_size")]
    pub page_size: usize,
    pub cursor: Option<String>,
}

fn country() -> String {
    "FR".into()
}

fn language() -> String {
    "fr-FR".into()
}

fn page_size() -> usize {
    crate::DEFAULT_PAGE_SIZE
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceOutcome {
    Available,
    NoMatch,
    Timeout,
    Quota,
    NotConfigured,
    NotSearched,
    Unavailable,
    StaleCache,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SourceStatus {
    pub source: String,
    pub provider: Provider,
    pub outcome: SourceOutcome,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SearchPage {
    pub items: Vec<ContentRef>,
    pub offers: Vec<Offer>,
    pub sources: Vec<SourceStatus>,
    pub next_cursor: Option<String>,
    pub truncated: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Intent {
    OpenDetails,
    PlayExact,
}

/// Declarative input only, not proof that a transport conserves an intention.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReaderCapabilities {
    pub providers: Vec<Provider>,
    pub kinds: Vec<ContentKind>,
    pub intents: Vec<Intent>,
}

/// Even a successful metadata check cannot authorize a command or playback.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SelectionCheck {
    RequiresAdapterEvidence,
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum CatalogError {
    #[error("invalid_request")]
    InvalidRequest,
    #[error("request_too_large")]
    RequestTooLarge,
    #[error("query_too_short")]
    QueryTooShort,
    #[error("query_too_long")]
    QueryTooLong,
    #[error("unsupported_filter")]
    UnsupportedFilter,
    #[error("too_many_results")]
    TooManyResults,
    #[error("duplicate_identity")]
    DuplicateIdentity,
    #[error("invalid_parent")]
    InvalidParent,
    #[error("no_usable_source")]
    NoUsableSource,
    #[error("unsafe_reference")]
    UnsafeReference,
    #[error("unvalidated")]
    Unvalidated,
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum SelectionError {
    #[error("missing_provider_reference")]
    MissingProviderReference,
    #[error("unsupported_kind")]
    UnsupportedKind,
    #[error("unsupported_provider")]
    UnsupportedProvider,
    #[error("unsupported_intent")]
    UnsupportedIntent,
    #[error("intent_not_established")]
    IntentNotEstablished,
    #[error("offer_mismatch")]
    OfferMismatch,
    #[error("offer_not_started")]
    OfferNotStarted,
    #[error("offer_expired")]
    OfferExpired,
    #[error("stale_evidence")]
    StaleEvidence,
}
