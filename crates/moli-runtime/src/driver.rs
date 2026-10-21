//! The contract between the core and a driver.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::Instant;

use moli_core::{Device, DeviceId, DriverStatus, InstanceId, Origin, PointId, RequestId, Value};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::Hub;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Trusted transport context, assigned by the hub rather than the caller's body.
#[derive(Clone, Debug)]
pub struct CommandMetadata {
    pub origin: Origin,
    pub actor: Option<String>,
    pub request_id: Option<RequestId>,
    pub correlation: Option<String>,
}

/// A driver connects one external system to the hub.
///
/// `run` is expected to run forever. Returning — with `Ok` or `Err` — or
/// panicking makes the supervisor restart it with backoff, reusing the same
/// [`DriverCtx`] so queued commands survive restarts.
pub trait Driver: Send + Sync + 'static {
    fn kind(&self) -> &'static str;
    /// The catalogue package it runs (`integrations/<id>/`): a native
    /// driver's own kind; the generic profile driver says which profile.
    fn integration(&self) -> Arc<str> {
        Arc::from(self.kind())
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>>;
}

type CommandReply = Arc<std::sync::Mutex<Option<oneshot::Sender<Result<(), crate::CommandError>>>>>;

/// A live authorization checked again until the transport sends an order.
#[derive(Clone)]
pub struct CommandPermit(Arc<dyn Fn(&PointId) -> bool + Send + Sync>);

impl std::fmt::Debug for CommandPermit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommandPermit").finish_non_exhaustive()
    }
}

impl CommandPermit {
    pub fn new(check: impl Fn(&PointId) -> bool + Send + Sync + 'static) -> Self {
        Self(Arc::new(check))
    }
    pub(crate) fn allows(&self, point: &PointId) -> bool {
        (self.0)(point)
    }
}

/// A validated write addressed to a driver. The driver must answer through
/// [`CommandRequest::reply`] once the order has been handed to the device
/// (state confirmation arrives later as a regular state update).
#[derive(Debug)]
pub struct CommandRequest {
    pub point: PointId,
    pub device: Arc<Device>,
    pub key: Arc<str>,
    /// Canonical value, already validated against the point spec.
    pub value: Value,
    /// When the hub stopped waiting: an order past it must never run (a
    /// lamp switched on hours after « failed » is worse than a refusal).
    pub deadline: Instant,
    pub metadata: CommandMetadata,
    pub(crate) permit: Option<CommandPermit>,
    pub(crate) reply: CommandReply,
    /// Who has the order: still queued, taken by the driver, or given up by
    /// its caller. Decided once, atomically (see [`Claim`]).
    pub(crate) claim: Arc<AtomicU8>,
}

/// The states of [`CommandRequest::claim`]. An order is either taken by its
/// driver or given up while queued, never both: a caller that leaves while
/// its order waits cancels it. One that leaves after the driver took it
/// marks it `LEFT`: a driver that has not sent it yet sees it abandoned
/// (checked right before sending), and the outcome is still journaled.
pub(crate) struct Claim;

impl Claim {
    pub(crate) const QUEUED: u8 = 0;
    pub(crate) const TAKEN: u8 = 1;
    pub(crate) const GIVEN_UP: u8 = 2;
    pub(crate) const LEFT: u8 = 3;

