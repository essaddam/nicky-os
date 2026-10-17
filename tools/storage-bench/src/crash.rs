//! Cohérence après `kill -9` pendant des écritures, et nombre de `fsync`.
//!
//! Un processus fils écrit en continu des transactions qui touchent à la fois
//! `samples`, `energy_hour`, `energy_total`, `meter_state`, `outbox` et `meta.next_seq`, avec
//! des points de contrôle réguliers ; le père le tue (SIGKILL) à un instant
//! aléatoire, rouvre le fichier et vérifie les invariants.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use rusqlite::Connection as Sqlite;
use serde_json::json;

use crate::schema::{ENERGY_UPSERT, OP_ENERGY_HOUR, OP_SAMPLE, SAMPLE_INSERT, TARGET, TOTAL_UPSERT, key, local_day_month};
use crate::util::{Lcg, R, block_on, emit, exec, file_size, ms, one, open, quantiles, real, remove_db, rows};

const H: i64 = 3_600_000;
const ENERGY_STEP: f64 = 0.001;

/// Mode fils : écrit jusqu'à être tué, et annonce chaque séquence validée.
pub fn writer(path: &str, seed: u64) -> R<()> {
    let (_db, c) = open(path)?;
    exec(&c, TARGET)?;
    exec(&c, "PRAGMA synchronous = FULL")?;
    let mut rng = Lcg(seed);
    let stdout = std::io::stdout();
    for tx in 0u64.. {
        exec(&c, "BEGIN IMMEDIATE")?;
        let next = one(&c, "SELECT next_seq FROM meta", ())?;
        let k = 1 + rng.below(40) as i64;
        for seq in next..next + k {
            if seq % 5 == 0 {
                let (meter, hour) = (seq % 7, (seq / 100) * H);
                let id = one(&c, ENERGY_UPSERT, (meter, hour, ENERGY_STEP, None::<f64>, seq))?;
                let (day, month) = local_day_month(hour, 2 * H);
                block_on(c.execute(TOTAL_UPSERT, ("day", day, meter, ENERGY_STEP, None::<f64>)))?;
                block_on(c.execute(TOTAL_UPSERT, ("month", month, meter, ENERGY_STEP, None::<f64>)))?;
                block_on(c.execute("INSERT INTO outbox (seq, kind, ref) VALUES (?1, ?2, ?3)", (seq, OP_ENERGY_HOUR, id)))?;
                block_on(c.execute(
                    "INSERT INTO meter_state (id, meter, last_kwh, last_ts, first_ts) VALUES (?1, 'm' || ?1, ?2, ?3, ?3)
                     ON CONFLICT (id) DO UPDATE SET last_kwh = last_kwh + excluded.last_kwh, last_ts = excluded.last_ts",
                    (meter, ENERGY_STEP, seq),
                ))?;
            } else {
                block_on(c.execute(SAMPLE_INSERT, (seq % 97, seq, seq, seq as f64)))?;
                block_on(c.execute("INSERT INTO outbox (seq, kind, ref) VALUES (?1, ?2, ?3)", (seq, OP_SAMPLE, key(seq % 97, seq))))?;
            }
        }
        block_on(c.execute("UPDATE meta SET next_seq = ?1", (next + k,)))?;
        exec(&c, "COMMIT")?;
        // Après le retour de COMMIT seulement : cette séquence doit survivre.
        let mut out = stdout.lock();
        writeln!(out, "{}", next + k - 1)?;
        out.flush()?;
        if tx % 25 == 24 {
            let _ = rows(&c, "PRAGMA wal_checkpoint(TRUNCATE)", ());
        }
    }
    Ok(())
}

struct Check {
    ok: bool,
    detail: serde_json::Value,
}

/// Les invariants, lus avec Turso.
fn verify(path: &str, reported: i64) -> R<(Check, f64)> {
    let t = Instant::now();
    let (_db, c) = open(path)?;
    let next = one(&c, "SELECT next_seq FROM meta", ())?;
    let open_ms = ms(t);
    let last = next - 1;
    let outbox = one(&c, "SELECT count(*) FROM outbox", ())?;
    let max_seq = one(&c, "SELECT coalesce(max(seq), 0) FROM outbox", ())?;
    let samples = one(&c, "SELECT count(*) FROM samples", ())?;
    let linked = one(
        &c,
        "SELECT count(*) FROM samples s JOIN outbox o ON o.seq = s.seq AND o.ref = s.id AND o.kind = 1",
        (),
    )?;
    let energy_ops = one(&c, "SELECT count(*) FROM outbox WHERE kind = 2", ())?;
    let expected_energy_ops = last / 5;
    let meters = rows(
        &c,
        "SELECT m.meter, m.last_kwh, (SELECT total(kwh) FROM energy_hour e WHERE e.meter_id = m.id),
                (SELECT total(kwh) FROM energy_total t WHERE t.meter_id = m.id AND t.span = 'day'),
                (SELECT total(kwh) FROM energy_total t WHERE t.meter_id = m.id AND t.span = 'month')
         FROM meter_state m",
        (),
    )?;
    let energy_consistent = meters
        .iter()
        .all(|r| [&r[2], &r[3], &r[4]].iter().all(|v| (real(&r[1]) - real(v)).abs() < 1e-6));
    let total_kwh: f64 = meters.iter().map(|r| real(&r[1])).sum();
    let integrity = rows(&c, "PRAGMA integrity_check", ())?;
    let integrity_ok = matches!(integrity.first().and_then(|r| r.first()), Some(turso::Value::Text(t)) if t == "ok");
    let ok = last >= reported
        && outbox == last
        && max_seq == last
        && samples == last - expected_energy_ops
        && linked == samples
        && energy_ops == expected_energy_ops
        && energy_consistent
        && (total_kwh - expected_energy_ops as f64 * ENERGY_STEP).abs() < 1e-6
        && integrity_ok;
    Ok((
        Check {
            ok,
            detail: json!({
                "reported_committed": reported, "recovered_last_seq": last,
                "outbox": outbox, "samples": samples, "linked": linked,
                "energy_ops": energy_ops, "energy_consistent": energy_consistent, "integrity": integrity_ok,
            }),
        },
        open_ms,
    ))
}

