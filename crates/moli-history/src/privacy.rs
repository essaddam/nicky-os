//! Startup-only repair, before read handles or the recorder exist.
//! Never select a code by value: credential points are identified by metadata.
use crate::HistoryError;
use moli_core::privacy;
use rusqlite::{Connection, params};

// No function of txt here: building the index must not load a giant old value.
// It contains identifiers and timestamps, never the text payload.
const TEXT_CANDIDATES: &str = "kind = 't'";

pub(crate) fn repair(conn: &mut Connection) -> Result<(), HistoryError> {
    conn.pragma_update(None, "secure_delete", true)?;
    conn.execute_batch(&format!(
        "CREATE INDEX IF NOT EXISTS samples_private_text ON samples(point_id,ts) WHERE {TEXT_CANDIDATES}"
    ))?;
    let tx = conn.transaction()?;
    let mut removed = 0;
    let mut first = i64::MIN;
    loop {
        let ids: Vec<(i64, String)> = tx
            .prepare("SELECT id, point FROM points WHERE id >= ?1 ORDER BY id LIMIT 128")?
            .query_map([first], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?;
        if ids.is_empty() {
            break;
        }
        for (id, point) in &ids {
            if privacy::code_point(point) {
                removed += tx.execute("DELETE FROM samples WHERE point_id = ?1", [id])?;
                tx.execute("DELETE FROM retention_consent WHERE point_id = ?1", [id])?;
                tx.execute("DELETE FROM retention WHERE point_id = ?1", [id])?;
                tx.execute("DELETE FROM points WHERE id = ?1", [id])?;
                removed += 1;
            }
        }
        let Some(next) = ids.last().unwrap().0.checked_add(1) else {
            break;
        };
        first = next;
    }
    let mut cursor = (i64::MIN, i64::MIN);
    loop {
        let sql = format!(
            "SELECT point_id, ts, octet_length(txt) FROM samples INDEXED BY samples_private_text WHERE {TEXT_CANDIDATES} AND (point_id,ts) >= (?1,?2) ORDER BY point_id,ts LIMIT 64"
        );
        let rows: Vec<(i64, i64, i64)> = tx
            .prepare(&sql)?
            .query_map(params![cursor.0, cursor.1], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .collect::<Result<_, _>>()?;
        if rows.is_empty() {
            break;
        }
        for (point, at, bytes) in &rows {
            // octet_length reads the stored size, not the text. Refuse an
            // uninspectable legacy value before loading it; never delete an
            // ordinary oversized value merely to make startup succeed.
            if *bytes > i64::try_from(privacy::MAX_JSON_TEXT_BYTES).unwrap() {
                return Err(HistoryError::Internal(
                    "private_history_text_too_large".into(),
                ));
            }
            let private = tx.query_row(
                "SELECT txt FROM samples WHERE point_id=?1 AND ts=?2",
                params![point, at],
                |row| {
                    let value = row.get_ref(0)?;
                    let text = value.as_str().map_err(|_| {
                        rusqlite::Error::InvalidColumnType(0, "txt".into(), value.data_type())
                    })?;
                    Ok(privacy::private_text(text))
                },
            )?;
            if private {
                removed += tx.execute(
                    "DELETE FROM samples WHERE point_id=?1 AND ts=?2",
                    params![point, at],
                )?;
            }
        }
        let (point, at, _) = rows.last().unwrap();
        cursor = match at.checked_add(1) {
            Some(next) => (*point, next),
            None => match point.checked_add(1) {
                Some(next) => (next, i64::MIN),
                None => break,
            },
        };
    }
    tx.commit()?;
    // Also completes a prior interrupted repair. A busy checkpoint is a failure,
    // never permission to serve while old WAL frames still hold private bytes.
    let busy: i64 = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))?;
    if busy != 0 {
        return Err(HistoryError::Internal(
            "private_history_checkpoint_busy".into(),
        ));
    }
    if removed != 0 {
        tracing::warn!(removed, "private keypad history removed");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;
    use moli_core::Value;
    use std::{fs, path::PathBuf};
    fn fixture() -> (PathBuf, String, Connection) {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "moli-history-private-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.db");
        drop(Store::open(&path).unwrap());
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA wal_autocheckpoint=0; INSERT INTO points(id,point) VALUES(1,'fixture:panel/action_code'),(2,'fixture:panel/battery'),(3,'fixture:panel/action'),(4,'fixture:panel/settings.pin')").unwrap();
        let code = format!("0{stamp}");
        conn.execute(
            "INSERT INTO samples(point_id,ts,kind,txt) VALUES(1,1,'t',?1)",
            [&code],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO samples(point_id,ts,kind,num) VALUES(2,1,'i',81)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO samples(point_id,ts,kind,txt) VALUES(3,1,'t',?1)",
            [serde_json::json!({"pin_code":code}).to_string()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO samples(point_id,ts,kind,txt) VALUES(4,1,'t',?1)",
            [&code],
        )
        .unwrap();
        (dir, code, conn)
    }
    #[test]
    fn startup_repairs_history_and_wal_and_preserves_legitimate_samples() {
        let (dir, code, old_writer) = fixture();
        let store = Store::open(&dir.join("history.db")).unwrap();
        let conn = Connection::open(dir.join("history.db")).unwrap();
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM samples", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            conn.query_row("SELECT num FROM samples", [], |r| r.get::<_, f64>(0))
                .unwrap(),
            81.0
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM points WHERE point LIKE '%action_code'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        for name in ["history.db", "history.db-wal"] {
            if let Ok(bytes) = fs::read(dir.join(name)) {
                assert!(
                    !bytes
                        .windows(code.len())
                        .any(|part| part == code.as_bytes()),
                    "credential left in active database files"
                );
            }
        }
        store
            .insert(
                &[(
                    "fixture:panel/pinCode".into(),
                    Value::from(code.as_str()),
                    2,
                )],
                false,
            )
            .unwrap();
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM samples", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        // The repair must still work after restoring an old snapshot or reopening.
        drop(store);
        drop(conn);
        drop(old_writer);
        let store = Store::open(&dir.join("history.db")).unwrap();
        assert!(
            store
                .insert(
                    &[(
                        "fixture:panel/ordinary".into(),
                        Value::Text("x".repeat(privacy::MAX_JSON_TEXT_BYTES + 1).into()),
                        3
                    )],
                    false
                )
                .is_err(),
            "new oversized text would poison the next restart"
        );
        drop(store);
        drop(Store::open(&dir.join("history.db")).unwrap());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn a_busy_wal_repair_fails_closed_then_recovers() {
        let (dir, _code, old_writer) = fixture();
        let reader = Connection::open(dir.join("history.db")).unwrap();
        reader
            .execute_batch("BEGIN; SELECT point_id FROM samples LIMIT 1")
            .unwrap();
        assert!(
            Store::open(&dir.join("history.db")).is_err(),
            "busy repair must stop startup"
        );
        reader.execute_batch("ROLLBACK").unwrap();
        drop(reader);
        drop(old_writer);
        drop(Store::open(&dir.join("history.db")).unwrap());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn repair_uses_bounded_keyset_scans_and_indexed_deletes() {
        let (dir, _code, old_writer) = fixture();
        let mut conn = Connection::open(dir.join("history.db")).unwrap();
        repair(&mut conn).unwrap();
        let detail: String = conn
            .query_row(
                "EXPLAIN QUERY PLAN DELETE FROM samples WHERE point_id=?1",
                [1],
                |r| r.get(3),
            )
            .unwrap();
        assert!(
            detail.contains("SEARCH") && detail.contains("PRIMARY KEY"),
            "{detail}"
        );
        let query = format!(
            "EXPLAIN QUERY PLAN SELECT point_id,ts,octet_length(txt) FROM samples INDEXED BY samples_private_text WHERE {TEXT_CANDIDATES} AND (point_id,ts)>=(?1,?2) ORDER BY point_id,ts LIMIT 64"
        );
        let detail: String = conn
            .query_row(&query, params![i64::MIN, i64::MIN], |r| r.get(3))
            .unwrap();
        assert!(detail.contains("samples_private_text"), "{detail}");
        drop(conn);
        drop(old_writer);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn large_ordinary_json_is_scanned_without_accumulating_a_text_batch() {
        let (dir, _code, writer) = fixture();
        // The old store can produce these. Keep only one 2 MiB value in the
        // fixture, and drop its connection before exercising startup repair.
        let text = serde_json::json!({"ordinary": "x".repeat(2 * 1024 * 1024)}).to_string();
        {
            let mut insert = writer
                .prepare("INSERT INTO samples(point_id,ts,kind,txt) VALUES(3,?1,'t',?2)")
                .unwrap();
            for at in 2..66 {
                insert.execute(params![at, &text]).unwrap();
            }
        }
        drop(text);
        drop(writer);
        let store = Store::open(&dir.join("history.db")).unwrap();
        let conn = Connection::open(dir.join("history.db")).unwrap();
        assert_eq!(
            conn.query_row("SELECT count(*) FROM samples WHERE point_id=3", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            64
        );
        drop(conn);
        drop(store);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn oversized_legacy_text_fails_closed_before_loading_then_recovers() {
        let (dir, _code, writer) = fixture();
        writer
            .execute(
                "INSERT INTO samples(point_id,ts,kind,txt) VALUES(3,2,'t',?1)",
                ["x".repeat(privacy::MAX_JSON_TEXT_BYTES + 1)],
            )
            .unwrap();
        drop(writer);
        assert!(
            Store::open(&dir.join("history.db")).is_err(),
            "oversized repair must refuse startup"
        );
        let conn = Connection::open(dir.join("history.db")).unwrap();
        // Refusal is transactional: do not silently delete ordinary old data.
        assert_eq!(
            conn.query_row(
                "SELECT count(*) FROM samples WHERE point_id=3 AND ts=2",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        conn.execute("DELETE FROM samples WHERE point_id=3 AND ts=2", [])
            .unwrap();
        drop(conn);
        drop(Store::open(&dir.join("history.db")).unwrap());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn startup_removes_uninspectable_json_text_and_keeps_the_ordinary_sample() {
        let (dir, code, writer) = fixture();
        for (at, text) in [
            (10, format!(r#"{{"pin":"{code}","tail":}}"#)),
            (11, format!(r#"{{"tail":,"pin":"{code}"}}"#)),
            (12, format!(r#"{{"pin":"{code}","n":1e400}}"#)),
            (13, format!(r#"{{"n":1e400,"pin":"{code}"}}"#)),
            (14, format!(r#"{{"pin":"{code}"}} trailing"#)),
        ] {
            writer
                .execute(
                    "INSERT INTO samples(point_id,ts,kind,txt) VALUES(3,?1,'t',?2)",
                    params![at, text],
                )
                .unwrap();
        }
        drop(writer);
        let store = Store::open(&dir.join("history.db")).unwrap();
        let reader = Connection::open(dir.join("history.db")).unwrap();
        assert_eq!(
            reader
                .query_row("SELECT count(*) FROM samples WHERE point_id=3", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0,
            "uninspectable private JSON survived restart"
        );
        assert_eq!(
            reader
                .query_row("SELECT num FROM samples WHERE point_id=2", [], |row| row
                    .get::<_, f64>(0))
                .unwrap(),
            81.0
        );
        drop(reader);
        drop(store);
        drop(Store::open(&dir.join("history.db")).unwrap());
        fs::remove_dir_all(dir).unwrap();
    }
}
