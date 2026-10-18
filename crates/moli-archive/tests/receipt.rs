use flate2::{Compression, write::GzEncoder};
use moli_archive::{
    protocol::decode,
    receipt::{Failure, ReceiptLog},
};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Dir(PathBuf);
impl Dir {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "moli-archive-{}-{}",
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
fn body(seq: i64, value: i64) -> Vec<u8> {
    format!("{{\"type\":\"sample\",\"seq\":{seq},\"point\":\"demo.power\",\"ts\":1000,\"kind\":\"i\",\"num\":{value},\"txt\":null}}\n").into_bytes()
}
fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut gz = GzEncoder::new(Vec::new(), Compression::fast());
    gz.write_all(bytes).unwrap();
    gz.finish().unwrap()
}

#[test]
fn duplicate_conflict_gap_and_reopen() {
    let d = Dir::new();
    let bytes = body(1, 42);
    let mut log = ReceiptLog::open(&d.0, 7).unwrap();
    let original = log.receive(1, 1, &bytes).unwrap();
    assert_eq!(original.durable_upto, 1);
    assert_eq!(original.published_upto, 0);
    assert_eq!(log.receive(1, 1, &bytes).unwrap(), original);
    assert!(log.receive(1, 1, &body(1, 43)).is_err());
    assert!(log.receive(3, 3, &body(3, 44)).is_err());
    drop(log);
    let mut log = ReceiptLog::open(&d.0, 7).unwrap();
    assert_eq!(log.durable_upto(), 1);
    assert_eq!(log.receive(1, 1, &bytes).unwrap(), original);
    assert_eq!(log.receive(2, 2, &body(2, 43)).unwrap().durable_upto, 2);
}

#[test]
fn interrupted_receipt_never_acknowledges_a_missing_payload() {
    for failure in [
        Failure::PayloadSynced,
        Failure::PayloadRenamed,
        Failure::IndexWritten,
        Failure::IndexSynced,
    ] {
        let d = Dir::new();
        let bytes = body(1, 42);
        let mut log = ReceiptLog::open(&d.0, 7).unwrap();
        log.fail_at(failure);
        assert!(log.receive(1, 1, &bytes).is_err());
        drop(log);
        let mut log = ReceiptLog::open(&d.0, 7).unwrap();
        assert!(log.durable_upto() <= 1);
        let ack = log.receive(1, 1, &bytes).unwrap();
        assert_eq!(ack.durable_upto, 1);
        assert_eq!(log.pending(0).unwrap().unwrap().bytes, bytes);
    }
}

#[test]
fn incomplete_index_tail_is_recovered_but_corruption_is_rejected() {
    let d = Dir::new();
    let bytes = body(1, 42);
    let mut log = ReceiptLog::open(&d.0, 7).unwrap();
    log.receive(1, 1, &bytes).unwrap();
    drop(log);
    fs::OpenOptions::new()
        .append(true)
        .open(d.0.join("receipts.bin"))
        .unwrap()
        .write_all(&[1, 2, 3])
        .unwrap();
    let log = ReceiptLog::open(&d.0, 7).unwrap();
    assert_eq!(log.durable_upto(), 1);
    drop(log);
    fs::write(d.0.join("journal/1-1.ndjson"), body(1, 99)).unwrap();
    assert!(ReceiptLog::open(&d.0, 7).is_err());
}

#[test]
fn protocol_bounds_and_scalar_types() {
    let bytes = body(1, 42);
    assert_eq!(decode(1, 1, &gzip(&bytes)).unwrap().bytes, bytes);
    assert!(decode(2, 2, &gzip(&bytes)).is_err());
    assert!(decode(1, 1, b"not gzip").is_err());
    let invalid = String::from_utf8(body(1, 42))
        .unwrap()
        .replace("\"kind\":\"i\"", "\"kind\":\"b\"");
    assert!(decode(1, 1, &gzip(invalid.as_bytes())).is_err());
    assert!(decode(1, 5001, &gzip(&bytes)).is_err());
    assert!(decode(1, 1, &gzip(&vec![b' '; 8 * 1024 * 1024 + 1])).is_err());
}

#[test]
fn publication_removes_only_covered_payloads_and_retains_idempotence() {
    let d = Dir::new();
    let one = body(1, 42);
    let two = body(2, 43);
    let mut log = ReceiptLog::open(&d.0, 7).unwrap();
    let original = log.receive(1, 1, &one).unwrap();
    log.receive(2, 2, &two).unwrap();
    log.published(1, 1).unwrap();
    assert!(!d.0.join("journal/1-1.ndjson").exists());
    assert!(d.0.join("journal/2-2.ndjson").exists());
    drop(log);
    let mut log = ReceiptLog::open_at(&d.0, 7, 1, 1).unwrap();
    assert_eq!(log.receive(1, 1, &one).unwrap(), log.status());
    assert_ne!(log.status(), original);
    assert_eq!(log.pending(1).unwrap().unwrap().bytes, two);
    assert_eq!(log.status().published_upto, 1);
}

#[test]
fn exclusive_writer_checksum_and_missing_pending_payload_are_enforced() {
    let d = Dir::new();
    let mut log = ReceiptLog::open(&d.0, 7).unwrap();
    assert!(ReceiptLog::open(&d.0, 7).is_err());
    log.receive(1, 1, &body(1, 42)).unwrap();
    drop(log);
    let mut index = fs::read(d.0.join("receipts.bin")).unwrap();
    index[8] ^= 1;
    fs::write(d.0.join("receipts.bin"), &index).unwrap();
    assert!(ReceiptLog::open(&d.0, 7).is_err());
    index[8] ^= 1;
    fs::write(d.0.join("receipts.bin"), &index).unwrap();
    fs::remove_file(d.0.join("journal/1-1.ndjson")).unwrap();
    assert!(ReceiptLog::open(&d.0, 7).is_err());
}

