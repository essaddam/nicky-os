//! Cancellable cycles, finish-based cadences and no deferred read queue.
use crate::{
    Config, Version,
    api::{Budget, Client, Wire},
    model::{self, Read},
    network::Network,
    protocol::Action,
    publication::Published,
    xml::Failure,
};
use moli_runtime::DriverCtx;
use std::future::{Future, pending};
use std::time::Duration;
use tokio::time::Instant;

fn message(error: Failure) -> String {
    match error {
        Failure::Refused | Failure::Locked => {
            moli_i18n::tr!("pilotes.netgear.authentification_refusee")
        }
        Failure::Certificate => moli_i18n::tr!("pilotes.netgear.certificat_modifie"),
        Failure::Identity => moli_i18n::tr!("pilotes.netgear.identite_modifiee"),
        Failure::Secrets => moli_i18n::tr!("pilotes.netgear.secret_manquant"),
        Failure::Vault => moli_i18n::tr!("pilotes.netgear.coffre_indisponible"),
        _ => moli_i18n::tr!("pilotes.netgear.reponse_invalide"),
    }
}
async fn drive<T>(ctx: &mut DriverCtx, future: impl Future<Output = T>) -> Option<T> {
    tokio::pin!(future);
    loop {
        tokio::select! {
            biased;
            command=ctx.next_command()=>match command {Some(command)=>command.reply(Err(moli_i18n::tr!("pilotes.netgear.lecture_seule"))),None=>return None},
            output=&mut future=>return Some(output),
        }
    }
}
async fn waiting(ctx: &mut DriverCtx, error: Failure) {
    ctx.wait_for(message(error));
    let _ = drive(ctx, pending::<()>()).await;
}
async fn prepare(
    config: &Config,
    ctx: &DriverCtx,
) -> Result<(Network, String, String, Option<String>), Failure> {
    if !ctx.can_store_secrets() {
        return Err(Failure::Vault);
    }
    let username = ctx.secret("username").ok_or(Failure::Secrets)?;
    let password = ctx.secret("password").ok_or(Failure::Secrets)?;
    if [&username, &password]
        .iter()
        .any(|s| s.is_empty() || s.len() > 1024 || s.chars().any(char::is_control))
    {
        return Err(Failure::Refused);
    }
    let identity = ctx.secret("device_identity");
    if identity.as_deref().is_some_and(|s| !model::identity(s)) {
        return Err(Failure::Identity);
    }
    let old_pin = ctx.secret("cert_sha256");
    if old_pin
        .as_ref()
        .is_some_and(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(Failure::Certificate);
    }
    let (network, pin) = Network::open(config, old_pin.clone()).await?;
    // This durable write must finish before the client can send credentials.
    if let Some(pin) = pin.filter(|_| old_pin.is_none()) {
        ctx.store_secret("cert_sha256", &pin)
            .await
            .map_err(|_| Failure::Vault)?;
    }
    Ok((network, username, password, identity))
}
async fn cycle(
    client: &mut Client,
    wire: &impl Wire,
    credentials: (&str, &str),
    actions: Vec<Action>,
) -> Result<Vec<(Action, Option<Read>)>, Failure> {
    let mut budget = Budget::default();
    let mut reads = Vec::new();
    for action in actions {
        reads.push((
            action,
            client.read(wire, action, credentials, &mut budget).await?,
        ));
    }
    // If every optional method was removed, retain a bounded liveness read.
    if budget.sent == 0 {
        reads.push((
            Action::Info,
            client
                .read(wire, Action::Info, credentials, &mut budget)
                .await?,
        ));
    }
    Ok(reads)
}
async fn timed_cycle(
    client: &mut Client,
    wire: &impl Wire,
    credentials: (&str, &str),
    actions: Vec<Action>,
) -> Result<Vec<(Action, Option<Read>)>, Failure> {
    tokio::time::timeout(
        Duration::from_secs(30),
        cycle(client, wire, credentials, actions),
    )
    .await
    .unwrap_or(Err(Failure::Budget))
}
pub(crate) async fn run(config: &Config, ctx: &mut DriverCtx) -> anyhow::Result<()> {
    let mut state = Published::new(ctx);
    if let Err(error) = state.reconcile(&config.selected) {
        waiting(state.ctx, error).await;
        return Ok(());
    }
    let prepared = tokio::select! {biased;()=state.ctx.cancelled()=>return Ok(()),result=prepare(config,state.ctx)=>result};
    let (wire, username, password, identity) = match prepared {
        Ok(value) => value,
        Err(Failure::Transport) => {
            return Err(anyhow::anyhow!(message(Failure::Transport)));
        }
        Err(error) => {
            waiting(state.ctx, error).await;
            return Ok(());
        }
    };
    let mut persisted = identity.is_some();
    let mut client = Client::new(config.login, identity);
    let mut next_info = Instant::now();
    let mut next_inventory = Instant::now();
    loop {
        let now = Instant::now();
        let info_due = now >= next_info;
        let inventory_due = now >= next_inventory && !config.selected.is_empty();
        let mut actions = Vec::new();
        if info_due {
            actions.push(Action::Info);
        }
        actions.extend([Action::System, Action::Ethernet, Action::Traffic]);
        if inventory_due {
            actions.push(if config.inventory == Version::V1 {
                Action::ClientsV1
            } else {
                Action::ClientsV2
            });
        }
        let Some(result) = drive(
            state.ctx,
            timed_cycle(&mut client, &wire, (&username, &password), actions),
        )
        .await
        else {
            return Ok(());
        };
        let reads = match result {
            Ok(reads) => reads,
            Err(error) => {
                state.offline();
                if matches!(
                    error,
                    Failure::Refused | Failure::Locked | Failure::Identity | Failure::Certificate
                ) {
                    waiting(state.ctx, error).await;
                    return Ok(());
                }
                return Err(anyhow::anyhow!(message(error)));
            }
        };
        let identity = client
            .identity
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!(message(Failure::Identity)))?;
        if let Err(error) = state.admit_router(identity) {
            waiting(state.ctx, error).await;
            return Ok(());
        }
        if !persisted {
            let result = tokio::select! {biased;()=state.ctx.cancelled()=>return Ok(()),result=state.ctx.store_secret("device_identity",identity)=>result};
            if result.is_err() {
                waiting(state.ctx, Failure::Vault).await;
                return Ok(());
            }
            persisted = true;
        }
        if let Err(error) = state.apply(identity, &config.selected, reads) {
            waiting(state.ctx, error).await;
            return Ok(());
        }
        state.ctx.ready();
        let finished = Instant::now();
        if info_due {
            next_info = finished + Duration::from_secs(config.info_every);
        }
        if inventory_due {
            next_inventory = finished + Duration::from_secs(config.inventory_every);
        }
        if drive(
            state.ctx,
            tokio::time::sleep(Duration::from_secs(config.every)),
        )
        .await
        .is_none()
        {
            return Ok(());
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{api::Answer, protocol};
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Slow {
        sent: AtomicUsize,
    }
    impl Wire for Slow {
        fn host(&self) -> &'static str {
            "router.example"
        }
        fn https(&self) -> bool {
            true
        }
        async fn send(
            &self,
            action: Action,
            _body: String,
            _cookie: Option<String>,
        ) -> Result<Answer, Failure> {
            let count = self.sent.fetch_add(1, Ordering::SeqCst) + 1;
            tokio::time::sleep(Duration::from_secs(4)).await;
            let code = if count == 3 { 401 } else { 0 };
            let fields = if action == Action::Info {
                "<SerialNumber>synthetic-router-01</SerialNumber>"
            } else {
                ""
            };
            let mut headers = http::HeaderMap::new();
            if action == Action::LoginV2 {
                headers.insert("set-cookie", format!("sid={count}").parse().unwrap());
            }
            Ok(Answer {
                status: http::StatusCode::OK,
                headers,
                body: protocol::response(action, fields, code).into_bytes(),
            })
        }
    }
    #[tokio::test(start_paused = true)]
    async fn the_entire_cycle_times_out_even_when_each_exchange_is_under_five_seconds() {
        let wire = Slow {
            sent: AtomicUsize::new(0),
        };
        let mut client = Client::new(Version::V2, None);
        let start = Instant::now();
        let generated = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .to_string();
        let result = timed_cycle(
            &mut client,
            &wire,
            (&generated, &generated),
            vec![
                Action::Info,
                Action::System,
                Action::Ethernet,
                Action::Traffic,
            ],
        )
        .await;
        assert_eq!(result.unwrap_err(), Failure::Budget);
        assert_eq!(start.elapsed(), Duration::from_secs(30));
        assert_eq!(wire.sent.load(Ordering::SeqCst), 8);
    }
}
