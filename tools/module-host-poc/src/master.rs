//! Minimal control plane: no business adapter, MQTT client, broker or audio code.
use anyhow::{Context, Result, ensure};
use bytes::Bytes;
use http_body_util::Full;
use hyper::{Request, Response, StatusCode, body::Incoming, service::service_fn};
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::BTreeSet,
    path::PathBuf,
    process::Stdio,
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    net::TcpListener,
    process::{Child, Command},
    sync::{Semaphore, mpsc},
};
use tokio_util::sync::CancellationToken;

const MAX_LINE: usize = 8192;
const MAX_GROUPS: usize = 8;
#[derive(Clone, Default, Serialize)]
struct Group {
    id: String,
    modules: Vec<String>,
    role: &'static str,
    phase: &'static str,
    pid: Option<u32>,
    broker_port: Option<u16>,
    host_reports_ready: bool,
    last_heartbeat_ms: Option<u64>,
    rss_bytes: Option<u64>,
    rss_peak_bytes: u64,
    cpu_percent_one_core: Option<f64>,
    exit_code: Option<i32>,
}
#[derive(Clone, Serialize)]
struct State {
    phase: &'static str,
    ready: bool,
    groups: Vec<Group>,
    error: Option<&'static str>,
    startup_order: Vec<&'static str>,
    master_rss_bytes: Option<u64>,
    surface: &'static str,
}
type Shared = Arc<Mutex<State>>;
fn change(state: &Shared, f: impl FnOnce(&mut State)) {
    f(&mut state.lock().unwrap_or_else(PoisonError::into_inner));
}
fn copy(state: &Shared) -> State {
    state.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

async fn response(
    request: Request<Incoming>,
    state: Shared,
) -> Result<Response<Full<Bytes>>, std::convert::Infallible> {
    let has_body = request.headers().contains_key("transfer-encoding")
        || request
            .headers()
            .get("content-length")
            .is_some_and(|v| v.to_str().ok().and_then(|s| s.parse::<u64>().ok()) != Some(0));
    let (status, body) = if request.method() != hyper::Method::GET {
        (StatusCode::METHOD_NOT_ALLOWED, json!({"error":"read_only"}))
    } else if has_body {
        (
            StatusCode::PAYLOAD_TOO_LARGE,
            json!({"error":"no_request_body"}),
        )
    } else {
        match request.uri().path() {
            "/health" => (
                StatusCode::OK,
                json!({"alive":true,"ready":copy(&state).ready,"phase":copy(&state).phase}),
            ),
            "/" | "/status" => (
                StatusCode::OK,
                serde_json::to_value(copy(&state))
                    .unwrap_or_else(|_| json!({"error":"status_serialization"})),
            ),
            _ => (StatusCode::NOT_FOUND, json!({"error":"not_found"})),
        }
    };
    let mut result = Response::new(Full::new(Bytes::from(
        serde_json::to_vec(&body).unwrap_or_else(|_| b"{}".to_vec()),
    )));
    *result.status_mut() = status;
    for (name, value) in [
        ("content-type", "application/json; charset=utf-8"),
        ("cache-control", "no-store"),
        ("connection", "close"),
        ("x-content-type-options", "nosniff"),
    ] {
        result.headers_mut().insert(
            hyper::header::HeaderName::from_static(name),
            hyper::header::HeaderValue::from_static(value),
        );
    }
    Ok(result)
}

async fn http(listener: TcpListener, state: Shared, cancel: CancellationToken) {
    let permits = Arc::new(Semaphore::new(16));
    let mut tasks = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            ()=cancel.cancelled()=>break,
            Some(_)=tasks.join_next(),if !tasks.is_empty()=>{},
            accepted=listener.accept()=>{let Ok((stream,address))=accepted else{break;};if !address.ip().is_loopback(){continue;}
                let Ok(permit)=permits.clone().try_acquire_owned()else{continue;};let state=state.clone();
                tasks.spawn(async move{let _permit=permit;let service=service_fn(move|request|response(request,state.clone()));let mut builder=hyper::server::conn::http1::Builder::new();builder.keep_alive(false).max_buf_size(8192);let _=tokio::time::timeout(Duration::from_secs(2),builder.serve_connection(TokioIo::new(stream),service)).await;});
            }
        }
    }
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    mode: String,
    groups: Vec<Planned>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Planned {
    id: String,
    modules: Vec<String>,
}
fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
impl Plan {
    fn validate(&self) -> Result<()> {
        ensure!(
            matches!(self.mode.as_str(), "merged" | "grouped" | "isolated")
                && self.groups.len() <= MAX_GROUPS,
            "invalid process plan"
        );
        let mut groups = BTreeSet::new();
        let mut modules = BTreeSet::new();
        for group in &self.groups {
            ensure!(
                safe_id(&group.id)
                    && group.id != "transport"
                    && group.id != "master"
                    && groups.insert(&group.id),
                "invalid group identity"
            );
            for module in &group.modules {
                ensure!(
                    safe_id(module) && modules.insert(module),
                    "invalid module assignment"
                );
            }
        }
        ensure!(modules.len() <= 32, "module bound");
        Ok(())
    }
}
async fn plan(host: &PathBuf, config: &PathBuf, cancel: &CancellationToken) -> Result<Plan> {
    ensure!(!cancel.is_cancelled(), "planning cancelled");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut child = Command::new(host)
        .arg("--plan")
        .arg(config)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .env_remove("MQTT_PASSWORD")
        .env_remove("MOLI_MASTER_KEY")
        .env_remove("MOLI_MASTER_KEY_FILE")
        .kill_on_drop(true)
        .spawn()?;
    let result = tokio::select! {
        biased;
        () = cancel.cancelled() => Err(anyhow::anyhow!("planning cancelled")),
        result = tokio::time::timeout_at(deadline, async {
            let mut bytes = Vec::new();
            child.stdout.take().context("planner output")?.take(65537).read_to_end(&mut bytes).await?;
            ensure!(bytes.len() <= 65536, "planner output bound");
            ensure!(child.wait().await?.success(), "planner failed");
            let plan: Plan = serde_json::from_slice(&bytes)?;
            plan.validate()?;
            Ok(plan)
        }) => result.context("planner deadline").and_then(|result| result),
    };
    if result.is_err() {
        // Child::kill includes wait: never leave a timed-out planner unreaped.
        let _ = child.kill().await;
    }
    result
}
enum Event {
    Ready { index: usize, port: Option<u16> },
    Heartbeat { index: usize },
}
async fn stdout(mut reader: impl AsyncRead + Unpin, index: usize, events: mpsc::Sender<Event>) {
    let mut chunk = [0; 2048];
    let mut line = Vec::new();
    let mut discard = false;
    while let Ok(count) = reader.read(&mut chunk).await {
        if count == 0 {
            break;
        }
        for byte in &chunk[..count] {
            if *byte == b'\n' {
                if !discard {
                    if let Some(raw) = line.strip_prefix(b"HOST_READY ")
                        && let Ok(v) = serde_json::from_slice::<serde_json::Value>(raw)
                    {
                        let port = v["port"].as_u64().and_then(|p| u16::try_from(p).ok());
                        let _ = events.try_send(Event::Ready { index, port });
                    } else if line.starts_with(b"HOST_HEARTBEAT ") {
                        let _ = events.try_send(Event::Heartbeat { index });
                    }
                }
                line.clear();
                discard = false;
            } else if !discard {
                if line.len() < MAX_LINE {
                    line.push(*byte);
                } else {
                    line.clear();
                    discard = true;
                }
            }
        }
    }
}
struct Running {
    child: Child,
    reader: tokio::task::JoinHandle<()>,
    last: Option<(Instant, f64)>,
}
async fn launch(
    host: &PathBuf,
    args: &[String],
    index: usize,
    events: mpsc::Sender<Event>,
    cancel: &CancellationToken,
) -> Result<Running> {
    ensure!(!cancel.is_cancelled(), "host launch cancelled");
    let mut child = Command::new(host)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env_remove("MQTT_PASSWORD")
        .env_remove("MOLI_MASTER_KEY")
        .env_remove("MOLI_MASTER_KEY_FILE")
        .kill_on_drop(true)
        .spawn()?;
    let reader = tokio::spawn(stdout(
        child.stdout.take().context("host output")?,
        index,
        events,
    ));
    Ok(Running {
        child,
        reader,
        last: None,
    })
}
async fn metrics(pid: u32, cancel: &CancellationToken) -> Result<(u64, f64)> {
    let mut command = Command::new("/bin/ps");
    command
        .args(["-o", "rss=,time=", "-p", &pid.to_string()])
        .kill_on_drop(true);
    let output = tokio::select! {
        biased;
        () = cancel.cancelled() => anyhow::bail!("metrics cancelled"),
        result = tokio::time::timeout(Duration::from_millis(500), command.output()) => result.context("metrics deadline")??,
    };
    ensure!(output.status.success(), "process unavailable");
    let text = std::str::from_utf8(&output.stdout)?;
    let parts: Vec<_> = text.split_whitespace().collect();
    ensure!(parts.len() == 2, "process metrics format");
    let mut cpu = 0.0;
    for part in parts[1].split(':') {
        cpu = cpu * 60.0 + part.parse::<f64>()?;
    }
    Ok((parts[0].parse::<u64>()? * 1024, cpu))
}

