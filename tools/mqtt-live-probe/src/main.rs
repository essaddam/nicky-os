use anyhow::{Context, Result, bail, ensure};
use moli_core::{
    Access, AggregateOp, AggregateSource, AggregateSpec, DependencyKind, DependencyRef, Device,
    DeviceId, Event, InstanceId, Kind, Origin, PointId, PointSpec, Semantic, Unit, Value,
};
use moli_runtime::{BoxFuture, Driver, DriverCtx, Hub, HubOptions, spawn_driver};
use moli_z2m::Catalog;
use rumqttc::{AsyncClient, Event as MqttEvent, MqttOptions, Packet, Publish, QoS};
use rustix::termios::{self, LocalModes, OptionalActions};
use serde::Serialize;
use serde_json::{Map, Value as Json, json};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::hint::black_box;
use std::io::{Write, stdin};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

mod alarm;
mod discovery;
mod enum_catalogue;
mod enum_store;
mod house;
mod lighting;
mod load_fixture;
mod load_test;
mod load_wire;
mod load_workflows;
#[path = "../../moteur-integre-banc/src/memory.rs"]
mod memory;
mod metadata;
mod switching;

#[global_allocator]
static ALLOCATOR: memory::Meter = memory::Meter;

/// Where a virtual is republished, with the display labels computed once from
/// memory: every retained message becomes self-describing.
#[derive(Clone)]
struct RepublishTarget {
    topic: String,
    nom: String,
    unite: Option<String>,
}

const AGGREGATE_NATIVE: &str = "probe-aggregate";
const MAX_PACKET: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug)]
struct Config {
    hosts: Vec<String>,
    port: u16,
    username: String,
    base: String,
    seconds: u64,
    bridges: u8,
    benchmark: bool,
    all_topics: bool,
    republish: Option<String>,
    republish_retain: bool,
    republish_clear: bool,
    virtual_tree: bool,
    self_test_commands: bool,
    virtual_mirror: bool,
    publish_config: bool,
    destination: Option<String>,
    metadata_memory: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Phase {
    Catalog,
    Parse,
    Decode,
    Apply,
    Message,
}

impl Phase {
    fn name(self) -> &'static str {
        match self {
            Self::Catalog => "catalog",
            Self::Parse => "parse",
            Self::Decode => "decode",
            Self::Apply => "hub_apply",
            Self::Message => "whole_message",
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
struct Latency {
    samples: usize,
    p50_ns: u128,
    p95_ns: u128,
    max_ns: u128,
}

fn latency(values: &mut [u128]) -> Latency {
    values.sort_unstable();
    Latency {
        samples: values.len(),
        p50_ns: values.get(values.len() / 2).copied().unwrap_or(0),
        p95_ns: values
            .get(values.len().saturating_sub(1) * 95 / 100)
            .copied()
            .unwrap_or(0),
        max_ns: values.last().copied().unwrap_or(0),
    }
}

#[derive(Debug, Default)]
struct Metrics {
    house_projection: bool,
    house_counts: Json,
    connected: bool,
    transport_errors: u64,
    catalogs: u64,
    catalog_errors: u64,
    devices: usize,
    numeric_points: usize,
    units: BTreeMap<&'static str, usize>,
    messages: u64,
    retained: u64,
    retained_reapplied: u64,
    republish_enabled: bool,
    subscriptions_accepted: u64,
    subscriptions_refused: u64,
    qos: BTreeMap<u8, u64>,
    payload_bytes: usize,
    max_payload_bytes: usize,
    duplicate_payloads: u64,
    invalid_json: u64,
    ignored_messages: u64,
    ignored_points: u64,
    values_applied: u64,
    applied_value_forms: BTreeMap<&'static str, u64>,
    applied_text_lengths: BTreeMap<&'static str, u64>,
    state_events: u64,
    computed_events: u64,
    republished: u64,
    republish_roundtrips: u64,
    republish_unanswered: u64,
    virtuals: usize,
    mirror_topics: usize,
    config_bytes: usize,
    aliases: usize,
    alias_republished: u64,
    commands_received: u64,
    commands_accepted: u64,
    commands_rejected: BTreeMap<&'static str, u64>,
    aggregate_sources: usize,
    aggregate_unit: Option<&'static str>,
    aggregate_checks: u64,
    aggregate_parity_failures: u64,
    aggregate_removed: u64,
    timings: BTreeMap<&'static str, Vec<u128>>,
    requested: BTreeMap<&'static str, usize>,
    catalog_live_bytes: usize,
    catalog_kinds: BTreeMap<&'static str, usize>,
    payload_hashes: HashSet<u64>,
    pending_subscriptions: Vec<String>,
    pending_republishes: Vec<(String, String)>,
    early_buffer: Vec<(String, Vec<u8>)>,
    pre_catalog_messages: u64,
    early_dropped: u64,
    early_replayed: u64,
    discovery: discovery::Discovery,
}

impl Metrics {
    fn lock(this: &Arc<Mutex<Self>>) -> MutexGuard<'_, Self> {
        this.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn measure<T>(&mut self, phase: Phase, operation: impl FnOnce() -> T) -> T {
        let before = memory::total();
        let started = Instant::now();
        let result = operation();
        self.timings
            .entry(phase.name())
            .or_default()
            .push(started.elapsed().as_nanos());
        *self.requested.entry(phase.name()).or_default() += memory::total() - before;
        result
    }

    fn public(&self) -> Json {
        let timings: BTreeMap<_, _> = self
            .timings
            .iter()
            .map(|(phase, values)| (*phase, latency(&mut values.clone())))
            .collect();
        let mut public = json!({
            "schema": 1,
            "mode": if self.house_projection { "real_registry_statestream_poc_projection_not_k1" } else { "rust_real_codec_hub_and_computed" },
            "house_counts": self.house_counts,
            "privacy": "no_topics_no_device_names_no_payload_values_no_files",
            "effects": if self.republish_enabled {
                "subscribe_plus_bounded_virtual_republish_under_moli_root"
            } else {
                "subscribe_only_no_application_publish"
            },
            "connected": self.connected,
            "transport_errors": self.transport_errors,
            "catalogs": self.catalogs,
            "catalog_errors": self.catalog_errors,
            "devices": self.devices,
            "numeric_points": self.numeric_points,
            "units": self.units,
            "messages": self.messages,
            "retained": self.retained,
            "retained_reapplied": self.retained_reapplied,
            "subscriptions_accepted": self.subscriptions_accepted,
            "subscriptions_refused": self.subscriptions_refused,
            "qos": self.qos,
            "payload_bytes": self.payload_bytes,
            "max_payload_bytes": self.max_payload_bytes,
            "duplicate_payloads": self.duplicate_payloads,
            "invalid_json": self.invalid_json,
            "ignored_messages": self.ignored_messages,
            "ignored_points": self.ignored_points,
            "values_applied": self.values_applied,
            "state_events": self.state_events,
            "computed_events": self.computed_events,
            "aggregate_sources": self.aggregate_sources,
            "aggregate_unit": self.aggregate_unit,
            "aggregate_checks": self.aggregate_checks,
            "aggregate_parity_failures": self.aggregate_parity_failures,
            "aggregate_removed": self.aggregate_removed,
        });
        let extra = json!({
            "applied_value_forms": self.applied_value_forms,
            "republished": self.republished,
            "republish_roundtrips": self.republish_roundtrips,
            "republish_unanswered": self.republish_unanswered,
            "virtuals": self.virtuals,
            "mirror_topics": self.mirror_topics,
            "config_bytes": self.config_bytes,
            "applied_text_lengths": self.applied_text_lengths,
            "aliases": self.aliases,
            "alias_republished": self.alias_republished,
            "pre_catalog_messages": self.pre_catalog_messages,
            "early_dropped": self.early_dropped,
            "early_replayed": self.early_replayed,
            "commands_received": self.commands_received,
            "commands_accepted": self.commands_accepted,
            "commands_rejected": self.commands_rejected,
            "catalog_kinds": self.catalog_kinds,
            "timings": timings,
            "requested_bytes": self.requested,
            "catalog_live_requested_bytes": self.catalog_live_bytes,
            "process_live_requested_bytes": memory::live(),
            "process_peak_requested_bytes": memory::peak(),
            "memory_definition": "allocator_requested_bytes_not_rss; process-wide counters",
            "ha_discovery": self.discovery.public(),
        });
        public
            .as_object_mut()
            .expect("metrics report is an object")
            .extend(
                extra
                    .as_object()
                    .expect("extra report is an object")
                    .clone(),
            );
        public
    }
}

fn safe_unit(unit: &Unit) -> &'static str {
    match unit.symbol() {
        "°C" => "°C",
        "%" => "%",
        "W" => "W",
        "Wh" => "Wh",
        "kWh" => "kWh",
        "V" => "V",
        "mV" => "mV",
        "A" => "A",
        "VA" => "VA",
        "lx" => "lx",
        "s" => "s",
        "min" => "min",
        "h" => "h",
        "dB" => "dB",
        "dBm" => "dBm",
        "ppm" => "ppm",
        "l" => "l",
        "bar" => "bar",
        _ => "custom",
    }
}

fn is_request(rest: &str) -> bool {
    rest.ends_with("/set")
        || rest.ends_with("/get")
        || rest.contains("/set/")
        || rest.contains("/get/")
}

fn kind_name(kind: &Kind) -> &'static str {
    match kind {
        Kind::Timer { .. } => "timer",
        Kind::TimerCommand { .. } => "timer_command",
        Kind::Temporal { .. } => "temporal",
        Kind::Computed => "computed",
        Kind::Schedule => "schedule",
        Kind::Counter { .. } => "counter",
        Kind::CounterCommand { .. } => "counter_command",
        Kind::Button => "button",
        Kind::Binary => "binary",
        Kind::Numeric { .. } => "numeric",
        Kind::Enum { .. } => "enum",
        Kind::EnumCode { .. } => "enum_code",
        Kind::Text => "text",
    }
}

fn value_form(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Int(_) => "int",
        Value::Float(_) => "float",
        Value::Text(_) => "text",
        Value::EnumCode(_) => "enum_code",
    }
}

fn text_bucket(len: usize) -> &'static str {
    match len {
        0..=8 => "<=8",
        9..=16 => "9-16",
        17..=64 => "17-64",
        65..=256 => "65-256",
        257..=1024 => "257-1024",
        1025..=4000 => "1025-4000",
        _ => ">4000",
    }
}

fn republish_value(value: &Value) -> Option<Json> {
    match value {
        Value::Null => Some(Json::Null),
        Value::Bool(b) => Some(Json::Bool(*b)),
        Value::Int(i) => Some(json!(i)),
        Value::Float(f) => f.is_finite().then(|| json!(f)),
        Value::Text(t) => (t.len() <= 256).then(|| json!(t)),
        Value::EnumCode(code) => Some(json!(code)),
    }
}

