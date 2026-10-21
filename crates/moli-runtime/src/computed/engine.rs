//! Coherent waves, stable order and monotonic observation ages.
use super::Signals;
use moli_core::{
    AggregateSource, AggregateSpec, DeviceId, MAX_FRESHNESS_S, MAX_PROPAGATION_WORK, PointId,
    Sample, Unit,
};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

const CLOCK_OFFSET: u64 = MAX_FRESHNESS_S * 1000 + 1;

#[derive(Clone, Debug)]
struct Anchor {
    ts: u64,
    born: Option<u64>,
    unit: Option<Unit>,
}

#[derive(Debug)]
pub(crate) struct Engine {
    pub order: Vec<DeviceId>,
    definitions: BTreeMap<DeviceId, AggregateSpec>,
    started: Instant,
    anchors: BTreeMap<PointId, Anchor>,
    provenance: BTreeMap<DeviceId, Option<u64>>,
    deadlines: BTreeSet<(u64, DeviceId)>,
    future: BTreeSet<(u64, DeviceId)>,
    wakes: BTreeMap<DeviceId, (Option<u64>, Option<u64>)>,
    rank: BTreeMap<DeviceId, usize>,
    pub last_work: usize,
}
impl Default for Engine {
    fn default() -> Self {
        Self {
            order: Vec::new(),
            definitions: BTreeMap::new(),
            started: Instant::now(),
            anchors: BTreeMap::new(),
            provenance: BTreeMap::new(),
            deadlines: BTreeSet::new(),
            future: BTreeSet::new(),
            wakes: BTreeMap::new(),
            rank: BTreeMap::new(),
            last_work: 0,
        }
    }
}

pub(crate) trait Projection {
    fn source(&self, point: &PointId) -> AggregateSource;
    fn project(&mut self, evaluation: &Evaluation);
}

#[derive(Debug)]
pub(crate) struct Evaluation {
    pub id: DeviceId,
    pub sample: Option<Sample>,
    pub unit: Option<Unit>,
    pub valid: usize,
}

