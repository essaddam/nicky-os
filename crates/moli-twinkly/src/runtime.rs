//! Bounded FIFO and one network job, beside the loop that answers deadlines.
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use moli_core::{Device, Value};
use moli_runtime::{CommandRequest, DriverCtx};
use tokio::task::JoinSet;
use tokio::time::Instant;

use crate::api::{Client, Failure, Snapshot};
use crate::model::{self, Capabilities, Mode};
use crate::{Config, ResumeMode};

const IDLE: Duration = Duration::from_secs(15);
const RETRY: Duration = Duration::from_secs(60);
const MAX_ORDERS: usize = 8;
type Gate = Arc<Mutex<Option<CommandRequest>>>;

fn message(error: Failure) -> String {
    let key = match error {
        Failure::NotSent => "pilotes.twinkly.non_envoye",
        Failure::Identity => "pilotes.twinkly.identite_differente",
        Failure::Session => "pilotes.twinkly.session_refusee",
        Failure::Expired => "pilotes.twinkly.session_expiree",
        Failure::Refused => "pilotes.twinkly.refus",
        Failure::Unavailable => "pilotes.twinkly.capacite_absente",
        Failure::Partial => "pilotes.twinkly.non_confirme",
        Failure::Transport => "pilotes.twinkly.injoignable",
        Failure::Protocol(key) => key,
    };
    moli_i18n::tr(key)
}

fn wanted(gate: &Gate) -> bool {
    gate.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .is_some_and(|command| !command.abandoned())
}

/// Consume the reply once. Even a departed caller's answer is kept by
/// the hub for its journal. Returns whether an error needs a notice too.
fn answer(gate: &Gate, outcome: Result<(), Failure>) -> bool {
    let command = gate.lock().unwrap_or_else(PoisonError::into_inner).take();
    let late = command.as_ref().is_none_or(CommandRequest::abandoned);
    if let Some(command) = command {
        command.reply(outcome.map_err(message));
    }
    late && outcome.is_err()
}

struct Finished {
    client: Client,
    observed: Option<Result<Snapshot, Failure>>,
    before: Option<(Mode, Capabilities)>,
    notice: Option<Failure>,
}

async fn order(mut client: Client, gate: Gate, resume: Option<Mode>) -> Finished {
    let input = gate
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .map(|command| (command.key.clone(), command.value.clone()));
    let Some((key, value)) = input else {
        return Finished {
            client,
            observed: None,
            before: None,
            notice: None,
        };
    };
    let before = match client.read(|| wanted(&gate)).await {
        Ok(state) => state,
        Err(error) => {
            let late = answer(&gate, Err(error));
            return Finished {
                client,
                observed: (error != Failure::NotSent).then_some(Err(error)),
                before: None,
                notice: late.then_some(error),
            };
        }
    };
    let remembered = Some((before.mode, before.caps));
    let steps = match model::plan(&key, &value, before.mode, resume, before.caps) {
        Ok(steps) => steps,
        Err(key) => {
            let error = Failure::Protocol(key);
            let late = answer(&gate, Err(error));
            return Finished {
                client,
                observed: Some(Ok(before)),
                before: remembered,
                notice: late.then_some(error),
            };
        }
    };
    if steps.is_empty() {
        answer(&gate, Ok(()));
        return Finished {
            client,
            observed: Some(Ok(before)),
            before: remembered,
            notice: None,
        };
    }
    let outcome = client.apply(&steps, || wanted(&gate)).await;
    // A lost response after entering the write path cannot prove no effect.
    let outcome = outcome.map_err(|e| {
        if e == Failure::Transport {
            Failure::Partial
        } else {
            e
        }
    });
    let late = answer(&gate, outcome);
    // State observation is independent of the caller that requested a write.
    // Never replay the steps, even if this read needs a new session.
    let observed = client.read(|| true).await;
    Finished {
        client,
        observed: Some(observed),
        before: remembered,
        notice: if late { outcome.err() } else { None },
    }
}

struct Loop {
    native: String,
    client: Option<Client>,
    queue: VecDeque<CommandRequest>,
    jobs: JoinSet<Finished>,
    active: Option<Gate>,
    next: Instant,
    observed_resume: Option<Mode>,
    configured_resume: Option<Mode>,
}

impl Loop {
    fn new(config: &Config, native: &str) -> Self {
        let configured_resume = config.resume_mode.map(|mode| match mode {
            ResumeMode::Color => Mode::Color,
            ResumeMode::Movie => Mode::Movie,
            ResumeMode::Playlist => Mode::Playlist,
        });
        Self {
            native: native.to_owned(),
            client: Some(Client::new(config.host, config.port, native.to_owned())),
            queue: VecDeque::new(),
            jobs: JoinSet::new(),
            active: None,
            next: Instant::now(),
            observed_resume: None,
            configured_resume,
        }
    }

