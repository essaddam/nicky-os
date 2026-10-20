//! Local, bounded exchanges. Credentials never enter diagnostics.
use moli_net::{PinnedHttps, PinnedTls, ws::WebSocket};
use moli_runtime::DriverCtx;
use serde_json::Value as Json;
use std::time::Duration;

const LIMIT: Duration = Duration::from_secs(4);
const MAX_JSON: usize = 64 * 1024;

pub(crate) async fn pair(
    ctx: &DriverCtx,
    host: &str,
    port: u16,
) -> anyhow::Result<(WebSocket, PinnedHttps)> {
    anyhow::ensure!(
        ctx.can_store_secrets(),
        moli_i18n::tr!("pilotes.samsung.coffre")
    );
    let tls = PinnedTls::new(ctx.secret("cert"))?;
    if tls.pinned_fingerprint().is_none() {
        drop(tokio::time::timeout(LIMIT, tls.connect(host, port)).await??);
        let pin = tls
            .pinned_fingerprint()
            .ok_or_else(|| anyhow::anyhow!("missing pin"))?;
        ctx.store_secret("cert", &pin).await?;
    }
    let credential = ctx.secret("token");
    if credential.is_none() {
        ctx.wait_for(moli_i18n::tr!("pilotes.samsung.appairage"));
    }
    let (ws, token) = open(
        host,
        port,
        &tls,
        credential.as_deref(),
        Duration::from_secs(60),
    )
    .await?;
    if let Some(token) = token {
        ctx.store_secret("token", &token).await?;
    }
    anyhow::ensure!(
        ctx.secret("token").is_some(),
        moli_i18n::tr!("pilotes.samsung.appairage")
    );
    let http = PinnedHttps::new(host, port, tls.pinned_fingerprint())?;
    Ok((ws, http))
}

/// Moli is a fixed base64-encoded client name. The only query credential
/// accepted here is numeric and bounded; never include it in an error.
async fn open(
    host: &str,
    port: u16,
    tls: &PinnedTls,
    token: Option<&str>,
    wait: Duration,
) -> anyhow::Result<(WebSocket, Option<String>)> {
    if let Some(token) = token {
        anyhow::ensure!(crate::model::valid_token(token), "invalid stored token");
    }
    let mut path = "/api/v2/channels/samsung.remote.control?name=TW9saQ==".to_owned();
    if let Some(token) = token {
        path.push_str("&token=");
        path.push_str(token);
    }
    tokio::time::timeout(wait, async {
        let mut ws = WebSocket::connect(host, port, &path, &[], tls).await?;
        for _ in 0..32 {
            let message = ws.recv_text(wait).await?;
            anyhow::ensure!(message.len() <= MAX_JSON, "oversized event");
            let event: Json = serde_json::from_str(&message)?;
            match event["event"].as_str() {
                Some("ms.channel.connect") => return Ok((ws, crate::model::token(&event))),
                Some("ms.channel.unauthorized" | "ms.channel.timeOut" | "ms.error") => {
                    anyhow::bail!("pairing refused")
                }
                _ => {}
            }
        }
        anyhow::bail!("too many pairing events")
    })
    .await?
}

