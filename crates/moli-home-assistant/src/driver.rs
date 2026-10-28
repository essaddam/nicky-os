//! MQTT bridge driver; physical writes remain exclusively in the guarded Gateway.
#[path = "driver_import.rs"]
mod imports;

use anyhow::{Context, Result, bail, ensure};
use moli_runtime::{CommandRequest, DriverCtx};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};
use tokio::task::JoinSet;

use crate::{
    Config, Gateway,
    inventory::{Catalog, HaState},
    protocol::{
        self, Availability, CoverState, Exports, HaCommand, NativeCommand, ProofMessage,
        ResultCode, ResultMessage,
    },
    transport::{Incoming, Link},
};
use imports::Imports;

const LIMIT: usize = 16;

fn encoded<T: Serialize>(message: &T, max: usize) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(message)?;
    ensure!(bytes.len() <= max, "bridge output exceeds its bound");
    Ok(bytes)
}

struct Pending {
    command: Option<CommandRequest>,
    emitted: bool,
    deadline: Instant,
    source: Option<(String, u64)>,
}
impl Pending {
    fn command(&self) -> &CommandRequest {
        self.command.as_ref().expect("owned pending command")
    }
    fn finish(mut self, result: Result<(), String>) {
        if let Some(command) = self.command.take() {
            command.reply(result);
        }
    }
}
impl Drop for Pending {
    fn drop(&mut self) {
        if let Some(command) = self.command.take() {
            command.reply(Err(if self.emitted {
                moli_i18n::tr!("home_assistant.status.uncertain")
            } else {
                moli_i18n::tr!("home_assistant.status.unavailable")
            }));
        }
    }
}

enum Job {
    Native(ResultMessage),
    Proof(bool),
    Export(Option<(Exports, Vec<CoverState>)>),
    Ownership(Pending, bool),
}

#[derive(Default)]
struct Publications {
    sent: BTreeMap<String, Vec<u8>>,
    waiting: BTreeMap<String, Vec<u8>>,
}
impl Publications {
    fn stage(
        &mut self,
        exports: &Exports,
        states: Vec<CoverState>,
        config: &Config,
        session: &str,
    ) -> Result<()> {
        exports.validate(&config.installation, session)?;
        ensure!(
            states.len() <= protocol::MAX_DEVICES,
            "excessive exported state count"
        );
        let mut desired = BTreeMap::new();
        desired.insert(
            "moli/exports".into(),
            encoded(exports, protocol::MAX_CATALOGUE_BYTES)?,
        );
        for state in states {
            state.validate(&config.installation)?;
            if !exports.items.iter().any(|item| {
                item.binding == state.binding
                    && item.revision == state.revision
                    && item.generation == state.generation
                    && item.feedback == state.feedback
            }) {
                continue;
            }
            let suffix = format!("moli/state/{}", crate::binding_id(&state.binding));
            ensure!(
                desired
                    .insert(suffix, encoded(&state, protocol::MAX_COMMAND_BYTES)?)
                    .is_none(),
                "duplicate exported state"
            );
        }
        for suffix in self.sent.keys() {
            if !desired.contains_key(suffix) {
                desired.insert(suffix.clone(), Vec::new());
            }
        }
        self.waiting
            .retain(|suffix, _| desired.contains_key(suffix));
        for (suffix, bytes) in desired {
            if self.sent.get(&suffix) == Some(&bytes) {
                self.waiting.remove(&suffix);
            } else {
                self.waiting.insert(suffix, bytes);
            }
        }
        Ok(())
    }
    fn flush(&mut self, link: &Link) {
        // Retained metadata/state may wait for capacity. Commands never enter this map.
        for _ in 0..4 {
            let Some((suffix, bytes)) = self.waiting.first_key_value() else {
                break;
            };
            if link.publish(suffix, bytes.clone(), true).is_err() {
                break;
            }
            let (suffix, bytes) = self
                .waiting
                .pop_first()
                .expect("queued retained publication");
            if bytes.is_empty() {
                self.sent.remove(&suffix);
            } else {
                self.sent.insert(suffix, bytes);
            }
        }
    }
}

struct Runner<'a> {
    config: &'a Config,
    gateway: &'a Gateway,
    session: String,
    link: Link,
    connected: bool,
    birth_pending: bool,
    last_birth: Option<Instant>,
    imports: Imports,
    pending: BTreeMap<String, Pending>,
    jobs: JoinSet<Job>,
    checking_ownership: usize,
    exporting: bool,
    publications: Publications,
}
impl Drop for Runner<'_> {
    fn drop(&mut self) {
        self.gateway.connected(false);
        self.jobs.abort_all();
        // Pending's Drop replies even when the supervisor drops this future.
    }
}

