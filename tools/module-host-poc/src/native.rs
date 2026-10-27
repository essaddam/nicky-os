//! Fixture-backed adapters for native Moli models. Commands are encoded for
//! inspection only; this module has no transport or credentials.

use std::collections::HashMap;

use anyhow::{Context, Result, ensure};
use moli_ariston::model as ariston_model;
use moli_core::{Device, DeviceId, InstanceId, PointId, Value};
use moli_overkiz::model as overkiz_model;
use serde_json::{Value as Json, json};

use crate::adapter::{Adapter, Observation};

const MAX_COMMAND_BYTES: usize = 256;
const OVERKIZ_SETUP: &str =
    include_str!("../../../integrations/overkiz/fixtures/setup-devices.json");
const ARISTON_PLANTS: &str =
    include_str!("../../../integrations/ariston/fixtures/velis-plants.json");
const ARISTON_STATE: &str =
    include_str!("../../../integrations/ariston/fixtures/med-plant-data.json");
const ARISTON_SETTINGS: &str =
    include_str!("../../../integrations/ariston/fixtures/plant-settings.json");
const ARISTON_ERRORS: &str = include_str!("../../../integrations/ariston/fixtures/bus-errors.json");

/// Creates the TaHoma fixture adapter. RTS devices remain write-only; their
/// fixture has no state entries, so neither this adapter nor its observer can
/// synthesize a position.
pub fn tahoma(instance: &InstanceId) -> Result<Box<dyn Adapter>> {
    let fixture: Json = serde_json::from_str(OVERKIZ_SETUP).context("Overkiz fixture JSON")?;
    let loaded = overkiz_model::load(instance, &fixture);
    let mut devices = Vec::with_capacity(loaded.devices.len());
    let mut states = HashMap::with_capacity(loaded.devices.len());
    for row in fixture.as_array().into_iter().flatten() {
        let Some(url) = row["deviceURL"].as_str() else {
            continue;
        };
        let Some(device) = loaded.index.device(url).cloned() else {
            continue;
        };
        let raw_states = row["states"].as_array().cloned().unwrap_or_default();
        states.insert(device, raw_states);
    }
    devices.extend(loaded.devices.into_iter().map(|found| found.device));
    Ok(Box::new(TahomaAdapter {
        description: Some(devices),
        index: loaded.index,
        states,
    }))
}

/// Creates the Ariston fixture adapter. The POC includes the exact model
/// decoder from the production crate and does not instantiate its cloud client.
pub fn ariston(instance: &InstanceId) -> Result<Box<dyn Adapter>> {
    let plants: Json = serde_json::from_str(ARISTON_PLANTS).context("Ariston plants fixture")?;
    let raw_state: Json = serde_json::from_str(ARISTON_STATE).context("Ariston state fixture")?;
    let settings: Json =
        serde_json::from_str(ARISTON_SETTINGS).context("Ariston settings fixture")?;
    let errors: Json = serde_json::from_str(ARISTON_ERRORS).context("Ariston errors fixture")?;
    let heaters = ariston_model::heaters(&plants);
    ensure!(
        !heaters.is_empty(),
        "Ariston fixture has no supported heater"
    );
    let (_, ceiling_range) = ariston_model::ceiling(&settings);
    let description = heaters
        .iter()
        .map(|heater| heater.device(instance, ceiling_range))
        .collect();
    Ok(Box::new(AristonAdapter {
        instance: instance.clone(),
        description: Some(description),
        heaters,
        state: raw_state,
        settings,
        errors,
    }))
}

struct TahomaAdapter {
    description: Option<Vec<Device>>,
    index: overkiz_model::Index,
    states: HashMap<DeviceId, Vec<Json>>,
}

impl Adapter for TahomaAdapter {
    fn take_description(&mut self) -> Vec<Device> {
        self.description.take().unwrap_or_default()
    }