#[test]
fn interrupted_writer_is_poisoned_until_reopen() {
    let d = Dir::new();
    let mut log = ReceiptLog::open(&d.0, 7).unwrap();
    log.fail_at(Failure::IndexWritten);
    assert!(log.receive(1, 1, &body(1, 42)).is_err());
    assert!(log.receive(1, 1, &body(1, 42)).is_err());
    assert!(log.receive(2, 2, &body(2, 42)).is_err());
}

#[test]
fn trailing_gzip_and_fractional_integers_are_rejected() {
    let mut compressed = gzip(&body(1, 42));
    compressed.extend_from_slice(b"trailing");
    assert!(decode(1, 1, &compressed).is_err());
    for replacement in ["42.5", "1e309"] {
        let invalid = String::from_utf8(body(1, 42))
            .unwrap()
            .replace("\"num\":42", &format!("\"num\":{replacement}"));
        assert!(decode(1, 1, &gzip(invalid.as_bytes())).is_err());
    }
}

#[test]
fn orphan_payload_without_receipt_is_discarded() {
    let d = Dir::new();
    let log = ReceiptLog::open(&d.0, 7).unwrap();
    drop(log);
    fs::write(d.0.join("journal/1-1.ndjson"), body(1, 99)).unwrap();
    let mut log = ReceiptLog::open(&d.0, 7).unwrap();
    assert_eq!(log.durable_upto(), 0);
    assert!(!d.0.join("journal/1-1.ndjson").exists());
    log.receive(1, 1, &body(1, 42)).unwrap();
}

#[test]
fn seven_day_receipt_window_preserves_contiguous_floor() {
    let d = Dir::new();
    let mut log = ReceiptLog::open(&d.0, 7).unwrap();
    log.receive(1, 1, &body(1, 42)).unwrap();
    log.published_at(1, 1, 1_000_000).unwrap();
    log.receive(2, 2, &body(2, 43)).unwrap();
    log.published_at(2, 2, 1_000_000 + 7 * 24 * 3600).unwrap();
    assert!(log.receive(1, 1, &body(1, 42)).is_err());
    assert_eq!(log.receive(2, 2, &body(2, 43)).unwrap(), log.status());
    assert_eq!(fs::metadata(d.0.join("receipts.bin")).unwrap().len(), 128);
    drop(log);
    let mut log = ReceiptLog::open_at(&d.0, 7, 2, 2).unwrap();
    assert_eq!(log.receive(3, 3, &body(3, 44)).unwrap().durable_upto, 3);
}

#[test]
fn quota_never_evicts_an_unpublished_receipt() {
    let d = Dir::new();
    let mut log = ReceiptLog::open(&d.0, 7).unwrap();
    log.restrict_receipts(1).unwrap();
    log.receive(1, 1, &body(1, 42)).unwrap();
    assert!(log.receive(2, 2, &body(2, 43)).is_err());
    assert_eq!(log.status().durable_upto, 1);
    assert!(d.0.join("journal/1-1.ndjson").exists());
    log.published_at(1, 1, 1_000_000).unwrap();
    assert_eq!(log.receive(2, 2, &body(2, 43)).unwrap().durable_upto, 2);
    assert!(log.receive(1, 1, &body(1, 42)).is_err());
}

#[test]
fn batch_status_keeps_exactly_the_three_d13_wire_fields() {
    let status = moli_archive::protocol::Status {
        durable_upto: 2,
        published_upto: 1,
        generation: 7,
        revision: 3,
    };
    assert_eq!(
        serde_json::to_value(status).unwrap(),
        serde_json::json!({"durable_upto":2,"published_upto":1,"generation":7})
    );
}

#[test]
fn catalog_accepts_a_house_with_4096_points() {
    use moli_archive::protocol::{Catalog, PointRule};
    let points = (0..4096)
        .map(|i| PointRule {
            id: format!("fixture.point.{i:04}"),
            family: "power".into(),
            unit: Some("W".into()),
            aggregation: Some(moli_rollup::Aggregation::TimeWeighted),
            freshness_ms: Some(300_000),
            origin: moli_rollup::Origin::Recorded,
        })
        .collect();
    Catalog {
        timezone: "UTC".into(),
        points,
        meters: vec![],
    }
    .validate()
    .unwrap();
}

#[test]
fn scalar_limits_match_the_core_export_contract() {
    let text = |value: String| {
        let mut bytes = serde_json::to_vec(&serde_json::json!({
            "type":"sample", "seq":1, "point":"demo.text", "ts":0,
            "kind":"t", "num":null, "txt":value
        }))
        .unwrap();
        bytes.push(b'\n');
        bytes
    };
    assert!(decode(1, 1, &gzip(&text("🦀".repeat(moli_core::MAX_TEXT)))).is_ok());
    assert!(decode(1, 1, &gzip(&text("a".repeat(moli_core::MAX_TEXT + 1)))).is_err());
    let large_integer = body(1, 42);
    let large_integer = String::from_utf8(large_integer)
        .unwrap()
        .replace("\"num\":42", "\"num\":1.152921504606847e18");
    assert!(decode(1, 1, &gzip(large_integer.as_bytes())).is_ok());
    let fractional = large_integer.replace("1.152921504606847e18", "1.5");
    assert!(decode(1, 1, &gzip(fractional.as_bytes())).is_err());
}
