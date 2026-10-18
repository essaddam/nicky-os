//! Ce dont Moli a besoin, prouvé par exécution à la version figée de Turso.
//! Chaque essai écrit une ligne JSON `{"test", "pass", …}` ; `pass` n'est
//! vrai que si le comportement constaté est celui que l'ADR suppose.

use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

use rusqlite::Connection as Sqlite;
use serde_json::{Value as Json, json};
use turso::Value;

use crate::schema::{
    ENERGY_CURRENT, ENERGY_RECORD_UPSERT, HISTORY_CURRENT, HISTORY_POINT_UPSERT, HISTORY_PRUNE,
    HISTORY_SAMPLE_INSERT, OP_ENERGY_HOUR, OP_SAMPLE, ROLLBACK_ENERGY, ROLLBACK_HISTORY, SAMPLE_INSERT, TARGET, TOTAL_UPSERT, ENERGY_UPSERT, hour_key, key, local_day_month,
};
use crate::util::{R, block_on, connect, emit, exec, file_size, int, ms, one, open, real, remove_db, rows, Lcg};

/// Décalage horaire fictif des totaux de jour et de mois dans ces essais.
const OFFSET: i64 = 2 * 3_600_000;

pub fn run(dir: &str) -> R<()> {
    std::fs::create_dir_all(dir)?;
    let mut failed = 0;
    let tests: [(&str, fn(&str) -> R<Json>); 18] = [
        ("rowid_index_dml_upsert_returning", dml),
        ("differential_vs_sqlite_20k_ops", differential),
        ("transactions_commit_rollback", transactions),
        ("temp_directory_requirements", temp_store),
        ("readers_during_write_snapshot", readers_during_write),
        ("mvcc_begin_concurrent", mvcc),
        ("durability_pragmas_checkpoint", durability),
        ("sqlite_file_opened_by_turso", sqlite_to_turso),
        ("turso_file_opened_by_sqlite_both_ways", turso_to_sqlite),
        ("migration_and_rollback_round_trip", migration_round_trip),
        ("sequence_never_reused", sequence),
        ("agent_sql_without_authorizer", agent_sql),
        ("second_process_is_refused", multiprocess),
        ("backup_vacuum_into", vacuum_into),
        ("io_backend_under_default_seccomp", seccomp),
        ("archive_export_retry_immutable", crate::journal::proof_retry),
        ("archive_restore_replay_journal", crate::journal::proof_restore),
        ("migration_batch_boundaries", migration_boundaries),
    ];
    for (name, test) in tests {
        let started = Instant::now();
        let mut out = match test(dir) {
            Ok(v) => v,
            Err(e) => json!({ "pass": false, "error": e.to_string() }),
        };
        if out["pass"] != Json::Bool(true) {
            failed += 1;
        }
        out["test"] = name.into();
        out["ms"] = ms(started).into();
        emit(&out);
    }
    emit(&json!({ "phase": "compat_done", "failed": failed, "turso": "0.8.3" }));
    if failed > 0 { return Err(format!("{failed} essais de compatibilité ont échoué").into()); }
    Ok(())
}

fn fresh(dir: &str, name: &str) -> String {
    let path = format!("{dir}/{name}.db");
    remove_db(&path);
    path
}

fn err_text<T, E: std::fmt::Display>(r: Result<T, E>) -> Json {
    match r {
        Ok(_) => Json::from("ok"),
        Err(e) => Json::from(format!("error: {e}")),
    }
}

fn integrity_turso(conn: &turso::Connection) -> R<String> {
    let r = rows(conn, "PRAGMA integrity_check", ())?;
    Ok(match r.first().and_then(|row| row.first()) {
        Some(Value::Text(t)) => t.clone(),
        other => format!("{other:?}"),
    })
}

fn integrity_sqlite(path: &str) -> R<String> {
    let c = Sqlite::open(path)?;
    Ok(c.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))?)
}

/// Tables rowid, index secondaires, UPDATE / DELETE par index, UPSERT du code
/// actuel, RETURNING, contraintes CHECK.
fn dml(dir: &str) -> R<Json> {
    let path = fresh(dir, "dml");
    let (_db, c) = open(&path)?;
    exec(&c, TARGET)?;
    let upsert = "INSERT INTO catalog (point, family) VALUES (?1, 'power')
                  ON CONFLICT (point) DO UPDATE SET point = excluded.point RETURNING id";
    let id = one(&c, upsert, ("dev/power",))?;
    let again = one(&c, upsert, ("dev/power",))?;
    for i in 0..100i64 {
        block_on(c.execute(SAMPLE_INSERT, (id, 1_000 + i * 10, i + 1, i as f64)))?;
    }
    // Le même instant une seconde fois : ignoré, comme aujourd'hui ; une clé
    // qui ne correspond pas au point et à l'instant : refusée.
    let duplicate = block_on(c.execute(SAMPLE_INSERT, (id, 1_000, 999i64, 0.0)))?;
    let wrong_key = err_text(block_on(c.execute(
        "INSERT INTO samples (id, point_id, ts, seq, kind) VALUES (1, ?1, 5, 1000, 'z')",
        (id,),
    )));
    let (start, end) = (key(id, 0), key(id + 1, 0));
    let updated = block_on(c.execute(
        "UPDATE samples SET num = num * 2 WHERE id >= ?1 AND id < ?2",
        (key(id, 1_000), key(id, 1_500)),
    ))?;
    let deleted = block_on(c.execute("DELETE FROM samples WHERE id >= ?1 AND id < ?2", (start, key(id, 1_200))))?;
    let via_index = one(&c, "SELECT count(*) FROM samples WHERE id >= ?1 AND id < ?2", (start, end))?;
    let via_table = one(&c, "SELECT count(*) FROM samples WHERE seq > 0", ())?;
    let sum = rows(&c, "SELECT sum(num) FROM samples WHERE id >= ?1 AND id < ?2", (start, end))?;
    // 20..49 doubled, 50..99 as is.
    let expected_sum: f64 = (20..50).map(|i| f64::from(i) * 2.0).sum::<f64>() + (50..100).map(f64::from).sum::<f64>();
    let returned = rows(&c, "DELETE FROM samples WHERE id >= ?1 AND id < ?2 RETURNING seq", (key(id, 1_900), end))?;

    // L'UPSERT de `moli-energy` (avec la séquence de la cible), trois relevés,
    // sur la clé rowid de l'heure, puis le même apport sur le total du jour
    // (index unique secondaire).
    let energy = "INSERT INTO energy_hour (id, meter_id, hour, kwh, cost, seq) VALUES ((?2 / 3600000) * 1024 + ?1, ?1, ?2, ?3, ?4, ?5)
        ON CONFLICT (id) DO UPDATE SET
           kwh = kwh + excluded.kwh,
           cost = CASE WHEN excluded.cost IS NULL THEN cost ELSE coalesce(cost, 0) + excluded.cost END,
           seq = excluded.seq
        RETURNING id, kwh, cost";
    let a = rows(&c, energy, (3i64, 0i64, 1.0, Some(0.1), 1i64))?;
    let b = rows(&c, energy, (3i64, 0i64, 0.5, Some(0.05), 2i64))?;
    let d = rows(&c, energy, (3i64, 0i64, 0.5, None::<f64>, 3i64))?;
    for (kwh, cost) in [(1.0, Some(0.1)), (0.5, Some(0.05)), (0.5, None::<f64>)] {
        block_on(c.execute(TOTAL_UPSERT, ("day", 0i64, 3i64, kwh, cost)))?;
    }
    let total = rows(&c, "SELECT kwh, cost FROM energy_total WHERE span = 'day' AND start = 0 AND meter_id = 3", ())?;
    let nothing = block_on(c.execute(
        "INSERT INTO energy_total (span, start, meter_id, kwh, cost) VALUES ('day', 0, 3, 9, 9) ON CONFLICT (span, start, meter_id) DO NOTHING",
        (),
    ))?;
    let repriced = block_on(c.execute(
        "UPDATE energy_total SET cost = ?3 WHERE span = 'day' AND start = ?1 AND meter_id = ?2 AND cost IS NULL AND ?3 IS NOT NULL",
        (0i64, 3i64, 1.0),
    ))?;
    let wrong_hour = err_text(block_on(c.execute(energy, (3i64, 1_000i64, 1.0, None::<f64>, 4i64))));
    let state = "INSERT INTO meter_state (id, meter, last_kwh, last_ts, first_ts) VALUES (?1, ?2, ?3, ?4, ?4)
        ON CONFLICT (meter) DO UPDATE SET last_kwh = excluded.last_kwh, last_ts = excluded.last_ts";
    block_on(c.execute(state, (3i64, "hc", 10.0, 5i64)))?;
    block_on(c.execute(state, (3i64, "hc", 12.0, 9i64)))?;
    let ms_row = rows(&c, "SELECT last_kwh, last_ts, first_ts FROM meter_state WHERE meter = 'hc'", ())?;
    let purged_totals = block_on(c.execute("DELETE FROM energy_total WHERE span = 'day' AND start < ?1", (1i64,)))?;
    let check = err_text(block_on(c.execute(
        "INSERT INTO meta (id, schema_version, installation, next_seq) VALUES (2, 1, 'x', 1)",
        (),
    )));
    let strict = err_text(block_on(c.execute(
        "INSERT INTO outbox (seq, kind, ref) VALUES (1, 'not a number', 1)",
        (),
    )));
    let integrity = integrity_turso(&c)?;
    let last = &d[0];
    let pass = id == again
        && duplicate == 0
        && wrong_key != "ok"
        && updated == 50
        && deleted == 20
        && via_index == 80
        && via_table == 80
        && (real(&sum[0][0]) - expected_sum).abs() < 1e-9
        && returned.len() == 10
        && int(&a[0][0]) == int(&b[0][0])
        && int(&b[0][0]) == int(&last[0])
        && (real(&last[1]) - 2.0).abs() < 1e-12
        && (real(&last[2]) - 0.15).abs() < 1e-12
        && (real(&total[0][0]) - 2.0).abs() < 1e-12
        && (real(&total[0][1]) - 0.15).abs() < 1e-12
        && wrong_hour != "ok"
        && purged_totals == 1
        && nothing == 0
        && repriced == 0
        && real(&ms_row[0][0]) == 12.0
        && int(&ms_row[0][2]) == 5
        && check != "ok"
        && strict != "ok"
        && integrity == "ok";
    Ok(json!({
        "pass": pass,
        "returning_id_stable": id == again,
        "duplicate_instant_changes": duplicate, "wrong_key": wrong_key,
        "update_by_key_range": updated, "delete_by_key_range": deleted,
        "count_by_key_range": via_index, "count_via_table": via_table,
        "delete_returning_rows": returned.len(),
        "energy_upsert_final": [real(&last[1]), real(&last[2])],
        "day_total_upsert_final": [real(&total[0][0]), real(&total[0][1])], "hour_not_aligned": wrong_hour,
        "delete_via_index": purged_totals,
        "do_nothing_changes": nothing, "conditional_update_changes": repriced,
        "check_constraint": check, "strict_type": strict, "integrity": integrity,
    }))
}

