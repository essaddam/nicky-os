//! Lightweight process groups. Only fixture adapters and loopback MQTT exist here.
use crate::{
    layout::{Layout, Mode, Plan},
    module::{Instance, availability},
};
use anyhow::{Context, Result, ensure};
use moli_core::{Device, Kind, PointId};
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    process::{Child, Command},
};
use tokio_util::sync::CancellationToken;

#[path = "../../mqtt-live-probe/src/gateway_runtime/broker.rs"]
#[allow(dead_code)]
mod broker;

fn now_ms() -> u64 {
    moli_core::now_ms()
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Worker {
    pub plan: Plan,
    pub port: u16,
    pub seconds: u64,
    pub fail: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TestCommand {
    request_id: String,
    generation: String,
    point: PointId,
    value: Json,
    expires_ms: u64,
}

pub async fn worker(config: Worker) -> Result<Json> {
    ensure!(
        config.port != 0
            && config.port != 1414
            && config.port != 1415
            && config.port != 1883
            && config.port != 1884,
        "reserved broker port"
    );
    ensure!(
        config.seconds <= 32 && config.plan.modules.len() <= 32,
        "worker bounds"
    );
    let mut modules = config
        .plan
        .modules
        .iter()
        .cloned()
        .map(Instance::new)
        .collect::<Result<Vec<_>>>()?;
    let mut options = MqttOptions::new(
        format!("capsule-{}", config.plan.id),
        "127.0.0.1",
        config.port,
    );
    options.set_credentials("capsule-fixture", "capsule-fixture");
    options.set_max_packet_size(4 * 1024 * 1024, 4 * 1024 * 1024);
    let (client, mut network) = AsyncClient::new(options, 128);
    let (incoming, mut events) = tokio::sync::mpsc::channel(128);
    let dropped_events = Arc::new(AtomicU64::new(0));
    let overflow = dropped_events.clone();
    let pump = tokio::spawn(async move {
        loop {
            let result = network.poll().await;
            let failed = result.is_err();
            if !failed
                && !matches!(
                    &result,
                    Ok(Event::Incoming(Packet::ConnAck(_) | Packet::Publish(_)))
                )
            {
                continue;
            }
            // Never stop the MQTT event loop while the bounded command queue
            // is full. Unadmitted commands receive no execution receipt.
            match incoming.try_send(result) {
                Ok(()) => {}
                Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                    overflow.fetch_add(1, Ordering::Relaxed);
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => break,
            }
            if failed {
                break;
            }
        }
    });
    // Keep initialization separate from updates and never send a command retained.
    let connected = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if matches!(
                events.recv().await.context("MQTT stopped")??,
                Event::Incoming(Packet::ConnAck(_))
            ) {
                break;
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await?;
    connected?;
    for module in &mut modules {
        client
            .subscribe(module.topic("simulate"), QoS::AtMostOnce)
            .await?;
        client
            .publish(
                module.topic("config"),
                QoS::AtLeastOnce,
                true,
                module.description(&config.plan.id)?,
            )
            .await?;
        let (topic, body) = availability(&module.spec.id, &module.generation, true);
        client.publish(topic, QoS::AtLeastOnce, true, body).await?;
        for (topic, body) in module.observe(0)? {
            client.publish(topic, QoS::AtLeastOnce, true, body).await?;
        }
    }
    println!(
        "HOST_READY {}",
        json!({"group":config.plan.id,"port":config.port,"module_count":modules.len()})
    );
    let started = Instant::now();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.tick().await;
    let (mut seen, mut accepted, mut rejected, mut step) = (0u64, 0u64, 0u64, 0u64);
    let mut receipts = BTreeMap::<String, u64>::new();
    loop {
        tokio::select! {
            ()=async {if config.seconds==0{std::future::pending::<()>().await}else{tokio::time::sleep_until(tokio::time::Instant::from_std(started+Duration::from_secs(config.seconds))).await}}=>break,
            _=tick.tick()=>{
                step+=1;
                println!("HOST_HEARTBEAT {}",json!({"group":config.plan.id}));
                if config.fail&&step==1 {std::process::exit(42);}
                for module in &mut modules {for (topic,body) in module.observe(step)? {client.publish(topic,QoS::AtLeastOnce,true,body).await?;}}
            }
            packet=events.recv()=>{
                let event=packet.context("module MQTT stream stopped")??;
                if let Event::Incoming(Packet::Publish(packet))=event {
                    let Some(module)=modules.iter_mut().find(|m|packet.topic==m.topic("simulate"))else{continue;};
                    seen+=1;
                    let result=(||->Result<(String,Json)>{
                        ensure!(!packet.retain&&packet.payload.len()<=1024,"retained or oversized test command");
                        let command:TestCommand=serde_json::from_slice(&packet.payload)?;
                        ensure!(command.generation==module.generation,"old module generation");
                        ensure!(!command.request_id.is_empty()&&command.request_id.len()<=64&&command.request_id.bytes().all(|b|b.is_ascii_alphanumeric()||b==b'-'),"invalid request identity");
                        let now=now_ms();receipts.retain(|_,expires|*expires>now);
                        ensure!(command.expires_ms>now&&command.expires_ms-now<=5000,"expired or excessive command lifetime");
                        ensure!(receipts.len()<128&&!receipts.contains_key(&command.request_id),"duplicate request or receipt bound");
                        receipts.insert(command.request_id.clone(),command.expires_ms);
                        Ok((command.request_id,module.command(&command.point,&command.value)?))
                    })();
                    let body=match result {
                        Ok((request_id,result))=>{accepted+=1;json!({"module":module.spec.id,"request_id":request_id,"result":result})},
                        Err(_)=>{rejected+=1;json!({"module":module.spec.id,"result":{"status":"rejected","sent":false}})},
                    };
                    client.publish(module.topic("command-result"),QoS::AtMostOnce,false,serde_json::to_vec(&body)?).await?;
                }
            }
        }
    }
    let values: Vec<_> = modules.iter().map(Instance::metrics).collect();
    let result = json!({"group":config.plan.id,"module_count":modules.len(),"modules":values,"shared_mqtt_connections":1,"allocator_live_bytes":crate::memory::live(),"allocator_peak_bytes":crate::memory::peak(),"commands_seen":seen,"commands_simulated":accepted,"commands_rejected":rejected,"dropped_incoming_events":dropped_events.load(Ordering::Relaxed),"physical_commands":0,"allocation_scope":"whole process, including shared resources; not a per-module RSS"});
    for module in &mut modules {
        let (topic, body) = availability(&module.spec.id, &module.generation, false);
        client.publish(topic, QoS::AtLeastOnce, true, body).await?;
    }
    tokio::time::sleep(Duration::from_millis(150)).await;
    let _ = client.disconnect().await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    pump.abort();
    let _ = pump.await;
    Ok(result)
}

#[derive(Default)]
struct Audit {
    descriptions: BTreeMap<String, Json>,
    first: BTreeMap<String, Json>,
    current: BTreeMap<String, Json>,
    owners: BTreeMap<String, String>,
    availability: BTreeMap<String, bool>,
    responses: Vec<Json>,
    messages: u64,
    pending: BTreeMap<String, (String, Json)>,
}
impl Audit {
    fn apply(&mut self, topic: &str, payload: &[u8]) {
        let Ok(v) = serde_json::from_slice::<Json>(payload) else {
            return;
        };
        let Some(id) = v["module"].as_str().map(str::to_owned) else {
            return;
        };
        self.messages += 1;
        if topic.ends_with("/config") {
            if self
                .descriptions
                .get(&id)
                .is_some_and(|old| old["generation"] != v["generation"])
            {
                self.current
                    .retain(|point, _| self.owners.get(point) != Some(&id));
                self.first
                    .retain(|point, _| self.owners.get(point) != Some(&id));
                self.owners.retain(|_, owner| owner != &id);
                self.availability.remove(&id);
            }
            self.descriptions.insert(id.clone(), v);
            let ready = self
                .pending
                .iter()
                .filter(|(_, (_, value))| {
                    value["module"] == id
                        && value["generation"] == self.descriptions[&id]["generation"]
                })
                .map(|(k, _)| k.clone())
                .collect::<Vec<_>>();
            for key in ready {
                if let Some((topic, value)) = self.pending.remove(&key) {
                    self.apply(
                        &topic,
                        &serde_json::to_vec(&value).expect("buffered observation"),
                    );
                }
            }
        } else if topic.ends_with("/availability") {
            if self
                .descriptions
                .get(&id)
                .is_some_and(|d| d["generation"] == v["generation"])
            {
                self.availability
                    .insert(id, v["online"].as_bool().unwrap_or(false));
            } else {
                self.buffer(format!("availability:{id}"), topic, payload.len(), v);
            }
        } else if topic.ends_with("/command-result") {
            self.responses.push(v);
        } else if let Some(point) = v["point"].as_str().map(str::to_owned) {
            if !self
                .descriptions
                .get(&id)
                .is_some_and(|d| d["generation"] == v["generation"])
            {
                self.buffer(point, topic, payload.len(), v);
                return;
            }
            // The authoritative schema is also retained; no numeric interpretation
            // of a code is needed to compare the transport representations here.
            self.first
                .entry(point.clone())
                .or_insert_with(|| v["value"].clone());
            self.owners.insert(point.clone(), id);
            self.current.insert(point, v["value"].clone());
        }
    }
    fn buffer(&mut self, key: String, topic: &str, bytes: usize, value: Json) {
        if self.pending.len() < 256
            && bytes <= 32768
            && self
                .pending
                .values()
                .map(|(_, v)| v.to_string().len())
                .sum::<usize>()
                + bytes
                <= 256 * 1024
        {
            self.pending.insert(key, (topic.into(), value));
        }
    }
    fn initial_digest(&self) -> Result<String> {
        use sha2::{Digest, Sha256};
        let mut descriptions = self.descriptions.clone();
        for value in descriptions.values_mut() {
            value
                .as_object_mut()
                .context("description object")?
                .remove("group");
            value
                .as_object_mut()
                .context("description object")?
                .remove("generation");
        }
        Ok(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&(descriptions, &self.first))?)
        ))
    }
}

