//! A civil predicate, including both copies of a repeated hour. No occurrence replay.

use jiff::{
    Timestamp, ToSpan,
    tz::{AmbiguousOffset, TimeZone},
};
use moli_core::WeeklySchedule;

use super::DependencyError;

#[derive(Clone, Debug)]
pub(crate) struct Calendar {
    plan: WeeklySchedule,
    zone: TimeZone,
}

impl Calendar {
    pub(crate) fn new(plan: WeeklySchedule) -> Result<Self, DependencyError> {
        let plan = plan.normalize().map_err(|_| DependencyError::Definition)?;
        let zone = TimeZone::get(&plan.timezone).map_err(|_| DependencyError::Definition)?;
        Ok(Self { plan, zone })
    }

    pub(crate) fn definition(&self) -> &WeeklySchedule {
        &self.plan
    }

    pub(crate) fn active(&self, at: Timestamp) -> bool {
        let civil = at.to_zoned(self.zone.clone());
        self.plan.contains(
            u8::try_from(civil.weekday().to_monday_one_offset()).unwrap_or(0),
            u16::try_from(i32::from(civil.hour()) * 60 + i32::from(civil.minute())).unwrap_or(1440),
        )
    }

    /// Bounded endpoint search over this and next week. Offset jumps are wakeups
    /// too: a skipped interval may start/end at the jump itself.
    pub(crate) fn next(&self, at: Timestamp) -> Option<Timestamp> {
        if self.plan.slots.is_empty() {
            return None;
        }
        let mut next = self.zone.following(at).next().map(|t| t.timestamp());
        let date = at.to_zoned(self.zone.clone()).date();
        for offset in 0_i32..=8 {
            let Ok(date) = date.checked_add(offset.days()) else {
                break;
            };
            let day = u8::try_from(date.weekday().to_monday_one_offset()).unwrap_or(0);
            for slot in self.plan.slots.iter().filter(|s| s.day == day) {
                for minute in [slot.start, slot.end] {
                    let Ok(civil) = date.at(0, 0, 0, 0).checked_add(i32::from(minute).minutes())
                    else {
                        continue;
                    };
                    let ambiguous = self.zone.to_ambiguous_timestamp(civil);
                    let choices = match ambiguous.offset() {
                        AmbiguousOffset::Gap { .. } => [None, None],
                        AmbiguousOffset::Fold { .. } => {
                            [ambiguous.earlier().ok(), ambiguous.later().ok()]
                        }
                        AmbiguousOffset::Unambiguous { .. } => [ambiguous.unambiguous().ok(), None],
                    };
                    for candidate in choices.into_iter().flatten().filter(|t| *t > at) {
                        next = Some(next.map_or(candidate, |current| current.min(candidate)));
                    }
                }
            }
        }
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moli_core::WeeklySlot;

    fn plan(start: u16, end: u16) -> Calendar {
        Calendar::new(WeeklySchedule {
            timezone: "Europe/Paris".into(),
            slots: vec![WeeklySlot { day: 7, start, end }],
        })
        .unwrap()
    }
    fn at(value: &str) -> Timestamp {
        value.parse().unwrap()
    }

    #[test]
    fn auto_h5_gap_never_invents_a_civil_instant_and_wakes_at_offset_jump() {
        let absent = plan(135, 165);
        assert!(!absent.active(at("2026-03-29T00:59:59Z")));
        assert!(!absent.active(at("2026-03-29T01:00:00Z")));
        let crossed = plan(150, 210);
        assert_eq!(
            crossed.next(at("2026-03-29T00:59:00Z")),
            Some(at("2026-03-29T01:00:00Z"))
        );
        assert!(crossed.active(at("2026-03-29T01:00:00Z")));
        assert!(!crossed.active(at("2026-03-29T01:30:00Z")));
    }
    #[test]
    fn auto_h5_fold_applies_both_copies_and_midnight_boundaries() {
        let repeated = plan(150, 165);
        assert!(repeated.active(at("2026-10-25T00:35:00Z")));
        assert!(!repeated.active(at("2026-10-25T01:00:00Z")));
        assert_eq!(
            repeated.next(at("2026-10-25T01:00:00Z")),
            Some(at("2026-10-25T01:30:00Z"))
        );
        assert!(repeated.active(at("2026-10-25T01:35:00Z")));
        assert!(!repeated.active(at("2026-10-25T01:45:00Z")));
        let full = plan(120, 180);
        assert!(full.active(at("2026-10-25T00:59:59Z")));
        assert!(full.active(at("2026-10-25T01:00:00Z")));
        assert!(!full.active(at("2026-10-25T02:00:00Z")));
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;
    #[test]
    fn auto_h5_calendar_budget_at_maximum_slots() {
        let plan = WeeklySchedule {
            timezone: "Europe/Paris".into(),
            slots: (0_u16..256)
                .map(|n| moli_core::WeeklySlot {
                    day: 7,
                    start: n * 2,
                    end: n * 2 + 1,
                })
                .collect(),
        };
        let bytes = serde_json::to_vec(&plan).unwrap().len();
        let calendar = Calendar::new(plan).unwrap();
        let at: Timestamp = "2026-10-25T00:00:00Z".parse().unwrap();
        let started = std::time::Instant::now();
        for _ in 0..100 {
            assert!(std::hint::black_box(&calendar).next(at).is_some());
        }
        eprintln!(
            "AUTO-H5 mesure native debug : 256 créneaux, définition {bytes} octets, 100 recherches {:?}",
            started.elapsed()
        );
    }
}
