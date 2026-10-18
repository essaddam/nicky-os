//! Petits outils communs : exécuteur, mémoire, quantiles, sortie JSON.

use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, Thread};
use std::time::Instant;

use serde_json::{Value as Json, json};
use turso::{Connection, Value};

pub type R<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

struct Unpark(Thread);
impl Wake for Unpark {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

/// Exécute une future Turso sur le fil courant. Les entrées / sorties du
/// backend `syscall` se terminent pendant le `poll` (`run_io`) : une future
/// en attente est repolluée tout de suite, comme le ferait tokio après le
/// réveil. Le fil ne cède la main qu'après 64 attentes de suite (un autre fil
/// tient la page), pour ne pas monopoliser le seul CPU du conteneur.
pub fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    let waker = Waker::from(Arc::new(Unpark(thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut pending = 0u32;
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        pending += 1;
        if pending % 64 == 0 {
            thread::yield_now();
        }
    }
}

/// RSS courant et pic (Kio), lus dans /proc comme l'étude de référence.
pub fn memory() -> Json {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let get = |key: &str| {
        s.lines()
            .find(|l| l.starts_with(key))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
    };
    json!({ "rss_kib": get("VmRSS:"), "peak_rss_kib": get("VmHWM:") })
}

pub fn emit(v: &Json) {
    println!("{v}");
}

pub fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

/// p50 / p95 / p99 / max d'une série de durées (ms), même règle de rang que
/// l'étude (rang = ceil(n·p)).
pub fn quantiles(times: &mut [f64]) -> Json {
    times.sort_by(f64::total_cmp);
    let q = |p: f64| {
        let i = ((times.len() as f64 * p).ceil() as usize).saturating_sub(1);
        times[i.min(times.len() - 1)]
    };
    json!({
        "iterations": times.len(),
        "p50_ms": q(0.50),
        "p95_ms": q(0.95),
        "p99_ms": q(0.99),
        "max_ms": times[times.len() - 1],
    })
}

/// Ouvre (ou crée) une base Turso locale avec le backend d'entrées / sorties
/// par défaut (appels système, pas d'io_uring).
pub fn open(path: &str) -> R<(turso::Database, Connection)> {
    let db = block_on(turso::Builder::new_local(path).build())?;
    let conn = connect(&db)?;
    Ok((db, conn))
}

/// Une connexion dont la base temporaire reste en mémoire. Par défaut, Turso
/// la crée sur disque dans `TURSO_TMPDIR`, `SQLITE_TMPDIR` ou `/tmp`, dès le
/// premier `BEGIN IMMEDIATE` : l'image `FROM scratch` n'a pas de `/tmp`, et
/// Moli n'a pas de table temporaire à y mettre.
pub fn connect(db: &turso::Database) -> R<Connection> {
    let conn = db.connect()?;
    block_on(conn.execute("PRAGMA temp_store = MEMORY", ()))?;
    Ok(conn)
}

pub fn exec(conn: &Connection, sql: &str) -> R<()> {
    block_on(conn.execute_batch(sql))?;
    Ok(())
}

/// Toutes les lignes d'une requête, valeurs brutes.
pub fn rows(conn: &Connection, sql: &str, params: impl turso::IntoParams) -> R<Vec<Vec<Value>>> {
    block_on(async {
        let mut rows = conn.query(sql, params).await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push((0..row.column_count()).map(|i| row.get_value(i)).collect::<Result<Vec<_>, _>>()?);
        }
        Ok::<_, turso::Error>(out)
    })
    .map_err(Into::into)
}

pub fn int(v: &Value) -> i64 {
    match v {
        Value::Integer(i) => *i,
        Value::Real(f) => *f as i64,
        _ => 0,
    }
}

pub fn real(v: &Value) -> f64 {
    match v {
        Value::Integer(i) => *i as f64,
        Value::Real(f) => *f,
        _ => 0.0,
    }
}

/// Une valeur entière d'une requête à une ligne et une colonne.
pub fn one(conn: &Connection, sql: &str, params: impl turso::IntoParams) -> R<i64> {
    let r = rows(conn, sql, params)?;
    Ok(r.first().and_then(|row| row.first()).map_or(0, int))
}

/// Générateur déterministe (LCG 64 bits) : mêmes données à chaque lancement.
#[derive(Debug, Clone)]
pub struct Lcg(pub u64);
impl Lcg {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

pub fn file_size(path: &str) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

pub fn remove_db(path: &str) {
    for suffix in ["", "-wal", "-shm", "-log"] {
        let _ = std::fs::remove_file(format!("{path}{suffix}"));
    }
}
