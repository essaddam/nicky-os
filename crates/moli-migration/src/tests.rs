use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
};

use moli_core::{DeviceId, PointId};
use moli_runtime::{StateFileError, WriteAuthority};

use crate::{
    registry::Policy,
    storage::{State, Store, StoreFuture},
    *,
};

type Events = Arc<Mutex<Vec<&'static str>>>;

#[derive(Debug, Default)]
struct MemoryStore {
    state: Mutex<Option<State>>,
    failure: AtomicU8,
    events: Events,
}
impl Store for MemoryStore {
    fn load(&self) -> StoreFuture<'_, std::io::Result<Option<State>>> {
        Box::pin(async { Ok(self.state.lock().unwrap().clone()) })
    }
    fn write<'a>(&'a self, state: &'a State) -> StoreFuture<'a, Result<(), StateFileError>> {
        Box::pin(async move {
            let failure = self.failure.swap(0, Ordering::SeqCst);
            if failure == 1 {
                return Err(StateFileError {
                    error: std::io::Error::other("before rename"),
                    commit_unknown: false,
                });
            }
            *self.state.lock().unwrap() = Some(state.clone());
            self.events.lock().unwrap().push("durable");
            if failure == 2 {
                return Err(StateFileError {
                    error: std::io::Error::other("after rename"),
                    commit_unknown: true,
                });
            }
            Ok(())
        })
    }
}

#[derive(Debug, Default)]
struct MemoryPolicy {
    entries: Mutex<BTreeMap<DeviceId, WriteAuthority>>,
    events: Events,
}
impl Policy for MemoryPolicy {
    fn resolved(&self, plan: &MigrationPlan) -> bool {
        plan.native_device.as_str().starts_with("fixture:")
    }
    fn current(&self, device: &DeviceId) -> Option<WriteAuthority> {
        self.entries.lock().unwrap().get(device).copied()
    }
    fn fence(&self, device: &DeviceId) -> WriteAuthority {
        let mut entries = self.entries.lock().unwrap();
        let authority = entries.entry(device.clone()).or_insert(WriteAuthority {
            generation: 1,
            allowed: false,
        });
        authority.allowed = false;
        self.events.lock().unwrap().push("fence");
        *authority
    }
    fn set(&self, device: &DeviceId, generation: u64, allowed: bool) -> Result<(), MigrationError> {
        let mut entries = self.entries.lock().unwrap();
        if let Some(current) = entries.get(device)
            && (generation < current.generation
                || (generation == current.generation && allowed != current.allowed))
        {
            return Err(MigrationError::Policy);
        }
        entries.insert(
            device.clone(),
            WriteAuthority {
                generation,
                allowed,
            },
        );
        self.events
            .lock()
            .unwrap()
            .push(if allowed { "grant" } else { "closed" });
        Ok(())
    }
}

fn plan(index: usize, direction: Direction) -> MigrationPlan {
    MigrationPlan {
        version: 1,
        binding: BindingKey {
            ha_installation: "fixture-ha".into(),
            registry_entity_id: format!("entity-{index}"),
            target: PointId::from(format!("fixture:device-{index}/state")),
            writer_scope: format!("writer-{index}"),
        },
        native_device: DeviceId::from(format!("fixture:device-{index}")),
        direction,
        automation_provenance: Some(AutomationProvenance {
            ha_installation: "fixture-ha".into(),
            ha_id: format!("automation-{index}"),
            source_sha256: "a".repeat(64),
            target_sha256: "b".repeat(64),
            mapping_revision: 1,
        }),
    }
}
async fn fixture() -> (Registry, Arc<MemoryStore>, Arc<MemoryPolicy>) {
    let events = Events::default();
    let store = Arc::new(MemoryStore {
        events: events.clone(),
        ..MemoryStore::default()
    });
    let policy = Arc::new(MemoryPolicy {
        events,
        ..MemoryPolicy::default()
    });
    let registry = Registry::from_parts(policy.clone(), store.clone())
        .await
        .unwrap();
    (registry, store, policy)
}
fn proof(binding: &Binding, owner: Owner) -> Proof {
    Proof {
        operation: binding.operation_id.clone(),
        plan_hash: binding.plan_hash.clone(),
        writer_scope: binding.key.writer_scope.clone(),
        revision: binding.revision,
        observed_owner: owner,
        quiescent: true,
    }
}
fn reconciled(challenge: ProofChallenge, owner: Owner) -> Proof {
    Proof {
        operation: challenge.operation,
        plan_hash: challenge.plan_hash,
        writer_scope: challenge.writer_scope,
        revision: challenge.revision,
        observed_owner: owner,
        quiescent: true,
    }
}
async fn active(registry: &Registry, index: usize) -> Binding {
    let plan = plan(index, Direction::ToMoli);
    let hash = plan.hash().unwrap();
    let prepared = registry
        .prepare(plan, 0, &format!("out-{index}"), &hash)
        .await
        .unwrap();
    let stopped = registry
        .acknowledge(&prepared.key, proof(&prepared, Owner::None))
        .await
        .unwrap();
    registry
        .acknowledge(&stopped.key, proof(&stopped, Owner::Moli))
        .await
        .unwrap()
}

