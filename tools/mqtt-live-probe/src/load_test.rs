//! Isolated load supervisor and system-under-test worker. No household socket.
use super::*;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpListener;
use tokio::process::Command;

#[path = "gateway_runtime/broker.rs"]
#[allow(dead_code)]
mod broker;

#[derive(Clone)]
struct Settings {
    sources: Vec<PathBuf>,
    copies: usize,
    seconds: u64,
    rate: usize,
    partitioned: bool,
    disk: bool,
    source_port: Option<u16>,
}

impl Settings {
    fn parse(args: &[String]) -> Result<Self> {
        let mut value = Self {
            sources: Vec::new(),
            copies: 10,
            seconds: 30,
            rate: 8,
            partitioned: false,
            disk: false,
            source_port: None,
        };
        let mut args = args.iter().skip(1);
        while let Some(arg) = args.next() {
            let next = args.next().context("missing load argument value")?;
            match arg.as_str() {
                "--source" => value.sources.push(next.into()),
                "--copies" => value.copies = next.parse()?,
                "--seconds" => value.seconds = next.parse()?,
                "--rate" => value.rate = next.parse()?,
                "--layout" => {
                    ensure!(
                        matches!(next.as_str(), "shared" | "per-house"),
                        "invalid layout"
                    );
                    value.partitioned = next == "per-house";
                }
                "--storage" => {
                    ensure!(
                        matches!(next.as_str(), "memory" | "disk"),
                        "invalid storage"
                    );
                    value.disk = next == "disk";
                }
                "--source-port" => value.source_port = Some(next.parse()?),
                _ => bail!("unknown load argument"),
            }
        }
        ensure!((1..=10).contains(&value.copies), "copies must be 1..10");
        ensure!((6..=120).contains(&value.seconds), "seconds must be 6..120");
        ensure!(
            (1..=256).contains(&value.rate),
            "rate must be 1..256 per house"
        );
        if value.source_port.is_some() {
            ensure!(
                value.sources.is_empty(),
                "worker cannot read source exports"
            );
            ensure!(
                !matches!(value.source_port, Some(0 | 1414 | 1415 | 1883 | 1884)),
                "reserved source port"
            );
        } else {
            ensure!(
                value.sources.len() == 2,
                "exactly two export roots required"
            );
        }
        Ok(value)
    }
    fn houses(&self) -> usize {
        2 * self.copies
    }
    fn layout(&self) -> &'static str {
        if self.partitioned {
            "per-house"
        } else {
            "shared"
        }
    }
    fn storage(&self) -> &'static str {
        if self.disk { "disk" } else { "memory" }
    }
}

pub(super) async fn run(args: &[String]) -> Result<()> {
    ensure!(
        matches!(
            args.first().map(String::as_str),
            Some("--synthetic-load" | "--synthetic-load-worker")
        ),
        "invalid load mode"
    );
    let settings = Settings::parse(args)?;
    if args[0] == "--synthetic-load-worker" {
        ensure!(
            settings.source_port.is_some(),
            "worker needs loopback source port"
        );
        worker(settings).await
    } else {
        ensure!(
            settings.source_port.is_none(),
            "supervisor cannot take a source port"
        );
        supervisor(settings).await
    }
}

fn with_controls(mut registry: Json, source: usize, copy: usize) -> Result<Json> {
    let input = load_workflows::input_points(source, copy)?;
    registry["house_id"] = json!(format!("load-house-{source}-{copy}"));
    registry["devices"]
        .as_array_mut()
        .context("device array")?
        .push(json!({
            "id":input.device_registry_id,"name":"Synthetic workflow inputs"
        }));
    registry["entities"].as_array_mut().context("entity array")?.extend([
        json!({"id":input.temperature_registry_id,"entity_id":input.temperature_entity_id,"device_id":input.device_registry_id,"original_device_class":"temperature","unit_of_measurement":"°C"}),
        json!({"id":input.opening_registry_id,"entity_id":input.opening_entity_id,"device_id":input.device_registry_id,"original_device_class":"opening"}),
        json!({"id":input.mode_registry_id,"entity_id":input.mode_entity_id,"device_id":input.device_registry_id}),
    ]);
    Ok(registry)
}

