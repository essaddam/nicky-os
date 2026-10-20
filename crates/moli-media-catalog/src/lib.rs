//! Pure media identities, bounded discovery data and metadata selection checks.
//! No discovery I/O, operational trust, command or playback is provided here.

mod model;
mod selection;
mod validate;

pub use model::{
    CatalogError, ContentKey, ContentKind, ContentRef, Entitlement, Freshness, Intent, Offer,
    OfferMode, Provenance, Provider, ProviderRef, ReaderCapabilities, SearchPage, SearchQuery,
    SelectionCheck, SelectionError, SourceOutcome, SourceStatus,
};
pub use selection::check_selection;

/// Maximum bytes accepted before decoding a query.
pub const MAX_REQUEST_BYTES: usize = 4 * 1024;
pub const MAX_QUERY_CHARS: usize = 200;
pub const MAX_QUERY_BYTES: usize = 800;
pub const MAX_PROVIDERS: usize = 7;
pub const DEFAULT_PAGE_SIZE: usize = 20;
pub const MAX_PAGE_SIZE: usize = 50;
pub const MAX_KEY_BYTES: usize = 256;
pub const MAX_TITLE_BYTES: usize = 512;
pub const MAX_SUMMARY_BYTES: usize = 4_000;
pub const MAX_REFERENCE_BYTES: usize = 2_048;
pub const MAX_RESPONSE_BYTES: usize = 256 * 1024;

impl SearchQuery {
    /// Decodes a bounded French discovery query and normalizes the `fr` alias.
    ///
    /// # Errors
    /// Rejects malformed/unknown/duplicate fields, excessive input or results,
    /// unsupported filters and a query outside the character/byte budget.
    pub fn parse(input: &[u8]) -> Result<Self, CatalogError> {
        if input.len() > MAX_REQUEST_BYTES {
            return Err(CatalogError::RequestTooLarge);
        }
        let mut query: Self =
            serde_json::from_slice(input).map_err(|_| CatalogError::InvalidRequest)?;
        query.query = query.query.trim().to_owned();
        if query.metadata_language == "fr" {
            query.metadata_language = "fr-FR".into();
        }
        validate::query(&query)?;
        Ok(query)
    }
}

impl SearchPage {
    /// Validates bounded identities, parent references and discovery provenance.
    ///
    /// # Errors
    /// Rejects invalid bounds, duplicate identities, inconsistent parents,
    /// unusable sources, stale/malformed metadata and unsafe references.
    pub fn validate(&self, query: &SearchQuery, now_ms: u64) -> Result<(), CatalogError> {
        validate::page(self, query, now_ms)
    }
}
