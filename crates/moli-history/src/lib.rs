//! History: every value change, kept in SQLite.
//!
//! A recorder subscribes to the hub's bus (which only carries changes) and
//! hands batches to a writer task, so the bus is always read promptly; the
//! retention pass runs on its own. Reads come in two shapes: series for
//! charts (raw, or bucketed when long) and — the agent-native part —
//! read-only SQL over a simple view, so any agent can ask its own questions
//! (« combien de fois la porte s'est ouverte cette semaine ? »).
//!
//! What is recorded and for how long follows a policy (`policy`): a
//! longer duration applies at once, a shorter one deletes nothing until a
//! person (PIN session) has seen what it would delete and validated it.

mod policy;
mod store;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use moli_core::{Event, Origin, PointId, Value, now_ms};
use moli_runtime::Hub;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub use policy::{
    Family, Filter, HistoryRules, Keep, PointKeep, Retention, StorageConfig, family, glob,
};
pub use store::{Bucket, HistoryError, Series, SqlResult, Store};

const BATCH_EVERY: Duration = Duration::from_secs(2);
const BATCH_MAX: usize = 1_000;
const WRITER_QUEUE: usize = 64;
const RETENTION_EVERY: Duration = Duration::from_secs(6 * 3600);
/// Rows the recorder holds at most while the writer is saturated.
const PENDING_MAX: usize = 4 * BATCH_MAX;
/// How long the last 24 hours' volume is reused before it is counted again.
const VOLUME_TTL: Duration = Duration::from_secs(60);
const DAY_MS: u64 = 86_400_000;
/// Rows one purge statement deletes at most…
const PRUNE_LOT: usize = 5_000;
/// …and the breath the writer is given between two of them.
const PRUNE_PAUSE: Duration = Duration::from_millis(2);
/// The last validated retention, next to the database.
pub const POLICY_FILE: &str = "history-policy.json";
/// How long an announced reduction may be validated.
const REDUCTION_TTL_MS: u64 = 15 * 60_000;
/// Points listed with an announced reduction.
const REDUCTION_TOP: usize = 10;

type Row = (PointId, Value, u64);

/// A batch for the writer, and whether it re-states known values.
type Batch = (Vec<Row>, bool);

#[derive(Clone, Debug)]
pub struct Options {
    pub path: PathBuf,
    /// Points not written (`[storage.history]`).
    pub filter: Filter,
    /// The configured durations (`StorageConfig::compile`).
    pub retention: Retention,
}

/// Cheap handle used by the surfaces to read history.
#[derive(Clone, Debug)]
pub struct History {
    store: Arc<Store>,
    counters: Arc<Counters>,
    policy: Arc<Policy>,
}

/// What is configured, what a person last validated, and where it is kept.
#[derive(Debug)]
struct Policy {
    /// To know each point's family (absent: every point's is unknown).
    hub: Option<Hub>,
    filter: Filter,
    configured: Retention,
    validated: Mutex<Validated>,
    /// `data/history-policy.json` (absent: kept in memory only).
    file: Option<PathBuf>,
    /// The file is to be written at the next pass (none yet).
    unsaved: std::sync::atomic::AtomicBool,
    /// One pass at a time: the timer's, a validation's.
    passes: tokio::sync::Mutex<()>,
}

/// The last durations validated, persisted in `data/`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Validated {
    pub retention: Retention,
    /// When (ms since the epoch; 0: never).
    pub at: u64,
    /// Who validated; `None`: adopted from the configuration, as it
    /// deleted nothing more than the previous one.
    pub by: Option<String>,
}

/// A shorter duration waiting for a person: what it would delete.
#[derive(Clone, Debug, Serialize)]
pub struct Reduction {
    /// When it was counted: validating applies it as of that instant.
    pub at: u64,
    /// The policy shown; a validation for another one is refused.
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
    pub validated: Validated,
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

/// One point at a pass: how long it is kept now, how long it would be once
/// the configuration is validated, and whether that deletes anything kept.
struct Due {
    id: i64,
    point: String,
    family: Option<Family>,
    keep_days: u32,
    after_days: u32,
    reduced: bool,
}

fn cutoff(now: u64, days: u32) -> u64 {
    now.saturating_sub(u64::from(days) * DAY_MS)
}

impl Policy {
    fn new(hub: Option<Hub>, filter: Filter, configured: Retention, file: Option<PathBuf>) -> Self {
        let unknown = |retention| Validated {
            retention,
            at: 0,
            by: None,
        };
        let (validated, unsaved) = match file.as_deref().map(std::fs::read) {
            Some(Ok(bytes)) => match serde_json::from_slice::<Validated>(&bytes) {
                Ok(v) => (v, false),
                // Nobody knows what was validated: nothing is deleted
                // until a person validates the configuration.
                Err(e) => {
                    tracing::warn!(error = %e, "{POLICY_FILE} unreadable: no purge until a person validates");
                    (unknown(Retention::uniform(policy::MAX_DAYS)), true)
                }
            },
            // A first start: what Moli kept before any policy.
            _ => (unknown(Retention::uniform(configured.days)), true),
        };
        Self {
            hub,
            filter,
            configured,
            validated: Mutex::new(validated),
            file,
            unsaved: unsaved.into(),
            passes: tokio::sync::Mutex::new(()),
        }
    }

