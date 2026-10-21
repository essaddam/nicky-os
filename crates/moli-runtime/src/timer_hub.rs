//! Timer commands and scheduler; the authoritative owner is timers::Timers.

use super::Hub;
use crate::{CommandError, CommandRequest, TimerError};
use std::collections::BTreeMap;
use std::sync::PoisonError;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

#[cfg(test)]
#[path = "timer_hub_tests.rs"]
mod tests;
use moli_core::{
    Action, DeviceId, Event, Notice, Occurrence, Origin, Outcome, PointId, TimerField,
    TimerOperation, Value, now_ms,
};
use tokio_util::sync::CancellationToken;

impl Hub {
    pub(crate) async fn initialize_timer(
        &self,
        device: &DeviceId,
        seconds: u32,
    ) -> Result<(), TimerError> {
        let hub = self.clone();
        let device = device.clone();
        tokio::spawn(async move {
            let mut timers = hub.0.timers.lock().await;
            timers
                .initialize(&device, seconds, (hub.0.opts.timer_clock)())
                .await?;
            if let Some(projection) = timers.projection(&device) {
                hub.project_timer(&projection);
            }
            hub.0.timer_notify.notify_one();
            Ok(())
        })
        .await
        .map_err(|_| TimerError::Uncertain)?
    }

    pub(crate) async fn timer_command(&self, command: CommandRequest, cancel: CancellationToken) {
        let hub = self.clone();
        // The owner keeps the commit and its journal alive if its waiter leaves.
        let _ = tokio::spawn(async move {
            let result = hub.commit_timer(&command, &cancel).await;
            command.reply_result(result.map_err(CommandError::from));
        })
        .await;
    }

