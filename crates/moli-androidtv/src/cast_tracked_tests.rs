use std::sync::Arc;

use tokio::time::{Instant, timeout};

use super::*;
use crate::tracked::{Delivery, TrackedOrder};

async fn next(peer: &mut tokio::io::DuplexStream) -> (String, String, Json) {
    let frame = read_frame(peer).await;
    assert!(frame.is_ok(), "Cast session must emit its protocol frame");
    let frame = frame.unwrap();
    let m = parse(&frame).unwrap();
    (
        m.str(3).unwrap().to_owned(),
        m.str(4).unwrap().to_owned(),
        serde_json::from_str(m.str(6).unwrap()).unwrap(),
    )
}

async fn tell(peer: &mut tokio::io::DuplexStream, source: &str, namespace: &str, body: Json) {
    let message = Writer::new()
        .uint(1, 0)
        .str(2, source)
        .str(3, SENDER)
        .str(4, namespace)
        .uint(5, 0)
        .str(6, &body.to_string())
        .finish();
    peer.write_all(&u32::try_from(message.len()).unwrap().to_be_bytes())
        .await
        .unwrap();
    peer.write_all(&message).await.unwrap();
}

async fn media_ready(peer: &mut tokio::io::DuplexStream, events: &mut mpsc::Receiver<Event>) {
    let (_, namespace, body) = next(peer).await;
    assert_eq!(namespace, NS_CONNECTION);
    assert_eq!(body["type"], "CONNECT");
    tell(
        peer,
        RECEIVER,
        NS_RECEIVER,
        json!({"type":"RECEIVER_STATUS","status":{"applications":[{
            "displayName":"Example player", "transportId":"t-1", "namespaces":[{"name":NS_MEDIA}]
        }]}}),
    )
    .await;
    loop {
        let (destination, namespace, body) = next(peer).await;
        if destination == "t-1" && namespace == NS_MEDIA && body["type"] == "GET_STATUS" {
            break;
        }
    }
    tell(
        peer,
        "t-1",
        NS_MEDIA,
        json!({"type":"MEDIA_STATUS","status":[{"mediaSessionId":7,"playerState":"PLAYING"}]}),
    )
    .await;
    loop {
        if let Some(Event::Playing(p)) = events.recv().await {
            if p.state.as_deref() == Some("playing") {
                break;
            }
        } else {
            panic!("Cast media observation missing");
        }
    }
}

#[tokio::test(start_paused = true)]
async fn tracked_cast_refuses_without_a_media_session() {
    let (stream, mut peer) = tokio::io::duplex(8192);
    let (tx, mut rx) = mpsc::channel(4);
    let (events, _incoming) = mpsc::channel(16);
    let task = tokio::spawn(async move { tracked_stream(stream, &mut rx, &events).await });
    let _ = next(&mut peer).await;
    let (order, result) = TrackedOrder::new(
        Order::Play,
        Instant::now() + Duration::from_secs(5),
        Arc::new(|| false),
    );
    tx.send(order).await.unwrap();
    assert_eq!(result.await.unwrap(), Delivery::Unavailable);
    drop(tx);
    assert!(
        timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .is_ok()
    );
}

