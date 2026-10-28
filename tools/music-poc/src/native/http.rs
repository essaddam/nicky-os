//! Bounded HTTP with deliberately non-diagnostic transport errors.
use anyhow::{Result, bail};
use reqwest::{Client, RequestBuilder};
use serde_json::Value;
use std::time::Duration;

pub(super) fn client() -> Result<Client> {
    Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .user_agent("Mozilla/5.0 (compatible; MoliMusicPOC/0.1)")
        .build()
        .map_err(|_| anyhow::anyhow!("native provider HTTP initialization failed"))
}

pub(super) async fn bytes(request: RequestBuilder, limit: usize) -> Result<Vec<u8>> {
    let mut response = request
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("native provider transport failed"))?;
    match response.status().as_u16() {
        200..=299 => {}
        401 | 403 => bail!("native provider authorization refused"),
        429 => bail!("native provider rate limited"),
        status => bail!("native provider request rejected (HTTP {status})"),
    }
    if response.content_length().is_some_and(|n| n > limit as u64) {
        bail!("native provider response exceeds bound");
    }
    let mut data = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow::anyhow!("native provider response interrupted"))?
    {
        if chunk.len() > limit.saturating_sub(data.len()) {
            bail!("native provider response exceeds bound");
        }
        data.extend_from_slice(&chunk);
    }
    Ok(data)
}

pub(super) async fn json(request: RequestBuilder) -> Result<Value> {
    serde_json::from_slice(&bytes(request, 1024 * 1024).await?)
        .map_err(|_| anyhow::anyhow!("native provider returned invalid JSON"))
}
