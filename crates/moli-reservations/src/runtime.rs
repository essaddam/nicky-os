use crate::{CalendarError, Config, Reservation, Timeline, client::Client, publisher::Publisher};
use moli_runtime::{BoxFuture, Driver, DriverCtx};
use std::time::Duration;

/// The URL is supplied only by the human through this instance's vault.
pub const URL: &str = "url";

#[derive(Debug)]
pub struct Reservations {
    config: Config,
}
impl Reservations {
    pub fn new(config: Config) -> Result<Self, crate::config::ConfigError> {
        config.validate()?;
        Ok(Self { config })
    }
}
impl Driver for Reservations {
    fn kind(&self) -> &'static str {
        "reservations"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            loop {
                if let Some(url) = ctx.secret(URL) {
                    match Client::new(&url) {
                        Ok(client) => return run_with(&self.config, &client, ctx).await,
                        Err(error) => ctx.wait_for(error.to_string()),
                    }
                } else {
                    ctx.wait_for(moli_i18n::tr!("pilotes.reservations.url_manquante"));
                }
                tokio::select! {
                    ()=ctx.cancelled()=>return Ok(()),
                    ()=tokio::time::sleep(Duration::from_secs(5))=>{}
                }
            }
        })
    }
}

struct Snapshot {
    stays: Vec<Reservation>,
    observed: jiff::Timestamp,
    expires: u64,
}
struct Live {
    snapshot: Option<Snapshot>,
    timeline: Timeline,
    publisher: Publisher,
}
impl Live {
    fn new(ctx: &DriverCtx) -> Self {
        Self {
            snapshot: None,
            timeline: Timeline::default(),
            publisher: Publisher::new(ctx),
        }
    }
    fn failed(&mut self, ctx: &DriverCtx, now: jiff::Timestamp, reason: String) {
        self.snapshot = None;
        let _ = self.timeline.update(None, now);
        ctx.wait_for(reason);
        self.publisher.offline(ctx);
    }
    fn accept(
        &mut self,
        ctx: &DriverCtx,
        config: &Config,
        body: &str,
        now: jiff::Timestamp,
    ) -> Result<(), CalendarError> {
        let stays = crate::parse(body, &config.timezone, now)?;
        let windows: Result<Vec<_>, CalendarError> = stays
            .iter()
            .map(|stay| {
                let start =
                    u64::try_from(stay.start.as_millisecond()).map_err(|_| CalendarError)?;
                let end = u64::try_from(stay.end.as_millisecond()).map_err(|_| CalendarError)?;
                Ok((
                    stay.id.clone(),
                    moli_core::AccessWindow::new(start, end).ok_or(CalendarError)?,
                ))
            })
            .collect();
        let checked = u64::try_from(now.as_millisecond()).map_err(|_| CalendarError)?;
        let expires = checked
            .checked_add((config.poll_seconds + 10) * 1000)
            .ok_or(CalendarError)?;
        ctx.ready();
        if !ctx.replace_reservations(windows?, expires) {
            return Err(CalendarError);
        }
        self.snapshot = Some(Snapshot {
            stays,
            observed: now,
            expires,
        });
        self.tick(ctx, now);
        Ok(())
    }
    fn tick(&mut self, ctx: &DriverCtx, now: jiff::Timestamp) {
        if self.snapshot.as_ref().is_some_and(|snapshot| {
            u64::try_from(now.as_millisecond()).map_or(true, |now| {
                now >= snapshot.expires
                    || i64::try_from(now)
                        .map_or(true, |now| now < snapshot.observed.as_millisecond())
            })
        }) {
            self.failed(
                ctx,
                now,
                moli_i18n::tr!("pilotes.reservations.lecture_impossible"),
            );
        }
        if let Some(snapshot) = &self.snapshot {
            let events = self
                .timeline
                .update(Some(&snapshot.stays), now)
                .unwrap_or_default();
            self.publisher.publish(
                ctx,
                &snapshot.stays,
                now,
                snapshot.observed.as_millisecond(),
                &events,
            );
        }
    }
}

