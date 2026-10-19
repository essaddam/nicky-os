//! Source-time freshness; reception timestamps remain the hub's own clock.

use std::collections::{BTreeMap, BTreeSet};

use moli_core::Value;
use moli_runtime::DriverCtx;

use crate::{Error, model::Weather};

#[derive(Default)]
struct Seen {
    measured_at: Option<u64>,
    valid_until: u64,
    online: bool,
    keys: Vec<String>,
}

#[derive(Default)]
pub(crate) struct Observed(BTreeMap<String, Seen>);

impl Observed {
    pub fn apply(&mut self, ctx: &DriverCtx, weather: Weather, now: u64) -> Result<(), Error> {
        let mut known: BTreeSet<_> = ctx.devices().into_iter().collect();
        for reading in &weather.readings {
            known.insert(ctx.device_id(&reading.native));
        }
        if known.len() > 128 {
            return Err(Error::Invalid);
        }
        let present: BTreeSet<_> = weather.readings.iter().map(|r| r.native.clone()).collect();
        for (native, seen) in &mut self.0 {
            if !present.contains(native) {
                offline(ctx, native, seen);
            }
        }
        for reading in weather.readings {
            ctx.upsert_device(reading.device(ctx.instance()));
            let seen = self.0.entry(reading.native.clone()).or_default();
            seen.keys = reading.values.keys().cloned().collect();
            let newer = reading
                .measured_at
                .is_some_and(|at| seen.measured_at.is_none_or(|old| at > old));
            let not_older =
                reading.measured_at.is_some() && reading.measured_at >= seen.measured_at;
            if newer {
                seen.measured_at = reading.measured_at;
                seen.valid_until = reading.valid_until;
            }
            if !reading.online || !not_older || now > seen.valid_until {
                offline(ctx, &reading.native, seen);
                continue;
            }
            let id = ctx.device_id(&reading.native);
            for (key, value) in reading.values {
                // Missing fields invalidate immediately. A cached timestamp
                // never introduces a new known value or renews its age.
                if newer || !seen.online || value == Value::Null {
                    ctx.set_state(&id, &key, value);
                }
            }
            seen.online = true;
            ctx.set_availability(&id, true);
        }
        Ok(())
    }

    pub fn expire(&mut self, ctx: &DriverCtx, now: u64) {
        for (native, seen) in &mut self.0 {
            if seen.online && now > seen.valid_until {
                offline(ctx, native, seen);
            }
        }
    }

    pub fn next_expiry(&self) -> Option<u64> {
        self.0
            .values()
            .filter(|seen| seen.online)
            .map(|seen| seen.valid_until.saturating_add(1))
            .min()
    }
}

fn offline(ctx: &DriverCtx, native: &str, seen: &mut Seen) {
    let id = ctx.device_id(native);
    ctx.set_availability(&id, false);
    for key in &seen.keys {
        ctx.set_state(&id, key, Value::Null);
    }
    seen.online = false;
}

/// Also invalidates devices left in the hub by a previous supervised run.
pub(crate) fn invalidate_all(ctx: &DriverCtx) {
    for id in ctx.devices() {
        ctx.set_availability(&id, false);
        for key in [
            "temperature",
            "humidity",
            "co2",
            "pressure",
            "noise",
            "battery",
            "rain",
            "rain_1h",
            "rain_24h",
        ] {
            ctx.set_state(&id, key, Value::Null);
        }
    }
}
