//! POC projection of the real HA registry and statestream, never inferred rooms.
//! All private labels remain on the household broker; stdout contains counts.
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use serde_json::{Value as Json, json};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MAX_ENTITIES: usize = 8192;
const MAX_CHANNELS: usize = 16384;
const MAX_TEXT: usize = 4096;
const MAX_SCHEMA: usize = 256 * 1024;

#[path = "house_audit.rs"]
mod audit;
#[path = "house_enums.rs"]
mod house_enums;
pub(super) fn audit_exports(paths: &[std::path::PathBuf]) -> Result<Json> {
    audit::run(paths)
}
#[path = "hub_projection.rs"]
mod hub_projection;
use crate::enum_catalogue::{CatalogRef, Pool};
#[cfg(test)]
#[path = "house_wire_tests.rs"]
mod wire_tests;

fn text<'a>(row: &'a Json, key: &str) -> &'a str {
    row[key].as_str().unwrap_or("")
}

fn first<'a>(row: &'a Json, keys: &[&str]) -> &'a str {
    keys.iter()
        .map(|key| text(row, key))
        .find(|s| !s.is_empty())
        .unwrap_or("")
}

fn id(kind: &str, native: &str) -> String {
    use std::fmt::Write;
    let mut result = format!("{kind}-");
    for byte in native.bytes() {
        write!(result, "{byte:02x}").expect("write string");
    }
    result
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[derive(Clone, Serialize)]
struct Node {
    id: String,
    genre: &'static str,
    parent: Option<String>,
    topic: String,
    ordre: Option<i64>,
    #[serde(skip)]
    temperature: String,
    #[serde(skip)]
    humidity: String,
}

#[derive(Clone, Serialize)]
struct Channel {
    id: String,
    unite: Option<String>,
    semantique: String,
    domaine: String,
    piece: String,
    valeur: Json,
    qualite: &'static str,
    retenu: bool,
    recu_ms: Option<u128>,
    source: String,
    #[serde(skip)]
    device: String,
    #[serde(skip)]
    base: bool,
    #[serde(skip)]
    enum_ref: Option<CatalogRef>,
    #[serde(skip)]
    enum_expected: bool,
    #[serde(skip)]
    enum_cache_checked: bool,
}

#[derive(Clone, Serialize)]
struct Device {
    id: String,
    piece: String,
    genre: &'static str,
    #[serde(skip)]
    channels: BTreeSet<String>,
}

#[derive(Default)]
pub(super) struct House {
    root: String,
    source_id: String,
    labels: BTreeMap<String, String>,
    nodes: BTreeMap<String, Node>,
    devices: BTreeMap<String, Device>,
    channels: BTreeMap<String, Channel>,
    entities: BTreeMap<String, String>,
    dirty_devices: BTreeSet<String>,
    dirty_channels: BTreeSet<String>,
    dirty_nodes: BTreeSet<String>,
    configured: bool,
    config_dirty: bool,
    temperature_unit: Option<String>,
    use_file_metadata: bool,
    metadata: crate::metadata::Bank,
    metadata_patches: BTreeMap<String, Json>,
    metadata_revision: u64,
    command_bindings: BTreeMap<String, String>,
    command_definitions: BTreeMap<String, Json>,
    discovery_topics: BTreeMap<String, String>,
    command_channels: BTreeSet<String>,
    enum_pool: Pool,
    enum_store: Option<std::sync::Arc<crate::enum_store::Store>>,
    enum_persistence_disabled: bool,
    enum_store_dirty: bool,
    enum_compacted: Option<Pool>,
    enum_compact_ready: bool,
    enum_pending: BTreeMap<String, house_enums::Pending>,
    calculation: std::cell::Cell<Timing>,
    pub(super) unmatched: u64,
    pub(super) rejected: u64,
}

fn semantic<'a>(domain: &str, class: &'a str, diagnostic: bool) -> &'a str {
    if diagnostic {
        return "diagnostic";
    }
    match (domain, class) {
        ("sensor", "temperature") => "temperature",
        ("sensor", "humidity") => "humidite",
        ("sensor", "power") => "puissance",
        ("sensor", "energy") => "energie",
        ("sensor", "battery") => "batterie",
        ("sensor", "illuminance") => "luminosite",
        ("binary_sensor", "door" | "window" | "opening" | "garage_door") => "ouverture",
        ("binary_sensor", "motion" | "occupancy" | "presence") => "presence",
        ("light", _) => "lumiere",
        ("climate", _) => "mode_chauffage",
        ("number", "temperature") => "consigne_chauffage",
        ("number", _) => "configuration",
        ("cover", _) => "volet",
        ("switch", _) => "interrupteur",
        _ => class,
    }
}

impl House {
    pub(super) fn new(root: String) -> Self {
        Self {
            root,
            ..Self::default()
        }
    }

    pub(super) fn registry(&mut self, payload: &[u8]) -> Result<()> {
        ensure!(payload.len() <= 4 * 1024 * 1024, "registry too large");
        let data: Json = serde_json::from_slice(payload).context("invalid registry JSON")?;
        let rows = |key: &str| -> Result<&Vec<Json>> {
            data[key].as_array().context("missing registry array")
        };
        ensure!(rows("entities")?.len() <= MAX_ENTITIES, "too many entities");
        ensure!(
            rows("areas")?.len() <= 256 && rows("floors")?.len() <= 64,
            "too many locations"
        );
        ensure!(rows("devices")?.len() <= 2048, "too many devices");
        let mut fresh = Self::new(self.root.clone());
        fresh.source_id = self.source_id.clone();
        fresh.use_file_metadata = self.use_file_metadata;
        fresh.metadata_revision = self.metadata_revision;
        fresh.command_definitions = std::mem::take(&mut self.command_definitions);
        fresh.discovery_topics = std::mem::take(&mut self.discovery_topics);
        if let Some(native) = data["house_id"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 256)
        {
            let prefix = self
                .root
                .split("/pont/")
                .next()
                .unwrap_or(&self.root)
                .split("/maison/")
                .next()
                .unwrap_or(&self.root);
            fresh.root = format!("{prefix}/maison/{}", id("h", native));
        }
        let same_house = fresh.root == self.root;
        fresh.init_enums(self, same_house)?;
        fresh.temperature_unit = data["temperature_unit"].as_str().map(str::to_owned);
        fresh.insert_node("maison".into(), "Maison".into(), "maison", None, "", "");
        fresh.insert_node(
            "sans-piece".into(),
            "Sans pièce attribuée".into(),
            "non_affecte",
            Some("maison".into()),
            "",
            "",
        );
        for row in rows("floors")? {
            let native = text(row, "floor_id");
            if !native.is_empty() {
                fresh.insert_node(
                    id("etage", native),
                    text(row, "name").into(),
                    "etage",
                    Some("maison".into()),
                    "",
                    "",
                );
                fresh
                    .nodes
                    .get_mut(&id("etage", native))
                    .expect("floor exists")
                    .ordre = row["level"].as_i64();
            }
        }
        for row in rows("areas")? {
            let native = text(row, "id");
            if native.is_empty() {
                continue;
            }
            let floor = id("etage", text(row, "floor_id"));
            let parent = if fresh.nodes.contains_key(&floor) {
                floor
            } else {
                "maison".into()
            };
            fresh.insert_node(
                id("piece", native),
                text(row, "name").into(),
                "piece",
                Some(parent),
                text(row, "temperature_entity_id"),
                text(row, "humidity_entity_id"),
            );
        }
        for row in rows("devices")? {
            if !row["disabled_by"].is_null() || text(row, "id").is_empty() {
                continue;
            }
            let native = text(row, "id");
            let key = id("appareil", native);
            let room = fresh.room(text(row, "area_id"));
            fresh
                .labels
                .insert(key.clone(), first(row, &["name_by_user", "name"]).into());
            fresh.devices.insert(
                key.clone(),
                Device {
                    id: key.clone(),
                    piece: room,
                    genre: "physique",
                    channels: BTreeSet::new(),
                },
            );
        }
        for row in rows("entities")? {
            if !row["disabled_by"].is_null() {
                continue;
            }
            let entity = text(row, "entity_id");
            let native = text(row, "id");
            if entity.len() > 256 || native.is_empty() || native.len() > 256 {
                continue;
            }
            let Some((domain, _)) = entity.split_once('.') else {
                continue;
            };
            // No alarm codes, keypad secrets or arbitrary input text in this POC.
            if matches!(domain, "input_text" | "text") {
                continue;
            }
            let key = id("canal", native);
            let mut device = id("appareil", text(row, "device_id"));
            if !fresh.devices.contains_key(&device) {
                device = id("entite", native);
                fresh
                    .labels
                    .entry(device.clone())
                    .or_insert_with(|| first(row, &["name", "original_name", "entity_id"]).into());
                fresh
                    .devices
                    .entry(device.clone())
                    .or_insert_with(|| Device {
                        id: device.clone(),
                        piece: "sans-piece".into(),
                        genre: "entite",
                        channels: BTreeSet::new(),
                    });
            }
            let parent = fresh.devices.get_mut(&device).expect("device inserted");
            let room = if text(row, "area_id").is_empty() {
                parent.piece.clone()
            } else {
                id("piece", text(row, "area_id"))
            };
            let room = if fresh.nodes.contains_key(&room) {
                room
            } else {
                "sans-piece".into()
            };
            parent.channels.insert(key.clone());
            let label = first(row, &["name", "original_name", "entity_id"]);
            fresh.labels.insert(key.clone(), label.into());
            let mut channel = Channel {
                id: key.clone(),
                unite: row["unit_of_measurement"].as_str().map(str::to_owned),
                semantique: semantic(
                    domain,
                    first(row, &["device_class", "original_device_class"]),
                    text(row, "entity_category") == "diagnostic",
                )
                .into(),
                domaine: domain.into(),
                piece: room,
                valeur: Json::Null,
                qualite: "absent",
                retenu: false,
                recu_ms: None,
                source: entity.into(),
                device: device.clone(),
                base: true,
                enum_ref: None,
                enum_cache_checked: false,
                enum_expected: matches!(
                    domain,
                    "select" | "input_select" | "water_heater" | "climate"
                ) || domain == "sensor"
                    && ["device_class", "original_device_class"]
                        .iter()
                        .any(|key| text(row, key) == "enum"),
            };
            if text(row, "entity_category") == "config" {
                channel.semantique = "configuration".into();
            }
            if same_house
                && let Some(old) = self.channels.get(&key)
                && old.domaine == domain
            {
                channel.valeur = old.valeur.clone();
                channel.qualite = old.qualite;
                channel.retenu = old.retenu;
                channel.recu_ms = old.recu_ms;
                channel.enum_ref = old.enum_ref;
                channel.enum_cache_checked = old.enum_cache_checked;
                if channel.enum_expected && !old.enum_expected {
                    channel.valeur = Json::Null;
                    channel.qualite = "catalogue_absent";
                }
                channel.enum_expected |= old.enum_expected;
            }
            fresh.channels.insert(key.clone(), channel);
            if !self.configured && fresh.channels[&key].enum_expected {
                fresh.restore_enum(&key)?;
            }
            if matches!(domain, "light" | "switch")
                && text(row, "platform") == "mqtt"
                && !matches!(text(row, "entity_category"), "config" | "diagnostic")
                && !text(row, "unique_id").is_empty()
                && text(row, "unique_id").len() <= 256
            {
                fresh
                    .command_bindings
                    .insert(key.clone(), format!("{domain}/{}", text(row, "unique_id")));
            }
            fresh.entities.insert(entity.into(), key);
        }
        // Include attributes learned since the last descriptor publication.
        // The old descriptor is only loaded while rebuilding, including in file mode.
        let mut previous = self.metadata.read()?;
        for (key, patch) in &self.metadata_patches {
            if previous["canaux"][key].is_null() {
                previous["canaux"][key] = json!({});
            }
            previous["canaux"][key]
                .as_object_mut()
                .context("invalid descriptor")?
                .extend(
                    patch
                        .as_object()
                        .context("invalid descriptor patch")?
                        .clone(),
                );
        }
        for old in self
            .channels
            .values()
            .filter(|channel| same_house && !channel.base)
        {
            let Some((entity, attribute)) = old.source.rsplit_once('/') else {
                continue;
            };
            let Some(base) = self
                .entities
                .get(entity)
                .and_then(|key| fresh.channels.get(key))
            else {
                continue;
            };
            if attribute == "app_name" || base.domaine != old.domaine {
                continue;
            }
            if fresh.channels.len() >= MAX_CHANNELS {
                fresh.rejected += 1;
                continue;
            }
            let mut channel = old.clone();
            channel.device.clone_from(&base.device);
            channel.piece.clone_from(&base.piece);
            channel.source = format!("{}/{attribute}", base.source);
            let label = previous["canaux"][&old.id]["nom"]
                .as_str()
                .unwrap_or(attribute_spec(attribute, base).0);
            fresh.labels.insert(channel.id.clone(), label.into());
            fresh
                .devices
                .get_mut(&channel.device)
                .expect("device exists")
                .channels
                .insert(channel.id.clone());
            fresh.channels.insert(channel.id.clone(), channel);
        }
        fresh.enum_pending.retain(|key, _| {
            fresh
                .channels
                .get(key)
                .zip(self.channels.get(key))
                .is_some_and(|(fresh, old)| fresh.domaine == old.domaine)
        });
        fresh.configured = true;
        fresh.config_dirty = true;
        fresh.dirty_nodes.extend(fresh.nodes.keys().cloned());
        fresh.dirty_devices.extend(fresh.devices.keys().cloned());
        fresh.dirty_channels.extend(fresh.channels.keys().cloned());
        fresh.command_channels.extend(
            fresh
                .command_bindings
                .iter()
                .filter(|(_, unique)| fresh.command_definitions.contains_key(*unique))
                .map(|(id, _)| id.clone()),
        );
        fresh.metadata = std::mem::take(&mut self.metadata);
        let mut description = fresh.descriptor();
        for channel in fresh.channels.values().filter(|channel| channel.base) {
            let catalogue = &previous["canaux"][&channel.id]["catalogue_sources"];
            if same_house
                && self
                    .channels
                    .get(&channel.id)
                    .is_some_and(|old| old.domaine == channel.domaine)
                && catalogue.is_array()
            {
                description["canaux"][&channel.id]["catalogue_sources"] = catalogue.clone();
            }
        }
        for (point, unique) in &fresh.command_bindings {
            if same_house
                && self.command_bindings.get(point) == Some(unique)
                && fresh.discovery_topics.values().any(|id| id == unique)
                && description["canaux"][point]["commande"].is_null()
                && previous["canaux"][point]["commande"].is_object()
            {
                description["canaux"][point]["commande"] =
                    previous["canaux"][point]["commande"].clone();
                fresh.command_channels.insert(point.clone());
            }
        }
        fresh
            .metadata
            .replace(description, fresh.use_file_metadata)?;
        fresh.release_labels();
        fresh.command_definitions.clear();
        for row in rows("entities")? {
            let entity = text(row, "entity_id");
            if let Some(base) = fresh.entities.get(entity).cloned() {
                for attribute in [
                    "options",
                    "operation_list",
                    "hvac_modes",
                    "source_list",
                    "applications",
                ] {
                    if !row[attribute].is_null() {
                        fresh.enum_attribute(&base, entity, attribute, &row[attribute], true)?;
                    }
                }
            }
        }
        *self = fresh;
        Ok(())
    }