#[test]
fn plan_hash_binds_identity_direction_native_reference_and_provenance() {
    let original = plan(0, Direction::ToMoli);
    let hash = original.hash().unwrap();
    let encoded = serde_json::to_vec(&original).unwrap();
    let decoded: MigrationPlan = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(hash, decoded.hash().unwrap());
    let mut changed = original.clone();
    changed.direction = Direction::ToHa;
    assert_ne!(hash, changed.hash().unwrap());
    let mut changed = original.clone();
    changed.binding.registry_entity_id = "other-entity".into();
    assert_ne!(hash, changed.hash().unwrap());
    let mut changed = original.clone();
    changed
        .automation_provenance
        .as_mut()
        .unwrap()
        .mapping_revision = 2;
    assert_ne!(hash, changed.hash().unwrap());
    let mut changed = original;
    changed.native_device = DeviceId::from("fixture:other");
    assert_eq!(changed.hash(), Err(MigrationError::Invalid));
}

#[tokio::test]
async fn exact_proofs_and_revisions_are_required_before_any_grant() {
    let (registry, _, policy) = fixture().await;
    let plan = plan(0, Direction::ToMoli);
    let hash = plan.hash().unwrap();
    assert_eq!(
        registry
            .prepare(plan.clone(), 0, "op", "0".repeat(64).as_str())
            .await,
        Err(MigrationError::PlanHash)
    );
    let prepared = registry.prepare(plan, 0, "op", &hash).await.unwrap();
    assert!(!policy.current(&prepared.native_device).unwrap().allowed);
    for mismatch in 0..6 {
        let mut wrong = proof(&prepared, Owner::None);
        match mismatch {
            0 => wrong.revision += 1,
            1 => wrong.plan_hash = "0".repeat(64),
            2 => wrong.operation = "different".into(),
            3 => wrong.writer_scope = "different".into(),
            4 => wrong.quiescent = false,
            _ => wrong.observed_owner = Owner::Unknown,
        }
        assert!(registry.acknowledge(&prepared.key, wrong).await.is_err());
        assert_eq!(
            registry.get(&prepared.key).await.unwrap().revision,
            prepared.revision
        );
        assert!(!registry.check_can_write(&prepared.key, prepared.revision, prepared.generation));
    }
    assert_eq!(
        registry
            .acknowledge(&prepared.key, proof(&prepared, Owner::Moli))
            .await,
        Err(MigrationError::Transition)
    );
    let stopped = registry
        .acknowledge(&prepared.key, proof(&prepared, Owner::None))
        .await
        .unwrap();
    assert_eq!(stopped.phase, Phase::HaStopped);
    assert!(!policy.current(&prepared.native_device).unwrap().allowed);
    assert_eq!(
        registry
            .acknowledge(&prepared.key, proof(&prepared, Owner::None))
            .await,
        Err(MigrationError::Revision)
    );
    let moli = registry
        .acknowledge(&stopped.key, proof(&stopped, Owner::Moli))
        .await
        .unwrap();
    assert!(registry.check_can_write(&moli.key, moli.revision, moli.generation));
    assert!(!registry.check_can_write(&moli.key, moli.revision - 1, moli.generation));
    assert!(!moli.automation_draft_enabled());
    let events = policy.events.lock().unwrap();
    let grant = events.iter().position(|event| *event == "grant").unwrap();
    assert_eq!(events[grant - 1], "durable");
}

