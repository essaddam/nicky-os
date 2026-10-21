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
mod privacy;
mod retention;
pub use moli_rollup as rollup;
mod store;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use moli_core::{Event, PointId, Value, now_ms};
use moli_runtime::Hub;
use serde::Serialize;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub use policy::{
    Family, Filter, HistoryRules, Keep, PointKeep, Retention, StorageConfig, family, glob,
};
pub use retention::{Policies, PolicyError, Reduced, Reduction};
pub use store::{Bucket, Cut, Doomed, Floors, HistoryError, Series, SqlResult, Store, Validation};

const BATCH_EVERY: Duration = Duration::from_secs(2);
const BATCH_MAX: usize = 1_000;
const WRITER_QUEUE: usize = 64;
const RETENTION_EVERY: Duration = Duration::from_secs(6 * 3600);
/// Rows the recorder holds at most while the writer is saturated.
const PENDING_MAX: usize = 4 * BATCH_MAX;
/// How long the last 24 hours' volume is reused before it is counted again.
const VOLUME_TTL: Duration = Duration::from_secs(60);
const DAY_MS: u64 = 86_400_000;
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

/// The recorder's handle (`History::start`).
pub type Recorder = JoinHandle<Result<(), HistoryError>>;

/// Cheap handle used by the surfaces to read history.
#[derive(Clone, Debug)]
pub struct History {
    store: Arc<Store>,
    counters: Arc<Counters>,
    policy: Arc<retention::Policy>,
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
    /// value is missed. The handle resolves once everything is flushed; in
    /// error when a batch or retention floors could not be written (purges are then
    /// suspended until a person validates them).
    pub fn start(
        hub: &Hub,
        options: &Options,
        cancel: CancellationToken,
    ) -> Result<(Self, Recorder), HistoryError> {
        let store = Arc::new(Store::open(&options.path)?);
        store.open_run()?;
        let hub_families = hub.clone();
        let policy = retention::Policy::open(
            &store,
            options.filter.clone(),
            options.retention.clone(),
            Box::new(move |point: &str| retention::family_in(&hub_families, point)),
            Some(hub.clone()),
        )?;
        let history = Self::with_policy(store, policy);
        let (batches, queue) = mpsc::channel::<Batch>(WRITER_QUEUE);
        let writer = tokio::spawn(write_loop(
            Arc::clone(&history.store),
            queue,
            Arc::clone(&history.counters),
            Arc::clone(&history.policy),
        ));
        let pruning = tokio::spawn(prune_loop(history.clone(), cancel.clone()));
        let raising = tokio::spawn(retention::raise_loop(
            Arc::clone(&history.store),
            Arc::clone(&history.policy),
        ));
        let source = hub_events(hub);
        let recording = history.clone();
        let filter = options.filter.clone();
        let recorder = tokio::spawn(async move {
            record(source, batches, &recording, &filter, cancel).await;
            // Everything queued is written before the handle resolves.
            recording.policy.stop_raising();
            let written = writer.await;
            let raised = raising.await;
            let pruned = pruning.await;
            written.map_err(|e| HistoryError::Internal(e.to_string()))??;
            raised.map_err(|e| HistoryError::Internal(e.to_string()))??;
            pruned.map_err(|e| HistoryError::Internal(e.to_string()))??;
            let store = Arc::clone(&recording.store);
            blocking(move || store.close_run()).await
        });
        Ok((history, recorder))
    }

    /// A history to read (no recording, no purge), kept 30 days.
    pub fn from_store(store: Arc<Store>) -> Result<Self, HistoryError> {
        let policy = retention::Policy::open(
            &store,
            Filter::default(),
            Retention::uniform(30),
            Box::new(|_: &str| None),
            None,
        )?;
        Ok(Self::with_policy(store, policy))
    }