    fn room(&self, native: &str) -> String {
        let key = id("piece", native);
        if self.nodes.contains_key(&key) {
            key
        } else {
            "sans-piece".into()
        }
    }

    fn insert_node(
        &mut self,
        key: String,
        name: String,
        kind: &'static str,
        parent: Option<String>,
        temperature: &str,
        humidity: &str,
    ) {
        self.labels.insert(key.clone(), name);
        self.nodes.insert(
            key.clone(),
            Node {
                id: key.clone(),
                genre: kind,
                parent,
                topic: format!("{}/noeud/{key}/state", self.root),
                ordre: None,
                temperature: temperature.into(),
                humidity: humidity.into(),
            },
        );
    }

    pub(super) fn ready(&self) -> bool {
        self.configured
    }

    fn has_pending(&self) -> bool {
        self.config_dirty || !self.dirty_channels.is_empty() || !self.dirty_nodes.is_empty()
    }

    fn channel_descriptor(&self, c: &Channel) -> Json {
        let command = self
            .command_bindings
            .get(&c.id)
            .and_then(|id| self.command_definitions.get(id));
        json!({"enumeration":c.enum_ref.map(|reference|json!({"catalogue":reference.id,"version":reference.version})),"enum_attendue":c.enum_expected,"categorie":match c.semantique.as_str() {"configuration"=>Some("config"),"diagnostic"=>Some("diagnostic"),_=>None},"id":c.id,"nom":self.labels.get(&c.id),"unite":c.unite,"semantique":c.semantique,"domaine":c.domaine,"piece":c.piece,"appareil":c.device,"source":c.source,"topic":format!("{}/canal/{}/state",self.root,c.id),"commande":command})
    }

    fn descriptor(&self) -> Json {
        let channels: BTreeMap<_, _> = self
            .channels
            .values()
            .map(|c| (c.id.clone(), self.channel_descriptor(c)))
            .collect();
        let devices:Vec<_>=self.devices.values().map(|d|json!({"id":d.id,"nom":self.labels.get(&d.id),"piece":d.piece,"description_topic":format!("{}/config",self.root),"genre":d.genre,"canaux":d.channels})).collect();
        json!({"v":3,"type":"description","nom":"Maison","pont":self.root,"source_id":self.source_id,"noeuds":self.nodes.values().map(|n|{let mut v=serde_json::to_value(n).expect("node descriptor");v["nom"]=json!(self.labels.get(&n.id));v}).collect::<Vec<_>>(),"appareils":devices,"canaux":channels,"catalogues":self.enum_descriptors(),"commandes_physiques":"uniquement_depuis_interface_humaine_et_contrats_source","stockage_description":if self.use_file_metadata {"fichier_prive"}else{"memoire"}})
    }

    fn release_labels(&mut self) {
        self.labels.clear();
    }

    fn patch_descriptor(&mut self, key: &str, patch: Json) {
        let Json::Object(fields) = patch else {
            return;
        };
        for (field, value) in fields {
            // Pending fields are the effective descriptor until publication.
            if self.metadata_patches.get(key).and_then(|p| p.get(&field)) == Some(&value) {
                continue;
            }
            let patch = Json::Object([(field.clone(), value)].into_iter().collect());
            // A cold read failure must remain visible to publications(), never
            // masquerade as an unchanged descriptor.
            if self.metadata.contains_patch(key, &patch).unwrap_or(false) {
                if let Some(target) = self
                    .metadata_patches
                    .get_mut(key)
                    .and_then(Json::as_object_mut)
                {
                    target.remove(&field);
                    if target.is_empty() {
                        self.metadata_patches.remove(key);
                    }
                }
                continue;
            }
            let target = self
                .metadata_patches
                .entry(key.into())
                .or_insert_with(|| json!({}));
            let Json::Object(patch) = patch else {
                unreachable!("patch object");
            };
            target
                .as_object_mut()
                .expect("descriptor patch object")
                .extend(patch);
            self.config_dirty = true;
        }
    }

    pub(super) fn discovery(&mut self, topic: &str, payload: &[u8]) -> bool {
        let Some((domain, _)) = topic
            .strip_prefix("homeassistant/")
            .and_then(|path| path.split_once('/'))
        else {
            return false;
        };
        if !matches!(domain, "light" | "switch")
            || !topic.ends_with("/config")
            || payload.len() > 65536
            || topic.len() > 1024
        {
            return false;
        }
        if payload.is_empty() {
            return self.revoke_discovery(topic);
        }
        let Ok(data) = serde_json::from_slice::<Json>(payload) else {
            self.revoke_discovery(topic);
            return false;
        };
        let description = if domain == "light" {
            crate::lighting::description(&data)
        } else {
            crate::switching::description(&data)
        };
        let Some((unique, command)) = description else {
            self.revoke_discovery(topic);
            return false;
        };
        let unique = format!("{domain}/{unique}");
        if (self.command_definitions.len() >= 2048
            && !self.command_definitions.contains_key(&unique))
            || (self.discovery_topics.len() >= 2048 && !self.discovery_topics.contains_key(topic))
        {
            self.rejected += 1;
            return false;
        }
        self.command_definitions
            .insert(unique.clone(), command.clone());
        if self
            .discovery_topics
            .get(topic)
            .is_some_and(|old| old != &unique)
        {
            self.revoke_discovery(topic);
        }
        self.discovery_topics.insert(topic.into(), unique.clone());
        let matches: Vec<_> = self
            .command_bindings
            .iter()
            .filter(|(_, id)| **id == unique)
            .map(|(id, _)| id.clone())
            .collect();
        for id in matches {
            self.command_channels.insert(id.clone());
            self.patch_descriptor(&id, json!({"commande":command}));
        }
        if self.ready() {
            self.command_definitions.remove(&unique);
        }
        true
    }

    fn revoke_discovery(&mut self, topic: &str) -> bool {
        let Some(unique) = self.discovery_topics.remove(topic) else {
            return false;
        };
        self.command_definitions.remove(&unique);
        let points: Vec<_> = self
            .command_bindings
            .iter()
            .filter(|(_, id)| **id == unique)
            .map(|(id, _)| id.clone())
            .collect();
        for point in points {
            self.command_channels.remove(&point);
            self.patch_descriptor(&point, json!({"commande":Json::Null}));
        }
        true
    }

