//! Real MQTT -> HA driver -> Gateway -> Hub -> Overkiz, with a synthetic box.
use crate::{Api, fake::Fake};
use moli_core::{Action, DeviceId, InstanceId, Origin, PointId};
use moli_home_assistant::{
    Config, Gateway, HomeAssistant,
    inventory::{Catalog, HaChannel, HaDevice, HaKind},
    protocol::{
        Availability, CoverAction, CoverState, Exports, NativeCommand, ProofMessage, ResultCode,
        ResultMessage,
    },
};
use moli_migration::{
    Binding, BindingKey, Direction, MigrationPlan, Owner, Phase, Proof, Registry,
};
use moli_runtime::{
    BoxFuture, Driver, DriverCtx, Hub, HubOptions, LabelPatch, MasterKey, guard::GuardPolicy,
    spawn_driver,
};
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use serde::Serialize;
use serde_json::{Value as Json, json};
use std::{
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    process::{Child, Command},
    sync::{Notify, Semaphore, mpsc},
    task::{JoinHandle, JoinSet},
    time::timeout,
};
use tokio_util::sync::CancellationToken;

const BASE: &str = "moli-probe/ha-bridge/test";
const USER: &str = "bridge-fixture-user";
const PASSWORD: &str = "bridge-fixture-password";
const INSTALLATION: &str = "ha-test";
const HA_SESSION: &str = "ha-fixture-session";
const ENTITY: &str = "ha-cover-a";
const DEVICE: &str = "rts://1234-5678-9012/16700001";
const WAIT: Duration = Duration::from_secs(5);

struct Test(Arc<Fake>);
impl Driver for Test {
    fn kind(&self) -> &'static str {
        "overkiz"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(super::drive(
            Api::Fake(self.0.clone()),
            ctx,
            (Duration::from_millis(20), Duration::from_millis(20)),
            "",
        ))
    }
}

#[derive(Default)]
struct Tasks {
    cancel: CancellationToken,
    handles: Vec<Option<JoinHandle<()>>>,
    children: Vec<Child>,
}
impl Tasks {
    fn add(&mut self, handle: JoinHandle<()>) -> usize {
        self.handles.push(Some(handle));
        self.handles.len() - 1
    }
    async fn reap(&mut self, index: usize) {
        if let Some(mut task) = self.handles[index].take()
            && timeout(WAIT, &mut task).await.is_err()
        {
            task.abort();
            let _ = task.await;
        }
    }
    async fn stop(&mut self) {
        self.cancel.cancel();
        for child in &mut self.children {
            if child.try_wait().unwrap().is_none() {
                child.start_kill().unwrap();
            }
            timeout(WAIT, child.wait()).await.unwrap().unwrap();
        }
        for index in 0..self.handles.len() {
            self.reap(index).await;
        }
    }
}
impl Drop for Tasks {
    fn drop(&mut self) {
        self.cancel.cancel();
        for child in &mut self.children {
            let _ = child.start_kill();
        }
        for task in self.handles.iter().flatten() {
            task.abort();
        }
    }
}

struct ConnectionHold {
    next: AtomicBool,
    entered: Notify,
    release: Semaphore,
}
impl Default for ConnectionHold {
    fn default() -> Self {
        Self {
            next: AtomicBool::new(false),
            entered: Notify::new(),
            release: Semaphore::new(0),
        }
    }
}

