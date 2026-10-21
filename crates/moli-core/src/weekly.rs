//! Civil weekly intervals. Time-zone resolution belongs to the runtime.

use serde::{Deserialize, Serialize};

pub const MAX_WEEKLY_SLOTS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeeklySlot {
    /// Monday = 1, Sunday = 7.
    pub day: u8,
    /// Minutes since midnight, inclusive (0..1440).
    pub start: u16,
    /// Exclusive end, up to 1440. An end before start crosses midnight.
    pub end: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeeklySchedule {
    pub timezone: String,
    pub slots: Vec<WeeklySlot>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidSchedule;

impl WeeklySchedule {
    /// Split overnight slots, sort, and reject overlaps. Adjacency is allowed.
    pub fn normalize(mut self) -> Result<Self, InvalidSchedule> {
        if self.timezone.is_empty()
            || self.timezone.len() > 128
            || self.slots.len() > MAX_WEEKLY_SLOTS
        {
            return Err(InvalidSchedule);
        }
        let mut slots = Vec::with_capacity(self.slots.len().saturating_mul(2));
        for slot in self.slots {
            if !(1..=7).contains(&slot.day)
                || slot.start >= 1440
                || slot.end > 1440
                || slot.start == slot.end
            {
                return Err(InvalidSchedule);
            }
            if slot.end < slot.start {
                slots.push(WeeklySlot { end: 1440, ..slot });
                if slot.end != 0 {
                    slots.push(WeeklySlot {
                        day: slot.day % 7 + 1,
                        start: 0,
                        end: slot.end,
                    });
                }
            } else {
                slots.push(slot);
            }
        }
        slots.sort_unstable();
        if slots.len() > MAX_WEEKLY_SLOTS
            || slots
                .windows(2)
                .any(|s| s[0].day == s[1].day && s[0].end > s[1].start)
        {
            return Err(InvalidSchedule);
        }
        self.slots = slots;
        Ok(self)
    }

    /// Use only after normalization. Endpoints are half-open.
    pub fn contains(&self, day: u8, minute: u16) -> bool {
        (1..=7).contains(&day)
            && minute < 1440
            && self
                .slots
                .iter()
                .any(|s| s.day == day && s.start <= minute && minute < s.end)
    }
}
