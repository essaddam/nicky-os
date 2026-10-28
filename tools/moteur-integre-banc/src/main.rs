//! Real runtime and automation crates; only the transport/data are synthetic.
mod memory;
mod replay;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use moli_automation::{Actor, Author, Automations, Config, RunStatus};
use moli_core::{
    Access, Action, AggregateOp, AggregateSpec, DependencyKind, DependencyRef, Device, DeviceId,
    Event, Kind, Origin, PointId, PointSpec, Semantic, Unit, Value,
};
use moli_runtime::{BoxFuture, CommandError, Driver, DriverCtx, Hub, HubOptions, spawn_driver};
use serde::Serialize;
use tokio::sync::{Mutex, broadcast, mpsc, oneshot};
use tokio_util::sync::CancellationToken;

#[global_allocator]
#[cfg(not(feature = "plain-timing"))]
static ALLOCATOR: memory::Meter = memory::Meter;

struct Batch {
    values: Vec<(DeviceId, &'static str, Value)>,
    done: oneshot::Sender<()>,
}

struct Synthetic {
    devices: usize,
    incoming: Mutex<mpsc::Receiver<Batch>>,
    effects: Arc<AtomicUsize>,
}

fn id(index: usize) -> DeviceId {
    format!("synthetic:device-{index:06}").into()
}
fn point(index: usize, key: &str) -> PointId {
    format!("{}/{key}", id(index)).into()
}
fn calculated(name: &str) -> DependencyRef {
    DependencyRef {
        kind: DependencyKind::Computed,
        id: format!("synthetic:{name}").into(),
    }
}
fn numeric() -> Kind {
    Kind::Numeric {
        min: None,
        max: None,
        step: None,
    }
}
fn spec(key: &str, kind: Kind, read: bool, write: bool) -> PointSpec {
    PointSpec {
        key: key.into(),
        label: key.into(),
        kind,
        access: Access { read, write },
        unit: (key == "value").then_some(Unit::Watt),
        semantic: Semantic::Other,
    }
}
fn device(device_id: DeviceId, points: Vec<PointSpec>, protected: bool) -> Device {
    Device {
        id: device_id,
        instance: "synthetic".into(),
        native_name: "Synthetic fixture".into(),
        manufacturer: None,
        model: None,
        description: None,
        native_room: Some(
            if protected {
                "Synthetic protected"
            } else {
                "Synthetic neutral"
            }
            .into(),
        ),
        members: Vec::new(),
        points,
    }
}
fn aggregate(sources: Vec<PointId>, operation: AggregateOp, max_age_s: u64) -> AggregateSpec {
    AggregateSpec {
        operation,
        sources,
        unit: Some(Unit::Watt),
        max_age_s,
    }
}

impl Driver for Synthetic {
    fn kind(&self) -> &'static str {
        "synthetic-integrated-benchmark"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            for index in 0..self.devices {
                let device_id = id(index);
                ctx.upsert_device(device(
                    device_id.clone(),
                    vec![
                        spec("value", numeric(), true, false),
                        spec("state", Kind::Binary, true, true),
                        spec(
                            "action",
                            Kind::Enum {
                                values: vec!["fire".into(), "error".into(), "slow".into()],
                            },
                            false,
                            true,
                        ),
                        spec("pulse", Kind::Text, true, true),
                    ],
                    index == 3,
                ));
                ctx.set_state(&device_id, "value", Value::Int(i64::try_from(index)?));
                ctx.set_state(&device_id, "state", Value::Bool(false));
                ctx.set_availability(&device_id, true);
            }
            for (name, definition) in [
                (
                    "mean",
                    aggregate(
                        vec![point(0, "value"), point(1, "value")],
                        AggregateOp::Mean,
                        604_800,
                    ),
                ),
                (
                    "chain",
                    aggregate(
                        vec!["synthetic:mean/value".into(), point(2, "value")],
                        AggregateOp::Max,
                        604_800,
                    ),
                ),
                (
                    "fresh",
                    aggregate(vec![point(0, "value")], AggregateOp::Mean, 1),
                ),
            ] {
                let device_id = calculated(name).id;
                ctx.upsert_device(device(
                    device_id.clone(),
                    vec![spec("value", Kind::Computed, true, false)],
                    false,
                ));
                ctx.set_availability(&device_id, true);
                ctx.initialize_computed(&device_id, definition).await?;
            }
            ctx.ready();
            let mut incoming = self.incoming.lock().await;
            loop {
                tokio::select! {
                    Some(batch) = incoming.recv() => {
                        for (device_id, key, value) in batch.values { ctx.set_state(&device_id, key, value); }
                        let _ = batch.done.send(());
                    }
                    command = ctx.next_command() => {
                        let Some(command) = command else { return Ok(()); };
                        if command.value == Value::from("error") {
                            command.reply(Err("synthetic transport rejection".into()));
                            continue;
                        }
                        if command.value == Value::from("slow") {
                            tokio::time::sleep(Duration::from_millis(80)).await;
                        }
                        let permit = command.send_permit(command.deadline);
                        if !permit.begin_send() { command.reply_result(Err(CommandError::AccessDenied)); continue; }
                        match command.key.as_ref() {
                            "action" => { self.effects.fetch_add(1, Ordering::Relaxed); }
                            "pulse" => ctx.pulse_state(&command.device.id, "pulse", command.value.clone()),
                            _ => ctx.set_state(&command.device.id, &command.key, command.value.clone()),
                        }
                        command.reply(Ok(()));
                    }
                }
            }
        })
    }
}