    pub(super) fn ingest(&mut self, topic: &str, payload: &[u8], retained: bool) -> bool {
        let Some(rest) = topic.strip_prefix("homeassistant/statestream/") else {
            return false;
        };
        if payload.len() > MAX_SCHEMA {
            self.rejected += 1;
            return false;
        }
        let parts: Vec<_> = rest.split('/').collect();
        let [domain, object, attribute] = parts.as_slice() else {
            return false;
        };
        let list = matches!(
            *attribute,
            "options"
                | "operation_list"
                | "hvac_modes"
                | "source_list"
                | "applications"
                | "application_list"
                | "apps"
        );
        if payload.len() > if list { MAX_SCHEMA } else { MAX_TEXT } {
            self.rejected += 1;
            return false;
        }
        let entity = format!("{domain}.{object}");
        let Some(base_id) = self.entities.get(&entity).cloned() else {
            self.unmatched += 1;
            return false;
        };
        let is_state = *attribute == "state";
        let allowed = matches!(
            *attribute,
            "friendly_name"
                | "unit_of_measurement"
                | "device_class"
                | "current_temperature"
                | "current_humidity"
                | "temperature"
                | "target_temp_low"
                | "target_temp_high"
                | "hvac_action"
                | "brightness"
                | "current_position"
                | "power"
                | "battery_level"
                | "volume_level"
                | "media_title"
                | "app_id"
                | "app_name"
                | "source"
                | "media_artist"
                | "media_album_name"
                | "media_duration"
                | "media_position"
                | "media_position_updated_at"
                | "is_volume_muted"
                | "supported_features"
                | "source_list"
                | "options"
                | "operation_list"
                | "operation_mode"
                | "hvac_modes"
                | "applications"
                | "application_list"
                | "apps"
                | "hs_color"
                | "xy_color"
                | "rgb_color"
                | "color_temp_kelvin"
                | "color_temp"
                | "color_mode"
        );
        if !is_state && !allowed {
            return false;
        }
        let Ok(raw) = std::str::from_utf8(payload) else {
            self.rejected += 1;
            return false;
        };
        let mut value: Json = if is_state && self.channels[&base_id].enum_expected {
            Json::String(raw.into())
        } else {
            serde_json::from_str(raw).unwrap_or_else(|_| Json::String(raw.into()))
        };
        if self.enum_native_expected(&base_id, attribute)
            && !value.is_null()
            && !value.is_string()
            && !value.is_array()
            && !value.is_object()
        {
            value = Json::String(raw.into());
        }
        match self.enum_attribute(&base_id, &entity, attribute, &value, retained) {
            Ok(Some(applied)) => return applied,
            Ok(None) => {}
            Err(_) => {
                self.rejected += 1;
                return false;
            }
        }

        if matches!(*attribute, "hs_color" | "xy_color" | "rgb_color") {
            let Some(components) =
                crate::lighting::color_components(attribute, &value).filter(|_| *domain == "light")
            else {
                self.rejected += 1;
                return false;
            };
            let missing = components
                .iter()
                .filter(|(attribute, _)| {
                    !self
                        .channels
                        .contains_key(&format!("{base_id}-{}", id("attribut", attribute)))
                })
                .count();
            if self.channels.len() + missing > MAX_CHANNELS {
                self.rejected += 1;
                return false;
            }
            for (attribute, value) in components {
                self.record_attribute(&base_id, &entity, attribute, value, retained);
            }
            return true;
        }
        let value = if value.is_array() || value.is_object() {
            self.rejected += 1;
            return false;
        } else {
            value
        };
        if matches!(
            *attribute,
            "friendly_name" | "unit_of_measurement" | "device_class"
        ) {
            let base = self.channels.get_mut(&base_id).expect("known entity");
            if let Some(s) = value.as_str() {
                match *attribute {
                    "friendly_name" => {}
                    "unit_of_measurement" => base.unite = Some(s.into()),
                    "device_class"
                        if !matches!(
                            base.semantique.as_str(),
                            "diagnostic" | "configuration" | "consigne_chauffage"
                        ) =>
                    {
                        base.semantique = semantic(domain, s, false).into()
                    }
                    _ => {}
                }
            }
            let patch = match *attribute {
                "friendly_name" => json!({"nom":value}),
                "unit_of_measurement" => json!({"unite":value}),
                _ => json!({"semantique":base.semantique}),
            };
            self.dirty_devices.insert(base.device.clone());
            self.patch_descriptor(&base_id, patch);
            if *attribute == "device_class"
                && value.as_str() == Some("enum")
                && self.expect_enum(&base_id).is_err()
            {
                self.rejected += 1;
                return false;
            }

            self.mark_room(&base_id);
            return true;
        }
        self.record_attribute(&base_id, &entity, attribute, value, retained)
    }

    fn record_attribute(
        &mut self,
        base_id: &str,
        entity: &str,
        attribute: &str,
        value: Json,
        retained: bool,
    ) -> bool {
        let Some(key) = self.ensure_channel(base_id, entity, attribute) else {
            return false;
        };
        let received = now_ms();
        let enum_input = if self.channels[&key].enum_expected {
            Some(self.enum_value(&key, &value, retained))
        } else {
            None
        };
        let (value, enum_quality) = match enum_input {
            Some((value, quality)) => (value, Some(quality)),
            None => (value, None),
        };
        if let Some(pending) = self.enum_pending.get_mut(&key) {
            pending.received = received;
        }
        let channel = self.channels.get_mut(&key).expect("channel exists");
        let missing = value.is_null()
            || value
                .as_str()
                .is_some_and(|s| matches!(s, "unknown" | "unavailable" | "None" | "null"));
        let quality = if missing {
            "inconnu"
        } else if retained {
            "retained_age_inconnu"
        } else {
            "observe"
        };
        let quality = enum_quality.unwrap_or(quality);
        let next = if missing { Json::Null } else { value };
        let changed = channel.valeur != next || channel.qualite != quality;
        channel.qualite = quality;
        channel.retenu = retained;
        channel.recu_ms = Some(received);
        channel.valeur = next;
        self.dirty_devices.insert(channel.device.clone());
        if changed {
            self.dirty_channels.insert(key.clone());
            self.mark_room(&key);
        }
        true
    }

    fn ensure_channel(&mut self, base_id: &str, entity: &str, attribute: &str) -> Option<String> {
        let is_state = attribute == "state";
        let key = if is_state {
            base_id.to_owned()
        } else {
            format!("{base_id}-{}", id("attribut", attribute))
        };
        if !self.channels.contains_key(&key) {
            if self.channels.len() >= MAX_CHANNELS {
                self.rejected += 1;
                return None;
            }
            let base = self.channels.get(base_id).expect("base channel").clone();
            let (name, mut unit, kind) = attribute_spec(attribute, &base);
            if matches!(
                attribute,
                "current_temperature" | "temperature" | "target_temp_low" | "target_temp_high"
            ) && unit.is_none()
            {
                unit = self.temperature_unit.clone();
            }
            self.labels.insert(key.clone(), name.into());
            self.channels.insert(
                key.clone(),
                Channel {
                    id: key.clone(),
                    unite: unit,
                    semantique: kind.into(),
                    source: format!("{entity}/{attribute}"),
                    base: false,
                    valeur: Json::Null,
                    qualite: "absent",
                    enum_ref: None,
                    enum_cache_checked: false,
                    enum_expected: matches!(attribute, "source" | "operation_mode" | "app_id"),
                    ..base.clone()
                },
            );
            self.devices
                .get_mut(&base.device)
                .expect("device exists")
                .channels
                .insert(key.clone());
            let descriptor = self.channel_descriptor(&self.channels[&key]);
            self.patch_descriptor(&key, descriptor);
            self.labels.remove(&key);
        }
        Some(key)
    }

    fn mark_room(&mut self, channel: &str) {
        let mut current = self.channels.get(channel).map(|c| c.piece.clone());
        while let Some(key) = current {
            self.dirty_nodes.insert(key.clone());
            current = self.nodes.get(&key).and_then(|n| n.parent.clone());
        }
    }

    fn includes(&self, scope: &str, room: &str) -> bool {
        let mut current = Some(room);
        while let Some(key) = current {
            if key == scope {
                return true;
            }
            current = self.nodes.get(key).and_then(|n| n.parent.as_deref());
        }
        false
    }

    fn generics(&self, scope: &str) -> Vec<Json> {
        let started = Instant::now();
        let result = self.build_generics(scope);
        let mut timing = self.calculation.get();
        timing.record(started.elapsed());
        self.calculation.set(timing);
        result
    }

    fn build_generics(&self, scope: &str) -> Vec<Json> {
        let channels: Vec<_> = self
            .channels
            .values()
            .filter(|c| !c.enum_expected && self.includes(scope, &c.piece))
            .collect();
        let mut groups: BTreeMap<(String, Option<String>), Vec<&Channel>> = BTreeMap::new();
        for c in channels {
            let node = &self.nodes[&c.piece];
            let op = match c.semantique.as_str() {
                "temperature" | "humidite" => "moyenne",
                "ouverture" | "presence" | "lumiere" | "chauffage_actif" => "nombre_actifs",
                _ => continue,
            };
            if op == "moyenne" && c.unite.is_none() {
                continue;
            }
            if c.semantique == "temperature"
                && !node.temperature.is_empty()
                && c.source != node.temperature
            {
                continue;
            }
            if c.semantique == "temperature"
                && node.temperature.is_empty()
                && self
                    .channels
                    .values()
                    .any(|other| other.device == c.device && other.domaine == "water_heater")
            {
                continue;
            }
            if c.semantique == "humidite" && !node.humidity.is_empty() && c.source != node.humidity
            {
                continue;
            }
            // A climate's current_temperature is a fallback for a physical
            // device which has no dedicated ambient sensor channel.
            if !c.base
                && c.semantique == "temperature"
                && self.channels.values().any(|other| {
                    other.base && other.device == c.device && other.semantique == "temperature"
                })
            {
                continue;
            }
            groups
                .entry((c.semantique.clone(), c.unite.clone()))
                .or_default()
                .push(c);
        }
        groups.into_iter().map(|((kind,unit),sources)| {
            let numeric=matches!(kind.as_str(),"temperature"|"humidite");
            let values:Vec<f64>=sources.iter().filter_map(|c| {
                if numeric {number(&c.valeur)} else {active(c).map(|b|if b {1.0}else{0.0})}
            }).collect();
            let sum:f64=values.iter().sum();
            let value=if values.is_empty(){Json::Null}else if numeric {json!(sum/values.len() as f64)}else{json!(sum as u64)};
            let label=match kind.as_str(){"temperature"=>"Température","humidite"=>"Humidité","ouverture"=>"Portes et fenêtres ouvertes","presence"=>"Présences détectées","lumiere"=>"Lumières allumées","chauffage_actif"=>"Chauffages actifs",_=>"Autre"};
            json!({"id":format!("{kind}:{}",unit.as_deref().unwrap_or("")),"nom":label,"semantique":kind,"unite":unit,"valeur":value,"operation":if numeric {"moyenne"}else{"nombre_actifs"},"sources":sources.len(),"connues":values.len(),"inconnues":sources.len()-values.len(),"retenues":sources.iter().filter(|c|c.retenu).count(),"provenance":sources.iter().map(|c|&c.id).collect::<Vec<_>>()})
        }).collect()
    }

