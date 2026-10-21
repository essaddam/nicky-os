//! When automations start: the hub's changes, a clock that ticks every
//! second, and the moment Moli starts. Only approved, switched-on
//! automations are considered.

use std::time::{Duration, Instant};

use moli_core::{Event, Notice, Occurrence, PointId, Value};
use moli_i18n::tr;
use tokio::sync::broadcast::error::RecvError;

use crate::model::{Automation, Mode, Step, SunEvent, minutes_of};
use crate::run::{canonical_expected, day_ok, execute, same};
use crate::{Automations, clock, decimal, lock, yes_no};

/// Runs per minute beyond which an automation is switched off.
const RUNAWAY: usize = 20;
/// Runs waiting their turn, per queued automation.
const MAX_QUEUE: usize = 10;
/// Let the drivers connect before « when Moli starts ».
const START_DELAY: Duration = Duration::from_secs(15);

#[cfg(test)]
#[path = "engine_timer_tests.rs"]
mod timer_tests;

pub(crate) async fn run(autos: Automations) {
    let mut events = autos.0.hub.subscribe();
    let has_occurrences = !autos
        .0
        .occurrence_index
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .is_empty();
    if has_occurrences {
        let stream = autos.0.hub.occurrence_stream();
        match autos
            .0
            .occurrence_consumer
            .lock()
            .await
            .recover(stream.as_deref().unwrap_or(""))
            .await
        {
            Ok(interrupted) => {
                for event in interrupted {
                    autos
                        .0
                        .hub
                        .occurrence_problem(&event, tr!("moteur.occurrence.interruption"));
                }
            }
            Err(error) => {
                tracing::warn!(%error, "occurrence recovery failed");
                autos.0.hub.notice(Notice {
                    title: None,
                    message: tr!("moteur.occurrence.persistance"),
                    from: None,
                    ts: moli_core::now_ms(),
                });
            }
        }
    }
    remember_all(&autos);
    mark_this_minute(&autos);
    autos.0.hub.timer_consumer_ready(true);
    let mut clock = tokio::time::interval(Duration::from_secs(1));
    clock.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let started = tokio::time::sleep(START_DELAY);
    tokio::pin!(started);
    let mut start_done = false;
    let cancel = autos.0.cancel.clone();
    loop {
        tokio::select! {
            () = cancel.cancelled() => break,
            event = events.recv() => match event {
                Ok(Event::State { point, value, .. }) => on_state(&autos, &point, &value),
                Ok(Event::Occurrence { occurrence }) => on_occurrence(&autos, occurrence).await,
                Ok(Event::DeviceUpserted { device }) if device.points.iter().any(|point| matches!(point.kind,moli_core::Kind::Timer{field:moli_core::TimerField::Expired})) => autos.refresh_live(),
                Ok(_) => {}
                Err(RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "automations lagged behind the hub: some changes were not seen");
                    remember_all(&autos);
                    if !autos.0.occurrence_index.read().unwrap_or_else(std::sync::PoisonError::into_inner).is_empty() {
                        autos.0.hub.notice(Notice { title: None,
                            message: tr!("moteur.occurrence.bus_perdu"), from: None, ts: moli_core::now_ms() });
                    }
                }
                Err(RecvError::Closed) => break,
            },
            _ = clock.tick() => on_clock(&autos),
            () = &mut started, if !start_done => {
                start_done = true;
                for a in live(&autos).iter() {
                    for t in a.graph.triggers().filter(|t| matches!(t.step, Step::OnStart)) {
                        fire(&autos, a, &t.id, tr!("moteur.pourquoi.demarrage"));
                    }
                }
            }
        }
    }
    // Shutdown: running automations stop here, and get a moment to record
    autos.0.hub.timer_consumer_ready(false);
    // themselves (« interrompu ») before the process ends: a light switched
    // on for five minutes then left on must show in the history.
    autos
        .0
        .shutting_down
        .store(true, std::sync::atomic::Ordering::Relaxed);
    for (_, all) in lock(&autos.0.stops).drain() {
        all.cancel();
    }
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    while !lock(&autos.0.active).is_empty() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