struct Server {
    port: u16,
    broker: Arc<broker::Broker>,
    cancel: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}
impl Server {
    async fn start() -> Result<Self> {
        let broker = broker::Broker::new(Arc::new(|_, topic, payload, _| {
            ensure!(
                topic.starts_with("moli-probe/module/"),
                "unknown module namespace"
            );
            Ok(payload.to_vec())
        }));
        broker.set_credentials("capsule-fixture".into(), "capsule-fixture".into());
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let port = listener.local_addr()?.port();
        let cancel = CancellationToken::new();
        let done = cancel.clone();
        let b = broker.clone();
        let task = tokio::spawn(async move {
            let mut tasks = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    ()=done.cancelled()=>break,
                    Some(_)=tasks.join_next(),if !tasks.is_empty()=>{},
                    item=listener.accept()=>{let Ok((stream,address))=item else{break;};if !address.ip().is_loopback(){continue;}let b=b.clone();let c=done.clone();tasks.spawn(async move{tokio::select!{()=c.cancelled()=>{},_ = b.handle(stream,broker::Role::Bridge)=>{}}});}
                }
            }
            tasks.abort_all();
            while tasks.join_next().await.is_some() {}
        });
        Ok(Self {
            port,
            broker,
            cancel,
            task,
        })
    }
    async fn stop(self) {
        self.cancel.cancel();
        let _ = self.task.await;
    }
}

