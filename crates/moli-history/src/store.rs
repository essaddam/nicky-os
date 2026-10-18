//! SQLite storage. Synchronous: always called from blocking threads.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use moli_core::{PointId, Value};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::limits::Limit;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::Serialize;
use serde_json::Value as Json;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS points (
    id    INTEGER PRIMARY KEY,
    point TEXT NOT NULL UNIQUE
);
CREATE TABLE IF NOT EXISTS samples (
    point_id INTEGER NOT NULL REFERENCES points(id),
    ts       INTEGER NOT NULL,           -- ms since the Unix epoch
    kind     TEXT    NOT NULL,           -- b(ool) i(nt) f(loat) t(ext) z(null)
    num      REAL,                       -- bool as 0/1, int, float
    txt      TEXT,
    PRIMARY KEY (point_id, ts)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS samples_ts ON samples(ts);
-- How long each point has been kept so far, in days: a shorter duration
-- waits for a person (see retention.rs).
CREATE TABLE IF NOT EXISTS retention (
    point_id INTEGER PRIMARY KEY REFERENCES points(id),
    days     INTEGER NOT NULL
);
-- When the durations were set up (baseline) and each reduction a person
-- validated.
CREATE TABLE IF NOT EXISTS retention_log (
    at     INTEGER NOT NULL,
    kind   TEXT    NOT NULL,           -- baseline | validated | suspended | resumed
    who    TEXT,
    points INTEGER NOT NULL,
    rows   INTEGER NOT NULL
);
-- A run is dirty before recording starts, clean only after a successful
-- drain. Recovery must not depend on writing a marker AFTER a disk error.
CREATE TABLE IF NOT EXISTS retention_guard (
    id         INTEGER PRIMARY KEY CHECK(id = 1),
    generation INTEGER NOT NULL,
    suspended  INTEGER NOT NULL,
    clean      INTEGER NOT NULL
);
INSERT INTO retention_guard(id, generation, suspended, clean)
    SELECT 1, 0, COALESCE((SELECT kind = 'suspended' FROM retention_log
        WHERE kind IN ('suspended', 'resumed') ORDER BY rowid DESC LIMIT 1), 0), 1
    WHERE NOT EXISTS(SELECT 1 FROM retention_guard WHERE id = 1);
-- A lower floor invalidates observations made before that consent.
CREATE TABLE IF NOT EXISTS retention_consent (
    point_id   INTEGER PRIMARY KEY REFERENCES points(id),
    generation INTEGER NOT NULL
);
-- What agents query: one row per recorded change.
CREATE VIEW IF NOT EXISTS history AS
    SELECT p.point AS point, s.ts AS ts, s.kind AS kind, s.num AS num, s.txt AS txt
    FROM samples s JOIN points p ON p.id = s.point_id;
";

/// Sets a point's floor, or raises it: never lowers it.
const RAISE_FLOOR: &str = "INSERT INTO retention(point_id, days) VALUES (?1, ?2)
     ON CONFLICT(point_id) DO UPDATE SET days = excluded.days WHERE excluded.days > retention.days";

/// Agents' SQL: time budget per query.
const SQL_BUDGET: Duration = Duration::from_secs(2);
/// Agents' SQL: largest string or blob any expression may build.
const AGENT_MAX_VALUE: i32 = 64 * 1024;
/// Agents' SQL: rough size cap of one answer.
const AGENT_MAX_ANSWER: usize = 1024 * 1024;
/// Same-millisecond samples of one point kept (then the rest is dropped).
const MAX_SAME_MS: usize = 16;

#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    #[error("database: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("only a single read-only statement is accepted")]
    NotReadOnly,
    #[error("query took longer than {}s", SQL_BUDGET.as_secs())]
    TooSlow,
    #[error("{0}")]
    Internal(String),
}

/// One bucket of a downsampled numeric series.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Bucket {
    /// First sample's time in the bucket.
    pub ts: u64,
    pub avg: f64,
    pub min: f64,
    pub max: f64,
    pub count: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Series {
    pub point: String,
    pub from: u64,
    pub to: u64,
    /// Number of changes recorded in the window.
    pub total: u64,
    /// Last value before the window (a step chart starts from it).
    pub before: Option<(u64, Value)>,
    /// First and last changes inside the window (whatever the shape below).
    pub first: Option<(u64, Value)>,
    pub last: Option<(u64, Value)>,
    /// Every change, when there are at most `max_points` of them.
    pub raw: Vec<(u64, Value)>,
    /// Otherwise, numeric points are bucketed…
    pub buckets: Vec<Bucket>,
    /// …and non-numeric ones keep their most recent changes.
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SqlResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Json>>,
    pub truncated: bool,
}

/// A point's floors when one of its rows is written (`Store::record`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Floors {
    /// For a point recorded for the first time.
    pub first: u32,
    /// What the configuration surely asks for an existing one.
    pub certain: u32,
}

/// One point's purge: rows from `oldest` to just before `anchor`, at most
/// `max` of them, while the point's floor is at most `floor` (the duration
/// the cut was decided with: raised since, the cut no longer holds).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cut {
    pub id: i64,
    pub oldest: u64,
    pub anchor: u64,
    pub max: u64,
    pub floor: u32,
    pub generation: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Guard {
    pub generation: i64,
    pub suspended: bool,
}

pub(crate) type Consents = HashMap<String, i64>;

/// The maximum observed within one generation of a point's consent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Observed {
    pub days: u32,
    pub generation: i64,
}

/// What a purge would delete for a point.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Doomed {
    /// The sample kept: the last one before the cutoff.
    pub anchor: u64,
    pub rows: u64,
    pub oldest: Option<u64>,
}

/// A reduction a person validated.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Validation {
    pub at: u64,
    pub by: Option<String>,
    pub points: u64,
    pub rows: u64,
}

