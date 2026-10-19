//! Human installation options; no provider endpoint or credentials.

use moli_core::InstanceId;
use serde::Deserialize;
use std::net::{IpAddr, Ipv6Addr};

use crate::Error;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub public_origin: String,
    #[serde(default = "poll_default")]
    pub poll_seconds: u64,
    #[serde(default = "stale_default")]
    pub stale_seconds: u64,
}

const fn poll_default() -> u64 {
    300
}

const fn stale_default() -> u64 {
    1800
}

impl Config {
    pub fn callback(&self, instance: &InstanceId) -> Result<String, Error> {
        if !(60..=3600).contains(&self.poll_seconds)
            || !(300..=86400).contains(&self.stale_seconds)
            || self.stale_seconds < self.poll_seconds.saturating_mul(2)
        {
            return Err(Error::Invalid);
        }
        let id = instance.as_str();
        if id.is_empty()
            || id.len() > 64
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        {
            return Err(Error::Invalid);
        }
        validate_origin(&self.public_origin)?;
        Ok(format!(
            "{}/api/integrations/netatmo/{id}/callback",
            self.public_origin
        ))
    }
}

fn validate_origin(origin: &str) -> Result<(), Error> {
    if origin.len() > 2048 || !origin.is_ascii() {
        return Err(Error::Invalid);
    }
    let (scheme, raw) = origin.split_once("://").ok_or(Error::Invalid)?;
    if !matches!(scheme, "http" | "https") || raw.contains(['/', '@', '?', '#', '\\']) {
        return Err(Error::Invalid);
    }
    let authority: http::uri::Authority = raw.parse().map_err(|_| Error::Invalid)?;
    let host = authority.host();
    let ip = if let Some(inner) = host.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        Some(IpAddr::V6(
            inner.parse::<Ipv6Addr>().map_err(|_| Error::Invalid)?,
        ))
    } else {
        if host.len() > 253
            || host.split('.').any(|label| {
                label.is_empty()
                    || label.len() > 63
                    || label.starts_with('-')
                    || label.ends_with('-')
                    || !label
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            })
        {
            return Err(Error::Invalid);
        }
        host.parse::<IpAddr>().ok()
    };
    let suffix = raw.strip_prefix(host).ok_or(Error::Invalid)?;
    if !suffix.is_empty() {
        let digits = suffix.strip_prefix(':').ok_or(Error::Invalid)?;
        if digits.is_empty()
            || !digits.bytes().all(|b| b.is_ascii_digit())
            || !digits.parse::<u16>().is_ok_and(|port| port != 0)
        {
            return Err(Error::Invalid);
        }
    }
    let loopback = host.eq_ignore_ascii_case("localhost") || ip.is_some_and(|ip| ip.is_loopback());
    if scheme == "http" && !loopback {
        return Err(Error::Invalid);
    }
    Ok(())
}
