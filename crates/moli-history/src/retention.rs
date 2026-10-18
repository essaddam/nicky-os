//! How long each point is kept, and the guard on shorter durations.
//!
//! Each point has a floor in the database (table `retention`): how long it
//! has been kept so far. A pass keeps it the longer of its floor and its
//! configured duration, and raises the floor as soon as the configuration
//! asks for longer — remembered at once, whatever else is waiting. A
//! configured duration below the floor deletes nothing by itself:
//! `policies` counts, point by point, what it would delete and keeps that
//! plan; a person validates exactly that plan (`validate`), which lowers
//! those floors and deletes those rows — no more — under the lock the
//! passes take. A point whose device is not known (not published yet after
//! a start, or gone) is kept its longest possible duration and enters no
//! reduction.
//!
//! Floors remember certain durations observed since the last consent, as
//! well as what is certain at writing (a device published, a row, a pass).
//! If a raise cannot be written, purges are suspended — durably — until a
//! person has seen what resuming them deletes and validated it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use moli_core::{Device, Origin, PointId, now_ms};
use moli_runtime::Hub;
use serde::Serialize;

use crate::policy::{Family, Filter, Retention};
use crate::store::{
    self, Cut, Doomed, Floors, Guard, HistoryError, Observations, Observed, Store, Validation,
};
use crate::{DAY_MS, History, Prune, blocking};

/// Rows one purge statement deletes at most…
const PRUNE_LOT: usize = 5_000;
/// …and the breath the writer is given between two of them.
const PRUNE_PAUSE: Duration = Duration::from_millis(2);
/// Tries of a floors raise before purges are suspended…
const RAISE_TRIES: u32 = 3;
/// …waiting this long (doubled each time) between them.
const RAISE_BACKOFF: Duration = Duration::from_millis(100);
/// How long an announced reduction may be validated.
const REDUCTION_TTL_MS: u64 = 15 * 60_000;
/// Points listed with an announced reduction.
const REDUCTION_TOP: usize = 10;

/// A point's family, `None` while its device is unknown.
pub(crate) type Families = Box<dyn Fn(&str) -> Option<Family> + Send + Sync>;

/// A point's family, as the hub describes its device right now.
pub(crate) fn family_in(hub: &Hub, point: &str) -> Option<Family> {
    let point = PointId::from(point);
    let (device, key) = point.split()?;
    let (device, _) = hub.describe(&device)?;
    device.point(key).map(Family::of)
}

pub(crate) struct Policy {
    pub(crate) filter: Filter,
    configured: Retention,
    families: Families,
    /// The last reduction shown: the only one a validation applies.
    shown: Mutex<Option<Plan>>,
    /// One at a time: a pass, a count, a validation.
    passes: tokio::sync::Mutex<()>,
    raises: Raises,
    observed: Observations,
    /// Orders consent against events emitted, including those still queued.
    hub: Option<Hub>,
    /// Serializes observations with the commit of a newer consent.
    consents: Mutex<HashMap<String, Consent>>,
}

struct Consent {
    generation: i64,
    through: u64,
}

/// Certain durations observed since each point's last consent. One entry per
/// point, so bounded by the points there are; written by one task
/// (`raise_loop`) and drained before the history stops.
#[derive(Default)]
struct Raises {
    pending: Mutex<HashMap<String, Observed>>,
    wake: tokio::sync::Notify,
    closed: AtomicBool,
}

impl std::fmt::Debug for Policy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Policy")
            .field("configured", &self.configured)
            .finish_non_exhaustive()
    }
}

/// A reduction as shown: the points it shortens, each with what it
/// deletes; while purges are suspended, every point, with what resuming
/// them deletes.
#[derive(Clone, Debug)]
struct Plan {
    at: u64,
    fingerprint: String,
    resume: bool,
    generation: i64,
    items: Vec<Item>,
}

#[derive(Clone, Debug, Hash)]
struct Item {
    id: i64,
    point: String,
    family: Option<Family>,
    /// Its configured duration when counted.
    configured: u32,
    /// Its floor so far…
    floor: u32,
    /// …and once validated.
    days: u32,
    /// What it is purged at: rows before `doomed.anchor` go.
    keep: u32,
    doomed: Option<Doomed>,
}

/// A shorter duration waiting for a person: what it deletes, exactly.
#[derive(Clone, Debug, Serialize)]
pub struct Reduction {
    /// When it was counted.
    pub at: u64,
    /// This plan; a validation names it.
    pub fingerprint: String,
    pub points: usize,
    pub rows: u64,
    /// Purges are suspended (a failed raise or unfinished run): validating
    /// resumes them, deleting what is counted here.
    pub resume: bool,
    /// The oldest row concerned, and the latest instant before which rows go.
    pub from: Option<u64>,
    pub to: Option<u64>,
    /// The points losing the most rows, most first.
    pub top: Vec<Reduced>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Reduced {
    pub point: String,
    pub family: Option<Family>,
    pub from_days: u32,
    pub to_days: u32,
    pub rows: u64,
}

/// What the history keeps (`GET /api/history/retention`).
#[derive(Clone, Debug, Serialize)]
pub struct Policies {
    pub filter: Filter,
    pub configured: Retention,
    pub last_validation: Option<Validation>,
    pub reduction: Option<Reduction>,
}

#[derive(Debug, thiserror::Error)]
pub enum PolicyError {
    /// Only a person (PIN session) validates: never an agent.
    #[error("only a person validates a reduction")]
    HumanOnly,
    #[error("no reduction is waiting")]
    Nothing,
    #[error("the reduction shown is no longer the one waiting")]
    Changed,
    #[error("the reduction shown is too old: count it again")]
    Expired,
    #[error(transparent)]
    Store(#[from] HistoryError),
}

fn cutoff(now: u64, days: u32) -> u64 {
    now.saturating_sub(u64::from(days) * DAY_MS)
}

/// One recorded point, as a pass sees it.
struct Seen {
    id: i64,
    point: String,
    family: Option<Family>,
    /// Its floor; `None`: a point new since the floors were set up.
    floor: Option<u32>,
}

impl Policy {
    /// The first time, every point recorded so far gets the duration Moli
    /// kept before any policy (`[server] history_days`).
    pub(crate) fn open(
        store: &Store,
        filter: Filter,
        configured: Retention,
        families: Families,
        hub: Option<Hub>,
    ) -> Result<Self, HistoryError> {
        if store.floors()?.is_none() {
            store.set_up_floors(configured.days, now_ms())?;
        }
        let suspended = store.suspended()?;
        if suspended {
            tracing::warn!("history purges suspended until a person validates them");
        }
        Ok(Self {
            filter,
            configured,
            families,
            shown: Mutex::new(None),
            passes: tokio::sync::Mutex::new(()),
            raises: Raises::default(),
            observed: store.observations(),
            hub,
            consents: Mutex::new(
                store
                    .consents()?
                    .into_iter()
                    .map(|(point, generation)| {
                        (
                            point,
                            Consent {
                                generation,
                                through: 0,
                            },
                        )
                    })
                    .collect(),
            ),
        })
    }

    /// Capture and admit a fresh snapshot under the consent lock. A snapshot
    /// taken before a consent cannot be stamped with its newer generation.
    pub(crate) fn snapshot(&self, hub: &Hub) -> moli_runtime::Snapshot {
        let consents = store::lock(&self.consents);
        let snapshot = hub.snapshot();
        for device in &snapshot.devices {
            self.admit(&device.device, None, &consents);
        }
        self.raises.wake.notify_one();
        snapshot
    }

    pub(crate) fn published_at(&self, device: &Device, position: Option<u64>) {
        let consents = store::lock(&self.consents);
        self.admit(device, position, &consents);
        self.raises.wake.notify_one();
    }

    fn admit(&self, device: &Device, position: Option<u64>, consents: &HashMap<String, Consent>) {
        // Same gate as each DELETE, held until admission is complete.
        let mut admitted = store::lock(&self.observed);
        let mut pending = store::lock(&self.raises.pending);
        for spec in &device.points {
            let point = PointId::new(&device.id, &spec.key).as_str().to_owned();
            let consent = consents.get(&point);
            if consent.is_some_and(|c| position.is_some_and(|p| p <= c.through)) {
                continue;
            }
            let observed = Observed {
                days: self.certain(&point, Some(Family::of(spec))),
                generation: consent.map_or(0, |c| c.generation),
            };
            merge_raise(&mut admitted, point.clone(), observed);
            merge_raise(&mut pending, point, observed);
        }
    }

    /// No more devices: `raise_loop` writes what is queued, then ends.
    pub(crate) fn stop_raising(&self) {
        self.raises.closed.store(true, SeqCst);
        self.raises.wake.notify_one();
    }

