//! Hub publication follows the authoritative temporal setting commit.
use super::Hub;
use crate::{CommandError, CommandRequest, TemporalError};
use moli_core::{Action, DeviceId, Kind, Outcome, PointId, TemporalSpec, Value};
use std::time::Instant;
use tokio_util::sync::CancellationToken;

impl Hub {
    pub(crate) async fn initialize_temporal(
        &self,
        id: &DeviceId,
        spec: TemporalSpec,
        initial: Value,
    ) -> Result<(), TemporalError> {
        let hub = self.clone();
        let id = id.clone();
        tokio::spawn(async move {
            let mut owner = hub.0.temporals.lock().await;
            let result = owner.initialize(&id, spec, initial).await;
            if result == Err(TemporalError::Uncertain) {
                hub.invalidate_temporals(&owner);
            }
            let value = result?;
            hub.set_state(&id, "value", value);
            Ok(())
        })
        .await
        .map_err(|_| TemporalError::Uncertain)?
    }
    pub(crate) async fn temporal_command(
        &self,
        command: CommandRequest,
        cancel: CancellationToken,
    ) {
        let hub = self.clone();
        let _ = tokio::spawn(async move {
            let result = hub.commit_temporal(&command, &cancel).await;
            command.reply_result(result.map_err(CommandError::from));
        })
        .await;
    }
    fn invalidate_temporals(&self, owner: &crate::temporal::Temporals) {
        for id in owner.devices() {
            self.set_state(id, "value", Value::Null);
            self.set_availability(id, false);
        }
    }
    async fn commit_temporal(
        &self,
        command: &CommandRequest,
        cancel: &CancellationToken,
    ) -> Result<(), TemporalError> {
        let Some(moli_core::PointSpec {
            kind: Kind::Temporal { spec },
            ..
        }) = command.device.point(&command.key)
        else {
            return Err(TemporalError::Definition);
        };
        if command.key.as_ref() != "value"
            || command.point != PointId::new(&command.device.id, "value")
        {
            return Err(TemporalError::Definition);
        }
        let started = Instant::now();
        let mut owner = self.0.temporals.lock().await;
        let result = owner
            .assign(&command.device.id, spec, &command.value, || {
                command.abandoned() || cancel.is_cancelled()
            })
            .await;
        match &result {
            Ok(value) => self.set_state(&command.device.id, "value", value.clone()),
            Err(TemporalError::Uncertain) => {
                self.invalidate_temporals(&owner);
            }
            Err(_) => {}
        }
        self.record_correlated(
            command.metadata.origin,
            command.metadata.actor.clone(),
            Action::Temporal {
                point: command.point.clone(),
            },
            result
                .as_ref()
                .map_or_else(|e| Outcome::Err(e.to_string()), |_| Outcome::Ok),
            started,
            command.metadata.correlation.clone(),
        );
        result.map(|_| ())
    }
}

#[cfg(test)]
#[path = "temporal_hub_tests.rs"]
mod tests;
