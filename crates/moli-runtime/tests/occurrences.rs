//! AUTO-H4: real DriverCtx -> Hub bus, entirely simulated.

use std::sync::Arc;
use std::time::Duration;

use moli_core::{Access, Device, InstanceId, Kind, Origin, PointId, PointSpec, Semantic, Value};
use moli_runtime::{BoxFuture, Driver, DriverCtx, Hub, HubOptions, spawn_driver};
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
struct Pulses;

impl Driver for Pulses {
    fn kind(&self) -> &'static str {
        "simulated-pulses"
    }

    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let id = ctx.device_id("remote");
            ctx.upsert_device(Device {
                id: id.clone(),
                instance: ctx.instance().clone(),
                native_name: "Simulated remote".into(),
                manufacturer: None,
                model: None,
                description: None,
                native_room: None,
                members: Vec::new(),
                points: vec![PointSpec {
                    key: "action".into(),
                    label: "Action".into(),
                    kind: Kind::Text,
                    access: Access {
                        read: true,
                        write: true,
                    },
                    semantic: Semantic::Other,
                    unit: None,
                }],
            });
            ctx.set_state(&id, "action", Value::from("tap"));
            ctx.set_availability(&id, true);
            ctx.ready();
            while let Some(command) = ctx.next_command().await {
                ctx.pulse_state(&id, "action", command.value.clone());
                command.reply(Ok(()));
            }
            Ok(())
        })
    }
}

#[tokio::test]
async fn auto_h4_equal_pulses_are_occurrences_but_initial_state_is_not() {
    let hub = Hub::new(HubOptions {
        occurrence_clock: || 42,
        ..HubOptions::default()
    })
    .unwrap();
    let cancel = CancellationToken::new();
    let mut events = hub.subscribe();
    spawn_driver(
        &hub,
        InstanceId::from("simulated"),
        Arc::new(Pulses),
        cancel.clone(),
    );
    let point = PointId::from("simulated:remote/action");
    for _ in 0..200 {
        if hub.state(&point).is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(hub.state(&point).is_some(), "simulated source started");
    while let Ok(event) = events.try_recv() {
        assert_ne!(
            serde_json::to_value(event).unwrap()["type"],
            "occurrence",
            "initialization never presses the button"
        );
    }
    for _ in 0..2 {
        hub.command(&point, Value::from("tap"), Origin::Ui, None)
            .await
            .unwrap();
    }
    let mut occurrences = Vec::new();
    while let Ok(event) = events.try_recv() {
        let event = serde_json::to_value(event).unwrap();
        if event["type"] == "occurrence" {
            occurrences.push(event["occurrence"].clone());
        }
    }
    cancel.cancel();
    assert_eq!(
        occurrences.len(),
        2,
        "each pulse uses the occurrence path, including equal values"
    );
    assert_ne!(occurrences[0]["id"], occurrences[1]["id"]);
    assert_eq!(occurrences[0]["value"], "tap");
    assert_eq!(occurrences[1]["value"], "tap");
    assert_eq!(occurrences[0]["ts"], 42);
    assert_eq!(occurrences[1]["ts"], 42);
}
