//! Exact integer counters, independent of transport and persistence.

use serde::{Deserialize, Serialize};

pub const MAX_COUNTER_VALUE: i64 = (1_i64 << 53) - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CounterSpec {
    pub initial: i64,
    pub min: i64,
    pub max: i64,
    pub step: i64,
}

impl CounterSpec {
    #[must_use]
    pub fn is_valid(self) -> bool {
        let safe = -MAX_COUNTER_VALUE..=MAX_COUNTER_VALUE;
        [self.initial, self.min, self.max, self.step]
            .iter()
            .all(|value| safe.contains(value))
            && self.step != 0
            && self.min <= self.max
            && self.contains(self.initial)
    }

    #[must_use]
    pub fn contains(self, value: i64) -> bool {
        (self.min..=self.max).contains(&value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CounterOperation {
    Increment,
    Reset,
}
