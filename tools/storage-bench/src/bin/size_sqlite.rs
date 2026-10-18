//! Taille : SQLite embarqué (rusqlite bundled, comme Moli aujourd'hui).
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).unwrap_or_else(|| "/tmp/size-sqlite.db".into());
    let c = rusqlite::Connection::open(&path)?;
    c.execute_batch("CREATE TABLE IF NOT EXISTS t (id INTEGER PRIMARY KEY, k INTEGER, v REAL); CREATE INDEX IF NOT EXISTS t_k ON t (k);")?;
    let tx = c.unchecked_transaction()?;
    for i in 0..100i64 {
        tx.execute("INSERT INTO t (k, v) VALUES (?1, ?2) ON CONFLICT DO NOTHING", rusqlite::params![i, i as f64])?;
    }
    tx.commit()?;
    let n: i64 = c.query_row("SELECT count(*) FROM t WHERE k >= ?1", [0], |r| r.get(0))?;
    println!("{}", serde_json::json!({ "engine": "sqlite", "rows": n }));
    Ok(())
}
