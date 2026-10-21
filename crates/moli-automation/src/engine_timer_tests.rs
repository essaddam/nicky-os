use super::*;
use serde_json::json;
use std::sync::Arc;

const HUMAN: crate::Actor = crate::Actor {
    human: true,
    author: crate::Author::Human,
};

struct Fixture {
    hub: moli_runtime::Hub,
    autos: Automations,
    graph: crate::Automation,
    event: Occurrence,
    cancel: tokio_util::sync::CancellationToken,
    worker: tokio::task::JoinHandle<()>,
    driver: tokio::task::JoinHandle<()>,
}

impl Fixture {
    async fn new() -> Self {
        let hub = moli_runtime::Hub::new(moli_runtime::HubOptions::default()).unwrap();
        let cancel = tokio_util::sync::CancellationToken::new();
        let config = serde_json::from_value(json!({"helper":[
            {"id":"clock","name":"Timer","kind":"timer"},
            {"id":"result","name":"Result","kind":"counter"}
        ]}))
        .unwrap();
        let driver = moli_runtime::spawn_driver(
            &hub,
            "fixture".into(),
            Arc::new(moli_helpers::Helpers::new(config).unwrap()),
            cancel.clone(),
        );
        for _ in 0..500 {
            if hub.state(&"fixture:result/value".into()).is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let (autos, engine) =
            Automations::start(&hub, crate::Config::default(), None, cancel.clone())
                .await
                .unwrap();
        engine.abort();
        let _ = engine.await;
        let draft = serde_json::from_value(json!({"name":"Expiry","mode":"queued","graph":{
            "nodes":[{"id":"expired","type":"when_occurrence","point":"fixture:clock/finished"},
                {"id":"count","type":"set","point":"fixture:result/increment","value":null}],
            "edges":[{"from":"expired","to":"count"}]
        }}))
        .unwrap();
        let draft = autos.save(draft, HUMAN).await.unwrap();
        let graph = autos
            .approve(&draft.id, &draft.fingerprint(), HUMAN)
            .await
            .unwrap();
        let mut events = hub.subscribe();
        hub.timer_consumer_ready(true);
        let worker = {
            let hub = hub.clone();
            let cancel = cancel.clone();
            tokio::spawn(async move {
                hub.run_timers(cancel).await;
            })
        };
        hub.command(
            &"fixture:clock/start".into(),
            Value::Int(1),
            moli_core::Origin::Ui,
            None,
        )
        .await
        .unwrap();
        let event = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Event::Occurrence { occurrence } = events.recv().await.unwrap() {
                    break occurrence;
                }
            }
        })
        .await
        .unwrap();
        Self {
            hub,
            autos,
            graph,
            event,
            cancel,
            worker,
            driver,
        }
    }

    async fn stop(self) {
        self.cancel.cancel();
        self.worker.await.unwrap();
        self.driver.await.unwrap();
    }
}

#[tokio::test]
async fn auto_h1_cancel_or_replace_while_claim_is_saving_prevents_old_effects() {
    for replace in [false, true] {
        let fixture = Fixture::new().await;
        let Fixture {
            hub, autos, event, ..
        } = &fixture;
        let entered = Arc::new(tokio::sync::Notify::new());
        let resume = Arc::new(tokio::sync::Notify::new());
        {
            let mut consumer = autos.0.occurrence_consumer.lock().await;
            consumer.recover(&event.stream).await.unwrap();
            consumer.pause_next_save(entered.clone(), resume.clone());
        }
        let handling = {
            let autos = autos.clone();
            let event = event.clone();
            tokio::spawn(async move {
                on_occurrence(&autos, event).await;
            })
        };
        tokio::time::timeout(Duration::from_secs(3), entered.notified())
            .await
            .unwrap();
        let (point, value) = if replace {
            ("fixture:clock/start", Value::Int(5))
        } else {
            ("fixture:clock/cancel", Value::Null)
        };
        hub.command(&point.into(), value, moli_core::Origin::Ui, None)
            .await
            .unwrap();
        resume.notify_one();
        tokio::time::timeout(Duration::from_secs(3), handling)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            hub.state(&"fixture:result/value".into()).unwrap().value,
            Value::Int(0)
        );
        assert!(autos.runs(None, 20).is_empty());
        hub.ack_timer(event).await.unwrap();
        assert_eq!(
            hub.state(&"fixture:clock/generation".into()).unwrap().value,
            Value::Int(if replace { 2 } else { 1 })
        );
        fixture.stop().await;
    }
}

#[tokio::test]
async fn auto_h1_cancel_or_replace_revokes_a_run_already_waiting_in_the_queue() {
    for replace in [false, true] {
        let fixture = Fixture::new().await;
        let autos = &fixture.autos;
        let queue = Arc::new(tokio::sync::Mutex::new(()));
        let held = queue.clone().lock_owned().await;
        lock(&autos.0.queues).insert(fixture.graph.id.clone(), queue);
        on_occurrence(autos, fixture.event.clone()).await;
        assert!(lock(&autos.0.active).contains_key(&fixture.graph.id));
        assert_eq!(lock(&autos.0.waiting).get(&fixture.graph.id), Some(&1));
        let (point, value) = if replace {
            ("fixture:clock/start", Value::Int(5))
        } else {
            ("fixture:clock/cancel", Value::Null)
        };
        fixture
            .hub
            .command(&point.into(), value, moli_core::Origin::Ui, None)
            .await
            .unwrap();
        drop(held);
        tokio::time::timeout(Duration::from_secs(3), async {
            while !lock(&autos.0.active).is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            fixture
                .hub
                .state(&"fixture:result/value".into())
                .unwrap()
                .value,
            Value::Int(0)
        );
        assert!(
            autos.runs(None, 20).is_empty(),
            "cancelled before execution"
        );
        assert_eq!(lock(&autos.0.waiting).get(&fixture.graph.id), Some(&0));
        fixture.stop().await;
    }
}
