//! An excluded point is no longer written to the history, while the energy,
//! which listens to the bus on its own, still counts every reading of it.

use std::sync::Arc;
use std::time::Duration;

use moli_core::{
    Access, Device, InstanceId, Kind, Origin, PointId, PointSpec, Semantic, Unit, Value,
};
use moli_history::{History, Options, StorageConfig};
use moli_runtime::{BoxFuture, Driver, DriverCtx, Hub, HubOptions, spawn_driver};
use tokio_util::sync::CancellationToken;

/// A meter: `e` counts Wh, `ptec` is the tariff period.
struct FakeMeter;

fn spec(key: &str, kind: Kind, unit: Option<Unit>) -> PointSpec {
    PointSpec {
        key: key.into(),
        label: key.into(),
        kind,
        access: Access {
            read: true,
            write: true,
        },
        unit,
        semantic: Semantic::Other,
    }
}

impl Driver for FakeMeter {
    fn kind(&self) -> &'static str {
        "fake"
    }

    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let id = ctx.device_id("m");
            ctx.upsert_device(Device {
                id: id.clone(),
                instance: ctx.instance().clone(),
                native_name: "m".into(),
                manufacturer: None,
                model: None,
                description: None,
                native_room: None,
                members: Vec::new(),
                points: vec![
                    spec(
                        "e",
                        Kind::Numeric {
                            min: None,
                            max: None,
                            step: None,
                        },
                        Some(Unit::WattHour),
                    ),
                    spec("ptec", Kind::Text, None),
                ],
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

const ENERGY: &str = r#"
    max_power_kw = 1e12
    [tariff]
    period = "fake:m/ptec"
    prices = { "HP" = 0.2 }
    [[meter]]
    id = "general"
    name = "Général"
    point = "fake:m/e"
    role = "total"
"#;

async fn set(hub: &Hub, key: &str, value: Value) {
    hub.command(
        &PointId::from(format!("fake:m/{key}")),
        value,
        Origin::Ui,
        None,
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(60)).await;
}

#[tokio::test]
async fn an_excluded_point_is_not_written_and_its_energy_stays_exact() {
    let dir = std::env::temp_dir().join(format!("moli-history-policy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let hub = Hub::new(HubOptions::default()).unwrap();
    let drivers = CancellationToken::new();
    spawn_driver(
        &hub,
        InstanceId::from("fake"),
        Arc::new(FakeMeter),
        drivers.clone(),
    );
    while hub.stats().drivers_running == 0 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let storage: StorageConfig = toml::from_str("[history]\nexclude = [\"fake:*/e\"]").unwrap();
    let (filter, retention) = storage.compile(30).unwrap();
    let cancel = CancellationToken::new();
    let options = Options {
        path: dir.join("history.db"),
        filter,
        retention,
    };
    let (history, recorder) = History::start(&hub, &options, cancel.clone()).unwrap();
    let (energy, metering) = moli_energy::Energy::start(
        &hub,
        &dir.join("energy.db"),
        toml::from_str(ENERGY).unwrap(),
        cancel.clone(),
    )
    .unwrap();

    set(&hub, "ptec", Value::from("HP..")).await;
    set(&hub, "e", Value::Float(1000.0)).await;
    set(&hub, "e", Value::Float(3500.0)).await;
    set(&hub, "e", Value::Float(4000.0)).await;
    let kwh = energy.summary().await.unwrap().today.total.kwh;
    assert!((kwh - 3.0).abs() < 1e-9, "every reading counted: {kwh}");

    // Everything recorded is flushed.
    cancel.cancel();
    recorder.await.unwrap();
    metering.await.unwrap();
    let day = 86_400_000;
    let now = moli_core::now_ms();
    let series = |point: &str| {
        let (history, point) = (history.clone(), point.to_owned());
        async move {
            history
                .series(&point, now - day, now + day, 100)
                .await
                .unwrap()
        }
    };
    assert_eq!(series("fake:m/e").await.total, 0, "excluded: never written");
    assert_eq!(series("fake:m/ptec").await.total, 1, "the others still are");
    drivers.cancel();
    let _ = std::fs::remove_dir_all(dir);
}
