//! Bounded private-link DNS discovery. Discovery never activates a receiver.
use crate::{Config, dns};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Duration;

const SERVICE: &str = "_googlecast._tcp.local";
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub id: String,
    pub name: String,
    pub host: Ipv4Addr,
    pub hostname: String,
    pub video: bool,
}
pub(crate) fn native_id(id: &str) -> Option<String> {
    let compact = id.replace('-', "");
    (compact.len() == 32 && compact.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| compact.to_ascii_lowercase())
}
fn text_fields(bytes: &[u8]) -> Option<BTreeMap<&str, &str>> {
    let mut at = 0;
    let mut fields = BTreeMap::new();
    while at < bytes.len() {
        let size = usize::from(*bytes.get(at)?);
        at += 1;
        let text = std::str::from_utf8(bytes.get(at..at + size)?).ok()?;
        at += size;
        let (key, value) = text.split_once('=')?;
        if fields.insert(key, value).is_some() {
            return None;
        }
    }
    Some(fields)
}
fn candidate(
    packet: &[u8],
    records: &[dns::Record],
    instance: &str,
    source: Ipv4Addr,
) -> Option<Candidate> {
    let srv = records
        .iter()
        .find(|r| r.name == instance && r.kind == 33)?;
    if srv.data.len() < 7 || dns::word(packet, srv.data.start + 4)? != 8009 {
        return None;
    }
    let mut cursor = srv.data.start + 6;
    let hostname = dns::name(packet, &mut cursor)?;
    if cursor != srv.data.end {
        return None;
    }
    let address = records
        .iter()
        .find(|r| r.name == hostname && r.kind == 1 && r.data.len() == 4)?;
    if packet.get(address.data.clone())? != source.octets() {
        return None;
    }
    let txt = records
        .iter()
        .find(|r| r.name == instance && r.kind == 16)?;
    let fields = text_fields(packet.get(txt.data.clone())?)?;
    let id = native_id(fields.get("id")?)?;
    let label = *fields.get("fn")?;
    if label.is_empty() || label.len()>256 || label.chars().count()>80 || label.chars().any(|c|c.is_control() || matches!(c,'\u{200b}'..='\u{200f}'|'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}')) {return None;}
    let capabilities = fields.get("ca")?.parse::<u32>().ok()?;
    Some(Candidate {
        id,
        name: label.into(),
        host: source,
        hostname,
        video: capabilities & 1 != 0,
    })
}
pub fn parse_packet(packet: &[u8], source: Ipv4Addr, config: &Config) -> Vec<Candidate> {
    if !config.in_scope(source) {
        return Vec::new();
    }
    let Some(records) = dns::records(packet) else {
        return Vec::new();
    };
    let mut seen = BTreeSet::new();
    records
        .iter()
        .filter(|r| r.name == SERVICE && r.kind == 12)
        .filter_map(|ptr| {
            let mut cursor = ptr.data.start;
            let instance = dns::name(packet, &mut cursor)?;
            if cursor != ptr.data.end || !instance.ends_with(&format!(".{SERVICE}")) {
                return None;
            }
            let found = candidate(packet, &records, &instance, source)?;
            seen.insert(found.id.clone()).then_some(found)
        })
        .take(64)
        .collect()
}
pub async fn discover(config: &Config) -> anyhow::Result<Vec<Candidate>> {
    config.check()?;
    if let Some(host) = config.host {
        return Ok(vec![Candidate {
            id: "receiver".into(),
            name: moli_i18n::tr!("pilotes.cast.receiver"),
            host,
            hostname: host.to_string(),
            video: true,
        }]);
    }
    let interface = config.interface.expect("validated interface");
    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )?;
    socket.set_multicast_if_v4(&interface)?;
    socket.set_multicast_ttl_v4(1)?;
    socket.bind(&SocketAddrV4::new(interface, 0).into())?;
    socket.set_nonblocking(true)?;
    let socket = tokio::net::UdpSocket::from_std(socket.into())?;
    let mut query = vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    for part in SERVICE.split('.') {
        query.push(u8::try_from(part.len())?);
        query.extend_from_slice(part.as_bytes());
    }
    query.extend_from_slice(&[0, 0, 12, 0, 1]);
    socket.send_to(&query, "224.0.0.251:5353").await?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    let mut bytes = vec![0; 9001];
    let mut found = BTreeMap::new();
    for _ in 0..256 {
        let Ok(Ok((size, source))) =
            tokio::time::timeout_at(deadline, socket.recv_from(&mut bytes)).await
        else {
            break;
        };
        if source.port() != 5353 {
            continue;
        }
        let std::net::IpAddr::V4(ip) = source.ip() else {
            continue;
        };
        for candidate in parse_packet(&bytes[..size], ip, config) {
            if found.len() < 64 {
                found.insert(candidate.id.clone(), candidate);
            }
        }
    }
    Ok(found.into_values().collect())
}
#[cfg(test)]
#[path = "discovery_tests.rs"]
mod tests;
