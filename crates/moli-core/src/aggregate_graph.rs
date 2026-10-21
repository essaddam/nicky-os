//! Stable bounded ordering of current calculated definitions.
use crate::{AggregateSpec, DeviceId, unit_ratio};
use std::collections::{BTreeMap, BTreeSet};
pub const MAX_CALCULATIONS: usize = 128;
pub const MAX_CALCULATION_DEPTH: usize = 16;
pub const MAX_PROPAGATION_WORK: usize = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AggregateGraphError {
    Cycle,
    Depth,
    Units,
    Capacity,
    Definition,
}

pub fn aggregate_order(
    definitions: &BTreeMap<DeviceId, AggregateSpec>,
) -> Result<Vec<DeviceId>, AggregateGraphError> {
    if definitions.len() > MAX_CALCULATIONS {
        return Err(AggregateGraphError::Capacity);
    }
    let mut incoming: BTreeMap<DeviceId, usize> =
        definitions.keys().map(|id| (id.clone(), 0)).collect();
    let mut children: BTreeMap<DeviceId, BTreeSet<DeviceId>> = BTreeMap::new();
    for (id, spec) in definitions {
        spec.clone()
            .normalize()
            .map_err(|_| AggregateGraphError::Definition)?;
        for point in &spec.sources {
            if let Some((parent, "value")) = point.split()
                && let Some(source) = definitions.get(&parent)
            {
                if unit_ratio(source.unit.as_ref(), spec.unit.as_ref()).is_none() {
                    return Err(AggregateGraphError::Units);
                }
                if children.entry(parent).or_default().insert(id.clone()) {
                    *incoming.get_mut(id).unwrap() += 1;
                }
            }
        }
    }
    let mut ready: BTreeSet<_> = incoming
        .iter()
        .filter(|(_, n)| **n == 0)
        .map(|(id, _)| id.clone())
        .collect();
    let mut depth: BTreeMap<DeviceId, usize> =
        definitions.keys().map(|id| (id.clone(), 1)).collect();
    let mut result = Vec::with_capacity(definitions.len());
    while let Some(id) = ready.pop_first() {
        if depth[&id] > MAX_CALCULATION_DEPTH {
            return Err(AggregateGraphError::Depth);
        }
        for child in children.get(&id).into_iter().flatten() {
            let next = depth[&id] + 1;
            depth
                .entry(child.clone())
                .and_modify(|old| *old = (*old).max(next));
            let pending = incoming.get_mut(child).unwrap();
            *pending -= 1;
            if *pending == 0 {
                ready.insert(child.clone());
            }
        }
        result.push(id);
    }
    if result.len() != definitions.len() {
        return Err(AggregateGraphError::Cycle);
    }
    Ok(result)
}