#[derive(Default)]
struct AuditClient {
    connected: bool,
    messages: u64,
    bytes: u64,
    pubacks: u64,
    errors: u64,
    topics: HashSet<String>,
    systems: BTreeMap<String, Json>,
}
impl AuditClient {
    fn snapshot(&self) -> Json {
        let system_gauges: Vec<_> = self.systems.values().map(|s| s["canaux"].clone()).collect();
        json!({"connected":self.connected,"messages":self.messages,"bytes":self.bytes,"pubacks":self.pubacks,"errors":self.errors,"unique_topics":self.topics.len(),"system_gauges":system_gauges,"error_scope":"includes normal final worker shutdown"})
    }
}

async fn observers(
    ports: &[u16],
    fixtures: &[Arc<load_fixture::SourceFixture>],
    settings: &Settings,
) -> Result<(
    Vec<Arc<Mutex<AuditClient>>>,
    Vec<tokio::task::JoinHandle<()>>,
)> {
    let mut handles = Vec::new();
    let mut states = Vec::new();
    for (index, port) in ports.iter().enumerate() {
        let mut options = MqttOptions::new(format!("load-observer-{index}"), "127.0.0.1", *port);
        options.set_credentials("load-only", "load-only");
        options.set_max_packet_size(MAX_PACKET, MAX_PACKET);
        let (client, mut network) = AsyncClient::new(options, 128);
        let stats = Arc::new(Mutex::new(AuditClient::default()));
        let output = stats.clone();
        let subscribed = client.clone();
        handles.push(tokio::spawn(async move {
            loop {
                match network.poll().await {
                    Ok(MqttEvent::Incoming(Packet::ConnAck(_))) => {
                        output.lock().expect("audit").connected = true;
                        if subscribed
                            .try_subscribe("moli-probe/maison/#", QoS::AtMostOnce)
                            .is_err()
                        {
                            break;
                        }
                    }
                    Ok(MqttEvent::Incoming(Packet::Publish(packet))) => {
                        let mut out = output.lock().expect("audit");
                        out.messages += 1;
                        out.bytes += packet.payload.len() as u64;
                        if packet.topic.ends_with("/systeme/state")
                            && let Ok(value) = serde_json::from_slice(&packet.payload)
                        {
                            out.systems.insert(packet.topic.clone(), value);
                        }
                        out.topics.insert(packet.topic);
                    }
                    Ok(MqttEvent::Incoming(Packet::PubAck(_))) => {
                        output.lock().expect("audit").pubacks += 1;
                    }
                    Ok(_) => {}
                    Err(_) => {
                        let mut out = output.lock().expect("audit");
                        out.connected = false;
                        out.errors += 1;
                        break;
                    }
                }
            }
        }));
        for (source, fixture) in fixtures.iter().enumerate() {
            for copy in 0..settings.copies {
                if settings.partitioned && source * settings.copies + copy != index {
                    continue;
                }
                let registry = with_controls(fixture.registry(copy), source, copy)?;
                client
                    .publish(
                        format!("moli-probe/source/load-{source}-{copy}/registry"),
                        QoS::AtLeastOnce,
                        true,
                        serde_json::to_vec(&registry)?,
                    )
                    .await?;
            }
        }
        states.push(stats);
    }
    Ok((states, handles))
}