fn valid_republish_prefix(prefix: &str) -> bool {
    let mut segments = prefix.split('/');
    let first = segments.next().unwrap_or_default();
    (first == "moli" || first == "moli-probe")
        && prefix.len() <= 64
        && !prefix.ends_with('/')
        && segments.all(|segment| !segment.is_empty())
        && prefix
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '/'))
}

struct RepublishTracker {
    sent: HashMap<u64, Instant>,
    order: VecDeque<u64>,
}

impl RepublishTracker {
    fn new() -> Self {
        Self {
            sent: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn mark_sent(&mut self, seq: u64, at: Instant) {
        if self.order.len() >= 64
            && let Some(oldest) = self.order.pop_front()
        {
            self.sent.remove(&oldest);
        }
        self.sent.insert(seq, at);
        self.order.push_back(seq);
    }

    fn observe(&mut self, payload: &[u8], at: Instant) -> Option<Duration> {
        let parsed = serde_json::from_slice::<Json>(payload).ok()?;
        let seq = parsed.get("seq")?.as_u64()?;
        let sent = self.sent.remove(&seq)?;
        self.order.retain(|pending| *pending != seq);
        Some(at.saturating_duration_since(sent))
    }

    fn unanswered(&self) -> u64 {
        self.sent.len() as u64
    }
}

fn online(payload: &[u8]) -> bool {
    #[derive(serde::Deserialize)]
    struct Availability {
        state: String,
    }
    serde_json::from_slice::<Availability>(payload).map_or_else(
        |_| String::from_utf8_lossy(payload).trim().to_owned(),
        |availability| availability.state,
    ) == "online"
}

fn apply_catalog(ctx: &DriverCtx, current: &mut Catalog, fresh: Catalog) {
    let aggregate = DeviceId::new(ctx.instance(), AGGREGATE_NATIVE);
    let keep: HashSet<DeviceId> = fresh
        .entries()
        .map(|entry| entry.device.id.clone())
        .collect();
    for id in ctx.devices() {
        if id != aggregate && !keep.contains(&id) {
            ctx.remove_device(&id);
        }
    }
    for entry in fresh.entries() {
        ctx.upsert_device(entry.device.clone());
    }
    *current = fresh;
}

fn catalog_shape(catalog: &Catalog, metrics: &mut Metrics) {
    metrics.devices = catalog.len();
    metrics.numeric_points = 0;
    metrics.units.clear();
    metrics.catalog_kinds.clear();
    for entry in catalog.entries() {
        for point in &entry.device.points {
            *metrics
                .catalog_kinds
                .entry(kind_name(&point.kind))
                .or_default() += 1;
            if point.access.read && point.kind.is_numeric() {
                metrics.numeric_points += 1;
                if let Some(unit) = &point.unit {
                    *metrics.units.entry(safe_unit(unit)).or_default() += 1;
                }
            }
        }
    }
}

fn process(
    hub: &Hub,
    ctx: &DriverCtx,
    base: &str,
    catalog: &mut Catalog,
    publish: &Publish,
    metrics: &mut Metrics,
    alias_topics: &HashMap<DeviceId, String>,
) {
    let discovered = metrics.discovery.inspect(&publish.topic, &publish.payload);
    metrics.pending_subscriptions.extend(discovered);
    metrics.messages += 1;
    metrics.retained += u64::from(publish.retain);
    *metrics.qos.entry(publish.qos as u8).or_default() += 1;
    metrics.payload_bytes += publish.payload.len();
    metrics.max_payload_bytes = metrics.max_payload_bytes.max(publish.payload.len());
    let Some(rest) = publish
        .topic
        .strip_prefix(base)
        .and_then(|topic| topic.strip_prefix('/'))
    else {
        metrics.ignored_messages += 1;
        return;
    };
    if rest == "bridge/devices" {
        match metrics.measure(Phase::Catalog, || {
            Catalog::from_bridge_devices(ctx.instance(), &publish.payload)
        }) {
            Ok(fresh) => {
                let before = memory::live();
                apply_catalog(ctx, catalog, fresh);
                metrics.catalog_live_bytes = memory::live().saturating_sub(before);
                metrics.catalogs += 1;
                catalog_shape(catalog, metrics);
            }
            Err(_) => metrics.catalog_errors += 1,
        }
        return;
    }
    if rest.starts_with("bridge/") || is_request(rest) {
        metrics.ignored_messages += 1;
        return;
    }
    if catalog.is_empty() {
        if metrics.early_buffer.len() < 512 {
            metrics
                .early_buffer
                .push((publish.topic.clone(), publish.payload.to_vec()));
            metrics.pre_catalog_messages += 1;
        } else {
            metrics.early_dropped += 1;
        }
        return;
    }
    if let Some(name) = rest.strip_suffix("/availability") {
        if let Some(entry) = catalog.by_name(name) {
            ctx.set_availability(&entry.device.id, online(&publish.payload));
        } else {
            metrics.ignored_messages += 1;
        }
        return;
    }
    let Some(entry) = catalog.by_name(rest) else {
        metrics.ignored_messages += 1;
        return;
    };
    let mut hash = DefaultHasher::new();
    publish.payload.hash(&mut hash);
    if !metrics.payload_hashes.insert(hash.finish()) {
        metrics.duplicate_payloads += 1;
    }
    let started = Instant::now();
    let allocated = memory::total();
    let parsed = metrics.measure(Phase::Parse, || {
        serde_json::from_slice::<Json>(&publish.payload)
    });
    let Ok(Json::Object(payload)) = parsed else {
        metrics.invalid_json += 1;
        return;
    };
    let decoded = metrics.measure(Phase::Decode, || entry.decode(&payload));
    let mut alias_payload = alias_topics.contains_key(&entry.device.id).then(Map::new);
    for (key, value) in decoded {
        let Some(spec) = entry.device.point(&key) else {
            metrics.ignored_points += 1;
            continue;
        };
        if !spec.kind.uses_state_cache() || spec.kind.is_operation() || spec.kind.is_occurrence() {
            metrics.ignored_points += 1;
            continue;
        }
        let point = PointId::new(&entry.device.id, &key);
        let before = publish.retain.then(|| hub.state(&point)).flatten();
        *metrics
            .applied_value_forms
            .entry(value_form(&value))
            .or_default() += 1;
        if let Value::Text(text) = &value {
            *metrics
                .applied_text_lengths
                .entry(text_bucket(text.len()))
                .or_default() += 1;
        }
        metrics.measure(Phase::Apply, || {
            ctx.set_state(&entry.device.id, &key, value.clone());
        });
        if let (Some(map), Some(json)) = (&mut alias_payload, republish_value(&value)) {
            map.insert(key.to_string(), json);
        }
        if let Some(before) = before
            && hub
                .state(&point)
                .is_some_and(|after| before.value == after.value && before.ts <= after.ts)
        {
            metrics.retained_reapplied += 1;
        }
        metrics.values_applied += 1;
    }
    if let (Some(topic), Some(map)) = (alias_topics.get(&entry.device.id), alias_payload) {
        metrics.alias_republished += 1;
        let payload = json!({
            "nom": entry.friendly_name,
            "secteur": entry.mains,
            "etat": Json::Object(map),
        })
        .to_string();
        metrics.pending_republishes.push((topic.clone(), payload));
    }
    drop(payload);
    metrics
        .timings
        .entry(Phase::Message.name())
        .or_default()
        .push(started.elapsed().as_nanos());
    *metrics.requested.entry(Phase::Message.name()).or_default() += memory::total() - allocated;
}

fn aggregate_definition(catalog: &Catalog) -> Option<AggregateSpec> {
    let mut groups: BTreeMap<String, (Unit, Vec<PointId>, usize)> = BTreeMap::new();
    for entry in catalog.entries() {
        for point in &entry.device.points {
            if !point.access.read
                || !point.kind.is_numeric()
                || !point.kind.uses_state_cache()
                || point.kind.is_operation()
                || point.kind.is_occurrence()
            {
                continue;
            }
            let Some(unit) = point.unit.clone() else {
                continue;
            };
            let priority = match point.semantic {
                Semantic::Temperature => 0,
                Semantic::Power => 1,
                _ => 2,
            };
            let group = groups
                .entry(unit.symbol().to_owned())
                .or_insert_with(|| (unit.clone(), Vec::new(), priority));
            let point_id = PointId::new(&entry.device.id, &point.key);
            if !group.1.contains(&point_id) {
                group.1.push(point_id);
            }
        }
    }
    let mut candidates: Vec<_> = groups.into_values().collect();
    candidates.sort_by_key(|(unit, sources, priority)| {
        (
            sources.len() < 2,
            *priority,
            std::cmp::Reverse(sources.len()),
            unit.symbol().to_owned(),
        )
    });
    let (unit, mut sources, _) = candidates.into_iter().next()?;
    sources.truncate(32);
    Some(AggregateSpec {
        operation: AggregateOp::Mean,
        sources,
        unit: Some(unit),
        max_age_s: 300,
    })
}

fn computed_device(instance: &InstanceId, unit: &Unit) -> Device {
    Device {
        id: DeviceId::new(instance, AGGREGATE_NATIVE),
        instance: instance.clone(),
        native_name: "Synthetic aggregate".into(),
        manufacturer: None,
        model: None,
        description: None,
        native_room: None,
        members: Vec::new(),
        points: vec![PointSpec {
            key: "value".into(),
            label: "Computed".into(),
            kind: Kind::Computed,
            access: Access {
                read: true,
                write: false,
            },
            unit: Some(unit.clone()),
            semantic: Semantic::Other,
        }],
    }
}

async fn install_aggregate(
    hub: &Hub,
    ctx: &DriverCtx,
    catalog: &Catalog,
    metrics: &Arc<Mutex<Metrics>>,
) -> Result<Option<AggregateSpec>> {
    let Some(spec) = aggregate_definition(catalog) else {
        return Ok(None);
    };
    let spec = spec
        .normalize()
        .map_err(|_| anyhow::anyhow!("invalid aggregate definition"))?;
    let id = DeviceId::new(ctx.instance(), AGGREGATE_NATIVE);
    let unit = spec.unit.clone().context("aggregate has explicit unit")?;
    ctx.upsert_device(computed_device(ctx.instance(), &unit));
    ctx.set_availability(&id, true);
    ctx.initialize_computed(&id, spec.clone()).await?;
    let reference = DependencyRef {
        kind: DependencyKind::Computed,
        id: id.clone(),
    };
    let preview = hub.preview_dependency(&reference).await?;
    hub.publish_dependency(preview, true, Origin::System, None)
        .await?;
    {
        let mut metrics = Metrics::lock(metrics);
        metrics.aggregate_sources = spec.sources.len();
        metrics.aggregate_unit = Some(safe_unit(&unit));
    }
    Ok(Some(spec))
}

fn computed_device_at(instance: &InstanceId, native: &str, unit: Option<Unit>) -> Device {
    Device {
        id: DeviceId::new(instance, native),
        instance: instance.clone(),
        native_name: native.into(),
        manufacturer: None,
        model: None,
        description: None,
        native_room: None,
        members: Vec::new(),
        points: vec![PointSpec {
            key: "value".into(),
            label: "Computed".into(),
            kind: Kind::Computed,
            access: Access {
                read: true,
                write: false,
            },
            unit,
            semantic: Semantic::Other,
        }],
    }
}

async fn install_computed(
    hub: &Hub,
    ctx: &DriverCtx,
    device: &DeviceId,
    spec: &AggregateSpec,
    unit: Option<Unit>,
    native: &str,
) -> Result<()> {
    ctx.upsert_device(computed_device_at(ctx.instance(), native, unit));
    ctx.set_availability(device, true);
    ctx.initialize_computed(device, spec.clone()).await?;
    let reference = DependencyRef {
        kind: DependencyKind::Computed,
        id: device.clone(),
    };
    let preview = hub.preview_dependency(&reference).await?;
    hub.publish_dependency(preview, true, Origin::System, None)
        .await?;
    Ok(())
}

struct TreeNode {
    device: DeviceId,
    native: String,
    point: PointId,
    spec: AggregateSpec,
    topic_suffix: String,
}

const TREE_LEVELS: &[(&str, u64)] = &[("etage", 2), ("piece", 4), ("zone", 8)];

fn stable_bucket(point: &PointId, buckets: u64) -> u64 {
    let mut hash = DefaultHasher::new();
    point.hash(&mut hash);
    hash.finish() % buckets
}

fn build_tree(instance: &InstanceId, spec: &AggregateSpec, unit: &Unit) -> Vec<TreeNode> {
    let mut nodes = Vec::new();
    let maison = DeviceId::new(instance, "probe-tree-maison");
    nodes.push(TreeNode {
        point: PointId::new(&maison, "value"),
        device: maison,
        native: "probe-tree-maison".to_owned(),
        spec: spec.clone(),
        topic_suffix: "maison/state".to_owned(),
    });
    for (level, buckets) in TREE_LEVELS {
        for index in 0..*buckets {
            let sources: Vec<PointId> = spec
                .sources
                .iter()
                .filter(|point| stable_bucket(point, *buckets) == index)
                .cloned()
                .collect();
            if sources.is_empty() {
                continue;
            }
            let native = format!("probe-tree-{level}-{index}");
            let device = DeviceId::new(instance, &native);
            nodes.push(TreeNode {
                point: PointId::new(&device, "value"),
                device,
                native,
                spec: AggregateSpec {
                    operation: spec.operation,
                    sources,
                    unit: Some(unit.clone()),
                    max_age_s: spec.max_age_s,
                },
                topic_suffix: format!("maison/{level}/{index}/state"),
            });
        }
    }
    nodes
}

const EXTRA_AGGREGATES: &[(&str, &str, AggregateOp, &str)] = &[
    (
        "reseau/linkquality-moyenne",
        "linkquality",
        AggregateOp::Mean,
        "Qualité de lien moyenne",
    ),
    (
        "reseau/linkquality-min",
        "linkquality",
        AggregateOp::Min,
        "Maillon le plus faible",
    ),
    (
        "energie/batterie-min",
        "battery",
        AggregateOp::Min,
        "Batterie la plus faible",
    ),
    (
        "energie/batterie-moyenne",
        "battery",
        AggregateOp::Mean,
        "Batterie moyenne",
    ),
];

fn build_extra_aggregates(instance: &InstanceId, catalog: &Catalog) -> Vec<TreeNode> {
    EXTRA_AGGREGATES
        .iter()
        .filter_map(|(name, key, operation, _label)| {
            let mut sources = Vec::new();
            let mut unit = None;
            for entry in catalog.entries() {
                let Some(point) = entry.device.point(key) else {
                    continue;
                };
                if point.access.read && point.kind.is_numeric() && point.kind.uses_state_cache() {
                    let point_id = PointId::new(&entry.device.id, &point.key);
                    if !sources.contains(&point_id) {
                        if unit.is_none() {
                            unit = point.unit.clone();
                        }
                        sources.push(point_id);
                    }
                }
            }
            sources.sort();
            sources.truncate(32);
            if sources.len() < 2 {
                return None;
            }
            let native = format!("probe-extra-{}", name.replace('/', "-"));
            let device = DeviceId::new(instance, &native);
            let spec = AggregateSpec {
                operation: *operation,
                sources,
                unit,
                max_age_s: 300,
            }
            .normalize()
            .ok()?;
            Some(TreeNode {
                point: PointId::new(&device, "value"),
                device,
                native,
                spec,
                topic_suffix: format!("{name}/state"),
            })
        })
        .collect()
}

fn role_name(role: &str) -> &'static str {
    match role {
        "porte" => "Porte",
        "presence" => "Présence",
        "lumiere" => "Lumière",
        "prise" => "Prise",
        "volet" => "Volet",
        _ => "Autre",
    }
}

fn node_label(suffix: &str) -> String {
    let parts: Vec<&str> = suffix.split('/').collect();
    match parts.as_slice() {
        ["maison", "state"] => "Maison".to_owned(),
        ["maison", level, index, "state"] => {
            let number = index.parse::<u64>().unwrap_or(0) + 1;
            match *level {
                "etage" => format!("Étage {number}"),
                "piece" => format!("Pièce {number}"),
                "zone" => format!("Zone {number}"),
                other => other.to_owned(),
            }
        }
        [group, name, "state"] => EXTRA_AGGREGATES
            .iter()
            .find(|(topic, _, _, _)| *topic == format!("{group}/{name}"))
            .map_or_else(
                || (*name).to_owned(),
                |(_, _, _, label)| (*label).to_owned(),
            ),
        _ => suffix.to_owned(),
    }
}

/// The retained configuration document: everything an interface needs to build
/// itself at first open (names, topics, units, command contracts), after which
/// only retained state changes flow. Names stay on the user's own broker and
/// are never part of the probe's own report.
fn build_config(
    prefix: &str,
    catalog: &Catalog,
    tree: &[TreeNode],
    alias_topics: &HashMap<DeviceId, String>,
) -> Json {
    let mut noeuds: Vec<Json> = tree
        .iter()
        .map(|node| {
            json!({
                "nom": node_label(&node.topic_suffix),
                "topic": format!("{prefix}/{}", node.topic_suffix),
                "unite": node.spec.unit.as_ref().map(|unit| unit.symbol()),
                "sources": node.spec.sources.len(),
            })
        })
        .collect();
    noeuds.sort_by(|a, b| a["topic"].as_str().cmp(&b["topic"].as_str()));
    let mut appareils: Vec<Json> = alias_topics
        .iter()
        .filter_map(|(id, topic)| {
            let entry = catalog.entries().find(|entry| &entry.device.id == id)?;
            let points: Vec<Json> = entry
                .device
                .points
                .iter()
                .filter(|point| point.access.read && point.kind.uses_state_cache())
                .take(24)
                .map(|point| {
                    json!({
                        "cle": point.key,
                        "nom": point.label,
                        "semantique": point.semantic,
                        "genre": kind_name(&point.kind),
                        "unite": point.unit.as_ref().map(|unit| unit.symbol()),
                    })
                })
                .collect();
            Some(json!({
                "nom": entry.friendly_name,
                "topic": topic,
                "secteur": entry.mains,
                "points": points,
            }))
        })
        .collect();
    appareils.sort_by(|a, b| a["topic"].as_str().cmp(&b["topic"].as_str()));
    let mut roles: Vec<Json> = alias_topics
        .iter()
        .filter(|(_, topic)| topic.contains("/alias/"))
        .filter_map(|(_, topic)| {
            let role = topic.rsplit('/').nth(1)?;
            let writable = alias_writable(role);
            Some(json!({
                "role": role,
                "nom": role_name(role),
                "etat": topic,
                "commande": writable.then(|| format!("{prefix}/alias/{role}/set")),
                "contrat": if role == "lumiere" {
                    json!({"state": ["ON", "OFF"], "brightness": {"min": 0, "max": 254}})
                } else if writable {
                    json!({"state": ["ON", "OFF"]})
                } else {
                    Json::Null
                },
            }))
        })
        .collect();
    roles.sort_by(|a, b| a["role"].as_str().cmp(&b["role"].as_str()));
    json!({
        "v": 1,
        "nom": "Maison",
        "prefix": prefix,
        "maison_topic": format!("{prefix}/maison/state"),
        "noeuds": noeuds,
        "appareils": appareils,
        "roles": roles,
    })
}

#[cfg(test)]
fn republish_topics(prefix: &str) -> Vec<String> {
    let mut topics = vec![
        format!("{prefix}/config"),
        format!("{prefix}/aggregate/state"),
        format!("{prefix}/maison/state"),
    ];
    for (level, buckets) in TREE_LEVELS {
        for index in 0..*buckets {
            topics.push(format!("{prefix}/maison/{level}/{index}/state"));
        }
    }
    for role in ALIAS_ROLES {
        topics.push(format!("{prefix}/alias/{role}/state"));
        topics.push(format!("{prefix}/alias/{role}/set"));
    }
    for (name, _, _, _) in EXTRA_AGGREGATES {
        topics.push(format!("{prefix}/{name}/state"));
    }
    topics
}

const ALIAS_ROLES: &[&str] = &["porte", "presence", "lumiere", "prise", "volet"];

fn alias_role(device: &Device) -> Option<&'static str> {
    let has = |key: &str| device.point(key).is_some();
    if has("contact") {
        return Some("porte");
    }
    if has("occupancy") {
        return Some("presence");
    }
    if has("brightness") {
        return Some("lumiere");
    }
    if has("position") {
        return Some("volet");
    }
    if has("power") && has("state") {
        return Some("prise");
    }
    None
}

