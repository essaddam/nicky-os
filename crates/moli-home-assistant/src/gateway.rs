use crate::{
    BridgeError, Config, Frame,
    inventory::Catalog,
    protocol::{
        self, Availability, CoverAction, CoverState, ExportCover, Exports, MigrationRequest,
        NativeCommand, ProofMessage, ResultCode, ResultMessage,
    },
};
use moli_core::{Kind, Origin, Value};
use moli_migration::{Binding, BindingKey, MigrationPlan, ProofChallenge, Registry};
use moli_runtime::{CommandError, Hub};
use serde_json::{Value as Json, json};
use std::{
    collections::BTreeSet,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

struct Inner {
    hub: Hub,
    registry: Registry,
    config: Config,
    session: RwLock<String>,
    ha_session: RwLock<Option<String>>,
    connected: AtomicBool,
    ha_online: AtomicBool,
    catalog: RwLock<Option<Arc<Catalog>>>,
    receipts: Mutex<BTreeSet<String>>,
    outgoing: mpsc::Sender<Frame>,
    incoming: tokio::sync::Mutex<mpsc::Receiver<Frame>>,
    migration_status: RwLock<Option<Json>>,
    ha_last_seen: RwLock<Option<Instant>>,
}

#[derive(Clone)]
pub struct Gateway(Arc<Inner>);
impl std::fmt::Debug for Gateway {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HomeAssistantGateway")
            .field("installation", &self.0.config.installation)
            .finish_non_exhaustive()
    }
}

