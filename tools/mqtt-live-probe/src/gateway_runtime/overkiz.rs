//! The Somfy box through the native `moli-overkiz` driver, run inside the
//! gateway on a private Hub. Its shutters and gate become one `control`
//! channel each (open/stop/my/close as the box declares); orders come from
//! the page, are validated here and go through the Hub, never replayed. RTS
//! reports nothing back: no position or state is ever invented.

use anyhow::{Context, Result, ensure};
use moli_core::{DeviceId, InstanceId, Kind, Origin, PointId, Value};
use moli_runtime::{Hub, HubOptions, MasterKey, Snapshot, spawn_driver};
use serde::Deserialize;
use serde_json::{Value as Json, json};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::Duration};

pub const ROOT: &str = "moli-probe/maison/overkiz";
const INSTANCE: &str = "overkiz";
const ROOM: &str = "piece-somfy";

/// What the control socket gives: the box and its developer-mode token.
#[derive(Deserialize)]
pub struct Setup {
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub token: String,
}
fn default_port() -> u16 {
    8443
}
impl Setup {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.host.is_empty()
                && self.host.len() <= 253
                && self
                    .host
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".:-".contains(&b)),
            "host"
        );
        ensure!(
            (8..=256).contains(&self.token.len())
                && self.token.bytes().all(|b| b.is_ascii_graphic())
                && !self.token.starts_with("{{secret:"),
            "token"
        );
        Ok(())
    }
}

/// A page order, already checked against the published contract.
pub struct Order {
    pub topic: String,
    pub channel: String,
    pub value: String,
}

