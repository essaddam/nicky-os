//! Explicit content targets, independent of a player or network catalogue.
//! Resolution validates known link shapes; it never contacts a provider or
//! proves that the requested content is playing on a receiver.

mod links;
mod uri;

use serde::{Deserialize, Serialize};

const MAX_REQUEST: usize = 4096;
const MAX_TARGET: usize = 2048;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Service {
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
    App,
    Title,
    Episode,
    Channel,
    Replay,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContentRequest {
    pub service: Service,
    pub kind: ContentKind,
    #[serde(default)]
    pub target: String,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Qualification {
    Documented,
    Unqualified,
}

/// A link to transmit, never proof that the requested content is playing.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct LaunchTarget {
    pub uri: String,
    pub expected_packages: Vec<String>,
    pub qualification: Qualification,
}

/// Machine-readable errors: never include a supplied link or token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentError {
    InvalidRequest,
    TooLong,
    InvalidTarget,
    UnsupportedTarget,
}

impl std::fmt::Display for ContentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidRequest => "invalid_request",
            Self::TooLong => "target_too_long",
            Self::InvalidTarget => "invalid_target",
            Self::UnsupportedTarget => "unsupported_target",
        })
    }
}

impl std::error::Error for ContentError {}

impl ContentRequest {
    /// Bounds the JSON before deserializing. Duplicate and unknown fields
    /// are errors, as are unknown providers and target kinds.
    pub fn parse(input: &str) -> Result<Self, ContentError> {
        if input.len() > MAX_REQUEST {
            return Err(ContentError::TooLong);
        }
        let request: Self =
            serde_json::from_str(input).map_err(|_| ContentError::InvalidRequest)?;
        if request.target.len() > MAX_TARGET {
            return Err(ContentError::TooLong);
        }
        Ok(request)
    }
}

/// Resolves only an explicit provider target. A refusal never falls back
/// to the provider's home screen; searching by name belongs to a catalogue.
pub fn resolve(request: &ContentRequest) -> Result<LaunchTarget, ContentError> {
    if request.target.len() > MAX_TARGET {
        return Err(ContentError::TooLong);
    }
    // Check before trimming: a newline must not become an apparently safe URL.
    if request.target.chars().any(char::is_control) {
        return Err(ContentError::InvalidTarget);
    }
    let target = request.target.trim();
    let (uri, qualification) = links::resolve(request.service, request.kind, target)?;
    Ok(LaunchTarget {
        uri,
        expected_packages: links::packages(request.service)
            .iter()
            .map(|p| (*p).to_owned())
            .collect(),
        qualification,
    })
}

#[cfg(test)]
mod tests;
