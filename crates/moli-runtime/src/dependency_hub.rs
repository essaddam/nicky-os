//! Trusted hub boundary for executable definitions. Network callers cannot set
//! authority: surfaces must derive it from their existing authenticated session.
use super::{
    Action, Arc, BTreeMap, DeviceId, Duration, Event, Hub, Instant, Ordering, Origin, Outcome,
    PoisonError, Value,
};
use crate::{
    DependencyConsumer, DependencyDefinition, DependencyError, DependencyPreview, DependencyView,
};
use moli_core::{DependencyChange, DependencyKind, DependencyRef, WeeklySchedule};

impl Hub {
    /// Graph edits and dependency publications share this gate. Acquire it before
    /// any graph-list/store lock and retain it through persistence and indexing.
    pub fn definition_gate(&self) -> Arc<tokio::sync::Mutex<()>> {
        self.0.definition_gate.clone()
    }

    /// Called by the automation owner while it holds its definition-list guard.
    /// A consumer source also supplies a fresh snapshot at publication.
    pub fn set_dependency_consumers(
        &self,
        consumers: BTreeMap<DependencyRef, Vec<DependencyConsumer>>,
    ) {
        *self
            .0
            .dependency_consumers
            .write()
            .unwrap_or_else(PoisonError::into_inner) = consumers;
    }
    pub fn set_dependency_consumer_source(&self, source: Arc<dyn crate::DependencyConsumerSource>) {
        *self
            .0
            .dependency_consumer_source
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Some(source);
    }
    pub fn dependency_ready(&self, reference: &DependencyRef) -> bool {
        self.read().dependency_heads.contains_key(reference)
    }

