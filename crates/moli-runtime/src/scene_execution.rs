use super::{Action, Event, Hub, Instant, Origin, Outcome, PoisonError, Value};
use crate::scenes::{self, Watch};
use crate::{
    CommandError, CommandPermit, SceneRun, SceneScope, SceneStatus, SceneTargetResult, TargetStatus,
};
use moli_core::{PointId, SceneSpec};
use std::sync::Arc;
use std::time::Duration;

impl Hub {
    pub(super) fn revoke_scene_approvals(&self, correlation: &str, point: Option<&PointId>) {
        let revoked = {
            let mut book = self
                .0
                .approvals
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let ids: Vec<_> = book
                .pending
                .values()
                .filter(|r| {
                    r.correlation.as_deref() == Some(correlation)
                        && point.is_none_or(|p| &r.point == p)
                })
                .map(|r| r.id)
                .collect();
            ids.into_iter()
                .filter_map(|id| {
                    book.permits.remove(&id);
                    book.pending.remove(&id)
                })
                .collect::<Vec<_>>()
        };
        for request in revoked {
            self.resolved(
                &request,
                moli_core::Decision::Expired,
                Origin::System,
                None,
                Outcome::Err(moli_i18n::tr!("serveur.scene.annulee")),
                Instant::now(),
            );
        }
    }
    pub(super) fn scene_audit(
        &self,
        run: &SceneRun,
        point: Option<PointId>,
        status: TargetStatus,
        origin: Origin,
        actor: Option<String>,
    ) {
        self.record_correlated(
            origin,
            actor,
            Action::Scene {
                id: run.scene.clone(),
                version: run.version,
                parent_correlation: run.parent_correlation.clone(),
                point,
                status: serde_json::to_value(status)
                    .expect("scene status serializable")
                    .as_str()
                    .expect("status is a string")
                    .to_owned(),
            },
            if matches!(
                status,
                TargetStatus::Failed
                    | TargetStatus::ConfirmationExpired
                    | TargetStatus::Superseded
                    | TargetStatus::Cancelled
            ) {
                Outcome::Err(moli_i18n::tr!("serveur.scene.incomplete"))
            } else {
                Outcome::Ok
            },
            Instant::now(),
            Some(run.id.clone()),
        );
    }
    fn scene_target_result(
        &self,
        id: &str,
        index: usize,
        result: SceneTargetResult,
        origin: Origin,
        actor: Option<String>,
    ) {
        let run = {
            let mut book = self.0.scenes.lock().unwrap_or_else(PoisonError::into_inner);
            let Some(active) = book.active.get_mut(id) else {
                return;
            };
            active.run.targets[index] = result.clone();
            active.run.clone()
        };
        self.scene_audit(&run, Some(result.point), result.status, origin, actor);
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn execute_scene(
        &self,
        run: SceneRun,
        spec: SceneSpec,
        scope: SceneScope,
        watches: Vec<Watch>,
        deadline: Instant,
        origin: Origin,
        actor: Option<String>,
        parent: Option<CommandPermit>,
    ) {
        let mut jobs = tokio::task::JoinSet::new();
        let parallel = Arc::new(tokio::sync::Semaphore::new(usize::from(spec.parallel)));
        let run = Arc::new(run);
        let spec = Arc::new(spec);
        let scope = Arc::new(scope);
        let watches = Arc::new(watches);
        for lane in scenes::lanes(&scope) {
            let hub = self.clone();
            let run = run.clone();
            let spec = spec.clone();
            let scope = scope.clone();
            let watches = watches.clone();
            let parallel = parallel.clone();
            let actor = actor.clone();
            let parent = parent.clone();
            jobs.spawn(async move {
                for index in lane {
                    let watch = &watches[index];
                    hub.scene_target_result(
                        &run.id,
                        index,
                        SceneTargetResult {
                            point: watch.point.clone(),
                            status: TargetStatus::Waiting,
                            acknowledged: false,
                            approval: None,
                            detail: None,
                        },
                        origin,
                        actor.clone(),
                    );
                    let result = hub
                        .execute_scene_target(
                            &scope.targets[index],
                            watch,
                            deadline,
                            spec.confirmation_s,
                            origin,
                            actor.clone(),
                            parent.clone(),
                            &parallel,
                        )
                        .await;
                    if !matches!(
                        result.status,
                        TargetStatus::Confirmed | TargetStatus::AlreadyConform
                    ) {
                        watch.state().superseded = true;
                        hub.revoke_scene_approvals(&run.id, Some(&result.point));
                    }
                    hub.scene_target_result(&run.id, index, result, origin, actor.clone());
                }
            });
        }
        let mut failed_task = false;
        while let Some(joined) = jobs.join_next().await {
            failed_task |= joined.is_err();
        }
        let Some(ended) = self.finish_scene(&run.id, failed_task) else {
            return;
        };
        self.revoke_scene_approvals(&ended.id, None);
        self.scene_audit(
            &ended,
            None,
            if ended.status == SceneStatus::Done {
                TargetStatus::Confirmed
            } else {
                TargetStatus::Failed
            },
            origin,
            actor,
        );
    }
    fn finish_scene(&self, id: &str, failed_task: bool) -> Option<SceneRun> {
        let mut book = self.0.scenes.lock().unwrap_or_else(PoisonError::into_inner);
        let mut active = book.active.remove(id)?;
        // Explicit cancellation revokes all effects. Normal completion keeps
        // confirmed composite continuations bounded by their original deadline.
        let cancelled = active.cancel.is_cancelled();
        active.run.ended = Some(moli_core::now_ms());
        active.run.status = if cancelled {
            SceneStatus::Cancelled
        } else if !failed_task
            && active.run.targets.iter().all(|r| {
                matches!(
                    r.status,
                    TargetStatus::Confirmed | TargetStatus::AlreadyConform
                )
            })
        {
            SceneStatus::Done
        } else {
            SceneStatus::Partial
        };
        book.reindex();
        let ended = active.run;
        if book.active.is_empty() {
            self.0.scenes_active.store(false, super::Ordering::Release);
        }
        while book.ended.len() >= 64 {
            book.ended.pop_front();
        }
        book.ended.push_back(ended.clone());
        Some(ended)
    }
    #[allow(clippy::too_many_arguments)]
    async fn execute_scene_target(
        &self,
        target: &crate::SceneScopeTarget,
        watch: &Watch,
        deadline: Instant,
        confirmation_s: u8,
        origin: Origin,
        actor: Option<String>,
        parent: Option<CommandPermit>,
        parallel: &tokio::sync::Semaphore,
    ) -> SceneTargetResult {
        let result = SceneTargetResult {
            point: target.point.clone(),
            status: TargetStatus::Waiting,
            acknowledged: false,
            approval: None,
            detail: None,
        };
        let finish = |mut r: SceneTargetResult, status| {
            r.status = status;
            r
        };
        let _permit = tokio::select! {
            biased;
            () = watch.cancel.cancelled() => return finish(result, TargetStatus::Cancelled),
            () = tokio::time::sleep_until(deadline.into()) => return finish(result, TargetStatus::ConfirmationExpired),
            permit = parallel.acquire() => match permit { Ok(p) => p, Err(_) => return finish(result, TargetStatus::Failed) }
        };
        let _global = tokio::select! {
            biased;
            () = watch.cancel.cancelled() => return finish(result, TargetStatus::Cancelled),
            () = tokio::time::sleep_until(deadline.into()) => return finish(result, TargetStatus::ConfirmationExpired),
            permit = self.0.scene_sends.get_or_init(|| Arc::new(tokio::sync::Semaphore::new(4))).acquire() => match permit {Ok(p)=>p,Err(_)=>return finish(result, TargetStatus::Failed)}
        };
        // Reread the immutable descriptors after waiting, without physical I/O.
        let spec = SceneSpec {
            targets: vec![moli_core::SceneTarget {
                point: target.point.clone(),
                value: target.value.clone(),
            }],
            parallel: 1,
            timeout_s: 120,
            confirmation_s: 30,
        };
        if self
            .scene_scope(&spec)
            .ok()
            .and_then(|s| s.targets.into_iter().next())
            .as_ref()
            != Some(target)
        {
            return finish(result, TargetStatus::Superseded);
        }
        {
            let model = self.read();
            if target
                .devices
                .iter()
                .any(|id| model.availability.get(id) == Some(&false))
            {
                return finish(result, TargetStatus::Failed);
            }
        }
        if !watch.wanted() {
            return finish(
                result,
                if watch.cancel.is_cancelled() {
                    TargetStatus::Cancelled
                } else {
                    TargetStatus::Superseded
                },
            );
        }
        self.0
            .scenes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .target_admitted(watch);
        // A recent matching snapshot already satisfies this target. It is a
        // separate outcome, with no fabricated driver acknowledgement.
        if let Some(sample) = self.state(&target.point) {
            let now = moli_core::now_ms();
            if sample.ts <= now
                && now - sample.ts <= u64::from(confirmation_s) * 1000
                && self.scene_value_matches(target, &sample.value)
            {
                return finish(result, TargetStatus::AlreadyConform);
            }
        }
        self.submit_scene_target(
            target,
            watch,
            deadline,
            confirmation_s,
            origin,
            actor,
            parent,
        )
        .await
    }

    fn scene_value_matches(&self, target: &crate::SceneScopeTarget, observed: &Value) -> bool {
        let Some((id, key)) = target.point.split() else {
            return false;
        };
        let model = self.read();
        let Some(point) = model.devices.get(&id).and_then(|device| device.point(key)) else {
            return false;
        };
        // The approved scope already contains the canonical wanted value. Apply
        // the same point validation to observations: Numeric Int/Float agree,
        // while wrong types, nonfinite numbers and values outside bounds fail.
        crate::validate_point_value(&point.kind, observed)
            .is_ok_and(|value| value.to_json() == target.value)
    }

    #[allow(clippy::too_many_arguments)]
    async fn submit_scene_target(
        &self,
        target: &crate::SceneScopeTarget,
        watch: &Watch,
        deadline: Instant,
        confirmation_s: u8,
        origin: Origin,
        actor: Option<String>,
        parent: Option<CommandPermit>,
    ) -> SceneTargetResult {
        let mut result = SceneTargetResult {
            point: target.point.clone(),
            status: TargetStatus::Waiting,
            acknowledged: false,
            approval: None,
            detail: None,
        };
        let finish = |mut r: SceneTargetResult, status| {
            r.status = status;
            r
        };
        let live = watch.clone();
        let command_permit = CommandPermit::new(move |point| {
            Instant::now() < deadline
                && live.wanted()
                && parent.as_ref().is_none_or(|p| p.allows(point))
        });
        let events = self.subscribe();
        let command = self.command_correlated_with_permit(
            &target.point,
            Value::from_json(&target.value),
            origin,
            actor.clone(),
            Some(watch.correlation.clone()),
            command_permit,
        );
        let response = tokio::select! {
            biased;
            () = watch.cancel.cancelled() => return finish(result, TargetStatus::Cancelled),
            () = tokio::time::sleep_until(deadline.into()) => return finish(result, TargetStatus::ConfirmationExpired),
            response = command => response,
        };
        match response {
            Ok(()) => {
                result.acknowledged = true;
                result.status = TargetStatus::Acknowledged;
            }
            Err(CommandError::NeedsApproval { id, .. }) => {
                result.approval = Some(id);
                result.status = TargetStatus::Held;
            }
            Err(error) => {
                result.detail = Some(error.to_string().chars().take(512).collect());
                return finish(
                    result,
                    if watch.wanted() {
                        TargetStatus::Failed
                    } else {
                        TargetStatus::Superseded
                    },
                );
            }
        }
        self.scene_intermediate(watch, &result, origin, actor.clone());
        self.confirm_scene_target(
            result,
            target,
            watch,
            deadline,
            confirmation_s,
            events,
            &(origin, actor),
        )
        .await
    }

    fn scene_intermediate(
        &self,
        watch: &Watch,
        result: &SceneTargetResult,
        origin: Origin,
        actor: Option<String>,
    ) {
        let index = self
            .0
            .scenes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .active
            .get(&watch.correlation)
            .and_then(|a| a.run.targets.iter().position(|r| r.point == result.point));
        if let Some(index) = index {
            self.scene_target_result(&watch.correlation, index, result.clone(), origin, actor);
        }
    }
    #[allow(clippy::too_many_arguments)]
    async fn confirm_scene_target(
        &self,
        mut result: SceneTargetResult,
        target: &crate::SceneScopeTarget,
        watch: &Watch,
        deadline: Instant,
        confirmation_s: u8,
        mut events: tokio::sync::broadcast::Receiver<Event>,
        audit: &(Origin, Option<String>),
    ) -> SceneTargetResult {
        let finish = |mut r: SceneTargetResult, status| {
            r.status = status;
            r
        };
        let mut confirm_by = if result.acknowledged {
            deadline.min(Instant::now() + Duration::from_secs(u64::from(confirmation_s)))
        } else {
            deadline
        };
        loop {
            let wake = watch.wake.notified();
            if watch.cancel.is_cancelled() {
                return finish(result, TargetStatus::Cancelled);
            }
            let observed = {
                let state = watch.state();
                if state.superseded {
                    return finish(result, TargetStatus::Superseded);
                }
                state.observed.clone()
            };
            // Release the watch before reading point descriptors in the model.
            if result.acknowledged
                && observed
                    .as_ref()
                    .is_some_and(|(v, at)| *at <= confirm_by && self.scene_value_matches(target, v))
            {
                return finish(result, TargetStatus::Confirmed);
            }
            tokio::select! {
                biased;
                () = watch.cancel.cancelled() => return finish(result, TargetStatus::Cancelled),
                () = tokio::time::sleep_until(confirm_by.into()) => return finish(result, TargetStatus::ConfirmationExpired),
                () = wake => {},
                event = events.recv() => {
                    match event {
                        Ok(Event::Journal {entry}) if entry.correlation.as_deref() == Some(watch.correlation.as_str()) => {
                            if let Action::Approval {request,decision,..} = &entry.action
                                && Some(*request) == result.approval {
                                if *decision == moli_core::Decision::Approved && entry.outcome == Outcome::Ok {
                                    result.acknowledged = true;result.status=TargetStatus::Acknowledged;
                                    self.scene_intermediate(watch,&result,audit.0,audit.1.clone());
                                    confirm_by=deadline.min(Instant::now()+Duration::from_secs(u64::from(confirmation_s)));
                                }
                                else {return finish(result, TargetStatus::Failed);}
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => return finish(result, TargetStatus::ConfirmationExpired),
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => return finish(result, TargetStatus::Failed),
                        _ => {},
                    }
                }
            }
        }
    }
}