fn alias_writable(role: &str) -> bool {
    matches!(role, "lumiere" | "prise")
}

fn build_aliases(catalog: &Catalog) -> Vec<(DeviceId, &'static str)> {
    let mut chosen: Vec<(DeviceId, &'static str)> = Vec::new();
    for entry in catalog.entries() {
        let Some(role) = alias_role(&entry.device) else {
            continue;
        };
        if !chosen.iter().any(|(_, existing)| *existing == role) {
            chosen.push((entry.device.id.clone(), role));
        }
    }
    chosen
}

/// Validates an inbound command for a virtual alias. Never executes anything:
/// the verdict only proves the bounded contract a guarded bridge would apply.
fn validate_command(role: &str, payload: &[u8]) -> Result<(), &'static str> {
    if !ALIAS_ROLES.contains(&role) {
        return Err("unknown_role");
    }
    if !alias_writable(role) {
        return Err("read_only_virtual");
    }
    if payload.len() > 256 {
        return Err("too_long");
    }
    let Ok(Json::Object(object)) = serde_json::from_slice::<Json>(payload) else {
        return Err("bad_json");
    };
    let mut accepted = false;
    for (key, value) in object {
        match (role, key.as_str(), &value) {
            (_, "state", Json::String(state)) if state == "ON" || state == "OFF" => accepted = true,
            ("lumiere", "brightness", Json::Number(level))
                if level.as_u64().is_some_and(|level| level <= 254) =>
            {
                accepted = true
            }
            _ => return Err("bad_value"),
        }
    }
    if accepted {
        Ok(())
    } else {
        Err("empty_contract")
    }
}

