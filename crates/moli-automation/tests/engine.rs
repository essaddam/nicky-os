//! The engine against a real hub and a fake driver: a switch and a lamp.

use std::sync::Arc;
use std::time::Duration;

use moli_automation::{
    Actor, Author, Automation, AutomationError, Automations, Config, Graph, Mode, RunStatus, Step,
    Writer,
};
use moli_core::{Access, Device, InstanceId, Kind, Origin, PointId, PointSpec, Semantic, Value};
use moli_runtime::{BoxFuture, Driver, DriverCtx, Hub, HubOptions, spawn_driver};
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
struct Fake;

fn onoff(key: &str) -> PointSpec {
    PointSpec {
        key: key.into(),
        label: key.into(),
        kind: Kind::Binary,
        access: Access {
            read: true,
            write: true,
        },
        unit: None,
        semantic: Semantic::OnOff,
    }
}

impl Driver for Fake {
    fn kind(&self) -> &'static str {
        "fake"
    }

    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            for name in ["switch", "lamp"] {
                let id = ctx.device_id(name);
                ctx.upsert_device(Device {
                    id: id.clone(),
                    instance: ctx.instance().clone(),
                    native_name: name.into(),
                    manufacturer: None,
                    model: None,
                    description: None,
                    native_room: None,
                    members: Vec::new(),
                    points: vec![onoff("on")],
                });
                ctx.set_state(&id, "on", Value::Bool(false));
            }
            // A speaker that can announce (write-only words).
            ctx.upsert_device(Device {
                id: ctx.device_id("speaker"),
                instance: ctx.instance().clone(),
                native_name: "speaker".into(),
                manufacturer: None,
                model: None,
                description: None,
                native_room: None,
                members: Vec::new(),
                points: vec![PointSpec {
                    key: "announce".into(),
                    label: "Annonce".into(),
                    kind: Kind::Text,
                    access: Access {
                        read: false,
                        write: true,
                    },
                    unit: None,
                    semantic: Semantic::Other,
                }],
            });
            ctx.ready();
            while let Some(cmd) = ctx.next_command().await {
                ctx.set_state(&cmd.device.id, &cmd.key, cmd.value.clone());
                cmd.reply(Ok(()));
            }
            Ok(())
        })
    }
}