    fn family_of(&self, point: &str) -> Option<Family> {
        let point = PointId::from(point);
        let (device, key) = point.split()?;
        let (device, _) = self.hub.as_ref()?.describe(&device)?;
        device.point(key).map(Family::of)
    }

    fn due(&self, points: Vec<(i64, String)>) -> Vec<Due> {
        let validated = store::lock(&self.validated).retention.clone();
        points
            .into_iter()
            .map(|(id, point)| {
                let family = self.family_of(&point);
                let (mut keep_days, mut after_days, mut reduced) = (0, 0, false);
                for &f in policy::candidates(family) {
                    let (c, v) = (self.configured.days(&point, f), validated.days(&point, f));
                    keep_days = keep_days.max(c.max(v));
                    after_days = after_days.max(c);
                    reduced |= c < v;
                }
                Due {
                    id,
                    point,
                    family,
                    keep_days,
                    after_days,
                    reduced,
                }
            })
            .collect()
    }

    fn fingerprint(&self) -> String {
        use std::hash::{Hash, Hasher};
        let validated = store::lock(&self.validated).retention.clone();
        let mut h = std::hash::DefaultHasher::new();
        serde_json::to_string(&(&self.configured, &validated))
            .unwrap_or_default()
            .hash(&mut h);
        format!("{:016x}", h.finish())
    }

    /// Validates `retention` and writes it down (nothing changes if it
    /// cannot be written).
    fn adopt(&self, by: Option<String>) -> Result<(), HistoryError> {
        let next = Validated {
            retention: self.configured.clone(),
            at: now_ms(),
            by,
        };
        if let Some(file) = &self.file {
            let tmp = file.with_extension("json.tmp");
            let json = serde_json::to_vec_pretty(&next)
                .map_err(|e| HistoryError::Internal(e.to_string()))?;
            std::fs::write(&tmp, json)
                .and_then(|()| std::fs::rename(&tmp, file))
                .map_err(|e| HistoryError::Internal(format!("{}: {e}", file.display())))?;
        }
        *store::lock(&self.validated) = next;
        self.unsaved.store(false, Relaxed);
        Ok(())
    }
}

/// What the recorder, the writer and the pruning did since start.
#[derive(Debug, Default)]
struct Counters {
    batches_written: AtomicU64,
    batches_failed: AtomicU64,
    rows_merged: AtomicU64,
    rows_dropped: AtomicU64,
    last_error: Mutex<Option<(u64, String)>>,
    last_prune: Mutex<Option<Prune>>,
    /// Held while counting: concurrent callers wait, then reuse the count.
    volume: tokio::sync::Mutex<Option<(Instant, Volume)>>,
}

/// The last retention pass.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Prune {
    pub at: u64,
    pub ms: u64,
    pub rows: usize,
}

/// What the last 24 hours added, and what counting it cost.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Volume {
    pub rows_24h: u64,
    /// The chattiest points: (point, rows), most first.
    pub top_24h: Vec<(String, u64)>,
    pub query_ms: f64,
    /// When it was counted (ms since the epoch).
    pub counted_at: u64,
}

/// How the history grows and how its writes go (`/api/system`).
#[derive(Clone, Debug, Serialize)]
pub struct Growth {
    pub batches_written: u64,
    pub batches_failed: u64,
    /// Changes folded into a later one of the same point (writer saturated).
    pub rows_merged: u64,
    /// Changes dropped (writer saturated, too many points at once).
    pub rows_dropped: u64,
    /// (when, what) of the last batch not written.
    pub last_error: Option<(u64, String)>,
    pub last_prune: Option<Prune>,
    /// Absent when it could not be counted: `volume_error` says why.
    #[serde(flatten)]
    pub volume: Option<Volume>,
    pub volume_error: Option<String>,
}