    fn seen(&self, store: &Store) -> Result<Vec<Seen>, HistoryError> {
        let floors = store.floors()?.unwrap_or_default();
        Ok(store
            .points()?
            .into_iter()
            .map(|(id, point)| Seen {
                id,
                family: (self.families)(&point),
                floor: floors.get(&id).copied(),
                point,
            })
            .collect())
    }

    /// The configured duration; for an unknown family, the longest any
    /// family could give it.
    fn configured(&self, point: &str, family: Option<Family>) -> u32 {
        match family {
            Some(f) => self.configured.days(point, f),
            None => Family::ALL
                .iter()
                .map(|&f| self.configured.days(point, f))
                .max()
                .unwrap_or(self.configured.days),
        }
    }

    /// What the configuration surely asks for: the configured duration;
    /// for an unknown family, the shortest any family could give it (a
    /// point's own rule gives the same to all).
    fn certain(&self, point: &str, family: Option<Family>) -> u32 {
        match family {
            Some(f) => self.configured.days(point, f),
            None => Family::ALL
                .iter()
                .map(|&f| self.configured.days(point, f))
                .min()
                .unwrap_or(self.configured.days),
        }
    }

    /// A point's floors when one of its rows is written (see
    /// `Store::record`): its longest duration if it is new, and what the
    /// configuration surely asks, kept as soon as its family is known.
    pub(crate) fn floors(&self, point: &str) -> Floors {
        let family = (self.families)(point);
        Floors {
            first: self.configured(point, family),
            certain: self.certain(point, family),
        }
    }

    /// Keep pending (including in-flight) observations visible to counts
    /// and passes until their transaction commits. No deletion can race
    /// ahead of a certain increase already received from the bus.
    fn raise_pending(&self, store: &Store) -> Result<(), HistoryError> {
        let pending = store::lock(&self.raises.pending)
            .clone()
            .into_iter()
            .collect::<Vec<_>>();
        store.raise_named_floors(&pending, &|p| self.floors(p).certain)
    }

    /// A pass as of `now`: floors raised to what the configuration surely
    /// asks for (a point without one starts at its longest duration), then
    /// every point purged at the longer of its floor and its longest
    /// configured duration. An unknown family raises no floor on a mere
    /// ambiguity: a validated floor survives restarts.
    fn pass(&self, store: &Store, now: u64) -> Result<usize, HistoryError> {
        let guard = store.guard()?;
        if guard.suspended {
            return Ok(0);
        }
        self.raise_pending(store)?;
        let mut raised = Vec::new();
        let mut cuts = Vec::new();
        for s in self.seen(store)? {
            let longest = self.configured(&s.point, s.family);
            let floor = s
                .floor
                .unwrap_or(longest)
                .max(self.certain(&s.point, s.family));
            if s.floor != Some(floor) {
                raised.push((s.id, floor));
            }
            let keep = floor.max(longest);
            if let Some(anchor) = store.anchor(s.id, cutoff(now, keep))? {
                cuts.push(Cut {
                    id: s.id,
                    oldest: 0,
                    anchor,
                    max: u64::MAX,
                    floor: keep,
                    generation: guard.generation,
                });
            }
        }
        store.raise_floors(&raised)?;
        store.prune(&cuts, PRUNE_LOT, || std::thread::sleep(PRUNE_PAUSE))
    }

    /// The points whose configured duration is below their floor, and what
    /// shortening each deletes as of `now`. While purges are suspended,
    /// every point with a floor, with what resuming purges deletes of it.
    fn plan(&self, store: &Store, now: u64) -> Result<Option<Plan>, HistoryError> {
        use std::hash::{Hash, Hasher};
        let guard = store.guard()?;
        let resume = guard.suspended;
        if !resume {
            self.raise_pending(store)?;
        }
        let mut items = Vec::new();
        for s in self.seen(store)? {
            let Some(floor) = s.floor else { continue };
            let configured = self.configured(&s.point, s.family);
            let (days, keep) = if s.family.is_some() && configured < floor {
                (configured, configured)
            } else if resume {
                let days = floor.max(self.certain(&s.point, s.family));
                (days, days.max(configured))
            } else {
                continue;
            };
            items.push(Item {
                doomed: store.doomed(s.id, cutoff(now, keep))?,
                id: s.id,
                point: s.point,
                family: s.family,
                configured,
                floor,
                days,
                keep,
            });
        }
        if items.is_empty() && !resume {
            return Ok(None);
        }
        let mut h = std::hash::DefaultHasher::new();
        (now, guard, &items).hash(&mut h);
        Ok(Some(Plan {
            at: now,
            fingerprint: format!("{:016x}", h.finish()),
            resume,
            generation: guard.generation,
            items,
        }))
    }

    /// Applies the plan shown, if each of its points still is what it was
    /// (floor, family, duration): their floors are lowered and exactly their
    /// rows deleted, within the period announced; a plan shown while purges
    /// were suspended resumes them. Returns how many.
    fn apply(&self, store: &Store, plan: &Plan, by: Option<&str>) -> Result<usize, PolicyError> {
        // Serialize the whole check and its commit with publication stamps.
        let mut consents = store::lock(&self.consents);
        // Events emitted before this point are replaced by this consent,
        // even if the recorder has not consumed their old descriptions yet.
        let through = self.hub.as_ref().map_or(0, Hub::event_position);
        if !plan.resume {
            self.raise_pending(store)?;
        }
        // Each point as it was counted: its floor, its family, its duration.
        let floors = store.floors()?.unwrap_or_default();
        let guard = Guard {
            generation: plan.generation,
            suspended: plan.resume,
        };
        if store.guard()? != guard
            || plan.items.iter().any(|i| {
                floors.get(&i.id) != Some(&i.floor)
                    || (self.families)(&i.point) != i.family
                    || self.configured(&i.point, i.family) != i.configured
            })
        {
            return Err(PolicyError::Changed);
        }
        let announced = plan
            .items
            .iter()
            .filter_map(|i| i.doomed)
            .map(|d| d.rows)
            .sum();
        // Lowered only from the floor counted: a write that raised one
        // meanwhile (its family just published) refuses the whole plan.
        let floors: Vec<_> = plan.items.iter().map(|i| (i.id, i.floor, i.days)).collect();
        let updates = store
            .validate_floors(&floors, now_ms(), by, announced, guard)?
            .ok_or(PolicyError::Changed)?;
        consents.extend(updates.into_iter().map(|(point, generation)| {
            (
                point,
                Consent {
                    generation,
                    through,
                },
            )
        }));
        drop(consents);
        let cuts: Vec<_> = plan
            .items
            .iter()
            .filter_map(|i| {
                i.doomed.map(|d| Cut {
                    id: i.id,
                    oldest: d.oldest.unwrap_or(0),
                    anchor: d.anchor,
                    max: d.rows,
                    floor: i.keep,
                    generation: plan.generation,
                })
            })
            .collect();
        Ok(store.prune(&cuts, PRUNE_LOT, || std::thread::sleep(PRUNE_PAUSE))?)
    }
}

fn merge_raise(pending: &mut HashMap<String, Observed>, point: String, observed: Observed) {
    pending
        .entry(point)
        .and_modify(|old| {
            if observed.generation > old.generation {
                *old = observed;
            } else if observed.generation == old.generation {
                old.days = old.days.max(observed.days);
            }
        })
        .or_insert(observed);
}

impl Plan {
    fn reduction(&self) -> Reduction {
        let mut top: Vec<Reduced> = self
            .items
            .iter()
            .map(|i| Reduced {
                point: i.point.clone(),
                family: i.family,
                from_days: i.floor,
                to_days: i.days,
                rows: i.doomed.map_or(0, |d| d.rows),
            })
            .collect();
        let doomed = || {
            self.items
                .iter()
                .filter(|i| i.doomed.is_some_and(|d| d.rows > 0))
        };
        let from = doomed()
            .filter_map(|i| i.doomed.and_then(|d| d.oldest))
            .min();
        let to = doomed().map(|i| cutoff(self.at, i.keep)).max();
        top.sort_by(|a, b| b.rows.cmp(&a.rows).then_with(|| a.point.cmp(&b.point)));
        let rows = top.iter().map(|r| r.rows).sum();
        let points = top.len();
        top.truncate(REDUCTION_TOP);
        Reduction {
            at: self.at,
            fingerprint: self.fingerprint.clone(),
            points,
            rows,
            resume: self.resume,
            from,
            to,
            top,
        }
    }
}

/// Raises the floors of the points queued by `Policy::published_at`, a batch
/// at a time, each from what is known when it is written, until
/// `stop_raising` and everything queued is written. A batch that cannot be
/// written after `RAISE_TRIES` tries stays queued and suspends purges,
/// durably, until a person validates them; the loop then ends in error once
/// stopped (the recorder's handle says so).
pub(crate) async fn raise_loop(store: Arc<Store>, policy: Arc<Policy>) -> Result<(), HistoryError> {
    let raises = &policy.raises;
    loop {
        let batch: Vec<(String, Observed)> =
            store::lock(&raises.pending).clone().into_iter().collect();
        if batch.is_empty() {
            if raises.closed.load(SeqCst) {
                if store::lock(&raises.pending).is_empty() {
                    return Ok(());
                }
                continue;
            }
            raises.wake.notified().await;
            continue;
        }
        let mut tries = 0;
        let failed = loop {
            let (store, policy, points) = (Arc::clone(&store), Arc::clone(&policy), batch.clone());
            let written = blocking(move || {
                store.raise_named_floors(&points, &|point| policy.floors(point).certain)
            })
            .await;
            match written {
                Ok(()) => break None,
                Err(e) if tries + 1 >= RAISE_TRIES => break Some(e),
                Err(e) => {
                    tracing::warn!(error = %e, "history floors not raised, trying again");
                    tokio::time::sleep(RAISE_BACKOFF * 2u32.pow(tries)).await;
                    tries += 1;
                }
            }
        };
        let Some(e) = failed else {
            let mut admitted = store::lock(&policy.observed);
            let mut pending = store::lock(&raises.pending);
            for (point, observed) in &batch {
                if pending.get(point) == Some(observed) {
                    pending.remove(point);
                }
                if admitted.get(point) == Some(observed) {
                    admitted.remove(point);
                }
            }
            continue;
        };
        tracing::error!(error = %e, points = batch.len(), "history floors not raised: purges suspended until a person validates them");
        let (log, at, n) = (Arc::clone(&store), now_ms(), batch.len());
        if let Err(e) = blocking(move || log.suspend(at, n)).await {
            tracing::error!(error = %e, "history suspension not written");
        }
        if raises.closed.load(SeqCst) {
            return Err(e);
        }
        // Tried again at the next publication (no busy retry).
        raises.wake.notified().await;
    }
}

impl History {
    /// One retention pass as of `now` (see `Policy::pass`).
    pub(crate) async fn prune_pass(&self, now: u64) -> Result<usize, HistoryError> {
        let _one = self.policy.passes.lock().await;
        let started = Instant::now();
        let (store, policy) = (Arc::clone(&self.store), Arc::clone(&self.policy));
        let rows = blocking(move || policy.pass(&store, now)).await?;
        *store::lock(&self.counters.last_prune) = Some(Prune {
            at: now_ms(),
            ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            rows,
        });
        Ok(rows)
    }

