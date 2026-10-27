//! Plugin-side cold enum metadata and bounded pre-schema lexical observations.
use super::*;
use crate::enum_catalogue::Choice;

const MAX_PENDING: usize = 512;
const MAX_PENDING_BYTES: usize = 128 * 1024;
const PENDING_AGE_MS: u128 = 30_000;

#[derive(Clone)]
pub(super) struct Pending {
    pub(super) native: String,
    retained: bool,
    pub(super) received: u128,
}

fn missing(value: &Json) -> bool {
    value.is_null()
        || value
            .as_str()
            .is_some_and(|value| matches!(value, "unknown" | "unavailable" | "None" | "null"))
}
fn target(base: &str, attribute: &str) -> String {
    if attribute == "state" {
        base.into()
    } else {
        format!("{base}-{}", id("attribut", attribute))
    }
}
fn choices(value: &Json) -> Result<Vec<Choice>> {
    let values = value.as_array().context("enum choices must be strings")?;
    ensure!(values.len() <= 256, "too many enum choices");
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let native = value.as_str().context("enum choice must be a string")?;
            ensure!(native.len() <= 256, "enum choice exceeds byte bound");
            ensure!(
                !values[..index]
                    .iter()
                    .any(|value| value.as_str() == Some(native)),
                "duplicate enum choice"
            );
            Ok(Choice {
                native: native.into(),
                label: native.into(),
            })
        })
        .collect()
}

impl House {
    pub(super) fn init_enums(&mut self, old: &Self, same_house: bool) -> Result<()> {
        self.enum_persistence_disabled = old.enum_persistence_disabled;
        if same_house {
            self.enum_pool = old.enum_pool.clone();
            self.enum_store = old.enum_store.clone();
            self.enum_store_dirty = old.enum_store_dirty;
            self.enum_compacted = old.enum_compacted.clone();
            self.enum_compact_ready = old.enum_compact_ready;
            self.enum_pending = old.enum_pending.clone();
        }
        if self.enum_persistence_disabled {
            self.enum_store = None;
        }
        if !self.enum_persistence_disabled
            && self.enum_store.is_none()
            && let Some(directory) = std::env::var_os("MOLI_ENUM_CATALOGUE_DIR")
        {
            let store = std::sync::Arc::new(crate::enum_store::Store::new(
                directory.into(),
                self.root.clone(),
            )?);
            self.enum_pool = store.load_pool()?;
            self.enum_store = Some(store);
        }
        Ok(())
    }

    pub(super) fn restore_enum(&mut self, key: &str) -> Result<()> {
        if self.channels[key].enum_cache_checked {
            return Ok(());
        }
        if self.channels[key].enum_ref.is_none()
            && let Some(store) = &self.enum_store
            && let Some(reference) = store.reference(key)?
        {
            ensure!(
                self.enum_pool.active_codes(reference).is_some(),
                "enum cache reference unavailable"
            );
            self.channels.get_mut(key).expect("channel exists").enum_ref = Some(reference);
            self.patch_descriptor(key,json!({"enumeration":{"catalogue":reference.id,"version":reference.version},"enum_attendue":true}));
        }
        self.channels
            .get_mut(key)
            .expect("channel exists")
            .enum_cache_checked = true;
        Ok(())
    }

    pub(super) fn expire_enum_pending(&mut self) {
        let now = now_ms();
        self.enum_pending
            .retain(|_, pending| now.saturating_sub(pending.received) <= PENDING_AGE_MS);
    }
    fn pending(&mut self, key: &str, native: &str, retained: bool) {
        self.expire_enum_pending();
        let old = self
            .enum_pending
            .get(key)
            .map_or(0, |pending| pending.native.len());
        let bytes = self
            .enum_pending
            .values()
            .map(|pending| pending.native.len())
            .sum::<usize>();
        if native.len() <= 256
            && (self.enum_pending.contains_key(key) || self.enum_pending.len() < MAX_PENDING)
            && bytes - old + native.len() <= MAX_PENDING_BYTES
        {
            self.enum_pending.insert(
                key.into(),
                Pending {
                    native: native.into(),
                    retained,
                    received: now_ms(),
                },
            );
        } else {
            self.rejected += 1;
        }
    }

    pub(super) fn enum_native_expected(&self, base: &str, attribute: &str) -> bool {
        matches!(attribute, "app_id" | "source" | "operation_mode")
            || self
                .channels
                .get(&target(base, attribute))
                .is_some_and(|channel| channel.enum_expected)
    }

    pub(super) fn expect_enum(&mut self, key: &str) -> Result<()> {
        let channel = &self.channels[key];
        if !channel.enum_expected {
            let previous = channel.valeur.as_str().map(str::to_owned);
            let retained = channel.retenu;
            let received = channel.recu_ms;
            let channel = self.channels.get_mut(key).expect("channel exists");
            channel.enum_expected = true;
            channel.valeur = Json::Null;
            channel.qualite = "catalogue_absent";
            if let Some(native) = previous {
                self.pending(key, &native, retained);
                if let (Some(received), Some(pending)) = (received, self.enum_pending.get_mut(key))
                {
                    pending.received = received;
                }
            }
            self.patch_descriptor(key, json!({"enum_attendue":true}));
            self.dirty_channels.insert(key.into());
        }
        self.restore_enum(key)
    }

    pub(super) fn enum_value(
        &mut self,
        key: &str,
        value: &Json,
        retained: bool,
    ) -> (Json, &'static str) {
        if missing(value) {
            self.enum_pending.remove(key);
            return (Json::Null, "inconnu");
        }
        let Some(native) = value.as_str() else {
            return (Json::Null, "valeur_non_reconnue");
        };
        self.enum_pending.remove(key);
        if let Some(reference) = self.channels[key].enum_ref {
            if let Some(code) = self.enum_pool.encode(reference, native) {
                return (
                    json!(code),
                    if retained {
                        "retained_age_inconnu"
                    } else {
                        "observe"
                    },
                );
            }
            self.pending(key, native, retained);
            return (Json::Null, "valeur_non_reconnue");
        }
        self.pending(key, native, retained);
        (Json::Null, "catalogue_absent")
    }