async fn sample(pid: u32) -> Result<Json> {
    let data = Command::new("ps")
        .args(["-o", "rss=,time=", "-p", &pid.to_string()])
        .output()
        .await?;
    ensure!(data.status.success(), "process no longer available");
    let output = std::str::from_utf8(&data.stdout)?;
    let parts: Vec<_> = output.split_whitespace().collect();
    ensure!(parts.len() == 2, "invalid process sample");
    let mut cpu = 0.0;
    for part in parts[1].split(':') {
        cpu = cpu * 60.0 + part.parse::<f64>()?;
    }
    Ok(json!({"rss_bytes":parts[0].parse::<u64>()?*1024,"cpu_seconds":cpu}))
}
struct Running {
    id: String,
    modules: Vec<String>,
    child: Child,
    reader: tokio::task::JoinHandle<Vec<Json>>,
}
async fn spawn(config: Worker) -> Result<Running> {
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg("--worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .env_remove("MQTT_PASSWORD")
        .env_remove("MOLI_MASTER_KEY")
        .env_remove("MOLI_MASTER_KEY_FILE");
    let mut child = command.spawn()?;
    let mut input = child.stdin.take().context("worker input")?;
    input.write_all(&serde_json::to_vec(&config)?).await?;
    input.shutdown().await?;
    let stdout = child.stdout.take().context("worker output")?;
    let reader = tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        let mut reports = Vec::new();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(raw) = line.strip_prefix("GROUP_RESULT ")
                && let Ok(value) = serde_json::from_str(raw)
            {
                reports.push(value);
            }
        }
        reports
    });
    Ok(Running {
        id: config.plan.id,
        modules: config.plan.modules.into_iter().map(|m| m.id).collect(),
        child,
        reader,
    })
}

