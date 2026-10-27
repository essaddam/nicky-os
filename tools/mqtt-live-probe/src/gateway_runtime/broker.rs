//! Deliberately bounded MQTT 3.1.1 subset for the local POC gateway.
//! No will, persistent session, QoS 2, or outbound delivery acknowledgement state.
use anyhow::{Context, Result, bail, ensure};
use bytes::{Bytes, BytesMut};
use rumqttc::mqttbytes::v4::{
    ConnAck, Connect, ConnectReturnCode, Packet, PubAck, Publish, SubAck, Subscribe,
    SubscribeReasonCode, UnsubAck,
};
use rumqttc::mqttbytes::{Error as CodecError, Protocol, QoS};
use serde_json::{Value as Json, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::OpenOptions;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use subtle::ConstantTimeEq;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

const MAX_PACKET: usize = 4 * 1024 * 1024;
const MAX_RETAINED_BYTES: usize = 32 * 1024 * 1024;
const MAX_DISK_BYTES: usize = 16 * 1024 * 1024;
const MAX_RETAINED_TOPICS: usize = 16384;
const MAX_CLIENTS: usize = 32;
const MAX_SUBSCRIPTIONS: usize = 256;
const MAX_TOPIC: usize = 1024;
const SEND_QUEUE: usize = 128;
const IO_TIMEOUT: Duration = Duration::from_secs(10);
static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Bridge,
    Browser,
}
pub type PublishHook = dyn Fn(Role, &str, &[u8], bool) -> Result<Vec<u8>> + Send + Sync;

struct Credentials {
    user: [u8; 256],
    password: [u8; 1024],
    user_len: usize,
    password_len: usize,
}
impl Credentials {
    fn new(user: &str, password: &str) -> Option<Self> {
        if user.is_empty() || password.is_empty() || user.len() > 256 || password.len() > 1024 {
            return None;
        }
        let mut value = Self {
            user: [0; 256],
            password: [0; 1024],
            user_len: user.len(),
            password_len: password.len(),
        };
        value.user[..user.len()].copy_from_slice(user.as_bytes());
        value.password[..password.len()].copy_from_slice(password.as_bytes());
        Some(value)
    }
    fn accepts(&self, user: &str, password: &str) -> bool {
        let Some(candidate) = Self::new(user, password) else {
            return false;
        };
        bool::from(
            self.user.ct_eq(&candidate.user)
                & self.password.ct_eq(&candidate.password)
                & self.user_len.ct_eq(&candidate.user_len)
                & self.password_len.ct_eq(&candidate.password_len),
        )
    }
}
struct DiskPayload {
    path: PathBuf,
    len: usize,
    hash: u64,
    allocated: Arc<AtomicUsize>,
}
impl Drop for DiskPayload {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        self.allocated.fetch_sub(self.len, Ordering::Relaxed);
    }
}
enum Body {
    Memory(Bytes),
    File(Arc<DiskPayload>),
}
impl Body {
    fn len(&self) -> usize {
        match self {
            Self::Memory(value) => value.len(),
            Self::File(value) => value.len,
        }
    }
    fn equals(&self, bytes: &[u8]) -> Result<bool> {
        match self {
            Self::Memory(value) => Ok(value.as_ref() == bytes),
            Self::File(value) => {
                if value.len != bytes.len() || value.hash != hash(bytes) {
                    return Ok(false);
                }
                let mut file =
                    std::fs::File::open(&value.path).context("descriptor read failed")?;
                let mut chunk = [0; 8192];
                let mut position = 0;
                while position < bytes.len() {
                    let size = (bytes.len() - position).min(chunk.len());
                    file.read_exact(&mut chunk[..size])
                        .context("descriptor read failed")?;
                    if chunk[..size] != bytes[position..position + size] {
                        return Ok(false);
                    }
                    position += size;
                }
                Ok(true)
            }
        }
    }
}
struct Message {
    topic: String,
    body: Body,
}
struct Delivery {
    message: Arc<Message>,
    retained: bool,
}
struct Client {
    sender: mpsc::Sender<Delivery>,
    cancel: CancellationToken,
    subscriptions: BTreeSet<String>,
    mqtt_id: Option<String>,
}
#[derive(Default)]
struct State {
    credentials: Option<Credentials>,
    clients: BTreeMap<u64, Client>,
    retained: BTreeMap<String, Arc<Message>>,
    retained_bytes: usize,
    description_directory: Option<PathBuf>,
    next_client: u64,
}
pub struct Broker {
    state: Mutex<State>,
    hook: Arc<PublishHook>,
    disk_bytes: Arc<AtomicUsize>,
}
struct Registration {
    broker: Arc<Broker>,
    id: u64,
}
impl Drop for Registration {
    fn drop(&mut self) {
        self.broker.lock().clients.remove(&self.id);
    }
}