impl Runner<'_> {
    fn birth(&mut self) -> Result<()> {
        if self.connected && self.birth_pending {
            let online = Availability {
                v: protocol::VERSION,
                installation: self.config.installation.clone(),
                session: self.session.clone(),
                online: true,
            };
            if self
                .link
                .publish(
                    "moli/availability",
                    encoded(&online, protocol::MAX_COMMAND_BYTES)?,
                    true,
                )
                .is_ok()
            {
                self.birth_pending = false;
                self.last_birth = Some(Instant::now());
            }
        }
        Ok(())
    }

    fn export(&mut self) {
        if !self.connected || self.exporting || self.jobs.len() >= LIMIT {
            return;
        }
        self.exporting = true;
        let gateway = self.gateway.clone();
        self.jobs.spawn(async move {
            let data = match gateway.exports().await {
                Ok(exports) => gateway.states().await.ok().map(|states| (exports, states)),
                Err(_) => None,
            };
            Job::Export(data)
        });
    }

    fn result(&self, result: &ResultMessage) -> Result<()> {
        result.validate(&self.config.installation)?;
        self.link.publish(
            &format!("moli/result/{}", result.request_id),
            encoded(result, protocol::MAX_COMMAND_BYTES)?,
            false,
        )
    }

    fn reject_payload(&self, bytes: &[u8]) -> Result<()> {
        // Decode only an identity for a rejection; malformed envelopes never become jobs.
        if bytes.len() <= protocol::MAX_COMMAND_BYTES
            && let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes)
            && let Some(id) = value["request_id"]
                .as_str()
                .filter(|id| protocol::valid_id(id))
        {
            self.result(&ResultMessage {
                v: protocol::VERSION,
                installation: self.config.installation.clone(),
                request_id: id.into(),
                code: ResultCode::Rejected,
            })?;
        }
        Ok(())
    }

    fn native(&mut self, bytes: &[u8], retained: bool) -> Result<()> {
        let Ok(command) = protocol::decode::<NativeCommand>(bytes, protocol::MAX_COMMAND_BYTES)
        else {
            return self.reject_payload(bytes);
        };
        if retained
            || self.jobs.len() >= LIMIT
            || command
                .validate(&self.config.installation, &self.session, crate::now_ms())
                .is_err()
        {
            return self.reject_payload(bytes);
        }
        let gateway = self.gateway.clone();
        self.jobs
            .spawn(async move { Job::Native(gateway.native_command(command, retained).await) });
        Ok(())
    }

    fn proof(&mut self, bytes: &[u8], retained: bool) {
        let Ok(proof) = protocol::decode::<ProofMessage>(bytes, protocol::MAX_CATALOGUE_BYTES)
        else {
            tracing::warn!("HA bridge rejected a migration proof envelope");
            return;
        };
        if retained
            || self.jobs.len() >= LIMIT
            || proof
                .validate(&self.config.installation, &self.session)
                .is_err()
        {
            tracing::warn!("HA bridge rejected a migration proof envelope");
            return;
        }
        let gateway = self.gateway.clone();
        self.jobs
            .spawn(async move { Job::Proof(gateway.proof(proof, retained).await.is_ok()) });
    }

    fn command(&mut self, command: CommandRequest, ctx: &DriverCtx) {
        let mut pending = Pending {
            deadline: command.deadline,
            command: Some(command),
            emitted: false,
            source: None,
        };
        let Some((entity, _)) = self.imports.command(pending.command(), ctx) else {
            pending.finish(Err(moli_i18n::tr!("home_assistant.status.unsupported")));
            return;
        };
        if !self.connected
            || !self.imports.online
            || self.jobs.len() >= LIMIT
            || self.pending.len() + self.checking_ownership >= LIMIT
        {
            pending.finish(Err(moli_i18n::tr!("home_assistant.status.unavailable")));
            return;
        }
        self.checking_ownership += 1;
        pending.source = Some((entity.clone(), self.gateway.source_generation(&entity)));
        let gateway = self.gateway.clone();
        self.jobs.spawn(async move {
            let owned = tokio::time::timeout_at(
                pending.deadline.into(),
                gateway.source_owned_by_ha(&entity),
            )
            .await
            .unwrap_or(false);
            Job::Ownership(pending, owned)
        });
    }

    fn send_ha(&mut self, mut pending: Pending, owned: bool, ctx: &DriverCtx) -> Result<()> {
        let Some((entity_registry_id, action)) = self.imports.command(pending.command(), ctx)
        else {
            pending.finish(Err(moli_i18n::tr!("home_assistant.status.unsupported")));
            return Ok(());
        };
        let Some(session) = self.gateway.ha_session() else {
            pending.finish(Err(moli_i18n::tr!("home_assistant.status.unavailable")));
            return Ok(());
        };
        if !owned
            || !self.connected
            || !self.imports.online
            || self.pending.len() >= LIMIT
            || pending.command().abandoned()
        {
            pending.finish(Err(moli_i18n::tr!("home_assistant.status.rejected")));
            return Ok(());
        }
        let deadline = pending
            .command()
            .deadline
            .min(Instant::now() + Duration::from_millis(protocol::MAX_COMMAND_TTL_MS));
        let remaining = deadline
            .saturating_duration_since(Instant::now())
            .as_millis();
        if remaining == 0 {
            pending.finish(Err(moli_i18n::tr!("home_assistant.status.expired")));
            return Ok(());
        }
        let request_id = crate::request_id()?;
        let generation = self.gateway.source_generation(&entity_registry_id);
        if pending.source.as_ref() != Some(&(entity_registry_id.clone(), generation)) {
            pending.finish(Err(moli_i18n::tr!("home_assistant.status.rejected")));
            return Ok(());
        }
        let command = HaCommand {
            v: protocol::VERSION,
            installation: self.config.installation.clone(),
            session,
            request_id: request_id.clone(),
            entity_registry_id,
            generation,
            action,
            expires_at_ms: crate::now_ms()
                .saturating_add(u64::try_from(remaining).unwrap_or(protocol::MAX_COMMAND_TTL_MS)),
        };
        let bytes = encoded(&command, protocol::MAX_COMMAND_BYTES)?;
        let permit = pending.command().send_permit(deadline);
        if !owned
            || !self
                .gateway
                .source_command_wanted(&command.entity_registry_id, generation)
            || self.gateway.ha_session().as_ref() != Some(&command.session)
            || !permit.begin_send()
            || self.link.publish("ha/command", bytes, false).is_err()
        {
            pending.finish(Err(moli_i18n::tr!("home_assistant.status.expired")));
            return Ok(());
        }
        pending.emitted = true;
        pending.deadline = deadline;
        self.pending.insert(request_id, pending);
        Ok(())
    }

    fn ha_result(&mut self, suffix: &str, bytes: &[u8], retained: bool) -> Result<()> {
        let result: ResultMessage = protocol::decode(bytes, protocol::MAX_COMMAND_BYTES)?;
        result.validate(&self.config.installation)?;
        ensure!(
            suffix == format!("ha/result/{}", result.request_id),
            "HA result topic mismatch"
        );
        if retained {
            return Ok(());
        }
        if let Some(pending) = self.pending.remove(&result.request_id) {
            if pending.command().abandoned() || Instant::now() >= pending.deadline {
                return Ok(());
            }
            let outcome = match result.code {
                ResultCode::Accepted => Ok(()),
                ResultCode::Expired => Err(moli_i18n::tr!("home_assistant.status.expired")),
                ResultCode::Uncertain => Err(moli_i18n::tr!("home_assistant.status.uncertain")),
                _ => Err(moli_i18n::tr!("home_assistant.status.rejected")),
            };
            pending.finish(outcome);
        }
        Ok(())
    }

    fn incoming(
        &mut self,
        suffix: &str,
        bytes: &[u8],
        retained: bool,
        ctx: &DriverCtx,
    ) -> Result<()> {
        match suffix {
            "ha/inventory" => {
                let catalog: Catalog = protocol::decode(bytes, protocol::MAX_CATALOGUE_BYTES)?;
                catalog.validate_installation(&self.config.installation)?;
                self.imports.check_revision(&catalog)?;
                self.gateway.catalogue_update(&catalog)?;
                self.imports.install(catalog, ctx)?;
            }
            "ha/availability" => {
                let availability: Availability =
                    protocol::decode(bytes, protocol::MAX_COMMAND_BYTES)?;
                let previous = self.gateway.ha_session();
                self.gateway.peer_update(availability.clone(), retained)?;
                let current = self.gateway.ha_session();
                self.imports.availability(current.is_some(), ctx);
                if !self.imports.online {
                    self.pending.clear();
                }
                if !retained && availability.online && current.is_some() && current != previous {
                    self.pending.clear();
                    self.birth_pending = true;
                    self.birth()?;
                }
            }
            "ha/migration/proof" => self.proof(bytes, retained),
            "moli/command" => self.native(bytes, retained)?,
            _ if suffix.starts_with("ha/state/") => {
                let state: HaState = protocol::decode(bytes, protocol::MAX_CATALOGUE_BYTES)?;
                protocol::version(state.v)?;
                protocol::installation(&state.installation, &self.config.installation)?;
                ensure!(
                    suffix == format!("ha/state/{}/{}", state.device, state.channel),
                    "HA state topic mismatch"
                );
                self.imports.state(&state, ctx)?;
            }
            _ if suffix.starts_with("ha/result/") => self.ha_result(suffix, bytes, retained)?,
            _ if suffix.starts_with("ha/migration/result/") => {
                self.gateway.migration_status(bytes, retained)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn expire(&mut self) {
        let ids: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, pending)| {
                Instant::now() >= pending.deadline || pending.command().abandoned()
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            self.pending.remove(&id);
        }
    }

    async fn serve(&mut self, ctx: &mut DriverCtx) -> Result<()> {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        let mut flush = tokio::time::interval(Duration::from_millis(50));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                command = ctx.next_command() => match command { Some(command) => self.command(command, ctx), None => return Ok(()) },
                incoming = self.link.next() => match incoming {
                    Some(Incoming::Connected) => {
                        self.connected = true; self.gateway.connected(true); ctx.ready();
                        self.birth_pending = true; self.birth()?;
                        self.export();
                    },
                    Some(Incoming::Message { suffix, payload, retained }) => {
                        if self.incoming(&suffix, &payload, retained, ctx).is_err() { tracing::warn!("HA bridge rejected an incoming envelope"); }
                    },
                    Some(Incoming::Disconnected) | None => bail!("HA MQTT link disconnected"),
                },
                frame = self.gateway.next_frame() => {
                    let frame = frame.context("HA bridge outgoing queue closed")?;
                    self.link.publish(&frame.suffix, frame.payload, frame.retained)?;
                },
                Some(done) = self.jobs.join_next(), if !self.jobs.is_empty() => match done.context("HA bridge worker failed")? {
                    Job::Native(result) => self.result(&result)?,
                    Job::Proof(ok) => { if !ok { tracing::warn!("HA bridge rejected a migration proof"); } },
                    Job::Ownership(pending, owned) => { self.checking_ownership -= 1; self.send_ha(pending, owned, ctx)?; },
                    Job::Export(data) => { self.exporting = false; if let Some((exports, states)) = data { self.publications.stage(&exports, states, self.config, &self.session)?; } },
                },
                _ = tick.tick() => {
                    let online = self.gateway.ha_session().is_some();
                    self.imports.availability(online, ctx);
                    if !online { self.pending.clear(); }
                    if self.last_birth.is_none_or(|at| at.elapsed() >= Duration::from_secs(5)) {
                        self.birth_pending = true;
                    }
                    self.birth()?;
                    self.expire(); self.export();
                },
                _ = flush.tick() => { self.birth()?; self.publications.flush(&self.link); },
            }
        }
    }
}

