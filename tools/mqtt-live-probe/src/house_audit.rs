//! Offline replay through the same API-to-MQTT projection, with no persistence.
use super::*;
use moli_core::Value;
use std::path::PathBuf;

#[derive(Default)]
struct Counts {
    forms: BTreeMap<&'static str, usize>,
    text_bytes: usize,
    text_buckets: BTreeMap<&'static str, usize>,
}
impl Counts {
    fn record(&mut self, value: &Value) {
        let form = match value {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Text(_) => "text",
            Value::EnumCode(_) => "enum_code",
        };
        *self.forms.entry(form).or_default() += 1;
        if let Value::Text(text) = value {
            self.text_bytes += text.len();
            *self
                .text_buckets
                .entry(crate::text_bucket(text.len()))
                .or_default() += 1;
        }
    }
    fn json(&self) -> Json {
        json!({"forms":self.forms,"text_utf8_bytes":self.text_bytes,"text_buckets":self.text_buckets})
    }
}
fn old_value(value: Json, domain: &str) -> Value {
    if value.is_null()
        || value
            .as_str()
            .is_some_and(|s| matches!(s, "unknown" | "unavailable" | "None" | "null"))
    {
        return Value::Null;
    }
    if matches!(
        domain,
        "light" | "switch" | "binary_sensor" | "input_boolean"
    ) {
        if value.as_str().is_some_and(|s| matches!(s, "on" | "ON")) {
            return Value::Bool(true);
        }
        if value.as_str().is_some_and(|s| matches!(s, "off" | "OFF")) {
            return Value::Bool(false);
        }
    }
    Value::from_json(&value)
}
fn baseline(rows: &[Json]) -> Counts {
    let mut counts = Counts::default();
    for row in rows {
        let domain = text(row, "entity_id").split('.').next().unwrap_or("");
        if matches!(domain, "text" | "input_text") {
            continue;
        }
        let state = &row["state"];
        let decoded = state.as_str().map_or_else(
            || state.clone(),
            |s| serde_json::from_str(s).unwrap_or_else(|_| json!(s)),
        );
        counts.record(&old_value(decoded, domain));
        for (name, value) in row["attributes"].as_object().into_iter().flatten() {
            if !matches!(
                name.as_str(),
                "current_temperature"
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
                    | "color_temp_kelvin"
                    | "color_temp"
                    | "color_mode"
            ) || value.is_array()
                || value.is_object()
            {
                continue;
            }
            counts.record(&old_value(value.clone(), domain));
        }
    }
    counts
}

pub(super) fn run(paths: &[PathBuf]) -> Result<Json> {
    ensure!(
        !paths.is_empty() && paths.len() <= 4,
        "provide one to four state snapshots"
    );
    let mut sources = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        ensure!(
            path.metadata()?.len() <= 16 * 1024 * 1024,
            "snapshot too large"
        );
        let data: Json = serde_json::from_slice(&std::fs::read(path)?)?;
        let rows = data.as_array().context("state snapshot must be an array")?;
        ensure!(rows.len() <= MAX_ENTITIES, "too many snapshot entities");
        let mut entities = Vec::with_capacity(rows.len());
        for row in rows {
            let entity = text(row, "entity_id");
            ensure!(
                !entity.is_empty() && entity.len() <= 512,
                "invalid snapshot identity"
            );
            entities.push(json!({"id":entity,"entity_id":entity,"original_device_class":row["attributes"]["device_class"],"unit_of_measurement":row["attributes"]["unit_of_measurement"]}));
        }
        let mut house = House::new(format!("moli-probe/pont/offline-{index}"));
        house.enum_persistence_disabled = true;
        house.registry(&serde_json::to_vec(
            &json!({"floors":[],"areas":[],"devices":[],"entities":entities}),
        )?)?;
        let mut accepted = 0;
        for row in rows {
            accepted += usize::from(house.ingest_snapshot(row, true));
        }
        let publications = house.publications()?;
        let mut after = Counts::default();
        let mut qualities = BTreeMap::<&str, usize>::new();
        for channel in house.channels.values() {
            after.record(&hub_projection::channel_value(channel));
            *qualities.entry(channel.qualite).or_default() += 1;
        }
        let code_states = publications
            .iter()
            .filter(|(_, payload)| payload["type"] == "canal" && payload["encodage"] == "enum")
            .count();
        sources.push(json!({"source":char::from(b'A'+u8::try_from(index)?).to_string(),"snapshot_entities":rows.len(),"accepted_snapshots":accepted,"before":baseline(rows).json(),"after":after.json(),"qualities":qualities,"enum_wire_states":code_states,"projection":house.counts()}));
    }
    Ok(
        json!({"sources":sources,"scope":"offline API snapshots replayed through House; no MQTT connection, device command or file write; before excludes old rejected array attributes; quality/null and removed app_name channels are not counted as successful coded selections"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn baseline_preserves_the_old_numeric_misclassification_for_comparison() {
        let counts = baseline(&[
            json!({"entity_id":"select.fixture","state":"1","attributes":{"options":["1","2"]}}),
        ]);
        assert_eq!(counts.forms.get("int"), Some(&1));
        assert_eq!(counts.text_bytes, 0);
    }
}