    /// The policy, and the reduction waiting for a person, counted now:
    /// that count is the plan a validation applies.
    pub async fn policies(&self) -> Result<Policies, HistoryError> {
        let _one = self.policy.passes.lock().await;
        let (store, policy) = (Arc::clone(&self.store), Arc::clone(&self.policy));
        let (plan, last_validation) =
            blocking(move || Ok((policy.plan(&store, now_ms())?, store.last_validation()?)))
                .await?;
        let reduction = plan.as_ref().map(Plan::reduction);
        *store::lock(&self.policy.shown) = plan;
        Ok(Policies {
            filter: self.policy.filter.clone(),
            configured: self.policy.configured.clone(),
            last_validation,
            reduction,
        })
    }

    /// A person validates the reduction shown (`at`, `fingerprint`): its
    /// points' durations are lowered and exactly the rows it announced are
    /// deleted, nothing else. Never an agent: `origin` must be a person with
    /// a PIN session (`Origin::Ui`). Returns the rows deleted.
    pub async fn validate(
        &self,
        at: u64,
        fingerprint: &str,
        origin: Origin,
        by: Option<String>,
    ) -> Result<usize, PolicyError> {
        if origin != Origin::Ui {
            return Err(PolicyError::HumanOnly);
        }
        // Held until the rows are deleted: no pass in between.
        let _one = self.policy.passes.lock().await;
        let plan = store::lock(&self.policy.shown).clone();
        let plan = plan.ok_or(PolicyError::Nothing)?;
        if plan.fingerprint != fingerprint || plan.at != at {
            return Err(PolicyError::Changed);
        }
        if now_ms().saturating_sub(plan.at) > REDUCTION_TTL_MS {
            return Err(PolicyError::Expired);
        }
        let (store, policy) = (Arc::clone(&self.store), Arc::clone(&self.policy));
        let removed =
            tokio::task::spawn_blocking(move || policy.apply(&store, &plan, by.as_deref()))
                .await
                .map_err(|e| HistoryError::Internal(e.to_string()))??;
        *store::lock(&self.policy.shown) = None;
        tracing::info!(removed, "history retention reduction validated");
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use moli_core::Value;

    use super::*;
    use crate::{Row, StorageConfig};

    const D: u64 = DAY_MS;

    /// Families a test sets, as a hub would publish them.
    type Known = Arc<Mutex<HashMap<String, Family>>>;

    struct Bench {
        store: Arc<Store>,
        dir: PathBuf,
        known: Known,
        now: u64,
    }

    impl Bench {
        /// A database holding `rows` (point, days ago), every point known
        /// as `States` unless `known` says otherwise.
        fn new(name: &str, rows: &[(&str, u64)]) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "moli-history-retention-{name}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            let store = Arc::new(Store::open(&dir.join("history.db")).unwrap());
            let now = now_ms();
            let rows: Vec<Row> = rows
                .iter()
                .map(|(p, ago)| (PointId::from(*p), Value::Int(1), now - ago))
                .collect();
            store.insert(&rows, false).unwrap();
            let known: Known = Arc::default();
            for (p, _, _) in &rows {
                known.lock().unwrap().insert(p.to_string(), Family::States);
            }
            Self {
                store,
                dir,
                known,
                now,
            }
        }

        /// Moli started with this configuration over the same database.
        fn history(&self, configured: Retention) -> History {
            self.store.close_run().unwrap();
            self.store.open_run().unwrap();
            let known = Arc::clone(&self.known);
            let families: Families = Box::new(move |p: &str| known.lock().unwrap().get(p).copied());
            let policy =
                Policy::open(&self.store, Filter::default(), configured, families, None).unwrap();
            History::with_policy(Arc::clone(&self.store), policy)
        }

        fn on_hub(&self, hub: &Hub, configured: Retention) -> History {
            let source = hub.clone();
            let policy = Policy::open(
                &self.store,
                Filter::default(),
                configured,
                Box::new(move |p| family_in(&source, p)),
                Some(hub.clone()),
            )
            .unwrap();
            History::with_policy(Arc::clone(&self.store), policy)
        }

        /// Reopen after an abrupt stop, without finalizing the previous run.
        fn reopen(&self, configured: Retention) -> History {
            let store = Arc::new(Store::open(&self.dir.join("history.db")).unwrap());
            store.open_run().unwrap();
            let known = Arc::clone(&self.known);
            let policy = Policy::open(
                &store,
                Filter::default(),
                configured,
                Box::new(move |p| known.lock().unwrap().get(p).copied()),
                None,
            )
            .unwrap();
            History::with_policy(store, policy)
        }

        /// Rows written the way the recorder writes them (`write_loop`).
        fn record(&self, history: &History, rows: &[(&str, u64)]) {
            let policy = Arc::clone(&history.policy);
            let rows: Vec<Row> = rows
                .iter()
                .map(|(p, ago)| (PointId::from(*p), Value::Int(1), self.now - ago))
                .collect();
            let floors = |point: &str| policy.floors(point);
            self.store.record(&rows, false, Some(&floors)).unwrap();
        }

        fn floor(&self, point: &str) -> Option<u32> {
            let id = self
                .store
                .points()
                .unwrap()
                .into_iter()
                .find(|(_, p)| p == point)?
                .0;
            self.store.floors().unwrap()?.get(&id).copied()
        }

        /// (point, days ago) of every row left.
        fn left(&self) -> Vec<(String, u64)> {
            self.store
                .query_sql("SELECT point, ts FROM history ORDER BY point, ts", 1_000)
                .unwrap()
                .rows
                .iter()
                .map(|r| {
                    let ts = r[1].as_u64().unwrap();
                    (r[0].as_str().unwrap().to_owned(), (self.now - ts) / D)
                })
                .collect()
        }
    }

    impl Drop for Bench {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn with(general: u32, families: &[(Family, u32)], points: &[(&str, u32)]) -> Retention {
        let mut r = Retention::uniform(general);
        r.families.extend(families.iter().copied());
        r.points
            .extend(points.iter().map(|(p, d)| ((*p).to_owned(), *d)));
        r
    }