pub struct Store {
    path: PathBuf,
    writer: Mutex<Connection>,
    reader: Mutex<Connection>,
    agents: Mutex<Connection>,
    ids: Mutex<HashMap<String, i64>>,
    /// If even the suspension transaction fails, no deletion or validation
    /// can proceed in this process. The dirty run protects the next one.
    suspension_failed: AtomicBool,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// (kind, num, txt) for storage.
fn encode(value: &Value) -> (&'static str, Option<f64>, Option<&str>) {
    #[allow(clippy::cast_precision_loss)]
    match value {
        Value::Null => ("z", None, None),
        Value::Bool(b) => ("b", Some(f64::from(u8::from(*b))), None),
        Value::Int(i) => ("i", Some(*i as f64), None),
        Value::Float(f) => ("f", Some(*f), None),
        Value::Text(t) => ("t", None, Some(t)),
    }
}

fn decode(kind: &str, num: Option<f64>, txt: Option<String>) -> Value {
    #[allow(clippy::cast_possible_truncation)]
    match (kind, num, txt) {
        ("b", Some(n), _) => Value::Bool(n != 0.0),
        ("i", Some(n), _) => Value::Int(n as i64),
        ("f", Some(n), _) => Value::Float(n),
        ("t", _, Some(t)) => Value::Text(t.into()),
        _ => Value::Null,
    }
}

#[allow(clippy::cast_sign_loss)]
fn ms(ts: i64) -> u64 {
    ts.max(0) as u64
}

fn ts(ms: u64) -> i64 {
    i64::try_from(ms).unwrap_or(i64::MAX)
}

fn read_guard(conn: &Connection) -> Result<Guard, rusqlite::Error> {
    conn.query_row(
        "SELECT generation, suspended FROM retention_guard WHERE id = 1",
        [],
        |r| {
            Ok(Guard {
                generation: r.get(0)?,
                suspended: r.get(1)?,
            })
        },
    )
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, HistoryError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| HistoryError::Internal(e.to_string()))?;
        }
        let writer = Connection::open(path)?;
        writer.pragma_update(None, "journal_mode", "WAL")?;
        writer.pragma_update(None, "synchronous", "NORMAL")?;
        writer.busy_timeout(Duration::from_secs(5))?;
        writer.execute_batch(SCHEMA)?;

        let read_only = || -> Result<Connection, HistoryError> {
            let conn = Connection::open_with_flags(
                path,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?;
            conn.busy_timeout(Duration::from_secs(5))?;
            Ok(conn)
        };
        let reader = read_only()?;
        let agents = read_only()?;
        agents.pragma_update(None, "query_only", true)?;
        // Agents read data; they never attach files, change settings, or
        // hold transactions open (a pinned read snapshot would stall the WAL).
        agents.authorizer(Some(|ctx: AuthContext<'_>| match ctx.action {
            AuthAction::Attach { .. }
            | AuthAction::Detach { .. }
            | AuthAction::Pragma { .. }
            | AuthAction::Transaction { .. }
            | AuthAction::Savepoint { .. } => Authorization::Deny,
            _ => Authorization::Allow,
        }))?;
        // No giant strings or blobs (printf('%.*c', 1e9, 'x'), zeroblob…).
        agents.set_limit(Limit::SQLITE_LIMIT_LENGTH, AGENT_MAX_VALUE)?;
        Ok(Self {
            path: path.to_path_buf(),
            writer: Mutex::new(writer),
            reader: Mutex::new(reader),
            agents: Mutex::new(agents),
            ids: Mutex::new(HashMap::new()),
            suspension_failed: AtomicBool::new(false),
        })
    }

    /// Records changes. `restate`: these rows re-state values already known
    /// (current state after a restart or a lag) — an identical sample at the
    /// same instant is then skipped instead of counted twice.
    pub fn insert(
        &self,
        rows: &[(PointId, Value, u64)],
        restate: bool,
    ) -> Result<(), HistoryError> {
        self.record(rows, restate, None)
    }

    /// `insert`, keeping each point's retention floor up to date in the
    /// same transaction (`floors`): a point recorded for the first time gets
    /// its `first` floor, an existing one is raised to what the
    /// configuration surely asks (`certain`), never lowered. A point never
    /// exists without a floor, and an increase is kept as soon as a row of
    /// the point is written, whatever happens before the next pass.
    pub fn record(
        &self,
        rows: &[(PointId, Value, u64)],
        restate: bool,
        floors: Option<&dyn Fn(&str) -> Floors>,
    ) -> Result<(), HistoryError> {
        let mut conn = lock(&self.writer);
        let tx = conn.transaction()?;
        // Ids created here are known to the cache only once committed: a
        // rolled-back id may be handed to another point.
        let mut fresh: HashMap<String, i64> = HashMap::new();
        {
            let ids = lock(&self.ids);
            let mut create = tx.prepare(
                "INSERT INTO points(point) VALUES (?1) ON CONFLICT(point) DO NOTHING RETURNING id",
            )?;
            let mut known = tx.prepare("SELECT id FROM points WHERE point = ?1")?;
            let mut raise = tx.prepare(RAISE_FLOOR)?;
            // Two changes in the same millisecond (two button presses…) are
            // two samples: a collision moves the later one by 1 ms.
            let mut sample = tx.prepare(
                "INSERT INTO samples(point_id, ts, kind, num, txt) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(point_id, ts) DO NOTHING",
            )?;
            let mut created = std::collections::HashSet::new();
            let mut floored = std::collections::HashSet::new();
            for (point, value, at) in rows {
                let cached = ids
                    .get(point.as_str())
                    .or_else(|| fresh.get(point.as_str()));
                let id = if let Some(id) = cached {
                    *id
                } else {
                    let new: Option<i64> = create
                        .query_row([point.as_str()], |r| r.get(0))
                        .optional()?;
                    let id = match new {
                        Some(id) => {
                            created.insert(id);
                            id
                        }
                        None => known.query_row([point.as_str()], |r| r.get(0))?,
                    };
                    fresh.insert(point.as_str().to_owned(), id);
                    id
                };
                if let Some(floors) = floors
                    && floored.insert(id)
                {
                    let f = floors(point.as_str());
                    let days = if created.contains(&id) {
                        f.first
                    } else {
                        f.certain
                    };
                    raise.execute(params![id, days])?;
                }
                let (kind, num, txt) = encode(value);
                for at in (ts(*at)..).take(MAX_SAME_MS) {
                    if sample.execute(params![id, at, kind, num, txt])? == 1 || restate {
                        break;
                    }
                }
            }
        }
        tx.commit()?;
        lock(&self.ids).extend(fresh);
        Ok(())
    }

    /// Every recorded point: (id, point).
    pub fn points(&self) -> Result<Vec<(i64, String)>, HistoryError> {
        let conn = lock(&self.reader);
        let mut stmt = conn.prepare("SELECT id, point FROM points")?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    /// The point's last sample before `cutoff`: the one a purge at that
    /// cutoff keeps (a chart starting there still knows its first value, a
    /// value stable for months is still known), everything older goes.
    pub fn anchor(&self, id: i64, cutoff: u64) -> Result<Option<u64>, HistoryError> {
        let at: Option<i64> = lock(&self.reader).query_row(
            "SELECT MAX(ts) FROM samples WHERE point_id = ?1 AND ts < ?2",
            params![id, ts(cutoff)],
            |r| r.get(0),
        )?;
        Ok(at.map(ms))
    }

    /// What a purge at `cutoff` deletes for this point: the anchor, the rows
    /// before it and the oldest one's time.
    pub fn doomed(&self, id: i64, cutoff: u64) -> Result<Option<Doomed>, HistoryError> {
        let Some(anchor) = self.anchor(id, cutoff)? else {
            return Ok(None);
        };
        let (rows, oldest): (i64, Option<i64>) = lock(&self.reader).query_row(
            "SELECT COUNT(*), MIN(ts) FROM samples WHERE point_id = ?1 AND ts < ?2",
            params![id, ts(anchor)],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok(Some(Doomed {
            anchor,
            rows: u64::try_from(rows).unwrap_or(0),
            oldest: oldest.map(ms),
        }))
    }

    /// For each cut, deletes the point's samples from `oldest` to just
    /// before its anchor, the oldest first and never more than its `max`:
    /// bounds fixed when the purge was decided, so a write arriving
    /// meanwhile never widens it. Each lot checks, in its own statement, that
    /// the point's floor was not raised above the cut's: if it was, nothing
    /// more goes. At
    /// most `lot` rows per statement, the writer released in between
    /// (`between` runs then): the recorder's batches pass between two lots.
    /// Returns how many.
    pub fn prune(
        &self,
        cuts: &[Cut],
        lot: usize,
        mut between: impl FnMut(),
    ) -> Result<usize, HistoryError> {
        let lot = u64::try_from(lot.max(1)).unwrap_or(u64::MAX);
        let mut removed = 0;
        for cut in cuts {
            let mut left = cut.max;
            while left > 0 {
                let limit = lot.min(left);
                // Through the primary key (point_id, ts) only; portable SQL
                // (no DELETE … LIMIT).
                let conn = lock(&self.writer);
                if self.suspension_failed.load(SeqCst) {
                    break;
                }
                let n = conn.execute(
                    "DELETE FROM samples WHERE point_id = ?1 AND ts IN (
                        SELECT ts FROM samples WHERE point_id = ?1 AND ts >= ?2 AND ts < ?3
                        AND COALESCE((SELECT days FROM retention WHERE point_id = ?1), 0) <= ?5
                        AND (SELECT suspended FROM retention_guard WHERE id = 1) = 0
                        AND (SELECT generation FROM retention_guard WHERE id = 1) = ?6
                        ORDER BY ts LIMIT ?4)",
                    params![
                        cut.id,
                        ts(cut.oldest),
                        ts(cut.anchor),
                        i64::try_from(limit).unwrap_or(i64::MAX),
                        cut.floor,
                        cut.generation
                    ],
                )?;
                drop(conn);
                removed += n;
                left = left.saturating_sub(n as u64);
                if (n as u64) < limit {
                    break;
                }
                if left > 0 {
                    between();
                }
            }
        }
        Ok(removed)
    }

    /// How long each point has been kept so far (point id → days); `None`
    /// before the durations were ever set up.
    pub fn floors(&self) -> Result<Option<HashMap<i64, u32>>, HistoryError> {
        let conn = lock(&self.reader);
        let set_up: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM retention_log WHERE kind = 'baseline')",
            [],
            |r| r.get(0),
        )?;
        if !set_up {
            return Ok(None);
        }
        let mut stmt = conn.prepare("SELECT point_id, days FROM retention")?;
        let floors = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        Ok(Some(floors))
    }

    /// The first time: every point recorded so far was kept `days` days
    /// (Moli before any policy).
    pub fn set_up_floors(&self, days: u32, at: u64) -> Result<(), HistoryError> {
        let mut conn = lock(&self.writer);
        let tx = conn.transaction()?;
        let points = tx.execute(
            "INSERT OR IGNORE INTO retention(point_id, days) SELECT id, ?1 FROM points",
            [days],
        )?;
        tx.execute(
            "INSERT INTO retention_log(at, kind, points, rows) VALUES (?1, 'baseline', ?2, 0)",
            params![ts(at), i64::try_from(points).unwrap_or(i64::MAX)],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Raises points' floors (a floor missing is set): never lowers one.
    pub fn raise_floors(&self, floors: &[(i64, u32)]) -> Result<(), HistoryError> {
        let mut conn = lock(&self.writer);
        let tx = conn.transaction()?;
        {
            let mut raise = tx.prepare(RAISE_FLOOR)?;
            for (id, days) in floors {
                raise.execute(params![id, days])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Observed maxima survive a changing family until a newer consent.
    /// Check that consent inside the writer, including for extracted lots.
    /// Current certain durations always apply, even after a consent.
    pub(crate) fn raise_named_floors(
        &self,
        points: &[(String, Observed)],
        days: &dyn Fn(&str) -> u32,
    ) -> Result<(), HistoryError> {
        let mut conn = lock(&self.writer);
        let tx = conn.transaction()?;
        {
            let mut raise = tx.prepare(
                "UPDATE retention SET days = ?2
                 WHERE point_id = (SELECT id FROM points WHERE point = ?1) AND days < ?2",
            )?;
            let mut consent = tx.prepare(
                "SELECT COALESCE((SELECT generation FROM retention_consent
                 WHERE point_id = (SELECT id FROM points WHERE point = ?1)), 0)",
            )?;
            for (point, observed) in points {
                let generation: i64 = consent.query_row([point], |r| r.get(0))?;
                let current = days(point);
                let keep = if generation == observed.generation {
                    current.max(observed.days)
                } else {
                    current
                };
                raise.execute(params![point, keep])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Open the recorder's run before any task can observe a higher duration.
    /// An unfinished predecessor requires a person's consent to resume.
    pub(crate) fn open_run(&self) -> Result<(), HistoryError> {
        let mut conn = lock(&self.writer);
        let tx = conn.transaction()?;
        let clean: bool =
            tx.query_row("SELECT clean FROM retention_guard WHERE id = 1", [], |r| {
                r.get(0)
            })?;
        if !clean {
            tx.execute("UPDATE retention_guard SET suspended = 1, generation = generation + 1 WHERE id = 1", [])?;
            tx.execute(
                "INSERT INTO retention_log(at, kind, points, rows) VALUES (?1, 'suspended', 0, 0)",
                [ts(moli_core::now_ms())],
            )?;
        }
        tx.execute("UPDATE retention_guard SET clean = 0 WHERE id = 1", [])?;
        tx.commit()?;
        Ok(())
    }

    /// Called only after the recorder, writer, raises and purge have ended.
    pub(crate) fn close_run(&self) -> Result<(), HistoryError> {
        let conn = lock(&self.writer);
        self.check_suspension_write()?;
        conn.execute("UPDATE retention_guard SET clean = 1 WHERE id = 1", [])?;
        Ok(())
    }

    fn check_suspension_write(&self) -> Result<(), HistoryError> {
        if self.suspension_failed.load(SeqCst) {
            return Err(HistoryError::Internal(moli_i18n::tr!(
                "serveur.historique.reprise_redemarrage"
            )));
        }
        Ok(())
    }

    pub(crate) fn guard(&self) -> Result<Guard, HistoryError> {
        let mut guard = read_guard(&lock(&self.reader))?;
        guard.suspended |= self.suspension_failed.load(SeqCst);
        Ok(guard)
    }

    pub(crate) fn consents(&self) -> Result<Consents, HistoryError> {
        let conn = lock(&self.reader);
        let mut stmt = conn.prepare("SELECT p.point, c.generation FROM retention_consent c JOIN points p ON p.id = c.point_id")?;
        Ok(stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?)
    }

    /// Both the durable transition and its fallback share the DELETE writer.
    pub fn suspend(&self, at: u64, points: usize) -> Result<(), HistoryError> {
        let mut conn = lock(&self.writer);
        self.suspension_failed.store(true, SeqCst);
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE retention_guard SET suspended = 1, generation = generation + 1 WHERE id = 1",
            [],
        )?;
        tx.execute(
            "INSERT INTO retention_log(at, kind, points, rows) VALUES (?1, 'suspended', ?2, 0)",
            params![ts(at), i64::try_from(points).unwrap_or(i64::MAX)],
        )?;
        tx.commit()?;
        self.suspension_failed.store(false, SeqCst);
        Ok(())
    }

    /// The database is authoritative; a failed suspension write also blocks.
    pub fn suspended(&self) -> Result<bool, HistoryError> {
        Ok(self.guard()?.suspended)
    }

    /// Apply the whole plan atomically: floors (including certain raises),
    /// point consent generations and the guard generation being resumed.
    /// None means a stale plan; otherwise return committed consent updates.
    pub(crate) fn validate_floors(
        &self,
        floors: &[(i64, u32, u32)],
        at: u64,
        who: Option<&str>,
        rows: u64,
        guard: Guard,
    ) -> Result<Option<Consents>, HistoryError> {
        let mut conn = lock(&self.writer);
        self.check_suspension_write()?;
        let tx = conn.transaction()?;
        if read_guard(&tx)? != guard {
            return Ok(None);
        }
        let mut consents = Consents::new();
        {
            let mut lower =
                tx.prepare("UPDATE retention SET days = ?3 WHERE point_id = ?1 AND days = ?2")?;
            for (id, from, to) in floors {
                if lower.execute(params![id, from, to])? != 1 {
                    return Ok(None);
                }
                if to < from {
                    tx.execute("INSERT INTO retention_consent(point_id, generation) VALUES (?1, 1)
                        ON CONFLICT(point_id) DO UPDATE SET generation = retention_consent.generation + 1", [id])?;
                    let (point, generation) = tx.query_row(
                        "SELECT p.point, c.generation FROM points p
                        JOIN retention_consent c ON c.point_id = p.id WHERE p.id = ?1",
                        [id],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )?;
                    consents.insert(point, generation);
                }
            }
        }
        tx.execute(
            "INSERT INTO retention_log(at, kind, who, points, rows) VALUES (?1, 'validated', ?2, ?3, ?4)",
            params![
                ts(at),
                who,
                i64::try_from(floors.len()).unwrap_or(i64::MAX),
                i64::try_from(rows).unwrap_or(i64::MAX)
            ],
        )?;
        if guard.suspended {
            tx.execute("UPDATE retention_guard SET suspended = 0 WHERE id = 1", [])?;
            tx.execute(
                "INSERT INTO retention_log(at, kind, who, points, rows) VALUES (?1, 'resumed', ?2, 0, 0)",
                params![ts(at), who],
            )?;
        }
        tx.commit()?;
        Ok(Some(consents))
    }

    /// The last reduction a person validated: (when, who, points, rows).
    pub fn last_validation(&self) -> Result<Option<Validation>, HistoryError> {
        Ok(lock(&self.reader)
            .query_row(
                "SELECT at, who, points, rows FROM retention_log WHERE kind = 'validated'
                 ORDER BY at DESC LIMIT 1",
                [],
                |r| {
                    Ok(Validation {
                        at: ms(r.get(0)?),
                        by: r.get(1)?,
                        points: u64::try_from(r.get::<_, i64>(2)?).unwrap_or(0),
                        rows: u64::try_from(r.get::<_, i64>(3)?).unwrap_or(0),
                    })
                },
            )
            .optional()?)
    }

    pub fn series(
        &self,
        point: &str,
        from: u64,
        to: u64,
        max_points: usize,
    ) -> Result<Series, HistoryError> {
        let conn = lock(&self.reader);
        let mut series = Series {
            point: point.to_owned(),
            from,
            to,
            ..Series::default()
        };
        let Some(id) = conn
            .query_row("SELECT id FROM points WHERE point = ?1", [point], |r| {
                r.get::<_, i64>(0)
            })
            .optional()?
        else {
            return Ok(series);
        };
        let (from_ts, to_ts) = (ts(from), ts(to));
        let row = |r: &rusqlite::Row<'_>| -> rusqlite::Result<(u64, Value)> {
            Ok((
                ms(r.get(0)?),
                decode(&r.get::<_, String>(1)?, r.get(2)?, r.get(3)?),
            ))
        };
        series.before = conn
            .query_row(
                "SELECT ts, kind, num, txt FROM samples WHERE point_id = ?1 AND ts < ?2 ORDER BY ts DESC LIMIT 1",
                params![id, from_ts],
                row,
            )
            .optional()?;
        let (total, numeric): (i64, i64) = conn.query_row(
            "SELECT COUNT(*), COUNT(num) FROM samples WHERE point_id = ?1 AND ts BETWEEN ?2 AND ?3",
            params![id, from_ts, to_ts],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        series.total = u64::try_from(total).unwrap_or(0);
        let max = i64::try_from(max_points.max(1)).unwrap_or(i64::MAX);
        let edge = |order: &str| {
            conn.query_row(
                &format!(
                    "SELECT ts, kind, num, txt FROM samples WHERE point_id = ?1 AND ts BETWEEN ?2 AND ?3 ORDER BY ts {order} LIMIT 1"
                ),
                params![id, from_ts, to_ts],
                row,
            )
            .optional()
        };
        series.first = edge("ASC")?;
        series.last = edge("DESC")?;

        if total <= max {
            let mut stmt = conn.prepare(
                "SELECT ts, kind, num, txt FROM samples WHERE point_id = ?1 AND ts BETWEEN ?2 AND ?3 ORDER BY ts",
            )?;
            series.raw = stmt
                .query_map(params![id, from_ts, to_ts], row)?
                .collect::<Result<_, _>>()?;
        } else if numeric > 0 {
            // Numeric points are bucketed even if some readings were null
            // (device offline a moment): nulls simply do not count.
            let width = ((to_ts - from_ts) / max).max(1);
            let mut stmt = conn.prepare(
                "SELECT MIN(ts), AVG(num), MIN(num), MAX(num), COUNT(*) FROM samples
                 WHERE point_id = ?1 AND ts BETWEEN ?2 AND ?3 AND num IS NOT NULL
                 GROUP BY (ts - ?2) / ?4 ORDER BY 1",
            )?;
            series.buckets = stmt
                .query_map(params![id, from_ts, to_ts, width], |r| {
                    Ok(Bucket {
                        ts: ms(r.get(0)?),
                        avg: r.get(1)?,
                        min: r.get(2)?,
                        max: r.get(3)?,
                        count: u64::try_from(r.get::<_, i64>(4)?).unwrap_or(0),
                    })
                })?
                .collect::<Result<_, _>>()?;
        } else {
            let mut stmt = conn.prepare(
                "SELECT ts, kind, num, txt FROM samples WHERE point_id = ?1 AND ts BETWEEN ?2 AND ?3
                 ORDER BY ts DESC LIMIT ?4",
            )?;
            let mut raw: Vec<_> = stmt
                .query_map(params![id, from_ts, to_ts, max], row)?
                .collect::<Result<_, _>>()?;
            raw.reverse();
            series.raw = raw;
            series.truncated = true;
        }
        Ok(series)
    }

    /// Rows recorded since `since_ms`, and the `top` points that wrote the
    /// most of them. One range of the primary key per point: no full scan
    /// (a `GROUP BY point_id` over `ts >= ?` walks the whole table).
    pub fn volume(
        &self,
        since_ms: u64,
        top: usize,
    ) -> Result<(u64, Vec<(String, u64)>), HistoryError> {
        let conn = lock(&self.reader);
        let mut stmt = conn.prepare(
            "SELECT p.point,
                    (SELECT COUNT(*) FROM samples s WHERE s.point_id = p.id AND s.ts >= ?1)
             FROM points p",
        )?;
        let mut counts: Vec<(String, u64)> = stmt
            .query_map([ts(since_ms)], |r| {
                Ok((r.get(0)?, u64::try_from(r.get::<_, i64>(1)?).unwrap_or(0)))
            })?
            .collect::<Result<_, _>>()?;
        let total = counts.iter().map(|(_, n)| n).sum();
        counts.retain(|(_, n)| *n > 0);
        counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        counts.truncate(top);
        Ok((total, counts))
    }

    /// Runs an agent's query: one read-only statement, bounded in time and rows.
    pub fn query_sql(&self, sql: &str, max_rows: usize) -> Result<SqlResult, HistoryError> {
        let conn = lock(&self.agents);
        let started = Instant::now();
        conn.progress_handler(10_000, Some(move || started.elapsed() > SQL_BUDGET))?;
        let result = run_sql(&conn, sql, max_rows);
        conn.progress_handler(0, None::<fn() -> bool>)?;
        // Belt and braces: never leave a transaction (and its snapshot) open.
        if !conn.is_autocommit() {
            let _ = conn.execute_batch("ROLLBACK");
        }
        result.map_err(|e| match e {
            HistoryError::Db(rusqlite::Error::SqliteFailure(f, _))
                if f.code == rusqlite::ErrorCode::OperationInterrupted =>
            {
                HistoryError::TooSlow
            }
            other => other,
        })
    }
}

fn run_sql(conn: &Connection, sql: &str, max_rows: usize) -> Result<SqlResult, HistoryError> {
    let mut stmt = conn.prepare(sql.trim().trim_end_matches(';'))?;
    if !stmt.readonly() {
        return Err(HistoryError::NotReadOnly);
    }
    let columns: Vec<String> = stmt
        .column_names()
        .iter()
        .map(|c| (*c).to_owned())
        .collect();
    let width = columns.len();
    let mut rows = Vec::new();
    let mut truncated = false;
    let mut bytes = 0usize;
    let mut cursor = stmt.query([])?;
    while let Some(row) = cursor.next()? {
        if rows.len() == max_rows || bytes > AGENT_MAX_ANSWER {
            truncated = true;
            break;
        }
        let mut out = Vec::with_capacity(width);
        for i in 0..width {
            out.push(match row.get_ref(i)? {
                ValueRef::Null => Json::Null,
                ValueRef::Integer(n) => n.into(),
                ValueRef::Real(f) => {
                    serde_json::Number::from_f64(f).map_or(Json::Null, Json::Number)
                }
                ValueRef::Text(t) => {
                    bytes += t.len();
                    String::from_utf8_lossy(t).into_owned().into()
                }
                ValueRef::Blob(b) => format!("<{} bytes>", b.len()).into(),
            });
            bytes += 8;
        }
        rows.push(out);
    }
    Ok(SqlResult {
        columns,
        rows,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(name: &str) -> (Store, PathBuf) {
        let dir = std::env::temp_dir().join(format!("moli-history-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("history.db");
        (Store::open(&path).unwrap(), dir)
    }

    fn p(s: &str) -> PointId {
        PointId::from(s)
    }

    /// Every point pruned at the same cutoff, in lots of 2.
    fn prune_all(store: &Store, cutoff: u64) -> usize {
        let cuts: Vec<_> = store
            .points()
            .unwrap()
            .into_iter()
            .filter_map(|(id, _)| {
                let anchor = store.anchor(id, cutoff).unwrap()?;
                Some(Cut {
                    id,
                    oldest: 0,
                    anchor,
                    max: u64::MAX,
                    floor: u32::MAX,
                    generation: store.guard().unwrap().generation,
                })
            })
            .collect();
        store.prune(&cuts, 2, || {}).unwrap()
    }

    #[test]
    fn values_round_trip_exactly() {
        let (store, dir) = store("types");
        let rows = vec![
            (p("d/on"), Value::Bool(true), 1_000),
            (p("d/on"), Value::Bool(false), 2_000),
            (p("d/n"), Value::Int(42), 1_000),
            (p("d/f"), Value::Float(21.5), 1_000),
            (p("d/t"), Value::from("short_release"), 1_000),
            (p("d/z"), Value::Null, 1_000),
        ];
        store.insert(&rows, false).unwrap();
        let s = store.series("d/on", 0, 10_000, 100).unwrap();
        assert_eq!(
            s.raw,
            vec![(1_000, Value::Bool(true)), (2_000, Value::Bool(false))]
        );
        assert_eq!(
            store.series("d/n", 0, 10_000, 100).unwrap().raw[0].1,
            Value::Int(42)
        );
        assert_eq!(
            store.series("d/t", 0, 10_000, 100).unwrap().raw[0].1,
            Value::from("short_release")
        );
        assert_eq!(
            store.series("d/z", 0, 10_000, 100).unwrap().raw[0].1,
            Value::Null
        );
        assert_eq!(store.series("unknown", 0, 10_000, 100).unwrap().total, 0);
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn long_numeric_series_are_bucketed_with_a_starting_value() {
        let (store, dir) = store("buckets");
        let rows: Vec<_> = (0..1_000u32)
            .map(|i| {
                (
                    p("d/power"),
                    Value::Float(f64::from(i)),
                    10_000 + u64::from(i) * 10,
                )
            })
            .collect();
        store.insert(&rows, false).unwrap();
        store
            .insert(&[(p("d/power"), Value::Float(-1.0), 5_000)], false)
            .unwrap();
        let s = store.series("d/power", 10_000, 20_000, 100).unwrap();
        assert_eq!(s.total, 1_000);
        assert!(s.raw.is_empty());
        assert!(
            s.buckets.len() <= 101 && s.buckets.len() >= 99,
            "{}",
            s.buckets.len()
        );
        assert_eq!(s.buckets[0].min, 0.0);
        assert_eq!(s.before, Some((5_000, Value::Float(-1.0))));
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn same_millisecond_changes_are_all_kept_and_restatements_are_not() {
        let (store, dir) = store("same-ms");
        let press = || (p("remote/button1"), Value::from("short_release"), 5_000);
        store.insert(&[press(), press()], false).unwrap();
        assert_eq!(
            store.series("remote/button1", 0, 10_000, 10).unwrap().total,
            2
        );
        // Re-recording the current state (after a restart or a lag) adds nothing.
        store.insert(&[press()], true).unwrap();
        assert_eq!(
            store.series("remote/button1", 0, 10_000, 10).unwrap().total,
            2
        );
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn nulls_do_not_prevent_bucketing() {
        let (store, dir) = store("nulls");
        let mut rows: Vec<_> = (0..500u32)
            .map(|i| (p("d/t"), Value::Float(f64::from(i)), 1_000 + u64::from(i)))
            .collect();
        rows.push((p("d/t"), Value::Null, 1_250));
        store.insert(&rows, false).unwrap();
        let s = store.series("d/t", 0, 10_000, 50).unwrap();
        assert!(!s.buckets.is_empty());
        assert_eq!(s.first.map(|f| f.1), Some(Value::Float(0.0)));
        assert_eq!(s.last.map(|l| l.1), Some(Value::Float(499.0)));
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn prune_removes_old_samples() {
        let (store, dir) = store("prune");
        let rows = [
            (p("d/x"), Value::Int(1), 1_000),
            (p("d/x"), Value::Int(2), 2_000),
            (p("d/x"), Value::Int(3), 9_000),
            (p("d/stable"), Value::Int(7), 1_000),
        ];
        store.insert(&rows, false).unwrap();
        assert_eq!(prune_all(&store, 5_000), 1);
        // 2 s stays: the value the window starts from.
        assert_eq!(store.series("d/x", 0, 10_000, 10).unwrap().total, 2);
        // A value that never changed is still known, however old.
        let stable = store.series("d/stable", 6_000, 10_000, 10).unwrap();
        assert_eq!(stable.before, Some((1_000, Value::Int(7))));
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn prune_keeps_the_value_just_before_the_cutoff() {
        let (store, dir) = store("prune-anchor");
        let rows = [
            (p("d/x"), Value::Int(1), 1_000),
            (p("d/x"), Value::Int(6), 6_000),
        ];
        store.insert(&rows, false).unwrap();
        assert_eq!(prune_all(&store, 5_000), 0);
        // A chart starting at the cutoff still knows where the curve was.
        let s = store.series("d/x", 5_000, 10_000, 10).unwrap();
        assert_eq!(s.before, Some((1_000, Value::Int(1))));
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn pruning_goes_by_lots_and_writes_pass_in_between() {
        let (store, dir) = store("prune-lots");
        let old: Vec<_> = (0..100u64)
            .map(|i| (p("d/x"), Value::Int(1), 1_000 + i))
            .collect();
        store.insert(&old, false).unwrap();
        let (id, _) = store.points().unwrap()[0].clone();
        // What the purge will delete: every old row but the anchor.
        let doomed = store.doomed(id, 50_000).unwrap().unwrap();
        assert_eq!(
            (doomed.anchor, doomed.rows, doomed.oldest),
            (1_099, 99, Some(1_000))
        );
        let cut = Cut {
            id,
            oldest: doomed.oldest.unwrap(),
            anchor: doomed.anchor,
            max: doomed.rows,
            floor: u32::MAX,
            generation: store.guard().unwrap().generation,
        };
        let mut lots = 0;
        let removed = store
            .prune(&[cut], 10, || {
                // The writer is free: a recorder's batch goes in now.
                lots += 1;
                assert!(store.writer.try_lock().is_ok(), "writer held between lots");
                store
                    .insert(
                        &[(p("d/live"), Value::Int(lots), 60_000 + lots.unsigned_abs())],
                        false,
                    )
                    .unwrap();
            })
            .unwrap();
        assert_eq!(removed, 99);
        assert_eq!(lots, 9, "99 rows in lots of 10: 9 full lots, then the rest");
        assert_eq!(store.series("d/live", 0, 100_000, 100).unwrap().total, 9);
        assert_eq!(
            store.series("d/x", 0, 100_000, 100).unwrap().raw,
            vec![(1_099, Value::Int(1))]
        );
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_write_between_lots_never_widens_the_purge() {
        let (store, dir) = store("prune-fixed");
        let rows: Vec<_> = (10..=30u64).map(|i| (p("d/x"), Value::Int(1), i)).collect();
        store.insert(&rows, false).unwrap();
        let (id, _) = store.points().unwrap()[0].clone();
        let doomed = store.doomed(id, 100).unwrap().unwrap();
        assert_eq!(
            (doomed.oldest, doomed.anchor, doomed.rows),
            (Some(10), 30, 20)
        );
        let cut = Cut {
            id,
            oldest: 10,
            anchor: doomed.anchor,
            max: doomed.rows,
            floor: u32::MAX,
            generation: store.guard().unwrap().generation,
        };
        // Old readings restated between two lots: newer than the anchor, and
        // older than the oldest row announced.
        let mut late = vec![(p("d/x"), Value::Int(2), 50), (p("d/x"), Value::Int(3), 0)];
        let removed = store
            .prune(&[cut], 5, || {
                if let Some(row) = late.pop() {
                    store.insert(&[row], false).unwrap();
                }
            })
            .unwrap();
        assert_eq!(removed, 20, "exactly what was announced");
        let left: Vec<u64> = store
            .series("d/x", 0, 1_000, 100)
            .unwrap()
            .raw
            .iter()
            .map(|(t, _)| *t)
            .collect();
        assert_eq!(
            left,
            vec![0, 30, 50],
            "nothing outside the period announced"
        );
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_point_gets_its_floor_with_its_first_row() {
        let (store, dir) = store("first-floor");
        store.set_up_floors(30, 1).unwrap();
        let floor = |point: &str| {
            let first = if point == "d/a" { 7 } else { 90 };
            Floors { first, certain: 1 }
        };
        store
            .record(&[(p("d/a"), Value::Int(1), 1)], false, Some(&floor))
            .unwrap();
        store
            .record(
                &[(p("d/b"), Value::Int(1), 1), (p("d/a"), Value::Int(2), 2)],
                false,
                Some(&floor),
            )
            .unwrap();
        let floors = store.floors().unwrap().unwrap();
        let mut got: Vec<u32> = floors.values().copied().collect();
        got.sort_unstable();
        assert_eq!(got, vec![7, 90], "set once, at the first row");
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_raised_floor_stops_a_purge_even_between_lots() {
        let (store, dir) = store("prune-raised");
        store.set_up_floors(30, 1).unwrap();
        let rows: Vec<_> = (0..=20u64).map(|i| (p("d/x"), Value::Int(1), i)).collect();
        let floors = |_: &str| Floors {
            first: 30,
            certain: 30,
        };
        store.record(&rows, false, Some(&floors)).unwrap();
        let (id, _) = store.points().unwrap()[0].clone();
        let cut = Cut {
            id,
            oldest: 0,
            anchor: 20,
            max: u64::MAX,
            floor: 30,
            generation: store.guard().unwrap().generation,
        };
        // Raised before the purge: nothing goes.
        store.raise_floors(&[(id, 90)]).unwrap();
        assert_eq!(store.prune(&[cut], 5, || {}).unwrap(), 0);
        // Raised between two lots: the first lot only.
        let cut = Cut { floor: 90, ..cut };
        let removed = store
            .prune(&[cut], 5, || {
                store.raise_floors(&[(id, 120)]).unwrap();
            })
            .unwrap();
        assert_eq!(removed, 5);
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_suspension_stops_a_purge_between_lots() {
        let (store, dir) = store("prune-suspended");
        store.set_up_floors(30, 1).unwrap();
        let rows: Vec<_> = (0..=20).map(|i| (p("d/x"), Value::Int(1), i)).collect();
        store.insert(&rows, false).unwrap();
        let cut = Cut {
            id: store.points().unwrap()[0].0,
            oldest: 0,
            anchor: 20,
            max: u64::MAX,
            floor: 30,
            generation: store.guard().unwrap().generation,
        };
        let removed = store
            .prune(&[cut], 5, || store.suspend(1, 1).unwrap())
            .unwrap();
        assert_eq!(removed, 5, "only the lot before the suspension");
        assert_eq!(store.series("d/x", 0, 100, 100).unwrap().total, 16);
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_resume_cannot_revive_an_old_cut_or_validate_an_old_guard() {
        let (store, dir) = store("old-guard");
        store.set_up_floors(30, 1).unwrap();
        let rows: Vec<_> = (0..=20).map(|i| (p("d/x"), Value::Int(1), i)).collect();
        store
            .record(
                &rows,
                false,
                Some(&|_| Floors {
                    first: 30,
                    certain: 30,
                }),
            )
            .unwrap();
        let id = store.points().unwrap()[0].0;
        let cut = Cut {
            id,
            oldest: 0,
            anchor: 20,
            max: u64::MAX,
            floor: 30,
            generation: store.guard().unwrap().generation,
        };
        let removed = store
            .prune(&[cut], 5, || {
                store.suspend(1, 1).unwrap();
                let old = store.guard().unwrap();
                store.suspend(2, 1).unwrap();
                assert!(
                    store
                        .validate_floors(&[(id, 30, 3)], 3, None, 0, old)
                        .unwrap()
                        .is_none()
                );
                assert_eq!(store.floors().unwrap().unwrap()[&id], 30);
                assert!(store.suspended().unwrap());
                let current = store.guard().unwrap();
                assert!(
                    store
                        .validate_floors(&[], 4, None, 0, current)
                        .unwrap()
                        .is_some()
                );
            })
            .unwrap();
        assert_eq!(
            removed, 5,
            "the pre-suspension cut remains stale after resuming"
        );
        assert!(!store.suspended().unwrap());
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_existing_suspension_survives_the_guard_schema_upgrade() {
        let (store, dir) = store("guard-migration");
        lock(&store.writer)
            .execute_batch(
                "INSERT INTO retention_log(at, kind, points, rows) VALUES (1, 'suspended', 1, 0);
             DROP TABLE retention_guard;",
            )
            .unwrap();
        drop(store);
        let store = Store::open(&dir.join("history.db")).unwrap();
        assert!(store.suspended().unwrap());
        store.open_run().unwrap();
        assert!(store.suspended().unwrap());
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_rolled_back_point_never_lends_its_id() {
        let (store, dir) = store("rollback");
        store.set_up_floors(30, 1).unwrap();
        let first = |point: &str| Floors {
            first: match point {
                "d/a" => 30,
                "d/b" => 13,
                _ => 1,
            },
            certain: 1,
        };
        // The second point's floor fails: the whole batch is rolled back.
        let other = Connection::open(dir.join("history.db")).unwrap();
        other
            .execute_batch(
                "CREATE TRIGGER boom BEFORE INSERT ON retention WHEN NEW.days = 13
                 BEGIN SELECT RAISE(ABORT, 'boom'); END;",
            )
            .unwrap();
        let failed = store.record(
            &[
                (p("d/a"), Value::Int(1), 111),
                (p("d/b"), Value::Int(1), 222),
            ],
            false,
            Some(&first),
        );
        assert!(failed.is_err());
        other.execute_batch("DROP TRIGGER boom").unwrap();
        // A new point may take the id the rolled-back one had…
        store
            .record(&[(p("d/c"), Value::Int(1), 333)], false, Some(&first))
            .unwrap();
        // …and the first point's next reading is still its own.
        store
            .record(&[(p("d/a"), Value::Int(2), 444)], false, Some(&first))
            .unwrap();
        let ts = |point: &str| -> Vec<u64> {
            let s = store.series(point, 0, 1_000, 10).unwrap();
            s.raw.iter().map(|(t, _)| *t).collect()
        };
        assert_eq!(ts("d/a"), vec![444]);
        assert_eq!(ts("d/c"), vec![333]);
        assert_eq!(ts("d/b"), Vec::<u64>::new());
        let floors = store.floors().unwrap().unwrap();
        let id = |point: &str| {
            store
                .points()
                .unwrap()
                .into_iter()
                .find(|(_, p)| p == point)
                .unwrap()
                .0
        };
        assert_eq!((floors[&id("d/a")], floors[&id("d/c")]), (30, 1));
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn volume_counts_recent_rows_and_the_chattiest_points() {
        let (store, dir) = store("volume");
        let mut rows: Vec<_> = (0..5u64)
            .map(|i| (p("d/busy"), Value::Int(1), 10_000 + i))
            .collect();
        rows.push((p("d/calm"), Value::Int(1), 10_000));
        rows.push((p("d/old"), Value::Int(1), 1_000));
        store.insert(&rows, false).unwrap();
        let (total, top) = store.volume(5_000, 1).unwrap();
        assert_eq!(total, 6);
        assert_eq!(top, vec![("d/busy".to_owned(), 5)]);
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn agents_read_but_never_write() {
        let (store, dir) = store("sql");
        store
            .insert(
                &[
                    (p("door/contact"), Value::Bool(false), 1_000),
                    (p("door/contact"), Value::Bool(true), 2_000),
                ],
                false,
            )
            .unwrap();
        let r = store
            .query_sql(
                "SELECT point, COUNT(*) AS changes FROM history GROUP BY point;",
                10,
            )
            .unwrap();
        assert_eq!(r.columns, ["point", "changes"]);
        assert_eq!(
            r.rows,
            vec![vec![Json::from("door/contact"), Json::from(2)]]
        );

        for forbidden in [
            "DELETE FROM samples",
            "DROP VIEW history",
            "INSERT INTO points(point) VALUES ('x')",
            "ATTACH DATABASE '/etc/passwd' AS x",
            "PRAGMA query_only = 0",
            "SELECT 1; DELETE FROM samples",
            "BEGIN",
            "SAVEPOINT pin",
            "SELECT printf('%.*c', 100000000, 'x')",
            "SELECT zeroblob(100000000)",
        ] {
            assert!(
                store.query_sql(forbidden, 10).is_err(),
                "{forbidden} must be refused"
            );
        }
        assert_eq!(
            store.series("door/contact", 0, 10_000, 10).unwrap().total,
            2,
            "nothing changed"
        );

        let capped = store.query_sql("SELECT ts FROM history", 1).unwrap();
        assert!(capped.truncated);
        let slow = store.query_sql(
            "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT COUNT(*) FROM c",
            10,
        );
        assert!(matches!(slow, Err(HistoryError::TooSlow)), "{slow:?}");
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }
}
