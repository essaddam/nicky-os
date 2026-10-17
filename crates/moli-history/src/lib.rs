//! History: every value change, kept in SQLite.
//!
//! A recorder subscribes to the hub's bus (which only carries changes) and
//! hands batches to a writer task, so the bus is always read promptly; the
//! retention pass runs on its own. Reads come in two shapes: series for
//! charts (raw, or bucketed when long) and — the agent-native part —
//! read-only SQL over a simple view, so any agent can ask its own questions
//! (« combien de fois la porte s'est ouverte cette semaine ? »).

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

type Row = (PointId, Value, u64);

/// A batch for the writer, and whether it re-states known values.
type Batch = (Vec<Row>, bool);

#[derive(Clone, Debug)]
pub struct Options {
    pub path: PathBuf,
    pub retention_days: u32,
}

/// Cheap handle used by the surfaces to read history.
#[derive(Clone, Debug)]
pub struct History {
    store: Arc<Store>,
    counters: Arc<Counters>,
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
    volume: Mutex<Option<(Instant, Volume)>>,
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
    #[serde(flatten)]
    pub volume: Volume,
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
        let counters = Arc::new(Counters::default());
        let (batches, queue) = mpsc::channel::<Batch>(WRITER_QUEUE);
        let writer = tokio::spawn(write_loop(Arc::clone(&store), queue, Arc::clone(&counters)));
        tokio::spawn(prune_loop(
            Arc::clone(&store),
            options.retention_days,
            Arc::clone(&counters),
            cancel.clone(),
        ));
        let source = hub_events(hub);
        let recording = Arc::clone(&counters);
        let recorder = tokio::spawn(async move {
            record(source, batches, &recording, cancel).await;
            let _ = writer.await;
        });
        Ok((Self { store, counters }, recorder))
    }

    #[must_use]
    pub fn from_store(store: Arc<Store>) -> Self {
        Self {
            store,
            counters: Arc::default(),
        }
    }

    /// Counted on demand, at most once a minute: no timer of its own.
    pub async fn growth(&self) -> Result<Growth, HistoryError> {
        let fresh = store::lock(&self.counters.volume)
            .as_ref()
            .filter(|(at, _)| at.elapsed() < VOLUME_TTL)
            .map(|(_, v)| v.clone());
        let volume = if let Some(v) = fresh {
            v
        } else {
            let store = Arc::clone(&self.store);
            let since = now_ms().saturating_sub(DAY_MS);
            let v = blocking(move || {
                let started = Instant::now();
                let (rows_24h, top_24h) = store.volume(since, 10)?;
                Ok(Volume {
                    rows_24h,
                    top_24h,
                    query_ms: started.elapsed().as_secs_f64() * 1000.0,
                })
            })
            .await?;
            *store::lock(&self.counters.volume) = Some((Instant::now(), v.clone()));
            v
        };
        let c = &self.counters;
        Ok(Growth {
            batches_written: c.batches_written.load(Relaxed),
            batches_failed: c.batches_failed.load(Relaxed),
            rows_merged: c.rows_merged.load(Relaxed),
            rows_dropped: c.rows_dropped.load(Relaxed),
            last_error: store::lock(&c.last_error).clone(),
            last_prune: *store::lock(&c.last_prune),
            volume,
        })
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

fn current_state(hub: &Hub) -> Vec<Row> {
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
        .collect()
}

async fn record(
    mut source: HubEvents,
    batches: mpsc::Sender<Batch>,
    counters: &Counters,
    cancel: CancellationToken,
) {
    let _ = batches.send((current_state(&source.hub), true)).await;
    let mut batch = Vec::new();
    let mut flush = tokio::time::interval(BATCH_EVERY);
    loop {
        tokio::select! {
            () = cancel.cancelled() => break,
            event = source.events.recv() => match event {
                Ok(Event::State { point, value, ts }) => {
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
                    if let Err(e) = batches.try_send((current_state(&source.hub), true)) {
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

async fn prune_loop(
    store: Arc<Store>,
    retention_days: u32,
    counters: Arc<Counters>,
    cancel: CancellationToken,
) {
    let mut tick = tokio::time::interval(RETENTION_EVERY);
    loop {
        tokio::select! {
            () = cancel.cancelled() => break,
            _ = tick.tick() => {}
        }
        let cutoff = now_ms().saturating_sub(u64::from(retention_days) * 86_400_000);
        let store = Arc::clone(&store);
        let started = Instant::now();
        match blocking(move || store.prune(cutoff)).await {
            Ok(rows) => {
                let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                *store::lock(&counters.last_prune) = Some(Prune {
                    at: now_ms(),
                    ms,
                    rows,
                });
                if rows > 0 {
                    tracing::info!(removed = rows, ms, "history pruned");
                }
            }
            Err(e) => tracing::warn!(error = %e, "history pruning failed"),
        }
    }
}

#[cfg(test)]
mod tests {
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
