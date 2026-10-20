//! Samsung Tizen TVs: pinned local remote and UPnP audio.
mod model;

mod audio;
mod device;
mod polling;
mod queue;
mod transport;
#[cfg(test)]
use queue::commands;

use moli_runtime::{BoxFuture, CommandRequest, DriverCtx};
use std::sync::Arc;

trait CommandSink: Send + Sync + 'static {
    fn send<'a>(&'a self, command: &'a CommandRequest) -> BoxFuture<'a, Result<(), String>>;
}

#[cfg(test)]
mod command_tests;

use anyhow::Context as _;
use moli_runtime::Driver;
use serde::Deserialize;
use tokio::sync::Mutex;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_upnp")]
    pub upnp_port: u16,
    #[serde(default)]
    pub mac: Option<String>,
    #[serde(default = "default_every")]
    pub every: u64,
}
fn default_port() -> u16 {
    8002
}
fn default_upnp() -> u16 {
    9197
}
fn default_every() -> u64 {
    30
}

#[derive(Debug)]
pub struct Samsung {
    config: Config,
}
impl Samsung {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        anyhow::ensure!(
            !config.host.is_empty()
                && config.host.len() <= 253
                && config
                    .host
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-:".contains(&b)),
            moli_i18n::tr!("pilotes.samsung.hote_invalide")
        );
        anyhow::ensure!(
            config.port != 0 && config.upnp_port != 0 && (10..=3600).contains(&config.every),
            moli_i18n::tr!("pilotes.samsung.options_invalides")
        );
        if let Some(mac) = &config.mac {
            moli_net::wol::packet(mac)
                .map_err(|_| anyhow::anyhow!(moli_i18n::tr!("pilotes.samsung.mac_invalide")))?;
        }
        Ok(Self { config })
    }
}
impl Driver for Samsung {
    fn kind(&self) -> &'static str {
        "samsung"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(run(&self.config, ctx))
    }
}

struct Tv {
    config: Config,
    remote: Mutex<transport::Remote>,
}
impl CommandSink for Tv {
    fn send<'a>(&'a self, c: &'a CommandRequest) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            if model::action(&c.key, &c.value).is_none() {
                return Err(moli_i18n::tr!("pilotes.samsung.non_valide"));
            }
            if model::action(&c.key, &c.value) == Some(model::Action::Wake)
                && self.config.mac.is_none()
            {
                return Err(moli_i18n::tr!("pilotes.samsung.mac_requise"));
            }
            self.apply(c).await.map_err(|e| {
                if e.is::<moli_net::NotSent>() {
                    moli_i18n::tr!("pilotes.samsung.expire")
                } else {
                    moli_i18n::tr!("pilotes.samsung.non_confirme")
                }
            })
        })
    }
}
impl Tv {
    async fn apply(&self, c: &CommandRequest) -> anyhow::Result<()> {
        let action = model::action(&c.key, &c.value).context("invalid command")?;
        if c.abandoned() {
            return Err(moli_net::NotSent.into());
        }
        match action {
            model::Action::Volume(v) => {
                audio::call(
                    &self.config.host,
                    self.config.upnp_port,
                    "SetVolume",
                    Some(&v.to_string()),
                    Some(c),
                )
                .await?;
            }
            model::Action::Mute(v) => {
                audio::call(
                    &self.config.host,
                    self.config.upnp_port,
                    "SetMute",
                    Some(if v { "1" } else { "0" }),
                    Some(c),
                )
                .await?;
            }
            model::Action::Wake => {
                let packet =
                    moli_net::wol::packet(self.config.mac.as_deref().context("missing mac")?)?;
                let udp = tokio::net::UdpSocket::bind("0.0.0.0:0").await?;
                udp.set_broadcast(true)?;
                if c.abandoned() {
                    return Err(moli_net::NotSent.into());
                }
                // One packet only; no delayed/repeated wake after caller departure.
                udp.send_to(&packet, (std::net::Ipv4Addr::BROADCAST, 9))
                    .await?;
            }
            action @ (model::Action::Key(_) | model::Action::Launch(_)) => {
                let payload = match action {
                    model::Action::Key(key) => {
                        serde_json::json!({"method":"ms.remote.control","params":{"Cmd":"Click","DataOfCmd":key,"Option":"false","TypeOfRemote":"SendRemoteKey"}})
                    }
                    model::Action::Launch(app) => {
                        serde_json::json!({"method":"ms.channel.emit","params":{"event":"ed.apps.launch","to":"host","data":{"action_type":"DEEP_LINK","appId":app,"metaTag":""}}})
                    }
                    _ => unreachable!(),
                };
                self.remote.lock().await.send(&payload, c).await?;
            }
        }
        Ok(())
    }
}