fn expected(hub: &Hub, spec: &AggregateSpec) -> Option<moli_core::Sample> {
    let snapshot = hub.snapshot();
    let devices: BTreeMap<DeviceId, _> = snapshot
        .devices
        .into_iter()
        .map(|view| (view.device.id.clone(), view))
        .collect();
    let inputs: Vec<_> = spec
        .sources
        .iter()
        .map(|point| {
            let Some((id, key)) = point.split() else {
                return AggregateSource {
                    sample: None,
                    unit: None,
                    online: false,
                    numeric: false,
                };
            };
            let Some(view) = devices.get(&id) else {
                return AggregateSource {
                    sample: None,
                    unit: None,
                    online: false,
                    numeric: false,
                };
            };
            let Some(point_spec) = view.device.point(key) else {
                return AggregateSource {
                    sample: None,
                    unit: None,
                    online: false,
                    numeric: false,
                };
            };
            AggregateSource {
                sample: view.state.get(key).cloned(),
                unit: point_spec.unit.clone(),
                online: view.online != Some(false),
                numeric: point_spec.access.read && point_spec.kind.is_numeric(),
            }
        })
        .collect();
    spec.evaluate(&inputs, moli_core::now_ms()).sample
}

fn computed_point(instance: &InstanceId) -> PointId {
    PointId::new(&DeviceId::new(instance, AGGREGATE_NATIVE), "value")
}

