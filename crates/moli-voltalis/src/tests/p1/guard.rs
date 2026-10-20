//! CHA-002 exercises the real hub verdict and the actual PUT composer.

use super::*;

use moli_core::Semantic;
use moli_runtime::guard::GuardPolicy;
use moli_runtime::{CommandError, LabelPatch};

fn appliance_in(mode: &str) -> Fake {
    let fake = Fake::default();
    {
        let mut appliances = fake.appliances.lock().unwrap();
        let p = &mut appliances[0]["programming"];
        p["progType"] = if mode == "programme" {
            "USER"
        } else {
            "MANUAL"
        }
        .into();
        p["isOn"] = (mode != "arret").into();
        p["mode"] = "TEMPERATURE".into();
        p["temperatureTarget"] = 19.0.into();
    }
    fake
}

async fn room(hub: &Hub, name: &str) {
    hub.set_label(
        &heater(),
        LabelPatch {
            room: Some(name.into()),
            ..LabelPatch::default()
        },
        Origin::Ui,
        None,
    )
    .await
    .unwrap();
}

fn assert_body(body: &Json, key: &str, requested: &Value) {
    assert_eq!(body["enabled"], true);
    assert_eq!(body["isOn"], true);
    assert_eq!(body["untilFurtherNotice"], true);
    assert_eq!(body["endDate"], Json::Null);
    assert_eq!(body["idAppliance"], 200_001);
    if key == "setpoint" {
        assert_eq!(body["mode"], "TEMPERATURE");
        assert_eq!(body["temperatureTarget"], requested.to_json());
    } else {
        assert_eq!(body["mode"], "CONFORT");
        assert_eq!(body["temperatureTarget"], 19.0);
    }
}

async fn held_until_approved(key: &'static str, requested: Value) {
    for mode in ["arret", "programme", "manuel"] {
        for origin in [
            Origin::Mcp,
            Origin::Api,
            Origin::Cli,
            Origin::Assistant,
            Origin::Household,
        ] {
            let requested = requested.clone();
            caller::with_rig(appliance_in(mode), HubOptions::default(), move |hub, fake| async move {
                room(&hub, "Pièce neutre").await;
                let point = PointId::new(&heater(), key);
                let result = hub.command(&point, requested.clone(), origin, None).await;
                let puts = fake.puts();
                assert!(puts.is_empty(), "CHA-002 {mode}/{origin:?}/{key}: PUT emitted before approval: {puts:?}; result={result:?}");
                let Err(CommandError::NeedsApproval { id, .. }) = result else {
                    panic!("CHA-002 {mode}/{origin:?}/{key}: approval required, got {result:?}");
                };
                assert_eq!(hub.device(&heater()).unwrap().device.point(key).unwrap().semantic, Semantic::Control);
                assert_eq!(hub.approvals().len(), 1);
                // A non-human cannot grant this approval, even knowing its id.
                assert!(hub.resolve_approval(id, &point, true, Origin::Mcp, None).await.is_err());
                assert!(fake.puts().is_empty());
                hub.resolve_approval(id, &point, true, Origin::Ui, None).await.unwrap();
                let puts = fake.puts();
                assert_eq!(puts.len(), 1, "exactly one PUT after approval");
                assert_eq!(puts[0].0, "/api/site/100001/manualsetting/300001");
                assert_body(&puts[0].1, key, &requested);
                until(|| value(&hub, &heater(), key) == Some(requested.clone())).await;
                assert_eq!(fake.puts().len(), 1, "readback is not a replay");
            }).await;
        }
    }
}

#[tokio::test]
async fn p1_setpoint_control_is_held_before_any_put() {
    held_until_approved("setpoint", Value::Float(22.0)).await;
}

#[tokio::test]
async fn p1_preset_control_is_held_before_any_put() {
    held_until_approved("preset", Value::from("confort")).await;
}

#[tokio::test]
async fn p1_ui_and_validated_automation_keep_their_existing_rights() {
    for origin in [Origin::Ui, Origin::Automation] {
        for mode in ["arret", "programme", "manuel"] {
            for (key, requested) in [
                ("setpoint", Value::Float(22.0)),
                ("preset", Value::from("confort")),
            ] {
                caller::with_rig(
                    appliance_in(mode),
                    HubOptions::default(),
                    move |hub, fake| async move {
                        room(&hub, "Pièce neutre").await;
                        hub.command(
                            &PointId::new(&heater(), key),
                            requested.clone(),
                            origin,
                            None,
                        )
                        .await
                        .unwrap();
                        assert!(
                            hub.approvals().is_empty(),
                            "no universal approval/PIN policy"
                        );
                        let puts = fake.puts();
                        assert_eq!(puts.len(), 1);
                        assert_body(&puts[0].1, key, &requested);
                    },
                )
                .await;
            }
        }
    }
}

#[tokio::test]
async fn p1_protected_room_policy_is_preserved() {
    for origin in [
        Origin::Mcp,
        Origin::Household,
        Origin::Ui,
        Origin::Automation,
    ] {
        for (key, requested) in [
            ("setpoint", Value::Float(22.0)),
            ("preset", Value::from("confort")),
        ] {
            let options = HubOptions {
                guard: GuardPolicy {
                    protected_rooms: vec!["Pièce protégée fictive".into()],
                    ..GuardPolicy::default()
                },
                ..HubOptions::default()
            };
            caller::with_rig(
                appliance_in("programme"),
                options,
                move |hub, fake| async move {
                    room(&hub, "Pièce protégée fictive").await;
                    let result = hub
                        .command(&PointId::new(&heater(), key), requested, origin, None)
                        .await;
                    if matches!(origin, Origin::Ui | Origin::Automation) {
                        assert_eq!(result, Ok(()));
                        assert_eq!(fake.puts().len(), 1);
                    } else {
                        assert!(matches!(result, Err(CommandError::NeedsApproval { .. })));
                        assert!(fake.puts().is_empty());
                    }
                },
            )
            .await;
        }
    }
}

#[tokio::test]
async fn p1_water_heater_relay_remains_read_only() {
    for origin in [Origin::Mcp, Origin::Ui, Origin::Automation] {
        caller::with_rig(
            Fake::default(),
            HubOptions::default(),
            move |hub, fake| async move {
                let water = device("appliance-200003");
                let view = hub.device(&water).unwrap();
                assert!(view.device.points.iter().all(|p| !p.access.write));
                let result = hub
                    .command(
                        &PointId::new(&water, "setpoint"),
                        Value::Float(50.0),
                        origin,
                        None,
                    )
                    .await;
                assert_eq!(result, Err(CommandError::ReadOnly));
                assert!(hub.approvals().is_empty());
                assert!(fake.puts().is_empty());
            },
        )
        .await;
    }
}