    fn with_policy(store: Arc<Store>, policy: retention::Policy) -> Self {
        Self {
            store,
            counters: Arc::default(),
            policy: Arc::new(policy),
        }
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
    position: u64,
}

fn hub_events(hub: &Hub) -> HubEvents {
    // Subscribe before reading the state: nothing falls between the two.
    let (events, position) = hub.subscribe_with_position();
    HubEvents {
        hub: hub.clone(),
        events,
        position,
    }
}

/// The present, read from the hub (at start, and after events were lost):
/// every device's description is queued for its floors (whatever the
/// filter, whether its points have a value or not), and the values of the
/// points recorded are returned.
fn present(hub: &Hub, filter: &Filter, policy: &retention::Policy) -> Vec<Row> {
    let snapshot = policy.snapshot(hub);
    snapshot
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
/// the bus and sees every reading. A device published has its floors queued
/// (`Policy::published_at`), without blocking the bus.
async fn record(
    mut source: HubEvents,
    batches: mpsc::Sender<Batch>,
    history: &History,
    filter: &Filter,
    cancel: CancellationToken,
) {
    let counters = &*history.counters;
    let _ = batches
        .send((present(&source.hub, filter, &history.policy), true))
        .await;
    let mut batch = Vec::new();
    let mut flush = tokio::time::interval(BATCH_EVERY);
    loop {
        tokio::select! {
            () = cancel.cancelled() => break,
            event = source.events.recv() => {
                source.position += match &event {
                    Ok(_) => 1,
                    Err(RecvError::Lagged(n)) => *n,
                    Err(RecvError::Closed) => 0,
                };
                match event {
                Ok(Event::State { point, value, ts, .. }) => {
                    if !filter.records(point.as_str()) {
                        continue;
                    }
                    batch.push((point, value, ts));
                    if batch.len() >= BATCH_MAX {
                        hand_over(&batches, &mut batch, counters);
                    }
                }
                Ok(Event::DeviceUpserted { device }) => history.policy.published_at(&device, Some(source.position)),
                Ok(_) => {}
                Err(RecvError::Lagged(missed)) => {
                    // Changes were missed; at least the present is recorded.
                    tracing::warn!(missed, "history recorder lagged, re-recording the current state");
                    hand_over(&batches, &mut batch, counters);
                    let present = present(&source.hub, filter, &history.policy);
                    if let Err(e) = batches.try_send((present, true)) {
                        let (rows, _) = e.into_inner();
                        counters.rows_dropped.fetch_add(rows.len() as u64, Relaxed);
                    }
                }
                Err(RecvError::Closed) => break,
                }
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

/// Writes the recorder's batches, each point's retention floor kept up to
/// date with its rows (`Store::record`).
async fn write_loop(
    store: Arc<Store>,
    mut queue: mpsc::Receiver<Batch>,
    counters: Arc<Counters>,
    policy: Arc<retention::Policy>,
) -> Result<(), HistoryError> {
    let mut failed = None;
    while let Some((rows, restate)) = queue.recv().await {
        let count = rows.len();
        let (db, rules) = (Arc::clone(&store), Arc::clone(&policy));
        let written = blocking(move || {
            let floors = |point: &str| rules.floors(point);
            db.record(&rows, restate, Some(&floors))
        });
        match written.await {
            Ok(()) => {
                counters.batches_written.fetch_add(1, Relaxed);
            }
            Err(e) => {
                tracing::warn!(error = %e, count, "history batch not written");
                counters.batches_failed.fetch_add(1, Relaxed);
                *store::lock(&counters.last_error) = Some((now_ms(), e.to_string()));
                // The batch's floor may have failed too, even before the
                // bus recorder has consumed the corresponding publication.
                // Keep the run unfinished and block every later purge lot.
                failed = Some(e);
                let db = Arc::clone(&store);
                if let Err(e) = blocking(move || db.suspend(now_ms(), 0)).await {
                    tracing::error!(error = %e, "history suspension not written");
                }
            }
        }
    }
    failed.map_or(Ok(()), Err)
}

async fn prune_loop(history: History, cancel: CancellationToken) -> Result<(), HistoryError> {
    let mut failed = None;
    let mut tick = tokio::time::interval(RETENTION_EVERY);
    loop {
        tokio::select! {
            () = cancel.cancelled() => break,
            _ = tick.tick() => {}
        }
        match history.prune_pass(now_ms()).await {
            Ok(0) => {}
            Ok(rows) => tracing::info!(removed = rows, "history pruned"),
            Err(e) => {
                tracing::warn!(error = %e, "history pruning failed");
                failed = Some(e);
                let store = Arc::clone(&history.store);
                if let Err(e) = blocking(move || store.suspend(now_ms(), 0)).await {
                    tracing::error!(error = %e, "history suspension not written");
                }
            }
        }
    }
    failed.map_or(Ok(()), Err)
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
        (History::from_store(store).unwrap(), dir)
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

    /// A thermometer: its point is a temperature.
    struct Thermometer;

    impl moli_runtime::Driver for Thermometer {
        fn kind(&self) -> &'static str {
            "fake"
        }

        fn run<'a>(
            &'a self,
            ctx: &'a mut moli_runtime::DriverCtx,
        ) -> moli_runtime::BoxFuture<'a, anyhow::Result<()>> {
            Box::pin(async move {
                let id = ctx.device_id("m");
                ctx.upsert_device(moli_core::Device {
                    id,
                    instance: ctx.instance().clone(),
                    native_name: "m".into(),
                    manufacturer: None,
                    model: None,
                    description: None,
                    native_room: None,
                    members: Vec::new(),
                    points: vec![moli_core::PointSpec {
                        key: "t".into(),
                        label: "t".into(),
                        kind: moli_core::Kind::Numeric {
                            min: None,
                            max: None,
                            step: None,
                        },
                        access: moli_core::Access::default(),
                        unit: Some(moli_core::Unit::Celsius),
                        semantic: moli_core::Semantic::Temperature,
                    }],
                });
                ctx.ready();
                std::future::pending::<()>().await;
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn the_present_read_again_raises_the_floors_of_excluded_points() {
        // A point recorded before it was excluded, floor 30; its device's
        // publication was lost in a lag. Reading the present again (as
        // after a lag) keeps climate's 90 days, though nothing of it is
        // written any more.
        let hub = Hub::new(moli_runtime::HubOptions::default()).unwrap();
        let drivers = CancellationToken::new();
        moli_runtime::spawn_driver(
            &hub,
            moli_core::InstanceId::from("fake"),
            Arc::new(Thermometer),
            drivers.clone(),
        );
        while hub.stats().drivers_running == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let dir = std::env::temp_dir().join(format!("moli-history-present-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Arc::new(Store::open(&dir.join("history.db")).unwrap());
        store
            .insert(&[(PointId::from("fake:m/t"), Value::Float(20.0), 1)], false)
            .unwrap();
        let (filter, retention) = toml::from_str::<StorageConfig>(
            "[history]\nexclude = [\"fake:*\"]\n[policy.climate]\ndays = 90",
        )
        .unwrap()
        .compile(30)
        .unwrap();
        let policy = Arc::new(
            retention::Policy::open(
                &store,
                filter.clone(),
                retention,
                Box::new({
                    let hub = hub.clone();
                    move |point: &str| retention::family_in(&hub, point)
                }),
                Some(hub.clone()),
            )
            .unwrap(),
        );
        let id = store.points().unwrap()[0].0;
        assert_eq!(store.floors().unwrap().unwrap()[&id], 30);
        let rows = present(&hub, &filter, &policy);
        assert!(rows.is_empty(), "excluded: nothing to write");
        policy.stop_raising();
        retention::raise_loop(Arc::clone(&store), Arc::clone(&policy))
            .await
            .unwrap();
        assert_eq!(store.floors().unwrap().unwrap()[&id], 90);
        drivers.cancel();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn the_handle_says_when_floors_could_not_be_written() {
        let hub = Hub::new(moli_runtime::HubOptions::default()).unwrap();
        let drivers = CancellationToken::new();
        moli_runtime::spawn_driver(
            &hub,
            moli_core::InstanceId::from("fake"),
            Arc::new(Thermometer),
            drivers.clone(),
        );
        while hub.stats().drivers_running == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let dir = std::env::temp_dir().join(format!("moli-history-handle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("history.db");
        {
            let store = Store::open(&path).unwrap();
            store
                .insert(&[(PointId::from("fake:m/t"), Value::Float(20.0), 1)], false)
                .unwrap();
            store.set_up_floors(30, 1).unwrap();
        }
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER boom BEFORE UPDATE ON retention WHEN NEW.days = 90
                 BEGIN SELECT RAISE(ABORT, 'boom'); END;",
            )
            .unwrap();
        let (filter, retention) = toml::from_str::<StorageConfig>(
            "[history]\nexclude = [\"fake:*\"]\n[policy.climate]\ndays = 90",
        )
        .unwrap()
        .compile(30)
        .unwrap();
        let cancel = CancellationToken::new();
        let options = Options {
            path: path.clone(),
            filter,
            retention,
        };
        let (_history, recorder) = History::start(&hub, &options, cancel.clone()).unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        cancel.cancel();
        let ended = tokio::time::timeout(Duration::from_secs(10), recorder)
            .await
            .unwrap()
            .unwrap();
        assert!(ended.is_err(), "not a normal stop: {ended:?}");
        assert!(Store::open(&path).unwrap().suspended().unwrap());
        drivers.cancel();
        let _ = std::fs::remove_dir_all(dir);
    }

    struct PruneFailureTrace(Arc<AtomicU64>);
    struct PruneEventText(String);
    impl tracing::field::Visit for PruneEventText {
        fn record_debug(&mut self, _field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            use std::fmt::Write as _;
            let _ = write!(self.0, "{value:?}");
        }
    }
    impl tracing::Subscriber for PruneFailureTrace {
        fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            let mut text = PruneEventText(String::new());
            event.record(&mut text);
            if text.0.contains("history pruning failed") {
                self.0.fetch_add(1, Relaxed);
            }
        }
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
    }

    #[tokio::test]
    async fn a_failed_pass_cannot_finalize_the_recorder_as_clean() {
        let dir =
            std::env::temp_dir().join(format!("moli-history-pass-fails-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("history.db");
        let now = now_ms();
        {
            let store = Store::open(&path).unwrap();
            let rows: Vec<_> = [50, 40, 5]
                .into_iter()
                .map(|days| (PointId::from("d/x"), Value::Int(1), now - days * DAY_MS))
                .collect();
            store.insert(&rows, false).unwrap();
            store.set_up_floors(30, now).unwrap();
        }
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("CREATE TRIGGER boom BEFORE UPDATE ON retention WHEN NEW.days=90 BEGIN SELECT RAISE(ABORT,'raise failed'); END;").unwrap();
        let errors = Arc::new(AtomicU64::new(0));
        let _trace = tracing::subscriber::set_default(PruneFailureTrace(Arc::clone(&errors)));
        let hub = Hub::new(moli_runtime::HubOptions::default()).unwrap();
        let cancel = CancellationToken::new();
        let mut options = Options {
            path,
            filter: Filter::default(),
            retention: Retention::uniform(90),
        };
        let (history, recorder) = History::start(&hub, &options, cancel.clone()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while errors.load(Relaxed) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        cancel.cancel();
        assert!(
            tokio::time::timeout(Duration::from_secs(5), recorder)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        assert!(history.store.suspended().unwrap());
        let clean: bool = db
            .query_row("SELECT clean FROM retention_guard", [], |r| r.get(0))
            .unwrap();
        assert!(!clean);
        db.execute_batch("DROP TRIGGER boom").unwrap();
        options.retention = Retention::uniform(30);
        let cancel = CancellationToken::new();
        let (back, recorder) = History::start(&hub, &options, cancel.clone()).unwrap();
        assert_eq!(back.prune_pass(now).await.unwrap(), 0);
        assert_eq!(back.series("d/x", 0, now, 10).await.unwrap().total, 3);
        assert!(back.policies().await.unwrap().reduction.unwrap().resume);
        cancel.cancel();
        recorder.await.unwrap().unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_failed_batch_suspends_purges_even_without_a_queued_publication() {
        let dir =
            std::env::temp_dir().join(format!("moli-history-batch-fails-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Arc::new(Store::open(&dir.join("history.db")).unwrap());
        store
            .insert(&[(PointId::from("d/x"), Value::Int(1), 1)], false)
            .unwrap();
        store.open_run().unwrap();
        let policy = Arc::new(
            retention::Policy::open(
                &store,
                Filter::default(),
                Retention::uniform(90),
                Box::new(|_| Some(Family::Climate)),
                None,
            )
            .unwrap(),
        );
        // Simulate an established 30-day floor and metadata already read as
        // 90 by the writer, before the recorder consumes that publication.
        let db = rusqlite::Connection::open(dir.join("history.db")).unwrap();
        db.execute_batch(
            "UPDATE retention SET days = 30;
             CREATE TRIGGER boom BEFORE UPDATE ON retention WHEN NEW.days = 90
             BEGIN SELECT RAISE(ABORT, 'raise failed'); END;",
        )
        .unwrap();
        let (tx, queue) = mpsc::channel(1);
        tx.send((vec![(PointId::from("d/x"), Value::Int(2), 2)], false))
            .await
            .unwrap();
        drop(tx);
        let counters = Arc::new(Counters::default());
        let ended = write_loop(Arc::clone(&store), queue, Arc::clone(&counters), policy).await;
        assert!(
            ended.is_err(),
            "the recorder must not finalize a failed drain"
        );
        assert_eq!(counters.batches_failed.load(Relaxed), 1);
        assert!(
            store.suspended().unwrap(),
            "an unwritten floor blocks every later purge lot"
        );
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_drained_recorder_restarts_without_suspending_purges() {
        let dir = std::env::temp_dir().join(format!("moli-history-clean-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let options = Options {
            path: dir.join("history.db"),
            filter: Filter::default(),
            retention: Retention::uniform(30),
        };
        let hub = Hub::new(moli_runtime::HubOptions::default()).unwrap();
        for _ in 0..2 {
            let cancel = CancellationToken::new();
            let (history, recorder) = History::start(&hub, &options, cancel.clone()).unwrap();
            assert!(!history.store.suspended().unwrap());
            // Opening a reader must not finalize this running recorder.
            let reader = History::from_store(Arc::clone(&history.store)).unwrap();
            assert_eq!(
                reader
                    .sql("SELECT clean FROM retention_guard", 1)
                    .await
                    .unwrap()
                    .rows[0][0],
                0
            );
            cancel.cancel();
            tokio::time::timeout(Duration::from_secs(5), recorder)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert_eq!(
                reader
                    .sql("SELECT clean FROM retention_guard", 1)
                    .await
                    .unwrap()
                    .rows[0][0],
                1
            );
        }
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
