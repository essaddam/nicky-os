//! The driver loop handles hub requests without awaiting device or vault I/O.
use crate::{
    AndroidTv, commands,
    model::{self, Observation},
    transport::{self, Abandoned, Wire},
};
use moli_core::Value;
use moli_runtime::{CommandRequest, DriverCtx};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio::sync::watch;
use tokio::time::Instant;

#[path = "execution.rs"]
mod execution;

fn publish(ctx: &DriverCtx, observed: &Observation) {
    let id = ctx.device_id("tv");
    ctx.upsert_device(model::device(
        ctx.instance(),
        observed.volume.map(|volume| volume.max),
    ));
    ctx.set_availability(&id, observed.online);
    ctx.set_state(
        &id,
        "power",
        observed.power.map_or(Value::Null, Value::Bool),
    );
    ctx.set_state(
        &id,
        "app",
        observed.app.as_deref().map_or(Value::Null, Value::from),
    );
    ctx.set_state(
        &id,
        "volume",
        observed
            .volume
            .and_then(|volume| i64::try_from(volume.level).ok())
            .map_or(Value::Null, Value::Int),
    );
    ctx.set_state(
        &id,
        "mute",
        observed
            .volume
            .map_or(Value::Null, |volume| Value::Bool(volume.muted)),
    );
    let playing = &observed.playing;
    for (key, value) in [
        ("media_app", &playing.app),
        ("media_state", &playing.state),
        ("media_title", &playing.title),
        ("media_subtitle", &playing.subtitle),
        ("media_image", &playing.image),
    ] {
        ctx.set_state(&id, key, value.as_deref().map_or(Value::Null, Value::from));
    }
    for (key, value) in [
        ("media_duration", playing.duration),
        ("media_position", playing.position),
    ] {
        ctx.set_state(
            &id,
            key,
            value
                .filter(|n| n.is_finite() && *n >= 0.0)
                .map_or(Value::Null, Value::Float),
        );
    }
}
fn refuse(queue: &mut VecDeque<CommandRequest>) {
    for command in queue.drain(..) {
        command.reply(Err(moli_i18n::tr!("pilotes.androidtv.indisponible")));
    }
}
pub(crate) async fn run(tv: &AndroidTv, ctx: &mut DriverCtx) -> anyhow::Result<()> {
    let vault = ctx.scoped_secrets();
    let mut background = tokio::task::JoinSet::new();
    let gateway = tv.gateway.clone();
    let factory = gateway.network_factory(tv.config.host.clone());
    let pair_vault = Arc::new(vault.clone());
    background.spawn(async move {
        gateway.run(pair_vault, factory).await;
    });
    let transport::Link {
        wire,
        mut events,
        run,
    } = tv.connector.open(
        tv.config.host.clone(),
        vault,
        tv.gateway.credentials(),
        tv.gateway.writes(),
    );
    background.spawn(run);
    let mut observed = Observation::default();
    let (observations, reader) = watch::channel(observed.clone());
    let mut queue = VecDeque::<CommandRequest>::new();
    let mut jobs = tokio::task::JoinSet::new();
    publish(ctx, &observed);
    ctx.wait_for(moli_i18n::tr!("pilotes.androidtv.appairage"));
    loop {
        queue.retain(|command| !command.abandoned());
        if jobs.is_empty() {
            while let Some(command) = queue.pop_front() {
                if command.abandoned() {
                    command.reply(Err(moli_i18n::tr!("pilotes.androidtv.delai")));
                    continue;
                }
                if !observed.online {
                    command.reply(Err(moli_i18n::tr!("pilotes.androidtv.indisponible")));
                    continue;
                }
                let plan = match commands::plan(&command.key, &command.value, &observed) {
                    Ok(plan) => plan,
                    Err(error) => {
                        command.reply(Err(error));
                        continue;
                    }
                };
                let wire = wire.clone();
                let reader = reader.clone();
                jobs.spawn(async move {
                    execute(wire, reader, command, plan).await;
                });
                break;
            }
        }
        let expiration = queue
            .iter()
            .map(|command| Instant::from_std(command.deadline))
            .min();
        tokio::select! {
            command=ctx.next_command()=> {
                let Some(command)=command else {refuse(&mut queue);return Ok(());};
                // One FIFO includes the running request and pending confirmations.
                if queue.len()+jobs.len()>=4 {command.reply(Err(moli_i18n::tr!("pilotes.androidtv.file_pleine")));}
                else {queue.push_back(command);}
            }
            event=events.recv()=> {
                let Some(event)=event else {anyhow::bail!(moli_i18n::tr!("pilotes.androidtv.indisponible"));};
                match event {transport::Event::Remote(event)=> {
                    if matches!(event,moli_androidtv::remote::Event::Lost) {refuse(&mut queue);}
                    observed.remote(event);
                },transport::Event::Cast(event)=>observed.cast(event)}
                observations.send_replace(observed.clone());publish(ctx,&observed);
                if observed.online {ctx.ready();}
            }
            _=jobs.join_next(),if !jobs.is_empty()=>{}
            _=background.join_next(),if !background.is_empty()=>{anyhow::bail!(moli_i18n::tr!("pilotes.androidtv.indisponible"));}
            ()=async { if let Some(deadline)=expiration {tokio::time::sleep_until(deadline).await;} else {std::future::pending::<()>().await;} }=> {
                queue.retain(|command|!command.abandoned());
            }
        }
    }
}
async fn execute(
    wire: Arc<dyn Wire>,
    observed: watch::Receiver<Observation>,
    command: CommandRequest,
    plan: commands::Plan,
) {
    let deadline = Instant::from_std(command.deadline);
    let original = Arc::new(Mutex::new(Some(command)));
    let epoch = observed.borrow().epoch;
    let holder = original.clone();
    let state = observed.clone();
    let abandoned: Abandoned = Arc::new(move || {
        let current = state.borrow();
        !current.online
            || current.epoch != epoch
            || holder
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_ref()
                .is_none_or(CommandRequest::abandoned)
    });
    let result = execution::run(wire, observed, deadline, abandoned, plan).await;
    if let Some(command) = original
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
    {
        command.reply(result);
    }
}
