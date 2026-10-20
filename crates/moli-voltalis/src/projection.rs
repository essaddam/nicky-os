//! Private composition state for complete PUTs. Never published as telemetry.
use crate::model::{Appliance, Role};
use moli_core::DeviceId;
use serde_json::Value as Json;
use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Debug)]
struct Pending {
    appliance: Option<Appliance>,
    until: Instant,
}

#[derive(Debug, Default)]
pub struct Projection {
    pending: HashMap<DeviceId, Pending>,
    pub revision: u64,
}

impl Projection {
    /// None is deliberately ambiguous after an uncertain emission: require
    /// a new full read before composing another complete setting.
    pub fn source(&self, id: &DeviceId, observed: &Appliance) -> Option<Appliance> {
        let mut appliance = observed.clone();
        if let Some(pending) = self.pending.get(id) {
            let expected = pending
                .appliance
                .as_ref()
                .filter(|_| Instant::now() < pending.until)?;
            appliance.programming.clone_from(&expected.programming);
        }
        Some(appliance)
    }

    pub fn accepted(&mut self, id: &DeviceId, observed: &Appliance, body: &Json, within: Duration) {
        let mut appliance = self
            .source(id, observed)
            .unwrap_or_else(|| observed.clone());
        let p = &mut appliance.programming;
        p.prog_type = if body["enabled"] == true {
            "MANUAL"
        } else {
            "USER"
        }
        .into();
        p.is_on = body["isOn"] == true;
        p.mode = body["mode"].as_str().unwrap_or_default().into();
        p.target = body["temperatureTarget"].as_f64();
        p.until_further_notice = body["untilFurtherNotice"] == true;
        self.revision += 1;
        self.pending.insert(
            id.clone(),
            Pending {
                appliance: Some(appliance),
                until: Instant::now() + within,
            },
        );
    }

    pub fn uncertain(&mut self, id: DeviceId) {
        self.revision += 1;
        self.pending.insert(
            id,
            Pending {
                appliance: None,
                until: Instant::now(),
            },
        );
    }

    pub fn invalidate_heaters(&mut self, appliances: &HashMap<DeviceId, Appliance>) {
        for (id, appliance) in appliances {
            if appliance.role == Role::Heater {
                self.uncertain(id.clone());
            }
        }
    }

    /// Only a full read started after the last mutation reconciles an
    /// uncertain result. Pending accepted settings survive lagging reads.
    pub fn observe(&mut self, observed: &HashMap<DeviceId, Appliance>, revision: u64, full: bool) {
        if revision != self.revision {
            return;
        }
        let now = Instant::now();
        self.pending.retain(|id, pending| {
            let Some(actual) = observed.get(id) else {
                return false;
            };
            match &pending.appliance {
                Some(expected) => {
                    let a = &actual.programming;
                    let e = &expected.programming;
                    let matches = a.prog_type == e.prog_type
                        && a.is_on == e.is_on
                        && a.mode == e.mode
                        && a.target == e.target;
                    if matches {
                        return false;
                    }
                    if now >= pending.until {
                        pending.appliance = None;
                    }
                    true
                }
                None => !full,
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model;
    use moli_core::{InstanceId, Value};

    #[test]
    fn lagging_reads_do_not_erase_an_ack_and_uncertainty_requires_a_new_full_read() {
        let json: Json = serde_json::from_str(include_str!(
            "../../../integrations/voltalis/fixtures/managed-appliance.json"
        ))
        .unwrap();
        let observed = model::appliances(&json).remove(0);
        let id = observed.device(&InstanceId::from("chauffage")).id;
        let appliances = HashMap::from([(id.clone(), observed.clone())]);
        let mut projection = Projection::default();
        let body = observed
            .order(300_001, "setpoint", &Value::Float(22.0))
            .unwrap();
        projection.accepted(&id, &observed, &body, Duration::from_secs(120));
        projection.observe(&appliances, 0, true);
        assert_eq!(
            projection
                .source(&id, &observed)
                .unwrap()
                .programming
                .target,
            Some(22.0)
        );
        projection.observe(&appliances, projection.revision, false);
        assert_eq!(
            projection
                .source(&id, &observed)
                .unwrap()
                .programming
                .target,
            Some(22.0)
        );
        projection.uncertain(id.clone());
        let revision = projection.revision;
        for (read, full) in [(revision - 1, true), (revision, false)] {
            projection.observe(&appliances, read, full);
            assert!(projection.source(&id, &observed).is_none());
        }
        projection.observe(&appliances, revision, true);
        assert_eq!(projection.source(&id, &observed), Some(observed.clone()));
        projection.invalidate_heaters(&appliances);
        assert!(projection.source(&id, &observed).is_none());
    }
}
