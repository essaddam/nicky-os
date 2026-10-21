use super::{Hub, Instant, Origin, PoisonError, Value};
use crate::scenes::{self, Active};
use crate::{
    CommandPermit, DependencyDefinition, DependencyError, DependencyView, SceneError, ScenePreview,
    SceneRun, SceneScope, SceneStatus, SceneTargetResult, TargetStatus,
};
use moli_core::{DependencyKind, DependencyRef, DeviceId, PointId, SceneSpec};
use std::collections::BTreeSet;

use tokio_util::sync::CancellationToken;

impl Hub {
    pub(super) fn scene_observe(&self, point: &PointId, value: &Value, at: Instant) {
        if self.0.scenes_active.load(super::Ordering::Acquire) {
            self.0
                .scenes
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .observe(point, value, at);
        }
    }
    pub(super) fn scene_order(&self, device: &moli_core::Device, correlation: Option<&str>) {
        if self.0.scene_leases_active.load(super::Ordering::Acquire) {
            let mut devices = device.members.clone();
            devices.push(device.id.clone());
            self.scene_command(&devices, correlation);
        }
    }
    pub(super) fn scene_command(&self, devices: &[DeviceId], correlation: Option<&str>) {
        if self.0.scene_leases_active.load(super::Ordering::Acquire) {
            let mut book = self.0.scenes.lock().unwrap_or_else(PoisonError::into_inner);
            book.command(devices, correlation);
            self.0
                .scene_leases_active
                .store(book.has_leases(), super::Ordering::Release);
        }
    }
    pub(super) fn scene_changed(&self, device: &DeviceId) {
        self.scene_command(std::slice::from_ref(device), None);
    }
    pub(super) fn fence_scenes(&self) {
        self.0
            .scenes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .fence();
        self.0
            .scene_leases_active
            .store(false, super::Ordering::Release);
    }
    pub fn current_scene(&self, id: &DeviceId) -> Option<SceneSpec> {
        let (spec, scope) = self.read().scene_specs.get(id)?.clone();
        (self.scene_scope(&spec).ok().as_ref() == Some(&scope)).then_some(spec)
    }