    pub(super) fn publications(&mut self) -> Result<Vec<(String, Json)>> {
        self.expire_enum_pending();
        self.compact_enums_after_publication();
        self.persist_enums()?;
        let mut out = Vec::new();
        if self.config_dirty {
            let mut metadata = self.metadata.read()?;
            let previous = metadata.clone();
            for (id, patch) in &self.metadata_patches {
                if metadata["canaux"][id].is_null() {
                    metadata["canaux"][id] = json!({});
                }
                metadata["canaux"][id]
                    .as_object_mut()
                    .context("invalid descriptor")?
                    .extend(
                        patch
                            .as_object()
                            .context("invalid descriptor patch")?
                            .clone(),
                    );
            }
            let generics: BTreeMap<_, _> = self
                .nodes
                .keys()
                .map(|key| {
                    (
                        key.clone(),
                        self.generics(key)
                            .into_iter()
                            .map(|mut g| {
                                let obj = g.as_object_mut().expect("generic object");
                                for field in ["valeur", "connues", "inconnues", "retenues"] {
                                    obj.remove(field);
                                }
                                g
                            })
                            .collect::<Vec<_>>(),
                    )
                })
                .collect();
            metadata["catalogues"] = self.enum_descriptors();
            metadata["generiques"] = json!(generics);
            metadata["systeme"] = system_descriptors();
            for device in metadata["appareils"].as_array_mut().into_iter().flatten() {
                if let Some(runtime) = device["id"].as_str().and_then(|id| self.devices.get(id)) {
                    device["canaux"] = json!(runtime.channels);
                }
            }
            if let Some(nodes) = metadata["noeuds"].as_array_mut() {
                for node in nodes {
                    let scope = node["id"].as_str().unwrap_or("");
                    node["appareils"] = json!(
                        self.devices
                            .values()
                            .filter(|d| d.channels.iter().any(|key| self
                                .channels
                                .get(key)
                                .is_some_and(|c| self.includes(scope, &c.piece)))
                                || d.channels.is_empty() && self.includes(scope, &d.piece))
                            .map(|d| &d.id)
                            .collect::<Vec<_>>()
                    );
                }
            }
            if metadata != previous {
                self.metadata_revision += 1;
                metadata["revision"] = json!(self.metadata_revision);
                self.metadata
                    .replace(metadata.clone(), self.use_file_metadata)?;
                out.push((format!("{}/config", self.root), metadata));
            }
            self.metadata_patches.clear();
            self.config_dirty = false;
        }
        self.dirty_devices.clear();
        for key in std::mem::take(&mut self.dirty_channels) {
            let c = &self.channels[&key];
            let mut state = json!({"v":3,"type":"canal","id":c.id,"description":self.metadata_revision,"valeur":c.valeur,"qualite":c.qualite,"retenu":c.retenu,"recu_ms":c.recu_ms,"pont":self.root});
            if c.enum_expected {
                state["encodage"] = json!("enum");
                state["catalogue"] = json!(c.enum_ref.map(|r| r.id));
                state["catalogue_version"] = json!(c.enum_ref.map(|r| r.version));
            }
            out.push((format!("{}/canal/{}/state", self.root, c.id), state));
        }
        for key in std::mem::take(&mut self.dirty_nodes) {
            let n = &self.nodes[&key];
            let generics:Vec<_>=self.generics(&key).into_iter().map(|g|json!({"id":g["id"],"valeur":g["valeur"],"connues":g["connues"],"inconnues":g["inconnues"],"retenues":g["retenues"]})).collect();
            out.push((n.topic.clone(),json!({"v":3,"type":"noeud","id":n.id,"description":self.metadata_revision,"generiques":generics,"pont":self.root})));
        }
        self.enum_compact_ready = true;
        Ok(out)
    }

    pub(super) fn counts(&self) -> Json {
        let mut counts = json!({"nodes":self.nodes.len(),"devices":self.devices.values().filter(|d|d.genre=="physique").count(),"standalone_entities":self.devices.values().filter(|d|d.genre=="entite").count(),"channels":self.channels.len(),"observed_channels":self.channels.values().filter(|c|c.recu_ms.is_some()).count(),"light_command_contracts":self.command_channels.iter().filter(|id|self.channels.get(*id).is_some_and(|c|c.domaine=="light")).count(),"switch_command_contracts":self.command_channels.iter().filter(|id|self.channels.get(*id).is_some_and(|c|c.domaine=="switch")).count(),"metadata_bytes":self.metadata.bytes(),"active_channel_struct_bytes":std::mem::size_of::<Channel>(),"hot_label_bytes":self.labels.values().map(String::len).sum::<usize>(),"unmatched_messages":self.unmatched,"rejected":self.rejected});
        counts["enum_catalogue"] = self.enum_pool.stats();
        counts["enum_channels"] = json!(self.channels.values().filter(|c| c.enum_expected).count());
        counts["enum_code_bytes"] = json!(
            self.channels
                .values()
                .filter(|c| c.enum_expected && c.valeur.as_u64().is_some())
                .count()
        );
        counts["enum_json_storage_bytes"] = json!(
            self.channels.values().filter(|c| c.enum_expected).count()
                * std::mem::size_of::<Json>()
        );
        counts["pending_native_bytes"] = json!(
            self.enum_pending
                .values()
                .map(|p| p.native.len())
                .sum::<usize>()
        );
        counts["text_values_utf8_bytes"] = json!(
            self.channels
                .values()
                .filter_map(|c| c.valeur.as_str())
                .map(str::len)
                .sum::<usize>()
        );
        counts
    }
}

fn attribute_spec<'a>(attribute: &'a str, base: &Channel) -> (&'a str, Option<String>, &'a str) {
    match attribute {
        "color_h" => ("Teinte", Some("°".into()), "color_h"),
        "color_s" => ("Saturation", Some("%".into()), "color_s"),
        "color_x" => ("Chromaticité x", Some("coord".into()), "color_x"),
        "color_y" => ("Chromaticité y", Some("coord".into()), "color_y"),
        "color_r" => ("Rouge", Some("/255".into()), "color_r"),
        "color_g" => ("Vert", Some("/255".into()), "color_g"),
        "color_b" => ("Bleu", Some("/255".into()), "color_b"),
        "color_temp_kelvin" => (
            "Température de couleur",
            Some("K".into()),
            "temperature_couleur_kelvin",
        ),
        "color_temp" => (
            "Température de couleur",
            Some("mired".into()),
            "temperature_couleur",
        ),
        "color_mode" => ("Mode de couleur", None, "mode_couleur"),
        "current_temperature" => ("Température mesurée", base.unite.clone(), "temperature"),
        "temperature" => ("Consigne", base.unite.clone(), "consigne_chauffage"),
        "current_humidity" => ("Humidité mesurée", Some("%".into()), "humidite"),
        "hvac_action" => ("Activité du chauffage", None, "chauffage_actif"),
        "brightness" => ("Luminosité", Some("/255".into()), "intensite"),
        "current_position" => ("Position", Some("%".into()), "position"),
        "battery_level" => ("Batterie", Some("%".into()), "batterie"),
        "volume_level" => ("Volume", Some("/1".into()), "volume"),
        "media_duration" => ("Durée", Some("s".into()), "media_duration"),
        "media_position" => ("Position de lecture", Some("s".into()), "media_position"),
        "app_id" => ("Identifiant de l’application", None, "app_id"),
        "app_name" => ("Application", None, "app_name"),
        "media_title" => ("Titre", None, "media_title"),
        _ => (attribute, None, "attribut"),
    }
}

fn number(value: &Json) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.parse().ok())
        .filter(|n| n.is_finite())
}