async fn check_parity(hub: &Hub, spec: &AggregateSpec, instance: &InstanceId) -> Result<bool> {
    let point = computed_point(instance);
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let expected = expected(hub, spec);
            let actual = hub.state(&point);
            let equal = match (&expected, &actual) {
                (None, Some(sample)) => sample.value == Value::Null,
                (Some(expected), Some(actual)) => {
                    expected.value == actual.value && expected.ts == actual.ts
                }
                _ => false,
            };
            if equal {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .context("aggregate parity timeout")
}

async fn record_parity_at(
    hub: &Hub,
    spec: &AggregateSpec,
    point: &PointId,
    metrics: &Arc<Mutex<Metrics>>,
) -> Result<()> {
    let equal = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let expected = expected(hub, spec);
            let actual = hub.state(point);
            let equal = match (&expected, &actual) {
                (None, Some(sample)) => sample.value == Value::Null,
                (Some(expected), Some(actual)) => {
                    expected.value == actual.value && expected.ts == actual.ts
                }
                _ => false,
            };
            if equal {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .context("aggregate parity timeout")?;
    let mut metrics = Metrics::lock(metrics);
    metrics.aggregate_checks += 1;
    metrics.aggregate_parity_failures += u64::from(!equal);
    Ok(())
}

async fn record_parity(
    hub: &Hub,
    spec: &AggregateSpec,
    instance: &InstanceId,
    metrics: &Arc<Mutex<Metrics>>,
) -> Result<()> {
    let equal = check_parity(hub, spec, instance).await?;
    let mut metrics = Metrics::lock(metrics);
    metrics.aggregate_checks += 1;
    metrics.aggregate_parity_failures += u64::from(!equal);
    Ok(())
}

struct Live {
    config: Config,
    host: String,
    password: String,
    hub: Hub,
    metrics: Arc<Mutex<Metrics>>,
    client_id: String,
    instance: InstanceId,
}

impl Driver for Live {
    fn kind(&self) -> &'static str {
        "z2m"
    }

    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            if self.config.virtual_tree {
                let result = house::run(self, ctx).await;
                if let Err(error) = &result {
                    // Top-level errors are controlled labels, without source data.
                    #[cfg(test)]
                    eprintln!("HOUSE_TEST_ERROR {error:#}");
                    eprintln!("HOUSE_ERROR {}", error);
                }
                return result;
            }
            let mut options = MqttOptions::new(&self.client_id, &self.host, self.config.port);
            options.set_keep_alive(Duration::from_secs(15));
            options.set_max_packet_size(MAX_PACKET, 256 * 1024);
            options.set_credentials(&self.config.username, &self.password);
            let (client, mut events) = AsyncClient::new(options, 64);
            let base = self.config.base.trim_end_matches('/');
            let republish_prefix = self.config.republish.as_ref().map(|p| format!("{p}/"));
            let aggregate_point = computed_point(&self.instance);
            let mut republish_topics: HashMap<PointId, RepublishTarget> = HashMap::new();
            let mut last_republished: HashMap<PointId, Value> = HashMap::new();
            let mut alias_topics: HashMap<DeviceId, String> = HashMap::new();
            let mut known_catalogs = 0_u64;
            let mut self_test_sent = false;
            let mut config_published = false;
            let mut tree_nodes: Vec<TreeNode> = Vec::new();
            let mut tracker = RepublishTracker::new();
            let mut sequence = 0_u64;
            let mut hub_events = self.hub.subscribe();
            let mut catalog = Catalog::default();
            let mut subscribed_states = HashSet::new();
            let mut aggregate = None;
            let mut last_parity = Instant::now() - Duration::from_secs(1);
            loop {
                tokio::select! {
                    () = ctx.cancelled() => {
                        Metrics::lock(&self.metrics).republish_unanswered = tracker.unanswered();
                        if let Some(spec) = aggregate {
                            record_parity(&self.hub, &spec, &self.instance, &self.metrics).await?;
                            for node in &tree_nodes {
                                record_parity_at(&self.hub, &node.spec, &node.point, &self.metrics).await?;
                            }
                        }
                        return Ok(());
                    }
                    hub_event = hub_events.recv() => {
                        if let Ok(Event::State { point, .. }) = hub_event
                            && let Some(target) = republish_topics.get(&point).cloned()
                            && let Some(sample) = self.hub.state(&point)
                        {
                            let changed = last_republished.get(&point) != Some(&sample.value);
                            let value = republish_value(&sample.value);
                            if changed
                                && let Some(value) = value
                            {
                                sequence += 1;
                                let payload = json!({
                                    "nom": target.nom,
                                    "unite": target.unite,
                                    "value": value,
                                    "ts": sample.ts,
                                    "seq": sequence,
                                }).to_string();
                                tracker.mark_sent(sequence, Instant::now());
                                client.try_publish(target.topic, QoS::AtLeastOnce, self.config.republish_retain, payload)
                                    .context("mqtt republish rejected locally")?;
                                last_republished.insert(point, sample.value.clone());
                                Metrics::lock(&self.metrics).republished += 1;
                            }
                        }
                    }
                    event = events.poll() => {
                        let event = match event {
                            Ok(event) => event,
                            Err(_) => {
                                Metrics::lock(&self.metrics).transport_errors += 1;
                                bail!("mqtt connection failed");
                            }
                        };
                        match event {
                            MqttEvent::Incoming(Packet::SubAck(ack)) => {
                                let mut metrics = Metrics::lock(&self.metrics);
                                for code in ack.return_codes {
                                    match code {
                                        rumqttc::SubscribeReasonCode::Success(_) => metrics.subscriptions_accepted += 1,
                                        rumqttc::SubscribeReasonCode::Failure => metrics.subscriptions_refused += 1,
                                    }
                                }
                            }
                            MqttEvent::Incoming(Packet::ConnAck(_)) => {
                                if self.config.all_topics {
                                    client.try_subscribe("#", QoS::AtLeastOnce)
                                        .context("mqtt wildcard subscription rejected locally")?;
                                    client.try_subscribe("$SYS/#", QoS::AtLeastOnce)
                                        .context("mqtt sys subscription rejected locally")?;
                                } else {
                                    client.try_subscribe(format!("{base}/#"), QoS::AtLeastOnce)
                                        .context("mqtt subscription rejected locally")?;
                                    client.try_subscribe("homeassistant/#", QoS::AtLeastOnce)
                                        .context("mqtt discovery subscription rejected locally")?;
                                    if let Some(prefix) = &self.config.republish {
                                        client.try_subscribe(format!("{prefix}/#"), QoS::AtLeastOnce)
                                            .context("mqtt republish subscription rejected locally")?;
                                    }
                                }
                                Metrics::lock(&self.metrics).connected = true;
                                ctx.ready();
                            }
                            MqttEvent::Incoming(Packet::Publish(publish)) => {
                                if let Some(prefix) = republish_prefix.as_deref()
                                    && let Some(rest) = publish.topic.strip_prefix(prefix)
                                {
                                    if let Some(role) = rest
                                        .strip_prefix("alias/")
                                        .and_then(|topic| topic.strip_suffix("/set"))
                                    {
                                        let mut metrics = Metrics::lock(&self.metrics);
                                        metrics.commands_received += 1;
                                        let started = Instant::now();
                                        match validate_command(role, &publish.payload) {
                                            Ok(()) => metrics.commands_accepted += 1,
                                            Err(reason) => {
                                                *metrics
                                                    .commands_rejected
                                                    .entry(reason)
                                                    .or_default() += 1;
                                            }
                                        }
                                        metrics
                                            .timings
                                            .entry("command_validate")
                                            .or_default()
                                            .push(started.elapsed().as_nanos());
                                        continue;
                                    }
                                    let mut metrics = Metrics::lock(&self.metrics);
                                    if let Some(elapsed) = tracker.observe(&publish.payload, Instant::now()) {
                                        metrics.republish_roundtrips += 1;
                                        metrics
                                            .timings
                                            .entry("republish_roundtrip")
                                            .or_default()
                                            .push(elapsed.as_nanos());
                                    }
                                    continue;
                                }
                                let (subscriptions, republishes) = {
                                    let mut metrics = Metrics::lock(&self.metrics);
                                    process(&self.hub, ctx, base, &mut catalog, &publish, &mut metrics, &alias_topics);
                                    (
                                        std::mem::take(&mut metrics.pending_subscriptions),
                                        std::mem::take(&mut metrics.pending_republishes),
                                    )
                                };
                                for (topic, payload) in republishes {
                                    client.try_publish(topic, QoS::AtLeastOnce, self.config.republish_retain, payload)
                                        .context("mqtt alias republish rejected locally")?;
                                }
                                let catalogs_now = Metrics::lock(&self.metrics).catalogs;
                                if catalogs_now != known_catalogs {
                                    known_catalogs = catalogs_now;
                                    if let Some(prefix) = &self.config.republish {
                                        let prefix = prefix.clone();
                                        let mut roles = 0_usize;
                                        alias_topics = build_aliases(&catalog)
                                            .into_iter()
                                            .map(|(id, role)| {
                                                roles += 1;
                                                (id, format!("{prefix}/alias/{role}/state"))
                                            })
                                            .collect();
                                        let mut mirror = 0_usize;
                                        if self.config.virtual_mirror {
                                            let mut ids: Vec<DeviceId> = catalog
                                                .entries()
                                                .map(|entry| entry.device.id.clone())
                                                .collect();
                                            ids.sort();
                                            for (index, id) in ids.into_iter().take(128).enumerate()
                                            {
                                                if alias_topics.contains_key(&id) {
                                                    continue;
                                                }
                                                alias_topics.insert(
                                                    id,
                                                    format!("{prefix}/appareil/{index:03}/state"),
                                                );
                                                mirror += 1;
                                            }
                                        }
                                        let mut metrics = Metrics::lock(&self.metrics);
                                        metrics.aliases = roles;
                                        metrics.mirror_topics = mirror;
                                    }
                                    let buffered = {
                                        let mut metrics = Metrics::lock(&self.metrics);
                                        std::mem::take(&mut metrics.early_buffer)
                                    };
                                    if !buffered.is_empty() {
                                        let mut replayed = 0_u64;
                                        for (topic, payload) in buffered {
                                            let mut publish = Publish::new(topic, QoS::AtLeastOnce, payload);
                                            publish.retain = true;
                                            let mut metrics = Metrics::lock(&self.metrics);
                                            process(&self.hub, ctx, base, &mut catalog, &publish, &mut metrics, &alias_topics);
                                            replayed += 1;
                                        }
                                        let replayed_republishes = {
                                            let mut metrics = Metrics::lock(&self.metrics);
                                            metrics.early_replayed += replayed;
                                            std::mem::take(&mut metrics.pending_republishes)
                                        };
                                        for (topic, payload) in replayed_republishes {
                                            client.try_publish(topic, QoS::AtLeastOnce, self.config.republish_retain, payload)
                                                .context("mqtt alias republish rejected locally")?;
                                        }
                                    }
                                }
                                if self.config.self_test_commands
                                    && !self_test_sent
                                    && !alias_topics.is_empty()
                                {
                                    self_test_sent = true;
                                    let prefix = self.config.republish.clone().expect("checked above");
                                    let accepted = format!("{prefix}/alias/lumiere/set");
                                    let refused = format!("{prefix}/alias/porte/set");
                                    if alias_topics.values().any(|topic| *topic == format!("{prefix}/alias/lumiere/state")) {
                                        client.try_publish(accepted, QoS::AtLeastOnce, false, r#"{"state":"ON"}"#)
                                            .context("mqtt self-test publish rejected locally")?;
                                    }
                                    if alias_topics.values().any(|topic| *topic == format!("{prefix}/alias/porte/state")) {
                                        client.try_publish(refused, QoS::AtLeastOnce, false, r#"{"state":"ON"}"#)
                                            .context("mqtt self-test publish rejected locally")?;
                                    }
                                }
                                if !self.config.all_topics {
                                    for topic in subscriptions {
                                        if subscribed_states.insert(topic.clone()) && discovery::safe_subscription(&topic) {
                                            client.try_subscribe(topic, QoS::AtLeastOnce)
                                                .context("mqtt state subscription rejected locally")?;
                                        }
                                    }
                                }
                                if aggregate.is_none() && !catalog.is_empty() {
                                    aggregate = install_aggregate(&self.hub, ctx, &catalog, &self.metrics).await?;
                                    if let (Some(spec), Some(prefix)) = (&aggregate, &self.config.republish) {
                                        let prefix = prefix.clone();
                                        if let Some(unit) = spec.unit.clone() {
                                            republish_topics.insert(
                                                aggregate_point.clone(),
                                                RepublishTarget {
                                                    topic: format!("{prefix}/aggregate/state"),
                                                    nom: "Agrégat".to_owned(),
                                                    unite: Some(unit.symbol().to_owned()),
                                                },
                                            );
                                            if self.config.virtual_tree {
                                                for node in build_tree(&self.instance, spec, &unit) {
                                                    install_computed(&self.hub, ctx, &node.device, &node.spec, Some(unit.clone()), &node.native)
                                                        .await?;
                                                    republish_topics.insert(
                                                        node.point.clone(),
                                                        RepublishTarget {
                                                            topic: format!("{prefix}/{}", node.topic_suffix),
                                                            nom: node_label(&node.topic_suffix),
                                                            unite: Some(unit.symbol().to_owned()),
                                                        },
                                                    );
                                                    tree_nodes.push(node);
                                                }
                                                for node in build_extra_aggregates(&self.instance, &catalog) {
                                                    install_computed(&self.hub, ctx, &node.device, &node.spec, node.spec.unit.clone(), &node.native)
                                                        .await?;
                                                    republish_topics.insert(
                                                        node.point.clone(),
                                                        RepublishTarget {
                                                            topic: format!("{prefix}/{}", node.topic_suffix),
                                                            nom: node_label(&node.topic_suffix),
                                                            unite: node.spec.unit.as_ref().map(|u| u.symbol().to_owned()),
                                                        },
                                                    );
                                                    tree_nodes.push(node);
                                                }
                                                Metrics::lock(&self.metrics).virtuals = tree_nodes.len();
                                            }
                                        }
                                    }
                                    if self.config.publish_config && !config_published && aggregate.is_some() {
                                        config_published = true;
                                        let prefix = self.config.republish.clone().expect("checked above");
                                        let document = build_config(&prefix, &catalog, &tree_nodes, &alias_topics).to_string();
                                        Metrics::lock(&self.metrics).config_bytes = document.len();
                                        client.try_publish(
                                            format!("{prefix}/config"),
                                            QoS::AtLeastOnce,
                                            true,
                                            document,
                                        )
                                        .context("mqtt config publish rejected locally")?;
                                    }
                                }
                                if let Some(spec) = &aggregate
                                    && last_parity.elapsed() >= Duration::from_millis(250)
                                {
                                    record_parity(&self.hub, spec, &self.instance, &self.metrics).await?;
                                    for node in &tree_nodes {
                                        record_parity_at(&self.hub, &node.spec, &node.point, &self.metrics).await?;
                                    }
                                    last_parity = Instant::now();
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        })
    }
}

struct HiddenEcho {
    original: termios::Termios,
}

impl HiddenEcho {
    fn new() -> std::io::Result<Self> {
        let input = stdin();
        let original = termios::tcgetattr(&input)?;
        let mut hidden = original.clone();
        hidden.local_modes.remove(LocalModes::ECHO);
        termios::tcsetattr(&input, OptionalActions::Now, &hidden)?;
        Ok(Self { original })
    }
}

impl Drop for HiddenEcho {
    fn drop(&mut self) {
        let _ = termios::tcsetattr(stdin(), OptionalActions::Now, &self.original);
    }
}

fn password() -> Result<String> {
    if let Ok(value) = std::env::var("MQTT_PASSWORD") {
        ensure!(
            !value.is_empty() && !value.starts_with("{{secret:"),
            "MQTT_PASSWORD is empty or unresolved"
        );
        return Ok(value);
    }
    let input = stdin();
    let _hidden = HiddenEcho::new().context("a terminal is required for hidden password input")?;
    eprint!("Mot de passe MQTT (non affiché) : ");
    std::io::stderr().flush()?;
    let mut value = String::new();
    input.read_line(&mut value)?;
    eprintln!();
    if value.ends_with('\n') {
        value.pop();
        if value.ends_with('\r') {
            value.pop();
        }
    }
    ensure!(!value.is_empty(), "empty MQTT password");
    Ok(value)
}

fn arguments() -> Result<Config> {
    let mut config = Config {
        hosts: Vec::new(),
        port: 1883,
        username: "test-mqtt".into(),
        base: "zigbee2mqtt".into(),
        seconds: 15,
        bridges: 1,
        benchmark: false,
        all_topics: false,
        republish: None,
        republish_retain: false,
        republish_clear: false,
        virtual_tree: false,
        self_test_commands: false,
        virtual_mirror: false,
        publish_config: false,
        destination: None,
        metadata_memory: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--benchmark" => config.benchmark = true,
            "--all-topics" => config.all_topics = true,
            "--host" => config
                .hosts
                .push(args.next().context("missing --host value")?),
            "--port" => config.port = args.next().context("missing --port value")?.parse()?,
            "--username" => config.username = args.next().context("missing --username value")?,
            "--base" => config.base = args.next().context("missing --base value")?,
            "--seconds" => {
                config.seconds = args.next().context("missing --seconds value")?.parse()?
            }
            "--bridges" => {
                config.bridges = args.next().context("missing --bridges value")?.parse()?
            }
            "--republish" => {
                config.republish = Some(args.next().context("missing --republish value")?)
            }
            "--republish-retain" => config.republish_retain = true,
            "--republish-clear" => config.republish_clear = true,
            "--virtual-tree" => config.virtual_tree = true,
            "--self-test-commands" => config.self_test_commands = true,
            "--virtual-mirror" => config.virtual_mirror = true,
            "--publish-config" => config.publish_config = true,
            "--destination" => {
                config.destination = Some(args.next().context("missing destination")?)
            }
            "--metadata-memory" => config.metadata_memory = true,
            "--help" => {
                println!(
                    "mqtt-live-probe --host HOST [--host HOST2] [--port 1883] [--username test-mqtt] [--base zigbee2mqtt] [--seconds 15] [--bridges 2] [--all-topics] [--republish moli-probe] [--republish-retain] [--republish-clear] [--virtual-tree]\nmqtt-live-probe --benchmark"
                );
                std::process::exit(0);
            }
            _ => bail!("unknown argument"),
        }
    }
    if !config.benchmark {
        ensure!(!config.hosts.is_empty(), "--host is required");
        ensure!(
            config.hosts.len() <= usize::from(config.bridges),
            "--host may be repeated up to --bridges times"
        );
        ensure!(
            config.seconds == 0 || (5..=60).contains(&config.seconds),
            "--seconds must be 0 (continuous) or 5..60"
        );
        ensure!((1..=4).contains(&config.bridges), "--bridges must be 1..4");
        ensure!(
            !config.base.is_empty() && !config.base.contains(['#', '+']),
            "invalid base topic"
        );
        if let Some(prefix) = &config.republish {
            ensure!(valid_republish_prefix(prefix), "invalid --republish prefix");
        }
        ensure!(
            !config.virtual_tree || config.republish.is_some(),
            "--virtual-tree requires --republish"
        );
        ensure!(
            !config.republish_clear || config.republish.is_some(),
            "--republish-clear requires --republish"
        );
        ensure!(
            !config.self_test_commands || config.republish.is_some(),
            "--self-test-commands requires --republish"
        );
        ensure!(
            !config.virtual_mirror || config.republish.is_some(),
            "--virtual-mirror requires --republish"
        );
        ensure!(
            !config.publish_config || config.republish.is_some(),
            "--publish-config requires --republish"
        );
    }
    Ok(config)
}

struct DeltaMean {
    current: [Option<f64>; 32],
    sum: f64,
    count: usize,
}

impl DeltaMean {
    fn new() -> Self {
        Self {
            current: [None; 32],
            sum: 0.0,
            count: 0,
        }
    }

    fn update(&mut self, index: usize, next: Option<f64>) -> Result<()> {
        if let Some(previous) = self.current[index].take() {
            self.sum -= previous;
            self.count -= 1;
        }
        if let Some(value) = next {
            self.sum += value;
            self.count += 1;
            self.current[index] = Some(value);
        }
        ensure!(self.sum.is_finite(), "delta needs bounded recomputation");
        Ok(())
    }

    fn value(&self) -> Option<f64> {
        (self.count > 0).then(|| self.sum / self.count as f64)
    }
}

fn arithmetic_benchmark(rounds: usize) -> Json {
    let spec = AggregateSpec {
        operation: AggregateOp::Mean,
        sources: (0..32)
            .map(|index| PointId::from(format!("synthetic:s{index}/value")))
            .collect(),
        unit: Some(Unit::Watt),
        max_age_s: 300,
    };
    let inputs: Vec<_> = (0..32)
        .map(|index| AggregateSource {
            sample: Some(moli_core::Sample {
                value: Value::Float((index + 1) as f64),
                ts: 0,
            }),
            unit: Some(Unit::Watt),
            online: true,
            numeric: true,
        })
        .collect();
    let mut delta = DeltaMean::new();
    for (index, input) in inputs.iter().enumerate() {
        delta
            .update(index, input.sample.as_ref().and_then(|s| s.value.as_f64()))
            .expect("initial delta");
    }
    let allocated = memory::total();
    let started = Instant::now();
    for _ in 0..rounds {
        black_box(spec.evaluate(black_box(&inputs), 1));
    }
    let recalc_ns = started.elapsed().as_nanos() / rounds as u128;
    let recalc_bytes = memory::total() - allocated;
    let allocated = memory::total();
    let started = Instant::now();
    for round in 0..rounds {
        delta
            .update(round % 32, Some((round % 32 + 1) as f64))
            .expect("bounded delta");
        black_box(delta.value());
    }
    let delta_ns = started.elapsed().as_nanos() / rounds as u128;
    let delta_bytes = memory::total() - allocated;
    json!({
        "scope": "arithmetic_only_not_hub",
        "sources": 32,
        "rounds": rounds,
        "recalculate_ns": recalc_ns,
        "delta_ns": delta_ns,
        "recalculate_requested_bytes": recalc_bytes,
        "delta_requested_bytes": delta_bytes,
    })
}

struct Running {
    client_id: String,
    metrics: Arc<Mutex<Metrics>>,
    driver: tokio::task::JoinHandle<()>,
    events: tokio::task::JoinHandle<()>,
}

fn start_bridge(
    hub: &Hub,
    config: &Config,
    password: &str,
    index: usize,
    cancel: CancellationToken,
) -> Result<Running> {
    let metrics = Arc::new(Mutex::new(Metrics::default()));
    Metrics::lock(&metrics).republish_enabled = config.republish.is_some();
    let host = config.hosts[index % config.hosts.len()].clone();
    let instance = InstanceId::from(format!("z2m-{index}"));
    let aggregate_point = computed_point(&instance);
    let mut receiver = hub.subscribe();
    let event_metrics = metrics.clone();
    let event_cancel = cancel.clone();
    let source = instance.clone();
    let events = tokio::spawn(async move {
        loop {
            tokio::select! {
                () = event_cancel.cancelled() => return,
                event = receiver.recv() => match event {
                    Ok(Event::State { point, .. }) => {
                        let own = point
                            .split()
                            .is_some_and(|(device, _)| device.instance() == source);
                        if !own {
                            continue;
                        }
                        let mut metrics = Metrics::lock(&event_metrics);
                        if point == aggregate_point
                            || point
                                .split()
                                .is_some_and(|(device, _)| device.as_str().contains("probe-tree-"))
                        {
                            metrics.computed_events += 1;
                        } else {
                            metrics.state_events += 1;
                        }
                    }
                    Ok(_) => {}
                    Err(_) => return,
                }
            }
        }
    });
    let client_id = format!(
        "moli-v6-kimi-{}-{}-{index}",
        std::process::id(),
        moli_core::now_ms()
    );
    let driver = spawn_driver(
        hub,
        instance.clone(),
        Arc::new(Live {
            config: config.clone(),
            host,
            password: password.to_owned(),
            hub: hub.clone(),
            metrics: metrics.clone(),
            client_id: client_id.clone(),
            instance,
        }),
        cancel,
    );
    Ok(Running {
        client_id,
        metrics,
        driver,
        events,
    })
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let audit_args: Vec<_> = std::env::args().skip(1).collect();
    if audit_args
        .first()
        .is_some_and(|arg| arg.starts_with("--synthetic-load"))
    {
        return load_test::run(&audit_args).await;
    }
    if audit_args.first().is_some_and(|arg| arg == "--ha-seed") {
        let paths: Vec<_> = audit_args
            .iter()
            .skip(1)
            .map(std::path::PathBuf::from)
            .collect();
        println!("{}", house::ha_seeds(&paths)?);
        return Ok(());
    }
    if audit_args
        .first()
        .is_some_and(|arg| arg == "--offline-enum-audit")
    {
        let paths: Vec<_> = audit_args
            .iter()
            .skip(1)
            .map(std::path::PathBuf::from)
            .collect();
        println!("ENUM_AUDIT {}", house::audit_exports(&paths)?);
        return Ok(());
    }
    let config = arguments()?;
    if config.benchmark {
        println!(
            "{}",
            serde_json::to_string_pretty(&arithmetic_benchmark(100_000))?
        );
        return Ok(());
    }
    memory::reset_peak();
    let password = password()?;
    if config.republish_clear {
        let prefix = config
            .republish
            .clone()
            .context("clear needs --republish")?;
        let host = config.hosts[0].clone();
        let mut options = MqttOptions::new(
            format!("moli-v6-kimi-clear-{}", std::process::id()),
            &host,
            config.port,
        );
        options.set_credentials(&config.username, &password);
        let (client, mut events) = AsyncClient::new(options, 64);
        let mut retained: Vec<String> = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(6);
        while Instant::now() < deadline {
            match events.poll().await {
                Ok(MqttEvent::Incoming(Packet::ConnAck(_))) => {
                    client
                        .try_subscribe(format!("{prefix}/#"), QoS::AtLeastOnce)
                        .context("mqtt clear subscription rejected locally")?;
                }
                Ok(MqttEvent::Incoming(Packet::Publish(publish))) => {
                    if publish.retain && !retained.contains(&publish.topic) {
                        retained.push(publish.topic.clone());
                    }
                }
                Ok(_) => {}
                Err(_) => bail!("mqtt connection failed during clear"),
            }
        }
        let count = retained.len();
        for topic in retained {
            client
                .try_publish(topic, QoS::AtLeastOnce, true, Vec::<u8>::new())
                .context("mqtt clear rejected locally")?;
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
        println!("{{\"cleared\": true, \"topics_cleared\": {count}}}");
        return Ok(());
    }
    let hub = Hub::new(HubOptions {
        event_capacity: 4096,
        journal_capacity: 16,
        ..HubOptions::default()
    })?;
    let cancel = CancellationToken::new();
    let mut runs = Vec::new();
    for index in 0..usize::from(config.bridges) {
        runs.push(start_bridge(
            &hub,
            &config,
            &password,
            index,
            cancel.clone(),
        )?);
    }
    if config.seconds == 0 {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let stats = hub.stats();
            let bridges:Vec<_>=runs.iter().map(|r|{let m=Metrics::lock(&r.metrics);json!({"connected":m.connected,"messages":m.messages,"published":m.republished,"projection":m.house_counts})}).collect();
            println!(
                "POC_RUNTIME {}",
                json!({"hub_devices":stats.devices,"hub_points":stats.points,"drivers_running":stats.drivers_running,"allocator_live_bytes":memory::live(),"allocator_peak_bytes":memory::peak(),"sources":bridges})
            );
        }
    }
    tokio::time::sleep(Duration::from_secs(config.seconds)).await;
    cancel.cancel();
    let mut client_ids = HashSet::new();
    let mut reports = Vec::with_capacity(runs.len());
    let metric_handles: Vec<_> = runs.iter().map(|run| run.metrics.clone()).collect();
    for run in runs {
        client_ids.insert(run.client_id);
        run.driver.await.context("driver task failed")?;
        run.events.await.context("event task failed")?;
        reports.push(Metrics::lock(&run.metrics).public());
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": 1,
            "bridge_count": reports.len(),
            "all_topics": config.all_topics,
            "unique_client_ids": client_ids.len() == reports.len(),
            "shared_hub_devices": hub.stats().devices,
            "shared_hub_points": hub.stats().points,
            "effects": if config.republish.is_some() { "bounded_publications_under_moli_root" } else { "subscribe_only_no_application_publish" },
            "bridges": reports,
        }))?
    );
    let summary: Vec<Json> = metric_handles
        .iter()
        .map(|handle| {
            let metrics = Metrics::lock(handle);
            json!({
                "devices": metrics.devices,
                "virtuals": metrics.virtuals,
                "mirror_topics": metrics.mirror_topics,
                "config_bytes": metrics.config_bytes,
                "aliases": metrics.aliases,
                "alias_republished": metrics.alias_republished,
                "republished": metrics.republished,
                "republish_roundtrips": metrics.republish_roundtrips,
                "republish_unanswered": metrics.republish_unanswered,
                "commands_received": metrics.commands_received,
                "commands_accepted": metrics.commands_accepted,
                "commands_rejected": metrics.commands_rejected,
                "pre_catalog_messages": metrics.pre_catalog_messages,
                "early_replayed": metrics.early_replayed,
                "values_applied": metrics.values_applied,
                "aggregate_parity_failures": metrics.aggregate_parity_failures,
            })
        })
        .collect();
    println!("SUMMARY {}", serde_json::to_string(&summary)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn fixture_catalog() -> Catalog {
        Catalog::from_bridge_devices(
            &"z2m".into(),
            include_bytes!("../../../integrations/z2m/fixtures/bridge_devices.json"),
        )
        .unwrap()
    }

    fn fixture_messages(catalog: &Catalog) -> Vec<Publish> {
        let all: Json = serde_json::from_slice(include_bytes!(
            "../../../integrations/z2m/fixtures/state.json"
        ))
        .unwrap();
        catalog
            .entries()
            .filter_map(|entry| {
                let payload = all.get(&entry.ieee)?;
                let mut publish = Publish::new(
                    format!("zigbee2mqtt/{}", entry.friendly_name),
                    QoS::AtLeastOnce,
                    payload.to_string().into_bytes(),
                );
                publish.retain = true;
                Some(publish)
            })
            .collect()
    }

    struct Scripted {
        hub: Hub,
        messages: Vec<Publish>,
        remove_sources: bool,
        metrics: Arc<Mutex<Metrics>>,
    }

    impl Driver for Scripted {
        fn kind(&self) -> &'static str {
            "z2m"
        }

        fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
            Box::pin(async move {
                let mut catalog = Catalog::default();
                apply_catalog(ctx, &mut catalog, fixture_catalog());
                catalog_shape(&catalog, &mut Metrics::lock(&self.metrics));
                let aggregate = install_aggregate(&self.hub, ctx, &catalog, &self.metrics).await?;
                ctx.ready();
                for publish in &self.messages {
                    process(
                        &self.hub,
                        ctx,
                        "zigbee2mqtt",
                        &mut catalog,
                        publish,
                        &mut Metrics::lock(&self.metrics),
                        &HashMap::new(),
                    );
                }
                if let Some(spec) = &aggregate {
                    let instance = ctx.instance().clone();
                    record_parity(&self.hub, spec, &instance, &self.metrics).await?;
                    if self.remove_sources {
                        let devices: BTreeSet<_> = spec
                            .sources
                            .iter()
                            .filter_map(|point| point.split().map(|(id, _)| id))
                            .collect();
                        for id in devices {
                            ctx.remove_device(&id);
                            Metrics::lock(&self.metrics).aggregate_removed += 1;
                        }
                        record_parity(&self.hub, spec, &instance, &self.metrics).await?;
                    }
                }
                Ok(())
            })
        }
    }

    async fn scripted(messages: Vec<Publish>, remove_sources: bool) -> (Hub, Arc<Mutex<Metrics>>) {
        let hub = Hub::new(HubOptions::default()).unwrap();
        let metrics = Arc::new(Mutex::new(Metrics::default()));
        let cancel = CancellationToken::new();
        let aggregate_point = PointId::from("z2m:probe-aggregate/value");
        let mut events = hub.subscribe();
        let event_metrics = metrics.clone();
        let event_cancel = cancel.clone();
        let event_task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    () = event_cancel.cancelled() => return,
                    event = events.recv() => match event {
                        Ok(Event::State { point, .. }) => {
                            let mut metrics = Metrics::lock(&event_metrics);
                            if point == aggregate_point {
                                metrics.computed_events += 1;
                            } else {
                                metrics.state_events += 1;
                            }
                        }
                        Ok(_) => {}
                        Err(_) => return,
                    }
                }
            }
        });
        let task = spawn_driver(
            &hub,
            "z2m".into(),
            Arc::new(Scripted {
                hub: hub.clone(),
                messages,
                remove_sources,
                metrics: metrics.clone(),
            }),
            cancel.clone(),
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
        cancel.cancel();
        task.await.unwrap();
        event_task.await.unwrap();
        (hub, metrics)
    }

    #[test]
    fn real_catalog_chooses_explicit_numeric_sources() {
        let spec = aggregate_definition(&fixture_catalog()).unwrap();
        assert!(!spec.sources.is_empty());
        assert!(spec.sources.len() <= 32);
        assert!(spec.unit.is_some());
        assert_eq!(spec.operation, AggregateOp::Mean);
        for source in &spec.sources {
            let (device, key) = source.split().unwrap();
            assert!(fixture_catalog().entries().any(|entry| {
                entry.device.id == device
                    && entry.device.point(key).is_some_and(|point| {
                        point.access.read && point.kind.is_numeric() && point.unit == spec.unit
                    })
            }));
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn real_messages_reach_hub_and_real_computed_projection() {
        let (hub, metrics) = scripted(fixture_messages(&fixture_catalog()), false).await;
        let metrics = Metrics::lock(&metrics);
        assert!(!metrics.connected);
        assert!(metrics.values_applied > 0);
        assert!(metrics.state_events > 0);
        assert!(metrics.aggregate_checks > 0);
        assert_eq!(metrics.aggregate_parity_failures, 0);
        assert!(metrics.computed_events > 0);
        assert_eq!(metrics.retained, metrics.messages);
        assert!(metrics.devices > 0);
        assert!(metrics.numeric_points > 0);
        assert!(hub.stats().points > 0);
        assert!(!metrics.public().to_string().contains("0x00124b"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn equal_payload_does_not_create_a_second_raw_state_event() {
        let messages: Vec<_> = fixture_messages(&fixture_catalog())
            .into_iter()
            .take(1)
            .flat_map(|publish| [publish.clone(), publish])
            .collect();
        let (_, metrics) = scripted(messages, false).await;
        let metrics = Metrics::lock(&metrics);
        assert!(metrics.values_applied > metrics.state_events);
        assert_eq!(metrics.duplicate_payloads, 1);
        assert!(metrics.retained_reapplied > 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn removed_sources_produce_unavailable_not_zero() {
        let (hub, metrics) = scripted(fixture_messages(&fixture_catalog()), true).await;
        let sample = hub
            .state(&PointId::from("z2m:probe-aggregate/value"))
            .unwrap();
        assert_eq!(sample.value, Value::Null);
        assert_eq!(Metrics::lock(&metrics).aggregate_parity_failures, 0);
        assert!(Metrics::lock(&metrics).aggregate_removed > 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn requests_and_unknown_payloads_have_no_effect_and_no_private_output() {
        let marker = "NE_DOIT_PAS_SORTIR";
        let publishes = vec![
            Publish::new(
                "zigbee2mqtt/synthetic/set",
                QoS::AtLeastOnce,
                marker.as_bytes().to_vec(),
            ),
            Publish::new(
                format!("zigbee2mqtt/{marker}"),
                QoS::AtLeastOnce,
                marker.as_bytes().to_vec(),
            ),
        ];
        let (_, metrics) = scripted(publishes, false).await;
        let metrics = Metrics::lock(&metrics);
        assert_eq!(metrics.values_applied, 0);
        assert!(metrics.ignored_messages >= 2);
        assert!(!metrics.public().to_string().contains(marker));
    }

    #[test]
    fn delta_reference_handles_removal_expiry_and_requires_fallback_on_overflow() {
        let spec = AggregateSpec {
            operation: AggregateOp::Mean,
            sources: vec!["fixture:a/value".into(), "fixture:b/value".into()],
            unit: Some(Unit::Watt),
            max_age_s: 1,
        };
        let mut inputs = vec![
            AggregateSource {
                sample: Some(moli_core::Sample {
                    value: Value::Float(10.0),
                    ts: 0,
                }),
                unit: Some(Unit::Watt),
                online: true,
                numeric: true,
            },
            AggregateSource {
                sample: Some(moli_core::Sample {
                    value: Value::Float(20.0),
                    ts: 0,
                }),
                unit: Some(Unit::Watt),
                online: true,
                numeric: true,
            },
        ];
        let mut delta = DeltaMean::new();
        delta.update(0, Some(10.0)).unwrap();
        delta.update(1, Some(20.0)).unwrap();
        assert_eq!(delta.value(), Some(15.0));
        delta.update(0, None).unwrap();
        assert_eq!(
            delta.value(),
            spec.evaluate(&inputs[1..], 500)
                .sample
                .map(|s| s.value.as_f64().unwrap())
        );
        inputs[0].sample = None;
        assert_eq!(
            spec.evaluate(&inputs, 500)
                .sample
                .map(|s| s.value.as_f64().unwrap()),
            Some(20.0)
        );
        assert_eq!(spec.evaluate(&inputs, 1_001).sample, None);
        let extreme = AggregateSource {
            sample: Some(moli_core::Sample {
                value: Value::Float(f64::MAX),
                ts: 0,
            }),
            unit: Some(Unit::Watt),
            online: true,
            numeric: true,
        };
        let mut delta = DeltaMean::new();
        delta.update(0, Some(f64::MAX)).unwrap();
        assert!(delta.update(1, Some(f64::MAX)).is_err());
        assert_eq!(
            spec.evaluate(&[extreme.clone(), extreme], 1)
                .sample
                .map(|s| s.value),
            Some(Value::Float(f64::MAX))
        );
    }

    #[test]
    #[ignore = "compteur global du processus : exécuter isolément, sans autre test en parallèle"]
    fn arithmetic_paths_are_bounded_and_allocation_counted() {
        let result = arithmetic_benchmark(1_000);
        assert_eq!(result["sources"], 32);
        assert_eq!(result["recalculate_requested_bytes"], 0);
        assert_eq!(result["delta_requested_bytes"], 0);
        assert!(result["recalculate_ns"].as_u64().is_some());
        assert!(result["delta_ns"].as_u64().is_some());
    }

    #[test]
    fn availability_matches_driver_contract() {
        assert!(online(br#"{"state":"online"}"#));
        assert!(!online(br#"{"state":"offline"}"#));
        assert!(online(b"online"));
        assert!(!online(b"offline"));
    }

    #[test]
    fn republish_prefix_is_confined_to_a_neutral_moli_root() {
        assert!(valid_republish_prefix("moli-probe"));
        assert!(valid_republish_prefix("moli-probe/aggregate"));
        assert!(valid_republish_prefix("moli"));
        assert!(!valid_republish_prefix("zigbee2mqtt"));
        assert!(!valid_republish_prefix("homeassistant/x"));
        assert!(!valid_republish_prefix("moli-probe/#"));
        assert!(!valid_republish_prefix("moli-probe/+"));
        assert!(!valid_republish_prefix("moli-probe/"));
        assert!(!valid_republish_prefix("Moli-probe"));
        assert!(!valid_republish_prefix(""));
        assert!(!valid_republish_prefix("moli-probe//aggregate"));
    }

    #[test]
    fn republish_values_are_bounded_and_tracker_measures_roundtrip() {
        assert!(republish_value(&Value::Float(f64::NAN)).is_none());
        assert!(republish_value(&Value::Float(f64::INFINITY)).is_none());
        assert!(republish_value(&Value::Float(21.5)).is_some());
        assert!(republish_value(&Value::Null).is_some());
        assert!(republish_value(&Value::Bool(true)).is_some());
        assert!(republish_value(&Value::Int(7)).is_some());
        assert!(republish_value(&Value::Text("court".into())).is_some());
        assert!(republish_value(&Value::Text("x".repeat(300).into())).is_none());

        let mut tracker = RepublishTracker::new();
        let now = Instant::now();
        tracker.mark_sent(1, now);
        tracker.mark_sent(2, now);
        let payload = json!({"value": 21.5, "ts": 42, "seq": 1}).to_string();
        let elapsed = tracker.observe(payload.as_bytes(), now + Duration::from_millis(7));
        assert_eq!(elapsed, Some(Duration::from_millis(7)));
        assert_eq!(tracker.unanswered(), 1);
        assert_eq!(tracker.observe(b"not json", now), None);
        assert_eq!(tracker.observe(br#"{"seq":99}"#, now), None);
        assert_eq!(tracker.observe(payload.as_bytes(), now), None);
        for seq in 10..100 {
            tracker.mark_sent(seq, now);
        }
        assert!(tracker.unanswered() <= 64);
    }

    #[test]
    fn kinds_and_value_forms_cover_the_real_taxonomy() {
        let catalog = fixture_catalog();
        let mut names: Vec<&'static str> = catalog
            .entries()
            .flat_map(|entry| entry.device.points.iter().map(|p| kind_name(&p.kind)))
            .collect();
        names.sort_unstable();
        names.dedup();
        assert!(names.contains(&"numeric"));
        assert!(!names.is_empty());
        assert_eq!(value_form(&Value::Null), "null");
        assert_eq!(value_form(&Value::Bool(false)), "bool");
        assert_eq!(value_form(&Value::Int(1)), "int");
        assert_eq!(value_form(&Value::Float(1.5)), "float");
        assert_eq!(value_form(&Value::Text("x".into())), "text");
    }

    #[test]
    fn text_buckets_cover_the_boundary_question() {
        assert_eq!(text_bucket(0), "<=8");
        assert_eq!(text_bucket(8), "<=8");
        assert_eq!(text_bucket(9), "9-16");
        assert_eq!(text_bucket(64), "17-64");
        assert_eq!(text_bucket(256), "65-256");
        assert_eq!(text_bucket(1024), "257-1024");
        assert_eq!(text_bucket(4000), "1025-4000");
        assert_eq!(text_bucket(4001), ">4000");
    }

    #[test]
    fn tree_partitions_all_sources_deterministically() {
        let catalog = fixture_catalog();
        let spec = aggregate_definition(&catalog).unwrap();
        let unit = spec.unit.clone().unwrap();
        let instance = InstanceId::from("z2m-9");
        let nodes = build_tree(&instance, &spec, &unit);
        assert!(nodes.len() > 1);
        assert_eq!(nodes[0].topic_suffix, "maison/state");
        assert_eq!(nodes[0].spec.sources.len(), spec.sources.len());
        let mut expected = spec.sources.clone();
        expected.sort();
        let mut levels: BTreeMap<&str, Vec<PointId>> = BTreeMap::new();
        for node in nodes.iter().skip(1) {
            let level = node.topic_suffix.split('/').nth(1).unwrap_or("?");
            levels
                .entry(level)
                .or_default()
                .extend(node.spec.sources.iter().cloned());
        }
        assert_eq!(levels.len(), TREE_LEVELS.len());
        for sources in levels.values_mut() {
            let mut seen = sources.clone();
            seen.sort();
            assert_eq!(seen, expected, "each level partitions all sources");
            let unique: HashSet<_> = sources.iter().collect();
            assert_eq!(unique.len(), sources.len(), "no duplicate within a level");
        }
        for node in &nodes {
            assert!(node.spec.clone().normalize().is_ok());
            assert!(node.topic_suffix.starts_with("maison"));
        }
        let again = build_tree(&instance, &spec, &unit);
        assert_eq!(
            nodes
                .iter()
                .map(|n| n.topic_suffix.clone())
                .collect::<Vec<_>>(),
            again
                .iter()
                .map(|n| n.topic_suffix.clone())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn each_channel_in_retained_config_has_its_own_label_and_semantic() {
        let catalog = fixture_catalog();
        let aliases = catalog
            .entries()
            .map(|e| {
                (
                    e.device.id.clone(),
                    "moli-probe/appareil/test/state".to_owned(),
                )
            })
            .collect();
        let config = build_config("moli-probe", &catalog, &[], &aliases);
        let points = config["appareils"][0]["points"].as_array().unwrap();
        assert!(!points.is_empty());
        for point in points {
            assert!(
                point["nom"].is_string(),
                "a channel must keep its display label"
            );
            assert!(
                point["semantique"].is_string(),
                "units alone cannot identify room generics"
            );
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pre_catalog_messages_are_buffered_then_replayed() {
        struct CatalogLast {
            hub: Hub,
            messages: Vec<Publish>,
            metrics: Arc<Mutex<Metrics>>,
        }
        impl Driver for CatalogLast {
            fn kind(&self) -> &'static str {
                "z2m"
            }
            fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
                Box::pin(async move {
                    let mut catalog = Catalog::default();
                    let aliases = HashMap::new();
                    for publish in &self.messages {
                        process(
                            &self.hub,
                            ctx,
                            "zigbee2mqtt",
                            &mut catalog,
                            publish,
                            &mut Metrics::lock(&self.metrics),
                            &aliases,
                        );
                    }
                    apply_catalog(ctx, &mut catalog, fixture_catalog());
                    let buffered = {
                        let mut metrics = Metrics::lock(&self.metrics);
                        std::mem::take(&mut metrics.early_buffer)
                    };
                    assert!(!buffered.is_empty());
                    let mut replayed = 0_u64;
                    for (topic, payload) in buffered {
                        let mut publish = Publish::new(topic, QoS::AtLeastOnce, payload);
                        publish.retain = true;
                        process(
                            &self.hub,
                            ctx,
                            "zigbee2mqtt",
                            &mut catalog,
                            &publish,
                            &mut Metrics::lock(&self.metrics),
                            &aliases,
                        );
                        replayed += 1;
                    }
                    Metrics::lock(&self.metrics).early_replayed = replayed;
                    ctx.ready();
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    Ok(())
                })
            }
        }
        let hub = Hub::new(HubOptions::default()).unwrap();
        let metrics = Arc::new(Mutex::new(Metrics::default()));
        let cancel = CancellationToken::new();
        let messages = fixture_messages(&fixture_catalog());
        let task = spawn_driver(
            &hub,
            "z2m".into(),
            Arc::new(CatalogLast {
                hub: hub.clone(),
                messages,
                metrics: metrics.clone(),
            }),
            cancel.clone(),
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
        cancel.cancel();
        task.await.unwrap();
        let metrics = Metrics::lock(&metrics);
        assert_eq!(metrics.pre_catalog_messages, metrics.early_replayed);
        assert!(metrics.values_applied > 0);
        assert_eq!(metrics.early_dropped, 0);
    }

    #[test]
    fn extra_aggregates_target_real_signals_with_bounded_sources() {
        let catalog = fixture_catalog();
        let instance = InstanceId::from("z2m-9");
        let nodes = build_extra_aggregates(&instance, &catalog);
        assert!(!nodes.is_empty());
        for node in &nodes {
            assert!(node.spec.sources.len() >= 2);
            assert!(node.spec.sources.len() <= 32);
            assert!(!node.topic_suffix.contains("probe"));
            for source in &node.spec.sources {
                let (device, key) = source.split().unwrap();
                let point = catalog
                    .entries()
                    .find(|entry| entry.device.id == device)
                    .and_then(|entry| entry.device.point(key))
                    .expect("source exists in catalog");
                assert!(point.access.read && point.kind.is_numeric());
            }
        }
    }

    #[test]
    fn config_document_carries_structure_names_and_contracts() {
        let catalog = fixture_catalog();
        let instance = InstanceId::from("z2m-9");
        let spec = aggregate_definition(&catalog).unwrap();
        let unit = spec.unit.clone().unwrap();
        let tree = build_tree(&instance, &spec, &unit);
        let mut alias_topics: HashMap<DeviceId, String> = build_aliases(&catalog)
            .into_iter()
            .map(|(id, role)| (id, format!("moli-probe/alias/{role}/state")))
            .collect();
        let mut ids: Vec<DeviceId> = catalog
            .entries()
            .map(|entry| entry.device.id.clone())
            .collect();
        ids.sort();
        for (index, id) in ids.into_iter().enumerate() {
            alias_topics
                .entry(id)
                .or_insert_with(|| format!("moli-probe/appareil/{index:03}/state"));
        }
        let config = build_config("moli-probe", &catalog, &tree, &alias_topics);
        assert_eq!(config["v"], 1);
        assert_eq!(config["nom"], "Maison");
        let noeuds = config["noeuds"].as_array().unwrap();
        assert!(!noeuds.is_empty());
        assert!(noeuds.iter().any(|node| node["nom"] == "Maison"));
        assert!(noeuds.iter().any(|node| {
            node["nom"]
                .as_str()
                .is_some_and(|nom| nom.starts_with("Étage"))
        }));
        let appareils = config["appareils"].as_array().unwrap();
        assert_eq!(appareils.len(), catalog.len());
        for appareil in appareils {
            assert!(appareil["nom"].as_str().is_some_and(|nom| !nom.is_empty()));
            assert!(
                appareil["topic"]
                    .as_str()
                    .unwrap()
                    .starts_with("moli-probe/")
            );
            assert!(!appareil["points"].as_array().unwrap().is_empty());
        }
        let roles = config["roles"].as_array().unwrap();
        for role in roles {
            let name = role["role"].as_str().unwrap();
            if alias_writable(name) {
                assert!(role["commande"].as_str().is_some_and(|topic| {
                    topic.starts_with("moli-probe/") && topic.ends_with("/set")
                }));
                assert!(role["contrat"].is_object());
            } else {
                assert!(role["commande"].is_null());
            }
        }
        let text = config.to_string();
        assert!(text.contains("moli-probe/"));
    }

    #[test]
    fn republish_topic_inventory_stays_under_the_neutral_root() {
        let topics = republish_topics("moli-probe");
        assert!(topics.len() > 10);
        for topic in &topics {
            assert!(topic.starts_with("moli-probe/"));
            if topic.ends_with("/set") {
                assert!(
                    !discovery::safe_subscription(topic),
                    "command topics are never subscribed as state"
                );
            } else {
                assert!(discovery::safe_subscription(topic));
            }
        }
        assert!(topics.contains(&"moli-probe/maison/state".to_owned()));
        assert!(topics.contains(&"moli-probe/maison/zone/7/state".to_owned()));
        assert!(topics.contains(&"moli-probe/alias/porte/state".to_owned()));
        assert!(topics.contains(&"moli-probe/alias/lumiere/set".to_owned()));
    }

    #[test]
    fn alias_roles_are_detected_by_capability_not_by_name() {
        let catalog = fixture_catalog();
        let aliases = build_aliases(&catalog);
        assert!(!aliases.is_empty());
        for (id, role) in &aliases {
            let entry = catalog
                .entries()
                .find(|entry| &entry.device.id == id)
                .expect("alias source exists");
            assert!(alias_role(&entry.device) == Some(*role));
        }
        let roles: Vec<_> = aliases.iter().map(|(_, role)| *role).collect();
        let mut unique = roles.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(roles.len(), unique.len(), "one device per role");
    }

    #[test]
    fn command_contract_is_bounded_and_never_executes_readonly() {
        assert!(validate_command("lumiere", br#"{"state":"ON"}"#).is_ok());
        assert!(validate_command("lumiere", br#"{"brightness":254}"#).is_ok());
        assert!(validate_command("prise", br#"{"state":"OFF"}"#).is_ok());
        assert_eq!(
            validate_command("porte", br#"{"state":"ON"}"#),
            Err("read_only_virtual")
        );
        assert_eq!(
            validate_command("lumiere", br#"{"brightness":255}"#),
            Err("bad_value")
        );
        assert_eq!(
            validate_command("lumiere", br#"{"transition":1}"#),
            Err("bad_value")
        );
        assert_eq!(validate_command("lumiere", b"not json"), Err("bad_json"));
        assert_eq!(
            validate_command("lumiere", &vec![b'x'; 300]),
            Err("too_long")
        );
        assert_eq!(validate_command("lumiere", br#"{"x":1}"#), Err("bad_value"));
        assert_eq!(validate_command("lumiere", b"{}"), Err("empty_contract"));
        assert_eq!(
            validate_command("inconnu", br#"{"state":"ON"}"#),
            Err("unknown_role")
        );
    }
}