    fn bind_choices(&mut self, key: &str, choices: &[Choice]) -> Result<()> {
        self.expect_enum(key)?;
        self.expire_enum_pending();
        let old = self.channels[key].enum_ref;
        let old_native = old
            .and_then(|reference| {
                self.channels[key]
                    .valeur
                    .as_u64()
                    .and_then(|code| u8::try_from(code).ok())
                    .and_then(|code| self.enum_pool.decode(reference, code))
            })
            .map(str::to_owned);
        let reference = self.enum_pool.update(old, choices)?;
        if old == Some(reference) && !self.enum_pending.contains_key(key) {
            return Ok(());
        }
        let pending = self.enum_pending.remove(key);
        let native = pending
            .as_ref()
            .map(|pending| pending.native.as_str())
            .or(old_native.as_deref());
        let code = native.and_then(|native| self.enum_pool.encode(reference, native));
        let channel = self.channels.get_mut(key).expect("channel exists");
        channel.enum_ref = Some(reference);
        channel.enum_expected = true;
        if let Some(pending) = pending.as_ref() {
            channel.retenu = pending.retained;
            channel.recu_ms = Some(pending.received);
        }
        channel.valeur = code.map_or(Json::Null, |code| json!(code));
        channel.qualite = if native.is_some() && code.is_none() {
            "valeur_non_reconnue"
        } else if code.is_some() {
            if channel.retenu {
                "retained_age_inconnu"
            } else {
                "observe"
            }
        } else if channel.qualite == "absent" {
            "absent"
        } else {
            "inconnu"
        };
        self.patch_descriptor(key,json!({"enumeration":{"catalogue":reference.id,"version":reference.version},"enum_attendue":true}));
        self.dirty_channels.insert(key.into());
        self.config_dirty = true;
        self.enum_store_dirty |= old != Some(reference);
        if code.is_none()
            && let Some(pending) = pending
        {
            self.enum_pending.insert(key.into(), pending);
        }
        Ok(())
    }

    fn current_choices(&self, key: &str) -> Vec<Choice> {
        self.channels[key]
            .enum_ref
            .and_then(|reference| self.enum_pool.descriptor(reference))
            .and_then(|descriptor| descriptor["entries"].as_array().cloned())
            .unwrap_or_default()
            .into_iter()
            .filter(|entry| entry["active"] == true)
            .filter_map(|entry| {
                Some(Choice {
                    native: entry["native"].as_str()?.into(),
                    label: entry["label"].as_str()?.into(),
                })
            })
            .collect()
    }

    fn app_choices(value: &Json) -> Result<Vec<Choice>> {
        let mut result = Vec::new();
        if let Some(values) = value.as_array() {
            ensure!(values.len() <= 256, "too many applications");
            for value in values {
                let native = value
                    .as_str()
                    .or_else(|| value["id"].as_str())
                    .context("application ID missing")?;
                let label = if value.is_string() {
                    native
                } else {
                    value["label"]
                        .as_str()
                        .context("application label missing")?
                };
                ensure!(
                    !native.is_empty() && native.len() <= 256 && label.len() <= 256,
                    "application text exceeds byte bound"
                );
                result.push(Choice {
                    native: native.into(),
                    label: label.into(),
                });
            }
        } else if let Some(values) = value.as_object() {
            ensure!(values.len() <= 256, "too many applications");
            for (native, label) in values {
                let label = label
                    .as_str()
                    .context("application label must be a string")?;
                ensure!(
                    !native.is_empty() && native.len() <= 256 && label.len() <= 256,
                    "application text exceeds byte bound"
                );
                result.push(Choice {
                    native: native.clone(),
                    label: label.into(),
                });
            }
        } else {
            anyhow::bail!("application list is invalid");
        }
        Ok(result)
    }