async fn run_with(config: &Config, client: &Client, ctx: &mut DriverCtx) -> anyhow::Result<()> {
    let mut live = Live::new(ctx);
    let poll = Duration::from_secs(config.poll_seconds);
    let mut polling = tokio::time::interval_at(tokio::time::Instant::now() + poll, poll);
    polling.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut fetching = true;
    let mut fetch = Box::pin(client.fetch());
    loop {
        tokio::select! {
            biased;
            ()=ctx.cancelled()=>return Ok(()),
            result=&mut fetch,if fetching=>{
                fetching=false;
                let now=jiff::Timestamp::now();
                match result {
                    Ok(body)=>if let Err(error)=live.accept(ctx,config,&body,now) {live.failed(ctx,now,error.to_string());},
                    Err(error)=>live.failed(ctx,now,error.to_string()),
                }
            }
            _=tick.tick()=>live.tick(ctx,jiff::Timestamp::now()),
            _=polling.tick(),if !fetching=>{fetch=Box::pin(client.fetch());fetching=true;}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moli_core::{Event, PointId, Value};
    use moli_runtime::{Hub, HubOptions, spawn_driver};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_util::sync::CancellationToken;

    #[derive(Debug)]
    struct Simulated {
        client: Client,
    }
    impl Driver for Simulated {
        fn kind(&self) -> &'static str {
            "reservations"
        }
        fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
            Box::pin(async move {
                run_with(
                    &Config {
                        timezone: "UTC".into(),
                        poll_seconds: 30,
                    },
                    &self.client,
                    ctx,
                )
                .await
            })
        }
    }

    #[tokio::test]
    async fn startup_reconciles_an_active_stay_without_arrival_or_lock_command() {
        let now = jiff::Timestamp::now();
        let start = now.checked_sub(jiff::Span::new().seconds(60)).unwrap();
        let end = now.checked_add(jiff::Span::new().seconds(60)).unwrap();
        let body = format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:fixture@example.com\r\nSUMMARY:private fixture title\r\nDTSTART:{}\r\nDTEND:{}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
            start.strftime("%Y%m%dT%H%M%SZ"),
            end.strftime("%Y%m%dT%H%M%SZ")
        );
        let id = crate::parse(&body, "UTC", now).unwrap()[0].id.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = Client::local_fixture(listener.local_addr().unwrap().port());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                socket.read_exact(&mut byte).await.unwrap();
                request.push(byte[0]);
            }
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/calendar\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        let hub = Hub::new(HubOptions::default()).unwrap();
        let mut events = hub.subscribe();
        let cancel = CancellationToken::new();
        let task = spawn_driver(
            &hub,
            "fixture".into(),
            Arc::new(Simulated { client }),
            cancel.clone(),
        );
        let cycle = PointId::from("fixture:calendar/state");
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if hub
                    .state(&cycle)
                    .is_some_and(|sample| sample.value == Value::from("checked_in"))
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert!(hub.reservation_allows(&moli_runtime::ReservationRef {
            instance: "fixture".into(),
            id
        }));
        let snapshot = hub.snapshot();
        assert_eq!(snapshot.devices.len(), 6);
        assert!(
            snapshot.devices.iter().all(|device| device
                .device
                .points
                .iter()
                .all(|point| !point.access.write))
        );
        assert!(
            !serde_json::to_string(&snapshot)
                .unwrap()
                .contains("private fixture title")
        );
        assert!(hub.journal(100).is_empty());
        while let Ok(event) = events.try_recv() {
            assert!(!matches!(event, Event::Occurrence { .. }));
        }
        cancel.cancel();
        task.await.unwrap();
        server.await.unwrap();
    }

    #[derive(Debug)]
    struct FailedCalendar {
        body: String,
        ready: Arc<tokio::sync::Notify>,
        fail: Arc<tokio::sync::Notify>,
        done: Arc<tokio::sync::Notify>,
    }
    impl Driver for FailedCalendar {
        fn kind(&self) -> &'static str {
            "reservations"
        }
        fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
            Box::pin(async move {
                let config = Config {
                    timezone: "UTC".into(),
                    poll_seconds: 30,
                };
                let mut live = Live::new(ctx);
                live.accept(ctx, &config, &self.body, jiff::Timestamp::now())
                    .unwrap();
                self.ready.notify_one();
                self.fail.notified().await;
                let now = jiff::Timestamp::now();
                let error = live
                    .accept(ctx, &config, "malformed synthetic calendar", now)
                    .unwrap_err();
                live.failed(ctx, now, error.to_string());
                self.done.notify_one();
                ctx.cancelled().await;
                Ok(())
            })
        }
    }
    #[tokio::test]
    async fn malformed_refresh_closes_admission_and_all_published_devices() {
        let now = jiff::Timestamp::now();
        let start = now.checked_sub(jiff::Span::new().seconds(60)).unwrap();
        let end = now.checked_add(jiff::Span::new().seconds(60)).unwrap();
        let body = format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:fixture@example.com\r\nDTSTART:{}\r\nDTEND:{}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
            start.strftime("%Y%m%dT%H%M%SZ"),
            end.strftime("%Y%m%dT%H%M%SZ")
        );
        let id = crate::parse(&body, "UTC", now).unwrap()[0].id.clone();
        let ready = Arc::new(tokio::sync::Notify::new());
        let fail = Arc::new(tokio::sync::Notify::new());
        let done = Arc::new(tokio::sync::Notify::new());
        let hub = Hub::new(HubOptions::default()).unwrap();
        let cancel = CancellationToken::new();
        let task = spawn_driver(
            &hub,
            "fixture".into(),
            Arc::new(FailedCalendar {
                body,
                ready: ready.clone(),
                fail: fail.clone(),
                done: done.clone(),
            }),
            cancel.clone(),
        );
        tokio::time::timeout(Duration::from_secs(2), ready.notified())
            .await
            .unwrap();
        let binding = moli_runtime::ReservationRef {
            instance: "fixture".into(),
            id,
        };
        assert!(hub.reservation_allows(&binding));
        fail.notify_one();
        tokio::time::timeout(Duration::from_secs(2), done.notified())
            .await
            .unwrap();
        assert!(!hub.reservation_allows(&binding));
        assert!(
            hub.snapshot()
                .devices
                .iter()
                .all(|device| device.online == Some(false))
        );
        cancel.cancel();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn an_instance_without_a_human_url_waits_without_devices_or_commands() {
        let hub = Hub::new(HubOptions::default()).unwrap();
        let mut events = hub.subscribe();
        let cancel = CancellationToken::new();
        let task = spawn_driver(
            &hub,
            "fixture".into(),
            Arc::new(
                Reservations::new(Config {
                    timezone: "UTC".into(),
                    poll_seconds: 30,
                })
                .unwrap(),
            ),
            cancel.clone(),
        );
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if matches!(
                    events.recv().await.unwrap(),
                    Event::DriverStatus {
                        status: moli_core::DriverStatus::Waiting { .. },
                        ..
                    }
                ) {
                    break;
                }
            }
        })
        .await
        .unwrap();
        assert!(hub.snapshot().devices.is_empty());
        assert!(hub.journal(100).is_empty());
        cancel.cancel();
        task.await.unwrap();
    }
}
