use super::*;
#[test]
fn actions_are_closed_pairs_and_credentials_are_xml_escaped() {
    assert_eq!(Action::Info.service(), "DeviceInfo:1");
    assert_eq!(Action::Traffic.service(), "DeviceConfig:1");
    assert_eq!(Action::Ethernet.service(), "WANEthernetLinkConfig:1");
    let v1 = envelope(Action::LoginV1, Some(("a<&", "b>")), "test-session");
    assert!(v1.contains(
        "<Authenticate><NewUsername>a&lt;&amp;</NewUsername><NewPassword>b&gt;</NewPassword>"
    ));
    let v2 = envelope(Action::LoginV2, Some(("a<&", "b>")), "test-session");
    assert!(v2.contains("<Username>a&lt;&amp;</Username><Password>b&gt;</Password>"));
    assert!(!v2.contains("NewUsername"));
}
#[test]
fn namespaces_faults_and_response_codes_are_checked() {
    let xml = response(Action::Info, "<SerialNumber>test-router</SerialNumber>", 0);
    assert!(decode(xml.as_bytes(), Action::Info).is_ok());
    assert!(
        decode(
            xml.replace("<s:", "<z:")
                .replace("</s:", "</z:")
                .replace("xmlns:s=", "xmlns:z=")
                .as_bytes(),
            Action::Info
        )
        .is_ok()
    );
    assert_eq!(
        decode(response(Action::Info, "", 401).as_bytes(), Action::Info).unwrap_err(),
        Failure::Refused
    );
    assert_eq!(
        decode(response(Action::Info, "", 501).as_bytes(), Action::Info).unwrap_err(),
        Failure::Unsupported
    );
    assert!(decode(xml.replace(SOAP, "urn:wrong").as_bytes(), Action::Info).is_err());
    assert!(
        decode(
            xml.replace("DeviceInfo:1", "DeviceConfig:1").as_bytes(),
            Action::Info
        )
        .is_err()
    );
    assert!(decode(b"<html>login</html>", Action::Info).is_err());
    assert!(decode(response(Action::System, "", 0).as_bytes(), Action::Info).is_err());
    let fault = format!(
        "<s:Envelope xmlns:s='{SOAP}'><s:Body><s:Fault><faultcode>s:Client</faultcode><detail><errorCode>401</errorCode></detail></s:Fault></s:Body></s:Envelope>"
    );
    assert_eq!(
        decode(fault.as_bytes(), Action::Info).unwrap_err(),
        Failure::Refused
    );
}
#[test]
fn login_code_does_not_require_an_invented_payload_but_read_does() {
    let xml = format!(
        "<s:Envelope xmlns:s='{SOAP}'><s:Body><ResponseCode>000</ResponseCode></s:Body></s:Envelope>"
    );
    for action in [Action::LoginV1, Action::LoginV2] {
        assert!(decode(xml.as_bytes(), action).is_ok());
    }
    assert!(decode(xml.as_bytes(), Action::Info).is_err());
}
#[test]
fn a_foreign_namespace_cannot_supply_a_success_code() {
    let xml = response(
        Action::Info,
        "<SerialNumber>synthetic-router-01</SerialNumber>",
        0,
    )
    .replace("<ResponseCode>", "<e:ResponseCode xmlns:e='urn:foreign'>")
    .replace("</ResponseCode>", "</e:ResponseCode>");
    assert!(decode(xml.as_bytes(), Action::Info).is_err());
}