    fn observe(&mut self, step: u64) -> Result<Vec<Observation>> {
        let mut observations = Vec::new();
        for (device, original) in &self.states {
            let mut states = original.clone();
            for state in &mut states {
                match state["name"].as_str() {
                    Some("core:OpenClosedState") => {
                        state["value"] = json!(if step.is_multiple_of(2) {
                            "closed"
                        } else {
                            "open"
                        });
                    }
                    Some("core:OnOffState") => {
                        state["value"] = json!(if step.is_multiple_of(2) { "off" } else { "on" });
                    }
                    Some("core:LightIntensityState") => {
                        state["value"] = json!(40 + step % 61);
                    }
                    _ => {}
                }
            }
            observations.extend(
                overkiz_model::decode(&states)
                    .into_iter()
                    .map(|(key, value)| Observation {
                        point: PointId::new(device, key),
                        value,
                    }),
            );
        }
        Ok(observations)
    }

    fn simulate_command(&self, point: &PointId, value: &Value) -> Result<Json> {
        let (device, key) = point.split().context("malformed Overkiz point ID")?;
        let order = self
            .index
            .encode(&device, key, value)
            .context("Overkiz model refused this command or value")?;
        bounded(order.body())
    }

    fn enum_label(&self, point: &PointId, native: &str) -> String {
        if point.split().is_some_and(|(_, key)| key == "control") && native == "my" {
            "Favori".into()
        } else {
            native.to_owned()
        }
    }
}

struct AristonAdapter {
    instance: InstanceId,
    description: Option<Vec<Device>>,
    heaters: Vec<ariston_model::Heater>,
    state: Json,
    settings: Json,
    errors: Json,
}

impl Adapter for AristonAdapter {
    fn take_description(&mut self) -> Vec<Device> {
        self.description.take().unwrap_or_default()
    }

    fn observe(&mut self, step: u64) -> Result<Vec<Observation>> {
        let mut state = self.state.clone();
        let offset = f64::from(u32::try_from(step % 7).unwrap_or_default());
        state["temp"] = json!(50.0 + offset * 0.5);
        state["reqTemp"] = json!(55.0 + offset * 0.5);
        state["procReqTemp"] = state["reqTemp"].clone();
        state["mode"] = json!(if step.is_multiple_of(2) { 1 } else { 5 });
        state["eco"] = json!(step.is_multiple_of(2));

        let mut values = ariston_model::state(&state);
        values.extend(ariston_model::settings(&self.settings));
        values.push(("errors", ariston_model::errors(&self.errors)));
        let mut observations = Vec::new();
        for heater in &self.heaters {
            let device = DeviceId::new(&self.instance, &heater.gateway);
            observations.extend(values.iter().map(|(key, value)| Observation {
                point: PointId::new(&device, key),
                value: value.clone(),
            }));
        }
        Ok(observations)
    }

    fn simulate_command(&self, point: &PointId, value: &Value) -> Result<Json> {
        let (device, key) = point.split().context("malformed Ariston point ID")?;
        let heater = self
            .heaters
            .iter()
            .find(|heater| DeviceId::new(&self.instance, &heater.gateway) == device)
            .context("Ariston model refused this device")?;
        let _heater = heater;
        let order = ariston_model::order(key, value, &self.settings)
            .map_err(anyhow::Error::msg)
            .context("Ariston model refused this command or value")?;
        bounded(json!({
            "gateway": device.as_str().split_once(':').map(|(_, id)| id).unwrap_or_default(),
            "action": order.action,
            "body": order.body,
        }))
    }
}