    #[tokio::test]
    async fn without_a_new_configuration_the_purge_is_the_one_before() {
        let rows = [
            ("d/x", 40 * D),
            ("d/x", 35 * D),
            ("d/x", 10 * D),
            ("d/y", 31 * D),
            ("d/y", D),
            ("d/stable", 100 * D),
        ];
        let bench = Bench::new("default", &rows);
        let before = Bench::new("default-before", &rows);
        // The same rows, purged by the statement Moli ran before.
        rusqlite::Connection::open(before.dir.join("history.db"))
            .unwrap()
            .execute(
                "DELETE FROM samples WHERE ts < ?1
                 AND ts < (SELECT MAX(s.ts) FROM samples s
                           WHERE s.point_id = samples.point_id AND s.ts < ?1)",
                [i64::try_from(cutoff(before.now, 30)).unwrap()],
            )
            .unwrap();
        let (_, retention) = StorageConfig::default().compile(30).unwrap();
        let history = bench.history(retention);
        assert_eq!(history.prune_pass(bench.now).await.unwrap(), 1);
        assert_eq!(bench.left(), before.left());
        assert!(history.policies().await.unwrap().reduction.is_none());
    }

    #[tokio::test]
    async fn a_longer_duration_applies_at_once_and_is_remembered() {
        // X longer, Y shorter in the same change: Y waits, X is kept 60
        // days at once, and a later return of X to 30 days waits too.
        let rows = [
            ("d/x", 50 * D),
            ("d/x", 40 * D),
            ("d/x", 5 * D),
            ("d/y", 5 * D),
        ];
        let bench = Bench::new("longer", &rows);
        let mixed = bench.history(with(30, &[], &[("d/x", 60), ("d/y", 1)]));
        assert_eq!(mixed.prune_pass(bench.now).await.unwrap(), 0);
        let shown = mixed.policies().await.unwrap().reduction.unwrap();
        assert_eq!(shown.top[0].point, "d/y");

        let back = bench.history(with(30, &[], &[("d/y", 1)]));
        assert_eq!(
            back.prune_pass(bench.now).await.unwrap(),
            0,
            "X's 60 days hold"
        );
        assert_eq!(bench.left().len(), 4);
        let shown = back.policies().await.unwrap().reduction.unwrap();
        let x = shown.top.iter().find(|r| r.point == "d/x").unwrap();
        assert_eq!((x.from_days, x.to_days, x.rows), (60, 30, 1));
    }

    #[tokio::test]
    async fn a_reduction_waits_for_a_person_then_deletes_exactly_what_was_announced() {
        let rows = [
            ("d/x", 25 * D),
            ("d/x", 20 * D),
            ("d/x", 15 * D),
            ("d/x", 5 * D),
            // Unchanged, and due for its ordinary purge: not part of it.
            ("d/y", 31 * D),
            ("d/y", 30 * D + D / 2),
            ("d/y", D),
        ];
        let bench = Bench::new("shorter", &rows);
        let history = bench.history(with(30, &[], &[("d/x", 10)]));
        let r = history.policies().await.unwrap().reduction.unwrap();
        // 25 and 20 days go; 15 stays, the value a chart at 10 days starts from.
        assert_eq!((r.points, r.rows), (1, 2));
        assert_eq!(r.from, Some(bench.now - 25 * D));
        assert_eq!((r.top[0].from_days, r.top[0].to_days), (30, 10));

        for origin in [
            Origin::Api,
            Origin::Mcp,
            Origin::Cli,
            Origin::Assistant,
            Origin::Household,
            Origin::Automation,
            Origin::System,
        ] {
            let refused = history.validate(r.at, &r.fingerprint, origin, None).await;
            assert!(matches!(refused, Err(PolicyError::HumanOnly)), "{origin:?}");
        }
        let changed = history.validate(r.at, "another", Origin::Ui, None).await;
        assert!(matches!(changed, Err(PolicyError::Changed)));
        assert_eq!(bench.left().len(), 7, "still nothing deleted");

        let removed = history
            .validate(r.at, &r.fingerprint, Origin::Ui, Some("a person".into()))
            .await
            .unwrap();
        assert_eq!(removed as u64, r.rows);
        assert_eq!(
            bench.left(),
            vec![
                ("d/x".into(), 15),
                ("d/x".into(), 5),
                ("d/y".into(), 31),
                ("d/y".into(), 30),
                ("d/y".into(), 1),
            ]
        );
        let p = history.policies().await.unwrap();
        assert!(p.reduction.is_none());
        let v = p.last_validation.unwrap();
        assert_eq!(
            (v.by.as_deref(), v.points, v.rows),
            (Some("a person"), 1, 2)
        );
        let again = history
            .validate(r.at, &r.fingerprint, Origin::Ui, None)
            .await;
        assert!(matches!(again, Err(PolicyError::Nothing)));
    }

    #[tokio::test]
    async fn a_new_count_replaces_the_one_shown() {
        let bench = Bench::new(
            "recount",
            &[("d/x", 25 * D), ("d/x", 20 * D), ("d/x", 5 * D)],
        );
        let history = bench.history(with(30, &[], &[("d/x", 10)]));
        let first = history.policies().await.unwrap().reduction.unwrap();
        tokio::time::sleep(Duration::from_millis(2)).await;
        let second = history.policies().await.unwrap().reduction.unwrap();
        let stale = history
            .validate(first.at, &first.fingerprint, Origin::Ui, None)
            .await;
        assert!(matches!(stale, Err(PolicyError::Changed)), "{stale:?}");
        assert_eq!(bench.left().len(), 3);
        let ok = history
            .validate(second.at, &second.fingerprint, Origin::Ui, None)
            .await;
        assert_eq!(ok.unwrap(), 1);
    }

    #[tokio::test]
    async fn an_old_count_is_refused() {
        let bench = Bench::new("old", &[("d/x", 25 * D), ("d/x", 20 * D), ("d/x", 5 * D)]);
        let history = bench.history(with(30, &[], &[("d/x", 10)]));
        let r = history.policies().await.unwrap().reduction.unwrap();
        if let Some(plan) = store::lock(&history.policy.shown).as_mut() {
            plan.at -= REDUCTION_TTL_MS + 1;
        }
        let stale = history
            .validate(
                r.at - REDUCTION_TTL_MS - 1,
                &r.fingerprint,
                Origin::Ui,
                None,
            )
            .await;
        assert!(matches!(stale, Err(PolicyError::Expired)), "{stale:?}");
        assert_eq!(bench.left().len(), 3);
    }

    #[tokio::test]
    async fn a_point_whose_device_is_unknown_is_never_shortened() {
        // Right after a start, devices are not published yet: a diagnostic
        // duration of 3 days must not reach a point that may be climate.
        let bench = Bench::new(
            "unknown",
            &[("d/x", 20 * D), ("d/x", 10 * D), ("d/x", 5 * D)],
        );
        bench.known.lock().unwrap().clear();
        let history = bench.history(with(30, &[(Family::Diagnostic, 3)], &[]));
        assert_eq!(history.prune_pass(bench.now).await.unwrap(), 0);
        // Nothing to count yet: it enters no plan…
        assert!(history.policies().await.unwrap().reduction.is_none());
        // …and once published as diagnostic, it waits for a person.
        bench
            .known
            .lock()
            .unwrap()
            .insert("d/x".into(), Family::Diagnostic);
        assert_eq!(history.prune_pass(bench.now).await.unwrap(), 0);
        let r = history.policies().await.unwrap().reduction.unwrap();
        assert_eq!((r.top[0].from_days, r.top[0].to_days, r.rows), (30, 3, 2));
    }

    #[tokio::test]
    async fn a_point_first_seen_without_its_device_still_gets_a_floor() {
        // Set up before the point existed; the point is then recorded, and
        // its device is unknown at the next start's first pass.
        let bench = Bench::new("late-floor", &[]);
        let history = bench.history(with(30, &[(Family::Diagnostic, 3)], &[]));
        let x = |ago| (PointId::from("d/x"), Value::Int(1), bench.now - ago);
        bench
            .store
            .insert(&[x(20 * D), x(10 * D), x(D)], false)
            .unwrap();
        assert_eq!(history.prune_pass(bench.now).await.unwrap(), 0);
        bench
            .known
            .lock()
            .unwrap()
            .insert("d/x".into(), Family::Diagnostic);
        assert_eq!(
            history.prune_pass(bench.now).await.unwrap(),
            0,
            "kept 30 days"
        );
        let r = history.policies().await.unwrap().reduction.unwrap();
        assert_eq!((r.top[0].from_days, r.top[0].to_days, r.rows), (30, 3, 1));
    }

