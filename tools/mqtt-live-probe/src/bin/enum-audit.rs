//! Offline schema-first enum accounting. Never prints identities, labels or values.
#[path = "../enum_catalogue.rs"]
#[allow(dead_code)]
mod enum_catalogue;
#[path = "../../../moteur-integre-banc/src/memory.rs"]
#[allow(dead_code)]
mod memory;

#[global_allocator]
static ALLOCATOR: memory::Meter = memory::Meter;

use anyhow::{Context, Result, ensure};
use enum_catalogue::{Choice, Pool};
use serde_json::{Value as Json, json};
use std::{collections::BTreeMap, path::Path};

fn missing(value: &Json) -> bool {
    value.is_null()
        || value
            .as_str()
            .is_some_and(|v| matches!(v, "unknown" | "unavailable" | "None" | "null"))
}
fn choices(value: &Json) -> Option<Vec<Choice>> {
    let rows = value.as_array()?;
    if rows.len() > 256 {
        return None;
    }
    rows.iter()
        .map(|row| {
            let native = row.as_str()?;
            (native.len() <= 256).then(|| Choice {
                native: native.into(),
                label: native.into(),
            })
        })
        .collect()
}
fn old_text_bytes(value: &Json, domain: &str) -> usize {
    if missing(value) {
        return 0;
    }
    let Some(native) = value.as_str() else {
        return 0;
    };
    if matches!(
        domain,
        "light" | "binary_sensor" | "switch" | "input_boolean"
    ) && matches!(native, "ON" | "OFF" | "on" | "off")
    {
        return 0;
    }
    match serde_json::from_str::<Json>(native) {
        Ok(Json::String(text)) => text.len(),
        Ok(Json::Number(_) | Json::Bool(_) | Json::Null) => 0,
        _ => native.len(),
    }
}

// Inputs already exist before the baseline; transient Choice and JSON buffers
// are dropped before measuring the immutable conversion tables.
fn measured_option_tables(rows: &[Json]) -> Result<usize> {
    let before = memory::live();
    let mut pool = Pool::default();
    for row in rows {
        if let Some(options) = choices(&row["attributes"]["options"]) {
            pool.update(None, &options)?;
        }
    }
    Ok(memory::live().saturating_sub(before))
}