async fn on_occurrence(autos: &Automations, event: Occurrence) {
    if !event.id.is_valid()
        || event.position == 0
        || event.position > moli_core::MAX_OCCURRENCE_SEQUENCE
        || autos.0.hub.occurrence_stream().as_deref() != Some(event.stream.as_ref())
    {
        return;
    }
    let generation = match autos.0.hub.timer_delivery(&event).await {
        Ok(generation) => generation,
        Err(moli_runtime::TimerError::Stale) => return,
        Err(error) => {
            autos.0.hub.occurrence_problem(&event, error.to_string());
            return;
        }
    };
    let recipients = if generation.is_some() {
        match autos.0.hub.timer_recipients(&event).await {
            Ok(recipients) => Some(recipients),
            Err(_) => return,
        }
    } else {
        None
    };
    let targets = eligible_targets(autos, &event, recipients.as_deref());
    if targets.is_empty() && generation.is_none() {
        return;
    }
    let admitted = async {
        let mut consumer = autos.0.occurrence_consumer.lock().await;
        for interrupted in consumer.recover(&event.stream).await? {
            autos
                .0
                .hub
                .occurrence_problem(&interrupted, tr!("moteur.occurrence.interruption"));
        }
        let targets = targets
            .iter()
            .map(|(a, trigger, _)| (a.id.clone(), trigger.clone()))
            .collect();
        if generation.is_some() {
            consumer.claim_timer(&event, targets).await
        } else {
            consumer.claim(&event, targets).await
        }
    }
    .await;
    match admitted {
        Ok(true) => {}
        Ok(false) => {
            if generation.is_some() {
                ack_timer(autos, &event).await;
            }
            return;
        }
        Err(error) => {
            tracing::warn!(%error, "occurrence admission failed");
            autos
                .0
                .hub
                .occurrence_problem(&event, tr!("moteur.occurrence.persistance"));
            return;
        }
    }
    let skipped = if generation.is_some() {
        match autos
            .0
            .hub
            .with_timer_generation(&event, |token| {
                launch_occurrences(autos, &event, &targets, Some(&token))
            })
            .await
        {
            Ok(skipped) => skipped,
            Err(error) => {
                if error != moli_runtime::TimerError::Stale {
                    autos.0.hub.occurrence_problem(&event, error.to_string());
                }
                targets
                    .iter()
                    .map(|(a, trigger, _)| (a.id.clone(), trigger.clone()))
                    .collect()
            }
        }
    } else {
        launch_occurrences(autos, &event, &targets, None)
    };
    for (automation, trigger) in skipped {
        finish_occurrence(autos, &event, &automation, &trigger).await;
    }

    if generation.is_some() {
        ack_timer(autos, &event).await;
    }
}

fn eligible_targets(
    autos: &Automations,
    event: &Occurrence,
    recipients: Option<&[moli_core::TimerRecipient]>,
) -> Vec<(Automation, String, tokio_util::sync::CancellationToken)> {
    let index = autos
        .0
        .occurrence_index
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // Children come from the indexed activation, never from a newly looked-up
    // parent after an off/on transition or a persistence wait.
    index
        .targets(event)
        .into_iter()
        .filter(|(automation, trigger, _)| {
            recipients.is_none_or(|recipients| {
                recipients.iter().any(|recipient| {
                    recipient.automation == automation.id
                        && recipient.trigger == *trigger
                        && automation.activation.as_ref() == Some(&recipient.activation)
                })
            })
        })
        .collect::<Vec<_>>()
}

fn launch_occurrences(
    autos: &Automations,
    event: &Occurrence,
    targets: &[(Automation, String, tokio_util::sync::CancellationToken)],
    generation: Option<&tokio_util::sync::CancellationToken>,
) -> Vec<(String, String)> {
    let mut skipped = Vec::new();
    for (a, trigger, token) in targets {
        let why = tr!("moteur.pourquoi.occurrence", nom = event.point);
        if token.is_cancelled()
            || generation.is_some_and(tokio_util::sync::CancellationToken::is_cancelled)
            || runaway(autos, a)
            || autos
                .start_occurrence_run(a, trigger, why, event, token.clone(), generation.cloned())
                .id
                == 0
        {
            skipped.push((a.id.clone(), trigger.clone()));
        }
    }
    skipped
}