#[tokio::test]
async fn reverse_transfer_closes_moli_before_ha_can_be_confirmed() {
    let (registry, _, policy) = fixture().await;
    let moli = active(&registry, 0).await;
    let mut reverse = moli.plan.clone();
    reverse.direction = Direction::ToHa;
    let hash = reverse.hash().unwrap();
    let prepared = registry
        .prepare(reverse, moli.revision, "return-ha", &hash)
        .await
        .unwrap();
    assert_eq!(prepared.phase, Phase::PreparedToHa);
    assert!(!registry.check_can_write(&moli.key, moli.revision, moli.generation));
    assert!(!policy.current(&moli.native_device).unwrap().allowed);
    assert_eq!(
        registry
            .acknowledge(&prepared.key, proof(&prepared, Owner::Ha))
            .await,
        Err(MigrationError::Transition)
    );
    let stopped = registry
        .acknowledge(&prepared.key, proof(&prepared, Owner::None))
        .await
        .unwrap();
    assert_eq!(stopped.phase, Phase::MoliStopped);
    let ha = registry
        .acknowledge(&stopped.key, proof(&stopped, Owner::Ha))
        .await
        .unwrap();
    assert_eq!(ha.phase, Phase::Ha);
    assert!(ha.generation > moli.generation);
    assert!(!ha.native_owner_moli());
    assert!(!policy.current(&ha.native_device).unwrap().allowed);
}

#[tokio::test]
async fn reopening_fences_every_target_and_requires_a_new_session_challenge() {
    let (first, store, policy) = fixture().await;
    let moli = active(&first, 0).await;
    let second = Registry::from_parts(policy.clone(), store.clone())
        .await
        .unwrap();
    assert!(!first.check_can_write(&moli.key, moli.revision, moli.generation));
    assert_eq!(
        second.get(&moli.key).await.unwrap().phase,
        Phase::ReconcileRequired
    );
    let old_challenge = second.reconcile_challenge(&moli.key).await.unwrap();
    let third = Registry::from_parts(policy.clone(), store).await.unwrap();
    assert_eq!(
        third
            .reconcile(&moli.key, reconciled(old_challenge, Owner::Moli))
            .await,
        Err(MigrationError::Proof)
    );
    let fresh = third.reconcile_challenge(&moli.key).await.unwrap();
    let resumed = third
        .reconcile(&moli.key, reconciled(fresh, Owner::Moli))
        .await
        .unwrap();
    assert!(resumed.generation > moli.generation);
    assert!(third.check_can_write(&resumed.key, resumed.revision, resumed.generation));
    assert!(!first.check_can_write(&moli.key, moli.revision, moli.generation));
}

#[tokio::test]
async fn failure_before_commit_keeps_the_previous_revision_but_closes_writes() {
    let (registry, store, policy) = fixture().await;
    let moli = active(&registry, 0).await;
    let mut reverse = moli.plan.clone();
    reverse.direction = Direction::ToHa;
    let hash = reverse.hash().unwrap();
    store.failure.store(1, Ordering::SeqCst);
    assert_eq!(
        registry
            .prepare(reverse, moli.revision, "back", &hash)
            .await,
        Err(MigrationError::Storage {
            commit_unknown: false
        })
    );
    let held = registry.get(&moli.key).await.unwrap();
    assert_eq!(held.revision, moli.revision);
    assert_eq!(held.plan, moli.plan);
    assert_eq!(held.phase, Phase::ReconcileRequired);
    assert!(!policy.current(&moli.native_device).unwrap().allowed);
    assert!(!registry.check_can_write(&moli.key, moli.revision, moli.generation));
    let fresh = registry.reconcile_challenge(&moli.key).await.unwrap();
    assert!(
        registry
            .reconcile(&moli.key, reconciled(fresh, Owner::Moli))
            .await
            .unwrap()
            .generation
            > moli.generation
    );
}