fn command_for(description: &Json) -> Result<Option<(PointId, Json)>> {
    let devices: Vec<Device> = serde_json::from_value(description["devices"].clone())?;
    for device in devices {
        for point in &device.points {
            if !point.access.write {
                continue;
            }
            let value = match &point.kind {
                Kind::Binary => json!(true),
                Kind::EnumCode { codes, .. } => {
                    let Some(code) = (0..=255).find(|c| codes.contains(*c)) else {
                        continue;
                    };
                    json!(code)
                }
                Kind::Numeric { min, max, .. } => {
                    json!(60.0_f64.max(min.unwrap_or(0.0)).min(max.unwrap_or(60.0)))
                }
                _ => continue,
            };
            return Ok(Some((PointId::new(&device.id, &point.key), value)));
        }
    }
    Ok(None)
}

pub async fn run(layout: Layout, fail_group: Option<&str>) -> Result<Json> {
    ensure!(
        layout.seconds > 0,
        "continuous groups must be launched by the master"
    );
    let plans = layout.plan()?;
    if plans.is_empty() {
        return Ok(
            json!({"mode":layout.mode,"active_modules":0,"module_groups":0,"child_processes":0,"broker_started":false,"physical_commands":0}),
        );
    }
    ensure!(
        fail_group.is_none() || layout.mode != Mode::Merged,
        "process failure injection requires a separate group"
    );
    let server = Server::start().await?;
    let audit = Arc::new(Mutex::new(Audit::default()));
    let mut options = MqttOptions::new("capsule-observer", "127.0.0.1", server.port);
    options.set_credentials("capsule-fixture", "capsule-fixture");
    options.set_max_packet_size(4 * 1024 * 1024, 4 * 1024 * 1024);
    let (observer, mut network) = AsyncClient::new(options, 128);
    observer
        .subscribe("moli-probe/module/#", QoS::AtMostOnce)
        .await?;
    let out = audit.clone();
    let pump = tokio::spawn(async move {
        loop {
            match network.poll().await {
                Ok(Event::Incoming(Packet::Publish(p))) => {
                    out.lock().expect("audit").apply(&p.topic, &p.payload)
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });
    let mut children = Vec::new();
    let mut inline = None;
    for plan in &plans {
        let config = Worker {
            plan: plan.clone(),
            port: server.port,
            seconds: if layout.seconds == 0 {
                0
            } else {
                layout.seconds + 2
            },
            fail: fail_group == Some(plan.id.as_str()),
        };
        if layout.mode == Mode::Merged {
            inline = Some(tokio::spawn(worker(config)));
        } else {
            children.push(spawn(config).await?);
        }
    }
    let active = plans.iter().map(|p| p.modules.len()).sum::<usize>();
    let warm = Instant::now();
    while warm.elapsed() < Duration::from_secs(5) {
        if audit.lock().expect("audit").descriptions.len() == active {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // All fixture modules publish their first observations before their first
    // one-second update, so wait for the bounded initial MQTT flush.
    tokio::time::sleep(Duration::from_millis(100)).await;
    ensure!(
        audit.lock().expect("audit").descriptions.len() == active,
        "missing module description"
    );
    let initial_digest = audit.lock().expect("audit").initial_digest()?;
    let initial_count = audit.lock().expect("audit").current.len();
    let descriptions = audit.lock().expect("audit").descriptions.clone();
    let mut expected_commands = 0;
    for (index, (id, description)) in descriptions.iter().enumerate() {
        if let Some((point, value)) = command_for(description)? {
            let bytes = serde_json::to_vec(
                &json!({"request_id":format!("probe-{index}"),"generation":description["generation"],"point":point,"value":value,"expires_ms":now_ms()+4000}),
            )?;
            ensure!(bytes.len() <= 1024, "test envelope exceeds bound");
            observer
                .publish(
                    format!("moli-probe/module/{id}/simulate"),
                    QoS::AtMostOnce,
                    false,
                    bytes,
                )
                .await?;
            expected_commands += 1;
        }
    }
    let measured = Instant::now();
    let mut samples = BTreeMap::<String, Vec<Json>>::new();
    let mut failed = BTreeMap::<String, Vec<String>>::new();
    while measured.elapsed() < Duration::from_secs(layout.seconds) {
        if let Ok(value) = sample(std::process::id()).await {
            samples
                .entry(if layout.mode == Mode::Merged {
                    "merged".into()
                } else {
                    "supervisor-broker-observer".into()
                })
                .or_default()
                .push(value);
        }
        for child in &mut children {
            if let Some(status) = child.child.try_wait()? {
                if !status.success() && !failed.contains_key(&child.id) {
                    failed.insert(child.id.clone(), child.modules.clone());
                    for module in &child.modules {
                        let generation = audit
                            .lock()
                            .expect("audit")
                            .descriptions
                            .get(module)
                            .and_then(|v| v["generation"].as_str())
                            .unwrap_or("")
                            .to_owned();
                        let (topic, body) = availability(module, &generation, false);
                        server.broker.publish(topic, body, true)?;
                    }
                }
            } else if let Some(pid) = child.child.id()
                && let Ok(value) = sample(pid).await
            {
                samples.entry(child.id.clone()).or_default().push(value);
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let phase_seconds = measured.elapsed().as_secs_f64();
    let (responses, live_availability, end_count, messages) = {
        let state = audit.lock().expect("audit");
        (
            state.responses.clone(),
            state.availability.clone(),
            state.current.len(),
            state.messages,
        )
    };
    let mut reports = Vec::new();
    if let Some(task) = inline {
        reports.push(task.await??);
    }
    for mut child in children {
        let status = child.child.wait().await?;
        let rows = child.reader.await?;
        reports.extend(rows);
        if !status.success() && !failed.contains_key(&child.id) {
            failed.insert(child.id, child.modules);
        }
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    let offline = audit
        .lock()
        .expect("audit")
        .availability
        .values()
        .all(|online| !*online);
    let resources:BTreeMap<_,_>=samples.into_iter().map(|(id,rows)|{
        let cpu=rows.first().zip(rows.last()).map(|(a,b)|(b["cpu_seconds"].as_f64().unwrap_or(0.0)-a["cpu_seconds"].as_f64().unwrap_or(0.0))/phase_seconds*100.0);
        let rss=rows.iter().filter_map(|v|v["rss_bytes"].as_u64()).max();
        (id,json!({"rss_peak_sampled_bytes":rss,"cpu_percent_one_core":cpu,"samples":rows.len(),"cpu_scope":"process over shared observation window; failed processes have shorter windows"}))
    }).collect();
    let simulated = responses
        .iter()
        .filter(|r| r["result"]["status"] == "simulated" && r["result"]["sent"] == false)
        .count();
    let broker_stats = server.broker.stats();
    let report = json!({"mode":layout.mode,"active_modules":active,"module_groups":plans.len(),"child_processes":if layout.mode==Mode::Merged{0}else{plans.len()},"total_processes":if layout.mode==Mode::Merged{1}else{plans.len()+1},"initial_semantic_digest":initial_digest,"initial_observed_points":initial_count,"observed_points_end":end_count,"messages":messages,"expected_test_commands":expected_commands,"simulated_commands":simulated,"command_results":responses,"availability_before_shutdown":live_availability,"all_modules_offline_after_shutdown":offline,"failed_groups":failed,"resources":resources,"groups":reports,"broker":broker_stats,"physical_commands":0,"scope":"fixture-based modular MQTT host; native codecs, no API connection or physical executor; RSS is per process; no cross-process mmap"});
    let _ = observer.disconnect().await;
    pump.abort();
    let _ = pump.await;
    server.stop().await;
    Ok(report)
}

pub async fn worker_from_stdin() -> Result<()> {
    let mut bytes = Vec::new();
    tokio::io::stdin()
        .take(65537)
        .read_to_end(&mut bytes)
        .await?;
    ensure!(bytes.len() <= 65536, "worker config bound");
    let config: Worker = serde_json::from_slice(&bytes)?;
    let report = worker(config).await?;
    println!("GROUP_RESULT {report}");
    Ok(())
}

pub fn read_layout(path: PathBuf) -> Result<Layout> {
    ensure!(path.metadata()?.len() <= 65536, "layout bound");
    Ok(toml::from_str(&std::fs::read_to_string(path)?)?)
}

pub async fn broker_service() -> Result<()> {
    let server = Server::start().await?;
    println!(
        "HOST_READY {}",
        json!({"group":"transport","port":server.port})
    );
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {_ = tokio::signal::ctrl_c()=>break,_=tick.tick()=>println!("HOST_HEARTBEAT {{}}")}
    }
    server.stop().await;
    Ok(())
}

pub async fn serve_group(path: PathBuf, id: &str, port: u16) -> Result<()> {
    let layout = read_layout(path)?;
    let plan = layout
        .plan()?
        .into_iter()
        .find(|p| p.id == id)
        .context("unknown process group")?;
    let server = if port == 0 {
        Some(Server::start().await?)
    } else {
        None
    };
    let port = server.as_ref().map_or(port, |s| s.port);
    let report = worker(Worker {
        plan,
        port,
        seconds: if layout.seconds == 0 {
            0
        } else {
            layout.seconds + 2
        },
        fail: false,
    })
    .await?;
    println!("GROUP_RESULT {report}");
    if let Some(server) = server {
        server.stop().await;
    }
    Ok(())
}

#[cfg(test)]
mod contract_tests {
    use super::*;
    #[test]
    fn new_generation_discards_old_values_and_availability() {
        let mut audit = Audit::default();
        audit.apply(
            "moli-probe/module/fixture/config",
            br#"{"module":"fixture","generation":"old"}"#,
        );
        audit.apply(
            "moli-probe/module/fixture/state/value",
            br#"{"module":"fixture","point":"fixture:device/value","generation":"old","value":7}"#,
        );
        audit.apply(
            "moli-probe/module/fixture/availability",
            br#"{"module":"fixture","generation":"old","online":true}"#,
        );
        audit.apply(
            "moli-probe/module/fixture/config",
            br#"{"module":"fixture","generation":"new"}"#,
        );
        assert!(audit.current.is_empty());
        assert!(audit.first.is_empty());
        assert!(audit.availability.is_empty());
    }
    #[tokio::test]
    async fn maximum_merged_group_boots_without_blocking_the_mqtt_pump() {
        let server = Server::start().await.unwrap();
        let config = Worker {
            plan: Plan {
                id: "full-fixture".into(),
                modules: (0..32)
                    .map(|i| crate::layout::Module {
                        id: format!("weather-{i}"),
                        kind: crate::layout::Kind::Weather,
                        enabled: true,
                        source_ref: format!("fixture-{i}"),
                    })
                    .collect(),
            },
            port: server.port,
            seconds: 1,
            fail: false,
        };
        let report = tokio::time::timeout(Duration::from_secs(6), worker(config)).await;
        server.stop().await;
        let report = report.expect("MQTT initialization deadlocked").unwrap();
        assert_eq!(report["module_count"], 32);
        assert!(
            report["modules"]
                .as_array()
                .unwrap()
                .iter()
                .all(|m| m["observed_slots"] == 38)
        );
    }
    #[test]
    fn retained_state_waits_for_the_exact_description_generation() {
        let mut audit = Audit::default();
        let state =
            json!({"module":"fixture","point":"fixture:device/value","generation":"new","value":0});
        audit.apply(
            "moli-probe/module/fixture/state/value",
            &serde_json::to_vec(&state).unwrap(),
        );
        assert!(audit.current.is_empty());
        audit.apply(
            "moli-probe/module/fixture/config",
            br#"{"module":"fixture","generation":"old"}"#,
        );
        assert!(audit.current.is_empty());
        audit.apply(
            "moli-probe/module/fixture/config",
            br#"{"module":"fixture","generation":"new"}"#,
        );
        assert_eq!(audit.current["fixture:device/value"], 0);
        audit.apply(
            "moli-probe/module/fixture/availability",
            br#"{"module":"fixture","generation":"old","online":false}"#,
        );
        assert!(audit.availability.is_empty());
        audit.apply(
            "moli-probe/module/fixture/availability",
            br#"{"module":"fixture","generation":"new","online":true}"#,
        );
        assert!(audit.availability["fixture"]);
    }
}
