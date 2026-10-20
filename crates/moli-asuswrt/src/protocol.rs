//! Closed read-only hooks from the pinned AsusRouter wire contract.
use crate::{Failure, model::Action};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::Value;
use std::fmt::Write as _;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Call {
    Login,
    Read(Action),
}
impl Call {
    pub fn path(self) -> &'static str {
        match self {
            Self::Login => "/login.cgi",
            Self::Read(_) => "/appGet.cgi",
        }
    }
}
pub(crate) fn body(call: Call, credentials: Option<(&str, &str)>) -> Result<String, Failure> {
    let raw = match call {
        Call::Login => {
            let (user, password) = credentials.ok_or(Failure::Refused)?;
            if user.contains(':')
                || [user, password].iter().any(|s| {
                    s.is_empty()
                        || s.len() > 1024
                        || !s.is_ascii()
                        || s.bytes().any(|b| b.is_ascii_control())
                })
            {
                return Err(Failure::Refused);
            }
            format!(
                "login_authorization={}",
                STANDARD.encode(format!("{user}:{password}"))
            )
        }
        Call::Read(Action::Info) => {
            "hook=nvram_get(label_mac);nvram_get(productid);nvram_get(firmver);nvram_get(buildno)"
                .into()
        }
        Call::Read(Action::Telemetry) => {
            "hook=cpu_usage(appobj);memory_usage(appobj);netdev(appobj)".into()
        }
        Call::Read(Action::Clients) => "hook=get_clientlist()".into(),
    };
    // Upstream quotes the entire payload, including '=' and ';', with '/' safe.
    let mut encoded = String::with_capacity(raw.len() * 3);
    for b in raw.bytes() {
        if b.is_ascii_alphanumeric() || b"_-.~/".contains(&b) {
            encoded.push(char::from(b));
        } else {
            write!(&mut encoded, "%{b:02X}").map_err(|_| Failure::Invalid)?;
        }
    }
    Ok(encoded)
}
pub(crate) fn access(json: &Value, login: bool) -> Result<(), Failure> {
    let Some(code) = json.get("error_status") else {
        return Ok(());
    };
    let code = code
        .as_u64()
        .or_else(|| code.as_str()?.parse().ok())
        .ok_or(Failure::Invalid)?;
    match code {
        0 | 200 => Ok(()),
        1 | 2 | 8 if !login => Err(Failure::Expired),
        1..=11 => Err(Failure::Refused),
        _ => Err(Failure::Invalid),
    }
}
pub(crate) fn token(json: &Value) -> Result<String, Failure> {
    let token = json
        .get("asus_token")
        .and_then(Value::as_str)
        .ok_or(Failure::Refused)?;
    // RFC 6265 cookie-octet, excluding delimiters/control/non-ASCII bytes.
    if token.is_empty()
        || token.len() > 1024
        || !token
            .bytes()
            .all(|b| matches!(b, 0x21 | 0x23..=0x2b | 0x2d..=0x3a | 0x3c..=0x5b | 0x5d..=0x7e))
    {
        return Err(Failure::Refused);
    }
    Ok(token.to_owned())
}