    /// Moves a queued order to `to`; false if it was already decided.
    pub(crate) fn settle(claim: &AtomicU8, to: u8) -> bool {
        claim
            .compare_exchange(Self::QUEUED, to, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    /// The caller of a taken order left.
    pub(crate) fn leave(claim: &AtomicU8) {
        let _ =
            claim.compare_exchange(Self::TAKEN, Self::LEFT, Ordering::AcqRel, Ordering::Acquire);
    }
}

/// A command's live authorization, retained by its network task until emission.
#[derive(Clone, Debug)]
pub struct SendPermit {
    point: PointId,
    permit: Option<CommandPermit>,
    reply: CommandReply,
    claim: Arc<AtomicU8>,
    deadline: Instant,
    emitted: Arc<AtomicBool>,
}

impl SendPermit {
    #[must_use]
    pub fn wanted(&self) -> bool {
        command_wanted(
            &self.reply,
            &self.claim,
            self.deadline,
            &self.point,
            self.permit.as_ref(),
        )
    }

    /// Conservative emission boundary: no await may separate this from
    /// handing the request to the transport. After it, retain the outcome.
    #[must_use]
    pub fn begin_send(&self) -> bool {
        if !self.wanted() {
            return false;
        }
        self.emitted.store(true, Ordering::Release);
        true
    }

    #[must_use]
    pub fn emitted(&self) -> bool {
        self.emitted.load(Ordering::Acquire)
    }

    #[must_use]
    pub fn deadline(&self) -> Instant {
        self.deadline
    }
}

// Both the driver's request and its detached transport enforce the same
// live policy, caller lifetime and deadline without an extra allocation.
fn command_wanted(
    reply: &CommandReply,
    claim: &AtomicU8,
    deadline: Instant,
    point: &PointId,
    permit: Option<&CommandPermit>,
) -> bool {
    Instant::now() < deadline
        && claim.load(Ordering::Acquire) != Claim::LEFT
        && permit.is_none_or(|permit| permit.allows(point))
        && reply
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .is_some_and(|reply| !reply.is_closed())
        // Policy evaluation or response-lock contention may cross the deadline.
        && Instant::now() < deadline
}

impl CommandRequest {
    /// Nobody waits for the answer any more: the hub timed out, or the
    /// caller gave up. A driver checks it right before each step that sends
    /// the order (connecting, writing): an order its caller renounced before
    /// it left must not leave.
    #[must_use]
    pub fn abandoned(&self) -> bool {
        !command_wanted(
            &self.reply,
            &self.claim,
            self.deadline,
            &self.point,
            self.permit.as_ref(),
        )
    }

    /// Authorization bounded by an earlier deadline chosen by the driver.
    #[must_use]
    pub fn send_permit(&self, deadline: Instant) -> SendPermit {
        SendPermit {
            point: self.point.clone(),
            permit: self.permit.clone(),
            reply: Arc::clone(&self.reply),
            claim: Arc::clone(&self.claim),
            deadline: deadline.min(self.deadline),
            emitted: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Delegates live authorization to this composite's declared members.
    /// The policy still evaluates the parent's point: a roomless physical
    /// bulb is controlled through its fixture, not by a new guest grant.
    /// This lease can outlive the parent's reply for a bounded follow-up.
    #[must_use]
    pub fn member_permit(&self) -> Option<CommandPermit> {
        let permit = self.permit.clone()?;
        let parent = self.point.clone();
        let members = self.device.members.clone();
        Some(CommandPermit::new(move |child| {
            child
                .split()
                .is_some_and(|(device, _)| members.contains(&device))
                && permit.allows(&parent)
        }))
    }

    /// Reports the outcome. Dropping the request without replying is
    /// reported to the caller as a driver failure.
    pub fn reply(self, result: Result<(), String>) {
        self.reply_result(result.map_err(crate::CommandError::Driver));
    }

    /// Returns a structured business-command error without losing its HTTP class.
    pub fn reply_result(self, result: Result<(), crate::CommandError>) {
        if let Some(reply) = self
            .reply
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            let _ = reply.send(result);
        }
    }
}

/// A driver's view of the hub, scoped to its own instance: a driver can only
/// publish and modify the devices it owns.
#[derive(Debug)]
pub struct DriverCtx {
    hub: Hub,
    instance: InstanceId,
    commands: mpsc::Receiver<CommandRequest>,
    cancel: CancellationToken,
}

impl DriverCtx {
    pub(crate) fn new(
        hub: Hub,
        instance: InstanceId,
        commands: mpsc::Receiver<CommandRequest>,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            hub,
            instance,
            commands,
            cancel,
        }
    }

    #[must_use]
    pub fn instance(&self) -> &InstanceId {
        &self.instance
    }

    /// Replaces this calendar's ephemeral access windows. Invalid input closes them.
    pub fn replace_reservations(
        &self,
        windows: Vec<(String, moli_core::AccessWindow)>,
        expires: u64,
    ) -> bool {
        self.hub
            .replace_reservations(&self.instance, windows, expires)
    }

    /// Builds a device id owned by this instance.
    #[must_use]
    pub fn device_id(&self, native: &str) -> DeviceId {
        DeviceId::new(&self.instance, native)
    }

    /// Signals that the driver is connected and operational.
    pub fn ready(&self) {
        self.hub
            .set_driver_status(&self.instance, DriverStatus::Running);
    }

    /// Signals that the driver is blocked on a human action (shown on every
    /// surface, so an agent can relay it). `ready()` clears it.
    pub fn wait_for(&self, reason: impl Into<String>) {
        self.hub.set_driver_status(
            &self.instance,
            DriverStatus::Waiting {
                reason: reason.into(),
            },
        );
    }

    /// Whether credentials obtained at runtime can be persisted.
    #[must_use]
    pub fn can_store_secrets(&self) -> bool {
        self.hub.has_secret_store()
    }

    /// A secret previously stored by this instance.
    #[must_use]
    pub fn secret(&self, name: &str) -> Option<String> {
        self.hub.secret(&self.instance, name)
    }

    /// Persists a secret for this instance (encrypted at rest).
    pub async fn store_secret(&self, name: &str, value: &str) -> std::io::Result<()> {
        self.hub.store_secret(&self.instance, name, value).await
    }

    /// A capability for owned background work, scoped to this driver instance.
    #[must_use]
    pub fn scoped_secrets(&self) -> ScopedSecrets {
        ScopedSecrets {
            hub: self.hub.clone(),
            instance: self.instance.clone(),
        }
    }

    /// Deletes a secret of this instance (revoked key…).
    pub async fn forget_secret(&self, name: &str) -> std::io::Result<bool> {
        self.hub.forget_secret(&self.instance, name).await
    }

    /// Why secrets cannot be stored, if they cannot (human-readable).
    #[must_use]
    pub fn secrets_problem(&self) -> Option<&str> {
        self.hub.secrets_problem()
    }

    /// Records an occurrence (button press…): broadcast even when the value
    /// equals the previous one, because each one is a new event.
    pub fn pulse_state(&self, device: &DeviceId, key: &str, value: Value) {
        if self.owns(device) {
            self.hub.pulse_state(device, key, value);
        }
    }

    /// Loads a timer from its authoritative business state.
    pub async fn initialize_timer(
        &self,
        device: &DeviceId,
        seconds: u32,
    ) -> Result<(), crate::TimerError> {
        if !self.owns(device) {
            return Err(crate::TimerError::Definition);
        }
        self.hub.initialize_timer(device, seconds).await
    }

    pub async fn timer_command(&self, command: CommandRequest) {
        if self.owns(&command.device.id) {
            self.hub.timer_command(command, self.cancel.clone()).await;
        } else {
            command.reply_result(Err(crate::TimerError::Definition.into()));
        }
    }

    pub async fn initialize_temporal(
        &self,
        device: &DeviceId,
        spec: moli_core::TemporalSpec,
        initial: Value,
    ) -> Result<(), crate::TemporalError> {
        if !self.owns(device) {
            return Err(crate::TemporalError::Definition);
        }
        self.hub.initialize_temporal(device, spec, initial).await
    }
    pub async fn temporal_command(&self, command: CommandRequest) {
        if !self.owns(&command.device.id) {
            command.reply_result(Err(crate::TemporalError::Definition.into()));
            return;
        }
        self.hub
            .temporal_command(command, self.cancel.clone())
            .await;
    }

    /// Loads the validated definition and initializes a calculated projection.
    pub async fn initialize_computed(
        &self,
        device: &DeviceId,
        spec: moli_core::AggregateSpec,
    ) -> Result<(), crate::DependencyError> {
        if !self.owns(device) {
            return Err(crate::DependencyError::Definition);
        }
        self.hub.initialize_computed(device, spec).await
    }

    /// Loads a schedule's draft/current heads from their authoritative file.
    pub async fn initialize_schedule(
        &self,
        device: &DeviceId,
        plan: moli_core::WeeklySchedule,
    ) -> Result<(), crate::DependencyError> {
        if !self.owns(device) {
            return Err(crate::DependencyError::Definition);
        }
        self.hub.initialize_schedule(device, plan).await
    }

    /// Loads and publishes a counter from its authoritative business state.
    pub async fn initialize_counter(
        &self,
        device: &DeviceId,
        spec: moli_core::CounterSpec,
    ) -> Result<(), crate::CounterError> {
        if !self.owns(device) {
            return Err(crate::CounterError::Definition);
        }
        self.hub.initialize_counter(device, spec).await
    }

    /// Hands an explicit counter operation to the shared, cancellation-safe owner.
    pub async fn counter_command(&self, command: CommandRequest) {
        if !self.owns(&command.device.id) {
            command.reply_result(Err(crate::CounterError::Definition.into()));
            return;
        }
        self.hub.counter_command(command, self.cancel.clone()).await;
    }

    /// Publishes a virtual button command once, after its business-state commit.
    pub async fn press_button(&self, command: &CommandRequest) -> Result<(), crate::CommandError> {
        if !self.owns(&command.device.id) {
            return Err(crate::OccurrenceError::Source.into());
        }
        self.hub.press_button(command).await
    }

    /// Publishes or replaces a device. Devices of other instances are refused.
    pub fn upsert_device(&self, device: Device) {
        if self.owns(&device.id) && device.instance == self.instance {
            self.hub.upsert_device(device);
        } else {
            tracing::warn!(instance = %self.instance, device = %device.id, "refused foreign device");
        }
    }

    /// Something that publishes media for devices to fetch, usable from a
    /// task the driver spawns (the context itself stays in its loop).
    #[must_use]
    pub fn media_publisher(&self) -> MediaPublisher {
        MediaPublisher(self.hub.clone())
    }

    /// The home's assistant, for a voice satellite (once the binary set it).
    #[must_use]
    pub fn voice_brain(&self) -> Option<std::sync::Arc<dyn crate::voice::VoiceBrain>> {
        self.hub.voice_brain()
    }

    /// Declares that `device` (one of ours) can produce images on demand.
    pub fn provide_snapshots(
        &self,
        device: &DeviceId,
        source: Arc<dyn crate::media::SnapshotSource>,
    ) {
        if self.owns(device) {
            self.hub.provide_snapshots(device, source);
        }
    }

    /// Declares that `device` (one of ours) prints documents.
    pub fn provide_printing(&self, device: &DeviceId, sink: Arc<dyn crate::media::PrintSink>) {
        if self.owns(device) {
            self.hub.provide_printing(device, sink);
        }
    }

    pub fn remove_device(&self, id: &DeviceId) {
        if self.owns(id) {
            self.hub.remove_device(id);
        }
    }

    /// Ids of the devices currently published by this instance.
    #[must_use]
    pub fn devices(&self) -> Vec<DeviceId> {
        self.hub.device_ids_of(&self.instance)
    }

    /// Ids of the devices this instance published, now or before a restart
    /// (who to look for before the device answered once in this run).
    #[must_use]
    pub fn known_devices(&self) -> Vec<DeviceId> {
        self.hub.known_device_ids_of(&self.instance)
    }

    /// Keys declared or cached for one of this instance's devices.
    #[must_use]
    pub fn known_point_keys(&self, device: &DeviceId) -> Vec<Arc<str>> {
        if self.owns(device) {
            self.hub.known_point_keys(device)
        } else {
            Vec::new()
        }
    }

    /// Where this instance last reached its device.
    #[must_use]
    pub fn known_address(&self) -> Option<crate::KnownAddress> {
        self.hub.known_address(&self.instance)
    }

    /// Remembers where the device answered (written only when it changed).
    pub async fn remember_address(&self, address: crate::KnownAddress) {
        if let Err(e) = self.hub.remember_address(&self.instance, address).await {
            tracing::warn!(instance = %self.instance, error = %e, "address not saved");
        }
    }

    /// The device answers at another address now: journaled for the people
    /// of the house, remembered for the next start.
    pub async fn moved(&self, device: Option<DeviceId>, from: &str, address: crate::KnownAddress) {
        let device = device.filter(|d| self.owns(d));
        self.hub
            .record_address(&self.instance, device, from, &address.host);
        self.remember_address(address).await;
    }

    /// The current value of one of this instance's points (restored from
    /// the state cache after a restart), if any.
    #[must_use]
    pub fn current(&self, device: &DeviceId, key: &str) -> Option<Value> {
        if !self.owns(device) {
            return None;
        }
        self.hub
            .state(&PointId::new(device, key))
            .map(|sample| sample.value)
    }

    /// Hands a keypad input directly to the sole internal consumer, never the bus.
    pub fn submit_keypad_input(&self, input: crate::KeypadInput) -> bool {
        if !self.owns(&input.device) {
            self.hub.reject_keypad_input();
            return false;
        }
        self.hub.submit_keypad_input(input)
    }
    pub fn reject_keypad_input(&self) {
        self.hub.reject_keypad_input();
    }

    /// Records a value for a point of one of this instance's devices.
    pub fn set_state(&self, device: &DeviceId, key: &str, value: Value) {
        if self.owns(device) {
            self.hub.set_state(device, key, value);
        }
    }

    pub fn set_availability(&self, device: &DeviceId, online: bool) {
        if self.owns(device) {
            self.hub.set_availability(device, online);
        }
    }

    /// Next command for this driver. Cancel-safe; `None` once shut down.
    /// Orders nobody waits for any more (past their deadline) are refused
    /// here and never reach the driver.
    pub async fn next_command(&mut self) -> Option<CommandRequest> {
        loop {
            let command = tokio::select! {
                () = self.cancel.cancelled() => None,
                cmd = self.commands.recv() => cmd,
            }?;
            if !command.abandoned() && Claim::settle(&command.claim, Claim::TAKEN) {
                return Some(command);
            }
            tracing::warn!(instance = %self.instance, point = %command.point, "stale order dropped");
            command.reply(Err("expired".into()));
        }
    }

    /// Refuses every queued order: the driver is restarting, and nothing
    /// queued now may run later by surprise.
    pub(crate) fn drain(&mut self, why: &str) {
        while let Ok(command) = self.commands.try_recv() {
            command.reply(Err(why.to_owned()));
        }
    }

    /// Resolves when the system is shutting down.
    pub async fn cancelled(&self) {
        self.cancel.cancelled().await;
    }

    fn owns(&self, id: &DeviceId) -> bool {
        id.instance() == self.instance
    }
}

/// The same secret namespace as a `DriverCtx`, usable without blocking its loop.
#[derive(Clone)]
pub struct ScopedSecrets {
    hub: Hub,
    instance: InstanceId,
}

impl std::fmt::Debug for ScopedSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScopedSecrets")
            .field("instance", &self.instance)
            .finish_non_exhaustive()
    }
}

