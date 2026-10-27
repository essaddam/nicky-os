//! Charge synthétique des workflows sur le vrai Hub et le moteur moli-automation.
//!
//! Les points d'entrée sont importés par House depuis le faux statestream du
//! banc. Ce module ne les écrit jamais dans le Hub. Il ne déclare qu'un pilote
//! local dédié pour absorber les deux sorties synthétiques de chaque copie.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use moli_automation::{
    Actor, Author, Automation, Automations, Channel, Config, Edge, Graph, Mode, Node, Op, Rule,
    Step,
};
use moli_core::{
    Access, Device, DeviceId, Event, InstanceId, Kind, PointId, PointSpec, Semantic, Unit, Value,
};
use moli_runtime::{BoxFuture, Driver, DriverCtx, Hub, spawn_driver};
use serde::Serialize;
use serde_json::json;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

const EXPECTED_SOURCES: usize = 2;
const MAX_COPIES_PER_SOURCE: usize = 10;
const INPUT_WAIT: Duration = Duration::from_secs(20);
const DRIVER_INSTANCE: &str = "load-workflow-output";
pub(super) const TEMP_ABOVE_C: f64 = 22.0;
const THRESHOLD_HOLD_S: u64 = 1;
const THRESHOLD_DELAY_S: u64 = 1;
const MAX_DURATION_SAMPLES: usize = 512;

const THRESHOLD_OUTPUT: &str = "threshold_output";
const OPENING_OUTPUT: &str = "opening_output";

/// Stable IDs and Hub point IDs expected from the MQTT bridge's synthetic
/// registry rows. Registry IDs use the exact prefix + UTF-8-hex scheme in
/// `House::id`; values are ordinary sensor text/numbers, never home data.
#[derive(Clone, Debug)]
pub(super) struct InputPoints {
    pub source_index: usize,
    pub copy: usize,
    pub instance: InstanceId,
    pub device_registry_id: String,
    pub temperature_registry_id: String,
    pub opening_registry_id: String,
    pub mode_registry_id: String,
    pub temperature_entity_id: String,
    pub opening_entity_id: String,
    pub mode_entity_id: String,
    pub temperature: PointId,
    pub opening: PointId,
    pub mode: PointId,
}

/// Derive the three input identities for one synthetic copy. The MQTT source
/// creates the corresponding registry/state rows; this helper is also used by
/// the workflow graphs, so they cannot drift from the bridge projection.
pub(super) fn input_points(source_index: usize, copy: usize) -> Result<InputPoints> {
    ensure!(source_index < EXPECTED_SOURCES, "source index out of range");
    ensure!(copy < MAX_COPIES_PER_SOURCE, "copy index out of range");

    let instance = InstanceId::from(format!("load-{source_index}-{copy}"));
    let suffix = format!("{source_index}-{copy:03}");
    let device_registry_id = format!("load-house-{suffix}");
    let temperature_registry_id = format!("load-temperature-{suffix}");
    let opening_registry_id = format!("load-opening-{suffix}");
    let mode_registry_id = format!("load-mode-{suffix}");
    let temperature_entity_id = format!("sensor.load_{source_index}_{copy:03}_temperature");
    let opening_entity_id = format!("binary_sensor.load_{source_index}_{copy:03}_opening");
    let mode_entity_id = format!("sensor.load_{source_index}_{copy:03}_mode");

    let device = DeviceId::new(&instance, &registry_id("appareil", &device_registry_id));
    let point = |native: &str| PointId::new(&device, &registry_id("canal", native));

    Ok(InputPoints {
        source_index,
        copy,
        instance,
        device_registry_id,
        temperature: point(&temperature_registry_id),
        opening: point(&opening_registry_id),
        mode: point(&mode_registry_id),
        temperature_registry_id,
        opening_registry_id,
        mode_registry_id,
        temperature_entity_id,
        opening_entity_id,
        mode_entity_id,
    })
}

fn registry_id(prefix: &str, native: &str) -> String {
    use std::fmt::Write as _;
    let mut result = format!("{prefix}-");
    for byte in native.bytes() {
        write!(result, "{byte:02x}").expect("writing to String cannot fail");
    }
    result
}

