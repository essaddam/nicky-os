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
    /// The browser's origin spelling: lowercase DNS, normalized IP and no
    /// default port. The callback and API must use this same authority.
    ///
    /// # Errors
    /// Returns `Invalid` for origins outside the installation policy.
    pub fn origin(&self) -> Result<String, Error> {
        canonical_origin(&self.public_origin)
    }

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
        let origin = self.origin()?;
        Ok(format!("{origin}/api/integrations/netatmo/{id}/callback"))
    }
}

fn canonical_origin(origin: &str) -> Result<String, Error> {
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
    // WHATWG URLs interpret numeric final labels as IPv4. Refuse alternate
    // numeric spellings rather than accepting a different browser authority.
    if ip.is_none()
        && host.rsplit('.').next().is_some_and(|last| {
            last.bytes().all(|byte| byte.is_ascii_digit())
                || last
                    .strip_prefix("0x")
                    .or_else(|| last.strip_prefix("0X"))
                    .is_some_and(|hex| hex.bytes().all(|b| b.is_ascii_hexdigit()))
        })
    {
        return Err(Error::Invalid);
    }
    let host = match ip {
        Some(IpAddr::V6(ip)) => match ip.segments() {
            // Keep the IPv6 origin, with WHATWG hexadecimal pieces. Display
            // uses dotted decimal for mapped addresses, unlike browsers.
            [0, 0, 0, 0, 0, 0xffff, high, low] => format!("[::ffff:{high:x}:{low:x}]"),
            _ => format!("[{ip}]"),
        },
        Some(IpAddr::V4(ip)) => ip.to_string(),
        None => host.to_ascii_lowercase(),
    };
    let default = if scheme == "https" { 443 } else { 80 };
    let port = authority.port_u16().filter(|port| *port != default);
    Ok(match port {
        Some(port) => format!("{scheme}://{host}:{port}"),
        None => format!("{scheme}://{host}"),
    })
}
