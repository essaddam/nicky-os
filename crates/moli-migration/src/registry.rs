use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Debug,
    path::PathBuf,
    sync::{Arc, RwLock},
    time::Instant,
};

use moli_core::{DeviceId, PointId};
use moli_runtime::{CommandPermit, Hub, StateFile, WriteAuthority};
use ring::rand::{SecureRandom, SystemRandom};

use crate::{
    Binding, BindingKey, Direction, MAX_STATE_BYTES, MigrationError, MigrationPlan, Owner, Phase,
    Proof, ProofChallenge, Snapshot, model,
    storage::{FileStore, State, Store},
};

pub(crate) trait Policy: Debug + Send + Sync {
    fn resolved(&self, plan: &MigrationPlan) -> bool;
    fn current(&self, device: &DeviceId) -> Option<WriteAuthority>;
    fn fence(&self, device: &DeviceId) -> WriteAuthority;
    fn set(&self, device: &DeviceId, generation: u64, allowed: bool) -> Result<(), MigrationError>;
}

#[derive(Debug)]
struct HubPolicy(Hub);
impl Policy for HubPolicy {
    fn resolved(&self, plan: &MigrationPlan) -> bool {
        plan.binding.target.split().is_some_and(|(id, key)| {
            id == plan.native_device
                && self.0.describe(&id).is_some_and(|(device, _)| {
                    device.point(key).is_some_and(|point| point.access.write)
                })
        })
    }
    fn current(&self, device: &DeviceId) -> Option<WriteAuthority> {
        self.0.write_authority(device)
    }
    fn fence(&self, device: &DeviceId) -> WriteAuthority {
        self.0.fence_write_authority(device)
    }
    fn set(&self, device: &DeviceId, generation: u64, allowed: bool) -> Result<(), MigrationError> {
        self.0
            .set_write_authority(device, generation, allowed)
            .map_err(|_| MigrationError::Policy)
    }
}

#[derive(Debug)]
struct Loaded {
    state: State,
    fenced: BTreeSet<BindingKey>,
    reload_required: bool,
    session: String,
}

#[derive(Clone, Debug)]
struct Permit {
    device: DeviceId,
    revision: u64,
    generation: u64,
    allowed: bool,
}

#[derive(Debug)]
struct Inner {
    store: Arc<dyn Store>,
    policy: Arc<dyn Policy>,
    loaded: tokio::sync::Mutex<Loaded>,
    snapshot: RwLock<Snapshot>,
    permits: RwLock<BTreeMap<BindingKey, Permit>>,
}

/// One bounded writer. Mutations finish in owned tasks even if their caller disconnects.
#[derive(Clone, Debug)]
pub struct Registry(Arc<Inner>);

impl Registry {
    pub async fn open(hub: Hub, path: PathBuf) -> Result<Self, MigrationError> {
        Self::from_parts(
            Arc::new(HubPolicy(hub)),
            Arc::new(FileStore(StateFile::new(Some(path), MAX_STATE_BYTES))),
        )
        .await
    }

    pub(crate) async fn from_parts(
        policy: Arc<dyn Policy>,
        store: Arc<dyn Store>,
    ) -> Result<Self, MigrationError> {
        let state = store
            .load()
            .await
            .map_err(|_| MigrationError::Load)?
            .unwrap_or_default();
        state.validate()?;
        let fenced = state
            .bindings
            .iter()
            .map(|binding| {
                policy.fence(&binding.native_device);
                binding.key.clone()
            })
            .collect();
        let loaded = Loaded {
            state,
            fenced,
            reload_required: false,
            session: random_session()?,
        };
        let registry = Self(Arc::new(Inner {
            store,
            policy,
            loaded: tokio::sync::Mutex::new(loaded),
            snapshot: RwLock::new(Snapshot::default()),
            permits: RwLock::new(BTreeMap::new()),
        }));
        {
            let loaded = registry.0.loaded.lock().await;
            registry.publish(&loaded);
        }
        Ok(registry)
    }