#[tokio::test]
async fn commit_unknown_requires_reload_without_rolling_back_or_retrying_the_operation() {
    let (registry, store, policy) = fixture().await;
    let moli = active(&registry, 0).await;
    let mut reverse = moli.plan.clone();
    reverse.direction = Direction::ToHa;
    let hash = reverse.hash().unwrap();
    store.failure.store(2, Ordering::SeqCst);
    assert_eq!(
        registry
            .prepare(reverse, moli.revision, "return", &hash)
            .await,
        Err(MigrationError::Storage {
            commit_unknown: true
        })
    );
    assert!(registry.snapshot().await.suspended);
    assert_eq!(
        registry.reconcile_challenge(&moli.key).await,
        Err(MigrationError::ReloadRequired)
    );
    assert!(!policy.current(&moli.native_device).unwrap().allowed);
    registry.reload().await.unwrap();
    let loaded = registry.get(&moli.key).await.unwrap();
    assert_eq!(loaded.revision, moli.revision + 1);
    assert_eq!(loaded.plan.direction, Direction::ToHa);
    let challenge = registry.reconcile_challenge(&moli.key).await.unwrap();
    assert_eq!(
        registry
            .reconcile(&moli.key, reconciled(challenge.clone(), Owner::Unknown))
            .await,
        Err(MigrationError::Proof)
    );
    let stopped = registry
        .reconcile(&moli.key, reconciled(challenge, Owner::None))
        .await
        .unwrap();
    assert_eq!(stopped.phase, Phase::MoliStopped);
    let ha = registry
        .acknowledge(&moli.key, proof(&stopped, Owner::Ha))
        .await
        .unwrap();
    assert_eq!(ha.phase, Phase::Ha);
    assert!(!policy.current(&moli.native_device).unwrap().allowed);
}

#[tokio::test]
async fn active_operations_native_aliases_and_unresolved_references_are_bounded() {
    let (registry, _, _) = fixture().await;
    let mut unresolved = plan(0, Direction::ToMoli);
    unresolved.native_device = DeviceId::from("missing:device");
    unresolved.binding.target = PointId::from("missing:device/state");
    let hash = unresolved.hash().unwrap();
    assert_eq!(
        registry.prepare(unresolved, 0, "missing", &hash).await,
        Err(MigrationError::Unresolved)
    );
    for index in 0..MAX_ACTIVE_OPERATIONS {
        let plan = plan(index, Direction::ToMoli);
        let hash = plan.hash().unwrap();
        registry
            .prepare(plan, 0, &format!("op-{index}"), &hash)
            .await
            .unwrap();
    }
    let extra = plan(MAX_ACTIVE_OPERATIONS, Direction::ToMoli);
    let hash = extra.hash().unwrap();
    assert_eq!(
        registry.prepare(extra, 0, "too-many", &hash).await,
        Err(MigrationError::Bound)
    );
    assert_eq!(
        registry.snapshot().await.active_operations,
        MAX_ACTIVE_OPERATIONS
    );
    let mut alias = plan(0, Direction::ToMoli);
    alias.binding.target = PointId::from("fixture:device-0/other");
    let hash = alias.hash().unwrap();
    assert_eq!(
        registry.prepare(alias, 0, "alias", &hash).await,
        Err(MigrationError::Invalid)
    );
}

#[tokio::test]
async fn corrupt_hashes_and_excessive_binding_files_are_not_loaded() {
    let (_, store, policy) = fixture().await;
    let mut bindings = Vec::new();
    for index in 0..=MAX_BINDINGS {
        let plan = plan(index, Direction::ToHa);
        let hash = plan.hash().unwrap();
        bindings.push(Binding {
            key: plan.binding.clone(),
            native_device: plan.native_device.clone(),
            phase: Phase::Ha,
            revision: 1,
            generation: 1,
            operation_id: format!("saved-{index}"),
            plan_hash: hash,
            plan,
            reconcile_required: false,
        });
    }
    *store.state.lock().unwrap() = Some(State {
        schema: 1,
        bindings: bindings.clone(),
    });
    assert!(matches!(
        Registry::from_parts(policy.clone(), store.clone()).await,
        Err(MigrationError::Bound)
    ));
    bindings.pop();
    bindings[0].plan_hash = "0".repeat(64);
    *store.state.lock().unwrap() = Some(State {
        schema: 1,
        bindings,
    });
    assert!(matches!(
        Registry::from_parts(policy, store).await,
        Err(MigrationError::PlanHash)
    ));
}

