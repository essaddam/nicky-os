//! Taille : Turso 0.8.3 sans fonctions par défaut.
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::yield_now();
    }
}

pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).unwrap_or_else(|| "/tmp/size-turso.db".into());
    let db = block_on(turso::Builder::new_local(&path).build())?;
    let c = db.connect()?;
    block_on(c.execute_batch("CREATE TABLE IF NOT EXISTS t (id INTEGER PRIMARY KEY, k INTEGER, v REAL); CREATE INDEX IF NOT EXISTS t_k ON t (k);"))?;
    block_on(c.execute("BEGIN", ()))?;
    for i in 0..100i64 {
        block_on(c.execute("INSERT INTO t (k, v) VALUES (?1, ?2) ON CONFLICT DO NOTHING", (i, i as f64)))?;
    }
    block_on(c.execute("COMMIT", ()))?;
    let n = block_on(async {
        let mut rows = c.query("SELECT count(*) FROM t WHERE k >= ?1", (0i64,)).await?;
        let row = rows.next().await?.ok_or("no row")?;
        Ok::<_, Box<dyn std::error::Error>>(row.get_value(0)?)
    })?;
    println!("{}", serde_json::json!({ "engine": "turso", "rows": format!("{n:?}") }));
    Ok(())
}
