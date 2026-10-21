//! Bounded executable definitions, independently of the periodic state cache.
mod calendar;
mod store;

use moli_core::{DependencyRef, PublicationVersion, VersionedDefinition, WeeklySchedule};
use serde::{Deserialize, Serialize};

pub(crate) use calendar::Calendar;
pub(crate) use store::Dependencies;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DependencyError {
    Definition,
    Unknown,
    Conflict,
    Capacity,
    Human,
    Corrupt,
    Storage,
    Uncertain,
    Consumers,
    Projection,
}

impl std::fmt::Display for DependencyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&match self {
            Self::Definition => moli_i18n::tr!("serveur.dependance.definition"),
            Self::Unknown => moli_i18n::tr!("serveur.dependance.inconnue"),
            Self::Conflict => moli_i18n::tr!("serveur.dependance.conflit"),
            Self::Capacity => moli_i18n::tr!("serveur.dependance.capacite"),
            Self::Human => moli_i18n::tr!("serveur.dependance.humain"),
            Self::Corrupt => moli_i18n::tr!("serveur.dependance.corrompu"),
            Self::Storage => moli_i18n::tr!("serveur.dependance.persistance"),
            Self::Uncertain => moli_i18n::tr!("serveur.dependance.incertain"),
            Self::Consumers => moli_i18n::tr!("serveur.dependance.consommateurs"),
            Self::Projection => moli_i18n::tr!("serveur.dependance.projection"),
        })
    }
}
impl std::error::Error for DependencyError {}

/// Extend with typed scene/calculation definitions when their lots are implemented.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum DependencyDefinition {
    Schedule(WeeklySchedule),
}

impl DependencyDefinition {
    pub(crate) fn normalized(self) -> Result<Self, DependencyError> {
        match self {
            Self::Schedule(plan) => Ok(Self::Schedule(Calendar::new(plan)?.definition().clone())),
        }
    }
    pub(crate) fn calendar(&self) -> Result<Calendar, DependencyError> {
        match self {
            Self::Schedule(plan) => Calendar::new(plan.clone()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DependencyView {
    pub reference: DependencyRef,
    pub versions: VersionedDefinition<DependencyDefinition>,
}

/// A consumer's identity and executable logic. Flags may only narrow authority;
/// edits to graph identity, name or logic invalidate a publication preview.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DependencyConsumer {
    pub id: String,
    pub name: String,
    pub fingerprint: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DependencyPreview {
    pub reference: DependencyRef,
    pub versions: PublicationVersion,
    pub consumers: Vec<DependencyConsumer>,
    pub fingerprint: String,
}

pub(crate) fn fingerprint(value: &impl Serialize) -> String {
    use std::fmt::Write as _;
    let bytes = serde_json::to_vec(value).expect("typed bounded definition is JSON serializable");
    let mut result = String::with_capacity(64);
    for byte in ring::digest::digest(&ring::digest::SHA256, &bytes).as_ref() {
        let _ = write!(result, "{byte:02x}");
    }
    result
}

/// Trusted executable owner, consulted under the shared publication gate.
pub trait DependencyConsumerSource: std::fmt::Debug + Send + Sync {
    fn consumers(
        &self,
        reference: &DependencyRef,
    ) -> Result<Vec<DependencyConsumer>, DependencyError>;
}