/// Aggregated counters only; no source/entity IDs or labels are retained here.
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub(super) struct WorkflowLoadStats {
    pub source_count: usize,
    pub copies_per_source: usize,
    pub houses: usize,
    pub automations: usize,
    pub threshold_runs_done: u64,
    pub opening_runs_done: u64,
    pub runs_failed: u64,
    pub synthetic_threshold_commands: u64,
    pub synthetic_opening_commands: u64,
    pub local_notices: u64,
    /// Number of run events dropped by the automation observer's broadcast.
    pub runs_observer_lagged_messages: u64,
    /// Number of Hub events dropped by the notice observer's broadcast.
    pub notices_observer_lagged_messages: u64,
    /// Durations use `Run.ended - Run.started` for settled runs (done/failed);
    /// the rolling sample window is capped at 512 entries total and includes
    /// the graph's Wait nodes, not device-command latency.
    pub threshold_duration_samples: usize,
    pub threshold_duration_mean_ms: Option<f64>,
    pub threshold_duration_p95_ms: Option<u64>,
    pub opening_duration_samples: usize,
    pub opening_duration_mean_ms: Option<f64>,
    pub opening_duration_p95_ms: Option<u64>,
}

#[derive(Debug, Default)]
struct Counters {
    threshold_runs_done: AtomicU64,
    opening_runs_done: AtomicU64,
    runs_failed: AtomicU64,
    threshold_commands: AtomicU64,
    opening_commands: AtomicU64,
    local_notices: AtomicU64,
    runs_observer_lagged_messages: AtomicU64,
    notices_observer_lagged_messages: AtomicU64,
    durations: Mutex<VecDeque<(WorkflowKind, u64)>>,
}

impl Counters {
    fn snapshot(&self, source_count: usize, copies_per_source: usize) -> WorkflowLoadStats {
        let threshold_duration = self.duration_summary(WorkflowKind::Threshold);
        let opening_duration = self.duration_summary(WorkflowKind::Opening);
        WorkflowLoadStats {
            source_count,
            copies_per_source,
            houses: source_count * copies_per_source,
            automations: source_count * copies_per_source * 2,
            threshold_runs_done: self.threshold_runs_done.load(Ordering::Relaxed),
            opening_runs_done: self.opening_runs_done.load(Ordering::Relaxed),
            runs_failed: self.runs_failed.load(Ordering::Relaxed),
            synthetic_threshold_commands: self.threshold_commands.load(Ordering::Relaxed),
            synthetic_opening_commands: self.opening_commands.load(Ordering::Relaxed),
            local_notices: self.local_notices.load(Ordering::Relaxed),
            runs_observer_lagged_messages: self
                .runs_observer_lagged_messages
                .load(Ordering::Relaxed),
            notices_observer_lagged_messages: self
                .notices_observer_lagged_messages
                .load(Ordering::Relaxed),
            threshold_duration_samples: threshold_duration.samples,
            threshold_duration_mean_ms: threshold_duration.mean_ms,
            threshold_duration_p95_ms: threshold_duration.p95_ms,
            opening_duration_samples: opening_duration.samples,
            opening_duration_mean_ms: opening_duration.mean_ms,
            opening_duration_p95_ms: opening_duration.p95_ms,
        }
    }

    fn record_duration(&self, kind: WorkflowKind, duration_ms: u64) {
        let mut samples = lock(&self.durations);
        if samples.len() == MAX_DURATION_SAMPLES {
            samples.pop_front();
        }
        samples.push_back((kind, duration_ms));
    }

