//! Bounded, display-only metadata. Addresses and executable capabilities are
//! never inferred from a service's description.

use anyhow::{Context as _, ensure};
use serde_json::Value;
use std::collections::BTreeSet;

const MAX_PROGRAMS: usize = 16;
const MAX_ARTIFACTS: usize = 64;
const MAX_FIELD: usize = 128;
const MAX_LIST: usize = 2048;

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Info {
    pub models: String,
    pub program: String,
    pub version: String,
    pub voices: String,
}

pub(crate) fn parse(data: &Value) -> anyhow::Result<Info> {
    ensure!(data.is_object(), "expected a Wyoming info object");
    let mut models = BTreeSet::new();
    let mut programs = BTreeSet::new();
    let mut versions = BTreeSet::new();
    let mut voices = BTreeSet::new();
    for (domain, field) in [("asr", "models"), ("tts", "voices"), ("wake", "models")] {
        let Some(entries) = data.get(domain) else {
            continue;
        };
        let entries = entries
            .as_array()
            .context("invalid Wyoming program array")?;
        ensure!(entries.len() <= MAX_PROGRAMS, "too many Wyoming programs");
        for program in entries {
            let program_name = name(program)?;
            if let Some(version) = program.get("version").filter(|v| !v.is_null()) {
                let version = text(version.as_str().context("invalid Wyoming version")?);
                if !version.is_empty() {
                    versions.insert(format!("{program_name}: {version}"));
                }
            }
            programs.insert(program_name);
            let artifacts = program
                .get(field)
                .and_then(Value::as_array)
                .context("invalid Wyoming artifact array")?;
            ensure!(
                artifacts.len() <= MAX_ARTIFACTS,
                "too many Wyoming artifacts"
            );
            let into = if domain == "tts" {
                &mut voices
            } else {
                &mut models
            };
            for artifact in artifacts {
                into.insert(name(artifact)?);
            }
        }
    }
    ensure!(!programs.is_empty(), "no supported Wyoming program");
    Ok(Info {
        models: joined(models),
        program: joined(programs),
        version: joined(versions),
        voices: joined(voices),
    })
}

fn name(artifact: &Value) -> anyhow::Result<String> {
    let name = artifact
        .get("name")
        .and_then(Value::as_str)
        .context("invalid Wyoming artifact name")?;
    let name = text(name);
    ensure!(!name.is_empty(), "empty Wyoming artifact name");
    Ok(name)
}

fn text(value: &str) -> String {
    value
        .chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(c, '\u{200e}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .take(MAX_FIELD)
        .collect::<String>()
        .trim()
        .to_owned()
}

fn joined(values: BTreeSet<String>) -> String {
    values
        .into_iter()
        .collect::<Vec<_>>()
        .join(", ")
        .chars()
        .take(MAX_LIST)
        .collect()
}
