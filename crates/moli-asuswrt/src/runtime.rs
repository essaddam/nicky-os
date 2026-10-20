//! Cancellable cycles, finish-based cadences and no deferred read queue.
use crate::{
    Config, Failure,
    api::{Budget, Client, Wire},
    model::{self, Action, Reading},
    network::Network,
    publication::Published,
};
use moli_runtime::DriverCtx;
use std::future::{Future, pending};
use std::time::Duration;
use tokio::time::Instant;

fn message(error: Failure) -> String {
    match error {
        Failure::Refused => {
            moli_i18n::tr!("pilotes.asuswrt.authentification_refusee")
        }
        Failure::Certificate => moli_i18n::tr!("pilotes.asuswrt.certificat_modifie"),
        Failure::Identity => moli_i18n::tr!("pilotes.asuswrt.identite_modifiee"),
        Failure::Secrets => moli_i18n::tr!("pilotes.asuswrt.secret_manquant"),
        Failure::Vault => moli_i18n::tr!("pilotes.asuswrt.coffre_indisponible"),
        _ => moli_i18n::tr!("pilotes.asuswrt.reponse_invalide"),
    }
}
async fn drive<T>(ctx: &mut DriverCtx, future: impl Future<Output = T>) -> Option<T> {
    tokio::pin!(future);
    loop {
        tokio::select! {
            biased;
            command=ctx.next_command()=>match command {Some(command)=>command.reply(Err(moli_i18n::tr!("pilotes.asuswrt.lecture_seule"))),None=>return None},
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
        .any(|s| s.is_empty() || s.len() > 1024 || !s.is_ascii() || s.chars().any(char::is_control))
    {
        return Err(Failure::Refused);
    }
    if username.contains(':') {
        return Err(Failure::Refused);
    }
    let identity = ctx.secret("device_identity");
    if identity
        .as_deref()
        .is_some_and(|s| model::mac(s).as_deref() != Some(s))
    {
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
) -> Result<Vec<(Action, Option<Reading>, u64)>, Failure> {
    let mut budget = Budget::default();
    let mut reads = Vec::new();
    for action in actions {
        reads.push((
            action,
            client.read(wire, action, credentials, &mut budget).await?,
            client.epoch,
        ));
    }
    // If every optional method was removed, retain a bounded liveness read.
    if budget.sent == 0 {
        reads.push((
            Action::Info,
            client
                .read(wire, Action::Info, credentials, &mut budget)
                .await?,
            client.epoch,
        ));
    }
    Ok(reads)
}
async fn timed_cycle(
    client: &mut Client,
    wire: &impl Wire,
    credentials: (&str, &str),
    actions: Vec<Action>,
) -> Result<Vec<(Action, Option<Reading>, u64)>, Failure> {
    tokio::time::timeout(
        Duration::from_secs(30),
        cycle(client, wire, credentials, actions),
    )
    .await
    .unwrap_or(Err(Failure::Budget))
}
pub(crate) async fn run(config: &Config, ctx: &mut DriverCtx) -> anyhow::Result<()> {
    let mut state = Published::new(ctx);
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
    let mut client = Client::new(identity);
    let mut next_info = Instant::now();
    let mut next_inventory = Instant::now();
    loop {
        let now = Instant::now();
        let info_due = now >= next_info;
        let inventory_due = now >= next_inventory;
        let mut actions = Vec::new();
        if info_due {
            actions.push(Action::Info);
        }
        actions.push(Action::Telemetry);
        if inventory_due {
            actions.push(Action::Clients);
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
                    Failure::Refused | Failure::Identity | Failure::Certificate | Failure::Invalid
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
        if !persisted {
            let result = tokio::select! {biased;()=state.ctx.cancelled()=>return Ok(()),result=state.ctx.store_secret("device_identity",identity)=>result};
            if result.is_err() {
                waiting(state.ctx, Failure::Vault).await;
                return Ok(());
            }
            persisted = true;
        }
        state.apply(identity, &config.selected, reads, client.epoch);
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
    use crate::{api::Answer, protocol::Call};
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Slow {
        sent: AtomicUsize,
    }
    impl Wire for Slow {
        async fn send(
            &self,
            call: Call,
            _body: String,
            _token: Option<String>,
        ) -> Result<Answer, Failure> {
            let index = self.sent.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(4800)).await;
            let body = match call {
                Call::Login => serde_json::json!({"asus_token":format!("synthetic-{index}")}),
                Call::Read(Action::Info) => serde_json::json!({"label_mac":"aa:bb:cc:dd:ee:01"}),
                _ if index == 2 => serde_json::json!({"error_status":1}),
                Call::Read(Action::Telemetry) => {
                    serde_json::json!({"memory_usage":{"mem_total":"100","mem_used":"25"}})
                }
                Call::Read(Action::Clients) => serde_json::json!({"get_clientlist":{}}),
            };
            Ok(Answer {
                status: http::StatusCode::OK,
                body: body.to_string().into_bytes(),
            })
        }
    }
    #[tokio::test(start_paused = true)]
    async fn total_cycle_deadline_includes_login_refresh_and_all_reads() {
        let wire = Slow {
            sent: AtomicUsize::new(0),
        };
        let mut client = Client::new(None);
        let start = Instant::now();
        let result = timed_cycle(
            &mut client,
            &wire,
            ("synthetic-user", "synthetic-password"),
            vec![Action::Info, Action::Telemetry, Action::Clients],
        )
        .await;
        assert_eq!(result.unwrap_err(), Failure::Budget);
        assert_eq!(start.elapsed(), Duration::from_secs(30));
        assert_eq!(wire.sent.load(Ordering::SeqCst), 7);
    }
}