async fn setup() -> (Hub, Automations, CancellationToken) {
    let hub = Hub::new(HubOptions {
        occurrence_clock: || 42,
        ..HubOptions::default()
    })
    .unwrap();
    let cancel = CancellationToken::new();
    spawn_driver(
        &hub,
        InstanceId::from("fake"),
        Arc::new(Fake),
        cancel.clone(),
    );
    for _ in 0..200 {
        if hub.state(&PointId::from("fake:lamp/on")).is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let (autos, _task) = Automations::start(&hub, Config::default(), None, cancel.clone())
        .await
        .unwrap();
    (hub, autos, cancel)
}

fn automation(graph: serde_json::Value) -> Automation {
    Automation {
        id: String::new(),
        name: "Test".into(),
        enabled: true,
        mode: Mode::Restart,
        graph: serde_json::from_value::<Graph>(graph).unwrap(),
        author: Author::Human,
        approved: None,
        approved_version: None,
        activation: None,
        note: None,
        created: 0,
        updated: 0,
    }
}

fn switch_lights_lamp() -> serde_json::Value {
    serde_json::json!({
        "nodes": [
            { "id": "t", "type": "when_state", "point": "fake:switch/on", "to": true },
            { "id": "s", "type": "set", "point": "fake:lamp/on", "value": true }
        ],
        "edges": [{ "from": "t", "to": "s" }]
    })
}

const HUMAN: Actor = Actor {
    human: true,
    author: Author::Human,
};
const AGENT: Actor = Actor {
    human: false,
    author: Author::Agent,
};

async fn lamp_turns(hub: &Hub, on: bool) -> bool {
    for _ in 0..100 {
        if hub.state(&PointId::from("fake:lamp/on")).map(|s| s.value) == Some(Value::Bool(on)) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

async fn flip_switch(hub: &Hub, on: bool) {
    hub.command(
        &PointId::from("fake:switch/on"),
        Value::Bool(on),
        Origin::Api,
        None,
    )
    .await
    .unwrap();
}

/// A person saves, looks, and approves what they saw.
async fn approved(autos: &Automations, graph: serde_json::Value) -> Automation {
    let a = autos.save(automation(graph), HUMAN).await.unwrap();
    assert!(!a.is_live(), "saving alone never approves");
    autos.approve(&a.id, &a.fingerprint(), HUMAN).await.unwrap()
}

async fn add_event_button(hub: &Hub, cancel: &CancellationToken) {
    let config = serde_json::from_value::<moli_helpers::Config>(serde_json::json!({
        "helper": [{ "id": "button", "name": "Simulated button", "kind": "button" }]
    }));
    assert!(
        config.is_ok(),
        "the helpers driver accepts an explicit event button: {config:?}"
    );
    let config = config.unwrap();
    spawn_driver(
        hub,
        InstanceId::from("helpers-h4"),
        Arc::new(moli_helpers::Helpers::new(config).unwrap()),
        cancel.clone(),
    );
    for _ in 0..200 {
        if hub
            .snapshot()
            .devices
            .iter()
            .any(|d| d.device.id.as_str() == "helpers-h4:button")
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("simulated button did not start");
}

fn button_graph() -> serde_json::Value {
    serde_json::json!({
        "nodes": [
            { "id": "t", "type": "when_occurrence", "point": "helpers-h4:button/press" },
            { "id": "s", "type": "set", "point": "fake:lamp/on", "value": true }
        ],
        "edges": [{ "from": "t", "to": "s" }]
    })
}

#[tokio::test]
async fn auto_h2_validated_graph_increments_counter_and_threshold_reads_its_state() {
    let (hub, autos, cancel) = setup().await;
    let config = serde_json::from_value::<moli_helpers::Config>(serde_json::json!({
        "helper": [{"id":"count", "name":"Counter", "kind":"counter"}]
    }))
    .unwrap();
    spawn_driver(
        &hub,
        "helpers-h2".into(),
        Arc::new(moli_helpers::Helpers::new(config).unwrap()),
        cancel.clone(),
    );
    let value_point = PointId::from("helpers-h2:count/value");
    for _ in 0..200 {
        if hub.state(&value_point).is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let draft = autos
        .save(
            automation(serde_json::json!({
                "nodes": [
                    {"id":"t", "type":"when_state", "point":"fake:switch/on", "to":true},
                    {"id":"s", "type":"set", "point":"helpers-h2:count/increment", "value":null}
                ], "edges":[{"from":"t", "to":"s"}]
            })),
            AGENT,
        )
        .await
        .unwrap();
    assert!(!draft.is_live());
    assert!(
        autos
            .run_now(&draft.id, false, Some(&draft.fingerprint()), HUMAN)
            .await
            .is_err()
    );
    assert_eq!(hub.state(&value_point).unwrap().value, Value::Int(0));
    autos
        .approve(&draft.id, &draft.fingerprint(), HUMAN)
        .await
        .unwrap();
    flip_switch(&hub, true).await;
    for _ in 0..200 {
        if hub.state(&value_point).unwrap().value == Value::Int(1) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(hub.state(&value_point).unwrap().value, Value::Int(1));
    approved(
        &autos,
        serde_json::json!({
            "nodes": [
                {"id":"t", "type":"when_threshold", "point":"helpers-h2:count/value", "above":1},
                {"id":"s", "type":"set", "point":"fake:lamp/on", "value":true}
            ], "edges":[{"from":"t", "to":"s"}]
        }),
    )
    .await;
    hub.command(
        &"helpers-h2:count/increment".into(),
        Value::Null,
        Origin::Ui,
        None,
    )
    .await
    .unwrap();
    assert!(lamp_turns(&hub, true).await);
    assert!(hub.journal(30).iter().any(|entry| entry.origin == Origin::Automation && matches!(&entry.action, moli_core::Action::Counter { point } if point.as_str() == "helpers-h2:count/increment")));
    cancel.cancel();
}

#[tokio::test]
async fn auto_h4_identical_presses_start_two_validated_runs() {
    let (hub, autos, cancel) = setup().await;
    add_event_button(&hub, &cancel).await;
    let point = PointId::from("helpers-h4:button/press");
    assert!(
        hub.state(&point).is_none(),
        "a button has no restored value"
    );
    let mut draft = automation(button_graph());
    draft.mode = Mode::Queued;
    let saved = autos.save(draft, HUMAN).await.unwrap();
    let live = autos
        .approve(&saved.id, &saved.fingerprint(), HUMAN)
        .await
        .unwrap();
    for _ in 0..2 {
        hub.command(&point, Value::Null, Origin::Ui, None)
            .await
            .unwrap();
    }
    for _ in 0..200 {
        let runs = autos.runs(Some(&live.id), 10);
        if runs.len() == 2 && runs.iter().all(|run| run.status == RunStatus::Done) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let runs = autos.runs(Some(&live.id), 10);
    assert_eq!(runs.len(), 2, "two equal appuis are two occurrences");
    assert!(runs.iter().all(|run| run.status == RunStatus::Done));
    let target = PointId::from("fake:lamp/on");
    assert_eq!(hub.journal(100).iter().filter(|entry| {
        entry.origin == Origin::Automation
            && matches!(&entry.action, moli_core::Action::Command { point, .. } if point == &target)
    }).count(), 2, "both executions passed through the guarded, journaled hub");
    cancel.cancel();
}

#[tokio::test]
async fn auto_h4_button_initialization_and_agent_draft_do_not_run() {
    let (hub, autos, cancel) = setup().await;
    add_event_button(&hub, &cancel).await;
    let draft = autos.save(automation(button_graph()), AGENT).await.unwrap();
    assert!(!draft.is_live());
    let point = PointId::from("helpers-h4:button/press");
    assert!(hub.state(&point).is_none());
    for _ in 0..2 {
        hub.command(&point, Value::Null, Origin::Ui, None)
            .await
            .unwrap();
    }
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert!(autos.runs(Some(&draft.id), 10).is_empty());
    cancel.cancel();
}

#[tokio::test]
async fn auto_h4_request_retry_runs_once_and_carries_correlation() {
    let (hub, autos, cancel) = setup().await;
    add_event_button(&hub, &cancel).await;
    let mut draft = automation(button_graph());
    draft.mode = Mode::Queued;
    let saved = autos.save(draft, HUMAN).await.unwrap();
    let live = autos
        .approve(&saved.id, &saved.fingerprint(), HUMAN)
        .await
        .unwrap();
    let point = PointId::from("helpers-h4:button/press");
    let request = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:1".to_owned();
    for _ in 0..2 {
        hub.command_with_request_id(&point, Value::Null, Origin::Ui, None, Some(request.clone()))
            .await
            .unwrap();
    }
    for _ in 0..200 {
        if autos
            .runs(Some(&live.id), 10)
            .iter()
            .any(|r| r.status == RunStatus::Done)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let runs = autos.runs(Some(&live.id), 10);
    assert_eq!(runs.len(), 1, "a retry cannot become a second run");
    assert_eq!(runs[0].status, RunStatus::Done);
    assert!(runs[0].occurrence.is_some());
    assert_eq!(runs[0].correlation.as_deref(), Some(request.as_str()));
    let actions: Vec<_> = hub
        .journal(100)
        .into_iter()
        .filter(|e| e.origin == Origin::Automation)
        .collect();
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].correlation.as_deref(), Some(request.as_str()));
    assert!(hub.state(&point).is_none());
    cancel.cancel();
}

#[tokio::test]
async fn auto_h4_guard_retains_two_distinct_presses_and_deduplicates_retry() {
    let hub = Hub::new(HubOptions {
        guard: moli_runtime::guard::GuardPolicy {
            protect_unassigned: true,
            ..Default::default()
        },
        ..Default::default()
    })
    .unwrap();
    let cancel = CancellationToken::new();
    add_event_button(&hub, &cancel).await;
    let point = PointId::from("helpers-h4:button/press");
    let mut events = hub.subscribe();
    let mut approvals = Vec::new();
    for seq in [1, 1, 2] {
        let result = hub
            .command_with_request_id(
                &point,
                Value::Null,
                Origin::Mcp,
                None,
                Some(format!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:{seq}")),
            )
            .await;
        let Err(moli_runtime::CommandError::NeedsApproval { id, .. }) = result else {
            panic!("guard must hold appuis")
        };
        approvals.push(id);
    }
    assert_eq!(approvals[0], approvals[1]);
    assert_ne!(approvals[0], approvals[2]);
    assert_eq!(hub.approvals().len(), 2);
    while let Ok(event) = events.try_recv() {
        assert!(
            !matches!(event, moli_core::Event::Occurrence { .. }),
            "held appui is not published"
        );
    }
    assert!(
        hub.resolve_approval(approvals[0], &point, true, Origin::Mcp, None)
            .await
            .is_err()
    );
    for id in [approvals[0], approvals[2]] {
        hub.resolve_approval(id, &point, true, Origin::Ui, None)
            .await
            .unwrap();
    }
    let mut count = 0;
    while let Ok(event) = events.try_recv() {
        count += usize::from(matches!(event, moli_core::Event::Occurrence { .. }));
    }
    assert_eq!(count, 2);
    cancel.cancel();
}

#[tokio::test]
async fn auto_h4_cached_button_state_and_restart_never_press() {
    let root = std::env::temp_dir().join(format!(
        "moli-h4-restore-{}-{}",
        std::process::id(),
        moli_core::now_ms()
    ));
    tokio::fs::create_dir_all(&root).await.unwrap();
    let cache = root.join("state.json");
    tokio::fs::write(
        &cache,
        br#"{"helpers-h4:button":{"press":{"value":true,"ts":1}}}"#,
    )
    .await
    .unwrap();
    let receipts = root.join("button-occurrences.json");
    let request = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:1".to_owned();
    for boot in 0..2 {
        let hub = Hub::new(HubOptions {
            state_path: Some(cache.clone()),
            button_state_path: Some(receipts.clone()),
            ..Default::default()
        })
        .unwrap();
        let cancel = CancellationToken::new();
        let mut events = hub.subscribe();
        add_event_button(&hub, &cancel).await;
        let point = PointId::from("helpers-h4:button/press");
        assert!(hub.state(&point).is_none());
        while let Ok(event) = events.try_recv() {
            assert!(!matches!(event, moli_core::Event::Occurrence { .. }));
        }
        hub.command_with_request_id(&point, Value::Null, Origin::Ui, None, Some(request.clone()))
            .await
            .unwrap();
        let mut count = 0;
        while let Ok(event) = events.try_recv() {
            count += usize::from(matches!(event, moli_core::Event::Occurrence { .. }));
        }
        assert_eq!(
            count,
            usize::from(boot == 0),
            "receipt survives restart without republishing"
        );
        hub.persist_state().await.unwrap();
        cancel.cancel();
    }
    tokio::fs::remove_dir_all(root).await.unwrap();
}

#[tokio::test]
async fn listed_fingerprints_follow_every_change() {
    let (_hub, autos, cancel) = setup().await;
    let listed = |autos: &Automations, id: &str| {
        autos
            .list_fingerprinted()
            .into_iter()
            .find(|(a, _)| a.id == id)
            .map(|(_, f)| f)
    };
    let a = approved(&autos, switch_lights_lamp()).await;
    assert_eq!(listed(&autos, &a.id), Some(a.fingerprint()));
    // Another version saved: the listing says so at once.
    let mut changed = a.clone();
    changed.graph.nodes[1].step = Step::Set {
        point: "fake:lamp/on".into(),
        value: serde_json::json!(false),
    };
    let changed = autos.save(changed, HUMAN).await.unwrap();
    assert_ne!(changed.fingerprint(), a.fingerprint());
    assert_eq!(listed(&autos, &a.id), Some(changed.fingerprint()));
    // Back to the approved version (an in-place change).
    let back = autos.restore(&a.id, HUMAN).await.unwrap();
    assert_eq!(listed(&autos, &a.id), Some(back.fingerprint()));
    assert_eq!(back.fingerprint(), a.fingerprint());
    autos.delete(&a.id, HUMAN).await.unwrap();
    assert_eq!(listed(&autos, &a.id), None);
    cancel.cancel();
}

#[tokio::test]
async fn an_approved_automation_acts() {
    let (hub, autos, _c) = setup().await;
    let a = approved(&autos, switch_lights_lamp()).await;
    assert!(a.is_live());
    assert_eq!(
        autos.check(&a.graph).summary,
        "Quand switch s'allume, allumer lamp."
    );

    flip_switch(&hub, true).await;
    assert!(lamp_turns(&hub, true).await, "the lamp follows the switch");
    tokio::time::sleep(Duration::from_millis(100)).await;
    let runs = autos.runs(Some(&a.id), 10);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, RunStatus::Done);
    let journal = hub.journal(5);
    assert!(
        journal.iter().any(|e| e.origin == Origin::Automation),
        "journaled as an automation"
    );
}

#[tokio::test]
async fn an_agents_draft_waits_for_a_person() {
    let (hub, autos, _c) = setup().await;
    let a = autos
        .save(automation(switch_lights_lamp()), AGENT)
        .await
        .unwrap();
    assert!(!a.is_live(), "an agent's automation is a draft");
    assert!(
        autos
            .set_enabled(&a.id, true, Some(&a.fingerprint()), AGENT)
            .await
            .is_err()
    );
    assert!(autos.approve(&a.id, &a.fingerprint(), AGENT).await.is_err());
    assert!(
        autos
            .run_now(&a.id, false, Some(&a.fingerprint()), AGENT)
            .await
            .is_err(),
        "agents only get dry runs"
    );

    flip_switch(&hub, true).await;
    assert!(
        !lamp_turns(&hub, true).await,
        "nothing happens before approval"
    );

    autos.approve(&a.id, &a.fingerprint(), HUMAN).await.unwrap();
    flip_switch(&hub, false).await;
    flip_switch(&hub, true).await;
    assert!(lamp_turns(&hub, true).await, "approved, it runs");
}

#[tokio::test]
async fn approving_what_you_did_not_see_is_refused() {
    let (_hub, autos, _c) = setup().await;
    let a = autos
        .save(automation(switch_lights_lamp()), AGENT)
        .await
        .unwrap();
    let seen = a.fingerprint();
    // Someone changes it while the person reads.
    let mut b = a.clone();
    b.graph.nodes[1].step = Step::Set {
        point: "fake:lamp/on".into(),
        value: serde_json::json!(false),
    };
    autos.save(b, AGENT).await.unwrap();
    assert!(matches!(
        autos.approve(&a.id, &seen, HUMAN).await,
        Err(AutomationError::Conflict)
    ));
    assert!(!autos.get(&a.id).unwrap().is_live());
}

#[tokio::test]
async fn an_agent_cannot_switch_it_back_on() {
    let (_hub, autos, _c) = setup().await;
    let a = approved(&autos, switch_lights_lamp()).await;
    autos.set_enabled(&a.id, false, None, AGENT).await.unwrap();
    let mut again = autos.get(&a.id).unwrap();
    again.enabled = true;
    let saved = autos.save(again, AGENT).await.unwrap();
    assert!(!saved.enabled, "an agent's save cannot switch on");
    assert!(
        autos
            .set_enabled(&a.id, true, Some(&a.fingerprint()), AGENT)
            .await
            .is_err()
    );
    assert!(!autos.get(&a.id).unwrap().is_live());
}

/// The name heads its messages (Telegram, notices) and the note is read
/// before trusting it: once a person approved it, an agent changes neither.
#[tokio::test]
async fn only_a_person_renames_an_approved_automation() {
    let (_hub, autos, _c) = setup().await;
    // A draft: whoever wrote it names it as they like.
    let draft = autos
        .save(automation(switch_lights_lamp()), AGENT)
        .await
        .unwrap();
    let mut renamed = draft.clone();
    renamed.name = "Entrée".into();
    let draft = autos.save(renamed, AGENT).await.unwrap();
    assert_eq!(draft.name, "Entrée");

    let a = autos
        .approve(&draft.id, &draft.fingerprint(), HUMAN)
        .await
        .unwrap();
    let edits: [fn(&mut Automation); 2] = [
        |it| it.name = "Rien à signaler".into(),
        |it| it.note = Some("Tout va bien.".into()),
    ];
    for edit in edits {
        let mut edited = a.clone();
        edit(&mut edited);
        match autos.save(edited, AGENT).await {
            Err(AutomationError::HumanOnly(why)) => {
                assert_eq!(why, moli_i18n::tr!("moteur.refus.nom_note"));
            }
            other => panic!("{other:?}"),
        }
        let kept = autos.get(&a.id).unwrap();
        assert_eq!((kept.name.as_str(), kept.note.as_deref()), ("Entrée", None));
        assert!(kept.is_live());
    }
    // A change of logic under the same name is still an agent's to make
    // (it loses the approval): only the name and the note are a person's.
    let mut other = a.clone();
    other.graph.nodes[1].step = Step::Set {
        point: "fake:lamp/on".into(),
        value: serde_json::json!(false),
    };
    assert!(!autos.save(other, AGENT).await.unwrap().is_live());
    // A person renames it as they like.
    let mut renamed = autos.restore(&a.id, HUMAN).await.unwrap();
    renamed.name = "Rien à signaler".into();
    let renamed = autos.save(renamed, HUMAN).await.unwrap();
    assert_eq!(renamed.name, "Rien à signaler");
    assert!(renamed.is_live());
}

#[tokio::test]
async fn a_real_run_needs_the_approved_version() {
    let (_hub, autos, _c) = setup().await;
    let a = autos
        .save(automation(switch_lights_lamp()), HUMAN)
        .await
        .unwrap();
    assert!(
        autos
            .run_now(&a.id, false, Some(&a.fingerprint()), HUMAN)
            .await
            .is_err(),
        "not approved yet"
    );
    let a = autos.approve(&a.id, &a.fingerprint(), HUMAN).await.unwrap();
    assert!(matches!(
        autos.run_now(&a.id, false, Some("other"), HUMAN).await,
        Err(AutomationError::Conflict)
    ));
    assert!(
        autos
            .run_now(&a.id, false, Some(&a.fingerprint()), HUMAN)
            .await
            .is_ok()
    );
    // Switched off, a real run would be cancelled at birth: refused, not « running ».
    autos.set_enabled(&a.id, false, None, HUMAN).await.unwrap();
    assert!(matches!(
        autos
            .run_now(&a.id, false, Some(&a.fingerprint()), HUMAN)
            .await,
        Err(AutomationError::Invalid(_))
    ));
}

#[tokio::test]
async fn a_dry_run_reads_but_never_acts() {
    let (hub, autos, _c) = setup().await;
    let a = autos
        .save(automation(switch_lights_lamp()), AGENT)
        .await
        .unwrap();
    let run = autos.run_now(&a.id, true, None, AGENT).await.unwrap();
    assert!(run.dry);
    assert_eq!(run.steps.len(), 1);
    assert!(!lamp_turns(&hub, true).await);
}

#[tokio::test]
async fn held_for_waits_and_gives_up_if_it_changes_back() {
    let (hub, autos, _c) = setup().await;
    let mut graph = switch_lights_lamp();
    graph["nodes"][0]["for_s"] = serde_json::json!(1);
    approved(&autos, graph).await;

    flip_switch(&hub, true).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    flip_switch(&hub, false).await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(
        hub.state(&PointId::from("fake:lamp/on")).unwrap().value,
        Value::Bool(false),
        "released too early"
    );

    flip_switch(&hub, true).await;
    assert!(lamp_turns(&hub, true).await, "held one second: fires");
}

#[tokio::test]
async fn switching_off_cancels_the_queue() {
    let (hub, autos, _c) = setup().await;
    let mut a = automation(serde_json::json!({
        "nodes": [
            { "id": "t", "type": "when_state", "point": "fake:switch/on" },
            { "id": "w", "type": "wait", "seconds": 1 },
            { "id": "s", "type": "set", "point": "fake:lamp/on", "value": true }
        ],
        "edges": [{ "from": "t", "to": "w" }, { "from": "w", "to": "s" }]
    }));
    a.mode = Mode::Queued;
    let a = autos.save(a, HUMAN).await.unwrap();
    let a = autos.approve(&a.id, &a.fingerprint(), HUMAN).await.unwrap();
    for on in [true, false, true] {
        flip_switch(&hub, on).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    autos.set_enabled(&a.id, false, None, AGENT).await.unwrap();
    tokio::time::sleep(Duration::from_millis(3500)).await;
    assert_eq!(
        hub.state(&PointId::from("fake:lamp/on")).unwrap().value,
        Value::Bool(false),
        "no queued run survives switching off"
    );
}

#[tokio::test]
async fn a_changed_graph_needs_a_new_approval() {
    let (_hub, autos, _c) = setup().await;
    let mut a = approved(&autos, switch_lights_lamp()).await;
    a.graph.nodes[1].step = Step::Set {
        point: "fake:lamp/on".into(),
        value: serde_json::json!(false),
    };
    let a = autos.save(a, AGENT).await.unwrap();
    assert!(!a.is_live(), "an agent's edit is not approved");
    let checked = autos.check(&a.graph);
    assert!(checked.ok(), "{:?}", checked.problems);
    assert!(a.approved_version.is_some(), "the approved version is kept");
    let back = autos.restore(&a.id, HUMAN).await.unwrap();
    assert!(back.is_live(), "back to the version the person approved");
}

#[tokio::test]
async fn an_agent_switching_it_off_ends_its_runs_and_is_journaled() {
    let (hub, autos, _c) = setup().await;
    let graph = serde_json::json!({
        "nodes": [
            { "id": "t", "type": "when_state", "point": "fake:switch/on", "to": true },
            { "id": "w", "type": "wait", "seconds": 600 },
            { "id": "s", "type": "set", "point": "fake:lamp/on", "value": true }
        ],
        "edges": [{ "from": "t", "to": "w" }, { "from": "w", "to": "s" }]
    });
    let a = autos.save(automation(graph), HUMAN).await.unwrap();
    let a = autos.approve(&a.id, &a.fingerprint(), HUMAN).await.unwrap();
    flip_switch(&hub, true).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        autos.runs(Some(&a.id), 1).is_empty(),
        "still waiting (finished runs only)"
    );

    // Through a plain save (PUT), not set_enabled: off must still mean off.
    let mut off = a.clone();
    off.enabled = false;
    let saved = autos.save(off, AGENT).await.unwrap();
    assert!(!saved.enabled);
    tokio::time::sleep(Duration::from_millis(200)).await;
    let runs = autos.runs(Some(&a.id), 1);
    assert_eq!(runs.len(), 1, "the waiting run ended");
    assert_eq!(runs[0].status, RunStatus::Cancelled);
    assert!(!lamp_turns(&hub, true).await, "and never acted");

    let changes: Vec<_> = hub
        .journal(20)
        .iter()
        .filter_map(|e| match &e.action {
            moli_core::Action::Automation { id, change, .. } if *id == a.id => {
                Some((e.origin, *change))
            }
            _ => None,
        })
        .collect();
    // Newest first, as the journal is read.
    assert_eq!(
        changes,
        vec![
            (Origin::Api, moli_core::AutomationChange::Updated),
            (Origin::Ui, moli_core::AutomationChange::Approved),
            (Origin::Ui, moli_core::AutomationChange::Created),
        ]
    );
}

/// A writer that keeps the prompts it is given, and never reads `on`.
#[derive(Debug, Default)]
struct Scribe(std::sync::Mutex<Vec<String>>);

impl Writer for Scribe {
    fn write(&self, prompt: String) -> BoxFuture<'_, anyhow::Result<String>> {
        self.0.lock().unwrap().push(prompt);
        Box::pin(async { Ok("Bonsoir".to_owned()) })
    }

    fn withholds(&self, key: &str) -> bool {
        key == "on"
    }
}

#[tokio::test]
async fn a_writer_never_reads_what_it_withholds() {
    let (_hub, autos, _c) = setup().await;
    let scribe = Arc::new(Scribe::default());
    autos.set_writer(scribe.clone());
    let a = approved(
        &autos,
        serde_json::json!({
            "nodes": [
                { "id": "t", "type": "manual" },
                { "id": "w", "type": "write", "prompt": "Lampe : {{fake:lamp/on}}. Il est {{heure}}." }
            ],
            "edges": [{ "from": "t", "to": "w" }]
        }),
    )
    .await;
    autos
        .run_now(&a.id, false, Some(&a.fingerprint()), HUMAN)
        .await
        .unwrap();
    for _ in 0..100 {
        if !autos.runs(Some(&a.id), 1).is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let prompts = scribe.0.lock().unwrap().clone();
    assert_eq!(prompts.len(), 1, "{prompts:?}");
    // Withheld: « ? », as an unknown value; the rest is filled in.
    assert!(prompts[0].starts_with("Lampe : ?. Il est "), "{prompts:?}");
    assert!(!prompts[0].contains("{{"), "{prompts:?}");
}

#[tokio::test]
async fn said_aloud_through_every_speaker() {
    let (hub, autos, _c) = setup().await;
    assert!(autos.abilities().voice, "the fake speaker can announce");
    let graph = serde_json::json!({
        "nodes": [
            { "id": "t", "type": "when_state", "point": "fake:switch/on", "to": true },
            { "id": "n", "type": "notify", "message": "Quelqu'un est à la porte", "channels": ["voix"] }
        ],
        "edges": [{ "from": "t", "to": "n" }]
    });
    let a = autos.save(automation(graph), HUMAN).await.unwrap();
    assert!(
        autos.check(&a.graph).summary.contains("à voix haute"),
        "{}",
        autos.check(&a.graph).summary
    );
    autos.approve(&a.id, &a.fingerprint(), HUMAN).await.unwrap();
    flip_switch(&hub, true).await;
    let said = PointId::from("fake:speaker/announce");
    for _ in 0..100 {
        if hub.state(&said).is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        hub.state(&said).map(|s| s.value),
        Some(Value::Text("Quelqu'un est à la porte".into()))
    );
}

async fn add_weekly_schedule(hub: &Hub, cancel: &CancellationToken) {
    let config = serde_json::from_value(serde_json::json!({"helper":[{"id":"weekly","name":"Weekly","kind":"schedule","weekly":{"timezone":"UTC","slots":[]}}]})).unwrap();
    spawn_driver(
        hub,
        InstanceId::from("fixture"),
        Arc::new(moli_helpers::Helpers::new(config).unwrap()),
        cancel.clone(),
    );
    for _ in 0..200 {
        if hub.state(&PointId::from("fixture:weekly/active")).is_some() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("simulated schedule did not initialize");
}

#[tokio::test]
async fn auto_h5_validated_graph_resolves_current_and_draft_cannot_trigger_it() {
    use moli_core::{DependencyKind, DependencyRef, WeeklySchedule, WeeklySlot};
    use moli_runtime::DependencyDefinition;
    let (hub, autos, cancel) = setup().await;
    let reference = DependencyRef {
        kind: DependencyKind::Schedule,
        id: "fixture:weekly".into(),
    };
    add_weekly_schedule(&hub, &cancel).await;
    let graph = serde_json::json!({"nodes":[{"id":"t","type":"when_state","point":"fixture:weekly/active","to":true},{"id":"s","type":"set","point":"fake:lamp/on","value":true}],"edges":[{"from":"t","to":"s"}]});
    let draft = autos.save(automation(graph), AGENT).await.unwrap();
    assert!(
        autos
            .approve(&draft.id, &draft.fingerprint(), HUMAN)
            .await
            .is_err(),
        "a dependency without current cannot be activated"
    );
    let shown = hub.preview_dependency(&reference).await.unwrap();
    assert_eq!(shown.consumers.len(), 1);
    assert_eq!(shown.consumers[0].id, draft.id);
    hub.publish_dependency(shown, true, Origin::Ui, None)
        .await
        .unwrap();
    let active = autos
        .approve(&draft.id, &draft.fingerprint(), HUMAN)
        .await
        .unwrap();
    let fingerprint = active.fingerprint();
    let on = WeeklySchedule {
        timezone: "UTC".into(),
        slots: (1..=7)
            .map(|day| WeeklySlot {
                day,
                start: 0,
                end: 1440,
            })
            .collect(),
    };
    hub.stage_dependency(
        reference.clone(),
        1,
        DependencyDefinition::Schedule(on),
        Origin::Mcp,
        None,
    )
    .await
    .unwrap();
    // A barrier through the engine's own clock lets pending metadata be consumed.
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(
        hub.state(&PointId::from("fake:lamp/on")).unwrap().value,
        Value::Bool(false)
    );
    assert_eq!(
        hub.state(&PointId::from("fixture:weekly/active"))
            .unwrap()
            .value,
        Value::Bool(false)
    );
    let shown = hub.preview_dependency(&reference).await.unwrap();
    hub.publish_dependency(shown, true, Origin::Ui, None)
        .await
        .unwrap();
    assert!(
        lamp_turns(&hub, true).await,
        "validated current publication is an explicit state change"
    );
    assert_eq!(
        autos.get(&draft.id).unwrap().fingerprint(),
        fingerprint,
        "dependency revisions are not pinned in graph fingerprint"
    );
    assert!(autos.get(&draft.id).unwrap().is_live());
    cancel.cancel();
}

#[tokio::test]
async fn auto_h5_graph_mutation_uses_publication_gate_and_invalidates_preview() {
    use moli_core::{DependencyKind, DependencyRef};
    let (hub, autos, cancel) = setup().await;
    let reference = DependencyRef {
        kind: DependencyKind::Schedule,
        id: "fixture:weekly".into(),
    };
    add_weekly_schedule(&hub, &cancel).await;
    let shown = hub.preview_dependency(&reference).await.unwrap();
    let held = hub.definition_gate().lock_owned().await;
    let a = automation(
        serde_json::json!({"nodes":[{"id":"t","type":"when_state","point":"fixture:weekly/active","to":true}],"edges":[]}),
    );
    let writer = autos.clone();
    let mut save = tokio::spawn(async move { writer.save(a, AGENT).await });
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut save)
            .await
            .is_err()
    );
    drop(held);
    save.await.unwrap().unwrap();
    assert_eq!(
        hub.publish_dependency(shown, true, Origin::Ui, None).await,
        Err(moli_runtime::DependencyError::Conflict)
    );
    assert_eq!(
        hub.preview_dependency(&reference)
            .await
            .unwrap()
            .consumers
            .len(),
        1
    );
    cancel.cancel();
}

#[tokio::test]
async fn auto_h5_preview_includes_template_consumers_and_never_trusts_late_bus_index() {
    use moli_core::{DependencyKind, DependencyRef};
    let (hub, autos, cancel) = setup().await;
    let reference = DependencyRef {
        kind: DependencyKind::Schedule,
        id: "fixture:weekly".into(),
    };
    let graph = automation(
        serde_json::json!({"nodes":[{"id":"t","type":"manual"},{"id":"n","type":"notify","message":"{{fixture:weekly/active}}","channels":["maison"]}],"edges":[{"from":"t","to":"n"}]}),
    );
    let draft = autos.save(graph, AGENT).await.unwrap();
    add_weekly_schedule(&hub, &cancel).await;
    hub.set_dependency_consumers(std::collections::BTreeMap::new());
    let shown = hub.preview_dependency(&reference).await.unwrap();
    assert_eq!(
        shown.consumers.len(),
        1,
        "the owner's fresh snapshot resolves newly registered points even with an empty bus index"
    );
    assert_eq!(shown.consumers[0].id, draft.id);
    assert!(!autos.check(&draft.graph).ok());
    cancel.cancel();
}

#[tokio::test]
async fn auto_h5_failed_graph_write_blocks_dependency_publication_until_saved_again() {
    let root = std::env::temp_dir().join(format!(
        "moli-h5-graph-save-{}-{}",
        std::process::id(),
        moli_core::now_ms()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let hub = Hub::new(HubOptions::default()).unwrap();
    let cancel = CancellationToken::new();
    add_weekly_schedule(&hub, &cancel).await;
    let (autos, _) = Automations::start(&hub, Config::default(), Some(&root), cancel.clone())
        .await
        .unwrap();
    let reference = moli_core::DependencyRef {
        kind: moli_core::DependencyKind::Schedule,
        id: "fixture:weekly".into(),
    };
    let graph = automation(
        serde_json::json!({"nodes":[{"id":"t","type":"manual"},{"id":"n","type":"notify","message":"{{fixture:weekly/active}}","channels":["maison"]}],"edges":[{"from":"t","to":"n"}]}),
    );
    let mut saved = autos.save(graph, AGENT).await.unwrap();
    let shown = hub.preview_dependency(&reference).await.unwrap();
    saved.graph = serde_json::from_value(
        serde_json::json!({"nodes":[{"id":"t","type":"manual"}],"edges":[]}),
    )
    .unwrap();
    std::fs::create_dir(root.join("automations.json.tmp")).unwrap();
    assert!(matches!(
        autos.save(saved.clone(), AGENT).await,
        Err(AutomationError::Io(_))
    ));
    assert_eq!(
        hub.preview_dependency(&reference).await,
        Err(moli_runtime::DependencyError::Consumers)
    );
    assert_eq!(
        hub.publish_dependency(shown, true, Origin::Ui, None).await,
        Err(moli_runtime::DependencyError::Consumers)
    );
    std::fs::remove_dir(root.join("automations.json.tmp")).unwrap();
    autos.save(saved, AGENT).await.unwrap();
    assert!(
        hub.preview_dependency(&reference)
            .await
            .unwrap()
            .consumers
            .is_empty()
    );
    cancel.cancel();
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn auto_s1_scene_graph_uses_current_head_and_agent_drafts_are_inert() {
    let (hub, autos, cancel) = setup().await;
    let id = moli_core::DeviceId::from("scenes:desired");
    let definition = |value| {
        serde_json::from_value(serde_json::json!({"targets":[{"point":"fake:lamp/on","value":value}],"parallel":2,"timeout_s":10,"confirmation_s":2})).unwrap()
    };
    hub.initialize_scene(id.clone(), definition(true))
        .await
        .unwrap();
    let draft=autos.save(automation(serde_json::json!({"nodes":[{"id":"t","type":"manual"},{"id":"s","type":"scene","scene":"scenes:desired"}],"edges":[{"from":"t","to":"s"}]})),AGENT).await.unwrap();
    assert!(
        autos
            .approve(&draft.id, &draft.fingerprint(), HUMAN)
            .await
            .is_err()
    );
    let reference = moli_core::DependencyRef {
        kind: moli_core::DependencyKind::Scene,
        id,
    };
    let preview = hub.preview_dependency(&reference).await.unwrap();
    assert_eq!(preview.consumers.len(), 1);
    assert_eq!(preview.consumers[0].id, draft.id);
    assert_eq!(
        hub.publish_dependency(preview.clone(), false, Origin::Mcp, None)
            .await,
        Err(moli_runtime::DependencyError::Human)
    );
    hub.publish_dependency(preview, true, Origin::Ui, None)
        .await
        .unwrap();
    let approved = autos
        .approve(&draft.id, &draft.fingerprint(), HUMAN)
        .await
        .unwrap();
    let fingerprint = approved.fingerprint();
    hub.stage_dependency(
        reference.clone(),
        1,
        moli_runtime::DependencyDefinition::Scene(definition(false)),
        Origin::Mcp,
        None,
    )
    .await
    .unwrap();
    assert!(autos.get(&draft.id).unwrap().is_approved());
    assert_eq!(autos.get(&draft.id).unwrap().fingerprint(), fingerprint);
    let run = autos
        .run_now(&draft.id, false, Some(&fingerprint), HUMAN)
        .await
        .unwrap();
    scene_graph_completed(&autos, &draft.id, run.id).await;
    assert_eq!(
        hub.state(&"fake:lamp/on".into()).unwrap().value,
        Value::Bool(true)
    );
    hub.publish_dependency(
        hub.preview_dependency(&reference).await.unwrap(),
        true,
        Origin::Ui,
        None,
    )
    .await
    .unwrap();
    assert_eq!(autos.get(&draft.id).unwrap().fingerprint(), fingerprint);
    let run = autos
        .run_now(&draft.id, false, Some(&fingerprint), HUMAN)
        .await
        .unwrap();
    scene_graph_completed(&autos, &draft.id, run.id).await;
    assert_eq!(
        hub.state(&"fake:lamp/on".into()).unwrap().value,
        Value::Bool(false)
    );
    assert!(
        hub.journal(100)
            .iter()
            .any(|entry| entry.origin == Origin::Automation
                && matches!(entry.action, moli_core::Action::Scene { .. }))
    );
    cancel.cancel();
}

async fn scene_graph_completed(autos: &Automations, id: &str, run: u64) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if autos
                .runs(Some(id), 10)
                .iter()
                .any(|r| r.id == run && r.status == RunStatus::Done)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