async fn process_sample(pid: u32) -> Result<Json> {
    let output = Command::new("ps")
        .args(["-o", "rss=,time=", "-p", &pid.to_string()])
        .output()
        .await?;
    ensure!(output.status.success(), "process measurement failed");
    let line = std::str::from_utf8(&output.stdout)?;
    let parts: Vec<_> = line.split_whitespace().collect();
    ensure!(parts.len() == 2, "invalid process measurement");
    Ok(json!({"rss_bytes":parts[0].parse::<u64>()? * 1024,"cpu_seconds":cpu_time(parts[1])?}))
}
fn cpu_time(raw: &str) -> Result<f64> {
    let mut total = 0.0;
    for part in raw.split(':') {
        total = total * 60.0 + part.parse::<f64>()?;
    }
    Ok(total)
}

async fn supervisor(settings: Settings) -> Result<()> {
    let fixtures: Vec<_> = settings
        .sources
        .iter()
        .map(|p| load_fixture::SourceFixture::load(p).map(Arc::new))
        .collect::<Result<_>>()?;
    let fixture_stats: Vec<_> = fixtures.iter().map(|f| f.stats()).collect();
    let source = load_wire::SourceServer::start(fixtures.clone(), settings.copies).await?;
    let mut command = Command::new(std::env::current_exe()?);
    command.args([
        "--synthetic-load-worker",
        "--source-port",
        &source.port().to_string(),
        "--copies",
        &settings.copies.to_string(),
        "--seconds",
        &settings.seconds.to_string(),
        "--rate",
        &settings.rate.to_string(),
        "--layout",
        settings.layout(),
        "--storage",
        settings.storage(),
    ]);
    command
        .env_remove("MQTT_PASSWORD")
        .env_remove("MOLI_ENUM_CATALOGUE_DIR")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    let mut child = command.spawn()?;
    let pid = child.id().context("worker process")?;
    let mut lines = BufReader::new(child.stdout.take().context("worker output")?).lines();
    let mut observer_states = Vec::new();
    let mut observer_tasks = Vec::new();
    let mut samples = Vec::new();
    let mut start_sample = None;
    let mut end_sample = None;
    let mut generator_end = None;
    let mut observer_end = None;
    let mut started = None;
    let mut measured_seconds = 0.0;
    let mut worker_result = None;
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let watchdog = tokio::time::sleep(Duration::from_secs(settings.seconds + 100));
    tokio::pin!(watchdog);
    loop {
        tokio::select! {
            ()=&mut watchdog=>bail!("load worker deadline exceeded"),
            _=tick.tick()=>{
                if let Ok(mut sample)=process_sample(pid).await {
                    sample["phase"]=json!(if started.is_some()&&end_sample.is_none(){"measured"}else{"bootstrap_or_drain"});
                    samples.push(sample);
                }
            }
            line=lines.next_line()=>{
                let Some(line)=line? else {break;};
                if let Some(raw)=line.strip_prefix("LOAD_READY ") {
                    let ready:Json=serde_json::from_str(raw)?;
                    let ports:Vec<_>=ready["ports"].as_array().context("broker ports")?.iter().map(|v|v.as_u64().and_then(|v|u16::try_from(v).ok()).context("broker port")).collect::<Result<_>>()?;
                    (observer_states,observer_tasks)=observers(&ports,&fixtures,&settings).await?;
                } else if line.starts_with("LOAD_RUNNING ") {
                    start_sample=Some(process_sample(pid).await?);started=Some(Instant::now());source.begin(settings.rate);
                    println!("LOAD_PHASE {}",json!({"houses":settings.houses(),"phase":"measured","seconds":settings.seconds,"rate_per_house":settings.rate,"layout":settings.layout(),"storage":settings.storage()}));
                } else if line.starts_with("LOAD_PAUSE ") {
                    source.pause();generator_end=Some(source.snapshot());
                    observer_end=Some(observer_states.iter().map(|s|s.lock().expect("audit").snapshot()).collect::<Vec<_>>());
                    end_sample=Some(process_sample(pid).await?);measured_seconds=started.map_or(0.0,|t|t.elapsed().as_secs_f64());
                } else if let Some(raw)=line.strip_prefix("LOAD_RESULT ") {
                    worker_result=Some(serde_json::from_str::<Json>(raw)?);
                } else if let Some(raw)=line.strip_prefix("LOAD_SAMPLE ") {
                    // Aggregates only; do not repeat full bootstrap output.
                    let value:Json=serde_json::from_str(raw)?;
                    println!("LOAD_SAMPLE {}",value);
                }
            }
        }
    }
    let status = child.wait().await?;
    source.pause();
    let generator = generator_end.context("missing measured source snapshot")?;
    source.stop().await;
    let observers: Vec<_> = observer_states
        .iter()
        .map(|s| s.lock().expect("audit").snapshot())
        .collect();
    for task in observer_tasks {
        task.abort();
        let _ = task.await;
    }
    ensure!(status.success(), "load worker failed");
    let worker = worker_result.context("load worker returned no result")?;
    let cpu_delta = start_sample
        .as_ref()
        .zip(end_sample.as_ref())
        .and_then(|(a, b)| Some(b["cpu_seconds"].as_f64()? - a["cpu_seconds"].as_f64()?));
    let cpu_percent = cpu_delta.map(|c| c / measured_seconds.max(0.001) * 100.0);
    let peak_rss = samples.iter().filter_map(|s| s["rss_bytes"].as_u64()).max();
    let measured_peak_rss = samples
        .iter()
        .filter(|s| s["phase"] == "measured")
        .filter_map(|s| s["rss_bytes"].as_u64())
        .max();
    let waves = generator["waves_completed"].as_u64().unwrap_or(0);
    let expected_runs = waves.div_ceil(2) * settings.houses() as u64;
    let flow = &worker["workflows"];
    let workflow_parity = waves > 0
        && flow["threshold_runs_done"].as_u64() == Some(expected_runs)
        && flow["opening_runs_done"].as_u64() == Some(expected_runs)
        && flow["synthetic_threshold_commands"].as_u64() == Some(expected_runs)
        && flow["synthetic_opening_commands"].as_u64() == Some(expected_runs)
        && flow["local_notices"].as_u64() == Some(expected_runs)
        && flow["runs_failed"].as_u64() == Some(0)
        && flow["runs_observer_lagged_messages"].as_u64() == Some(0)
        && flow["notices_observer_lagged_messages"].as_u64() == Some(0);
    let houses = settings.houses() as u64;
    let generator_valid = generator["sessions"].as_u64() == Some(houses)
        && generator["ready_sessions"].as_u64() == Some(houses)
        && generator["active_sessions"].as_u64() == Some(houses)
        && generator["protocol_errors"].as_u64() == Some(0)
        && generator["commands_refused"].as_u64() == Some(0)
        && generator["slow_writes"].as_u64() == Some(0)
        && waves >= (settings.seconds - 1) / 6
        && generator["session_waves_completed"].as_u64() == Some(waves * houses)
        && generator["update_messages_sent"].as_u64().unwrap_or(0)
            >= (settings.seconds - 1) * settings.rate as u64 * houses;
    let broker_errors = &worker["measured_end"]["broker"]["errors"];
    let broker_healthy = broker_errors
        .as_object()
        .is_some_and(serde_json::Map::is_empty);
    let healthy = workflow_parity
        && generator_valid
        && broker_healthy
        && worker["bootstrap_stable"] == true
        && worker["measured_end"]["drivers_running"].as_u64() == Some(houses + 1);
    let outcome = if healthy {
        "passed"
    } else if !broker_healthy {
        "broker_limit_or_transport_failure"
    } else {
        "failed"
    };
    println!(
        "LOAD_REPORT {}",
        json!({
            "schema":1,"houses":settings.houses(),"copies_per_source":settings.copies,"sources":fixture_stats,
            "seconds":measured_seconds,"configured_rate_per_house":settings.rate,"layout":settings.layout(),"storage":settings.storage(),
            "worker":worker,"generator":generator,"observers":observers,"observers_measured_end":observer_end,
            "resources":{"start":start_sample,"end":end_sample,"cpu_seconds_delta":cpu_delta,"cpu_percent_one_core":cpu_percent,"rss_peak_sampled_bytes":peak_rss,"rss_measured_peak_sampled_bytes":measured_peak_rss,"samples":samples},
            "workflow_expected_per_kind":expected_runs,"workflow_parity":workflow_parity,"generator_valid":generator_valid,"healthy":healthy,"outcome":outcome,
            "physical_commands":0,"scope":"synthetic replay from historical exports; real House/Hub/MQTT/automation/broker; generator excluded from worker resources; no web UI or physical device",
            "profile":if cfg!(debug_assertions){"debug"}else{"release"}
        })
    );
    Ok(())
}

