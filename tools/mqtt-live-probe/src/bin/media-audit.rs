//! Read-only observation of media field shapes and byte lengths, never values.
use anyhow::{Context, Result, ensure};
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use serde_json::{Value as Json, json};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

fn shape(payload: &[u8]) -> Json {
    let value = serde_json::from_slice::<Json>(payload)
        .unwrap_or_else(|_| Json::String(String::from_utf8_lossy(payload).into()));
    match value {
        Json::String(s) => json!({"kind":"text","bytes":s.len()}),
        Json::Array(a) => {
            json!({"kind":"list","bytes":payload.len(),"items":a.len(),"max_item_bytes":a.iter().filter_map(Json::as_str).map(str::len).max().unwrap_or(0)})
        }
        Json::Null => json!({"kind":"null","bytes":0}),
        Json::Number(_) => json!({"kind":"number","bytes":8}),
        Json::Bool(_) => json!({"kind":"bool","bytes":1}),
        _ => json!({"kind":"object","bytes":payload.len()}),
    }
}
fn summarize(
    players: &BTreeMap<String, BTreeMap<String, Json>>,
    filter: Option<&BTreeSet<String>>,
) -> Json {
    let mut fields: BTreeMap<String, Vec<&Json>> = BTreeMap::new();
    for (entity, values) in players {
        if filter.is_some_and(|f| !f.contains(entity)) {
            continue;
        }
        for (key, value) in values {
            fields.entry(key.clone()).or_default().push(value);
        }
    }
    let out:BTreeMap<_,_>=fields.into_iter().map(|(key,values)|{
        let mut kinds=BTreeMap::<&str,usize>::new();for v in &values{*kinds.entry(v["kind"].as_str().unwrap_or("other")).or_default()+=1;}
        let lens:Vec<_>=values.iter().filter(|v|v["kind"]=="text").filter_map(|v|v["bytes"].as_u64()).collect();
        (key,json!({"samples":values.len(),"kinds":kinds,"max_text_bytes":lens.iter().max(),"text_le_16":lens.iter().filter(|n|**n<=16).count(),"text_gt_16":lens.iter().filter(|n|**n>16).count(),"largest_list":values.iter().filter_map(|v|v["items"].as_u64()).max(),"max_list_item_bytes":values.iter().filter_map(|v|v["max_item_bytes"].as_u64()).max()}))
    }).collect();
    json!(out)
}
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let host = std::env::args().nth(1).context("host required")?;
    let password = std::env::var("MQTT_PASSWORD").context("MQTT_PASSWORD required")?;
    ensure!(
        !password.is_empty() && !password.starts_with("{{secret:"),
        "invalid MQTT_PASSWORD"
    );
    let mut options = MqttOptions::new(
        format!("moli-media-audit-{}", std::process::id()),
        host,
        1883,
    );
    options.set_credentials("test-mqtt", password);
    options.set_max_packet_size(4 * 1024 * 1024, 1024);
    let (client, mut events) = AsyncClient::new(options, 16);
    let end = Instant::now() + Duration::from_secs(5);
    let mut players = BTreeMap::<String, BTreeMap<String, Json>>::new();
    let mut apple = BTreeSet::new();
    let mut registry = false;
    let fields = [
        "state",
        "app_id",
        "app_name",
        "source",
        "source_list",
        "media_title",
        "media_artist",
        "media_album_name",
        "media_position",
        "media_position_updated_at",
        "media_duration",
        "volume_level",
        "supported_features",
    ];
    while Instant::now() < end {
        let Ok(event) =
            tokio::time::timeout(end.saturating_duration_since(Instant::now()), events.poll())
                .await
        else {
            break;
        };
        match event.context("media audit transport failed")? {
            Event::Incoming(Packet::ConnAck(_)) => {
                client
                    .try_subscribe("homeassistant/statestream/media_player/#", QoS::AtLeastOnce)?;
                client.try_subscribe("moli-probe/source/registry", QoS::AtLeastOnce)?;
            }
            Event::Incoming(Packet::Publish(p)) => {
                if p.topic == "moli-probe/source/registry" {
                    let data: Json = serde_json::from_slice(&p.payload)?;
                    registry = true;
                    if let Some(entities) = data["entities"].as_array() {
                        for entity in entities {
                            if entity["platform"] == "apple_tv"
                                && let Some(id) = entity["entity_id"]
                                    .as_str()
                                    .filter(|s| s.starts_with("media_player."))
                            {
                                apple.insert(id.into());
                            }
                        }
                    }
                    continue;
                }
                let Some(rest) = p
                    .topic
                    .strip_prefix("homeassistant/statestream/media_player/")
                else {
                    continue;
                };
                let Some((object, field)) = rest.split_once('/') else {
                    continue;
                };
                if !fields.contains(&field) || p.payload.len() > 65536 || players.len() >= 1024 {
                    continue;
                }
                players
                    .entry(format!("media_player.{object}"))
                    .or_default()
                    .insert(field.into(), shape(&p.payload));
            }
            _ => {}
        }
    }
    println!(
        "{}",
        json!({"registry_seen":registry,"media_players_seen":players.len(),"apple_tv_entities_in_registry":apple.len(),"apple_tv_entities_seen":players.keys().filter(|id|apple.contains(*id)).count(),"rust_value_bytes":std::mem::size_of::<moli_core::Value>(),"rust_sample_bytes":std::mem::size_of::<moli_core::Sample>(),"all_media_fields":summarize(&players,None),"apple_tv_fields":summarize(&players,Some(&apple))})
    );
    Ok(())
}
