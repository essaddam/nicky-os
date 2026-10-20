//! Occurrences have identities, not synthetic changing state values.

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::{PointId, Value};

/// Exactly representable by JSON consumers, including the dashboard.
pub const MAX_OCCURRENCE_SEQUENCE: u64 = (1_u64 << 53) - 1;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OccurrenceId {
    pub source: Arc<str>,
    pub sequence: u64,
}

impl OccurrenceId {
    #[must_use]
    pub fn is_valid(&self) -> bool {
        valid_client(&self.source) && (1..=MAX_OCCURRENCE_SEQUENCE).contains(&self.sequence)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Occurrence {
    pub id: OccurrenceId,
    /// Hub lifetime and publication position: never restored from a snapshot.
    pub stream: Arc<str>,
    pub position: u64,
    pub point: PointId,
    pub value: Value,
    pub ts: u64,
    pub durable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OccurrenceStatus {
    Published,
    Replayed,
    Ambiguous,
}

/// Optional caller identity: a random client namespace and a monotone sequence.
/// The string form avoids rounding a caller's u64 sequence in JSON clients.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RequestId {
    client: String,
    sequence: u64,
}

impl RequestId {
    #[must_use]
    pub fn client(&self) -> &str {
        &self.client
    }

    #[must_use]
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("expected a 32-hex client namespace and a positive sequence")]
pub struct InvalidRequestId;

fn valid_client(client: &str) -> bool {
    client.len() == 32 && client.bytes().all(|byte| byte.is_ascii_hexdigit())
}

impl FromStr for RequestId {
    type Err = InvalidRequestId;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() > 53 {
            return Err(InvalidRequestId);
        }
        let (client, sequence) = value.split_once(':').ok_or(InvalidRequestId)?;
        if !valid_client(client)
            || sequence.is_empty()
            || sequence.starts_with('0')
            || !sequence.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(InvalidRequestId);
        }
        let sequence = sequence.parse().map_err(|_| InvalidRequestId)?;
        Ok(Self {
            client: client.to_ascii_lowercase(),
            sequence,
        })
    }
}

impl TryFrom<String> for RequestId {
    type Error = InvalidRequestId;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(output, "{}:{}", self.client, self.sequence)
    }
}

impl From<RequestId> for String {
    fn from(value: RequestId) -> Self {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_h4_request_ids_are_bounded_and_do_not_round_sequences() {
        let id: RequestId = "ABCDEF0123456789ABCDEF0123456789:18446744073709551615"
            .parse()
            .unwrap();
        assert_eq!(id.sequence(), u64::MAX);
        assert_eq!(
            id.to_string(),
            "abcdef0123456789abcdef0123456789:18446744073709551615"
        );
        assert_eq!(
            serde_json::from_str::<RequestId>(&serde_json::to_string(&id).unwrap()).unwrap(),
            id
        );
        for bad in [
            "",
            "client:1",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:0",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:01",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:+1",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:18446744073709551616",
        ] {
            assert!(bad.parse::<RequestId>().is_err(), "{bad}");
        }
    }
}
