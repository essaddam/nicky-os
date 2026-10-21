//! The hub loop never waits for discovery, TLS, storage or an appliance.
use crate::{
    Cast,
    model::{self, Observed},
    transport::{Guard, Wire},
};
use moli_androidtv::{cast::ReceiverOrder, tracked::Delivery};
use moli_core::Value;
use moli_runtime::{CommandRequest, DriverCtx};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio::sync::watch;
use tokio::time::Instant;
struct Task(tokio::task::JoinHandle<()>);
impl Drop for Task {
    fn drop(&mut self) {
        self.0.abort();
    }
}
struct Pending {
    command: CommandRequest,
    order: ReceiverOrder,
    activation: u64,
    epoch: u64,
    media_epoch: Option<u64>,
    bound: Guard,
}
fn unavailable() -> String {
    moli_i18n::tr!("pilotes.cast.indisponible")
}
fn refuse(queue: &mut VecDeque<Pending>) {
    for pending in queue.drain(..) {
        pending.command.reply(Err(unavailable()));
    }
}
fn publish(ctx: &DriverCtx, id: &moli_core::DeviceId, state: &Observed) {
    ctx.set_availability(id, state.online);
    ctx.set_state(
        id,
        "volume",
        state
            .volume
            .level
            .map_or(Value::Null, |level| Value::Float((level * 100.0).round())),
    );
    ctx.set_state(
        id,
        "mute",
        state.volume.muted.map_or(Value::Null, Value::Bool),
    );
    for (key, value) in [
        ("media_app", &state.playing.app),
        ("media_state", &state.playing.state),
        ("media_title", &state.playing.title),
        ("media_subtitle", &state.playing.subtitle),
        ("media_image", &state.playing.image),
    ] {
        ctx.set_state(id, key, value.as_deref().map_or(Value::Null, Value::from));
    }
    for (key, value) in [
        ("media_duration", state.playing.duration),
        ("media_position", state.playing.position),
    ] {
        ctx.set_state(
            id,
            key,
            value
                .filter(|v| v.is_finite() && *v >= 0.0)
                .map_or(Value::Null, Value::Float),
        );
    }
}
pub(crate) async fn run(receiver: &Cast, ctx: &mut DriverCtx) -> anyhow::Result<()> {
    for id in ctx.devices() {
        ctx.remove_device(&id);
    }
    let gateway = receiver.gateway.clone();
    let vault = Arc::new(ctx.scoped_secrets());
    let discover = receiver.discover.clone();
    let mut gateway_task = Task(tokio::spawn(async move {
        gateway.run(vault, discover).await;
    }));
    let mut selected = receiver.gateway.selected();
    let mut active = None;
    let mut current_id = None;
    let mut connection: Option<Task> = None;
    let mut wire: Option<Arc<dyn Wire>> = None;
    let mut events = None;
    let mut observed = Observed::default();
    let (states, reader) = watch::channel(observed.clone());
    let mut queue = VecDeque::<Pending>::new();
    let mut jobs = tokio::task::JoinSet::new();
    ctx.wait_for(moli_i18n::tr!("pilotes.cast.validation"));
    loop {
        let target = selected.borrow_and_update().clone();
        if target.as_ref().map(|t| t.generation) != active {
            connection.take();
            wire = None;
            events = None;
            refuse(&mut queue);
            if let Some(id) = current_id.take() {
                ctx.remove_device(&id);
            }
            observed = Observed::default();
            states.send_replace(observed.clone());
            active = target.as_ref().map(|t| t.generation);
            if let Some(target) = target {
                let device = model::device(ctx.instance(), &target.candidate);
                let id = device.id.clone();
                ctx.upsert_device(device);
                publish(ctx, &id, &observed);
                current_id = Some(id);
                let link = receiver.connector.open(target, receiver.gateway.clone());
                wire = Some(link.wire);
                events = Some(link.events);
                connection = Some(Task(tokio::spawn(link.run)));
            } else {
                ctx.wait_for(moli_i18n::tr!("pilotes.cast.validation"));
            }
        }
        queue.retain(|pending| !pending.command.abandoned());
        if jobs.is_empty() {
            while let Some(pending) = queue.pop_front() {
                let Some(wire) = wire.as_ref().filter(|_| {
                    active == Some(pending.activation)
                        && pending.epoch == observed.epoch
                        && observed.online
                        && !(pending.bound)()
                        && pending
                            .media_epoch
                            .is_none_or(|e| e == observed.media_epoch)
                }) else {
                    pending.command.reply(Err(unavailable()));
                    continue;
                };
                jobs.spawn(execute(
                    wire.clone(),
                    selected.clone(),
                    reader.clone(),
                    pending,
                ));
                break;
            }
        }
        let expiration = queue
            .iter()
            .map(|p| Instant::from_std(p.command.deadline))
            .min();
        tokio::select! {
            command=ctx.next_command()=>{
                let Some(command)=command else {refuse(&mut queue);return Ok(());};
                if queue.len()+jobs.len()>=4 {command.reply(Err(moli_i18n::tr!("pilotes.androidtv.file_pleine")));continue;}
                let order=match model::plan(&command.key,&command.value){Ok(order)=>order,Err(error)=>{command.reply(Err(error));continue;}};
                let Some((activation,wire))=active.zip(wire.as_ref()).filter(|_|observed.online) else {command.reply(Err(unavailable()));continue;};
                if matches!(order,ReceiverOrder::Media(_)) && observed.playing.state.is_none(){command.reply(Err(unavailable()));continue;}
                let Some(bound)=wire.bind(matches!(order, ReceiverOrder::Media(_))) else {command.reply(Err(unavailable()));continue;};
                queue.push_back(Pending{command,order,activation,epoch:observed.epoch,media_epoch:matches!(order,ReceiverOrder::Media(_)).then_some(observed.media_epoch),bound});
            }
            event=async {if let Some(events)=events.as_mut(){events.recv().await}else{std::future::pending().await}}=>{
                let Some(event)=event else {anyhow::bail!(unavailable());};observed.apply(event);states.send_replace(observed.clone());
                if let Some(id)=&current_id {publish(ctx,id,&observed);}
                if observed.online {ctx.ready();}else {ctx.wait_for(unavailable());refuse(&mut queue);}
            }
            changed=selected.changed()=>{if changed.is_err(){anyhow::bail!(unavailable());}}
            _=jobs.join_next(),if !jobs.is_empty()=>{}
            _=&mut gateway_task.0=>{anyhow::bail!(unavailable());}
            ()=async {if let Some(deadline)=expiration{tokio::time::sleep_until(deadline).await;}else{std::future::pending().await}}=>{}
        }
    }
}
fn execute(
    wire: Arc<dyn Wire>,
    selected: watch::Receiver<Option<Arc<crate::gateway::Approved>>>,
    state: watch::Receiver<Observed>,
    pending: Pending,
) -> impl std::future::Future<Output = ()> + Send {
    let deadline = Instant::from_std(pending.command.deadline);
    let original = Arc::new(Mutex::new(Some(pending.command)));
    let holder = original.clone();
    let abandoned: Guard = Arc::new(move || {
        let observed = state.borrow();
        (pending.bound)()
            || !observed.online
            || observed.epoch != pending.epoch
            || pending
                .media_epoch
                .is_some_and(|e| e != observed.media_epoch)
            || selected
                .borrow()
                .as_ref()
                .is_none_or(|target| target.generation != pending.activation)
            || holder
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_ref()
                .is_none_or(CommandRequest::abandoned)
    });
    async move {
        let delivery = wire.send(pending.order, deadline, abandoned).await;
        let result = match delivery {
            Delivery::Sent | Delivery::Acknowledged => Ok(()),
            Delivery::Refused => Err(moli_i18n::tr!("pilotes.cast.refusee")),
            Delivery::Uncertain => Err(moli_i18n::tr!("pilotes.cast.incertain")),
            _ => Err(unavailable()),
        };
        if let Some(command) = original
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            command.reply(result);
        }
    }
}
