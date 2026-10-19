//! Human authorization attempts and credential bundles. No network or logging.

use std::fmt::{self, Write as _};
use std::time::Duration;

use ring::digest::{SHA256, digest};
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::Value as Json;
use tokio::time::Instant;

use crate::Error;

pub(crate) struct Pending {
    state_hash: Option<[u8; 32]>,
    binding: [u8; 32],
    deadline: Instant,
}

impl fmt::Debug for Pending {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pending")
            .field("consumed", &self.state_hash.is_none())
            .finish_non_exhaustive()
    }
}

fn fingerprint(text: &str) -> [u8; 32] {
    let hash = digest(&SHA256, text.as_bytes());
    let mut bytes = [0; 32];
    bytes.copy_from_slice(hash.as_ref());
    bytes
}

/// Fixed-size comparison without an early mismatch return.
fn same(left: &[u8; 32], right: &[u8; 32]) -> bool {
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}

impl Pending {
    pub fn issue(binding: [u8; 32], now: Instant) -> Result<(Self, String), Error> {
        let mut bytes = [0; 32];
        SystemRandom::new()
            .fill(&mut bytes)
            .map_err(|_| Error::Unavailable)?;
        let mut state = String::with_capacity(64);
        for byte in bytes {
            write!(&mut state, "{byte:02x}").map_err(|_| Error::Unavailable)?;
        }
        let pending = Self {
            state_hash: Some(fingerprint(&state)),
            binding,
            deadline: now + Duration::from_secs(600),
        };
        Ok((pending, state))
    }

    pub fn consume(&mut self, state: &str, binding: [u8; 32], now: Instant) -> Result<(), Error> {
        let expected = self.state_hash.ok_or(Error::Unauthorized)?;
        if now >= self.deadline {
            self.state_hash = None;
            return Err(Error::Expired);
        }
        if state.len() != 64 || !state.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(Error::Unauthorized);
        }
        let bound = same(&self.binding, &binding);
        let matched = same(&expected, &fingerprint(state));
        if !(bound & matched) {
            return Err(Error::Unauthorized);
        }
        self.state_hash = None;
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct TokenSet {
    pub access: String,
    pub refresh: String,
    pub expires_at: u64,
}

impl fmt::Debug for TokenSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenSet")
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

impl TokenSet {
    /// One versioned value in the hub's encrypted store, never a point.
    pub fn persisted(&self) -> Result<String, Error> {
        let text = serde_json::json!({
            "version": 1,
            "access_token": self.access,
            "refresh_token": self.refresh,
            "expires_at": self.expires_at,
        })
        .to_string();
        if text.len() > 16 * 1024 {
            return Err(Error::Storage);
        }
        Ok(text)
    }

    pub fn restore(bytes: &[u8], now: u64) -> Result<Self, Error> {
        if bytes.len() > 16 * 1024 {
            return Err(Error::Invalid);
        }
        let json: Json = serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?;
        if json.get("version").and_then(Json::as_u64) != Some(1) {
            return Err(Error::Invalid);
        }
        let expires_at = json
            .get("expires_at")
            .and_then(Json::as_u64)
            .ok_or(Error::Invalid)?;
        if expires_at == 0 || expires_at > now.saturating_add(86400) {
            return Err(Error::Invalid);
        }
        Ok(Self {
            access: token(&json, "access_token")?,
            refresh: token(&json, "refresh_token")?,
            expires_at,
        })
    }

    pub fn parse(bytes: &[u8], now: u64) -> Result<Self, Error> {
        if bytes.len() > 16 * 1024 {
            return Err(Error::Invalid);
        }
        let json: Json = serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?;
        let seconds = json
            .get("expires_in")
            .and_then(Json::as_u64)
            .ok_or(Error::Invalid)?;
        if !(60..=86400).contains(&seconds) {
            return Err(Error::Invalid);
        }
        if let Some(scope) = json.get("scope") {
            let scopes = scope.as_array().ok_or(Error::Invalid)?;
            if scopes.len() != 1 || scopes[0].as_str() != Some("read_station") {
                return Err(Error::Invalid);
            }
        }
        Ok(Self {
            access: token(&json, "access_token")?,
            refresh: token(&json, "refresh_token")?,
            expires_at: now.checked_add(seconds).ok_or(Error::Invalid)?,
        })
    }
}

fn token(json: &Json, field: &str) -> Result<String, Error> {
    let value = json
        .get(field)
        .and_then(Json::as_str)
        .ok_or(Error::Invalid)?;
    if value.is_empty() || value.len() > 4096 || !value.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err(Error::Invalid);
    }
    Ok(value.to_owned())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use ring::rand::{SecureRandom, SystemRandom};
    use serde_json::json;

    use super::*;

    fn generated() -> String {
        let mut bytes = [0_u8; 32];
        SystemRandom::new().fill(&mut bytes).unwrap();
        let mut encoded = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            write!(encoded, "{byte:02x}").unwrap();
        }
        encoded
    }

    #[test]
    fn state_is_single_use_and_expires() {
        let now = Instant::now();
        let (mut pending, state) = Pending::issue([1; 32], now).unwrap();
        assert_eq!(state.len(), 64);
        assert_eq!(
            pending.consume(&state, [2; 32], now),
            Err(Error::Unauthorized)
        );
        assert_eq!(
            pending.consume(&generated(), [1; 32], now),
            Err(Error::Unauthorized)
        );
        assert_eq!(pending.consume(&state, [1; 32], now), Ok(()));
        assert_eq!(
            pending.consume(&state, [1; 32], now),
            Err(Error::Unauthorized)
        );
        let (mut pending, state) = Pending::issue([1; 32], now).unwrap();
        assert_eq!(
            pending.consume(&state, [1; 32], now + Duration::from_secs(600)),
            Err(Error::Expired)
        );
    }

    #[test]
    fn token_bundle_uses_provider_duration_and_rejects_hostile_values() {
        let access = generated();
        let refresh = generated();
        let mut value = json!({"access_token":access,"refresh_token":refresh,"expires_in":3600,"scope":["read_station"]});
        let tokens = TokenSet::parse(&serde_json::to_vec(&value).unwrap(), 1000).unwrap();
        // Equality failures must not print the generated credentials.
        assert!(tokens.access.eq(&access));
        assert!(tokens.refresh.eq(&refresh));
        assert_eq!(tokens.expires_at, 4600);
        for bad in [json!(0), json!(-1), json!(86401), json!("3600")] {
            value["expires_in"] = bad;
            assert!(TokenSet::parse(&serde_json::to_vec(&value).unwrap(), 1000).is_err());
        }
        value["expires_in"] = json!(3600);
        value["access_token"] = json!(format!("{}\r\n", generated()));
        assert!(TokenSet::parse(&serde_json::to_vec(&value).unwrap(), 1000).is_err());
        assert!(TokenSet::parse(&vec![b' '; 16 * 1024 + 1], 1000).is_err());
    }
}
