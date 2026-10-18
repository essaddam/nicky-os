//! Export immuable et reprise après restauration : le contenu ne vient jamais
//! des tables métier au moment d'un réessai. Le journal et l'outbox sont écrits
//! dans la transaction de la donnée ; l'outbox peut être vidée à publication.

use serde_json::{Value as Json, json};
use turso::{Connection, Value};

use crate::schema::{OP_ENERGY_HOUR, OP_SAMPLE};
use crate::util::{R, block_on, exec, int, one, rows};

pub const REPLAY_WINDOW_MS: i64 = 7 * 86_400_000;
pub const MAX_BATCH: i64 = 5_000;

fn value(v: &Value) -> Json {
    match v {
        Value::Integer(n) => json!(n),
        Value::Real(n) => json!(n),
        Value::Text(s) => json!(s),
        _ => Json::Null,
    }
}

/// À appeler après l'écriture métier, dans la même transaction du rédacteur.
/// Le contrôle de `seq` empêche de capturer une version ultérieure par erreur.
pub fn append(c: &Connection, seq: i64, kind: i64, reference: i64) -> R<()> {
    let payload = match kind {
        OP_SAMPLE => {
            let data = rows(c,
                "SELECT c.point, s.ts, s.kind, s.num, s.txt FROM samples s JOIN catalog c ON c.id = s.point_id WHERE s.id = ?1 AND s.seq = ?2",
                (reference, seq))?;
            let r = data.first().ok_or("journal : relevé ou séquence absent")?;
            json!({"type": "sample", "seq": seq, "point": value(&r[0]), "ts": int(&r[1]),
                   "kind": value(&r[2]), "num": value(&r[3]), "txt": value(&r[4])})
        }
        OP_ENERGY_HOUR => {
            let data = rows(c,
                "SELECT m.meter, e.hour, e.kwh, e.cost FROM energy_hour e JOIN meter_state m ON m.id = e.meter_id WHERE e.id = ?1 AND e.seq = ?2",
                (reference, seq))?;
            let r = data.first().ok_or("journal : heure ou séquence absente")?;
            json!({"type": "energy_hour", "seq": seq, "meter": value(&r[0]), "hour": int(&r[1]),
                   "kwh": value(&r[2]), "cost": value(&r[3])})
        }
        _ => return Err("journal : type d'opération inconnu".into()),
    };
    let encoded = serde_json::to_string(&payload)?;
    block_on(c.execute("INSERT INTO outbox (seq, kind, ref) VALUES (?1, ?2, ?3)", (seq, kind, reference)))?;
    block_on(c.execute("INSERT INTO export_journal (seq, payload) VALUES (?1, ?2)", (seq, encoded)))?;
    Ok(())
}

/// Les octets NDJSON sont copiés tels qu'enregistrés, dans l'ordre des séquences.
/// Une plage tronquée ou sortie de la fenêtre de reprise est une erreur explicite.
pub fn body(c: &Connection, after: i64, count: i64) -> R<Vec<u8>> {
    if after < 0 || !(1..=MAX_BATCH).contains(&count) {
        return Err("journal : plage invalide".into());
    }
    let floor = one(c, "SELECT replay_floor FROM archive_ack", ())?;
    if after < floor {
        return Err(format!("reprise expirée : archive à {after}, journal disponible après {floor}").into());
    }
    let end = after.checked_add(count).ok_or("journal : dépassement de séquence")?;
    let data = rows(c, "SELECT seq, payload FROM export_journal WHERE seq > ?1 AND seq <= ?2 ORDER BY seq", (after, end))?;
    if data.len() != count as usize || data.iter().enumerate().any(|(k, r)| int(&r[0]) != after + k as i64 + 1) {
        return Err("journal : plage non contiguë".into());
    }
    let mut body = Vec::new();
    for row in data {
        let Value::Text(payload) = &row[1] else { return Err("journal : contenu absent".into()) };
        body.extend_from_slice(payload.as_bytes());
        body.push(b'\n');
    }
    Ok(body)
}

fn transaction<T>(c: &Connection, write: impl FnOnce() -> R<T>) -> R<T> {
    exec(c, "BEGIN IMMEDIATE")?;
    match write() {
        Ok(result) => { exec(c, "COMMIT")?; Ok(result) }
        Err(error) => { let _ = exec(c, "ROLLBACK"); Err(error) }
    }
}