    #[tokio::test]
    async fn a_point_recorded_just_before_a_stop_keeps_its_duration() {
        // Recorded after the first pass, stopped before the next one, then
        // started again two days later with a shorter duration.
        let bench = Bench::new("stop", &[]);
        bench
            .known
            .lock()
            .unwrap()
            .insert("d/x".into(), Family::States);
        let before = bench.history(with(30, &[], &[]));
        bench.record(&before, &[("d/x", 20 * D), ("d/x", 10 * D), ("d/x", 5 * D)]);
        assert_eq!(bench.floor("d/x"), Some(30), "set with its first row");
        let after = bench.history(with(1, &[], &[]));
        assert_eq!(after.prune_pass(bench.now + 2 * D).await.unwrap(), 0);
        assert_eq!(bench.left().len(), 3);
        let r = after.policies().await.unwrap().reduction.unwrap();
        assert_eq!((r.top[0].from_days, r.top[0].to_days), (30, 1));
    }

    #[tokio::test]
    async fn a_certain_increase_is_remembered_even_without_the_device() {
        let bench = Bench::new(
            "certain",
            &[("d/x", 50 * D), ("d/x", 40 * D), ("d/x", 5 * D)],
        );
        bench.known.lock().unwrap().clear();
        // The point's own rule: 60 days whatever its family.
        let longer = bench.history(with(30, &[], &[("d/x", 60)]));
        assert_eq!(longer.prune_pass(bench.now).await.unwrap(), 0);
        assert_eq!(bench.floor("d/x"), Some(60));
        let back = bench.history(with(30, &[], &[]));
        assert_eq!(back.prune_pass(bench.now).await.unwrap(), 0, "60 days hold");
        bench
            .known
            .lock()
            .unwrap()
            .insert("d/x".into(), Family::States);
        let r = back.policies().await.unwrap().reduction.unwrap();
        assert_eq!((r.top[0].from_days, r.top[0].to_days, r.rows), (60, 30, 1));
    }

    #[tokio::test]
    async fn a_family_published_after_the_first_pass_keeps_its_increase() {
        // Started before the drivers: the pass sees no device. The device is
        // then published as climate (90 days) and a reading written; Moli
        // stops before the next pass and comes back with 30 days.
        let bench = Bench::new(
            "published",
            &[("d/x", 32 * D), ("d/x", 31 * D), ("d/x", 2 * D)],
        );
        bench.known.lock().unwrap().clear();
        let history = bench.history(with(30, &[(Family::Climate, 90)], &[]));
        assert_eq!(history.prune_pass(bench.now).await.unwrap(), 0);
        assert_eq!(bench.floor("d/x"), Some(30), "an ambiguity raises nothing");
        bench
            .known
            .lock()
            .unwrap()
            .insert("d/x".into(), Family::Climate);
        bench.record(&history, &[("d/x", 0)]);
        assert_eq!(bench.floor("d/x"), Some(90), "kept with the reading");
        let back = bench.history(with(30, &[], &[]));
        assert_eq!(back.prune_pass(bench.now).await.unwrap(), 0);
        assert_eq!(bench.left().len(), 4);
        let r = back.policies().await.unwrap().reduction.unwrap();
        assert_eq!((r.top[0].from_days, r.top[0].to_days, r.rows), (90, 30, 1));
    }

    #[tokio::test]
    async fn a_published_device_keeps_its_increase_before_any_write() {
        // Same start, but nothing of the point is written after its device
        // is published: the publication alone keeps the 90 days.
        let bench = Bench::new(
            "publication",
            &[("d/x", 32 * D), ("d/x", 31 * D), ("d/x", 2 * D)],
        );
        bench.known.lock().unwrap().clear();
        let history = bench.history(with(30, &[(Family::Climate, 90)], &[]));
        assert_eq!(history.prune_pass(bench.now).await.unwrap(), 0);
        assert_eq!(bench.floor("d/x"), Some(30));
        // Published: the hub now describes it as a thermometer.
        let device = device(moli_core::Semantic::Temperature, moli_core::Unit::Celsius);
        bench
            .known
            .lock()
            .unwrap()
            .insert("d/x".into(), Family::Climate);
        history.policy.published_at(&device, None);
        history.policy.stop_raising();
        raise_loop(Arc::clone(&history.store), Arc::clone(&history.policy))
            .await
            .unwrap();
        assert_eq!(bench.floor("d/x"), Some(90));
        let back = bench.history(with(30, &[], &[]));
        assert_eq!(back.prune_pass(bench.now).await.unwrap(), 0);
        let r = back.policies().await.unwrap().reduction.unwrap();
        assert_eq!((r.top[0].from_days, r.top[0].to_days, r.rows), (90, 30, 1));
    }

    fn device(semantic: moli_core::Semantic, unit: moli_core::Unit) -> Device {
        Device {
            id: moli_core::DeviceId::from("d"),
            instance: moli_core::InstanceId::from("d"),
            native_name: "d".into(),
            manufacturer: None,
            model: None,
            description: None,
            native_room: None,
            members: Vec::new(),
            points: vec![moli_core::PointSpec {
                key: "x".into(),
                label: "x".into(),
                kind: moli_core::Kind::Numeric {
                    min: None,
                    max: None,
                    step: None,
                },
                access: moli_core::Access::default(),
                unit: Some(unit),
                semantic,
            }],
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn raises_are_coalesced_bounded_and_drained() {
        let bench = Bench::new("raises", &[("d/x", D)]);
        let history = bench.history(with(
            30,
            &[(Family::Climate, 90), (Family::Diagnostic, 3)],
            &[],
        ));
        let policy = &history.policy;
        // 128 descriptions of one device, its point's meaning changing.
        for i in 0..128 {
            if i % 2 == 0 {
                policy.published_at(
                    &device(moli_core::Semantic::Temperature, moli_core::Unit::Celsius),
                    None,
                );
            } else {
                policy.published_at(
                    &device(moli_core::Semantic::Battery, moli_core::Unit::Percent),
                    None,
                );
            }
        }
        assert_eq!(
            store::lock(&policy.raises.pending).clone(),
            HashMap::from([(
                "d/x".to_owned(),
                Observed {
                    days: 90,
                    generation: 0
                }
            )]),
            "one entry per point"
        );
        // The last description must not erase the earlier 90-day observation.
        bench
            .known
            .lock()
            .unwrap()
            .insert("d/x".into(), Family::Diagnostic);
        let raising = tokio::spawn(raise_loop(Arc::clone(&history.store), Arc::clone(policy)));
        policy.stop_raising();
        tokio::time::timeout(Duration::from_secs(5), raising)
            .await
            .expect("drained, then ended")
            .unwrap()
            .unwrap();
        assert!(store::lock(&policy.raises.pending).is_empty());
        assert_eq!(bench.floor("d/x"), Some(90));
    }

    #[tokio::test]
    async fn an_old_publication_never_undoes_a_validated_reduction() {
        // Kept 90 days as climate; published as climate, then as a battery
        // gauge, both still queued. A person validates 90 → 3; the queue
        // written afterwards follows what the point is now.
        let bench = Bench::new(
            "old-publication",
            &[("d/x", 20 * D), ("d/x", 10 * D), ("d/x", D)],
        );
        let history = bench.history(with(
            30,
            &[(Family::Climate, 90), (Family::Diagnostic, 3)],
            &[],
        ));
        let id = bench.store.points().unwrap()[0].0;
        bench.store.raise_floors(&[(id, 90)]).unwrap();
        history.policy.published_at(
            &device(moli_core::Semantic::Temperature, moli_core::Unit::Celsius),
            None,
        );
        history.policy.published_at(
            &device(moli_core::Semantic::Battery, moli_core::Unit::Percent),
            None,
        );
        bench
            .known
            .lock()
            .unwrap()
            .insert("d/x".into(), Family::Diagnostic);
        let r = history.policies().await.unwrap().reduction.unwrap();
        assert_eq!((r.top[0].from_days, r.top[0].to_days, r.rows), (90, 3, 1));
        history
            .validate(r.at, &r.fingerprint, Origin::Ui, None)
            .await
            .unwrap();
        history.policy.stop_raising();
        raise_loop(Arc::clone(&history.store), Arc::clone(&history.policy))
            .await
            .unwrap();
        assert_eq!(bench.floor("d/x"), Some(3), "the consent holds");
        assert!(history.policies().await.unwrap().reduction.is_none());
    }

    #[test]
    fn a_certain_increase_received_between_lots_stops_the_next_delete() {
        let b = Bench::new(
            "pending-lots",
            &[
                ("d/x", 50 * D),
                ("d/x", 45 * D),
                ("d/x", 40 * D),
                ("d/x", 35 * D),
                ("d/x", 5 * D),
            ],
        );
        let h = b.history(with(
            30,
            &[(Family::Climate, 90), (Family::Diagnostic, 3)],
            &[],
        ));
        let id = b.store.points().unwrap()[0].0;
        let doomed = b.store.doomed(id, cutoff(b.now, 30)).unwrap().unwrap();
        let cut = Cut {
            id,
            oldest: doomed.oldest.unwrap(),
            anchor: doomed.anchor,
            max: doomed.rows,
            floor: 30,
            generation: b.store.guard().unwrap().generation,
        };
        let removed = b
            .store
            .prune(&[cut], 1, || {
                b.known
                    .lock()
                    .unwrap()
                    .insert("d/x".into(), Family::Climate);
                h.policy.published_at(
                    &device(moli_core::Semantic::Temperature, moli_core::Unit::Celsius),
                    None,
                );
                b.known
                    .lock()
                    .unwrap()
                    .insert("d/x".into(), Family::Diagnostic);
                h.policy.published_at(
                    &device(moli_core::Semantic::Battery, moli_core::Unit::Percent),
                    None,
                );
            })
            .unwrap();
        assert_eq!(
            removed, 1,
            "the pending maximum must protect the remaining lots"
        );
        assert_eq!(b.left().len(), 4);
        assert_eq!(
            b.floor("d/x"),
            Some(30),
            "no worker has persisted the increase yet"
        );
    }

    struct BusDescriptions {
        ready: Arc<tokio::sync::Notify>,
        again: Arc<tokio::sync::Notify>,
    }

    impl moli_runtime::Driver for BusDescriptions {
        fn kind(&self) -> &'static str {
            "fake"
        }

        fn run<'a>(
            &'a self,
            ctx: &'a mut moli_runtime::DriverCtx,
        ) -> moli_runtime::BoxFuture<'a, anyhow::Result<()>> {
            Box::pin(async move {
                for value in [1, 2, 3] {
                    for (semantic, unit) in [
                        (moli_core::Semantic::Temperature, moli_core::Unit::Celsius),
                        (moli_core::Semantic::Battery, moli_core::Unit::Percent),
                    ] {
                        let mut d = device(semantic, unit);
                        d.id = ctx.device_id("device");
                        d.instance = ctx.instance().clone();
                        ctx.upsert_device(d);
                    }
                    ctx.set_state(&ctx.device_id("device"), "x", Value::Int(value));
                    ctx.ready();
                    self.ready.notify_one();
                    if value < 3 {
                        tokio::select! { () = self.again.notified() => {}, () = ctx.cancelled() => return Ok(()) }
                    }
                }
                ctx.cancelled().await;
                Ok(())
            })
        }
    }