/// 20 000 opérations aléatoires (UPSERT, UPDATE, DELETE par plages d'index,
/// transactions validées ou annulées) appliquées à Turso et à SQLite :
/// contenus identiques, puis le fichier Turso passe `integrity_check` de SQLite.
fn differential(dir: &str) -> R<Json> {
    let path = fresh(dir, "differential");
    let (db, c) = open(&path)?;
    let schema = "
        CREATE TABLE e (id INTEGER PRIMARY KEY, meter TEXT NOT NULL, hour INTEGER NOT NULL, kwh REAL NOT NULL, cost REAL);
        CREATE UNIQUE INDEX e_key ON e (meter, hour);
        CREATE INDEX e_hour ON e (hour);
        CREATE TABLE s (id INTEGER PRIMARY KEY, point_id INTEGER NOT NULL, ts INTEGER NOT NULL, seq INTEGER NOT NULL, num REAL);";
    exec(&c, schema)?;
    let lite = Sqlite::open_in_memory()?;
    lite.execute_batch(schema)?;
    let upsert = "INSERT INTO e (meter, hour, kwh, cost) VALUES (?1, ?2, ?3, ?4)
        ON CONFLICT (meter, hour) DO UPDATE SET kwh = kwh + excluded.kwh,
        cost = CASE WHEN excluded.cost IS NULL THEN cost ELSE coalesce(cost, 0) + excluded.cost END";
    let mut rng = Lcg(42);
    let (mut seq, mut tx, mut rolled_back) = (0i64, 0u32, 0u32);
    let ops = 20_000;
    for op in 0..ops {
        if op % 50 == 0 {
            exec(&c, "BEGIN")?;
            lite.execute_batch("BEGIN")?;
        }
        let meter = format!("m{}", rng.below(20));
        let hour = rng.below(500) as i64;
        let point = rng.below(50) as i64;
        let ts = rng.below(100_000) as i64;
        match rng.below(10) {
            0..=3 => {
                let kwh = rng.below(1000) as f64 / 1000.0;
                let cost = (rng.below(3) > 0).then(|| rng.below(100) as f64 / 100.0);
                block_on(c.execute(upsert, (meter.as_str(), hour, kwh, cost)))?;
                lite.execute(upsert, rusqlite::params![meter, hour, kwh, cost])?;
            }
            4 => {
                let q = "UPDATE e SET cost = ?3 WHERE meter = ?1 AND hour = ?2 AND cost IS NULL";
                block_on(c.execute(q, (meter.as_str(), hour, 0.5)))?;
                lite.execute(q, rusqlite::params![meter, hour, 0.5])?;
            }
            5 => {
                let q = "DELETE FROM e WHERE hour >= ?1 AND hour < ?2";
                block_on(c.execute(q, (hour, hour + 3)))?;
                lite.execute(q, rusqlite::params![hour, hour + 3])?;
            }
            6..=7 => {
                seq += 1;
                let q = "INSERT INTO s (id, point_id, ts, seq, num) VALUES ((?1 << 44) + ?2, ?1, ?2, ?3, ?4) ON CONFLICT (id) DO NOTHING";
                block_on(c.execute(q, (point, ts, seq, ts as f64)))?;
                lite.execute(q, rusqlite::params![point, ts, seq, ts as f64])?;
            }
            8 => {
                let q = "DELETE FROM s WHERE id >= ?1 AND id < ?2";
                block_on(c.execute(q, (key(point, 0), key(point, ts / 4))))?;
                lite.execute(q, rusqlite::params![key(point, 0), key(point, ts / 4)])?;
            }
            _ => {
                let q = "UPDATE s SET num = num + 1 WHERE id BETWEEN ?1 AND ?2";
                block_on(c.execute(q, (key(point, ts), key(point, ts + 5_000))))?;
                lite.execute(q, rusqlite::params![key(point, ts), key(point, ts + 5_000)])?;
            }
        }
        if op % 50 == 49 {
            tx += 1;
            let end = if tx % 7 == 0 {
                rolled_back += 1;
                "ROLLBACK"
            } else {
                "COMMIT"
            };
            exec(&c, end)?;
            lite.execute_batch(end)?;
        }
    }
    let queries = [
        "SELECT meter, hour, kwh, cost FROM e ORDER BY meter, hour",
        "SELECT point_id, ts, seq, num FROM s ORDER BY point_id, ts, seq",
        "SELECT count(*), sum(kwh) FROM e WHERE hour >= 100 AND hour < 300",
        "SELECT count(*), sum(num) FROM s WHERE id > (7 << 44) + 5000 AND id < (8 << 44)",
        "SELECT ts, num FROM s WHERE id < (12 << 44) + 40000 AND id >= (12 << 44) ORDER BY id DESC LIMIT 1",
        "SELECT meter, count(*) FROM e WHERE meter = 'm3' GROUP BY meter",
    ];
    let mut mismatches = Vec::new();
    let mut compared = 0usize;
    for q in queries {
        let t = rows(&c, q, ())?;
        let mut stmt = lite.prepare(q)?;
        let n = stmt.column_count();
        let l: Vec<Vec<rusqlite::types::Value>> = stmt
            .query_map([], |r| (0..n).map(|i| r.get::<_, rusqlite::types::Value>(i)).collect())?
            .collect::<Result<_, _>>()?;
        compared += t.len();
        if t.len() != l.len() || !t.iter().zip(&l).all(|(a, b)| same_row(a, b)) {
            mismatches.push(q);
        }
    }
    let integrity = integrity_turso(&c)?;
    drop(c);
    drop(db);
    let sqlite_integrity = integrity_sqlite(&path)?;
    Ok(json!({
        "pass": mismatches.is_empty() && integrity == "ok" && sqlite_integrity == "ok",
        "ops": ops, "transactions": tx, "rolled_back": rolled_back, "rows_compared": compared,
        "mismatches": mismatches, "turso_integrity": integrity, "sqlite_integrity_on_turso_file": sqlite_integrity,
    }))
}

