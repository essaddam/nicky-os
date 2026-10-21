use serde::Deserialize;
use std::net::Ipv4Addr;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub host: Option<Ipv4Addr>,
    pub interface: Option<Ipv4Addr>,
    #[serde(default = "prefix")]
    pub prefix: u8,
    #[serde(default)]
    pub external_ids: Vec<String>,
}
fn prefix() -> u8 {
    24
}
impl Config {
    pub fn check(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            (16..=30).contains(&self.prefix)
                && (self.host.is_some() ^ self.interface.is_some())
                && self
                    .host
                    .or(self.interface)
                    .is_some_and(|ip| ip.is_private())
                && self.external_ids.len() <= 64
                && self
                    .external_ids
                    .iter()
                    .all(|id| crate::discovery::native_id(id).is_some()),
            moli_i18n::tr!("pilotes.cast.configuration")
        );
        Ok(())
    }
    pub fn in_scope(&self, ip: Ipv4Addr) -> bool {
        if !ip.is_private() || !(16..=30).contains(&self.prefix) {
            return false;
        }
        if let Some(host) = self.host {
            return host == ip;
        }
        let Some(interface) = self.interface else {
            return false;
        };
        let mask = u32::MAX << (32 - self.prefix);
        let value = u32::from(ip);
        let network = u32::from(interface) & mask;
        ip != interface && value & mask == network && value != network && value != (network | !mask)
    }
}
