//! Publish a validated cycle; never turn an unseen client into a presence fact.
use crate::{
    model::{Action, Reading, Samples},
    tracker::Tracker,
};
use moli_core::{Access, Device, Kind, PointSpec, Semantic, Unit, Value};
use moli_runtime::DriverCtx;
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};
use tokio::time::Instant;
const ROUTER: &[&str] = &[
    "firmware",
    "cpu",
    "memory",
    "memory_total",
    "memory_used",
    "memory_free",
    "rx_bytes",
    "tx_bytes",
    "rx_rate",
    "tx_rate",
    "client_count",
];
const CLIENT: &[&str] = &["link", "signal", "rate_rx", "rate_tx"];
fn group(action: Action) -> &'static [&'static str] {
    match action {
        Action::Info => &["firmware"],
        Action::Telemetry => &[
            "cpu",
            "memory",
            "memory_total",
            "memory_used",
            "memory_free",
            "rx_bytes",
            "tx_bytes",
            "rx_rate",
            "tx_rate",
        ],
        Action::Clients => &["client_count"],
    }
}
fn specification(key: &str) -> PointSpec {
    let (label, unit) = match key {
        "firmware" => (moli_i18n::tr!("pilotes.asuswrt.firmware"), None),
        "cpu" => (moli_i18n::tr!("pilotes.asuswrt.cpu"), Some(Unit::Percent)),
        "memory" => (
            moli_i18n::tr!("pilotes.asuswrt.memory"),
            Some(Unit::Percent),
        ),
        "memory_total" => (
            moli_i18n::tr!("pilotes.asuswrt.memory_total"),
            Some(Unit::Other("B".into())),
        ),
        "memory_used" => (
            moli_i18n::tr!("pilotes.asuswrt.memory_used"),
            Some(Unit::Other("B".into())),
        ),
        "memory_free" => (
            moli_i18n::tr!("pilotes.asuswrt.memory_free"),
            Some(Unit::Other("B".into())),
        ),
        "rx_bytes" => (
            moli_i18n::tr!("pilotes.asuswrt.rx_bytes"),
            Some(Unit::Other("B".into())),
        ),
        "tx_bytes" => (
            moli_i18n::tr!("pilotes.asuswrt.tx_bytes"),
            Some(Unit::Other("B".into())),
        ),
        "rx_rate" => (
            moli_i18n::tr!("pilotes.asuswrt.rx_rate"),
            Some(Unit::Other("B/s".into())),
        ),
        "tx_rate" => (
            moli_i18n::tr!("pilotes.asuswrt.tx_rate"),
            Some(Unit::Other("B/s".into())),
        ),
        "client_count" => (moli_i18n::tr!("pilotes.asuswrt.client_count"), None),
        "link" => (moli_i18n::tr!("pilotes.asuswrt.link"), None),
        "signal" => (moli_i18n::tr!("pilotes.asuswrt.signal"), None),
        "rate_rx" => (
            moli_i18n::tr!("pilotes.asuswrt.rate_rx"),
            Some(Unit::Other("Mbit/s".into())),
        ),
        "rate_tx" => (
            moli_i18n::tr!("pilotes.asuswrt.rate_tx"),
            Some(Unit::Other("Mbit/s".into())),
        ),
        _ => unreachable!("closed point vocabulary"),
    };
    PointSpec {
        key: key.into(),
        label: label.into(),
        unit,
        kind: if matches!(key, "firmware" | "link") {
            Kind::Text
        } else {
            Kind::Numeric {
                min: None,
                max: None,
                step: None,
            }
        },
        access: Access {
            read: true,
            write: false,
        },
        semantic: if key == "signal" {
            Semantic::SignalStrength
        } else {
            Semantic::Diagnostic
        },
    }
}
pub(crate) struct Published<'a> {
    pub ctx: &'a mut DriverCtx,
    points: BTreeMap<String, BTreeSet<&'static str>>,
    model: Option<String>,
    tracker: Tracker,
    last: Option<Instant>,
    epoch: u64,
}
impl<'a> Published<'a> {
    pub fn new(ctx: &'a mut DriverCtx) -> Self {
        let state = Self {
            ctx,
            points: BTreeMap::new(),
            model: None,
            tracker: Tracker::default(),
            last: None,
            epoch: 0,
        };
        state.offline();
        state
    }
    pub fn offline(&self) {
        for device in self.ctx.devices() {
            for key in ROUTER.iter().chain(CLIENT) {
                if self.ctx.current(&device, key).is_some() {
                    self.ctx.set_state(&device, key, Value::Null);
                }
            }
            self.ctx.set_availability(&device, false);
        }
    }
    fn update(
        &mut self,
        native: &str,
        client: bool,
        values: &Samples,
        refreshed: &BTreeSet<&'static str>,
        online: bool,
    ) {
        let known = self.points.entry(native.to_owned()).or_default();
        let previous = known.len();
        known.extend(
            values
                .iter()
                .filter(|(_, value)| !matches!(value, Value::Null))
                .map(|(key, _)| *key),
        );
        if known.is_empty() {
            return;
        }
        let device = self.ctx.device_id(native);
        if previous != known.len() {
            self.ctx.upsert_device(Device {
                id: device.clone(),
                instance: self.ctx.instance().clone(),
                native_name: if client {
                    moli_i18n::tr!("pilotes.asuswrt.client").into()
                } else {
                    moli_i18n::tr!("pilotes.asuswrt.routeur").into()
                },
                manufacturer: Some("ASUS".into()),
                model: if client {
                    None
                } else {
                    self.model.clone().map(Into::into)
                },
                description: None,
                native_room: None,
                members: vec![],
                points: known.iter().map(|key| specification(key)).collect(),
            });
        }
        for key in known.iter().filter(|key| refreshed.contains(*key)) {
            let value = match values.get(key) {
                Some(Value::Text(text)) if *key == "link" && text.as_ref() == "wired" => {
                    Value::from(moli_i18n::tr!("pilotes.asuswrt.filaire").as_str())
                }
                Some(Value::Text(text)) if *key == "link" && text.as_ref() == "wireless" => {
                    Value::from(moli_i18n::tr!("pilotes.asuswrt.sans_fil").as_str())
                }
                v => v.cloned().unwrap_or(Value::Null),
            };
            self.ctx.set_state(&device, key, value);
        }
        self.ctx.set_availability(&device, online);
    }
    pub fn apply(
        &mut self,
        identity: &str,
        selected: &[String],
        reads: Vec<(Action, Option<Reading>, u64)>,
        epoch: u64,
    ) {
        if self.epoch != epoch {
            self.tracker = Tracker::default();
            self.last = None;
            self.epoch = epoch;
        }
        let mut values = Samples::new();
        let mut refreshed = BTreeSet::new();
        let mut inventory = None;
        for (action, read, read_epoch) in reads {
            refreshed.extend(group(action).iter().copied());
            if let Some(mut read) = read {
                match action {
                    Action::Info => {
                        self.model = read.model;
                        values.extend(read.values);
                    }
                    Action::Telemetry => {
                        if read_epoch != epoch {
                            read.cpu = None;
                            read.wan = None;
                        }
                        let now = Instant::now();
                        values.extend(
                            self.tracker
                                .apply(&read, self.last.map_or(Duration::ZERO, |last| now - last)),
                        );
                        self.last = Some(now);
                    }
                    Action::Clients => {
                        if let Some(clients) = &read.clients {
                            values.insert(
                                "client_count",
                                Value::Int(
                                    i64::try_from(clients.len()).expect("inventory bounded to 512"),
                                ),
                            );
                        }
                        inventory = Some(read.clients);
                    }
                }
            } else if action == Action::Clients {
                inventory = Some(None);
            }
        }
        self.update(
            &format!("router_{identity}"),
            false,
            &values,
            &refreshed,
            true,
        );
        if let Some(inventory) = inventory {
            let refreshed = CLIENT.iter().copied().collect();
            for native in selected {
                let observed = inventory.as_ref().and_then(|clients| clients.get(native));
                self.update(
                    &format!("client_{native}"),
                    true,
                    observed.unwrap_or(&Samples::new()),
                    &refreshed,
                    observed.is_some_and(|s| !s.is_empty()),
                );
            }
        }
    }
}
impl Drop for Published<'_> {
    fn drop(&mut self) {
        self.offline();
    }
}