impl History {
    /// Opens the database and starts recording. Start it *before* the
    /// drivers: it subscribes first, then records the current state, so no
    /// value is missed. The handle resolves once everything is flushed.
    pub fn start(
        hub: &Hub,
        options: &Options,
        cancel: CancellationToken,
    ) -> Result<(Self, JoinHandle<()>), HistoryError> {
        let store = Arc::new(Store::open(&options.path)?);
        let file = options.path.parent().map(|d| d.join(POLICY_FILE));
        let history = Self::with_policy(
            store,
            Policy::new(
                Some(hub.clone()),
                options.filter.clone(),
                options.retention.clone(),
                file,
            ),
        );
        let (batches, queue) = mpsc::channel::<Batch>(WRITER_QUEUE);
        let writer = tokio::spawn(write_loop(
            Arc::clone(&history.store),
            queue,
            Arc::clone(&history.counters),
        ));
        tokio::spawn(prune_loop(history.clone(), cancel.clone()));
        let source = hub_events(hub);
        let recording = Arc::clone(&history.counters);
        let filter = options.filter.clone();
        let recorder = tokio::spawn(async move {
            record(source, batches, &recording, &filter, cancel).await;
            let _ = writer.await;
        });
        Ok((history, recorder))
    }

    /// A history to read (no recording, no purge), kept 30 days.
    #[must_use]
    pub fn from_store(store: Arc<Store>) -> Self {
        Self::with_policy(
            store,
            Policy::new(None, Filter::default(), Retention::uniform(30), None),
        )
    }

    fn with_policy(store: Arc<Store>, policy: Policy) -> Self {
        Self {
            store,
            counters: Arc::default(),
            policy: Arc::new(policy),
        }
    }

    async fn due(&self) -> Result<Vec<Due>, HistoryError> {
        let store = Arc::clone(&self.store);
        let points = blocking(move || store.points()).await?;
        Ok(self.policy.due(points))
    }

    /// One retention pass as of `now`: each point loses what is older than
    /// its duration, the longer of the configured and the validated one. A
    /// configuration that deletes nothing more than the validated one is
    /// adopted by itself (an increase applies at once).
    async fn prune_pass(&self, now: u64) -> Result<usize, HistoryError> {
        let _one = self.policy.passes.lock().await;
        let started = Instant::now();
        let due = self.due().await?;
        let policy = &self.policy;
        let changed = store::lock(&policy.validated).retention != policy.configured;
        if !due.iter().any(|d| d.reduced) && (changed || policy.unsaved.load(Relaxed)) {
            let policy = Arc::clone(policy);
            blocking(move || policy.adopt(None)).await?;
        }
        let plan: Vec<_> = due
            .iter()
            .map(|d| (d.id, cutoff(now, d.keep_days)))
            .collect();
        let store = Arc::clone(&self.store);
        let rows =
            blocking(move || store.prune(&plan, PRUNE_LOT, || std::thread::sleep(PRUNE_PAUSE)))
                .await?;
        *store::lock(&self.counters.last_prune) = Some(Prune {
            at: now_ms(),
            ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            rows,
        });
        Ok(rows)
    }

    /// The policy, and the reduction waiting for a person, counted now.
    pub async fn policies(&self) -> Result<Policies, HistoryError> {
        let now = now_ms();
        let due = self.due().await?;
        let reduction = if due.iter().any(|d| d.reduced) {
            Some(self.reduction(now, due).await?)
        } else {
            None
        };
        Ok(Policies {
            filter: self.policy.filter.clone(),
            configured: self.policy.configured.clone(),
            validated: store::lock(&self.policy.validated).clone(),
            reduction,
        })
    }

