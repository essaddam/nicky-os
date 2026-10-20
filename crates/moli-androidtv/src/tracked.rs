//! Deadline and abandonment follow an order to the final write readiness poll.

use std::fmt;
use std::future::poll_fn;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;

use tokio::io::AsyncWrite;
use tokio::sync::oneshot;
use tokio::time::Instant;

/// Own the reader task even when its parent session is cancelled or panics.
#[derive(Debug)]
pub(crate) struct ReaderTask(tokio::task::JoinHandle<()>);

impl ReaderTask {
    pub(crate) fn new(task: tokio::task::JoinHandle<()>) -> Self {
        Self(task)
    }

    pub(crate) async fn close(mut self) {
        self.0.abort();
        let _ = (&mut self.0).await;
    }
}

impl Drop for ReaderTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Transport evidence, never an assertion about which programme is playing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    Sent,
    Acknowledged,
    Refused,
    Unavailable,
    Expired,
    Abandoned,
    Uncertain,
}

pub struct TrackedOrder<T> {
    pub payload: T,
    pub deadline: Instant,
    abandoned: Arc<dyn Fn() -> bool + Send + Sync>,
    result: oneshot::Sender<Delivery>,
}

impl<T> fmt::Debug for TrackedOrder<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TrackedOrder")
            .field("deadline", &self.deadline)
            .finish_non_exhaustive()
    }
}

