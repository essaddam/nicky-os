//! Fixed destination, fixed path and no redirects or implicit retries.
use crate::{
    Config, Transport,
    api::{Answer, Wire},
    protocol::Action,
    xml::{Failure, MAX_BODY},
};
use http::{HeaderValue, Method, Request};
use http_body_util::Full;
use std::time::Duration;
pub(crate) struct Network {
    pub config: Config,
    pub tls: Option<moli_net::PinnedHttps>,
}
fn failure(error: &anyhow::Error) -> Failure {
    if format!("{error:#}").contains(moli_net::PIN_MISMATCH) {
        Failure::Certificate
    } else {
        Failure::Transport
    }
}
impl Wire for Network {
    fn host(&self) -> &str {
        &self.config.host
    }
    fn https(&self) -> bool {
        self.tls.is_some()
    }
    async fn send(
        &self,
        action: Action,
        body: String,
        cookie: Option<String>,
    ) -> Result<Answer, Failure> {
        let mut builder = Request::builder()
            .method(Method::POST)
            .uri("/soap/server_sa/")
            .header("host", format!("{}:{}", self.config.host, self.config.port))
            .header("content-type", "multipart/form-data")
            .header(
                "soapaction",
                format!(
                    "urn:NETGEAR-ROUTER:service:{}#{}",
                    action.service(),
                    action.name()
                ),
            )
            .header("cache-control", "no-cache");
        if let Some(cookie) = cookie {
            let mut value = HeaderValue::from_str(&cookie).map_err(|_| Failure::Invalid)?;
            value.set_sensitive(true);
            builder = builder.header("cookie", value);
        }
        let request = builder
            .body(Full::new(moli_net::Body::from(body)))
            .map_err(|_| Failure::Invalid)?;
        let exchange = async {
            let (status, headers, body) = if let Some(tls) = &self.tls {
                tls.send_once_limited_with_headers(request, MAX_BODY).await
            } else {
                moli_net::web_with_headers(
                    &self.config.host,
                    self.config.port,
                    false,
                    request,
                    Duration::from_secs(5),
                    MAX_BODY,
                )
                .await
            }
            .map_err(|e| failure(&e))?;
            Ok(Answer {
                status,
                headers,
                body: body.to_vec(),
            })
        };
        tokio::time::timeout(Duration::from_secs(5), exchange)
            .await
            .map_err(|_| Failure::Transport)?
    }
}
impl Network {
    pub async fn open(
        config: &Config,
        pin: Option<String>,
    ) -> Result<(Self, Option<String>), Failure> {
        let tls = if config.transport == Transport::Https {
            let tls = moli_net::PinnedHttps::new(&config.host, config.port, pin)
                .map_err(|e| failure(&e))?;
            tokio::time::timeout(Duration::from_secs(5), tls.handshake())
                .await
                .map_err(|_| Failure::Transport)?
                .map_err(|e| failure(&e))?;
            Some(tls)
        } else {
            if !config.allow_plain_auth {
                return Err(Failure::Refused);
            }
            None
        };
        let fingerprint = tls
            .as_ref()
            .and_then(moli_net::PinnedHttps::pinned_fingerprint);
        Ok((
            Self {
                config: config.clone(),
                tls,
            },
            fingerprint,
        ))
    }
}
#[cfg(test)]
#[path = "network_tests.rs"]
pub(crate) mod tests;
