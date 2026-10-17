//! Taille : le moteur `turso_core` 0.8.3 seul, sans fonctions par défaut, API
//! synchrone (ni `turso`, ni le kit de synchronisation, ni tracing-subscriber).
use std::sync::Arc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).unwrap_or_else(|| "/tmp/size-core.db".into());
    let io: Arc<dyn turso_core::IO> = Arc::new(turso_core::PlatformIO::new()?);
    let db = turso_core::Database::open_file(io, &path, Arc::new(turso_core::SqliteDialect))?;
    let c = db.connect()?;
    c.execute("CREATE TABLE IF NOT EXISTS t (id INTEGER PRIMARY KEY, k INTEGER, v REAL)")?;
    c.execute("CREATE INDEX IF NOT EXISTS t_k ON t (k)")?;
    c.execute("BEGIN")?;
    for i in 0..100 {
        c.execute(format!("INSERT INTO t (k, v) VALUES ({i}, {i}.0) ON CONFLICT DO NOTHING"))?;
    }
    c.execute("COMMIT")?;
    let mut n = 0i64;
    if let Some(mut stmt) = c.query("SELECT count(*) FROM t WHERE k >= 0")? {
        stmt.run_with_row_callback(|row| {
            if let turso_core::Value::Numeric(turso_core::Numeric::Integer(v)) = row.get_value(0) {
                n = *v;
            }
            Ok(())
        })?;
    }
    println!("{{\"engine\":\"turso_core\",\"rows\":{n}}}");
    Ok(())
}
