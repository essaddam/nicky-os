//! `moli-os backup <dir>` and `moli-os restore <dir>`: everything Moli knows
//! about the house, in one folder.
//!
//! - SQLite databases are copied with `VACUUM INTO`: a consistent snapshot,
//!   safe while Moli runs (WAL), compact, a standalone file.
//! - Small files are copied as they are (`secrets.enc` stays encrypted: the
//!   master key lives in a secrets manager, never in a backup).
//! - The house plan's images (`plan/`) go along with `plan.json`.
//! - `manifest.json` says when, which version, and each file's size.
//!
//! Restoring is for a stopped Moli and never overwrites without `--force`.
//! Every file is checked against the manifest before anything is written:
//! a damaged backup restores nothing rather than half of the house.

use std::path::Path;

use anyhow::{Context as _, bail};

/// SQLite databases: copied through SQLite itself.
const DATABASES: [&str; 2] = ["history.db", "energy.db"];
/// Everything else worth keeping (missing ones are skipped).
const FILES: [&str; 21] = [
    "moli.toml",
    "plan.json",
    "phones.json",
    "secrets.enc",
    "labels.toml",
    "home.json",
    "tuya.json",
    "automations.json",
    "automation-runs.jsonl",
    "button-occurrences.json",
    "automation-occurrences.json",
    "helper-counters.json",
    "helper-datetimes.json",
    "helper-timers.json",
    "journal.jsonl",
    "journal.jsonl.1",
    "state.json",
    "trusted-profiles.toml",
    "perf.jsonl",
    "bench.json",
    "assistant-exchanges.jsonl",
];

pub fn backup(data_dir: &Path, dir: &Path) -> anyhow::Result<()> {
    if dir.exists() && std::fs::read_dir(dir)?.next().is_some() {
        bail!(
            "{} is not empty: a backup goes into a new folder",
            dir.display()
        );
    }
    std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let mut files = serde_json::Map::new();
    for name in DATABASES {
        let source = data_dir.join(name);
        if !source.exists() {
            continue;
        }
        let target = dir.join(name);
        let db = rusqlite::Connection::open_with_flags(
            &source,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .with_context(|| format!("cannot open {name}"))?;
        db.execute("VACUUM INTO ?1", [target.to_string_lossy().as_ref()])
            .with_context(|| format!("cannot snapshot {name}"))?;
        files.insert(name.into(), std::fs::metadata(&target)?.len().into());
    }
    for name in FILES {
        let source = data_dir.join(name);
        if !source.exists() {
            continue;
        }
        let bytes = std::fs::copy(&source, dir.join(name))
            .with_context(|| format!("cannot copy {name}"))?;
        files.insert(name.into(), bytes.into());
    }
    if let Ok(entries) = std::fs::read_dir(data_dir.join("plan")) {
        std::fs::create_dir_all(dir.join("plan"))?;
        for entry in entries.filter_map(Result::ok) {
            let Some(name) = entry
                .file_name()
                .to_str()
                .filter(|n| moli_api::plan_image_name_ok(n))
                .map(str::to_owned)
            else {
                continue;
            };
            let bytes = std::fs::copy(entry.path(), dir.join("plan").join(&name))
                .with_context(|| format!("cannot copy plan/{name}"))?;
            files.insert(format!("plan/{name}"), bytes.into());
        }
    }
    let manifest = serde_json::json!({
        "moli_os": env!("CARGO_PKG_VERSION"),
        "taken_ms": moli_core::now_ms(),
        "files": files,
    });
    std::fs::write(
        dir.join("manifest.json"),
        serde_json::to_string_pretty(&manifest)? + "\n",
    )?;
    println!(
        "backup: {} files in {} (secrets stay encrypted; the master key is not in it)",
        files.len(),
        dir.display()
    );
    Ok(())
}

pub fn restore(data_dir: &Path, dir: &Path, force: bool) -> anyhow::Result<()> {
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(dir.join("manifest.json"))
            .with_context(|| format!("{} has no manifest.json: not a backup", dir.display()))?,
    )?;
    let listed = manifest["files"]
        .as_object()
        .context("manifest without files")?;
    let names: Vec<String> = listed.keys().cloned().collect();
    // Only names this program writes: a manifest never chooses a path.
    if let Some(odd) = names.iter().find(|n| !known_name(n)) {
        bail!("unexpected file in the manifest: {odd:?}");
    }
    // The whole backup is checked before the first byte is written.
    for name in &names {
        let expected = listed[name.as_str()].as_u64();
        let found = std::fs::metadata(dir.join(name)).map(|m| m.len()).ok();
        if expected.is_none() || found != expected {
            bail!(
                "{name}: {} bytes in the backup, {} in its manifest: damaged backup, nothing restored",
                found.map_or_else(|| "no".to_owned(), |n| n.to_string()),
                expected.map_or_else(|| "?".to_owned(), |n| n.to_string()),
            );
        }
    }
    let existing: Vec<&String> = names.iter().filter(|n| data_dir.join(n).exists()).collect();
    if !existing.is_empty() && !force {
        bail!(
            "{} already holds {} of these files: stop moli-os, then restore with --force",
            data_dir.display(),
            existing.len()
        );
    }
    std::fs::create_dir_all(data_dir)?;
    if names.iter().any(|n| n.starts_with("plan/")) {
        std::fs::create_dir_all(data_dir.join("plan"))?;
    }
    for name in &names {
        let target = data_dir.join(name);
        // A database restored next to a stale WAL would replay it.
        if DATABASES.contains(&name.as_str()) {
            for suffix in ["-wal", "-shm"] {
                let _ = std::fs::remove_file(data_dir.join(format!("{name}{suffix}")));
            }
        }
        std::fs::copy(dir.join(name), &target).with_context(|| format!("cannot restore {name}"))?;
    }
    println!(
        "restored {} files into {} (taken {} ms since the epoch)",
        names.len(),
        data_dir.display(),
        manifest["taken_ms"]
    );
    Ok(())
}

