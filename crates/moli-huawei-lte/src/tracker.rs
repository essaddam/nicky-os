//! Exact integer differences, converted only within the safe integer interval.
use crate::model::{MAX_EXACT, Reading, Samples};
use moli_core::Value;
use std::time::Duration;
#[derive(Default)]
pub(crate) struct Tracker {
    counters: Option<(u64, u64)>,
    duration: Option<u64>,
}
impl Tracker {
    pub fn apply(&mut self, read: &Reading, elapsed: Duration) -> Samples {
        let mut values = read.values.clone();
        let continuous = read
            .duration
            .zip(self.duration)
            .is_some_and(|(now, old)| now > old);
        let deltas = read
            .counters
            .zip(self.counters)
            .filter(|_| continuous)
            .and_then(|((rx, tx), (old_rx, old_tx))| {
                Some((rx.checked_sub(old_rx)?, tx.checked_sub(old_tx)?))
            });
        if let Some((rx, tx)) = deltas.filter(|_| (1.0..=86400.0).contains(&elapsed.as_secs_f64()))
        {
            for (key, delta) in [("rx_rate", rx), ("tx_rate", tx)] {
                if !values.contains_key(key) && delta <= MAX_EXACT {
                    // Bounded to 2^53-1: this integer conversion is exact, only division rounds.
                    #[allow(clippy::cast_precision_loss)]
                    let rate = delta as f64 / elapsed.as_secs_f64();
                    values.insert(key, Value::Float(rate));
                }
            }
        }
        self.counters = read.counters;
        self.duration = read.duration;
        values
    }
}
