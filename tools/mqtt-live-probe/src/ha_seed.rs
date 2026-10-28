//! Export anonymized HA replay through the existing house projection, for a local HA POC.
use super::*;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

pub(super) fn run(paths: &[PathBuf]) -> Result<Json> {
    ensure!(paths.len() == 2, "exactly two replay sources required");
    let mut houses = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        let fixture = crate::load_fixture::SourceFixture::load(path)?;
        let house_id = if index == 0 { "house-a" } else { "house-b" };
        let mut house = House::new(format!("moli-probe/ha-replay/{house_id}"));
        house.enum_persistence_disabled = true;
        house.registry(&serde_json::to_vec(&fixture.registry(0))?)?;
        for (topic, payload) in fixture.initial_messages(0) {
            house.ingest(&topic, &payload, true);
        }
        let mut document = BTreeMap::new();
        let mut description_bytes = 0;
        for (topic, payload) in house.publications()? {
            if payload["type"] == "description" {
                description_bytes += serde_json::to_vec(&payload)?.len();
            }
            let key = format!("record-{:x}", Sha256::digest(topic.as_bytes()));
            document.insert(key, json!({"topic":topic,"payload":payload}));
        }
        ensure!(!document.is_empty(), "empty replay projection");
        let bytes = serde_json::to_vec(&document)?;
        ensure!(
            bytes.len() <= 8 * 1024 * 1024,
            "replay snapshot exceeds bound"
        );
        let counts = house.counts();
        houses.push(json!({"id":house_id,"document":document,"summary":{
            "house":house_id,"kind":"private_ha_replay","entities":fixture.stats()["registry_entities"],
            "devices":counts["devices"],"points":counts["channels"],"nodes":counts["nodes"],
            "observed_points":counts["observed_channels"],"description_bytes":description_bytes,
            "snapshot_bytes":bytes.len(),"fixture":fixture.stats(),"physical_commands":0,
            "synthetic_identities":true,"physical_freshness_proven":false
        }}));
    }
    Ok(json!({"v":1,"kind":"private_ha_replay","houses":houses}))
}