fn audit(rows: &[Json], source: &str) -> Result<Json> {
    ensure!(rows.len() <= 8192, "too many entities");
    let live_requested = measured_option_tables(rows)?;
    let mut pool = Pool::default();
    let mut options_pool = Pool::default();
    let mut counts = BTreeMap::<&str, u64>::new();
    let mut option_domains = BTreeMap::<String, u64>::new();
    let mut option_sets = 0;
    let mut option_entries = 0;
    let mut option_utf8 = 0;
    let mut option_known = 0;
    let mut option_unavailable = 0;
    let mut option_unrecognized = 0;
    let mut numeric_looking = 0;
    let mut old_total_text = 0;
    let mut remaining_text = 0;
    let mut coded_states = 0;
    let mut before_option_text = 0;
    for row in rows {
        let domain = row["entity_id"]
            .as_str()
            .unwrap_or("")
            .split('.')
            .next()
            .unwrap_or("");
        let state = &row["state"];
        let attrs = &row["attributes"];
        let baseline = old_text_bytes(state, domain);
        old_total_text += baseline;
        let key = if attrs["options"].is_array() {
            Some("options")
        } else if domain == "water_heater" && attrs["operation_list"].is_array() {
            Some("operation_list")
        } else if domain == "climate" && attrs["hvac_modes"].is_array() {
            Some("hvac_modes")
        } else {
            None
        };
        if let Some(key) = key {
            if let Some(options) = choices(&attrs[key]) {
                let reference = pool.update(None, &options)?;
                *counts.entry(key).or_default() += 1;
                if key == "options" {
                    options_pool.update(None, &options)?;
                    option_sets += 1;
                    option_entries += options.len();
                    option_utf8 += options.iter().map(|v| v.native.len()).sum::<usize>();
                    *option_domains.entry(domain.into()).or_default() += 1;
                    before_option_text += baseline;
                }
                if missing(state) {
                    if key == "options" {
                        option_unavailable += 1;
                    }
                } else if let Some(native) = state.as_str() {
                    if pool.encode(reference, native).is_some() {
                        coded_states += 1;
                        if key == "options" {
                            option_known += 1;
                            if native.parse::<f64>().is_ok() {
                                numeric_looking += 1;
                            }
                        }
                    } else if key == "options" {
                        option_unrecognized += 1;
                    }
                } else if key == "options" {
                    option_unrecognized += 1;
                }
                continue;
            }
            *counts.entry("invalid_schema").or_default() += 1;
        }
        remaining_text += baseline;
        if let Some(options) = choices(&attrs["source_list"]) {
            pool.update(None, &options)?;
            *counts.entry("source_list").or_default() += 1;
            if attrs["source"]
                .as_str()
                .is_some_and(|v| options.iter().any(|c| c.native == v))
            {
                *counts.entry("coded_media_source").or_default() += 1;
            }
        }
        if let Some(native) = attrs["app_id"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 256)
        {
            let label = attrs["app_name"]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 256)
                .unwrap_or(native);
            let reference = pool.update(
                None,
                &[Choice {
                    native: native.into(),
                    label: label.into(),
                }],
            )?;
            ensure!(
                pool.encode(reference, native).is_some(),
                "application selection missing"
            );
            *counts.entry("observed_application_pairs").or_default() += 1;
        }
    }
    Ok(
        json!({"source":source,"entities":rows.len(),"explicit_options":{"sets":option_sets,"entries":option_entries,"native_utf8_bytes":option_utf8,"domains":option_domains,"selected_known":option_known,"selected_unavailable":option_unavailable,"selected_unrecognized":option_unrecognized,"numeric_looking_values_kept_as_enum":numeric_looking,"old_selected_text_utf8_bytes":before_option_text,"new_selected_text_utf8_bytes":0,"selected_code_payload_bytes":option_known,"converter":options_pool.stats(),"table_live_requested_bytes":live_requested},"schemas":counts,"base_state_accounting":{"old_text_utf8_bytes":old_total_text,"remaining_text_utf8_bytes":remaining_text,"coded_selections":coded_states},"converter":pool.stats(),"scope":"offline snapshot; no commands; application catalogues contain observed pairs only, not all installed apps; allocator headers/RSS excluded"}),
    )
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        !args.is_empty() && args.len() <= 4,
        "provide one to four private states.json files"
    );
    let mut reports = Vec::new();
    for (index, path) in args.iter().enumerate() {
        let path = Path::new(path);
        ensure!(
            path.metadata()?.len() <= 16 * 1024 * 1024,
            "snapshot too large"
        );
        let data: Json =
            serde_json::from_slice(&std::fs::read(path)?).context("invalid snapshot")?;
        let rows = data.as_array().context("snapshot must be a state array")?;
        let source = char::from(b'A' + u8::try_from(index)?).to_string();
        reports.push(audit(rows, &source)?);
    }
    println!(
        "{}",
        json!({"sources":reports,"method":"explicit schemas precede numeric parsing; code payload size is distinct from the Value envelope and total process memory"})
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numeric_looking_enums_do_not_become_numbers_or_free_text() {
        let report=audit(&[json!({"entity_id":"select.fixture","state":"001","attributes":{"options":["1","001","1e2"]}}),json!({"entity_id":"sensor.fixture","state":"1e2","attributes":{"options":["1e2","100.0"]}})],"A").unwrap();
        assert_eq!(report["explicit_options"]["selected_known"], 2);
        assert_eq!(
            report["explicit_options"]["numeric_looking_values_kept_as_enum"],
            2
        );
        assert_eq!(
            report["explicit_options"]["new_selected_text_utf8_bytes"],
            0
        );
        assert_eq!(report["base_state_accounting"]["coded_selections"], 2);
    }
    #[test]
    fn unavailable_and_unrecognized_values_never_create_selections() {
        let report=audit(&[json!({"entity_id":"select.fixture","state":"unavailable","attributes":{"options":["a","b"]}}),json!({"entity_id":"select.second","state":"other","attributes":{"options":["a","b"]}})],"A").unwrap();
        assert_eq!(report["explicit_options"]["selected_unavailable"], 1);
        assert_eq!(report["explicit_options"]["selected_unrecognized"], 1);
        assert_eq!(report["explicit_options"]["selected_known"], 0);
    }
}
