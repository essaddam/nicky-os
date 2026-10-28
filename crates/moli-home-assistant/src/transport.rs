//! Bounded MQTT transport. A failed connection is never reused or reconnected.
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Result, ensure};
use rumqttc::{
    AsyncClient, ConnectReturnCode, Event, LastWill, MqttOptions, Packet, QoS, SubscribeFilter,
    SubscribeReasonCode, Transport,
};
use tokio::{sync::mpsc, task::JoinHandle};

const MAX_PAYLOAD: usize = 1024 * 1024;
const MAX_TOPIC: usize = 4096;
const FILTERS: [&str; 7] = [
    "ha/inventory",
    "ha/state/#",
    "ha/availability",
    "ha/result/+",
    "ha/migration/proof",
    "ha/migration/result/+",
    "moli/command",
];

pub(crate) enum Incoming {
    Connected,
    Message {
        suffix: String,
        payload: Vec<u8>,
        retained: bool,
    },
    Disconnected,
}

pub(crate) struct Link {
    client: AsyncClient,
    incoming: mpsc::Receiver<Incoming>,
    alive: Arc<AtomicBool>,
    ended: bool,
    prefix: String,
    pump: JoinHandle<()>,
}

impl Drop for Link {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
        self.pump.abort();
    }
}

fn plain_topic(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('/')
        && !value.ends_with('/')
        && !value.contains(['+', '#'])
        && !value.chars().any(char::is_control)
        && value.split('/').all(|part| !part.is_empty())
}

fn topic(base: &str, suffix: &str, bytes: usize, retained: bool) -> Result<String> {
    ensure!(
        plain_topic(base) && plain_topic(suffix),
        "invalid MQTT bridge topic"
    );
    ensure!(
        base.len() + suffix.len() < MAX_TOPIC && bytes <= MAX_PAYLOAD,
        "MQTT bridge message exceeds its bound"
    );
    ensure!(
        !retained || !suffix.split('/').any(|part| part == "command"),
        "MQTT bridge commands cannot be retained"
    );
    Ok(format!("{base}/{suffix}"))
}

fn will(base: &str, installation: &str, session: &str) -> Result<LastWill> {
    let message = serde_json::to_vec(
        &serde_json::json!({"v":1,"installation":installation,"session":session,"online":false}),
    )?;
    Ok(LastWill::new(
        topic(base, "moli/availability", message.len(), true)?,
        message,
        QoS::AtLeastOnce,
        true,
    ))
}

impl Link {
    pub(crate) fn open(
        config: &crate::Config,
        username: String,
        password: String,
        session: &str,
    ) -> Result<Self> {
        ensure!(
            crate::protocol::valid_id(session) && crate::protocol::valid_id(&config.installation),
            "invalid MQTT bridge session identity"
        );
        ensure!(
            !username.is_empty()
                && !password.is_empty()
                && username.len() <= 4096
                && password.len() <= 4096,
            "MQTT credentials exceed their bound"
        );
        let prefix = format!("{}/", config.base_topic);
        let id = config
            .client_id
            .clone()
            .unwrap_or_else(|| format!("moli-ha-{}-{session}", config.installation));
        ensure!(
            !id.is_empty() && id.len() <= 512 && !id.chars().any(char::is_control),
            "invalid MQTT client identity"
        );
        let mut options = MqttOptions::new(id, &config.host, config.port);
        options.set_credentials(username, password);
        options.set_clean_session(true);
        options.set_keep_alive(Duration::from_secs(10));
        options.set_max_packet_size(MAX_PAYLOAD + 8192, MAX_PAYLOAD + 8192);
        options.set_last_will(will(&config.base_topic, &config.installation, session)?);
        if config.tls {
            options.set_transport(Transport::tls_with_default_config());
        }
        let (client, mut events) = AsyncClient::new(options, 16);
        let (sender, incoming) = mpsc::channel(32);
        let alive = Arc::new(AtomicBool::new(true));
        let active = alive.clone();
        let publisher = client.clone();
        let base = config.base_topic.clone();
        let expected = prefix.clone();
        // Only this task polls the event loop. First error drops its entire
        // pending MQTT state; the driver supervisor owns the next session.
        let pump = tokio::spawn(async move {
            loop {
                let incoming = match events.poll().await {
                    Ok(Event::Incoming(Packet::ConnAck(ack))) => {
                        if ack.code != ConnectReturnCode::Success {
                            break;
                        }
                        let filters = FILTERS.iter().map(|suffix| {
                            SubscribeFilter::new(format!("{base}/{suffix}"), QoS::AtLeastOnce)
                        });
                        if publisher.try_subscribe_many(filters).is_err() {
                            break;
                        }
                        Some(Incoming::Connected)
                    }
                    Ok(Event::Incoming(Packet::SubAck(ack))) => {
                        if ack.return_codes.len() != FILTERS.len()
                            || ack.return_codes.contains(&SubscribeReasonCode::Failure)
                        {
                            break;
                        }
                        None
                    }
                    Ok(Event::Incoming(Packet::Publish(message))) => {
                        if message.payload.len() > MAX_PAYLOAD || message.topic.len() > MAX_TOPIC {
                            break;
                        }
                        let Some(suffix) = message.topic.strip_prefix(&expected) else {
                            break;
                        };
                        if !plain_topic(suffix) {
                            break;
                        }
                        Some(Incoming::Message {
                            suffix: suffix.into(),
                            payload: message.payload.to_vec(),
                            retained: message.retain,
                        })
                    }
                    Ok(_) => None,
                    Err(_) => break,
                };
                if let Some(incoming) = incoming
                    && sender.send(incoming).await.is_err()
                {
                    break;
                }
            }
            active.store(false, Ordering::Release);
            let _ = sender.try_send(Incoming::Disconnected);
        });
        Ok(Self {
            client,
            incoming,
            alive,
            ended: false,
            prefix,
            pump,
        })
    }

