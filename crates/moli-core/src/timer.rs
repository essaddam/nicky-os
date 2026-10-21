//! Timer vocabulary. Clocks, persistence and dispatch belong to the runtime.

use serde::{Deserialize, Serialize};

pub const MAX_TIMER_SECONDS: u32 = 604_800;
pub const DEFAULT_TIMER_SECONDS: u32 = 300;

/// Eligible at timer start, never inferred from an activation created later.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimerRecipient {
    pub automation: String,
    pub trigger: String,
    pub activation: String,
}

impl TimerRecipient {
    #[must_use]
    pub fn is_valid(&self) -> bool {
        [&self.automation, &self.trigger].iter().all(|value| {
            !value.is_empty() && value.len() <= 160 && !value.chars().any(char::is_control)
        }) && self.activation.len() == 32
            && self.activation.bytes().all(|byte| byte.is_ascii_hexdigit())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimerOperation {
    Start,
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimerField {
    State,
    Remaining,
    Generation,
    Deadline,
    Expired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimerStatus {
    Idle,
    Running,
    Cancelled,
    Finished,
}

impl TimerStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "running",
            Self::Cancelled => "cancelled",
            Self::Finished => "finished",
        }
    }
}
