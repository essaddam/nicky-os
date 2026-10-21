//! Finish-based cycles, no deferred polls, and cancellation at every await.
use crate::{
    Auth, Config, Failure,
    api::{Budget, Client, Wire},
    model::{self, Action},
    network::Network,
    publication::{Published, Reads},
};
use moli_runtime::DriverCtx;
use std::{
    future::{Future, pending},
    time::Duration,
};
use tokio::time::Instant;
fn message(error: Failure) -> String {
    match error {
        Failure::Refused => moli_i18n::tr!("pilotes.huawei_lte.authentification_refusee"),
        Failure::Variant => moli_i18n::tr!("pilotes.huawei_lte.variante_incompatible"),
        Failure::Certificate => moli_i18n::tr!("pilotes.huawei_lte.certificat_modifie"),
        Failure::Identity => moli_i18n::tr!("pilotes.huawei_lte.identite_modifiee"),
        Failure::Secrets => moli_i18n::tr!("pilotes.huawei_lte.secret_manquant"),
        Failure::Vault => moli_i18n::tr!("pilotes.huawei_lte.coffre_indisponible"),
        _ => moli_i18n::tr!("pilotes.huawei_lte.reponse_invalide"),
    }
}
async fn drive<T>(ctx: &mut DriverCtx, future: impl Future<Output = T>) -> Option<T> {
    tokio::pin!(future);
    loop {
        tokio::select! {biased;command=ctx.next_command()=>match command{Some(command)=>command.reply(Err(moli_i18n::tr!("pilotes.huawei_lte.lecture_seule"))),None=>return None},output=&mut future=>return Some(output),}
    }
}
async fn waiting(ctx: &mut DriverCtx, error: Failure) {
    ctx.wait_for(message(error));
    let _ = drive(ctx, pending::<()>()).await;
}
struct Prepared {
    network: Network,
    credentials: Option<(String, String)>,
    identity: Option<String>,
}
async fn prepare(config: &Config, ctx: &DriverCtx) -> Result<Prepared, Failure> {
    if !ctx.can_store_secrets() {
        return Err(Failure::Vault);
    }
    let credentials = if config.auth == Auth::Sha256V4 {
        let user = ctx.secret("username").ok_or(Failure::Secrets)?;
        let password = ctx.secret("password").ok_or(Failure::Secrets)?;
        if [&user, &password].iter().any(|s| {
            s.is_empty()
                || s.len() > 1024
                || !s.is_ascii()
                || s.bytes().any(|b| b.is_ascii_control())
        }) {
            return Err(Failure::Refused);
        }
        Some((user, password))
    } else {
        None
    };
    let identity = ctx.secret("device_identity");
    if identity.as_deref().is_some_and(|id| !model::identity(id)) {
        return Err(Failure::Identity);
    }
    let pin = ctx.secret("cert_sha256");
    if pin
        .as_ref()
        .is_some_and(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(Failure::Certificate);
    }
    let (network, observed) = Network::open(config, pin.clone()).await?;
    if let Some(pin) = observed.filter(|_| pin.is_none()) {
        ctx.store_secret("cert_sha256", &pin)
            .await
            .map_err(|_| Failure::Vault)?;
    }
    Ok(Prepared {
        network,
        credentials,
        identity,
    })
}
async fn cycle(
    client: &mut Client,
    wire: &impl Wire,
    credentials: Option<(&str, &str)>,
    info_due: bool,
) -> Result<Reads, Failure> {
    let mut budget = Budget::default();
    let mut reads = Vec::new();
    if info_due {
        let info = client
            .read(wire, Action::Info, credentials, &mut budget)
            .await?;
        reads.push((Action::Info, info, client.epoch, Instant::now()));
    }
    let status = client
        .read(wire, Action::Status, credentials, &mut budget)
        .await?;
    let no_sim = status.as_ref().is_some_and(|r| r.sim == Some(false));
    let connected = status.as_ref().is_some_and(|r| r.connected == Some(true));
    reads.push((Action::Status, status, client.epoch, Instant::now()));
    let signal = if no_sim {
        None
    } else {
        client
            .read(wire, Action::Signal, credentials, &mut budget)
            .await?
    };
    reads.push((Action::Signal, signal, client.epoch, Instant::now()));
    let traffic = if connected {
        client
            .read(wire, Action::Traffic, credentials, &mut budget)
            .await?
    } else {
        None
    };
    reads.push((Action::Traffic, traffic, client.epoch, Instant::now()));
    if budget.sent == 0 {
        let info = client
            .read(wire, Action::Info, credentials, &mut budget)
            .await?;
        reads.push((Action::Info, info, client.epoch, Instant::now()));
    }
    Ok(reads)
}
async fn timed_cycle(
    client: &mut Client,
    wire: &impl Wire,
    credentials: Option<(&str, &str)>,
    info_due: bool,
) -> Result<Reads, Failure> {
    tokio::time::timeout(
        Duration::from_secs(30),
        cycle(client, wire, credentials, info_due),
    )
    .await
    .unwrap_or(Err(Failure::Budget))
}
pub(crate) async fn run(config: &Config, ctx: &mut DriverCtx) -> anyhow::Result<()> {
    let mut state = Published::new(ctx);
    if let Err(error) = state.check_known() {
        waiting(state.ctx, error).await;
        return Ok(());
    }
    let prepared = tokio::select! {biased;()=state.ctx.cancelled()=>return Ok(()),result=prepare(config,state.ctx)=>result};
    let Prepared {
        network,
        credentials,
        identity,
    } = match prepared {
        Ok(prepared) => prepared,
        Err(Failure::Transport) => return Err(anyhow::anyhow!(message(Failure::Transport))),
        Err(error) => {
            waiting(state.ctx, error).await;
            return Ok(());
        }
    };
    let mut persisted = identity.is_some();
    let mut client = Client::new(config.auth, config.session, identity);
    let mut next_info = Instant::now();
    loop {
        let info_due = Instant::now() >= next_info;
        let credentials = credentials
            .as_ref()
            .map(|(user, password)| (user.as_str(), password.as_str()));
        let Some(result) = drive(
            state.ctx,
            timed_cycle(&mut client, &network, credentials, info_due),
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
                    Failure::Refused
                        | Failure::Variant
                        | Failure::Identity
                        | Failure::Certificate
                        | Failure::Invalid
                        | Failure::Secrets
                        | Failure::Vault
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
        if let Err(error) = state.admit(identity) {
            waiting(state.ctx, error).await;
            return Ok(());
        }
        if !persisted {
            let saved = tokio::select! {biased;()=state.ctx.cancelled()=>return Ok(()),result=state.ctx.store_secret("device_identity",identity)=>result};
            if saved.is_err() {
                waiting(state.ctx, Failure::Vault).await;
                return Ok(());
            }
            persisted = true;
        }
        if let Err(error) = state.apply(identity, reads, client.epoch) {
            state.offline();
            waiting(state.ctx, error).await;
            return Ok(());
        }
        state.ctx.ready();
        if info_due {
            next_info = Instant::now() + Duration::from_secs(config.info_every);
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
    use crate::{Session, api::Answer, protocol::Call};
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Slow {
        sent: AtomicUsize,
    }
    impl Wire for Slow {
        fn host(&self) -> &'static str {
            "modem.example.invalid"
        }
        fn https(&self) -> bool {
            true
        }
        async fn send(
            &self,
            call: Call,
            _body: Option<String>,
            _token: Option<String>,
            _cookie: Option<String>,
        ) -> Result<Answer, Failure> {
            self.sent.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(4800)).await;
            let mut headers = http::HeaderMap::new();
            let body = match call {
                Call::Session => {
                    "<response><SesInfo>SessionID=synthetic</SesInfo><TokInfo>synthetic</TokInfo></response>"
                }
                Call::State => {
                    "<response><State>-1</State><password_type>4</password_type></response>"
                }
                Call::Login => {
                    headers.insert("__requestverificationtoken", "synthetic".parse().unwrap());
                    "<response>OK</response>"
                }
                Call::Read(Action::Info) => {
                    "<response><SerialNumber>synthetic-modem-01</SerialNumber></response>"
                }
                Call::Read(Action::Status) => {
                    "<response><SimStatus>1</SimStatus><ConnectionStatus>901</ConnectionStatus></response>"
                }
                Call::Read(Action::Signal) => "<response><rssi>-75dBm</rssi></response>",
                Call::Read(Action::Traffic) => {
                    "<response><CurrentDownload>100</CurrentDownload></response>"
                }
                Call::Token => unreachable!(),
            };
            Ok(Answer {
                status: http::StatusCode::OK,
                headers,
                body: body.as_bytes().to_vec(),
            })
        }
    }
    #[tokio::test(start_paused = true)]
    async fn thirty_second_deadline_covers_bootstrap_login_and_measurements() {
        let wire = Slow {
            sent: AtomicUsize::new(0),
        };
        let mut client = Client::new(Auth::Sha256V4, Session::Sestok, None);
        let start = Instant::now();
        let result = timed_cycle(
            &mut client,
            &wire,
            Some(("synthetic-user", "synthetic-password")),
            true,
        )
        .await;
        assert_eq!(result.unwrap_err(), Failure::Budget);
        assert_eq!(start.elapsed(), Duration::from_secs(30));
        assert_eq!(wire.sent.load(Ordering::SeqCst), 7);
    }
}