impl ScopedSecrets {
    #[must_use]
    pub fn available(&self) -> bool {
        self.hub.has_secret_store()
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<String> {
        self.hub.secret(&self.instance, name)
    }

    /// # Errors
    /// An unavailable or failing vault never becomes a successful write.
    pub async fn store(&self, name: &str, value: &str) -> std::io::Result<()> {
        self.hub.store_secret(&self.instance, name, value).await
    }
}

#[cfg(test)]
mod scoped_secret_tests {
    use super::*;
    use crate::{HubOptions, MasterKey};

    #[tokio::test]
    async fn scoped_secret_writer_persists_only_its_own_namespace_after_context_drop() {
        let dir = std::env::temp_dir().join(format!("moli-scoped-secrets-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("secrets.enc");
        let options = HubOptions {
            secrets: Some((path.clone(), MasterKey::new("42".repeat(32)))),
            ..HubOptions::default()
        };
        let hub = Hub::new(options.clone()).unwrap();
        hub.store_secret(&InstanceId::from("beta"), "identity", "test-beta")
            .await
            .unwrap();
        let (_, commands) = mpsc::channel(1);
        let ctx = DriverCtx::new(
            hub.clone(),
            InstanceId::from("alpha"),
            commands,
            CancellationToken::new(),
        );
        let writer = ctx.scoped_secrets();
        drop(ctx);
        assert!(writer.available());
        tokio::spawn(async move { writer.store("identity", "test-alpha").await })
            .await
            .unwrap()
            .unwrap();
        let reopened = Hub::new(options).unwrap();
        assert_eq!(
            reopened
                .secret(&InstanceId::from("alpha"), "identity")
                .as_deref(),
            Some("test-alpha")
        );
        assert_eq!(
            reopened
                .secret(&InstanceId::from("beta"), "identity")
                .as_deref(),
            Some("test-beta")
        );
        assert!(
            !std::fs::read(&path)
                .unwrap()
                .windows(b"test-alpha".len())
                .any(|bytes| bytes == b"test-alpha")
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// Publishes short-lived media (see [`Hub::publish_media`]).
#[derive(Clone, Debug)]
pub struct MediaPublisher(Hub);

impl MediaPublisher {
    /// The name under which `/api/media/<name>` serves it for a few minutes.
    #[must_use]
    /// `None` only if the system could not draw a random name.
    pub fn publish(&self, extension: &str, image: crate::media::Image) -> Option<String> {
        self.0.publish_media(extension, image)
    }

    /// Tells the people at home that something the driver did in the
    /// background did not happen (the command itself already answered).
    pub fn tell(&self, from: &str, message: String) {
        self.0.notice(moli_core::Notice {
            title: None,
            message,
            from: Some(from.to_owned()),
            ts: moli_core::now_ms(),
        });
    }
}

#[cfg(test)]
mod reconciliation_tests {
    use super::*;
    use crate::HubOptions;
    use moli_core::{Access, Kind, PointSpec, Semantic};
    #[test]
    fn point_keys_cover_definition_cache_and_state_but_never_another_instance() {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "moli-owned-keys-{}-{suffix}.json",
            std::process::id()
        ));
        std::fs::write(&path,br#"{"own:cached":{"cached":{"value":12,"ts":1}},"foreign:cached":{"foreign":{"value":20,"ts":1}}}"#).unwrap();
        let hub = Hub::new(HubOptions {
            state_path: Some(path.clone()),
            ..HubOptions::default()
        })
        .unwrap();
        let (_, commands) = mpsc::channel(1);
        let ctx = DriverCtx::new(
            hub.clone(),
            InstanceId::from("own"),
            commands,
            CancellationToken::new(),
        );
        let id = ctx.device_id("cached");
        assert_eq!(ctx.known_point_keys(&id), vec![Arc::<str>::from("cached")]);
        assert!(
            ctx.known_point_keys(&DeviceId::from("foreign:cached"))
                .is_empty()
        );
        ctx.upsert_device(Device {
            id: id.clone(),
            instance: ctx.instance().clone(),
            native_name: "synthetic".into(),
            manufacturer: None,
            model: None,
            description: None,
            native_room: None,
            members: vec![],
            points: vec![PointSpec {
                key: "declared".into(),
                label: "synthetic".into(),
                kind: Kind::Numeric {
                    min: None,
                    max: None,
                    step: None,
                },
                unit: None,
                access: Access {
                    read: true,
                    write: false,
                },
                semantic: Semantic::Other,
            }],
        });
        assert_eq!(
            ctx.known_point_keys(&id),
            vec![Arc::<str>::from("cached"), Arc::<str>::from("declared")]
        );
        ctx.set_state(&id, "declared", Value::Int(7));
        assert_eq!(ctx.known_point_keys(&id).len(), 2);
        assert!(
            ctx.known_point_keys(&DeviceId::from("foreign:cached"))
                .is_empty()
        );
        std::fs::remove_file(path).unwrap();
    }
}

#[cfg(test)]
mod member_permission_tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    #[test]
    fn delegated_policy_remains_parent_bound_member_scoped_and_live_after_reply() {
        let point = PointId::from("fixtures:light/on");
        let allowed = Arc::new(AtomicBool::new(true));
        let check = allowed.clone();
        let parent = point.clone();
        let permit =
            CommandPermit::new(move |target| target == &parent && check.load(Ordering::Acquire));
        let (reply, _answer) = oneshot::channel();
        let command = CommandRequest {
            point,
            device: Arc::new(Device {
                id: "fixtures:light".into(),
                instance: "fixtures".into(),
                native_name: "Synthetic composite".into(),
                manufacturer: None,
                model: None,
                description: None,
                native_room: None,
                members: vec!["physical:member".into()],
                points: Vec::new(),
            }),
            key: "on".into(),
            value: Value::Bool(true),
            deadline: Instant::now() + std::time::Duration::from_secs(5),
            metadata: CommandMetadata {
                origin: Origin::Ui,
                actor: None,
                request_id: None,
                correlation: None,
            },
            permit: Some(permit),
            reply: Arc::new(std::sync::Mutex::new(Some(reply))),
            claim: Arc::new(AtomicU8::new(Claim::TAKEN)),
        };
        let child = command.member_permit().unwrap();
        assert!(child.allows(&"physical:member/on".into()));
        assert!(!child.allows(&"physical:unrelated/on".into()));
        command.reply(Ok(()));
        assert!(child.allows(&"physical:member/on".into()));
        allowed.store(false, Ordering::Release);
        assert!(!child.allows(&"physical:member/on".into()));
    }
}

#[cfg(test)]
#[path = "driver/send_permission_tests.rs"]
mod send_permission_tests;
