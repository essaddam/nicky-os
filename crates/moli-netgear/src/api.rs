//! A single serialized SOAP session with a closed set of actions.
use crate::{
    Version,
    cookies::Cookies,
    model::{self, Read},
    protocol::{self, Action},
    xml::Failure,
};
use http::StatusCode;
use ring::rand::{SecureRandom as _, SystemRandom};
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::future::Future;
pub(crate) struct Answer {
    pub status: StatusCode,
    pub headers: http::HeaderMap,
    pub body: Vec<u8>,
}
pub(crate) trait Wire: Sync {
    fn host(&self) -> &str;
    fn https(&self) -> bool;
    fn send(
        &self,
        action: Action,
        body: String,
        cookie: Option<String>,
    ) -> impl Future<Output = Result<Answer, Failure>> + Send;
}
#[derive(Default)]
pub(crate) struct Budget {
    pub sent: usize,
    pub relogin: bool,
}
// No Debug: session material and credentials must never appear in diagnostics.
pub(crate) struct Client {
    pub identity: Option<String>,
    version: Version,
    cookies: Cookies,
    authenticated: bool,
    parked: Option<Failure>,
    unsupported: BTreeSet<Action>,
    session: String,
}
impl Client {
    pub fn new(version: Version, identity: Option<String>) -> Self {
        Self {
            identity,
            version,
            cookies: Cookies::default(),
            authenticated: false,
            parked: None,
            unsupported: BTreeSet::new(),
            session: String::new(),
        }
    }
    async fn exchange(
        &mut self,
        wire: &impl Wire,
        action: Action,
        credentials: Option<(&str, &str)>,
        budget: &mut Budget,
    ) -> Result<Vec<u8>, Failure> {
        if budget.sent >= 16 {
            return Err(Failure::Budget);
        }
        budget.sent += 1;
        let cookie = credentials
            .is_none()
            .then(|| self.cookies.header())
            .flatten();
        let answer = wire
            .send(
                action,
                protocol::envelope(action, credentials, &self.session),
                cookie,
            )
            .await?;
        match answer.status {
            StatusCode::LOCKED => return Err(Failure::Locked),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                return Err(Failure::Refused);
            }
            StatusCode::NOT_FOUND | StatusCode::NOT_IMPLEMENTED => {
                return Err(Failure::Unsupported);
            }
            status if status.is_redirection() => return Err(Failure::Invalid),
            _ => {}
        }
        // SOAP Faults also arrive with HTTP 500. Interpret only known codes,
        // never propagate the remote fault text or raw authentication body.
        if let Err(error) = protocol::decode(&answer.body, action) {
            return Err(
                if (answer.status.is_server_error()
                    || answer.status == StatusCode::TOO_MANY_REQUESTS)
                    && !matches!(
                        error,
                        Failure::Refused | Failure::Locked | Failure::Unsupported
                    )
                {
                    Failure::Transport
                } else {
                    error
                },
            );
        }
        if answer.status != StatusCode::OK {
            return Err(Failure::Transport);
        }
        let cookies = answer
            .headers
            .get_all("set-cookie")
            .iter()
            .take(9)
            .map(|v| v.to_str().map(str::to_owned).map_err(|_| Failure::Invalid))
            .collect::<Result<Vec<_>, _>>()?;
        self.cookies.apply(&cookies, wire.host(), wire.https())?;
        Ok(answer.body)
    }
    async fn login(
        &mut self,
        wire: &impl Wire,
        credentials: (&str, &str),
        budget: &mut Budget,
    ) -> Result<Read, Failure> {
        self.cookies.clear();
        self.authenticated = false;
        if [credentials.0, credentials.1]
            .iter()
            .any(|s| s.is_empty() || s.len() > 1024 || s.chars().any(char::is_control))
        {
            self.parked = Some(Failure::Refused);
            return Err(Failure::Refused);
        }
        let mut random = [0u8; 10];
        SystemRandom::new()
            .fill(&mut random)
            .map_err(|_| Failure::Transport)?;
        self.session = String::with_capacity(random.len() * 2);
        for byte in random {
            write!(&mut self.session, "{byte:02x}").map_err(|_| Failure::Transport)?;
        }
        let action = if self.version == Version::V1 {
            Action::LoginV1
        } else {
            Action::LoginV2
        };
        if let Err(failure) = self.exchange(wire, action, Some(credentials), budget).await {
            if matches!(
                failure,
                Failure::Refused | Failure::Locked | Failure::Invalid | Failure::Unsupported
            ) {
                let terminal = if failure == Failure::Locked {
                    Failure::Locked
                } else {
                    Failure::Refused
                };
                self.parked = Some(terminal);
                return Err(terminal);
            }
            return Err(failure);
        }
        if self.version == Version::V2 && self.cookies.header().is_none() {
            self.parked = Some(Failure::Refused);
            return Err(Failure::Refused);
        }
        let body = self
            .exchange(wire, Action::Info, None, budget)
            .await
            .map_err(|e| {
                if e == Failure::Unsupported {
                    Failure::Identity
                } else {
                    e
                }
            })?;
        let info = model::parse(Action::Info, &body)?;
        self.check_identity(&info)?;
        self.unsupported.clear();
        self.authenticated = true;
        Ok(info)
    }
    fn check_identity(&mut self, info: &Read) -> Result<(), Failure> {
        let observed = info.identity.as_ref().ok_or(Failure::Identity)?;
        if self
            .identity
            .as_ref()
            .is_some_and(|expected| expected != observed)
        {
            self.parked = Some(Failure::Identity);
            return Err(Failure::Identity);
        }
        self.identity = Some(observed.clone());
        Ok(())
    }
    pub async fn read(
        &mut self,
        wire: &impl Wire,
        action: Action,
        credentials: (&str, &str),
        budget: &mut Budget,
    ) -> Result<Option<Read>, Failure> {
        if let Some(reason) = self.parked {
            return Err(reason);
        }
        if !self.authenticated {
            let info = self.login(wire, credentials, budget).await?;
            if action == Action::Info {
                return Ok(Some(info));
            }
        }
        if self.unsupported.contains(&action) {
            return Ok(None);
        }
        let mut result = self.exchange(wire, action, None, budget).await;
        if matches!(result, Err(Failure::Refused)) {
            self.authenticated = false;
            if budget.relogin {
                self.parked = Some(Failure::Refused);
                return Err(Failure::Refused);
            }
            budget.relogin = true;
            let info = self.login(wire, credentials, budget).await?;
            if action == Action::Info {
                return Ok(Some(info));
            }
            result = self.exchange(wire, action, None, budget).await;
        }
        match result {
            Ok(body) => {
                let read = model::parse(action, &body)?;
                if action == Action::Info {
                    self.check_identity(&read)?;
                }
                Ok(Some(read))
            }
            Err(Failure::Unsupported) if action != Action::Info => {
                self.unsupported.insert(action);
                Ok(None)
            }
            Err(Failure::Unsupported) if action == Action::Info => {
                self.parked = Some(Failure::Identity);
                Err(Failure::Identity)
            }
            Err(terminal @ (Failure::Refused | Failure::Locked)) => {
                self.parked = Some(terminal);
                Err(terminal)
            }
            Err(error) => Err(error),
        }
    }
}
#[cfg(test)]
#[path = "api_tests.rs"]
mod tests;
