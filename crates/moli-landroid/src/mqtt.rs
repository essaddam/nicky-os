//! Just enough MQTT 3.1.1 to talk to Worx's broker: connect, subscribe,
//! publish an order once (QoS 1, acknowledged by the broker), keep the
//! connection alive. Packets are written and read by `rumqttc`'s codec;
//! the transport is a websocket ([`moli_net::ws::WebSocket`]), or a fake
//! broker in tests.

use std::future::Future;
use std::time::Duration;

use anyhow::{Context as _, bail};
use bytes::{Bytes, BytesMut};
use rumqttc::mqttbytes::v4::{Connect, ConnectReturnCode, Login, Packet, Publish, Subscribe};
use rumqttc::mqttbytes::{Error as CodecError, QoS};

/// A status is a few kilobytes: anything bigger is not one.
pub const MAX_PACKET: usize = 256 * 1024;
const SETUP: Duration = Duration::from_secs(10);

/// Whole messages over a connection, in both directions.
pub trait Frames: Send {
    fn send_if(
        &mut self,
        data: &[u8],
        wanted: impl Fn() -> bool + Send + Sync,
    ) -> impl Future<Output = anyhow::Result<bool>> + Send;
    /// Must be cancel-safe: it sits in a `select!` beside the orders.
    fn recv(&mut self) -> impl Future<Output = anyhow::Result<Vec<u8>>> + Send;
}

impl Frames for moli_net::ws::WebSocket {
    async fn send_if(
        &mut self,
        data: &[u8],
        wanted: impl Fn() -> bool + Send + Sync,
    ) -> anyhow::Result<bool> {
        self.send_binary_if(data, wanted).await
    }

    async fn recv(&mut self) -> anyhow::Result<Vec<u8>> {
        self.recv_data().await
    }
}

/// What the broker sent that the driver cares about.
#[derive(Debug, PartialEq)]
pub enum Incoming {
    Message {
        topic: String,
        payload: Bytes,
    },
    /// The broker has the order published with this packet id.
    PubAck(u16),
    Pong,
    Other,
}

pub struct Mqtt<F> {
    link: F,
    buf: BytesMut,
    last_id: u16,
}

impl<F> std::fmt::Debug for Mqtt<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mqtt").finish_non_exhaustive()
    }
}

fn encode(packet: &Packet) -> anyhow::Result<Vec<u8>> {
    let mut out = BytesMut::new();
    packet
        .write(&mut out, MAX_PACKET)
        .map_err(|e| anyhow::anyhow!("mqtt: {e:?}"))?;
    Ok(out.to_vec())
}

impl<F: Frames> Mqtt<F> {
    /// Connects as `client_id`, the credential in the user name (no
    /// password), and waits for the broker's yes.
    pub async fn open(
        link: F,
        client_id: &str,
        username: &str,
        keep_alive: Duration,
    ) -> anyhow::Result<Self> {
        let mut mqtt = Self {
            link,
            buf: BytesMut::new(),
            last_id: 0,
        };
        let mut connect = Connect::new(client_id);
        connect.keep_alive = u16::try_from(keep_alive.as_secs()).unwrap_or(u16::MAX);
        connect.clean_session = true;
        connect.login = Some(Login::new(username, ""));
        mqtt.write(&Packet::Connect(connect)).await?;
        let answer = tokio::time::timeout(SETUP, async {
            loop {
                if let Packet::ConnAck(ack) = mqtt.packet().await? {
                    break anyhow::Ok(ack);
                }
            }
        })
        .await
        .context("mqtt: no answer to the connection")??;
        if answer.code != ConnectReturnCode::Success {
            bail!("mqtt: connection refused ({:?})", answer.code);
        }
        Ok(mqtt)
    }

    /// Subscribes to `topics` (QoS 0: a status lost is replaced by the
    /// next one) and waits for the broker's yes to each.
    pub async fn subscribe(&mut self, topics: &[String]) -> anyhow::Result<()> {
        if topics.is_empty() {
            return Ok(());
        }
        let mut subscribe = Subscribe::new_many(
            topics
                .iter()
                .map(|t| rumqttc::mqttbytes::v4::SubscribeFilter::new(t.clone(), QoS::AtMostOnce)),
        );
        subscribe.pkid = self.next_id();
        let pkid = subscribe.pkid;
        self.write(&Packet::Subscribe(subscribe)).await?;
        let ack = tokio::time::timeout(SETUP, async {
            loop {
                if let Packet::SubAck(ack) = self.packet().await?
                    && ack.pkid == pkid
                {
                    break anyhow::Ok(ack);
                }
            }
        })
        .await
        .context("mqtt: no answer to the subscription")??;
        if ack.return_codes.len() != topics.len()
            || ack
                .return_codes
                .iter()
                .any(|c| matches!(c, rumqttc::mqttbytes::v4::SubscribeReasonCode::Failure))
        {
            bail!("mqtt: subscription refused");
        }
        Ok(())
    }

