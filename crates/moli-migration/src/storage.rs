use std::{collections::BTreeSet, fmt::Debug, future::Future, pin::Pin};

use moli_runtime::{StateFile, StateFileError};
use serde::{Deserialize, Serialize};

use crate::{Binding, MAX_ACTIVE_OPERATIONS, MAX_BINDINGS, MAX_STATE_BYTES, MigrationError};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct State {
    pub schema: u8,
    pub bindings: Vec<Binding>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            schema: 1,
            bindings: Vec::new(),
        }
    }
}

impl State {
    pub fn active(&self) -> usize {
        self.bindings
            .iter()
            .filter(|binding| binding.phase.active())
            .count()
    }

    pub fn validate(&self) -> Result<(), MigrationError> {
        if self.schema != 1
            || self.bindings.len() > MAX_BINDINGS
            || self.active() > MAX_ACTIVE_OPERATIONS
        {
            return Err(MigrationError::Bound);
        }
        let mut keys = BTreeSet::new();
        let mut devices = BTreeSet::new();
        let mut active_operations = BTreeSet::new();
        for binding in &self.bindings {
            if !keys.insert(&binding.key)
                || !devices.insert(&binding.native_device)
                || (binding.phase.active() && !active_operations.insert(&binding.operation_id))
                || binding.key != binding.plan.binding
                || binding.native_device != binding.plan.native_device
                || binding.revision == 0
                || binding.generation == 0
                || binding.reconcile_required
                || !crate::model::operation(&binding.operation_id)
            {
                return Err(MigrationError::Invalid);
            }
            if binding.plan.hash()? != binding.plan_hash {
                return Err(MigrationError::PlanHash);
            }
        }
        if serde_json::to_vec(self)
            .map_err(|_| MigrationError::Invalid)?
            .len()
            > MAX_STATE_BYTES
        {
            return Err(MigrationError::Bound);
        }
        Ok(())
    }
}

pub(crate) type StoreFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Private seam for deterministic pre/post-rename failure tests.
pub(crate) trait Store: Debug + Send + Sync {
    fn load(&self) -> StoreFuture<'_, std::io::Result<Option<State>>>;
    fn write<'a>(&'a self, state: &'a State) -> StoreFuture<'a, Result<(), StateFileError>>;
}

#[derive(Debug)]
pub(crate) struct FileStore(pub StateFile);

impl Store for FileStore {
    fn load(&self) -> StoreFuture<'_, std::io::Result<Option<State>>> {
        Box::pin(self.0.load())
    }

    fn write<'a>(&'a self, state: &'a State) -> StoreFuture<'a, Result<(), StateFileError>> {
        Box::pin(self.0.write(state))
    }
}
