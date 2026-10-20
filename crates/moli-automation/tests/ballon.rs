//! The hot water tank's drafts (`integrations/helpers/recettes/ballon.json`)
//! against the real engine: they check, they do what the three automations
//! they replace did, and nothing happens on their own when Moli comes up.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering::SeqCst};
use std::time::Duration;

use moli_automation::{Actor, Author, Automation, Automations, Config, Mode, Step};
use moli_core::{Access, Device, InstanceId, Kind, Origin, PointId, PointSpec, Semantic, Value};
use moli_runtime::{BoxFuture, Driver, DriverCtx, Hub, HubOptions, spawn_driver};
use serde_json::Value as Json;
use tokio_util::sync::CancellationToken;

const MODE: &str = "maison:ballon_mode/state";
const RELAY: &str = "maison:relais/state";
const HUMAN: Actor = Actor {
    human: true,
    author: Author::Human,
};

/// The selector (as the helpers driver shows it) and the relay.
#[derive(Debug)]
struct House {
    mode: &'static str,
    initial_on: bool,
    /// Orders the relay received.
    orders: Arc<AtomicU32>,
}

fn spec(key: &str, kind: Kind, semantic: Semantic) -> PointSpec {
    PointSpec {
        key: key.into(),
        label: key.into(),
        kind,
        access: Access {
            read: true,
            write: true,
        },
        unit: None,
        semantic,
    }
}

impl Driver for House {
    fn kind(&self) -> &'static str {
        "fake"
    }

    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let options = ["Éteint", "Automatique", "Forcer"];
            for (native, point) in [
                (
                    "ballon_mode",
                    spec(
                        "state",
                        Kind::Enum {
                            values: options.iter().map(|o| (*o).into()).collect(),
                        },
                        Semantic::Other,
                    ),
                ),
                ("relais", spec("state", Kind::Binary, Semantic::OnOff)),
            ] {
                ctx.upsert_device(Device {
                    id: ctx.device_id(native),
                    instance: ctx.instance().clone(),
                    native_name: native.into(),
                    manufacturer: None,
                    model: None,
                    description: None,
                    native_room: None,
                    members: Vec::new(),
                    points: vec![point],
                });
            }
            ctx.set_state(
                &ctx.device_id("ballon_mode"),
                "state",
                Value::from(self.mode),
            );
            ctx.set_state(
                &ctx.device_id("relais"),
                "state",
                Value::Bool(self.initial_on),
            );
            ctx.ready();
            while let Some(cmd) = ctx.next_command().await {
                if cmd.device.id == ctx.device_id("relais") {
                    self.orders.fetch_add(1, SeqCst);
                }
                ctx.set_state(&cmd.device.id, &cmd.key, cmd.value.clone());
                cmd.reply(Ok(()));
            }
            Ok(())
        })
    }
}

/// The drafts, with this house's points in place of the placeholders.
fn drafts() -> Vec<Automation> {
    let path = format!(
        "{}/../../integrations/helpers/recettes/ballon.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(path)
        .unwrap()
        .replace("{aides}", "maison")
        .replace("{relais}", RELAY);
    let recipe: Json = serde_json::from_str(&text).unwrap();
    assert_eq!(
        recipe["helper"]["options"],
        serde_json::json!(["Éteint", "Automatique", "Forcer"])
    );
    recipe["automations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            let mut a: Automation = serde_json::from_value(a.clone()).unwrap();
            assert!(!a.enabled, "a draft, never on by itself");
            a.enabled = true;
            a
        })
        .collect()
}

async fn setup(mode: &'static str) -> (Hub, Automations, CancellationToken, Arc<AtomicU32>) {
    setup_with_relay(mode, false).await
}

async fn setup_with_relay(
    mode: &'static str,
    initial_on: bool,
) -> (Hub, Automations, CancellationToken, Arc<AtomicU32>) {
    let hub = Hub::new(HubOptions::default()).unwrap();
    let cancel = CancellationToken::new();
    let orders = Arc::new(AtomicU32::new(0));
    spawn_driver(
        &hub,
        InstanceId::from("maison"),
        Arc::new(House {
            mode,
            initial_on,
            orders: Arc::clone(&orders),
        }),
        cancel.clone(),
    );
    for _ in 0..200 {
        if hub.state(&PointId::from(RELAY)).is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let (autos, _task) = Automations::start(&hub, Config::default(), None, cancel.clone())
        .await
        .unwrap();
    (hub, autos, cancel, orders)
}

/// A person reads each draft and approves what they saw.
async fn approve_all(autos: &Automations) {
    for draft in drafts() {
        let saved = autos.save(draft, HUMAN).await.unwrap();
        autos
            .approve(&saved.id, &saved.fingerprint(), HUMAN)
            .await
            .unwrap();
    }
}

fn relay(hub: &Hub) -> Option<Value> {
    hub.state(&PointId::from(RELAY)).map(|s| s.value)
}

async fn relay_becomes(hub: &Hub, on: bool) -> bool {
    for _ in 0..100 {
        if relay(hub) == Some(Value::Bool(on)) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

async fn choose(hub: &Hub, mode: &str) {
    hub.command(&PointId::from(MODE), Value::from(mode), Origin::Ui, None)
        .await
        .unwrap();
}

#[tokio::test]
async fn the_drafts_check() {
    let (_hub, autos, cancel, _) = setup("Éteint").await;
    let drafts = drafts();
    assert_eq!(
        drafts.len(),
        3,
        "three automations, as in the package replaced"
    );
    for draft in &drafts {
        let check = autos.check(&draft.graph);
        assert!(check.ok(), "{}: {:?}", draft.name, check);
        assert_eq!(draft.mode, Mode::Single);
        assert!(
            !draft
                .graph
                .triggers()
                .any(|t| matches!(t.step, Step::OnStart)),
            "nothing runs because Moli started"
        );
    }
    cancel.cancel();
}

#[tokio::test]
async fn they_do_what_the_three_did() {
    let (hub, autos, cancel, _) = setup("Éteint").await;
    approve_all(&autos).await;
    choose(&hub, "Forcer").await;
    assert!(relay_becomes(&hub, true).await, "forced: heats");
    choose(&hub, "Éteint").await;
    assert!(relay_becomes(&hub, false).await, "off: never heats");
    // Automatic: heats only in the off-peak hours, house time.
    let now = jiff::Zoned::now().with_time_zone(jiff::tz::TimeZone::get("Europe/Paris").unwrap());
    let m = u32::try_from(now.hour()).unwrap() * 60 + u32::try_from(now.minute()).unwrap();
    let off_peak = (80..440).contains(&m) || (710..830).contains(&m);
    choose(&hub, "Forcer").await;
    assert!(relay_becomes(&hub, true).await);
    choose(&hub, "Automatique").await;
    assert!(relay_becomes(&hub, off_peak).await, "off-peak: {off_peak}");
    cancel.cancel();
}

#[tokio::test]
async fn coming_up_does_nothing_by_itself() {
    for mode in ["Éteint", "Automatique", "Forcer"] {
        for initial_on in [false, true] {
            let (hub, autos, cancel, orders) = setup_with_relay(mode, initial_on).await;
            approve_all(&autos).await;
            tokio::time::sleep(Duration::from_millis(400)).await;
            assert_eq!(
                orders.load(SeqCst),
                0,
                "no startup order: {mode}/{initial_on}"
            );
            assert_eq!(relay(&hub), Some(Value::Bool(initial_on)));
            cancel.cancel();
        }
    }
}
