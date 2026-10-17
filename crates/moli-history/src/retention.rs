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
//! a start, or gone) is kept its longest possible duration and never enters
//! a plan.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use moli_core::{Origin, PointId, now_ms};
use moli_runtime::Hub;
use serde::Serialize;

use crate::policy::{Family, Filter, Retention};
use crate::store::{self, Cut, Doomed, HistoryError, Store, Validation};
use crate::{DAY_MS, History, Prune, blocking};

/// Rows one purge statement deletes at most…
const PRUNE_LOT: usize = 5_000;
/// …and the breath the writer is given between two of them.
const PRUNE_PAUSE: Duration = Duration::from_millis(2);
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
}

impl std::fmt::Debug for Policy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Policy")
            .field("configured", &self.configured)
            .finish_non_exhaustive()
    }
}

/// A reduction as shown: the points it shortens, each with what it deletes.
#[derive(Clone, Debug)]
struct Plan {
    at: u64,
    fingerprint: String,
    items: Vec<Item>,
}

#[derive(Clone, Debug, Hash)]
struct Item {
    id: i64,
    point: String,
    family: Family,
    /// Kept so far…
    floor: u32,
    /// …and from now on.
    days: u32,
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
    /// The oldest row concerned, and the latest instant before which rows go.
    pub from: Option<u64>,
    pub to: Option<u64>,
    /// The points losing the most rows, most first.
    pub top: Vec<Reduced>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Reduced {
    pub point: String,
    pub family: Family,
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
    ) -> Result<Self, HistoryError> {
        if store.floors()?.is_none() {
            store.set_up_floors(configured.days, now_ms())?;
        }
        Ok(Self {
            filter,
            configured,
            families,
            shown: Mutex::new(None),
            passes: tokio::sync::Mutex::new(()),
        })
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

    /// A pass as of `now`: floors raised where the configuration asks for
    /// longer (a new point starts at its configured duration, the longest
    /// one while its device is unknown), then every point purged at the
    /// longer of its floor and its configured duration.
    fn pass(&self, store: &Store, now: u64) -> Result<usize, HistoryError> {
        let mut raised = Vec::new();
        let mut cuts = Vec::new();
        for s in self.seen(store)? {
            let configured = self.configured(&s.point, s.family);
            let keep = match (s.family, s.floor) {
                (Some(_), floor) => {
                    let keep = configured.max(floor.unwrap_or(0));
                    if floor != Some(keep) {
                        raised.push((s.id, keep));
                    }
                    keep
                }
                // Not raised for an unknown family (a validated floor
                // survives restarts), but never left without one: a point
                // first seen this way starts at its longest duration.
                (None, Some(floor)) => configured.max(floor),
                (None, None) => {
                    raised.push((s.id, configured));
                    configured
                }
            };
            if let Some(anchor) = store.anchor(s.id, cutoff(now, keep))? {
                cuts.push(Cut {
                    id: s.id,
                    anchor,
                    max: u64::MAX,
                });
            }
        }
        store.set_floors(&raised, None)?;
        store.prune(&cuts, PRUNE_LOT, || std::thread::sleep(PRUNE_PAUSE))
    }

    /// The points whose configured duration is below their floor, and what
    /// shortening each deletes as of `now`.
    fn plan(&self, store: &Store, now: u64) -> Result<Option<Plan>, HistoryError> {
        use std::hash::{Hash, Hasher};
        let mut items = Vec::new();
        for s in self.seen(store)? {
            let (Some(family), Some(floor)) = (s.family, s.floor) else {
                continue;
            };
            let days = self.configured.days(&s.point, family);
            if days < floor {
                items.push(Item {
                    doomed: store.doomed(s.id, cutoff(now, days))?,
                    id: s.id,
                    point: s.point,
                    family,
                    floor,
                    days,
                });
            }
        }
        if items.is_empty() {
            return Ok(None);
        }
        let mut h = std::hash::DefaultHasher::new();
        (now, &items).hash(&mut h);
        Ok(Some(Plan {
            at: now,
            fingerprint: format!("{:016x}", h.finish()),
            items,
        }))
    }
}

impl Plan {
    /// Applies the plan shown, if it still is what it was: its floors are
    /// lowered and exactly its rows deleted. Returns how many.
    fn apply(&self, store: &Store, by: Option<&str>) -> Result<usize, PolicyError> {
        let floors = store.floors()?.unwrap_or_default();
        if self
            .items
            .iter()
            .any(|i| floors.get(&i.id) != Some(&i.floor))
        {
            return Err(PolicyError::Changed);
        }
        let announced = self
            .items
            .iter()
            .filter_map(|i| i.doomed)
            .map(|d| d.rows)
            .sum();
        let floors: Vec<_> = self.items.iter().map(|i| (i.id, i.days)).collect();
        store.set_floors(&floors, Some((now_ms(), by, announced)))?;
        let cuts: Vec<_> = self
            .items
            .iter()
            .filter_map(|i| {
                i.doomed.map(|d| Cut {
                    id: i.id,
                    anchor: d.anchor,
                    max: d.rows,
                })
            })
            .collect();
        Ok(store.prune(&cuts, PRUNE_LOT, || std::thread::sleep(PRUNE_PAUSE))?)
    }

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
        let to = doomed().map(|i| cutoff(self.at, i.days)).max();
        top.sort_by(|a, b| b.rows.cmp(&a.rows).then_with(|| a.point.cmp(&b.point)));
        let rows = top.iter().map(|r| r.rows).sum();
        let points = top.len();
        top.truncate(REDUCTION_TOP);
        Reduction {
            at: self.at,
            fingerprint: self.fingerprint.clone(),
            points,
            rows,
            from,
            to,
            top,
        }
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
        let store = Arc::clone(&self.store);
        let removed = tokio::task::spawn_blocking(move || plan.apply(&store, by.as_deref()))
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
            let known = Arc::clone(&self.known);
            let families: Families = Box::new(move |p: &str| known.lock().unwrap().get(p).copied());
            let policy =
                Policy::open(&self.store, Filter::default(), configured, families).unwrap();
            History::with_policy(Arc::clone(&self.store), policy)
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
}
