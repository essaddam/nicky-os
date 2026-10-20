//! Fixed JSON schemas from the pinned appGet protocol, with explicit units.
use crate::{Failure, json};
use moli_core::Value;
use serde_json::Value as Json;
use std::collections::{BTreeMap, BTreeSet};
pub(crate) type Samples = BTreeMap<&'static str, Value>;
pub(crate) type Clients = BTreeMap<String, Samples>;
#[derive(Debug, Default)]
pub(crate) struct Reading {
    pub identity: Option<String>,
    pub model: Option<String>,
    pub values: Samples,
    pub clients: Option<Clients>,
    pub cpu: Option<BTreeMap<String, (u64, u64)>>,
    pub wan: Option<(u64, u64)>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Action {
    Info,
    Telemetry,
    Clients,
}
pub(crate) fn mac(raw: &str) -> Option<String> {
    let value = raw.to_ascii_lowercase();
    let compact = if value.len() == 12 {
        value
    } else if value.len() == 17 {
        let sep = value.as_bytes()[2];
        if !matches!(sep, b':' | b'-')
            || !value.bytes().enumerate().all(|(i, b)| {
                if i % 3 == 2 {
                    b == sep
                } else {
                    b.is_ascii_hexdigit()
                }
            })
        {
            return None;
        }
        value.chars().filter(|c| !matches!(c, ':' | '-')).collect()
    } else {
        return None;
    };
    (compact.bytes().all(|b| b.is_ascii_hexdigit())
        && compact != "000000000000"
        && compact != "ffffffffffff")
        .then_some(compact)
}
fn number(v: Option<&Json>) -> Result<Option<u64>, Failure> {
    match v {
        None | Some(Json::Null) => Ok(None),
        Some(Json::String(s)) if s.is_empty() || s == "--" => Ok(None),
        Some(Json::String(s)) => s.parse::<u64>().map(Some).map_err(|_| Failure::Invalid),
        Some(v) => v.as_u64().map(Some).ok_or(Failure::Invalid),
    }
}
fn object<'a>(
    v: &'a Json,
    key: &str,
) -> Result<Option<&'a serde_json::Map<String, Json>>, Failure> {
    match v.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(v) => v.as_object().map(Some).ok_or(Failure::Invalid),
    }
}
fn bytes(samples: &mut Samples, key: &'static str, value: u64) -> Result<(), Failure> {
    let n = i64::try_from(value).map_err(|_| Failure::Invalid)?;
    samples.insert(key, Value::Int(n));
    Ok(())
}
fn telemetry(v: &Json) -> Result<Reading, Failure> {
    let mut r = Reading::default();
    let mut found = false;
    if let Some(cpu) = object(v, "cpu_usage")? {
        found = true;
        let mut counters = BTreeMap::new();
        for (key, val) in cpu {
            if let Some(core) = key
                .strip_prefix("cpu")
                .and_then(|s| s.strip_suffix("_total"))
            {
                let index = core.parse::<u8>().map_err(|_| Failure::Invalid)?;
                if !(1..=64).contains(&index) {
                    return Err(Failure::Invalid);
                }
                if let (Some(total), Some(used)) = (
                    number(Some(val))?,
                    number(cpu.get(&format!("cpu{index}_usage")))?,
                ) {
                    if used > total {
                        return Err(Failure::Invalid);
                    }
                    counters.insert(core.to_owned(), (total, used));
                }
            }
        }
        if !counters.is_empty() {
            r.cpu = Some(counters);
        }
    }
    if let Some(mem) = object(v, "memory_usage")? {
        found = true;
        let total = number(mem.get("mem_total"))?;
        let used = number(mem.get("mem_used"))?;
        if used.zip(total).is_some_and(|(u, t)| u > t) {
            return Err(Failure::Invalid);
        }
        for (wire, key) in [
            ("mem_total", "memory_total"),
            ("mem_used", "memory_used"),
            ("mem_free", "memory_free"),
        ] {
            if let Some(n) = number(mem.get(wire))? {
                bytes(
                    &mut r.values,
                    key,
                    n.checked_mul(1024).ok_or(Failure::Invalid)?,
                )?;
            }
        }
        if let Some((u, t)) = used.zip(total).filter(|(_, t)| *t > 0) {
            // Ratios are approximate floats; byte counters remain exact integers.
            #[allow(clippy::cast_precision_loss)]
            let percent = u as f64 / t as f64 * 100.0;
            r.values.insert("memory", Value::Float(percent));
        }
    }
    if let Some(wan) = object(v, "netdev")? {
        found = true;
        let counter = |key| -> Result<Option<u64>, Failure> {
            match wan.get(key) {
                None | Some(Json::Null) => Ok(None),
                Some(Json::String(s)) => {
                    let s = s.strip_prefix("0x").ok_or(Failure::Invalid)?;
                    u64::from_str_radix(s, 16)
                        .map(Some)
                        .map_err(|_| Failure::Invalid)
                }
                _ => Err(Failure::Invalid),
            }
        };
        if let Some((rx, tx)) = counter("INTERNET_rx")?.zip(counter("INTERNET_tx")?) {
            bytes(&mut r.values, "rx_bytes", rx)?;
            bytes(&mut r.values, "tx_bytes", tx)?;
            r.wan = Some((rx, tx));
        }
    }
    if !found {
        return Err(Failure::Invalid);
    }
    Ok(r)
}
fn online(v: Option<&Json>) -> Result<bool, Failure> {
    match v {
        None | Some(Json::Null) => Ok(false),
        Some(Json::Bool(b)) => Ok(*b),
        Some(v) if v.as_str() == Some("1") || v.as_u64() == Some(1) => Ok(true),
        Some(v) if v.as_str() == Some("0") || v.as_u64() == Some(0) => Ok(false),
        _ => Err(Failure::Invalid),
    }
}
fn clients(v: &Json) -> Result<Reading, Failure> {
    let raw = object(v, "get_clientlist")?.ok_or(Failure::Invalid)?;
    let mut clients = Clients::new();
    let mut seen = BTreeSet::new();
    for (key, entry) in raw {
        if matches!(key.as_str(), "maclist" | "ClientAPILevel") {
            continue;
        }
        if seen.len() >= 512 {
            return Err(Failure::Invalid);
        }
        let id = mac(key).ok_or(Failure::Identity)?;
        if !seen.insert(id.clone()) {
            return Err(Failure::Invalid);
        }
        let entry = entry.as_object().ok_or(Failure::Invalid)?;
        if online(entry.get("amesh_isRe"))? {
            continue;
        }
        let mut values = Samples::new();
        if online(entry.get("isOnline"))? {
            if let Some(link) = number(entry.get("isWL"))? {
                values.insert(
                    "link",
                    Value::from(match link {
                        0 => "wired",
                        1..=4 => "wireless",
                        _ => return Err(Failure::Invalid),
                    }),
                );
            }
            if let Some(s) = entry
                .get("rssi")
                .filter(|v| !v.is_null() && v.as_str() != Some(""))
            {
                let n = if let Some(s) = s.as_str() {
                    s.parse::<i64>().ok()
                } else {
                    s.as_i64()
                }
                .filter(|n| (-200..=0).contains(n))
                .ok_or(Failure::Invalid)?;
                values.insert("signal", Value::Int(n));
            }
            for (wire, key) in [("curRx", "rate_rx"), ("curTx", "rate_tx")] {
                if let Some(v) = entry
                    .get(wire)
                    .filter(|v| !v.is_null() && v.as_str() != Some("") && v.as_str() != Some("--"))
                {
                    let n = if let Some(s) = v.as_str() {
                        s.parse::<f64>().ok()
                    } else {
                        v.as_f64()
                    }
                    .filter(|n| n.is_finite() && (0.0..=1_000_000.0).contains(n))
                    .ok_or(Failure::Invalid)?;
                    values.insert(key, Value::Float(n * 1.048_576));
                }
            }
        }
        clients.insert(id, values);
    }
    if let Some(list) = raw.get("maclist") {
        let list = list.as_array().ok_or(Failure::Invalid)?;
        let mut stated = BTreeSet::new();
        for id in list {
            if !stated.insert(mac(id.as_str().ok_or(Failure::Invalid)?).ok_or(Failure::Invalid)?) {
                return Err(Failure::Invalid);
            }
        }
        if stated != seen {
            return Err(Failure::Invalid);
        }
    }
    Ok(Reading {
        clients: Some(clients),
        ..Reading::default()
    })
}
pub(crate) fn parse(action: Action, body: &[u8]) -> Result<Reading, Failure> {
    let v = json::parse(body)?;
    match action {
        Action::Info => {
            let id = v
                .get("label_mac")
                .and_then(Json::as_str)
                .and_then(mac)
                .ok_or(Failure::Identity)?;
            let mut r = Reading {
                identity: Some(id),
                ..Reading::default()
            };
            for (key, wire) in [("firmware", "firmver"), ("model", "productid")] {
                if let Some(text) = v.get(wire).and_then(Json::as_str).filter(|s| !s.is_empty()) {
                    if text.len() > 128 {
                        return Err(Failure::Invalid);
                    }
                    if key == "model" {
                        r.model = Some(text.to_owned());
                    } else {
                        r.values.insert(key, Value::from(text));
                    }
                }
            }
            Ok(r)
        }
        Action::Telemetry => telemetry(&v),
        Action::Clients => clients(&v),
    }
}
