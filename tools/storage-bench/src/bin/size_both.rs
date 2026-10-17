//! Taille pendant la période de retour arrière : SQLite et Turso liés ensemble.
#[path = "size_turso.rs"]
mod turso_part;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).unwrap_or_else(|| "/tmp/size-both.db".into());
    let c = rusqlite::Connection::open(format!("{path}.sqlite"))?;
    c.execute_batch("CREATE TABLE IF NOT EXISTS t (id INTEGER PRIMARY KEY, k INTEGER); INSERT INTO t (k) VALUES (1);")?;
    let n: i64 = c.query_row("SELECT count(*) FROM t", [], |r| r.get(0))?;
    println!("{}", serde_json::json!({ "engine": "sqlite", "rows": n }));
    turso_part::main()
}