async fn ack_timer(autos: &Automations, event: &Occurrence) {
    if let Err(error) = autos.0.hub.ack_timer(event).await {
        autos.0.hub.occurrence_problem(event, error.to_string());
    }
}

async fn finish_occurrence(
    autos: &Automations,
    event: &Occurrence,
    automation: &str,
    trigger: &str,
) {
    if let Err(error) = autos
        .0
        .occurrence_consumer
        .lock()
        .await
        .complete(&event.id, automation, trigger)
        .await
    {
        tracing::warn!(%error, "occurrence completion was not durable");
        autos
            .0
            .hub
            .occurrence_problem(event, tr!("moteur.occurrence.persistance"));
    }
}

/// Clock triggers due this very minute count as fired: after a restart
/// within the minute, they do not fire twice.
fn mark_this_minute(autos: &Automations) {
    let now = jiff::Timestamp::now().to_zoned(autos.0.tz.clone());
    let minute = u32::from(now.hour().unsigned_abs()) * 60 + u32::from(now.minute().unsigned_abs());
    let stamp = format!("{} {minute}", now.date());
    let mut fired = lock(&autos.0.fired);
    for a in live(autos).iter() {
        for t in a.graph.triggers() {
            if matches!(
                t.step,
                Step::AtTime { .. } | Step::AtSun { .. } | Step::Every { .. }
            ) {
                fired.insert((a.id.clone(), t.id.clone()), stamp.clone());
            }
        }
    }
}

fn live(autos: &Automations) -> std::sync::Arc<Vec<Automation>> {
    autos.live()
}

/// Current values, so the first change seen has a « before ».
fn remember_all(autos: &Automations) {
    let snapshot = autos.0.hub.snapshot();
    let mut last = lock(&autos.0.last);
    for d in snapshot.devices {
        for (key, sample) in d.state {
            last.insert(format!("{}/{key}", d.device.id), sample.value);
        }
    }
}

fn on_state(autos: &Automations, point: &PointId, value: &Value) {
    let key = point.to_string();
    let prev = lock(&autos.0.last).insert(key.clone(), value.clone());
    if prev.as_ref() == Some(value) {
        return;
    }
    for a in live(autos).iter() {
        for t in a.graph.triggers() {
            match &t.step {
                Step::WhenState {
                    point: p,
                    to,
                    from,
                    for_s,
                } if *p == key => {
                    let to_ok = to.as_ref().is_none_or(|to| {
                        canonical_expected(autos, &key, to).is_some_and(|to| same(value, &to))
                    });
                    let from_ok = from.as_ref().is_none_or(|from| {
                        prev.as_ref().is_some_and(|p| {
                            canonical_expected(autos, &key, from).is_some_and(|from| same(p, &from))
                        })
                    });
                    // A point seen for the first time is not a change.
                    if prev.is_none() {
                        continue;
                    }
                    if to_ok && from_ok {
                        let why = format!("{} → {}", label(autos, &key), words(value));
                        if *for_s == 0 {
                            fire(autos, a, &t.id, why);
                        } else {
                            if to.is_none() {
                                unhold(autos, &a.id, &t.id);
                            }
                            hold(
                                autos,
                                a,
                                &t.id,
                                *for_s,
                                why,
                                key.clone(),
                                to.clone().or(Some(value.to_json())),
                            );
                        }
                    } else {
                        unhold(autos, &a.id, &t.id);
                    }
                }
                Step::WhenThreshold {
                    point: p,
                    above,
                    below,
                    for_s,
                } if *p == key => {
                    let inside = |v: &Value| {
                        v.as_f64().is_some_and(|x| {
                            above.is_none_or(|a| x > a) && below.is_none_or(|b| x < b)
                        })
                    };
                    let now_in = inside(value);
                    let was_in = prev.as_ref().map(inside);
                    if now_in && was_in == Some(false) {
                        let why = format!("{} → {}", label(autos, &key), words(value));
                        if *for_s == 0 {
                            fire(autos, a, &t.id, why);
                        } else {
                            let (above, below) = (*above, *below);
                            hold_until(autos, a, &t.id, *for_s, why, key.clone(), move |v| {
                                v.as_f64().is_some_and(|x| {
                                    above.is_none_or(|a| x > a) && below.is_none_or(|b| x < b)
                                })
                            });
                        }
                    } else if !now_in {
                        unhold(autos, &a.id, &t.id);
                    }
                }
                _ => {}
            }
        }
    }
}

