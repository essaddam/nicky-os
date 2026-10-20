//! Additive Remote transport: one session, no queued order survives its loss.

use super::*;
use crate::tracked::{Delivery, ReaderTask, TrackedOrder, write_packet};

fn reject_pending(orders: &mut mpsc::Receiver<TrackedOrder<Order>>) {
    while let Ok(order) = orders.try_recv() {
        order.complete(Delivery::Unavailable);
    }
}

pub(super) async fn session(
    tls: &PinnedTls,
    host: &str,
    orders: &mut mpsc::Receiver<TrackedOrder<Order>>,
    events: &mpsc::Sender<Event>,
) -> anyhow::Result<()> {
    reject_pending(orders);
    let connected = tls.connect(host, PORT).await;
    // Requests arriving during reconnect were not made against a ready session.
    reject_pending(orders);
    stream(connected.context("android tv remote")?, orders, events).await
}

pub(super) async fn stream<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>(
    stream: S,
    orders: &mut mpsc::Receiver<TrackedOrder<Order>>,
    events: &mpsc::Sender<Event>,
) -> anyhow::Result<()> {
    let (mut read, mut write) = tokio::io::split(stream);
    let (frames_tx, mut frames) = mpsc::channel(16);
    let reader = ReaderTask::new(tokio::spawn(async move {
        while let Ok(frame) = read_frame(&mut read).await {
            if frames_tx.send(frame).await.is_err() {
                break;
            }
        }
    }));
    let result = async {
        let mut active = false;
        let mut field = (0, 0);
        let mut quiet = tokio::time::Instant::now() + IDLE;
        loop {
            tokio::select! {
                frame = tokio::time::timeout_at(quiet, frames.recv()) => {
                    let frame = frame.context("the TV went quiet")?.context("remote disconnected")?;
                    quiet = tokio::time::Instant::now() + IDLE;
                    if let Some(counters) = text_field(&frame) { field = counters; }
                    let was_active = parse(&frame).is_some_and(|message| message.has(2));
                    if let Some(reply) = handle(&frame, events).await? {
                        tokio::time::timeout(Duration::from_secs(4), send(&mut write, &reply))
                            .await.context("remote handshake write timed out")??;
                    }
                    active |= was_active;
                }
                order = orders.recv() => {
                    let Some(order) = order else { return Ok(()) };
                    if !active {
                        order.complete(Delivery::Unavailable);
                        continue;
                    }
                    if let Some(reason) = order.unsent_reason() {
                        order.complete(reason);
                        continue;
                    }
                    if matches!(&order.payload, Order::Launch(text) | Order::Text(text) if text.len() > MAX_MESSAGE - 64) {
                        order.complete(Delivery::Refused);
                        continue;
                    }
                    let body = encode(&order.payload, field);
                    let mut packet = Vec::with_capacity(body.len() + 5);
                    varint(body.len() as u64, &mut packet);
                    packet.extend_from_slice(&body);
                    let delivery = write_packet(&mut write, &packet, &order).await;
                    order.complete(delivery);
                    if matches!(delivery, Delivery::Unavailable | Delivery::Uncertain) {
                        bail!("remote write lost; no replay");
                    }
                }
            }
        }
    }.await;
    reader.close().await;
    reject_pending(orders);
    result
}