    pub(super) fn dependency_project(&self, view: &DependencyView, value: Option<bool>) {
        let reference = &view.reference;
        let version = view.versions.current().map(|v| v.version);
        if reference.kind == DependencyKind::Scene {
            let mut model = self.write();
            if let Some(current) = view.versions.current()
                && let DependencyDefinition::Scene(spec) = &current.definition
            {
                if let Some(scope) = &view.scope {
                    model
                        .scene_specs
                        .insert(reference.id.clone(), (spec.clone(), scope.clone()));
                } else {
                    model.scene_specs.remove(&reference.id);
                }
            } else {
                model.scene_specs.remove(&reference.id);
            }
            if let Some(version) = version {
                model.dependency_heads.insert(reference.clone(), version);
            } else {
                model.dependency_heads.remove(reference);
            }
            drop(model);
            self.emit(Event::DependencyChanged {
                reference: reference.clone(),
            });
            return;
        }
        self.record_state_update(
            &reference.id,
            "active",
            value.map_or(Value::Null, Value::Bool),
            false,
            |model| {
                if let Some(version) = version {
                    model.dependency_heads.insert(reference.clone(), version);
                } else {
                    model.dependency_heads.remove(reference);
                }
            },
        );
        self.emit(Event::DependencyChanged {
            reference: reference.clone(),
        });
    }
    pub(super) fn dependency_fence(&self) {
        self.fence_scenes();
        let references: Vec<_> = self.read().dependency_heads.keys().cloned().collect();
        for reference in references {
            if reference.kind == DependencyKind::Scene {
                {
                    let mut model = self.write();
                    model.dependency_heads.remove(&reference);
                    model.scene_specs.remove(&reference.id);
                }
                self.emit(Event::DependencyChanged { reference });
                continue;
            }
            self.record_state_update(&reference.id, "active", Value::Null, false, |model| {
                model.dependency_heads.remove(&reference);
            });
            self.emit(Event::DependencyChanged { reference });
        }
    }
    pub(super) fn dependency_audit(
        &self,
        reference: DependencyRef,
        operation: DependencyChange,
        (origin, actor): (Origin, Option<String>),
        result: &Result<DependencyView, DependencyError>,
        active: Option<bool>,
        started: Instant,
    ) {
        self.record_correlated(
            origin,
            actor,
            Action::Dependency {
                reference,
                operation,
                version: result
                    .as_ref()
                    .ok()
                    .map(|v| {
                        if operation == DependencyChange::Drafted {
                            v.versions.draft().version
                        } else {
                            v.versions.current().map_or(0, |v| v.version)
                        }
                    })
                    .filter(|v| *v != 0),
                active,
            },
            result
                .as_ref()
                .map_or_else(|e| Outcome::Err(e.to_string()), |_| Outcome::Ok),
            started,
            None,
        );
    }
    pub async fn initialize_schedule(
        &self,
        device: &DeviceId,
        plan: WeeklySchedule,
    ) -> Result<(), DependencyError> {
        let reference = DependencyRef {
            kind: DependencyKind::Schedule,
            id: device.clone(),
        };
        if self
            .describe(device)
            .and_then(|(d, _)| d.point("active").map(|p| p.kind.clone()))
            != Some(moli_core::Kind::Schedule)
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
                    DependencyDefinition::Schedule(plan),
                    jiff::Timestamp::now(),
                )
                .await;
            let active = owner.projection(&reference).ok().flatten();
            if let Ok(view) = &result {
                hub.dependency_project(view, active);
            } else if matches!(
                result,
                Err(DependencyError::Uncertain | DependencyError::Corrupt)
            ) {
                hub.dependency_fence();
            }
            hub.dependency_audit(
                reference,
                DependencyChange::Initialized,
                (Origin::System, None),
                &result,
                active,
                started,
            );
            result.map(|_| ())
        })
        .await
        .map_err(|_| DependencyError::Uncertain)??;
        self.start_dependency_clock();
        Ok(())
    }
    pub async fn dependencies(&self) -> Result<Vec<DependencyView>, DependencyError> {
        self.0.dependencies.lock().await.list()
    }
    pub async fn dependency(
        &self,
        reference: &DependencyRef,
    ) -> Result<DependencyView, DependencyError> {
        self.0.dependencies.lock().await.view(reference)
    }

    pub async fn stage_dependency(
        &self,
        reference: DependencyRef,
        expected: u64,
        definition: DependencyDefinition,
        origin: Origin,
        actor: Option<String>,
    ) -> Result<DependencyView, DependencyError> {
        let hub = self.clone();
        tokio::spawn(async move {
            let started = Instant::now();
            let mut owner = hub.0.dependencies.lock().await;
            let result = owner.stage(&reference, expected, definition).await;
            if matches!(result, Err(DependencyError::Uncertain)) {
                hub.dependency_fence();
            }
            hub.dependency_audit(
                reference,
                DependencyChange::Drafted,
                (origin, actor),
                &result,
                None,
                started,
            );
            result
        })
        .await
        .map_err(|_| DependencyError::Uncertain)?
    }
    fn dependency_preview_with(
        &self,
        view: &DependencyView,
    ) -> Result<DependencyPreview, DependencyError> {
        let mut consumers = self
            .0
            .dependency_consumers
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&view.reference)
            .cloned()
            .unwrap_or_default();
        if let Some(source) = self
            .0
            .dependency_consumer_source
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            consumers = source.consumers(&view.reference)?;
        }
        consumers.sort();
        consumers.dedup();
        let scene = match &view.versions.draft().definition {
            DependencyDefinition::Scene(scene) => Some(self.scene_scope(scene)?),
            DependencyDefinition::Schedule(_) => None,
        };
        let fingerprint = crate::dependencies::fingerprint(&(view, &consumers, &scene));
        Ok(DependencyPreview {
            reference: view.reference.clone(),
            versions: view.versions.versions(),
            consumers,
            scene,
            fingerprint,
        })
    }
    pub async fn preview_dependency(
        &self,
        reference: &DependencyRef,
    ) -> Result<DependencyPreview, DependencyError> {
        Ok(self.inspect_dependency(reference).await?.1)
    }
    /// Both heads and their preview from one consistent snapshot.
    pub async fn inspect_dependency(
        &self,
        reference: &DependencyRef,
    ) -> Result<(DependencyView, DependencyPreview), DependencyError> {
        let _gate = self.definition_gate().lock_owned().await;
        let owner = self.0.dependencies.lock().await;
        let view = owner.view(reference)?;
        let preview = self.dependency_preview_with(&view)?;
        Ok((view, preview))
    }
    /// `human` is supplied only by trusted session-aware backend code, as with
    /// automation Actor. It is never a field accepted from a client body.
    pub async fn publish_dependency(
        &self,
        preview: DependencyPreview,
        human: bool,
        origin: Origin,
        actor: Option<String>,
    ) -> Result<DependencyView, DependencyError> {
        let hub = self.clone();
        tokio::spawn(async move {
            let started = Instant::now();
            let _gate = hub.definition_gate().lock_owned().await;
            let mut owner = hub.0.dependencies.lock().await;
            let reference = preview.reference.clone();
            let result = async {
                if !human {
                    return Err(DependencyError::Human);
                }
                let current = owner.view(&reference)?;
                if hub.dependency_preview_with(&current)? != preview {
                    return Err(DependencyError::Conflict);
                }
                owner
                    .publish(
                        &reference,
                        preview.versions,
                        jiff::Timestamp::now(),
                        preview.scene.clone(),
                    )
                    .await
            }
            .await;
            let active = owner.projection(&reference).ok().flatten();
            if let Ok(view) = &result {
                hub.dependency_project(view, active);
                hub.0.dependency_wake.notify_one();
            } else if matches!(result, Err(DependencyError::Uncertain)) {
                hub.dependency_fence();
            }
            hub.dependency_audit(
                reference,
                DependencyChange::Published,
                (origin, actor),
                &result,
                active,
                started,
            );
            result
        })
        .await
        .map_err(|_| DependencyError::Uncertain)?
    }
    pub(super) async fn dependency_tick(
        &self,
        owner: &mut crate::dependencies::Dependencies,
        now: jiff::Timestamp,
        jumped: bool,
    ) -> bool {
        let started = Instant::now();
        let due = owner.due(now, jumped);
        match owner.tick(now, jumped).await {
            Ok(changes) => {
                for (reference, active) in changes {
                    if let Ok(view) = owner.view(&reference) {
                        // Version unchanged: state transition only, no graph reindex.
                        self.set_state(&reference.id, "active", Value::Bool(active));
                        self.dependency_audit(
                            reference,
                            DependencyChange::Transition,
                            (Origin::System, None),
                            &Ok(view),
                            Some(active),
                            started,
                        );
                    }
                }
                false
            }
            Err(error) => {
                if error == DependencyError::Storage {
                    owner.suspend_projection();
                }
                self.dependency_fence();
                for reference in due {
                    self.dependency_audit(
                        reference,
                        DependencyChange::Transition,
                        (Origin::System, None),
                        &Err(error),
                        None,
                        started,
                    );
                }
                tracing::warn!(%error, "schedule projection suspended");
                true
            }
        }
    }

    fn start_dependency_clock(&self) {
        self.0.dependency_wake.notify_one();
        if self.0.dependency_started.swap(true, Ordering::AcqRel) {
            return;
        }
        let weak = Arc::downgrade(&self.0);
        let wake = self.0.dependency_wake.clone();
        tokio::spawn(async move {
            let mut anchor = (Instant::now(), jiff::Timestamp::now());
            loop {
                let Some(inner) = weak.upgrade() else { return };
                let hub = Hub(inner);
                let mut owner = hub.0.dependencies.lock().await;
                let now = jiff::Timestamp::now();
                let expected = anchor.1.as_millisecond().saturating_add(
                    i64::try_from(anchor.0.elapsed().as_millis()).unwrap_or(i64::MAX),
                );
                let jumped = now.as_millisecond().abs_diff(expected) > 1000;
                let failed = hub.dependency_tick(&mut owner, now, jumped).await;
                anchor = (Instant::now(), now);
                let next = owner.next();
                drop(owner);
                drop(hub);
                // One heartbeat only for current schedules. It detects wall-clock
                // jumps; normal work visits due definitions only. No current = idle.
                let delay = next.map(|next| {
                    Duration::from_millis(
                        u64::try_from(next.as_millisecond().saturating_sub(now.as_millisecond()))
                            .unwrap_or(0)
                            .clamp(if failed { 1000 } else { 1 }, 60_000),
                    )
                });
                if let Some(delay) = delay {
                    tokio::select! { () = wake.notified() => {}, () = tokio::time::sleep(delay) => {} }
                } else {
                    wake.notified().await;
                }
            }
        });
    }
}
