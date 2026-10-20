use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub host: String,
}

impl Config {
    pub(crate) fn check(&self) -> anyhow::Result<()> {
        let host = &self.host;
        if !host.is_empty() && host.len() <= 253 && host.parse::<std::net::IpAddr>().is_ok() {
            return Ok(());
        }
        let valid = !host.is_empty()
            && host.len() <= 253
            && host.is_ascii()
            && host.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && label.as_bytes()[0].is_ascii_alphanumeric()
                    && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                    && label
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            });
        anyhow::ensure!(valid, moli_i18n::tr!("pilotes.androidtv.hote_invalide"));
        Ok(())
    }
}
