//! Référence de taille : le même programme sans moteur SQL.
fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "/tmp/size.db".into());
    let n: i64 = path.len() as i64;
    println!("{}", serde_json::json!({ "engine": "none", "rows": n }));
}
