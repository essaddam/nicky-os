//! One serialized session; refusal is terminal, expiry has one bounded refresh.
use crate::{
    Failure, json,
    model::{self, Action, Reading},
    protocol::{self, Call},
};
use http::StatusCode;
use std::{collections::BTreeSet, future::Future};
pub(crate) struct Answer {
    pub status: StatusCode,
    pub body: Vec<u8>,
}
pub(crate) trait Wire: Sync {
    fn send(
        &self,
        call: Call,
        body: String,
        token: Option<String>,
    ) -> impl Future<Output = Result<Answer, Failure>> + Send;
}
#[derive(Default)]
pub(crate) struct Budget {
    pub sent: usize,
    pub relogin: bool,
}
// Never Debug: contains credentials derived session material.
pub(crate) struct Client {
    pub identity: Option<String>,
    pub epoch: u64,
    token: Option<String>,
    parked: Option<Failure>,
    unsupported: BTreeSet<Action>,
}
impl Client {
    pub fn new(identity: Option<String>) -> Self {
        Self {
            identity,
            epoch: 0,
            token: None,
            parked: None,
            unsupported: BTreeSet::new(),
        }
    }
    async fn exchange(
        &self,
        wire: &impl Wire,
        call: Call,
        credentials: Option<(&str, &str)>,
        budget: &mut Budget,
    ) -> Result<Vec<u8>, Failure> {
        if budget.sent >= 16 {
            return Err(Failure::Budget);
        }
        let body = protocol::body(call, credentials)?;
        budget.sent += 1;
        let answer = wire
            .send(
                call,
                body,
                if call == Call::Login {
                    None
                } else {
                    self.token.clone()
                },
            )
            .await?;
        match answer.status {
            StatusCode::LOCKED | StatusCode::TOO_MANY_REQUESTS | StatusCode::FORBIDDEN => {
                return Err(Failure::Refused);
            }
            status if status.is_redirection() => return Err(Failure::Invalid),
            _ => {}
        }
        let decoded =
            json::parse(&answer.body).and_then(|v| protocol::access(&v, call == Call::Login));
        // A challenge must not hide a JSON credential refusal, lock or captcha.
        if decoded == Err(Failure::Refused) {
            return Err(Failure::Refused);
        }
        match answer.status {
            StatusCode::UNAUTHORIZED => {
                return Err(if call == Call::Login {
                    Failure::Refused
                } else if decoded == Err(Failure::Invalid) {
                    Failure::Invalid
                } else {
                    Failure::Expired
                });
            }
            StatusCode::NOT_FOUND | StatusCode::NOT_IMPLEMENTED => {
                return Err(Failure::Unsupported);
            }
            _ => {}
        }
        if let Err(e) = decoded {
            return Err(
                if answer.status.is_server_error()
                    && !matches!(e, Failure::Refused | Failure::Expired)
                {
                    Failure::Transport
                } else {
                    e
                },
            );
        }
        if answer.status != StatusCode::OK {
            return Err(Failure::Transport);
        }
        Ok(answer.body)
    }
    fn check_identity(&mut self, reading: &Reading) -> Result<(), Failure> {
        let observed = reading.identity.as_ref().ok_or(Failure::Identity)?;
        if self
            .identity
            .as_ref()
            .is_some_and(|expected| expected != observed)
        {
            return Err(Failure::Identity);
        }
        self.identity = Some(observed.clone());
        Ok(())
    }
    async fn login(
        &mut self,
        wire: &impl Wire,
        credentials: (&str, &str),
        budget: &mut Budget,
    ) -> Result<Reading, Failure> {
        self.token = None;
        let body = self
            .exchange(wire, Call::Login, Some(credentials), budget)
            .await
            .map_err(|e| {
                if matches!(e, Failure::Unsupported | Failure::Expired) {
                    Failure::Refused
                } else {
                    e
                }
            })?;
        self.token = Some(protocol::token(&json::parse(&body)?)?);
        let info = self
            .exchange(wire, Call::Read(Action::Info), None, budget)
            .await
            .map_err(|e| match e {
                Failure::Unsupported => Failure::Identity,
                Failure::Expired => Failure::Refused,
                e => e,
            })?;
        let reading = model::parse(Action::Info, &info)?;
        self.check_identity(&reading)?;
        self.epoch = self.epoch.wrapping_add(1);
        self.unsupported.clear();
        Ok(reading)
    }
    async fn read_inner(
        &mut self,
        wire: &impl Wire,
        action: Action,
        credentials: (&str, &str),
        budget: &mut Budget,
    ) -> Result<Option<Reading>, Failure> {
        if self.token.is_none() {
            let info = match self.login(wire, credentials, budget).await {
                Ok(info) => info,
                Err(e) => {
                    self.token = None;
                    return Err(e);
                }
            };
            if action == Action::Info {
                return Ok(Some(info));
            }
        }
        if self.unsupported.contains(&action) {
            return Ok(None);
        }
        let mut result = self.exchange(wire, Call::Read(action), None, budget).await;
        if matches!(result, Err(Failure::Expired)) {
            if budget.relogin {
                return Err(Failure::Refused);
            }
            budget.relogin = true;
            let info = match self.login(wire, credentials, budget).await {
                Ok(info) => info,
                Err(e) => {
                    self.token = None;
                    return Err(e);
                }
            };
            if action == Action::Info {
                return Ok(Some(info));
            }
            result = self.exchange(wire, Call::Read(action), None, budget).await;
        }
        match result {
            Err(Failure::Unsupported) if action != Action::Info => {
                self.unsupported.insert(action);
                Ok(None)
            }
            Err(Failure::Unsupported) => Err(Failure::Identity),
            Err(Failure::Expired) => Err(Failure::Refused),
            Err(e) => Err(e),
            Ok(body) => {
                let reading = model::parse(action, &body)?;
                if action == Action::Info {
                    self.check_identity(&reading)?;
                }
                Ok(Some(reading))
            }
        }
    }
    pub async fn read(
        &mut self,
        wire: &impl Wire,
        action: Action,
        credentials: (&str, &str),
        budget: &mut Budget,
    ) -> Result<Option<Reading>, Failure> {
        if let Some(e) = self.parked {
            return Err(e);
        }
        let result = self.read_inner(wire, action, credentials, budget).await;
        if let Err(e) = result
            && matches!(e, Failure::Refused | Failure::Identity | Failure::Invalid)
        {
            self.token = None;
            self.parked = Some(e);
        }
        result
    }
}
#[cfg(test)]
mod tests;
