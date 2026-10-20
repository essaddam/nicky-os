//! Metadata selection boundary; no operational target is constructed.

use crate::{
    ContentKind, ContentRef, Intent, MAX_KEY_BYTES, Offer, ReaderCapabilities, SelectionCheck,
    SelectionError, validate,
};

/// Checks metadata suitability without authorizing or constructing a command.
///
/// # Errors
/// Rejects mismatched offers, unsupported readers/types/intentions, missing
/// provider references, out-of-window offers and stale evidence. `PlayExact`
/// always requires evidence from a future adapter and is refused here.
pub fn check_selection(
    content: &ContentRef,
    offer: &Offer,
    intent: Intent,
    capabilities: &ReaderCapabilities,
    now_ms: u64,
) -> Result<SelectionCheck, SelectionError> {
    if offer.content != content.key || offer.country != "FR" {
        return Err(SelectionError::OfferMismatch);
    }
    if !capabilities.providers.contains(&offer.provider) {
        return Err(SelectionError::UnsupportedProvider);
    }
    if !capabilities.kinds.contains(&content.kind)
        || !matches!(
            content.kind,
            ContentKind::Movie
                | ContentKind::Series
                | ContentKind::Episode
                | ContentKind::Channel
                | ContentKind::ReplayProgram
        )
    {
        return Err(SelectionError::UnsupportedKind);
    }
    if !capabilities.intents.contains(&intent) {
        return Err(SelectionError::UnsupportedIntent);
    }
    let reference = content
        .provider_refs
        .iter()
        .find(|reference| {
            reference.provider == offer.provider
                && validate::text(&reference.namespace, MAX_KEY_BYTES)
                && validate::text(&reference.id, MAX_KEY_BYTES)
        })
        .ok_or(SelectionError::MissingProviderReference)?;
    if offer.expires_at.is_some_and(|v| v <= now_ms) {
        return Err(SelectionError::OfferExpired);
    }
    if offer.starts_at.is_some_and(|v| v > now_ms) {
        return Err(SelectionError::OfferNotStarted);
    }
    if !validate::provenance(&content.provenance, now_ms)
        || !validate::provenance(&offer.provenance, now_ms)
        || !validate::provenance(&reference.provenance, now_ms)
    {
        return Err(SelectionError::StaleEvidence);
    }
    if intent == Intent::PlayExact {
        return Err(SelectionError::IntentNotEstablished);
    }
    Ok(SelectionCheck::RequiresAdapterEvidence)
}