async fn supervise(
    host: PathBuf,
    config: PathBuf,
    state: Shared,
    cancel: CancellationToken,
) -> Result<()> {
    let mut running = Vec::new();
    let result = supervise_inner(&host, &config, &state, &cancel, &mut running).await;
    change(&state, |s| {
        s.ready = false;
        s.phase = "stopping";
    });
    // Signal every owned child before joining any of them.
    for process in &mut running {
        let _ = process.child.start_kill();
    }
    for (index, mut process) in running.into_iter().enumerate() {
        let exit = process.child.wait().await.ok();
        process.reader.abort();
        let _ = process.reader.await;
        change(&state, |s| {
            if let Some(group) = s.groups.get_mut(index) {
                if !terminal(group) {
                    group.phase = "stopped";
                }
                if let Some(exit) = exit {
                    group.exit_code = exit.code();
                }
                group.pid = None;
                group.host_reports_ready = false;
                group.last_heartbeat_ms = None;
                group.rss_bytes = None;
                group.cpu_percent_one_core = None;
            }
        });
    }
    change(&state, mark_stopped);
    result
}

fn terminal(group: &Group) -> bool {
    matches!(group.phase, "exited" | "failed" | "stopped")
}

fn mark_stopped(state: &mut State) {
    state.ready = false;
    state.phase = "stopped";
    for group in &mut state.groups {
        if !terminal(group) {
            group.phase = "stopped";
        }
        group.pid = None;
        group.host_reports_ready = false;
        group.last_heartbeat_ms = None;
        group.rss_bytes = None;
        group.cpu_percent_one_core = None;
    }
}

