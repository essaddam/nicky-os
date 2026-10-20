use super::*;
use std::fmt::Write as _;
fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/../../integrations/netgear/fixtures/{name}.xml",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
#[test]
fn stable_identity_and_known_values_without_inventory_metadata() {
    let read = parse(Action::Info, &fixture("info")).unwrap();
    assert_eq!(read.identity.as_deref(), Some("synthetic-router-01"));
    assert_eq!(read.values["firmware"], Value::from("test-1"));
    let sys = parse(Action::System, &fixture("system")).unwrap();
    assert_eq!(sys.values["cpu"], Value::Int(12));
    assert_eq!(
        parse(Action::Ethernet, &fixture("ethernet"))
            .unwrap()
            .values["ethernet"],
        Value::Bool(true)
    );
    assert!(parse(Action::Info, &fixture("info")).is_ok());
    assert!(parse(Action::Info, b"<html>private response</html>").is_err());
}
#[test]
fn inventory_variants_do_not_infer_association_or_units() {
    for (action, name) in [
        (Action::ClientsV1, "clients-v1"),
        (Action::ClientsV2, "clients-v2"),
    ] {
        let clients = parse(action, &fixture(name)).unwrap().clients.unwrap();
        assert_eq!(clients.len(), 2);
        assert_eq!(clients["aabbccddee01"]["signal"], Value::Int(75));
        assert_eq!(clients["aabbccddee01"]["link"], Value::from("5GHz"));
        assert!(!clients["aabbccddee01"].contains_key("associated"));
        assert!(!clients["aabbccddee01"].contains_key("rate"));
        for sample in clients.values() {
            for key in sample.keys() {
                assert!(!["ip", "name", "mac", "ssid", "presence", "occupancy"].contains(key));
            }
        }
    }
}
#[test]
fn malformed_or_duplicate_inventory_is_never_an_empty_success() {
    let data = String::from_utf8(fixture("clients-v1")).unwrap();
    for bad in [
        data.replace("2@", "3@"),
        data.replace("ee:02", "ee:01"),
        data.replace(";wired;", "@wired;"),
        data.replace("ee:01", "xx:01"),
        data.replace("</NewAttachDevice>", ""),
    ] {
        assert!(parse(Action::ClientsV1, bad.as_bytes()).is_err());
    }
    let data = String::from_utf8(fixture("clients-v2")).unwrap();
    assert!(parse(Action::ClientsV2, data.replace("ee:02", "ee:01").as_bytes()).is_err());
    assert!(
        parse(
            Action::ClientsV2,
            data.replace("<MAC>", "<Missing>")
                .replace("</MAC>", "</Missing>")
                .as_bytes()
        )
        .is_err()
    );
    let mut nodes = String::new();
    for i in 0..513 {
        write!(
            &mut nodes,
            "<Device><MAC>aa:bb:cc:dd:{:02x}:{:02x}</MAC></Device>",
            i / 256,
            i % 256
        )
        .unwrap();
    }
    assert!(
        parse(
            Action::ClientsV2,
            protocol::response(
                Action::ClientsV2,
                &format!("<NewAttachDevice>{nodes}</NewAttachDevice>"),
                0
            )
            .as_bytes()
        )
        .is_err()
    );
}
#[test]
fn disabled_or_unqualified_traffic_is_not_zero() {
    assert!(
        parse(Action::Traffic, &fixture("traffic-disabled"))
            .unwrap()
            .values
            .is_empty()
    );
    let values = parse(Action::Traffic, &fixture("traffic")).unwrap().values;
    assert_eq!(values["today_connection"], Value::Int(5400));
    assert!(!values.contains_key("today_upload"));
    assert!(!values.contains_key("today_download"));
}
#[test]
fn required_identity_and_duplicate_fields_fail_closed() {
    for fields in [
        "<SerialNumber/>",
        "<SerialNumber>same</SerialNumber><SerialNumber>other</SerialNumber>",
        "<SerialNumber>ip/name</SerialNumber>",
    ] {
        assert!(
            parse(
                Action::Info,
                protocol::response(Action::Info, fields, 0).as_bytes()
            )
            .is_err()
        );
    }
}
#[test]
fn explicit_link_rate_is_finite_and_optional_fields_do_not_become_zero() {
    let fields = "<NewAttachDevice><Device><MAC>aa:bb:cc:dd:ee:01</MAC><Linkspeed>1000 Mbit/s</Linkspeed></Device></NewAttachDevice>";
    let clients = parse(
        Action::ClientsV2,
        protocol::response(Action::ClientsV2, fields, 0).as_bytes(),
    )
    .unwrap()
    .clients
    .unwrap();
    assert_eq!(clients["aabbccddee01"]["rate"], Value::Float(1000.0));
    assert!(!clients["aabbccddee01"].contains_key("signal"));
    assert!(
        parse(
            Action::ClientsV2,
            protocol::response(
                Action::ClientsV2,
                &fields.replace("1000 Mbit/s", "NaN Mbit/s"),
                0
            )
            .as_bytes()
        )
        .is_err()
    );
    assert!(
        parse(
            Action::System,
            protocol::response(
                Action::System,
                "<NewCPUUtilization>NaN</NewCPUUtilization>",
                0
            )
            .as_bytes()
        )
        .is_err()
    );
}