    fn prune(&mut self) {
        let count = self.queue.len();
        for _ in 0..count {
            if let Some(command) = self.queue.pop_front() {
                if command.abandoned() {
                    command.reply(Err(message(Failure::NotSent)));
                } else {
                    self.queue.push_back(command);
                }
            }
        }
        if let Some(gate) = &self.active {
            let due = {
                let mut slot = gate.lock().unwrap_or_else(PoisonError::into_inner);
                if slot
                    .as_ref()
                    .is_some_and(|c| std::time::Instant::now() >= c.deadline)
                {
                    slot.take()
                } else {
                    None
                }
            };
            if let Some(command) = due {
                command.reply(Err(message(Failure::Partial)));
            }
        }
    }

    fn admit(&mut self, ctx: &DriverCtx, command: CommandRequest) {
        self.prune();
        if command.device.id != ctx.device_id(&self.native) {
            command.reply(Err(message(Failure::Identity)));
        } else if self.queue.len() + usize::from(self.active.is_some()) >= MAX_ORDERS {
            command.reply(Err(moli_i18n::tr!("pilotes.twinkly.file_pleine")));
        } else {
            self.queue.push_back(command);
        }
    }

    fn pump(&mut self) {
        if !self.jobs.is_empty() {
            return;
        }
        if let Some(command) = self.queue.pop_front() {
            if let Some(client) = self.client.take() {
                let gate = Arc::new(Mutex::new(Some(command)));
                self.active = Some(Arc::clone(&gate));
                self.jobs.spawn(order(
                    client,
                    gate,
                    self.observed_resume.or(self.configured_resume),
                ));
            }
        } else if Instant::now() >= self.next
            && let Some(mut client) = self.client.take()
        {
            self.jobs.spawn(async move {
                let observed = client.read(|| true).await;
                Finished {
                    client,
                    observed: Some(observed),
                    before: None,
                    notice: None,
                }
            });
        }
    }

    fn remember(&mut self, mode: Mode, caps: Capabilities) {
        match mode {
            Mode::Movie | Mode::Playlist => self.observed_resume = Some(mode),
            Mode::Color if caps.color => self.observed_resume = Some(mode),
            Mode::Color | Mode::Other => self.observed_resume = None,
            Mode::Off => {}
        }
    }

    fn done(&mut self, ctx: &DriverCtx, done: Finished) {
        self.client = Some(done.client);
        self.active = None;
        if let Some((mode, caps)) = done.before {
            self.remember(mode, caps);
        }
        if let Some(error) = done.notice {
            ctx.media_publisher().tell("Twinkly", message(error));
        }
        self.next = Instant::now();
        match done.observed {
            Some(Ok(state)) => {
                self.remember(state.mode, state.caps);
                publish(ctx, state);
                ctx.ready();
                self.next += IDLE;
            }
            Some(Err(error)) => {
                for id in ctx.known_devices() {
                    ctx.set_availability(&id, false);
                }
                ctx.wait_for(message(error));
                self.next += RETRY;
            }
            None => {}
        }
    }

    fn wake(&self) -> Option<Instant> {
        let active = self.active.as_ref().and_then(|gate| {
            gate.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .as_ref()
                .map(|c| Instant::from_std(c.deadline))
        });
        self.queue
            .iter()
            .map(|c| Instant::from_std(c.deadline))
            .chain(active)
            .chain(self.jobs.is_empty().then_some(self.next))
            .min()
    }

    async fn serve(&mut self, ctx: &mut DriverCtx) -> anyhow::Result<()> {
        loop {
            self.prune();
            self.pump();
            let wake = self.wake();
            tokio::select! {
                command=ctx.next_command()=>match command {
                    Some(command)=>self.admit(ctx,command),
                    None=>return Ok(()),
                },
                Some(done)=self.jobs.join_next()=>self.done(ctx,done?),
                ()=sleep(wake)=>{},
            }
        }
    }
}

async fn sleep(at: Option<Instant>) {
    if let Some(at) = at {
        tokio::time::sleep_until(at).await;
    } else {
        std::future::pending::<()>().await;
    }
}

fn publish(ctx: &DriverCtx, state: Snapshot) {
    let id = ctx.device_id(&state.info.native);
    ctx.upsert_device(Device {
        id: id.clone(),
        instance: ctx.instance().clone(),
        native_name: state.info.name.into(),
        manufacturer: Some("Twinkly".into()),
        model: Some(state.info.model.into()),
        description: Some(moli_i18n::tr!("pilotes.twinkly.lampe").into()),
        native_room: None,
        members: Vec::new(),
        points: model::points(state.caps),
    });
    ctx.set_state(&id, "on", Value::Bool(state.mode != Mode::Off));
    if state.caps.brightness {
        ctx.set_state(&id, "brightness", Value::Int(i64::from(state.brightness)));
    }
    if state.caps.color {
        let value = match (state.mode, state.color) {
            (Mode::Color, Some([r, g, b])) => {
                Value::from(format!("#{r:02x}{g:02x}{b:02x}").as_str())
            }
            _ => Value::Null,
        };
        ctx.set_state(&id, "color", value);
    }
    ctx.set_availability(&id, true);
}

pub(crate) async fn run(config: &Config, native: &str, ctx: &mut DriverCtx) -> anyhow::Result<()> {
    for id in ctx.known_devices() {
        ctx.set_availability(&id, false);
    }
    Loop::new(config, native).serve(ctx).await
}
