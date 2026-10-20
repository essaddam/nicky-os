use crate::{Boundary, Reservation, Transition, overview::overview};
use moli_core::{Access, Device, DeviceId, Kind, PointId, PointSpec, Semantic, Value};
use moli_runtime::DriverCtx;
use std::collections::HashMap;

fn point(key: &str, label: String, kind: Kind) -> PointSpec {
    PointSpec {
        key: key.into(),
        label: label.into(),
        kind,
        access: Access {
            read: true,
            write: false,
        },
        unit: None,
        semantic: Semantic::Other,
    }
}
fn number() -> Kind {
    Kind::Numeric {
        min: None,
        max: None,
        step: None,
    }
}
fn device(ctx: &DriverCtx, native: &str, name: String, points: Vec<PointSpec>) -> Device {
    Device {
        id: ctx.device_id(native),
        instance: ctx.instance().clone(),
        native_name: name.into(),
        manufacturer: None,
        model: Some("ICS".into()),
        description: None,
        native_room: None,
        members: Vec::new(),
        points,
    }
}

#[derive(Debug)]
pub(crate) struct Publisher {
    root: DeviceId,
    slots: Vec<DeviceId>,
    states: HashMap<PointId, Value>,
}

impl Publisher {
    pub(crate) fn new(ctx: &DriverCtx) -> Self {
        let root = device(
            ctx,
            "calendar",
            moli_i18n::tr!("pilotes.reservations.calendrier"),
            vec![
                point(
                    "state",
                    moli_i18n::tr!("pilotes.reservations.etat"),
                    Kind::Enum {
                        values: [
                            "no_reservation",
                            "awaiting_checkin",
                            "checked_in",
                            "checked_out",
                        ]
                        .into_iter()
                        .map(Into::into)
                        .collect(),
                    },
                ),
                point(
                    "observed",
                    moli_i18n::tr!("pilotes.reservations.observe"),
                    number(),
                ),
                point(
                    "arrival",
                    moli_i18n::tr!("pilotes.reservations.arrivee"),
                    Kind::Text,
                ),
                point(
                    "departure",
                    moli_i18n::tr!("pilotes.reservations.depart"),
                    Kind::Text,
                ),
            ],
        );
        let root_id = root.id.clone();
        ctx.upsert_device(root);
        ctx.set_availability(&root_id, false);
        let mut slots = Vec::new();
        for index in 1..=5 {
            let next = device(
                ctx,
                &format!("next-{index}"),
                moli_i18n::tr!("pilotes.reservations.prochaine", numero = index.to_string()),
                vec![
                    point(
                        "reservation_id",
                        moli_i18n::tr!("pilotes.reservations.identifiant"),
                        Kind::Text,
                    ),
                    point(
                        "start",
                        moli_i18n::tr!("pilotes.reservations.debut"),
                        number(),
                    ),
                    point("end", moli_i18n::tr!("pilotes.reservations.fin"), number()),
                    point(
                        "access",
                        moli_i18n::tr!("pilotes.reservations.acces"),
                        Kind::Binary,
                    ),
                ],
            );
            let id = next.id.clone();
            ctx.upsert_device(next);
            ctx.set_availability(&id, false);
            slots.push(id);
        }
        Self {
            root: root_id,
            slots,
            states: HashMap::new(),
        }
    }

    fn state(&mut self, ctx: &DriverCtx, id: &DeviceId, key: &str, value: Value) {
        let point = PointId::new(id, key);
        if self.states.get(&point) != Some(&value) {
            ctx.set_state(id, key, value.clone());
            self.states.insert(point, value);
        }
    }

    pub(crate) fn publish(
        &mut self,
        ctx: &DriverCtx,
        stays: &[Reservation],
        now: jiff::Timestamp,
        observed: i64,
        events: &[Transition],
    ) {
        let view = overview(stays, now);
        let root = self.root.clone();
        self.state(ctx, &root, "state", Value::from(view.cycle.as_str()));
        self.state(ctx, &root, "observed", Value::Int(observed));
        ctx.set_availability(&root, true);
        for index in 0..self.slots.len() {
            let id = self.slots[index].clone();
            if let Some(stay) = view.next.get(index) {
                self.state(ctx, &id, "reservation_id", Value::from(stay.id.as_str()));
                self.state(ctx, &id, "start", Value::Int(stay.start.as_millisecond()));
                self.state(ctx, &id, "end", Value::Int(stay.end.as_millisecond()));
                self.state(ctx, &id, "access", Value::Bool(stay.allows(now)));
                ctx.set_availability(&id, true);
            } else {
                ctx.set_availability(&id, false);
            }
        }
        for event in events {
            let key = match event.boundary {
                Boundary::Arrival => "arrival",
                Boundary::Departure => "departure",
            };
            ctx.pulse_state(&root, key, Value::from(event.reservation.as_str()));
        }
    }

    pub(crate) fn offline(&self, ctx: &DriverCtx) {
        ctx.set_availability(&self.root, false);
        for id in &self.slots {
            ctx.set_availability(id, false);
        }
    }
}