    pub(super) fn enum_attribute(
        &mut self,
        base: &str,
        entity: &str,
        attribute: &str,
        value: &Json,
        retained: bool,
    ) -> Result<Option<bool>> {
        match attribute {
            "options" | "hvac_modes" | "operation_list" | "source_list" => {
                let choices = choices(value)?;
                let key = if attribute == "source_list" {
                    self.ensure_channel(base, entity, "source")
                        .context("channel budget exceeded")?
                } else {
                    base.into()
                };
                self.bind_choices(&key, &choices)?;
                if attribute == "operation_list" {
                    let key = self
                        .ensure_channel(base, entity, "operation_mode")
                        .context("channel budget exceeded")?;
                    self.bind_choices(&key, &choices)?;
                }
                if attribute == "source_list" {
                    self.patch_descriptor(base, json!({"catalogue_sources":value}));
                }
                Ok(Some(true))
            }
            "applications" | "application_list" | "apps" => {
                let choices = Self::app_choices(value)?;
                let key = self
                    .ensure_channel(base, entity, "app_id")
                    .context("channel budget exceeded")?;
                self.bind_choices(&key, &choices)?;
                Ok(Some(true))
            }
            "app_id" => {
                let key = self
                    .ensure_channel(base, entity, attribute)
                    .context("channel budget exceeded")?;
                self.expect_enum(&key)?;
                if value.as_str() == Some("") {
                    return Ok(Some(self.record_attribute(
                        base,
                        entity,
                        attribute,
                        Json::Null,
                        retained,
                    )));
                }
                if !missing(value) {
                    let native = value.as_str().context("application ID must be a string")?;
                    ensure!(native.len() <= 256, "application ID exceeds byte bound");
                    if !self.channels[&key]
                        .enum_ref
                        .is_some_and(|reference| self.enum_pool.encode(reference, native).is_some())
                    {
                        let mut choices = self.current_choices(&key);
                        choices.push(Choice {
                            native: native.into(),
                            label: native.into(),
                        });
                        self.bind_choices(&key, &choices)?;
                    }
                }
                Ok(Some(self.record_attribute(
                    base,
                    entity,
                    attribute,
                    value.clone(),
                    retained,
                )))
            }
            "app_name" => {
                if missing(value) {
                    return Ok(Some(true));
                }
                let key = target(base, "app_id");
                let native = self
                    .channels
                    .get(&key)
                    .and_then(|channel| {
                        channel.enum_ref.and_then(|reference| {
                            channel
                                .valeur
                                .as_u64()
                                .and_then(|code| u8::try_from(code).ok())
                                .and_then(|code| self.enum_pool.decode(reference, code))
                        })
                    })
                    .map(str::to_owned);
                if let (Some(native), Some(label)) = (native, value.as_str()) {
                    ensure!(label.len() <= 256, "application label exceeds byte bound");
                    let mut choices = self.current_choices(&key);
                    if let Some(choice) = choices.iter_mut().find(|choice| choice.native == native)
                    {
                        choice.label = label.into();
                        self.bind_choices(&key, &choices)?;
                    }
                }
                Ok(Some(true))
            }
            "source" | "operation_mode" => {
                let key = self
                    .ensure_channel(base, entity, attribute)
                    .context("channel budget exceeded")?;
                self.expect_enum(&key)?;
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    pub(super) fn enum_descriptors(&self) -> Json {
        let mut catalogues = serde_json::Map::new();
        for reference in self
            .channels
            .values()
            .filter_map(|channel| channel.enum_ref)
        {
            if !catalogues.contains_key(&reference.id.to_string())
                && let Some(descriptor) = self.enum_pool.descriptor(reference)
            {
                catalogues.insert(reference.id.to_string(), descriptor);
            }
        }
        Json::Object(catalogues)
    }

    pub(super) fn persist_enums(&mut self) -> Result<()> {
        if !self.enum_store_dirty {
            return Ok(());
        }
        let mut compacted = self.enum_pool.clone();
        if let Some(store) = &self.enum_store {
            let bases = self
                .channels
                .values()
                .filter(|channel| channel.base)
                .map(|channel| channel.id.clone())
                .collect();
            let current: Vec<_> = self
                .channels
                .values()
                .filter_map(|channel| {
                    channel
                        .enum_ref
                        .map(|reference| (channel.id.clone(), reference))
                })
                .collect();
            store.save(&mut compacted, &bases, current)?;
        } else {
            compacted.retain_refs(
                self.channels
                    .values()
                    .filter_map(|channel| channel.enum_ref),
            );
        }
        self.enum_compacted = Some(compacted);
        self.enum_compact_ready = false;
        self.enum_store_dirty = false;
        Ok(())
    }

    pub(super) fn compact_enums_after_publication(&mut self) {
        if !self.enum_compact_ready {
            return;
        }
        self.enum_compact_ready = false;
        if let Some(compacted) = self.enum_compacted.take()
            && !self.enum_store_dirty
            && self
                .channels
                .values()
                .filter_map(|channel| channel.enum_ref)
                .all(|reference| compacted.active_codes(reference).is_some())
        {
            self.enum_pool = compacted;
        }
    }

    pub(super) fn ingest_snapshot_payload(&mut self, payload: &[u8], retained: bool) -> bool {
        if !self.ready() || payload.len() > MAX_SCHEMA {
            self.rejected += 1;
            return false;
        }
        let Ok(snapshot) = serde_json::from_slice(payload) else {
            self.rejected += 1;
            return false;
        };
        self.ingest_snapshot(&snapshot, retained)
    }

    pub(crate) fn ingest_snapshot(&mut self, snapshot: &Json, retained: bool) -> bool {
        let Some(entity) = snapshot["entity_id"]
            .as_str()
            .filter(|entity| entity.len() <= 256)
        else {
            return false;
        };
        let Some((domain, object)) = entity.split_once('.') else {
            return false;
        };
        let Some(attributes) = snapshot["attributes"]
            .as_object()
            .filter(|attributes| attributes.len() <= 256)
        else {
            return false;
        };
        if attributes
            .get("app_id")
            .is_some_and(|value| !value.is_string() && !value.is_null())
        {
            return false;
        }
        let mut accepted = true;
        for attribute in [
            "options",
            "operation_list",
            "hvac_modes",
            "source_list",
            "applications",
            "application_list",
            "apps",
            "app_id",
            "app_name",
        ] {
            if let Some(value) = attributes.get(attribute) {
                let applied = self.ingest(
                    &format!("homeassistant/statestream/{domain}/{object}/{attribute}"),
                    value.to_string().as_bytes(),
                    retained,
                );
                accepted &= applied;
                if !applied && attribute == "app_id" {
                    return false;
                }
            }
        }
        for (attribute, value) in attributes {
            if matches!(
                attribute.as_str(),
                "options"
                    | "operation_list"
                    | "hvac_modes"
                    | "source_list"
                    | "applications"
                    | "application_list"
                    | "apps"
                    | "app_id"
                    | "app_name"
            ) {
                continue;
            }
            self.ingest(
                &format!("homeassistant/statestream/{domain}/{object}/{attribute}"),
                value.to_string().as_bytes(),
                retained,
            );
        }
        if let Some(state) = snapshot.get("state") {
            let payload = state
                .as_str()
                .map_or_else(|| state.to_string(), str::to_owned);
            accepted &= self.ingest(
                &format!("homeassistant/statestream/{domain}/{object}/state"),
                payload.as_bytes(),
                retained,
            );
        }
        accepted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn registry() -> Json {
        json!({"floors":[],"areas":[],"devices":[{"id":"fixture-device","name":"Fixture"}],"entities":[
            {"id":"select-fixture","entity_id":"select.fixture","device_id":"fixture-device"},
            {"id":"climate-fixture","entity_id":"climate.fixture","device_id":"fixture-device"},
            {"id":"media-fixture","entity_id":"media_player.fixture","device_id":"fixture-device"},
            {"id":"sensor-enum","entity_id":"sensor.enum_fixture","original_device_class":"enum","device_id":"fixture-device"},
            {"id":"sensor-legacy","entity_id":"sensor.legacy_fixture","device_id":"fixture-device"}
        ]})
    }
    fn house(file: bool) -> House {
        let mut house = House::new("moli-probe/pont/enum-fixture".into());
        house.use_file_metadata = file;
        house
            .registry(&serde_json::to_vec(&registry()).unwrap())
            .unwrap();
        house
    }
    fn send(house: &mut House, entity: &str, attribute: &str, payload: &str) -> bool {
        let (domain, object) = entity.split_once('.').unwrap();
        house.ingest(
            &format!("homeassistant/statestream/{domain}/{object}/{attribute}"),
            payload.as_bytes(),
            true,
        )
    }
    fn channel<'a>(house: &'a House, entity: &str, attribute: &str) -> &'a Channel {
        let source = if attribute == "state" {
            entity.to_owned()
        } else {
            format!("{entity}/{attribute}")
        };
        house
            .channels
            .values()
            .find(|channel| channel.source == source)
            .unwrap()
    }

    #[test]
    fn snapshot_numeric_states_use_native_payloads_and_reach_core_as_numbers() {
        let mut house = house(false);
        assert!(house.ingest_snapshot(&json!({"entity_id":"sensor.legacy_fixture","state":"21.5","attributes":{"device_class":"temperature","unit_of_measurement":"°C"}}),true));
        let value = channel(&house, "sensor.legacy_fixture", "state");
        assert_eq!(value.valeur, json!(21.5));
        assert_eq!(
            super::super::hub_projection::channel_value(value),
            moli_core::Value::Float(21.5)
        );
        assert!(house.ingest_snapshot(
            &json!({"entity_id":"sensor.legacy_fixture","state":"21","attributes":{}}),
            true
        ));
        assert_eq!(
            super::super::hub_projection::channel_value(channel(
                &house,
                "sensor.legacy_fixture",
                "state"
            )),
            moli_core::Value::Int(21)
        );
        assert!(house.ingest_snapshot(
            &json!({"entity_id":"sensor.legacy_fixture","state":"standby","attributes":{}}),
            true
        ));
        assert_eq!(
            channel(&house, "sensor.legacy_fixture", "state").valeur,
            "standby"
        );
    }

    #[test]
    fn enum_state_lexemes_keep_exponents_decimals_quotes_and_whitespace_exactly() {
        let mut house = house(false);
        let natives = ["1e2", "100.0", "\"x\"", " padded "];
        assert!(send(
            &mut house,
            "select.fixture",
            "options",
            &serde_json::to_string(&natives).unwrap()
        ));
        for (code, native) in natives.iter().enumerate() {
            assert!(send(&mut house, "select.fixture", "state", native));
            assert_eq!(
                channel(&house, "select.fixture", "state").valeur,
                json!(code)
            );
            assert!(house.ingest_snapshot(
                &json!({"entity_id":"select.fixture","state":native,"attributes":{}}),
                true
            ));
            let channel = channel(&house, "select.fixture", "state");
            assert_eq!(channel.valeur, json!(code));
            assert_eq!(
                house
                    .enum_pool
                    .decode(channel.enum_ref.unwrap(), code as u8),
                Some(*native)
            );
        }
        assert!(house.ingest_snapshot(&json!({"entity_id":"media_player.fixture","state":"playing","attributes":{"app_id":"quoted.app","app_name":"Application"}}),true));
        let channel = channel(&house, "media_player.fixture", "app_id");
        assert_eq!(
            house.enum_pool.decode(
                channel.enum_ref.unwrap(),
                channel.valeur.as_u64().unwrap() as u8
            ),
            Some("quoted.app")
        );
    }

    #[test]
    fn enum_states_before_and_after_lists_preserve_numeric_looking_native_strings() {
        for file in [false, true] {
            let mut house = house(file);
            assert!(send(&mut house, "select.fixture", "state", "1"));
            assert_eq!(
                channel(&house, "select.fixture", "state").qualite,
                "catalogue_absent"
            );
            assert!(channel(&house, "select.fixture", "state").valeur.is_null());
            assert!(send(
                &mut house,
                "select.fixture",
                "options",
                "[\"1\",\"2\"]"
            ));
            let selected = channel(&house, "select.fixture", "state");
            let reference = selected.enum_ref.unwrap();
            assert_eq!(selected.valeur, 0);
            assert_eq!(selected.qualite, "retained_age_inconnu");
            assert_eq!(house.enum_pool.decode(reference, 0), Some("1"));
            assert!(house.enum_pending.is_empty());
            let output = house.publications().unwrap();
            assert_eq!(output[0].1["type"], "description");
            assert_eq!(
                output[0].1["catalogues"][reference.id.to_string()]["entries"][0]["native"],
                "1"
            );
            let state = &output
                .iter()
                .find(|(_, payload)| {
                    payload["type"] == "canal" && payload["id"] == id("canal", "select-fixture")
                })
                .unwrap()
                .1;
            assert_eq!(state["encodage"], "enum");
            assert_eq!(state["catalogue"], reference.id);
            assert_eq!(state["valeur"], 0);
            assert!(send(
                &mut house,
                "select.fixture",
                "options",
                "[\"2\",\"1\"]"
            ));
            assert_eq!(
                channel(&house, "select.fixture", "state").enum_ref,
                Some(reference)
            );
            assert!(send(
                &mut house,
                "select.fixture",
                "options",
                "[\"2\",\"3\"]"
            ));
            let next = channel(&house, "select.fixture", "state").enum_ref.unwrap();
            assert_ne!(next, reference);
            assert_eq!(
                channel(&house, "select.fixture", "state").qualite,
                "valeur_non_reconnue"
            );
            assert_eq!(house.enum_pool.encode(next, "2"), Some(1));
            assert_eq!(house.enum_pool.encode(next, "3"), Some(2));
            assert!(send(&mut house, "select.fixture", "state", "3"));
            assert_eq!(channel(&house, "select.fixture", "state").valeur, 2);
            assert!(send(&mut house, "select.fixture", "state", "unavailable"));
            assert!(channel(&house, "select.fixture", "state").valeur.is_null());
            assert_eq!(
                channel(&house, "select.fixture", "state").qualite,
                "inconnu"
            );
        }
    }

    #[test]
    fn late_schema_does_not_reconstruct_a_lost_numeric_lexeme_or_invent_unknown_choices() {
        let mut house = house(false);
        assert!(send(&mut house, "sensor.legacy_fixture", "state", "1"));
        assert_eq!(channel(&house, "sensor.legacy_fixture", "state").valeur, 1);
        assert!(send(
            &mut house,
            "sensor.legacy_fixture",
            "options",
            "[\"1\",\"01\"]"
        ));
        assert!(
            channel(&house, "sensor.legacy_fixture", "state")
                .valeur
                .is_null()
        );
        assert!(send(&mut house, "sensor.legacy_fixture", "state", "01"));
        assert_eq!(channel(&house, "sensor.legacy_fixture", "state").valeur, 1);
        let reference = channel(&house, "sensor.legacy_fixture", "state").enum_ref;
        assert!(send(&mut house, "sensor.legacy_fixture", "state", "new"));
        assert_eq!(
            channel(&house, "sensor.legacy_fixture", "state").qualite,
            "valeur_non_reconnue"
        );
        assert_eq!(
            channel(&house, "sensor.legacy_fixture", "state").enum_ref,
            reference
        );
        assert!(send(&mut house, "sensor.enum_fixture", "state", "1"));
        assert!(send(
            &mut house,
            "sensor.enum_fixture",
            "options",
            "[\"1\"]"
        ));
        assert_eq!(channel(&house, "sensor.enum_fixture", "state").valeur, 0);
    }

    #[test]
    fn registry_schema_promotion_does_not_reinterpret_an_old_numeric_observation() {
        let mut house = house(false);
        assert!(send(&mut house, "sensor.legacy_fixture", "state", "1"));
        let mut data = registry();
        data["entities"][4]["device_class"] = json!("enum");
        house.registry(&serde_json::to_vec(&data).unwrap()).unwrap();
        assert!(channel(&house, "sensor.legacy_fixture", "state").enum_expected);
        assert!(
            channel(&house, "sensor.legacy_fixture", "state")
                .valeur
                .is_null()
        );
        assert!(send(
            &mut house,
            "sensor.legacy_fixture",
            "options",
            r#"["1"]"#
        ));
        assert!(
            channel(&house, "sensor.legacy_fixture", "state")
                .valeur
                .is_null()
        );
        assert!(send(&mut house, "sensor.legacy_fixture", "state", "1"));
        assert_eq!(channel(&house, "sensor.legacy_fixture", "state").valeur, 0);
    }

    #[test]
    fn enum_schema_never_leaks_to_numeric_attributes_and_source_lists_are_cold() {
        let mut house = house(false);
        assert!(send(
            &mut house,
            "climate.fixture",
            "hvac_modes",
            "[\"off\",\"heat\"]"
        ));
        assert!(send(&mut house, "climate.fixture", "state", "heat"));
        assert!(send(
            &mut house,
            "climate.fixture",
            "current_temperature",
            "21.5"
        ));
        assert!(send(&mut house, "climate.fixture", "brightness", "128"));
        for attribute in ["current_temperature", "brightness"] {
            assert!(
                channel(&house, "climate.fixture", attribute)
                    .enum_ref
                    .is_none()
            );
            assert!(!channel(&house, "climate.fixture", attribute).enum_expected);
        }
        assert!(send(
            &mut house,
            "media_player.fixture",
            "source_list",
            "[\"HDMI 1\",\"HDMI 2\"]"
        ));
        assert!(send(&mut house, "media_player.fixture", "source", "HDMI 2"));
        assert_eq!(channel(&house, "media_player.fixture", "source").valeur, 1);
        assert!(
            house
                .channels
                .values()
                .all(|channel| !channel.valeur.is_array() && !channel.valeur.is_object())
        );
    }

    #[test]
    fn application_empty_ids_never_create_codes_and_missing_names_keep_known_labels() {
        let mut house = house(false);
        assert!(send(
            &mut house,
            "media_player.fixture",
            "app_id",
            &serde_json::to_string("").unwrap()
        ));
        let app = channel(&house, "media_player.fixture", "app_id");
        assert!(app.valeur.is_null());
        assert!(app.enum_ref.is_none());
        assert!(!send(
            &mut house,
            "media_player.fixture",
            "applications",
            r#"[{"id":"","label":"invalid"}]"#
        ));
        assert!(!send(
            &mut house,
            "media_player.fixture",
            "apps",
            r#"{"":"invalid"}"#
        ));
        assert!(house.ingest_snapshot(&json!({"entity_id":"media_player.fixture","state":"playing","attributes":{"app_id":"app.fixture","app_name":"Known label"}}),true));
        let reference = channel(&house, "media_player.fixture", "app_id")
            .enum_ref
            .unwrap();
        for name in ["unknown", "unavailable", "None", "null"] {
            assert!(send(&mut house, "media_player.fixture", "app_name", name));
            assert_eq!(
                channel(&house, "media_player.fixture", "app_id").enum_ref,
                Some(reference)
            );
            assert_eq!(
                house.enum_pool.descriptor(reference).unwrap()["entries"][0]["label"],
                "Known label"
            );
        }
        assert!(send(&mut house, "select.fixture", "options", r#"[""]"#));
        assert!(send(&mut house, "select.fixture", "state", ""));
        assert_eq!(channel(&house, "select.fixture", "state").valeur, 0);
    }

    #[test]
    fn application_snapshots_associate_exact_ids_and_names_without_a_hot_name_channel() {
        let mut house = house(false);
        assert!(send(
            &mut house,
            "media_player.fixture",
            "app_name",
            "Before ID"
        ));
        assert!(house.ingest_snapshot(&json!({"entity_id":"media_player.fixture","state":"playing","attributes":{"app_id":"app.a","app_name":"Application A","applications":[{"id":"app.a","label":"A source"}]}}),true));
        let app = channel(&house, "media_player.fixture", "app_id");
        let first = app.enum_ref.unwrap();
        let first_code = app.valeur.as_u64().unwrap() as u8;
        assert_eq!(house.enum_pool.decode(first, first_code), Some("app.a"));
        assert_eq!(
            house.enum_pool.descriptor(first).unwrap()["entries"][first_code as usize]["label"],
            "Application A"
        );
        assert!(house.ingest_snapshot(&json!({"entity_id":"media_player.fixture","state":"playing","attributes":{"app_id":"app.b","app_name":"Application B"}}),false));
        let app = channel(&house, "media_player.fixture", "app_id");
        let next = app.enum_ref.unwrap();
        let code = app.valeur.as_u64().unwrap() as u8;
        let description = house.enum_pool.descriptor(next).unwrap();
        assert_eq!(
            description["entries"][first_code as usize]["label"],
            "Application A"
        );
        assert_eq!(description["entries"][code as usize]["native"], "app.b");
        assert_eq!(
            description["entries"][code as usize]["label"],
            "Application B"
        );
        assert!(
            !house
                .channels
                .values()
                .any(|channel| channel.source.ends_with("/app_name"))
        );
        let output = house.publications().unwrap();
        let config = output
            .iter()
            .position(|(_, value)| value["type"] == "description")
            .unwrap();
        let state = output
            .iter()
            .position(|(_, value)| {
                value["source"] == "media_player.fixture/app_id"
                    || value["id"] == target(&id("canal", "media-fixture"), "app_id")
            })
            .unwrap();
        assert!(config < state);
    }

    #[test]
    fn unknown_native_waits_for_a_new_catalogue_without_fabricating_receipt_time() {
        let mut house = house(false);
        assert!(send(&mut house, "select.fixture", "options", r#"["a"]"#));
        assert!(house.ingest(
            "homeassistant/statestream/select/fixture/state",
            b"b",
            false
        ));
        let key = id("canal", "select-fixture");
        assert_eq!(house.channels[&key].qualite, "valeur_non_reconnue");
        let received = house.channels[&key].recu_ms;
        assert_eq!(house.enum_pending[&key].received, received.unwrap());
        assert!(send(
            &mut house,
            "select.fixture",
            "options",
            r#"["a","b"]"#
        ));
        assert_eq!(house.channels[&key].valeur, 1);
        assert_eq!(house.channels[&key].recu_ms, received);
        assert!(!house.channels[&key].retenu);
        assert_eq!(house.channels[&key].qualite, "observe");
        assert!(send(&mut house, "select.fixture", "state", "c"));
        house.enum_pending.get_mut(&key).unwrap().received =
            now_ms().saturating_sub(PENDING_AGE_MS + 1);
        assert!(send(
            &mut house,
            "select.fixture",
            "options",
            r#"["a","b","c"]"#
        ));
        assert!(house.channels[&key].valeur.is_null());
        assert!(house.enum_pending.is_empty());
    }

    #[test]
    fn pending_unknown_survives_an_intermediate_catalogue_that_still_omits_it() {
        let mut house = house(false);
        assert!(send(&mut house, "select.fixture", "options", r#"["a"]"#));
        assert!(send(&mut house, "select.fixture", "state", "b"));
        let received = channel(&house, "select.fixture", "state").recu_ms;
        assert!(send(
            &mut house,
            "select.fixture",
            "options",
            r#"["a","c"]"#
        ));
        assert_eq!(
            house.enum_pending[&id("canal", "select-fixture")].native,
            "b"
        );
        assert!(send(
            &mut house,
            "select.fixture",
            "options",
            r#"["a","b","c"]"#
        ));
        assert_eq!(channel(&house, "select.fixture", "state").valeur, 2);
        assert_eq!(channel(&house, "select.fixture", "state").recu_ms, received);
        assert!(house.enum_pending.is_empty());
    }

    #[test]
    fn registry_domain_changes_drop_old_pending_native_observations() {
        let mut house = house(false);
        assert!(send(&mut house, "select.fixture", "state", "old-native"));
        let mut data = registry();
        data["entities"][0]["entity_id"] = json!("sensor.replacement");
        data["entities"][0]["device_class"] = json!("enum");
        house.registry(&serde_json::to_vec(&data).unwrap()).unwrap();
        assert!(house.enum_pending.is_empty());
        assert!(send(
            &mut house,
            "sensor.replacement",
            "options",
            r#"["old-native"]"#
        ));
        assert!(
            channel(&house, "sensor.replacement", "state")
                .valeur
                .is_null()
        );
    }

    #[test]
    fn pending_lexemes_have_a_global_entry_byte_and_lifetime_bound() {
        let mut data = registry();
        for index in 0..=MAX_PENDING {
            data["entities"].as_array_mut().unwrap().push(json!({"id":format!("pending-{index}"),"entity_id":format!("select.pending_{index}")}));
        }
        let mut house = House::new("moli-probe/pont/enum-fixture".into());
        house.registry(&serde_json::to_vec(&data).unwrap()).unwrap();
        let native = "x".repeat(256);
        for index in 0..=MAX_PENDING {
            assert!(send(
                &mut house,
                &format!("select.pending_{index}"),
                "state",
                &native
            ));
        }
        assert_eq!(house.enum_pending.len(), MAX_PENDING);
        assert_eq!(
            house
                .enum_pending
                .values()
                .map(|pending| pending.native.len())
                .sum::<usize>(),
            MAX_PENDING_BYTES
        );
        for pending in house.enum_pending.values_mut() {
            pending.received = now_ms().saturating_sub(PENDING_AGE_MS + 1);
        }
        house.publications().unwrap();
        assert!(house.enum_pending.is_empty());
        assert!(send(
            &mut house,
            "select.pending_0",
            "options",
            &serde_json::to_string(&vec![native]).unwrap()
        ));
        assert!(
            channel(&house, "select.pending_0", "state")
                .valeur
                .is_null()
        );
    }

    #[test]
    fn cold_schema_payloads_can_exceed_four_kib_without_relaxing_scalar_bounds() {
        let mut house = house(false);
        let values: Vec<_> = (0..256)
            .map(|index| format!("{index:03}{}", "x".repeat(253)))
            .collect();
        let payload = serde_json::to_vec(&values).unwrap();
        assert!(payload.len() > MAX_TEXT);
        assert!(house.ingest(
            "homeassistant/statestream/select/fixture/options",
            &payload,
            true
        ));
        assert_eq!(
            house
                .enum_pool
                .active_codes(channel(&house, "select.fixture", "state").enum_ref.unwrap())
                .unwrap()
                .len(),
            256
        );
        assert!(!house.ingest(
            "homeassistant/statestream/media_player/fixture/media_title",
            &vec![b'x'; MAX_TEXT + 1],
            false
        ));
        assert!(send(
            &mut house,
            "select.fixture",
            "state",
            &"x".repeat(257)
        ));
        assert!(house.enum_pending.is_empty());
        assert!(channel(&house, "select.fixture", "state").valeur.is_null());
        assert_eq!(house.counts()["pending_native_bytes"], 0);
    }

    #[test]
    fn enum_typing_ignores_configuration_roles_and_never_aggregates_codes_as_temperature() {
        let mut data = registry();
        data["entities"][3]["entity_category"] = json!("diagnostic");
        data["entities"][3]["device_class"] = json!("temperature");
        data["entities"][3]["unit_of_measurement"] = json!("°C");
        data["entities"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"water","entity_id":"water_heater.fixture"}));
        data["entities"].as_array_mut().unwrap().push(json!({"id":"enum-temperature","entity_id":"sensor.temperature_enum","device_class":"temperature","original_device_class":"enum","unit_of_measurement":"°C"}));
        let mut house = House::new("moli-probe/pont/enum-fixture".into());
        house.registry(&serde_json::to_vec(&data).unwrap()).unwrap();
        assert!(send(
            &mut house,
            "sensor.temperature_enum",
            "options",
            r#"["1","2"]"#
        ));
        assert!(send(&mut house, "sensor.temperature_enum", "state", "2"));
        assert_eq!(
            channel(&house, "sensor.temperature_enum", "state").semantique,
            "temperature"
        );
        assert_eq!(
            channel(&house, "sensor.temperature_enum", "state").valeur,
            1
        );
        assert!(channel(&house, "sensor.enum_fixture", "state").enum_expected);
        assert!(send(
            &mut house,
            "sensor.enum_fixture",
            "options",
            r#"["1","2"]"#
        ));
        assert!(send(&mut house, "sensor.enum_fixture", "state", "2"));
        assert_eq!(channel(&house, "sensor.enum_fixture", "state").valeur, 1);
        assert!(send(
            &mut house,
            "water_heater.fixture",
            "operation_mode",
            "eco"
        ));
        assert!(send(
            &mut house,
            "water_heater.fixture",
            "operation_list",
            r#"["eco","on"]"#
        ));
        assert_eq!(
            channel(&house, "water_heater.fixture", "operation_mode").valeur,
            0
        );
        assert_eq!(
            channel(&house, "water_heater.fixture", "state").enum_ref,
            channel(&house, "water_heater.fixture", "operation_mode").enum_ref
        );
        assert!(
            house
                .generics("maison")
                .iter()
                .all(|generic| generic["semantique"] != "temperature")
        );
    }

    #[test]
    fn failed_enum_cache_write_keeps_description_and_state_pending_for_retry() {
        let directory = std::env::temp_dir().join(format!(
            "moli-house-enum-failure-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let store = std::sync::Arc::new(
            crate::enum_store::Store::new(directory.clone(), "moli-probe/pont/enum-fixture".into())
                .unwrap(),
        );
        let mut house = house(false);
        house.enum_pool = store.load_pool().unwrap();
        house.enum_store = Some(store);
        assert!(send(&mut house, "select.fixture", "options", r#"["a"]"#));
        assert!(send(&mut house, "select.fixture", "state", "a"));
        house.publications().unwrap();
        let file = std::fs::read_dir(&directory)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let saved = std::fs::read(&file).unwrap();
        std::fs::write(&file, b"{").unwrap();
        assert!(send(
            &mut house,
            "select.fixture",
            "options",
            r#"["a","b"]"#
        ));
        assert!(house.publications().is_err());
        assert!(house.config_dirty);
        assert!(
            house
                .dirty_channels
                .contains(&id("canal", "select-fixture"))
        );
        std::fs::write(&file, saved).unwrap();
        let output = house.publications().unwrap();
        assert_eq!(output[0].1["type"], "description");
        assert!(
            output
                .iter()
                .any(|(_, payload)| payload["type"] == "canal" && payload["encodage"] == "enum")
        );
        drop(house);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn registry_and_private_cache_restart_keep_catalogue_codes_without_state_writes() {
        let directory = std::env::temp_dir().join(format!(
            "moli-house-enum-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let store = std::sync::Arc::new(
            crate::enum_store::Store::new(directory.clone(), "moli-probe/pont/enum-fixture".into())
                .unwrap(),
        );
        let mut house = house(true);
        house.enum_pool = store.load_pool().unwrap();
        house.enum_store = Some(store.clone());
        assert!(send(
            &mut house,
            "select.fixture",
            "options",
            "[\"1\",\"2\"]"
        ));
        assert!(send(&mut house, "select.fixture", "state", "2"));
        house.publications().unwrap();
        let reference = channel(&house, "select.fixture", "state").enum_ref.unwrap();
        let files: Vec<_> = std::fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        let before = std::fs::read(&files[0]).unwrap();
        let modified = std::fs::metadata(&files[0]).unwrap().modified().unwrap();
        assert!(send(&mut house, "select.fixture", "state", "1"));
        house.publications().unwrap();
        assert_eq!(std::fs::read(&files[0]).unwrap(), before);
        assert_eq!(
            std::fs::metadata(&files[0]).unwrap().modified().unwrap(),
            modified
        );
        house
            .registry(&serde_json::to_vec(&registry()).unwrap())
            .unwrap();
        assert_eq!(
            channel(&house, "select.fixture", "state").enum_ref,
            Some(reference)
        );
        let mut restored = House::new("moli-probe/pont/enum-fixture".into());
        restored.enum_pool = store.load_pool().unwrap();
        restored.enum_store = Some(store);
        restored
            .registry(&serde_json::to_vec(&registry()).unwrap())
            .unwrap();
        assert!(send(&mut restored, "select.fixture", "state", "2"));
        assert_eq!(
            channel(&restored, "select.fixture", "state").enum_ref,
            Some(reference)
        );
        assert_eq!(channel(&restored, "select.fixture", "state").valeur, 1);
        drop(house);
        drop(restored);
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[cfg(test)]
mod wire_snapshot_tests {
    use super::*;
    use bytes::BytesMut;
    use rumqttc::mqttbytes::v4::{
        ConnAck, ConnectReturnCode, Packet, PubAck, Publish, SubAck, SubscribeReasonCode,
    };
    use rumqttc::mqttbytes::{Error as CodecError, QoS};
    use std::io::{Read, Write};
    use std::sync::{Arc, Mutex};

    fn write(stream: &mut std::net::TcpStream, packet: Packet) {
        let mut bytes = BytesMut::new();
        packet.write(&mut bytes, 4 * 1024 * 1024).unwrap();
        stream.write_all(&bytes).unwrap();
    }
    fn publish(stream: &mut std::net::TcpStream, topic: &str, payload: &Json) {
        write(
            stream,
            Packet::Publish(Publish::new(
                topic,
                QoS::AtMostOnce,
                payload.to_string().into_bytes(),
            )),
        );
    }

    #[tokio::test]
    async fn normalized_mqtt_snapshots_reach_real_hub_as_enum_codes_and_publish_schema_first() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let received = Arc::new(Mutex::new(Vec::<Json>::new()));
        let output = received.clone();
        let broker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(4)))
                .unwrap();
            let mut buffer = BytesMut::new();
            loop {
                let packet = match Packet::read(&mut buffer, 4 * 1024 * 1024) {
                    Ok(packet) => packet,
                    Err(CodecError::InsufficientBytes(_)) => {
                        let mut bytes = [0; 8192];
                        match stream.read(&mut bytes) {
                            Ok(0) | Err(_) => break,
                            Ok(read) => buffer.extend_from_slice(&bytes[..read]),
                        };
                        continue;
                    }
                    Err(_) => break,
                };
                match packet {
                    Packet::Connect(_) => write(
                        &mut stream,
                        Packet::ConnAck(ConnAck::new(ConnectReturnCode::Success, false)),
                    ),
                    Packet::Subscribe(subscribe) => {
                        let snapshot = subscribe
                            .filters
                            .iter()
                            .any(|filter| filter.path == "moli-probe/source/z2m-0/snapshot");
                        write(
                            &mut stream,
                            Packet::SubAck(SubAck::new(
                                subscribe.pkid,
                                vec![
                                    SubscribeReasonCode::Success(QoS::AtMostOnce);
                                    subscribe.filters.len()
                                ],
                            )),
                        );
                        if snapshot {
                            // Explicitly rejected before the registry; no snapshot replay buffer.
                            publish(
                                &mut stream,
                                "moli-probe/source/z2m-0/snapshot",
                                &json!({"entity_id":"select.fixture","state":"1","attributes":{"options":["1","2"]}}),
                            );
                            publish(
                                &mut stream,
                                "moli-probe/source/registry",
                                &json!({"floors":[],"areas":[],"devices":[],"entities":[{"id":"fixture","entity_id":"select.fixture"}]}),
                            );
                            publish(
                                &mut stream,
                                "moli-probe/source/z2m-0/snapshot",
                                &json!({"entity_id":"select.fixture","state":"2","attributes":{"options":["1","2"]}}),
                            );
                            // A second source cannot overwrite this source's normalized snapshot.
                            publish(
                                &mut stream,
                                "moli-probe/source/z2m-1/snapshot",
                                &json!({"entity_id":"select.fixture","state":"1","attributes":{"options":["1","2"]}}),
                            );
                        }
                    }
                    Packet::Publish(publication) => {
                        if publication.qos == QoS::AtLeastOnce {
                            write(&mut stream, Packet::PubAck(PubAck::new(publication.pkid)));
                        }
                        if let Ok(value) = serde_json::from_slice(&publication.payload) {
                            output.lock().unwrap().push(value);
                        }
                    }
                    Packet::PingReq => write(&mut stream, Packet::PingResp),
                    Packet::Disconnect => break,
                    _ => {}
                }
            }
        });
        let config = crate::Config {
            hosts: vec!["127.0.0.1".into()],
            port,
            username: "fixture".into(),
            base: "zigbee2mqtt".into(),
            seconds: 5,
            bridges: 1,
            benchmark: false,
            all_topics: false,
            republish: Some("moli-probe".into()),
            republish_retain: true,
            republish_clear: false,
            virtual_tree: true,
            self_test_commands: false,
            virtual_mirror: true,
            publish_config: true,
            destination: None,
            metadata_memory: true,
        };
        let hub = moli_runtime::Hub::new(moli_runtime::HubOptions::default()).unwrap();
        let cancel = tokio_util::sync::CancellationToken::new();
        let running = crate::start_bridge(&hub, &config, "fixture", 0, cancel.clone()).unwrap();
        let point = moli_core::PointId::from(format!(
            "z2m-0:{}/{}",
            id("entite", "fixture"),
            id("canal", "fixture")
        ));
        let complete = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let in_hub = hub
                    .state(&point)
                    .is_some_and(|sample| sample.value == moli_core::Value::EnumCode(1));
                let published = received.lock().unwrap().iter().any(|payload| {
                    payload["type"] == "canal"
                        && payload["encodage"] == "enum"
                        && payload["valeur"] == 1
                });
                if in_hub && published {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        cancel.cancel();
        running.driver.await.unwrap();
        running.events.await.unwrap();
        broker.join().unwrap();
        complete.expect("snapshot must reach the real Hub as EnumCode");
        let output = received.lock().unwrap();
        let state = output
            .iter()
            .position(|payload| {
                payload["type"] == "canal"
                    && payload["encodage"] == "enum"
                    && payload["valeur"] == 1
            })
            .unwrap();
        let reference = output[state]["catalogue"].as_u64().unwrap().to_string();
        let description = output
            .iter()
            .position(|payload| {
                payload["type"] == "description"
                    && payload["catalogues"][&reference]["entries"][1]["native"] == "2"
            })
            .unwrap();
        assert!(description < state);
    }
}
