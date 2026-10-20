//! Ephemeral reservation admission. Never restored from cached device states.

use moli_core::{AccessWindow, InstanceId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Instant;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReservationRef {
    pub instance: InstanceId,
    pub id: String,
}

impl ReservationRef {
    #[must_use]
    pub fn valid(&self) -> bool {
        let instance = self.instance.as_str();
        !instance.is_empty()
            && instance.len() <= 64
            && instance
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
            && self.id.len() == 32
            && self.id.bytes().all(|c| c.is_ascii_hexdigit())
    }
}

#[derive(Debug)]
struct Snapshot {
    checked: u64,
    expires: u64,
    deadline: Instant,
    windows: BTreeMap<String, AccessWindow>,
}

#[derive(Debug, Default)]
pub(crate) struct Book(BTreeMap<InstanceId, Snapshot>);

impl Book {
    pub(crate) fn replace(
        &mut self,
        instance: &InstanceId,
        windows: Vec<(String, AccessWindow)>,
        expires: u64,
        now: u64,
        monotonic: Instant,
    ) -> bool {
        self.remove(instance);
        let Some(ttl) = expires
            .checked_sub(now)
            .filter(|ttl| (1..=3_610_000).contains(ttl))
        else {
            return false;
        };
        if windows.len() > 4096 {
            return false;
        }
        let mut indexed = BTreeMap::new();
        for (id, window) in windows {
            if !(ReservationRef {
                instance: instance.clone(),
                id: id.clone(),
            })
            .valid()
                || indexed.insert(id, window).is_some()
            {
                return false;
            }
        }
        self.0.insert(
            instance.clone(),
            Snapshot {
                checked: now,
                expires,
                deadline: monotonic + std::time::Duration::from_millis(ttl),
                windows: indexed,
            },
        );
        true
    }

    pub(crate) fn remove(&mut self, instance: &InstanceId) {
        self.0.remove(instance);
    }

    pub(crate) fn allows(
        &self,
        reservation: &ReservationRef,
        now: u64,
        monotonic: Instant,
    ) -> bool {
        reservation.valid()
            && self.0.get(&reservation.instance).is_some_and(|snapshot| {
                now >= snapshot.checked
                    && now < snapshot.expires
                    && monotonic < snapshot.deadline
                    && snapshot
                        .windows
                        .get(&reservation.id)
                        .is_some_and(|window| window.allows(now))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    const ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    fn reference(instance: &str) -> ReservationRef {
        ReservationRef {
            instance: instance.into(),
            id: ID.into(),
        }
    }

    #[test]
    fn both_window_bounds_and_snapshot_expiry_are_exact() {
        let mut book = Book::default();
        let mono = Instant::now();
        let binding = reference("calendar");
        assert!(book.replace(
            &binding.instance,
            vec![(ID.into(), AccessWindow::new(1100, 1200).unwrap())],
            1500,
            1000,
            mono
        ));
        assert!(!book.allows(&binding, 1099, mono));
        assert!(book.allows(&binding, 1100, mono));
        assert!(book.allows(&binding, 1199, mono));
        assert!(!book.allows(&binding, 1200, mono));
        assert!(!book.allows(&binding, 1100, mono + Duration::from_millis(500)));
        assert!(!book.allows(&binding, 999, mono));
        assert!(!book.allows(&reference("other"), 1100, mono));
    }

    #[test]
    fn replacement_cancellation_and_removal_never_reuse_old_windows() {
        let mut book = Book::default();
        let mono = Instant::now();
        let binding = reference("calendar");
        let window = AccessWindow::new(1000, 2000).unwrap();
        assert!(book.replace(
            &binding.instance,
            vec![(ID.into(), window)],
            1500,
            1000,
            mono
        ));
        assert!(book.allows(&binding, 1000, mono));
        assert!(book.replace(&binding.instance, vec![], 1500, 1000, mono));
        assert!(!book.allows(&binding, 1000, mono));
        assert!(book.replace(
            &binding.instance,
            vec![(ID.into(), window)],
            1500,
            1000,
            mono
        ));
        book.remove(&binding.instance);
        assert!(!book.allows(&binding, 1000, mono));
    }

    #[test]
    fn hostile_or_stale_replacements_close_the_previous_snapshot() {
        let mut book = Book::default();
        let mono = Instant::now();
        let binding = reference("calendar");
        let window = AccessWindow::new(1000, 2000).unwrap();
        for (windows, expires) in [
            (vec![(ID.into(), window), (ID.into(), window)], 1500),
            (vec![("supplier-private-uid".into(), window)], 1500),
            (vec![(ID.into(), window); 4097], 1500),
            (vec![(ID.into(), window)], 1000),
            (vec![(ID.into(), window)], 4_000_000),
        ] {
            assert!(book.replace(
                &binding.instance,
                vec![(ID.into(), window)],
                1500,
                1000,
                mono
            ));
            assert!(!book.replace(&binding.instance, windows, expires, 1000, mono));
            assert!(!book.allows(&binding, 1000, mono));
        }
    }
}
