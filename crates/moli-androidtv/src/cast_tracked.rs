//! Correlated Cast controls, separate from the compatible Philips session.

use super::*;
use crate::tracked::{Delivery, ReaderTask, TrackedOrder, write_packet};
#[path = "cast_receiver.rs"]
mod receiver;

fn reject_pending<T>(orders: &mut mpsc::Receiver<TrackedOrder<T>>) {
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
    reject_pending(orders);
    stream(connected.context("cast")?, orders, events).await
}

async fn control<W: AsyncWrite + Unpin>(
    write: &mut W,
    destination: &str,
    ns: &str,
    body: &Json,
) -> anyhow::Result<()> {
    tokio::time::timeout(Duration::from_secs(4), send(write, destination, ns, body))
        .await
        .context("Cast protocol write timed out")?
}

fn packet(destination: &str, namespace: &str, body: &Json) -> Vec<u8> {
    let message = Writer::new()
        .uint(1, 0)
        .str(2, SENDER)
        .str(3, destination)
        .str(4, namespace)
        .uint(5, 0)
        .str(6, &body.to_string())
        .finish();
    let mut result = u32::try_from(message.len())
        .unwrap_or(u32::MAX)
        .to_be_bytes()
        .to_vec();
    result.extend_from_slice(&message);
    result
}

async fn observe<W: AsyncWrite + Unpin>(
    frame: &[u8],
    write: &mut W,
    follow: &mut Follow,
    request: &mut u64,
    events: &mpsc::Sender<Event>,
    volumes: Option<&mpsc::Sender<ReceiverVolume>>,
) -> anyhow::Result<()> {
    let Some((source, namespace, body)) = envelope(frame) else {
        return Ok(());
    };
    match (namespace.as_str(), body["type"].as_str()) {
        (NS_HEARTBEAT, Some("PING")) => {
            control(write, &source, NS_HEARTBEAT, &json!({"type":"PONG"})).await?;
        }
        (NS_RECEIVER, Some("RECEIVER_STATUS")) if source == RECEIVER => {
            if let Some(volumes) = volumes
                && let Some(volume) = receiver::observation(&body)
            {
                volumes.send(volume).await?;
            }
            if body["status"]["applications"].is_array()
                && let Some(transport) = follow.receiver(&body)
            {
                follow.playing = Playing {
                    app: follow.playing.app.take(),
                    ..Playing::default()
                };
                *request += 1;
                control(write, &transport, NS_CONNECTION, &json!({"type":"CONNECT"})).await?;
                control(
                    write,
                    &transport,
                    NS_MEDIA,
                    &json!({"type":"GET_STATUS","requestId":*request}),
                )
                .await?;
            }
        }
        (NS_MEDIA, Some("MEDIA_STATUS")) if follow.transport.as_deref() == Some(&source) => {
            follow.media(&body);
        }
        (NS_CONNECTION, Some("CLOSE")) if follow.transport.as_deref() == Some(&source) => {
            follow.clear();
        }
        _ => {}
    }
    events.send(Event::Playing(follow.playing.clone())).await?;
    Ok(())
}

#[derive(Debug)]
struct Pending<'a> {
    transport: &'a str,
    session: u64,
    id: u64,
    deadline: tokio::time::Instant,
    operation: Order,
}

async fn response<W: AsyncWrite + Unpin>(
    frames: &mut mpsc::Receiver<Vec<u8>>,
    write: &mut W,
    follow: &mut Follow,
    request: &mut u64,
    events: &mpsc::Sender<Event>,
    pending: &Pending<'_>,
    volumes: Option<&mpsc::Sender<ReceiverVolume>>,
) -> anyhow::Result<Delivery> {
    loop {
        if tokio::time::Instant::now() >= pending.deadline {
            return Ok(Delivery::Uncertain);
        }
        let Ok(Some(frame)) = tokio::time::timeout_at(pending.deadline, frames.recv()).await else {
            return Ok(Delivery::Uncertain);
        };
        let matched = envelope(&frame).filter(|(source, ns, body)| {
            source == pending.transport
                && ns == NS_MEDIA
                && body["requestId"].as_u64() == Some(pending.id)
        });
        let outcome = matched
            .as_ref()
            .and_then(|(_, _, body)| match body["type"].as_str() {
                Some("INVALID_REQUEST" | "INVALID_PLAYER_STATE" | "LOAD_FAILED" | "ERROR") => {
                    Some(Delivery::Refused)
                }
                Some("MEDIA_STATUS")
                    if body["status"].as_array().is_some_and(|items| {
                        // STOP unloads the media and invalidates mediaSessionId.
                        // Only an explicit empty array, with the exact correlation
                        // above, can acknowledge that terminal response.
                        (pending.operation == Order::Stop && items.is_empty())
                            || items.iter().any(|item| {
                                item["mediaSessionId"].as_u64() == Some(pending.session)
                            })
                    }) =>
                {
                    Some(Delivery::Acknowledged)
                }
                _ => None,
            });
        observe(&frame, write, follow, request, events, volumes).await?;
        if let Some(outcome) = outcome {
            return Ok(outcome);
        }
        if follow.transport.as_deref() != Some(pending.transport)
            || follow.session != Some(pending.session)
        {
            return Ok(Delivery::Uncertain);
        }
    }
}

