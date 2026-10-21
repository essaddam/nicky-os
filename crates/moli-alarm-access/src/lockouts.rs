use serde::{Deserialize, Serialize};

pub(crate) const WINDOW: u64 = 10 * 60 * 1000;
pub(crate) const FIRST_LOCK: u64 = 15 * 60 * 1000;
pub(crate) const MAX_LOCK: u64 = 24 * 60 * 60 * 1000;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Attempts {
    pub failures: Vec<u64>,
    pub until: u64,
    pub strikes: u8,
    pub last: u64,
}

impl Attempts {
    pub fn locked(&self, now: u64) -> bool {
        self.until > 0 && (now.max(self.last) < self.until || self.until == u64::MAX)
    }
    pub fn refuse(&mut self, now: u64) -> Option<u64> {
        if self.locked(now) {
            return None;
        }
        let now = now.max(self.last);
        self.last = now;
        self.failures.retain(|at| now.saturating_sub(*at) < WINDOW);
        self.failures.push(now);
        if self.failures.len() < 5 {
            return None;
        }
        let duration = FIRST_LOCK
            .saturating_mul(1u64 << self.strikes.min(7))
            .min(MAX_LOCK);
        self.until = now.saturating_add(duration);
        self.strikes = self.strikes.saturating_add(1).min(8);
        self.failures.clear();
        Some(self.until)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fifth_refusal_locks_only_its_source_and_each_recurrence_doubles_to_a_day() {
        let mut attempts = Attempts::default();
        let other = Attempts::default();
        let mut now = 1_000;
        for duration in [
            FIRST_LOCK,
            FIRST_LOCK * 2,
            FIRST_LOCK * 4,
            FIRST_LOCK * 8,
            FIRST_LOCK * 16,
            FIRST_LOCK * 32,
            FIRST_LOCK * 64,
            MAX_LOCK,
            MAX_LOCK,
        ] {
            for _ in 0..4 {
                assert_eq!(attempts.refuse(now), None);
            }
            let end = attempts.refuse(now).unwrap();
            assert_eq!(end, now + duration);
            assert!(attempts.locked(end - 1));
            assert!(!attempts.locked(end));
            assert!(!other.locked(now));
            assert_eq!(attempts.refuse(now), None);
            now = end;
        }
    }
    #[test]
    fn old_failures_expire_and_a_clock_rollback_never_shortens_a_lock() {
        let mut a = Attempts::default();
        for _ in 0..4 {
            a.refuse(1_000);
        }
        assert_eq!(a.refuse(1_000 + WINDOW), None);
        for _ in 0..3 {
            a.refuse(1_000 + WINDOW);
        }
        let end = a.refuse(1_000 + WINDOW).unwrap();
        assert!(a.locked(0));
        assert_eq!(a.refuse(0), None);
        assert_eq!(a.until, end);
        let restored: Attempts = serde_json::from_slice(&serde_json::to_vec(&a).unwrap()).unwrap();
        assert!(restored.locked(end - 1));
        assert_eq!(restored.strikes, a.strikes);
    }
    #[test]
    fn a_deadline_overflow_keeps_the_source_locked_instead_of_wrapping() {
        let mut attempts = Attempts::default();
        for _ in 0..5 {
            attempts.refuse(u64::MAX - 10);
        }
        assert!(attempts.locked(u64::MAX));
        assert!(attempts.locked(0));
        assert_eq!(attempts.refuse(u64::MAX), None);
    }
}
