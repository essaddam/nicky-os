//! The only SOAP actions this client can send.
use crate::xml::{self, Failure};
pub(crate) const SOAP: &str = "http://schemas.xmlsoap.org/soap/envelope/";
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Action {
    LoginV1,
    LoginV2,
    Info,
    System,
    Ethernet,
    Traffic,
    ClientsV1,
    ClientsV2,
}
impl Action {
    pub fn service(self) -> &'static str {
        match self {
            Self::LoginV1 => "ParentalControl:1",
            Self::LoginV2 | Self::Traffic => "DeviceConfig:1",
            Self::Ethernet => "WANEthernetLinkConfig:1",
            _ => "DeviceInfo:1",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::LoginV1 => "Authenticate",
            Self::LoginV2 => "SOAPLogin",
            Self::Info => "GetInfo",
            Self::System => "GetSystemInfo",
            Self::Ethernet => "GetEthernetLinkStatus",
            Self::Traffic => "GetTrafficMeterStatistics",
            Self::ClientsV1 => "GetAttachDevice",
            Self::ClientsV2 => "GetAttachDevice2",
        }
    }
}
pub(crate) fn envelope(action: Action, credentials: Option<(&str, &str)>, session: &str) -> String {
    fn escape(value: &str) -> String {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }
    let fields = credentials.map_or_else(String::new, |(user, password)| {
        let (u, p) = (escape(user), escape(password));
        if action == Action::LoginV1 {
            format!("<NewUsername>{u}</NewUsername><NewPassword>{p}</NewPassword>")
        } else {
            format!("<Username>{u}</Username><Password>{p}</Password>")
        }
    });
    let call = if action == Action::LoginV1 {
        format!("<Authenticate>{fields}</Authenticate>")
    } else {
        format!(
            "<u:{name} xmlns:u='urn:NETGEAR-ROUTER:service:{service}'>{fields}</u:{name}>",
            name = action.name(),
            service = action.service()
        )
    };
    format!(
        "<?xml version='1.0' encoding='utf-8'?><s:Envelope xmlns:s='{SOAP}'><s:Header><SessionID>{}</SessionID></s:Header><s:Body>{call}</s:Body></s:Envelope>",
        escape(session)
    )
}

pub(crate) fn field<'a>(
    node: roxmltree::Node<'a, '_>,
    name: &str,
) -> Result<Option<&'a str>, Failure> {
    let mut matches = node
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == name);
    let Some(found) = matches.next() else {
        return Ok(None);
    };
    if matches.next().is_some()
        || found.children().any(|n| n.is_element())
        || found
            .tag_name()
            .namespace()
            .is_some_and(|ns| Some(ns) != node.tag_name().namespace())
    {
        return Err(Failure::Invalid);
    }
    Ok(Some(found.text().unwrap_or("").trim()))
}

pub(crate) fn payload<'a, 'input>(
    doc: &'a roxmltree::Document<'input>,
    action: Action,
) -> Result<roxmltree::Node<'a, 'input>, Failure> {
    let root = doc.root_element();
    if root.tag_name().name() != "Envelope" || root.tag_name().namespace() != Some(SOAP) {
        return Err(Failure::Invalid);
    }
    let mut bodies = root.children().filter(|n| {
        n.is_element() && n.tag_name().name() == "Body" && n.tag_name().namespace() == Some(SOAP)
    });
    let body = bodies.next().ok_or(Failure::Invalid)?;
    if bodies.next().is_some() {
        return Err(Failure::Invalid);
    }
    let name = format!("{}Response", action.name());
    let mut responses = body
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == name);
    let node = responses.next().ok_or(Failure::Invalid)?;
    let service = format!("urn:NETGEAR-ROUTER:service:{}", action.service());
    if responses.next().is_some() || node.tag_name().namespace().is_some_and(|ns| ns != service) {
        return Err(Failure::Invalid);
    }
    Ok(node)
}

fn status(code: &str) -> Result<(), Failure> {
    match code.parse::<u16>() {
        Ok(0) => Ok(()),
        Ok(401 | 403) => Err(Failure::Refused),
        Ok(423) => Err(Failure::Locked),
        Ok(503) => Err(Failure::Transport),
        Ok(404 | 501) => Err(Failure::Unsupported),
        _ => Err(Failure::Invalid),
    }
}

pub(crate) fn decode(body: &[u8], action: Action) -> Result<roxmltree::Document<'_>, Failure> {
    let doc = xml::parse(body)?;
    let root = doc.root_element();
    if root.tag_name().name() != "Envelope" || root.tag_name().namespace() != Some(SOAP) {
        return Err(Failure::Invalid);
    }
    let mut bodies = root.children().filter(|n| {
        n.is_element() && n.tag_name().name() == "Body" && n.tag_name().namespace() == Some(SOAP)
    });
    let soap_body = bodies.next().ok_or(Failure::Invalid)?;
    if bodies.next().is_some() {
        return Err(Failure::Invalid);
    }
    if let Some(fault) = soap_body.children().find(|n| {
        n.is_element() && n.tag_name().name() == "Fault" && n.tag_name().namespace() == Some(SOAP)
    }) {
        let mut codes = fault
            .descendants()
            .filter(|n| n.is_element() && n.tag_name().name() == "errorCode");
        let code = codes
            .next()
            .and_then(|n| n.text())
            .ok_or(Failure::Invalid)?;
        if codes.next().is_some() {
            return Err(Failure::Invalid);
        }
        return Err(status(code.trim()).err().unwrap_or(Failure::Invalid));
    }
    let mut codes = doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "ResponseCode");
    let code = codes.next().ok_or(Failure::Invalid)?;
    if codes.next().is_some()
        || !code.ancestors().any(|n| n == soap_body)
        || code.children().any(|n| n.is_element())
    {
        return Err(Failure::Invalid);
    }
    let service = format!("urn:NETGEAR-ROUTER:service:{}", action.service());
    if code
        .tag_name()
        .namespace()
        .is_some_and(|ns| ns != SOAP && ns != service)
    {
        return Err(Failure::Invalid);
    }
    status(code.text().unwrap_or("").trim())?;
    // Authentication contracts guarantee a response code and (v2) cookies,
    // not an action payload. Read actions require their exact response node.
    if !matches!(action, Action::LoginV1 | Action::LoginV2) {
        payload(&doc, action)?;
    }
    Ok(doc)
}
#[cfg(test)]
pub(crate) fn response(action: Action, fields: &str, code: u16) -> String {
    format!(
        "<s:Envelope xmlns:s='{SOAP}'><s:Body><u:{}Response xmlns:u='urn:NETGEAR-ROUTER:service:{}'>{fields}</u:{}Response><ResponseCode>{code}</ResponseCode></s:Body></s:Envelope>",
        action.name(),
        action.service(),
        action.name()
    )
}
#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