fn same_row(a: &[Value], b: &[rusqlite::types::Value]) -> bool {
    use rusqlite::types::Value as L;
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| match (x, y) {
            (Value::Null, L::Null) => true,
            (Value::Integer(i), L::Integer(j)) => i == j,
            (Value::Real(f), L::Real(g)) => (f - g).abs() <= 1e-9 * f.abs().max(1.0),
            (Value::Text(s), L::Text(t)) => s == t,
            (Value::Blob(s), L::Blob(t)) => s == t,
            _ => false,
        })
}

fn transactions(dir: &str) -> R<Json> {
    let path = fresh(dir, "tx");
    let (_db, c) = open(&path)?;
    exec(&c, TARGET)?;
    let count = |c: &turso::Connection| one(c, "SELECT count(*) FROM outbox", ());
    exec(&c, "BEGIN")?;
    block_on(c.execute("INSERT INTO outbox (seq, kind, ref) VALUES (1, 1, 1)", ()))?;
    exec(&c, "ROLLBACK")?;
    let after_rollback = count(&c)?;
    exec(&c, "BEGIN IMMEDIATE")?;
    block_on(c.execute("INSERT INTO outbox (seq, kind, ref) VALUES (1, 1, 1)", ()))?;
    block_on(c.execute("UPDATE meta SET next_seq = 2", ()))?;
    exec(&c, "COMMIT")?;
    let after_commit = count(&c)?;
    // Une instruction qui échoue (clé en double) n'annule pas la transaction.
    exec(&c, "BEGIN")?;
    block_on(c.execute("INSERT INTO outbox (seq, kind, ref) VALUES (2, 1, 2)", ()))?;
    let duplicate = err_text(block_on(c.execute("INSERT INTO outbox (seq, kind, ref) VALUES (1, 1, 1)", ())));
    exec(&c, "COMMIT")?;
    let after_failed_statement = count(&c)?;
    let savepoint = err_text(exec(&c, "BEGIN; SAVEPOINT a; INSERT INTO outbox VALUES (9, 1, 9); ROLLBACK TO a; COMMIT;"));
    let autocommit = c.is_autocommit()?;
    if !autocommit {
        let _ = exec(&c, "ROLLBACK");
    }
    Ok(json!({
        "pass": after_rollback == 0 && after_commit == 1 && duplicate != "ok" && after_failed_statement == 2,
        "after_rollback": after_rollback, "after_commit": after_commit,
        "duplicate_key_in_tx": duplicate, "rows_after_failed_statement": after_failed_statement,
        "savepoint": savepoint, "note": "SAVEPOINT n'est pas utilisé par la cible",
    }))
}

/// `BEGIN IMMEDIATE` ouvre la base temporaire de la connexion, sur disque par
/// défaut (`TURSO_TMPDIR`, `SQLITE_TMPDIR`, sinon `/tmp`) : sans dossier
/// temporaire, comme dans l'image `FROM scratch`, un rédacteur échoue ; avec
/// `temp_store = MEMORY` (`util::connect`), il passe. `VACUUM INTO` (la
/// sauvegarde) exige le dossier quand même : d'où `TURSO_TMPDIR` sur le
/// volume de données (`run.sh`, et Moli au démarrage).
fn temp_store(dir: &str) -> R<Json> {
    let path = fresh(dir, "temp-store");
    let missing = format!("{dir}/pas-de-dossier-temporaire");
    let previous = std::env::var_os("TURSO_TMPDIR");
    // SAFETY: les essais tournent l'un après l'autre, aucun autre fil vivant.
    unsafe { std::env::set_var("TURSO_TMPDIR", &missing) };
    let result = (|| -> R<(Json, Json, Json)> {
        let db = block_on(turso::Builder::new_local(&path).build())?;
        let on_disk = {
            let default = db.connect()?;
            err_text(exec(&default, "BEGIN IMMEDIATE"))
        };
        let memory = connect(&db)?;
        let in_memory = err_text(exec(&memory, "BEGIN IMMEDIATE; CREATE TABLE t (x INTEGER); COMMIT;"));
        let backup = format!("{dir}/temp-store-backup.db");
        remove_db(&backup);
        let vacuum = err_text(block_on(memory.execute(&format!("VACUUM INTO '{backup}'"), ())));
        Ok((on_disk, in_memory, vacuum))
    })();
    // SAFETY: idem.
    unsafe {
        match previous {
            Some(v) => std::env::set_var("TURSO_TMPDIR", v),
            None => std::env::remove_var("TURSO_TMPDIR"),
        }
    }
    let (on_disk, in_memory, vacuum) = result?;
    Ok(json!({
        "pass": on_disk != "ok" && in_memory == "ok" && vacuum != "ok",
        "without_temp_dir": {
            "default_connection_begin_immediate": on_disk,
            "temp_store_memory_begin_immediate": in_memory,
            "temp_store_memory_vacuum_into": vacuum,
        },
        "rule": "TURSO_TMPDIR sur le volume de données, temp_store = MEMORY sur chaque connexion",
    }))
}

/// Un lecteur ne bloque pas pendant une écriture longue, voit l'état validé
/// précédent, et garde son instantané pendant sa propre transaction.
fn readers_during_write(dir: &str) -> R<Json> {
    let path = fresh(dir, "readers");
    let (db, c) = open(&path)?;
    exec(&c, TARGET)?;
    for i in 0..1_000i64 {
        block_on(c.execute("INSERT INTO outbox (seq, kind, ref) VALUES (?1, 1, ?1)", (i + 1,)))?;
    }
    let reader = connect(&db)?;
    exec(&reader, "BEGIN")?;
    let snapshot_before = one(&reader, "SELECT count(*) FROM outbox", ())?;
    let barrier = Arc::new(Barrier::new(2));
    let writer = {
        let db = db.clone();
        let barrier = barrier.clone();
        thread::spawn(move || -> R<i64> {
            let started = (|| -> R<turso::Connection> {
                let w = connect(&db)?;
                exec(&w, "BEGIN IMMEDIATE")?;
                for i in 1_000..2_000i64 {
                    block_on(w.execute("INSERT INTO outbox (seq, kind, ref) VALUES (?1, 1, ?1)", (i + 1,)))?;
                }
                Ok(w)
            })();
            // Les rendez-vous ont lieu même en cas d'échec : le père
            // n'attend jamais pour rien, l'erreur revient à la jointure.
            barrier.wait(); // écriture en cours, non validée
            barrier.wait();
            let committed = started.and_then(|w| exec(&w, "COMMIT"));
            barrier.wait();
            committed.map(|()| 1_000)
        })
    };
    barrier.wait();
    let other = connect(&db)?;
    let t = Instant::now();
    let during_write = one(&other, "SELECT count(*) FROM outbox", ())?;
    let read_ms = ms(t);
    barrier.wait();
    barrier.wait(); // validé
    let snapshot_after_commit = one(&reader, "SELECT count(*) FROM outbox", ())?;
    exec(&reader, "COMMIT")?;
    let fresh_read = one(&reader, "SELECT count(*) FROM outbox", ())?;
    writer.join().map_err(|_| "writer panicked")??;
    Ok(json!({
        "pass": snapshot_before == 1_000 && during_write == 1_000 && snapshot_after_commit == 1_000 && fresh_read == 2_000,
        "uncommitted_invisible": during_write == 1_000, "read_during_write_ms": read_ms,
        "snapshot_kept_until_commit": snapshot_after_commit == 1_000, "after_commit": fresh_read,
    }))
}

