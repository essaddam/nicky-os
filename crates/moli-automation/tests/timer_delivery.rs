//! Revocation reaches a driver that took an order but has not emitted it.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use moli_automation::{Actor, Author, Automation, Automations, Config, RunStatus};
use moli_core::{Access, Device, Kind, Origin, PointSpec, Semantic, Value};
use moli_runtime::{BoxFuture, Driver, DriverCtx, Hub, HubOptions, spawn_driver};
use serde_json::{Value as Json, json};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

const HUMAN: Actor = Actor {
    human: true,
    author: Author::Human,
};

#[derive(Default)]
struct Sink {
    taken: Notify,
    release: Notify,
    done: Notify,
    emitted: AtomicBool,
    abandoned: AtomicBool,
}

impl Driver for Sink {
    fn kind(&self) -> &'static str {
        "simulated"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let id = ctx.device_id("target");
            let points = ["on", "announce", "notify", "push"]
                .into_iter()
                .map(|key| PointSpec {
                    key: key.into(),
                    label: key.into(),
                    kind: if key == "on" {
                        Kind::Binary
                    } else {
                        Kind::Text
                    },
                    unit: None,
                    access: Access {
                        read: key == "on",
                        write: true,
                    },
                    semantic: Semantic::Other,
                })
                .collect();
            ctx.upsert_device(Device {
                id: id.clone(),
                instance: ctx.instance().clone(),
                native_name: "Simulated target".into(),
                manufacturer: None,
                model: None,
                description: None,
                native_room: None,
                members: Vec::new(),
                points,
            });
            ctx.set_state(&id, "on", Value::Bool(false));
            ctx.ready();
            while let Some(command) = ctx.next_command().await {
                self.taken.notify_one();
                self.release.notified().await;
                let abandoned = command.abandoned();
                self.abandoned.store(abandoned, Ordering::SeqCst);
                if abandoned {
                    command.reply(Err("simulated cancellation".into()));
                } else {
                    assert_eq!(command.metadata.origin, Origin::Automation);
                    assert!(command.metadata.correlation.is_some());
                    self.emitted.store(true, Ordering::SeqCst);
                    command.reply(Ok(()));
                }
                self.done.notify_one();
            }
            Ok(())
        })
    }
}

struct Fixture {
    hub: Hub,
    autos: Automations,
    graph: Automation,
    sink: Arc<Sink>,
    cancel: CancellationToken,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}
impl Fixture {
    async fn new(effect: Json) -> Self {
        let hub = Hub::new(HubOptions::default()).unwrap();
        let cancel = CancellationToken::new();
        let sink = Arc::new(Sink::default());
        let config = serde_json::from_value(json!({"helper":[
            {"id":"clock","name":"Timer","kind":"timer"}
        ]}))
        .unwrap();
        let mut tasks = vec![
            spawn_driver(&hub, "sink".into(), sink.clone(), cancel.clone()),
            spawn_driver(
                &hub,
                "fixture".into(),
                Arc::new(moli_helpers::Helpers::new(config).unwrap()),
                cancel.clone(),
            ),
        ];
        tokio::time::timeout(Duration::from_secs(3), async {
            while hub.state(&"fixture:clock/state".into()).is_none()
                || hub.state(&"sink:target/on".into()).is_none()
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let (autos, engine) = Automations::start(&hub, Config::default(), None, cancel.clone())
            .await
            .unwrap();
        tasks.push(engine);
        let draft = serde_json::from_value(json!({"name":"Simulated expiry","mode":"queued","graph":{
            "nodes":[{"id":"expired","type":"when_occurrence","point":"fixture:clock/finished"}, effect],
            "edges":[{"from":"expired","to":"send"}]
        }})).unwrap();
        let draft = autos.save(draft, HUMAN).await.unwrap();
        let graph = autos
            .approve(&draft.id, &draft.fingerprint(), HUMAN)
            .await
            .unwrap();
        let h = hub.clone();
        let c = cancel.clone();
        tasks.push(tokio::spawn(async move {
            h.run_timers(c).await;
        }));
        hub.command(
            &"fixture:clock/start".into(),
            Value::Int(1),
            Origin::Ui,
            None,
        )
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(3), sink.taken.notified())
            .await
            .unwrap();
        Self {
            hub,
            autos,
            graph,
            sink,
            cancel,
            tasks,
        }
    }

    async fn revoke(&self, action: &str) {
        match action {
            "cancel" => self
                .hub
                .command(
                    &"fixture:clock/cancel".into(),
                    Value::Null,
                    Origin::Ui,
                    None,
                )
                .await
                .unwrap(),
            "replace" => self
                .hub
                .command(
                    &"fixture:clock/start".into(),
                    Value::Int(100),
                    Origin::Ui,
                    None,
                )
                .await
                .unwrap(),
            "off" => {
                self.autos
                    .set_enabled(&self.graph.id, false, None, HUMAN)
                    .await
                    .unwrap();
            }
            "control" | "already_sent" => {}
            _ => panic!("unknown test action"),
        }
    }
    async fn release(&self) {
        self.sink.release.notify_one();
        tokio::time::timeout(Duration::from_secs(3), self.sink.done.notified())
            .await
            .unwrap();
    }
    async fn stop(self) {
        self.cancel.cancel();
        for task in self.tasks {
            tokio::time::timeout(Duration::from_secs(3), task)
                .await
                .unwrap()
                .unwrap();
        }
    }
}

#[tokio::test]
async fn auto_h1_r1_taken_but_unsent_orders_obey_generation_and_activation() {
    let effects = [
        json!({"id":"send","type":"set","point":"sink:target/on","value":true}),
        json!({"id":"send","type":"toggle","point":"sink:target/on"}),
        json!({"id":"send","type":"notify","message":"Simulated message","channels":["voix"]}),
        json!({"id":"send","type":"notify","message":"Simulated message","channels":["telegram"]}),
        json!({"id":"send","type":"notify","message":"Simulated message","channels":["telephone"]}),
    ];
    for effect in effects {
        for action in ["control", "cancel", "replace", "off", "already_sent"] {
            let fixture = Fixture::new(effect.clone()).await;
            if action == "already_sent" {
                fixture.release().await;
                fixture.revoke("cancel").await;
            } else {
                fixture.revoke(action).await;
                fixture.release().await;
            }
            let should_emit = matches!(action, "control" | "already_sent");
            assert_eq!(
                fixture.sink.emitted.load(Ordering::SeqCst),
                should_emit,
                "{action}: {effect}"
            );
            assert_eq!(fixture.sink.abandoned.load(Ordering::SeqCst), !should_emit);
            if !should_emit {
                tokio::time::timeout(Duration::from_secs(3), async {
                    while !fixture
                        .autos
                        .runs(None, 10)
                        .iter()
                        .any(|run| run.status == RunStatus::Cancelled)
                    {
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();
            }
            fixture.stop().await;
        }
    }
}