    pub async fn snapshot(&self) -> Snapshot {
        let loaded = self.0.loaded.lock().await;
        self.publish(&loaded);
        self.0
            .snapshot
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub async fn get(&self, key: &BindingKey) -> Option<Binding> {
        self.snapshot()
            .await
            .bindings
            .into_iter()
            .find(|binding| &binding.key == key)
    }

    /// Safe for a synchronous send-permit callback; also checks the current Hub generation.
    pub fn check_can_write(&self, key: &BindingKey, revision: u64, generation: u64) -> bool {
        let permits = self
            .0
            .permits
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        permits.get(key).is_some_and(|permit| {
            permit.allowed
                && permit.revision == revision
                && permit.generation == generation
                && self
                    .0
                    .policy
                    .current(&permit.device)
                    .is_some_and(|current| current.allowed && current.generation == generation)
        })
    }

    /// An unmanaged HA source starts at generation zero. Ambiguous sources have no generation.
    pub fn source_generation(&self, installation: &str, entity_id: &str) -> Option<u64> {
        if !model::opaque(installation) || !model::opaque(entity_id) {
            return None;
        }
        let snapshot = self
            .0
            .snapshot
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut matches = snapshot.bindings.iter().filter(|binding| {
            binding.key.ha_installation == installation
                && binding.key.registry_entity_id == entity_id
        });
        let generation = matches.next().map_or(0, |binding| binding.generation);
        matches.next().is_none().then_some(generation)
    }

    /// Rechecked just before publishing an HA command; a prepared or fenced source stays closed.
    pub fn source_owned_by_ha(&self, installation: &str, entity_id: &str) -> bool {
        if !model::opaque(installation) || !model::opaque(entity_id) {
            return false;
        }
        let snapshot = self
            .0
            .snapshot
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut matches = snapshot.bindings.iter().filter(|binding| {
            binding.key.ha_installation == installation
                && binding.key.registry_entity_id == entity_id
        });
        let Some(binding) = matches.next() else {
            return true;
        };
        matches.next().is_none()
            && !snapshot.suspended
            && !binding.reconcile_required
            && binding.phase == Phase::Ha
            && self
                .0
                .policy
                .current(&binding.native_device)
                .is_some_and(|authority| {
                    authority.generation == binding.generation && !authority.allowed
                })
    }

    /// Capture no Registry/Hub owner: an approval holding this permit cannot retain the hub.
    pub fn command_permit(
        &self,
        key: BindingKey,
        revision: u64,
        generation: u64,
        deadline: Instant,
    ) -> CommandPermit {
        CommandPermit::new(self.permit_check(key, revision, generation, deadline))
    }

    pub(crate) fn permit_check(
        &self,
        key: BindingKey,
        revision: u64,
        generation: u64,
        deadline: Instant,
    ) -> impl Fn(&PointId) -> bool + Send + Sync + 'static + use<> {
        let weak = Arc::downgrade(&self.0);
        move |point| {
            if point != &key.target || Instant::now() >= deadline {
                return false;
            }
            let Some(inner) = weak.upgrade() else {
                return false;
            };
            let permit = inner
                .permits
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&key)
                .cloned();
            let allowed = permit.is_some_and(|permit| {
                permit.allowed
                    && permit.revision == revision
                    && permit.generation == generation
                    && inner
                        .policy
                        .current(&permit.device)
                        .is_some_and(|authority| {
                            authority.allowed && authority.generation == generation
                        })
            });
            allowed && Instant::now() < deadline
        }
    }

    pub async fn prepare(
        &self,
        plan: MigrationPlan,
        expected_revision: u64,
        operation_id: &str,
        plan_hash: &str,
    ) -> Result<Binding, MigrationError> {
        let registry = self.clone();
        let operation_id = operation_id.to_owned();
        let plan_hash = plan_hash.to_owned();
        tokio::spawn(async move {
            registry
                .prepare_inner(plan, expected_revision, &operation_id, &plan_hash)
                .await
        })
        .await
        .map_err(|_| MigrationError::Interrupted)?
    }

    async fn prepare_inner(
        &self,
        plan: MigrationPlan,
        expected_revision: u64,
        operation_id: &str,
        plan_hash: &str,
    ) -> Result<Binding, MigrationError> {
        if !model::operation(operation_id) {
            return Err(MigrationError::Invalid);
        }
        if plan.hash()? != plan_hash {
            return Err(MigrationError::PlanHash);
        }
        if !self.0.policy.resolved(&plan) {
            return Err(MigrationError::Unresolved);
        }
        let mut loaded = self.0.loaded.lock().await;
        if loaded.reload_required {
            return Err(MigrationError::ReloadRequired);
        }
        let old = loaded
            .state
            .bindings
            .iter()
            .find(|binding| binding.key == plan.binding);
        if let Some(binding) = old {
            if binding.revision != expected_revision {
                return Err(MigrationError::Revision);
            }
            if self.needs_reconcile(&loaded, binding) {
                return Err(MigrationError::ReconcileRequired);
            }
            if binding.operation_id == operation_id
                || !matches!(
                    (binding.phase, plan.direction),
                    (Phase::Ha, Direction::ToMoli) | (Phase::Moli, Direction::ToHa)
                )
            {
                return Err(MigrationError::Transition);
            }
        } else {
            if expected_revision != 0 {
                return Err(MigrationError::Revision);
            }
            if plan.direction != Direction::ToMoli {
                return Err(MigrationError::Transition);
            }
            if loaded
                .state
                .bindings
                .iter()
                .any(|binding| binding.native_device == plan.native_device)
            {
                return Err(MigrationError::Invalid);
            }
        }
        if loaded
            .state
            .bindings
            .iter()
            .any(|binding| binding.phase.active() && binding.operation_id == operation_id)
        {
            return Err(MigrationError::Proof);
        }
        let old_generation = old.map_or(0, |binding| binding.generation);
        let mut candidate = loaded.state.clone();
        // Closing is always allowed before an uncertain write; granting is never allowed here.
        self.0.policy.fence(&plan.native_device);
        let generation = self.next_generation(&plan.native_device, old_generation)?;
        let binding = Binding {
            key: plan.binding.clone(),
            native_device: plan.native_device.clone(),
            phase: match plan.direction {
                Direction::ToMoli => Phase::PreparedToMoli,
                Direction::ToHa => Phase::PreparedToHa,
            },
            revision: increment(expected_revision)?,
            generation,
            operation_id: operation_id.into(),
            plan_hash: plan_hash.into(),
            plan,
            reconcile_required: false,
        };
        if let Some(previous) = candidate
            .bindings
            .iter_mut()
            .find(|entry| entry.key == binding.key)
        {
            *previous = binding.clone();
        } else {
            candidate.bindings.push(binding.clone());
        }
        if let Err(error) = candidate.validate() {
            self.publish(&loaded);
            return Err(error);
        }
        self.commit(&mut loaded, candidate, &binding.key).await?;
        Ok(binding)
    }

    pub async fn acknowledge(
        &self,
        key: &BindingKey,
        proof: Proof,
    ) -> Result<Binding, MigrationError> {
        let registry = self.clone();
        let key = key.clone();
        tokio::spawn(async move { registry.acknowledge_inner(&key, proof).await })
            .await
            .map_err(|_| MigrationError::Interrupted)?
    }

    async fn acknowledge_inner(
        &self,
        key: &BindingKey,
        proof: Proof,
    ) -> Result<Binding, MigrationError> {
        let mut loaded = self.0.loaded.lock().await;
        if loaded.reload_required {
            return Err(MigrationError::ReloadRequired);
        }
        let current = loaded
            .state
            .bindings
            .iter()
            .find(|binding| &binding.key == key)
            .ok_or(MigrationError::Unknown)?;
        if self.needs_reconcile(&loaded, current) {
            return Err(MigrationError::ReconcileRequired);
        }
        validate_proof(current, &proof, &current.operation_id)?;
        if !self.0.policy.resolved(&current.plan) {
            return Err(MigrationError::Unresolved);
        }
        let phase = match (current.phase, proof.observed_owner) {
            (Phase::PreparedToMoli, Owner::None) => Phase::HaStopped,
            (Phase::HaStopped, Owner::Moli) => Phase::Moli,
            (Phase::PreparedToHa, Owner::None) => Phase::MoliStopped,
            (Phase::MoliStopped, Owner::Ha) => Phase::Ha,
            _ => return Err(MigrationError::Transition),
        };
        let mut binding = current.clone();
        binding.phase = phase;
        binding.revision = increment(current.revision)?;
        binding.generation = self.next_generation(&binding.native_device, current.generation)?;
        let mut candidate = loaded.state.clone();
        *candidate
            .bindings
            .iter_mut()
            .find(|entry| &entry.key == key)
            .ok_or(MigrationError::Unknown)? = binding.clone();
        self.commit(&mut loaded, candidate, key).await?;
        Ok(binding)
    }

    /// This challenge is bound to this process/reload and cannot be replayed after a crash.
    pub async fn reconcile_challenge(
        &self,
        key: &BindingKey,
    ) -> Result<ProofChallenge, MigrationError> {
        let loaded = self.0.loaded.lock().await;
        if loaded.reload_required {
            return Err(MigrationError::ReloadRequired);
        }
        let current = loaded
            .state
            .bindings
            .iter()
            .find(|binding| &binding.key == key)
            .ok_or(MigrationError::Unknown)?;
        if !self.needs_reconcile(&loaded, current) {
            return Err(MigrationError::Transition);
        }
        Ok(challenge(&loaded.session, current))
    }

    pub async fn reconcile(
        &self,
        key: &BindingKey,
        proof: Proof,
    ) -> Result<Binding, MigrationError> {
        let registry = self.clone();
        let key = key.clone();
        tokio::spawn(async move { registry.reconcile_inner(&key, proof).await })
            .await
            .map_err(|_| MigrationError::Interrupted)?
    }

    async fn reconcile_inner(
        &self,
        key: &BindingKey,
        proof: Proof,
    ) -> Result<Binding, MigrationError> {
        let mut loaded = self.0.loaded.lock().await;
        if loaded.reload_required {
            return Err(MigrationError::ReloadRequired);
        }
        let current = loaded
            .state
            .bindings
            .iter()
            .find(|binding| &binding.key == key)
            .ok_or(MigrationError::Unknown)?;
        if !self.needs_reconcile(&loaded, current) {
            return Err(MigrationError::Transition);
        }
        validate_proof(
            current,
            &proof,
            &challenge(&loaded.session, current).operation,
        )?;
        if !self.0.policy.resolved(&current.plan) {
            return Err(MigrationError::Unresolved);
        }
        let phase = match proof.observed_owner {
            Owner::Ha => Phase::Ha,
            Owner::Moli => Phase::Moli,
            Owner::None => match current.plan.direction {
                Direction::ToMoli => Phase::HaStopped,
                Direction::ToHa => Phase::MoliStopped,
            },
            Owner::Unknown => return Err(MigrationError::Proof),
        };
        let mut binding = current.clone();
        binding.phase = phase;
        binding.operation_id = proof.operation;
        binding.revision = increment(current.revision)?;
        binding.generation = self.next_generation(&binding.native_device, current.generation)?;
        let mut candidate = loaded.state.clone();
        *candidate
            .bindings
            .iter_mut()
            .find(|entry| &entry.key == key)
            .ok_or(MigrationError::Unknown)? = binding.clone();
        self.commit(&mut loaded, candidate, key).await?;
        Ok(binding)
    }

    /// Required after commit_unknown; reads the authoritative file before issuing a new challenge.
    pub async fn reload(&self) -> Result<(), MigrationError> {
        let registry = self.clone();
        tokio::spawn(async move {
            let mut loaded = registry.0.loaded.lock().await;
            registry.fence_all(&mut loaded, None);
            loaded.reload_required = true;
            registry.publish(&loaded);
            let state = registry
                .0
                .store
                .load()
                .await
                .map_err(|_| MigrationError::Load)?
                .ok_or(MigrationError::Load)?;
            state.validate()?;
            for binding in &state.bindings {
                registry.0.policy.fence(&binding.native_device);
            }
            loaded.fenced = state
                .bindings
                .iter()
                .map(|binding| binding.key.clone())
                .collect();
            loaded.state = state;
            loaded.session = model::sha256(loaded.session.as_bytes());
            loaded.reload_required = false;
            registry.publish(&loaded);
            Ok(())
        })
        .await
        .map_err(|_| MigrationError::Interrupted)?
    }

    fn needs_reconcile(&self, loaded: &Loaded, binding: &Binding) -> bool {
        loaded.fenced.contains(&binding.key)
            || loaded.reload_required
            || self
                .0
                .policy
                .current(&binding.native_device)
                .is_none_or(|authority| {
                    authority.generation != binding.generation
                        || authority.allowed != (binding.phase == Phase::Moli)
                })
    }

    fn next_generation(&self, device: &DeviceId, stored: u64) -> Result<u64, MigrationError> {
        increment(
            stored.max(
                self.0
                    .policy
                    .current(device)
                    .map_or(0, |authority| authority.generation),
            ),
        )
    }

    async fn commit(
        &self,
        loaded: &mut Loaded,
        candidate: State,
        key: &BindingKey,
    ) -> Result<(), MigrationError> {
        candidate.validate()?;
        if let Err(error) = self.0.store.write(&candidate).await {
            self.fence_all(loaded, Some(&candidate));
            loaded.reload_required |= error.commit_unknown;
            loaded.session = model::sha256(loaded.session.as_bytes());
            self.publish(loaded);
            return Err(MigrationError::Storage {
                commit_unknown: error.commit_unknown,
            });
        }
        loaded.state = candidate;
        let binding = loaded
            .state
            .bindings
            .iter()
            .find(|binding| &binding.key == key)
            .ok_or(MigrationError::Unknown)?;
        if let Err(error) = self.0.policy.set(
            &binding.native_device,
            binding.generation,
            binding.phase == Phase::Moli,
        ) {
            self.fence_all(loaded, None);
            loaded.session = model::sha256(loaded.session.as_bytes());
            self.publish(loaded);
            return Err(error);
        }
        loaded.fenced.remove(key);
        self.publish(loaded);
        Ok(())
    }

    fn fence_all(&self, loaded: &mut Loaded, candidate: Option<&State>) {
        for binding in loaded
            .state
            .bindings
            .iter()
            .chain(candidate.into_iter().flat_map(|state| &state.bindings))
        {
            self.0.policy.fence(&binding.native_device);
            loaded.fenced.insert(binding.key.clone());
        }
    }

    fn publish(&self, loaded: &Loaded) {
        let mut permits = BTreeMap::new();
        let bindings = loaded
            .state
            .bindings
            .iter()
            .map(|binding| {
                let mut view = binding.clone();
                view.reconcile_required = self.needs_reconcile(loaded, binding);
                if view.reconcile_required {
                    view.phase = Phase::ReconcileRequired;
                }
                permits.insert(
                    view.key.clone(),
                    Permit {
                        device: view.native_device.clone(),
                        revision: view.revision,
                        generation: view.generation,
                        allowed: view.native_owner_moli(),
                    },
                );
                view
            })
            .collect();
        *self
            .0
            .permits
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = permits;
        *self
            .0
            .snapshot
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Snapshot {
            schema: 1,
            bindings,
            active_operations: loaded.state.active(),
            suspended: loaded.reload_required,
        };
    }
}