struct Peer {
    client: AsyncClient,
    inbox: mpsc::Receiver<(String, Vec<u8>, bool)>,
    states: Vec<CoverState>,
    exports: Option<Exports>,
}
impl Peer {
    async fn open(port: u16, tasks: &mut Tasks) -> Self {
        let mut options = MqttOptions::new("ha-fixture-peer", "127.0.0.1", port);
        options.set_credentials(USER, PASSWORD);
        options.set_clean_session(true);
        options.set_max_packet_size(1024 * 1024 + 8192, 1024 * 1024 + 8192);
        let (client, mut events) = AsyncClient::new(options, 16);
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let (sender, inbox) = mpsc::channel(128);
        tasks.add(tokio::spawn(async move {
            let mut ready = Some(ready_tx);
            while let Ok(event) = events.poll().await {
                match event {
                    Event::Incoming(Packet::SubAck(ack)) => {
                        assert!(
                            ack.return_codes
                                .iter()
                                .all(|code| !matches!(code, rumqttc::SubscribeReasonCode::Failure))
                        );
                        if let Some(ready) = ready.take() {
                            let _ = ready.send(());
                        }
                    }
                    Event::Incoming(Packet::Publish(message)) => {
                        assert!(message.payload.len() <= 1024 * 1024);
                        let suffix = message
                            .topic
                            .strip_prefix(&format!("{BASE}/"))
                            .unwrap()
                            .to_owned();
                        if sender
                            .send((suffix, message.payload.to_vec(), message.retain))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }));
        client
            .subscribe(format!("{BASE}/moli/#"), QoS::AtLeastOnce)
            .await
            .unwrap();
        timeout(WAIT, ready_rx).await.unwrap().unwrap();
        Self {
            client,
            inbox,
            states: vec![],
            exports: None,
        }
    }
    async fn publish<T: Serialize>(&self, suffix: &str, body: &T, retained: bool) {
        self.client
            .publish(
                format!("{BASE}/{suffix}"),
                QoS::AtLeastOnce,
                retained,
                serde_json::to_vec(body).unwrap(),
            )
            .await
            .unwrap();
    }
    async fn frame(&mut self, wanted: &str) -> (Vec<u8>, bool) {
        timeout(WAIT, async {
            loop {
                let (suffix, bytes, retained) = self.inbox.recv().await.expect("peer pump alive");
                if suffix.starts_with("moli/state/") && !bytes.is_empty() {
                    self.states.push(serde_json::from_slice(&bytes).unwrap());
                }
                if suffix == "moli/exports" && !bytes.is_empty() {
                    self.exports = Some(serde_json::from_slice(&bytes).unwrap());
                }
                if suffix == wanted || (wanted.ends_with('/') && suffix.starts_with(wanted)) {
                    return (bytes, retained);
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("missing fixture MQTT frame {wanted}"))
    }
    async fn result(&mut self, id: &str) -> ResultCode {
        let (bytes, retained) = self.frame(&format!("moli/result/{id}")).await;
        assert!(!retained, "command results must never be retained");
        let result: ResultMessage = serde_json::from_slice(&bytes).unwrap();
        result.validate(INSTALLATION).unwrap();
        assert_eq!(result.request_id, id);
        result.code
    }

    async fn current_exports(&mut self, binding: &Binding) -> Exports {
        timeout(WAIT, async {
            loop {
                if let Some(exports) = &self.exports
                    && exports.items.iter().any(|item| {
                        item.binding == binding.key
                            && item.revision == binding.revision
                            && item.generation == binding.generation
                    })
                {
                    return exports.clone();
                }
                let (bytes, _) = self.frame("moli/exports").await;
                let exports: Exports = serde_json::from_slice(&bytes).unwrap();
                if exports.items.iter().any(|item| {
                    item.binding == binding.key
                        && item.revision == binding.revision
                        && item.generation == binding.generation
                }) {
                    return exports;
                }
            }
        })
        .await
        .expect("current exports observed over MQTT")
    }

    async fn current_state(&mut self, binding: &Binding) -> CoverState {
        timeout(WAIT, async {
            loop {
                if let Some(state) = self.states.iter().rev().find(|state| {
                    state.binding == binding.key
                        && state.revision == binding.revision
                        && state.generation == binding.generation
                }) {
                    return state.clone();
                }
                let (bytes, _) = self.frame("moli/state/").await;
                if bytes.is_empty() {
                    continue;
                }
                let state: CoverState = serde_json::from_slice(&bytes).unwrap();
                if state.binding == binding.key
                    && state.revision == binding.revision
                    && state.generation == binding.generation
                {
                    return state;
                }
            }
        })
        .await
        .expect("current state observed over MQTT")
    }
}

struct Rig {
    _directory: tempfile::TempDir,
    tasks: Tasks,
    hub: Hub,
    registry: Registry,
    gateway: Gateway,
    fake: Arc<Fake>,
    peer: Peer,
    config: Config,
    ha_cancel: CancellationToken,
    ha_task: usize,
    hold: Arc<ConnectionHold>,
}

fn native() -> DeviceId {
    DeviceId::new(&InstanceId::from("volets"), DEVICE)
}
fn key() -> BindingKey {
    BindingKey {
        ha_installation: INSTALLATION.into(),
        registry_entity_id: ENTITY.into(),
        target: PointId::new(&native(), "control"),
        writer_scope: "fixture-cover-writer".into(),
    }
}
fn catalog() -> Catalog {
    Catalog {
        v: 1,
        installation: INSTALLATION.into(),
        revision: 1,
        devices: vec![HaDevice {
            id: "ha-device-a".into(),
            name: "Synthetic HA cover".into(),
            channels: vec![HaChannel {
                id: "ha-channel-a".into(),
                entity_id: "cover.fixture".into(),
                registry_entity_id: ENTITY.into(),
                name: "Synthetic cover".into(),
                unit: None,
                semantic: None,
                kind: HaKind::Text,
                readable: false,
                actions: vec![
                    CoverAction::Open,
                    CoverAction::Stop,
                    CoverAction::Close,
                    CoverAction::My,
                ],
            }],
        }],
    }
}
fn availability() -> Availability {
    Availability {
        v: 1,
        installation: INSTALLATION.into(),
        session: HA_SESSION.into(),
        online: true,
    }
}

async fn until(mut condition: impl FnMut() -> bool) {
    timeout(WAIT, async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("fixture condition deadline");
}

async fn node_broker(tasks: &mut Tasks) -> u16 {
    let node = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".local/bin/node"))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from("node"));
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-fixtures/broker.cjs");
    let mut command = Command::new(node);
    command
        .arg(fixture)
        .env_clear()
        .env("LANG", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let index = tasks.children.len();
    tasks
        .children
        .push(command.spawn().expect("installed Node fixture runtime"));
    let stderr = tasks.children[index].stderr.take().unwrap();
    tasks.add(tokio::spawn(async move {
        let mut stderr = stderr;
        let _ = tokio::io::copy(&mut stderr, &mut tokio::io::sink()).await;
    }));
    let bytes =
        serde_json::to_vec(&json!({"prefix":BASE,"username":USER,"password":PASSWORD})).unwrap();
    let mut input = tasks.children[index].stdin.take().unwrap();
    timeout(WAIT, input.write_all(&bytes))
        .await
        .unwrap()
        .unwrap();
    drop(input);
    let stdout = tasks.children[index].stdout.take().unwrap();
    let mut reader = BufReader::new(stdout.take(256));
    let mut line = String::new();
    timeout(WAIT, reader.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    let port = line.trim().parse::<u16>().expect("fixture broker port");
    assert!(port > 1024 && ![1414, 1415, 1416, 1883, 1884, 8790].contains(&port));
    port
}

async fn broker_fixture(tasks: &mut Tasks, held: Arc<ConnectionHold>) -> u16 {
    let upstream = node_broker(tasks).await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let stop = tasks.cancel.clone();
    tasks.add(tokio::spawn(async move {
        let mut connections = JoinSet::new();
        loop {
            tokio::select! {
                () = stop.cancelled() => break,
                Some(_) = connections.join_next(), if !connections.is_empty() => {},
                accepted = listener.accept() => {
                    let (mut stream, peer) = accepted.unwrap();
                    assert!(peer.ip().is_loopback());
                    assert!(connections.len() < 16);
                    stream.set_nodelay(true).unwrap();
                    let held = held.clone();
                    let wait = held.next.swap(false, Ordering::AcqRel);
                    connections.spawn(async move {
                        if wait { held.entered.notify_one(); held.release.acquire().await.unwrap().forget(); }
                        let mut broker = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, upstream)).await.unwrap();
                        broker.set_nodelay(true).unwrap();
                        // Fully transparent: CONNECT (including will), subscriptions
                        // and application payloads are copied without modification.
                        let _ = tokio::io::copy_bidirectional(&mut stream, &mut broker).await;
                    });
                },
            }
        }
        connections.abort_all();
        while connections.join_next().await.is_some() {}
    }));
    port
}

fn synthetic_box() -> Arc<Fake> {
    let fixtures: Json = serde_json::from_str(include_str!(
        "../../../integrations/overkiz/fixtures/setup-devices.json"
    ))
    .unwrap();
    let device = fixtures
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["deviceURL"] == DEVICE)
        .unwrap()
        .clone();
    Arc::new(Fake {
        setup: json!([device]),
        ends: Some(("COMPLETED", None)),
        ..Fake::default()
    })
}

impl Rig {
    async fn new(protected: bool) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let mut tasks = Tasks::default();
        let hold = Arc::new(ConnectionHold::default());
        let port = broker_fixture(&mut tasks, hold.clone()).await;
        let hub = Hub::new(HubOptions {
            secrets: Some((
                directory.path().join("vault.enc"),
                MasterKey::new("11".repeat(32)),
            )),
            guard: GuardPolicy {
                protected_rooms: vec!["Fixture protected".into()],
                ..GuardPolicy::default()
            },
            ..HubOptions::default()
        })
        .unwrap();
        hub.manage_write_instance(&InstanceId::from("volets"));
        let fake = synthetic_box();
        tasks.add(spawn_driver(
            &hub,
            "volets".into(),
            Arc::new(Test(fake.clone())),
            tasks.cancel.child_token(),
        ));
        until(|| {
            hub.device(&native())
                .is_some_and(|device| device.online == Some(true))
        })
        .await;
        if protected {
            hub.set_label(
                &native(),
                LabelPatch {
                    room: Some("Fixture protected".into()),
                    ..LabelPatch::default()
                },
                Origin::Ui,
                None,
            )
            .await
            .unwrap();
        }
        let registry = Registry::open(hub.clone(), directory.path().join("migration.json"))
            .await
            .unwrap();
        let config = Config {
            host: "127.0.0.1".into(),
            port,
            base_topic: BASE.into(),
            installation: INSTALLATION.into(),
            client_id: None,
            tls: false,
            native_instances: vec!["volets".into()],
        };
        let gateway = Gateway::new(&config, hub.clone(), registry.clone()).unwrap();
        let ha_instance = InstanceId::from("ha");
        hub.store_secret(&ha_instance, "mqtt_username", USER)
            .await
            .unwrap();
        hub.store_secret(&ha_instance, "mqtt_password", PASSWORD)
            .await
            .unwrap();
        let mut peer = Peer::open(port, &mut tasks).await;
        let ha_cancel = tasks.cancel.child_token();
        let ha_task = tasks.add(spawn_driver(
            &hub,
            ha_instance,
            Arc::new(HomeAssistant::with_gateway(config.clone(), gateway.clone()).unwrap()),
            ha_cancel.clone(),
        ));
        let (birth, _) = peer.frame("moli/availability").await;
        let birth: Availability = serde_json::from_slice(&birth).unwrap();
        assert_eq!(birth.session, gateway.session());
        peer.publish("ha/inventory", &catalog(), true).await;
        peer.publish("ha/availability", &availability(), false)
            .await;
        until(|| gateway.ha_session().as_deref() == Some(HA_SESSION)).await;
        timeout(WAIT, async {
            while gateway.snapshot().await["catalogue_revision"] != 1 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        Self {
            _directory: directory,
            tasks,
            hub,
            registry,
            gateway,
            fake,
            peer,
            config,
            ha_cancel,
            ha_task,
            hold,
        }
    }

    async fn phase(&self, phase: Phase) -> Binding {
        timeout(WAIT, async {
            loop {
                if let Some(binding) = self.registry.get(&key()).await
                    && binding.phase == phase
                {
                    return binding;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("migration proof applied through MQTT")
    }
    async fn prepare(&self, direction: Direction, revision: u64, operation: &str) -> Binding {
        let plan = MigrationPlan {
            version: 1,
            binding: key(),
            native_device: native(),
            direction,
            automation_provenance: None,
        };
        let hash = plan.hash().unwrap();
        self.gateway
            .prepare(plan, revision, operation, &hash)
            .await
            .unwrap()
    }
    async fn prove(&mut self, binding: &Binding, owner: Owner, expected: Phase) -> Binding {
        let message = ProofMessage {
            v: 1,
            installation: INSTALLATION.into(),
            session: self.gateway.session(),
            binding: key(),
            proof: Proof {
                operation: binding.operation_id.clone(),
                plan_hash: binding.plan_hash.clone(),
                writer_scope: key().writer_scope,
                revision: binding.revision,
                observed_owner: owner,
                quiescent: true,
            },
        };
        self.peer
            .publish("ha/migration/proof", &message, false)
            .await;
        self.phase(expected).await
    }
    async fn grant(&mut self, revision: u64, operation: &str) -> Binding {
        let prepared = self.prepare(Direction::ToMoli, revision, operation).await;
        let stopped = self.prove(&prepared, Owner::None, Phase::HaStopped).await;
        self.prove(&stopped, Owner::Moli, Phase::Moli).await
    }
    fn command(&self, binding: &Binding, id: &str, action: CoverAction) -> NativeCommand {
        NativeCommand {
            v: 1,
            installation: INSTALLATION.into(),
            session: self.gateway.session(),
            request_id: id.into(),
            binding: key(),
            revision: binding.revision,
            generation: binding.generation,
            expires_at_ms: moli_core::now_ms() + 5000,
            action,
        }
    }
    async fn command_result(&mut self, command: &NativeCommand) -> ResultCode {
        self.peer.publish("moli/command", command, false).await;
        self.peer.result(&command.request_id).await
    }
    fn arrivals(&self) -> Vec<String> {
        self.fake
            .arrivals
            .lock()
            .unwrap()
            .iter()
            .map(|(command, _)| command.clone())
            .collect()
    }
    async fn stop(&mut self) {
        self.tasks.stop().await;
    }
}

#[tokio::test]
async fn mqtt_handoff_orders_and_epoch_cycles_reach_the_real_overkiz_loop_once() {
    let mut rig = Rig::new(false).await;
    let prepared = rig.prepare(Direction::ToMoli, 0, "prepare-first").await;
    let denied = rig.command(&prepared, "before-ownership", CoverAction::Open);
    assert_eq!(rig.command_result(&denied).await, ResultCode::Rejected);
    assert!(rig.arrivals().is_empty());
    let stopped = rig.prove(&prepared, Owner::None, Phase::HaStopped).await;
    let binding = rig.prove(&stopped, Owner::Moli, Phase::Moli).await;
    let mut first = None;
    for (id, action) in [
        ("open-one", CoverAction::Open),
        ("stop-one", CoverAction::Stop),
        ("close-one", CoverAction::Close),
    ] {
        let command = rig.command(&binding, id, action);
        assert_eq!(rig.command_result(&command).await, ResultCode::Accepted);
        if first.is_none() {
            first = Some(command);
        }
    }
    assert_eq!(rig.arrivals(), ["open", "stop", "close"]);
    assert_eq!(
        rig.command_result(first.as_ref().unwrap()).await,
        ResultCode::Rejected
    );
    let returning = rig
        .prepare(Direction::ToHa, binding.revision, "return-to-ha")
        .await;
    let stopped = rig.prove(&returning, Owner::None, Phase::MoliStopped).await;
    let owned_ha = rig.prove(&stopped, Owner::Ha, Phase::Ha).await;
    let newest = rig.grant(owned_ha.revision, "back-to-moli").await;
    assert!(newest.generation > binding.generation);
    let mut stale = rig.command(&newest, "unseen-old-generation", CoverAction::My);
    stale.generation = binding.generation;
    assert_eq!(rig.command_result(&stale).await, ResultCode::Rejected);
    assert_eq!(
        rig.command_result(first.as_ref().unwrap()).await,
        ResultCode::Rejected
    );
    let fresh = rig.command(&newest, "fresh-favorite", CoverAction::My);
    assert_eq!(rig.command_result(&fresh).await, ResultCode::Accepted);
    assert_eq!(rig.arrivals(), ["open", "stop", "close", "my"]);
    let exports = rig.peer.current_exports(&newest).await;
    assert!(exports.items.iter().all(|item| !item.feedback));
    let feedback = rig.peer.current_state(&newest).await;
    assert!(!feedback.feedback && feedback.position.is_none() && feedback.closed.is_none());
    let states = rig.gateway.states().await.unwrap();
    assert!(
        states
            .iter()
            .all(|state| !state.feedback && state.position.is_none() && state.closed.is_none())
    );
    let (device, _) = rig.hub.describe(&native()).unwrap();
    assert!(device.point("position").is_none() && device.point("opening").is_none());
    let commands: Vec<_> = rig.hub.journal(100).into_iter().filter(|entry| matches!(&entry.action, Action::Command { point, .. } if point == &key().target)).collect();
    assert_eq!(commands.len(), 4);
    assert!(commands.iter().all(|entry| entry.origin == Origin::Api));
    assert!(
        rig.peer
            .states
            .iter()
            .all(|state| !state.feedback && state.position.is_none() && state.closed.is_none())
    );
    rig.stop().await;
}

#[tokio::test]
async fn mqtt_guard_refuses_protected_cover_origin_spoof_expiry_and_duplicates() {
    let mut rig = Rig::new(true).await;
    let binding = rig.grant(0, "protected-transfer").await;
    let guarded = rig.command(&binding, "guarded-api", CoverAction::Open);
    assert_eq!(
        rig.command_result(&guarded).await,
        ResultCode::ApprovalRequired
    );
    let mut spoofed =
        serde_json::to_value(rig.command(&binding, "spoofed-origin", CoverAction::Close)).unwrap();
    spoofed["origin"] = json!("ui");
    rig.peer.publish("moli/command", &spoofed, false).await;
    assert_eq!(
        rig.peer.result("spoofed-origin").await,
        ResultCode::Rejected
    );
    let mut expired = rig.command(&binding, "expired-request", CoverAction::My);
    expired.expires_at_ms = 0;
    assert_eq!(rig.command_result(&expired).await, ResultCode::Rejected);
    assert_eq!(rig.command_result(&guarded).await, ResultCode::Rejected);
    assert!(rig.arrivals().is_empty());
    assert_eq!(rig.fake.applied.load(Ordering::SeqCst), 0);
    rig.stop().await;
}

#[tokio::test]
async fn mqtt_retained_command_with_current_nonce_is_rejected_before_a_live_command() {
    let mut rig = Rig::new(false).await;
    let binding = rig.grant(0, "retained-transfer").await;
    let old_session = rig.gateway.session();
    rig.ha_cancel.cancel();
    rig.tasks.reap(rig.ha_task).await;
    // The real driver drops its TCP stream without DISCONNECT: Aedes must
    // deliver its exact offline testament, not a test-normalized CONNECT.
    timeout(WAIT, async {
        loop {
            let (bytes, _) = rig.peer.frame("moli/availability").await;
            let availability: Availability = serde_json::from_slice(&bytes).unwrap();
            if availability.session == old_session && !availability.online {
                break;
            }
        }
    })
    .await
    .expect("real MQTT last will observed after consumer shutdown");
    rig.hold.next.store(true, Ordering::Release);
    rig.ha_cancel = rig.tasks.cancel.child_token();
    rig.ha_task = rig.tasks.add(spawn_driver(
        &rig.hub,
        "ha".into(),
        Arc::new(HomeAssistant::with_gateway(rig.config.clone(), rig.gateway.clone()).unwrap()),
        rig.ha_cancel.clone(),
    ));
    timeout(WAIT, rig.hold.entered.notified()).await.unwrap();
    // This fixture alone primes liveness directly while the consumer TCP stream
    // is held. The command still crosses MQTT with the exact current nonce and
    // active ownership, isolating retained rejection from offline/session denial.
    rig.gateway.peer_update(availability(), false).unwrap();
    let retained = rig.command(&binding, "retained-current-session", CoverAction::Open);
    rig.peer.publish("moli/command", &retained, true).await;
    // A same-client QoS1 subscription acknowledgement is an ordering barrier
    // proving the retained publication has reached the broker before release.
    rig.peer
        .client
        .subscribe(format!("{BASE}/barrier"), QoS::AtLeastOnce)
        .await
        .unwrap();
    // Read the broker's retained publication via a second subscription on the
    // fixture peer; it is observable only after storage has completed.
    rig.peer
        .client
        .subscribe(format!("{BASE}/moli/command"), QoS::AtLeastOnce)
        .await
        .unwrap();
    let (_, replayed) = rig.peer.frame("moli/command").await;
    if !replayed {
        let (_, replayed) = rig.peer.frame("moli/command").await;
        assert!(replayed);
    }
    rig.hold.release.add_permits(1);
    assert_eq!(
        rig.peer.result(&retained.request_id).await,
        ResultCode::Rejected
    );
    assert!(rig.arrivals().is_empty());
    rig.peer
        .publish("ha/availability", &availability(), false)
        .await;
    until(|| rig.gateway.ha_session().is_some()).await;
    let live = rig.command(&binding, "live-after-retained", CoverAction::Stop);
    assert_eq!(rig.command_result(&live).await, ResultCode::Accepted);
    assert_eq!(rig.arrivals(), ["stop"]);
    rig.stop().await;
}
