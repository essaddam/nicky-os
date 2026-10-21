//! A serialized, instance-local HiLink session with one recovery per cycle.
use crate::{
    Auth, Failure, Session,
    cookies::Cookies,
    model::{self, Action, Reading},
    protocol::{self, Call},
    tokens::Tokens,
    xml,
};
use http::StatusCode;
use std::{collections::BTreeSet, future::Future};
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
        call: Call,
        body: Option<String>,
        token: Option<String>,
        cookie: Option<String>,
    ) -> impl Future<Output = Result<Answer, Failure>> + Send;
}
#[derive(Default)]
pub(crate) struct Budget {
    pub sent: usize,
    pub relogin: bool,
}
// Deliberately no Debug: session material must never appear in diagnostics.
pub(crate) struct Client {
    pub identity: Option<String>,
    pub epoch: u64,
    auth: Auth,
    session: Session,
    cookies: Cookies,
    tokens: Tokens,
    ready: bool,
    parked: Option<Failure>,
    unsupported: BTreeSet<Action>,
}
impl Client {
    pub fn new(auth: Auth, session: Session, identity: Option<String>) -> Self {
        Self {
            identity,
            epoch: 0,
            auth,
            session,
            cookies: Cookies::default(),
            tokens: Tokens::default(),
            ready: false,
            parked: None,
            unsupported: BTreeSet::new(),
        }
    }
    async fn exchange(
        &mut self,
        wire: &impl Wire,
        call: Call,
        credentials: Option<(&str, &str)>,
        budget: &mut Budget,
    ) -> Result<Vec<u8>, Failure> {
        if budget.sent >= 16 {
            return Err(Failure::Budget);
        }
        let bootstrap = matches!(call, Call::Session | Call::Token);
        let token = if bootstrap {
            None
        } else if call == Call::Login {
            self.tokens.take()
        } else {
            self.tokens.for_read()
        };
        let body = if call == Call::Login {
            let (user, password) = credentials.ok_or(Failure::Secrets)?;
            Some(protocol::login(
                user,
                password,
                token.as_deref().ok_or(Failure::Invalid)?,
            )?)
        } else {
            None
        };
        if call == Call::Login {
            self.tokens.clear();
        }
        budget.sent += 1;
        let answer = wire
            .send(
                call,
                body,
                token,
                if bootstrap {
                    None
                } else {
                    self.cookies.header()
                },
            )
            .await?;
        match answer.status {
            StatusCode::FORBIDDEN | StatusCode::LOCKED | StatusCode::TOO_MANY_REQUESTS => {
                return Err(Failure::Refused);
            }
            s if s.is_redirection() => return Err(Failure::Invalid),
            _ => {}
        }
        let decoded = xml::parse(&answer.body);
        if matches!(decoded, Err(Failure::Refused)) {
            return Err(Failure::Refused);
        }
        match answer.status {
            StatusCode::UNAUTHORIZED => {
                return Err(if matches!(call, Call::Login | Call::State) {
                    Failure::Refused
                } else if matches!(decoded, Err(Failure::Invalid)) {
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
                    && !matches!(e, Failure::Expired | Failure::Unsupported)
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
        let cookies = answer
            .headers
            .get_all("set-cookie")
            .iter()
            .take(9)
            .map(|v| v.to_str().map(str::to_owned).map_err(|_| Failure::Invalid))
            .collect::<Result<Vec<_>, _>>()?;
        self.cookies.apply(&cookies, wire.host(), wire.https())?;
        self.tokens.replace(&answer.headers)?;
        Ok(answer.body)
    }
    fn verify(&mut self, read: &Reading) -> Result<(), Failure> {
        let observed = read.identity.as_ref().ok_or(Failure::Identity)?;
        if self.identity.as_ref().is_some_and(|id| id != observed) {
            return Err(Failure::Identity);
        }
        self.identity = Some(observed.clone());
        Ok(())
    }
    async fn open(
        &mut self,
        wire: &impl Wire,
        credentials: Option<(&str, &str)>,
        budget: &mut Budget,
    ) -> Result<Reading, Failure> {
        self.ready = false;
        self.cookies.clear();
        self.tokens.clear();
        let route = match self.session {
            Session::Sestok => Call::Session,
            Session::Token => Call::Token,
        };
        let raw = self
            .exchange(wire, route, None, budget)
            .await
            .map_err(|e| {
                if e == Failure::Unsupported {
                    Failure::Variant
                } else {
                    e
                }
            })?;
        let fields = xml::parse(&raw)?;
        match self.session {
            Session::Sestok => {
                let cookie = fields.values.get("SesInfo").ok_or(Failure::Variant)?;
                self.cookies
                    .apply(std::slice::from_ref(cookie), wire.host(), wire.https())?;
                self.tokens
                    .seed(fields.values.get("TokInfo").ok_or(Failure::Variant)?)?;
            }
            Session::Token => self
                .tokens
                .seed(fields.values.get("token").ok_or(Failure::Variant)?)?,
        }
        if self.cookies.header().is_none() {
            return Err(Failure::Variant);
        }
        if self.auth == Auth::Sha256V4 {
            let raw = self
                .exchange(wire, Call::State, None, budget)
                .await
                .map_err(|e| {
                    if e == Failure::Unsupported {
                        Failure::Variant
                    } else {
                        e
                    }
                })?;
            let fields = xml::parse(&raw)?;
            if fields.values.get("password_type").map(String::as_str) != Some("4") {
                return Err(Failure::Variant);
            }
            match fields.values.get("State").map(String::as_str) {
                Some("0") => {}
                Some("-1") => {
                    let response = self
                        .exchange(wire, Call::Login, credentials, budget)
                        .await
                        .map_err(|e| {
                            if e == Failure::Unsupported {
                                Failure::Variant
                            } else {
                                e
                            }
                        })?;
                    if !xml::parse(&response)?.ok {
                        return Err(Failure::Refused);
                    }
                    if self.tokens.is_empty() {
                        return Err(Failure::Variant);
                    }
                }
                _ => return Err(Failure::Refused),
            }
        }
        let raw = self
            .exchange(wire, Call::Read(Action::Info), None, budget)
            .await
            .map_err(|e| {
                if e == Failure::Unsupported {
                    Failure::Identity
                } else {
                    e
                }
            })?;
        let info = model::parse(Action::Info, &raw)?;
        self.verify(&info)?;
        self.epoch = self.epoch.wrapping_add(1);
        self.ready = true;
        self.unsupported.clear();
        Ok(info)
    }
    async fn verified_open(
        &mut self,
        wire: &impl Wire,
        credentials: Option<(&str, &str)>,
        budget: &mut Budget,
    ) -> Result<Reading, Failure> {
        self.open(wire, credentials, budget)
            .await
            .map_err(|e| {
                self.ready = false;
                if e == Failure::Expired {
                    Failure::Refused
                } else {
                    e
                }
            })
            .inspect(|_| {
                self.ready = true;
            })
    }
    async fn read_inner(
        &mut self,
        wire: &impl Wire,
        action: Action,
        credentials: Option<(&str, &str)>,
        budget: &mut Budget,
    ) -> Result<Option<Reading>, Failure> {
        if !self.ready {
            let info = self.verified_open(wire, credentials, budget).await?;
            if action == Action::Info {
                return Ok(Some(info));
            }
        }
        if self.unsupported.contains(&action) {
            return Ok(None);
        }
        let mut result = self.exchange(wire, Call::Read(action), None, budget).await;
        if matches!(result, Err(Failure::Expired)) {
            self.ready = false;
            if budget.relogin {
                return Err(Failure::Refused);
            }
            budget.relogin = true;
            let info = self.verified_open(wire, credentials, budget).await?;
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
            Ok(raw) => {
                let read = model::parse(action, &raw)?;
                if action == Action::Info {
                    self.verify(&read)?;
                }
                Ok(Some(read))
            }
        }
    }
    pub async fn read(
        &mut self,
        wire: &impl Wire,
        action: Action,
        credentials: Option<(&str, &str)>,
        budget: &mut Budget,
    ) -> Result<Option<Reading>, Failure> {
        if let Some(e) = self.parked {
            return Err(e);
        }
        let result = self.read_inner(wire, action, credentials, budget).await;
        if let Err(e) = result
            && matches!(
                e,
                Failure::Refused
                    | Failure::Invalid
                    | Failure::Identity
                    | Failure::Variant
                    | Failure::Certificate
            )
        {
            self.parked = Some(e);
            self.ready = false;
            self.cookies.clear();
            self.tokens.clear();
        }
        result
    }
}
#[cfg(test)]
mod tests;
