//! Calculations project from one coherent model view. Only definitions persist;
//! observation clocks are monotonic in process, absolute on cache restoration.
use super::{
    Action, DeviceId, Event, Hub, Instant, Model, Origin, Outcome, PointId, PoisonError, Sample,
    Value,
};
use crate::computed::{Evaluation, Projection};
use crate::{DependencyDefinition, DependencyError, DependencyView};
use moli_core::{AggregateSource, AggregateSpec, DependencyKind, DependencyRef, Kind};

impl Hub {
    pub async fn initialize_computed(
        &self,
        device: &DeviceId,
        spec: AggregateSpec,
    ) -> Result<(), DependencyError> {
        let reference = DependencyRef {
            kind: DependencyKind::Computed,
            id: device.clone(),
        };
        if self
            .describe(device)
            .and_then(|(d, _)| d.point("value").map(|p| p.kind.clone()))
            != Some(Kind::Computed)
        {
            return Err(DependencyError::Definition);
        }
        let hub = self.clone();
        tokio::spawn(async move {
            let _gate = hub.definition_gate().lock_owned().await;
            let started = Instant::now();
            let mut owner = hub.0.dependencies.lock().await;
            let result = owner
                .initialize(
                    reference.clone(),
                    DependencyDefinition::Aggregate(spec),
                    jiff::Timestamp::now(),
                )
                .await;
            if let Ok(view) = &result {
                hub.rebuild_computations(&owner)?;
                hub.emit(Event::DependencyChanged {
                    reference: view.reference.clone(),
                });
            } else if matches!(
                result,
                Err(DependencyError::Uncertain | DependencyError::Corrupt)
            ) {
                hub.dependency_fence();
            }
            hub.dependency_audit(
                reference,
                moli_core::DependencyChange::Initialized,
                (Origin::System, None),
                &result,
                None,
                started,
            );
            result.map(|_| ())
        })
        .await
        .map_err(|_| DependencyError::Uncertain)??;
        Ok(())
    }
    pub(super) fn validate_computed(
        &self,
        owner: &crate::dependencies::Dependencies,
        view: &DependencyView,
    ) -> Result<(), DependencyError> {
        let DependencyDefinition::Aggregate(spec) = &view.versions.draft().definition else {
            return Ok(());
        };
        let mut prospective = owner.aggregates(false)?;
        prospective.insert(view.reference.id.clone(), spec.clone());
        moli_core::aggregate_order(&prospective).map_err(crate::dependencies::graph_error)?;
        let model = self.read();
        for source in &spec.sources {
            let (device, key) = source.split().ok_or(DependencyError::Units)?;
            let point = model
                .devices
                .get(&device)
                .and_then(|d| d.point(key))
                .ok_or(DependencyError::Units)?;
            if !point.access.read
                || !matches!(
                    point.kind,
                    Kind::Numeric { .. } | Kind::Counter { .. } | Kind::Computed
                )
            {
                return Err(DependencyError::Units);
            }
            let unit = if point.kind == Kind::Computed {
                let dependency = DependencyRef {
                    kind: DependencyKind::Computed,
                    id: device.clone(),
                };
                if owner.view(&dependency)?.versions.current().is_none() {
                    return Err(DependencyError::Units);
                }
                prospective
                    .get(&device)
                    .ok_or(DependencyError::Units)?
                    .unit
                    .as_ref()
            } else {
                point.unit.as_ref()
            };
            if moli_core::unit_ratio(unit, spec.unit.as_ref()).is_none() {
                return Err(DependencyError::Units);
            }
        }
        Ok(())
    }
    pub(super) fn rebuild_computations(
        &self,
        owner: &crate::dependencies::Dependencies,
    ) -> Result<(), DependencyError> {
        self.drive_computations(Some(owner))?;
        self.start_computed_clock();
        Ok(())
    }
    pub(super) fn run_computations(&self) -> Result<(), DependencyError> {
        self.drive_computations(None)
    }
    fn drive_computations(
        &self,
        owner: Option<&crate::dependencies::Dependencies>,
    ) -> Result<(), DependencyError> {
        if owner.is_none() && !self.0.computed.active() {
            return Ok(());
        }
        let install = owner
            .map(|owner| Ok::<_, DependencyError>((owner.aggregates(true)?, owner.list()?)))
            .transpose()?;
        let started = Instant::now();
        let mut engine = self
            .0
            .computed
            .engine
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut model = self.write();
        let mut signals = self
            .0
            .computed
            .signals
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some((definitions, views)) = install {
            for view in views
                .iter()
                .filter(|v| v.reference.kind == DependencyKind::Computed)
            {
                if let Some(current) = view.versions.current() {
                    model
                        .dependency_heads
                        .insert(view.reference.clone(), current.version);
                } else {
                    model.dependency_heads.remove(&view.reference);
                }
                model
                    .states
                    .entry(view.reference.id.clone())
                    .or_default()
                    .entry("value".into())
                    .or_insert(Sample {
                        value: Value::Null,
                        ts: moli_core::now_ms(),
                    });
            }
            self.0
                .computed
                .install(&mut engine, &mut signals, definitions)?;
        }
        let mut projection = ProjectionModel {
            model: &mut model,
            events: Vec::new(),
            changed: Vec::new(),
        };
        engine.propagate(
            &mut signals,
            Instant::now(),
            moli_core::now_ms(),
            &mut projection,
        )?;
        let events = projection.events;
        let changed = projection.changed;
        drop(signals);
        drop(model);
        drop(engine);
        for event in events {
            self.emit(event);
        }
        for (evaluation, version) in changed {
            self.record_correlated(
                Origin::System,
                None,
                Action::Computed {
                    point: PointId::new(&evaluation.id, "value"),
                    version,
                    value: evaluation
                        .sample
                        .as_ref()
                        .map_or(Value::Null, |s| s.value.clone()),
                    source_ts: evaluation.sample.as_ref().map(|s| s.ts),
                    unit: evaluation.unit,
                    valid: u8::try_from(evaluation.valid).unwrap_or(0),
                },
                Outcome::Ok,
                started,
                None,
            );
        }
        Ok(())
    }
    pub(super) fn computed_delay(&self, wall: u64) -> Option<std::time::Duration> {
        self.0
            .computed
            .engine
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .next(Instant::now(), wall)
    }
}

