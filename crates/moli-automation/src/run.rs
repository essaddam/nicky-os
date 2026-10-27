//! One run: from the trigger, along the edges, one step at a time.
//! Every step is recorded and streamed (the editor lights the nodes up).

use std::time::Duration;

use moli_core::{Notice, Occurrence, OccurrenceId, Origin, PointId, Value};
use moli_i18n::tr;
use moli_runtime::CommandError;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use tokio_util::sync::CancellationToken;

use crate::model::{Automation, Channel, DayNight, Op, Rule, Step, minutes_of};
use crate::{Automations, clock, decimal, fold, lock, store, yes_no};

/// Steps per run: a graph is small; more means something is wrong.
const MAX_STEPS: usize = 100;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Run {
    pub id: u64,
    pub automation: String,
    pub name: String,
    /// The trigger node.
    pub trigger: String,
    /// Why it started, in words.
    pub why: String,
    pub started: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended: Option<u64>,
    pub status: RunStatus,
    #[serde(default)]
    pub dry: bool,
    pub steps: Vec<StepLog>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<OccurrenceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation: Option<String>,
}

impl Run {
    pub(crate) fn new(
        automation: &Automation,
        trigger: &str,
        why: String,
        id: u64,
        dry: bool,
        occurrence: Option<&Occurrence>,
    ) -> Self {
        Self {
            id,
            automation: automation.id.clone(),
            name: automation.name.clone(),
            trigger: trigger.to_owned(),
            why,
            started: moli_core::now_ms(),
            ended: None,
            status: RunStatus::Running,
            dry,
            steps: Vec::new(),
            occurrence: occurrence.map(|e| e.id.clone()),
            correlation: occurrence.map(|e| {
                e.correlation
                    .clone()
                    .unwrap_or_else(|| crate::occurrences::key(&e.id))
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Done,
    Failed,
    /// Replaced by a newer run, or switched off.
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StepLog {
    pub node: String,
    pub at: u64,
    pub status: StepStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Done,
    Failed,
    /// A dry run did not act.
    Simulated,
    Yes,
    No,
    Timeout,
    /// Waiting (only seen live).
    Waiting,
}

/// What the editor follows live.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Live {
    RunStarted {
        run: Run,
    },
    Step {
        run: u64,
        automation: String,
        step: StepLog,
    },
    RunEnded {
        run: Run,
    },
    Changed {
        automation: String,
    },
}

struct Ctx {
    text: Option<String>,
}

/// Both lineages are checked directly, including after waits and before effects.
#[derive(Clone)]
pub(crate) struct RunCancellation {
    pub activation: CancellationToken,
    pub generation: Option<CancellationToken>,
}

impl From<CancellationToken> for RunCancellation {
    fn from(activation: CancellationToken) -> Self {
        Self {
            activation,
            generation: None,
        }
    }
}

impl RunCancellation {
    fn permit(&self) -> moli_runtime::CommandPermit {
        let token = self.clone();
        moli_runtime::CommandPermit::new(move |_| !token.is_cancelled())
    }

    pub fn is_cancelled(&self) -> bool {
        self.activation.is_cancelled()
            || self
                .generation
                .as_ref()
                .is_some_and(CancellationToken::is_cancelled)
    }

    pub async fn cancelled(&self) {
        if let Some(generation) = &self.generation {
            tokio::select! { () = self.activation.cancelled() => {}, () = generation.cancelled() => {} }
        } else {
            self.activation.cancelled().await;
        }
    }
}

/// Runs `a` from the trigger `start`. Dry: conditions are read for real,
/// nothing acts and nothing waits.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute(
    autos: &Automations,
    a: &Automation,
    start: &str,
    why: String,
    dry: bool,
    token: RunCancellation,
    id: u64,
    occurrence: Option<&Occurrence>,
) -> Run {
    let mut run = Run::new(a, start, why, id, dry, occurrence);
    let _ = autos
        .0
        .live
        .send(crate::Live::RunStarted { run: run.clone() });
    let mut ctx = Ctx { text: None };
    let mut stack: Vec<String> = a
        .graph
        .next(start, "out")
        .iter()
        .rev()
        .map(|n| n.id.clone())
        .collect();
    let mut count = 0;
    let mut failed = false;
    while let Some(node_id) = stack.pop() {
        if token.is_cancelled() {
            run.status = RunStatus::Cancelled;
            break;
        }
        count += 1;
        if count > MAX_STEPS {
            failed = true;
            log(
                autos,
                &mut run,
                &node_id,
                StepStatus::Failed,
                Some(tr!("moteur.etape.trop_etapes")),
            );
            break;
        }
        let Some(node) = a.graph.node(&node_id) else {
            continue;
        };
        let (status, detail, port) =
            step(autos, a, &node.step, &node_id, &run, dry, &token, &mut ctx).await;
        if token.is_cancelled() && status == StepStatus::Waiting {
            run.status = RunStatus::Cancelled;
            break;
        }
        failed |= status == StepStatus::Failed;
        log(autos, &mut run, &node_id, status, detail);
        if let Some(port) = port {
            stack.extend(
                a.graph
                    .next(&node_id, port)
                    .iter()
                    .rev()
                    .map(|n| n.id.clone()),
            );
        }
    }
    if run.status == RunStatus::Cancelled
        && autos
            .0
            .shutting_down
            .load(std::sync::atomic::Ordering::Relaxed)
    {
        run.why = tr!("moteur.pourquoi.interrompu", pourquoi = run.why);
    }
    if run.status == RunStatus::Running {
        run.status = if failed {
            RunStatus::Failed
        } else {
            RunStatus::Done
        };
    }
    run.ended = Some(moli_core::now_ms());
    let recent: Vec<Run> = {
        let mut runs = lock(&autos.0.runs);
        runs.push_back(run.clone());
        while runs.len() > store::KEEP_RUNS {
            runs.pop_front();
        }
        runs.iter().cloned().collect()
    };
    {
        let _one = autos.0.runs_writing.lock().await;
        store::append_run(autos.0.runs_path.as_deref(), &run, recent).await;
    }
    let _ = autos
        .0
        .live
        .send(crate::Live::RunEnded { run: run.clone() });
    run
}

fn log(autos: &Automations, run: &mut Run, node: &str, status: StepStatus, detail: Option<String>) {
    let detail = detail.map(|d| {
        if d.chars().count() > 300 {
            d.chars().take(300).collect::<String>() + "…"
        } else {
            d
        }
    });
    let entry = StepLog {
        node: node.to_owned(),
        at: moli_core::now_ms(),
        status,
        detail,
    };
    let _ = autos.0.live.send(crate::Live::Step {
        run: run.id,
        automation: run.automation.clone(),
        step: entry.clone(),
    });
    run.steps.push(entry);
}

fn waiting(autos: &Automations, run: &Run, node: &str, detail: String) {
    let _ = autos.0.live.send(crate::Live::Step {
        run: run.id,
        automation: run.automation.clone(),
        step: StepLog {
            node: node.to_owned(),
            at: moli_core::now_ms(),
            status: StepStatus::Waiting,
            detail: Some(detail),
        },
    });
}

/// One step: (status, detail, port to follow — none stops this branch).
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
async fn step(
    autos: &Automations,
    a: &Automation,
    step: &Step,
    node_id: &str,
    run: &Run,
    dry: bool,
    token: &RunCancellation,
    ctx: &mut Ctx,
) -> (StepStatus, Option<String>, Option<&'static str>) {
    let hub = &autos.0.hub;
    match step {
        Step::If { rules, all } => {
            let results: Vec<bool> = rules.iter().map(|r| holds(autos, r)).collect();
            let yes = if *all {
                results.iter().all(|r| *r)
            } else {
                results.iter().any(|r| *r)
            };
            if yes {
                (StepStatus::Yes, None, Some("yes"))
            } else {
                (StepStatus::No, None, Some("no"))
            }
        }
        Step::Scene { scene } => {
            if dry {
                return (
                    StepStatus::Simulated,
                    Some(tr!("moteur.action.scene", scene = scene)),
                    Some("out"),
                );
            }
            let preview = match hub.preview_scene(&scene.as_str().into()).await {
                Ok(p) => p,
                Err(e) => return (StepStatus::Failed, Some(e.to_string()), None),
            };
            let active = match hub
                .start_scene_correlated(
                    preview,
                    Origin::Automation,
                    Some(a.name.clone()),
                    Some(token.permit()),
                    run.correlation.clone(),
                )
                .await
            {
                Ok(r) => r,
                Err(e) => return (StepStatus::Failed, Some(e.to_string()), None),
            };
            let result = tokio::select! {
                biased;
                () = token.cancelled() => {let _ = hub.cancel_scene(&active.id, Origin::Automation, Some(a.name.clone())); return (StepStatus::Failed, Some(tr!("serveur.scene.annulee")), None);}
                result = hub.wait_scene(&active.id) => result,
            };
            match result {
                Ok(r) if r.status == moli_runtime::SceneStatus::Done => {
                    (StepStatus::Done, Some(r.id), Some("out"))
                }
                Ok(r) => (StepStatus::Failed, Some(r.id), None),
                Err(e) => (StepStatus::Failed, Some(e.to_string()), None),
            }
        }
        Step::Set { point, value } => {
            act(
                autos,
                a,
                point,
                Value::from_json(value),
                dry,
                run.correlation.clone(),
                token,
            )
            .await
        }
        Step::Toggle { point } => {
            let pid = PointId::from(point.as_str());
            let next = match hub.state(&pid).map(|s| s.value) {
                Some(Value::Bool(b)) => Value::Bool(!b),
                Some(Value::Text(t)) if t.eq_ignore_ascii_case("on") => Value::Text("OFF".into()),
                Some(Value::Text(t)) if t.eq_ignore_ascii_case("off") => Value::Text("ON".into()),
                _ => {
                    return (
                        StepStatus::Failed,
                        Some(tr!("moteur.etape.etat_inconnu")),
                        None,
                    );
                }
            };
            act(autos, a, point, next, dry, run.correlation.clone(), token).await
        }
        Step::Wait { seconds } => {
            if dry {
                return (
                    StepStatus::Simulated,
                    Some(tr!("moteur.etape.attendrait", secondes = seconds)),
                    Some("out"),
                );
            }
            waiting(autos, run, node_id, format!("{seconds} s"));
            tokio::select! {
                () = tokio::time::sleep(Duration::from_secs(*seconds)) => (StepStatus::Done, None, Some("out")),
                () = token.cancelled() => (StepStatus::Waiting, None, None),
            }
        }
        Step::WaitFor { rule, timeout_s } => {
            if holds(autos, rule) {
                return (StepStatus::Done, None, Some("ok"));
            }
            if dry {
                return (
                    StepStatus::Simulated,
                    Some(tr!("moteur.etape.attendrait_jusqua", secondes = timeout_s)),
                    Some("ok"),
                );
            }
            waiting(
                autos,
                run,
                node_id,
                tr!("moteur.etape.au_plus", secondes = timeout_s),
            );
            let mut events = hub.subscribe();
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            let deadline = tokio::time::sleep(Duration::from_secs(*timeout_s));
            tokio::pin!(deadline);
            loop {
                tokio::select! {
                    () = &mut deadline => return (StepStatus::Timeout, None, Some("timeout")),
                    () = token.cancelled() => return (StepStatus::Waiting, None, None),
                    _ = events.recv() => {}
                    _ = tick.tick() => {}
                }
                if holds(autos, rule) {
                    return (StepStatus::Done, None, Some("ok"));
                }
            }
        }
        Step::Notify {
            title,
            message,
            channels,
        } => {
            let text = render(autos, message, ctx);
            let title = title.as_deref().map(|t| render(autos, t, ctx));
            if dry {
                return (StepStatus::Simulated, Some(text), Some("out"));
            }
            let result = tokio::select! {
                biased;
                () = token.cancelled() => return (StepStatus::Waiting, None, None),
                result = notify(autos, a, title, &text, channels, run.correlation.clone(), token) => result,
            };
            match result {
                Ok(()) => (StepStatus::Done, Some(text), Some("out")),
                Err(e) => (StepStatus::Failed, Some(e.to_string()), Some("out")),
            }
        }
        Step::Write { prompt } => {
            if dry {
                ctx.text = Some(tr!("moteur.etape.texte_moli"));
                return (
                    StepStatus::Simulated,
                    Some(tr!("moteur.etape.moli_ecrirait")),
                    Some("out"),
                );
            }
            let Some(writer) = autos.writer() else {
                return (
                    StepStatus::Failed,
                    Some(tr!("moteur.etape.moli_absent")),
                    None,
                );
            };
            let prompt = render_without(autos, prompt, ctx, |key| writer.withholds(key));
            match tokio::time::timeout(Duration::from_secs(60), writer.write(prompt)).await {
                Ok(Ok(text)) => {
                    let short: String = text.chars().take(120).collect();
                    ctx.text = Some(text);
                    (StepStatus::Done, Some(short), Some("out"))
                }
                Ok(Err(e)) => (StepStatus::Failed, Some(e.to_string()), None),
                Err(_) => (
                    StepStatus::Failed,
                    Some(tr!("moteur.etape.moli_lent")),
                    None,
                ),
            }
        }
        // Triggers are where runs start, not steps.
        _ => (
            StepStatus::Failed,
            Some(tr!("moteur.etape.declencheur_milieu")),
            None,
        ),
    }
}

async fn act(
    autos: &Automations,
    a: &Automation,
    point: &str,
    value: Value,
    dry: bool,
    correlation: Option<String>,
    token: &RunCancellation,
) -> (StepStatus, Option<String>, Option<&'static str>) {
    if dry {
        return (
            StepStatus::Simulated,
            Some(format!("→ {}", show(&value))),
            Some("out"),
        );
    }
    let pid = PointId::from(point);
    let result = tokio::select! {
        biased;
        () = token.cancelled() => return (StepStatus::Waiting, None, None),
        result = autos.0.hub.command_correlated_with_permit(
            &pid, value.clone(), Origin::Automation, Some(a.name.clone()),
            correlation, token.permit(),
        ) => result,
    };
    match result {
        Ok(()) => (
            StepStatus::Done,
            Some(format!("→ {}", show(&value))),
            Some("out"),
        ),
        Err(CommandError::NeedsApproval { reason, .. }) => (
            StepStatus::Failed,
            Some(tr!("moteur.etape.retenu", raison = reason)),
            None,
        ),
        Err(e) => (StepStatus::Failed, Some(e.to_string()), None),
    }
}

/// Normalize a temporal literal exactly as the point's command path does.
pub(crate) fn canonical_expected(autos: &Automations, point: &str, value: &Json) -> Option<Json> {
    let temporal = PointId::from(point).split().and_then(|(id, key)| {
        autos
            .0
            .hub
            .describe(&id)
            .and_then(|(d, _)| d.point(key).map(|p| p.kind.clone()))
    });
    if let Some(moli_core::Kind::Temporal { spec }) = temporal {
        moli_runtime::normalize_temporal(&spec, &Value::from_json(value))
            .ok()
            .map(|v| v.to_json())
    } else {
        Some(value.clone())
    }
}

fn show(v: &Value) -> String {
    match v {
        Value::Bool(b) => yes_no(*b),
        Value::Int(i) => i.to_string(),
        Value::EnumCode(code) => format!("enum_code:{code}"),
        Value::Float(f) => decimal(&format!("{f}")),
        Value::Text(t) => t.to_string(),
        Value::Null => "—".into(),
    }
}

// ---- rules ---------------------------------------------------------------------

/// Same value, loosely: ON ≡ true, numbers by value, text without case.
pub(crate) fn same(current: &Value, wanted: &Json) -> bool {
    let truth = |v: &Value| match v {
        Value::Bool(b) => Some(*b),
        Value::Text(t) if t.eq_ignore_ascii_case("on") => Some(true),
        Value::Text(t) if t.eq_ignore_ascii_case("off") => Some(false),
        _ => None,
    };
    let wanted_v = Value::from_json(wanted);
    if let (Some(a), Some(b)) = (truth(current), truth(&wanted_v)) {
        return a == b;
    }
    if let (Some(a), Some(b)) = (current.as_f64(), wanted_v.as_f64()) {
        return (a - b).abs() < 1e-9;
    }
    match (current, &wanted_v) {
        (Value::Text(a), Value::Text(b)) => a.eq_ignore_ascii_case(b),
        _ => false,
    }
}

pub(crate) fn holds(autos: &Automations, rule: &Rule) -> bool {
    match rule {
        Rule::State { point, op, value } => {
            let Some(current) = autos
                .0
                .hub
                .state(&PointId::from(point.as_str()))
                .map(|s| s.value)
            else {
                return false;
            };
            if matches!(current, Value::Null) {
                return false;
            }
            let Some(value) = canonical_expected(autos, point, value) else {
                return false;
            };
            match op {
                Op::Eq => same(&current, &value),
                Op::Ne => !same(&current, &value),
                _ => {
                    let (Some(c), Some(v)) = (current.as_f64(), value.as_f64()) else {
                        return false;
                    };
                    match op {
                        Op::Gt => c > v,
                        Op::Ge => c >= v,
                        Op::Lt => c < v,
                        _ => c <= v,
                    }
                }
            }
        }
        Rule::Time {
            after,
            before,
            days,
        } => {
            let now = jiff::Timestamp::now().to_zoned(autos.0.tz.clone());
            let m =
                u32::from(now.hour().unsigned_abs()) * 60 + u32::from(now.minute().unsigned_abs());
            if !day_ok(days, &now) {
                return false;
            }
            in_window(
                m,
                after.as_deref().and_then(minutes_of),
                before.as_deref().and_then(minutes_of),
            )
        }
        Rule::Sun { is } => {
            let day = autos.is_day();
            if *is == DayNight::Day { day } else { !day }
        }
    }
}

pub(crate) fn day_ok(days: &[u8], now: &jiff::Zoned) -> bool {
    let today = u8::try_from(now.weekday().to_monday_one_offset()).unwrap_or(1);
    days.is_empty() || days.contains(&today)
}

/// Overnight windows (22:00 → 06:00) wrap around midnight.
pub(crate) fn in_window(m: u32, after: Option<u32>, before: Option<u32>) -> bool {
    match (after, before) {
        (Some(a), Some(b)) if a <= b => a <= m && m < b,
        (Some(a), Some(b)) => m >= a || m < b,
        (Some(a), None) => m >= a,
        (None, Some(b)) => m < b,
        (None, None) => true,
    }
}

impl Automations {
    /// Sun up, from the house's position, else from a weather `daylight`.
    pub(crate) fn is_day(&self) -> bool {
        let now = jiff::Timestamp::now();
        if let (Some(lat), Some(lon)) = (self.0.config.latitude, self.0.config.longitude) {
            let date = now.to_zoned(self.0.tz.clone()).date();
            return crate::sun::times(date, lat, lon)
                .is_some_and(|(rise, set)| rise <= now && now < set);
        }
        self.0
            .hub
            .snapshot()
            .devices
            .iter()
            .find_map(|d| d.state.get("daylight").map(|s| s.value.clone()))
            .is_some_and(|v| matches!(v, Value::Bool(true)))
    }
}

// ---- messages ------------------------------------------------------------------

/// `{{texte}}`, `{{heure}}`, `{{<device>/<key>}}` (value and unit).
fn render(autos: &Automations, template: &str, ctx: &Ctx) -> String {
    render_without(autos, template, ctx, |_| false)
}

/// `render`, the values whose key is `withheld` left « ? ».
fn render_without(
    autos: &Automations,
    template: &str,
    ctx: &Ctx,
    withheld: impl Fn(&str) -> bool,
) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let Some(len) = rest[start + 2..].find("}}") else {
            out.push_str(&rest[start..]);
            return out;
        };
        let key = rest[start + 2..start + 2 + len].trim();
        out.push_str(&fill(autos, key, ctx, &withheld));
        rest = &rest[start + 2 + len + 2..];
    }
    out.push_str(rest);
    out
}

fn fill(autos: &Automations, key: &str, ctx: &Ctx, withheld: &dyn Fn(&str) -> bool) -> String {
    match key {
        "texte" | "text" => ctx.text.clone().unwrap_or_default(),
        "heure" => {
            let now = jiff::Timestamp::now().to_zoned(autos.0.tz.clone());
            clock(now.hour(), now.minute())
        }
        point if point.contains('/') => {
            let pid = PointId::from(point);
            if pid.split().is_some_and(|(_, k)| withheld(k)) {
                return "?".into();
            }
            let unit = pid.split().and_then(|(d, k)| {
                autos.0.hub.device(&d).and_then(|v| {
                    v.device
                        .points
                        .iter()
                        .find(|p| *p.key == *k)
                        .and_then(|p| p.unit.as_ref().map(|u| u.symbol().to_owned()))
                })
            });
            match autos.0.hub.state(&pid).map(|s| s.value) {
                Some(Value::Float(f)) => {
                    let rounded = (f * 10.0).round() / 10.0;
                    format!(
                        "{}{}",
                        decimal(&rounded.to_string()),
                        unit.map(|u| format!(" {u}")).unwrap_or_default()
                    )
                }
                Some(v) => format!(
                    "{}{}",
                    show(&v),
                    unit.map(|u| format!(" {u}")).unwrap_or_default()
                ),
                None => "?".into(),
            }
        }
        _ => String::new(),
    }
}

/// Every channel is tried, whatever happens to the others: a speaker that
/// does not answer never keeps the Telegram message from leaving. Voice and
/// Telegram are commands like any other (journaled, as this automation).
async fn notify(
    autos: &Automations,
    a: &Automation,
    title: Option<String>,
    message: &str,
    channels: &[Channel],
    correlation: Option<String>,
    token: &RunCancellation,
) -> anyhow::Result<()> {
    let messengers = autos.messengers();
    if channels.contains(&Channel::Maison) || messengers.is_empty() {
        autos.0.hub.notice(Notice {
            title: title.clone(),
            message: message.to_owned(),
            from: Some(a.name.clone()),
            ts: moli_core::now_ms(),
        });
    }
    let mut failures = Vec::new();
    let mut say = async |points: Vec<PointId>, text: String, what: &str| {
        for point in points {
            if let Err(e) = autos
                .0
                .hub
                .command_correlated_with_permit(
                    &point,
                    Value::Text(text.clone().into()),
                    Origin::Automation,
                    Some(a.name.clone()),
                    correlation.clone(),
                    token.permit(),
                )
                .await
            {
                failures.push(tr!(
                    "moteur.canal.echec",
                    canal = what,
                    point = point,
                    erreur = e
                ));
            }
        }
    };
    if channels.contains(&Channel::Voix) {
        let text: String = message.chars().take(500).collect();
        say(autos.speakers(), text, &tr!("moteur.canal.annonce")).await;
    }
    let heading = title.unwrap_or_else(|| a.name.clone());
    if channels.contains(&Channel::Telegram) {
        say(messengers, format!("{heading}\n{message}"), "Telegram").await;
    }
    if channels.contains(&Channel::Telephone) {
        say(
            autos.phones(),
            format!("{heading}\n{message}"),
            &tr!("moteur.canal.telephone"),
        )
        .await;
    }
    if failures.is_empty() {
        Ok(())
    } else {
        anyhow::bail!("{}", fold(failures, "moteur.liste.point_virgule"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_codes_never_compare_as_numbers_or_booleans_without_schema_context() {
        assert!(!same(&Value::EnumCode(7), &serde_json::json!(7)));
        assert!(!same(&Value::EnumCode(1), &serde_json::json!(true)));
        assert!(!same(&Value::EnumCode(0), &serde_json::json!(false)));
        assert!(!same(&Value::EnumCode(7), &serde_json::json!("7")));
        assert!(same(&Value::Int(7), &serde_json::json!(7.0)));
        assert!(same(&Value::from("ON"), &serde_json::json!(true)));
    }

    #[test]
    fn windows_wrap_around_midnight() {
        let at = |s: &str| minutes_of(s);
        assert!(in_window(23 * 60, at("22:00"), at("06:00")));
        assert!(in_window(5 * 60, at("22:00"), at("06:00")));
        assert!(!in_window(12 * 60, at("22:00"), at("06:00")));
        assert!(in_window(12 * 60, at("08:00"), at("18:00")));
        assert!(!in_window(18 * 60, at("08:00"), at("18:00")));
    }

    #[test]
    fn tank_recipe_windows_and_all_four_clock_triggers_match() {
        let recipe: Json = serde_json::from_str(include_str!(
            "../../../integrations/helpers/recettes/ballon.json"
        ))
        .unwrap();
        let automatic: Automation =
            serde_json::from_value(recipe["automations"][0].clone()).unwrap();
        let hours: Vec<_> = automatic
            .graph
            .triggers()
            .filter_map(|node| match &node.step {
                Step::AtTime { at, days } => {
                    assert!(days.is_empty());
                    Some((node.id.as_str(), at.as_str()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            hours,
            [
                ("h0120", "01:20"),
                ("h0720", "07:20"),
                ("h1150", "11:50"),
                ("h1350", "13:50")
            ]
        );
        for (id, _) in hours {
            assert!(
                automatic
                    .graph
                    .edges
                    .iter()
                    .any(|edge| edge.from == id && edge.to == "automatique")
            );
        }
        let node = automatic
            .graph
            .nodes
            .iter()
            .find(|node| node.id == "creuses")
            .unwrap();
        let Step::If { rules, all } = &node.step else {
            panic!("missing time condition")
        };
        assert!(!all, "off-peak windows are alternatives");
        for (at, expected) in [
            ("01:19", false),
            ("01:20", true),
            ("07:19", true),
            ("07:20", false),
            ("11:49", false),
            ("11:50", true),
            ("13:49", true),
            ("13:50", false),
        ] {
            let heat = rules.iter().any(|rule| {
                let Rule::Time {
                    after,
                    before,
                    days,
                } = rule
                else {
                    panic!("expected a time window")
                };
                assert!(days.is_empty());
                in_window(
                    minutes_of(at).unwrap(),
                    after.as_deref().and_then(minutes_of),
                    before.as_deref().and_then(minutes_of),
                )
            });
            assert_eq!(heat, expected, "window at {at}");
        }
    }

    #[test]
    fn values_compare_loosely() {
        assert!(same(&Value::Text("ON".into()), &Json::Bool(true)));
        assert!(same(&Value::Bool(false), &Json::String("off".into())));
        assert!(same(&Value::Int(21), &serde_json::json!(21.0)));
        assert!(same(
            &Value::Text("Cool".into()),
            &Json::String("cool".into())
        ));
        assert!(!same(&Value::Bool(true), &Json::Bool(false)));
    }
}
