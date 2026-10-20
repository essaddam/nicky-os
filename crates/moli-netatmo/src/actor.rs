//! Single-flight lifecycle, durable credentials and independent freshness.

use std::sync::Arc;
use std::time::Duration;

use moli_net::Body;
use moli_runtime::{BoxFuture, CommandRequest, DriverCtx};
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;

use crate::api::Api;
use crate::gateway::{Authorization, Control, Status};
use crate::oauth::{Pending, TokenSet};
use crate::observed::{Observed, invalidate_all};
use crate::{Error, Netatmo, model};

const TOKENS: &str = "oauth_tokens";

#[derive(Clone, Copy, Debug)]
pub(crate) struct Clock {
    epoch: u64,
    started: Instant,
}

impl Clock {
    pub fn at(epoch: u64) -> Self {
        Self {
            epoch,
            started: Instant::now(),
        }
    }

    pub fn now(self) -> u64 {
        self.epoch.saturating_add(self.started.elapsed().as_secs())
    }
}

#[cfg(test)]
#[derive(Default)]
pub(crate) struct SaveGate {
    /// Signalled after the real store future has been polled once.
    pub entered: std::sync::atomic::AtomicBool,
    pub release: tokio::sync::Notify,
}

enum Answer {
    Weather(Result<Body, Error>),
    Tokens(Result<TokenSet, Error>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Weather,
    Refresh,
    Exchange,
}

struct Job {
    kind: Kind,
    future: BoxFuture<'static, Answer>,
    reply: Option<Arc<oneshot::Sender<Result<(), Error>>>>,
    deadline: Option<Instant>,
    started: bool,
}

impl Job {
    async fn result(&mut self) -> Answer {
        if !self.started
            && (self.reply.as_ref().is_some_and(|reply| reply.is_closed())
                || self.deadline.is_some_and(|at| Instant::now() >= at))
        {
            return Answer::Tokens(Err(Error::Expired));
        }
        self.started = true;
        self.future.as_mut().await
    }
}

enum Wake {
    Command(Option<CommandRequest>),
    Control(Option<Control>),
    Finished(Answer),
    Tick,
}

struct Run<'a> {
    driver: &'a Netatmo,
    ctx: &'a mut DriverCtx,
    controls: &'a mut mpsc::Receiver<Control>,
    api: Option<Arc<Api>>,
    tokens: Option<TokenSet>,
    pending: Option<Pending>,
    observed: Observed,
    job: Option<Job>,
    next: Option<Instant>,
    backoff: u64,
    force_refresh: bool,
    retried_read: bool,
}

impl Drop for Run<'_> {
    fn drop(&mut self) {
        // The supervisor may drop run without polling an async finalizer.
        // Nothing detached can send later; received credentials are never
        // activated before the store succeeded. Atomic storage handles drops.
        invalidate_all(self.ctx);
        self.driver.status.send_replace(Status::Unavailable);
        while let Ok(control) = self.controls.try_recv() {
            control.reject(Error::Unavailable);
        }
    }
}