#[tokio::test]
async fn command_predicates_are_weak_deadline_bound_and_exact_to_the_point() {
    let (registry, _, policy) = fixture().await;
    let moli = active(&registry, 0).await;
    let future = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let check = registry.permit_check(moli.key.clone(), moli.revision, moli.generation, future);
    assert!(check(&moli.key.target));
    assert!(!check(&PointId::from("fixture:other/state")));
    let expired = registry.permit_check(
        moli.key.clone(),
        moli.revision,
        moli.generation,
        std::time::Instant::now(),
    );
    assert!(!expired(&moli.key.target));
    let stale = registry.permit_check(moli.key.clone(), moli.revision + 1, moli.generation, future);
    assert!(!stale(&moli.key.target));
    let old_generation =
        registry.permit_check(moli.key.clone(), moli.revision, moli.generation + 1, future);
    assert!(!old_generation(&moli.key.target));
    drop(registry);
    // Keeping the policy alive does not keep the registry behind a permit alive.
    assert!(policy.current(&moli.native_device).unwrap().allowed);
    assert!(!check(&moli.key.target));
}

#[tokio::test]
async fn command_predicate_closes_on_native_fencing_and_snapshot_transition() {
    let (registry, _, policy) = fixture().await;
    let moli = active(&registry, 0).await;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let check = registry.permit_check(moli.key.clone(), moli.revision, moli.generation, deadline);
    assert!(check(&moli.key.target));
    policy.fence(&moli.native_device);
    assert!(!check(&moli.key.target));
    let challenge = registry.reconcile_challenge(&moli.key).await.unwrap();
    let current = registry
        .reconcile(&moli.key, reconciled(challenge, Owner::Moli))
        .await
        .unwrap();
    assert!(!check(&moli.key.target));
    let current_check = registry.permit_check(
        current.key.clone(),
        current.revision,
        current.generation,
        deadline,
    );
    assert!(current_check(&current.key.target));
    let mut back = current.plan.clone();
    back.direction = Direction::ToHa;
    let hash = back.hash().unwrap();
    registry
        .prepare(back, current.revision, "permit-return", &hash)
        .await
        .unwrap();
    assert!(!current_check(&current.key.target));
}

#[tokio::test]
async fn ha_source_ownership_and_generation_track_handoff_and_reconciliation() {
    let (registry, store, policy) = fixture().await;
    assert_eq!(
        registry.source_generation("fixture-ha", "entity-0"),
        Some(0)
    );
    assert!(registry.source_owned_by_ha("fixture-ha", "entity-0"));
    assert_eq!(registry.source_generation("", "entity-0"), None);
    assert!(!registry.source_owned_by_ha("fixture-ha", "invalid/source"));
    let moli = active(&registry, 0).await;
    assert_eq!(
        registry.source_generation("fixture-ha", "entity-0"),
        Some(moli.generation)
    );
    assert!(!registry.source_owned_by_ha("fixture-ha", "entity-0"));
    assert_eq!(
        registry.source_generation("another-fixture", "entity-0"),
        Some(0)
    );
    assert!(registry.source_owned_by_ha("another-fixture", "entity-0"));
    let mut back = moli.plan.clone();
    back.direction = Direction::ToHa;
    let hash = back.hash().unwrap();
    let prepared = registry
        .prepare(back, moli.revision, "source-return", &hash)
        .await
        .unwrap();
    assert!(!registry.source_owned_by_ha("fixture-ha", "entity-0"));
    let stopped = registry
        .acknowledge(&prepared.key, proof(&prepared, Owner::None))
        .await
        .unwrap();
    assert!(!registry.source_owned_by_ha("fixture-ha", "entity-0"));
    let ha = registry
        .acknowledge(&stopped.key, proof(&stopped, Owner::Ha))
        .await
        .unwrap();
    assert_eq!(
        registry.source_generation("fixture-ha", "entity-0"),
        Some(ha.generation)
    );
    assert!(registry.source_owned_by_ha("fixture-ha", "entity-0"));
    let reopened = Registry::from_parts(policy, store).await.unwrap();
    assert_eq!(
        reopened.source_generation("fixture-ha", "entity-0"),
        Some(ha.generation)
    );
    assert!(!reopened.source_owned_by_ha("fixture-ha", "entity-0"));
}

#[tokio::test]
async fn an_ambiguous_ha_source_never_opens_a_write_route() {
    let (registry, _, _) = fixture().await;
    active(&registry, 0).await;
    let mut other = plan(1, Direction::ToMoli);
    other.binding.registry_entity_id = "entity-0".into();
    let hash = other.hash().unwrap();
    registry
        .prepare(other, 0, "same-source", &hash)
        .await
        .unwrap();
    assert_eq!(registry.source_generation("fixture-ha", "entity-0"), None);
    assert!(!registry.source_owned_by_ha("fixture-ha", "entity-0"));
}
