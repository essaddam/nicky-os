use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

/// Borrowed only for one request. Never serialized, cloned or printed.
pub struct Code<'a>(pub(crate) Option<&'a str>);

impl<'a> Code<'a> {
    pub fn provided(value: &'a str) -> Self {
        Self(Some(value))
    }
    pub fn missing() -> Self {
        Self(None)
    }
    pub(crate) fn valid(&self) -> bool {
        self.0.is_some_and(|value| {
            (4..=8).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_digit())
        })
    }
}

impl Drop for Code<'_> {
    fn drop(&mut self) {
        self.0 = None;
    }
}

impl fmt::Debug for Code<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Code([redacted])")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Away,
    Home,
    Night,
    Vacation,
}

pub(crate) fn all_modes() -> BTreeSet<Mode> {
    [Mode::Away, Mode::Home, Mode::Night, Mode::Vacation]
        .into_iter()
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileDraft {
    pub name: String,
    #[serde(default)]
    pub person: Option<String>,
    pub can_arm: bool,
    pub can_disarm: bool,
    #[serde(default = "all_modes")]
    pub modes: BTreeSet<Mode>,
    #[serde(default)]
    pub from: Option<u64>,
    #[serde(default)]
    pub until: Option<u64>,
}

/// Output metadata: an empty live window is represented by `from == until`.
/// It is not a valid new draft and never changes the stored authorization window.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProfileView {
    pub id: String,
    #[serde(flatten)]
    pub profile: ProfileDraft,
    pub revoked: bool,
}

/// A live directory result. Missing people never authorize a linked profile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Person {
    pub guest: bool,
    pub from: Option<u64>,
    pub until: Option<u64>,
    pub reservation_active: bool,
}

/// Trusted household adapter, never populated from a request body.
pub trait PersonDirectory: Send + Sync + fmt::Debug {
    fn person(&self, id: &str, now: u64) -> Option<Person>;
}

// Only the private transaction task owns this short-lived copy. It has no
// serialization, Clone, Display or content-bearing Debug implementation.
pub(crate) enum Input {
    Missing,
    Invalid,
    Digits(Digits),
}
pub(crate) struct Digits {
    bytes: [u8; 8],
    len: usize,
}
impl Drop for Digits {
    fn drop(&mut self) {
        self.bytes.fill(0);
    }
}
impl Input {
    pub fn capture(mut code: Code<'_>) -> Self {
        if code.0.is_none() {
            return Self::Missing;
        }
        if !code.valid() {
            return Self::Invalid;
        }
        let value = code.0.take().unwrap_or_default().as_bytes();
        let mut bytes = [0; 8];
        bytes[..value.len()].copy_from_slice(value);
        Self::Digits(Digits {
            bytes,
            len: value.len(),
        })
    }
    pub fn borrow(&self) -> Code<'_> {
        match self {
            Self::Missing => Code::missing(),
            Self::Invalid => Code::provided(""),
            Self::Digits(v) => {
                Code::provided(std::str::from_utf8(&v.bytes[..v.len]).unwrap_or_default())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_four_to_eight_ascii_digits_are_codes_and_debug_is_redacted() {
        for valid in ["0042", "70710937", "12345"] {
            let code = Code::provided(valid);
            assert!(code.valid());
            assert!(!format!("{code:?}").contains(valid));
        }
        for invalid in ["", "123", "123456789", "１２３４", "12 34", "+1234", "12a4"] {
            assert!(!Code::provided(invalid).valid());
        }
        assert!(!Code::missing().valid());
    }

    #[test]
    fn default_modes_are_all_four_and_profile_output_has_no_credential() {
        let draft: ProfileDraft = serde_json::from_str(
            r#"{"name":"Synthetic profile","can_arm":true,"can_disarm":true}"#,
        )
        .unwrap();
        assert_eq!(draft.modes, all_modes());
        let view = ProfileView {
            id: "synthetic".into(),
            profile: draft,
            revoked: false,
        };
        let json = serde_json::to_value(view).unwrap();
        assert!(json.get("code").is_none());
        assert!(json.get("code_hmac").is_none());
    }
}