impl Run<'_> {
    fn load(&mut self) {
        invalidate_all(self.ctx);
        let Ok(callback) = self.driver.config.callback(self.ctx.instance()) else {
            self.state(Status::Unavailable, "configuration");
            return;
        };
        let (Some(id), Some(secret)) = (
            self.ctx.secret("client_id"),
            self.ctx.secret("client_secret"),
        ) else {
            self.state(Status::NeedsCredentials, "credentials");
            return;
        };
        let api = Api::new(id, secret, callback);
        #[cfg(test)]
        let api = {
            let mut api = api;
            api.exchange = self.driver.exchange.clone();
            api
        };
        if api.validate().is_err() {
            self.state(Status::NeedsCredentials, "credentials");
            return;
        }
        self.api = Some(Arc::new(api));
        if !self.ctx.can_store_secrets() {
            self.state(Status::NeedsAuthorization, "storage");
            return;
        }
        self.tokens = self
            .ctx
            .secret(TOKENS)
            .and_then(|text| TokenSet::restore(text.as_bytes(), self.driver.clock.now()).ok());
        if self.tokens.is_some() {
            self.next = Some(Instant::now());
            self.state(Status::Unavailable, "unavailable");
        } else {
            self.state(Status::NeedsAuthorization, "authorization");
        }
    }

    fn state(&self, status: Status, reason: &str) {
        self.driver.status.send_replace(status);
        self.ctx
            .wait_for(moli_i18n::tr!(&format!("netatmo.status.{reason}")));
    }

    fn connected(&self) {
        self.driver.status.send_replace(Status::Connected);
        self.ctx.ready();
    }

    fn launch(&mut self) {
        self.next = None;
        let (Some(api), Some(tokens)) = (self.api.clone(), self.tokens.clone()) else {
            return;
        };
        let clock = self.driver.clock;
        let refresh = self.force_refresh || tokens.expires_at <= clock.now().saturating_add(30);
        self.force_refresh = false;
        let future: BoxFuture<'static, Answer> = if refresh {
            Box::pin(async move { Answer::Tokens(api.refresh(&tokens, clock.now()).await) })
        } else {
            Box::pin(async move { Answer::Weather(api.weather(&tokens.access).await) })
        };
        self.job = Some(Job {
            kind: if refresh {
                Kind::Refresh
            } else {
                Kind::Weather
            },
            future,
            reply: None,
            deadline: None,
            started: false,
        });
    }

    fn retry(&mut self, minimum: u64) {
        let seconds = minimum.max(self.backoff).clamp(60, 900);
        self.backoff = self.backoff.saturating_mul(2).min(900);
        self.next = Some(Instant::now() + Duration::from_secs(seconds));
        self.state(Status::Unavailable, "unavailable");
    }

    fn reauthorize(&mut self, error: Error) {
        self.tokens = None;
        self.next = None;
        invalidate_all(self.ctx);
        self.state(
            Status::NeedsAuthorization,
            if error == Error::Storage {
                "storage"
            } else {
                "authorization"
            },
        );
    }

    fn weather(&mut self, result: Result<Body, Error>) {
        let now = self.driver.clock.now();
        let result =
            result.and_then(|body| model::decode(&body, now, self.driver.config.stale_seconds));
        match result.and_then(|weather| self.observed.apply(self.ctx, weather, now)) {
            Ok(()) => {
                self.backoff = 60;
                self.retried_read = false;
                self.next =
                    Some(Instant::now() + Duration::from_secs(self.driver.config.poll_seconds));
                self.connected();
            }
            Err(Error::Unauthorized) if !self.retried_read && self.tokens.is_some() => {
                self.retried_read = true;
                self.force_refresh = true;
                self.next = Some(Instant::now());
                self.state(Status::Unavailable, "unavailable");
            }
            Err(Error::Unauthorized) => self.reauthorize(Error::Unauthorized),
            Err(Error::RateLimited(seconds)) => self.retry(seconds),
            Err(_) => self.retry(60),
        }
    }

    async fn finished(&mut self, answer: Answer) {
        let Some(job) = self.job.take() else { return };
        let result = match answer {
            Answer::Weather(result) => {
                self.weather(result);
                return;
            }
            Answer::Tokens(result) => result,
        };
        let result = match result {
            Ok(tokens) => {
                // A previous pair may already have been rotated upstream.
                self.tokens = None;
                match persist_while_expiring(self.driver, self.ctx, &tokens, &mut self.observed)
                    .await
                {
                    Ok(()) => {
                        self.tokens = Some(tokens);
                        self.backoff = 60;
                        self.next = Some(Instant::now());
                        self.connected();
                        Ok(())
                    }
                    Err(error) => {
                        self.reauthorize(error);
                        Err(error)
                    }
                }
            }
            Err(error) if job.kind == Kind::Refresh && error == Error::BeforeSend => {
                self.force_refresh = true;
                self.retry(60);
                Err(error)
            }
            Err(Error::RateLimited(seconds)) if job.kind == Kind::Refresh => {
                self.force_refresh = true;
                self.retry(seconds);
                Err(Error::RateLimited(seconds))
            }
            Err(error) => {
                self.reauthorize(error);
                Err(error)
            }
        };
        if let Some(reply) = job.reply {
            let reply = Arc::try_unwrap(reply).expect("actor owns completion sender");
            let _ = reply.send(result);
        }
    }

    fn control(&mut self, control: Control) {
        if control.abandoned() {
            control.reject(Error::Expired);
            return;
        }
        let Some(api) = self.api.clone() else {
            control.reject(Error::Unauthorized);
            return;
        };
        if !self.ctx.can_store_secrets() {
            control.reject(Error::Storage);
            return;
        }
        match control {
            Control::Authorize { binding, reply, .. } => {
                let result =
                    Pending::issue(binding, Instant::now()).and_then(|(pending, state)| {
                        let url = api.authorization_url(&state)?;
                        self.pending = Some(pending);
                        self.driver.status.send_replace(Status::Authorizing);
                        Ok(Authorization {
                            url,
                            expires_in: 600,
                        })
                    });
                let _ = reply.send(result);
            }
            Control::Complete {
                binding,
                state,
                code,
                deadline,
                reply,
            } => {
                if self.job.is_some() {
                    let _ = reply.send(Err(Error::Busy));
                    return;
                }
                let result = self
                    .pending
                    .as_mut()
                    .ok_or(Error::Unauthorized)
                    .and_then(|pending| pending.consume(&state, binding, Instant::now()));
                if let Err(error) = result {
                    let _ = reply.send(Err(error));
                    return;
                }
                self.pending = None;
                self.next = None;
                let clock = self.driver.clock;
                let reply = Arc::new(reply);
                let waiting = Arc::downgrade(&reply);
                let allowed = Arc::new(move || {
                    Instant::now() < deadline
                        && waiting.upgrade().is_some_and(|reply| !reply.is_closed())
                });
                self.job = Some(Job {
                    kind: Kind::Exchange,
                    future: Box::pin(async move {
                        Answer::Tokens(api.exchange_code(&code, clock.now(), allowed).await)
                    }),
                    reply: Some(reply),
                    deadline: Some(deadline),
                    started: false,
                });
            }
        }
    }

    fn deadline(&self) -> Instant {
        let now = Instant::now();
        let expiry = self
            .observed
            .next_expiry()
            .map(|at| now + Duration::from_secs(at.saturating_sub(self.driver.clock.now())));
        self.next
            .into_iter()
            .chain(expiry)
            .min()
            .unwrap_or_else(|| now + Duration::from_secs(86400))
    }

    async fn serve(&mut self) {
        self.load();
        loop {
            self.observed.expire(self.ctx, self.driver.clock.now());
            if self.job.is_none() && self.next.is_some_and(|at| at <= Instant::now()) {
                self.launch();
            }
            let deadline = self.deadline();
            let has_job = self.job.is_some();
            let job = &mut self.job;
            let wake = tokio::select! {
                command = self.ctx.next_command() => Wake::Command(command),
                control = self.controls.recv() => Wake::Control(control),
                answer = async { job.as_mut().expect("active job").result().await }, if has_job => Wake::Finished(answer),
                () = tokio::time::sleep_until(deadline) => Wake::Tick,
            };
            match wake {
                Wake::Command(None) | Wake::Control(None) => return,
                Wake::Command(Some(command)) => {
                    command.reply(Err(moli_i18n::tr!("netatmo.read_only")));
                }
                Wake::Control(Some(control)) => self.control(control),
                Wake::Finished(answer) => self.finished(answer).await,
                Wake::Tick => {}
            }
        }
    }
}