/// MVCC expérimental : deux rédacteurs simultanés. Information seulement :
/// la cible garde un rédacteur unique et n'en dépend pas.
fn mvcc(dir: &str) -> R<Json> {
    let path = fresh(dir, "mvcc");
    let (db, c) = open(&path)?;
    let mode = err_text(rows(&c, "PRAGMA journal_mode = 'mvcc'", ()));
    let current = rows(&c, "PRAGMA journal_mode", ())?;
    exec(&c, "CREATE TABLE t (id INTEGER PRIMARY KEY, v INTEGER)")?;
    let a = connect(&db)?;
    let b = connect(&db)?;
    let both = (|| -> R<()> {
        exec(&a, "BEGIN CONCURRENT")?;
        exec(&b, "BEGIN CONCURRENT")?;
        block_on(a.execute("INSERT INTO t VALUES (1, 1)", ()))?;
        block_on(b.execute("INSERT INTO t VALUES (2, 2)", ()))?;
        exec(&a, "COMMIT")?;
        exec(&b, "COMMIT")?;
        Ok(())
    })();
    let n = one(&c, "SELECT count(*) FROM t", ()).unwrap_or(-1);
    Ok(json!({
        "pass": true, "informational": true,
        "set_mvcc": mode, "journal_mode": format!("{:?}", current.first().and_then(|r| r.first())),
        "two_concurrent_writers": err_text(both), "rows": n,
    }))
}

/// Réglages de durabilité acceptés, journal WAL, point de contrôle TRUNCATE.
/// Le nombre de fsync par validation se mesure à part (strace, `run.sh`).
fn durability(dir: &str) -> R<Json> {
    let path = fresh(dir, "durability");
    let (_db, c) = open(&path)?;
    exec(&c, TARGET)?;
    let journal = rows(&c, "PRAGMA journal_mode", ())?;
    let default_sync = rows(&c, "PRAGMA synchronous", ())?;
    let mut accepted = serde_json::Map::new();
    for mode in ["OFF", "NORMAL", "FULL"] {
        let set = err_text(exec(&c, &format!("PRAGMA synchronous = {mode}")));
        let now = rows(&c, "PRAGMA synchronous", ())?;
        accepted.insert(mode.into(), json!([set, format!("{:?}", now.first().and_then(|r| r.first()))]));
    }
    exec(&c, "PRAGMA synchronous = FULL")?;
    for i in 0..5_000i64 {
        block_on(c.execute("INSERT INTO outbox (seq, kind, ref) VALUES (?1, 1, ?1)", (i + 1,)))?;
    }
    let wal_before = file_size(&format!("{path}-wal"));
    let checkpoint = rows(&c, "PRAGMA wal_checkpoint(TRUNCATE)", ())?;
    let wal_after = file_size(&format!("{path}-wal"));
    let cache = rows(&c, "PRAGMA cache_size", ())?;
    let page = rows(&c, "PRAGMA page_size", ())?;
    Ok(json!({
        "pass": matches!(journal.first().and_then(|r| r.first()), Some(Value::Text(t)) if t == "wal")
            && wal_before > 0 && wal_after == 0,
        "journal_mode": format!("{:?}", journal.first().and_then(|r| r.first())),
        "default_synchronous": format!("{:?}", default_sync.first().and_then(|r| r.first())),
        "synchronous_settings": accepted,
        "wal_bytes_before_checkpoint": wal_before, "wal_bytes_after_truncate": wal_after,
        "checkpoint_result": format!("{checkpoint:?}"),
        "cache_size": format!("{cache:?}"), "page_size": format!("{page:?}"),
    }))
}