    /// What validating would delete as of `now`: exactly what the pass
    /// that follows a validation deletes from the points it shortens.
    async fn reduction(&self, now: u64, due: Vec<Due>) -> Result<Reduction, HistoryError> {
        let fingerprint = self.policy.fingerprint();
        let store = Arc::clone(&self.store);
        blocking(move || {
            let mut top = Vec::new();
            let (mut rows, mut from, mut to) = (0, None::<u64>, None::<u64>);
            for d in due.into_iter().filter(|d| d.after_days < d.keep_days) {
                let at = cutoff(now, d.after_days);
                let (n, oldest) = store.doomed(d.id, at)?;
                if n == 0 {
                    continue;
                }
                rows += n;
                from = match (from, oldest) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                };
                to = to.max(Some(at));
                top.push(Reduced {
                    point: d.point,
                    family: d.family,
                    from_days: d.keep_days,
                    to_days: d.after_days,
                    rows: n,
                });
            }
            top.sort_by(|a, b| b.rows.cmp(&a.rows).then_with(|| a.point.cmp(&b.point)));
            let points = top.len();
            top.truncate(REDUCTION_TOP);
            Ok(Reduction {
                at: now,
                fingerprint,
                points,
                rows,
                from,
                to,
                top,
            })
        })
        .await
    }

    /// A person validates the reduction counted at `at` (`fingerprint`, as
    /// shown): it becomes the validated policy, and is applied as of `at`,
    /// deleting what was announced. Never an agent: `origin` must be a
    /// person with a PIN session (`Origin::Ui`). Returns the rows deleted.
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
        let now = now_ms();
        if at > now || now - at > REDUCTION_TTL_MS {
            return Err(PolicyError::Expired);
        }
        {
            let _one = self.policy.passes.lock().await;
            if !self.due().await?.iter().any(|d| d.reduced) {
                return Err(PolicyError::Nothing);
            }
            if self.policy.fingerprint() != fingerprint {
                return Err(PolicyError::Changed);
            }
            let policy = Arc::clone(&self.policy);
            blocking(move || policy.adopt(by)).await?;
        }
        tracing::info!("history retention reduction validated");
        Ok(self.prune_pass(at).await?)
    }

    /// The counters always; the volume when it can be counted (otherwise
    /// why not). On demand, no timer of its own.
    pub async fn growth(&self) -> Growth {
        let (volume, volume_error) = match self.volume().await {
            Ok(v) => (Some(v), None),
            Err(e) => (None, Some(e.to_string())),
        };
        let c = &self.counters;
        Growth {
            batches_written: c.batches_written.load(Relaxed),
            batches_failed: c.batches_failed.load(Relaxed),
            rows_merged: c.rows_merged.load(Relaxed),
            rows_dropped: c.rows_dropped.load(Relaxed),
            last_error: store::lock(&c.last_error).clone(),
            last_prune: *store::lock(&c.last_prune),
            volume,
            volume_error,
        }
    }

    /// The last 24 hours' volume, counted at most once a minute and once at
    /// a time: callers arriving during a count wait for it and reuse it. A
    /// failed count is not kept (the next call tries again).
    async fn volume(&self) -> Result<Volume, HistoryError> {
        let mut cache = self.counters.volume.lock().await;
        if let Some((_, v)) = cache.as_ref().filter(|(at, _)| at.elapsed() < VOLUME_TTL) {
            return Ok(v.clone());
        }
        let store = Arc::clone(&self.store);
        let since = now_ms().saturating_sub(DAY_MS);
        let v = blocking(move || {
            let started = Instant::now();
            let (rows_24h, top_24h) = store.volume(since, 10)?;
            Ok(Volume {
                rows_24h,
                top_24h,
                query_ms: started.elapsed().as_secs_f64() * 1000.0,
                counted_at: now_ms(),
            })
        })
        .await?;
        *cache = Some((Instant::now(), v.clone()));
        Ok(v)
    }

    pub async fn series(
        &self,
        point: &str,
        from_ms: u64,
        to_ms: u64,
        max_points: usize,
    ) -> Result<Series, HistoryError> {
        let (store, point) = (Arc::clone(&self.store), point.to_owned());
        blocking(move || store.series(&point, from_ms, to_ms, max_points)).await
    }

    pub async fn sql(&self, sql: &str, max_rows: usize) -> Result<SqlResult, HistoryError> {
        let (store, sql) = (Arc::clone(&self.store), sql.to_owned());
        blocking(move || store.query_sql(&sql, max_rows)).await
    }
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, HistoryError> + Send + 'static,
) -> Result<T, HistoryError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| HistoryError::Internal(e.to_string()))?
}

/// What the recorder listens to: the hub's bus, and its current state
/// (re-read at start and after any lag).
struct HubEvents {
    hub: Hub,
    events: tokio::sync::broadcast::Receiver<Event>,
}

fn hub_events(hub: &Hub) -> HubEvents {
    // Subscribe before reading the state: nothing falls between the two.
    let events = hub.subscribe();
    HubEvents {
        hub: hub.clone(),
        events,
    }
}