async fn persist_while_expiring(
    driver: &Netatmo,
    ctx: &DriverCtx,
    tokens: &TokenSet,
    observed: &mut Observed,
) -> Result<(), Error> {
    let save = persist(driver, ctx, tokens);
    tokio::pin!(save);
    loop {
        observed.expire(ctx, driver.clock.now());
        let Some(expiry) = observed.next_expiry() else {
            return save.await;
        };
        let deadline =
            Instant::now() + Duration::from_secs(expiry.saturating_sub(driver.clock.now()));
        tokio::select! {
            result = &mut save => {
                observed.expire(ctx, driver.clock.now());
                return result;
            }
            () = tokio::time::sleep_until(deadline) => {}
        }
    }
}

async fn persist(driver: &Netatmo, ctx: &DriverCtx, tokens: &TokenSet) -> Result<(), Error> {
    let text = tokens.persisted()?;
    let store = ctx.store_secret(TOKENS, &text);
    tokio::pin!(store);
    #[cfg(test)]
    if let Some(gate) = &driver.persist_gate {
        use std::future::{Future as _, poll_fn};
        use std::task::Poll;
        let completed = poll_fn(|cx| {
            Poll::Ready(match store.as_mut().poll(cx) {
                Poll::Ready(result) => Some(result),
                Poll::Pending => None,
            })
        })
        .await;
        gate.entered
            .store(true, std::sync::atomic::Ordering::SeqCst);
        gate.release.notified().await;
        if let Some(result) = completed {
            return result.map_err(|_| Error::Storage);
        }
    }
    #[cfg(not(test))]
    let _ = driver;
    store.await.map_err(|_| Error::Storage)
}

pub(crate) async fn run(driver: &Netatmo, ctx: &mut DriverCtx) -> anyhow::Result<()> {
    let mut controls = driver.controls.lock().await;
    Run {
        driver,
        ctx,
        controls: &mut controls,
        api: None,
        tokens: None,
        pending: None,
        observed: Observed::default(),
        job: None,
        next: None,
        backoff: 60,
        force_refresh: false,
        retried_read: false,
    }
    .serve()
    .await;
    Ok(())
}