struct ProjectionModel<'a> {
    model: &'a mut Model,
    events: Vec<Event>,
    changed: Vec<(Evaluation, u64)>,
}
impl Projection for ProjectionModel<'_> {
    fn source(&self, point: &PointId) -> AggregateSource {
        let described = point.split().and_then(|(id, key)| {
            self.model
                .devices
                .get(&id)
                .and_then(|d| d.point(key))
                .map(|spec| (id, spec))
        });
        let Some((device, spec)) = described else {
            return AggregateSource {
                sample: None,
                unit: None,
                online: false,
                numeric: false,
            };
        };
        AggregateSource {
            sample: point
                .split()
                .and_then(|(id, key)| self.model.states.get(&id)?.get(key).cloned()),
            unit: spec.unit.clone(),
            online: self.model.availability.get(&device) != Some(&false),
            numeric: spec.access.read
                && matches!(
                    spec.kind,
                    Kind::Numeric { .. } | Kind::Counter { .. } | Kind::Computed
                ),
        }
    }
    fn project(&mut self, evaluation: &Evaluation) {
        let Some(device) = self.model.devices.get(&evaluation.id) else {
            return;
        };
        if device
            .point("value")
            .is_none_or(|p| p.kind != Kind::Computed)
        {
            return;
        }
        if device
            .point("value")
            .is_some_and(|p| p.unit != evaluation.unit)
        {
            let mut next = (**device).clone();
            next.points
                .iter_mut()
                .find(|p| p.key.as_ref() == "value")
                .unwrap()
                .unit
                .clone_from(&evaluation.unit);
            let next = std::sync::Arc::new(next);
            self.model
                .devices
                .insert(evaluation.id.clone(), next.clone());
            self.events.push(Event::DeviceUpserted { device: next });
        }
        let value = evaluation
            .sample
            .as_ref()
            .map_or(Value::Null, |s| s.value.clone());
        let states = self.model.states.entry(evaluation.id.clone()).or_default();
        let changed = states
            .get("value")
            .is_none_or(|sample| sample.value != value);
        let ts = evaluation
            .sample
            .as_ref()
            .map_or_else(moli_core::now_ms, |s| s.ts);
        states.insert(
            "value".into(),
            Sample {
                value: value.clone(),
                ts,
            },
        );
        if changed {
            self.events.push(Event::State {
                point: PointId::new(&evaluation.id, "value"),
                value,
                ts: moli_core::now_ms(),
                source_ts: evaluation.sample.as_ref().map(|s| s.ts),
            });
            let reference = DependencyRef {
                kind: DependencyKind::Computed,
                id: evaluation.id.clone(),
            };
            if let Some(version) = self.model.dependency_heads.get(&reference) {
                self.changed.push((
                    Evaluation {
                        id: evaluation.id.clone(),
                        sample: evaluation.sample.clone(),
                        unit: evaluation.unit.clone(),
                        valid: evaluation.valid,
                    },
                    *version,
                ));
            }
        }
    }
}
