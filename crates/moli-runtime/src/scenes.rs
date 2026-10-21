//! Ephemeral scene runs. Definitions use the shared durable dependency owner.
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use moli_core::{Device, DeviceId, Label, PointId, SceneSpec, Value};
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneScope {
    pub targets: Vec<SceneScopeTarget>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneScopeTarget {
    pub point: PointId,
    pub value: serde_json::Value,
    pub name: String,
    pub native_name: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<moli_core::Unit>,
    pub devices: Vec<DeviceId>,
    pub rooms: Vec<String>,
    /// Binds the exact point specs, membership and labels the person saw.
    pub fingerprint: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenePreview {
    pub id: DeviceId,
    pub version: u64,
    pub scope: SceneScope,
    pub fingerprint: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SceneStatus {
    Running,
    Done,
    Partial,
    Cancelled,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetStatus {
    Waiting,
    Held,
    Acknowledged,
    Confirmed,
    AlreadyConform,
    Failed,
    ConfirmationExpired,
    Superseded,
    Cancelled,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SceneTargetResult {
    pub point: PointId,
    pub status: TargetStatus,
    pub acknowledged: bool,
    pub approval: Option<u64>,
    pub detail: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SceneRun {
    pub id: String,
    pub scene: DeviceId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_correlation: Option<String>,
    pub version: u64,
    pub status: SceneStatus,
    pub started: u64,
    pub ended: Option<u64>,
    pub targets: Vec<SceneTargetResult>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneError {
    Definition,
    Unknown,
    Conflict,
    Busy,
    Storage,
}
impl std::fmt::Display for SceneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&moli_i18n::tr!(match self {
            Self::Definition => "serveur.scene.definition",
            Self::Unknown => "serveur.scene.inconnue",
            Self::Conflict => "serveur.scene.conflit",
            Self::Busy => "serveur.scene.occupee",
            Self::Storage => "serveur.scene.stockage",
        }))
    }
}
impl std::error::Error for SceneError {}

pub(crate) struct WatchState {
    pub superseded: bool,
    pub admitted: Option<Instant>,
    pub observed: Option<(Value, Instant)>,
}
#[derive(Clone)]
pub(crate) struct Watch {
    pub state: Arc<Mutex<WatchState>>,
    pub wake: Arc<Notify>,
    pub cancel: CancellationToken,
    pub devices: BTreeSet<DeviceId>,
    pub point: PointId,
    pub correlation: String,
}
impl std::fmt::Debug for Watch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SceneWatch")
    }
}
impl Watch {
    pub fn state(&self) -> std::sync::MutexGuard<'_, WatchState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
    pub fn wanted(&self) -> bool {
        !self.cancel.is_cancelled() && !self.state().superseded
    }
}
#[derive(Debug)]
pub(crate) struct Active {
    pub run: SceneRun,
    pub cancel: CancellationToken,
    pub watches: Vec<Watch>,
}
#[derive(Debug, Default)]
pub(crate) struct Scenes {
    pub active: BTreeMap<String, Active>,
    pub ended: VecDeque<SceneRun>,
    points: BTreeMap<PointId, Vec<Watch>>,
    leases: BTreeMap<DeviceId, Vec<Lease>>,
}
#[derive(Debug)]
struct Lease {
    state: std::sync::Weak<Mutex<WatchState>>,
    wake: std::sync::Weak<Notify>,
    correlation: String,
    deadline: Instant,
}
impl Scenes {
    pub fn reindex(&mut self) {
        self.points.clear();

        for watch in self.active.values().flat_map(|a| &a.watches) {
            self.points
                .entry(watch.point.clone())
                .or_default()
                .push(watch.clone());
        }
    }
    pub fn observe(&self, point: &PointId, value: &Value, at: Instant) {
        for watch in self.points.get(point).into_iter().flatten() {
            let mut state = watch.state();
            if state.admitted.is_some_and(|admitted| at >= admitted) {
                state.observed = Some((value.clone(), at));
                watch.wake.notify_one();
            }
        }
    }
    fn prune(&mut self) {
        let now = Instant::now();
        self.leases.retain(|_, leases| {
            leases.retain(|lease| {
                lease.deadline > now
                    && lease.state.upgrade().is_some_and(|state| {
                        !state
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .superseded
                    })
            });
            !leases.is_empty()
        });
    }
    pub fn admit(&mut self, watches: &[Watch], deadline: Instant) -> Result<(), SceneError> {
        self.prune();
        let devices: BTreeSet<_> = watches
            .iter()
            .flat_map(|w| w.devices.iter().cloned())
            .collect();
        if self
            .leases
            .keys()
            .cloned()
            .chain(devices.iter().cloned())
            .collect::<BTreeSet<_>>()
            .len()
            > 512
        {
            return Err(SceneError::Busy);
        }
        // A new admitted scene supersedes previous completed continuations.
        self.command(&devices.iter().cloned().collect::<Vec<_>>(), None);
        self.prune();
        for watch in watches {
            for device in &watch.devices {
                self.leases.entry(device.clone()).or_default().push(Lease {
                    state: Arc::downgrade(&watch.state),
                    wake: Arc::downgrade(&watch.wake),
                    correlation: watch.correlation.clone(),
                    deadline,
                });
            }
        }
        Ok(())
    }
    pub fn target_admitted(&self, current: &Watch) {
        if let Some(active) = self.active.get(&current.correlation) {
            for previous in &active.watches {
                if previous.point != current.point
                    && !previous.devices.is_disjoint(&current.devices)
                    && previous.state().admitted.is_some()
                {
                    previous.state().superseded = true;
                    previous.wake.notify_one();
                }
            }
        }
        current.state().admitted = Some(Instant::now());
    }
    pub fn fence(&mut self) {
        for active in self.active.values() {
            active.cancel.cancel();
        }
        for lease in self.leases.values().flatten() {
            Self::invalidate(lease);
        }
        self.prune();
    }
    pub fn has_leases(&self) -> bool {
        !self.leases.is_empty()
    }
    pub fn revoke(&mut self, correlation: &str) {
        for lease in self
            .leases
            .values()
            .flatten()
            .filter(|l| l.correlation == correlation)
        {
            Self::invalidate(lease);
        }
        self.prune();
    }
    fn invalidate(lease: &Lease) {
        if let Some(state) = lease.state.upgrade() {
            state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .superseded = true;
        }
        if let Some(wake) = lease.wake.upgrade() {
            wake.notify_one();
        }
    }
    pub fn command(&mut self, devices: &[DeviceId], correlation: Option<&str>) {
        let now = Instant::now();
        for device in devices {
            let empty = if let Some(leases) = self.leases.get_mut(device) {
                leases.retain(|lease| lease.deadline > now && lease.state.strong_count() > 0);
                for lease in leases.iter() {
                    if correlation != Some(lease.correlation.as_str()) {
                        Self::invalidate(lease);
                    }
                }
                leases.retain(|lease| {
                    lease.state.upgrade().is_some_and(|state| {
                        !state
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .superseded
                    })
                });
                leases.is_empty()
            } else {
                false
            };
            if empty {
                self.leases.remove(device);
            }
        }
    }
    pub fn get(&self, id: &str) -> Option<SceneRun> {
        self.active
            .get(id)
            .map(|a| a.run.clone())
            .or_else(|| self.ended.iter().find(|r| r.id == id).cloned())
    }
}

/// Immutable scope snapshot used to validate all targets before any is sent.
pub(crate) fn scope_target(
    point: &PointId,
    value: &Value,
    descriptors: &[(Arc<Device>, Label)],
    rooms: Vec<String>,
    spec: &moli_core::PointSpec,
) -> SceneScopeTarget {
    SceneScopeTarget {
        point: point.clone(),
        value: value.to_json(),
        name: descriptors[0]
            .1
            .name
            .as_deref()
            .unwrap_or(&descriptors[0].0.native_name)
            .chars()
            .take(240)
            .collect(),
        native_name: descriptors[0].0.native_name.chars().take(240).collect(),
        label: spec.label.chars().take(240).collect(),
        unit: spec.unit.clone(),
        devices: descriptors.iter().map(|(d, _)| d.id.clone()).collect(),
        rooms,
        fingerprint: crate::dependencies::fingerprint(&descriptors),
    }
}

/// Connected scopes share a sequential lane. Stable target order is retained
/// even for a composite and a direct member in the same desired state.
pub(crate) fn lanes(scope: &SceneScope) -> Vec<Vec<usize>> {
    let mut lanes: Vec<(BTreeSet<DeviceId>, Vec<usize>)> = Vec::new();
    for (i, target) in scope.targets.iter().enumerate() {
        let mut devices: BTreeSet<_> = target.devices.iter().cloned().collect();
        let mut indices = vec![i];
        let mut cursor = 0;
        while cursor < lanes.len() {
            if lanes[cursor].0.is_disjoint(&devices) {
                cursor += 1;
            } else {
                let (found, ids) = lanes.remove(cursor);
                devices.extend(found);
                indices.extend(ids);
                cursor = 0;
            }
        }
        indices.sort_unstable();
        lanes.push((devices, indices));
    }
    lanes.sort_by_key(|(_, ids)| ids[0]);
    lanes.into_iter().map(|(_, ids)| ids).collect()
}

pub(crate) fn deadline(spec: &SceneSpec) -> Instant {
    Instant::now() + Duration::from_secs(u64::from(spec.timeout_s))
}

pub(crate) fn watches(
    scope: &SceneScope,
    correlation: &str,
    cancel: &CancellationToken,
) -> Vec<Watch> {
    scope
        .targets
        .iter()
        .map(|t| Watch {
            state: Arc::new(Mutex::new(WatchState {
                superseded: false,
                admitted: None,
                observed: None,
            })),
            wake: Arc::default(),
            cancel: cancel.clone(),
            devices: t.devices.iter().cloned().collect(),
            point: t.point.clone(),
            correlation: correlation.to_owned(),
        })
        .collect()
}

impl SceneScope {
    pub(crate) fn valid_for(&self, spec: &SceneSpec) -> bool {
        self.targets.len() == spec.targets.len()
            && self
                .targets
                .iter()
                .zip(&spec.targets)
                .all(|(scope, target)| {
                    scope.point == target.point
                        && !scope.devices.is_empty()
                        && scope
                            .point
                            .split()
                            .is_some_and(|(root, _)| scope.devices.first() == Some(&root))
                        && scope.devices.iter().collect::<BTreeSet<_>>().len()
                            == scope.devices.len()
                        && scope.fingerprint.len() == 64
                        && scope.fingerprint.bytes().all(|b| b.is_ascii_hexdigit())
                        && scope.rooms.iter().all(|r| r.len() <= 240)
                })
            && self.targets.iter().map(|t| t.devices.len()).sum::<usize>() <= 128
            && serde_json::to_vec(self).is_ok_and(|bytes| bytes.len() <= 64 * 1024)
    }
}