fn current_state(hub: &Hub, filter: &Filter) -> Vec<Row> {
    hub.snapshot()
        .devices
        .iter()
        .flat_map(|d| {
            d.state.iter().map(|(key, sample)| {
                (
                    PointId::new(&d.device.id, key),
                    sample.value.clone(),
                    sample.ts,
                )
            })
        })
        .filter(|(point, _, _)| filter.records(point.as_str()))
        .collect()
}

/// Excluded points are not written; the energy has its own subscription to
/// the bus and sees every reading.
async fn record(
    mut source: HubEvents,
    batches: mpsc::Sender<Batch>,
    counters: &Counters,
    filter: &Filter,
    cancel: CancellationToken,
) {
    let _ = batches
        .send((current_state(&source.hub, filter), true))
        .await;
    let mut batch = Vec::new();
    let mut flush = tokio::time::interval(BATCH_EVERY);
    loop {
        tokio::select! {
            () = cancel.cancelled() => break,
            event = source.events.recv() => match event {
                Ok(Event::State { point, value, ts }) => {
                    if !filter.records(point.as_str()) {
                        continue;
                    }
                    batch.push((point, value, ts));
                    if batch.len() >= BATCH_MAX {
                        hand_over(&batches, &mut batch, counters);
                    }
                }
                Ok(_) => {}
                Err(RecvError::Lagged(missed)) => {
                    // Changes were missed; at least the present is recorded.
                    tracing::warn!(missed, "history recorder lagged, re-recording the current state");
                    hand_over(&batches, &mut batch, counters);
                    if let Err(e) = batches.try_send((current_state(&source.hub, filter), true)) {
                        let (rows, _) = e.into_inner();
                        counters.rows_dropped.fetch_add(rows.len() as u64, Relaxed);
                    }
                }
                Err(RecvError::Closed) => break,
            },
            _ = flush.tick() => hand_over(&batches, &mut batch, counters),
        }
    }
    if !batch.is_empty() {
        let _ = batches.send((batch, false)).await;
    }
}

/// Never blocks the bus: a full writer queue keeps the batch for later,
/// within `PENDING_MAX` rows (see `compact`).
fn hand_over(batches: &mpsc::Sender<Batch>, batch: &mut Vec<Row>, counters: &Counters) {
    if batch.is_empty() {
        return;
    }
    match batches.try_send((std::mem::take(batch), false)) {
        Ok(()) => {}
        Err(
            mpsc::error::TrySendError::Full((rows, _))
            | mpsc::error::TrySendError::Closed((rows, _)),
        ) => {
            *batch = rows;
            if batch.len() >= PENDING_MAX {
                compact(batch, counters);
            }
        }
    }
}

/// The writer is saturated: each point keeps only its latest change (the
/// value a chart ends on, and what the present is), the others are counted
/// as merged. With more points than half the bound, the oldest are dropped
/// and counted as lost. Memory stays bounded, and compacting happens at most
/// once every `PENDING_MAX / 2` changes.
fn compact(batch: &mut Vec<Row>, counters: &Counters) {
    let before = batch.len();
    let keep: Vec<bool> = {
        let mut seen = std::collections::HashSet::new();
        let mut keep: Vec<bool> = batch
            .iter()
            .rev()
            .map(|(point, _, _)| seen.insert(point.as_str()))
            .collect();
        keep.reverse();
        keep
    };
    let mut flags = keep.into_iter();
    batch.retain(|_| flags.next().unwrap_or(false));
    let merged = before - batch.len();
    let dropped = batch.len().saturating_sub(PENDING_MAX / 2);
    batch.drain(..dropped);
    counters.rows_merged.fetch_add(merged as u64, Relaxed);
    counters.rows_dropped.fetch_add(dropped as u64, Relaxed);
    tracing::warn!(
        merged,
        dropped,
        kept = batch.len(),
        "history writer saturated, batch compacted"
    );
}

async fn write_loop(store: Arc<Store>, mut queue: mpsc::Receiver<Batch>, counters: Arc<Counters>) {
    while let Some((rows, restate)) = queue.recv().await {
        let count = rows.len();
        let store = Arc::clone(&store);
        match blocking(move || store.insert(&rows, restate)).await {
            Ok(()) => {
                counters.batches_written.fetch_add(1, Relaxed);
            }
            Err(e) => {
                tracing::warn!(error = %e, count, "history batch not written");
                counters.batches_failed.fetch_add(1, Relaxed);
                *store::lock(&counters.last_error) = Some((now_ms(), e.to_string()));
            }
        }
    }
}