/// A topic segment from a device id (`overkiz/rts://…/16758431`).
fn channel_id(device: &DeviceId) -> String {
    let raw: String = device
        .to_string()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    raw.split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// Description, channel → point map, and the state messages, from the Hub.
pub fn project(snapshot: &Snapshot) -> (Json, BTreeMap<String, PointId>, Vec<(String, Json)>) {
    let mut channels = serde_json::Map::new();
    let mut devices = Vec::new();
    let mut points = BTreeMap::new();
    let mut states = Vec::new();
    for view in &snapshot.devices {
        let device = &view.device;
        if device.instance.to_string() != INSTANCE {
            continue;
        }
        let Some(control) = device.points.iter().find(|p| &*p.key == "control") else {
            continue;
        };
        let Kind::Enum { values } = &control.kind else {
            continue;
        };
        let id = channel_id(&device.id);
        let gate = device.model.as_deref().is_some_and(|m| m.contains("Gate"));
        let name = view
            .label
            .name
            .clone()
            .unwrap_or_else(|| device.native_name.to_string());
        channels.insert(
            id.clone(),
            json!({"id":id,"nom":name,"unite":null,"semantique":if gate {"portail"} else {"volet"},
                "domaine":"cover","piece":ROOM,"appareil":id,"source":format!("overkiz/{}",device.id),
                "topic":format!("{ROOT}/canal/{id}/state"),"categorie":null,
                "commande":{"schema":"virtual","topic":format!("{ROOT}/canal/{id}/set"),"retain":false,
                    "parametres":{"control":{"type":"enum","valeurs":values.iter().map(ToString::to_string).collect::<Vec<_>>()}}}}),
        );
        devices.push(json!({"id":id,"nom":name,"piece":ROOM,"genre":"physique",
            "description_topic":format!("{ROOT}/config"),"canaux":[id]}));
        points.insert(id.clone(), PointId::new(&device.id, "control"));
        // Write-only: the box never says where an RTS motor is.
        states.push((
            format!("{ROOT}/canal/{id}/state"),
            json!({"v":3,"type":"canal","id":id,"valeur":null,
                "qualite":if view.online == Some(false) {"absent"} else {"ecriture_seule"},
                "retenu":false,"recu_ms":null,"pont":ROOT}),
        ));
    }
    let ids: Vec<_> = devices.iter().map(|d| d["id"].clone()).collect();
    let description = json!({"v":3,"type":"description","nom":"Somfy TaHoma","pont":ROOT,"source_id":"overkiz",
        "noeuds":[
            {"id":"maison","genre":"maison","parent":null,"topic":format!("{ROOT}/noeud/maison"),"ordre":null,"nom":"Somfy TaHoma","appareils":ids},
            {"id":ROOM,"genre":"piece","parent":"maison","topic":format!("{ROOT}/noeud/{ROOM}"),"ordre":null,"nom":"Volets et portail","appareils":ids}
        ],
        "appareils":devices,"canaux":channels,
        "commandes_physiques":"uniquement_depuis_interface_humaine_et_contrats_source"});
    (description, points, states)
}

/// Checks a page body against the published `control` values.
pub fn order(topic: &str, native: &Json, body: &Json) -> Result<Order> {
    let values = body.as_object().context("payload")?;
    ensure!(values.len() == 1, "payload");
    let value = values
        .get("control")
        .and_then(Json::as_str)
        .context("control")?;
    ensure!(
        native["parametres"]["control"]["valeurs"]
            .as_array()
            .is_some_and(|allowed| allowed.iter().any(|v| v == value)),
        "control"
    );
    let channel = topic
        .strip_prefix(&format!("{ROOT}/canal/"))
        .and_then(|t| t.strip_suffix("/set"))
        .context("topic")?;
    Ok(Order {
        topic: topic.into(),
        channel: channel.into(),
        value: value.into(),
    })
}

/// A Hub with an ephemeral secret store (key in memory only) and the
/// driver started; the file dies with the process.
pub async fn start(setup: Setup, secrets: PathBuf) -> Result<Hub> {
    setup.validate()?;
    let _ = std::fs::remove_file(&secrets);
    let hub = Hub::new(HubOptions {
        secrets: Some((secrets, MasterKey::generate()?)),
        ..HubOptions::default()
    })?;
    let instance = InstanceId::from(INSTANCE);
    hub.store_secret(&instance, "token", &setup.token).await?;
    let driver = moli_overkiz::Overkiz::new(moli_overkiz::Config {
        transport: moli_overkiz::Transport::Local,
        host: Some(setup.host),
        port: setup.port,
    })?;
    spawn_driver(
        &hub,
        instance,
        Arc::new(driver),
        tokio_util::sync::CancellationToken::new(),
    );
    Ok(hub)
}

/// Sends one order through the Hub (guard, deadline, single send).
pub async fn send(hub: &Hub, point: &PointId, value: &str) -> Result<()> {
    tokio::time::timeout(
        Duration::from_secs(8),
        hub.command(
            point,
            Value::Text(value.into()),
            Origin::Ui,
            Some("poc".into()),
        ),
    )
    .await
    .context("timeout")?
    .map_err(|e| anyhow::anyhow!("{e}"))
}

/// Driver health for `/api/status` (no secret, no address).
pub fn status(snapshot: &Snapshot) -> Json {
    let driver = snapshot
        .drivers
        .iter()
        .find(|d| d.instance.to_string() == INSTANCE);
    json!({"driver":driver.map(|d| serde_json::to_value(d).unwrap_or(Json::Null)),
        "devices":snapshot.devices.iter().filter(|d| d.device.instance.to_string()==INSTANCE).count()})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orders_follow_the_published_contract() {
        let native =
            json!({"parametres":{"control":{"type":"enum","valeurs":["open","stop","close"]}}});
        let topic = format!("{ROOT}/canal/overkiz-rts-1/set");
        let ok = order(&topic, &native, &json!({"control":"stop"})).unwrap();
        assert_eq!(
            (ok.channel.as_str(), ok.value.as_str()),
            ("overkiz-rts-1", "stop")
        );
        assert!(
            order(&topic, &native, &json!({"control":"my"})).is_err(),
            "not declared"
        );
        assert!(order(&topic, &native, &json!({"control":"open","x":1})).is_err());
        assert!(order(&topic, &native, &json!({"state":"ON"})).is_err());
        assert!(
            order(
                "moli-probe/maison/other/canal/a/set",
                &native,
                &json!({"control":"open"})
            )
            .is_err()
        );
    }

    #[test]
    fn setup_refuses_literal_secrets_and_odd_hosts() {
        let good = || Setup {
            host: "192.168.1.2".into(),
            port: 8443,
            token: "abcdefgh12345678".into(),
        };
        assert!(good().validate().is_ok());
        assert!(
            Setup {
                token: "{{secret:somfy}}".into(),
                ..good()
            }
            .validate()
            .is_err()
        );
        assert!(
            Setup {
                host: "a/b".into(),
                ..good()
            }
            .validate()
            .is_err()
        );
        assert!(
            Setup {
                token: "short".into(),
                ..good()
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn channel_ids_are_topic_segments() {
        let id = DeviceId::new(&InstanceId::from(INSTANCE), "rts://1222-6647-0581/16758431");
        let segment = channel_id(&id);
        assert!(
            segment
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        );
        assert!(segment.ends_with("16758431"), "{segment}");
    }
}
