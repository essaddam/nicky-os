//! Executable definitions have a draft and a separately published current head.
//! Authorization, consumer previews, hashing and durability belong to the runtime.

use serde::{Deserialize, Serialize};

use crate::DeviceId;

/// Versions stay exactly representable in JSON clients.
pub const MAX_DEFINITION_VERSION: u64 = (1_u64 << 53) - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyKind {
    Schedule,
    Scene,
}

/// Stable identity, independent of any revision, label or time zone.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DependencyRef {
    pub kind: DependencyKind,
    pub id: DeviceId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DefinitionVersion<T> {
    pub version: u64,
    pub definition: T,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicationVersion {
    pub draft: u64,
    pub current: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VersionError {
    Conflict,
    Exhausted,
    Invalid,
}

/// Only two heads are retained. Editing never changes the published definition.
/// Deserialized state must pass `is_valid` and its domain validation before use.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VersionedDefinition<T> {
    draft: DefinitionVersion<T>,
    current: Option<DefinitionVersion<T>>,
}

impl<T: Clone + PartialEq> VersionedDefinition<T> {
    pub fn new(definition: T) -> Self {
        Self {
            draft: DefinitionVersion {
                version: 1,
                definition,
            },
            current: None,
        }
    }

    pub fn draft(&self) -> &DefinitionVersion<T> {
        &self.draft
    }

    pub fn current(&self) -> Option<&DefinitionVersion<T>> {
        self.current.as_ref()
    }

    pub fn versions(&self) -> PublicationVersion {
        PublicationVersion {
            draft: self.draft.version,
            current: self.current.as_ref().map(|v| v.version),
        }
    }

    pub fn is_valid(&self) -> bool {
        (1..=MAX_DEFINITION_VERSION).contains(&self.draft.version)
            && self.current.as_ref().is_none_or(|v| {
                (1..=self.draft.version).contains(&v.version)
                    && (v.version != self.draft.version || v.definition == self.draft.definition)
            })
    }

    pub fn stage(&mut self, expected: u64, definition: T) -> Result<(), VersionError> {
        if !self.is_valid() {
            return Err(VersionError::Invalid);
        }
        if self.draft.version != expected {
            return Err(VersionError::Conflict);
        }
        if self.draft.version == MAX_DEFINITION_VERSION {
            return Err(VersionError::Exhausted);
        }
        self.draft = DefinitionVersion {
            version: expected + 1,
            definition,
        };
        Ok(())
    }

    /// The caller separately verifies human authority and the consumer preview.
    /// Returns false for an already-current revision with matching preconditions.
    pub fn publish(&mut self, expected: PublicationVersion) -> Result<bool, VersionError> {
        if !self.is_valid() {
            return Err(VersionError::Invalid);
        }
        if self.versions() != expected {
            return Err(VersionError::Conflict);
        }
        if self
            .current
            .as_ref()
            .is_some_and(|v| v.version == expected.draft)
        {
            return Ok(false);
        }
        self.current = Some(self.draft.clone());
        Ok(true)
    }
}
