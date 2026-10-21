//! One bounded signal queue and projection graph per hub. The hub model lock
//! precedes `signals`; only the executor takes `engine` (before the model).
mod engine;
use crate::DependencyError;
pub(crate) use engine::{Engine, Evaluation, Projection};
use moli_core::{AggregateSpec, DeviceId, PointId, Unit, aggregate_order};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{
    Mutex, PoisonError,
    atomic::{AtomicBool, Ordering},
};
use std::time::Instant;

#[derive(Clone, Debug)]
pub(crate) struct Observation {
    pub ts: u64,
    pub at: Instant,
    pub unit: Option<Unit>,
}

#[derive(Debug, Default)]
pub(crate) struct Signals {
    pub index: BTreeMap<PointId, BTreeSet<DeviceId>>,
    pub devices: BTreeMap<DeviceId, BTreeSet<DeviceId>>,
    pub dirty: BTreeSet<DeviceId>,
    pub observed: BTreeMap<PointId, Observation>,
    pub removed: BTreeSet<DeviceId>,
}
#[derive(Debug, Default)]
pub(crate) struct Computed {
    pub engine: Mutex<Engine>,
    pub signals: Mutex<Signals>,
    active: AtomicBool,
}
impl Computed {
    /// Called under the hub model write guard, including identical values.
    pub(crate) fn sample(&self, point: PointId, ts: u64, at: Instant, unit: Option<Unit>) -> bool {
        if !self.active.load(Ordering::Acquire) {
            return false;
        }
        let mut signals = self.signals.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(roots) = signals.index.get(&point).cloned() else {
            return false;
        };
        signals.dirty.extend(roots);
        signals.observed.insert(point, Observation { ts, at, unit });
        true
    }
    /// Description/availability events never renew an observation's clock.
    pub(crate) fn device(&self, device: &DeviceId, removed: bool) -> bool {
        if !self.active.load(Ordering::Acquire) {
            return false;
        }
        let mut signals = self.signals.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(roots) = signals.devices.get(device).cloned() else {
            return false;
        };
        signals.dirty.extend(roots);
        if removed {
            signals
                .observed
                .retain(|point, _| point.split().is_none_or(|(id, _)| &id != device));
            signals.removed.insert(device.clone());
        }
        true
    }
    /// Caller holds engine and model, so no signal can fall between indexing
    /// and the first coherent projection of the new current definitions.
    pub(crate) fn install(
        &self,
        engine: &mut Engine,
        signals: &mut Signals,
        definitions: BTreeMap<DeviceId, AggregateSpec>,
    ) -> Result<(), DependencyError> {
        let order = aggregate_order(&definitions).map_err(crate::dependencies::graph_error)?;
        signals.index.clear();
        signals.devices.clear();
        for (id, spec) in &definitions {
            signals
                .devices
                .entry(id.clone())
                .or_default()
                .insert(id.clone());
            for source in &spec.sources {
                signals
                    .index
                    .entry(source.clone())
                    .or_default()
                    .insert(id.clone());
                if let Some((device, _)) = source.split() {
                    signals
                        .devices
                        .entry(device)
                        .or_default()
                        .insert(id.clone());
                }
            }
        }
        signals
            .observed
            .retain(|point, _| signals.index.contains_key(point));
        signals.dirty = definitions.keys().cloned().collect();
        engine.install(definitions, order, &signals.index);
        self.active
            .store(!engine.order.is_empty(), Ordering::Release);
        Ok(())
    }
    pub(crate) fn active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }
    pub(crate) fn fence(&self) {
        self.active.store(false, Ordering::Release);
        *self.engine.lock().unwrap_or_else(PoisonError::into_inner) = Engine::default();
        *self.signals.lock().unwrap_or_else(PoisonError::into_inner) = Signals::default();
    }
}
