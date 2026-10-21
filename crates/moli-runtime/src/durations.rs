//! Synchronous indexed observation capture; one asynchronous durable checkpoint writer.
use crate::StateFile;
use moli_core::{
    DeviceId, DurationLedger, DurationSpec, DurationStats, Kind, PointId, Unit, Value,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{
    Mutex, PoisonError,
    atomic::{AtomicBool, Ordering},
};
use std::time::Instant;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Record {
    pub version: u64,
    pub spec: DurationSpec,
    pub ledger: DurationLedger,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct State {
    pub schema: u32,
    pub records: BTreeMap<DeviceId, Record>,
}
impl State {
    pub fn valid(&self) -> bool {
        self.schema == 1
            && self.records.len() <= 128
            && self.records.iter().all(|(id, r)| {
                crate::counters::valid_device(id)
                    && (1..=moli_core::MAX_DEFINITION_VERSION).contains(&r.version)
                    && r.spec.clone().normalize().is_ok_and(|s| s == r.spec)
                    && r.ledger.valid()
            })
    }
}
#[derive(Debug)]
pub(crate) struct Durable {
    pub file: StateFile,
    pub loaded: bool,
}
#[derive(Debug)]
pub(crate) struct Book {
    pub records: BTreeMap<DeviceId, Record>,
    pub index: BTreeMap<PointId, BTreeSet<DeviceId>>,
    pub devices: BTreeMap<DeviceId, BTreeSet<DeviceId>>,
    pub active: BTreeSet<DeviceId>,
    pub committed: BTreeMap<DeviceId, DurationStats>,
    pub epoch: u64,
    pub fenced: bool,
    pub anchor: Instant,
    pub wall: u64,
}
impl Default for Book {
    fn default() -> Self {
        Self {
            records: BTreeMap::new(),
            index: BTreeMap::new(),
            devices: BTreeMap::new(),
            active: BTreeSet::new(),
            committed: BTreeMap::new(),
            epoch: 0,
            fenced: false,
            anchor: Instant::now(),
            wall: moli_core::now_ms(),
        }
    }
}
impl Book {
    pub fn now(&self, at: Instant) -> u64 {
        self.wall.saturating_add(
            u64::try_from(at.saturating_duration_since(self.anchor).as_millis())
                .unwrap_or(u64::MAX),
        )
    }
    pub fn advance(&mut self, now: u64) {
        for id in &self.active {
            let record = self.records.get_mut(id).expect("active duration exists");
            record.ledger.advance(now);
            record.ledger.prune(record.spec.window_s * 1000);
        }
    }
    pub fn state(&self) -> State {
        State {
            schema: 1,
            records: self.records.clone(),
        }
    }
}
#[derive(Debug)]
pub(crate) struct Durations {
    pub book: Mutex<Book>,
    pub writer: tokio::sync::Mutex<Durable>,
    pub active: AtomicBool,
    pub started: AtomicBool,
}
impl Durations {
    pub fn new(path: Option<std::path::PathBuf>) -> Self {
        Self {
            book: Mutex::default(),
            writer: tokio::sync::Mutex::new(Durable {
                file: StateFile::new(path, 4 * 1024 * 1024),
                loaded: false,
            }),
            active: AtomicBool::new(false),
            started: AtomicBool::new(false),
        }
    }
    /// Called while the model is coherent. No IO, queue or task per event.
    #[allow(clippy::too_many_arguments)]
    pub fn sample(
        &self,
        point: &PointId,
        value: &Value,
        kind: &Kind,
        unit: Option<&Unit>,
        online: bool,
        at: Instant,
        source_at: Option<Instant>,
    ) {
        if !self.active.load(Ordering::Acquire) {
            return;
        }
        let mut book = self.book.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(roots) = book.index.get(point).cloned() else {
            return;
        };
        let now = book.now(at);
        for id in roots {
            let record = book.records.get_mut(&id).expect("indexed duration exists");
            let remaining = source_at
                .and_then(|observed| {
                    observed.checked_add(std::time::Duration::from_secs(record.spec.max_age_s))
                })
                .map_or(0, |expires| {
                    u64::try_from(expires.saturating_duration_since(at).as_millis()).unwrap_or(0)
                });
            let normalize = |value: &Value| {
                if *kind == Kind::Computed {
                    value.as_f64().filter(|n| n.is_finite()).map(Value::Float)
                } else {
                    crate::validate_point_value(kind, value).ok()
                }
            };
            let matched = (online && remaining > 0 && unit == record.spec.source_unit.as_ref())
                .then(|| {
                    normalize(value)
                        .zip(normalize(&Value::from_json(&record.spec.value)))
                        .map(|(a, b)| a == b)
                })
                .flatten();
            record.ledger.observe(now, matched, remaining);
        }
    }
    pub fn invalidate(&self, id: &DeviceId) {
        if !self.active.load(Ordering::Acquire) {
            return;
        }
        let mut book = self.book.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(roots) = book.devices.get(id).cloned() else {
            return;
        };
        let now = book.now(Instant::now());
        for id in roots {
            book.records
                .get_mut(&id)
                .unwrap()
                .ledger
                .observe(now, None, 0);
        }
    }
}