    /// Publishes once, QoS 1: the answer is [`Incoming::PubAck`] with the
    /// id returned. Never sent again (no `dup`): a lost answer stays lost.
    pub async fn publish(&mut self, topic: &str, payload: Vec<u8>) -> anyhow::Result<u16> {
        self.publish_if(topic, payload, || true)
            .await?
            .context("mqtt: publish not sent")
    }

    pub async fn publish_if(
        &mut self,
        topic: &str,
        payload: Vec<u8>,
        wanted: impl Fn() -> bool + Send + Sync,
    ) -> anyhow::Result<Option<u16>> {
        let mut publish = Publish::new(topic, QoS::AtLeastOnce, payload);
        publish.pkid = self.next_id();
        let pkid = publish.pkid;
        let bytes = encode(&Packet::Publish(publish))?;
        Ok(self.link.send_if(&bytes, wanted).await?.then_some(pkid))
    }

    pub async fn ping(&mut self) -> anyhow::Result<()> {
        self.write(&Packet::PingReq).await
    }

    /// The next packet the driver cares about. Cancel-safe.
    pub async fn next(&mut self) -> anyhow::Result<Incoming> {
        Ok(match self.packet().await? {
            Packet::Publish(p) => Incoming::Message {
                topic: p.topic,
                payload: p.payload,
            },
            Packet::PubAck(ack) => Incoming::PubAck(ack.pkid),
            Packet::PingResp => Incoming::Pong,
            Packet::Disconnect => bail!("mqtt: the broker disconnected"),
            _ => Incoming::Other,
        })
    }

    /// One packet. Cancel-safe: bytes are kept until a whole packet is
    /// there, and nothing is awaited after taking it.
    async fn packet(&mut self) -> anyhow::Result<Packet> {
        loop {
            match Packet::read(&mut self.buf, MAX_PACKET) {
                Ok(packet) => return Ok(packet),
                Err(CodecError::InsufficientBytes(_)) => {}
                Err(e) => bail!("mqtt: {e:?}"),
            }
            let data = self.link.recv().await?;
            self.buf.extend_from_slice(&data);
        }
    }

    async fn write(&mut self, packet: &Packet) -> anyhow::Result<()> {
        let bytes = encode(packet)?;
        self.link.send_if(&bytes, || true).await?;
        Ok(())
    }

    /// Packet ids from 1, wrapping (0 is not one).
    fn next_id(&mut self) -> u16 {
        self.last_id = self.last_id.checked_add(1).unwrap_or(1);
        self.last_id
    }
}

/// A broker in memory, for the tests: what the client wrote, and a way to
/// answer.
#[cfg(test)]
pub mod fake {
    use super::{Frames, MAX_PACKET, encode};
    use bytes::BytesMut;
    use rumqttc::mqttbytes::v4::Packet;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use tokio::sync::{Notify, mpsc};

    #[derive(Debug, Default)]
    pub struct Preparation {
        pub hold_next: AtomicBool,
        pub entered: Notify,
        pub release: Notify,
    }

    #[derive(Debug)]
    pub struct Link {
        pub to_broker: mpsc::UnboundedSender<Vec<u8>>,
        pub from_broker: mpsc::UnboundedReceiver<Vec<u8>>,
        pub preparation: Arc<Preparation>,
    }

    impl Frames for Link {
        async fn send_if(
            &mut self,
            data: &[u8],
            wanted: impl Fn() -> bool + Send + Sync,
        ) -> anyhow::Result<bool> {
            if data.first().is_some_and(|byte| byte >> 4 == 3)
                && self.preparation.hold_next.swap(false, Ordering::SeqCst)
            {
                self.preparation.entered.notify_one();
                self.preparation.release.notified().await;
            }
            if !wanted() {
                return Ok(false);
            }
            self.to_broker
                .send(data.to_vec())
                .map_err(|_| anyhow::anyhow!("broker gone"))?;
            Ok(true)
        }

