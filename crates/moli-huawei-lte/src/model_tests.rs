use super::*;
use model::Action;
use moli_core::Value;
use std::time::Duration;
fn cfg() -> serde_json::Value {
    serde_json::json!({"host":"modem.example.invalid","port":443,"transport":"https","auth":"sha256-v4"})
}
#[test]
fn configuration_is_closed_explicit_and_has_bounded_finish_based_periods() {
    let c = serde_json::from_value::<Config>(cfg())
        .unwrap()
        .checked()
        .unwrap();
    assert_eq!(c.every, 60);
    assert_eq!(c.info_every, 3600);
    for (key, value) in [
        (
            "host",
            serde_json::json!("https://modem.example.invalid/path"),
        ),
        ("transport", serde_json::json!("http")),
        ("port", serde_json::json!(0)),
        ("every", serde_json::json!(29)),
        ("auth", serde_json::json!("try-all")),
        ("path", serde_json::json!("/api/sms/sms-list")),
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
}
#[test]
fn xml_is_strict_bounded_and_never_resolves_entities() {
    assert!(xml::parse(b"<response><x>1</x></response>").is_ok());
    for raw in [
        "<!DOCTYPE response><response/>",
        "<!DOCTYPE response [<!ENTITY ext SYSTEM 'http://example.invalid/'>]><response>&ext;</response>",
        "<response><x>",
        "<html>login</html>",
        "<response><x>1</x><x>2</x></response>",
    ] {
        assert!(xml::parse(raw.as_bytes()).is_err());
    }
    for raw in [
        " ".repeat(xml::MAX_BODY + 1),
        format!("<response><x>{}</x></response>", "x".repeat(4097)),
        format!("{}{}", "<a>".repeat(17), "</a>".repeat(17)),
        format!("<response>{}</response>", "<x/>".repeat(10000)),
    ] {
        assert!(xml::parse(raw.as_bytes()).is_err());
    }
}
#[test]
fn xml_errors_are_errors_even_inside_http_success() {
    assert_eq!(
        xml::parse(b"<error><code>125003</code></error>").unwrap_err(),
        Failure::Expired
    );
    assert_eq!(
        xml::parse(b"<error><code>108007</code><message>private</message></error>").unwrap_err(),
        Failure::Refused
    );
    assert_eq!(
        xml::parse(b"<error><code>100002</code></error>").unwrap_err(),
        Failure::Unsupported
    );
    assert!(xml::parse(b"<response>OK</response>").unwrap().ok);
}
#[test]
fn native_serial_is_required_without_imei_name_or_address_fallback() {
    let read = model::parse(
        Action::Info,
        b"<response><SerialNumber>synthetic-modem-01</SerialNumber><Imei>ignored</Imei></response>",
    )
    .unwrap();
    assert_eq!(read.identity.as_deref(), Some("synthetic-modem-01"));
    assert!(read.values.is_empty());
    assert!(
        model::parse(
            Action::Info,
            b"<response><DeviceName>name</DeviceName><Imei>ignored</Imei></response>"
        )
        .is_err()
    );
}
#[test]
fn signal_units_and_sentinels_are_explicit_without_false_exactness() {
    let r=model::parse(Action::Signal,b"<response><rssi>-75dBm</rssi><rsrp>-95dBm</rsrp><rsrq>-9.5dB</rsrq><sinr>20 dB</sinr><cell_id>ignored</cell_id></response>").unwrap();
    assert_eq!(r.values.len(), 4);
    assert_eq!(r.values["rssi"], Value::Float(-75.0));
    assert_eq!(r.values["rsrq"], Value::Float(-9.5));
    let missing=model::parse(Action::Signal,b"<response><rssi>&lt;=-113dBm</rssi><rsrp>2147483647</rsrp><rsrq/><sinr>--</sinr></response>").unwrap();
    assert!(missing.values.is_empty());
    let with_units = model::parse(
        Action::Signal,
        b"<response><rssi>-32768dBm</rssi><rsrp>2147483647dBm</rsrp><sinr>255dB</sinr></response>",
    )
    .unwrap();
    assert!(
        with_units.values.is_empty(),
        "sentinels remain unknown when a unit suffix is present"
    );
}
#[test]
fn status_keeps_network_absence_distinct_from_fresh_traffic() {
    let r=model::parse(Action::Status,b"<response><SimStatus>1</SimStatus><ConnectionStatus>901</ConnectionStatus><CurrentNetworkType>7</CurrentNetworkType></response>").unwrap();
    assert_eq!(r.connected, Some(true));
    assert_eq!(r.sim, Some(true));
    assert_eq!(r.values["technology"], Value::Int(7));
    let r = model::parse(
        Action::Status,
        b"<response><SimStatus>0</SimStatus><ConnectionStatus>901</ConnectionStatus></response>",
    )
    .unwrap();
    assert_eq!(r.sim, Some(false));
    assert_eq!(r.connected, Some(false));
}
#[test]
fn raw_counters_survive_precision_limits_and_reset_deltas_safely() {
    let raw=b"<response><CurrentDownload>9007199254740993</CurrentDownload><CurrentUpload>18446744073709551615</CurrentUpload><CurrentConnectTime>100</CurrentConnectTime></response>";
    let read = model::parse(Action::Traffic, raw).unwrap();
    assert_eq!(read.counters, Some((9_007_199_254_740_993, u64::MAX)));
    assert!(!read.values.contains_key("rx_bytes"));
    let mut tracker = tracker::Tracker::default();
    let _ = tracker.apply(&read, Duration::ZERO);
    let next=model::parse(Action::Traffic,b"<response><CurrentDownload>9007199254741113</CurrentDownload><CurrentUpload>18446744073709551615</CurrentUpload><CurrentConnectTime>160</CurrentConnectTime></response>").unwrap();
    let values = tracker.apply(&next, Duration::from_secs(60));
    assert_eq!(values["rx_rate"], Value::Float(2.0));
    assert_eq!(values["tx_rate"], Value::Float(0.0));
    let reset=model::parse(Action::Traffic,b"<response><CurrentDownload>1</CurrentDownload><CurrentUpload>2</CurrentUpload><CurrentConnectTime>1</CurrentConnectTime></response>").unwrap();
    assert!(
        !tracker
            .apply(&reset, Duration::from_secs(60))
            .contains_key("rx_rate")
    );
    assert!(
        model::parse(
            Action::Traffic,
            b"<response><CurrentDownload>18446744073709551616</CurrentDownload></response>"
        )
        .is_err()
    );
    let direct=model::parse(Action::Traffic,b"<response><CurrentDownloadRate>10</CurrentDownloadRate><CurrentUploadRate>20</CurrentUploadRate><CurrentDownload>100</CurrentDownload></response>").unwrap();
    assert_eq!(direct.values["rx_rate"], Value::Int(10));
    assert_eq!(direct.values["tx_rate"], Value::Int(20));
    assert_eq!(direct.values["rx_bytes"], Value::Int(100));
}

#[test]
fn catalogue_fixtures_are_anonymous_executable_contracts() {
    for (action, raw) in [
        (
            Action::Info,
            include_bytes!("../../../integrations/huawei-lte/fixtures/information.xml").as_slice(),
        ),
        (
            Action::Status,
            include_bytes!("../../../integrations/huawei-lte/fixtures/status.xml"),
        ),
        (
            Action::Signal,
            include_bytes!("../../../integrations/huawei-lte/fixtures/signal.xml"),
        ),
        (
            Action::Traffic,
            include_bytes!("../../../integrations/huawei-lte/fixtures/traffic.xml"),
        ),
        (
            Action::Status,
            include_bytes!("../../../integrations/huawei-lte/fixtures/no-sim.xml"),
        ),
    ] {
        assert!(model::parse(action, raw).is_ok());
    }
    assert_eq!(
        xml::parse(include_bytes!(
            "../../../integrations/huawei-lte/fixtures/expired.xml"
        ))
        .unwrap_err(),
        Failure::Expired
    );
    assert_eq!(
        xml::parse(include_bytes!(
            "../../../integrations/huawei-lte/fixtures/locked.xml"
        ))
        .unwrap_err(),
        Failure::Refused
    );
}