/// Les fichiers actuels (`history.db`, `energy.db`, écrits par SQLite avec
/// leurs tables `WITHOUT ROWID`) ouverts par Turso : lecture, puis chaque
/// écriture du code actuel, avec et sans l'option expérimentale.
fn sqlite_to_turso(dir: &str) -> R<Json> {
    let mut out = serde_json::Map::new();
    let mut reads_ok = true;
    for without_rowid in [false, true] {
        let h = fresh(dir, &format!("current-history-{without_rowid}"));
        let e = fresh(dir, &format!("current-energy-{without_rowid}"));
        current_files(&h, &e, 20, 200, 5, 300)?;
        let build = |p: &str| {
            block_on(turso::Builder::new_local(p).experimental_without_rowid(without_rowid).build())
        };
        let mut steps = serde_json::Map::new();
        match build(&h) {
            Err(e) => {
                steps.insert("open_history".into(), format!("error: {e}").into());
                reads_ok = false;
            }
            Ok(db) => {
                let c = connect(&db)?;
                let n = rows(&c, "SELECT count(*) FROM samples", ());
                let view = rows(&c, "SELECT count(*) FROM history WHERE point = 'dev0/value'", ());
                let range = rows(&c, "SELECT count(*) FROM samples WHERE point_id = 1 AND ts BETWEEN 0 AND 1000000", ());
                reads_ok &= n.is_ok() && view.is_ok() && range.is_ok();
                // Les mêmes lectures par la clé primaire, comparées à SQLite.
                let lite = Sqlite::open_with_flags(&h, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
                let mut wrong = Vec::new();
                for (q, p) in [
                    ("SELECT count(*) FROM samples WHERE point_id = ?1 AND ts BETWEEN 0 AND 1000000", 1i64),
                    ("SELECT count(*) FROM samples WHERE point_id = ?1 AND ts BETWEEN 0 AND 1000000", 3),
                    ("SELECT coalesce(max(ts), -1) FROM samples WHERE point_id = ?1 AND ts < 50000", 2),
                    ("SELECT coalesce((SELECT ts FROM samples WHERE point_id = ?1 AND ts < 50000 ORDER BY ts DESC LIMIT 1), -1)", 2),
                    ("SELECT count(*) FROM samples WHERE ts BETWEEN 0 AND 50000 AND point_id = ?1", 4),
                ] {
                    let t = one(&c, q, (p,)).unwrap_or(i64::MIN);
                    let l: i64 = lite.query_row(q, [p], |r| r.get(0))?;
                    if t != l {
                        wrong.push(json!({"query": q, "point": p, "turso": t, "sqlite": l}));
                    }
                }
                steps.insert("primary_key_reads_differ_from_sqlite".into(), json!(wrong));
                steps.insert("read_samples".into(), json!(format!("{:?}", n.map(|r| int(&r[0][0])).map_err(|e| e.to_string()))));
                steps.insert("read_history_view".into(), json!(format!("{:?}", view.map(|r| int(&r[0][0])).map_err(|e| e.to_string()))));
                steps.insert("read_primary_key_range".into(), json!(format!("{:?}", range.map(|r| int(&r[0][0])).map_err(|e| e.to_string()))));
                steps.insert("point_upsert_returning".into(), err_text(rows(&c, HISTORY_POINT_UPSERT, ("devX/new",))));
                steps.insert("sample_insert_on_conflict".into(), err_text(block_on(c.execute(HISTORY_SAMPLE_INSERT, (1i64, 99_999_999i64, "f", 1.0, None::<String>)))));
                steps.insert("prune".into(), err_text(block_on(c.execute(HISTORY_PRUNE, (1_000i64,)))));
                drop(c);
                drop(db);
                steps.insert("sqlite_integrity_after".into(), integrity_sqlite(&h)?.into());
            }
        }
        match build(&e) {
            Err(err) => {
                steps.insert("open_energy".into(), format!("error: {err}").into());
                reads_ok = false;
            }
            Ok(db) => {
                let c = connect(&db)?;
                let range = rows(&c, "SELECT meter, hour, kwh, cost FROM energy_hour WHERE hour >= ?1 AND hour < ?2 ORDER BY hour", (0i64, 3_600_000i64 * 48));
                reads_ok &= range.is_ok();
                steps.insert("read_hours_by_index".into(), json!(format!("{:?}", range.map(|r| r.len()).map_err(|e| e.to_string()))));
                steps.insert("energy_upsert".into(), err_text(block_on(c.execute(ENERGY_RECORD_UPSERT, ("m0", 0i64, 0.5, Some(0.1))))));
                steps.insert("meter_last_upsert".into(), err_text(block_on(c.execute(
                    "INSERT INTO meter_last (meter, kwh, ts) VALUES (?1, ?2, ?3) ON CONFLICT (meter) DO UPDATE SET kwh = excluded.kwh, ts = excluded.ts",
                    ("m0", 1.0, 1i64),
                ))));
                steps.insert("import_reprice_update".into(), err_text(block_on(c.execute(
                    "UPDATE energy_hour SET cost = ?3 WHERE meter = ?1 AND hour = ?2 AND cost IS NULL AND ?3 IS NOT NULL",
                    ("m0", 0i64, 1.0),
                ))));
                steps.insert("delete_hours".into(), err_text(block_on(c.execute("DELETE FROM energy_hour WHERE hour < ?1", (3_600_000i64,)))));
                drop(c);
                drop(db);
                steps.insert("sqlite_integrity_after".into(), integrity_sqlite(&e)?.into());
            }
        }
        out.insert(if without_rowid { "experimental_without_rowid" } else { "default" }.into(), steps.into());
    }
    // Ce que l'ADR suppose : Turso refuse (sans rien abîmer) toute écriture
    // sur les tables WITHOUT ROWID actuelles ; la bascule passe donc par une
    // copie lue avec SQLite vers un nouveau fichier.
    let writes = ["sample_insert_on_conflict", "prune", "energy_upsert", "import_reprice_update", "delete_hours"];
    let refused = out.values().all(|steps| {
        writes.iter().all(|w| steps[*w].as_str().is_some_and(|v| v.starts_with("error")))
            && steps["sqlite_integrity_after"] == "ok"
    });
    let wrong_reads = out.values().any(|steps| steps["primary_key_reads_differ_from_sqlite"].as_array().is_some_and(|a| !a.is_empty()));
    Ok(json!({
        "pass": refused && reads_ok,
        "without_rowid_writes_refused_files_intact": refused,
        "without_rowid_primary_key_reads_wrong": wrong_reads,
        "conclusion": "les fichiers actuels ne s'utilisent pas en place : copie lue par SQLite vers un fichier rowid",
        "results": out,
    }))
}

/// Crée `history.db` et `energy.db` au schéma actuel, avec SQLite et les
/// requêtes du code actuel. Deux appuis à la même milliseconde sont décalés
/// d'une milliseconde, comme le fait `moli-history`.
fn current_files(history: &str, energy: &str, points: i64, per_point: i64, meters: i64, hours: i64) -> R<()> {
    let h = Sqlite::open(history)?;
    h.pragma_update(None, "journal_mode", "WAL")?;
    h.execute_batch(HISTORY_CURRENT)?;
    let tx = h.unchecked_transaction()?;
    for p in 0..points {
        let id: i64 = tx.query_row(HISTORY_POINT_UPSERT, [format!("dev{p}/value")], |r| r.get(0))?;
        for i in 0..per_point {
            let ts = 1_000 + i * 1_000;
            let (kind, num, txt): (&str, Option<f64>, Option<String>) = match (p % 4, i % 7) {
                (0, _) => ("f", Some(20.0 + (i % 50) as f64 / 10.0), None),
                (1, _) => ("b", Some((i % 2) as f64), None),
                (2, 0) => ("z", None, None),
                (2, _) => ("t", None, Some(format!("état {}", i % 3))),
                _ => ("i", Some(i as f64), None),
            };
            for at in ts..ts + 16 {
                if tx.execute(HISTORY_SAMPLE_INSERT, rusqlite::params![id, at, kind, num, txt])? == 1 {
                    break;
                }
            }
            if p == 3 && i % 10 == 0 {
                // Un deuxième événement à la même milliseconde.
                for at in ts..ts + 16 {
                    if tx.execute(HISTORY_SAMPLE_INSERT, rusqlite::params![id, at, kind, num, txt])? == 1 {
                        break;
                    }
                }
            }
        }
    }
    tx.commit()?;
    drop(h);
    let e = Sqlite::open(energy)?;
    e.execute_batch(ENERGY_CURRENT)?;
    let tx = e.unchecked_transaction()?;
    for m in 0..meters {
        let meter = format!("m{m}");
        for hour in 0..hours {
            let cost = (hour % 5 != 0).then(|| 0.01 * (m + 1) as f64);
            tx.execute(ENERGY_RECORD_UPSERT, rusqlite::params![meter, hour * 3_600_000, 0.1 * (m + 1) as f64, cost])?;
        }
        tx.execute("INSERT OR IGNORE INTO meter_first (meter, ts) VALUES (?1, ?2)", rusqlite::params![meter, 1_800_000])?;
        tx.execute(
            "INSERT INTO meter_last (meter, kwh, ts) VALUES (?1, ?2, ?3) ON CONFLICT (meter) DO UPDATE SET kwh = excluded.kwh, ts = excluded.ts",
            rusqlite::params![meter, 1_000.0 + m as f64, hours * 3_600_000],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// Un fichier écrit par Turso, relu et modifié par SQLite, puis rouvert par
/// Turso : les deux moteurs lisent et écrivent le même format, l'un après l'autre.
fn turso_to_sqlite(dir: &str) -> R<Json> {
    let path = fresh(dir, "turso-written");
    {
        let (db, c) = open(&path)?;
        exec(&c, TARGET)?;
        exec(&c, "BEGIN")?;
        for i in 0..5_000i64 {
            block_on(c.execute(SAMPLE_INSERT, (i % 50, 1_000 + i, i + 1, i as f64)))?;
            block_on(c.execute("INSERT INTO outbox (seq, kind, ref) VALUES (?1, 1, ?2)", (i + 1, key(i % 50, 1_000 + i))))?;
        }
        exec(&c, "COMMIT")?;
        drop(c);
        drop(db);
    }
    let sqlite_integrity = integrity_sqlite(&path)?;
    let (n, sum, by_index) = {
        let l = Sqlite::open(&path)?;
        let n: i64 = l.query_row("SELECT count(*) FROM samples", [], |r| r.get(0))?;
        let sum: f64 = l.query_row("SELECT sum(num) FROM samples", [], |r| r.get(0))?;
        let by_index: i64 = l.query_row("SELECT count(*) FROM samples WHERE id >= (7 << 44) AND id < (8 << 44)", [], |r| r.get(0))?;
        l.execute(SAMPLE_INSERT, rusqlite::params![7, 1, 99999, 1.5])?;
        l.execute("UPDATE samples SET num = num + 1 WHERE id >= (8 << 44) AND id < (9 << 44)", [])?;
        l.execute("DELETE FROM samples WHERE id >= (9 << 44) AND id < (10 << 44)", [])?;
        (n, sum, by_index)
    };
    let (db, c) = open(&path)?;
    let after_sqlite = one(&c, "SELECT count(*) FROM samples WHERE id >= (7 << 44) AND id < (8 << 44)", ())?;
    let deleted = one(&c, "SELECT count(*) FROM samples WHERE point_id = 9", ())?;
    let turso_integrity = integrity_turso(&c)?;
    block_on(c.execute("DELETE FROM samples WHERE id >= (10 << 44) AND id < (11 << 44)", ()))?;
    drop(c);
    drop(db);
    let sqlite_integrity_again = integrity_sqlite(&path)?;
    let expected_sum: f64 = (0..5_000).map(|i| i as f64).sum();
    Ok(json!({
        "pass": sqlite_integrity == "ok" && n == 5_000 && (sum - expected_sum).abs() < 1e-6 && by_index == 100
            && after_sqlite == 101 && deleted == 0 && turso_integrity == "ok" && sqlite_integrity_again == "ok",
        "sqlite_reads_turso_file": { "integrity": sqlite_integrity, "rows": n, "rows_via_index": by_index },
        "turso_reads_sqlite_writes": { "rows_via_index": after_sqlite, "deleted_rows_left": deleted, "integrity": turso_integrity },
        "sqlite_integrity_after_second_turso_write": sqlite_integrity_again,
    }))
}

/// Bascule (copie SQLite → fichier Turso, séquences et outbox comprises) puis
/// retour arrière (fichier Turso → schémas actuels, avec SQLite seul) :
/// les données reviennent à l'identique.
fn migration_round_trip(dir: &str) -> R<Json> {
    let h = fresh(dir, "rt-history");
    let e = fresh(dir, "rt-energy");
    current_files(&h, &e, 40, 2_000, 10, 2_000)?;
    let target = fresh(dir, "rt-moli");
    let started = Instant::now();
    let migrated = migrate(&h, &e, &target)?;
    let migrate_ms = ms(started);
    // Comparaison source ↔ cible.
    let src = Sqlite::open(&h)?;
    let src_points: Vec<(String, i64, i64, i64, f64)> = src
        .prepare("SELECT p.point, count(*), min(ts), max(ts), total(num) FROM samples s JOIN points p ON p.id = s.point_id GROUP BY p.point ORDER BY p.point")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
        .collect::<Result<_, _>>()?;
    let (db, c) = open(&target)?;
    let dst_points = rows(&c, "SELECT c.point, count(*), min(ts), max(ts), total(num) FROM samples s JOIN catalog c ON c.id = s.point_id GROUP BY c.point ORDER BY c.point", ())?;
    let history_equal = src_points.len() == dst_points.len()
        && src_points.iter().zip(&dst_points).all(|(a, b)| {
            matches!(&b[0], Value::Text(t) if *t == a.0) && int(&b[1]) == a.1 && int(&b[2]) == a.2 && int(&b[3]) == a.3 && (real(&b[4]) - a.4).abs() < 1e-6
        });
    let outbox = one(&c, "SELECT count(*) FROM outbox", ())?;
    let next_seq = one(&c, "SELECT next_seq FROM meta", ())?;
    let src_e = Sqlite::open(&e)?;
    let src_energy: Vec<(String, i64, f64, f64)> = src_e
        .prepare("SELECT meter, count(*), total(kwh), total(cost) FROM energy_hour GROUP BY meter ORDER BY meter")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<Result<_, _>>()?;
    let dst_energy = rows(
        &c,
        "SELECT m.meter, count(*), total(kwh), total(cost) FROM energy_hour e JOIN meter_state m ON m.id = e.meter_id GROUP BY m.meter ORDER BY m.meter",
        (),
    )?;
    // Les totaux de jour et de mois, recalculés à la bascule, valent les heures.
    let hours_kwh = real(&rows(&c, "SELECT total(kwh) FROM energy_hour", ())?[0][0]);
    let totals_equal = ["day", "month"].iter().all(|span| {
        rows(&c, "SELECT total(kwh) FROM energy_total WHERE span = ?1", (*span,))
            .is_ok_and(|r| (real(&r[0][0]) - hours_kwh).abs() < 1e-6)
    });
    let energy_equal = src_energy.len() == dst_energy.len()
        && src_energy.iter().zip(&dst_energy).all(|(a, b)| {
            matches!(&b[0], Value::Text(t) if *t == a.0) && int(&b[1]) == a.1 && (real(&b[2]) - a.2).abs() < 1e-6 && (real(&b[3]) - a.3).abs() < 1e-6
        });
    drop(c);
    drop(db);

    // Retour arrière, avec SQLite seul : nouveaux fichiers aux schémas actuels.
    let back_h = fresh(dir, "rt-back-history");
    let back_e = fresh(dir, "rt-back-energy");
    let started = Instant::now();
    let bh = Sqlite::open(&back_h)?;
    bh.execute_batch(HISTORY_CURRENT)?;
    bh.execute("ATTACH DATABASE ?1 AS n", [&target])?;
    bh.execute_batch(&format!("BEGIN; {ROLLBACK_HISTORY} COMMIT;"))?;
    bh.execute_batch("DETACH DATABASE n")?;
    let be = Sqlite::open(&back_e)?;
    be.execute_batch(ENERGY_CURRENT)?;
    be.execute("ATTACH DATABASE ?1 AS n", [&target])?;
    be.execute_batch(&format!("BEGIN; {ROLLBACK_ENERGY} COMMIT;"))?;
    be.execute_batch("DETACH DATABASE n")?;
    let rollback_ms = ms(started);
    let same = |a: &Sqlite, b: &Sqlite, q: &str| -> R<bool> {
        let read = |c: &Sqlite| -> R<Vec<Vec<rusqlite::types::Value>>> {
            let mut s = c.prepare(q)?;
            let n = s.column_count();
            Ok(s.query_map([], |r| (0..n).map(|i| r.get(i)).collect())?.collect::<Result<_, _>>()?)
        };
        Ok(read(a)? == read(b)?)
    };
    let history_back = same(&src, &bh, "SELECT p.point, s.ts, s.kind, s.num, s.txt FROM samples s JOIN points p ON p.id = s.point_id ORDER BY 1, 2")?;
    let energy_back = same(&src_e, &be, "SELECT meter, hour, kwh, cost FROM energy_hour ORDER BY 1, 2")?
        && same(&src_e, &be, "SELECT meter, kwh, ts FROM meter_last ORDER BY 1")?
        && same(&src_e, &be, "SELECT meter, ts FROM meter_first ORDER BY 1")?;
    Ok(json!({
        "pass": history_equal && energy_equal && totals_equal && outbox == migrated && next_seq == migrated + 1 && history_back && energy_back,
        "migrated_ops": migrated, "migrate_ms": migrate_ms, "rollback_sql_ms": rollback_ms,
        "history_equal": history_equal, "energy_equal": energy_equal, "day_month_totals_equal_hours": totals_equal,
        "outbox_rows": outbox, "next_seq": next_seq,
        "rollback_history_identical": history_back, "rollback_energy_identical": energy_back,
    }))
}

/// Copie déterministe de `history.db` + `energy.db` (fermés) vers le fichier
/// Turso cible : chaque ligne reçoit sa séquence et son entrée d'outbox dans
/// la même transaction (l'archive reçoit aussi l'historique existant).
pub fn migrate(history: &str, energy: &str, target: &str) -> R<i64> {
    let (_db, c) = open(target)?;
    exec(&c, TARGET)?;
    exec(&c, "PRAGMA synchronous = FULL")?;
    let mut seq = one(&c, "SELECT next_seq FROM meta", ())?;
    let h = Sqlite::open_with_flags(history, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    exec(&c, "BEGIN")?;
    let mut stmt = h.prepare("SELECT id, point FROM points ORDER BY id")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
        let (id, point) = row?;
        block_on(c.execute("INSERT INTO catalog (id, point, family) VALUES (?1, ?2, 'other')", (id, point)))?;
    }
    exec(&c, "COMMIT")?;
    let mut stmt = h.prepare("SELECT point_id, ts, kind, num, txt FROM samples ORDER BY ts, point_id")?;
    let mut cursor = stmt.query([])?;
    let mut in_tx = 0;
    let mut transaction_open = false;
    while let Some(r) = cursor.next()? {
        if !transaction_open {
            exec(&c, "BEGIN")?;
            transaction_open = true;
        }
        let (point, ts, kind, num, txt): (i64, i64, String, Option<f64>, Option<String>) =
            (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?);
        block_on(c.execute(
            "INSERT INTO samples (id, point_id, ts, seq, kind, num, txt) VALUES ((?1 << 44) + ?2, ?1, ?2, ?3, ?4, ?5, ?6)",
            (point, ts, seq, kind, num, txt),
        ))?;
        crate::journal::append(&c, seq, OP_SAMPLE, key(point, ts))?;
        seq += 1;
        in_tx += 1;
        if in_tx == 5_000 {
            block_on(c.execute("UPDATE meta SET next_seq = ?1", (seq,)))?;
            exec(&c, "COMMIT")?;
            in_tx = 0;
            transaction_open = false;
        }
    }
    let e = Sqlite::open_with_flags(energy, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    // Un numéro par compteur, dans l'ordre des noms.
    let meters: Vec<String> = e
        .prepare("SELECT meter FROM meter_last UNION SELECT meter FROM meter_first UNION SELECT meter FROM energy_hour ORDER BY 1")?
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let ids: HashMap<&str, i64> = meters.iter().enumerate().map(|(i, m)| (m.as_str(), i as i64)).collect();
    if !transaction_open {
        exec(&c, "BEGIN")?;
        transaction_open = true;
    }
    for (meter, id) in &ids {
        block_on(c.execute("INSERT INTO meter_state (id, meter) VALUES (?1, ?2)", (*id, *meter)))?;
    }
    let mut stmt = e.prepare("SELECT meter, hour, kwh, cost FROM energy_hour ORDER BY hour, meter")?;
    let mut cursor = stmt.query([])?;
    while let Some(r) = cursor.next()? {
        if !transaction_open {
            exec(&c, "BEGIN")?;
            transaction_open = true;
        }
        let (meter, hour, kwh, cost): (String, i64, f64, Option<f64>) = (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?);
        let id = ids[meter.as_str()];
        block_on(c.execute(ENERGY_UPSERT, (id, hour, kwh, cost, seq)))?;
        let row = hour_key(id, hour);
        let (day, month) = local_day_month(hour, OFFSET);
        block_on(c.execute(TOTAL_UPSERT, ("day", day, id, kwh, cost)))?;
        block_on(c.execute(TOTAL_UPSERT, ("month", month, id, kwh, cost)))?;
        crate::journal::append(&c, seq, OP_ENERGY_HOUR, row)?;
        seq += 1;
        in_tx += 1;
        if in_tx == 5_000 {
            block_on(c.execute("UPDATE meta SET next_seq = ?1", (seq,)))?;
            exec(&c, "COMMIT")?;
            in_tx = 0;
            transaction_open = false;
        }
    }
    if !transaction_open {
        exec(&c, "BEGIN")?;
    }
    let mut stmt = e.prepare("SELECT meter, kwh, ts FROM meter_last")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?, r.get::<_, i64>(2)?)))? {
        let (meter, kwh, ts) = row?;
        block_on(c.execute("UPDATE meter_state SET last_kwh = ?2, last_ts = ?3 WHERE meter = ?1", (meter, kwh, ts)))?;
    }
    let mut stmt = e.prepare("SELECT meter, ts FROM meter_first")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
        let (meter, ts) = row?;
        block_on(c.execute("UPDATE meter_state SET first_ts = ?2 WHERE meter = ?1", (meter, ts)))?;
    }
    block_on(c.execute(
        "UPDATE meta SET next_seq = ?1, first_energy_hour = (SELECT min(hour) FROM energy_hour)",
        (seq,),
    ))?;
    exec(&c, "COMMIT")?;
    Ok(seq - 1)
}

/// Une clé `INTEGER PRIMARY KEY` sans AUTOINCREMENT réutilise les numéros
/// libérés : la séquence de l'outbox vient donc d'un compteur persistant.
fn sequence(dir: &str) -> R<Json> {
    let path = fresh(dir, "sequence");
    let (_db, c) = open(&path)?;
    exec(&c, "CREATE TABLE t (seq INTEGER PRIMARY KEY, v INTEGER)")?;
    exec(&c, "INSERT INTO t (v) VALUES (1); INSERT INTO t (v) VALUES (2); DELETE FROM t;")?;
    block_on(c.execute("INSERT INTO t (v) VALUES (3)", ()))?;
    let reused = one(&c, "SELECT seq FROM t", ())?;
    let autoincrement = err_text(exec(&c, "CREATE TABLE a (seq INTEGER PRIMARY KEY AUTOINCREMENT, v INTEGER)"));
    let autoincrement_next = if autoincrement == "ok" {
        exec(&c, "INSERT INTO a (v) VALUES (1); INSERT INTO a (v) VALUES (2); DELETE FROM a;")?;
        block_on(c.execute("INSERT INTO a (v) VALUES (3)", ()))?;
        one(&c, "SELECT seq FROM a", ())?
    } else {
        0
    };
    exec(&c, TARGET)?;
    // Le compteur : allouer, vider l'outbox, rouvrir, allouer encore.
    let alloc = |c: &turso::Connection| -> R<i64> {
        exec(c, "BEGIN IMMEDIATE")?;
        let s = one(c, "UPDATE meta SET next_seq = next_seq + 1 RETURNING next_seq - 1", ())?;
        block_on(c.execute("INSERT INTO outbox (seq, kind, ref) VALUES (?1, 1, 0)", (s,)))?;
        exec(c, "COMMIT")?;
        Ok(s)
    };
    let first = alloc(&c)?;
    let second = alloc(&c)?;
    exec(&c, "DELETE FROM outbox")?;
    drop(c);
    drop(_db);
    let (_db, c) = open(&path)?;
    let third = alloc(&c)?;
    Ok(json!({
        "pass": reused == 1 && first == 1 && second == 2 && third == 3,
        "rowid_reused_after_delete": reused == 1,
        "autoincrement": autoincrement,
        "autoincrement_after_delete": autoincrement_next,
        "counter_sequence": [first, second, third],
    }))
}

/// Le SQL libre des agents : Turso 0.8.3 n'a pas d'autorisateur
/// (`sqlite3_set_authorizer`), ni dans `turso` ni dans `turso_core`.
/// On constate ici ce que la base refuse d'elle-même.
fn agent_sql(dir: &str) -> R<Json> {
    let path = fresh(dir, "agents");
    {
        let (_db, c) = open(&path)?;
        exec(&c, TARGET)?;
        exec(&c, "INSERT INTO catalog (point, family) VALUES ('door/contact', 'event')")?;
    }
    let ro = block_on(turso::Builder::new_local(&path).read_only(true).build());
    let (db, c, mode) = match ro {
        Ok(db) => {
            let c = connect(&db)?;
            (db, c, "read_only")
        }
        Err(_) => {
            let (db, c) = open(&path)?;
            (db, c, "read_write")
        }
    };
    let forbidden = [
        "DELETE FROM samples",
        "DROP TABLE catalog",
        "INSERT INTO catalog (point, family) VALUES ('x', 'y')",
        "ATTACH DATABASE '/etc/passwd' AS x",
        "PRAGMA query_only = 0",
        "PRAGMA journal_mode = DELETE",
        "SELECT 1; DELETE FROM samples",
        "BEGIN",
        "SAVEPOINT pin",
        "SELECT length(printf('%.*c', 10000000, 'x'))",
        "SELECT length(zeroblob(10000000))",
    ];
    let mut accepted = Vec::new();
    let mut results = serde_json::Map::new();
    for sql in forbidden {
        let r = rows(&c, sql, ());
        if r.is_ok() {
            accepted.push(sql);
        }
        results.insert(sql.into(), err_text(r));
        if !c.is_autocommit().unwrap_or(true) {
            let _ = exec(&c, "ROLLBACK");
        }
    }
    let catalog = one(&c, "SELECT count(*) FROM catalog", ())?;
    drop(c);
    drop(db);
    Ok(json!({
        "pass": true, "informational": true,
        "connection": mode,
        "authorizer_api": "absent de turso 0.8.3",
        "accepted_without_authorizer": accepted,
        "results": results, "catalog_rows_after": catalog,
        "conclusion": "sans autorisateur, le SQL libre n'est pas bornable comme aujourd'hui : history_sql devient un outil structuré (C2)",
    }))
}

/// Un deuxième processus ne doit pas écrire dans un fichier ouvert par Moli :
/// Turso pose un verrou exclusif ; on constate aussi ce que voient un lecteur
/// Turso en lecture seule et SQLite pendant ce temps.
fn multiprocess(dir: &str) -> R<Json> {
    let path = fresh(dir, "locked");
    {
        let (_db, c) = open(&path)?;
        exec(&c, TARGET)?;
        rows(&c, "PRAGMA wal_checkpoint(TRUNCATE)", ())?;
    }
    let exe = std::env::current_exe()?;
    let mut child = Command::new(exe).args(["hold", &path]).stdout(Stdio::piped()).spawn()?;
    // Le fils écrit puis tient le fichier ouvert ; on attend sa ligne « ready ».
    let mut line = String::new();
    {
        use std::io::BufRead;
        let stdout = child.stdout.as_mut().ok_or("no stdout")?;
        std::io::BufReader::new(stdout).read_line(&mut line)?;
    }
    let rw = block_on(turso::Builder::new_local(&path).build()).map(|_| ());
    let ro = (|| -> R<i64> {
        let db = block_on(turso::Builder::new_local(&path).read_only(true).build())?;
        let c = connect(&db)?;
        one(&c, "SELECT count(*) FROM outbox", ())
    })();
    let lite = (|| -> R<i64> {
        let l = Sqlite::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        l.busy_timeout(Duration::from_millis(200))?;
        Ok(l.query_row("SELECT count(*) FROM outbox", [], |r| r.get(0))?)
    })();
    child.kill()?;
    child.wait()?;
    Ok(json!({
        "pass": rw.is_err(),
        "holder": line.trim(),
        "second_turso_read_write": err_text(rw),
        "second_turso_read_only_count": format!("{ro:?}"),
        "sqlite_read_only_count": format!("{lite:?}"),
        "rule": "un seul processus par fichier ; sauvegarde et import passent par le processus qui le tient",
    }))
}

/// Mode `hold` : écrit 100 lignes validées (sans point de contrôle) et garde
/// le fichier ouvert jusqu'à être tué.
pub fn hold(path: &str) -> R<()> {
    let (_db, c) = open(path)?;
    for i in 0..100i64 {
        block_on(c.execute("INSERT INTO outbox (seq, kind, ref) VALUES (?1, 1, ?1)", (i + 1,)))?;
    }
    println!("ready 100 committed");
    loop {
        thread::sleep(Duration::from_secs(60));
    }
}

/// Sauvegarde cohérente depuis le processus qui tient la base.
fn vacuum_into(dir: &str) -> R<Json> {
    let path = fresh(dir, "backup-src");
    let copy = format!("{dir}/backup-copy.db");
    remove_db(&copy);
    let mut out = serde_json::Map::new();
    let mut pass = false;
    for enabled in [false, true] {
        remove_db(&copy);
        let db = block_on(turso::Builder::new_local(&path).experimental_vacuum(enabled).build())?;
        let c = connect(&db)?;
        exec(&c, TARGET)?;
        exec(&c, "DELETE FROM outbox")?;
        for i in 0..2_000i64 {
            block_on(c.execute("INSERT INTO outbox (seq, kind, ref) VALUES (?1, 1, ?1)", (i + 1,)))?;
        }
        let r = exec(&c, &format!("VACUUM INTO '{copy}'"));
        let copied = if r.is_ok() {
            let l = Sqlite::open(&copy)?;
            let n: i64 = l.query_row("SELECT count(*) FROM outbox", [], |r| r.get(0))?;
            let ok: String = l.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
            pass |= n == 2_000 && ok == "ok";
            json!({ "rows": n, "sqlite_integrity": ok })
        } else {
            Json::Null
        };
        out.insert(
            if enabled { "experimental_vacuum" } else { "default" }.into(),
            json!({ "vacuum_into": err_text(r), "copy": copied }),
        );
    }
    Ok(json!({ "pass": pass, "results": out }))
}

/// Le backend d'entrées / sorties par défaut fonctionne là où io_uring est
/// interdit (profil seccomp par défaut de Docker) : on vérifie les deux.
fn seccomp(dir: &str) -> R<Json> {
    let io_uring = crate::probe_io_uring();
    let path = fresh(dir, "seccomp");
    let (_db, c) = open(&path)?;
    exec(&c, TARGET)?;
    exec(&c, "PRAGMA synchronous = FULL")?;
    for i in 0..1_000i64 {
        block_on(c.execute("INSERT INTO outbox (seq, kind, ref) VALUES (?1, 1, ?1)", (i + 1,)))?;
    }
    let n = one(&c, "SELECT count(*) FROM outbox", ())?;
    Ok(json!({
        "pass": n == 1_000,
        "io_uring_setup": io_uring,
        "turso_writes_with_default_backend": n,
    }))
}

fn migration_boundaries(dir: &str) -> R<Json> {
    let mut cases = Vec::new();
    for samples in [0, 1, 4_999, 5_000, 5_001] {
        for hours in [0, 1] {
            let history = fresh(dir, &format!("bound-h-{samples}-{hours}"));
            let energy = fresh(dir, &format!("bound-e-{samples}-{hours}"));
            let target = fresh(dir, &format!("bound-m-{samples}-{hours}"));
            current_files(&history, &energy, 1, samples, 1, hours)?;
            let result = match migrate(&history, &energy, &target) {
                Ok(n) => {
                    let (_db, c) = open(&target)?;
                    let journal = one(&c, "SELECT count(*) FROM export_journal", ())?;
                    let next = one(&c, "SELECT next_seq FROM meta", ())?;
                    json!({"samples": samples, "hours": hours, "pass": n == samples + hours && journal == n && next == n + 1})
                }
                Err(error) => json!({"samples": samples, "hours": hours, "pass": false, "error": error.to_string()}),
            };
            cases.push(result);
        }
    }
    Ok(json!({"pass": cases.iter().all(|c| c["pass"] == true), "cases": cases}))
}

#[cfg(test)]
mod review_regressions {
    use super::*;

    #[test]
    fn migration_handles_empty_and_full_sample_batches() -> R<()> {
        let dir = format!("/tmp/moli-migration-boundaries-{}", std::process::id());
        std::fs::create_dir_all(&dir)?;
        let result = migration_boundaries(&dir)?;
        assert_eq!(result["pass"], true, "{result}");
        Ok(())
    }
}
