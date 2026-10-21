//! Pure, bounded coverage ledger. Intervals are half-open; gaps are unknown.
use crate::{AggregateOp, AggregateSpec, MAX_FRESHNESS_S, PointId};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

pub const MAX_DURATION_SEGMENTS: usize = 256;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DurationSpec {
    pub source: PointId,
    #[serde(default)]
    pub source_unit: Option<crate::Unit>,
    pub value: serde_json::Value,
    pub max_age_s: u64,
    pub window_s: u64,
}
impl DurationSpec {
    pub fn normalize(self) -> Result<Self, crate::InvalidAggregate> {
        AggregateSpec {
            operation: AggregateOp::Mean,
            sources: vec![self.source.clone()],
            unit: self.source_unit.clone(),
            max_age_s: self.max_age_s,
        }
        .normalize()?;
        if !(1..=MAX_FRESHNESS_S).contains(&self.window_s)
            || self.value.is_null()
            || self.value.is_array()
            || self.value.is_object()
            || self.value.as_str().is_some_and(|s| s.len() > 2000)
            || crate::privacy::private_value(&crate::Value::from_json(&self.value))
        {
            return Err(crate::InvalidAggregate);
        }
        Ok(self)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Segment {
    from: u64,
    to: u64,
    matched: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Open {
    matched: bool,
    entered: u64,
    expires: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DurationLedger {
    created: u64,
    cursor: u64,
    last_event: u64,
    segments: VecDeque<Segment>,
    open: Option<Open>,
    limited_until: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DurationStats {
    pub window_start_ms: u64,
    pub as_of_ms: u64,
    pub acquired_ms: u64,
    pub known_ms: u64,
    pub unknown_ms: u64,
    pub entered_ms: Option<u64>,
    pub observing: bool,
    pub limited: bool,
}
impl DurationLedger {
    #[must_use]
    pub fn new(now: u64) -> Self {
        Self {
            created: now,
            cursor: now,
            last_event: now,
            segments: VecDeque::new(),
            open: None,
            limited_until: 0,
        }
    }
    #[must_use]
    pub fn cursor(&self) -> u64 {
        self.cursor
    }
    #[must_use]
    pub fn valid(&self) -> bool {
        self.created <= self.last_event
            && self.last_event <= self.cursor
            && self.segments.len() <= MAX_DURATION_SEGMENTS
            && self
                .segments
                .iter()
                .all(|s| self.created <= s.from && s.from < s.to && s.to <= self.cursor)
            && self
                .segments
                .iter()
                .zip(self.segments.iter().skip(1))
                .all(|(a, b)| a.to <= b.from)
            && self.open.as_ref().is_none_or(|o| {
                self.created <= o.entered
                    && o.entered <= self.last_event
                    && self.last_event <= o.expires
                    && o.expires - self.last_event <= MAX_FRESHNESS_S * 1000
                    && self.cursor < o.expires
            })
    }
    /// Restart never extrapolates an open interval past its durable checkpoint.
    pub fn resume(&mut self) {
        self.open = None;
        self.last_event = self.cursor;
    }
    /// `matched=None` closes coverage. Equal observations renew freshness only.
    /// Older events cannot replace a newer observation. A delayed event between
    /// the last observation and a checkpoint corrects that assumed suffix.
    pub fn observe(&mut self, at: u64, matched: Option<bool>, freshness_ms: u64) {
        if at < self.last_event {
            return;
        }
        let previous = self.open.clone().filter(|o| at <= o.expires);
        if at < self.cursor {
            self.segments.retain(|s| s.from < at);
            if let Some(s) = self.segments.back_mut() {
                s.to = s.to.min(at);
            }
            self.cursor = at;
        }
        self.advance(at);
        self.last_event = at;
        self.open = matched.and_then(|matched| {
            at.checked_add(freshness_ms)
                .filter(|end| *end > at)
                .map(|expires| Open {
                    matched,
                    expires,
                    entered: previous
                        .filter(|o| o.matched == matched)
                        .map_or(at, |o| o.entered),
                })
        });
    }
    pub fn advance(&mut self, now: u64) {
        if now < self.cursor {
            return;
        }
        if let Some(open) = &self.open {
            let to = now.min(open.expires);
            let matched = open.matched;
            if to > self.cursor {
                if let Some(last) = self
                    .segments
                    .back_mut()
                    .filter(|s| s.to == self.cursor && s.matched == matched)
                {
                    last.to = to;
                } else {
                    self.segments.push_back(Segment {
                        from: self.cursor,
                        to,
                        matched,
                    });
                }
            }
            if now >= open.expires {
                self.open = None;
            }
        }
        self.cursor = now;
        while self.segments.len() > MAX_DURATION_SEGMENTS {
            self.limited_until = self
                .limited_until
                .max(self.segments.pop_front().unwrap().to);
        }
    }
    pub fn prune(&mut self, window_ms: u64) {
        let start = self.cursor.saturating_sub(window_ms);
        while self.segments.front().is_some_and(|s| s.to <= start) {
            self.segments.pop_front();
        }
        if let Some(first) = self.segments.front_mut() {
            first.from = first.from.max(start);
        }
    }
    #[must_use]
    pub fn stats(&self, now: u64, window_ms: u64) -> DurationStats {
        let now = now.max(self.cursor);
        let start = self.created.max(now.saturating_sub(window_ms));
        let mut known = 0;
        let mut acquired = 0;
        for s in &self.segments {
            let length = s.to.min(now).saturating_sub(s.from.max(start));
            known += length;
            if s.matched {
                acquired += length;
            }
        }
        DurationStats {
            window_start_ms: start,
            as_of_ms: now,
            acquired_ms: acquired,
            known_ms: known,
            unknown_ms: now.saturating_sub(start).saturating_sub(known),
            entered_ms: self
                .open
                .as_ref()
                .filter(|o| o.matched && now < o.expires)
                .map(|o| o.entered),
            observing: self.open.as_ref().is_some_and(|o| now < o.expires),
            limited: self.limited_until > start,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn auto_k3_coverage_stops_at_freshness_and_restart_is_idempotent() {
        let mut ledger = DurationLedger::new(1000);
        ledger.observe(1000, Some(true), 2000);
        ledger.advance(5000);
        let before = ledger.stats(5000, 10000);
        assert_eq!(
            (before.acquired_ms, before.known_ms, before.unknown_ms),
            (2000, 2000, 2000)
        );
        let mut resumed = ledger.clone();
        resumed.resume();
        resumed.resume();
        resumed.advance(7000);
        let after = resumed.stats(7000, 10000);
        assert_eq!(
            (after.acquired_ms, after.known_ms, after.unknown_ms),
            (2000, 2000, 4000)
        );
    }
}

#[cfg(test)]
mod coverage_tests {
    use super::*;
    #[test]
    fn auto_k3_equal_frontier_and_disordered_events_preserve_entry_and_bounds() {
        let mut ledger = DurationLedger::new(0);
        ledger.observe(0, Some(true), 1000);
        ledger.observe(1000, Some(true), 1000);
        assert_eq!(ledger.stats(1000, 10000).entered_ms, Some(0));
        ledger.observe(1500, Some(false), 1000);
        ledger.observe(1400, Some(true), 1000); // Older telemetry cannot undo a newer state.
        ledger.advance(2500);
        let stats = ledger.stats(2500, 2000);
        assert_eq!(
            (stats.acquired_ms, stats.known_ms, stats.unknown_ms),
            (1000, 2000, 0)
        );
        assert!(ledger.valid());
    }
    #[test]
    fn auto_k3_gap_rollback_and_window_edges_never_invent_observations() {
        let mut ledger = DurationLedger::new(1000);
        ledger.observe(1000, Some(true), 2000);
        ledger.observe(2000, None, 0);
        ledger.observe(1500, Some(false), 10000);
        ledger.advance(3000);
        ledger.advance(1000); // A civil rollback cannot subtract or count twice.
        let stats = ledger.stats(3000, 1000);
        assert_eq!(
            (stats.acquired_ms, stats.known_ms, stats.unknown_ms),
            (0, 0, 1000)
        );
        ledger.resume();
        ledger.observe(2999, Some(true), 10000);
        ledger.advance(4000);
        assert_eq!(ledger.stats(4000, 2000).known_ms, 0);
        ledger.observe(4000, Some(false), 1000);
        ledger.advance(5000);
        assert_eq!(ledger.stats(5000, 2000).known_ms, 1000);
    }
    #[test]
    fn auto_k3_segment_budget_discards_only_to_unknown() {
        let mut ledger = DurationLedger::new(0);
        for at in 0..600 {
            ledger.observe(at, Some(at % 2 == 0), 10000);
        }
        ledger.advance(600);
        let stats = ledger.stats(600, 1000);
        assert_eq!(stats.known_ms, 256);
        assert_eq!(stats.unknown_ms, 344);
        assert!(stats.limited);
        ledger.prune(100);
        assert_eq!(ledger.stats(600, 100).unknown_ms, 0);
        assert!(!ledger.stats(600, 100).limited);
        assert!(ledger.valid());
    }
    #[test]
    fn auto_k3_late_observation_corrects_an_assumed_checkpoint_suffix() {
        let mut ledger = DurationLedger::new(0);
        ledger.observe(0, Some(true), 1000);
        ledger.advance(500);
        ledger.observe(250, Some(false), 1000);
        ledger.advance(750);
        assert_eq!(ledger.stats(750, 1000).acquired_ms, 250);
        assert_eq!(ledger.stats(750, 1000).known_ms, 750);
    }
}

#[cfg(test)]
mod journal_tests {
    #[test]
    fn auto_k3_legacy_computed_journal_remains_readable_without_coverage() {
        let old = serde_json::json!({"kind":"computed","point":"fixture:average/value","version":1,"value":2.5,"source_ts":1000,"unit":"s","valid":1});
        let action: crate::Action = serde_json::from_value(old.clone()).unwrap();
        assert!(matches!(
            action,
            crate::Action::Computed { coverage: None, .. }
        ));
        assert_eq!(serde_json::to_value(action).unwrap(), old);
    }
}