impl Engine {
    pub(crate) fn install(
        &mut self,
        definitions: BTreeMap<DeviceId, AggregateSpec>,
        order: Vec<DeviceId>,
        index: &BTreeMap<PointId, BTreeSet<DeviceId>>,
    ) {
        self.definitions = definitions;
        self.rank = order
            .iter()
            .enumerate()
            .map(|(rank, id)| (id.clone(), rank))
            .collect();
        self.order = order;
        self.wakes.clear();
        self.deadlines.clear();
        self.future.clear();
        self.provenance
            .retain(|id, _| self.definitions.contains_key(id));
        self.anchors.retain(|point, _| index.contains_key(point));
    }
    fn now(&self, at: Instant) -> u64 {
        CLOCK_OFFSET.saturating_add(
            u64::try_from(at.saturating_duration_since(self.started).as_millis())
                .unwrap_or(u64::MAX),
        )
    }
    pub(crate) fn next(&self, at: Instant, wall: u64) -> Option<Duration> {
        let mono = self.now(at);
        let duration = self
            .deadlines
            .first()
            .map(|(deadline, _)| Duration::from_millis(deadline.saturating_sub(mono)));
        let future = self
            .future
            .first()
            .map(|(ts, _)| Duration::from_millis(ts.saturating_sub(wall)));
        match (duration, future) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }
    pub(crate) fn propagate(
        &mut self,
        signals: &mut Signals,
        at: Instant,
        wall: u64,
        projection: &mut impl Projection,
    ) -> Result<Vec<Evaluation>, crate::DependencyError> {
        let now = self.now(at);
        for device in std::mem::take(&mut signals.removed) {
            self.anchors
                .retain(|point, _| point.split().is_none_or(|(id, _)| id != device));
        }
        let mut dirty = std::mem::take(&mut signals.dirty);
        for (deadline, id) in &self.deadlines {
            if *deadline > now {
                break;
            }
            dirty.insert(id.clone());
        }
        for (ts, id) in &self.future {
            if *ts > wall {
                break;
            }
            dirty.insert(id.clone());
        }
        let mut pending: Vec<_> = dirty.iter().cloned().collect();
        let mut work = 0;
        while let Some(id) = pending.pop() {
            if let Some(children) = signals.index.get(&PointId::new(&id, "value")) {
                for child in children {
                    work += 1;
                    if dirty.insert(child.clone()) {
                        pending.push(child.clone());
                    }
                }
            }
        }
        let mut result = Vec::new();
        let mut visited: Vec<_> = dirty
            .into_iter()
            .filter(|id| self.rank.contains_key(id))
            .collect();
        visited.sort_by_key(|id| self.rank[id]);
        for id in visited {
            let spec = self.definitions[&id].clone();
            work += spec.sources.len();
            if work > MAX_PROPAGATION_WORK {
                return Err(crate::DependencyError::Budget);
            }
            let mut actual = Vec::with_capacity(spec.sources.len());
            let mut inputs = Vec::with_capacity(spec.sources.len());
            let mut future = None;
            for point in &spec.sources {
                let input = projection.source(point);
                let original = input.sample.as_ref().map(|s| s.ts);
                let born = self.birth(point, &input, signals, at, wall);
                if born.is_none()
                    && let Some(ts) = original
                    && ts > wall
                {
                    future = Some(future.map_or(ts, |old: u64| old.min(ts)));
                }
                actual.push(original);
                inputs.push(AggregateSource {
                    sample: input
                        .sample
                        .and_then(|sample| born.map(|ts| Sample { ts, ..sample })),
                    ..input
                });
            }
            let computed = spec.evaluate(&inputs, now);
            self.set_wake(&id, computed.next, future);
            self.provenance
                .insert(id.clone(), computed.sample.as_ref().map(|s| s.ts));
            let source_ts = actual
                .iter()
                .enumerate()
                .filter(|(index, _)| computed.used & (1_u32 << index) != 0)
                .filter_map(|(_, ts)| *ts)
                .min();
            let evaluation = Evaluation {
                id,
                sample: computed.sample.map(|s| Sample {
                    ts: source_ts.expect("valid input has a timestamp"),
                    ..s
                }),
                unit: spec.unit,
                valid: computed.valid,
            };
            projection.project(&evaluation);
            result.push(evaluation);
        }
        self.last_work = work;
        Ok(result)
    }
    fn set_wake(&mut self, id: &DeviceId, mono: Option<u64>, wall: Option<u64>) {
        if let Some((old_mono, old_wall)) = self.wakes.remove(id) {
            if let Some(deadline) = old_mono {
                self.deadlines.remove(&(deadline, id.clone()));
            }
            if let Some(deadline) = old_wall {
                self.future.remove(&(deadline, id.clone()));
            }
        }
        if let Some(deadline) = mono {
            self.deadlines.insert((deadline, id.clone()));
        }
        if let Some(deadline) = wall {
            self.future.insert((deadline, id.clone()));
        }
        if mono.is_some() || wall.is_some() {
            self.wakes.insert(id.clone(), (mono, wall));
        }
    }
    fn birth(
        &mut self,
        point: &PointId,
        input: &AggregateSource,
        signals: &Signals,
        at: Instant,
        wall: u64,
    ) -> Option<u64> {
        let sample = input.sample.as_ref()?;
        if let Some((id, "value")) = point.split()
            && self.definitions.contains_key(&id)
        {
            return self.provenance.get(&id).copied().flatten();
        }
        if let Some(observed) = signals.observed.get(point)
            && observed.ts == sample.ts
        {
            return (observed.unit == input.unit).then(|| self.now(observed.at));
        }
        let now = self.now(at);
        let anchor = self.anchors.entry(point.clone()).or_insert(Anchor {
            ts: sample.ts,
            born: None,
            unit: input.unit.clone(),
        });
        if anchor.ts != sample.ts {
            anchor.ts = sample.ts;
            anchor.born = None;
            anchor.unit.clone_from(&input.unit);
        }
        if anchor.unit != input.unit {
            return None;
        }
        if anchor.born.is_none() && sample.ts <= wall {
            anchor.born =
                Some(now.saturating_sub(wall.saturating_sub(sample.ts).min(CLOCK_OFFSET)));
        }
        anchor.born
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
