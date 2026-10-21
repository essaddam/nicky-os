use moli_automation::{Actor, Author, Automations, Config, Graph};
use moli_core::{Origin, Value};
use moli_runtime::{Hub, HubOptions, spawn_driver};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
#[tokio::test]
async fn auto_h3_graphs_validate_and_compare_the_same_instant_without_implicit_triggers() {
    let hub = Hub::new(HubOptions::default()).unwrap();
    let cancel = CancellationToken::new();
    let config=serde_json::from_value(json!({"helper":[{"id":"date","name":"Synthetic instant","kind":"datetime","timezone":"Europe/Paris","initial":"2000-01-01T00:00:00Z"},{"id":"result","name":"Result","kind":"counter"}]})).unwrap();
    let driver = spawn_driver(
        &hub,
        "fixture".into(),
        Arc::new(moli_helpers::Helpers::new(config).unwrap()),
        cancel.clone(),
    );
    tokio::time::timeout(Duration::from_secs(3), async {
        while hub.state(&"fixture:result/value".into()).is_none() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let (autos, engine) = Automations::start(&hub, Config::default(), None, cancel.clone())
        .await
        .unwrap();
    for node in [
        json!({"id":"bad","type":"set","point":"fixture:date/value","value":"2026-03-29T02:30:00"}),
        json!({"id":"bad","type":"when_state","point":"fixture:date/value","to":"2026-10-25T02:30:00"}),
        json!({"id":"bad","type":"if","rules":[{"kind":"state","point":"fixture:date/value","value":"2026-03-29T02:30:00"}]}),
    ] {
        let graph: Graph = serde_json::from_value(json!({"nodes":[node],"edges":[]})).unwrap();
        assert!(!autos.check(&graph).ok());
    }
    let graph=serde_json::from_value(json!({"name":"Synthetic condition","graph":{"nodes":[{"id":"change","type":"when_state","point":"fixture:date/value","to":"2026-10-25T02:30:00+01:00"},{"id":"check","type":"if","rules":[{"kind":"state","point":"fixture:date/value","value":"2026-10-25T01:30:00Z"}]},{"id":"count","type":"set","point":"fixture:result/increment","value":null}],"edges":[{"from":"change","to":"check"},{"from":"check","port":"yes","to":"count"}]}})).unwrap();
    let human = Actor {
        human: true,
        author: Author::Human,
    };
    let draft = autos
        .save(
            graph,
            Actor {
                human: false,
                author: Author::Agent,
            },
        )
        .await
        .unwrap();
    let point = "fixture:date/value".into();
    hub.command(
        &point,
        Value::Text("2026-10-25T01:30:00Z".into()),
        Origin::Ui,
        None,
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        hub.state(&"fixture:result/value".into()).unwrap().value,
        Value::Int(0)
    );
    hub.command(
        &point,
        Value::Text("2000-01-01T00:00:00Z".into()),
        Origin::Ui,
        None,
    )
    .await
    .unwrap();
    autos
        .approve(&draft.id, &draft.fingerprint(), human)
        .await
        .unwrap();
    hub.command(
        &point,
        Value::Text("2026-10-25T02:30:00+01:00".into()),
        Origin::Ui,
        None,
    )
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while hub.state(&"fixture:result/value".into()).unwrap().value != Value::Int(1) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    hub.command(
        &point,
        Value::Text("2026-10-25T01:30:00Z".into()),
        Origin::Ui,
        None,
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        hub.state(&"fixture:result/value".into()).unwrap().value,
        Value::Int(1)
    );
    cancel.cancel();
    driver.await.unwrap();
    engine.await.unwrap();
}