    pub(crate) fn publish(&self, suffix: &str, bytes: Vec<u8>, retained: bool) -> Result<()> {
        ensure!(
            self.alive.load(Ordering::Acquire),
            "MQTT bridge link is closed"
        );
        let base = self
            .prefix
            .strip_suffix('/')
            .expect("configured topic prefix");
        let topic = topic(base, suffix, bytes.len(), retained)?;
        let qos = if retained {
            QoS::AtLeastOnce
        } else {
            QoS::AtMostOnce
        };
        self.client
            .try_publish(topic, qos, retained, bytes)
            .map_err(|_| anyhow::anyhow!("MQTT bridge outgoing queue is full or closed"))
    }

    pub(crate) async fn next(&mut self) -> Option<Incoming> {
        if self.ended {
            return None;
        }
        let incoming = if self.alive.load(Ordering::Acquire) {
            self.incoming.recv().await
        } else {
            None
        };
        if !self.alive.load(Ordering::Acquire) || incoming.is_none() {
            self.ended = true;
            self.incoming.close();
            while self.incoming.try_recv().is_ok() {}
            return Some(Incoming::Disconnected);
        }
        incoming
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topics_and_payloads_are_bounded_and_commands_never_retained() {
        for invalid in [
            "",
            "/ha/state",
            "ha//state",
            "ha/state/",
            "ha/+",
            "ha/#",
            "ha/\0state",
        ] {
            assert!(topic("fixture/bridge", invalid, 1, false).is_err());
        }
        assert!(topic("fixture/bridge", "moli/command", 1, true).is_err());
        assert!(topic("fixture/bridge", "moli/command/result", 1, true).is_err());
        assert!(topic("fixture/bridge", "ha/inventory", MAX_PAYLOAD + 1, true).is_err());
        assert!(topic(&"x".repeat(MAX_TOPIC), "ha/state", 1, false).is_err());
        assert_eq!(
            topic("fixture/bridge", "moli/command", MAX_PAYLOAD, false).unwrap(),
            "fixture/bridge/moli/command"
        );
    }

    #[test]
    fn last_will_identifies_the_installation_and_exact_consumer_session() {
        let will = will("fixture/bridge", "installation-a", "session-a").unwrap();
        assert_eq!(will.topic, "fixture/bridge/moli/availability");
        assert!(will.retain);
        assert_eq!(will.qos, QoS::AtLeastOnce);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&will.message).unwrap(),
            serde_json::json!({"v":1,"installation":"installation-a","session":"session-a","online":false})
        );
        assert_eq!(
            FILTERS,
            [
                "ha/inventory",
                "ha/state/#",
                "ha/availability",
                "ha/result/+",
                "ha/migration/proof",
                "ha/migration/result/+",
                "moli/command"
            ]
        );
    }
}