pub(crate) async fn run(config: &Config, gateway: &Gateway, ctx: &mut DriverCtx) -> Result<()> {
    let (username, password) = loop {
        if let (Some(username), Some(password)) =
            (ctx.secret("mqtt_username"), ctx.secret("mqtt_password"))
        {
            break (username, password);
        }
        ctx.wait_for(moli_i18n::tr!("home_assistant.status.credentials"));
        tokio::select! { ()=ctx.cancelled()=>return Ok(()), ()=tokio::time::sleep(Duration::from_secs(10))=>{} }
    };
    gateway.connected(false);
    for device in ctx.devices() {
        ctx.set_availability(&device, false);
    }
    let session = gateway.renew_session()?;
    let link = Link::open(config, username, password, &session)?;
    let mut runner = Runner {
        config,
        gateway,
        session,
        link,
        connected: false,
        birth_pending: false,
        last_birth: None,
        imports: Imports::default(),
        pending: BTreeMap::new(),
        jobs: JoinSet::new(),
        checking_ownership: 0,
        exporting: false,
        publications: Publications::default(),
    };
    let outcome = runner.serve(ctx).await;
    gateway.connected(false);
    runner.pending.clear();
    runner.jobs.abort_all();
    while runner.jobs.join_next().await.is_some() {}
    for device in ctx.devices() {
        ctx.set_availability(&device, false);
    }
    outcome
}
