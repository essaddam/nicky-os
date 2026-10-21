use super::*;
fn config() -> Config {
    Config {
        host: None,
        interface: Some("192.168.1.2".parse().unwrap()),
        prefix: 24,
        external_ids: Vec::new(),
    }
}
fn name(value: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for part in value.trim_end_matches('.').split('.') {
        out.push(u8::try_from(part.len()).unwrap());
        out.extend_from_slice(part.as_bytes());
    }
    out.push(0);
    out
}
fn record(owner: &str, kind: u16, bytes: &[u8]) -> Vec<u8> {
    let mut out = name(owner);
    out.extend_from_slice(&kind.to_be_bytes());
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&120u32.to_be_bytes());
    out.extend_from_slice(&u16::try_from(bytes.len()).unwrap().to_be_bytes());
    out.extend_from_slice(bytes);
    out
}
fn answer(ip: Ipv4Addr, port: u16, id: &str) -> Vec<u8> {
    let service = "_googlecast._tcp.local";
    let instance = "Example._googlecast._tcp.local";
    let host = "receiver.local";
    let mut out = vec![0, 0, 0x84, 0, 0, 0, 0, 4, 0, 0, 0, 0];
    out.extend(record(service, 12, &name(instance)));
    let mut srv = vec![0, 0, 0, 0];
    srv.extend_from_slice(&port.to_be_bytes());
    srv.extend(name(host));
    out.extend(record(instance, 33, &srv));
    let mut txt = Vec::new();
    for value in [
        format!("id={id}"),
        "fn=Example screen".into(),
        "ca=5".into(),
    ] {
        txt.push(u8::try_from(value.len()).unwrap());
        txt.extend_from_slice(value.as_bytes());
    }
    out.extend(record(instance, 16, &txt));
    out.extend(record(host, 1, &ip.octets()));
    out
}
#[test]
fn valid_private_discovery_is_a_candidate_not_an_activation() {
    let ip = "192.168.1.10".parse().unwrap();
    let found = parse_packet(
        &answer(ip, 8009, "00000000000000000000000000000001"),
        ip,
        &config(),
    );
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, "00000000000000000000000000000001");
    assert_eq!(found[0].host, ip);
    assert_eq!(found[0].name, "Example screen");
    assert!(found[0].video);
}
#[test]
fn private_scope_is_the_declared_subnet_not_all_private_addresses() {
    let cfg = config();
    cfg.check().unwrap();
    assert!(cfg.in_scope("192.168.1.10".parse().unwrap()));
    for bad in [
        "192.168.2.10",
        "8.8.8.8",
        "127.0.0.1",
        "192.168.1.255",
        "192.168.1.0",
        "192.168.1.2",
    ] {
        assert!(!cfg.in_scope(bad.parse().unwrap()), "{bad}");
    }
}
#[test]
fn invalid_config_cannot_start_a_discovery_socket() {
    let mut c = config();
    c.prefix = 8;
    assert!(c.check().is_err());
    c.prefix = 24;
    c.interface = None;
    assert!(c.check().is_err());
    c.host = Some("8.8.8.8".parse().unwrap());
    assert!(c.check().is_err());
}
#[test]
fn discovery_cannot_redirect_to_a_third_party_or_another_port() {
    let ip = "192.168.1.10".parse().unwrap();
    let other = "192.168.1.11".parse().unwrap();
    assert!(
        parse_packet(
            &answer(other, 8009, "00000000000000000000000000000001"),
            ip,
            &config()
        )
        .is_empty()
    );
    assert!(
        parse_packet(
            &answer(ip, 8790, "00000000000000000000000000000001"),
            ip,
            &config()
        )
        .is_empty()
    );
}
#[test]
fn malformed_dns_and_claimed_names_do_not_create_an_identity() {
    let ip = "192.168.1.10".parse().unwrap();
    for id in ["", "screen", "a", "0000000000000000000000000000000z"] {
        assert!(parse_packet(&answer(ip, 8009, id), ip, &config()).is_empty());
    }
    for data in [
        vec![0; 10],
        vec![0; 9001],
        vec![0, 0, 0x84, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0xc0, 12],
    ] {
        assert!(parse_packet(&data, ip, &config()).is_empty());
    }
}
