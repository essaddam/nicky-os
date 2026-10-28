//! A box in memory, for the driver's own tests: no network, no device.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::time::{Duration, Instant};

use anyhow::bail;
use serde_json::{Value as Json, json};

use crate::api::Refused;
use crate::model::Refusal;

#[derive(Debug, Default)]
pub struct Fake {
    pub setup: Json,
    pub slow_fetch: Option<Duration>,
    pub slow_apply: Option<Duration>,
    pub apply_refusal: Option<Refusal>,
    /// How each execution ends, told by the next fetch (`None`: never).
    pub ends: Option<(&'static str, Option<&'static str>)>,
    /// Answers to the next fetches, before the executions' news.
    pub fetches: Mutex<VecDeque<Result<Json, Refusal>>>,
    pub news: Mutex<Vec<Json>>,
    /// Before each order reaches the box (a slow connection), in turn.
    pub delays: Mutex<VecDeque<Duration>>,
    /// Test-only connection gate, before the HTTP order is handed off.
    pub before_send: Option<std::sync::Arc<tokio::sync::Semaphore>>,
    pub checked_before_send: AtomicUsize,
    /// The orders as they reached the box: command, when.
    pub arrivals: Mutex<Vec<(String, Instant)>>,
    pub applied: AtomicUsize,
    pub fetched: AtomicUsize,
    /// Requests that change the box but may be sent twice.
    pub replayable: AtomicUsize,
}

impl Fake {
    pub async fn call(&self, path: &str, body: Option<&Json>, once: bool) -> anyhow::Result<Json> {
        self.call_guarded(path, body, once, None).await
    }

    pub async fn call_guarded(
        &self,
        path: &str,
        body: Option<&Json>,
        once: bool,
        guard: Option<&moli_net::WebGuard<'_>>,
    ) -> anyhow::Result<Json> {
        if path != "/setup/devices" && !once {
            self.replayable.fetch_add(1, SeqCst);
        }
        match path {
            "/events/register" => Ok(json!({ "id": "listener-1" })),
            "/setup/devices" => Ok(self.setup.clone()),
            "/exec/apply" => {
                let n = self.applied.fetch_add(1, SeqCst) + 1;
                let delay = self.delays.lock().unwrap().pop_front();
                if let Some(delay) = delay {
                    tokio::time::sleep(delay).await;
                }
                if let Some(gate) = &self.before_send {
                    gate.acquire().await?.forget();
                }
                self.checked_before_send.fetch_add(1, SeqCst);
                if guard.is_some_and(|guard| !(guard.begin_send)()) {
                    return Err(crate::api::NotSent.into());
                }
                let command = body.map_or_else(String::new, |b| {
                    b["actions"][0]["commands"][0]["name"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned()
                });
                self.arrivals
                    .lock()
                    .unwrap()
                    .push((command, Instant::now()));
                if let Some(slow) = self.slow_apply {
                    tokio::time::sleep(slow).await;
                }
                if let Some(kind) = self.apply_refusal {
                    return Err(Refused {
                        kind,
                        message: "EXEC_QUEUE_FULL".into(),
                    }
                    .into());
                }
                let id = format!("exec-{n}");
                if let Some((state, cause)) = self.ends {
                    self.news.lock().unwrap().push(json!({
                        "name": "ExecutionStateChangedEvent",
                        "execId": id,
                        "newState": state,
                        "failureType": cause,
                    }));
                }
                Ok(json!({ "execId": id }))
            }
            p if p.ends_with("/fetch") => {
                self.fetched.fetch_add(1, SeqCst);
                if let Some(slow) = self.slow_fetch {
                    tokio::time::sleep(slow).await;
                }
                let next = self.fetches.lock().unwrap().pop_front();
                match next {
                    Some(Ok(answer)) => Ok(answer),
                    Some(Err(kind)) => Err(Refused {
                        kind,
                        message: "refused".into(),
                    }
                    .into()),
                    None => Ok(Json::Array(std::mem::take(&mut *self.news.lock().unwrap()))),
                }
            }
            other => bail!("unexpected request {other}"),
        }
    }
}