fn bounded(value: Json) -> Result<Json> {
    let bytes = serde_json::to_vec(&value)?;
    ensure!(
        bytes.len() <= MAX_COMMAND_BYTES,
        "simulated command exceeds byte bound"
    );
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observations(adapter: &mut dyn Adapter, step: u64) -> Vec<(PointId, Value)> {
        adapter
            .observe(step)
            .unwrap()
            .into_iter()
            .map(|observation| (observation.point, observation.value))
            .collect()
    }

    #[test]
    fn tahoma_publishes_rts_write_only_and_only_encodes_declared_moves() {
        let instance = InstanceId::from("tahoma-fixture");
        let mut adapter = tahoma(&instance).unwrap();
        let devices = adapter.take_description();
        assert!(!devices.is_empty());
        let shutter = devices
            .iter()
            .find(|device| {
                device.point("control").is_some_and(|point| {
                    matches!(&point.kind, moli_core::Kind::Enum { values } if values.len() == 4)
                })
            })
            .expect("fixture RTS shutter with four declared moves");
        let control = shutter.point("control").unwrap();
        assert!(control.access.write);
        assert!(!control.access.read);
        assert!(
            shutter.points.iter().all(|point| {
                !matches!(&*point.key, "opening" | "position" | "current_position")
            })
        );

        let point = PointId::new(&shutter.id, "control");
        for command in ["open", "close", "stop", "my"] {
            let body = adapter
                .simulate_command(&point, &Value::from(command))
                .unwrap();
            assert_eq!(body["actions"][0]["commands"][0]["name"], command);
            assert_eq!(body["actions"][0]["commands"][0]["parameters"], json!([]));
        }
        assert!(
            adapter
                .simulate_command(&point, &Value::from("identify"))
                .is_err()
        );

        let gate = devices
            .iter()
            .find(|device| device.point("opening").is_some())
            .expect("fixture OGP gate with reported opening state");
        assert!(!gate.point("opening").unwrap().access.write);
        assert!(
            adapter
                .simulate_command(&PointId::new(&gate.id, "opening"), &Value::from("open"))
                .is_err()
        );

        let before = observations(adapter.as_mut(), 0);
        adapter
            .simulate_command(&point, &Value::from("open"))
            .unwrap();
        let after = observations(adapter.as_mut(), 0);
        assert_eq!(before, after, "encoding must not change observations");
    }

    #[test]
    fn ariston_decodes_multichannel_state_and_bounds_simulated_writes() {
        let instance = InstanceId::from("ariston-fixture");
        let mut adapter = ariston(&instance).unwrap();
        let devices = adapter.take_description();
        assert_eq!(devices.len(), 1);
        assert!(adapter.take_description().is_empty());
        let heater = &devices[0];
        assert_eq!(heater.points.len(), 13);

        let values = observations(adapter.as_mut(), 2);
        for key in [
            "on",
            "mode",
            "setpoint",
            "temperature",
            "heating",
            "showers",
            "remaining",
            "antilegionella",
            "max_setpoint",
            "errors",
        ] {
            assert!(
                values
                    .iter()
                    .any(|(point, _)| point == &PointId::new(&heater.id, key)),
                "missing decoded Ariston point {key}"
            );
        }
        let mode = heater.point("mode").unwrap();
        assert!(mode.access.write);
        assert!(
            matches!(&mode.kind, moli_core::Kind::Enum { values } if values.iter().any(|value| value.as_ref() == "manuel") && values.iter().any(|value| value.as_ref() == "programme"))
        );

        let setpoint = PointId::new(&heater.id, "setpoint");
        let native = adapter
            .simulate_command(&setpoint, &Value::Float(60.0))
            .unwrap();
        assert_eq!(native["action"], "temperature");
        assert_eq!(native["body"]["new"], 60.0);
        assert!(
            adapter
                .simulate_command(&setpoint, &Value::Float(81.0))
                .is_err()
        );
        assert!(
            adapter
                .simulate_command(
                    &PointId::new(&heater.id, "temperature"),
                    &Value::Float(50.0)
                )
                .is_err()
        );
        let mode = adapter
            .simulate_command(&PointId::new(&heater.id, "mode"), &Value::from("programme"))
            .unwrap();
        assert_eq!(mode["action"], "mode");
        assert_eq!(mode["body"]["new"], 5);
        assert!(
            adapter
                .simulate_command(&PointId::new(&heater.id, "mode"), &Value::from("boost"),)
                .is_err()
        );
    }
}