    async fn commit_timer(
        &self,
        command: &CommandRequest,
        cancel: &CancellationToken,
    ) -> Result<(), TimerError> {
        let Some(moli_core::PointSpec {
            kind: moli_core::Kind::TimerCommand { operation, .. },
            ..
        }) = command.device.point(&command.key)
        else {
            return Err(TimerError::Definition);
        };
        let seconds = match operation {
            TimerOperation::Start => match command.value {
                Value::Int(seconds) => Some(u32::try_from(seconds).map_err(|_| TimerError::Value)?),
                _ => return Err(TimerError::Value),
            },
            TimerOperation::Cancel if command.value == Value::Null => None,
            TimerOperation::Cancel => return Err(TimerError::Value),
        };
        let expected_key = match operation {
            TimerOperation::Start => "start",
            TimerOperation::Cancel => "cancel",
        };
        if command.key.as_ref() != expected_key
            || command.point != PointId::new(&command.device.id, expected_key)
        {
            return Err(TimerError::Definition);
        }
        let started = Instant::now();
        let mut timers = self.0.timers.lock().await;
        let audience = self
            .0
            .timer_audience
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&PointId::new(&command.device.id, "finished"))
            .cloned()
            .unwrap_or_default();
        let input = crate::timers::Input {
            operation: *operation,
            seconds,
            request: command.metadata.request_id.clone(),
            correlation: command.metadata.correlation.clone(),
            audience,
        };
        let result = timers
            .command(
                &command.device.id,
                input,
                (self.0.opts.timer_clock)(),
                || command.abandoned() || cancel.is_cancelled(),
            )
            .await;
        let result = match result {
            Ok(change) => {
                if change.changed {
                    if let Some(projection) = timers.projection(&command.device.id) {
                        self.project_timer(&projection);
                    }
                    self.0.timer_notify.notify_one();
                }
                change.result
            }
            Err(error) => {
                if error == TimerError::Uncertain {
                    self.invalidate_timers(&timers);
                }
                Err(error)
            }
        };
        self.record_correlated(
            command.metadata.origin,
            command.metadata.actor.clone(),
            Action::Timer {
                point: command.point.clone(),
                generation: timers.projection(&command.device.id).map(|p| p.generation),
            },
            result
                .as_ref()
                .map_or_else(|error| Outcome::Err(error.to_string()), |()| Outcome::Ok),
            started,
            command.metadata.correlation.clone(),
        );
        result
    }

    fn invalidate_timers(&self, timers: &crate::timers::Timers) {
        for projection in timers.projections() {
            self.set_state(&projection.device, "state", Value::Null);
            self.set_state(&projection.device, "remaining", Value::Null);
            self.set_availability(&projection.device, false);
        }
    }

    fn project_timer(&self, p: &crate::timers::Projection) {
        self.project_timer_value(
            &p.device,
            "state",
            if p.remaining.is_none() {
                Value::Null
            } else {
                Value::Text(p.status.as_str().into())
            },
        );
        let remaining_changed = self.project_timer_value(
            &p.device,
            "remaining",
            p.remaining
                .and_then(|seconds| i64::try_from(seconds).ok())
                .map_or(Value::Null, Value::Int),
        );
        self.project_timer_value(
            &p.device,
            "generation",
            i64::try_from(p.generation).map_or(Value::Null, Value::Int),
        );
        self.project_timer_value(
            &p.device,
            "deadline",
            p.deadline
                .and_then(|at| i64::try_from(at).ok())
                .map_or(Value::Null, Value::Int),
        );
        if p.remaining.is_none() && remaining_changed {
            self.notice(Notice {
                title: None,
                message: TimerError::Clock.to_string(),
                from: None,
                ts: now_ms(),
            });
        }
    }

    fn project_timer_value(&self, device: &DeviceId, key: &str, value: Value) -> bool {
        if self
            .state(&PointId::new(device, key))
            .is_some_and(|sample| sample.value == value)
        {
            return false;
        }
        self.set_state(device, key, value);
        true
    }

    /// One scheduler for configured timers. No polling when there is no work.
    pub async fn run_timers(&self, cancel: CancellationToken) {
        let mut previous_error = None;
        loop {
            let notified = self.0.timer_notify.notified();
            let delay = match self.tick_timers().await {
                Ok(delay) => {
                    previous_error = None;
                    delay
                }
                Err(error) => {
                    if previous_error != Some(error) {
                        self.notice(Notice {
                            title: None,
                            message: error.to_string(),
                            from: None,
                            ts: now_ms(),
                        });
                        previous_error = Some(error);
                    }
                    (error != TimerError::Uncertain).then_some(Duration::from_secs(1))
                }
            };
            tokio::select! {
                () = cancel.cancelled() => break,
                () = notified => {},
                () = async { if let Some(delay) = delay { tokio::time::sleep(delay).await; } else { std::future::pending::<()>().await; } } => {},
            }
        }
    }

    async fn tick_timers(&self) -> Result<Option<Duration>, TimerError> {
        let mut timers = self.0.timers.lock().await;
        let expired = match timers.expire().await {
            Ok(expired) => expired,
            Err(error) => {
                if error == TimerError::Uncertain {
                    self.invalidate_timers(&timers);
                }
                return Err(error);
            }
        };
        for projection in timers.projections() {
            self.project_timer(&projection);
            if expired.contains(&projection.device) {
                self.record_correlated(
                    Origin::System,
                    None,
                    Action::Timer {
                        point: PointId::new(&projection.device, "finished"),
                        generation: Some(projection.generation),
                    },
                    Outcome::Ok,
                    Instant::now(),
                    projection.correlation.clone(),
                );
            }
        }
        let ready = self.0.timer_consumer_ready.load(Ordering::Acquire);
        if ready {
            for occurrence in timers.events(&self.0.occurrence_stream)? {
                self.emit(Event::Occurrence { occurrence });
            }
        }
        Ok(timers.wait(ready))
    }

    /// The automation consumer subscribes before enabling outbox drainage.
    pub fn timer_consumer_ready(&self, ready: bool) {
        self.0.timer_consumer_ready.store(ready, Ordering::Release);
        self.0.timer_notify.notify_one();
    }

    pub fn set_timer_audience(&self, audience: BTreeMap<PointId, Vec<moli_core::TimerRecipient>>) {
        *self
            .0
            .timer_audience
            .write()
            .unwrap_or_else(PoisonError::into_inner) = audience;
    }

    pub async fn timer_recipients(
        &self,
        event: &Occurrence,
    ) -> Result<Vec<moli_core::TimerRecipient>, TimerError> {
        self.0.timers.lock().await.audience(event)
    }

    /// None for live-only sources; a timer delivery additionally carries its generation.
    pub async fn timer_delivery(
        &self,
        event: &Occurrence,
    ) -> Result<Option<CancellationToken>, TimerError> {
        let timer = event
            .point
            .split()
            .and_then(|(id, key)| {
                self.describe(&id).map(|(device, _)| {
                    device.point(key).is_some_and(|spec| {
                        matches!(
                            spec.kind,
                            moli_core::Kind::Timer {
                                field: TimerField::Expired
                            }
                        )
                    })
                })
            })
            .unwrap_or(false);
        if !timer {
            return Ok(None);
        }
        self.0.timers.lock().await.delivery(event).map(Some)
    }

    pub async fn ack_timer(&self, event: &Occurrence) -> Result<(), TimerError> {
        let hub = self.clone();
        let event = event.clone();
        tokio::spawn(async move {
            let mut timers = hub.0.timers.lock().await;
            let result = timers.ack(&event).await;
            if result == Err(TimerError::Uncertain) {
                hub.invalidate_timers(&timers);
            }
            if result == Ok(true) { tracing::info!(point = %event.point, source = %event.id.source, generation = event.id.sequence, correlation = ?event.correlation, "timer expiration durably consumed"); }
            result.map(|_| ())
        })
        .await
        .map_err(|_| TimerError::Uncertain)?
    }

    /// Serialize the final, synchronous admission against producer commits.
    /// The consumer persistence lock must already have been released.
    pub async fn with_timer_generation<T>(
        &self,
        event: &Occurrence,
        admit: impl FnOnce(CancellationToken) -> T,
    ) -> Result<T, TimerError> {
        let timers = self.0.timers.lock().await;
        let token = timers.delivery(event)?;
        if token.is_cancelled() {
            return Err(TimerError::Stale);
        }
        let result = admit(token);
        drop(timers);
        Ok(result)
    }
}