/// Acquittement reçu durablement / publié. Une nouvelle génération peut reculer
/// jusqu'au plancher de reprise. Les opérations à republier repartent pour une
/// fenêtre entière, même si leur première publication datait d'avant restauration.
pub fn acknowledge(c: &Connection, generation: i64, durable: i64, published: i64, at: i64) -> R<()> {
    transaction(c, || {
        let ack = rows(c, "SELECT durable_upto, published_upto, generation, replay_floor FROM archive_ack", ())?;
        let ack = &ack[0];
        let next = one(c, "SELECT next_seq FROM meta", ())?;
        if at < 0 || published < 0 || published > durable || durable >= next || durable < int(&ack[3]) {
            return Err("accusé invalide ou sauvegarde hors de la fenêtre de reprise".into());
        }
        if generation == int(&ack[2]) && (durable < int(&ack[0]) || published < int(&ack[1])) {
            return Err("accusé rétrograde dans la même génération".into());
        }
        if generation != int(&ack[2]) {
            block_on(c.execute("UPDATE export_journal SET published_at = NULL WHERE seq > ?1", (published,)))?;
        }
        let from = if generation == int(&ack[2]) { int(&ack[1]) } else { 0 };
        block_on(c.execute("UPDATE export_journal SET published_at = ?1 WHERE seq > ?2 AND seq <= ?3 AND published_at IS NULL", (at, from, published)))?;
        block_on(c.execute("DELETE FROM outbox WHERE seq <= ?1", (published,)))?;
        block_on(c.execute("UPDATE archive_ack SET generation = ?1, durable_upto = ?2, published_upto = ?3, at = ?4", (generation, durable, published, at)))?;
        Ok(())
    })
}

/// Ne retire qu'un préfixe contigu publié depuis au moins sept jours. Un recul
/// d'horloge ou une publication partielle laisse le préfixe protégé en place.
/// Le plancher persistant permet de refuser une restauration trop ancienne.
pub fn prune(c: &Connection, now: i64, limit: i64) -> R<u64> {
    if !(1..=10_000).contains(&limit) { return Err("journal : tranche de purge invalide".into()) }
    transaction(c, || {
        let ack = rows(c, "SELECT published_upto, replay_floor FROM archive_ack", ())?;
        let (published, floor) = (int(&ack[0][0]), int(&ack[0][1]));
        let data = rows(c, "SELECT seq, published_at FROM export_journal WHERE seq > ?1 ORDER BY seq LIMIT ?2", (floor, limit))?;
        let mut end = floor;
        for row in data {
            let seq = int(&row[0]);
            if seq != end + 1 { return Err("journal : trou avant purge".into()) }
            let Value::Integer(at) = row[1] else { break };
            if seq > published || at > now.saturating_sub(REPLAY_WINDOW_MS) { break }
            end = seq;
        }
        if end == floor { return Ok(0) }
        let deleted = block_on(c.execute("DELETE FROM export_journal WHERE seq > ?1 AND seq <= ?2", (floor, end)))?;
        block_on(c.execute("UPDATE archive_ack SET replay_floor = ?1", (end,)))?;
        Ok(deleted)
    })
}

fn fixture(dir: &str, name: &str) -> R<(turso::Database, Connection, String)> {
    let path = format!("{dir}/{name}.db");
    crate::util::remove_db(&path);
    let (db, c) = crate::util::open(&path)?;
    exec(&c, crate::schema::TARGET)?;
    exec(&c, "INSERT INTO catalog (id, point, family) VALUES (1, 'fixture:state/value', 'states'); INSERT INTO meter_state (id, meter) VALUES (0, 'meter-test')")?;
    Ok((db, c, path))
}

fn write_hour(c: &Connection, seq: i64, cost: Option<f64>) -> R<()> {
    transaction(c, || {
        block_on(c.execute(crate::schema::ENERGY_UPSERT, (0i64, 0i64, 1.0, cost, seq)))?;
        append(c, seq, OP_ENERGY_HOUR, 0)?;
        block_on(c.execute("UPDATE meta SET next_seq = ?1", (seq + 1,)))?;
        Ok(())
    })
}