    fn duration_summary(&self, kind: WorkflowKind) -> DurationSummary {
        let samples = lock(&self.durations);
        let mut durations: Vec<_> = samples
            .iter()
            .filter_map(|(sample_kind, duration)| (*sample_kind == kind).then_some(*duration))
            .collect();
        if durations.is_empty() {
            return DurationSummary::default();
        }
        let sample_count =
            u32::try_from(durations.len()).expect("duration sample count is bounded below 512");
        let mean_ms =
            durations.iter().map(|value| *value as f64).sum::<f64>() / f64::from(sample_count);
        durations.sort_unstable();
        let rank = (durations.len() * 95).div_ceil(100).max(1);
        DurationSummary {
            samples: durations.len(),
            mean_ms: Some(mean_ms),
            p95_ms: durations.get(rank - 1).copied(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WorkflowKind {
    Threshold,
    Opening,
}

#[derive(Clone, Copy, Debug, Default)]
struct DurationSummary {
    samples: usize,
    mean_ms: Option<f64>,
    p95_ms: Option<u64>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Running deterministic automation workload. Call `snapshot` at sample
/// boundaries and `stop` before shutting down the Hub.
pub(super) struct LoadWorkflows {
    source_count: usize,
    copies_per_source: usize,
    counters: Arc<Counters>,
    automation_cancel: CancellationToken,
    driver_cancel: CancellationToken,
    observer_cancel: CancellationToken,
    automations: Option<Automations>,
    automation_task: Option<JoinHandle<()>>,
    driver_task: Option<JoinHandle<()>>,
    observer_tasks: Vec<JoinHandle<()>>,
}

impl LoadWorkflows {
    /// Waits for the read-only House projection, starts a private synthetic
    /// output driver, then installs two approved workflows per synthetic home.
    pub(super) async fn start(
        hub: &Hub,
        copies_per_source: usize,
        source_count: usize,
    ) -> Result<Self> {
        ensure!(
            source_count == EXPECTED_SOURCES,
            "expected exactly two sources"
        );
        ensure!(
            (1..=MAX_COPIES_PER_SOURCE).contains(&copies_per_source),
            "copies per source must be between one and ten"
        );

        let counters = Arc::new(Counters::default());
        let automation_cancel = CancellationToken::new();
        let driver_cancel = CancellationToken::new();
        let observer_cancel = CancellationToken::new();
        let outputs = output_devices(source_count, copies_per_source);
        let output_ids: Vec<DeviceId> = outputs.iter().map(|device| device.id.clone()).collect();
        let driver = Arc::new(OutputDriver {
            devices: outputs,
            counters: Arc::clone(&counters),
        });
        let driver_task = spawn_driver(
            hub,
            InstanceId::from(DRIVER_INSTANCE),
            driver,
            driver_cancel.clone(),
        );

        let mut load = Self {
            source_count,
            copies_per_source,
            counters: Arc::clone(&counters),
            automation_cancel: automation_cancel.clone(),
            driver_cancel,
            observer_cancel: observer_cancel.clone(),
            automations: None,
            automation_task: None,
            driver_task: Some(driver_task),
            observer_tasks: Vec::new(),
        };

        wait_for_devices(hub, &output_ids, INPUT_WAIT)
            .await
            .context("synthetic output driver did not publish its devices")?;

        let inputs = (0..source_count)
            .flat_map(|source| (0..copies_per_source).map(move |copy| (source, copy)))
            .map(|(source, copy)| input_points(source, copy))
            .collect::<Result<Vec<_>>>()?;
        wait_for_inputs(hub, &inputs, INPUT_WAIT).await?;

        let (automations, automation_task) =
            Automations::start(hub, Config::default(), None, automation_cancel)
                .await
                .context("starting the existing automation engine")?;
        load.automations = Some(automations.clone());
        load.automation_task = Some(automation_task);

        let mut live = automations.subscribe();
        let mut kinds = HashMap::new();
        let actor = Actor {
            human: true,
            author: Author::Human,
        };
        for input in &inputs {
            let output = output_device_id(input.source_index, input.copy);
            let threshold = threshold_workflow(input, &output);
            let saved = automations
                .save(threshold, actor)
                .await
                .context("saving synthetic threshold workflow")?;
            let fingerprint = saved.fingerprint();
            automations
                .approve(&saved.id, &fingerprint, actor)
                .await
                .context("activating synthetic threshold workflow")?;
            kinds.insert(saved.id, WorkflowKind::Threshold);

            let opening = opening_workflow(input, &output);
            let saved = automations
                .save(opening, actor)
                .await
                .context("saving synthetic opening workflow")?;
            let fingerprint = saved.fingerprint();
            automations
                .approve(&saved.id, &fingerprint, actor)
                .await
                .context("activating synthetic opening workflow")?;
            kinds.insert(saved.id, WorkflowKind::Opening);
        }
        let kinds = Arc::new(kinds);
        let live_counters = Arc::clone(&counters);
        let live_cancel = observer_cancel.clone();
        load.observer_tasks.push(tokio::spawn(async move {
            observe_runs(&mut live, kinds, live_counters, live_cancel).await;
        }));

        let mut events = hub.subscribe();
        let notice_counters = Arc::clone(&counters);
        let notice_cancel = observer_cancel;
        load.observer_tasks.push(tokio::spawn(async move {
            observe_notices(&mut events, notice_counters, notice_cancel).await;
        }));
        Ok(load)
    }

    pub(super) fn snapshot(&self) -> WorkflowLoadStats {
        self.counters
            .snapshot(self.source_count, self.copies_per_source)
    }

    pub(super) async fn stop(mut self) {
        self.observer_cancel.cancel();
        self.automation_cancel.cancel();
        self.driver_cancel.cancel();
        for task in self.observer_tasks.drain(..) {
            let _ = task.await;
        }
        if let Some(task) = self.automation_task.take() {
            let _ = task.await;
        }
        if let Some(task) = self.driver_task.take() {
            let _ = task.await;
        }
        self.automations.take();
    }
}

impl Drop for LoadWorkflows {
    fn drop(&mut self) {
        self.observer_cancel.cancel();
        self.automation_cancel.cancel();
        self.driver_cancel.cancel();
    }
}

fn output_device_id(source_index: usize, copy: usize) -> DeviceId {
    DeviceId::new(
        &InstanceId::from(DRIVER_INSTANCE),
        &format!("house-{source_index}-{copy:03}"),
    )
}

fn output_devices(source_count: usize, copies_per_source: usize) -> Vec<Device> {
    (0..source_count)
        .flat_map(|source| (0..copies_per_source).map(move |copy| (source, copy)))
        .map(|(source, copy)| {
            let instance = InstanceId::from(DRIVER_INSTANCE);
            let id = output_device_id(source, copy);
            let point = |key: &str, label: &str| PointSpec {
                key: key.into(),
                label: label.into(),
                kind: Kind::Binary,
                access: Access {
                    read: true,
                    write: true,
                },
                unit: None,
                semantic: Semantic::Other,
            };
            Device {
                id,
                instance,
                native_name: format!("Synthetic output {source}-{copy:03}").into(),
                manufacturer: Some("Moli load harness".into()),
                model: Some("synthetic".into()),
                description: None,
                native_room: None,
                members: Vec::new(),
                points: vec![
                    point(THRESHOLD_OUTPUT, "Synthetic threshold output"),
                    point(OPENING_OUTPUT, "Synthetic opening output"),
                ],
            }
        })
        .collect()
}

#[derive(Debug)]
struct OutputDriver {
    devices: Vec<Device>,
    counters: Arc<Counters>,
}

impl Driver for OutputDriver {
    fn kind(&self) -> &'static str {
        "load-workflows"
    }

    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            for device in &self.devices {
                ctx.upsert_device(device.clone());
                for point in &device.points {
                    ctx.set_state(&device.id, &point.key, Value::Bool(false));
                }
            }
            ctx.ready();
            while let Some(command) = ctx.next_command().await {
                if command.abandoned() {
                    command.reply(Err("expired".into()));
                    continue;
                }
                match command.key.as_ref() {
                    THRESHOLD_OUTPUT => {
                        self.counters
                            .threshold_commands
                            .fetch_add(1, Ordering::Relaxed);
                    }
                    OPENING_OUTPUT => {
                        self.counters
                            .opening_commands
                            .fetch_add(1, Ordering::Relaxed);
                    }
                    _ => {}
                }
                ctx.set_state(&command.device.id, &command.key, command.value.clone());
                command.reply(Ok(()));
            }
            for device in &self.devices {
                ctx.remove_device(&device.id);
            }
            Ok(())
        })
    }
}

async fn wait_for_devices(hub: &Hub, ids: &[DeviceId], timeout: Duration) -> Result<()> {
    tokio::time::timeout(timeout, async {
        loop {
            if ids.iter().all(|id| hub.describe(id).is_some()) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .context("device publication timed out")
}

async fn wait_for_inputs(hub: &Hub, inputs: &[InputPoints], timeout: Duration) -> Result<()> {
    tokio::time::timeout(timeout, async {
        loop {
            if inputs.iter().all(|input| {
                is_point(hub, &input.temperature, |spec| {
                    matches!(&spec.kind, Kind::Numeric { .. }) && spec.unit == Some(Unit::Celsius)
                }) && is_point(hub, &input.opening, |spec| {
                    matches!(&spec.kind, Kind::Binary)
                }) && is_point(hub, &input.mode, |spec| matches!(&spec.kind, Kind::Text))
            }) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .context("waiting for the MQTT-projected synthetic inputs timed out")
}

fn is_point(hub: &Hub, point: &PointId, valid: impl FnOnce(&PointSpec) -> bool) -> bool {
    point
        .split()
        .and_then(|(device, key)| {
            hub.describe(&device)
                .map(|(device, _)| (device, key.to_owned()))
        })
        .and_then(|(device, key)| device.point(&key).cloned())
        .is_some_and(|spec| spec.access.read && valid(&spec))
}

fn threshold_workflow(input: &InputPoints, output: &DeviceId) -> Automation {
    let temperature = input.temperature.to_string();
    let mode = input.mode.to_string();
    let output = PointId::new(output, THRESHOLD_OUTPUT).to_string();
    Automation {
        id: String::new(),
        name: format!(
            "Synthetic threshold {}-{:03}",
            input.source_index, input.copy
        ),
        enabled: false,
        mode: Mode::Single,
        graph: Graph {
            nodes: vec![
                node(
                    "temperature-threshold",
                    Step::WhenThreshold {
                        point: temperature,
                        above: Some(TEMP_ABOVE_C),
                        below: None,
                        for_s: THRESHOLD_HOLD_S,
                    },
                ),
                node(
                    "mode-condition",
                    Step::If {
                        rules: vec![Rule::State {
                            point: mode,
                            op: Op::Eq,
                            value: json!("comfort"),
                        }],
                        all: true,
                    },
                ),
                node(
                    "delay",
                    Step::Wait {
                        seconds: THRESHOLD_DELAY_S,
                    },
                ),
                node(
                    "synthetic-output",
                    Step::Set {
                        point: output,
                        value: json!(true),
                    },
                ),
            ],
            edges: vec![
                edge("temperature-threshold", "out", "mode-condition"),
                edge("mode-condition", "yes", "delay"),
                edge("delay", "out", "synthetic-output"),
            ],
        },
        author: Author::Human,
        approved: None,
        approved_version: None,
        activation: None,
        note: Some("Synthetic load workflow; no physical target.".into()),
        created: 0,
        updated: 0,
    }
}

fn opening_workflow(input: &InputPoints, output: &DeviceId) -> Automation {
    let opening = input.opening.to_string();
    let output = PointId::new(output, OPENING_OUTPUT).to_string();
    Automation {
        id: String::new(),
        name: format!("Synthetic opening {}-{:03}", input.source_index, input.copy),
        enabled: false,
        mode: Mode::Single,
        graph: Graph {
            nodes: vec![
                node(
                    "opening-detected",
                    Step::WhenState {
                        point: opening,
                        to: Some(json!(true)),
                        from: Some(json!(false)),
                        for_s: 0,
                    },
                ),
                node(
                    "local-notice",
                    Step::Notify {
                        title: Some("Synthetic opening".into()),
                        message: "Opening detected in a synthetic workflow.".into(),
                        channels: vec![Channel::Maison],
                    },
                ),
                node(
                    "synthetic-output",
                    Step::Set {
                        point: output,
                        value: json!(true),
                    },
                ),
            ],
            edges: vec![
                edge("opening-detected", "out", "local-notice"),
                edge("local-notice", "out", "synthetic-output"),
            ],
        },
        author: Author::Human,
        approved: None,
        approved_version: None,
        activation: None,
        note: Some("Synthetic load workflow; local notice only.".into()),
        created: 0,
        updated: 0,
    }
}

fn node(id: &str, step: Step) -> Node {
    Node {
        id: id.into(),
        step,
        x: 0.0,
        y: 0.0,
    }
}

fn edge(from: &str, port: &str, to: &str) -> Edge {
    Edge {
        from: from.into(),
        port: port.into(),
        to: to.into(),
    }
}

async fn observe_runs(
    receiver: &mut tokio::sync::broadcast::Receiver<moli_automation::Live>,
    kinds: Arc<HashMap<String, WorkflowKind>>,
    counters: Arc<Counters>,
    cancel: CancellationToken,
) {
    loop {
        tokio::select! {
            () = cancel.cancelled() => break,
            event = receiver.recv() => match event {
                Ok(moli_automation::Live::RunEnded { run }) => {
                    let duration_ms = run.ended.and_then(|ended| ended.checked_sub(run.started));
                    if run.status == moli_automation::RunStatus::Failed {
                        counters.runs_failed.fetch_add(1, Ordering::Relaxed);
                        if let (Some(kind), Some(duration_ms)) =
                            (kinds.get(&run.automation), duration_ms)
                        {
                            counters.record_duration(*kind, duration_ms);
                        }
                    } else if run.status == moli_automation::RunStatus::Done {
                        match kinds.get(&run.automation) {
                            Some(kind @ WorkflowKind::Threshold) => {
                                counters.threshold_runs_done.fetch_add(1, Ordering::Relaxed);
                                if let Some(duration_ms) = duration_ms {
                                    counters.record_duration(*kind, duration_ms);
                                }
                            }
                            Some(kind @ WorkflowKind::Opening) => {
                                counters.opening_runs_done.fetch_add(1, Ordering::Relaxed);
                                if let Some(duration_ms) = duration_ms {
                                    counters.record_duration(*kind, duration_ms);
                                }
                            }
                            None => {}
                        }
                    }
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    counters
                        .runs_observer_lagged_messages
                        .fetch_add(missed, Ordering::Relaxed);
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    }
}

async fn observe_notices(
    receiver: &mut tokio::sync::broadcast::Receiver<Event>,
    counters: Arc<Counters>,
    cancel: CancellationToken,
) {
    loop {
        tokio::select! {
            () = cancel.cancelled() => break,
            event = receiver.recv() => match event {
                Ok(Event::Notice { notice }) if notice.from.as_deref().is_some_and(|name| name.starts_with("Synthetic opening ")) => {
                    counters.local_notices.fetch_add(1, Ordering::Relaxed);
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    counters
                        .notices_observer_lagged_messages
                        .fetch_add(missed, Ordering::Relaxed);
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moli_core::EnumCodes;
    use moli_runtime::HubOptions;

    #[test]
    fn duration_window_is_bounded_and_p95_uses_nearest_rank() {
        let counters = Counters::default();
        for duration in 1..=20 {
            counters.record_duration(WorkflowKind::Threshold, duration);
        }
        let summary = counters.duration_summary(WorkflowKind::Threshold);
        assert_eq!(summary.samples, 20);
        assert_eq!(summary.mean_ms, Some(10.5));
        assert_eq!(summary.p95_ms, Some(19));
        for _ in 0..MAX_DURATION_SAMPLES {
            counters.record_duration(WorkflowKind::Opening, 7);
        }
        assert_eq!(lock(&counters.durations).len(), MAX_DURATION_SAMPLES);
        assert_eq!(
            counters.duration_summary(WorkflowKind::Threshold).samples,
            0
        );
        assert_eq!(
            counters.duration_summary(WorkflowKind::Opening).p95_ms,
            Some(7)
        );
    }

    #[test]
    fn input_points_are_stable_per_bridge_copy_and_distinct_between_copies() {
        let first = input_points(0, 3).unwrap();
        let repeated = input_points(0, 3).unwrap();
        let other_source = input_points(1, 3).unwrap();
        let other_copy = input_points(0, 4).unwrap();
        assert_eq!(first.temperature, repeated.temperature);
        assert_eq!(first.opening, repeated.opening);
        assert_eq!(first.mode, repeated.mode);
        assert_ne!(first.temperature, other_source.temperature);
        assert_ne!(first.temperature, other_copy.temperature);
        assert_eq!(first.instance.as_str(), "load-0-3");
        assert_eq!(first.temperature_entity_id, "sensor.load_0_003_temperature");
    }

    #[derive(Debug)]
    struct EnumDriver {
        device: Device,
        mode_key: String,
    }

    impl Driver for EnumDriver {
        fn kind(&self) -> &'static str {
            "load-enum-condition-test"
        }

        fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
            Box::pin(async move {
                ctx.upsert_device(self.device.clone());
                ctx.set_state(&self.device.id, &self.mode_key, Value::EnumCode(1));
                ctx.ready();
                ctx.cancelled().await;
                ctx.remove_device(&self.device.id);
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn enum_code_does_not_match_a_text_rule_in_the_current_automation_engine() {
        let hub = Hub::new(HubOptions::default()).unwrap();
        let instance = InstanceId::from("enum-workflow-test");
        let device_id = DeviceId::new(&instance, "mode");
        let mode_key = "mode";
        let mode_point = PointId::new(&device_id, mode_key);
        let mut codes = EnumCodes::empty();
        codes.insert(0);
        codes.insert(1);
        let device = Device {
            id: device_id.clone(),
            instance: instance.clone(),
            native_name: "Synthetic enum input".into(),
            manufacturer: None,
            model: None,
            description: None,
            native_room: None,
            members: Vec::new(),
            points: vec![PointSpec {
                key: mode_key.into(),
                label: "Synthetic mode".into(),
                kind: Kind::EnumCode {
                    catalogue: 1,
                    version: 1,
                    codes,
                },
                access: Access {
                    read: true,
                    write: false,
                },
                unit: None,
                semantic: Semantic::Other,
            }],
        };
        let driver_cancel = CancellationToken::new();
        let driver_task = spawn_driver(
            &hub,
            instance,
            Arc::new(EnumDriver {
                device,
                mode_key: mode_key.into(),
            }),
            driver_cancel.clone(),
        );
        wait_for_devices(&hub, &[device_id], Duration::from_secs(2))
            .await
            .unwrap();

        let automation_cancel = CancellationToken::new();
        let (automations, engine_task) =
            Automations::start(&hub, Config::default(), None, automation_cancel.clone())
                .await
                .unwrap();
        let automation = Automation {
            id: String::new(),
            name: "Synthetic enum rule test".into(),
            enabled: false,
            mode: Mode::Single,
            graph: Graph {
                nodes: vec![
                    node("manual", Step::Manual),
                    node(
                        "condition",
                        Step::If {
                            rules: vec![Rule::State {
                                point: mode_point.to_string(),
                                op: Op::Eq,
                                value: json!("comfort"),
                            }],
                            all: true,
                        },
                    ),
                ],
                edges: vec![edge("manual", "out", "condition")],
            },
            author: Author::Agent,
            approved: None,
            approved_version: None,
            activation: None,
            note: None,
            created: 0,
            updated: 0,
        };
        let actor = Actor {
            human: false,
            author: Author::Agent,
        };
        let saved = automations.save(automation, actor).await.unwrap();
        let run = automations
            .run_now(&saved.id, true, None, actor)
            .await
            .unwrap();
        assert_eq!(hub.state(&mode_point).unwrap().value, Value::EnumCode(1));
        assert_eq!(
            run.steps
                .iter()
                .find(|step| step.node == "condition")
                .unwrap()
                .status,
            moli_automation::StepStatus::No
        );

        automation_cancel.cancel();
        driver_cancel.cancel();
        let _ = engine_task.await;
        let _ = driver_task.await;
    }
}