async fn prune_loop(history: History, cancel: CancellationToken) {
    let mut tick = tokio::time::interval(RETENTION_EVERY);
    loop {
        tokio::select! {
            () = cancel.cancelled() => break,
            _ = tick.tick() => {}
        }
        match history.prune_pass(now_ms()).await {
            Ok(0) => {}
            Ok(rows) => tracing::info!(removed = rows, "history pruned"),
            Err(e) => tracing::warn!(error = %e, "history pruning failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// The writer never reads: its queue is full after one batch.
    fn saturated() -> (mpsc::Sender<Batch>, mpsc::Receiver<Batch>) {
        let (tx, rx) = mpsc::channel(1);
        tx.try_send((Vec::new(), false)).unwrap();
        (tx, rx)
    }

    /// What the recorder does with each change.
    fn feed(
        tx: &mpsc::Sender<Batch>,
        counters: &Counters,
        rows: impl Iterator<Item = Row>,
    ) -> usize {
        let (mut batch, mut peak) = (Vec::new(), 0);
        for row in rows {
            batch.push(row);
            if batch.len() >= BATCH_MAX {
                hand_over(tx, &mut batch, counters);
            }
            peak = peak.max(batch.len());
        }
        hand_over(tx, &mut batch, counters);
        assert!(batch.len() < PENDING_MAX);
        peak
    }

    #[test]
    fn a_saturated_writer_merges_each_points_changes() {
        let (tx, _rx) = saturated();
        let counters = Counters::default();
        let rows = (0..200_000u64).map(|i| {
            (
                PointId::from(format!("d/p{}", i % 10)),
                Value::Int(i64::try_from(i).unwrap()),
                i,
            )
        });
        let peak = feed(&tx, &counters, rows);
        assert!(peak <= PENDING_MAX, "{peak}");
        assert!(counters.rows_merged.load(Relaxed) > 190_000);
        assert_eq!(counters.rows_dropped.load(Relaxed), 0);
    }

    #[test]
    fn a_saturated_writer_drops_the_oldest_beyond_the_bound() {
        let (tx, _rx) = saturated();
        let counters = Counters::default();
        // Every change on a new point: nothing to merge.
        let rows = (0..200_000u64).map(|i| (PointId::from(format!("d/p{i}")), Value::Null, i));
        let peak = feed(&tx, &counters, rows);
        assert!(peak <= PENDING_MAX, "{peak}");
        assert!(counters.rows_dropped.load(Relaxed) > 190_000);
    }

    fn history(name: &str) -> (History, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("moli-history-lib-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Arc::new(Store::open(&dir.join("history.db")).unwrap());
        let rows: Vec<Row> = (0..10_000u64)
            .map(|i| (PointId::from("d/p"), Value::Null, now_ms() - 60_000 + i))
            .collect();
        store.insert(&rows, false).unwrap();
        (History::from_store(store), dir)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_callers_share_one_count() {
        let (history, dir) = history("concurrent");
        let barrier = Arc::new(tokio::sync::Barrier::new(16));
        let calls: Vec<_> = (0..16)
            .map(|_| {
                let (history, barrier) = (history.clone(), Arc::clone(&barrier));
                tokio::spawn(async move {
                    barrier.wait().await;
                    history.growth().await.volume.unwrap()
                })
            })
            .collect();
        let mut counted = std::collections::HashSet::new();
        for call in calls {
            let v = call.await.unwrap();
            assert_eq!(v.rows_24h, 10_000);
            counted.insert((v.counted_at, v.query_ms.to_bits()));
        }
        assert_eq!(counted.len(), 1, "one count for all: {counted:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_failed_count_keeps_the_counters_and_says_why() {
        let (history, dir) = history("broken");
        history.counters.batches_written.store(3, Relaxed);
        history.counters.batches_failed.store(1, Relaxed);
        rusqlite::Connection::open(dir.join("history.db"))
            .unwrap()
            .execute_batch("ALTER TABLE samples RENAME TO gone")
            .unwrap();
        let g = history.growth().await;
        assert!(g.volume.is_none());
        assert!(
            g.volume_error
                .as_ref()
                .is_some_and(|e| e.contains("samples"))
        );
        assert_eq!((g.batches_written, g.batches_failed), (3, 1));
        let json = serde_json::to_value(&g).unwrap();
        assert!(
            json.get("rows_24h").is_none(),
            "unknown, never zero: {json}"
        );
        assert_eq!(json["batches_written"], 3);
        let _ = std::fs::remove_dir_all(dir);
    }

    const D: u64 = DAY_MS;

    /// A history over `rows` (point, days ago), its validated retention
    /// filed (`None`: a first start), `configured` read from `moli.toml`.
    fn guarded(
        name: &str,
        rows: &[(&str, u64)],
        validated: Option<Retention>,
        configured: Retention,
    ) -> (History, PathBuf, u64) {
        let dir =
            std::env::temp_dir().join(format!("moli-history-policy-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Arc::new(Store::open(&dir.join("history.db")).unwrap());
        let now = now_ms();
        let rows: Vec<Row> = rows
            .iter()
            .map(|(p, ago)| (PointId::from(*p), Value::Int(1), now - ago))
            .collect();
        store.insert(&rows, false).unwrap();
        let file = dir.join(POLICY_FILE);
        if let Some(retention) = validated {
            let v = Validated {
                retention,
                at: 1,
                by: Some("someone".into()),
            };
            std::fs::write(&file, serde_json::to_vec(&v).unwrap()).unwrap();
        }
        let policy = Policy::new(None, Filter::default(), configured, Some(file));
        (History::with_policy(store, policy), dir, now)
    }

    fn kept(history: &History) -> Vec<Vec<serde_json::Value>> {
        history
            .store
            .query_sql("SELECT point, ts FROM history ORDER BY point, ts", 1_000)
            .unwrap()
            .rows
    }

    fn validated(dir: &std::path::Path) -> Validated {
        serde_json::from_slice(&std::fs::read(dir.join(POLICY_FILE)).unwrap()).unwrap()
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
        let (filter, retention) = StorageConfig::default().compile(30).unwrap();
        assert!(filter.records("d/x"));
        let (history, dir, now) = guarded("default", &rows, None, retention);
        // The same rows, purged by the statement Moli ran before.
        let (before, before_dir, then) =
            guarded("default-before", &rows, None, Retention::uniform(30));
        let conn = rusqlite::Connection::open(before_dir.join("history.db")).unwrap();
        conn.execute(
            "DELETE FROM samples WHERE ts < ?1
             AND ts < (SELECT MAX(s.ts) FROM samples s
                       WHERE s.point_id = samples.point_id AND s.ts < ?1)",
            [i64::try_from(cutoff(then, 30)).unwrap()],
        )
        .unwrap();

        assert_eq!(history.prune_pass(now).await.unwrap(), 1);
        // Rows as (point, age): the two stores were filled a moment apart.
        let aged = |h: &History, at: u64| -> Vec<(String, u64)> {
            kept(h)
                .iter()
                .map(|r| {
                    (
                        r[0].as_str().unwrap().to_owned(),
                        at - r[1].as_u64().unwrap(),
                    )
                })
                .collect()
        };
        assert_eq!(aged(&history, now), aged(&before, then));
        let policies = history.policies().await.unwrap();
        assert!(policies.reduction.is_none());
        assert_eq!(validated(&dir).retention, Retention::uniform(30));
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(before_dir);
    }

    #[tokio::test]
    async fn a_longer_duration_applies_at_once() {
        let rows = [("d/x", 25 * D), ("d/x", 20 * D), ("d/x", 5 * D)];
        let (history, dir, now) = guarded(
            "longer",
            &rows,
            Some(Retention::uniform(10)),
            Retention::uniform(30),
        );
        assert_eq!(history.prune_pass(now).await.unwrap(), 0);
        assert_eq!(kept(&history).len(), 3);
        assert!(history.policies().await.unwrap().reduction.is_none());
        let v = validated(&dir);
        assert_eq!((v.retention, v.by), (Retention::uniform(30), None));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_reduction_waits_for_a_person_then_deletes_exactly_what_was_announced() {
        let rows = [
            ("d/x", 25 * D),
            ("d/x", 20 * D),
            ("d/x", 15 * D),
            ("d/x", 5 * D),
            ("d/y", 2 * D),
        ];
        let (history, dir, now) = guarded(
            "shorter",
            &rows,
            Some(Retention::uniform(30)),
            Retention::uniform(10),
        );
        assert_eq!(
            history.prune_pass(now).await.unwrap(),
            0,
            "nothing goes yet"
        );
        assert_eq!(kept(&history).len(), 5);

        let reduction = history.policies().await.unwrap().reduction.unwrap();
        // 25 and 20 days go; 15 stays, the value a chart at 10 days starts from.
        assert_eq!((reduction.points, reduction.rows), (1, 2));
        assert_eq!(reduction.from, Some(now - 25 * D));
        let top = &reduction.top[0];
        assert_eq!(
            (top.point.as_str(), top.from_days, top.to_days),
            ("d/x", 30, 10)
        );

        // No agent validates, whatever it declares.
        for origin in [
            Origin::Api,
            Origin::Mcp,
            Origin::Cli,
            Origin::Assistant,
            Origin::Household,
            Origin::Automation,
            Origin::System,
        ] {
            let refused = history
                .validate(reduction.at, &reduction.fingerprint, origin, None)
                .await;
            assert!(matches!(refused, Err(PolicyError::HumanOnly)), "{origin:?}");
        }
        let changed = history
            .validate(reduction.at, "another", Origin::Ui, None)
            .await;
        assert!(matches!(changed, Err(PolicyError::Changed)));
        let stale = history
            .validate(
                reduction.at - REDUCTION_TTL_MS - 1,
                &reduction.fingerprint,
                Origin::Ui,
                None,
            )
            .await;
        assert!(matches!(stale, Err(PolicyError::Expired)));
        assert_eq!(kept(&history).len(), 5, "still nothing deleted");
        assert_eq!(validated(&dir).retention, Retention::uniform(30));

        let removed = history
            .validate(
                reduction.at,
                &reduction.fingerprint,
                Origin::Ui,
                Some("a person".into()),
            )
            .await
            .unwrap();
        assert_eq!(removed as u64, reduction.rows);
        let ts = |r: &Vec<serde_json::Value>| r[1].as_u64().unwrap();
        let left: Vec<u64> = kept(&history).iter().map(ts).collect();
        assert_eq!(left, vec![now - 15 * D, now - 5 * D, now - 2 * D]);
        let v = validated(&dir);
        assert_eq!(
            (v.retention, v.by.as_deref()),
            (Retention::uniform(10), Some("a person"))
        );
        assert!(history.policies().await.unwrap().reduction.is_none());
        let again = history
            .validate(reduction.at, &reduction.fingerprint, Origin::Ui, None)
            .await;
        assert!(matches!(again, Err(PolicyError::Nothing)));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_point_whose_family_is_unknown_keeps_the_longest_duration() {
        // Right after a start, devices are not published yet: a diagnostic
        // duration of 3 days must not reach a point that may be climate.
        let rows = [("d/x", 20 * D), ("d/x", 10 * D), ("d/x", 5 * D)];
        let mut configured = Retention::uniform(30);
        configured.families.insert(Family::Diagnostic, 3);
        let (history, dir, now) =
            guarded("unknown", &rows, Some(Retention::uniform(30)), configured);
        assert_eq!(history.prune_pass(now).await.unwrap(), 0);
        assert!(history.policies().await.unwrap().reduction.is_some());
        assert_eq!(
            validated(&dir).retention,
            Retention::uniform(30),
            "not adopted"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn an_unreadable_validation_deletes_nothing() {
        let rows = [("d/x", 400 * D), ("d/x", 300 * D), ("d/x", 5 * D)];
        let (history, dir, now) = guarded("unreadable", &rows, None, Retention::uniform(30));
        std::fs::write(dir.join(POLICY_FILE), b"{ broken").unwrap();
        let policy = Policy::new(
            None,
            Filter::default(),
            Retention::uniform(30),
            Some(dir.join(POLICY_FILE)),
        );
        let history = History::with_policy(Arc::clone(&history.store), policy);
        assert_eq!(history.prune_pass(now).await.unwrap(), 0);
        assert!(history.policies().await.unwrap().reduction.is_some());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn compacting_keeps_each_points_latest_change() {
        let counters = Counters::default();
        let mut batch = vec![
            (PointId::from("d/a"), Value::Int(1), 1),
            (PointId::from("d/b"), Value::Int(2), 2),
            (PointId::from("d/a"), Value::Int(3), 3),
        ];
        compact(&mut batch, &counters);
        assert_eq!(
            batch,
            vec![
                (PointId::from("d/b"), Value::Int(2), 2),
                (PointId::from("d/a"), Value::Int(3), 3),
            ]
        );
        assert_eq!(counters.rows_merged.load(Relaxed), 1);
    }
}
