use crate::{ProfileDraft, lockouts::Attempts};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt};

pub(crate) const MAX_PROFILES: usize = 128;
pub(crate) const MAX_SOURCES: usize = 1024;
pub(crate) const VERSION: u8 = 1;
pub(crate) const PROFILES: &str = "alarm-profiles.json";
pub(crate) const ALARM: &str = "alarm.json";
pub(crate) const CODES: &str = "profile_codes";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Record {
    pub profile: ProfileDraft,
    pub revoked: bool,
    pub slot: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProfileFile {
    pub version: u8,
    pub profiles: BTreeMap<String, Record>,
}
impl Default for ProfileFile {
    fn default() -> Self {
        Self {
            version: VERSION,
            profiles: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AlarmFile {
    pub version: u8,
    pub code_locks: BTreeMap<String, Attempts>,
    #[serde(default)]
    pub runtime: Option<serde_json::Value>,
}
impl Default for AlarmFile {
    fn default() -> Self {
        Self {
            version: VERSION,
            code_locks: BTreeMap::new(),
            runtime: None,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Fingerprint {
    pub profile: String,
    pub mac: String,
}
impl fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Fingerprint([redacted])")
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Credentials {
    #[serde(default)]
    pub entries: BTreeMap<String, Fingerprint>,
}
impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("count", &self.entries.len())
            .finish()
    }
}