async fn wait_for(mut predicate: impl FnMut() -> bool) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .context("integrated-engine condition timed out")
}
async fn observe(
    tx: &mpsc::Sender<Batch>,
    values: Vec<(DeviceId, &'static str, Value)>,
) -> Result<()> {
    let (done, finished) = oneshot::channel();
    tx.send(Batch { values, done })
        .await
        .map_err(|_| anyhow::anyhow!("driver stopped"))?;
    finished.await?;
    Ok(())
}
fn value(hub: &Hub, name: &str) -> Option<Value> {
    hub.state(
        &calculated(name)
            .point()
            .expect("computed reference has point"),
    )
    .map(|s| s.value)
}
fn drain(events: &mut broadcast::Receiver<Event>) -> Result<Vec<Event>> {
    let mut result = Vec::new();
    loop {
        match events.try_recv() {
            Ok(event) => result.push(event),
            Err(broadcast::error::TryRecvError::Empty) => return Ok(result),
            Err(error) => anyhow::bail!("benchmark subscriber lost events: {error}"),
        }
    }
}

#[derive(Serialize)]
struct Report {
    schema: u8,
    engine: &'static str,
    measurement_mode: &'static str,
    replay_layout: String,
    adapter_payload_and_handles_bytes: usize,
    adapter_live_requested_bytes: usize,
    devices: usize,
    points: usize,
    rounds: usize,
    raw_observations: usize,
    elapsed_ns: u128,
    raw_observations_per_second: f64,
    baseline_live_requested_bytes: usize,
    populated_live_requested_bytes: usize,
    hub_setup_net_requested_bytes: usize,
    final_live_requested_bytes: usize,
    workload_peak_requested_bytes: usize,
    workload_allocated_bytes: usize,
    rss_bytes: Option<u64>,
    loaded_rss_bytes: Option<u64>,
    loaded_anonymous_bytes: Option<u64>,
    steady_anonymous_bytes: Option<u64>,
    process_peak_rss_bytes: Option<u64>,
    sampled_peak_anonymous_bytes: Option<u64>,
    batch_ack_latency_ns: Latency,
    propagation_and_automation_latency_ns: Latency,
    setter_round_trip_latency_ns: Latency,
    action_round_trip_latency_ns: Latency,
    bus_state_events: usize,
    bus_occurrences: usize,
    automation_runs: usize,
    final_value_checksum: i64,
    assertions: Vec<&'static str>,
}

#[derive(Serialize)]
struct Latency {
    samples: usize,
    p50: u128,
    p95: u128,
    max: u128,
}
fn latency(mut values: Vec<u128>) -> Latency {
    values.sort_unstable();
    Latency {
        samples: values.len(),
        p50: values[(values.len() - 1) / 2],
        p95: values[(values.len() * 95).div_ceil(100) - 1],
        max: *values.last().unwrap(),
    }
}
fn anonymous_bytes() -> Option<u64> {
    status_bytes("RssAnon:")
}
fn status_bytes(key: &str) -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with(key))?;
    Some(line.split_whitespace().nth(1)?.parse::<u64>().ok()? * 1024)
}

