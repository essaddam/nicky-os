//! Bounded human control messages to the single credential-owning actor.

use serde::Serialize;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::Instant;

use crate::Error;

const CONTROL_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    NeedsCredentials,
    NeedsAuthorization,
    Authorizing,
    Connected,
    Unavailable,
}

pub struct Authorization {
    pub url: String,
    pub expires_in: u64,
}

impl std::fmt::Debug for Authorization {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Authorization")
            .field("expires_in", &self.expires_in)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct Gateway {
    tx: mpsc::Sender<Control>,
    status: watch::Receiver<Status>,
}

impl std::fmt::Debug for Gateway {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gateway").finish_non_exhaustive()
    }
}

pub(crate) enum Control {
    Authorize {
        binding: [u8; 32],
        deadline: Instant,
        reply: oneshot::Sender<Result<Authorization, Error>>,
    },
    Complete {
        binding: [u8; 32],
        state: String,
        code: String,
        deadline: Instant,
        reply: oneshot::Sender<Result<(), Error>>,
    },
}

impl std::fmt::Debug for Control {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Control").finish_non_exhaustive()
    }
}

impl Control {
    pub fn abandoned(&self) -> bool {
        match self {
            Self::Authorize {
                deadline, reply, ..
            } => reply.is_closed() || Instant::now() >= *deadline,
            Self::Complete {
                deadline, reply, ..
            } => reply.is_closed() || Instant::now() >= *deadline,
        }
    }

    pub fn reject(self, error: Error) {
        match self {
            Self::Authorize { reply, .. } => {
                let _ = reply.send(Err(error));
            }
            Self::Complete { reply, .. } => {
                let _ = reply.send(Err(error));
            }
        }
    }
}

pub(crate) fn channel() -> (Gateway, mpsc::Receiver<Control>, watch::Sender<Status>) {
    let (tx, rx) = mpsc::channel(4);
    let (status_tx, status) = watch::channel(Status::NeedsCredentials);
    (Gateway { tx, status }, rx, status_tx)
}

impl Gateway {
    pub fn status(&self) -> Status {
        *self.status.borrow()
    }

    pub async fn authorize(&self, binding: [u8; 32]) -> Result<Authorization, Error> {
        let deadline = Instant::now() + CONTROL_TIMEOUT;
        let (reply, rx) = oneshot::channel();
        self.submit(Control::Authorize {
            binding,
            deadline,
            reply,
        })?;
        response(rx, deadline).await
    }

    pub async fn complete(
        &self,
        binding: [u8; 32],
        state: String,
        code: String,
    ) -> Result<(), Error> {
        if state.len() != 64
            || !state.bytes().all(|byte| byte.is_ascii_hexdigit())
            || code.is_empty()
            || code.len() > 2048
            || !code.bytes().all(|byte| byte.is_ascii_graphic())
        {
            return Err(Error::Invalid);
        }
        let deadline = Instant::now() + CONTROL_TIMEOUT;
        let (reply, rx) = oneshot::channel();
        self.submit(Control::Complete {
            binding,
            state,
            code,
            deadline,
            reply,
        })?;
        response(rx, deadline).await
    }

    fn submit(&self, control: Control) -> Result<(), Error> {
        self.tx.try_send(control).map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => Error::Busy,
            mpsc::error::TrySendError::Closed(_) => Error::Unavailable,
        })
    }
}

async fn response<T>(
    rx: oneshot::Receiver<Result<T, Error>>,
    deadline: Instant,
) -> Result<T, Error> {
    tokio::time::timeout_at(deadline, rx)
        .await
        .map_err(|_| Error::Expired)?
        .unwrap_or(Err(Error::Unavailable))
}
