//! Live boundaries only. Restoring a calendar is not an arrival.

use jiff::Timestamp;
use std::collections::BTreeMap;

use crate::{CalendarError, Reservation};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Boundary {
    Departure,
    Arrival,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transition {
    pub reservation: String,
    pub at: Timestamp,
    pub boundary: Boundary,
}

#[derive(Debug, Default)]
pub struct Timeline {
    previous: BTreeMap<String, Reservation>,
    last: Option<Timestamp>,
    high_water: Option<Timestamp>,
}

impl Timeline {
    pub fn update(
        &mut self,
        snapshot: Option<&[Reservation]>,
        now: Timestamp,
    ) -> Result<Vec<Transition>, CalendarError> {
        let old_high_water = self.high_water;
        self.high_water = Some(self.high_water.map_or(now, |high| high.max(now)));
        let Some(snapshot) = snapshot else {
            self.previous.clear();
            self.last = None;
            return Ok(Vec::new());
        };
        let next = match indexed(snapshot) {
            Ok(next) => next,
            Err(error) => {
                self.previous.clear();
                self.last = None;
                return Err(error);
            }
        };
        let previous = std::mem::replace(&mut self.previous, next);
        let Some(last) = self.last.replace(now) else {
            return Ok(Vec::new());
        };
        // A restored snapshot, a backwards clock or a scheduling outage is
        // state reconciliation, never a queue of historical physical actions.
        if now <= last
            || old_high_water.is_some_and(|high| now <= high)
            || now.as_millisecond() - last.as_millisecond() > 2_000
        {
            return Ok(Vec::new());
        }
        let mut events = Vec::new();
        for (id, current) in &self.previous {
            if previous.get(id) != Some(current) {
                continue;
            }
            if last < current.start && current.start <= now && now < current.end {
                events.push(Transition {
                    reservation: id.clone(),
                    at: current.start,
                    boundary: Boundary::Arrival,
                });
            }
            if last < current.end && current.end <= now {
                events.push(Transition {
                    reservation: id.clone(),
                    at: current.end,
                    boundary: Boundary::Departure,
                });
            }
        }
        events.sort_by(|left, right| {
            (left.at, left.boundary, &left.reservation).cmp(&(
                right.at,
                right.boundary,
                &right.reservation,
            ))
        });
        Ok(events)
    }
}

fn indexed(snapshot: &[Reservation]) -> Result<BTreeMap<String, Reservation>, CalendarError> {
    if snapshot.len() > 4096 {
        return Err(CalendarError);
    }
    let mut indexed = BTreeMap::new();
    for reservation in snapshot {
        if reservation.start >= reservation.end
            || reservation.id.is_empty()
            || reservation.id.len() > 64
            || reservation.id.chars().any(char::is_control)
        {
            return Err(CalendarError);
        }
        if let Some(previous) = indexed.insert(reservation.id.clone(), reservation.clone())
            && previous != *reservation
        {
            return Err(CalendarError);
        }
    }
    Ok(indexed)
}