/// Fires after `for_s` if the point still has the value by then.
#[allow(clippy::too_many_arguments)]
fn hold(
    autos: &Automations,
    a: &Automation,
    trigger: &str,
    for_s: u64,
    why: String,
    point: String,
    to: Option<serde_json::Value>,
) {
    let to = match to {
        Some(value) => match canonical_expected(autos, &point, &value) {
            Some(value) => Some(value),
            None => return,
        },
        None => None,
    };
    hold_until(autos, a, trigger, for_s, why, point, move |v| {
        to.as_ref().is_none_or(|to| same(v, to))
    });
}

fn hold_until(
    autos: &Automations,
    a: &Automation,
    trigger: &str,
    for_s: u64,
    why: String,
    point: String,
    still: impl Fn(&Value) -> bool + Send + 'static,
) {
    let key = (a.id.clone(), trigger.to_owned());
    // Already counting: the value did not leave, the clock keeps running.
    if lock(&autos.0.holds)
        .get(&key)
        .is_some_and(|h| !h.is_finished())
    {
        return;
    }
    let (autos2, a_id, trigger2) = (autos.clone(), a.id.clone(), trigger.to_owned());
    let task = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(for_s)).await;
        let now = autos2
            .0
            .hub
            .state(&PointId::from(point.as_str()))
            .map(|s| s.value);
        lock(&autos2.0.holds).remove(&(a_id.clone(), trigger2.clone()));
        if now.as_ref().is_some_and(&still)
            && let Some(a) = autos2.get(&a_id).filter(Automation::is_live)
        {
            fire(&autos2, &a, &trigger2, why);
        }
    });
    lock(&autos.0.holds).insert(key, task.abort_handle());
}

fn unhold(autos: &Automations, automation: &str, trigger: &str) {
    if let Some(h) = lock(&autos.0.holds).remove(&(automation.to_owned(), trigger.to_owned())) {
        h.abort();
    }
}

fn on_clock(autos: &Automations) {
    let now = jiff::Timestamp::now().to_zoned(autos.0.tz.clone());
    let minute = u32::from(now.hour().unsigned_abs()) * 60 + u32::from(now.minute().unsigned_abs());
    let stamp = format!("{} {minute}", now.date());
    for a in live(autos).iter() {
        for t in a.graph.triggers() {
            let due = match &t.step {
                Step::AtTime { at, days } => day_ok(days, &now) && minutes_of(at) == Some(minute),
                Step::AtSun {
                    event,
                    offset_min,
                    days,
                } => {
                    day_ok(days, &now)
                        && sun_minute(autos, &now, *event, *offset_min) == Some(minute)
                }
                Step::Every { minutes } => *minutes > 0 && minute % minutes == 0,
                _ => false,
            };
            if !due {
                continue;
            }
            let key = (a.id.clone(), t.id.clone());
            let first = lock(&autos.0.fired).insert(key, stamp.clone()).as_deref() != Some(&*stamp);
            if first {
                fire(
                    autos,
                    a,
                    &t.id,
                    tr!(
                        "moteur.pourquoi.heure",
                        heure = clock(now.hour(), now.minute())
                    ),
                );
            }
        }
    }
}

/// Today's sunrise or sunset (+ offset), in local minutes.
fn sun_minute(
    autos: &Automations,
    now: &jiff::Zoned,
    event: SunEvent,
    offset_min: i32,
) -> Option<u32> {
    let (lat, lon) = (autos.0.config.latitude?, autos.0.config.longitude?);
    let (rise, set) = crate::sun::times(now.date(), lat, lon)?;
    let at = if event == SunEvent::Rise { rise } else { set };
    let local = at.to_zoned(autos.0.tz.clone());
    let m = i32::from(local.hour()) * 60 + i32::from(local.minute()) + offset_min;
    u32::try_from(m.rem_euclid(24 * 60)).ok()
}