async fn initialize(
    config: &Config,
    ctx: &DriverCtx,
) -> anyhow::Result<(Tv, Arc<moli_net::PinnedHttps>, String)> {
    anyhow::ensure!(
        ctx.can_store_secrets(),
        moli_i18n::tr!("pilotes.samsung.coffre")
    );
    // A fully paired TV can start asleep: the cached native identity lets
    // the hub offer Wake-on-LAN without requiring a network connection.
    let cached = ctx
        .secret("native_id")
        .filter(|id| model::native_id(&serde_json::json!({"device":{"id":id}})).is_some());
    let (http, native) = if let (Some(pin), Some(token), Some(native)) =
        (ctx.secret("cert"), ctx.secret("token"), cached)
    {
        anyhow::ensure!(model::valid_token(&token), "invalid stored token");
        (
            moli_net::PinnedHttps::new(&config.host, config.port, Some(pin))?,
            native,
        )
    } else {
        let (socket, http) = transport::pair(ctx, &config.host, config.port).await?;
        drop(socket);
        let info = polling::info(&http).await?;
        let native = model::native_id(&info).context("missing native identity")?;
        ctx.store_secret("native_id", &native).await?;
        (http, native)
    };
    let remote = transport::Remote {
        host: config.host.clone(),
        port: config.port,
        tls: moli_net::PinnedTls::new(ctx.secret("cert"))?,
        token: ctx.secret("token").context("missing token")?,
    };
    Ok((
        Tv {
            config: config.clone(),
            remote: Mutex::new(remote),
        },
        Arc::new(http),
        native,
    ))
}

async fn run(config: &Config, ctx: &mut DriverCtx) -> anyhow::Result<()> {
    let initialized = tokio::select! {
        ()=ctx.cancelled()=>return Ok(()),
        result=initialize(config,ctx)=>result,
    };
    let (tv, http, native) = match initialized {
        Ok(initialized) => initialized,
        Err(e) => {
            if !ctx.can_store_secrets() || format!("{e:#}").contains(moli_net::PIN_MISMATCH) {
                let reason = if ctx.can_store_secrets() {
                    moli_i18n::tr!(
                        "pilotes.samsung.certificat",
                        instance = ctx.instance().as_str()
                    )
                } else {
                    moli_i18n::tr!("pilotes.samsung.coffre")
                };
                ctx.wait_for(reason);
                ctx.cancelled().await;
                return Ok(());
            }
            anyhow::bail!(moli_i18n::tr!("pilotes.samsung.indisponible"));
        }
    };
    let id = ctx.device_id(&native);
    ctx.upsert_device(device::device(ctx, &id));
    ctx.set_availability(&id, false);
    ctx.ready();
    queue::serve(ctx, &id, Arc::new(tv), Some((http, config.clone(), native))).await;
    Ok(())
}

#[cfg(test)]
mod configuration_tests {
    use super::*;
    use moli_core::{DeviceId, InstanceId};
    use moli_runtime::{Hub, HubOptions, MasterKey};
    use serde_json::json;
    use tokio_util::sync::CancellationToken;

    #[test]
    fn configuration_rejects_urls_secrets_unknown_fields_and_unbounded_polling() {
        for value in [
            json!({"host":"https://example.com"}),
            json!({"host":"example.com/path"}),
            json!({"host":"example.com\r\n"}),
            json!({"host":"127.0.0.1","token":"1234"}),
            json!({"host":"127.0.0.1","every":1}),
            json!({"host":"127.0.0.1","port":0}),
            json!({"host":"127.0.0.1","mac":"not-a-mac"}),
        ] {
            let result = serde_json::from_value::<Config>(value)
                .map_err(anyhow::Error::from)
                .and_then(Samsung::new);
            assert!(result.is_err());
        }
        assert!(Samsung::new(serde_json::from_value(json!({"host":"127.0.0.1"})).unwrap()).is_ok());
    }

    #[tokio::test]
    async fn paired_tv_is_published_while_asleep_without_waiting_for_network() {
        let dir = std::env::temp_dir().join(format!("moli-samsung-cached-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let hub = Hub::new(HubOptions {
            secrets: Some((dir.join("secrets.enc"), MasterKey::new("01".repeat(32)))),
            ..HubOptions::default()
        })
        .unwrap();
        let instance = InstanceId::from("tv");
        for (key, value) in [
            ("cert", "00".repeat(32)),
            ("token", "12345678".into()),
            ("native_id", "00000000-0000-0000-0000-000000000001".into()),
        ] {
            hub.store_secret(&instance, key, &value).await.unwrap();
        }
        let config =
            serde_json::from_value(json!({"host":"127.0.0.1","port":1,"upnp_port":1})).unwrap();
        let cancel = CancellationToken::new();
        let driver = moli_runtime::spawn_driver(
            &hub,
            instance,
            Arc::new(Samsung::new(config).unwrap()),
            cancel.clone(),
        );
        let id = DeviceId::from("tv:00000000-0000-0000-0000-000000000001");
        for _ in 0..200 {
            if let Some((device, _)) = hub.describe(&id) {
                assert!(!device.point("app").unwrap().access.read);
                assert!(
                    hub.state(&moli_core::PointId::from(format!("{id}/power")))
                        .is_none_or(|s| s.value == moli_core::Value::Null)
                );
                cancel.cancel();
                driver.await.unwrap();
                assert!(
                    hub.snapshot()
                        .devices
                        .iter()
                        .any(|d| d.device.id == id && d.online == Some(false))
                );
                tokio::fs::remove_dir_all(dir).await.unwrap();
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
        cancel.cancel();
        driver.await.unwrap();
        tokio::fs::remove_dir_all(dir).await.unwrap();
        panic!("cached identity not published");
    }
}