#[tokio::test(start_paused = true)]
async fn cast_ack_requires_both_matching_request_id_and_transport_source() {
    let (stream, mut peer) = tokio::io::duplex(8192);
    let (tx, mut rx) = mpsc::channel(4);
    let (events, mut incoming) = mpsc::channel(16);
    let task = tokio::spawn(async move { tracked_stream(stream, &mut rx, &events).await });
    media_ready(&mut peer, &mut incoming).await;
    let (order, mut result) = TrackedOrder::new(
        Order::Pause,
        Instant::now() + Duration::from_secs(5),
        Arc::new(|| false),
    );
    tx.send(order).await.unwrap();
    let request_id = loop {
        let (_, _, body) = next(&mut peer).await;
        if body["type"] == "PAUSE" {
            break body["requestId"].as_u64().unwrap();
        }
    };
    for (source, id) in [("t-1", request_id + 1), ("unrelated", request_id)] {
        tell(&mut peer, source, NS_MEDIA, json!({"type":"MEDIA_STATUS","requestId":id,"status":[{"mediaSessionId":7,"playerState":"PAUSED"}]})).await;
        tokio::task::yield_now().await;
        assert!(matches!(
            result.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
    }
    tell(&mut peer, "t-1", NS_MEDIA, json!({"type":"MEDIA_STATUS","requestId":request_id,"status":[{"mediaSessionId":7,"playerState":"PAUSED"}]})).await;
    assert_eq!(result.await.unwrap(), Delivery::Acknowledged);
    drop(tx);
    task.await.unwrap().unwrap();
}

#[tokio::test(start_paused = true)]
async fn lost_cast_response_is_uncertain_not_a_success_or_a_replay() {
    let (stream, mut peer) = tokio::io::duplex(8192);
    let (tx, mut rx) = mpsc::channel(4);
    let (events, mut incoming) = mpsc::channel(16);
    let task = tokio::spawn(async move { tracked_stream(stream, &mut rx, &events).await });
    media_ready(&mut peer, &mut incoming).await;
    let (order, result) = TrackedOrder::new(
        Order::Pause,
        Instant::now() + Duration::from_secs(5),
        Arc::new(|| false),
    );
    tx.send(order).await.unwrap();
    loop {
        if next(&mut peer).await.2["type"] == "PAUSE" {
            break;
        }
    }
    drop(peer);
    assert_eq!(result.await.unwrap(), Delivery::Uncertain);
    assert!(
        timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
}

#[tokio::test(start_paused = true)]
async fn silent_cast_response_expires_without_claiming_playback() {
    let (stream, mut peer) = tokio::io::duplex(8192);
    let (tx, mut rx) = mpsc::channel(4);
    let (events, mut incoming) = mpsc::channel(16);
    let task = tokio::spawn(async move { tracked_stream(stream, &mut rx, &events).await });
    media_ready(&mut peer, &mut incoming).await;
    let (order, result) = TrackedOrder::new(
        Order::Pause,
        Instant::now() + Duration::from_secs(5),
        Arc::new(|| false),
    );
    tx.send(order).await.unwrap();
    loop {
        if next(&mut peer).await.2["type"] == "PAUSE" {
            break;
        }
    }
    tokio::time::advance(Duration::from_secs(6)).await;
    assert_eq!(result.await.unwrap(), Delivery::Uncertain);
    assert!(task.await.unwrap().is_err());
}

#[tokio::test(start_paused = true)]
async fn cancelling_cast_session_closes_its_spawned_reader() {
    let (stream, mut peer) = tokio::io::duplex(8192);
    let (_tx, mut rx) = mpsc::channel(4);
    let (events, mut incoming) = mpsc::channel(16);
    let task = tokio::spawn(async move { tracked_stream(stream, &mut rx, &events).await });
    media_ready(&mut peer, &mut incoming).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let drain = async { while read_frame(&mut peer).await.is_ok() {} };
    assert!(
        timeout(Duration::from_secs(1), drain).await.is_ok(),
        "cancelling Cast must release the reader's stream half"
    );
}

#[tokio::test(start_paused = true)]
async fn new_cast_transport_in_the_same_app_clears_old_media() {
    let (stream, mut peer) = tokio::io::duplex(8192);
    let (_tx, mut rx) = mpsc::channel(4);
    let (events, mut incoming) = mpsc::channel(16);
    let task = tokio::spawn(async move { tracked_stream(stream, &mut rx, &events).await });
    media_ready(&mut peer, &mut incoming).await;
    tell(&mut peer, "t-1", NS_MEDIA, json!({"type":"MEDIA_STATUS","status":[{"mediaSessionId":7,"playerState":"PLAYING","media":{"metadata":{"title":"Old title"}}}]})).await;
    let Some(Event::Playing(old)) = incoming.recv().await else {
        panic!("old status missing");
    };
    assert_eq!(old.title.as_deref(), Some("Old title"));
    tell(&mut peer, RECEIVER, NS_RECEIVER, json!({"type":"RECEIVER_STATUS","status":{"applications":[{"displayName":"Example player","transportId":"t-2","namespaces":[{"name":NS_MEDIA}]}]}})).await;
    let Some(Event::Playing(new)) = incoming.recv().await else {
        panic!("new status missing");
    };
    assert_eq!(new.state, None);
    assert_eq!(new.title, None);
    task.abort();
    let _ = task.await;
}

struct ControlCase {
    peer: tokio::io::DuplexStream,
    orders: mpsc::Sender<TrackedOrder<Order>>,
    incoming: mpsc::Receiver<Event>,
    task: tokio::task::JoinHandle<anyhow::Result<()>>,
    result: tokio::sync::oneshot::Receiver<Delivery>,
    id: u64,
}
impl ControlCase {
    async fn new(operation: Order) -> Self {
        let (stream, mut peer) = tokio::io::duplex(8192);
        let (orders, mut rx) = mpsc::channel(4);
        let (events, mut incoming) = mpsc::channel(16);
        let task = tokio::spawn(async move { tracked_stream(stream, &mut rx, &events).await });
        media_ready(&mut peer, &mut incoming).await;
        tell(&mut peer, "t-1", NS_MEDIA, json!({"type":"MEDIA_STATUS","status":[{"mediaSessionId":7,"playerState":"PLAYING","media":{"metadata":{"title":"Example title"}}}]})).await;
        loop {
            let Some(Event::Playing(playing)) = incoming.recv().await else {
                panic!("media observation missing");
            };
            if playing.title.as_deref() == Some("Example title") {
                break;
            }
        }
        let (order, result) = TrackedOrder::new(
            operation,
            Instant::now() + Duration::from_secs(5),
            Arc::new(|| false),
        );
        orders.send(order).await.unwrap();
        let id = loop {
            let (_, _, body) = next(&mut peer).await;
            if body["type"]
                == match operation {
                    Order::Play => "PLAY",
                    Order::Pause => "PAUSE",
                    Order::Stop => "STOP",
                }
            {
                break body["requestId"].as_u64().unwrap();
            }
        };
        Self {
            peer,
            orders,
            incoming,
            task,
            result,
            id,
        }
    }
}

#[tokio::test(start_paused = true)]
async fn stop_acknowledges_an_exact_empty_status_and_keeps_the_connection_without_replay() {
    let mut c = ControlCase::new(Order::Stop).await;
    tell(
        &mut c.peer,
        "t-1",
        NS_MEDIA,
        json!({"type":"MEDIA_STATUS","requestId":c.id,"status":[]}),
    )
    .await;
    assert_eq!((&mut c.result).await.unwrap(), Delivery::Acknowledged);
    let Some(Event::Playing(playing)) = c.incoming.recv().await else {
        panic!("terminal status missing");
    };
    assert_eq!(playing.app.as_deref(), Some("Example player"));
    assert_eq!(playing.state, None);
    assert_eq!(playing.title, None);
    let (play, result) = TrackedOrder::new(
        Order::Play,
        Instant::now() + Duration::from_secs(5),
        Arc::new(|| false),
    );
    c.orders.send(play).await.unwrap();
    assert_eq!(result.await.unwrap(), Delivery::Unavailable);
    // The connection can observe a new session; the completed STOP is never replayed.
    tell(&mut c.peer, "t-1", NS_MEDIA, json!({"type":"MEDIA_STATUS","status":[{"mediaSessionId":8,"playerState":"PAUSED","media":{"metadata":{"title":"New title"}}}]})).await;
    let Some(Event::Playing(playing)) = c.incoming.recv().await else {
        panic!("new observation missing");
    };
    assert_eq!(playing.title.as_deref(), Some("New title"));
    drop(c.orders);
    c.task.await.unwrap().unwrap();
    while let Ok(frame) = read_frame(&mut c.peer).await {
        let (_, _, body) = envelope(&frame).unwrap();
        assert_ne!(body["type"], "STOP", "the completed STOP was replayed");
    }
}

#[tokio::test(start_paused = true)]
async fn stop_empty_status_from_another_source_or_namespace_is_not_an_ack() {
    let mut c = ControlCase::new(Order::Stop).await;
    for (source, ns) in [("other-transport", NS_MEDIA), ("t-1", NS_RECEIVER)] {
        tell(
            &mut c.peer,
            source,
            ns,
            json!({"type":"MEDIA_STATUS","requestId":c.id,"status":[]}),
        )
        .await;
        let Some(Event::Playing(playing)) = c.incoming.recv().await else {
            panic!("observation missing");
        };
        assert_eq!(playing.title.as_deref(), Some("Example title"));
        assert!(matches!(
            c.result.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
    }
    tell(
        &mut c.peer,
        "t-1",
        NS_MEDIA,
        json!({"type":"MEDIA_STATUS","requestId":c.id,"status":[]}),
    )
    .await;
    assert_eq!((&mut c.result).await.unwrap(), Delivery::Acknowledged);
    drop(c.orders);
    c.task.await.unwrap().unwrap();
}

#[tokio::test(start_paused = true)]
async fn stop_spontaneous_or_mismatched_empty_status_never_proves_completion() {
    for id in [None, Some(0), Some(u64::MAX)] {
        let mut c = ControlCase::new(Order::Stop).await;
        let mut body = json!({"type":"MEDIA_STATUS","status":[]});
        if let Some(id) = id {
            body["requestId"] = json!(id);
        }
        tell(&mut c.peer, "t-1", NS_MEDIA, body).await;
        assert_eq!(c.result.await.unwrap(), Delivery::Uncertain);
        assert!(c.task.await.unwrap().is_err());
    }
}

#[tokio::test(start_paused = true)]
async fn stop_missing_or_malformed_status_is_not_a_success() {
    for status in [None, Some(Json::Null), Some(json!({})), Some(json!([{}]))] {
        let mut c = ControlCase::new(Order::Stop).await;
        let mut body = json!({"type":"MEDIA_STATUS","requestId":c.id});
        if let Some(status) = status {
            body["status"] = status;
        }
        tell(&mut c.peer, "t-1", NS_MEDIA, body).await;
        assert_eq!(c.result.await.unwrap(), Delivery::Uncertain);
        assert!(c.task.await.unwrap().is_err());
    }
}

#[tokio::test(start_paused = true)]
async fn stop_protocol_errors_are_refused_without_fabricating_media_state() {
    for kind in [
        "INVALID_REQUEST",
        "INVALID_PLAYER_STATE",
        "LOAD_FAILED",
        "ERROR",
    ] {
        let mut c = ControlCase::new(Order::Stop).await;
        tell(
            &mut c.peer,
            "t-1",
            NS_MEDIA,
            json!({"type":kind,"requestId":c.id}),
        )
        .await;
        assert_eq!(c.result.await.unwrap(), Delivery::Refused);
        let Some(Event::Playing(playing)) = c.incoming.recv().await else {
            panic!("observation missing");
        };
        assert_eq!(playing.title.as_deref(), Some("Example title"));
        assert_eq!(playing.state.as_deref(), Some("playing"));
        drop(c.orders);
        c.task.await.unwrap().unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn play_and_pause_do_not_accept_stops_empty_terminal_status() {
    for operation in [Order::Play, Order::Pause] {
        let mut c = ControlCase::new(operation).await;
        tell(
            &mut c.peer,
            "t-1",
            NS_MEDIA,
            json!({"type":"MEDIA_STATUS","requestId":c.id,"status":[]}),
        )
        .await;
        assert_eq!(c.result.await.unwrap(), Delivery::Uncertain);
        assert!(c.task.await.unwrap().is_err());
    }
}