/// Starts a run, by the automation's mode, unless it is running away.
pub(crate) fn fire(autos: &Automations, a: &Automation, trigger: &str, why: String) {
    if runaway(autos, a) {
        return;
    }
    autos.start_run(a, trigger, why, false);
}

fn runaway(autos: &Automations, a: &Automation) -> bool {
    let now = Instant::now();
    let too_many = {
        let mut rate = lock(&autos.0.rate);
        let starts = rate.entry(a.id.clone()).or_default();
        while starts
            .front()
            .is_some_and(|t| now.duration_since(*t) > Duration::from_secs(60))
        {
            starts.pop_front();
        }
        starts.push_back(now);
        starts.len() > RUNAWAY
    };
    if too_many {
        tracing::warn!(automation = a.id, "automation running away: switched off");
        let autos2 = autos.clone();
        let (id, name) = (a.id.clone(), a.name.clone());
        tokio::spawn(async move {
            let _ = autos2.update(&id, |a| {
                a.enabled = false;
                a.note = Some(tr!("moteur.notice.boucle_note", max = RUNAWAY));
            });
            if let Err(e) = autos2.persist().await {
                tracing::warn!(error = %e, "could not save the switched-off automation");
            }
            autos2.changed(&id);
            autos2.0.hub.notice(Notice {
                title: Some(tr!("moteur.notice.boucle_titre")),
                message: tr!("moteur.notice.boucle", nom = name),
                from: None,
                ts: moli_core::now_ms(),
            });
        });
    }
    too_many
}

impl Automations {
    /// Spawns a run by the automation's mode; returns the run as it starts
    /// (its id is final).
    pub(crate) fn start_run(
        &self,
        a: &Automation,
        trigger: &str,
        why: String,
        manual: bool,
    ) -> crate::Run {
        self.start_context_run(a, trigger, why, manual, None, None)
    }

    fn start_occurrence_run(
        &self,
        a: &Automation,
        trigger: &str,
        why: String,
        event: &Occurrence,
        token: tokio_util::sync::CancellationToken,
        generation: Option<tokio_util::sync::CancellationToken>,
    ) -> crate::Run {
        self.start_context_run(
            a,
            trigger,
            why,
            false,
            Some(event),
            Some(crate::run::RunCancellation {
                activation: token,
                generation,
            }),
        )
    }

