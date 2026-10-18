//! Last known values, persisted across restarts.
//!
//! Many devices (sleepy Zigbee sensors above all) only report on change or
//! every few hours. Without a cache, a restart would show "unknown" until
//! they speak again. Samples keep their original timestamp: the UI shows
//! how old a restored value is, nothing is passed off as fresh.

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::Path;
use std::sync::Arc;

use moli_core::{DeviceId, Sample};

pub(crate) type States = HashMap<DeviceId, BTreeMap<Arc<str>, Sample>>;

pub(crate) fn load(path: &Path) -> io::Result<States> {
    crate::private_files::discard_pending(path)?;
    match std::fs::read(path) {
        Ok(bytes) => {
            let mut states: States = if let Ok(states) = serde_json::from_slice(&bytes) {
                states
            } else {
                crate::private_files::replace(path, b"{}")?;
                tracing::warn!("invalid state cache discarded");
                return Ok(States::new());
            };
            let mut removed = 0;
            for values in states.values_mut() {
                values.retain(|key, sample| {
                    let keep = !moli_core::privacy::code_property(key)
                        && !moli_core::privacy::private_value(&sample.value);
                    removed += usize::from(!keep);
                    keep
                });
            }
            if removed != 0 {
                let clean = serde_json::to_vec(&states).map_err(io::Error::other)?;
                crate::private_files::replace(path, &clean)?;
                tracing::warn!(removed, "private keypad values removed from state cache");
            }
            Ok(states)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(States::new()),
        Err(e) => Err(e),
    }
}

pub(crate) async fn save(path: &Path, states: &States) -> io::Result<()> {
    let bytes = serde_json::to_vec(states).map_err(io::Error::other)?;
    crate::labels::write_atomic(path, &bytes).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture() -> (std::path::PathBuf, String) {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("moli-cache-private-{}-{stamp}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        (dir, format!("0{stamp}"))
    }
    #[test]
    fn startup_removes_codes_and_pending_copies_but_keeps_ordinary_states() {
        let (dir, code) = fixture();
        let path = dir.join("state.json");
        let bytes = serde_json::to_vec(&json!({"fixture:panel":{
            "action_code":{"value":code,"ts":1},"pin_code.user":{"value":code,"ts":1},
            "settings.pin":{"value":code,"ts":1},
            "settings.code":{"value":code,"ts":1},
            "settings.key_code":{"value":code,"ts":1},
            "action":{"value":json!({"pin_code":code}).to_string(),"ts":1},
            "action_after":{"value":format!(r#"{{"pin":"{code}","tail":}}"#),"ts":1},
            "action_before":{"value":format!(r#"{{"tail":,"pin":"{code}"}}"#),"ts":1},
            "action_range":{"value":format!(r#"{{"pin":"{code}","n":1e400}}"#),"ts":1},
            "battery":{"value":81,"ts":1}}}))
        .unwrap();
        std::fs::write(&path, &bytes).unwrap();
        std::fs::write(dir.join("state.json.tmp"), &bytes).unwrap();
        let states = load(&path).unwrap();
        assert_eq!(states[&DeviceId::from("fixture:panel")].len(), 1);
        let disk = std::fs::read(&path).unwrap();
        assert!(
            !disk.windows(code.len()).any(|part| part == code.as_bytes()),
            "credential left in current cache"
        );
        assert!(!dir.join("state.json.tmp").exists());
        assert_eq!(load(&path).unwrap(), states);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn failed_repair_is_not_an_empty_success_and_can_resume() {
        let (dir, code) = fixture();
        let path = dir.join("state.json");
        std::fs::write(
            &path,
            serde_json::to_vec(&json!({"fixture:panel":{"action_code":{"value":code,"ts":1}}}))
                .unwrap(),
        )
        .unwrap();
        std::fs::create_dir(dir.join("state.json.privacy.tmp")).unwrap();
        assert!(load(&path).is_err(), "repair failure must stop startup");
        std::fs::remove_dir(dir.join("state.json.privacy.tmp")).unwrap();
        assert!(
            load(&path)
                .unwrap()
                .values()
                .all(std::collections::BTreeMap::is_empty)
        );
        std::fs::write(&path, format!("{{invalid:{code}")).unwrap();
        assert!(load(&path).unwrap().is_empty());
        assert_eq!(std::fs::read(&path).unwrap(), b"{}");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
