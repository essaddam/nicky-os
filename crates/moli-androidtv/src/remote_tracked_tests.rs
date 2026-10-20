use std::sync::Arc;

use tokio::time::{Instant, timeout};

use super::*;
use crate::tracked::{Delivery, TrackedOrder};

async fn ready(peer: &mut tokio::io::DuplexStream, events: &mut mpsc::Receiver<Event>) {
    send(
        peer,
        &Writer::new().msg(1, Writer::new().uint(1, 639)).finish(),
    )
    .await
    .unwrap();
    let reply = read_frame(peer).await;
    assert!(reply.is_ok(), "remote must answer configuration");
    assert!(parse(&reply.unwrap()).unwrap().has(1));
    send(
        peer,
        &Writer::new().msg(2, Writer::new().uint(1, 639)).finish(),
    )
    .await
    .unwrap();
    assert!(parse(&read_frame(peer).await.unwrap()).unwrap().has(2));
    assert_eq!(events.recv().await, Some(Event::Ready));
}

#[tokio::test(start_paused = true)]
async fn tracked_remote_preserves_fifo_and_reports_real_writes() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let (tx, mut rx) = mpsc::channel(4);
    let (events, mut incoming) = mpsc::channel(8);
    let task = tokio::spawn(async move { tracked_stream(stream, &mut rx, &events).await });
    ready(&mut peer, &mut incoming).await;
    for key in [19, 20, 66] {
        let (order, result) = TrackedOrder::new(
            Order::Key(key),
            Instant::now() + Duration::from_secs(5),
            Arc::new(|| false),
        );
        tx.send(order).await.unwrap();
        let frame = read_frame(&mut peer).await.unwrap();
        assert_eq!(
            parse(&frame).unwrap().msg(10).unwrap().uint(1),
            Some(u64::from(key))
        );
        assert_eq!(result.await.unwrap(), Delivery::Sent);
    }
    drop(tx);
    assert!(
        timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .is_ok()
    );
    assert!(
        timeout(Duration::from_secs(1), read_frame(&mut peer))
            .await
            .unwrap()
            .is_err()
    );
}

#[tokio::test(start_paused = true)]
async fn expired_remote_order_is_not_sent_before_the_next_live_order() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let (tx, mut rx) = mpsc::channel(4);
    let (events, mut incoming) = mpsc::channel(8);
    let task = tokio::spawn(async move { tracked_stream(stream, &mut rx, &events).await });
    ready(&mut peer, &mut incoming).await;
    let (expired, result) = TrackedOrder::new(Order::Key(19), Instant::now(), Arc::new(|| false));
    tx.send(expired).await.unwrap();
    assert_eq!(result.await.unwrap(), Delivery::Expired);
    let (live, result) = TrackedOrder::new(
        Order::Key(66),
        Instant::now() + Duration::from_secs(5),
        Arc::new(|| false),
    );
    tx.send(live).await.unwrap();
    assert_eq!(
        parse(&read_frame(&mut peer).await.unwrap())
            .unwrap()
            .msg(10)
            .unwrap()
            .uint(1),
        Some(66)
    );
    assert_eq!(result.await.unwrap(), Delivery::Sent);
    drop(tx);
    task.await.unwrap().unwrap();
}

#[tokio::test(start_paused = true)]
async fn remote_loss_resolves_queued_orders_and_does_not_keep_a_reader_alive() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let (tx, mut rx) = mpsc::channel(4);
    let (events, mut incoming) = mpsc::channel(8);
    let task = tokio::spawn(async move { tracked_stream(stream, &mut rx, &events).await });
    ready(&mut peer, &mut incoming).await;
    drop(peer);
    let (order, result) = TrackedOrder::new(
        Order::Key(66),
        Instant::now() + Duration::from_secs(5),
        Arc::new(|| false),
    );
    let _ = tx.send(order).await;
    assert!(
        timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    assert!(matches!(
        result.await,
        Ok(Delivery::Unavailable | Delivery::Uncertain) | Err(_)
    ));
}

#[tokio::test(start_paused = true)]
async fn cancelling_remote_session_closes_its_spawned_reader() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let (_tx, mut rx) = mpsc::channel(4);
    let (events, mut incoming) = mpsc::channel(8);
    let task = tokio::spawn(async move { tracked_stream(stream, &mut rx, &events).await });
    ready(&mut peer, &mut incoming).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(
        timeout(Duration::from_secs(1), read_frame(&mut peer))
            .await
            .is_ok_and(|result| result.is_err()),
        "cancelling the session must not detach a reader holding the connection open"
    );
}