        async fn recv(&mut self) -> anyhow::Result<Vec<u8>> {
            self.from_broker
                .recv()
                .await
                .ok_or_else(|| anyhow::anyhow!("broker closed the connection"))
        }
    }

    /// The broker's end.
    #[derive(Debug)]
    pub struct Broker {
        pub heard: mpsc::UnboundedReceiver<Vec<u8>>,
        pub say: mpsc::UnboundedSender<Vec<u8>>,
        buf: BytesMut,
        pub preparation: Arc<Preparation>,
    }

    #[must_use]
    pub fn pair() -> (Link, Broker) {
        let (to_broker, heard) = mpsc::unbounded_channel();
        let (say, from_broker) = mpsc::unbounded_channel();
        let preparation = Arc::new(Preparation::default());
        (
            Link {
                to_broker,
                from_broker,
                preparation: Arc::clone(&preparation),
            },
            Broker {
                heard,
                say,
                buf: BytesMut::new(),
                preparation,
            },
        )
    }

    impl Broker {
        /// The next packet the client wrote; `None` once it is gone.
        pub async fn packet(&mut self) -> Option<Packet> {
            loop {
                if let Ok(p) = Packet::read(&mut self.buf, MAX_PACKET) {
                    return Some(p);
                }
                let data = self.heard.recv().await?;
                self.buf.extend_from_slice(&data);
            }
        }

        pub fn send(&self, packet: &Packet) {
            let _ = self.say.send(encode(packet).unwrap());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rumqttc::mqttbytes::v4::{ConnAck, PubAck, SubAck, SubscribeReasonCode};

    #[tokio::test]
    async fn connects_subscribes_and_publishes_once() {
        let (link, mut broker) = fake::pair();
        let client = tokio::spawn(async move {
            let mut mqtt = Mqtt::open(
                link,
                "WX/USER/1/moli/x",
                "bot?jwt=a.b",
                Duration::from_secs(60),
            )
            .await?;
            mqtt.subscribe(&["a/out".to_owned()]).await?;
            let id = mqtt.publish("a/in", b"{\"cmd\":1}".to_vec()).await?;
            let next = mqtt.next().await?;
            anyhow::Ok((id, next))
        });
        let Some(Packet::Connect(connect)) = broker.packet().await else {
            panic!("connect");
        };
        assert_eq!(connect.client_id, "WX/USER/1/moli/x");
        let login = connect.login.unwrap();
        assert_eq!(
            (login.username.as_str(), login.password.as_str()),
            ("bot?jwt=a.b", "")
        );
        // Split across two frames: still one packet.
        let ack = encode(&Packet::ConnAck(ConnAck::new(
            ConnectReturnCode::Success,
            false,
        )))
        .unwrap();
        broker.say.send(ack[..1].to_vec()).unwrap();
        broker.say.send(ack[1..].to_vec()).unwrap();
        let Some(Packet::Subscribe(sub)) = broker.packet().await else {
            panic!("subscribe");
        };
        assert_eq!(sub.filters[0].path, "a/out");
        broker.send(&Packet::SubAck(SubAck::new(
            sub.pkid,
            vec![SubscribeReasonCode::Success(QoS::AtMostOnce)],
        )));
        let Some(Packet::Publish(publish)) = broker.packet().await else {
            panic!("publish");
        };
        assert_eq!(publish.qos, QoS::AtLeastOnce);
        assert!(!publish.dup);
        assert_eq!(&publish.payload[..], b"{\"cmd\":1}");
        broker.send(&Packet::PubAck(PubAck::new(publish.pkid)));
        let (id, next) = client.await.unwrap().unwrap();
        assert_eq!(next, Incoming::PubAck(id));
    }

    #[tokio::test]
    async fn a_refusal_is_an_error() {
        let (link, mut broker) = fake::pair();
        let client = tokio::spawn(Mqtt::open(link, "c", "u", Duration::from_secs(60)));
        broker.packet().await.unwrap();
        broker.send(&Packet::ConnAck(ConnAck::new(
            ConnectReturnCode::NotAuthorized,
            false,
        )));
        assert!(client.await.unwrap().is_err());
    }

    #[test]
    fn packet_ids_skip_zero() {
        let (link, _broker) = fake::pair();
        let mut mqtt = Mqtt {
            link,
            buf: BytesMut::new(),
            last_id: u16::MAX - 1,
        };
        assert_eq!(mqtt.next_id(), u16::MAX);
        assert_eq!(mqtt.next_id(), 1);
    }
}
