//! Durable ownership handoff contracts. No external action or physical execution.
mod model;
mod registry;
mod storage;

pub use model::{
    AutomationProvenance, Binding, BindingKey, Direction, MigrationPlan, Owner, Phase, Proof,
    ProofChallenge, Snapshot,
};
pub use registry::Registry;

pub const MAX_BINDINGS: usize = 512;
pub const MAX_ACTIVE_OPERATIONS: usize = 128;
pub const MAX_STATE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MigrationError {
    #[error("invalid migration contract")]
    Invalid,
    #[error("unknown migration binding")]
    Unknown,
    #[error("native reference is unresolved or not writable")]
    Unresolved,
    #[error("migration revision conflict")]
    Revision,
    #[error("migration plan fingerprint mismatch")]
    PlanHash,
    #[error("migration proof does not match the operation")]
    Proof,
    #[error("migration transition refused")]
    Transition,
    #[error("migration registry bound exceeded")]
    Bound,
    #[error("migration registry must be reloaded before reconciliation")]
    ReloadRequired,
    #[error("fresh migration reconciliation is required")]
    ReconcileRequired,
    #[error("migration state cannot be read")]
    Load,
    #[error("migration persistence failed (commit unknown: {commit_unknown})")]
    Storage { commit_unknown: bool },
    #[error("native write authority refused the generation")]
    Policy,
    #[error("migration generation or revision exhausted")]
    Exhausted,
    #[error("migration operation task interrupted")]
    Interrupted,
}

#[cfg(test)]
mod tests;
