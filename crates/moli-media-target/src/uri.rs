//! A deliberately restricted grammar for provider links, not a general URL
//! parser: exact authorities, bounded paths and explicit query parameters.

use crate::ContentError;

#[derive(Debug)]
pub(crate) struct Link<'a> {
    pub parts: Vec<&'a str>,
    query: Option<&'a str>,
}

impl<'a> Link<'a> {
    pub fn parse(input: &'a str, hosts: &[&str]) -> Result<Self, ContentError> {
        if input.contains('\\') || input.contains('#') || input.chars().any(char::is_whitespace) {
            return Err(ContentError::InvalidTarget);
        }
        let (host, rest) = input
            .strip_prefix("https://")
            .and_then(|s| s.split_once('/'))
            .ok_or(ContentError::InvalidTarget)?;
        // This also rejects userinfo, explicit ports and encoded authorities.
        if !hosts.iter().any(|h| host.eq_ignore_ascii_case(h)) {
            return Err(ContentError::InvalidTarget);
        }
        let (path, query) = rest
            .split_once('?')
            .map_or((rest, None), |(p, q)| (p, Some(q)));
        let path = path.strip_suffix('/').unwrap_or(path);
        if path.split('/').count() > 6 {
            return Err(ContentError::InvalidTarget);
        }
        let parts: Vec<_> = path.split('/').collect();
        if parts
            .iter()
            .any(|p| p.is_empty() || matches!(*p, "." | ".."))
        {
            return Err(ContentError::InvalidTarget);
        }
        Ok(Self { parts, query })
    }

    /// Tracking is disposable; unrecognized semantic parameters are refused
    /// instead of silently changing which content the person selected.
    pub fn check_query(&self, meaningful: &[&str]) -> Result<(), ContentError> {
        let Some(query) = self.query.filter(|q| !q.is_empty()) else {
            return Ok(());
        };
        for item in query.split('&') {
            let key = item.split_once('=').map_or(item, |(k, _)| k);
            if !meaningful.contains(&key) && !tracking(key) {
                return Err(ContentError::InvalidTarget);
            }
        }
        Ok(())
    }

    pub fn query_value(&self, key: &str) -> Result<&'a str, ContentError> {
        let mut values = self
            .query
            .into_iter()
            .flat_map(|q| q.split('&'))
            .filter_map(|item| item.split_once('='))
            .filter_map(|(k, v)| (k == key).then_some(v));
        let value = values.next().ok_or(ContentError::InvalidTarget)?;
        if values.next().is_some() {
            return Err(ContentError::InvalidTarget);
        }
        Ok(value)
    }
}

fn tracking(key: &str) -> bool {
    key.starts_with("utm_")
        || matches!(
            key,
            "trackingId"
                | "trackId"
                | "ref"
                | "ref_"
                | "refRID"
                | "itsct"
                | "itscg"
                | "at"
                | "ct"
                | "cid"
                | "campaignId"
                | "distributionPartner"
                | "irclickid"
                | "irgwc"
                | "afsrc"
                | "sharedId"
                | "fbclid"
                | "gclid"
                | "msockid"
        )
}

pub(crate) fn without_locale<'parts, 'text>(parts: &'parts [&'text str]) -> &'parts [&'text str] {
    let Some(first) = parts.first() else {
        return parts;
    };
    let locale = first.len() == 2 && first.bytes().all(|b| b.is_ascii_alphabetic())
        || first.len() == 5
            && first.as_bytes()[2] == b'-'
            && first
                .bytes()
                .enumerate()
                .all(|(i, b)| i == 2 || b.is_ascii_alphabetic());
    if locale { &parts[1..] } else { parts }
}

pub(crate) fn digits(value: &str) -> bool {
    (1..=20).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_digit())
}

pub(crate) fn identifier(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        && value != "."
        && value != ".."
}

/// A cosmetic slug can contain encoded Unicode letters, but never encoded
/// separators, whitespace, control characters or dot path components.
pub(crate) fn slug(value: &str) -> bool {
    let mut decoded = Vec::with_capacity(value.len());
    let mut bytes = value.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let Some(high) = bytes.next().and_then(|b| char::from(b).to_digit(16)) else {
                return false;
            };
            let Some(low) = bytes.next().and_then(|b| char::from(b).to_digit(16)) else {
                return false;
            };
            let Ok(byte) = u8::try_from(high * 16 + low) else {
                return false;
            };
            decoded.push(byte);
        } else {
            decoded.push(byte);
        }
    }
    std::str::from_utf8(&decoded).is_ok_and(|s| {
        !s.is_empty()
            && s != "."
            && s != ".."
            && s.chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '~'))
    })
}
