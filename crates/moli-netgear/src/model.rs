//! Native identities and allowlisted, typed observations.
use crate::{
    protocol::{self, Action},
    xml::Failure,
};
use moli_core::Value;
use std::collections::BTreeMap;
pub(crate) type Samples = BTreeMap<&'static str, Value>;
pub(crate) type Clients = BTreeMap<String, Samples>;
#[derive(Debug, Default)]
pub(crate) struct Read {
    pub identity: Option<String>,
    pub values: Samples,
    pub clients: Option<Clients>,
}

pub(crate) fn mac(value: &str) -> Option<String> {
    let hex = value
        .chars()
        .filter(|c| !matches!(c, ':' | '-' | '.'))
        .collect::<String>()
        .to_ascii_lowercase();
    (hex.len() == 12
        && hex.bytes().all(|b| b.is_ascii_hexdigit())
        && hex != "000000000000"
        && hex != "ffffffffffff")
        .then_some(hex)
}
pub(crate) fn identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}
fn known(value: Option<&str>) -> Option<&str> {
    value.filter(|s| !matches!(*s, "" | "--" | "unknown" | "<unknown>"))
}
fn integer(value: &str, min: i64, max: i64) -> Result<i64, Failure> {
    value
        .parse::<i64>()
        .ok()
        .filter(|v| (min..=max).contains(v))
        .ok_or(Failure::Invalid)
}
fn client(
    link: Option<&str>,
    signal: Option<&str>,
    rate: Option<&str>,
) -> Result<Samples, Failure> {
    let mut values = Samples::new();
    if let Some(link) = known(link) {
        let link = match link {
            "wired" | "Wired" | "Ethernet" => "wired",
            "wireless" | "Wireless" => "wireless",
            "2.4GHz" => "2.4GHz",
            "5GHz" => "5GHz",
            "6GHz" => "6GHz",
            _ => return Err(Failure::Invalid),
        };
        values.insert("link", Value::from(link));
    }
    if let Some(signal) = known(signal) {
        values.insert("signal", Value::Int(integer(signal, -200, 100)?));
    }
    // A bare numeric rate has no qualified unit. Explicit units are never
    // inferred from an interface name or another firmware's convention.
    if let Some(raw) = known(rate).and_then(|v| v.strip_suffix(" Mbit/s")) {
        let rate = raw.parse::<f64>().map_err(|_| Failure::Invalid)?;
        if !rate.is_finite() || !(0.0..=1_000_000.0).contains(&rate) {
            return Err(Failure::Invalid);
        }
        values.insert("rate", Value::Float(rate));
    }
    Ok(values)
}
fn insert(clients: &mut Clients, native: &str, samples: Samples) -> Result<(), Failure> {
    let native = mac(native).ok_or(Failure::Identity)?;
    if clients.len() >= 512 || clients.insert(native, samples).is_some() {
        return Err(Failure::Invalid);
    }
    Ok(())
}
fn inventory(node: roxmltree::Node<'_, '_>, action: Action) -> Result<Clients, Failure> {
    let mut lists = node
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "NewAttachDevice");
    let list = lists.next().ok_or(Failure::Invalid)?;
    if lists.next().is_some()
        || list
            .tag_name()
            .namespace()
            .is_some_and(|ns| Some(ns) != node.tag_name().namespace())
    {
        return Err(Failure::Invalid);
    }
    let mut clients = Clients::new();
    if action == Action::ClientsV1 {
        if list.children().any(|n| n.is_element()) {
            return Err(Failure::Invalid);
        }
        let text = list.text().ok_or(Failure::Invalid)?.trim();
        if text == "0" {
            return Ok(clients);
        }
        let mut entries = text.split('@');
        let count = entries
            .next()
            .and_then(|n| n.parse::<usize>().ok())
            .filter(|n| *n <= 512)
            .ok_or(Failure::Invalid)?;
        for entry in entries {
            let fields = entry.split(';').collect::<Vec<_>>();
            if !matches!(fields.len(), 4 | 7 | 8) {
                return Err(Failure::Invalid);
            }
            let samples = if fields.len() >= 7 {
                client(Some(fields[4]), Some(fields[6]), Some(fields[5]))?
            } else {
                Samples::new()
            };
            insert(&mut clients, fields[3], samples)?;
        }
        if clients.len() != count {
            return Err(Failure::Invalid);
        }
    } else {
        if list
            .children()
            .any(|n| n.is_text() && !n.text().unwrap_or("").trim().is_empty())
        {
            return Err(Failure::Invalid);
        }
        for device in list.children().filter(roxmltree::Node::is_element) {
            if device.tag_name().name() != "Device"
                || device
                    .tag_name()
                    .namespace()
                    .is_some_and(|ns| Some(ns) != list.tag_name().namespace())
            {
                return Err(Failure::Invalid);
            }
            let native = protocol::field(device, "MAC")?.ok_or(Failure::Identity)?;
            let samples = client(
                protocol::field(device, "ConnectionType")?,
                protocol::field(device, "SignalStrength")?,
                protocol::field(device, "Linkspeed")?,
            )?;
            insert(&mut clients, native, samples)?;
        }
    }
    Ok(clients)
}
pub(crate) fn parse(action: Action, body: &[u8]) -> Result<Read, Failure> {
    let doc = protocol::decode(body, action)?;
    let node = protocol::payload(&doc, action)?;
    let mut read = Read::default();
    match action {
        Action::Info => {
            let native = protocol::field(node, "SerialNumber")?
                .filter(|s| identity(s))
                .ok_or(Failure::Identity)?;
            read.identity = Some(native.into());
            if let Some(version) = known(protocol::field(node, "Firmwareversion")?) {
                if version.len() > 128 || version.chars().any(char::is_control) {
                    return Err(Failure::Invalid);
                }
                read.values.insert("firmware", Value::from(version));
            }
        }
        Action::System => {
            for (field, key) in [
                ("NewCPUUtilization", "cpu"),
                ("NewMemoryUtilization", "memory"),
            ] {
                if let Some(value) = known(protocol::field(node, field)?) {
                    read.values.insert(
                        key,
                        Value::Int(integer(value.trim_end_matches('%').trim(), 0, 100)?),
                    );
                }
            }
        }
        Action::Ethernet => {
            if let Some(value) = known(protocol::field(node, "NewEthernetLinkStatus")?) {
                let online = match value {
                    "Up" => true,
                    "Down" => false,
                    _ => return Err(Failure::Invalid),
                };
                read.values.insert("ethernet", Value::Bool(online));
            }
        }
        Action::Traffic => {
            if let Some(value) = known(protocol::field(node, "NewTodayConnectionTime")?) {
                let (hours, minutes) = value.split_once(':').ok_or(Failure::Invalid)?;
                let seconds = integer(hours, 0, 1_000_000)?
                    .checked_mul(3600)
                    .and_then(|h| integer(minutes, 0, 59).ok().map(|m| h + m * 60))
                    .ok_or(Failure::Invalid)?;
                read.values.insert("today_connection", Value::Int(seconds));
            }
        }
        Action::ClientsV1 | Action::ClientsV2 => {
            read.clients = Some(inventory(node, action)?);
        }
        Action::LoginV1 | Action::LoginV2 => {}
    }
    Ok(read)
}
#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
