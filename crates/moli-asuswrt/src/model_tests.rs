use super::*;
use crate::tracker::Tracker;
use model::{Action, Samples};
use moli_core::Value;
use std::time::Duration;
fn cfg() -> serde_json::Value {
    serde_json::json!({"host":"router.example","port":443,"transport":"https"})
}
#[test]
fn endpoints_plain_auth_and_client_selection_are_explicit_and_bounded() {
    let c = serde_json::from_value::<Config>(cfg())
        .unwrap()
        .checked()
        .unwrap();
    assert!(c.selected.is_empty());
    for (key, value) in [
        ("host", serde_json::json!("http://router.example/x")),
        ("transport", serde_json::json!("http")),
        ("every", serde_json::json!(29)),
        ("port", serde_json::json!(0)),
        ("hook", serde_json::json!("apply()")),
    ] {
        let mut v = cfg();
        v[key] = value;
        assert!(
            serde_json::from_value::<Config>(v)
                .ok()
                .and_then(|c| c.checked().ok())
                .is_none()
        );
    }
    let mut v = cfg();
    v["transport"] = serde_json::json!("http");
    v["allow_plain_auth"] = serde_json::json!(true);
    assert!(
        serde_json::from_value::<Config>(v)
            .unwrap()
            .checked()
            .is_ok()
    );
    let mut v = cfg();
    v["selected"] = serde_json::json!(["aa:bb:cc:dd:ee:01", "AABBCCDDEE01"]);
    assert!(
        serde_json::from_value::<Config>(v)
            .unwrap()
            .checked()
            .is_err()
    );
    let mut v = cfg();
    v["selected"] = serde_json::json!(
        (0..33)
            .map(|i| format!("aabbccddee{i:02x}"))
            .collect::<Vec<_>>()
    );
    assert!(
        serde_json::from_value::<Config>(v)
            .unwrap()
            .checked()
            .is_err()
    );
}
#[test]
fn json_is_strict_and_bounded_including_duplicate_keys() {
    assert!(json::parse(br#"{"value":12}"#).is_ok());
    for invalid in [
        br#"{"value":1,"value":2}"#.as_slice(),
        b"var x={}",
        b"<html>login</html>",
        b"{} trailing",
        b"[1,2]",
        br#"{"x":NaN}"#,
    ] {
        assert!(json::parse(invalid).is_err());
    }
    assert!(json::parse(&vec![b' '; 1024 * 1024 + 1]).is_err());
    assert!(json::parse(format!("{{\"x\":\"{}\"}}", "x".repeat(4097)).as_bytes()).is_err());
    assert!(
        json::parse(format!("{}0{}", "{\"x\":".repeat(17), "}".repeat(17)).as_bytes()).is_err()
    );
}
#[test]
fn native_identity_is_required_and_never_derived_from_name_or_address() {
    let r = model::parse(
        Action::Info,
        br#"{"label_mac":"aa:bb:cc:dd:ee:01","productid":"model-test","firmver":"test"}"#,
    )
    .unwrap();
    assert_eq!(r.identity.as_deref(), Some("aabbccddee01"));
    assert_eq!(r.values["firmware"], Value::from("test"));
    for body in [
        br#"{"productid":"name","ip":"192.168.1.1"}"#.as_slice(),
        br#"{"label_mac":"not-a-mac"}"#,
        br#"{"label_mac":"00:00:00:00:00:00"}"#,
    ] {
        assert!(model::parse(Action::Info, body).is_err());
    }
}
const TELEMETRY:&[u8]=br#"{"cpu_usage":{"cpu1_total":"100","cpu1_usage":"20"},"memory_usage":{"mem_total":"1024","mem_used":"256","mem_free":"768"},"netdev":{"INTERNET_rx":"0x1000","INTERNET_tx":"0x2000"}}"#;
#[test]
fn counters_have_documented_units_and_deltas_never_span_resets() {
    let a = model::parse(Action::Telemetry, TELEMETRY).unwrap();
    let mut t = tracker::Tracker::default();
    let first = t.apply(&a, Duration::ZERO);
    assert_eq!(first["memory_used"], Value::Int(262_144));
    assert_eq!(first["memory"], Value::Float(25.0));
    assert_eq!(first["rx_bytes"], Value::Int(4096));
    assert_eq!(first["rx_rate"], Value::Null);
    assert_eq!(first["cpu"], Value::Null);
    let b=model::parse(Action::Telemetry,br#"{"cpu_usage":{"cpu1_total":"200","cpu1_usage":"45"},"netdev":{"INTERNET_rx":"0x1200","INTERNET_tx":"0x2400"}}"#).unwrap();
    let next = t.apply(&b, Duration::from_secs(2));
    assert_eq!(next["cpu"], Value::Float(25.0));
    assert_eq!(next["rx_rate"], Value::Float(256.0));
    let reset = t.apply(&a, Duration::from_secs(1));
    assert_eq!(reset["rx_rate"], Value::Null);
    assert_eq!(reset["cpu"], Value::Null);
    assert!(
        model::parse(
            Action::Telemetry,
            br#"{"netdev":{"INTERNET_rx":"0x10","INTERNET_rx":"0x20","INTERNET_tx":"0x30"}}"#
        )
        .is_err()
    );
    assert!(
        model::parse(
            Action::Telemetry,
            br#"{"memory_usage":{"mem_total":"1","mem_used":"2"}}"#
        )
        .is_err()
    );
}
#[test]
fn live_inventory_is_not_wifi_association_or_presence() {
    let r=model::parse(Action::Clients,br#"{"get_clientlist":{"aa:bb:cc:dd:ee:01":{"name":"synthetic","ip":"192.168.1.10","isOnline":"1","isWL":"2","rssi":"-51","curRx":"100","curTx":"50"},"maclist":["aa:bb:cc:dd:ee:01"]}}"#).unwrap();
    let list = r.clients.unwrap();
    let s = &list["aabbccddee01"];
    assert_eq!(s["link"], Value::from("wireless"));
    assert_eq!(s["signal"], Value::Int(-51));
    assert_eq!(list.len(), 1);
    for key in [
        "name",
        "ip",
        "mac",
        "ssid",
        "associated",
        "presence",
        "occupancy",
    ] {
        assert!(!s.contains_key(key));
    }
    assert_eq!(
        model::parse(Action::Clients, br#"{"get_clientlist":{}}"#)
            .unwrap()
            .clients
            .unwrap()
            .len(),
        0
    );
    assert!(model::parse(Action::Clients, b"{}").is_err());
}
#[test]
fn invalid_inventory_is_not_a_partial_empty_success() {
    for body in [
        br#"{"get_clientlist":{"bad":{}}}"#.as_slice(),
        br#"{"get_clientlist":{"aa:bb:cc:dd:ee:01":{},"aabbccddee01":{}}}"#,
        br#"{"get_clientlist":{"maclist":["aa:bb:cc:dd:ee:01"]}}"#,
    ] {
        assert!(model::parse(Action::Clients, body).is_err());
    }
    let entries = (0..513)
        .map(|i| {
            (
                format!("aa:bb:cc:dd:{:02x}:{:02x}", i / 256, i % 256),
                serde_json::json!({}),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    let body = serde_json::json!({"get_clientlist":entries}).to_string();
    assert!(model::parse(Action::Clients, body.as_bytes()).is_err());
    let _ = Samples::new();
}

#[test]
fn catalogue_fixtures_are_executable_contracts() {
    let info = model::parse(
        Action::Info,
        include_bytes!("../../../integrations/asuswrt/fixtures/info.json"),
    )
    .unwrap();
    assert_eq!(info.identity.as_deref(), Some("aabbccddee01"));
    let first = model::parse(
        Action::Telemetry,
        include_bytes!("../../../integrations/asuswrt/fixtures/telemetry.json"),
    )
    .unwrap();
    let next = model::parse(
        Action::Telemetry,
        include_bytes!("../../../integrations/asuswrt/fixtures/telemetry-next.json"),
    )
    .unwrap();
    let mut tracker = Tracker::default();
    let _ = tracker.apply(&first, Duration::ZERO);
    let values = tracker.apply(&next, Duration::from_secs(60));
    assert_eq!(values["cpu"], Value::Float(30.0));
    assert_eq!(values["rx_rate"], Value::Float(60.0));
    assert_eq!(values["tx_rate"], Value::Float(120.0));
    assert_eq!(
        model::parse(
            Action::Clients,
            include_bytes!("../../../integrations/asuswrt/fixtures/clients.json")
        )
        .unwrap()
        .clients
        .unwrap()
        .len(),
        2
    );
    assert!(
        model::parse(
            Action::Clients,
            include_bytes!("../../../integrations/asuswrt/fixtures/clients-empty.json")
        )
        .unwrap()
        .clients
        .unwrap()
        .is_empty()
    );
    for body in [
        include_bytes!("../../../integrations/asuswrt/fixtures/duplicate-wan.json").as_slice(),
        include_bytes!("../../../integrations/asuswrt/fixtures/login-html.txt").as_slice(),
    ] {
        assert!(json::parse(body).is_err());
    }
}
