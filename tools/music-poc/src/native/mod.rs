//! Direct service clients. Catalogue access is separate from full audio access.
pub mod apple;
mod apple_token;
pub mod browser_import;
mod http;
pub mod pairing;
pub mod youtube;

use anyhow::{Result, bail};
use serde_json::{Value, json};

pub fn status() -> Value {
    json!({"native":true,"music_assistant_required":false,
        "apple_music":apple::status(),"ytmusic":youtube::status()})
}

pub async fn search(provider: &str, query: &str) -> Result<Value> {
    match provider {
        "apple_music" => apple::search(query).await,
        "ytmusic" => youtube::search(query).await,
        _ => bail!("unsupported native provider"),
    }
}

pub async fn account(provider: &str) -> Result<Value> {
    match provider {
        "apple_music" => apple::account().await,
        "ytmusic" => youtube::account().await,
        _ => bail!("unsupported native provider"),
    }
}

pub(super) fn query_is_valid(query: &str) -> bool {
    !query.trim().is_empty()
        && query.len() <= 800
        && query.chars().count() <= 200
        && !query.chars().any(char::is_control)
        && !query.contains("{{secret:")
}

pub(super) fn text(raw: &str, max: usize) -> String {
    raw.chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect::<String>()
        .trim()
        .to_owned()
}
