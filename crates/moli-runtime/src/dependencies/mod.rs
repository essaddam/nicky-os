//! Bounded executable definitions, independently of the periodic state cache.
mod calendar;
mod context;
mod store;
pub(crate) use context::{affected, bindings};

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
    Cycle,
    Units,
    Budget,
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
            Self::Cycle => moli_i18n::tr!("serveur.calcul.cycle"),
            Self::Units => moli_i18n::tr!("serveur.calcul.unites"),
            Self::Budget => moli_i18n::tr!("serveur.calcul.budget"),
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
    Aggregate(moli_core::AggregateSpec),
}

impl DependencyDefinition {
    pub(crate) fn normalized(self) -> Result<Self, DependencyError> {
        match self {
            Self::Schedule(plan) => Ok(Self::Schedule(Calendar::new(plan)?.definition().clone())),
            Self::Aggregate(spec) => Ok(Self::Aggregate(
                spec.normalize().map_err(|_| DependencyError::Definition)?,
            )),
        }
    }
    pub(crate) fn kind(&self) -> moli_core::DependencyKind {
        match self {
            Self::Schedule(_) => moli_core::DependencyKind::Schedule,
            Self::Aggregate(_) => moli_core::DependencyKind::Computed,
        }
    }
    pub(crate) fn sources(&self) -> &[moli_core::PointId] {
        match self {
            Self::Schedule(_) => &[],
            Self::Aggregate(spec) => &spec.sources,
        }
    }
    pub(crate) fn calendar(&self) -> Result<Option<Calendar>, DependencyError> {
        match self {
            Self::Schedule(plan) => Calendar::new(plan.clone()).map(Some),
            Self::Aggregate(_) => Ok(None),
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<DependencyBinding>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingRole {
    Source,
    Affected,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DependencyBinding {
    pub reference: DependencyRef,
    pub role: BindingRole,
    pub current: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draft: Option<u64>,
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
    fn consumers_for(
        &self,
        references: &[DependencyRef],
    ) -> Result<Vec<DependencyConsumer>, DependencyError> {
        let mut result = Vec::new();
        for reference in references {
            result.extend(self.consumers(reference)?);
        }
        Ok(result)
    }
}

pub(crate) fn graph_error(error: moli_core::AggregateGraphError) -> DependencyError {
    match error {
        moli_core::AggregateGraphError::Cycle => DependencyError::Cycle,
        moli_core::AggregateGraphError::Units => DependencyError::Units,
        moli_core::AggregateGraphError::Capacity | moli_core::AggregateGraphError::Depth => {
            DependencyError::Budget
        }
        moli_core::AggregateGraphError::Definition => DependencyError::Definition,
    }
}
