//! Publication happens only after the complete cycle passed validation.
use crate::{
    model::{Clients, Read, Samples},
    protocol::Action,
};
use moli_core::{Access, Device, Kind, PointSpec, Semantic, Unit, Value};
use moli_runtime::DriverCtx;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const KEYS: [&str; 8] = [
    "cpu",
    "memory",
    "firmware",
    "ethernet",
    "today_connection",
    "signal",
    "link",
    "rate",
];
fn group(action: Action) -> &'static [&'static str] {
    match action {
        Action::Info => &["firmware"],
        Action::System => &["cpu", "memory"],
        Action::Ethernet => &["ethernet"],
        Action::Traffic => &["today_connection"],
        _ => &[],
    }
}
fn specification(key: &str) -> PointSpec {
    let (label, unit) = match key {
        "cpu" => (moli_i18n::tr!("pilotes.netgear.cpu"), Some(Unit::Percent)),
        "memory" => (
            moli_i18n::tr!("pilotes.netgear.memory"),
            Some(Unit::Percent),
        ),
        "firmware" => (moli_i18n::tr!("pilotes.netgear.firmware"), None),
        "ethernet" => (moli_i18n::tr!("pilotes.netgear.ethernet"), None),
        "today_connection" => (
            moli_i18n::tr!("pilotes.netgear.today_connection"),
            Some(Unit::Second),
        ),
        "signal" => (moli_i18n::tr!("pilotes.netgear.signal"), None),
        "link" => (moli_i18n::tr!("pilotes.netgear.link"), None),
        "rate" => (
            moli_i18n::tr!("pilotes.netgear.rate"),
            Some(Unit::Other("Mbit/s".into())),
        ),
        _ => unreachable!("only fixed keys enter a point description"),
    };
    let kind = match key {
        "firmware" | "link" => Kind::Text,
        "ethernet" => Kind::Binary,
        _ => Kind::Numeric {
            min: None,
            max: None,
            step: None,
        },
    };
    PointSpec {
        key: key.into(),
        label: label.into(),
        kind,
        unit,
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
}
impl<'a> Published<'a> {
    pub fn new(ctx: &'a mut DriverCtx) -> Self {
        let state = Self {
            ctx,
            points: BTreeMap::new(),
        };
        state.offline();
        state
    }
    pub fn offline(&self) {
        for device in self.ctx.devices() {
            for key in KEYS {
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
        if values.is_empty() && !self.points.contains_key(native) {
            return;
        }
        let known = self.points.entry(native.to_owned()).or_default();
        let previous = known.len();
        known.extend(values.keys().copied());
        let device = self.ctx.device_id(native);
        if previous != known.len() {
            self.ctx.upsert_device(Device {
                id: device.clone(),
                instance: self.ctx.instance().clone(),
                native_name: if client {
                    moli_i18n::tr!("pilotes.netgear.client").into()
                } else {
                    moli_i18n::tr!("pilotes.netgear.routeur").into()
                },
                manufacturer: Some("Netgear".into()),
                model: None,
                description: None,
                native_room: None,
                members: vec![],
                points: known.iter().map(|key| specification(key)).collect(),
            });
        }
        for key in known.iter().filter(|key| refreshed.contains(*key)) {
            let value = match values.get(key) {
                Some(Value::Text(text)) if *key == "link" && text.as_ref() == "wired" => {
                    Value::from(moli_i18n::tr!("pilotes.netgear.filaire").as_str())
                }
                Some(Value::Text(text)) if *key == "link" && text.as_ref() == "wireless" => {
                    Value::from(moli_i18n::tr!("pilotes.netgear.sans_fil").as_str())
                }
                value => value.cloned().unwrap_or(Value::Null),
            };
            self.ctx.set_state(&device, key, value);
        }
        self.ctx.set_availability(&device, online);
    }
    pub fn apply(
        &mut self,
        identity: &str,
        selected: &[String],
        reads: Vec<(Action, Option<Read>)>,
    ) {
        let mut values = Samples::new();
        let mut refreshed = BTreeSet::new();
        let mut inventory: Option<Option<Clients>> = None;
        for (action, read) in reads {
            refreshed.extend(group(action).iter().copied());
            if matches!(action, Action::ClientsV1 | Action::ClientsV2) {
                inventory = Some(read.and_then(|r| r.clients));
            } else if let Some(read) = read {
                values.extend(read.values);
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
            let refreshed = BTreeSet::from(["link", "signal", "rate"]);
            for native in selected {
                let observed = inventory.as_ref().and_then(|clients| clients.get(native));
                self.update(
                    &format!("client_{native}"),
                    true,
                    observed.unwrap_or(&Samples::new()),
                    &refreshed,
                    observed.is_some(),
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