struct Brokers {
    ports: Vec<u16>,
    brokers: Vec<Arc<broker::Broker>>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    cancel: CancellationToken,
    errors: Arc<Mutex<BTreeMap<String, u64>>>,
    directory: Option<PathBuf>,
}
impl Brokers {
    async fn start(settings: &Settings) -> Result<Self> {
        let cancel = CancellationToken::new();
        let errors = Arc::new(Mutex::new(BTreeMap::new()));
        let directory = if settings.disk {
            let p = std::env::temp_dir().join(format!(
                "moli-synthetic-load-{}-{}",
                std::process::id(),
                moli_core::now_ms()
            ));
            std::fs::create_dir(&p)?;
            Some(p)
        } else {
            None
        };
        let mut out = Self {
            ports: Vec::new(),
            brokers: Vec::new(),
            tasks: Vec::new(),
            cancel,
            errors,
            directory,
        };
        for index in 0..if settings.partitioned {
            settings.houses()
        } else {
            1
        } {
            let broker = broker::Broker::new(Arc::new(|_, topic, payload, _| {
                ensure!(
                    topic.starts_with("moli-probe/") && !topic.ends_with("/set"),
                    "synthetic broker rejects commands"
                );
                Ok(payload.to_vec())
            }));
            broker.set_credentials("load-only".into(), "load-only".into());
            if let Some(p) = &out.directory {
                broker.set_description_directory(p.join(index.to_string()))?;
            }
            let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
            out.ports.push(listener.local_addr()?.port());
            out.brokers.push(broker.clone());
            let cancel = out.cancel.clone();
            let errors = out.errors.clone();
            out.tasks.push(tokio::spawn(async move {
                let mut connections=tokio::task::JoinSet::new();
                loop {
                    tokio::select! {
                        ()=cancel.cancelled()=>break,
                        Some(_)=connections.join_next(),if !connections.is_empty()=>{},
                        accepted=listener.accept()=>{
                            let Ok((stream,address))=accepted else {break;};
                            if !address.ip().is_loopback(){continue;}
                            let b=broker.clone();let e=errors.clone();let c=cancel.clone();
                            connections.spawn(async move {
                                tokio::select! {
                                    ()=c.cancelled()=>{},
                                    result=b.handle(stream,broker::Role::Bridge)=>{
                                        if let Err(error)=result {*e.lock().expect("broker errors").entry(error.to_string()).or_default()+=1;}
                                    }
                                }
                            });
                        }
                    }
                }
                connections.abort_all();while connections.join_next().await.is_some(){}
            }));
        }
        Ok(out)
    }
    fn stats(&self) -> Json {
        let brokers: Vec<_> = self.brokers.iter().map(|b| b.stats()).collect();
        let total = |field: &str| {
            brokers
                .iter()
                .filter_map(|b| b[field].as_u64())
                .sum::<u64>()
        };
        json!({"count":brokers.len(),"clients":total("clients"),"retained_topics":total("retained_topics"),"retained_payload_bytes":total("retained_payload_bytes"),"description_ram_bytes":total("description_ram_bytes"),"description_disk_bytes":total("description_disk_bytes"),"errors":*self.errors.lock().expect("errors"),"brokers":brokers})
    }
    async fn stop(mut self) {
        self.cancel.cancel();
        for task in self.tasks.drain(..) {
            let _ = task.await;
        }
        self.brokers.clear();
        if let Some(p) = self.directory.take() {
            let _ = std::fs::remove_dir_all(p);
        }
    }
}