    async fn changed_batch(queue: &mut tokio::sync::mpsc::Receiver<crate::Batch>) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while let Some((rows, restate)) = queue.recv().await {
                if !restate && !rows.is_empty() {
                    return;
                }
            }
            panic!("the recorder stopped before the bus event was consumed");
        })
        .await
        .unwrap();
    }

    fn notices(hub: &Hub, count: usize) {
        for _ in 0..count {
            hub.notice(moli_core::Notice {
                title: None,
                message: "test".into(),
                from: None,
                ts: now_ms(),
            });
        }
    }

    #[tokio::test]
    async fn consent_replaces_old_bus_events_but_keeps_new_publications() {
        let point = "queued:device/x";
        let b = Bench::new(
            "bus-consent",
            &[(point, 20 * D), (point, 10 * D), (point, D)],
        );
        let hub = Hub::new(moli_runtime::HubOptions {
            event_capacity: 16,
            ..Default::default()
        })
        .unwrap();
        notices(&hub, 17); // The subscription must start after prior emissions.
        let source = crate::hub_events(&hub);
        let h = b.on_hub(
            &hub,
            with(30, &[(Family::Climate, 90), (Family::Diagnostic, 3)], &[]),
        );
        let id = b.store.points().unwrap()[0].0;
        b.store.raise_floors(&[(id, 90)]).unwrap();
        let ready = Arc::new(tokio::sync::Notify::new());
        let again = Arc::new(tokio::sync::Notify::new());
        let drivers = tokio_util::sync::CancellationToken::new();
        let driver = moli_runtime::spawn_driver(
            &hub,
            moli_core::InstanceId::from("queued"),
            Arc::new(BusDescriptions {
                ready: Arc::clone(&ready),
                again: Arc::clone(&again),
            }),
            drivers.clone(),
        );
        tokio::time::timeout(Duration::from_secs(5), ready.notified())
            .await
            .unwrap();
        let r = h.policies().await.unwrap().reduction.unwrap();
        h.validate(r.at, &r.fingerprint, Origin::Ui, None)
            .await
            .unwrap();
        let (tx, mut queue) = tokio::sync::mpsc::channel(8);
        let cancel = tokio_util::sync::CancellationToken::new();
        let recording = tokio::spawn({
            let h = h.clone();
            let cancel = cancel.clone();
            async move {
                crate::record(source, tx, &h, &Filter::default(), cancel).await;
            }
        });
        changed_batch(&mut queue).await;
        h.policy.raise_pending(&b.store).unwrap();
        assert_eq!(
            b.floor(point),
            Some(3),
            "pre-consent publications cannot restore 90"
        );
        assert_eq!(family_in(&hub, point), Some(Family::Diagnostic));
        // Force a bus gap before the genuinely new publications.
        notices(&hub, 32);
        again.notify_one();
        changed_batch(&mut queue).await;
        h.policy.raise_pending(&b.store).unwrap();
        assert_eq!(
            b.floor(point),
            Some(90),
            "a genuinely new maximum must survive"
        );
        // A second consent after Lagged must still have the right cursor.
        let r = h.policies().await.unwrap().reduction.unwrap();
        h.validate(r.at, &r.fingerprint, Origin::Ui, None)
            .await
            .unwrap();
        again.notify_one();
        changed_batch(&mut queue).await;
        h.policy.raise_pending(&b.store).unwrap();
        assert_eq!(
            b.floor(point),
            Some(90),
            "new observations survive a consent after a bus gap"
        );
        cancel.cancel();
        recording.await.unwrap();
        h.policy.stop_raising();
        raise_loop(Arc::clone(&b.store), Arc::clone(&h.policy))
            .await
            .unwrap();
        drivers.cancel();
        driver.await.unwrap();
    }

    #[tokio::test]
    async fn a_resume_at_the_same_floor_replaces_the_old_failed_observation() {
        let b = Bench::new(
            "resume-same",
            &[("d/x", 50 * D), ("d/x", 40 * D), ("d/x", 5 * D)],
        );
        let h = b.history(with(30, &[(Family::Climate, 90)], &[]));
        let db = rusqlite::Connection::open(b.dir.join("history.db")).unwrap();
        db.execute_batch("CREATE TRIGGER boom BEFORE UPDATE ON retention WHEN NEW.days=90 BEGIN SELECT RAISE(ABORT,'raise failed'); END;").unwrap();
        b.known
            .lock()
            .unwrap()
            .insert("d/x".into(), Family::Climate);
        h.policy.published_at(
            &device(moli_core::Semantic::Temperature, moli_core::Unit::Celsius),
            None,
        );
        h.policy.stop_raising();
        assert!(
            raise_loop(Arc::clone(&b.store), Arc::clone(&h.policy))
                .await
                .is_err()
        );
        b.known.lock().unwrap().insert("d/x".into(), Family::States);
        db.execute_batch("DROP TRIGGER boom").unwrap();
        let r = h.policies().await.unwrap().reduction.unwrap();
        assert!(r.resume);
        assert_eq!((r.top[0].from_days, r.top[0].to_days), (30, 30));
        h.validate(r.at, &r.fingerprint, Origin::Ui, None)
            .await
            .unwrap();
        raise_loop(Arc::clone(&b.store), Arc::clone(&h.policy))
            .await
            .unwrap();
        assert_eq!(b.floor("d/x"), Some(30));
        assert!(h.policies().await.unwrap().reduction.is_none());
    }

    #[tokio::test]
    async fn a_queued_increase_is_kept_before_the_next_pass_or_count() {
        let bench = Bench::new("queued", &[("d/x", 50 * D), ("d/x", 40 * D), ("d/x", D)]);
        let history = bench.history(with(
            30,
            &[(Family::Climate, 90), (Family::Diagnostic, 3)],
            &[],
        ));
        history.policy.published_at(
            &device(moli_core::Semantic::Temperature, moli_core::Unit::Celsius),
            None,
        );
        history.policy.published_at(
            &device(moli_core::Semantic::Battery, moli_core::Unit::Percent),
            None,
        );
        bench
            .known
            .lock()
            .unwrap()
            .insert("d/x".into(), Family::Diagnostic);
        assert_eq!(history.prune_pass(bench.now).await.unwrap(), 0);
        assert_eq!(bench.floor("d/x"), Some(90));
        let r = history.policies().await.unwrap().reduction.unwrap();
        assert_eq!((r.top[0].from_days, r.top[0].to_days), (90, 3));
        // A lot already captured before the consent must obey it too.
        let old = store::lock(&history.policy.raises.pending)
            .clone()
            .into_iter()
            .collect::<Vec<_>>();
        history
            .validate(r.at, &r.fingerprint, Origin::Ui, None)
            .await
            .unwrap();
        history
            .store
            .raise_named_floors(&old, &|p| history.policy.floors(p).certain)
            .unwrap();
        assert_eq!(bench.floor("d/x"), Some(3));
        // A fresh observation belongs to the newer consent generation.
        history.policy.published_at(
            &device(moli_core::Semantic::Temperature, moli_core::Unit::Celsius),
            None,
        );
        history.policy.stop_raising();
        raise_loop(Arc::clone(&history.store), Arc::clone(&history.policy))
            .await
            .unwrap();
        assert_eq!(bench.floor("d/x"), Some(90));
    }

    #[tokio::test]
    async fn resuming_an_unknown_family_persists_only_a_certain_increase() {
        let bench = Bench::new("resume-unknown", &[("d/x", D), ("d/y", D)]);
        bench.known.lock().unwrap().clear();
        let history = bench.history(with(30, &[(Family::Climate, 90)], &[("d/x", 60)]));
        bench.store.suspend(bench.now, 1).unwrap();
        let r = history.policies().await.unwrap().reduction.unwrap();
        history
            .validate(r.at, &r.fingerprint, Origin::Ui, None)
            .await
            .unwrap();
        assert_eq!(
            bench.floor("d/x"),
            Some(60),
            "the point's explicit rule is certain"
        );
        assert_eq!(
            bench.floor("d/y"),
            Some(30),
            "climate is only a possibility"
        );
    }

    #[tokio::test]
    async fn a_raise_that_cannot_be_written_suspends_purges_until_a_person_resumes_them() {
        let bench = Bench::new(
            "raise-fails",
            &[("d/x", 50 * D), ("d/x", 40 * D), ("d/x", 5 * D)],
        );
        bench
            .known
            .lock()
            .unwrap()
            .insert("d/x".into(), Family::Climate);
        let history = bench.history(with(30, &[(Family::Climate, 90)], &[]));
        let db = rusqlite::Connection::open(bench.dir.join("history.db")).unwrap();
        db.execute_batch(
            "CREATE TRIGGER boom BEFORE UPDATE ON retention WHEN NEW.days = 90
             BEGIN SELECT RAISE(ABORT, 'boom'); END;",
        )
        .unwrap();
        history.policy.published_at(
            &device(moli_core::Semantic::Temperature, moli_core::Unit::Celsius),
            None,
        );
        history.policy.stop_raising();
        let failed = raise_loop(Arc::clone(&history.store), Arc::clone(&history.policy)).await;
        assert!(failed.is_err(), "the stop says it failed");
        assert_eq!(bench.floor("d/x"), Some(30));
        assert!(bench.store.suspended().unwrap(), "durably");
        db.execute_batch("DROP TRIGGER boom").unwrap();

        // Started again, the 90 days no longer configured: nothing goes
        // until a person has seen what resuming deletes.
        let back = bench.history(with(30, &[], &[]));
        assert_eq!(back.prune_pass(bench.now).await.unwrap(), 0);
        assert_eq!(bench.left().len(), 3);
        let r = back.policies().await.unwrap().reduction.unwrap();
        assert!(r.resume);
        assert_eq!(r.rows, 1);
        let refused = back.validate(r.at, &r.fingerprint, Origin::Api, None).await;
        assert!(matches!(refused, Err(PolicyError::HumanOnly)));
        let removed = back
            .validate(r.at, &r.fingerprint, Origin::Ui, Some("a person".into()))
            .await
            .unwrap();
        assert_eq!(removed, 1);
        assert!(!bench.store.suspended().unwrap());
        assert!(back.policies().await.unwrap().reduction.is_none());
    }

    #[tokio::test]
    async fn a_double_write_failure_still_requires_a_person_after_restart() {
        let bench = Bench::new(
            "double-failure",
            &[("d/x", 50 * D), ("d/x", 40 * D), ("d/x", 5 * D)],
        );
        let history = bench.history(with(30, &[(Family::Climate, 90)], &[]));
        let db = rusqlite::Connection::open(bench.dir.join("history.db")).unwrap();
        db.execute_batch(
            "CREATE TRIGGER boom BEFORE UPDATE ON retention WHEN NEW.days = 90
             BEGIN SELECT RAISE(ABORT, 'raise failed'); END;
             CREATE TRIGGER no_log BEFORE INSERT ON retention_log WHEN NEW.kind = 'suspended'
             BEGIN SELECT RAISE(ABORT, 'log failed'); END;",
        )
        .unwrap();
        bench
            .known
            .lock()
            .unwrap()
            .insert("d/x".into(), Family::Climate);
        history.policy.published_at(
            &device(moli_core::Semantic::Temperature, moli_core::Unit::Celsius),
            None,
        );
        history.policy.stop_raising();
        assert!(
            raise_loop(Arc::clone(&history.store), Arc::clone(&history.policy))
                .await
                .is_err()
        );
        // Even a pass under the old, shorter family cannot delete in this run.
        bench
            .known
            .lock()
            .unwrap()
            .insert("d/x".into(), Family::States);
        assert_eq!(history.prune_pass(bench.now).await.unwrap(), 0);
        db.execute_batch("DROP TRIGGER boom; DROP TRIGGER no_log;")
            .unwrap();
        let back = bench.reopen(with(30, &[], &[]));
        assert_eq!(back.prune_pass(bench.now).await.unwrap(), 0);
        assert_eq!(bench.left().len(), 3);
        assert!(back.policies().await.unwrap().reduction.unwrap().resume);
    }

    #[tokio::test]
    async fn an_unfinished_run_suspends_purges_on_restart() {
        let bench = Bench::new("unclean", &[("d/x", 50 * D), ("d/x", 40 * D), ("d/x", D)]);
        let _before = bench.history(with(30, &[], &[]));
        let back = bench.reopen(with(30, &[], &[]));
        assert_eq!(back.prune_pass(bench.now).await.unwrap(), 0);
        assert!(back.policies().await.unwrap().reduction.unwrap().resume);
    }

    #[tokio::test]
    async fn a_new_suspension_invalidates_an_older_resume_plan() {
        let bench = Bench::new(
            "suspension-generation",
            &[("d/x", 50 * D), ("d/x", 40 * D), ("d/x", D)],
        );
        let history = bench.history(with(30, &[], &[]));
        bench.store.suspend(bench.now, 1).unwrap();
        let r = history.policies().await.unwrap().reduction.unwrap();
        assert!(r.resume);
        bench.store.suspend(bench.now + 1, 1).unwrap();
        let refused = history
            .validate(r.at, &r.fingerprint, Origin::Ui, None)
            .await;
        assert!(matches!(refused, Err(PolicyError::Changed)), "{refused:?}");
        assert_eq!(bench.left().len(), 3);
        let r = history.policies().await.unwrap().reduction.unwrap();
        history
            .validate(r.at, &r.fingerprint, Origin::Ui, None)
            .await
            .unwrap();
        bench.store.suspend(bench.now + 2, 1).unwrap();
        bench.record(
            &history,
            &[("d/new", 50 * D), ("d/new", 40 * D), ("d/new", D)],
        );
        assert_eq!(history.prune_pass(bench.now).await.unwrap(), 0);
        assert!(history.policies().await.unwrap().reduction.unwrap().resume);
    }

    #[tokio::test]
    async fn resuming_requires_the_certain_increases_to_be_written() {
        let bench = Bench::new(
            "resume-raise",
            &[("d/x", 50 * D), ("d/x", 40 * D), ("d/x", D)],
        );
        bench
            .known
            .lock()
            .unwrap()
            .insert("d/x".into(), Family::Climate);
        let history = bench.history(with(30, &[(Family::Climate, 90)], &[]));
        // Set up a suspension through the same failure as the recorder.
        let db = rusqlite::Connection::open(bench.dir.join("history.db")).unwrap();
        db.execute_batch(
            "CREATE TRIGGER boom BEFORE UPDATE ON retention WHEN NEW.days = 90
             BEGIN SELECT RAISE(ABORT, 'raise failed'); END;",
        )
        .unwrap();
        history.policy.published_at(
            &device(moli_core::Semantic::Temperature, moli_core::Unit::Celsius),
            None,
        );
        history.policy.stop_raising();
        assert!(
            raise_loop(Arc::clone(&history.store), Arc::clone(&history.policy))
                .await
                .is_err()
        );
        let r = history.policies().await.unwrap().reduction.unwrap();
        assert!(r.resume);
        assert_eq!(r.rows, 0);
        let refused = history
            .validate(r.at, &r.fingerprint, Origin::Ui, None)
            .await;
        assert!(matches!(refused, Err(PolicyError::Store(_))), "{refused:?}");
        assert!(bench.store.suspended().unwrap());
        assert_eq!(bench.floor("d/x"), Some(30));
        db.execute_batch("DROP TRIGGER boom").unwrap();
        let r = history.policies().await.unwrap().reduction.unwrap();
        history
            .validate(r.at, &r.fingerprint, Origin::Ui, None)
            .await
            .unwrap();
        assert_eq!(bench.floor("d/x"), Some(90));
        let back = bench.history(with(30, &[], &[]));
        assert_eq!(back.prune_pass(bench.now).await.unwrap(), 0);
        assert_eq!(bench.left().len(), 3);
    }

    #[tokio::test]
    async fn a_floor_raised_after_the_count_refuses_the_plan() {
        let bench = Bench::new(
            "raised",
            &[("d/x", 20 * D), ("d/x", 10 * D), ("d/x", 5 * D)],
        );
        bench
            .known
            .lock()
            .unwrap()
            .insert("d/x".into(), Family::Diagnostic);
        let history = bench.history(with(30, &[(Family::Diagnostic, 3)], &[]));
        let r = history.policies().await.unwrap().reduction.unwrap();
        // A write raised the floor between the count and the validation.
        let id = bench.store.points().unwrap()[0].0;
        bench.store.raise_floors(&[(id, 60)]).unwrap();
        let refused = history
            .validate(r.at, &r.fingerprint, Origin::Ui, None)
            .await;
        assert!(matches!(refused, Err(PolicyError::Changed)), "{refused:?}");
        assert_eq!(bench.left().len(), 3);
        assert_eq!(bench.floor("d/x"), Some(60));
    }

    #[tokio::test]
    async fn an_ambiguous_family_raises_no_floor() {
        // A diagnostic point validated at 3 days, its device not published
        // yet after a restart: its floor stays 3, not climate's 90.
        let bench = Bench::new("ambiguous", &[("d/x", 2 * D)]);
        bench.known.lock().unwrap().clear();
        let history = bench.history(with(
            30,
            &[(Family::Diagnostic, 3), (Family::Climate, 90)],
            &[],
        ));
        let id = bench.store.points().unwrap()[0].0;
        assert!(
            bench
                .store
                .validate_floors(&[(id, 30, 3)], 1, None, 0, bench.store.guard().unwrap())
                .unwrap()
                .is_some()
        );
        history.prune_pass(bench.now).await.unwrap();
        assert_eq!(bench.floor("d/x"), Some(3));
    }

    #[tokio::test]
    async fn a_point_reclassified_after_the_count_is_refused() {
        let bench = Bench::new(
            "reclassified",
            &[("d/x", 20 * D), ("d/x", 10 * D), ("d/x", 5 * D)],
        );
        bench
            .known
            .lock()
            .unwrap()
            .insert("d/x".into(), Family::Diagnostic);
        let history = bench.history(with(
            30,
            &[(Family::Diagnostic, 3), (Family::Climate, 90)],
            &[],
        ));
        let r = history.policies().await.unwrap().reduction.unwrap();
        assert_eq!(r.rows, 2);
        // The device is published again, its point now a temperature.
        bench
            .known
            .lock()
            .unwrap()
            .insert("d/x".into(), Family::Climate);
        let refused = history
            .validate(r.at, &r.fingerprint, Origin::Ui, None)
            .await;
        assert!(matches!(refused, Err(PolicyError::Changed)), "{refused:?}");
        assert_eq!(bench.left().len(), 3);
        assert_eq!(bench.floor("d/x"), Some(30));
    }

    #[tokio::test]
    async fn a_device_published_after_the_count_is_not_in_the_plan() {
        let bench = Bench::new("late", &[("d/x", 25 * D), ("d/x", 20 * D), ("d/x", 5 * D)]);
        // d/z is recorded, its device not published yet.
        let z = |ago| (PointId::from("d/z"), Value::Int(1), bench.now - ago);
        bench
            .store
            .insert(&[z(25 * D), z(20 * D), z(D)], false)
            .unwrap();
        let history = bench.history(with(30, &[(Family::Diagnostic, 3)], &[("d/x", 10)]));
        let r = history.policies().await.unwrap().reduction.unwrap();
        assert_eq!((r.points, r.rows), (1, 1));
        // d/z appears as diagnostic between the count and the validation.
        bench
            .known
            .lock()
            .unwrap()
            .insert("d/z".into(), Family::Diagnostic);
        let removed = history
            .validate(r.at, &r.fingerprint, Origin::Ui, None)
            .await
            .unwrap();
        assert_eq!(removed, 1, "only what was shown");
        assert_eq!(bench.left().iter().filter(|(p, _)| p == "d/z").count(), 3);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn no_pass_slips_between_a_validation_and_its_rows() {
        let bench = Bench::new(
            "race",
            &[("d/x", 25 * D), ("d/x", 20 * D), ("d/x", 9 * D), ("d/x", D)],
        );
        let history = bench.history(with(30, &[], &[("d/x", 10)]));
        let r = history.policies().await.unwrap().reduction.unwrap();
        assert_eq!(r.rows, 1);
        // The validation and a pass much later both wait on the lock.
        let held = history.policy.passes.lock().await;
        let validating = {
            let (h, fp) = (history.clone(), r.fingerprint.clone());
            tokio::spawn(async move { h.validate(r.at, &fp, Origin::Ui, None).await })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        let passing = {
            let h = history.clone();
            let later = bench.now + 2 * D;
            tokio::spawn(async move { h.prune_pass(later).await })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(held);
        assert_eq!(
            validating.await.unwrap().unwrap(),
            1,
            "exactly the announced row"
        );
        // The pass came after, with the validated duration and its own time.
        assert_eq!(passing.await.unwrap().unwrap(), 1);
    }

    #[tokio::test]
    async fn n0_reclassification_keeps_its_persisted_floor_after_reopen() {
        let diagnostic: moli_core::Semantic = serde_json::from_str("\"diagnostic\"").unwrap();
        let bench = Bench::new(
            "n0-diagnostic",
            &[
                ("d/x", 25 * D),
                ("d/x", 20 * D),
                ("d/x", 15 * D),
                ("d/x", 5 * D),
            ],
        );
        bench
            .known
            .lock()
            .unwrap()
            .insert("d/x".into(), Family::Other);
        let configured = with(30, &[(Family::Diagnostic, 7)], &[]);
        let history = bench.history(configured.clone());
        assert_eq!(bench.floor("d/x"), Some(30));
        let telemetry = device(diagnostic, moli_core::Unit::Other("B/s".into()));
        let family = Family::of(&telemetry.points[0]);
        assert_eq!(family, Family::Diagnostic);
        bench.known.lock().unwrap().insert("d/x".into(), family);
        history.policy.published_at(&telemetry, None);
        assert_eq!(history.prune_pass(bench.now).await.unwrap(), 0);
        assert_eq!(bench.floor("d/x"), Some(30));
        let before = history.policies().await.unwrap().reduction.unwrap();
        assert_eq!(
            (before.top[0].from_days, before.top[0].to_days, before.rows),
            (30, 7, 2)
        );
        drop(history);
        let reopened = bench.reopen(configured);
        assert_eq!(reopened.prune_pass(bench.now).await.unwrap(), 0);
        assert_eq!(bench.floor("d/x"), Some(30));
        assert_eq!(bench.left().len(), 4);
        let proposed = reopened.policies().await.unwrap().reduction.unwrap();
        assert_eq!(
            (
                proposed.top[0].from_days,
                proposed.top[0].to_days,
                proposed.rows
            ),
            (30, 7, 2)
        );
        for origin in [
            Origin::Api,
            Origin::Mcp,
            Origin::Cli,
            Origin::Assistant,
            Origin::Household,
            Origin::Automation,
            Origin::System,
        ] {
            assert!(matches!(
                reopened
                    .validate(proposed.at, &proposed.fingerprint, origin, None)
                    .await,
                Err(PolicyError::HumanOnly)
            ));
        }
        assert_eq!(bench.left().len(), 4);
        let removed = reopened
            .validate(proposed.at, &proposed.fingerprint, Origin::Ui, None)
            .await
            .unwrap();
        assert_eq!(removed as u64, proposed.rows);
        assert_eq!(bench.left(), vec![("d/x".into(), 15), ("d/x".into(), 5)]);
        assert_eq!(bench.floor("d/x"), Some(7));
    }
}
