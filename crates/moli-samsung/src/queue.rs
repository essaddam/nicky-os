//! At most four queued callers and one in flight. No network in the receiver loop.
use crate::CommandSink;
use futures::FutureExt as _;
use moli_core::DeviceId;
use moli_runtime::{CommandRequest, DriverCtx};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::task::JoinSet;

#[cfg(test)]
pub(crate) async fn commands(ctx: &mut DriverCtx, id: &DeviceId, sink: Arc<dyn CommandSink>) {
    serve(ctx, id, sink, None).await;
}

/// The supervisor may drop the whole driver future on cancellation. Code
/// after `serve().await` is not a cleanup guarantee; this guard is.
struct OfflineOnDrop<'a> {
    ctx: &'a mut DriverCtx,
    id: DeviceId,
}
impl Drop for OfflineOnDrop<'_> {
    fn drop(&mut self) {
        self.ctx.set_availability(&self.id, false);
        for key in ["power", "volume", "mute"] {
            self.ctx.set_state(&self.id, key, moli_core::Value::Null);
        }
    }
}

type Polling = (Arc<moli_net::PinnedHttps>, crate::Config, String);
pub(crate) async fn serve(
    ctx: &mut DriverCtx,
    id: &DeviceId,
    sink: Arc<dyn CommandSink>,
    polling: Option<Polling>,
) {
    let cleanup = OfflineOnDrop {
        ctx,
        id: id.clone(),
    };
    let ctx = &mut *cleanup.ctx;
    let mut readings = JoinSet::new();
    let mut poll_at = tokio::time::Instant::now();
    let mut queue: VecDeque<CommandRequest> = VecDeque::new();
    let mut active = JoinSet::new();
    loop {
        if ctx.cancelled().now_or_never().is_some() {
            break;
        }
        queue.retain(|c| !c.abandoned());
        if active.is_empty()
            && let Some(c) = queue.pop_front()
        {
            let sink = sink.clone();
            let publisher = ctx.media_publisher();
            let from = ctx.instance().to_string();
            active.spawn(async move {
                if c.abandoned() {
                    c.reply(Err(moli_i18n::tr!("pilotes.samsung.expire")));
                    return;
                }
                let result = tokio::time::timeout_at(c.deadline.into(), sink.send(&c))
                    .await
                    .unwrap_or_else(|_| Err(moli_i18n::tr!("pilotes.samsung.non_confirme")));
                if c.abandoned()
                    && let Err(e) = &result
                {
                    publisher.tell(&from, e.clone());
                }
                c.reply(result);
            });
        }
        let next = queue.iter().map(|c| c.deadline).min();
        tokio::select! {
            command=ctx.next_command()=>match command {
                None=>break,
                Some(c) if queue.len()>=4 =>c.reply(Err(moli_i18n::tr!("pilotes.samsung.occupe"))),
                Some(c)=>queue.push_back(c),
            },
            _=active.join_next(),if !active.is_empty()=>{},
            ()=until(next)=>{},
            ()=tokio::time::sleep_until(poll_at),if polling.is_some() && readings.is_empty()=>{
                let (http,config,native)=polling.clone().expect("polling configured");
                readings.spawn(async move {
                    tokio::time::timeout(std::time::Duration::from_secs(12),crate::polling::poll(&http,&config.host,config.upnp_port,&native)).await
                        .map_err(anyhow::Error::from).and_then(|r|r)
                });
            },
            result=readings.join_next(),if !readings.is_empty()=>{
                poll_at=tokio::time::Instant::now()+std::time::Duration::from_secs(polling.as_ref().map_or(30,|(_,c,_)|c.every));
                match result {
                    Some(Ok(Ok(values)))=>{
                        ctx.ready();ctx.set_availability(id,true);
                        for key in ["power","volume","mute"] {
                            let value=values.iter().find(|(k,_)|*k==key).map_or(moli_core::Value::Null,|(_,v)|v.clone());
                            ctx.set_state(id,key,value);
                        }
                    },
                    other=>{
                        ctx.set_availability(id,false);
                        for key in ["power","volume","mute"] {ctx.set_state(id,key,moli_core::Value::Null);}
                        if format!("{other:?}").contains(moli_net::PIN_MISMATCH) {ctx.wait_for(moli_i18n::tr!("pilotes.samsung.certificat",instance=ctx.instance().as_str()));}
                    },
                }
            },
        }
    }
    readings.abort_all();
    while readings.join_next().await.is_some() {}
    active.abort_all();
    while active.join_next().await.is_some() {}
    for c in queue {
        c.reply(Err(moli_i18n::tr!("pilotes.samsung.arret")));
    }
    ctx.set_availability(id, false);
}
async fn until(at: Option<std::time::Instant>) {
    if let Some(at) = at {
        tokio::time::sleep_until(at.into()).await;
    } else {
        std::future::pending::<()>().await;
    }
}
