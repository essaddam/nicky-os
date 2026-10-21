//! Fixed fields and explicit units; identifiers never become measurements.
use crate::{Failure, xml};
use moli_core::Value;
use std::collections::BTreeMap;
pub(crate) type Samples = BTreeMap<&'static str, Value>;
pub(crate) const MAX_EXACT: u64 = (1_u64 << 53) - 1;
#[derive(Debug, Default)]
pub(crate) struct Reading {
    pub identity: Option<String>,
    pub values: Samples,
    pub counters: Option<(u64, u64)>,
    pub duration: Option<u64>,
    pub connected: Option<bool>,
    pub sim: Option<bool>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Action {
    Info,
    Status,
    Signal,
    Traffic,
}
pub(crate) fn identity(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}
fn number(raw: Option<&String>) -> Result<Option<u64>, Failure> {
    match raw.map(String::as_str) {
        None | Some("" | "--" | "N/A") => Ok(None),
        Some(s) => s.parse().map(Some).map_err(|_| Failure::Invalid),
    }
}
fn integer(values: &mut Samples, key: &'static str, n: Option<u64>) {
    if let Some(n) = n.filter(|n| *n <= MAX_EXACT) {
        values.insert(
            key,
            Value::Int(i64::try_from(n).expect("below exact IEEE integer limit")),
        );
    }
}
fn signal(raw: &str, unit: &str, min: f64, max: f64) -> Result<Option<f64>, Failure> {
    if raw.is_empty()
        || matches!(
            raw,
            "--" | "N/A" | "unknown" | "255" | "2147483647" | "-32768"
        )
        || raw.starts_with(['<', '>'])
    {
        return Ok(None);
    }
    let Some(number) = raw.strip_suffix(unit) else {
        return Ok(None);
    };
    let number = number.trim();
    if matches!(number, "255" | "2147483647" | "-32768") {
        return Ok(None);
    }
    if number.is_empty()
        || !number
            .bytes()
            .all(|b| b.is_ascii_digit() || b"-+.".contains(&b))
    {
        return Err(Failure::Invalid);
    }
    let value = number
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && (min..=max).contains(v))
        .ok_or(Failure::Invalid)?;
    Ok(Some(value))
}
pub(crate) fn parse(action: Action, body: &[u8]) -> Result<Reading, Failure> {
    let fields = xml::parse(body)?;
    if fields.ok {
        return Err(Failure::Invalid);
    }
    let f = &fields.values;
    let mut r = Reading::default();
    match action {
        Action::Info => {
            let id = f
                .get("SerialNumber")
                .filter(|id| identity(id))
                .ok_or(Failure::Identity)?;
            r.identity = Some(id.clone());
        }
        Action::Status => {
            r.sim = match f.get("SimStatus").map(String::as_str) {
                Some("0") => Some(false),
                Some("1") => Some(true),
                _ => None,
            };
            r.connected = if r.sim == Some(false) {
                Some(false)
            } else {
                match f.get("ConnectionStatus").map(String::as_str) {
                    Some("901") => Some(true),
                    Some("902" | "904") => Some(false),
                    _ => None,
                }
            };
            if let Some(value) = r.connected {
                r.values.insert("connected", Value::Bool(value));
            }
            if r.sim != Some(false)
                && let Some(n) =
                    number(f.get("CurrentNetworkType"))?.filter(|n| *n > 0 && *n <= 65_535)
            {
                integer(&mut r.values, "technology", Some(n));
            }
        }
        Action::Signal => {
            for (key, unit, min, max) in [
                ("rssi", "dBm", -200.0, 0.0),
                ("rsrp", "dBm", -200.0, 0.0),
                ("rsrq", "dB", -50.0, 30.0),
                ("sinr", "dB", -50.0, 100.0),
            ] {
                if let Some(raw) = f.get(key)
                    && let Some(value) = signal(raw, unit, min, max)?
                {
                    r.values.insert(key, Value::Float(value));
                }
            }
        }
        Action::Traffic => {
            let rx = number(f.get("CurrentDownload"))?;
            let tx = number(f.get("CurrentUpload"))?;
            r.counters = rx.zip(tx);
            r.duration = number(f.get("CurrentConnectTime"))?;
            for (key, value) in [
                ("rx_bytes", rx),
                ("tx_bytes", tx),
                ("duration", r.duration),
                ("rx_rate", number(f.get("CurrentDownloadRate"))?),
                ("tx_rate", number(f.get("CurrentUploadRate"))?),
            ] {
                integer(&mut r.values, key, value);
            }
        }
    }
    Ok(r)
}
