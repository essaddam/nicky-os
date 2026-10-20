//! Calendar state only: never evidence that a person is physically present.

use crate::Reservation;
use jiff::Timestamp;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cycle {
    NoReservation,
    AwaitingCheckin,
    CheckedIn,
    CheckedOut,
}

impl Cycle {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoReservation => "no_reservation",
            Self::AwaitingCheckin => "awaiting_checkin",
            Self::CheckedIn => "checked_in",
            Self::CheckedOut => "checked_out",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Overview {
    pub cycle: Cycle,
    pub next: Vec<Reservation>,
}

#[must_use]
pub fn overview(snapshot: &[Reservation], now: Timestamp) -> Overview {
    let cycle = if snapshot.iter().any(|stay| stay.allows(now)) {
        Cycle::CheckedIn
    } else if snapshot.iter().any(|stay| stay.start > now) {
        Cycle::AwaitingCheckin
    } else if snapshot.is_empty() {
        Cycle::NoReservation
    } else {
        Cycle::CheckedOut
    };
    let mut next: Vec<_> = snapshot.iter().filter(|stay| stay.end > now).collect();
    next.sort_by(|left, right| (left.start, &left.id).cmp(&(right.start, &right.id)));
    let next = next.into_iter().take(5).cloned().collect();
    Overview { cycle, next }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ts(value: &str) -> Timestamp {
        value.parse().unwrap()
    }
    fn stay(id: &str, start: &str, end: &str) -> Reservation {
        Reservation {
            id: id.into(),
            start: ts(start),
            end: ts(end),
        }
    }

    #[test]
    fn four_calendar_states_follow_the_exact_window_without_claiming_presence() {
        let reservation = stay("fixture", "2026-10-20T12:00:00Z", "2026-10-20T13:00:00Z");
        let reservations = [reservation.clone()];
        assert_eq!(overview(&[], reservation.start).cycle, Cycle::NoReservation);
        assert_eq!(
            overview(&reservations, ts("2026-10-20T11:59:59.999Z")).cycle,
            Cycle::AwaitingCheckin
        );
        assert_eq!(
            overview(&reservations, reservation.start).cycle,
            Cycle::CheckedIn
        );
        assert_eq!(
            overview(&reservations, ts("2026-10-20T12:59:59.999Z")).cycle,
            Cycle::CheckedIn
        );
        assert_eq!(
            overview(&reservations, reservation.end).cycle,
            Cycle::CheckedOut
        );
        assert!(overview(&reservations, reservation.end).next.is_empty());
    }

    #[test]
    fn the_five_next_windows_are_ordered_and_include_a_current_stay() {
        let now = ts("2026-10-20T12:00:00Z");
        let mut snapshot = Vec::new();
        for day in (18..=28).rev() {
            snapshot.push(stay(
                &format!("fixture-{day}"),
                &format!("2026-10-{day}T11:00:00Z"),
                &format!("2026-10-{day}T13:00:00Z"),
            ));
        }
        let view = overview(&snapshot, now);
        assert_eq!(view.cycle, Cycle::CheckedIn);
        assert_eq!(
            view.next.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            [
                "fixture-20",
                "fixture-21",
                "fixture-22",
                "fixture-23",
                "fixture-24"
            ]
        );
    }
}
