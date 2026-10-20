//! Bounded reservation calendars. A reservation is not a person's presence.

use jiff::Timestamp;
use std::fmt::Write as _;

mod calendar;
pub mod client;
mod recurrence;
mod revision;
mod series;
mod timeline;

pub use timeline::{Boundary, Timeline, Transition};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reservation {
    pub id: String,
    pub start: Timestamp,
    pub end: Timestamp,
}

impl Reservation {
    #[must_use]
    pub fn allows(&self, now: Timestamp) -> bool {
        self.start <= now && now < self.end
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{}", moli_i18n::tr!("pilotes.reservations.calendrier_invalide"))]
pub struct CalendarError;

fn public_id(uid: &str) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, uid.as_bytes());
    let mut id = String::with_capacity(32);
    for byte in &digest.as_ref()[..16] {
        write!(id, "{byte:02x}").expect("writing to a string");
    }
    id
}

pub fn parse(
    calendar: &str,
    timezone: &str,
    now: Timestamp,
) -> Result<Vec<Reservation>, CalendarError> {
    calendar::parse(calendar, timezone, now)
}
