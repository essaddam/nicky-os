use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use moli_automation::{Actor, Author, Automation, Automations, Config};
use moli_core::{Event, Origin, Value};
use moli_runtime::{Hub, HubOptions, spawn_driver};
use serde_json::json;
use tokio_util::sync::CancellationToken;

const HUMAN: Actor = Actor {
    human: true,
    author: Author::Human,
};
static NEXT: AtomicU64 = AtomicU64::new(0);
fn before() -> u64 {
    1_000
}
fn after() -> u64 {
    10_000
}

fn directory() -> PathBuf {
    std::env::temp_dir().join(format!(
        "moli-timer-engine-{}-{}-{}",
        std::process::id(),
        moli_core::now_ms(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

struct Fixture {
    hub: Hub,
    autos: Automations,
    cancel: CancellationToken,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl Fixture {
    async fn new(path: &Path, clock: fn() -> u64, engine: bool, worker: bool) -> Self {
        std::fs::create_dir_all(path).unwrap();
        let hub = Hub::new(HubOptions {
            timer_state_path: Some(path.join("helper-timers.json")),
            timer_clock: clock,
            counter_state_path: Some(path.join("helper-counters.json")),
            ..Default::default()
        })
        .unwrap();
        let cancel = CancellationToken::new();
        let config = serde_json::from_value(json!({"helper": [
            {"id":"clock","name":"Timer","kind":"timer"},
            {"id":"result","name":"Result","kind":"counter"}
        ]}))
        .unwrap();
        let mut tasks = vec![spawn_driver(
            &hub,
            "fixture".into(),
            Arc::new(moli_helpers::Helpers::new(config).unwrap()),
            cancel.clone(),
        )];
        for _ in 0..500 {
            if hub.state(&"fixture:result/value".into()).is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(hub.state(&"fixture:clock/state".into()).is_some());
        let (autos, task) = Automations::start(&hub, Config::default(), Some(path), cancel.clone())
            .await
            .unwrap();
        if engine {
            tasks.push(task);
        } else {
            task.abort();
            let _ = task.await;
        }
        if worker {
            let h = hub.clone();
            let c = cancel.clone();
            tasks.push(tokio::spawn(async move {
                h.run_timers(c).await;
            }));
        }
        Self {
            hub,
            autos,
            cancel,
            tasks,
        }
    }

    async fn graph(&self, approve: bool) -> Automation {
        let graph: Automation =
            serde_json::from_value(json!({"name":"Simulated expiry","mode":"queued","graph":{
                "nodes":[{"id":"expired","type":"when_occurrence","point":"fixture:clock/finished"},
                    {"id":"count","type":"set","point":"fixture:result/increment","value":null}],
                "edges":[{"from":"expired","to":"count"}]
            }}))
            .unwrap();
        let draft = self.autos.save(graph, HUMAN).await.unwrap();
        if approve {
            self.autos
                .approve(&draft.id, &draft.fingerprint(), HUMAN)
                .await
                .unwrap()
        } else {
            draft
        }
    }

    async fn start(&self) {
        self.hub
            .command_with_request_id(
                &"fixture:clock/start".into(),
                Value::Int(1),
                Origin::Ui,
                None,
                Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:1".into()),
            )
            .await
            .unwrap();
    }

    fn count(&self) -> Value {
        self.hub
            .state(&"fixture:result/value".into())
            .unwrap()
            .value
    }

    async fn stop(self) {
        self.cancel.cancel();
        for task in self.tasks {
            tokio::time::timeout(Duration::from_secs(5), task)
                .await
                .unwrap()
                .unwrap();
        }
    }
}

async fn drained(path: &Path) {
    for _ in 0..500 {
        let state: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path.join("helper-timers.json")).unwrap())
                .unwrap();
        if state["values"]["fixture:clock"]["status"] == "finished"
            && state["values"]["fixture:clock"]["pending"] == false
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("timer outbox was not acknowledged");
}

#[tokio::test]
async fn auto_h1_expiry_recovers_for_the_same_durable_activation_only() {
    for (replace, publish_before_crash) in [(false, false), (true, false), (true, true)] {
        let path = directory();
        let first = Fixture::new(&path, before, false, publish_before_crash).await;
        let approved = first.graph(true).await;
        let activation = approved.activation.clone();
        let mut events = first.hub.subscribe();
        first.start().await;
        if publish_before_crash {
            first.hub.timer_consumer_ready(true);
            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    if matches!(events.recv().await.unwrap(), Event::Occurrence { .. }) {
                        break;
                    }
                }
            })
            .await
            .unwrap();
        }
        if replace {
            first
                .autos
                .set_enabled(&approved.id, false, None, HUMAN)
                .await
                .unwrap();
            let next = first
                .autos
                .approve(&approved.id, &approved.fingerprint(), HUMAN)
                .await
                .unwrap();
            assert_ne!(next.activation, activation);
        }
        first.stop().await;
        let fresh = Fixture::new(&path, after, true, true).await;
        if !replace {
            assert_eq!(
                fresh.autos.get(&approved.id).unwrap().activation,
                activation
            );
        }
        drained(&path).await;
        for _ in 0..200 {
            if fresh.count() == Value::Int(i64::from(!replace)) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(fresh.count(), Value::Int(i64::from(!replace)));
        fresh.stop().await;
        let last = Fixture::new(&path, after, true, true).await;
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(last.count(), Value::Int(i64::from(!replace)));
        last.stop().await;
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[tokio::test]
async fn auto_h1_draft_or_later_approval_does_not_inherit_an_old_generation() {
    let path = directory();
    let first = Fixture::new(&path, before, false, false).await;
    let draft = first.graph(false).await;
    first.start().await;
    first
        .autos
        .approve(&draft.id, &draft.fingerprint(), HUMAN)
        .await
        .unwrap();
    first.stop().await;
    let next = Fixture::new(&path, after, true, true).await;
    drained(&path).await;
    assert_eq!(next.count(), Value::Int(0));
    next.stop().await;
    std::fs::remove_dir_all(path).unwrap();
}

#[tokio::test]
async fn auto_h1_automation_backup_never_restores_a_revoked_timer_lineage() {
    let path = directory();
    let first = Fixture::new(&path, before, false, false).await;
    let active = first.graph(true).await;
    first.start().await;
    let restored = first.autos.restore(&active.id, HUMAN).await.unwrap();
    assert_ne!(restored.activation, active.activation);
    let backup = std::fs::read(path.join("automations.json.bak")).unwrap();
    let original: serde_json::Value = serde_json::from_slice(&backup).unwrap();
    assert_eq!(
        original["automations"][0]["activation"],
        active.activation.as_deref().unwrap()
    );
    first.stop().await;
    std::fs::write(path.join("automations.json"), b"damaged").unwrap();
    let fresh = Fixture::new(&path, after, true, true).await;
    assert_ne!(
        fresh.autos.get(&active.id).unwrap().activation,
        active.activation
    );
    drained(&path).await;
    assert_eq!(fresh.count(), Value::Int(0));
    fresh.stop().await;
    assert_eq!(
        std::fs::read(path.join("automations.json.bak")).unwrap(),
        backup
    );
    assert!(std::fs::read_dir(&path).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("automations.json.damaged-")
    }));
    std::fs::remove_dir_all(path).unwrap();
}
