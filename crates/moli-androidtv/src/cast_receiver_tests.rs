use super::*;
use crate::tracked::{Delivery, TrackedOrder};
use std::sync::Arc;
use tokio::time::{Instant, timeout};
async fn next(peer: &mut tokio::io::DuplexStream) -> (String, String, Json) {
    let frame = timeout(Duration::from_secs(1), read_frame(peer))
        .await
        .expect("receiver must emit before timeout")
        .expect("receiver must emit a protocol frame");
    let data = parse(&frame).unwrap();
    (
        data.str(3).unwrap().into(),
        data.str(4).unwrap().into(),
        serde_json::from_str(data.str(6).unwrap()).unwrap(),
    )
}
async fn tell(peer: &mut tokio::io::DuplexStream, source: &str, ns: &str, body: Json) {
    let bytes = Writer::new()
        .uint(1, 0)
        .str(2, source)
        .str(3, SENDER)
        .str(4, ns)
        .uint(5, 0)
        .str(6, &body.to_string())
        .finish();
    peer.write_all(&u32::try_from(bytes.len()).unwrap().to_be_bytes())
        .await
        .unwrap();
    peer.write_all(&bytes).await.unwrap();
}
#[tokio::test(start_paused = true)]
async fn receiver_volume_uses_receiver_namespace_without_a_media_session_and_exact_ack() {
    let (stream, mut peer) = tokio::io::duplex(8192);
    let (tx, mut rx) = mpsc::channel(4);
    let (events, _media) = mpsc::channel(32);
    let (volumes, mut incoming) = mpsc::channel(8);
    let task = tokio::spawn(async move {
        tracked_receiver_stream(stream, &mut rx, &events, &volumes, Arc::default()).await
    });
    let _ = next(&mut peer).await;
    let (order, mut result) = TrackedOrder::new(
        ReceiverOrder::Volume(0.4),
        Instant::now() + Duration::from_secs(5),
        Arc::new(|| false),
    );
    tx.send(order).await.unwrap();
    let id = loop {
        let (dest, ns, body) = next(&mut peer).await;
        if body["type"] == "SET_VOLUME" {
            assert_eq!(dest, RECEIVER);
            assert_eq!(ns, NS_RECEIVER);
            assert_eq!(body["volume"]["level"], 0.4);
            assert!(body.get("mediaSessionId").is_none());
            break body["requestId"].as_u64().unwrap();
        }
    };
    tell(
        &mut peer,
        RECEIVER,
        NS_RECEIVER,
        json!({"type":"RECEIVER_STATUS","requestId":id+1,"status":{"volume":{"level":0.8}}}),
    )
    .await;
    assert_eq!(incoming.recv().await.unwrap().level, Some(0.8));
    assert!(matches!(
        result.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    tell(&mut peer,RECEIVER,NS_RECEIVER,json!({"type":"RECEIVER_STATUS","requestId":id,"status":{"volume":{"level":0.4,"muted":false}}})).await;
    assert_eq!(result.await.unwrap(), Delivery::Acknowledged);
    assert_eq!(
        incoming.recv().await.unwrap(),
        ReceiverVolume {
            level: Some(0.4),
            muted: Some(false)
        }
    );
    drop(tx);
    task.await.unwrap().unwrap();
}
#[tokio::test(start_paused = true)]
async fn receiver_mute_is_absolute_and_foreign_status_does_not_acknowledge_it() {
    let (stream, mut peer) = tokio::io::duplex(8192);
    let (tx, mut rx) = mpsc::channel(4);
    let (events, _media) = mpsc::channel(32);
    let (volumes, mut incoming) = mpsc::channel(8);
    let task = tokio::spawn(async move {
        tracked_receiver_stream(stream, &mut rx, &events, &volumes, Arc::default()).await
    });
    let _ = next(&mut peer).await;
    let (order, mut result) = TrackedOrder::new(
        ReceiverOrder::Mute(true),
        Instant::now() + Duration::from_secs(5),
        Arc::new(|| false),
    );
    tx.send(order).await.unwrap();
    let id = loop {
        let (_, _, body) = next(&mut peer).await;
        if body["type"] == "SET_VOLUME" {
            assert_eq!(body["volume"]["muted"], true);
            assert!(body["volume"].get("level").is_none());
            break body["requestId"].as_u64().unwrap();
        }
    };
    tell(
        &mut peer,
        "other",
        NS_RECEIVER,
        json!({"type":"RECEIVER_STATUS","requestId":id,"status":{"volume":{"muted":true}}}),
    )
    .await;
    tokio::task::yield_now().await;
    assert!(incoming.try_recv().is_err());
    assert!(result.try_recv().is_err());
    tell(
        &mut peer,
        RECEIVER,
        NS_RECEIVER,
        json!({"type":"RECEIVER_STATUS","requestId":id,"status":{"volume":{"muted":true}}}),
    )
    .await;
    assert_eq!(result.await.unwrap(), Delivery::Acknowledged);
    assert_eq!(incoming.recv().await.unwrap().muted, Some(true));
    drop(tx);
    task.await.unwrap().unwrap();
}
#[tokio::test(start_paused = true)]
async fn receiver_refusal_is_distinct_from_a_lost_response_and_never_replayed() {
    for refused in [true, false] {
        let (stream, mut peer) = tokio::io::duplex(8192);
        let (tx, mut rx) = mpsc::channel(4);
        let (events, _media) = mpsc::channel(32);
        let (volumes, _volume_events) = mpsc::channel(8);
        let task = tokio::spawn(async move {
            tracked_receiver_stream(stream, &mut rx, &events, &volumes, Arc::default()).await
        });
        let _ = next(&mut peer).await;
        let (order, result) = TrackedOrder::new(
            ReceiverOrder::Volume(0.2),
            Instant::now() + Duration::from_secs(5),
            Arc::new(|| false),
        );
        tx.send(order).await.unwrap();
        let id = loop {
            let (_, _, body) = next(&mut peer).await;
            if body["type"] == "SET_VOLUME" {
                break body["requestId"].as_u64().unwrap();
            }
        };
        if refused {
            tell(
                &mut peer,
                RECEIVER,
                NS_RECEIVER,
                json!({"type":"INVALID_REQUEST","requestId":id}),
            )
            .await;
            assert_eq!(result.await.unwrap(), Delivery::Refused);
            drop(tx);
            task.await.unwrap().unwrap();
        } else {
            drop(peer);
            assert_eq!(result.await.unwrap(), Delivery::Uncertain);
            assert!(task.await.unwrap().is_err());
        }
    }
}

#[test]
fn receiver_generation_changes_for_media_replacement_in_the_same_application() {
    use std::sync::atomic::{AtomicU64, Ordering};
    let generation = Arc::new(AtomicU64::new(0));
    let mut follow = Follow {
        generation: Some(generation.clone()),
        ..Follow::default()
    };
    let app = json!({"status":{"applications":[{"displayName":"Example", "transportId":"app-1", "namespaces":[{"name":NS_MEDIA}]}]}});
    follow.receiver(&app);
    follow.media(&json!({"status":[{"mediaSessionId":1,"playerState":"PLAYING","media":{"metadata":{"title":"Old"}}}]}));
    let captured = generation.load(Ordering::Acquire);
    follow.media(&json!({"status":[{"mediaSessionId":1,"currentTime":2}]}));
    assert_eq!(captured, generation.load(Ordering::Acquire));
    follow.media(&json!({"status":[{"mediaSessionId":2,"playerState":"PLAYING"}]}));
    assert!(generation.load(Ordering::Acquire) > captured);
    assert!(follow.playing.title.is_none());
    let captured = generation.load(Ordering::Acquire);
    follow.receiver(&json!({"status":{"applications":[]}}));
    assert!(generation.load(Ordering::Acquire) > captured);
    assert!(follow.session.is_none());
    assert!(follow.transport.is_none());
    assert!(follow.generation.is_some());
}