fn bridge_config(settings: &Settings, port: u16, destination: u16) -> Config {
    Config {
        hosts: vec!["127.0.0.1".into()],
        port,
        username: "load-only".into(),
        base: "zigbee2mqtt".into(),
        seconds: settings.seconds,
        bridges: 1,
        benchmark: false,
        all_topics: false,
        republish: Some("moli-probe".into()),
        republish_retain: true,
        republish_clear: false,
        virtual_tree: true,
        self_test_commands: false,
        virtual_mirror: true,
        publish_config: true,
        destination: Some(format!("127.0.0.1:{destination}")),
        metadata_memory: true,
    }
}
fn worker_snapshot(hub: &Hub, metrics: &[Arc<Mutex<Metrics>>], brokers: &Brokers) -> Json {
    let stats = hub.stats();
    let sources:Vec<_>=metrics.iter().map(|m|{let m=Metrics::lock(m);json!({"connected":m.connected,"received":m.messages,"published":m.republished,"projection":m.house_counts})}).collect();
    json!({"hub_devices":stats.devices,"hub_points":stats.points,"drivers_running":stats.drivers_running,"allocator_live_bytes":memory::live(),"allocator_peak_bytes":memory::peak(),"broker":brokers.stats(),"sources":sources})
}
async fn worker(settings: Settings) -> Result<()> {
    let source_port = settings.source_port.context("source port")?;
    let hub = Hub::new(HubOptions {
        event_capacity: 4096,
        journal_capacity: 64,
        ..HubOptions::default()
    })?;
    let brokers = Brokers::start(&settings).await?;
    let cancel = CancellationToken::new();
    let mut drivers = Vec::new();
    let mut metrics = Vec::new();
    for source in 0..2 {
        for copy in 0..settings.copies {
            let instance = load_workflows::input_points(source, copy)?.instance;
            let m = Arc::new(Mutex::new(Metrics::default()));
            let index = if settings.partitioned {
                source * settings.copies + copy
            } else {
                0
            };
            drivers.push(spawn_driver(
                &hub,
                instance.clone(),
                Arc::new(Live {
                    config: bridge_config(&settings, source_port, brokers.ports[index]),
                    host: "127.0.0.1".into(),
                    password: "load-only".into(),
                    hub: hub.clone(),
                    metrics: m.clone(),
                    client_id: instance.to_string(),
                    instance,
                }),
                cancel.clone(),
            ));
            metrics.push(m);
        }
    }
    println!("LOAD_READY {}", json!({"ports":brokers.ports}));
    let bootstrap = Instant::now();
    let mut last_received = 0;
    let mut stable = 0;
    while bootstrap.elapsed() < Duration::from_secs(40) {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let received = metrics
            .iter()
            .map(|m| Metrics::lock(m).messages)
            .sum::<u64>();
        let ready = metrics.iter().all(|m| {
            Metrics::lock(m).house_counts["observed_channels"]
                .as_u64()
                .unwrap_or(0)
                > 0
        });
        let expected_topics = metrics
            .iter()
            .map(|m| {
                let m = Metrics::lock(m);
                m.house_counts["channels"].as_u64().unwrap_or(0)
                    + m.house_counts["nodes"].as_u64().unwrap_or(0)
                    + 3
            })
            .sum::<u64>();
        let broker_stats = brokers.stats();
        let no_errors = broker_stats["errors"]
            .as_object()
            .is_some_and(serde_json::Map::is_empty);
        let retained_complete =
            broker_stats["retained_topics"].as_u64().unwrap_or(0) >= expected_topics;
        stable = if ready && retained_complete && no_errors && received == last_received {
            stable + 1
        } else {
            0
        };
        last_received = received;
        if stable >= 3 || !no_errors {
            break;
        }
    }
    let flows = load_workflows::LoadWorkflows::start(&hub, settings.copies, 2).await;
    let (flows, flow_error) = match flows {
        Ok(f) => (Some(f), None),
        Err(e) => (None, Some(e.to_string())),
    };
    let bootstrap_state = worker_snapshot(&hub, &metrics, &brokers);
    let bootstrap_seconds = bootstrap.elapsed().as_secs_f64();
    memory::reset_peak();
    println!(
        "LOAD_RUNNING {}",
        json!({"bootstrap_seconds":bootstrap_seconds,"ready":stable>=3})
    );
    let started = Instant::now();
    let mut tick = tokio::time::interval(Duration::from_secs(5));
    tick.tick().await;
    loop {
        tokio::select! {
            ()=tokio::time::sleep_until(tokio::time::Instant::from_std(started+Duration::from_secs(settings.seconds)))=>break,
            _=tick.tick()=>{
                let snap=worker_snapshot(&hub,&metrics,&brokers);
                println!("LOAD_SAMPLE {}",json!({"elapsed":started.elapsed().as_secs_f64(),"hub_devices":snap["hub_devices"],"hub_points":snap["hub_points"],"allocator_live_bytes":snap["allocator_live_bytes"],"retained_topics":snap["broker"]["retained_topics"],"broker_errors":snap["broker"]["errors"]}));
            }
        }
    }
    let measured = worker_snapshot(&hub, &metrics, &brokers);
    println!("LOAD_PAUSE {{}}");
    tokio::time::sleep(Duration::from_secs(2)).await;
    let workflow_stats = flows.as_ref().map(|f| f.snapshot());
    let result = json!({"bootstrap_seconds":bootstrap_seconds,"bootstrap_stable":stable>=3,"bootstrap":bootstrap_state,"measured_end":measured,"after_drain":worker_snapshot(&hub,&metrics,&brokers),"workflows":workflow_stats,"workflow_start_error":flow_error,"physical_commands":0});
    if let Some(f) = flows {
        f.stop().await;
    }
    cancel.cancel();
    for driver in drivers {
        let _ = driver.await;
    }
    brokers.stop().await;
    println!("LOAD_RESULT {result}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(tail: &[&str]) -> Vec<String> {
        std::iter::once("--synthetic-load")
            .chain(tail.iter().copied())
            .map(str::to_owned)
            .collect()
    }
    #[test]
    fn load_cannot_target_household_ports_or_exceed_twenty_houses() {
        assert!(Settings::parse(&args(&["--source-port", "1415"])).is_err());
        assert!(Settings::parse(&args(&["--source-port", "1883"])).is_err());
        assert!(Settings::parse(&args(&["--source-port", "35000", "--copies", "11"])).is_err());
        assert!(
            Settings::parse(&args(&["--source", "a", "--source", "b", "--copies", "10"])).is_ok()
        );
    }
    #[test]
    fn process_cpu_time_is_measured_in_seconds_not_wall_percent() {
        assert!((cpu_time("1:02.50").unwrap() - 62.5).abs() < f64::EPSILON);
        assert!((cpu_time("01:02:03.50").unwrap() - 3723.5).abs() < f64::EPSILON);
    }
    #[test]
    fn workflow_registry_and_points_use_the_same_house_identity() {
        let registry = json!({"devices":[],"entities":[],"areas":[],"floors":[]});
        let out = with_controls(registry, 1, 9).unwrap();
        let points = load_workflows::input_points(1, 9).unwrap();
        assert_eq!(
            out["entities"][0]["entity_id"],
            points.temperature_entity_id
        );
        assert_eq!(out["entities"].as_array().unwrap().len(), 3);
        assert_eq!(out["house_id"], "load-house-1-9");
    }
}
