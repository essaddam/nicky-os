//! Cold catalogue projection and schema-bound HA values, never a physical input.
use anyhow::{Result, ensure};
use moli_core::{Access, Device, EnumCodes, Kind, PointSpec, Semantic, Unit, Value};
use moli_runtime::{CommandRequest, DriverCtx};

use crate::{
    inventory::{Catalog, HaChannel, HaKind, HaState},
    protocol::CoverAction,
};

#[derive(Default)]
pub(super) struct Imports {
    pub catalog: Option<Catalog>,
    pub online: bool,
}

fn state_key(channel: &str) -> String {
    format!("state.{channel}")
}
fn command_key(channel: &str) -> String {
    format!("command.{channel}")
}

fn kind(channel: &HaChannel) -> Kind {
    match &channel.kind {
        HaKind::Binary => Kind::Binary,
        HaKind::Numeric => Kind::Numeric {
            min: None,
            max: None,
            step: None,
        },
        HaKind::Text => Kind::Text,
        HaKind::EnumCode {
            catalogue,
            version,
            choices,
        } => {
            let mut codes = EnumCodes::empty();
            for choice in choices {
                codes.insert(choice.code);
            }
            Kind::EnumCode {
                catalogue: *catalogue,
                version: *version,
                codes,
            }
        }
    }
}

fn points(channel: &HaChannel) -> Vec<PointSpec> {
    let mut points = Vec::with_capacity(2);
    if channel.readable {
        let unit = channel.unit.as_deref().and_then(Unit::parse);
        let semantic = channel
            .semantic
            .unwrap_or_else(|| Semantic::infer(&channel.entity_id, unit.as_ref()));
        points.push(PointSpec {
            key: state_key(&channel.id).into(),
            label: channel.name.as_str().into(),
            kind: kind(channel),
            access: Access {
                read: true,
                write: false,
            },
            unit,
            semantic,
        });
    }
    if channel.writable() {
        points.push(PointSpec {
            key: command_key(&channel.id).into(),
            label: channel.name.as_str().into(),
            kind: Kind::Enum {
                values: channel
                    .actions
                    .iter()
                    .map(|action| action.as_str().into())
                    .collect(),
            },
            access: Access {
                read: false,
                write: true,
            },
            unit: None,
            semantic: Semantic::Control,
        });
    }
    points
}

impl Imports {
    pub fn check_revision(&self, catalog: &Catalog) -> Result<()> {
        if let Some(current) = &self.catalog {
            ensure!(catalog.revision >= current.revision, "stale HA catalogue");
            if catalog.revision == current.revision {
                ensure!(
                    serde_json::to_vec(current)? == serde_json::to_vec(&catalog)?,
                    "conflicting HA catalogue revision"
                );
            }
        }
        Ok(())
    }

    pub fn install(&mut self, catalog: Catalog, ctx: &DriverCtx) -> Result<()> {
        self.check_revision(&catalog)?;
        if self
            .catalog
            .as_ref()
            .is_some_and(|current| current.revision == catalog.revision)
        {
            return Ok(());
        }
        let keep: Vec<_> = catalog
            .devices
            .iter()
            .map(|device| ctx.device_id(&device.id))
            .collect();
        for device in ctx.devices() {
            if !keep.contains(&device) {
                ctx.remove_device(&device);
            }
        }
        for device in &catalog.devices {
            let id = ctx.device_id(&device.id);
            ctx.upsert_device(Device {
                id: id.clone(),
                instance: ctx.instance().clone(),
                native_name: device.name.as_str().into(),
                manufacturer: None,
                model: None,
                description: None,
                native_room: None,
                members: vec![],
                points: device.channels.iter().flat_map(points).collect(),
            });
            ctx.set_availability(&id, self.online);
        }
        self.catalog = Some(catalog);
        Ok(())
    }

    pub fn availability(&mut self, online: bool, ctx: &DriverCtx) {
        self.online = online;
        for device in ctx.devices() {
            ctx.set_availability(&device, online);
        }
    }

    pub fn state(&self, state: &HaState, ctx: &DriverCtx) -> Result<()> {
        let Some(catalog) = &self.catalog else {
            return Ok(());
        };
        // MQTT retained ordering is not a catalogue barrier. A fresh inventory
        // publication resends states; no unbounded early-state queue is kept.
        if state.catalogue_revision != catalog.revision {
            return Ok(());
        }
        let value = if state.available {
            state.validate(catalog)?
        } else {
            let mut unavailable = state.clone();
            unavailable.value = serde_json::Value::Null;
            unavailable.validate(catalog)?;
            Value::Null
        };
        ctx.set_state(
            &ctx.device_id(&state.device),
            &state_key(&state.channel),
            value,
        );
        Ok(())
    }

    pub fn command(
        &self,
        command: &CommandRequest,
        ctx: &DriverCtx,
    ) -> Option<(String, CoverAction)> {
        let Value::Text(action) = &command.value else {
            return None;
        };
        let action = CoverAction::from_core_str(action)?;
        let channel_id = command.key.strip_prefix("command.")?;
        let catalog = self.catalog.as_ref()?;
        let device = catalog
            .devices
            .iter()
            .find(|device| ctx.device_id(&device.id) == command.device.id)?;
        let channel = device
            .channels
            .iter()
            .find(|channel| channel.id == channel_id && channel.actions.contains(&action))?;
        Some((channel.registry_entity_id.clone(), action))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::HaChoice;

    #[test]
    fn enum_state_and_closed_cover_actions_have_distinct_kinds_and_keys() {
        let channel = HaChannel {
            id: "cover-a".into(),
            entity_id: "cover.fixture".into(),
            registry_entity_id: "registry-a".into(),
            name: "Fixture cover".into(),
            unit: Some("%".into()),
            readable: true,
            semantic: None,
            kind: HaKind::EnumCode {
                catalogue: 7,
                version: 1,
                choices: vec![HaChoice {
                    code: 4,
                    native: "open".into(),
                    label: "Open".into(),
                }],
            },
            actions: vec![CoverAction::Open, CoverAction::Stop],
        };
        let specs = points(&channel);
        assert_eq!(specs[0].key.as_ref(), "state.cover-a");
        assert!(matches!(&specs[0].kind, Kind::EnumCode { codes, .. } if codes.contains(4)));
        assert_eq!(specs[0].unit, Some(Unit::Percent));
        assert_eq!(specs[1].key.as_ref(), "command.cover-a");
        assert!(
            matches!(&specs[1].kind, Kind::Enum { values } if values.iter().map(AsRef::as_ref).collect::<Vec<&str>>() == ["open", "stop"])
        );
        assert_eq!(specs[1].semantic, Semantic::Control);
    }
}