fn apply_event(state: &mut State, event: Event, elapsed_ms: u64) {
    let index = match &event {
        Event::Ready { index, .. } | Event::Heartbeat { index } => *index,
    };
    let Some(group) = state.groups.get_mut(index) else {
        return;
    };
    if terminal(group) {
        return;
    }
    match event {
        Event::Ready { port, .. } => {
            group.host_reports_ready = true;
            group.broker_port = port;
            group.phase = "running";
        }
        Event::Heartbeat { .. } => {}
    }
    group.last_heartbeat_ms = Some(elapsed_ms);
}

async fn supervise_inner(
    host: &PathBuf,
    config: &PathBuf,
    state: &Shared,
    cancel: &CancellationToken,
    running: &mut Vec<Running>,
) -> Result<()> {
    let origin = Instant::now();
    change(state, |s| s.startup_order.push("planner_started"));
    let plan = plan(host, config, cancel).await?;
    let (events, mut inbox) = mpsc::channel(32);
    let mut port = 0;
    if !plan.groups.is_empty() && plan.mode != "merged" {
        change(state, |s| {
            s.groups.push(Group {
                id: "transport".into(),
                role: "broker",
                phase: "starting",
                ..Group::default()
            })
        });
        running.push(launch(host, &["--broker".into()], 0, events.clone(), cancel).await?);
        change(state, |s| s.groups[0].pid = running[0].child.id());
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while port == 0 {
            ensure!(
                tokio::time::Instant::now() < deadline,
                "broker readiness deadline"
            );
            let event = tokio::select! {
                biased;
                () = cancel.cancelled() => anyhow::bail!("broker startup cancelled"),
                event = tokio::time::timeout_at(deadline, inbox.recv()) => event.context("broker readiness deadline")?.context("broker readiness")?,
            };
            if let Event::Ready {
                index: 0,
                port: Some(p),
            } = event
            {
                ensure!(
                    p != 0 && p != 1414 && p != 1415,
                    "invalid fixture broker port"
                );
                port = p;
                change(state, |s| {
                    s.groups[0].host_reports_ready = true;
                    s.groups[0].broker_port = Some(p);
                    s.groups[0].phase = "running";
                    s.groups[0].last_heartbeat_ms = Some(origin.elapsed().as_millis() as u64);
                });
            }
        }
    }
    for group in plan.groups {
        ensure!(!cancel.is_cancelled(), "group startup cancelled");
        let index = running.len();
        let id = group.id.clone();
        change(state, |s| {
            s.groups.push(Group {
                id: id.clone(),
                modules: group.modules,
                role: if plan.mode == "merged" {
                    "broker-and-modules"
                } else {
                    "modules"
                },
                phase: "starting",
                ..Group::default()
            })
        });
        let args = vec![
            "--serve-group".into(),
            config.to_string_lossy().into_owned(),
            id,
            port.to_string(),
        ];
        running.push(launch(host, &args, index, events.clone(), cancel).await?);
        change(state, |s| s.groups[index].pid = running[index].child.id());
    }
    change(state, |s| {
        s.phase = "starting";
        s.startup_order.push("hosts_started");
    });
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            ()=cancel.cancelled()=>break,
            Some(event)=inbox.recv()=>change(state,|s|apply_event(s,event,origin.elapsed().as_millis() as u64)),
            _=tick.tick()=>{
                for (index,process) in running.iter_mut().enumerate(){
                    if cancel.is_cancelled() { break; }
                    if let Some(exit)=process.child.try_wait()?{change(state,|s|{let g=&mut s.groups[index];g.phase=if exit.success(){"exited"}else{"failed"};g.exit_code=exit.code();g.host_reports_ready=false;g.pid=None;g.rss_bytes=None;g.cpu_percent_one_core=None;});}
                    else if let Some(pid)=process.child.id()&&let Ok((rss,cpu))=metrics(pid,cancel).await{
                        let current=Instant::now();let pct=process.last.map(|(at,last)|(cpu-last)/current.duration_since(at).as_secs_f64()*100.0);process.last=Some((current,cpu));
                        change(state,|s|{let g=&mut s.groups[index];g.rss_bytes=Some(rss);g.rss_peak_bytes=g.rss_peak_bytes.max(rss);g.cpu_percent_one_core=pct;});
                    }
                }
                if !cancel.is_cancelled() && let Ok((rss,_))=metrics(std::process::id(),cancel).await{change(state,|s|s.master_rss_bytes=Some(rss));}
            }
        }
        change(state, |s| {
            let now = origin.elapsed().as_millis() as u64;
            s.ready = s.groups.iter().all(|g| {
                g.host_reports_ready
                    && g.phase == "running"
                    && g.last_heartbeat_ms
                        .is_some_and(|at| now.saturating_sub(at) < 5000)
            });
            s.phase = if s.ready {
                "ready"
            } else if s.groups.iter().any(|g| g.phase == "failed") {
                "degraded"
            } else {
                "starting_or_finished"
            };
        });
    }
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    #[cfg(unix)]
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    #[cfg(unix)]
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 2 || args.len() == 4 || args.len() == 6,
        "moli-master-poc --config <layout> [--port <port>] [--seconds <duration>]"
    );
    ensure!(args[0] == "--config", "configuration required");
    let config = PathBuf::from(&args[1]);
    let (mut port, mut seconds) = (0u16, 0u64);
    for pair in args[2..].chunks(2) {
        match pair[0].as_str() {
            "--port" => port = pair[1].parse()?,
            "--seconds" => seconds = pair[1].parse()?,
            _ => anyhow::bail!("unknown master option"),
        }
    }
    ensure!(
        port != 1414 && port != 1415 && seconds <= 300,
        "reserved port or excessive test duration"
    );
    let state = Arc::new(Mutex::new(State {
        phase: "loading",
        ready: false,
        groups: Vec::new(),
        error: None,
        startup_order: vec!["http_bound"],
        master_rss_bytes: None,
        surface: "loopback HTTP GET health/status; bounded connections; no business, MQTT, audio or write API",
    }));
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    let port = listener.local_addr()?.port();
    let cancel = CancellationToken::new();
    let http = tokio::spawn(http(listener, state.clone(), cancel.clone()));
    println!(
        "MASTER_LISTENING {}",
        json!({"port":port,"http_bound_before_planning":true})
    );
    let host = std::env::current_exe()?.with_file_name("moli-module-host-poc");
    let s = state.clone();
    let c = cancel.clone();
    let supervisor = tokio::spawn(async move {
        let stopping = c.clone();
        if supervise(host, config, s.clone(), c).await.is_err() && !stopping.is_cancelled() {
            change(&s, |v| {
                v.phase = "error";
                v.ready = false;
                v.error = Some("startup_or_supervision_failed");
            });
        }
    });
    #[cfg(unix)]
    tokio::select! {
        _ = interrupt.recv() => {},
        _ = terminate.recv() => {},
        _ = tokio::time::sleep(Duration::from_secs(seconds)), if seconds > 0 => {},
    }
    #[cfg(not(unix))]
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {},
        _ = tokio::time::sleep(Duration::from_secs(seconds)), if seconds > 0 => {},
    }
    cancel.cancel();
    if supervisor.await.is_err() {
        change(&state, |s| s.error = Some("supervisor_task_failed"));
    }
    let _ = http.await;
    change(&state, mark_stopped);
    println!("MASTER_STOPPED {}", serde_json::to_string(&copy(&state))?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state(groups: Vec<Group>) -> State {
        State {
            phase: "ready",
            ready: true,
            groups,
            error: None,
            startup_order: Vec::new(),
            master_rss_bytes: Some(1024),
            surface: "fixture",
        }
    }

    #[test]
    fn terminal_groups_ignore_late_ready_and_heartbeat_events() {
        for phase in ["exited", "failed", "stopped"] {
            let mut state = state(vec![Group {
                phase,
                ..Group::default()
            }]);
            apply_event(
                &mut state,
                Event::Ready {
                    index: 0,
                    port: Some(12345),
                },
                100,
            );
            apply_event(&mut state, Event::Heartbeat { index: 0 }, 200);
            assert_eq!(state.groups[0].phase, phase);
            assert!(!state.groups[0].host_reports_ready);
            assert_eq!(state.groups[0].last_heartbeat_ms, None);
        }
    }

    #[test]
    fn stopped_state_clears_live_gauges_and_keeps_failure_evidence() {
        let mut state = state(vec![
            Group {
                phase: "running",
                pid: Some(42),
                host_reports_ready: true,
                rss_bytes: Some(4096),
                rss_peak_bytes: 8192,
                cpu_percent_one_core: Some(5.0),
                last_heartbeat_ms: Some(10),
                ..Group::default()
            },
            Group {
                phase: "failed",
                exit_code: Some(42),
                ..Group::default()
            },
            Group {
                phase: "starting",
                ..Group::default()
            },
        ]);
        mark_stopped(&mut state);
        assert!(!state.ready);
        assert_eq!(state.phase, "stopped");
        assert_eq!(state.groups[0].phase, "stopped");
        assert_eq!(state.groups[0].rss_peak_bytes, 8192);
        assert_eq!(state.groups[1].phase, "failed");
        assert_eq!(state.groups[1].exit_code, Some(42));
        assert_eq!(state.groups[2].phase, "stopped");
        assert!(state.groups.iter().all(|group| group.pid.is_none()
            && group.rss_bytes.is_none()
            && group.cpu_percent_one_core.is_none()
            && group.last_heartbeat_ms.is_none()
            && !group.host_reports_ready));
    }

    #[tokio::test]
    async fn cancelled_plan_cannot_launch_an_executable() {
        let cancel = CancellationToken::new();
        cancel.cancel();
        let error = match plan(
            &PathBuf::from("fixture-does-not-exist"),
            &PathBuf::from("fixture-config"),
            &cancel,
        )
        .await
        {
            Ok(_) => panic!("cancelled planner must fail"),
            Err(error) => error,
        };
        assert_eq!(error.to_string(), "planning cancelled");
    }

    #[test]
    fn master_plan_has_no_business_types_and_rejects_duplicate_or_reserved_groups() {
        let p = Plan {
            mode: "grouped".into(),
            groups: vec![Planned {
                id: "one".into(),
                modules: vec!["anything".into()],
            }],
        };
        assert!(p.validate().is_ok());
        let p = Plan {
            mode: "grouped".into(),
            groups: vec![Planned {
                id: "transport".into(),
                modules: vec![],
            }],
        };
        assert!(p.validate().is_err());
    }
}