pub(crate) struct Remote {
    pub host: String,
    pub port: u16,
    pub tls: PinnedTls,
    pub token: String,
}
impl Remote {
    pub async fn send(
        &mut self,
        payload: &Json,
        command: &moli_runtime::CommandRequest,
    ) -> anyhow::Result<()> {
        if command.abandoned() {
            return Err(moli_net::NotSent.into());
        }
        // One authenticated connection per order: no idle socket can lose
        // heartbeats, and a previous connection closing cannot consume the
        // next key. Reconnection is preparation, never a replay of an order.
        let (mut socket, token) =
            open(&self.host, self.port, &self.tls, Some(&self.token), LIMIT).await?;
        if let Some(token) = token {
            self.token = token;
        }
        if command.abandoned() {
            return Err(moli_net::NotSent.into());
        }
        socket.send_text(&payload.to_string()).await?;
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use moli_core::InstanceId;
    use moli_runtime::{BoxFuture, Driver, Hub, HubOptions};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::sync::{mpsc, oneshot};
    use tokio_util::sync::CancellationToken;

    struct Pairing {
        port: u16,
        report: std::sync::Mutex<Option<oneshot::Sender<bool>>>,
    }
    impl Driver for Pairing {
        fn kind(&self) -> &'static str {
            "samsung"
        }
        fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
            Box::pin(async move {
                let ok = pair(ctx, "127.0.0.1", self.port).await.is_ok();
                if let Some(tx) = self.report.lock().unwrap().take() {
                    let _ = tx.send(ok);
                }
                ctx.cancelled().await;
                Ok(())
            })
        }
    }

    pub(crate) async fn server() -> (u16, mpsc::Receiver<String>, tokio::task::JoinHandle<()>) {
        server_with(br#"{"event":"ms.channel.connect","data":{"token":"12345678"}}"#.to_vec()).await
    }
    async fn server_with(
        event: Vec<u8>,
    ) -> (u16, mpsc::Receiver<String>, tokio::task::JoinHandle<()>) {
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let key = rustls::pki_types::PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der());
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert.cert.der().clone()], key.into())
        .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(async move {
            loop {
                let (tcp, _) = listener.accept().await.unwrap();
                let Ok(mut tls) = acceptor.accept(tcp).await else {
                    continue;
                };
                let mut head = Vec::new();
                let mut byte = [0];
                while tls.read_exact(&mut byte).await.is_ok() {
                    head.push(byte[0]);
                    if head.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                if head.is_empty() {
                    let _ = tx.send(String::new()).await;
                    continue;
                }
                let head = String::from_utf8(head).unwrap();
                tx.send(head.clone()).await.unwrap();
                let key = head
                    .lines()
                    .find_map(|s| s.strip_prefix("Sec-WebSocket-Key: "))
                    .unwrap();
                let digest = ring::digest::digest(
                    &ring::digest::SHA1_FOR_LEGACY_USE_ONLY,
                    format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes(),
                );
                let accept = base64(digest.as_ref());
                tls.write_all(format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n").as_bytes()).await.unwrap();
                let mut frame = vec![0x81];
                if event.len() < 126 {
                    frame.push(u8::try_from(event.len()).unwrap());
                } else if let Ok(n) = u16::try_from(event.len()) {
                    frame.push(126);
                    frame.extend_from_slice(&n.to_be_bytes());
                } else {
                    frame.push(127);
                    frame.extend_from_slice(&u64::try_from(event.len()).unwrap().to_be_bytes());
                }
                frame.extend_from_slice(&event);
                tls.write_all(&frame).await.unwrap();
                let mut header = [0; 2];
                if tls.read_exact(&mut header).await.is_err() {
                    continue;
                }
                let len = match header[1] & 127 {
                    126 => usize::from(tls.read_u16().await.unwrap()),
                    n => usize::from(n),
                };
                let mut mask = [0; 4];
                tls.read_exact(&mut mask).await.unwrap();
                let mut payload = vec![0; len];
                tls.read_exact(&mut payload).await.unwrap();
                for (i, b) in payload.iter_mut().enumerate() {
                    *b ^= mask[i % 4];
                }
                tx.send(String::from_utf8(payload).unwrap()).await.unwrap();
            }
        });
        (port, rx, task)
    }

    fn base64(bytes: &[u8]) -> String {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for c in bytes.chunks(3) {
            let n = c
                .iter()
                .enumerate()
                .fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
            for i in 0..4 {
                out.push(if i <= c.len() {
                    char::from(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize])
                } else {
                    '='
                });
            }
        }
        out
    }

    #[tokio::test]
    async fn pairing_persists_pin_before_sending_existing_token_and_refuses_changed_cert() {
        let (port, mut requests, server_task) = server().await;
        let dir = std::env::temp_dir().join(format!("moli-samsung-{}-{port}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let key = moli_runtime::MasterKey::new("01".repeat(32));
        let hub = Hub::new(HubOptions {
            secrets: Some((dir.join("secrets.enc"), key)),
            ..HubOptions::default()
        })
        .unwrap();
        let instance = InstanceId::from("tv");
        hub.store_secret(&instance, "token", "87654321")
            .await
            .unwrap();
        let (tx, rx) = oneshot::channel();
        let cancel = CancellationToken::new();
        moli_runtime::spawn_driver(
            &hub,
            instance.clone(),
            Arc::new(Pairing {
                port,
                report: std::sync::Mutex::new(Some(tx)),
            }),
            cancel.clone(),
        );
        assert!(
            tokio::time::timeout(Duration::from_secs(5), rx)
                .await
                .unwrap()
                .unwrap()
        );
        assert_eq!(
            requests.recv().await.unwrap(),
            "",
            "first connection carries no application bytes"
        );
        let request = requests.recv().await.unwrap();
        assert!(request.contains("token=87654321"));
        assert!(hub.secret(&instance, "cert").is_some());
        assert_eq!(hub.secret(&instance, "token").as_deref(), Some("12345678"));
        cancel.cancel();
        server_task.abort();
        let (other_port, mut other_requests, other_server) = server().await;
        let (tx, rx) = oneshot::channel();
        let cancel = CancellationToken::new();
        moli_runtime::spawn_driver(
            &hub,
            instance,
            Arc::new(Pairing {
                port: other_port,
                report: std::sync::Mutex::new(Some(tx)),
            }),
            cancel.clone(),
        );
        assert!(!rx.await.unwrap());
        assert!(
            other_requests.try_recv().is_err(),
            "changed certificate receives no token"
        );
        cancel.cancel();
        other_server.abort();
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }
    #[tokio::test]
    async fn denied_and_oversized_pairing_events_never_authenticate() {
        for event in [
            br#"{"event":"ms.channel.unauthorized"}"#.to_vec(),
            vec![b' '; MAX_JSON + 1],
        ] {
            let (port, _requests, server) = server_with(event).await;
            let tls = PinnedTls::new(None).unwrap();
            assert!(
                open("127.0.0.1", port, &tls, None, Duration::from_secs(2))
                    .await
                    .is_err()
            );
            server.abort();
        }
    }
}