impl Broker {
    pub fn new(hook: Arc<PublishHook>) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State::default()),
            hook,
            disk_bytes: Arc::new(AtomicUsize::new(0)),
        })
    }
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    pub fn set_credentials(&self, user: String, password: String) {
        self.lock().credentials = Credentials::new(&user, &password);
    }
    pub fn set_description_directory(&self, path: PathBuf) -> Result<()> {
        ensure!(path.is_absolute(), "descriptor directory must be absolute");
        let mut state = self.lock();
        ensure!(
            state.retained.is_empty() && state.clients.is_empty(),
            "configure descriptor storage before bootstrap"
        );
        std::fs::create_dir_all(&path).context("descriptor directory creation failed")?;
        ensure!(
            !std::fs::symlink_metadata(&path)?.file_type().is_symlink(),
            "descriptor directory cannot be a symlink"
        );
        let path = path.canonicalize()?;
        let checkout = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .context("invalid source location")?
            .canonicalize()?;
        ensure!(
            !path.starts_with(checkout),
            "descriptor directory must be outside checkout"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
        }
        state.description_directory = Some(path);
        Ok(())
    }
    fn disk_body(&self, directory: &std::path::Path, payload: &[u8]) -> Result<Body> {
        ensure!(
            self.disk_bytes.load(Ordering::Relaxed) + payload.len() <= MAX_DISK_BYTES,
            "descriptor disk budget exceeded"
        );
        let path = directory.join(format!(
            "descriptor-{}-{}.mqtt",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path).context("descriptor creation failed")?;
        if let Err(error) = file.write_all(payload) {
            let _ = std::fs::remove_file(&path);
            return Err(error).context("descriptor write failed");
        }
        self.disk_bytes.fetch_add(payload.len(), Ordering::Relaxed);
        Ok(Body::File(Arc::new(DiskPayload {
            path,
            len: payload.len(),
            hash: hash(payload),
            allocated: self.disk_bytes.clone(),
        })))
    }
    pub fn publish(&self, topic: String, payload: Vec<u8>, retain: bool) -> Result<()> {
        ensure!(valid_topic(&topic), "invalid publication topic");
        ensure!(
            payload.len() + topic.len() + 7 <= MAX_PACKET,
            "publication exceeds packet bound"
        );
        let mut state = self.lock();
        if retain {
            let old = state
                .retained
                .get(&topic)
                .map_or(0, |message| message.body.len());
            ensure!(
                state.retained_bytes - old + payload.len() <= MAX_RETAINED_BYTES,
                "retained byte budget exceeded"
            );
            ensure!(
                payload.is_empty()
                    || state.retained.contains_key(&topic)
                    || state.retained.len() < MAX_RETAINED_TOPICS,
                "retained topic budget exceeded"
            );
        }
        let message = if retain
            && !payload.is_empty()
            && state
                .retained
                .get(&topic)
                .is_some_and(|message| message.body.len() == payload.len())
            && state.retained[&topic].body.equals(&payload)?
        {
            state.retained[&topic].clone()
        } else {
            let body = if retain
                && !payload.is_empty()
                && topic.ends_with("/config")
                && let Some(directory) = &state.description_directory
            {
                self.disk_body(directory, &payload)?
            } else {
                Body::Memory(Bytes::from(payload))
            };
            Arc::new(Message {
                topic: topic.clone(),
                body,
            })
        };
        if retain {
            if let Some(old) = state.retained.remove(&topic) {
                state.retained_bytes -= old.body.len();
            }
            if message.body.len() != 0 {
                state.retained_bytes += message.body.len();
                state.retained.insert(topic, message.clone());
            }
        }
        let mut slow = Vec::new();
        for (&id, client) in &state.clients {
            if client
                .subscriptions
                .iter()
                .any(|filter| matches_filter(filter, &message.topic))
                && client
                    .sender
                    .try_send(Delivery {
                        message: message.clone(),
                        retained: false,
                    })
                    .is_err()
            {
                client.cancel.cancel();
                slow.push(id);
            }
        }
        for id in slow {
            state.clients.remove(&id);
        }
        Ok(())
    }
    pub fn stats(&self) -> Json {
        let state = self.lock();
        let ram: usize = state
            .retained
            .values()
            .filter(|message| message.topic.ends_with("/config"))
            .filter_map(|message| match &message.body {
                Body::Memory(bytes) => Some(bytes.len()),
                Body::File(_) => None,
            })
            .sum();
        json!({"clients":state.clients.len(),"retained_topics":state.retained.len(),"retained_payload_bytes":state.retained_bytes,"description_storage":if state.description_directory.is_some(){"disk"}else{"memory"},"description_disk_bytes":self.disk_bytes.load(Ordering::Relaxed),"description_ram_bytes":ram})
    }
    fn register(
        self: &Arc<Self>,
    ) -> Result<(Registration, mpsc::Receiver<Delivery>, CancellationToken)> {
        let mut state = self.lock();
        ensure!(state.clients.len() < MAX_CLIENTS, "client budget exceeded");
        let id = state.next_client;
        state.next_client = state.next_client.wrapping_add(1);
        let (sender, receiver) = mpsc::channel(SEND_QUEUE);
        let cancel = CancellationToken::new();
        state.clients.insert(
            id,
            Client {
                sender,
                cancel: cancel.clone(),
                subscriptions: BTreeSet::new(),
                mqtt_id: None,
            },
        );
        Ok((
            Registration {
                broker: self.clone(),
                id,
            },
            receiver,
            cancel,
        ))
    }
    fn authorize(&self, id: u64, connect: &Connect) -> ConnectReturnCode {
        if connect.protocol != Protocol::V4 {
            return ConnectReturnCode::RefusedProtocolVersion;
        }
        if !connect.clean_session || connect.last_will.is_some() {
            return ConnectReturnCode::NotAuthorized;
        }
        if connect.client_id.is_empty() || connect.client_id.len() > 256 {
            return ConnectReturnCode::BadClientId;
        }
        let mut state = self.lock();
        let valid = connect.login.as_ref().is_some_and(|login| {
            state
                .credentials
                .as_ref()
                .is_some_and(|credentials| credentials.accepts(&login.username, &login.password))
        });
        if !valid {
            return ConnectReturnCode::BadUserNamePassword;
        }
        let replaced: Vec<_> = state
            .clients
            .iter()
            .filter(|(other, client)| {
                **other != id && client.mqtt_id.as_deref() == Some(&connect.client_id)
            })
            .map(|(id, _)| *id)
            .collect();
        for old in replaced {
            if let Some(client) = state.clients.remove(&old) {
                client.cancel.cancel();
            }
        }
        if let Some(client) = state.clients.get_mut(&id) {
            client.mqtt_id = Some(connect.client_id.clone());
        }
        ConnectReturnCode::Success
    }
    fn subscribe(
        &self,
        id: u64,
        role: Role,
        subscribe: Subscribe,
    ) -> Result<(Packet, Vec<Arc<Message>>)> {
        ensure!(
            subscribe.pkid != 0 && subscribe.filters.len() <= MAX_SUBSCRIPTIONS,
            "invalid subscribe bound"
        );
        let mut state = self.lock();
        let client = state.clients.get_mut(&id).context("client disconnected")?;
        let mut codes = Vec::with_capacity(subscribe.filters.len());
        let mut accepted = Vec::new();
        for filter in subscribe.filters {
            if filter.qos == QoS::ExactlyOnce
                || !allowed_filter(role, &filter.path)
                || !client.subscriptions.contains(&filter.path)
                    && client.subscriptions.len() >= MAX_SUBSCRIPTIONS
            {
                codes.push(SubscribeReasonCode::Failure);
            } else {
                client.subscriptions.insert(filter.path.clone());
                accepted.push(filter.path);
                codes.push(SubscribeReasonCode::Success(QoS::AtMostOnce));
            }
        }
        let retained = state
            .retained
            .values()
            .filter(|message| {
                accepted
                    .iter()
                    .any(|filter| matches_filter(filter, &message.topic))
            })
            .cloned()
            .collect();
        Ok((Packet::SubAck(SubAck::new(subscribe.pkid, codes)), retained))
    }
    pub async fn handle<S>(self: Arc<Self>, stream: S, role: Role) -> Result<()>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let (registration, mut outgoing, cancel) = self.register()?;
        let (mut reader, mut writer) = tokio::io::split(stream);
        let mut buffer = BytesMut::new();
        let first = tokio::time::timeout(IO_TIMEOUT, read_packet(&mut reader, &mut buffer))
            .await
            .context("CONNECT timeout")??;
        let Packet::Connect(connect) = first else {
            bail!("CONNECT required");
        };
        let result = self.authorize(registration.id, &connect);
        send_packet(
            &mut writer,
            &Packet::ConnAck(ConnAck::new(result, false)),
            &cancel,
        )
        .await?;
        ensure!(result == ConnectReturnCode::Success, "connection refused");
        let keep_alive = if connect.keep_alive == 0 {
            None
        } else {
            Some(Duration::from_millis(u64::from(connect.keep_alive) * 1500))
        };
        let mut received_at = tokio::time::Instant::now();
        loop {
            let deadline = received_at + keep_alive.unwrap_or(Duration::from_secs(86400));
            tokio::select! {
                ()=cancel.cancelled()=>bail!("client disconnected for retained resynchronization"),
                ()=tokio::time::sleep_until(deadline),if keep_alive.is_some()=>bail!("keepalive expired"),
                delivery=outgoing.recv()=>{
                    let delivery=delivery.context("client send queue closed")?;
                    send_delivery(&mut writer,&delivery,&cancel).await?;
                },
                packet=read_packet(&mut reader,&mut buffer)=>{
                    received_at=tokio::time::Instant::now();
                    match packet? {
                        Packet::Publish(publication)=>{
                            ensure!(publication.qos!=QoS::ExactlyOnce,"QoS 2 unsupported");
                            ensure!(valid_topic(&publication.topic),"invalid publication topic");
                            let payload=(self.hook)(role,&publication.topic,&publication.payload,publication.retain)?;
                            self.publish(publication.topic,payload,publication.retain)?;
                            if publication.qos==QoS::AtLeastOnce { send_packet(&mut writer,&Packet::PubAck(PubAck::new(publication.pkid)),&cancel).await?; }
                        },
                        Packet::Subscribe(subscribe)=>{
                            let (ack,retained)=self.subscribe(registration.id,role,subscribe)?;
                            send_packet(&mut writer,&ack,&cancel).await?;
                            // Snapshot contains shared handles only. Slow bootstrap readers
                            // are cancelled if live updates fill their bounded queue.
                            for message in retained { send_delivery(&mut writer,&Delivery { message,retained:true },&cancel).await?; }
                        },
                        Packet::Unsubscribe(unsubscribe)=>{
                            ensure!(unsubscribe.pkid!=0 && !unsubscribe.topics.is_empty() && unsubscribe.topics.len()<=MAX_SUBSCRIPTIONS,"invalid unsubscribe bound");
                            { let mut state=self.lock(); if let Some(client)=state.clients.get_mut(&registration.id) { for topic in unsubscribe.topics { client.subscriptions.remove(&topic); } } }
                            send_packet(&mut writer,&Packet::UnsubAck(UnsubAck::new(unsubscribe.pkid)),&cancel).await?;
                        },
                        Packet::PingReq=>send_packet(&mut writer,&Packet::PingResp,&cancel).await?,
                        Packet::Disconnect=>return Ok(()),
                        _=>bail!("unsupported MQTT packet"),
                    }
                }
            }
        }
    }
}
fn hash(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}
fn valid_topic(topic: &str) -> bool {
    !topic.is_empty()
        && topic.len() <= MAX_TOPIC
        && !topic
            .chars()
            .any(|c| c.is_control() || matches!(c, '+' | '#'))
}
fn allowed_filter(role: Role, filter: &str) -> bool {
    if filter.is_empty() || filter.len() > MAX_TOPIC || filter.chars().any(char::is_control) {
        return false;
    }
    let parts: Vec<_> = filter.split('/').collect();
    if parts.iter().enumerate().any(|(index, part)| {
        part.contains('#') && (*part != "#" || index + 1 != parts.len())
            || part.contains('+') && *part != "+"
    }) {
        return false;
    }
    match role {
        Role::Bridge => filter.starts_with("moli-probe/"),
        Role::Browser => {
            filter.starts_with("moli-probe/maison/") || filter.starts_with("moli-probe/runtime/")
        }
    }
}
fn matches_filter(filter: &str, topic: &str) -> bool {
    let mut levels = topic.split('/');
    for level in filter.split('/') {
        if level == "#" {
            return true;
        }
        match levels.next() {
            Some(value) if level == "+" || level == value => {}
            _ => return false,
        }
    }
    levels.next().is_none()
}
async fn read_packet<R: AsyncRead + Unpin>(
    reader: &mut R,
    buffer: &mut BytesMut,
) -> Result<Packet> {
    loop {
        if let Some(header) = buffer.first() {
            let flags = header & 0x0f;
            let kind = header >> 4;
            let valid = match kind {
                3 => flags & 0x06 != 0x06 && !(flags & 0x06 == 0 && flags & 0x08 != 0),
                6 | 8 | 10 => flags == 2,
                1 | 2 | 4 | 5 | 7 | 9 | 11 | 12 | 13 | 14 => flags == 0,
                _ => false,
            };
            ensure!(valid, "invalid MQTT fixed header");
            if matches!(kind, 12..=14)
                && let Some(length) = buffer.get(1)
            {
                ensure!(*length == 0, "invalid MQTT control length");
            }
        }
        match Packet::read(buffer, MAX_PACKET) {
            Ok(packet) => return Ok(packet),
            Err(CodecError::InsufficientBytes(_)) => {}
            Err(_) => bail!("invalid MQTT packet"),
        }
        ensure!(buffer.len() < MAX_PACKET, "MQTT packet bound exceeded");
        let mut chunk = [0; 8192];
        let limit = chunk.len().min(MAX_PACKET - buffer.len());
        let read = reader
            .read(&mut chunk[..limit])
            .await
            .context("MQTT read failed")?;
        ensure!(read != 0, "MQTT connection closed");
        buffer.extend_from_slice(&chunk[..read]);
    }
}
async fn send_packet<W: AsyncWrite + Unpin>(
    writer: &mut W,
    packet: &Packet,
    cancel: &CancellationToken,
) -> Result<()> {
    let mut bytes = BytesMut::new();
    packet
        .write(&mut bytes, MAX_PACKET)
        .map_err(|_| anyhow::anyhow!("MQTT encoding failed"))?;
    tokio::select! { ()=cancel.cancelled()=>bail!("client disconnected"),result=tokio::time::timeout(IO_TIMEOUT,async { writer.write_all(&bytes).await?;writer.flush().await })=>result.context("MQTT send timeout")?.context("MQTT send failed") }
}
async fn send_delivery<W: AsyncWrite + Unpin>(
    writer: &mut W,
    delivery: &Delivery,
    cancel: &CancellationToken,
) -> Result<()> {
    match &delivery.message.body {
        Body::Memory(bytes) => {
            let mut publication =
                Publish::from_bytes(&delivery.message.topic, QoS::AtMostOnce, bytes.clone());
            publication.retain = delivery.retained;
            send_packet(writer, &Packet::Publish(publication), cancel).await
        }
        Body::File(payload) => {
            let mut file = tokio::fs::File::open(&payload.path)
                .await
                .context("descriptor open failed")?;
            let mut header = Vec::with_capacity(delivery.message.topic.len() + 7);
            header.push(0x30 | u8::from(delivery.retained));
            let mut remaining = 2 + delivery.message.topic.len() + payload.len;
            loop {
                let mut byte = (remaining % 128) as u8;
                remaining /= 128;
                if remaining != 0 {
                    byte |= 0x80;
                }
                header.push(byte);
                if remaining == 0 {
                    break;
                }
            }
            header.extend_from_slice(&(delivery.message.topic.len() as u16).to_be_bytes());
            header.extend_from_slice(delivery.message.topic.as_bytes());
            tokio::select! {
                ()=cancel.cancelled()=>bail!("client disconnected"),
                result=tokio::time::timeout(IO_TIMEOUT,async {
                    writer.write_all(&header).await?; let mut chunk=[0;8192]; let mut remaining=payload.len;
                    while remaining!=0 { let limit=remaining.min(chunk.len());let read=file.read(&mut chunk[..limit]).await?;if read==0 {return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));}writer.write_all(&chunk[..read]).await?;remaining-=read; }
                    writer.flush().await
                })=>result.context("descriptor send timeout")?.context("descriptor send failed")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rumqttc::mqttbytes::v4::{LastWill, SubscribeFilter, Unsubscribe};
    use tokio::io::{DuplexStream, duplex};

    fn fixture() -> Arc<Broker> {
        let broker = Broker::new(Arc::new(|_, _, payload, _| Ok(payload.to_vec())));
        broker.set_credentials("fixture-user".into(), "fixture-password".into());
        broker
    }
    fn connect(id: &str) -> Connect {
        let mut value = Connect::new(id);
        value.set_login("fixture-user", "fixture-password");
        value
    }
    fn spawn(
        broker: &Arc<Broker>,
        role: Role,
    ) -> (DuplexStream, tokio::task::JoinHandle<Result<()>>) {
        let (client, server) = duplex(8192);
        let broker = broker.clone();
        (
            client,
            tokio::spawn(async move { broker.handle(server, role).await }),
        )
    }
    async fn send(stream: &mut DuplexStream, packet: Packet) {
        send_packet(stream, &packet, &CancellationToken::new())
            .await
            .unwrap();
    }
    async fn receive(stream: &mut DuplexStream, buffer: &mut BytesMut) -> Packet {
        tokio::time::timeout(Duration::from_secs(2), read_packet(stream, buffer))
            .await
            .unwrap()
            .unwrap()
    }
    async fn login(
        broker: &Arc<Broker>,
        role: Role,
        id: &str,
    ) -> (DuplexStream, BytesMut, tokio::task::JoinHandle<Result<()>>) {
        let (mut stream, task) = spawn(broker, role);
        let mut buffer = BytesMut::new();
        send(&mut stream, Packet::Connect(connect(id))).await;
        assert_eq!(
            receive(&mut stream, &mut buffer).await,
            Packet::ConnAck(ConnAck::new(ConnectReturnCode::Success, false))
        );
        (stream, buffer, task)
    }
    fn subscription(filter: &str, pkid: u16) -> Packet {
        let mut subscribe = Subscribe::new(filter, QoS::AtLeastOnce);
        subscribe.pkid = pkid;
        Packet::Subscribe(subscribe)
    }
    fn publication(packet: Packet) -> Publish {
        match packet {
            Packet::Publish(value) => value,
            _ => panic!("expected publication"),
        }
    }
    async fn disconnect(mut stream: DuplexStream, task: tokio::task::JoinHandle<Result<()>>) {
        send(&mut stream, Packet::Disconnect).await;
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn fragmented_connect_requires_credentials_and_refuses_unsupported_sessions() {
        let broker = fixture();
        let (mut stream, task) = spawn(&broker, Role::Browser);
        let mut bytes = BytesMut::new();
        Packet::Connect(connect("fragmented"))
            .write(&mut bytes, MAX_PACKET)
            .unwrap();
        for byte in bytes {
            stream.write_all(&[byte]).await.unwrap();
            tokio::task::yield_now().await;
        }
        let mut buffer = BytesMut::new();
        assert_eq!(
            receive(&mut stream, &mut buffer).await,
            Packet::ConnAck(ConnAck::new(ConnectReturnCode::Success, false))
        );
        disconnect(stream, task).await;
        for variant in 0..4 {
            let mut request = connect("rejected");
            let expected = match variant {
                0 => {
                    request.set_login("fixture-user", "wrong");
                    ConnectReturnCode::BadUserNamePassword
                }
                1 => {
                    request.login = None;
                    ConnectReturnCode::BadUserNamePassword
                }
                2 => {
                    request.clean_session = false;
                    ConnectReturnCode::NotAuthorized
                }
                _ => {
                    request.last_will = Some(LastWill::new(
                        "moli-probe/runtime/will",
                        "offline",
                        QoS::AtMostOnce,
                        false,
                    ));
                    ConnectReturnCode::NotAuthorized
                }
            };
            let (mut stream, task) = spawn(&broker, Role::Bridge);
            let mut buffer = BytesMut::new();
            send(&mut stream, Packet::Connect(request)).await;
            assert_eq!(
                receive(&mut stream, &mut buffer).await,
                Packet::ConnAck(ConnAck::new(expected, false))
            );
            assert!(task.await.unwrap().is_err());
        }
        assert_eq!(broker.stats()["clients"], 0);
    }

    #[tokio::test]
    async fn retained_delivery_uses_qos_zero_and_live_delivery_clears_retain() {
        let broker = fixture();
        let topic = "moli-probe/maison/fixture/canal/state";
        broker
            .publish(topic.into(), b"initial".to_vec(), true)
            .unwrap();
        broker
            .publish(
                "moli-probe/source/fixture/registry".into(),
                b"private".to_vec(),
                true,
            )
            .unwrap();
        let (mut stream, mut buffer, task) = login(&broker, Role::Browser, "retained").await;
        send(&mut stream, subscription("moli-probe/source/#", 1)).await;
        assert_eq!(
            receive(&mut stream, &mut buffer).await,
            Packet::SubAck(SubAck::new(1, vec![SubscribeReasonCode::Failure]))
        );
        send(&mut stream, subscription("moli-probe/maison/+/canal/#", 2)).await;
        assert_eq!(
            receive(&mut stream, &mut buffer).await,
            Packet::SubAck(SubAck::new(
                2,
                vec![SubscribeReasonCode::Success(QoS::AtMostOnce)]
            ))
        );
        let initial = publication(receive(&mut stream, &mut buffer).await);
        assert_eq!(initial.payload.as_ref(), b"initial");
        assert!(initial.retain);
        assert_eq!(initial.qos, QoS::AtMostOnce);
        broker
            .publish(topic.into(), b"live".to_vec(), true)
            .unwrap();
        let live = publication(receive(&mut stream, &mut buffer).await);
        assert!(!live.retain);
        assert_eq!(live.payload.as_ref(), b"live");
        broker.publish(topic.into(), Vec::new(), true).unwrap();
        assert!(
            publication(receive(&mut stream, &mut buffer).await)
                .payload
                .is_empty()
        );
        assert_eq!(broker.stats()["retained_topics"], 1);
        let mut unsubscribe = Unsubscribe::new("moli-probe/maison/+/canal/#");
        unsubscribe.pkid = 3;
        send(&mut stream, Packet::Unsubscribe(unsubscribe)).await;
        assert_eq!(
            receive(&mut stream, &mut buffer).await,
            Packet::UnsubAck(UnsubAck::new(3))
        );
        broker
            .publish(topic.into(), b"after unsubscribe".to_vec(), true)
            .unwrap();
        send(&mut stream, Packet::PingReq).await;
        assert_eq!(receive(&mut stream, &mut buffer).await, Packet::PingResp);
        disconnect(stream, task).await;
    }

    #[tokio::test]
    async fn incoming_qos_one_runs_hook_before_storage_and_is_acknowledged() {
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let broker = Broker::new(Arc::new(move |role, topic, payload, retained| {
            assert_eq!(role, Role::Bridge);
            assert_eq!(topic, "moli-probe/runtime/fixture");
            assert!(retained);
            count.fetch_add(1, Ordering::Relaxed);
            Ok(payload.to_ascii_uppercase())
        }));
        broker.set_credentials("fixture-user".into(), "fixture-password".into());
        let (mut stream, mut buffer, task) = login(&broker, Role::Bridge, "publisher").await;
        let mut publish = Publish::new(
            "moli-probe/runtime/fixture",
            QoS::AtLeastOnce,
            b"transformed".to_vec(),
        );
        publish.pkid = 7;
        publish.retain = true;
        send(&mut stream, Packet::Publish(publish)).await;
        assert_eq!(
            receive(&mut stream, &mut buffer).await,
            Packet::PubAck(PubAck::new(7))
        );
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        send(&mut stream, subscription("moli-probe/runtime/fixture", 8)).await;
        assert!(matches!(
            receive(&mut stream, &mut buffer).await,
            Packet::SubAck(_)
        ));
        assert_eq!(
            publication(receive(&mut stream, &mut buffer).await)
                .payload
                .as_ref(),
            b"TRANSFORMED"
        );
        disconnect(stream, task).await;
    }

    #[tokio::test]
    async fn hook_denial_and_qos_two_never_reach_retained_storage() {
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let broker = Broker::new(Arc::new(move |_, _, _, _| {
            count.fetch_add(1, Ordering::Relaxed);
            bail!("publication denied")
        }));
        broker.set_credentials("fixture-user".into(), "fixture-password".into());
        for qos in [QoS::ExactlyOnce, QoS::AtLeastOnce] {
            let (mut stream, _, task) = login(&broker, Role::Bridge, "denied").await;
            let mut publish = Publish::new("moli-probe/runtime/denied", qos, b"denied".to_vec());
            publish.pkid = 1;
            publish.retain = true;
            send(&mut stream, Packet::Publish(publish)).await;
            assert!(task.await.unwrap().is_err());
        }
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(broker.stats()["retained_topics"], 0);
    }

    #[test]
    fn subscription_grammar_and_role_namespaces_do_not_leak_other_roots() {
        for filter in [
            "#",
            "+/#",
            "moli-probe/+/#",
            "moli-probe/source/#",
            "moli-probe/maison/#/secret",
            "moli-probe/maison/partial+",
            "moli-probe/maison/part#",
        ] {
            assert!(!allowed_filter(Role::Browser, filter), "{filter}");
        }
        for filter in [
            "moli-probe/maison/#",
            "moli-probe/runtime/#",
            "moli-probe/maison/fixture/config",
            "moli-probe/maison/+/canal/+/state",
        ] {
            assert!(allowed_filter(Role::Browser, filter));
        }
        assert!(allowed_filter(Role::Bridge, "moli-probe/#"));
        assert!(!allowed_filter(Role::Bridge, "#"));
        assert!(matches_filter("moli-probe/maison/#", "moli-probe/maison"));
        assert!(matches_filter(
            "moli-probe/+/state",
            "moli-probe/fixture/state"
        ));
        assert!(!matches_filter(
            "moli-probe/+/state",
            "moli-probe/fixture/extra/state"
        ));
    }

    #[tokio::test]
    async fn clients_subscriptions_and_slow_queues_have_hard_bounds() {
        let broker = fixture();
        let mut registrations = Vec::new();
        for _ in 0..MAX_CLIENTS {
            registrations.push(broker.register().unwrap());
        }
        assert!(broker.register().is_err());
        registrations.pop();
        assert!(broker.register().is_ok());
        drop(registrations);
        let (registration, _receiver, cancel) = broker.register().unwrap();
        let filters = (0..MAX_SUBSCRIPTIONS)
            .map(|i| SubscribeFilter {
                path: format!("moli-probe/maison/fixture/{i}"),
                qos: QoS::AtMostOnce,
            })
            .collect();
        broker
            .subscribe(
                registration.id,
                Role::Browser,
                Subscribe { pkid: 1, filters },
            )
            .unwrap();
        let (ack, _) = broker
            .subscribe(
                registration.id,
                Role::Browser,
                Subscribe {
                    pkid: 2,
                    filters: vec![SubscribeFilter {
                        path: "moli-probe/maison/fixture/overflow".into(),
                        qos: QoS::AtMostOnce,
                    }],
                },
            )
            .unwrap();
        assert_eq!(
            ack,
            Packet::SubAck(SubAck::new(2, vec![SubscribeReasonCode::Failure]))
        );
        for i in 0..=SEND_QUEUE {
            broker
                .publish(
                    "moli-probe/maison/fixture/0".into(),
                    i.to_string().into_bytes(),
                    true,
                )
                .unwrap();
        }
        assert!(cancel.is_cancelled());
        assert_eq!(broker.stats()["clients"], 0);
        assert_eq!(broker.stats()["retained_topics"], 1);
    }

    #[test]
    fn retained_topics_payloads_and_packets_have_hard_bounds() {
        let broker = fixture();
        for i in 0..MAX_RETAINED_TOPICS {
            broker
                .publish(format!("moli-probe/runtime/{i}"), vec![1], true)
                .unwrap();
        }
        assert!(
            broker
                .publish("moli-probe/runtime/overflow".into(), vec![1], true)
                .is_err()
        );
        broker
            .publish("moli-probe/runtime/0".into(), Vec::new(), true)
            .unwrap();
        broker
            .publish("moli-probe/runtime/overflow".into(), vec![1], true)
            .unwrap();
        assert!(
            broker
                .publish("moli-probe/runtime/huge".into(), vec![0; MAX_PACKET], true)
                .is_err()
        );
        let broker = fixture();
        for i in 0..8 {
            broker
                .publish(
                    format!("moli-probe/runtime/{i}"),
                    vec![0; MAX_PACKET - 2048],
                    true,
                )
                .unwrap();
        }
        assert!(
            broker
                .publish("moli-probe/runtime/overflow".into(), vec![0; 32768], true)
                .is_err()
        );
        assert_eq!(broker.stats()["retained_topics"], 8);
    }

    #[tokio::test]
    async fn retained_snapshot_larger_than_send_queue_streams_without_dropping_client() {
        let broker = fixture();
        for i in 0..SEND_QUEUE + 16 {
            broker
                .publish(
                    format!("moli-probe/maison/fixture/{i}"),
                    i.to_string().into_bytes(),
                    true,
                )
                .unwrap();
        }
        let (mut stream, mut buffer, task) = login(&broker, Role::Browser, "large-snapshot").await;
        send(&mut stream, subscription("moli-probe/maison/#", 1)).await;
        assert!(matches!(
            receive(&mut stream, &mut buffer).await,
            Packet::SubAck(_)
        ));
        let mut topics = BTreeSet::new();
        for _ in 0..SEND_QUEUE + 16 {
            let value = publication(receive(&mut stream, &mut buffer).await);
            assert!(value.retain);
            topics.insert(value.topic);
        }
        assert_eq!(topics.len(), SEND_QUEUE + 16);
        assert_eq!(broker.stats()["clients"], 1);
        disconnect(stream, task).await;
    }

    #[tokio::test]
    async fn malformed_control_flags_are_rejected_without_waiting_for_more_data() {
        let (mut client, mut server) = duplex(64);
        client.write_all(&[0xc1, 0]).await.unwrap();
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            read_packet(&mut server, &mut BytesMut::new()),
        )
        .await
        .unwrap();
        assert!(result.is_err());
    }

    #[test]
    fn disk_budget_counts_old_versions_until_the_last_reader_releases_them() {
        let broker = fixture();
        let path = std::env::temp_dir().join(format!(
            "moli-broker-budget-{}-{}",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        broker.set_description_directory(path.clone()).unwrap();
        let first = "moli-probe/maison/fixture/first/config";
        broker
            .publish(first.into(), vec![0; MAX_PACKET - 2048], true)
            .unwrap();
        let old = broker.lock().retained[first].clone();
        broker
            .publish(first.into(), vec![1; MAX_PACKET - 2048], true)
            .unwrap();
        for name in ["second", "third"] {
            broker
                .publish(
                    format!("moli-probe/maison/fixture/{name}/config"),
                    vec![2; MAX_PACKET - 2048],
                    true,
                )
                .unwrap();
        }
        let overflow = "moli-probe/maison/fixture/overflow/config";
        assert!(
            broker
                .publish(overflow.into(), vec![3; 16384], true)
                .is_err()
        );
        assert_eq!(std::fs::read_dir(&path).unwrap().count(), 4);
        drop(old);
        broker
            .publish(overflow.into(), vec![3; 16384], true)
            .unwrap();
        assert!(
            broker.stats()["description_disk_bytes"].as_u64().unwrap() <= MAX_DISK_BYTES as u64
        );
        drop(broker);
        assert_eq!(std::fs::read_dir(&path).unwrap().count(), 0);
        std::fs::remove_dir(path).unwrap();
    }

    #[tokio::test]
    async fn disk_descriptions_stream_and_only_change_on_new_metadata() {
        let broker = fixture();
        let path = std::env::temp_dir().join(format!(
            "moli-broker-fixture-{}-{}",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        broker.set_description_directory(path.clone()).unwrap();
        let topic = "moli-probe/maison/fixture/config";
        let payload = vec![b'x'; 40000];
        broker.publish(topic.into(), payload.clone(), true).unwrap();
        assert_eq!(broker.stats()["description_storage"], "disk");
        assert_eq!(broker.stats()["description_ram_bytes"], 0);
        assert_eq!(broker.stats()["description_disk_bytes"], 40000);
        let files: Vec<_> = std::fs::read_dir(&path)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        broker.publish(topic.into(), payload.clone(), true).unwrap();
        broker
            .publish(
                "moli-probe/maison/fixture/canal/state".into(),
                vec![1],
                true,
            )
            .unwrap();
        assert_eq!(
            std::fs::read_dir(&path)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect::<Vec<_>>(),
            files
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                std::fs::metadata(&files[0]).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let (mut stream, mut buffer, task) = login(&broker, Role::Browser, "disk-reader").await;
        send(&mut stream, subscription(topic, 1)).await;
        assert!(matches!(
            receive(&mut stream, &mut buffer).await,
            Packet::SubAck(_)
        ));
        let initial = publication(receive(&mut stream, &mut buffer).await);
        assert!(initial.retain);
        assert_eq!(initial.payload.as_ref(), payload);
        broker
            .publish(topic.into(), b"new metadata".to_vec(), true)
            .unwrap();
        let live = publication(receive(&mut stream, &mut buffer).await);
        assert!(!live.retain);
        assert_eq!(live.payload.as_ref(), b"new metadata");
        disconnect(stream, task).await;
        assert_eq!(std::fs::read_dir(&path).unwrap().count(), 1);
        assert_eq!(broker.stats()["description_disk_bytes"], 12);
        drop(broker);
        assert_eq!(std::fs::read_dir(&path).unwrap().count(), 0);
        std::fs::remove_dir(path).unwrap();
    }
}
