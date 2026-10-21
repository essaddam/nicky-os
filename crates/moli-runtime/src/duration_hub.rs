//! Duration publication/checkpoints share the validated computed identities.
use super::{Action, Arc, Hub, Instant, Ordering, Origin, Outcome, PoisonError, Value};
use crate::durations::{Record, State};
use crate::{DependencyError, dependencies::Dependencies};
use moli_core::{DependencyKind, DependencyRef, DeviceId, DurationLedger, DurationStats, Unit};

impl Hub {
    pub fn duration_stats(&self, id: &DeviceId) -> Result<DurationStats, DependencyError> {
        let book = self
            .0
            .durations
            .book
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if book.fenced {
            return Err(DependencyError::Uncertain);
        }
        book.committed
            .get(id)
            .cloned()
            .ok_or(DependencyError::Unknown)
    }
    pub(super) fn fence_durations(&self) {
        self.0.durations.active.store(false, Ordering::Release);
        let mut book = self
            .0
            .durations
            .book
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        book.fenced = true;
        book.committed.clear();
        book.index.clear();
        book.devices.clear();
    }
    pub(super) async fn install_durations(
        &self,
        owner: &Dependencies,
    ) -> Result<(), DependencyError> {
        let definitions = owner.durations()?;
        if definitions.is_empty() && !self.0.durations.active.load(Ordering::Acquire) {
            return Ok(());
        }
        let mut writer = self.0.durations.writer.lock().await;
        if !writer.loaded {
            let mut state = writer
                .file
                .load::<State>()
                .await
                .map_err(|_| DependencyError::Corrupt)?
                .unwrap_or(State {
                    schema: 1,
                    records: std::collections::BTreeMap::new(),
                });
            if !state.valid() {
                self.fence_durations();
                return Err(DependencyError::Corrupt);
            }
            for record in state.records.values_mut() {
                record.ledger.resume();
            }
            self.0
                .durations
                .book
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .records = state.records;
            writer.loaded = true;
        }
        {
            // Model before book, matching source producers; no cache is observed.
            let _model = self.read();
            let mut book = self
                .0
                .durations
                .book
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if book.fenced {
                return Err(DependencyError::Uncertain);
            }
            book.epoch = book.epoch.checked_add(1).ok_or(DependencyError::Budget)?;
            book.active.clear();
            book.index.clear();
            book.devices.clear();
            let keep: std::collections::BTreeSet<_> = owner
                .calculations()?
                .into_iter()
                .filter_map(|(id, s)| {
                    matches!(s, moli_core::CalculationSpec::Duration(_)).then_some(id)
                })
                .collect();
            book.records.retain(|id, _| keep.contains(id));
            let now = book.now(Instant::now());
            for (id, (version, spec)) in definitions {
                if book.records.get(&id).is_some_and(|r| {
                    r.version > version || (r.version == version && r.spec != spec)
                }) {
                    return Err(DependencyError::Corrupt);
                }
                if book
                    .records
                    .get(&id)
                    .is_none_or(|r| r.version != version || r.spec != spec)
                {
                    book.records.insert(
                        id.clone(),
                        Record {
                            version,
                            spec: spec.clone(),
                            ledger: DurationLedger::new(now),
                        },
                    );
                }
                book.index
                    .entry(spec.source.clone())
                    .or_default()
                    .insert(id.clone());
                let (source, _) = spec.source.split().ok_or(DependencyError::Definition)?;
                book.devices.entry(source).or_default().insert(id.clone());
                book.active.insert(id);
            }
            self.0
                .durations
                .active
                .store(!book.active.is_empty(), Ordering::Release);
        }
        drop(writer);
        self.write_duration_checkpoint().await?;
        self.start_duration_clock();
        Ok(())
    }
    pub(super) async fn checkpoint_durations(&self) -> Result<(), DependencyError> {
        let _gate = self.definition_gate().lock_owned().await;
        let result = self.write_duration_checkpoint().await;
        if let Err(error) = result {
            let ids: Vec<_> = self
                .0
                .durations
                .book
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .active
                .iter()
                .cloned()
                .collect();
            for id in ids {
                self.dependency_audit(
                    DependencyRef {
                        kind: DependencyKind::Computed,
                        id,
                    },
                    moli_core::DependencyChange::Transition,
                    (Origin::System, None),
                    &Err::<crate::DependencyView, _>(error),
                    None,
                    Instant::now(),
                );
            }
        }
        result
    }
    async fn write_duration_checkpoint(&self) -> Result<(), DependencyError> {
        let writer = self.0.durations.writer.lock().await;
        let (checkpoint, epoch, active) = {
            let mut book = self
                .0
                .durations
                .book
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if book.fenced {
                return Err(DependencyError::Uncertain);
            }
            let now = book.now(Instant::now());
            book.advance(now);
            (book.state(), book.epoch, book.active.clone())
        };
        if let Err(error) = writer.file.write(&checkpoint).await {
            tracing::warn!(%error,"duration checkpoint was not acknowledged");
            self.fence_durations();
            for id in active {
                self.record_state_update(&id, "value", Value::Null, false, |_| {});
            }
            return Err(if error.commit_unknown {
                DependencyError::Uncertain
            } else {
                DependencyError::Storage
            });
        }
        // A definition may have changed while fsync ran: never project a stale head.
        let outputs = {
            let mut book = self
                .0
                .durations
                .book
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if book.fenced || book.epoch != epoch {
                return Ok(());
            }
            let mut outputs = Vec::new();
            book.committed.retain(|id, _| active.contains(id));
            for id in active {
                let record = &checkpoint.records[&id];
                let stats = record
                    .ledger
                    .stats(record.ledger.cursor(), record.spec.window_s * 1000);
                book.committed.insert(id.clone(), stats.clone());
                outputs.push((id, record.version, stats));
            }
            outputs
        };
        for (id, version, stats) in outputs {
            self.project_duration(&id, version, stats);
        }
        Ok(())
    }
    fn project_duration(&self, id: &DeviceId, version: u64, stats: DurationStats) {
        if self.describe(id).is_none_or(|(device, _)| {
            device.point("value").is_none_or(|p| {
                p.kind != moli_core::Kind::Computed || !p.access.read || p.access.write
            })
        }) {
            self.0
                .durations
                .book
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .committed
                .remove(id);
            return;
        }
        #[allow(clippy::cast_precision_loss)]
        let value = Value::Float(stats.acquired_ms as f64 / 1000.0);
        let reference = DependencyRef {
            kind: DependencyKind::Computed,
            id: id.clone(),
        };
        // Units are part of the immutable description seen by readers.
        if let Some((device, _)) = self.describe(id)
            && device
                .point("value")
                .is_some_and(|p| p.unit != Some(Unit::Second))
        {
            let mut device = (*device).clone();
            if let Some(point) = device.points.iter_mut().find(|p| p.key.as_ref() == "value") {
                point.unit = Some(Unit::Second);
            }
            self.upsert_device(device);
        }
        self.record_state_update(id, "value", value.clone(), false, |model| {
            model.dependency_heads.insert(reference, version);
        });
        self.record_correlated(
            Origin::System,
            None,
            Action::Computed {
                point: moli_core::PointId::new(id, "value"),
                version,
                value,
                source_ts: Some(stats.as_of_ms),
                unit: Some(Unit::Second),
                valid: u8::from(stats.observing),
                coverage: Some(stats),
            },
            Outcome::Ok,
            Instant::now(),
            None,
        );
    }
    fn start_duration_clock(&self) {
        if !self.0.durations.active.load(Ordering::Acquire)
            || self.0.durations.started.swap(true, Ordering::AcqRel)
        {
            return;
        }
        let weak = Arc::downgrade(&self.0);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                let Some(inner) = weak.upgrade() else {
                    return;
                };
                let hub = Hub(inner);
                if !hub.0.durations.active.load(Ordering::Acquire) {
                    hub.0.durations.started.store(false, Ordering::Release);
                    if hub.0.durations.active.load(Ordering::Acquire)
                        && !hub.0.durations.started.swap(true, Ordering::AcqRel)
                    {
                        continue;
                    }
                    return;
                }
                let _ = hub.checkpoint_durations().await;
            }
        });
    }
}