impl<T> TrackedOrder<T> {
    #[must_use]
    pub fn new(
        payload: T,
        deadline: Instant,
        abandoned: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> (Self, oneshot::Receiver<Delivery>) {
        let (result, receiver) = oneshot::channel();
        (
            Self {
                payload,
                deadline,
                abandoned,
                result,
            },
            receiver,
        )
    }

    pub(crate) fn complete(self, delivery: Delivery) {
        let _ = self.result.send(delivery);
    }

    pub(crate) fn unsent_reason(&self) -> Option<Delivery> {
        if Instant::now() >= self.deadline {
            Some(Delivery::Expired)
        } else if self.result.is_closed() || (self.abandoned)() {
            Some(Delivery::Abandoned)
        } else {
            None
        }
    }
}

pub(crate) async fn write_packet<W: AsyncWrite + Unpin, T>(
    writer: &mut W,
    packet: &[u8],
    order: &TrackedOrder<T>,
) -> Delivery {
    let mut sent = 0;
    let result = tokio::time::timeout_at(order.deadline, async {
        while sent < packet.len() {
            let count = poll_fn(|cx| {
                // This runs on EVERY readiness poll, including the wake-up
                // after Pending. Checking only before write().await is too early.
                if sent == 0 {
                    if let Some(reason) = order.unsent_reason() {
                        return Poll::Ready(Err(reason));
                    }
                } else if Instant::now() >= order.deadline {
                    return Poll::Ready(Err(Delivery::Uncertain));
                }
                match Pin::new(&mut *writer).poll_write(cx, &packet[sent..]) {
                    Poll::Ready(Ok(0) | Err(_)) => Poll::Ready(Err(Delivery::Uncertain)),
                    Poll::Ready(Ok(count)) => Poll::Ready(Ok(count)),
                    Poll::Pending => Poll::Pending,
                }
            })
            .await?;
            sent += count;
        }
        if Instant::now() >= order.deadline {
            return Err(Delivery::Uncertain);
        }
        poll_fn(|cx| Pin::new(&mut *writer).poll_flush(cx))
            .await
            .map_err(|_| Delivery::Uncertain)?;
        Ok(Delivery::Sent)
    })
    .await;
    match result {
        Ok(Ok(delivery)) => delivery,
        Ok(Err(reason)) => reason,
        Err(_) if sent == 0 => Delivery::Expired,
        Err(_) => Delivery::Uncertain,
    }
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::{Context, Poll, Waker};
    use std::time::Duration;

    use tokio::io::{AsyncReadExt as _, AsyncWrite};
    use tokio::sync::Notify;

    use super::*;

    #[derive(Debug, Default)]
    struct Gate {
        ready: AtomicBool,
        entered: Notify,
        waker: std::sync::Mutex<Option<Waker>>,
        bytes: std::sync::Mutex<Vec<u8>>,
    }

    #[derive(Debug)]
    struct GatedWriter(Arc<Gate>);

    impl AsyncWrite for GatedWriter {
        fn poll_write(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            if !self.0.ready.load(Ordering::Acquire) {
                *self.0.waker.lock().unwrap() = Some(cx.waker().clone());
                self.0.entered.notify_one();
                return Poll::Pending;
            }
            self.0.bytes.lock().unwrap().extend_from_slice(bytes);
            Poll::Ready(Ok(bytes.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test(start_paused = true)]
    async fn expired_order_never_reaches_the_writer() {
        let gate = Arc::new(Gate::default());
        gate.ready.store(true, Ordering::Release);
        let mut writer = GatedWriter(Arc::clone(&gate));
        let (order, _result) = TrackedOrder::new((), Instant::now(), Arc::new(|| false));
        assert_eq!(
            write_packet(&mut writer, b"packet", &order).await,
            Delivery::Expired
        );
        assert!(gate.bytes.lock().unwrap().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn abandonment_is_rechecked_after_write_readiness_wait() {
        let gate = Arc::new(Gate::default());
        let abandoned = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&abandoned);
        let (order, _result) = TrackedOrder::new(
            (),
            Instant::now() + Duration::from_secs(5),
            Arc::new(move || flag.load(Ordering::Acquire)),
        );
        let mut writer = GatedWriter(Arc::clone(&gate));
        let mut task =
            tokio::spawn(async move { write_packet(&mut writer, b"packet", &order).await });
        tokio::select! {
            biased;
            () = gate.entered.notified() => {},
            _ = &mut task => panic!("writer was never polled"),
            () = tokio::time::sleep(Duration::from_secs(1)) => panic!("writer readiness was never reached"),
        }
        abandoned.store(true, Ordering::Release);
        gate.ready.store(true, Ordering::Release);
        gate.waker.lock().unwrap().take().unwrap().wake();
        assert_eq!(task.await.unwrap(), Delivery::Abandoned);
        assert!(gate.bytes.lock().unwrap().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn dropped_result_before_emission_is_abandonment() {
        let gate = Arc::new(Gate::default());
        gate.ready.store(true, Ordering::Release);
        let mut writer = GatedWriter(Arc::clone(&gate));
        let (order, result) = TrackedOrder::new(
            (),
            Instant::now() + Duration::from_secs(5),
            Arc::new(|| false),
        );
        drop(result);
        assert_eq!(
            write_packet(&mut writer, b"packet", &order).await,
            Delivery::Abandoned
        );
        assert!(gate.bytes.lock().unwrap().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn abandonment_after_first_byte_keeps_tracking_the_original_write() {
        let (mut writer, mut peer) = tokio::io::duplex(1);
        let abandoned = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&abandoned);
        let (order, _result) = TrackedOrder::new(
            (),
            Instant::now() + Duration::from_secs(5),
            Arc::new(move || flag.load(Ordering::Acquire)),
        );
        let task = tokio::spawn(async move { write_packet(&mut writer, b"AB", &order).await });
        assert_eq!(peer.read_u8().await.unwrap(), b'A');
        abandoned.store(true, Ordering::Release);
        assert_eq!(peer.read_u8().await.unwrap(), b'B');
        assert_eq!(task.await.unwrap(), Delivery::Sent);
    }

    #[tokio::test(start_paused = true)]
    async fn partially_written_frame_expiring_is_uncertain_and_never_replayed() {
        let (mut writer, mut peer) = tokio::io::duplex(1);
        let (order, _result) = TrackedOrder::new(
            (),
            Instant::now() + Duration::from_secs(5),
            Arc::new(|| false),
        );
        let task = tokio::spawn(async move { write_packet(&mut writer, b"AB", &order).await });
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(6)).await;
        assert_eq!(task.await.unwrap(), Delivery::Uncertain);
        assert_eq!(peer.read_u8().await.unwrap(), b'A');
        assert!(peer.read_u8().await.is_err());
    }
}