    fn start_context_run(
        &self,
        a: &Automation,
        trigger: &str,
        why: String,
        manual: bool,
        event: Option<&Occurrence>,
        token: Option<crate::run::RunCancellation>,
    ) -> crate::Run {
        let skipped = |why: String| skipped_run(a, trigger, why, event);
        // Definition -> parents -> active/queues is the mutation lock order.
        // A definition cannot be replaced between admission and registration.
        let definitions = self
            .0
            .list
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if definitions
            .iter()
            .find(|current| current.id == a.id)
            .is_none_or(|current| !current.is_live() || current.fingerprint() != a.fingerprint())
        {
            return skipped(why);
        }
        // Every run's token is a child of the automation's: switching it off
        // cancels them all, the waiting ones too. Keep mode/queue registration
        // atomic with respect to invalidating this activation's parent.
        let mut parents = lock(&self.0.stops);
        let token = token.unwrap_or_else(|| {
            parents
                .entry(a.id.clone())
                .or_default()
                .child_token()
                .into()
        });
        if token.is_cancelled() {
            return skipped(why);
        }
        match a.mode {
            Mode::Restart => {
                if let Some((_, old)) = lock(&self.0.active).remove(&a.id) {
                    old.cancel();
                }
            }
            Mode::Single => {
                let busy = lock(&self.0.active)
                    .get(&a.id)
                    .is_some_and(|(_, t)| !t.is_cancelled());
                if busy && !manual {
                    tracing::debug!(automation = a.id, "already running: trigger ignored");
                    return skipped(why);
                }
            }
            Mode::Queued => {
                let mut waiting = lock(&self.0.waiting);
                let n = waiting.entry(a.id.clone()).or_default();
                if *n >= MAX_QUEUE {
                    tracing::warn!(automation = a.id, "queue full: trigger ignored");
                    return skipped(why);
                }
                *n += 1;
            }
        }
        let id = self.next_id();
        lock(&self.0.active).insert(a.id.clone(), (id, token.activation.clone()));
        let queue = (a.mode == Mode::Queued).then(|| {
            lock(&self.0.queues)
                .entry(a.id.clone())
                .or_default()
                .clone()
        });
        drop(parents);
        drop(definitions);
        let (this, a2, trigger2, why2) = (self.clone(), a.clone(), trigger.to_owned(), why.clone());
        let event2 = event.cloned();
        tokio::spawn(async move {
            let turn = match &queue {
                Some(q) => {
                    let turn = q.lock().await;
                    if let Some(n) = lock(&this.0.waiting).get_mut(&a2.id) {
                        *n = n.saturating_sub(1);
                    }
                    Some(turn)
                }
                None => None,
            };
            // After waiting its turn, it runs only if that same version is
            // still approved and on.
            let current = this.get(&a2.id);
            let still = current.is_some_and(|c| c.is_live() && c.fingerprint() == a2.fingerprint());
            if still && !token.is_cancelled() {
                execute(
                    &this,
                    &a2,
                    &trigger2,
                    why2,
                    false,
                    token,
                    id,
                    event2.as_ref(),
                )
                .await;
            }
            if let Some(event) = &event2 {
                finish_occurrence(&this, event, &a2.id, &trigger2).await;
            }
            drop(turn);
            // Only clear our own entry: a newer run may have replaced it.
            let mut active = lock(&this.0.active);
            if active.get(&a2.id).is_some_and(|(rid, _)| *rid == id) {
                active.remove(&a2.id);
            }
        });
        crate::Run::new(a, trigger, why, id, false, event)
    }
}

fn skipped_run(
    a: &Automation,
    trigger: &str,
    why: String,
    event: Option<&Occurrence>,
) -> crate::Run {
    let mut run = crate::Run::new(a, trigger, why, 0, false, event);
    run.ended = Some(moli_core::now_ms());
    run.status = crate::RunStatus::Cancelled;
    run
}

fn label(autos: &Automations, point: &str) -> String {
    PointId::from(point)
        .split()
        .and_then(|(d, _)| autos.0.hub.device(&d))
        .map_or_else(
            || point.to_owned(),
            |v| {
                v.label
                    .name
                    .clone()
                    .unwrap_or_else(|| v.device.native_name.to_string())
            },
        )
}