fn active(channel: &Channel) -> Option<bool> {
    if channel.valeur.is_null() {
        return None;
    }
    if channel.semantique == "chauffage_actif" {
        return match channel.valeur.as_str()? {
            "heating" | "preheating" => Some(true),
            "idle" | "off" | "cooling" | "fan" | "drying" => Some(false),
            _ => None,
        };
    }
    match &channel.valeur {
        Json::Bool(value) => Some(*value),
        Json::String(s) => match s.as_str() {
            "on" | "ON" | "open" | "true" => Some(true),
            "off" | "OFF" | "closed" | "false" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

#[derive(Default, Serialize, Clone, Copy)]
struct Timing {
    samples: u64,
    total_ns: u128,
    max_ns: u128,
}

fn system_descriptors() -> Json {
    json!([
        {"id":"messages_recus","nom":"Messages reçus","unite":"messages"},{"id":"messages_retenus","nom":"Messages retained reçus","unite":"messages"},
        {"id":"messages_publies","nom":"Messages publiés","unite":"messages"},{"id":"messages_par_seconde","nom":"Débit reçu moyen","unite":"messages/s"},
        {"id":"transformation_moyenne","nom":"Transformation moyenne","unite":"µs"},{"id":"calcul_moyen","nom":"Calcul des génériques moyen","unite":"µs"},{"id":"preparation_moyenne","nom":"Préparation des publications moyenne","unite":"µs"},
        {"id":"reponse_moyenne","nom":"Aller-retour MQTT moyen","unite":"µs"},{"id":"echos_recus","nom":"Échos MQTT reçus","unite":"messages"},
        {"id":"echos_en_attente","nom":"Échos en attente","unite":"messages"},{"id":"messages_ecartes","nom":"Messages écartés","unite":"messages"},
        {"id":"canaux","nom":"Canaux décrits","unite":"canaux"},{"id":"canaux_observes","nom":"Canaux reçus","unite":"canaux"}])
}
impl Timing {
    fn record(&mut self, d: Duration) {
        self.samples += 1;
        self.total_ns += d.as_nanos();
        self.max_ns = self.max_ns.max(d.as_nanos());
    }
    fn mean_us(&self) -> Option<f64> {
        (self.samples > 0).then(|| self.total_ns as f64 / self.samples as f64 / 1000.0)
    }
}

/// Every bridge has its own namespace, including on a shared destination broker.
/// This proof mode projects HA statestream; it never routes physical commands.
pub(super) async fn run(live: &super::Live, ctx: &mut moli_runtime::DriverCtx) -> Result<()> {
    use rumqttc::{AsyncClient, Event, Packet, QoS};
    let prefix = live
        .config
        .republish
        .as_deref()
        .context("house projection requires --republish")?;
    let mut root = format!("{prefix}/pont/{}", live.instance);
    let mut house = House::new(root.clone());
    house.source_id = live.instance.to_string();
    house.use_file_metadata = !live.config.metadata_memory;
    super::Metrics::lock(&live.metrics).house_projection = true;
    let mut options = rumqttc::MqttOptions::new(&live.client_id, &live.host, live.config.port);
    options.set_credentials(&live.config.username, &live.password);
    options.set_max_packet_size(super::MAX_PACKET, super::MAX_PACKET);
    options.set_keep_alive(Duration::from_secs(15));
    let (client, mut events) = AsyncClient::new(options, 128);
    // Poll continuously: cancelling a poll during a large descriptor flush
    // can prevent progress. Application ticks must not interrupt network I/O.
    let (incoming, mut packets) = tokio::sync::mpsc::channel(256);
    let destination_incoming = incoming.clone();
    let mut network = tokio::task::JoinSet::new();
    network.spawn(async move {
        loop {
            let result = events.poll().await;
            let failed = result.is_err();
            if incoming.send((false, result)).await.is_err() || failed {
                break;
            }
        }
    });
    let registry_topic = if live.config.destination.is_some() {
        format!("{prefix}/source/{}/registry", live.instance)
    } else {
        format!("{prefix}/source/registry")
    };
    let snapshot_topic = format!("{prefix}/source/{}/snapshot", live.instance);
    let publisher = if let Some(destination) = &live.config.destination {
        let (host, port) = destination
            .rsplit_once(':')
            .context("invalid destination")?;
        let mut options = rumqttc::MqttOptions::new(
            format!("moli-poc-{}", live.client_id),
            host,
            port.parse::<u16>()?,
        );
        options.set_credentials(&live.config.username, &live.password);
        options.set_max_packet_size(super::MAX_PACKET, super::MAX_PACKET);
        let (output, mut output_events) = AsyncClient::new(options, 128);
        network.spawn(async move {
            loop {
                let result = output_events.poll().await;
                let failed = result.is_err();
                if destination_incoming.send((true, result)).await.is_err() || failed {
                    break;
                }
            }
        });
        output
    } else {
        client.clone()
    };
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    let mut early: BTreeMap<String, (Vec<u8>, bool)> = BTreeMap::new();
    let mut pending: BTreeMap<String, Json> = BTreeMap::new();
    let mut last: BTreeMap<String, String> = BTreeMap::new();
    let mut tracker = super::RepublishTracker::new();
    let mut transform = Timing::default();
    let mut compute = Timing::default();
    let mut response = Timing::default();
    let start = Instant::now();
    let mut metrics_at = Instant::now() - Duration::from_secs(2);
    let (mut received, mut retained, mut published, mut echoes, mut seq, mut dropped) =
        (0_u64, 0_u64, 0_u64, 0_u64, 0_u64, 0_u64);
    loop {
        tokio::select! {
            ()=ctx.cancelled()=>{
                let mut metrics=super::Metrics::lock(&live.metrics);
                metrics.messages=received;metrics.retained=retained;metrics.republished=published;
                metrics.republish_roundtrips=echoes;metrics.republish_unanswered=tracker.unanswered();
                metrics.mirror_topics=house.devices.len();metrics.virtuals=house.nodes.len();
                // Count diagnostics only. Private metadata is never printed.
                println!("HOUSE_SUMMARY {}",house.counts());
                return Ok(());
            },
            _=tick.tick()=>{
                {
                    let mut metrics=super::Metrics::lock(&live.metrics);
                    metrics.messages=received;metrics.retained=retained;metrics.republished=published;
                    metrics.republish_roundtrips=echoes;metrics.republish_unanswered=tracker.unanswered();
                    metrics.house_counts=house.counts();
                    metrics.mirror_topics=house.devices.len();metrics.virtuals=house.nodes.len();
                }
                if house.ready() {
                    let had_work=house.has_pending();
                    let started=Instant::now();
                    for (topic,payload) in house.publications()? {
                        if payload["type"]=="description" {house.hub_schema(ctx,&payload);}
                        house.hub_publication(ctx,&payload);
                        let raw=payload.to_string();
                        if payload["type"]=="description"||payload["type"]=="canal" {
                            pending.insert(topic,payload);
                        }else if last.get(&topic)!=Some(&raw) {
                            last.insert(topic.clone(),raw);pending.insert(topic,payload);
                        }
                    }
                    if had_work { compute.record(started.elapsed()); }
                    if metrics_at.elapsed()>=Duration::from_secs(1) {
                        metrics_at=Instant::now();
                        let gauges=vec![
                            json!({"id":"messages_recus","nom":"Messages reçus","unite":"messages","valeur":received}),
                            json!({"id":"messages_retenus","nom":"Messages retained reçus","unite":"messages","valeur":retained}),
                            json!({"id":"messages_publies","nom":"Messages publiés","unite":"messages","valeur":published}),
                            json!({"id":"messages_par_seconde","nom":"Débit reçu moyen","unite":"messages/s","valeur":received as f64/start.elapsed().as_secs_f64().max(0.001)}),
                            json!({"id":"transformation_moyenne","nom":"Transformation moyenne","unite":"µs","valeur":transform.mean_us(),"echantillons":transform.samples}),
                            json!({"id":"calcul_moyen","nom":"Calcul des génériques moyen","unite":"µs","valeur":house.calculation.get().mean_us(),"echantillons":house.calculation.get().samples}),
                            json!({"id":"preparation_moyenne","nom":"Préparation des publications moyenne","unite":"µs","valeur":compute.mean_us(),"echantillons":compute.samples}),
                            json!({"id":"reponse_moyenne","nom":"Aller-retour MQTT moyen","unite":"µs","valeur":response.mean_us(),"echantillons":response.samples}),
                            json!({"id":"echos_recus","nom":"Échos MQTT reçus","unite":"messages","valeur":echoes}),
                            json!({"id":"echos_en_attente","nom":"Échos en attente (fenêtre bornée)","unite":"messages","valeur":tracker.unanswered()}),
                            json!({"id":"messages_ecartes","nom":"Messages écartés par les bornes","unite":"messages","valeur":dropped+house.rejected}),
                            json!({"id":"canaux","nom":"Canaux décrits","unite":"canaux","valeur":house.channels.len()}),
                            json!({"id":"canaux_observes","nom":"Canaux reçus","unite":"canaux","valeur":house.channels.values().filter(|c|c.recu_ms.is_some()).count()}),
                        ];
                        let gauges:Vec<_>=gauges.into_iter().map(|mut g|{let obj=g.as_object_mut().expect("metric object");obj.remove("nom");obj.remove("unite");g}).collect();
                        pending.insert(format!("{root}/systeme/state"),json!({"v":3,"type":"systeme","id":"pont","pont":root,"canaux":gauges,"compteurs":house.counts(),"mesure":"horloge_monotone_locale; aller_retour_broker_pas_appareil","ts":now_ms()}));
                    }
                }
                // Bounded queue; coalesce retained snapshots per topic during
                // the initial replay instead of overflowing the MQTT client.
                for _ in 0..32 {
                    let priority=pending.keys().find(|topic|topic.ends_with("/config"))
                        .or_else(||pending.keys().find(|topic|topic.ends_with("/systeme/state")))
                        .or_else(||pending.keys().find(|topic|topic.contains("/noeud/"))).cloned();
                    let entry=if let Some(topic)=priority { pending.remove(&topic).map(|payload|(topic,payload)) } else {pending.pop_first()};
                    let Some((topic,mut payload))=entry else {break};
                    seq+=1;
                    payload["seq"]=json!(seq);
                    if publisher.try_publish(topic.clone(),QoS::AtLeastOnce,true,payload.to_string()).is_err(){
                        // The local MQTT request queue can fill while the
                        // retained bootstrap is arriving. Keep the last state
                        // per topic and retry after network polling progresses.
                        payload.as_object_mut().expect("publication object").remove("seq");
                        pending.insert(topic,payload);break;
                    }
                    tracker.mark_sent(seq,Instant::now());
                    published+=1;
                }
            },
            event=packets.recv()=>{
                let (destination_event,event)=event.context("house MQTT network task stopped")?;
                match event.context("house MQTT transport failed")? {
                    Event::Incoming(Packet::ConnAck(_))=>{
                        let mut topics=Vec::new();
                        if !destination_event {topics.extend(["homeassistant/statestream/#".to_owned(),"homeassistant/light/#".to_owned(),"homeassistant/switch/#".to_owned()]);}
                        if destination_event||live.config.destination.is_none(){topics.extend([registry_topic.clone(),snapshot_topic.clone(),format!("{prefix}/maison/#"),format!("{root}/#")]);}
                        let target=if destination_event{&publisher}else{&client};
                        for topic in topics {target.try_subscribe(topic,QoS::AtLeastOnce).context("house subscription failed")?;
                        }
                        if !destination_event {super::Metrics::lock(&live.metrics).connected=true;ctx.ready();}
                    },
                    Event::Incoming(Packet::SubAck(ack))=>{
                        ensure!(ack.return_codes.iter().all(|c|matches!(c,rumqttc::SubscribeReasonCode::Success(_))),"house subscription refused");
                    },
                    Event::Incoming(Packet::Publish(p))=>{
                        if p.topic.starts_with(&format!("{root}/"))||p.topic.starts_with(&format!("{prefix}/maison/")) {
                            // Retained data from a previous process cannot be an echo.
                            if p.topic.starts_with(&format!("{root}/"))&&!p.retain && let Some(d)=tracker.observe(&p.payload,Instant::now()) {response.record(d);echoes+=1;}
                            continue;
                        }
                        received+=1;retained+=u64::from(p.retain);
                        if p.topic==registry_topic {
                            let started=Instant::now();house.registry(&p.payload)?;transform.record(started.elapsed());
                            root=house.root.clone();
                            for (topic,(payload,retained)) in std::mem::take(&mut early) {
                                let started=Instant::now();if !house.discovery(&topic,&payload){house.ingest(&topic,&payload,retained);}transform.record(started.elapsed());
                            }
                            continue;
                        }
                        if p.topic==snapshot_topic {
                            let started=Instant::now();
                            if !house.ingest_snapshot_payload(&p.payload,p.retain) {dropped+=1;}
                            transform.record(started.elapsed());
                            continue;
                        }
                        if !house.ready() {
                            let limit=if p.topic.starts_with("homeassistant/light/")||p.topic.starts_with("homeassistant/switch/"){65536}else{MAX_TEXT};
                            if p.payload.len()<=limit && early.len()<12000 {early.insert(p.topic,(p.payload.to_vec(),p.retain));} else {dropped+=1;}
                        }else{
                            let started=Instant::now();if !house.discovery(&p.topic,&p.payload){house.ingest(&p.topic,&p.payload,p.retain);}transform.record(started.elapsed());
                        }
                    },
                    _=>{},
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Json {
        json!({
        "floors":[{"floor_id":"f1","name":"Niveau principal"}],
        "areas":[{"id":"r1","name":"Séjour","floor_id":"f1"},{"id":"r2","name":"Atelier","floor_id":null}],
        "devices":[{"id":"d1","name":"Multicapteur","area_id":"r1"}],
        "entities":[
            {"id":"c1","entity_id":"sensor.ambient_temperature","device_id":"d1","original_device_class":"temperature","unit_of_measurement":"°C"},
            {"id":"c2","entity_id":"sensor.ambient_humidity","device_id":"d1","original_device_class":"humidity","unit_of_measurement":"%"},
            {"id":"c3","entity_id":"binary_sensor.window","device_id":"d1","original_device_class":"window"},
            {"id":"c4","entity_id":"sensor.remote_temperature","device_id":"d1","area_id":"r2","original_device_class":"temperature","unit_of_measurement":"°F"},
            {"id":"c5","entity_id":"climate.radiator","device_id":"d1"}
        ]})
    }
    fn house() -> House {
        let mut h = House::new("moli-probe/pont/a".into());
        h.registry(&serde_json::to_vec(&fixture()).unwrap())
            .unwrap();
        h
    }
    #[test]
    fn repeated_statestream_metadata_never_dirties_the_descriptor() {
        for file in [false, true] {
            let mut h = House::new("moli-probe/pont/fixture".into());
            h.use_file_metadata = file;
            h.registry(&serde_json::to_vec(&fixture()).unwrap())
                .unwrap();
            h.publications().unwrap();
            let revision = h.metadata_revision;
            let initial = h.metadata.read().unwrap();
            for (attribute, value) in [
                ("friendly_name", "\"sensor.ambient_temperature\""),
                ("unit_of_measurement", "\"°C\""),
                ("device_class", "\"temperature\""),
            ] {
                send(&mut h, "sensor.ambient_temperature", attribute, value);
                assert!(!h.config_dirty, "unchanged {attribute}, file={file}");
                assert!(h.metadata_patches.is_empty());
                assert!(
                    !h.publications()
                        .unwrap()
                        .iter()
                        .any(|(topic, _)| topic.ends_with("/config"))
                );
                assert_eq!(h.metadata_revision, revision);
            }
            for value in ["21", "22"] {
                send(&mut h, "sensor.ambient_temperature", "state", value);
                send(
                    &mut h,
                    "sensor.ambient_temperature",
                    "friendly_name",
                    "\"sensor.ambient_temperature\"",
                );
                assert!(!h.config_dirty, "state echo dirtied metadata, file={file}");
                let publications = h.publications().unwrap();
                assert!(
                    publications
                        .iter()
                        .any(|(_, payload)| payload["type"] == "canal")
                );
                assert!(
                    !publications
                        .iter()
                        .any(|(topic, _)| topic.ends_with("/config"))
                );
                assert_eq!(h.metadata_revision, revision);
            }
            assert_eq!(h.metadata.read().unwrap(), initial);
            send(
                &mut h,
                "sensor.ambient_temperature",
                "friendly_name",
                "\"Température ambiante\"",
            );
            assert!(h.config_dirty);
            let publications = h.publications().unwrap();
            assert_eq!(
                publications
                    .iter()
                    .filter(|(topic, _)| topic.ends_with("/config"))
                    .count(),
                1
            );
            assert_eq!(h.metadata_revision, revision + 1);
            assert!(h.labels.is_empty());
            send(
                &mut h,
                "sensor.ambient_temperature",
                "friendly_name",
                "\"Température ambiante\"",
            );
            assert!(!h.config_dirty);
            assert!(
                !h.publications()
                    .unwrap()
                    .iter()
                    .any(|(topic, _)| topic.ends_with("/config"))
            );
            assert_eq!(h.metadata_revision, revision + 1);
        }
    }

    #[test]
    fn descriptor_patches_compare_the_pending_overlay_and_keep_only_changes() {
        for file in [false, true] {
            let mut h = House::new("moli-probe/pont/fixture".into());
            h.use_file_metadata = file;
            h.registry(&serde_json::to_vec(&fixture()).unwrap())
                .unwrap();
            h.publications().unwrap();
            let key = id("canal", "c1");
            h.patch_descriptor(&key, json!({"nom":"Renommé","unite":"°C"}));
            assert_eq!(h.metadata_patches[&key], json!({"nom":"Renommé"}));
            h.patch_descriptor(&key, json!({"nom":"Renommé","unite":"°F"}));
            assert_eq!(
                h.metadata_patches[&key],
                json!({"nom":"Renommé","unite":"°F"})
            );
            h.patch_descriptor(&key, json!({"nom":"sensor.ambient_temperature"}));
            assert_eq!(h.metadata_patches[&key], json!({"unite":"°F"}));
            let publications = h.publications().unwrap();
            let (_, config) = publications
                .iter()
                .find(|(topic, _)| topic.ends_with("/config"))
                .unwrap();
            assert_eq!(config["canaux"][&key]["nom"], "sensor.ambient_temperature");
            assert_eq!(config["canaux"][&key]["unite"], "°F");
        }
    }
    #[test]
    fn cold_descriptor_failure_keeps_the_pending_change_visible() {
        let mut h = House::new("moli-probe/pont/fixture".into());
        h.use_file_metadata = true;
        h.registry(&serde_json::to_vec(&fixture()).unwrap())
            .unwrap();
        h.publications().unwrap();
        let stored = h.metadata.read().unwrap();
        if let crate::metadata::Bank::File(path) = &h.metadata {
            std::fs::write(path, b"invalid JSON").unwrap();
        }
        send(
            &mut h,
            "sensor.ambient_temperature",
            "friendly_name",
            "\"Renommé\"",
        );
        assert!(h.config_dirty);
        assert!(h.publications().is_err());
        assert!(h.config_dirty);
        assert_eq!(h.metadata_patches[&id("canal", "c1")]["nom"], "Renommé");
        h.metadata.replace(stored, true).unwrap();
        assert_eq!(
            h.publications()
                .unwrap()
                .iter()
                .filter(|(topic, _)| topic.ends_with("/config"))
                .count(),
            1
        );
        assert!(!h.config_dirty);
        assert!(h.metadata_patches.is_empty());
    }
    #[test]
    fn only_changed_channels_are_published_and_unchanged_metadata_is_not_replayed() {
        let mut h = house();
        h.publications().unwrap();
        send(&mut h, "sensor.ambient_temperature", "state", "21");
        let first = h.publications().unwrap();
        assert_eq!(
            first.iter().filter(|(_, p)| p["type"] == "canal").count(),
            1
        );
        send(&mut h, "sensor.ambient_temperature", "state", "21");
        assert!(h.publications().unwrap().is_empty());
        send(
            &mut h,
            "sensor.ambient_temperature",
            "friendly_name",
            "\"sensor.ambient_temperature\"",
        );
        assert!(
            !h.publications()
                .unwrap()
                .iter()
                .any(|(t, _)| t.ends_with("/config"))
        );
    }
    #[test]
    fn command_contracts_bind_by_unique_id_and_application_catalogues_stay_in_metadata() {
        let mut h = House::new("moli-probe/pont/fixture".into());
        assert!(h.discovery("homeassistant/light/fixture/config",br#"{"unique_id":"native-fixture","command_topic":"zigbee2mqtt/fixture/set","schema":"json","brightness":true,"brightness_scale":254}"#));
        let mut f = fixture();
        f["entities"][4]["unique_id"] = json!("native-fixture");
        f["entities"][4]["entity_id"] = json!("light.fixture");
        f["entities"][4]["platform"] = json!("mqtt");
        f["entities"][0]["unique_id"] = json!("native-fixture");
        f["entities"][0]["platform"] = json!("mqtt");
        f["entities"].as_array_mut().unwrap().push(
            json!({"id":"media-fixture","entity_id":"media_player.fixture","device_id":"d1"}),
        );
        h.registry(&serde_json::to_vec(&f).unwrap()).unwrap();
        assert!(h.command_definitions.is_empty());
        let pubs = h.publications().unwrap();
        let config = &pubs.iter().find(|(t, _)| t.ends_with("/config")).unwrap().1;
        assert_eq!(
            config["canaux"][id("canal", "c5")]["commande"]["parametres"]["brightness"]["max"],
            254
        );
        assert!(config["canaux"][id("canal", "c1")]["commande"].is_null());
        send(
            &mut h,
            "media_player.fixture",
            "source_list",
            "[\"Application A\",\"Application B\"]",
        );
        let pubs = h.publications().unwrap();
        assert!(
            pubs.iter()
                .filter(|(_, p)| p["type"] == "canal")
                .all(|(_, p)| p["valeur"].is_null() && p["encodage"] == "enum")
        );
        assert_eq!(
            pubs.iter().find(|(t, _)| t.ends_with("/config")).unwrap().1["canaux"]
                [id("canal", "media-fixture")]["catalogue_sources"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        h.registry(&serde_json::to_vec(&f).unwrap()).unwrap();
        let pubs = h.publications().unwrap();
        assert!(
            pubs.iter().find(|(t, _)| t.ends_with("/config")).unwrap().1["canaux"]
                [id("canal", "c5")]["commande"]
                .is_object()
        );
        assert!(h.discovery("homeassistant/light/fixture/config", b""));
        let pubs = h.publications().unwrap();
        assert!(
            pubs.iter().find(|(t, _)| t.ends_with("/config")).unwrap().1["canaux"]
                [id("canal", "c5")]["commande"]
                .is_null()
        );
    }
    #[test]
    fn mqtt_switch_contracts_are_scoped_distinct_and_survive_reload() {
        for file in [false, true] {
            let mut h = House::new("moli-probe/pont/fixture".into());
            h.use_file_metadata = file;
            let light = json!({"unique_id":"shared-fixture","command_topic":"fixture/lamp/set","schema":"json"});
            let switch = json!({"unique_id":"shared-fixture","command_topic":"fixture/strip/set","payload_on":"{\"state_l1\":\"ON\"}","payload_off":"{\"state_l1\":\"OFF\"}"});
            let second = json!({"unique_id":"second-fixture","command_topic":"fixture/strip/set","payload_on":"{\"state_l2\":\"ON\"}","payload_off":"{\"state_l2\":\"OFF\"}"});
            assert!(h.discovery(
                "homeassistant/light/fixture/config",
                &serde_json::to_vec(&light).unwrap()
            ));
            assert!(h.discovery(
                "homeassistant/switch/first/config",
                &serde_json::to_vec(&switch).unwrap()
            ));
            assert!(h.discovery(
                "homeassistant/switch/second/config",
                &serde_json::to_vec(&second).unwrap()
            ));
            let mut f = fixture();
            f["entities"] = json!([
                {"id":"lamp","entity_id":"light.fixture","device_id":"d1","platform":"mqtt","unique_id":"shared-fixture"},
                {"id":"out1","entity_id":"switch.first","device_id":"d1","platform":"mqtt","unique_id":"shared-fixture"},
                {"id":"out2","entity_id":"switch.second","device_id":"d1","platform":"mqtt","unique_id":"second-fixture"},
                {"id":"sensor","entity_id":"sensor.fixture","device_id":"d1","platform":"mqtt","unique_id":"shared-fixture"},
                {"id":"config","entity_id":"switch.configuration","device_id":"d1","platform":"mqtt","unique_id":"shared-fixture","entity_category":"config"},
                {"id":"diagnostic","entity_id":"switch.diagnostic","device_id":"d1","platform":"mqtt","unique_id":"shared-fixture","entity_category":"diagnostic"},
                {"id":"native","entity_id":"switch.native","device_id":"d1","platform":"native","unique_id":"shared-fixture"}
            ]);
            for _ in 0..2 {
                h.registry(&serde_json::to_vec(&f).unwrap()).unwrap();
                let pubs = h.publications().unwrap();
                let config = &pubs.iter().find(|(t, _)| t.ends_with("/config")).unwrap().1;
                let channels = &config["canaux"];
                assert_eq!(
                    channels[id("canal", "lamp")]["commande"]["topic"],
                    "fixture/lamp/set"
                );
                assert_eq!(
                    channels[id("canal", "out1")]["commande"]["on"],
                    "{\"state_l1\":\"ON\"}"
                );
                assert_eq!(
                    channels[id("canal", "out2")]["commande"]["on"],
                    "{\"state_l2\":\"ON\"}"
                );
                for native in ["sensor", "config", "diagnostic", "native"] {
                    assert!(channels[id("canal", native)]["commande"].is_null());
                }
                assert_eq!(channels[id("canal", "config")]["categorie"], "config");
                assert_eq!(
                    channels[id("canal", "diagnostic")]["categorie"],
                    "diagnostic"
                );
                assert_eq!(h.counts()["light_command_contracts"], 1);
                assert_eq!(h.counts()["switch_command_contracts"], 2);
                assert_eq!(h.devices[&id("appareil", "d1")].genre, "physique");
            }
            assert!(h.discovery("homeassistant/switch/first/config", b""));
            h.registry(&serde_json::to_vec(&f).unwrap()).unwrap();
            let pubs = h.publications().unwrap();
            let config = &pubs.iter().find(|(t, _)| t.ends_with("/config")).unwrap().1;
            assert!(config["canaux"][id("canal", "out1")]["commande"].is_null());
            assert!(config["canaux"][id("canal", "out2")]["commande"].is_object());
            assert!(config["canaux"][id("canal", "lamp")]["commande"].is_object());
            let invalid = json!({"unique_id":"second-fixture","command_topic":"fixture/strip/set","command_template":"{{ value }}"});
            assert!(!h.discovery(
                "homeassistant/switch/second/config",
                &serde_json::to_vec(&invalid).unwrap()
            ));
            h.registry(&serde_json::to_vec(&f).unwrap()).unwrap();
            assert!(
                h.metadata.read().unwrap()["canaux"][id("canal", "out2")]["commande"].is_null()
            );
            assert_eq!(h.counts()["switch_command_contracts"], 0);
            // A discovery received after the registry installs a cold patch.
            assert!(h.discovery(
                "homeassistant/switch/first/config",
                &serde_json::to_vec(&switch).unwrap()
            ));
            h.publications().unwrap();
            assert_eq!(
                h.metadata.read().unwrap()["canaux"][id("canal", "out1")]["commande"]["on"],
                "{\"state_l1\":\"ON\"}"
            );
            assert_eq!(h.counts()["switch_command_contracts"], 1);
            assert_eq!(h.counts()["light_command_contracts"], 1);
        }
    }

    #[test]
    fn color_absence_invalidates_every_coordinate_without_retaining_old_values() {
        let mut f = fixture();
        f["entities"] = json!([{"id":"lamp","entity_id":"light.fixture","device_id":"d1"}]);
        let mut h = House::new("moli-probe/pont/fixture".into());
        h.registry(&serde_json::to_vec(&f).unwrap()).unwrap();
        for (attribute, valid, coordinates) in [
            ("xy_color", "[0.3,0.4]", &["color_x", "color_y"][..]),
            ("hs_color", "[120,50]", &["color_h", "color_s"][..]),
            (
                "rgb_color",
                "[1,2,3]",
                &["color_r", "color_g", "color_b"][..],
            ),
        ] {
            let topic = format!("homeassistant/statestream/light/fixture/{attribute}");
            for absent in ["null", "unknown", "unavailable", "None", "\"null\""] {
                assert!(h.ingest(&topic, valid.as_bytes(), true));
                assert!(h.ingest(&topic, absent.as_bytes(), false));
                for coordinate in coordinates {
                    let channel = h
                        .channels
                        .values()
                        .find(|c| c.source == format!("light.fixture/{coordinate}"))
                        .unwrap();
                    assert!(channel.valeur.is_null());
                    assert_eq!(channel.qualite, "inconnu");
                    assert!(!channel.retenu);
                }
                let pubs = h.publications().unwrap();
                for coordinate in coordinates {
                    let key = format!("{}-{}", id("canal", "lamp"), id("attribut", coordinate));
                    let state = &pubs
                        .iter()
                        .find(|(_, p)| p["type"] == "canal" && p["id"] == key)
                        .unwrap()
                        .1;
                    assert!(state["valeur"].is_null());
                    assert_eq!(state["qualite"], "inconnu");
                }
            }
        }
        assert!(!h.ingest(
            "homeassistant/statestream/light/fixture/xy_color",
            b"[0.8,0.3]",
            false
        ));
        assert!(h.ingest(
            "homeassistant/statestream/light/fixture/xy_color",
            b"[0.5,0.5000000005]",
            false
        ));
        assert!(!h.ingest(
            "homeassistant/statestream/light/fixture/xy_color",
            b"[0.5,0.500000002]",
            false
        ));
    }

    #[test]
    fn color_arrays_become_strict_scalar_channels_with_explicit_units() {
        let mut f = fixture();
        f["entities"] = json!([{"id":"lamp","entity_id":"light.fixture","device_id":"d1"}]);
        let mut h = House::new("moli-probe/pont/fixture".into());
        h.registry(&serde_json::to_vec(&f).unwrap()).unwrap();
        for (attribute, payload) in [
            ("hs_color", "[360,100]"),
            ("xy_color", "[0,1]"),
            ("rgb_color", "[0,128,255]"),
        ] {
            assert!(h.ingest(
                &format!("homeassistant/statestream/light/fixture/{attribute}"),
                payload.as_bytes(),
                true
            ));
        }
        for (attribute, value, unit) in [
            ("color_h", 360.0, "°"),
            ("color_s", 100.0, "%"),
            ("color_x", 0.0, "coord"),
            ("color_y", 1.0, "coord"),
            ("color_r", 0.0, "/255"),
            ("color_g", 128.0, "/255"),
            ("color_b", 255.0, "/255"),
        ] {
            let source = format!("light.fixture/{attribute}");
            let channel = h.channels.values().find(|c| c.source == source).unwrap();
            assert_eq!(number(&channel.valeur), Some(value));
            assert_eq!(channel.semantique, attribute);
            assert_eq!(channel.unite.as_deref(), Some(unit));
            assert_eq!(channel.qualite, "retained_age_inconnu");
        }
        let previous: BTreeMap<_, _> = h
            .channels
            .iter()
            .map(|(k, c)| (k.clone(), (c.valeur.clone(), c.recu_ms)))
            .collect();
        for (attribute, payload) in [
            ("hs_color", "[361,50]"),
            ("hs_color", "[1,-1]"),
            ("hs_color", "[1,101]"),
            ("xy_color", "[0,1.1]"),
            ("xy_color", "[0]"),
            ("xy_color", "[0,1,0]"),
            ("xy_color", "[true,1]"),
            ("rgb_color", "[0,1,256]"),
            ("rgb_color", "[0,1,-1]"),
            ("rgb_color", "[0,1,1.5]"),
            ("rgb_color", "[0,1]"),
            ("rgb_color", "{\"r\":0,\"g\":1,\"b\":2}"),
        ] {
            assert!(!h.ingest(
                &format!("homeassistant/statestream/light/fixture/{attribute}"),
                payload.as_bytes(),
                false
            ));
        }
        let after: BTreeMap<_, _> = h
            .channels
            .iter()
            .map(|(k, c)| (k.clone(), (c.valeur.clone(), c.recu_ms)))
            .collect();
        assert_eq!(after, previous);
        assert!(
            h.channels
                .values()
                .all(|c| !c.valeur.is_array() && !c.valeur.is_object())
        );
        send(&mut h, "light.fixture", "color_temp_kelvin", "3000");
        send(&mut h, "light.fixture", "color_temp", "333");
        send(&mut h, "light.fixture", "color_mode", "\"xy\"");
        h.publications().unwrap();
        h.registry(&serde_json::to_vec(&f).unwrap()).unwrap();
        assert!(
            h.channels
                .values()
                .any(|c| c.source == "light.fixture/color_x" && c.valeur == 0)
        );
    }

    fn media_fixture() -> Json {
        let mut registry = fixture();
        registry["entities"] = json!([{
            "id":"media-fixture", "entity_id":"media_player.fixture",
            "device_id":"d1", "name":"Lecteur fixture"
        }]);
        registry
    }

    fn ingest_media(h: &mut House) {
        for (attribute, value, retained) in [
            ("state", "\"playing\"", true),
            ("media_title", "\"Titre fixture\"", false),
            ("media_position", "42.5", true),
            (
                "media_position_updated_at",
                "\"2026-01-01T12:00:00Z\"",
                false,
            ),
            ("source_list", "[\"Application A\",\"Application B\"]", true),
        ] {
            assert!(h.ingest(
                &format!("homeassistant/statestream/media_player/fixture/{attribute}"),
                value.as_bytes(),
                retained,
            ));
        }
        // Rebuilding metadata must never turn an old receipt into a new measure.
        for channel in h.channels.values_mut() {
            channel.recu_ms = Some(1234);
        }
    }

    #[test]
    fn registry_reload_preserves_media_attributes_and_cold_catalogues() {
        for file in [false, true] {
            for published in [false, true] {
                for renamed in [false, true] {
                    let mut registry = media_fixture();
                    let mut h = House::new("moli-probe/pont/fixture".into());
                    h.use_file_metadata = file;
                    h.registry(&serde_json::to_vec(&registry).unwrap()).unwrap();
                    ingest_media(&mut h);
                    let old = h.channels.clone();
                    if published {
                        h.publications().unwrap();
                    }
                    if renamed {
                        registry["entities"][0]["entity_id"] = json!("media_player.renamed");
                        registry["entities"][0]["name"] = json!("Lecteur renommé");
                        registry["entities"][0]["device_id"] = json!("d2");
                        registry["entities"][0]["area_id"] = json!("r2");
                        registry["devices"]
                            .as_array_mut()
                            .unwrap()
                            .push(json!({"id":"d2","name":"Appareil déplacé","area_id":"r2"}));
                    }
                    h.registry(&serde_json::to_vec(&registry).unwrap()).unwrap();
                    assert_eq!(h.channels.len(), old.len());
                    let entity = if renamed {
                        "media_player.renamed"
                    } else {
                        "media_player.fixture"
                    };
                    let device = id("appareil", if renamed { "d2" } else { "d1" });
                    let room = id("piece", if renamed { "r2" } else { "r1" });
                    let publications = h.publications().unwrap();
                    let config = &publications
                        .iter()
                        .find(|(t, _)| t.ends_with("/config"))
                        .unwrap()
                        .1;
                    let base = id("canal", "media-fixture");
                    assert_eq!(
                        config["canaux"][&base]["catalogue_sources"],
                        json!(["Application A", "Application B"])
                    );
                    assert_eq!(
                        config["canaux"][&base]["nom"],
                        if renamed {
                            "Lecteur renommé"
                        } else {
                            "Lecteur fixture"
                        }
                    );
                    for (key, previous) in &old {
                        let channel = &h.channels[key];
                        assert_eq!(channel.valeur, previous.valeur);
                        assert_eq!(channel.qualite, previous.qualite);
                        assert_eq!(channel.retenu, previous.retenu);
                        assert_eq!(channel.recu_ms, previous.recu_ms);
                        assert_eq!(channel.unite, previous.unite);
                        assert_eq!(channel.semantique, previous.semantique);
                        assert_eq!(channel.piece, room);
                        assert_eq!(channel.device, device);
                        let source = if previous.base {
                            entity.to_owned()
                        } else {
                            format!("{entity}/{}", previous.source.rsplit_once('/').unwrap().1)
                        };
                        assert_eq!(channel.source, source);
                        assert_eq!(config["canaux"][key]["source"], source);
                        assert_eq!(config["canaux"][key]["piece"], room);
                        assert_eq!(config["canaux"][key]["appareil"], device);
                        assert!(config["canaux"][key]["nom"].is_string());
                        assert!(h.devices[&device].channels.contains(key));
                        let state = &publications
                            .iter()
                            .find(|(_, p)| p["type"] == "canal" && p["id"] == *key)
                            .unwrap()
                            .1;
                        assert_eq!(state["valeur"], previous.valeur);
                        assert_eq!(state["qualite"], previous.qualite);
                        assert_eq!(state["recu_ms"], 1234);
                    }
                    assert!(h.labels.is_empty());
                    assert!(h.metadata_patches.is_empty());
                }
            }
        }
    }

    #[test]
    fn registry_reload_does_not_transfer_media_to_removed_or_replaced_identity() {
        for file in [false, true] {
            for replaced in [false, true] {
                let mut registry = media_fixture();
                let mut h = House::new("moli-probe/pont/fixture".into());
                h.use_file_metadata = file;
                h.registry(&serde_json::to_vec(&registry).unwrap()).unwrap();
                ingest_media(&mut h);
                h.publications().unwrap();
                if replaced {
                    registry["entities"][0]["id"] = json!("replacement");
                } else {
                    registry["entities"] = json!([]);
                }
                h.registry(&serde_json::to_vec(&registry).unwrap()).unwrap();
                assert_eq!(h.channels.len(), usize::from(replaced));
                assert!(
                    h.channels
                        .values()
                        .all(|c| c.base && c.valeur.is_null() && c.recu_ms.is_none())
                );
                let config = h.metadata.read().unwrap();
                assert!(config["canaux"][id("canal", "media-fixture")].is_null());
                assert!(
                    config["canaux"][id("canal", "replacement")]["catalogue_sources"].is_null()
                );
            }
        }
    }

    fn assert_media_identity_change_drops_state(house_changed: bool) {
        for file in [false, true] {
            for published in [false, true] {
                let mut registry = media_fixture();
                registry["house_id"] = json!("fixture-house-a");
                let mut h = House::new("moli-probe/pont/fixture".into());
                h.use_file_metadata = file;
                h.registry(&serde_json::to_vec(&registry).unwrap()).unwrap();
                ingest_media(&mut h);
                if published {
                    h.publications().unwrap();
                }
                if house_changed {
                    registry["house_id"] = json!("fixture-house-b");
                } else {
                    registry["entities"][0]["entity_id"] = json!("sensor.fixture");
                }
                h.registry(&serde_json::to_vec(&registry).unwrap()).unwrap();
                assert_eq!(h.channels.len(), 1);
                let base = id("canal", "media-fixture");
                let channel = &h.channels[&base];
                assert!(channel.base && channel.valeur.is_null());
                assert_eq!(channel.qualite, "absent");
                assert!(!channel.retenu);
                assert!(channel.recu_ms.is_none());
                let config = h.metadata.read().unwrap();
                assert_eq!(config["canaux"].as_object().unwrap().len(), 1);
                assert!(config["canaux"][&base]["catalogue_sources"].is_null());
                assert!(h.metadata_patches.is_empty());
            }
        }
    }

    #[test]
    fn registry_reload_does_not_transfer_media_across_domains() {
        assert_media_identity_change_drops_state(false);
    }

    #[test]
    fn registry_reload_does_not_transfer_media_across_houses() {
        assert_media_identity_change_drops_state(true);
    }

    #[test]
    fn water_heater_temperature_is_never_room_ambience_without_explicit_selection() {
        let mut f = fixture();
        f["devices"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"water","name":"Eau chaude","area_id":"r1"}));
        f["entities"].as_array_mut().unwrap().extend([
            json!({"id":"water-device","entity_id":"water_heater.fixture","device_id":"water"}),
            json!({"id":"water-temperature","entity_id":"sensor.water_temperature","device_id":"water","original_device_class":"temperature","unit_of_measurement":"°C"})
        ]);
        let mut h = House::new("moli-probe/pont/a".into());
        h.registry(&serde_json::to_vec(&f).unwrap()).unwrap();
        send(&mut h, "sensor.ambient_temperature", "state", "21");
        send(&mut h, "sensor.water_temperature", "state", "60");
        assert_eq!(
            h.generics("maison")
                .iter()
                .find(|g| g["id"] == "temperature:°C")
                .unwrap()["valeur"],
            21.0
        );
    }
    #[test]
    fn setpoints_and_configuration_channels_never_become_ambient_temperature() {
        let mut f = fixture();
        f["entities"].as_array_mut().unwrap().extend([
            json!({"id":"setpoint","entity_id":"number.setpoint","device_id":"d1","original_device_class":"temperature","unit_of_measurement":"°C"}),
            json!({"id":"configuration","entity_id":"sensor.calibration","device_id":"d1","entity_category":"config","original_device_class":"temperature","unit_of_measurement":"°C"})
        ]);
        let mut h = House::new("moli-probe/pont/fixture".into());
        h.registry(&serde_json::to_vec(&f).unwrap()).unwrap();
        send(&mut h, "sensor.ambient_temperature", "state", "21");
        send(&mut h, "number.setpoint", "state", "35");
        send(&mut h, "sensor.calibration", "state", "2");
        assert_eq!(
            h.generics("maison")
                .iter()
                .find(|g| g["id"] == "temperature:°C")
                .unwrap()["valeur"],
            21.0
        );
    }
    #[test]
    fn file_metadata_releases_labels_and_house_identity_survives_bridge_reordering() {
        let mut f = fixture();
        f["house_id"] = json!("stable-house-fixture");
        let mut a = House::new("moli-probe/pont/z2m-0".into());
        a.use_file_metadata = true;
        a.registry(&serde_json::to_vec(&f).unwrap()).unwrap();
        let mut b = House::new("moli-probe/pont/z2m-1".into());
        b.registry(&serde_json::to_vec(&f).unwrap()).unwrap();
        assert_eq!(a.root, b.root);
        assert_eq!(a.counts()["hot_label_bytes"], 0);
        let out = a.publications().unwrap();
        let meta = &out.iter().find(|(t, _)| t.ends_with("/config")).unwrap().1;
        assert_eq!(meta["canaux"][id("canal", "c1")]["unite"], "°C");
        assert!(
            out.iter()
                .filter(|(_, p)| p["type"] == "appareil")
                .all(|(_, p)| p.get("nom").is_none())
        );
    }
    fn send(h: &mut House, entity: &str, attribute: &str, value: &str) {
        let (d, n) = entity.split_once('.').unwrap();
        assert!(h.ingest(
            &format!("homeassistant/statestream/{d}/{n}/{attribute}"),
            value.as_bytes(),
            true
        ));
    }
    #[test]
    fn real_hierarchy_keeps_room_below_its_floor_and_allows_direct_room() {
        let h = house();
        assert_eq!(h.nodes[&id("piece", "r1")].parent, Some(id("etage", "f1")));
        assert_eq!(h.nodes[&id("piece", "r2")].parent, Some("maison".into()));
        assert!(h.includes(&id("etage", "f1"), &id("piece", "r1")));
        assert!(!h.includes(&id("etage", "f1"), &id("piece", "r2")));
    }
    #[test]
    fn all_channels_keep_their_units_values_and_entity_room_override() {
        let mut h = house();
        send(&mut h, "sensor.ambient_temperature", "state", "21.5");
        send(&mut h, "sensor.ambient_humidity", "state", "54");
        assert_eq!(h.channels[&id("canal", "c1")].valeur, 21.5);
        assert_eq!(h.channels[&id("canal", "c2")].valeur, 54);
        assert_eq!(h.channels[&id("canal", "c1")].unite.as_deref(), Some("°C"));
        assert_eq!(h.channels[&id("canal", "c2")].unite.as_deref(), Some("%"));
        assert_eq!(h.channels[&id("canal", "c4")].piece, id("piece", "r2"));
    }
    #[test]
    fn aggregates_never_mix_units_or_turn_missing_openings_into_closed() {
        let mut h = house();
        send(&mut h, "sensor.ambient_temperature", "state", "20");
        send(&mut h, "sensor.remote_temperature", "state", "70");
        let g = h.generics("maison");
        let temps: Vec<_> = g
            .iter()
            .filter(|g| g["semantique"] == "temperature")
            .collect();
        assert_eq!(temps.len(), 2);
        let opening = g.iter().find(|g| g["semantique"] == "ouverture").unwrap();
        assert_eq!(opening["valeur"], Json::Null);
        assert_eq!(opening["inconnues"], 1);
        send(&mut h, "binary_sensor.window", "state", "\"on\"");
        assert_eq!(
            h.generics("maison")
                .iter()
                .find(|g| g["semantique"] == "ouverture")
                .unwrap()["valeur"],
            1
        );
    }
    #[test]
    fn heating_mode_is_not_heating_activity_and_partial_messages_preserve_other_channels() {
        let mut h = house();
        send(&mut h, "climate.radiator", "state", "\"heat\"");
        assert!(
            !h.generics("maison")
                .iter()
                .any(|g| g["semantique"] == "chauffage_actif")
        );
        send(&mut h, "climate.radiator", "hvac_action", "\"idle\"");
        send(&mut h, "sensor.ambient_temperature", "state", "23");
        send(&mut h, "sensor.ambient_humidity", "state", "50");
        let g = h.generics("maison");
        assert_eq!(
            g.iter()
                .find(|g| g["semantique"] == "chauffage_actif")
                .unwrap()["valeur"],
            0
        );
        assert_eq!(h.channels[&id("canal", "c1")].valeur, 23);
    }
    #[test]
    fn renaming_preserves_ids_topics_and_values() {
        let mut h = house();
        send(&mut h, "sensor.ambient_temperature", "state", "22");
        let old = h.devices[&id("appareil", "d1")].id.clone();
        let mut f = fixture();
        f["devices"][0]["name"] = json!("Autre nom");
        h.registry(&serde_json::to_vec(&f).unwrap()).unwrap();
        assert_eq!(h.devices[&id("appareil", "d1")].id, old);
        assert_eq!(h.channels[&id("canal", "c1")].valeur, 22);
    }
    #[test]
    fn replay_keeps_unknown_age_and_rejects_unbounded_or_sensitive_attributes() {
        let mut h = house();
        send(&mut h, "sensor.ambient_temperature", "state", "22");
        assert_eq!(
            h.channels[&id("canal", "c1")].qualite,
            "retained_age_inconnu"
        );
        assert!(!h.ingest(
            "homeassistant/statestream/climate/radiator/code",
            b"1234",
            true
        ));
        assert!(!h.ingest(
            "homeassistant/statestream/climate/radiator/media_title",
            &vec![b'x'; MAX_TEXT + 1],
            true
        ));
    }
}