impl Gateway {
    pub fn new(config: &Config, hub: Hub, registry: Registry) -> Result<Self, BridgeError> {
        let config = config.clone().validate()?;
        for instance in &config.native_instances {
            hub.manage_write_instance(instance);
        }
        let (outgoing, incoming) = mpsc::channel(16);
        Ok(Self(Arc::new(Inner {
            hub,
            registry,
            config,
            session: RwLock::new(crate::request_id()?),
            ha_session: RwLock::new(None),
            connected: AtomicBool::new(false),
            ha_online: AtomicBool::new(false),
            catalog: RwLock::new(None),
            receipts: Mutex::new(BTreeSet::new()),
            outgoing,
            incoming: tokio::sync::Mutex::new(incoming),
            migration_status: RwLock::new(None),
            ha_last_seen: RwLock::new(None),
        })))
    }
    pub fn session(&self) -> String {
        self.0
            .session
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
    pub fn renew_session(&self) -> Result<String, BridgeError> {
        self.connected(false);
        let session = crate::request_id()?;
        self.0
            .session
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone_from(&session);
        self.0
            .receipts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        let mut rx = self.0.incoming.try_lock().map_err(|_| BridgeError::Busy)?;
        while rx.try_recv().is_ok() {}
        Ok(session)
    }
    pub fn connected(&self, connected: bool) {
        self.0.connected.store(connected, Ordering::Release);
        if !connected {
            self.0.ha_online.store(false, Ordering::Release);
            *self
                .0
                .ha_last_seen
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
            *self
                .0
                .ha_session
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        }
    }
    pub fn ha_session(&self) -> Option<String> {
        let recent = self
            .0
            .ha_last_seen
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some_and(|seen| seen.elapsed() < Duration::from_secs(15));
        if !self.0.ha_online.load(Ordering::Acquire) || !recent {
            return None;
        }
        self.0
            .ha_session
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
    pub fn peer_update(
        &self,
        availability: Availability,
        retained: bool,
    ) -> Result<(), BridgeError> {
        availability.validate(&self.0.config.installation, &availability.session)?;
        if retained {
            return Ok(());
        }
        *self
            .0
            .ha_last_seen
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            availability.online.then(Instant::now);
        *self
            .0
            .ha_session
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(availability.session);
        self.0
            .ha_online
            .store(availability.online, Ordering::Release);
        Ok(())
    }
    pub fn catalogue_update(&self, catalog: &Catalog) -> Result<(), BridgeError> {
        catalog.validate_installation(&self.0.config.installation)?;
        let mut current = self
            .0
            .catalog
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(previous) = current.as_ref()
            && (catalog.revision < previous.revision
                || (catalog.revision == previous.revision
                    && serde_json::to_vec(catalog)? != serde_json::to_vec(previous.as_ref())?))
        {
            return Err(BridgeError::Conflict);
        }
        *current = Some(Arc::new(catalog.clone()));
        Ok(())
    }
    pub(crate) async fn next_frame(&self) -> Option<Frame> {
        self.0.incoming.lock().await.recv().await
    }
    fn enqueue<T: serde::Serialize>(&self, suffix: &str, message: &T) -> Result<(), BridgeError> {
        if !self.0.connected.load(Ordering::Acquire) {
            return Err(BridgeError::Unavailable);
        }
        let payload = serde_json::to_vec(message)?;
        if payload.len() > protocol::MAX_CATALOGUE_BYTES {
            return Err(BridgeError::Configuration);
        }
        self.0
            .outgoing
            .try_send(Frame {
                suffix: suffix.into(),
                payload,
                retained: false,
            })
            .map_err(|_| BridgeError::Busy)
    }
    pub fn source_owned_by_ha(&self, entity: &str) -> std::future::Ready<bool> {
        std::future::ready(
            self.0
                .registry
                .source_owned_by_ha(&self.0.config.installation, entity),
        )
    }
    pub fn source_generation(&self, entity: &str) -> u64 {
        self.0
            .registry
            .source_generation(&self.0.config.installation, entity)
            .unwrap_or(0)
    }
    pub fn source_command_wanted(&self, entity: &str, generation: u64) -> bool {
        self.0
            .registry
            .source_owned_by_ha(&self.0.config.installation, entity)
            && self.source_generation(entity) == generation
    }
    fn actions(&self, binding: &Binding) -> Vec<CoverAction> {
        self.0
            .hub
            .describe(&binding.native_device)
            .and_then(|(device, _)| {
                let (_, key) = binding.key.target.split()?;
                let point = device.point(key)?;
                if !point.access.write {
                    return None;
                }
                match &point.kind {
                    Kind::Enum { values } => Some(
                        values
                            .iter()
                            .filter_map(|value| CoverAction::from_core_str(value))
                            .collect(),
                    ),
                    _ => None,
                }
            })
            .unwrap_or_default()
    }
    fn check_key(&self, key: &BindingKey) -> Result<(), BridgeError> {
        protocol::validate_binding(key, &self.0.config.installation)?;
        if !self.0.config.native_instances.contains(
            &key.target
                .split()
                .ok_or(BridgeError::Configuration)?
                .0
                .instance(),
        ) {
            return Err(BridgeError::Unsupported);
        }
        Ok(())
    }
    pub async fn exports(&self) -> Result<Exports, BridgeError> {
        let snapshot = self.0.registry.snapshot().await;
        let items = snapshot
            .bindings
            .into_iter()
            .filter(|b| b.key.ha_installation == self.0.config.installation)
            .map(|binding| {
                let label = self.0.hub.describe(&binding.native_device).map_or_else(
                    || binding.key.registry_entity_id.clone(),
                    |(device, label)| label.name.unwrap_or_else(|| device.native_name.to_string()),
                );
                ExportCover {
                    actions: self.actions(&binding),
                    binding: binding.key,
                    label,
                    revision: binding.revision,
                    generation: binding.generation,
                    feedback: false,
                }
            })
            .collect();
        let exports = Exports {
            v: protocol::VERSION,
            installation: self.0.config.installation.clone(),
            session: self.session(),
            items,
        };
        exports.validate(&self.0.config.installation, &self.session())?;
        Ok(exports)
    }
    pub async fn states(&self) -> Result<Vec<CoverState>, BridgeError> {
        Ok(self
            .0
            .registry
            .snapshot()
            .await
            .bindings
            .into_iter()
            .filter(|b| b.key.ha_installation == self.0.config.installation)
            .map(|binding| {
                let available = binding.native_owner_moli()
                    && self.0.registry.check_can_write(
                        &binding.key,
                        binding.revision,
                        binding.generation,
                    )
                    && self
                        .0
                        .hub
                        .device(&binding.native_device)
                        .is_some_and(|device| device.online == Some(true));
                CoverState {
                    v: protocol::VERSION,
                    installation: self.0.config.installation.clone(),
                    binding: binding.key,
                    revision: binding.revision,
                    generation: binding.generation,
                    available,
                    feedback: false,
                    position: None,
                    closed: None,
                }
            })
            .collect())
    }
    pub async fn native_command(&self, command: NativeCommand, retained: bool) -> ResultMessage {
        let request_id = command.request_id.clone();
        let code = self.native_command_inner(command, retained).await;
        ResultMessage {
            v: protocol::VERSION,
            installation: self.0.config.installation.clone(),
            request_id,
            code,
        }
    }
    async fn native_command_inner(&self, command: NativeCommand, retained: bool) -> ResultCode {
        let now = crate::now_ms();
        if retained
            || command
                .validate(&self.0.config.installation, &self.session(), now)
                .is_err()
        {
            return ResultCode::Rejected;
        }
        if !self.0.connected.load(Ordering::Acquire)
            || self.ha_session().is_none()
            || self.check_key(&command.binding).is_err()
        {
            return ResultCode::Rejected;
        }
        {
            let mut receipts = self
                .0
                .receipts
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if receipts.len() >= 256 || !receipts.insert(command.request_id.clone()) {
                return ResultCode::Rejected;
            }
        }
        let Some(binding) = self.0.registry.get(&command.binding).await else {
            return ResultCode::Rejected;
        };
        if !self
            .0
            .registry
            .check_can_write(&command.binding, command.revision, command.generation)
            || !self.actions(&binding).contains(&command.action)
        {
            return ResultCode::Rejected;
        }
        let remaining = command.expires_at_ms.saturating_sub(crate::now_ms());
        if remaining == 0 || remaining > protocol::MAX_COMMAND_TTL_MS {
            return ResultCode::Expired;
        }
        let deadline = Instant::now() + Duration::from_millis(remaining);
        let permit = self.0.registry.command_permit(
            command.binding.clone(),
            command.revision,
            command.generation,
            deadline,
        );
        let result = tokio::time::timeout_at(
            deadline.into(),
            self.0.hub.command_with_permit(
                &command.binding.target,
                Value::Text(command.action.as_str().into()),
                Origin::Api,
                Some(format!("home-assistant:{}", self.0.config.installation)),
                None,
                permit,
            ),
        )
        .await;
        match result {
            Ok(Ok(())) => ResultCode::Accepted,
            Ok(Err(CommandError::NeedsApproval { .. })) => ResultCode::ApprovalRequired,
            Ok(Err(CommandError::Timeout | CommandError::Driver(_))) | Err(_) => {
                ResultCode::Uncertain
            }
            Ok(Err(_)) => ResultCode::Rejected,
        }
    }
    pub async fn proof(&self, message: ProofMessage, retained: bool) -> Result<(), BridgeError> {
        if retained || !self.0.connected.load(Ordering::Acquire) || self.ha_session().is_none() {
            return Err(BridgeError::Unavailable);
        }
        message.validate(&self.0.config.installation, &self.session())?;
        self.check_key(&message.binding)?;
        let result = self
            .0
            .registry
            .acknowledge(&message.binding, message.proof.clone())
            .await;
        match result {
            Ok(_) => Ok(()),
            Err(moli_migration::MigrationError::ReconcileRequired) => {
                self.0
                    .registry
                    .reconcile(&message.binding, message.proof)
                    .await?;
                Ok(())
            }
            Err(error) => Err(error.into()),
        }
    }
    pub async fn snapshot(&self) -> Json {
        let catalog = self
            .0
            .catalog
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let status = self
            .0
            .migration_status
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        json!({"installation":self.0.config.installation,"base_topic":self.0.config.base_topic,"connected":self.0.connected.load(Ordering::Acquire),"ha_online":self.ha_session().is_some(),"catalogue_revision":catalog.as_ref().map(|c|c.revision),"catalogue":catalog,"migration":self.0.registry.snapshot().await,"migration_status":status})
    }
    pub fn migration_status(&self, bytes: &[u8], retained: bool) -> Result<(), BridgeError> {
        if retained {
            return Err(BridgeError::Unavailable);
        }
        let session = self.ha_session().ok_or(BridgeError::Unavailable)?;
        let value: Json = protocol::decode(bytes, protocol::MAX_COMMAND_BYTES)?;
        if value["installation"] != self.0.config.installation
            || value["v"] != 1
            || value["session"].as_str() != Some(session.as_str())
            || !matches!(value["status"].as_str(), Some("blocked" | "refused"))
            || !value["operation"].as_str().is_some_and(protocol::valid_id)
        {
            return Err(BridgeError::Configuration);
        }
        *self
            .0
            .migration_status
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(
            json!({"operation":value["operation"],"status":value["status"],"reason":"quiescence_not_proven"}),
        );
        Ok(())
    }
    pub async fn prepare(
        &self,
        plan: MigrationPlan,
        expected_revision: u64,
        operation_id: &str,
        plan_hash: &str,
    ) -> Result<Binding, BridgeError> {
        self.check_key(&plan.binding)?;
        let catalog = self
            .0
            .catalog
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .ok_or(BridgeError::Unavailable)?;
        if !catalog
            .devices
            .iter()
            .flat_map(|device| &device.channels)
            .any(|point| {
                point.registry_entity_id == plan.binding.registry_entity_id
                    && !point.actions.is_empty()
            })
        {
            return Err(BridgeError::Unsupported);
        }
        let existing = self.0.registry.snapshot().await;
        if existing.bindings.iter().any(|b| {
            b.key != plan.binding
                && b.key.ha_installation == plan.binding.ha_installation
                && b.key.registry_entity_id == plan.binding.registry_entity_id
        }) {
            return Err(BridgeError::Conflict);
        }
        self.0
            .registry
            .prepare(plan, expected_revision, operation_id, plan_hash)
            .await
            .map_err(Into::into)
    }
    fn migration_frame(&self, binding: Binding) -> Result<(), BridgeError> {
        let session = self.ha_session().ok_or(BridgeError::Unavailable)?;
        let message = MigrationRequest {
            v: protocol::VERSION,
            installation: self.0.config.installation.clone(),
            session,
            expires_at_ms: crate::now_ms().saturating_add(protocol::MAX_COMMAND_TTL_MS),
            binding,
        };
        self.enqueue("moli/migration/request", &message)
    }
    pub async fn request_step(
        &self,
        key: &BindingKey,
        expected_revision: u64,
    ) -> Result<Binding, BridgeError> {
        self.check_key(key)?;
        let binding = self
            .0
            .registry
            .get(key)
            .await
            .ok_or(BridgeError::Unsupported)?;
        if binding.revision != expected_revision {
            return Err(BridgeError::Conflict);
        }
        self.migration_frame(binding.clone())?;
        Ok(binding)
    }
    pub async fn request_reconcile(&self, key: &BindingKey) -> Result<ProofChallenge, BridgeError> {
        self.check_key(key)?;
        let challenge = self.0.registry.reconcile_challenge(key).await?;
        let mut binding = self
            .0
            .registry
            .get(key)
            .await
            .ok_or(BridgeError::Unsupported)?;
        binding.operation_id.clone_from(&challenge.operation);
        self.migration_frame(binding)?;
        Ok(challenge)
    }
    pub async fn reload(&self) -> Result<(), BridgeError> {
        self.0.registry.reload().await.map_err(Into::into)
    }
}
