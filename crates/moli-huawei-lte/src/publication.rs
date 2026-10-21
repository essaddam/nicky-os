//! Fixed diagnostic points, published only after a complete validated cycle.
use crate::{
    Failure,
    model::{self, Action, Reading, Samples},
    tracker::Tracker,
};
use moli_core::{Access, Device, DeviceId, Kind, PointSpec, Semantic, Unit, Value};
use moli_runtime::DriverCtx;
use std::{collections::BTreeSet, time::Duration};
use tokio::time::Instant;
pub(crate) const KEYS: [&str; 11] = [
    "connected",
    "technology",
    "rssi",
    "rsrp",
    "rsrq",
    "sinr",
    "rx_bytes",
    "tx_bytes",
    "rx_rate",
    "tx_rate",
    "duration",
];
pub(crate) type Reads = Vec<(Action, Option<Reading>, u64, Instant)>;
fn group(action: Action) -> &'static [&'static str] {
    match action {
        Action::Info => &[],
        Action::Status => &["connected", "technology"],
        Action::Signal => &["rssi", "rsrp", "rsrq", "sinr"],
        Action::Traffic => &["rx_bytes", "tx_bytes", "rx_rate", "tx_rate", "duration"],
    }
}
fn spec(key: &str) -> PointSpec {
    let (label, unit) = match key {
        "connected" => (moli_i18n::tr!("pilotes.huawei_lte.connected"), None),
        "technology" => (moli_i18n::tr!("pilotes.huawei_lte.technology"), None),
        "rssi" => (
            moli_i18n::tr!("pilotes.huawei_lte.rssi"),
            Some(Unit::Other("dBm".into())),
        ),
        "rsrp" => (
            moli_i18n::tr!("pilotes.huawei_lte.rsrp"),
            Some(Unit::Other("dBm".into())),
        ),
        "rsrq" => (
            moli_i18n::tr!("pilotes.huawei_lte.rsrq"),
            Some(Unit::Decibel),
        ),
        "sinr" => (
            moli_i18n::tr!("pilotes.huawei_lte.sinr"),
            Some(Unit::Decibel),
        ),
        "rx_bytes" => (
            moli_i18n::tr!("pilotes.huawei_lte.rx_bytes"),
            Some(Unit::Other("B".into())),
        ),
        "tx_bytes" => (
            moli_i18n::tr!("pilotes.huawei_lte.tx_bytes"),
            Some(Unit::Other("B".into())),
        ),
        "rx_rate" => (
            moli_i18n::tr!("pilotes.huawei_lte.rx_rate"),
            Some(Unit::Other("B/s".into())),
        ),
        "tx_rate" => (
            moli_i18n::tr!("pilotes.huawei_lte.tx_rate"),
            Some(Unit::Other("B/s".into())),
        ),
        "duration" => (
            moli_i18n::tr!("pilotes.huawei_lte.duration"),
            Some(Unit::Second),
        ),
        _ => unreachable!("fixed point vocabulary"),
    };
    PointSpec {
        key: key.into(),
        label: label.into(),
        kind: if key == "connected" {
            Kind::Binary
        } else {
            Kind::Numeric {
                min: None,
                max: None,
                step: None,
            }
        },
        unit,
        access: Access {
            read: true,
            write: false,
        },
        semantic: if matches!(key, "rssi" | "rsrp" | "rsrq" | "sinr") {
            Semantic::SignalStrength
        } else {
            Semantic::Diagnostic
        },
    }
}
pub(crate) struct Published<'a> {
    pub ctx: &'a mut DriverCtx,
    known: BTreeSet<DeviceId>,
    points: BTreeSet<&'static str>,
    epoch: u64,
    tracker: Tracker,
    last: Option<Instant>,
}
impl<'a> Published<'a> {
    pub fn new(ctx: &'a mut DriverCtx) -> Self {
        let known = ctx.known_devices().into_iter().collect();
        let state = Self {
            ctx,
            known,
            points: BTreeSet::new(),
            epoch: 0,
            tracker: Tracker::default(),
            last: None,
        };
        state.offline();
        state
    }
    pub fn check_known(&self) -> Result<(), Failure> {
        if self.known.len() > 1
            || self.known.iter().any(|id| {
                !id.as_str()
                    .split_once(':')
                    .and_then(|(_, native)| native.strip_prefix("modem_"))
                    .is_some_and(model::identity)
            })
        {
            return Err(Failure::Identity);
        }
        Ok(())
    }
    pub fn admit(&mut self, identity: &str) -> Result<(), Failure> {
        let target = self.ctx.device_id(&format!("modem_{identity}"));
        if self.known.iter().any(|id| id != &target) {
            return Err(Failure::Identity);
        }
        self.known.insert(target);
        Ok(())
    }
    pub fn offline(&self) {
        for id in self.ctx.devices() {
            for key in KEYS {
                if self.ctx.current(&id, key).is_some() {
                    self.ctx.set_state(&id, key, Value::Null);
                }
            }
            self.ctx.set_availability(&id, false);
        }
    }
    pub fn apply(&mut self, identity: &str, reads: Reads, epoch: u64) -> Result<(), Failure> {
        self.admit(identity)?;
        if self.epoch != epoch {
            self.tracker = Tracker::default();
            self.last = None;
            self.epoch = epoch;
        }
        let mut values = Samples::new();
        let mut refreshed = BTreeSet::new();
        for (action, read, read_epoch, observed) in reads {
            refreshed.extend(group(action).iter().copied());
            if action == Action::Traffic {
                if let Some(mut read) = read {
                    if read_epoch != epoch {
                        read.counters = None;
                        read.duration = None;
                    }
                    values.extend(self.tracker.apply(
                        &read,
                        self.last.map_or(Duration::ZERO, |last| observed - last),
                    ));
                    self.last = Some(observed);
                } else {
                    self.tracker = Tracker::default();
                    self.last = None;
                }
            } else if let Some(read) = read {
                values.extend(read.values);
            }
        }
        let previous = self.points.len();
        self.points.extend(values.keys().copied());
        if self.points.is_empty() {
            return Ok(());
        }
        let id = self.ctx.device_id(&format!("modem_{identity}"));
        if previous != self.points.len() {
            self.ctx.upsert_device(Device {
                id: id.clone(),
                instance: self.ctx.instance().clone(),
                native_name: moli_i18n::tr!("pilotes.huawei_lte.modem").into(),
                manufacturer: Some("Huawei".into()),
                model: None,
                description: None,
                native_room: None,
                members: vec![],
                points: self.points.iter().map(|key| spec(key)).collect(),
            });
        }
        for key in self.points.iter().filter(|key| refreshed.contains(*key)) {
            self.ctx
                .set_state(&id, key, values.get(key).cloned().unwrap_or(Value::Null));
        }
        self.ctx.set_availability(&id, true);
        Ok(())
    }
}
impl Drop for Published<'_> {
    fn drop(&mut self) {
        self.offline();
    }
}