/// Les mêmes invariants, lus par SQLite sur le fichier laissé par Turso tué
/// (journal WAL non vidé) : la voie du retour arrière après un crash.
fn verify_with_sqlite(path: &str, reported: i64) -> R<Check> {
    let l = Sqlite::open(path)?;
    let next: i64 = l.query_row("SELECT next_seq FROM meta", [], |r| r.get(0))?;
    let outbox: i64 = l.query_row("SELECT count(*) FROM outbox", [], |r| r.get(0))?;
    let integrity: String = l.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    let consistent: i64 = l.query_row(
        "SELECT count(*) FROM meter_state m WHERE abs(m.last_kwh - (SELECT total(kwh) FROM energy_hour e WHERE e.meter_id = m.id)) > 1e-6
            OR abs(m.last_kwh - (SELECT total(kwh) FROM energy_total t WHERE t.meter_id = m.id AND t.span = 'day')) > 1e-6",
        [],
        |r| r.get(0),
    )?;
    Ok(Check {
        ok: next - 1 >= reported && outbox == next - 1 && integrity == "ok" && consistent == 0,
        detail: json!({ "reported_committed": reported, "recovered_last_seq": next - 1, "outbox": outbox, "integrity": integrity, "inconsistent_meters": consistent }),
    })
}

pub fn run(dir: &str, iterations: u32) -> R<()> {
    std::fs::create_dir_all(dir)?;
    let path = format!("{dir}/crash.db");
    remove_db(&path);
    let exe = std::env::current_exe()?;
    let mut rng = Lcg(7);
    let mut failures = Vec::new();
    let mut open_times = Vec::new();
    let mut wal_at_kill = Vec::new();
    let mut killed_after = Vec::new();
    for it in 0..=iterations {
        let mut child = Command::new(&exe)
            .args(["crash-writer", &path, &it.to_string()])
            .stdout(Stdio::piped())
            .spawn()?;
        let last = Arc::new(Mutex::new(0i64));
        let reader = {
            let last = last.clone();
            let stdout = child.stdout.take().ok_or("no stdout")?;
            thread::spawn(move || {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    if let Ok(v) = line.trim().parse::<i64>() {
                        *last.lock().unwrap() = v;
                    }
                }
            })
        };
        let wait = 60 + rng.below(500);
        thread::sleep(Duration::from_millis(wait));
        child.kill()?; // SIGKILL
        child.wait()?;
        reader.join().map_err(|_| "reader panicked")?;
        let reported = *last.lock().unwrap();
        killed_after.push(wait as f64);
        wal_at_kill.push(file_size(&format!("{path}-wal")) as f64);
        // Dernier tour : SQLite relit d'abord le fichier laissé par le crash.
        let check = if it == iterations {
            verify_with_sqlite(&path, reported)?
        } else {
            let (check, open_ms) = verify(&path, reported)?;
            open_times.push(open_ms);
            check
        };
        if !check.ok {
            failures.push(json!({ "iteration": it, "detail": check.detail }));
        }
        if it == iterations {
            emit(&json!({ "test": "crash_recovery_read_by_sqlite", "pass": check.ok, "detail": check.detail }));
            let (after, _) = verify(&path, reported)?;
            emit(&json!({ "test": "turso_reopens_after_sqlite_recovery", "pass": after.ok, "detail": after.detail }));
        }
    }
    let (_db, c) = open(&path)?;
    let final_seq = one(&c, "SELECT next_seq - 1 FROM meta", ())?;
    let samples = one(&c, "SELECT count(*) FROM samples", ())?;
    emit(&json!({
        "test": "kill9_during_writes",
        "pass": failures.is_empty(),
        "kills": iterations + 1,
        "failures": failures,
        "final_last_seq": final_seq, "final_samples": samples,
        "reopen_ms": quantiles(&mut open_times),
        "wal_bytes_at_kill_max": wal_at_kill.iter().copied().fold(0.0, f64::max),
        "kill_delay_ms": quantiles(&mut killed_after),
        "checks": "séquence validée annoncée ≤ séquence relue ; outbox contiguë 1..n ; chaque échantillon lié à son entrée d'outbox ; heures, totaux de jour et de mois et meter_state égaux (même transaction) ; integrity_check",
    }));
    Ok(())
}

/// `n` transactions d'une ligne : à lancer sous `strace -c` pour compter les
/// fsync par validation selon `PRAGMA synchronous`.
pub fn fsync_probe(path: &str, sync: &str, n: i64) -> R<()> {
    remove_db(path);
    let (_db, c) = open(path)?;
    exec(&c, TARGET)?;
    exec(&c, &format!("PRAGMA synchronous = {sync}"))?;
    let t = Instant::now();
    for i in 0..n {
        block_on(c.execute("INSERT INTO outbox (seq, kind, ref) VALUES (?1, 1, ?1)", (i + 1,)))?;
    }
    emit(&json!({ "phase": "fsync_probe", "synchronous": sync, "commits": n, "ms": ms(t) }));
    Ok(())
}
