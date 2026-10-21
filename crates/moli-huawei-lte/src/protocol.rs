use crate::{Failure, model::Action};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Call {
    Session,
    Token,
    State,
    Login,
    Read(Action),
}
impl Call {
    pub fn path(self) -> &'static str {
        match self {
            Self::Session => "/api/webserver/SesTokInfo",
            Self::Token => "/api/webserver/token",
            Self::State => "/api/user/state-login",
            Self::Login => "/api/user/login",
            Self::Read(Action::Info) => "/api/device/information",
            Self::Read(Action::Status) => "/api/monitoring/status",
            Self::Read(Action::Signal) => "/api/device/signal",
            Self::Read(Action::Traffic) => "/api/monitoring/traffic-statistics",
        }
    }
}
pub(crate) fn login(user: &str, password: &str, token: &str) -> Result<String, Failure> {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use std::fmt::Write as _;
    if [user, password, token].iter().any(|s| {
        s.is_empty() || s.len() > 1024 || !s.is_ascii() || s.bytes().any(|b| b.is_ascii_control())
    }) {
        return Err(Failure::Refused);
    }
    let digest_hex = |bytes: &[u8]| -> Result<String, Failure> {
        let hash = ring::digest::digest(&ring::digest::SHA256, bytes);
        let mut hex = String::with_capacity(64);
        for b in hash.as_ref() {
            write!(&mut hex, "{b:02x}").map_err(|_| Failure::Invalid)?;
        }
        Ok(hex)
    };
    let first = STANDARD.encode(digest_hex(password.as_bytes())?);
    let proof = STANDARD.encode(digest_hex(format!("{user}{first}{token}").as_bytes())?);
    let escaped = user
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\"', "&quot;")
        .replace('\'', "&apos;");
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><request><Username>{escaped}</Username><Password>{proof}</Password><password_type>4</password_type></request>"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sha256_type_four_matches_an_independently_calculated_synthetic_vector() {
        let body = login("synthetic-user", "synthetic-password", "synthetic-token").unwrap();
        assert!(body.contains("<Password>MWRlOWQ5NzU4ZTE1M2JjNzYzMzAyNzczNzdiZDI0OTVkMjI3NDhjZDcxY2U0NzE3YzhiOTMxYTVmMmU4ODQyYQ==</Password>"));
        assert!(!body.contains("synthetic-password"));
        assert!(!body.contains("synthetic-token"));
        let escaped = login("synthetic<&", "synthetic-password", "synthetic-token").unwrap();
        assert!(escaped.contains("<Username>synthetic&lt;&amp;</Username>"));
        assert!(roxmltree::Document::parse(&escaped).is_ok());
        assert!(login("bad\r\nuser", "synthetic-password", "synthetic-token").is_err());
    }
}