fn known_name(name: &str) -> bool {
    DATABASES.contains(&name)
        || FILES.contains(&name)
        || name
            .strip_prefix("plan/")
            .is_some_and(moli_api::plan_image_name_ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_h3_backup_keeps_temporal_definitions_and_canonical_values() {
        let root = std::env::temp_dir().join(format!(
            "moli-h3-backup-{}-{}",
            std::process::id(),
            moli_core::now_ms()
        ));
        let data = root.join("data");
        let saved = root.join("saved");
        let fresh = root.join("fresh");
        std::fs::create_dir_all(&data).unwrap();
        let bytes=br#"{"version":1,"values":{"fixture:sample":{"spec":{"format":"datetime","timezone":"Europe/Paris"},"value":"2026-10-25T01:30:00Z"}}}"#;
        std::fs::write(data.join("helper-datetimes.json"), bytes).unwrap();
        backup(&data, &saved).unwrap();
        restore(&fresh, &saved, false).unwrap();
        assert_eq!(
            std::fs::read(fresh.join("helper-datetimes.json")).unwrap(),
            bytes
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn auto_h1_backup_restores_both_delivery_horizons_and_activation_lineages() {
        let root = std::env::temp_dir().join(format!(
            "moli-h1-backup-{}-{}",
            std::process::id(),
            moli_core::now_ms()
        ));
        let data = root.join("data");
        let saved = root.join("saved");
        let fresh = root.join("fresh");
        std::fs::create_dir_all(&data).unwrap();
        let bundle = [
            ("helper-timers.json", br#"{"version":1,"timers":{"fixture:clock":{"generation":1,"pending":true,"audience":[]}},"clients":{}}"#.as_slice()),
            ("automation-occurrences.json", br#"{"version":1,"sources":{"fixture":{"highest":1}},"pending":{}}"#.as_slice()),
            ("automations.json", br#"[{"id":"expiry","activation":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}]"#.as_slice()),
        ];
        for (name, bytes) in bundle {
            std::fs::write(data.join(name), bytes).unwrap();
        }
        backup(&data, &saved).unwrap();
        restore(&fresh, &saved, false).unwrap();
        for (name, bytes) in bundle {
            assert_eq!(std::fs::read(fresh.join(name)).unwrap(), bytes, "{name}");
        }
        std::fs::write(saved.join("helper-timers.json"), b"damaged").unwrap();
        assert!(restore(&root.join("other"), &saved, false).is_err());
        assert!(!root.join("other").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn auto_h2_backup_restores_counter_values_and_receipts_together() {
        let root = std::env::temp_dir().join(format!(
            "moli-h2-backup-{}-{}",
            std::process::id(),
            moli_core::now_ms()
        ));
        let data = root.join("data");
        let saved = root.join("saved");
        let fresh = root.join("fresh");
        std::fs::create_dir_all(&data).unwrap();
        let contents = br#"{"version":1,"values":{"fixture:count":9},"clients":{"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa":{"highest":1,"requests":{"1":{"point":"fixture:count/increment","outcome":"applied"}}}}}"#;
        std::fs::write(data.join("helper-counters.json"), contents).unwrap();
        backup(&data, &saved).unwrap();
        assert_eq!(
            std::fs::read(saved.join("helper-counters.json")).unwrap(),
            contents
        );
        restore(&fresh, &saved, false).unwrap();
        assert_eq!(
            std::fs::read(fresh.join("helper-counters.json")).unwrap(),
            contents
        );
        std::fs::write(saved.join("helper-counters.json"), b"damaged").unwrap();
        assert!(restore(&root.join("other"), &saved, false).is_err());
        assert!(!root.join("other").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn auto_h4_backup_restore_does_not_republish_an_acknowledged_request() {
        use moli_core::{Event, Origin, PointId, Value};
        use moli_runtime::{Hub, HubOptions, StateFile, spawn_driver};
        use std::sync::Arc;
        use std::time::Duration;
        use tokio_util::sync::CancellationToken;

        let root = std::env::temp_dir().join(format!(
            "moli-h4-backup-{}-{}",
            std::process::id(),
            moli_core::now_ms()
        ));
        let data = root.join("data");
        let saved = root.join("saved");
        let fresh = root.join("fresh");
        std::fs::create_dir_all(&data).unwrap();
        // A valid, empty consumer state also belongs to the official backup.
        StateFile::new(Some(data.join("automation-occurrences.json")), 1024 * 1024)
            .write(&serde_json::json!({ "version": 1, "windows": {}, "pending": {} }))
            .await
            .unwrap();
        let point = PointId::from("fixture:button/press");
        let request = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:1".to_owned();
        for (boot, directory) in [&data, &fresh].into_iter().enumerate() {
            let hub = Hub::new(HubOptions {
                button_state_path: Some(directory.join("button-occurrences.json")),
                ..Default::default()
            })
            .unwrap();
            let cancel = CancellationToken::new();
            let config = serde_json::from_value(serde_json::json!({ "helper": [
                { "id": "button", "name": "Simulated button", "kind": "button" }
            ] }))
            .unwrap();
            spawn_driver(
                &hub,
                "fixture".into(),
                Arc::new(moli_helpers::Helpers::new(config).unwrap()),
                cancel.clone(),
            );
            for _ in 0..200 {
                if hub.device(&"fixture:button".into()).is_some() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            assert!(hub.device(&"fixture:button".into()).is_some());
            let mut events = hub.subscribe();
            hub.command_with_request_id(
                &point,
                Value::Null,
                Origin::Ui,
                None,
                Some(request.clone()),
            )
            .await
            .unwrap();
            let mut published = 0;
            while let Ok(event) = events.try_recv() {
                published += usize::from(matches!(event, Event::Occurrence { .. }));
            }
            assert_eq!(
                published,
                usize::from(boot == 0),
                "a request acknowledged before backup is not new after restore"
            );
            if boot == 0 {
                backup(&data, &saved).unwrap();
                let manifest: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(saved.join("manifest.json")).unwrap())
                        .unwrap();
                for name in ["button-occurrences.json", "automation-occurrences.json"] {
                    assert!(
                        manifest["files"][name].is_u64(),
                        "backup must include {name}"
                    );
                }
                restore(&fresh, &saved, false).unwrap();
                for name in ["button-occurrences.json", "automation-occurrences.json"] {
                    assert_eq!(
                        std::fs::read(data.join(name)).unwrap(),
                        std::fs::read(fresh.join(name)).unwrap()
                    );
                }
            }
            cancel.cancel();
        }
        std::fs::write(saved.join("button-occurrences.json"), b"damaged").unwrap();
        let other = root.join("other");
        assert!(restore(&other, &saved, false).is_err());
        assert!(
            !other.exists(),
            "validate all occurrence files before restoring anything"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_backup_restores_into_an_empty_home() {
        let base = std::env::temp_dir().join(format!("moli-backup-{}", std::process::id()));
        let (data, saved, fresh) = (base.join("data"), base.join("saved"), base.join("fresh"));
        std::fs::create_dir_all(&data).unwrap();
        let db = rusqlite::Connection::open(data.join("history.db")).unwrap();
        db.execute_batch(
            "PRAGMA journal_mode = WAL; CREATE TABLE t (v INTEGER); INSERT INTO t VALUES (42);",
        )
        .unwrap();
        std::fs::write(data.join("labels.toml"), "x = 1\n").unwrap();
        std::fs::write(data.join("assistant-exchanges.jsonl"), "{}\n").unwrap();
        std::fs::create_dir_all(data.join("plan")).unwrap();
        std::fs::write(data.join("plan").join("0123456789abcdef.png"), b"png").unwrap();
        std::fs::write(data.join("plan").join("notes.txt"), b"not a plan").unwrap();

        backup(&data, &saved).unwrap();
        assert!(
            backup(&data, &saved).is_err(),
            "never into a non-empty folder"
        );
        restore(&fresh, &saved, false).unwrap();
        let back = rusqlite::Connection::open(fresh.join("history.db")).unwrap();
        let v: i64 = back.query_row("SELECT v FROM t", [], |r| r.get(0)).unwrap();
        assert_eq!(v, 42);
        assert_eq!(
            std::fs::read_to_string(fresh.join("labels.toml")).unwrap(),
            "x = 1\n"
        );
        assert_eq!(
            std::fs::read_to_string(fresh.join("assistant-exchanges.jsonl")).unwrap(),
            "{}\n",
            "Moli's exchanges come back"
        );
        assert_eq!(
            std::fs::read(fresh.join("plan").join("0123456789abcdef.png")).unwrap(),
            b"png",
            "the plan's images come back"
        );
        assert!(!fresh.join("plan").join("notes.txt").exists());
        assert!(
            restore(&fresh, &saved, false).is_err(),
            "no overwrite without --force"
        );
        restore(&fresh, &saved, true).unwrap();
        // A damaged backup restores nothing.
        std::fs::write(saved.join("labels.toml"), "x = 12345\n").unwrap();
        let other = base.join("other");
        assert!(restore(&other, &saved, false).is_err());
        assert!(!other.join("history.db").exists(), "nothing half-restored");
        drop((db, back));
        let _ = std::fs::remove_dir_all(base);
    }
}
