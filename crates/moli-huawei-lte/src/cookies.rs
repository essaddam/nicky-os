//! An instance-local jar; attributes can never change the destination.
use crate::Failure;
use std::collections::{BTreeMap, BTreeSet};
#[derive(Default)]
pub(crate) struct Cookies {
    values: BTreeMap<String, String>,
}
impl Cookies {
    pub fn apply(&mut self, headers: &[String], host: &str, https: bool) -> Result<(), Failure> {
        if headers.len() > 8 || headers.iter().map(String::len).sum::<usize>() > 4096 {
            return Err(Failure::Invalid);
        }
        let mut next = self.values.clone();
        let mut seen = BTreeSet::new();
        for header in headers {
            if !header.is_ascii() || header.bytes().any(|b| b.is_ascii_control()) {
                return Err(Failure::Invalid);
            }
            let mut parts = header.split(';');
            let (name, value) = parts
                .next()
                .and_then(|v| v.trim().split_once('='))
                .ok_or(Failure::Invalid)?;
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
                || !value.bytes().all(
                    |b| matches!(b,0x21 | 0x23..=0x2b | 0x2d..=0x3a | 0x3c..=0x5b | 0x5d..=0x7e),
                )
                || !seen.insert(name.to_owned())
            {
                return Err(Failure::Invalid);
            }
            let mut remove = value.is_empty();
            for attribute in parts {
                let (key, val) = attribute
                    .trim()
                    .split_once('=')
                    .unwrap_or((attribute.trim(), ""));
                if key.eq_ignore_ascii_case("domain") && !val.eq_ignore_ascii_case(host)
                    || key.eq_ignore_ascii_case("path") && !matches!(val, "/" | "/api/")
                    || key.eq_ignore_ascii_case("secure") && !https
                {
                    return Err(Failure::Invalid);
                }
                if key.eq_ignore_ascii_case("max-age") {
                    remove |= val.parse::<i64>().map_err(|_| Failure::Invalid)? <= 0;
                }
            }
            if remove {
                next.remove(name);
            } else {
                next.insert(name.into(), value.into());
            }
        }
        let size = next
            .iter()
            .map(|(key, value)| key.len() + value.len() + 3)
            .sum::<usize>();
        if next.len() > 8 || size > 4096 {
            return Err(Failure::Invalid);
        }
        self.values = next;
        Ok(())
    }
    pub fn header(&self) -> Option<String> {
        (!self.values.is_empty()).then(|| {
            self.values
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; ")
        })
    }
    pub fn clear(&mut self) {
        self.values.clear();
    }
}
