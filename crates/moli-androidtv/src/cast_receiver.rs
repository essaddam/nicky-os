//! Receiver volume is independent from the application's media session.
use super::{
    AsyncWrite, Delivery, Event, Follow, Json, NS_RECEIVER, RECEIVER, ReceiverOrder,
    ReceiverVolume, TrackedOrder, envelope, json, mpsc, observe, packet, write_packet,
};

pub(super) fn observation(body: &Json) -> Option<ReceiverVolume> {
    let volume = &body["status"]["volume"];
    if !volume.is_object() {
        return None;
    }
    Some(ReceiverVolume {
        level: volume["level"]
            .as_f64()
            .filter(|value| value.is_finite() && (0.0..=1.0).contains(value)),
        muted: volume["muted"].as_bool(),
    })
}
fn confirmed(operation: ReceiverOrder, body: &Json) -> bool {
    let Some(volume) = observation(body) else {
        return false;
    };
    match operation {
        ReceiverOrder::Volume(target) => volume
            .level
            .is_some_and(|actual| (actual - target).abs() < 0.00001),
        ReceiverOrder::Mute(target) => volume.muted == Some(target),
        ReceiverOrder::Media(_) => false,
    }
}
pub(super) async fn execute<W: AsyncWrite + Unpin, O: Copy + Into<ReceiverOrder>>(
    write: &mut W,
    frames: &mut mpsc::Receiver<Vec<u8>>,
    follow: &mut Follow,
    request: &mut u64,
    events: &mpsc::Sender<Event>,
    volumes: Option<&mpsc::Sender<ReceiverVolume>>,
    order: &TrackedOrder<O>,
) -> anyhow::Result<Delivery> {
    let operation = order.payload.into();
    let volume = match operation {
        ReceiverOrder::Volume(level) if level.is_finite() && (0.0..=1.0).contains(&level) => {
            json!({"level":level})
        }
        ReceiverOrder::Mute(muted) => json!({"muted":muted}),
        _ => return Ok(Delivery::Refused),
    };
    *request += 1;
    let id = *request;
    let packet = packet(
        RECEIVER,
        NS_RECEIVER,
        &json!({"type":"SET_VOLUME","volume":volume,"requestId":id}),
    );
    let sent = write_packet(write, &packet, order).await;
    if sent != Delivery::Sent {
        return Ok(sent);
    }
    loop {
        let Ok(Some(frame)) = tokio::time::timeout_at(order.deadline, frames.recv()).await else {
            return Ok(Delivery::Uncertain);
        };
        let answer = envelope(&frame).filter(|(source, namespace, body)| {
            source == RECEIVER && namespace == NS_RECEIVER && body["requestId"].as_u64() == Some(id)
        });
        let outcome = answer
            .as_ref()
            .and_then(|(_, _, body)| match body["type"].as_str() {
                Some("INVALID_REQUEST" | "ERROR") => Some(Delivery::Refused),
                Some("RECEIVER_STATUS") if confirmed(operation, body) => {
                    Some(Delivery::Acknowledged)
                }
                _ => None,
            });
        observe(&frame, write, follow, request, events, volumes).await?;
        if let Some(outcome) = outcome {
            return Ok(outcome);
        }
    }
}
