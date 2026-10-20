//! Readbacks are independent from command admission and carry no token.
use moli_core::Value;
use moli_net::{Method, PinnedHttps};
use serde_json::Value as Json;

pub(crate) async fn info(http: &PinnedHttps) -> anyhow::Result<Json> {
    let request = moli_net::empty(http.request(Method::GET, "/api/v2/"))?;
    let (status, bytes) = http.send(request).await?;
    anyhow::ensure!(
        status.is_success() && bytes.len() <= 64 * 1024,
        "invalid device info"
    );
    Ok(serde_json::from_slice(&bytes)?)
}

pub(crate) async fn poll(
    http: &PinnedHttps,
    host: &str,
    port: u16,
    native: &str,
) -> anyhow::Result<Vec<(&'static str, Value)>> {
    let json = info(http).await?;
    anyhow::ensure!(
        crate::model::native_id(&json).as_deref() == Some(native),
        "identity changed"
    );
    let mut values = vec![];
    if let Some(power) = json["device"]["PowerState"].as_str() {
        match power {
            "on" => values.push(("power", Value::Bool(true))),
            "standby" | "off" => values.push(("power", Value::Bool(false))),
            _ => {}
        }
    }
    // Some models do not expose RenderingControl. An audio failure never
    // turns a live TV offline or fabricates volume/mute observations.
    for (action, tag, key) in [
        ("GetVolume", "CurrentVolume", "volume"),
        ("GetMute", "CurrentMute", "mute"),
    ] {
        if let Ok(xml) = crate::audio::call(host, port, action, None, None).await
            && let Some(text) = moli_net::upnp::tag(&xml, tag)
        {
            let value = if key == "volume" {
                text.parse::<u8>()
                    .ok()
                    .filter(|v| *v <= 100)
                    .map(|v| Value::Int(i64::from(v)))
            } else {
                match text {
                    "1" | "true" => Some(Value::Bool(true)),
                    "0" | "false" => Some(Value::Bool(false)),
                    _ => None,
                }
            };
            if let Some(value) = value {
                values.push((key, value));
            }
        }
    }
    Ok(values)
}
