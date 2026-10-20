//! Derive usage only between compatible successful observations.
use crate::model::{Reading, Samples};
use moli_core::Value;
use std::{collections::BTreeMap, time::Duration};
#[derive(Default)]
pub(crate) struct Tracker {
    cpu: Option<BTreeMap<String, (u64, u64)>>,
    wan: Option<(u64, u64)>,
}
impl Tracker {
    pub fn apply(&mut self, read: &Reading, elapsed: Duration) -> Samples {
        let mut samples = read.values.clone();
        if let Some(current) = &read.cpu {
            let percent = self
                .cpu
                .as_ref()
                .filter(|old| old.keys().eq(current.keys()))
                .and_then(|old| {
                    let mut total = 0u64;
                    let mut used = 0u64;
                    for (key, (t, u)) in current {
                        let (prev_t, prev_u) = old.get(key)?;
                        let dt = t.checked_sub(*prev_t)?;
                        let du = u.checked_sub(*prev_u)?;
                        if du > dt {
                            return None;
                        }
                        total = total.checked_add(dt)?;
                        used = used.checked_add(du)?;
                    }
                    (total > 0).then(|| {
                        // Only the dimensionless ratio is approximate, never raw counters.
                        #[allow(clippy::cast_precision_loss)]
                        let percent = used as f64 / total as f64 * 100.0;
                        percent
                    })
                });
            samples.insert("cpu", percent.map_or(Value::Null, Value::Float));
        }
        if let Some((rx, tx)) = read.wan {
            let rates = self.wan.and_then(|(old_rx, old_tx)| {
                if !(1.0..=86400.0).contains(&elapsed.as_secs_f64()) {
                    return None;
                }
                // Rates are floating estimates after exact checked integer deltas.
                #[allow(clippy::cast_precision_loss)]
                let rates = (
                    rx.checked_sub(old_rx)? as f64 / elapsed.as_secs_f64(),
                    tx.checked_sub(old_tx)? as f64 / elapsed.as_secs_f64(),
                );
                Some(rates)
            });
            samples.insert("rx_rate", rates.map_or(Value::Null, |v| Value::Float(v.0)));
            samples.insert("tx_rate", rates.map_or(Value::Null, |v| Value::Float(v.1)));
        }
        self.cpu.clone_from(&read.cpu);
        self.wan = read.wan;
        samples
    }
}
