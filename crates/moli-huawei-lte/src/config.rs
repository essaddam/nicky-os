use serde::Deserialize;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    Http,
    Https,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Auth {
    None,
    Sha256V4,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Session {
    #[default]
    Sestok,
    Token,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub transport: Transport,
    pub auth: Auth,
    #[serde(default)]
    pub session: Session,
    #[serde(default)]
    pub allow_plain_auth: bool,
    #[serde(default = "every")]
    pub every: u64,
    #[serde(default = "info")]
    pub info_every: u64,
}
fn every() -> u64 {
    60
}
fn info() -> u64 {
    3600
}
impl Config {
    pub fn checked(self) -> anyhow::Result<Self> {
        let host = &self.host;
        let valid = if let Ok(ip) = host.parse::<std::net::Ipv4Addr>() {
            ip.is_private() || ip.is_loopback()
        } else {
            !host.is_empty()
                && host.len() <= 253
                && host.split('.').all(|label| {
                    !label.is_empty()
                        && label.len() <= 63
                        && !label.starts_with('-')
                        && !label.ends_with('-')
                        && label
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                })
        };
        if !valid
            || self.port == 0
            || [self.every, self.info_every]
                .iter()
                .any(|n| !(30..=86400).contains(n))
            || self.info_every < self.every
            || self.transport == Transport::Http && !self.allow_plain_auth
        {
            anyhow::bail!(moli_i18n::tr!("pilotes.huawei_lte.configuration_invalide"));
        }
        Ok(self)
    }
}
