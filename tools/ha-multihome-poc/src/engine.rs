//! Interactive fail-stop bench, with one independent authority domain per house.
use crate::{
    authority::{Authority, Effect, Lease, Scope},
    config::{self, Topology},
    transport::Cluster,
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{Arc, Mutex, RwLock},
    time::{Duration, Instant},
};
use tokio::task::JoinHandle;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Seed {
    pub id: String,
    pub document: Json,
    pub summary: Json,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
    pub v: u8,
    pub kind: String,
    pub houses: Vec<Seed>,
}
impl Bundle {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            matches!(
                self.kind.as_str(),
                "private_ha_replay" | "synthetic_fixture"
            ),
            "unknown seed kind"
        );
        ensure!(
            self.v == 1 && self.houses.len() == 2,
            "two versioned house seeds required"
        );
        for (i, h) in self.houses.iter().enumerate() {
            ensure!(
                h.id == if i == 0 { "house-a" } else { "house-b" },
                "invalid house seed identity"
            );
            ensure!(
                h.document
                    .as_object()
                    .is_some_and(|o| !o.is_empty() && o.len() <= 20000),
                "empty or excessive seed"
            );
            ensure!(
                serde_json::to_vec(&h.document)?.len() <= 8 * 1024 * 1024,
                "seed byte bound"
            );
        }
        Ok(())
    }
    pub fn fixture() -> Self {
        Self{v:1,kind:"synthetic_fixture".into(),houses:["house-a","house-b"].into_iter().map(|id|Seed{id:id.into(),document:json!({"temperature":{"value":21.0,"unit":"°C"},"humidity":{"value":45,"unit":"%"}}),summary:json!({"house":id,"kind":"synthetic_fixture","entities":1,"points":2,"devices":1,"description_bytes":0})}).collect()}
    }
}
#[derive(Clone)]
struct Delta {
    sequence: u64,
    key: String,
    value: Json,
    bytes: usize,
}
#[derive(Default)]
struct NodeState {
    online: bool,
    isolated: bool,
    fenced: bool,
    pending_reacquire: BTreeSet<String>,
    status: Json,
    last_cpu: Option<(Instant, f64)>,
    cpu: Option<f64>,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    pub op: String,
    #[serde(default)]
    pub node: String,
    #[serde(default)]
    pub house: String,
}
pub struct Engine {
    seeds: Vec<Seed>,
    cluster: Option<Cluster>,
    authority: Option<Arc<Mutex<Authority>>>,
    allowed_links: Arc<RwLock<BTreeSet<(String, String)>>>,
    renewer: Option<JoinHandle<()>>,
    pub topology: Topology,
    nodes: BTreeMap<String, NodeState>,
    origin: Instant,
    events: VecDeque<Json>,
    history: BTreeMap<String, VecDeque<Delta>>,
    pub simulated_effects: u64,
    pub rejected_effects: u64,
    ticks: u64,
    pub last_error: Option<String>,
    configured: bool,
}
impl Engine {
    pub fn new(bundle: Bundle) -> Result<Self> {
        bundle.validate()?;
        Ok(Self {
            seeds: bundle.houses,
            cluster: None,
            authority: None,
            allowed_links: Arc::new(RwLock::new(BTreeSet::new())),
            renewer: None,
            topology: config::preset("cross")?,
            nodes: BTreeMap::new(),
            origin: Instant::now(),
            events: VecDeque::new(),
            history: BTreeMap::new(),
            simulated_effects: 0,
            rejected_effects: 0,
            ticks: 0,
            last_error: None,
            configured: false,
        })
    }
    fn now(&self) -> u64 {
        clock_ms(self.origin)
    }
    fn with_authority<T>(&self, f: impl FnOnce(&mut Authority, u64) -> Result<T>) -> Result<T> {
        let shared = self.authority.as_ref().context("not configured")?;
        let mut authority = shared
            .lock()
            .map_err(|_| anyhow::anyhow!("authority lock poisoned"))?;
        // Read the clock after acquiring the lock, including against the renewer.
        f(&mut authority, clock_ms(self.origin))
    }
    fn lease(&self, house: &str) -> Result<Lease> {
        self.with_authority(|authority, _| authority.lease(house).cloned().context("unknown house"))
    }
    fn event(&mut self, kind: &str, house: &str, node: &str) {
        if self.events.len() == 64 {
            self.events.pop_front();
        }
        self.events
            .push_back(json!({"at_ms":self.now(),"kind":kind,"house":house,"node":node}));
    }
    fn failure(&mut self, house: &str, node: &str, error: &anyhow::Error) {
        self.last_error = Some(format!("{house}/{node}: {error:#}"));
        self.event("operation_failed", house, node);
    }
    fn reachable(&self, node: &str, house: &str) -> bool {
        reachable(&self.topology, &self.nodes, node, house)
    }
    fn recompute_links(&self) {
        let mut links = BTreeSet::new();
        for house in &self.topology.houses {
            for node in self.topology.holders(&house.id) {
                if link_allowed(&self.topology, &self.nodes, &node, &house.id)
                    && self
                        .nodes
                        .get(&node)
                        .is_some_and(|state| !state.pending_reacquire.contains(&house.id))
                {
                    links.insert((house.id.clone(), node));
                }
            }
        }
        if let Ok(mut allowed) = self.allowed_links.write() {
            *allowed = links;
        }
    }
    fn start_renewer(&mut self) {
        let Some(authority) = self.authority.clone() else {
            return;
        };
        let Some(cluster) = &self.cluster else {
            return;
        };
        let heartbeats = cluster.heartbeats();
        let links = self.allowed_links.clone();
        let origin = self.origin;
        self.renewer = Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(200));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                let allowed = match links.read() {
                    Ok(value) => value.clone(),
                    Err(_) => continue,
                };
                let fresh_nodes = match heartbeats.lock() {
                    Ok(beats) => {
                        let now = Instant::now();
                        beats
                            .iter()
                            .filter(|(_, at)| {
                                now.saturating_duration_since(**at) < Duration::from_millis(1500)
                            })
                            .map(|(node, _)| node.clone())
                            .collect::<BTreeSet<_>>()
                    }
                    Err(_) => continue,
                };
                let Ok(mut guard) = authority.lock() else {
                    continue;
                };
                let now_ms = clock_ms(origin);
                for lease in guard.leases() {
                    if allowed.contains(&(lease.house.clone(), lease.owner.clone()))
                        && fresh_nodes.contains(&lease.owner)
                    {
                        // Authority rejects renewal after expiry; only a new epoch can recover it.
                        let _ = guard.renew(&lease.house, &lease.owner, lease.epoch, now_ms);
                    }
                }
            }
        }));
    }
    fn copy(&self, node: &str, house: &str) -> Option<&Json> {
        self.nodes.get(node)?.status["houses"]
            .as_array()?
            .iter()
            .find(|value| value["house"] == house)
    }
    fn cache_copy(&mut self, node: &str, house: &str, summary: Json) {
        let Some(state) = self.nodes.get_mut(node) else {
            return;
        };
        if !state.status.is_object() {
            state.status = json!({"houses":[]});
        }
        if !state.status["houses"].is_array() {
            state.status["houses"] = json!([]);
        }
        let houses = state.status["houses"].as_array_mut().unwrap();
        if let Some(current) = houses.iter_mut().find(|value| value["house"] == house) {
            *current = summary;
        } else {
            houses.push(summary);
        }
    }
    fn copy_current(&self, node: &str, lease: &Lease) -> bool {
        self.copy(node, &lease.house).is_some_and(|copy| {
            copy["records"].as_u64().unwrap_or(0) > 0
                && copy["epoch"] == lease.epoch
                && copy["sequence"] == lease.sequence
                && self
                    .expected_hash(&lease.house)
                    .is_ok_and(|expected| copy["sha256"] == expected)
        })
    }
    fn owner_ready(&self, lease: &Lease) -> bool {
        self.reachable(&lease.owner, &lease.house)
            && self
                .nodes
                .get(&lease.owner)
                .is_some_and(|state| !state.pending_reacquire.contains(&lease.house))
            && self.copy_current(&lease.owner, lease)
            && self
                .copy(&lease.owner, &lease.house)
                .is_some_and(|copy| copy["role"] == "active")
    }
    fn document(&self, house: &str) -> Result<&Json> {
        Ok(&self
            .seeds
            .iter()
            .find(|seed| seed.id == house)
            .context("unknown seed")?
            .document)
    }
    pub fn expected_hash(&self, house: &str) -> Result<String> {
        Ok(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(self.document(house)?)?)
        ))
    }
    pub async fn configure(&mut self, topology: Topology) -> Result<()> {
        topology.validate()?;
        self.shutdown().await;
        self.topology = topology;
        self.nodes.clear();
        self.history.clear();
        self.events.clear();
        self.ticks = 0;
        self.simulated_effects = 0;
        self.rejected_effects = 0;
        self.last_error = None;
        for seed in &mut self.seeds {
            let document = seed.document.as_object_mut().context("seed object")?;
            document.remove("poc/tick");
            document.remove("workflow/probe");
        }
        self.cluster = Some(Cluster::new().await?);
        let result = self.initialize().await;
        if let Err(ref error) = result {
            self.last_error = Some(error.to_string());
            self.shutdown().await;
        }
        result
    }
    async fn initialize(&mut self) -> Result<()> {
        let mut ids = BTreeSet::new();
        for house in &self.topology.houses {
            ids.extend(self.topology.holders(&house.id));
        }
        for id in ids {
            self.cluster.as_mut().unwrap().spawn_node(&id).await?;
            self.nodes.insert(
                id,
                NodeState {
                    online: true,
                    ..NodeState::default()
                },
            );
        }
        let grants = self
            .topology
            .houses
            .iter()
            .flat_map(|house| {
                self.topology.holders(&house.id).into_iter().map(move |id| {
                    let primary = id == house.primary;
                    (id, house.id.clone(), primary)
                })
            })
            .collect::<Vec<_>>();
        for (id, house, primary) in grants {
            let document = self.document(&house)?.clone();
            let summary = self
                .cluster
                .as_mut()
                .unwrap()
                .seed(&id, &house, 1, 0, &document)
                .await?;
            self.cache_copy(&id, &house, summary);
            let summary = self
                .cluster
                .as_mut()
                .unwrap()
                .set_role(&id, &house, 1, if primary { "active" } else { "passive" })
                .await?;
            self.cache_copy(&id, &house, summary);
        }
        self.refresh().await;
        self.authority = Some(Arc::new(Mutex::new(Authority::new(
            self.topology.clone(),
            self.now(),
        )?)));
        self.configured = true;
        self.recompute_links();
        self.start_renewer();
        self.event("configured", "", "");
        Ok(())
    }
    pub async fn refresh(&mut self) {
        let ids = self
            .nodes
            .iter()
            .filter(|(_, state)| !state.fenced)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in ids {
            let Some(cluster) = self.cluster.as_mut() else {
                break;
            };
            match cluster.status(&id).await {
                Ok(status) => {
                    let state = self.nodes.get_mut(&id).unwrap();
                    state.online = true;
                    if let Some(cpu) = status["cpu_seconds"].as_f64() {
                        let at = Instant::now();
                        state.cpu = state.last_cpu.and_then(|(previous, old)| {
                            let elapsed = at.saturating_duration_since(previous).as_secs_f64();
                            (elapsed > 0.0).then_some(((cpu - old) / elapsed * 100.0).max(0.0))
                        });
                        state.last_cpu = Some((at, cpu));
                    }
                    state.status = status;
                }
                Err(error) => {
                    let state = self.nodes.get_mut(&id).unwrap();
                    state.online = false;
                    state.status = Json::Null;
                    // A status timeout is never proof that the child process exited.
                    self.failure("", &id, &error);
                }
            }
        }
        self.recompute_links();
    }
    pub async fn tick(&mut self) -> Result<()> {
        if !self.configured {
            return Ok(());
        }
        self.refresh().await;
        self.ticks = self
            .ticks
            .checked_add(1)
            .context("tick counter exhausted")?;
        let houses = self
            .topology
            .houses
            .iter()
            .map(|house| house.id.clone())
            .collect::<Vec<_>>();
        for house in houses {
            if let Err(error) = self.tick_house(&house).await {
                self.failure(&house, "", &error);
            }
        }
        self.refresh().await;
        Ok(())
    }
    async fn tick_house(&mut self, house: &str) -> Result<()> {
        let lease = self.lease(house)?;
        if self.now() >= lease.expires_ms || !self.reachable(&lease.owner, house) {
            let candidates = self
                .topology
                .house(house)
                .context("unknown house")?
                .standbys
                .iter()
                .filter(|standby| standby.automatic)
                .map(|standby| standby.node.clone())
                .collect::<Vec<_>>();
            for node in candidates {
                match self.promote(house, &node, false).await {
                    Ok(()) => break,
                    Err(error) => self.failure(house, &node, &error),
                }
            }
            return Ok(());
        }
        // Repair the owner too: a lost MQTT acknowledgement must not strand the next sequence.
        self.synchronize(&lease.owner, house).await?;
        ensure!(self.owner_ready(&lease), "owner replica is not current");
        let sequence = lease
            .sequence
            .checked_add(1)
            .context("sequence exhausted")?;
        let observed = self.now();
        let value = json!({"value":self.ticks,"quality":"simulated","observed_ms":observed,"changed_ms":observed});
        let mut document = self.document(house)?.clone();
        document["poc/tick"] = value.clone();
        ensure!(
            document
                .as_object()
                .is_some_and(|object| object.len() <= 20_000),
            "record limit reached"
        );
        ensure!(
            serde_json::to_vec(&document)?.len() <= 8 * 1024 * 1024,
            "document byte bound"
        );
        let delta = Delta {
            sequence,
            key: "poc/tick".into(),
            bytes: serde_json::to_vec(&value)?.len() + "poc/tick".len(),
            value,
        };
        // The lab coordinator is the exclusive generator. Commit the canonical assignment and
        // retain its replay data before any MQTT send, irrespective of the owner's response.
        self.with_authority(|authority, now| {
            authority.committed(house, &lease.owner, lease.epoch, now, sequence)
        })?;
        self.seeds
            .iter_mut()
            .find(|seed| seed.id == house)
            .context("unknown seed")?
            .document = document;
        let history = self.history.entry(house.into()).or_default();
        history.push_back(delta);
        while history.len() > 256
            || history.iter().map(|delta| delta.bytes).sum::<usize>() > 1024 * 1024
        {
            history.pop_front();
        }
        let updated = self.lease(house)?;
        if let Err(error) = self.synchronize(&lease.owner, house).await {
            self.failure(house, &lease.owner, &error);
        }
        for node in self.topology.holders(house) {
            if node != lease.owner
                && self.reachable(&node, house)
                && let Err(error) = self.synchronize(&node, house).await
            {
                self.failure(house, &node, &error);
            }
        }
        if self.ticks.is_multiple_of(5) {
            let id = format!("workflow-{}", self.ticks / 5);
            match self.test_effect(house, &updated.owner, updated.epoch, &id) {
                Ok(true) => self.simulated_effects += 1,
                Ok(false) => {}
                Err(error) => {
                    self.rejected_effects += 1;
                    self.failure(house, &updated.owner, &error);
                }
            }
        }
        Ok(())
    }
    async fn synchronize(&mut self, node: &str, house: &str) -> Result<()> {
        ensure!(self.reachable(node, house), "replica link unavailable");
        let lease = self.lease(house)?;
        if !self.copy_current(node, &lease) {
            let current = self.copy(node, house).cloned().unwrap_or(Json::Null);
            let from = current["sequence"].as_u64().unwrap_or(u64::MAX);
            let deltas = self
                .history
                .get(house)
                .map(|history| {
                    history
                        .iter()
                        .filter(|delta| delta.sequence > from)
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            // Each delta currently recomputes a whole JSON digest in the worker.
            // A short bounded replay avoids holding the interactive control plane for a long gap.
            let contiguous = deltas.len() <= 16
                && current["epoch"] == lease.epoch
                && deltas
                    .first()
                    .is_some_and(|delta| Some(delta.sequence) == from.checked_add(1))
                && deltas
                    .last()
                    .is_some_and(|delta| delta.sequence == lease.sequence);
            if contiguous {
                for delta in deltas {
                    let summary = self
                        .cluster
                        .as_mut()
                        .context("cluster unavailable")?
                        .update(
                            node,
                            house,
                            lease.epoch,
                            delta.sequence,
                            &delta.key,
                            &delta.value,
                        )
                        .await?;
                    self.cache_copy(node, house, summary);
                }
            }
            if !self.copy_current(node, &lease) {
                let document = self.document(house)?.clone();
                let summary = self
                    .cluster
                    .as_mut()
                    .context("cluster unavailable")?
                    .seed(node, house, lease.epoch, lease.sequence, &document)
                    .await?;
                self.cache_copy(node, house, summary);
                self.event("snapshot_recovery", house, node);
            }
        }
        ensure!(
            self.copy_current(node, &lease),
            "replica acknowledgement differs from canonical document"
        );
        let role = if node == lease.owner {
            "active"
        } else {
            "passive"
        };
        if !self
            .copy(node, house)
            .is_some_and(|copy| copy["role"] == role)
        {
            let summary = self
                .cluster
                .as_mut()
                .context("cluster unavailable")?
                .set_role(node, house, lease.epoch, role)
                .await?;
            self.cache_copy(node, house, summary);
        }
        Ok(())
    }
    pub async fn action(&mut self, action: Action) -> Result<()> {
        ensure!(self.configured, "not configured");
        ensure!(self.topology.node(&action.node).is_some(), "unknown node");
        let result = self.perform_action(&action).await;
        if let Err(ref error) = result {
            self.failure(&action.house, &action.node, error);
        }
        self.recompute_links();
        self.refresh().await;
        result
    }
    async fn perform_action(&mut self, action: &Action) -> Result<()> {
        match action.op.as_str() {
            "stop" => {
                ensure!(self.nodes.contains_key(&action.node), "node not active");
                self.cluster
                    .as_mut()
                    .context("cluster unavailable")?
                    .kill(&action.node)
                    .await?;
                let state = self.nodes.get_mut(&action.node).unwrap();
                mark_fenced(state);
                self.recompute_links();
                self.event("node_stopped", "", &action.node);
            }
            "restart" => {
                let state = self.nodes.get(&action.node).context("inactive node")?;
                ensure!(!state.online, "node already online");
                let isolated = state.isolated;
                let pending_reacquire = self
                    .topology
                    .houses
                    .iter()
                    .filter_map(|house| {
                        self.lease(&house.id)
                            .ok()
                            .filter(|lease| lease.owner == action.node)
                            .map(|lease| lease.house)
                    })
                    .collect();
                if !state.fenced {
                    self.cluster
                        .as_mut()
                        .context("cluster unavailable")?
                        .kill(&action.node)
                        .await?;
                    mark_fenced(self.nodes.get_mut(&action.node).unwrap());
                }
                self.recompute_links();
                self.cluster
                    .as_mut()
                    .context("cluster unavailable")?
                    .spawn_node(&action.node)
                    .await?;
                self.nodes.insert(
                    action.node.clone(),
                    NodeState {
                        online: true,
                        isolated,
                        pending_reacquire,
                        ..NodeState::default()
                    },
                );
                self.recompute_links();
                let homes = self
                    .topology
                    .houses
                    .iter()
                    .filter(|house| self.topology.holders(&house.id).contains(&action.node))
                    .map(|house| house.id.clone())
                    .collect::<Vec<_>>();
                let mut first_error = None;
                for house in homes {
                    let result = async {
                        self.synchronize(&action.node, &house).await?;
                        if self.lease(&house)?.owner == action.node {
                            // The previous child was killed and reaped above (or by stop).
                            // Only this restart path carries that proof across the new spawn.
                            self.promote_inner(&house, &action.node, true, Some(true))
                                .await?;
                        }
                        Ok::<(), anyhow::Error>(())
                    }
                    .await;
                    if let Err(error) = result {
                        self.failure(&house, &action.node, &error);
                        if first_error.is_none() {
                            first_error = Some(error);
                        }
                    }
                }
                self.event("node_restarted", "", &action.node);
                if let Some(error) = first_error {
                    return Err(error);
                }
            }
            "isolate" => {
                self.nodes
                    .get_mut(&action.node)
                    .context("inactive node")?
                    .isolated = true;
                self.recompute_links();
                self.event("wan_isolated", "", &action.node);
            }
            "heal" => {
                self.nodes
                    .get_mut(&action.node)
                    .context("inactive node")?
                    .isolated = false;
                self.recompute_links();
                let targets = self
                    .topology
                    .houses
                    .iter()
                    .flat_map(|house| {
                        self.topology
                            .holders(&house.id)
                            .into_iter()
                            .map(move |node| (node, house.id.clone()))
                    })
                    .collect::<Vec<_>>();
                let mut first_error = None;
                for (node, house) in targets {
                    if self.reachable(&node, &house)
                        && let Err(error) = self.synchronize(&node, &house).await
                    {
                        self.failure(&house, &node, &error);
                        if first_error.is_none() {
                            first_error = Some(error);
                        }
                    }
                }
                self.event("wan_restored", "", &action.node);
                if let Some(error) = first_error {
                    return Err(error);
                }
            }
            "promote" => self.promote(&action.house, &action.node, true).await?,
            _ => bail!("unknown simulation action"),
        }
        Ok(())
    }
    pub async fn promote(&mut self, house: &str, node: &str, manual: bool) -> Result<()> {
        self.promote_inner(house, node, manual, None).await
    }
    async fn promote_inner(
        &mut self,
        house: &str,
        node: &str,
        manual: bool,
        confirmed_fence: Option<bool>,
    ) -> Result<()> {
        self.refresh().await;
        ensure!(
            self.reachable(node, house),
            "candidate has no replication link"
        );
        let current = self.lease(house)?;
        ensure!(
            self.copy_current(node, &current),
            "candidate copy is not complete at the current epoch and sequence"
        );
        let fenced = confirmed_fence.unwrap_or_else(|| {
            self.nodes
                .get(&current.owner)
                .is_some_and(|state| state.fenced)
        });
        let preview = self.with_authority(|authority, now| {
            let mut preview = authority.clone();
            preview.promote(house, node, now, manual, fenced, current.sequence)
        })?;
        let document = self.document(house)?.clone();
        let summary = self
            .cluster
            .as_mut()
            .context("cluster unavailable")?
            .seed(node, house, preview.epoch, preview.sequence, &document)
            .await?;
        self.cache_copy(node, house, summary);
        ensure!(
            self.copy_current(node, &preview),
            "candidate did not acknowledge the new epoch snapshot"
        );
        let granted = self.with_authority(|authority, now| {
            let live = authority.lease(house).context("unknown house")?;
            ensure!(
                live.epoch == current.epoch
                    && live.owner == current.owner
                    && live.sequence == current.sequence,
                "authority changed during promotion"
            );
            authority.promote(house, node, now, manual, fenced, current.sequence)
        })?;
        if let Some(state) = self.nodes.get_mut(node) {
            state.pending_reacquire.remove(house);
        }
        self.recompute_links();
        // The grant exists only after the complete new-epoch copy was acknowledged.
        let summary = self
            .cluster
            .as_mut()
            .context("cluster unavailable")?
            .set_role(node, house, granted.epoch, "active")
            .await?;
        self.cache_copy(node, house, summary);
        for holder in self.topology.holders(house) {
            if holder != node
                && self.reachable(&holder, house)
                && let Err(error) = self.synchronize(&holder, house).await
            {
                self.failure(house, &holder, &error);
            }
        }
        self.event("promoted", house, node);
        self.recompute_links();
        Ok(())
    }
    pub fn test_effect(&mut self, house: &str, node: &str, epoch: u64, id: &str) -> Result<bool> {
        let lease = self.lease(house)?;
        ensure!(
            lease.owner == node && lease.epoch == epoch,
            "effect token is not current"
        );
        ensure!(
            self.owner_ready(&lease),
            "effect owner is unavailable or its copy is incomplete"
        );
        let effect = Effect {
            house: house.into(),
            node: node.into(),
            epoch,
            id: id.into(),
            scope: if self.topology.local(node, house) {
                Scope::Local
            } else {
                Scope::Cloud
            },
        };
        self.with_authority(|authority, now| authority.accept_effect(&effect, now))
    }
    pub fn status(&self) -> Json {
        let leases = self
            .with_authority(|authority, _| Ok(authority.leases()))
            .unwrap_or_default();
        let now = self.now();
        let houses = leases.into_iter().map(|lease| {
            let copies = self.topology.holders(&lease.house).into_iter().map(|node| {
                let raw = self.copy(&node, &lease.house).cloned().unwrap_or(Json::Null);
                let online = self.nodes.get(&node).is_some_and(|state| state.online);
                let in_sync = online && self.copy_current(&node, &lease);
                json!({"node":node,"role":if !online {"offline"} else if node == lease.owner {"active"} else {"passive"},"reported_role":raw["role"],"epoch":raw["epoch"],"sequence":raw["sequence"],"in_sync":in_sync,"records":raw["records"],"document_bytes":raw["document_bytes"],"sha256":raw["sha256"]})
            }).collect::<Vec<_>>();
            json!({"id":lease.house,"owner":lease.owner,"epoch":lease.epoch,"sequence":lease.sequence,"scope":if self.topology.local(&lease.owner,&lease.house) {"local"} else {"cloud"},"lease_valid":now < lease.expires_ms && self.owner_ready(&lease),"copies":copies})
        }).collect::<Vec<_>>();
        let nodes = self.nodes.iter().map(|(id, state)| json!({"id":id,"site":self.topology.node(id).map(|node|&node.site),"online":state.online,"isolated":state.isolated,"fenced":state.fenced,"rss_bytes":if state.online {state.status["rss_bytes"].clone()} else {Json::Null},"cpu_percent_one_core":state.cpu,"allocator_live_bytes":if state.online {state.status["allocator_live_bytes"].clone()} else {Json::Null},"error":Json::Null})).collect::<Vec<_>>();
        json!({"v":1,"mode":"simulation","computed_at_ms":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis(),"configured":self.configured,"topology":self.topology,"sources":self.seeds.iter().map(|seed|&seed.summary).collect::<Vec<_>>(),"houses":houses,"nodes":nodes,"events":self.events,"metrics":{"physical_commands":0,"simulated_effects":self.simulated_effects,"rejected_effects":self.rejected_effects,"ticks":self.ticks},"error":self.last_error,"limitations":["central_test_witness","local_processes_not_remote_sites","ha_export_replay_not_live_state","no_physical_executor","no_secret_replication","no_real_radio_failover"]})
    }
    pub async fn shutdown(&mut self) {
        if let Some(renewer) = self.renewer.take() {
            renewer.abort();
            let _ = renewer.await;
        }
        if let Ok(mut links) = self.allowed_links.write() {
            links.clear();
        }
        if let Some(cluster) = &mut self.cluster {
            cluster.shutdown().await;
        }
        self.cluster = None;
        self.authority = None;
        self.configured = false;
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        if let Some(renewer) = &self.renewer {
            renewer.abort();
        }
    }
}
fn clock_ms(origin: Instant) -> u64 {
    origin.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}
fn mark_fenced(state: &mut NodeState) {
    state.online = false;
    state.fenced = true;
    state.status = Json::Null;
    state.last_cpu = None;
    state.cpu = None;
}
fn reachable(
    topology: &Topology,
    nodes: &BTreeMap<String, NodeState>,
    node: &str,
    house: &str,
) -> bool {
    nodes.get(node).is_some_and(|state| state.online) && link_allowed(topology, nodes, node, house)
}
fn link_allowed(
    topology: &Topology,
    nodes: &BTreeMap<String, NodeState>,
    node: &str,
    house: &str,
) -> bool {
    let Some(state) = nodes.get(node) else {
        return false;
    };
    if state.fenced || topology.house(house).is_none() {
        return false;
    }
    if topology.local(node, house) {
        return true;
    }
    let primary_isolated = topology
        .house(house)
        .and_then(|home| nodes.get(&home.primary))
        .is_some_and(|home| home.isolated);
    !state.isolated && !primary_isolated
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nodes() -> BTreeMap<String, NodeState> {
        ["node-a", "node-b"]
            .into_iter()
            .map(|id| {
                (
                    id.into(),
                    NodeState {
                        online: true,
                        ..NodeState::default()
                    },
                )
            })
            .collect()
    }
    #[test]
    fn wan_isolation_keeps_local_houses_reachable_and_blocks_both_cross_copies() {
        let topology = config::preset("cross").unwrap();
        let mut nodes = nodes();
        nodes.get_mut("node-a").unwrap().isolated = true;
        assert!(reachable(&topology, &nodes, "node-a", "house-a"));
        assert!(reachable(&topology, &nodes, "node-b", "house-b"));
        assert!(!reachable(&topology, &nodes, "node-b", "house-a"));
        assert!(!reachable(&topology, &nodes, "node-a", "house-b"));
        assert!(!nodes["node-a"].fenced);
        nodes.get_mut("node-a").unwrap().isolated = false;
        assert!(reachable(&topology, &nodes, "node-b", "house-a"));
        assert!(reachable(&topology, &nodes, "node-a", "house-b"));
    }
    #[test]
    fn offline_is_not_fenced_and_confirmed_fencing_preserves_the_wan_link() {
        let mut state = NodeState {
            online: true,
            isolated: true,
            ..NodeState::default()
        };
        state.online = false;
        assert!(!state.fenced);
        mark_fenced(&mut state);
        assert!(state.fenced);
        assert!(!state.online);
        assert!(state.isolated);
        let restarted = NodeState {
            online: true,
            isolated: state.isolated,
            ..NodeState::default()
        };
        assert!(restarted.isolated);
        assert!(!restarted.fenced);
    }
    #[test]
    fn a_failed_status_does_not_revoke_the_heartbeat_renewal_link() {
        let topology = config::preset("cross").unwrap();
        let mut nodes = nodes();
        nodes.get_mut("node-a").unwrap().online = false;
        assert!(!reachable(&topology, &nodes, "node-a", "house-a"));
        assert!(link_allowed(&topology, &nodes, "node-a", "house-a"));
        mark_fenced(nodes.get_mut("node-a").unwrap());
        assert!(!link_allowed(&topology, &nodes, "node-a", "house-a"));
    }
    #[test]
    fn restart_and_unacknowledged_epoch_cannot_expose_a_valid_owner() {
        let mut engine = Engine::new(Bundle::fixture()).unwrap();
        let authority = Authority::new(engine.topology.clone(), 0).unwrap();
        let lease = authority.lease("house-a").unwrap().clone();
        engine.authority = Some(Arc::new(Mutex::new(authority)));
        engine.nodes = nodes();
        let hash = engine.expected_hash("house-a").unwrap();
        engine.cache_copy("node-a", "house-a", json!({"house":"house-a","epoch":1,"sequence":0,"records":2,"sha256":hash,"role":"active"}));
        assert!(engine.owner_ready(&lease));
        engine
            .nodes
            .get_mut("node-a")
            .unwrap()
            .pending_reacquire
            .insert("house-a".into());
        assert!(!engine.owner_ready(&lease));
        engine.recompute_links();
        assert!(
            !engine
                .allowed_links
                .read()
                .unwrap()
                .contains(&("house-a".into(), "node-a".into()))
        );
        engine
            .nodes
            .get_mut("node-a")
            .unwrap()
            .pending_reacquire
            .clear();
        engine.nodes.get_mut("node-a").unwrap().status["houses"][0]["epoch"] = 2.into();
        assert!(!engine.owner_ready(&lease));
    }
    #[test]
    fn confirmed_stop_blocks_effects_before_lease_expiry() {
        let mut engine = Engine::new(Bundle::fixture()).unwrap();
        engine.authority = Some(Arc::new(Mutex::new(
            Authority::new(engine.topology.clone(), engine.now()).unwrap(),
        )));
        engine.nodes = nodes();
        let hash = engine.expected_hash("house-a").unwrap();
        engine.cache_copy("node-a", "house-a", json!({"house":"house-a","epoch":1,"sequence":0,"records":2,"sha256":hash,"role":"active"}));
        assert!(
            engine
                .test_effect("house-a", "node-a", 1, "before-stop")
                .unwrap()
        );
        mark_fenced(engine.nodes.get_mut("node-a").unwrap());
        assert!(
            engine
                .test_effect("house-a", "node-a", 1, "after-stop")
                .is_err()
        );
    }
    #[test]
    fn clock_is_monotonic_and_measured_from_the_same_origin() {
        let origin = Instant::now();
        let first = clock_ms(origin);
        let second = clock_ms(origin);
        assert!(second >= first);
        let earlier = origin.checked_sub(Duration::from_millis(50)).unwrap();
        assert!(clock_ms(earlier) >= 50);
    }
}