fn words(v: &Value) -> String {
    match v {
        Value::Bool(b) => yes_no(*b),
        Value::Float(f) => decimal(&format!("{}", (f * 10.0).round() / 10.0)),
        Value::Int(i) => i.to_string(),
        Value::Text(t) => t.to_string(),
        Value::Null => "—".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future as _;
    use std::sync::Arc;
    use std::task::Poll;

    #[derive(Debug)]
    struct Fixture {
        hub: moli_runtime::Hub,
        autos: Automations,
        active: Automation,
        cancel: tokio_util::sync::CancellationToken,
    }

    const HUMAN: crate::Actor = crate::Actor {
        human: true,
        author: crate::Author::Human,
    };

    async fn fixture() -> Fixture {
        let hub = moli_runtime::Hub::new(moli_runtime::HubOptions::default()).unwrap();
        let cancel = tokio_util::sync::CancellationToken::new();
        let config = serde_json::from_value(serde_json::json!({ "helper": [
            { "id": "button", "name": "Simulated button", "kind": "button" },
            { "id": "lamp", "name": "Simulated lamp", "kind": "switch" }
        ] }))
        .unwrap();
        moli_runtime::spawn_driver(
            &hub,
            "fixture".into(),
            Arc::new(moli_helpers::Helpers::new(config).unwrap()),
            cancel.clone(),
        );
        let lamp = PointId::from("fixture:lamp/on");
        for _ in 0..200 {
            if hub.state(&lamp).is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(hub.state(&lamp).unwrap().value, Value::Bool(false));
        let (autos, engine) =
            Automations::start(&hub, crate::Config::default(), None, cancel.clone())
                .await
                .unwrap();
        // Drive the real handler ourselves, so its admission wait is deterministic.
        engine.abort();
        let _ = engine.await;
        let human = crate::Actor {
            human: true,
            author: crate::Author::Human,
        };
        let graph = serde_json::from_value(serde_json::json!({ "nodes": [
            { "id": "press", "type": "when_occurrence", "point": "fixture:button/press" },
            { "id": "set", "type": "set", "point": "fixture:lamp/on", "value": true }
        ], "edges": [{ "from": "press", "to": "set" }] }))
        .unwrap();
        let draft = Automation {
            id: String::new(),
            name: "Simulated".into(),
            enabled: false,
            mode: Mode::Queued,
            graph,
            author: crate::Author::Human,
            approved: None,
            approved_version: None,
            activation: None,
            note: None,
            created: 0,
            updated: 0,
        };
        let draft = autos.save(draft, human).await.unwrap();
        let active = autos
            .approve(&draft.id, &draft.fingerprint(), human)
            .await
            .unwrap();
        Fixture {
            hub,
            autos,
            active,
            cancel,
        }
    }

    async fn press(fixture: &Fixture) -> Occurrence {
        let mut events = fixture.hub.subscribe();
        fixture
            .hub
            .command(
                &PointId::from("fixture:button/press"),
                Value::Null,
                moli_core::Origin::Ui,
                None,
            )
            .await
            .unwrap();
        loop {
            if let Event::Occurrence { occurrence } = events.try_recv().unwrap() {
                return occurrence;
            }
        }
    }

    async fn assert_fresh_run(fixture: &Fixture, event: Occurrence) {
        let identity = event.id.clone();
        on_occurrence(&fixture.autos, event).await;
        for _ in 0..200 {
            if fixture
                .autos
                .runs(Some(&fixture.active.id), 10)
                .iter()
                .any(|run| run.status == crate::RunStatus::Done)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let runs = fixture.autos.runs(Some(&fixture.active.id), 10);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, crate::RunStatus::Done);
        assert_eq!(runs[0].occurrence.as_ref(), Some(&identity));
        assert_eq!(
            fixture
                .hub
                .state(&PointId::from("fixture:lamp/on"))
                .unwrap()
                .value,
            Value::Bool(true)
        );
    }

    #[tokio::test]
    async fn auto_h4_admission_wait_cannot_cross_off_on_of_the_same_graph() {
        let Fixture {
            hub,
            autos,
            active,
            cancel,
        } = fixture().await;
        let human = HUMAN;
        let lamp = PointId::from("fixture:lamp/on");
        let mut events = hub.subscribe();
        hub.command(
            &PointId::from("fixture:button/press"),
            Value::Null,
            moli_core::Origin::Ui,
            None,
        )
        .await
        .unwrap();
        let event = loop {
            if let Event::Occurrence { occurrence } = events.try_recv().unwrap() {
                break occurrence;
            }
        };
        let writer = autos.0.occurrence_consumer.lock().await;
        let handling = on_occurrence(&autos, event);
        tokio::pin!(handling);
        // This first poll copies the old targets then waits for the held writer.
        std::future::poll_fn(|cx| {
            assert!(handling.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        autos
            .set_enabled(&active.id, false, None, human)
            .await
            .unwrap();
        let reactivated = autos
            .approve(&active.id, &active.fingerprint(), human)
            .await
            .unwrap();
        assert_eq!(reactivated.fingerprint(), active.fingerprint());
        drop(writer);
        handling.await;
        assert!(
            lock(&autos.0.active).is_empty(),
            "old admission cannot create a run in the new activation"
        );
        assert!(autos.runs(Some(&active.id), 10).is_empty());
        assert_eq!(hub.state(&lamp).unwrap().value, Value::Bool(false));
        // A fresh activation must still work, including an explicit restore
        // of the identical validated content (do not cancel its new parent).
        let restored = autos.restore(&active.id, human).await.unwrap();
        assert!(restored.is_live());
        hub.command(
            &PointId::from("fixture:button/press"),
            Value::Null,
            moli_core::Origin::Ui,
            None,
        )
        .await
        .unwrap();
        let fresh = loop {
            if let Event::Occurrence { occurrence } = events.try_recv().unwrap() {
                break occurrence;
            }
        };
        let fresh_id = fresh.id.clone();
        on_occurrence(&autos, fresh).await;
        for _ in 0..200 {
            if autos
                .runs(Some(&active.id), 10)
                .iter()
                .any(|run| run.status == crate::RunStatus::Done)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let runs = autos.runs(Some(&active.id), 10);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, crate::RunStatus::Done);
        assert_eq!(runs[0].occurrence.as_ref(), Some(&fresh_id));
        assert_eq!(hub.state(&lamp).unwrap().value, Value::Bool(true));
        cancel.cancel();
    }

    #[tokio::test]
    async fn auto_h4_refresh_snapshot_cannot_outlive_its_definition_lock() {
        let fixture = fixture().await;
        let (captured, snapshot) = std::sync::mpsc::channel();
        let (release, resume) = std::sync::mpsc::channel();
        let autos = fixture.autos.clone();
        let refresher = std::thread::spawn(move || {
            autos.refresh_live_after_snapshot(|| {
                captured.send(()).unwrap();
                resume.recv().unwrap();
            });
        });
        snapshot.recv_timeout(Duration::from_secs(2)).unwrap();
        // Deterministic probe of the exact lock every off/on mutation requires.
        // A copied Vec with its lock already dropped fails this assertion.
        let writer_excluded = fixture.autos.0.list.try_write().is_err();
        release.send(()).unwrap();
        refresher.join().unwrap();
        assert!(
            writer_excluded,
            "definition cannot turn off between snapshot and activation publication"
        );

        fixture
            .autos
            .set_enabled(&fixture.active.id, false, None, HUMAN)
            .await
            .unwrap();
        let off_event = press(&fixture).await;
        fixture
            .autos
            .approve(&fixture.active.id, &fixture.active.fingerprint(), HUMAN)
            .await
            .unwrap();
        on_occurrence(&fixture.autos, off_event).await;
        assert!(fixture.autos.runs(Some(&fixture.active.id), 10).is_empty());
        assert!(lock(&fixture.autos.0.active).is_empty());
        assert_eq!(
            fixture
                .hub
                .state(&PointId::from("fixture:lamp/on"))
                .unwrap()
                .value,
            Value::Bool(false)
        );
        let fresh = press(&fixture).await;
        assert_fresh_run(&fixture, fresh).await;
        fixture.cancel.cancel();
    }

    #[tokio::test]
    async fn auto_h4_completed_off_cannot_cancel_a_newer_activation() {
        let fixture = fixture().await;
        let (updated, off_complete) = std::sync::mpsc::channel();
        let (release, resume) = std::sync::mpsc::channel();
        let autos = fixture.autos.clone();
        let id = fixture.active.id.clone();
        let handle = tokio::runtime::Handle::current();
        let old_off = std::thread::spawn(move || {
            handle
                .block_on(
                    autos.set_enabled_after_update(&id, false, None, HUMAN, move || {
                        updated.send(()).unwrap();
                        resume.recv().unwrap();
                    }),
                )
                .unwrap();
        });
        off_complete.recv_timeout(Duration::from_secs(2)).unwrap();
        fixture
            .autos
            .approve(&fixture.active.id, &fixture.active.fingerprint(), HUMAN)
            .await
            .unwrap();
        // This press belongs to the new activation while the old request has
        // not yet returned from its post-update barrier.
        let fresh = press(&fixture).await;
        release.send(()).unwrap();
        old_off.join().unwrap();
        assert_eq!(
            fixture
                .autos
                .0
                .occurrence_index
                .read()
                .unwrap()
                .targets(&fresh)
                .len(),
            1,
            "old off must not leave the live index holding a cancelled new parent"
        );
        assert_fresh_run(&fixture, fresh).await;
        fixture.cancel.cancel();
    }
}