pub fn proof_retry(dir: &str) -> R<Json> {
    let (db, c, path) = fixture(dir, "journal-retry")?;
    write_hour(&c, 1, None)?;
    let original = body(&c, 0, 1)?;
    write_hour(&c, 2, Some(0.2))?;
    drop(c);
    drop(db);
    let (_db, c) = crate::util::open(&path)?;
    let retry = body(&c, 0, 1)?;
    let operation: Json = serde_json::from_slice(&original)?;
    let all = body(&c, 0, 2)?;
    let operations: Vec<Json> = all.split(|b| *b == b'\n').filter(|s| !s.is_empty())
        .map(serde_json::from_slice).collect::<Result<_, _>>()?;
    let current = rows(&c, "SELECT kwh FROM energy_hour", ())?;
    let pass = original == retry && operation["kwh"] == 1.0 && operation["cost"].is_null()
        && operations.len() == 2 && operations[0]["seq"] == 1 && operations[1]["seq"] == 2
        && operations[1]["kwh"] == 2.0 && crate::util::real(&current[0][0]) == 2.0;
    Ok(json!({"pass": pass, "retry_bytes_identical_after_update_and_reopen": original == retry,
              "first_kwh": operation["kwh"], "latest_kwh": operations[1]["kwh"], "unknown_cost_kept_null": operation["cost"].is_null()}))
}

pub fn proof_restore(dir: &str) -> R<Json> {
    let (db, c, path) = fixture(dir, "journal-restore")?;
    write_hour(&c, 1, None)?;
    write_hour(&c, 2, Some(0.2))?;
    transaction(&c, || {
        block_on(c.execute(crate::schema::SAMPLE_INSERT, (1i64, 0i64, 3i64, 3.0)))?;
        append(&c, 3, OP_SAMPLE, crate::schema::key(1, 0))?;
        exec(&c, "UPDATE meta SET next_seq = 4")
    })?;
    let expected = body(&c, 0, 3)?;
    let published_at = 10 * 86_400_000;
    acknowledge(&c, 1, 3, 3, published_at)?;
    let pending = one(&c, "SELECT count(*) FROM outbox", ())?;
    // L'import est ancien en temps métier mais nouveau dans le journal : les
    // tables peuvent être purgées immédiatement après publication.
    exec(&c, "DELETE FROM samples; DELETE FROM energy_hour")?;
    let before_restore = prune(&c, published_at + 6 * 86_400_000, 10_000)?;
    drop(c);
    drop(db);
    let (_db, c) = crate::util::open(&path)?;
    let restored_at = published_at + 6 * 86_400_000;
    acknowledge(&c, 2, 0, 0, restored_at)?;
    let replay = body(&c, 0, 3)?;
    let operations: Vec<Json> = replay.split(|b| *b == b'\n').filter(|s| !s.is_empty())
        .map(serde_json::from_slice).collect::<Result<_, _>>()?;
    acknowledge(&c, 2, 3, 3, restored_at)?;
    let after_original_window = prune(&c, published_at + REPLAY_WINDOW_MS + 1, 10_000)?;
    let after_new_window = prune(&c, restored_at + REPLAY_WINDOW_MS + 1, 10_000)?;
    let expired = acknowledge(&c, 3, 0, 0, restored_at + REPLAY_WINDOW_MS + 1).is_err();
    let floor = one(&c, "SELECT replay_floor FROM archive_ack", ())?;
    let ordered = operations.iter().map(|o| o["seq"].as_i64().unwrap_or(-1)).collect::<Vec<_>>() == [1, 2, 3];
    let late_import = operations.iter().any(|o| o["type"] == "sample" && o["ts"] == 0 && o["num"] == 3.0);
    Ok(json!({"pass": pending == 0 && expected == replay && ordered && late_import
                  && before_restore == 0 && after_original_window == 0 && after_new_window == 3 && floor == 3 && expired,
              "pending_outbox_after_publish": pending, "replay_bytes_identical_after_hot_purge": expected == replay,
              "contiguous_sequences": ordered, "late_import_replayed_after_purge": late_import,
              "protected_before_window": before_restore == 0, "window_restarted_after_generation_change": after_original_window == 0,
              "expired_journal_rows_removed": after_new_window, "replay_floor": floor, "expired_restore_refused": expired}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn immutable_retry_survives_a_reopen() -> R<()> {
        let dir = format!("/tmp/moli-journal-retry-{}", std::process::id());
        std::fs::create_dir_all(&dir)?;
        let result = proof_retry(&dir)?;
        assert_eq!(result["pass"], true, "{result}");
        Ok(())
    }

    #[test]
    fn restoration_covers_late_imports_and_restarts_retention() -> R<()> {
        let dir = format!("/tmp/moli-journal-restore-{}", std::process::id());
        std::fs::create_dir_all(&dir)?;
        let result = proof_restore(&dir)?;
        assert_eq!(result["pass"], true, "{result}");
        Ok(())
    }
}
