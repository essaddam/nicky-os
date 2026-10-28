use anyhow::{Context, Result, bail, ensure};
use moli_music_poc::{
    audio,
    broker::{Broker, Role},
    control::{Admission, Command, ROOT},
    media::{MediaStore, serve},
    native, providers,
};
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{process::Stdio, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    sync::oneshot,
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

// Only passed through a private parent/child pipe; never serialized to a report.
#[derive(Serialize, Deserialize)]
struct Seed {
    mqtt_port: u16,
    password: String,
    session: String,
    media_url: String,
    media_token: String,
}

fn random_id() -> Result<String> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|_| anyhow::anyhow!("random source unavailable"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("pair-youtube-stdin") => {
            let import = native::browser_import::BrowserImport::read(std::io::stdin().lock())?;
            run_pairing(Some(import)).await
        }
        Some("pair-youtube") => run_pairing(None).await,
        Some("native-status") => {
            println!("{}", native::status());
            Ok(())
        }
        Some("native-search") if args.len() == 3 => {
            println!("{}", native::search(&args[1], &args[2]).await?);
            Ok(())
        }
        Some("native-account") if args.len() == 2 => {
            println!("{}", native::account(&args[1]).await?);
            Ok(())
        }
        Some("demo") => {
            let mode = args.get(1).map_or("merged", String::as_str);
            ensure!(
                ["merged", "isolated"].contains(&mode),
                "use merged or isolated"
            );
            demo(mode).await
        }
        Some("receiver") => {
            let mut input = String::new();
            BufReader::new(tokio::io::stdin())
                .take(8192)
                .read_line(&mut input)
                .await?;
            let seed: Seed = serde_json::from_str(&input).context("invalid private IPC seed")?;
            let decoded = receiver(seed, None).await?;
            println!("{}", serde_json::to_string(&decoded)?);
            Ok(())
        }
        Some("inspect-ma") => {
            println!("{}", providers::inspect_from_env().await?);
            Ok(())
        }
        Some("search-ma") if args.len() == 3 => {
            println!("{}", providers::search_from_env(&args[1], &args[2]).await?);
            Ok(())
        }
        _ => bail!(
            "usage: moli-music-poc demo [merged|isolated] | pair-youtube | pair-youtube-stdin | native-status | native-search <apple_music|ytmusic> <query> | native-account <apple_music|ytmusic> | inspect-ma | search-ma <apple_music|ytmusic> <query>"
        ),
    }
}

async fn run_pairing(import: Option<native::browser_import::BrowserImport>) -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let cancel = CancellationToken::new();
    let mut server = tokio::spawn(native::pairing::serve(listener, cancel.clone()));
    let paired = if let Some(import) = import {
        let result = import.submit(port).await;
        match result {
            Ok(value) if value["paired"] == true => true,
            result => {
                if let Ok(value) = result {
                    println!("{value}");
                }
                cancel.cancel();
                let _ = server.await;
                bail!("YouTube association was not confirmed");
            }
        }
    } else {
        false
    };
    println!(
        "{}",
        json!({"ready":true,"service":"native-youtube-pairing","port":port,
        "lifetime_seconds":600,"paired":paired,"persistent":false})
    );
    tokio::select! {
        result = &mut server => result?,
        _ = tokio::signal::ctrl_c() => { cancel.cancel(); server.await? }
    }
}