async fn run(devices: usize, rounds: usize, layout: &str) -> Result<Report> {
    ensure!(
        (4..=100_000).contains(&devices),
        "--devices must be 4..100000"
    );
    ensure!(
        (1..=20).contains(&rounds),
        "--rounds must be 1..20 (production runaway guard)"
    );
    let before_adapter = memory::live();
    let mut replay = replay::Replay::new(layout, devices)?;
    let adapter_live = memory::live() - before_adapter;
    let baseline = memory::live();
    let hub = Hub::new(HubOptions {
        event_capacity: devices * 4 + 2048,
        journal_capacity: 128,
        command_timeout: Duration::from_millis(40),
        guard: moli_runtime::guard::GuardPolicy {
            protected_rooms: vec!["Synthetic protected".into()],
            ..Default::default()
        },
        ..HubOptions::default()
    })?;
    let cancel = CancellationToken::new();
    let (tx, incoming) = mpsc::channel(4);
    let effects = Arc::new(AtomicUsize::new(0));
    let driver = spawn_driver(
        &hub,
        "synthetic".into(),
        Arc::new(Synthetic {
            devices,
            incoming: Mutex::new(incoming),
            effects: effects.clone(),
        }),
        cancel.clone(),
    );
    wait_for(|| hub.stats().drivers_running == 1).await?;
    ensure!(
        value(&hub, "mean") == Some(Value::Null),
        "draft calculation must stay inert"
    );
    for name in ["mean", "chain", "fresh"] {
        let preview = hub.preview_dependency(&calculated(name)).await?;
        ensure!(
            hub.publish_dependency(preview.clone(), false, Origin::Mcp, None)
                .await
                .is_err(),
            "agent must not publish"
        );
        hub.publish_dependency(preview, true, Origin::Ui, None)
            .await?;
    }
    wait_for(|| value(&hub, "chain") == Some(Value::Float(2.0))).await?;
    let (automations, automation_task) =
        Automations::start(&hub, Config::default(), None, cancel.clone()).await?;
    let agent = Actor {
        human: false,
        author: Author::Agent,
    };
    let human = Actor {
        human: true,
        author: Author::Human,
    };
    let draft = automations
        .save(
            serde_json::from_value(serde_json::json!({
                "id":"integrated", "name":"Synthetic state to setter", "mode":"queued",
                "graph": { "nodes": [
                    {"id":"trigger", "type":"when_state", "point":point(0,"state").as_str()},
                    {"id":"set", "type":"toggle", "point":point(1,"state").as_str()}
                ], "edges":[{"from":"trigger","to":"set"}] }
            }))?,
            agent,
        )
        .await?;
    ensure!(!draft.is_live(), "saved graph must remain a draft");
    ensure!(
        automations
            .approve(&draft.id, &draft.fingerprint(), agent)
            .await
            .is_err(),
        "agent must not approve graph"
    );
    automations
        .approve(&draft.id, &draft.fingerprint(), human)
        .await?;
    tokio::task::yield_now().await;

    ensure!(
        hub.command(&point(0, "value"), Value::Int(7), Origin::Ui, None)
            .await
            == Err(CommandError::ReadOnly)
    );
    ensure!(matches!(
        hub.command(&point(0, "state"), Value::from("wrong"), Origin::Ui, None)
            .await,
        Err(CommandError::Invalid(_))
    ));
    for key in ["state", "action"] {
        let desired = if key == "state" {
            Value::Bool(true)
        } else {
            Value::from("fire")
        };
        let held = hub
            .command(&point(3, key), desired, Origin::Mcp, None)
            .await;
        let Err(CommandError::NeedsApproval { id, .. }) = held else {
            anyhow::bail!("protected {key} was not held");
        };
        ensure!(
            effects.load(Ordering::Relaxed) == 0,
            "held action reached transport"
        );
        if key == "state" {
            ensure!(hub.state(&point(3, key)).unwrap().value == Value::Bool(false));
        }
        hub.resolve_approval(id, &point(3, key), true, Origin::Ui, None)
            .await?;
    }
    ensure!(effects.load(Ordering::Relaxed) == 1);
    ensure!(
        hub.state(&point(3, "action")).is_none(),
        "standalone action must not create state"
    );
    ensure!(
        hub.journal(128)
            .iter()
            .any(|entry| matches!(entry.action, Action::Command { .. })),
        "commands must be audited"
    );
    ensure!(
        matches!(
            hub.command(&point(2, "action"), Value::from("error"), Origin::Ui, None)
                .await,
            Err(CommandError::Driver(_))
        ),
        "driver rejection must propagate"
    );
    ensure!(
        hub.command(&point(2, "action"), Value::from("slow"), Origin::Ui, None)
            .await
            == Err(CommandError::Timeout),
        "delayed driver must time out"
    );
    // A barrier after the delayed transport checks that expiration prevents a late effect.
    observe(&tx, Vec::new()).await?;
    ensure!(
        effects.load(Ordering::Relaxed) == 1,
        "expired command executed late"
    );

    // Real monotonic freshness scheduler: no synthetic clock or copied evaluator.
    wait_for(|| value(&hub, "fresh") == Some(Value::Null)).await?;
    let mut events = hub.subscribe();
    observe(&tx, vec![(id(0), "value", Value::Int(0))]).await?;
    wait_for(|| value(&hub, "fresh") == Some(Value::Float(0.0))).await?;
    ensure!(
        !drain(&mut events)?
            .iter()
            .any(|event| matches!(event, Event::State {point: p,..} if *p == point(0,"value"))),
        "equal observation fabricated raw state event"
    );
    for _ in 0..2 {
        hub.command(&point(2, "pulse"), Value::from("tap"), Origin::Ui, None)
            .await?;
    }
    let pulses = drain(&mut events)?;
    let occurrence_ids: Vec<_> = pulses
        .iter()
        .filter_map(|event| match event {
            Event::Occurrence { occurrence } => Some(&occurrence.id),
            _ => None,
        })
        .collect();
    ensure!(
        occurrence_ids.len() == 2 && occurrence_ids[0] != occurrence_ids[1],
        "equal pulses require distinct occurrence identities"
    );
    let populated = memory::live();
    let loaded_rss = hub.stats().rss_bytes;
    let loaded_anonymous = anonymous_bytes();
    let mut sampled_peak_anonymous = loaded_anonymous;
    memory::reset_peak();
    let allocation_start = memory::total();
    let mut batch_latencies = Vec::with_capacity(rounds);
    let mut propagation_latencies = Vec::with_capacity(rounds);
    let mut setter_latencies = Vec::with_capacity(rounds);
    let mut action_latencies = Vec::with_capacity(rounds);
    let started = Instant::now();
    let mut state_events = 0;
    for round in 0..rounds {
        let mut signals = hub.subscribe();
        let mut live = automations.subscribe();
        let round_started = Instant::now();
        let observations = replay.batch(round)?;
        observe(&tx, observations).await?;
        batch_latencies.push(round_started.elapsed().as_nanos());
        sampled_peak_anonymous = sampled_peak_anonymous.max(anonymous_bytes());
        let expected_chain = Value::Float(f64::from(u32::try_from(round + 1)?) * 1_000_000.0 + 2.0);
        let settled = tokio::time::timeout(Duration::from_secs(10), async {
            while !(value(&hub, "chain") == Some(expected_chain.clone())
                && automations
                    .runs(Some(&draft.id), 1)
                    .first()
                    .is_some_and(|run| {
                        usize::try_from(run.id).ok() == Some(round + 1)
                            && run.status == RunStatus::Done
                    }))
            {
                tokio::select! {
                    signal = signals.recv() => { signal.context("propagation signal lost")?; }
                    signal = live.recv() => { signal.context("automation signal lost")?; }
                }
            }
            Ok::<_, anyhow::Error>(())
        })
        .await;
        if settled.is_err() {
            eprintln!(
                "{}",
                serde_json::json!({
                    "diagnostic":"round_timeout", "round":round,
                    "mean":value(&hub,"mean"), "chain":value(&hub,"chain"), "fresh":value(&hub,"fresh"),
                    "trigger_state":hub.state(&point(0,"state")), "target_state":hub.state(&point(1,"state")),
                    "automations":automations.list(), "all_runs":automations.runs(None,10),
                    "expected_automation_id":draft.id,
                    "filtered_runs":automations.runs(Some(&draft.id),10)
                })
            );
        }
        settled.context("round did not settle")??;
        propagation_latencies.push(round_started.elapsed().as_nanos());
        let base = f64::from(u32::try_from(round + 1)?) * 1_000_000.0;
        let mean = value(&hub, "mean").and_then(|value| value.as_f64());
        ensure!(
            mean.is_some_and(|mean| (mean - (base + 0.5)).abs() <= 4.0 * f64::EPSILON * base),
            "mean projection mismatch"
        );
        ensure!(
            value(&hub, "fresh") == Some(Value::Float(base)),
            "fresh projection mismatch"
        );
        ensure!(
            hub.state(&point(1, "state")).unwrap().value == Value::Bool(round.is_multiple_of(2)),
            "automation setter mismatch"
        );
        let setter_started = Instant::now();
        hub.command(
            &point(2, "state"),
            Value::Bool(round.is_multiple_of(2)),
            Origin::Api,
            None,
        )
        .await?;
        setter_latencies.push(setter_started.elapsed().as_nanos());
        let action_started = Instant::now();
        hub.command(&point(2, "action"), Value::from("fire"), Origin::Api, None)
            .await?;
        action_latencies.push(action_started.elapsed().as_nanos());
        ensure!(
            effects.load(Ordering::Relaxed) == round + 2,
            "standalone action effect count mismatch"
        );
        ensure!(
            hub.state(&point(2, "action")).is_none(),
            "standalone action must not create state"
        );
        let round_events = drain(&mut events)?;
        let raw_count = round_events.iter().filter(|event| matches!(event, Event::State {point: p,..} if p.as_str().starts_with("synthetic:device-") && p.as_str().ends_with("/value"))).count();
        ensure!(
            raw_count == devices,
            "expected {devices} raw changes, got {raw_count}"
        );
        state_events += round_events
            .iter()
            .filter(|event| matches!(event, Event::State { .. }))
            .count();
    }
    let elapsed = started.elapsed();
    let final_live = memory::live();
    let peak = memory::peak();
    let allocated = memory::total() - allocation_start;
    let mut checksum = 0_i64;
    let final_base = i64::try_from(rounds)? * 1_000_000;
    for index in 0..devices {
        let expected = final_base + i64::try_from(index)?;
        ensure!(hub.state(&point(index, "value")).unwrap().value == Value::Int(expected));
        checksum = checksum
            .checked_add(expected)
            .context("checksum overflow")?;
    }
    let stats = hub.stats();
    let steady_anonymous = anonymous_bytes();
    sampled_peak_anonymous = sampled_peak_anonymous.max(steady_anonymous);
    let process_peak_rss = status_bytes("VmHWM:");
    ensure!(stats.devices == devices + 3 && stats.points == devices * 4 + 3);
    ensure!(effects.load(Ordering::Relaxed) == rounds + 1);
    cancel.cancel();
    driver.await?;
    automation_task.await?;
    Ok(Report {
        schema: 1,
        engine: "production-moli-runtime+production-moli-automation/synthetic-transport",
        measurement_mode: if cfg!(feature = "plain-timing") {
            "timing_plain"
        } else {
            "counted"
        },
        replay_layout: layout.into(),
        adapter_payload_and_handles_bytes: replay.bytes(),
        adapter_live_requested_bytes: adapter_live,
        devices: stats.devices,
        points: stats.points,
        rounds,
        raw_observations: devices * rounds,
        elapsed_ns: elapsed.as_nanos(),
        raw_observations_per_second: f64::from(u32::try_from(devices * rounds)?)
            / elapsed.as_secs_f64(),
        baseline_live_requested_bytes: baseline,
        populated_live_requested_bytes: populated,
        hub_setup_net_requested_bytes: populated.saturating_sub(baseline),
        final_live_requested_bytes: final_live,
        workload_peak_requested_bytes: peak,
        workload_allocated_bytes: allocated,
        rss_bytes: stats.rss_bytes,
        bus_state_events: state_events,
        bus_occurrences: occurrence_ids.len(),
        loaded_rss_bytes: loaded_rss,
        loaded_anonymous_bytes: loaded_anonymous,
        steady_anonymous_bytes: steady_anonymous,
        process_peak_rss_bytes: process_peak_rss,
        sampled_peak_anonymous_bytes: sampled_peak_anonymous,
        batch_ack_latency_ns: latency(batch_latencies),
        propagation_and_automation_latency_ns: latency(propagation_latencies),
        setter_round_trip_latency_ns: latency(setter_latencies),
        action_round_trip_latency_ns: latency(action_latencies),
        automation_runs: rounds,
        final_value_checksum: checksum,
        assertions: vec![
            "drafts_inert",
            "human_publication_required",
            "real_computed_chain",
            "readonly_and_type_validation",
            "guard_holds_setter_and_action",
            "approval_executes",
            "standalone_action_has_no_state",
            "commands_audited",
            "driver_error_propagates",
            "driver_timeout_without_late_effect",
            "monotonic_expiry_without_input",
            "equal_observation_refresh_without_raw_event",
            "distinct_equal_occurrences",
            "approved_state_trigger_to_setter",
            "lossless_bus",
            "all_final_values_match_oracle",
        ],
    })
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let mut devices = 1000;
    let mut rounds = 10;
    let mut layout = "uniform96".to_owned();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--devices" => devices = args.next().context("missing --devices value")?.parse()?,
            "--rounds" => rounds = args.next().context("missing --rounds value")?.parse()?,
            "--layout" => layout = args.next().context("missing --layout value")?,
            "--help" => {
                println!(
                    "moteur-integre-banc [--devices 4..100000] [--rounds 1..20] [--layout uniform96|compact64|columns]"
                );
                return Ok(());
            }
            _ => anyhow::bail!("unknown argument: {arg}"),
        }
    }
    println!(
        "{}",
        serde_json::to_string(&run(devices, rounds, &layout).await?)?
    );
    Ok(())
}