fn increment(value: u64) -> Result<u64, MigrationError> {
    value.checked_add(1).ok_or(MigrationError::Exhausted)
}
fn random_session() -> Result<String, MigrationError> {
    let mut bytes = [0; 32];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| MigrationError::Interrupted)?;
    Ok(model::sha256(&bytes))
}
fn challenge(session: &str, binding: &Binding) -> ProofChallenge {
    let operation = model::sha256(
        format!(
            "{session}:{}:{}:{}",
            binding.operation_id, binding.plan_hash, binding.revision
        )
        .as_bytes(),
    );
    ProofChallenge {
        operation,
        plan_hash: binding.plan_hash.clone(),
        writer_scope: binding.key.writer_scope.clone(),
        revision: binding.revision,
    }
}
fn validate_proof(binding: &Binding, proof: &Proof, operation: &str) -> Result<(), MigrationError> {
    if proof.revision != binding.revision {
        return Err(MigrationError::Revision);
    }
    if proof.plan_hash != binding.plan_hash {
        return Err(MigrationError::PlanHash);
    }
    if !proof.quiescent
        || proof.operation != operation
        || proof.writer_scope != binding.key.writer_scope
        || proof.observed_owner == Owner::Unknown
    {
        return Err(MigrationError::Proof);
    }
    Ok(())
}
