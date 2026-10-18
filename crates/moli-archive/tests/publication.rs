use moli_archive::{
    Archive, PublicationFailure,
    protocol::{Query, Resolution, Source},
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
const INSTALL: &str = "00000000000000000000000000000001";
struct Dir(PathBuf);
impl Dir {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "moli-publish-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn energy(seq: i64, kwh: f64, cost: Option<f64>) -> Vec<u8> {
    format!("{{\"type\":\"energy_hour\",\"seq\":{seq},\"meter\":\"demo.meter\",\"hour\":0,\"kwh\":{kwh},\"cost\":{}}}\n",cost.map_or_else(||"null".into(),|c|c.to_string())).into_bytes()
}
fn query() -> Query {
    Query {
        source: Source::Energy,
        points: vec!["demo.meter".into()],
        from: 0,
        to: 3_600_000,
        resolution: Resolution::Raw,
        max_points: 200,
    }
}

#[test]
fn durable_receipt_is_invisible_until_manifest_and_correction_counts_once() {
    let d = Dir::new();
    let mut a = Archive::open(&d.0, INSTALL).unwrap();
    let old = a.snapshot();
    let one = energy(1, 1.25, None);
    assert_eq!(a.receive(1, 1, &one).unwrap().published_upto, 0);
    assert!(old.query(&query()).unwrap().series[0].values.is_empty());
    assert_eq!(a.publish().unwrap().published_upto, 1);
    let before = a.snapshot();
    let result = before.query(&query()).unwrap();
    assert_eq!(result.published_upto, 1);
    assert_eq!(result.series[0].values.len(), 1);
    a.receive(2, 2, &energy(2, 1.5, Some(0.3))).unwrap();
    a.publish().unwrap();
    let result = a.snapshot().query(&query()).unwrap();
    assert_eq!(result.series[0].values.len(), 1);
    assert_eq!(result.series[0].values[0]["kwh"], 1.5);
    assert_eq!(result.series[0].values[0]["cost"], 0.3);
    assert_eq!(
        before.query(&query()).unwrap().series[0].values[0]["kwh"],
        1.25
    );
    assert!(old.query(&query()).unwrap().series[0].values.is_empty());
    let generation = a.status().generation;
    drop(a);
    let a = Archive::open(&d.0, INSTALL).unwrap();
    assert_eq!(a.status().generation, generation);
    assert_eq!(a.status().published_upto, 2);
}

#[test]
fn every_publication_interruption_recovers_without_partial_visibility() {
    for at in [
        PublicationFailure::PartSynced,
        PublicationFailure::PartRenamed,
        PublicationFailure::ManifestSynced,
        PublicationFailure::ManifestRenamed,
        PublicationFailure::PointerSynced,
        PublicationFailure::PointerRenamed,
        PublicationFailure::ReceiptsUpdated,
    ] {
        let d = Dir::new();
        let mut a = Archive::open(&d.0, INSTALL).unwrap();
        a.receive(1, 1, &energy(1, 1.25, None)).unwrap();
        a.fail_at(at);
        assert!(a.publish().is_err());
        drop(a);
        let mut a = Archive::open(&d.0, INSTALL).unwrap();
        assert_eq!(a.status().durable_upto, 1);
        a.publish().unwrap();
        let result = a.snapshot().query(&query()).unwrap();
        assert_eq!(result.series[0].values.len(), 1);
        assert_eq!(result.series[0].values[0]["kwh"], 1.25);
    }
}

#[test]
fn corrupt_manifest_or_missing_part_fails_closed() {
    for corrupt_manifest in [true, false] {
        let d = Dir::new();
        let mut a = Archive::open(&d.0, INSTALL).unwrap();
        a.receive(1, 1, &energy(1, 1.25, None)).unwrap();
        a.publish().unwrap();
        let snapshot = a.snapshot();
        let path = if corrupt_manifest {
            snapshot.manifest_path()
        } else {
            snapshot.part_paths()[0].clone()
        };
        drop(snapshot);
        drop(a);
        if corrupt_manifest {
            fs::write(path, b"corrupt").unwrap();
        } else {
            fs::remove_file(path).unwrap();
        }
        assert!(Archive::open(&d.0, INSTALL).is_err());
    }
}

#[test]
fn response_quota_rejects_excess_values_without_truncation() {
    let d = Dir::new();
    let mut a = Archive::open(&d.0, INSTALL).unwrap();
    let bytes=b"{\"type\":\"sample\",\"seq\":1,\"point\":\"demo.event\",\"ts\":1000,\"kind\":\"t\",\"num\":null,\"txt\":\"press\"}\n{\"type\":\"sample\",\"seq\":2,\"point\":\"demo.event\",\"ts\":1000,\"kind\":\"t\",\"num\":null,\"txt\":\"press\"}\n";
    a.receive(1, 2, bytes).unwrap();
    a.publish().unwrap();
    let mut q = query();
    q.source = Source::Samples;
    q.points = vec!["demo.event".into()];
    q.max_points = 1;
    assert!(a.snapshot().query(&q).is_err());
    q.max_points = 2;
    assert_eq!(a.snapshot().query(&q).unwrap().series[0].values.len(), 2);
}

#[test]
fn late_catalog_and_rule_change_recompute_without_mutating_received_operations() {
    use moli_archive::protocol::{Catalog, CatalogUpdate, PointRule};
    use moli_rollup::{Aggregation, Origin};
    let d = Dir::new();
    let mut a = Archive::open(&d.0, INSTALL).unwrap();
    let bytes=b"{\"type\":\"sample\",\"seq\":1,\"point\":\"demo.power\",\"ts\":0,\"kind\":\"i\",\"num\":10,\"txt\":null}\n{\"type\":\"sample\",\"seq\":2,\"point\":\"demo.power\",\"ts\":1800000,\"kind\":\"i\",\"num\":20,\"txt\":null}\n";
    a.receive(1, 2, bytes).unwrap();
    a.publish().unwrap();
    let raw = a.snapshot();
    let mut q = query();
    q.source = Source::Samples;
    q.points = vec!["demo.power".into()];
    q.resolution = Resolution::Hour;
    assert!(a.snapshot().query(&q).unwrap().series[0].values.is_empty());
    let mut catalog = Catalog {
        timezone: "UTC".into(),
        points: vec![PointRule {
            id: "demo.power".into(),
            family: "power".into(),
            unit: Some("W".into()),
            aggregation: Some(Aggregation::TimeWeighted),
            freshness_ms: None,
            origin: Origin::Recorded,
        }],
        meters: vec![],
    };
    a.set_catalog(CatalogUpdate {
        expected_revision: a.status().revision,
        catalog: catalog.clone(),
    })
    .unwrap();
    let result = a.snapshot().query(&q).unwrap();
    assert_eq!(
        result.series[0].values[0]["stat"]["aggregate"]["mean"],
        15.0
    );
    for (resolution, to) in [
        (Resolution::Day, 86_400_000),
        (Resolution::Month, 31 * 86_400_000),
    ] {
        q.resolution = resolution;
        q.to = to;
        let result = a.snapshot().query(&q).unwrap();
        assert_eq!(
            result.series[0].values[0]["stat"]["aggregate"]["mean"],
            15.0
        );
        assert_eq!(result.series[0].values[0]["stat"]["covered_ms"], 3_600_000);
    }
    catalog.points[0].aggregation = Some(Aggregation::Count);
    a.set_catalog(CatalogUpdate {
        expected_revision: a.status().revision,
        catalog,
    })
    .unwrap();
    q.resolution = Resolution::Hour;
    q.to = 3_600_000;
    let result = a.snapshot().query(&q).unwrap();
    assert_eq!(result.series[0].values.len(), 1);
    assert_eq!(result.series[0].values[0]["stat"]["aggregate"]["events"], 2);
    q.resolution = Resolution::Raw;
    assert_eq!(
        a.snapshot().query(&q).unwrap().series[0].values,
        raw.query(&q).unwrap().series[0].values
    );
}

#[test]
fn compaction_and_collection_preserve_a_pinned_snapshot() {
    let d = Dir::new();
    let mut a = Archive::open(&d.0, INSTALL).unwrap();
    a.receive(1, 1, &energy(1, 1.0, None)).unwrap();
    a.publish().unwrap();
    let old = a.snapshot();
    let old_files = old.part_paths();
    for seq in 2..=9 {
        a.receive(seq, seq, &energy(seq, 2.0, Some(0.4))).unwrap();
        a.publish().unwrap();
    }
    a.compact().unwrap();
    a.collect().unwrap();
    assert!(old_files.iter().all(|p| p.exists()));
    assert_eq!(old.query(&query()).unwrap().series[0].values[0]["kwh"], 1.0);
    assert_eq!(
        a.snapshot().query(&query()).unwrap().series[0].values.len(),
        1
    );
    drop(old);
    a.collect().unwrap();
}

#[test]
fn restoring_rotates_epoch_once_even_if_rotation_is_interrupted() {
    for failure in [
        PublicationFailure::EpochIntentSynced,
        PublicationFailure::EpochManifestPublished,
        PublicationFailure::EpochIdentityPublished,
    ] {
        let d = Dir::new();
        let mut a = Archive::open(&d.0, INSTALL).unwrap();
        a.receive(1, 1, &energy(1, 1.0, None)).unwrap();
        a.publish().unwrap();
        let old = a.status().generation;
        a.fail_at(failure);
        assert!(a.rotate_epoch().is_err());
        drop(a);
        let a = Archive::open(&d.0, INSTALL).unwrap();
        assert_ne!(a.status().generation, old);
        let fresh = a.status().generation;
        assert_eq!(a.status().published_upto, 1);
        drop(a);
        let a = Archive::open(&d.0, INSTALL).unwrap();
        assert_eq!(a.status().generation, fresh);
    }
}

#[test]
fn response_byte_limit_is_applied_before_collecting_all_values() {
    let d = Dir::new();
    let mut a = Archive::open(&d.0, INSTALL).unwrap();
    let mut bytes = Vec::new();
    for seq in 1..=300 {
        let line = serde_json::json!({"type":"sample","seq":seq,"point":"demo.text","ts":seq,"kind":"t","num":null,"txt":"x".repeat(4096)});
        serde_json::to_writer(&mut bytes, &line).unwrap();
        bytes.push(b'\n');
    }
    a.receive(1, 300, &bytes).unwrap();
    a.publish().unwrap();
    let mut q = query();
    q.source = Source::Samples;
    q.points = vec!["demo.text".into()];
    q.max_points = 5000;
    assert!(a.snapshot().query(&q).is_err());
}

#[test]
fn a_stable_state_is_carried_across_months_without_an_invented_gap() {
    use moli_archive::protocol::{Catalog, CatalogUpdate, PointRule};
    use moli_rollup::{Aggregation, Origin};
    let d = Dir::new();
    let mut a = Archive::open(&d.0, INSTALL).unwrap();
    a.set_catalog(CatalogUpdate {
        expected_revision: 0,
        catalog: Catalog {
            timezone: "UTC".into(),
            points: vec![PointRule {
                id: "demo.power".into(),
                family: "power".into(),
                unit: Some("W".into()),
                aggregation: Some(Aggregation::TimeWeighted),
                freshness_ms: None,
                origin: Origin::Recorded,
            }],
            meters: vec![],
        },
    })
    .unwrap();
    for (seq, ts, num) in [(1, 1_767_225_600_000i64, 10), (2, 1_772_323_200_000i64, 20)] {
        let mut bytes=serde_json::to_vec(&serde_json::json!({"type":"sample","seq":seq,"point":"demo.power","ts":ts,"kind":"i","num":num,"txt":null})).unwrap();
        bytes.push(b'\n');
        a.receive(seq, seq, &bytes).unwrap();
        a.publish().unwrap();
    }
    let q = Query {
        source: Source::Samples,
        points: vec!["demo.power".into()],
        from: 1_770_000_000_000 / 3_600_000 * 3_600_000,
        to: 1_770_000_000_000 / 3_600_000 * 3_600_000 + 3_600_000,
        resolution: Resolution::Hour,
        max_points: 200,
    };
    let response = a.snapshot().query(&q).unwrap();
    assert_eq!(response.series[0].values.len(), 1);
    assert_eq!(
        response.series[0].values[0]["stat"]["aggregate"]["mean"],
        10.0
    );
    assert_eq!(
        response.series[0].values[0]["stat"]["covered_ms"],
        3_600_000
    );
}

#[test]
fn large_valid_text_batch_publishes_with_bounded_parquet_metadata() {
    let d = Dir::new();
    let mut a = Archive::open(&d.0, INSTALL).unwrap();
    let mut bytes = Vec::new();
    for seq in 1..=1800 {
        serde_json::to_writer(&mut bytes,&serde_json::json!({"type":"sample","seq":seq,"point":"demo.text","ts":seq,"kind":"t","num":null,"txt":"x".repeat(4096)})).unwrap();
        bytes.push(b'\n');
    }
    assert!(bytes.len() < 8 * 1024 * 1024);
    a.receive(1, 1800, &bytes).unwrap();
    a.publish().unwrap();
    let q = Query {
        source: Source::Samples,
        points: vec!["demo.text".into()],
        from: 1,
        to: 2,
        resolution: Resolution::Raw,
        max_points: 1,
    };
    assert_eq!(a.snapshot().query(&q).unwrap().series[0].values.len(), 1);
}

#[test]
fn a_partial_calendar_bucket_never_silently_includes_out_of_range_energy() {
    let d = Dir::new();
    let mut a = Archive::open(&d.0, INSTALL).unwrap();
    let mut bytes = energy(1, 1.0, None);
    let mut second: serde_json::Value = serde_json::from_slice(&energy(2, 2.0, None)).unwrap();
    second["hour"] = (20 * 86_400_000i64).into();
    serde_json::to_writer(&mut bytes, &second).unwrap();
    bytes.push(b'\n');
    a.receive(1, 2, &bytes).unwrap();
    a.publish().unwrap();
    let mut q = query();
    q.resolution = Resolution::Month;
    q.to = 15 * 86_400_000;
    assert!(a.snapshot().query(&q).is_err());
}

#[test]
fn a_reader_keeps_its_manifest_during_concurrent_publication_and_collection() {
    use std::sync::{
        Arc, Barrier,
        atomic::{AtomicBool, Ordering},
    };
    let d = Dir::new();
    let mut a = Archive::open(&d.0, INSTALL).unwrap();
    a.receive(1, 1, &energy(1, 1.0, None)).unwrap();
    a.publish().unwrap();
    let snapshot = a.snapshot();
    let active = Arc::new(AtomicBool::new(true));
    let started = Arc::new(Barrier::new(2));
    let reader_active = active.clone();
    let reader_started = started.clone();
    let reader = std::thread::spawn(move || {
        let mut q = query();
        q.resolution = Resolution::Month;
        q.to = 31 * 86_400_000;
        reader_started.wait();
        let mut count = 0;
        while reader_active.load(Ordering::Acquire) {
            let result = snapshot.query(&q).unwrap();
            assert_eq!(
                result.series[0].values[0]["stat"]["aggregate"]["total"],
                1.0
            );
            count += 1;
        }
        count
    });
    started.wait();
    for seq in 2..=9 {
        a.receive(seq, seq, &energy(seq, 2.0, Some(0.4))).unwrap();
        a.publish().unwrap();
    }
    active.store(false, Ordering::Release);
    assert!(reader.join().unwrap() > 0);
}

#[test]
fn a_persistent_backup_pin_survives_restart_until_released() {
    let d = Dir::new();
    let mut a = Archive::open(&d.0, INSTALL).unwrap();
    a.receive(1, 1, &energy(1, 1.0, None)).unwrap();
    a.publish().unwrap();
    let pinned = a.pin_current("fixture-backup").unwrap();
    let files = pinned.part_paths();
    drop(pinned);
    for seq in 2..=6 {
        a.receive(seq, seq, &energy(seq, 2.0, None)).unwrap();
        a.publish().unwrap();
    }
    a.compact().unwrap();
    drop(a);
    let mut a = Archive::open(&d.0, INSTALL).unwrap();
    a.collect().unwrap();
    assert!(files.iter().all(|p| p.exists()));
    a.release_pin("fixture-backup").unwrap();
    a.collect().unwrap();
    assert!(files.iter().any(|p| !p.exists()));
}

#[test]
fn lost_metadata_never_creates_a_new_identity_or_a_zero_ack() {
    let d = Dir::new();
    let mut a = Archive::open(&d.0, INSTALL).unwrap();
    a.receive(1, 1, &energy(1, 1.0, None)).unwrap();
    a.publish().unwrap();
    drop(a);
    let identity = d.0.join("identity.json");
    let original = fs::read(&identity).unwrap();
    fs::remove_file(&identity).unwrap();
    assert!(Archive::open(&d.0, INSTALL).is_err());
    assert!(!identity.exists());
    fs::write(&identity, original).unwrap();
    let pointer = d.0.join("current.json");
    fs::remove_file(&pointer).unwrap();
    assert!(Archive::open(&d.0, INSTALL).is_err());
    assert!(!pointer.exists());
}