async fn demo(mode: &str) -> Result<()> {
    let cancel = CancellationToken::new();
    let mut tasks = JoinSet::new();
    let mqtt = TcpListener::bind("127.0.0.1:0").await?;
    let mqtt_port = mqtt.local_addr()?.port();
    let broker = Broker::new(Arc::new(|_, topic, payload, retained| {
        ensure!(
            topic.starts_with(&format!("{ROOT}/")),
            "unknown control namespace"
        );
        ensure!(payload.len() <= 4096, "media is forbidden on control plane");
        ensure!(
            !retained || !topic.ends_with("/command"),
            "retained command forbidden"
        );
        let _: serde_json::Value = serde_json::from_slice(payload)?;
        Ok(payload.to_vec())
    }));
    let password = random_id()?;
    broker.set_credentials("poc".into(), password.clone());
    let mqtt_broker = broker.clone();
    let stop = cancel.clone();
    tasks.spawn(async move {
        let mut connections = JoinSet::new();
        loop {
            tokio::select! {
                _ = stop.cancelled() => return Ok::<(), anyhow::Error>(()),
                accepted = mqtt.accept() => {
                    let (stream, _) = accepted?;
                    let broker = mqtt_broker.clone();
                    connections.spawn(async move { broker.handle(stream, Role::Bridge).await });
                }
                Some(_) = connections.join_next(), if !connections.is_empty() => {}
            }
        }
    });
    let store = MediaStore::new();
    let ticket = store.insert("audio/flac", audio::fixture(), Duration::from_secs(30))?;
    let http = TcpListener::bind("127.0.0.1:0").await?;
    let http_port = http.local_addr()?.port();
    tasks.spawn(serve(store.clone(), http, cancel.clone()));
    let session = random_id()?;
    let seed = Seed {
        mqtt_port,
        password: password.clone(),
        session: session.clone(),
        media_url: format!("http://127.0.0.1:{http_port}/media/{}", ticket.id),
        media_token: ticket.token,
    };
    let command = Command {
        v: 1,
        id: random_id()?,
        session,
        epoch: 1,
        media_id: ticket.id.clone(),
        operation: "decode-to-null".into(),
    };
    let (ready_tx, ready_rx) = oneshot::channel();
    let output = if mode == "merged" {
        tokio::spawn(async move { receiver(seed, Some(ready_tx)).await })
    } else {
        let mut child = tokio::process::Command::new(std::env::current_exe()?)
            .arg("receiver")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
        let mut input = child
            .stdin
            .take()
            .context("private child input unavailable")?;
        let mut line = serde_json::to_vec(&seed)?;
        line.push(b'\n');
        input.write_all(&line).await?;
        input.shutdown().await?;
        tokio::spawn(async move {
            let mut output =
                BufReader::new(child.stdout.take().context("child output unavailable")?);
            let mut ready = String::new();
            output.read_line(&mut ready).await?;
            ensure!(ready.trim() == "READY", "receiver did not become ready");
            let _ = ready_tx.send(());
            let mut result = String::new();
            output.take(8192).read_line(&mut result).await?;
            ensure!(child.wait().await?.success(), "isolated receiver failed");
            Ok(serde_json::from_str(&result)?)
        })
    };
    tokio::time::timeout(Duration::from_secs(10), ready_rx).await??;
    let mut options = MqttOptions::new("music-poc-source", "127.0.0.1", mqtt_port);
    options.set_credentials("poc", password);
    options.set_max_packet_size(4096, 4096);
    let (publisher, mut events) = AsyncClient::new(options, 8);
    tasks.spawn(async move {
        loop {
            events.poll().await?;
        }
        #[allow(unreachable_code)]
        Ok::<(), anyhow::Error>(())
    });
    publisher.publish(format!("{ROOT}/source/config"), QoS::AtLeastOnce, true,
        serde_json::to_vec(&json!({"v":1,"source":"synthetic-flac","codec":"flac","sample_rate":48000,"channels":1,"execution":"simulation"}))?).await?;
    publisher
        .publish(
            format!("{ROOT}/receiver/command"),
            QoS::AtLeastOnce,
            false,
            serde_json::to_vec(&command)?,
        )
        .await?;
    let decoded = tokio::time::timeout(Duration::from_secs(15), output).await???;
    ensure!(
        decoded.fixture_exact,
        "decoded samples differ from the source"
    );
    store.revoke(&ticket.id);
    let client = reqwest::Client::builder().no_proxy().build()?;
    let revoked = client
        .get(format!("http://127.0.0.1:{http_port}/media/{}", ticket.id))
        .send()
        .await?
        .status()
        .as_u16();
    ensure!(
        revoked == 404 || revoked == 401,
        "revoked media remains exposed"
    );
    println!(
        "{}",
        json!({"mode":mode,"processes_in_demo":if mode=="isolated"{2}else{1},"control":"MQTT/TCP","media":"authenticated HTTP","decoded":decoded,"mqtt":broker.stats(),"revoked_status":revoked,"physical_commands":0,"providers_live":false,"per_module_rss":null})
    );
    cancel.cancel();
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    Ok(())
}

async fn receiver(seed: Seed, ready: Option<oneshot::Sender<()>>) -> Result<audio::Decoded> {
    let mut options = MqttOptions::new("music-poc-receiver", "127.0.0.1", seed.mqtt_port);
    options.set_credentials("poc", &seed.password);
    options.set_max_packet_size(4096, 4096);
    let (client, mut events) = AsyncClient::new(options, 8);
    client
        .subscribe(format!("{ROOT}/receiver/command"), QoS::AtLeastOnce)
        .await?;
    let mut ready = ready;
    let mut admission = Admission::new(seed.session);
    loop {
        match tokio::time::timeout(Duration::from_secs(10), events.poll()).await?? {
            Event::Incoming(Packet::SubAck(_)) => {
                if let Some(tx) = ready.take() {
                    let _ = tx.send(());
                } else {
                    println!("READY");
                }
            }
            Event::Incoming(Packet::Publish(message)) => {
                let command = admission.accept(&message.payload, message.retain)?;
                ensure!(
                    seed.media_url
                        .ends_with(&format!("/media/{}", command.media_id)),
                    "media reference mismatch"
                );
                let client = reqwest::Client::builder()
                    .no_proxy()
                    .redirect(reqwest::redirect::Policy::none())
                    .timeout(Duration::from_secs(5))
                    .build()?;
                let mut response = client
                    .get(&seed.media_url)
                    .bearer_auth(&seed.media_token)
                    .send()
                    .await?
                    .error_for_status()?;
                let mut body = Vec::new();
                while let Some(chunk) = response.chunk().await? {
                    ensure!(
                        chunk.len() <= 1024 * 1024 - body.len(),
                        "fixture stream exceeds bound"
                    );
                    body.extend_from_slice(&chunk);
                }
                return tokio::task::spawn_blocking(move || audio::decode_fixture(body)).await?;
            }
            _ => {}
        }
    }
}
