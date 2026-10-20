use super::*;
fn config() -> serde_json::Value {
    serde_json::json!({"host":"127.0.0.1","port":443,"transport":"https","login":"v2","inventory":"v2"})
}
fn valid(value: serde_json::Value) -> bool {
    serde_json::from_value::<Config>(value).is_ok_and(|c| Netgear::new(c).is_ok())
}
#[test]
fn endpoint_variants_and_plaintext_require_human_choices() {
    assert!(valid(config()));
    for (key, value) in [
        ("host", serde_json::json!("http://192.168.1.1/path")),
        ("host", serde_json::json!("user@example.com")),
        ("port", serde_json::json!(0)),
        ("every", serde_json::json!(29)),
        ("transport", serde_json::json!("http")),
        ("hook", serde_json::json!("GetInfo")),
    ] {
        let mut c = config();
        c[key] = value;
        assert!(!valid(c));
    }
    for key in ["host", "port", "transport", "login", "inventory"] {
        let mut c = config();
        c.as_object_mut().unwrap().remove(key);
        assert!(!valid(c));
    }
    let mut c = config();
    c["transport"] = serde_json::json!("http");
    c["allow_plain_auth"] = serde_json::json!(true);
    assert!(valid(c));
}
#[test]
fn empty_selection_is_default_and_duplicates_and_excess_are_rejected() {
    let c: Config = serde_json::from_value(config()).unwrap();
    assert!(c.selected.is_empty());
    let mut c = config();
    c["selected"] = serde_json::json!(["aa:bb:cc:dd:ee:ff", "AABBCCDDEEFF"]);
    assert!(!valid(c));
    let mut c = config();
    c["selected"] = serde_json::json!(
        (0..33)
            .map(|i| format!("aabbccddee{i:02x}"))
            .collect::<Vec<_>>()
    );
    assert!(!valid(c));
}