    pub async fn initialize_scene(
        &self,
        id: DeviceId,
        definition: SceneSpec,
    ) -> Result<(), DependencyError> {
        let hub = self.clone();
        tokio::spawn(async move {
            let _gate = hub.definition_gate().lock_owned().await;
            let mut owner = hub.0.dependencies.lock().await;
            let reference = DependencyRef {
                kind: DependencyKind::Scene,
                id,
            };
            let started = Instant::now();
            let result = owner
                .initialize(
                    reference.clone(),
                    DependencyDefinition::Scene(definition),
                    jiff::Timestamp::now(),
                )
                .await;
            if let Ok(view) = &result {
                hub.dependency_project(view, None);
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
        .map_err(|_| DependencyError::Uncertain)?
    }

    pub fn scene_scope(&self, spec: &SceneSpec) -> Result<SceneScope, DependencyError> {
        let spec = spec
            .clone()
            .normalized()
            .map_err(|_| DependencyError::Definition)?;
        let model = self.read();
        let mut targets = Vec::new();
        let mut count = 0;
        for target in spec.targets {
            let (id, key) = target.point.split().ok_or(DependencyError::Definition)?;
            let device = model.devices.get(&id).ok_or(DependencyError::Definition)?;
            let point = device.point(key).ok_or(DependencyError::Definition)?;
            // A scene applies state, never a press, increment, announcement or action.
            if !point.access.write
                || !point.access.read
                || point.semantic == moli_core::Semantic::Control
                || point.kind.is_operation()
                || point.kind.is_occurrence()
                || !matches!(
                    point.kind,
                    moli_core::Kind::Binary
                        | moli_core::Kind::Numeric { .. }
                        | moli_core::Kind::Enum { .. }
                        | moli_core::Kind::Text
                        | moli_core::Kind::Temporal { .. }
                )
            {
                return Err(DependencyError::Definition);
            }
            let value = crate::validate_point_value(&point.kind, &Value::from_json(&target.value))
                .map_err(|_| DependencyError::Definition)?;
            let mut descriptors = vec![(
                device.clone(),
                model.labels.get(&id).cloned().unwrap_or_default(),
            )];
            let mut seen = BTreeSet::from([id.clone()]);
            for member in &device.members {
                let found = model
                    .devices
                    .get(member)
                    .ok_or(DependencyError::Definition)?;
                // The guard's declared membership is one level. Do not conceal nested effects.
                if !found.members.is_empty() || !seen.insert(member.clone()) {
                    return Err(DependencyError::Definition);
                }
                descriptors.push((
                    found.clone(),
                    model.labels.get(member).cloned().unwrap_or_default(),
                ));
            }
            count += descriptors.len();
            if count > 128 {
                return Err(DependencyError::Definition);
            }
            let mut rooms: Vec<String> = descriptors
                .iter()
                .flat_map(|(d, l)| {
                    l.room
                        .iter()
                        .cloned()
                        .chain(d.native_room.iter().map(ToString::to_string))
                })
                .collect();
            rooms.sort();
            rooms.dedup();
            if rooms.iter().any(|room| room.len() > 240) {
                return Err(DependencyError::Definition);
            }
            targets.push(scenes::scope_target(
                &target.point,
                &value,
                &descriptors,
                rooms,
                point,
            ));
        }
        let scope = SceneScope { targets };
        if serde_json::to_vec(&scope).map_or(true, |bytes| bytes.len() > 64 * 1024) {
            return Err(DependencyError::Definition);
        }
        Ok(scope)
    }
    pub async fn preview_scene(&self, id: &DeviceId) -> Result<ScenePreview, SceneError> {
        let reference = DependencyRef {
            kind: DependencyKind::Scene,
            id: id.clone(),
        };
        let owner = self.0.dependencies.lock().await;
        let view = owner.view(&reference).map_err(scene_dependency_error)?;
        self.scene_preview_with(&view)
    }
    fn scene_preview_with(&self, view: &DependencyView) -> Result<ScenePreview, SceneError> {
        let current = view.versions.current().ok_or(SceneError::Conflict)?;
        let DependencyDefinition::Scene(spec) = &current.definition else {
            return Err(SceneError::Definition);
        };
        let scope = self.scene_scope(spec).map_err(scene_dependency_error)?;
        if view.scope.as_ref() != Some(&scope) {
            return Err(SceneError::Conflict);
        }
        let fingerprint = crate::dependencies::fingerprint(&(&view.reference, current, &scope));
        Ok(ScenePreview {
            id: view.reference.id.clone(),
            version: current.version,
            scope,
            fingerprint,
        })
    }
    /// The expected preview is server-produced and checked again before admission.
    /// The optional permit is trusted internal authority, never a wire field.
    pub async fn start_scene(
        &self,
        expected: ScenePreview,
        origin: Origin,
        actor: Option<String>,
        parent: Option<CommandPermit>,
    ) -> Result<SceneRun, SceneError> {
        self.start_scene_correlated(expected, origin, actor, parent, None)
            .await
    }
    pub async fn start_scene_correlated(
        &self,
        expected: ScenePreview,
        origin: Origin,
        actor: Option<String>,
        parent: Option<CommandPermit>,
        parent_correlation: Option<String>,
    ) -> Result<SceneRun, SceneError> {
        let _gate = self.definition_gate().lock_owned().await;
        let owner = self.0.dependencies.lock().await;
        let reference = DependencyRef {
            kind: DependencyKind::Scene,
            id: expected.id.clone(),
        };
        let view = owner.view(&reference).map_err(scene_dependency_error)?;
        if self.scene_preview_with(&view)? != expected {
            return Err(SceneError::Conflict);
        }
        let DependencyDefinition::Scene(spec) = &view
            .versions
            .current()
            .ok_or(SceneError::Conflict)?
            .definition
        else {
            return Err(SceneError::Definition);
        };
        let spec = spec.clone();
        let id = crate::occurrences::random_id()
            .map_err(|_| SceneError::Storage)?
            .to_string();
        let cancel = CancellationToken::new();
        let deadline = scenes::deadline(&spec);
        let run = SceneRun {
            id: id.clone(),
            scene: expected.id,
            parent_correlation,
            version: expected.version,
            started: moli_core::now_ms(),
            ended: None,
            status: SceneStatus::Running,
            targets: expected
                .scope
                .targets
                .iter()
                .map(|t| SceneTargetResult {
                    point: t.point.clone(),
                    status: TargetStatus::Waiting,
                    acknowledged: false,
                    approval: None,
                    detail: None,
                })
                .collect(),
        };
        let watches = scenes::watches(&expected.scope, &run.id, &cancel);
        {
            let mut book = self.0.scenes.lock().unwrap_or_else(PoisonError::into_inner);
            if book.active.len() >= 4
                || book
                    .active
                    .values()
                    .flat_map(|a| &a.watches)
                    .any(|old| watches.iter().any(|w| !w.devices.is_disjoint(&old.devices)))
            {
                return Err(SceneError::Busy);
            }
            book.admit(&watches, deadline)?;
            self.0
                .scene_leases_active
                .store(true, super::Ordering::Release);
            book.active.insert(
                id.clone(),
                Active {
                    run: run.clone(),
                    cancel: cancel.clone(),
                    watches: watches.clone(),
                },
            );
            book.reindex();
            self.0.scenes_active.store(true, super::Ordering::Release);
        }

        drop(owner);
        self.scene_audit(&run, None, TargetStatus::Waiting, origin, actor.clone());
        let hub = self.clone();
        tokio::spawn(async move {
            hub.execute_scene(
                run,
                spec,
                expected.scope,
                watches,
                deadline,
                origin,
                actor,
                parent,
            )
            .await;
        });
        Ok(self.scene_run(&id).expect("admitted scene retained"))
    }
    pub fn scene_run(&self, id: &str) -> Option<SceneRun> {
        self.0
            .scenes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id)
    }
    pub fn cancel_scene(
        &self,
        id: &str,
        origin: Origin,
        actor: Option<String>,
    ) -> Result<(), SceneError> {
        let mut book = self.0.scenes.lock().unwrap_or_else(PoisonError::into_inner);
        let run = book.get(id).ok_or(SceneError::Unknown)?;
        if let Some(active) = book.active.get(id) {
            active.cancel.cancel();
        }
        book.revoke(id);
        self.0
            .scene_leases_active
            .store(book.has_leases(), super::Ordering::Release);
        drop(book);
        self.revoke_scene_approvals(id, None);
        self.scene_audit(&run, None, TargetStatus::Cancelled, origin, actor);
        Ok(())
    }
    pub async fn wait_scene(&self, id: &str) -> Result<SceneRun, SceneError> {
        let mut events = self.subscribe();
        loop {
            let run = self.scene_run(id).ok_or(SceneError::Unknown)?;
            if run.status != SceneStatus::Running {
                return Ok(run);
            }
            if matches!(
                events.recv().await,
                Err(tokio::sync::broadcast::error::RecvError::Closed)
            ) {
                return Err(SceneError::Unknown);
            }
        }
    }
}

fn scene_dependency_error(error: DependencyError) -> SceneError {
    match error {
        DependencyError::Unknown => SceneError::Unknown,
        DependencyError::Definition => SceneError::Definition,
        DependencyError::Conflict => SceneError::Conflict,
        _ => SceneError::Storage,
    }
}
