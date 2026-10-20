use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub timezone: String,
    #[serde(default = "default_poll")]
    pub poll_seconds: u64,
}

fn default_poll() -> u64 {
    300
}

#[derive(Debug, thiserror::Error)]
#[error("{}", moli_i18n::tr!("pilotes.reservations.configuration_invalide"))]
pub struct ConfigError;

impl Config {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if !(30..=3600).contains(&self.poll_seconds)
            || jiff::tz::TimeZone::get(&self.timezone).is_err()
        {
            return Err(ConfigError);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timezone_is_explicit_and_the_url_is_never_an_agent_config_option() {
        let config: Config = serde_json::from_str(r#"{"timezone":"Europe/Paris"}"#).unwrap();
        assert_eq!(config.poll_seconds, 300);
        assert!(config.validate().is_ok());
        for value in [
            "{}",
            r#"{"timezone":"UTC","url":"https://calendar.example.com/"}"#,
        ] {
            assert!(serde_json::from_str::<Config>(value).is_err());
        }
    }
    #[test]
    fn unknown_zones_and_polling_floods_or_unbounded_staleness_are_refused() {
        for (timezone, poll_seconds) in
            [("not/a-zone", 300), ("UTC", 0), ("UTC", 29), ("UTC", 3601)]
        {
            assert!(
                Config {
                    timezone: timezone.into(),
                    poll_seconds
                }
                .validate()
                .is_err()
            );
        }
        for poll_seconds in [30, 300, 3600] {
            assert!(
                Config {
                    timezone: "UTC".into(),
                    poll_seconds
                }
                .validate()
                .is_ok()
            );
        }
    }
}