pub(super) async fn stream<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>(
    stream: S,
    orders: &mut mpsc::Receiver<TrackedOrder<Order>>,
    events: &mpsc::Sender<Event>,
) -> anyhow::Result<()> {
    run(stream, orders, events, None, None).await
}
pub(super) async fn receiver_stream<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>(
    stream: S,
    orders: &mut mpsc::Receiver<TrackedOrder<ReceiverOrder>>,
    events: &mpsc::Sender<Event>,
    volumes: &mpsc::Sender<ReceiverVolume>,
    generation: std::sync::Arc<std::sync::atomic::AtomicU64>,
) -> anyhow::Result<()> {
    run(stream, orders, events, Some(volumes), Some(generation)).await
}
async fn run<S, O>(
    stream: S,
    orders: &mut mpsc::Receiver<TrackedOrder<O>>,
    events: &mpsc::Sender<Event>,
    volumes: Option<&mpsc::Sender<ReceiverVolume>>,
    generation: Option<std::sync::Arc<std::sync::atomic::AtomicU64>>,
) -> anyhow::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    O: Copy + Into<ReceiverOrder> + Send,
{
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
        let mut follow = Follow { generation, ..Follow::default() };
        let mut request = 0;
        let mut quiet = tokio::time::Instant::now() + QUIET;
        control(&mut write, RECEIVER, NS_CONNECTION, &json!({"type":"CONNECT"})).await?;
        let mut tick = tokio::time::interval(POLL);
        let mut ticks = 0u32;
        loop {
            tokio::select! {
                _ = tick.tick() => {
                    ticks = ticks.wrapping_add(1);
                    request += 1;
                    control(&mut write, RECEIVER, NS_HEARTBEAT, &json!({"type":"PING"})).await?;
                    control(&mut write, RECEIVER, NS_RECEIVER, &json!({"type":"GET_STATUS","requestId":request})).await?;
                    if let Some(transport) = follow.transport.as_deref().filter(|_| ticks.is_multiple_of(MEDIA_EVERY)) {
                        request += 1;
                        control(&mut write, transport, NS_MEDIA, &json!({"type":"GET_STATUS","requestId":request})).await?;
                    }
                }
                frame = tokio::time::timeout_at(quiet, frames.recv()) => {
                    let frame = frame.context("Cast went quiet")?.context("Cast disconnected")?;
                    quiet = tokio::time::Instant::now() + QUIET;
                    observe(&frame, &mut write, &mut follow, &mut request, events, volumes).await?;
                }
                order = orders.recv() => {
                    let Some(order) = order else { return Ok(()) };
                    if let Some(reason) = order.unsent_reason() { order.complete(reason); continue; }
                    let ReceiverOrder::Media(operation) = order.payload.into() else {
                            let result=receiver::execute(&mut write,&mut frames,&mut follow,&mut request,events,volumes,&order).await;
                            let delivery=result.as_ref().copied().unwrap_or(Delivery::Uncertain);
                            order.complete(delivery);
                            result?;
                            if delivery==Delivery::Uncertain {bail!("Cast receiver response lost; no replay");}
                            continue;
                    };
                    let (Some(transport), Some(session)) = (follow.transport.clone(), follow.session) else {
                        order.complete(Delivery::Unavailable);
                        continue;
                    };
                    request += 1;
                    let id = request;
                    let kind = match operation { Order::Play => "PLAY", Order::Pause => "PAUSE", Order::Stop => "STOP" };
                    let bytes = packet(&transport, NS_MEDIA, &json!({"type":kind,"mediaSessionId":session,"requestId":id}));
                    let sent = write_packet(&mut write, &bytes, &order).await;
                    if sent != Delivery::Sent {
                        order.complete(sent);
                        if matches!(sent, Delivery::Uncertain | Delivery::Unavailable) { bail!("Cast write lost; no replay"); }
                        continue;
                    }
                    let pending = Pending { transport: &transport, session, id, deadline: order.deadline, operation };
                    let answer = tokio::time::timeout_at(order.deadline,
                        response(&mut frames, &mut write, &mut follow, &mut request, events, &pending, volumes))
                        .await.unwrap_or(Ok(Delivery::Uncertain));
                    let delivery = answer.as_ref().copied().unwrap_or(Delivery::Uncertain);
                    order.complete(delivery);
                    answer?;
                    if delivery == Delivery::Uncertain { bail!("Cast response lost; no replay"); }
                    quiet = tokio::time::Instant::now() + QUIET;
                }
            }
        }
    }.await;
    reader.close().await;
    reject_pending(orders);
    result
}
