//! Bounded loopback TCP relay for real transport fault injection in the lab.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::{
    net::{Ipv4Addr, SocketAddrV4},
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    task::{Context as PollContext, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
    sync::watch,
    task::{JoinHandle, JoinSet},
};
use tokio_util::sync::CancellationToken;

const MAX_CONNECTIONS: usize = 16;
const DIRECTION_BUFFER: usize = 8 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Fault {
    Up,
    Cut,
    Blackhole,
}

impl Fault {
    fn name(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Cut => "cut",
            Self::Blackhole => "blackhole",
        }
    }
}

#[derive(Clone, Copy)]
struct Control {
    mode: Fault,
    // A coalesced watch notification must still close streams from before a cut
    // or a blackhole recovery, even when the reader only observes the final Up.
    disconnect_epoch: u64,
}

#[derive(Default)]
struct Counters {
    accepted: AtomicU64,
    active: AtomicUsize,
    forced: AtomicU64,
    forwarded: AtomicU64,
    max_active: AtomicUsize,
}

struct ActiveConnection(Arc<Counters>);

impl ActiveConnection {
    fn new(counters: Arc<Counters>) -> Self {
        let active = counters.active.fetch_add(1, Ordering::Relaxed) + 1;
        counters.max_active.fetch_max(active, Ordering::Relaxed);
        Self(counters)
    }
}

impl Drop for ActiveConnection {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::Relaxed);
    }
}

pub struct LinkFault {
    port: u16,
    control: watch::Sender<Control>,
    counters: Arc<Counters>,
    cancel: CancellationToken,
    listener: Option<JoinHandle<()>>,
}

impl LinkFault {
    pub async fn start(upstream_port: u16) -> Result<Self> {
        ensure!(
            safe_port(upstream_port),
            "reserved or invalid upstream port"
        );
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let port = listener.local_addr()?.port();
        ensure!(safe_port(port), "reserved relay port");
        let (control, changes) = watch::channel(Control {
            mode: Fault::Up,
            disconnect_epoch: 0,
        });
        let counters = Arc::new(Counters::default());
        let cancel = CancellationToken::new();
        let owned_counters = counters.clone();
        let stop = cancel.clone();
        let listener = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    biased;
                    () = stop.cancelled() => break,
                    Some(_) = connections.join_next(), if !connections.is_empty() => {},
                    accepted = listener.accept() => {
                        let Ok((downstream, peer)) = accepted else { break; };
                        owned_counters.accepted.fetch_add(1, Ordering::Relaxed);
                        let accepted_control = *changes.borrow();
                        if !peer.ip().is_loopback()
                            || accepted_control.mode == Fault::Cut
                            || connections.len() >= MAX_CONNECTIONS
                        {
                            owned_counters.forced.fetch_add(1, Ordering::Relaxed);
                            drop(downstream);
                            continue;
                        }
                        if downstream.set_nodelay(true).is_err() { continue; }
                        let active = ActiveConnection::new(owned_counters.clone());
                        let counts = owned_counters.clone();
                        let changes = changes.clone();
                        let stop = stop.clone();
                        connections.spawn(async move {
                            let _active = active;
                            if relay(downstream, upstream_port, accepted_control, changes, stop, counts.clone()).await {
                                counts.forced.fetch_add(1, Ordering::Relaxed);
                            }
                        });
                    }
                }
            }
            // Cancellation is observed by both connect and copy loops. Reap all
            // owned tasks before the listener task itself reports completion.
            stop.cancel();
            while connections.join_next().await.is_some() {}
        });
        Ok(Self {
            port,
            control,
            counters,
            cancel,
            listener: Some(listener),
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn set(&self, fault: Fault) {
        self.control.send_if_modified(|control| {
            if control.mode == fault {
                return false;
            }
            if fault == Fault::Cut || (control.mode == Fault::Blackhole && fault == Fault::Up) {
                control.disconnect_epoch = control.disconnect_epoch.wrapping_add(1);
            }
            control.mode = fault;
            true
        });
    }

    pub fn stats(&self) -> Value {
        json!({
            "mode": self.control.borrow().mode.name(),
            "accepted_connections": self.counters.accepted.load(Ordering::Relaxed),
            "active_connections": self.counters.active.load(Ordering::Relaxed),
            "forced_closures": self.counters.forced.load(Ordering::Relaxed),
            "forwarded_bytes": self.counters.forwarded.load(Ordering::Relaxed),
            "max_active_seen": self.counters.max_active.load(Ordering::Relaxed),
        })
    }

    pub async fn stop(&mut self) {
        self.cancel.cancel();
        if let Some(listener) = self.listener.take() {
            let _ = listener.await;
        }
    }
}

impl Drop for LinkFault {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(listener) = &self.listener {
            // Dropping the listener future also drops its JoinSet, aborting
            // every connection it owns and releasing the TCP streams.
            listener.abort();
        }
    }
}

fn safe_port(port: u16) -> bool {
    port != 0 && ![1414, 1415, 1416, 1883, 1884].contains(&port)
}

fn must_close(control: Control, accepted: Control) -> bool {
    control.mode == Fault::Cut || control.disconnect_epoch != accepted.disconnect_epoch
}

// Returns true only when policy or shutdown forced the stream closed.
async fn relay(
    downstream: TcpStream,
    upstream_port: u16,
    accepted: Control,
    mut changes: watch::Receiver<Control>,
    stop: CancellationToken,
    counters: Arc<Counters>,
) -> bool {
    let connect = tokio::time::timeout(
        CONNECT_TIMEOUT,
        TcpStream::connect(SocketAddrV4::new(Ipv4Addr::LOCALHOST, upstream_port)),
    );
    tokio::pin!(connect);
    let upstream = loop {
        if must_close(*changes.borrow_and_update(), accepted) {
            return true;
        }
        tokio::select! {
            biased;
            () = stop.cancelled() => return true,
            changed = changes.changed() => {
                if changed.is_err() { return true; }
            }
            connected = &mut connect => {
                match connected {
                    Ok(Ok(upstream)) => break upstream,
                    _ => return false,
                }
            }
        }
    };
    if upstream.set_nodelay(true).is_err() {
        return false;
    }
    let mut downstream = CountedStream {
        stream: downstream,
        counters: counters.clone(),
    };
    let mut upstream = CountedStream {
        stream: upstream,
        counters,
    };
    loop {
        let current = *changes.borrow_and_update();
        if must_close(current, accepted) {
            return true;
        }
        if current.mode == Fault::Blackhole {
            // Hold the sockets without reading or writing either side. There
            // are no application queues, so backpressure bounds kernel data.
            tokio::select! {
                biased;
                () = stop.cancelled() => return true,
                changed = changes.changed() => {
                    if changed.is_err() { return true; }
                }
            }
        } else {
            tokio::select! {
                biased;
                () = stop.cancelled() => return true,
                changed = changes.changed() => {
                    if changed.is_err() { return true; }
                }
                _ = tokio::io::copy_bidirectional_with_sizes(
                    &mut downstream, &mut upstream, DIRECTION_BUFFER, DIRECTION_BUFFER,
                ) => return false,
            }
            // Cancelling the copy drops its two bounded buffers. A subsequent
            // Blackhole -> Up always closes these sockets, never replaying a
            // partially read frame or bytes still waiting in a socket buffer.
        }
    }
}

struct CountedStream {
    stream: TcpStream,
    counters: Arc<Counters>,
}

impl AsyncRead for CountedStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut PollContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for CountedStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut PollContext<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match Pin::new(&mut self.stream).poll_write(cx, buf) {
            Poll::Ready(Ok(written)) => {
                self.counters
                    .forwarded
                    .fetch_add(written as u64, Ordering::Relaxed);
                Poll::Ready(Ok(written))
            }
            result => result,
        }
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut PollContext<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }
    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut PollContext<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    struct Echo {
        port: u16,
        task: JoinHandle<()>,
    }
    impl Echo {
        async fn start() -> Result<Self> {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
            let port = listener.local_addr()?.port();
            let task = tokio::spawn(async move {
                let mut peers = JoinSet::new();
                loop {
                    tokio::select! {
                        Some(_) = peers.join_next(), if !peers.is_empty() => {},
                        peer = listener.accept() => {
                            let Ok((mut peer, _)) = peer else { break; };
                            peers.spawn(async move {
                                let (mut read, mut write) = peer.split();
                                let _ = tokio::io::copy(&mut read, &mut write).await;
                            });
                        }
                    }
                }
            });
            Ok(Self { port, task })
        }
    }
    impl Drop for Echo {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    async fn roundtrip(stream: &mut TcpStream, bytes: &[u8]) {
        stream.write_all(bytes).await.unwrap();
        let mut echoed = vec![0; bytes.len()];
        tokio::time::timeout(Duration::from_secs(1), stream.read_exact(&mut echoed))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(echoed, bytes);
    }
    async fn closed(stream: &mut TcpStream) {
        let mut byte = [0];
        let result = tokio::time::timeout(Duration::from_secs(1), stream.read(&mut byte))
            .await
            .expect("relay did not close TCP stream");
        assert!(
            matches!(result, Ok(0) | Err(_)),
            "unexpected replayed bytes"
        );
    }
    async fn wait_active(relay: &LinkFault, expected: u64) {
        tokio::time::timeout(Duration::from_secs(1), async {
            while relay.stats()["active_connections"] != expected {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn forwards_both_directions_and_reaps_on_stop() {
        let echo = Echo::start().await.unwrap();
        let mut relay = LinkFault::start(echo.port).await.unwrap();
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, relay.port()))
            .await
            .unwrap();
        roundtrip(&mut stream, b"hello").await;
        let stats = relay.stats();
        assert_eq!(stats["mode"], "up");
        assert_eq!(stats["accepted_connections"], 1);
        assert_eq!(stats["active_connections"], 1);
        assert_eq!(stats["forwarded_bytes"], 10);
        assert_eq!(stats["max_active_seen"], 1);
        relay.stop().await;
        closed(&mut stream).await;
        assert_eq!(relay.stats()["active_connections"], 0);
        relay.stop().await;
    }

    #[tokio::test]
    async fn cut_closes_current_and_new_connections_then_recovers() {
        let echo = Echo::start().await.unwrap();
        let mut relay = LinkFault::start(echo.port).await.unwrap();
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, relay.port()))
            .await
            .unwrap();
        roundtrip(&mut stream, b"before").await;
        relay.set(Fault::Cut);
        closed(&mut stream).await;
        wait_active(&relay, 0).await;
        let forwarded = relay.stats()["forwarded_bytes"].clone();
        let mut refused = TcpStream::connect((Ipv4Addr::LOCALHOST, relay.port()))
            .await
            .unwrap();
        closed(&mut refused).await;
        assert_eq!(relay.stats()["forwarded_bytes"], forwarded);
        assert!(relay.stats()["forced_closures"].as_u64().unwrap() >= 2);
        relay.set(Fault::Up);
        let mut fresh = TcpStream::connect((Ipv4Addr::LOCALHOST, relay.port()))
            .await
            .unwrap();
        roundtrip(&mut fresh, b"after").await;
        relay.stop().await;
    }

    #[tokio::test]
    async fn blackhole_keeps_streams_bounded_and_recovery_never_replays_pending_bytes() {
        let echo = Echo::start().await.unwrap();
        let mut relay = LinkFault::start(echo.port).await.unwrap();
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, relay.port()))
            .await
            .unwrap();
        roundtrip(&mut stream, b"before").await;
        relay.set(Fault::Blackhole);
        tokio::time::sleep(Duration::from_millis(20)).await;
        let before = relay.stats();
        stream.write_all(b"must-not-replay").await.unwrap();
        let mut byte = [0];
        assert!(
            tokio::time::timeout(Duration::from_millis(100), stream.read(&mut byte))
                .await
                .is_err()
        );
        let during = relay.stats();
        assert_eq!(during["mode"], "blackhole");
        assert_eq!(during["forwarded_bytes"], before["forwarded_bytes"]);
        assert_eq!(
            during["accepted_connections"],
            before["accepted_connections"]
        );
        assert_eq!(during["active_connections"], 1);
        relay.set(Fault::Up);
        closed(&mut stream).await;
        wait_active(&relay, 0).await;
        assert_eq!(relay.stats()["forwarded_bytes"], before["forwarded_bytes"]);
        let mut fresh = TcpStream::connect((Ipv4Addr::LOCALHOST, relay.port()))
            .await
            .unwrap();
        roundtrip(&mut fresh, b"fresh-only").await;
        relay.stop().await;
    }

    #[tokio::test]
    async fn coalesced_fault_notifications_still_close_old_streams() {
        let echo = Echo::start().await.unwrap();
        let mut relay = LinkFault::start(echo.port).await.unwrap();
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, relay.port()))
            .await
            .unwrap();
        roundtrip(&mut stream, b"ready").await;
        relay.set(Fault::Blackhole);
        relay.set(Fault::Up);
        closed(&mut stream).await;
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, relay.port()))
            .await
            .unwrap();
        roundtrip(&mut stream, b"ready-again").await;
        relay.set(Fault::Cut);
        relay.set(Fault::Up);
        closed(&mut stream).await;
        relay.stop().await;
    }

    #[tokio::test]
    async fn blackhole_also_blocks_unsolicited_upstream_data() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (connected_tx, connected_rx) = tokio::sync::oneshot::channel();
        let (emit_tx, emit_rx) = tokio::sync::oneshot::channel();
        let (written_tx, written_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            stream.set_nodelay(true).unwrap();
            let _ = connected_tx.send(());
            if emit_rx.await.is_ok() {
                stream.write_all(b"upstream-must-not-replay").await.unwrap();
                let _ = written_tx.send(());
                std::future::pending::<()>().await;
            }
        });
        let upstream = Echo { port, task };
        let mut relay = LinkFault::start(upstream.port).await.unwrap();
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, relay.port()))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), connected_rx)
            .await
            .unwrap()
            .unwrap();
        relay.set(Fault::Blackhole);
        tokio::time::sleep(Duration::from_millis(20)).await;
        emit_tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(1), written_rx)
            .await
            .unwrap()
            .unwrap();
        let mut byte = [0];
        assert!(
            tokio::time::timeout(Duration::from_millis(100), stream.read(&mut byte))
                .await
                .is_err()
        );
        assert_eq!(relay.stats()["forwarded_bytes"], 0);
        assert_eq!(relay.stats()["active_connections"], 1);
        relay.set(Fault::Up);
        closed(&mut stream).await;
        assert_eq!(relay.stats()["forwarded_bytes"], 0);
        relay.stop().await;
    }

    #[tokio::test]
    async fn blackhole_connection_count_is_capped_at_sixteen() {
        let echo = Echo::start().await.unwrap();
        let mut relay = LinkFault::start(echo.port).await.unwrap();
        relay.set(Fault::Blackhole);
        let mut streams = Vec::new();
        for _ in 0..MAX_CONNECTIONS {
            streams.push(
                TcpStream::connect((Ipv4Addr::LOCALHOST, relay.port()))
                    .await
                    .unwrap(),
            );
        }
        wait_active(&relay, MAX_CONNECTIONS as u64).await;
        let mut refused = TcpStream::connect((Ipv4Addr::LOCALHOST, relay.port()))
            .await
            .unwrap();
        closed(&mut refused).await;
        assert_eq!(relay.stats()["max_active_seen"], MAX_CONNECTIONS as u64);
        assert_eq!(relay.stats()["forwarded_bytes"], 0);
        relay.stop().await;
        assert_eq!(relay.stats()["active_connections"], 0);
    }

    #[tokio::test]
    async fn reserved_upstream_ports_are_never_contacted() {
        for port in [0, 1414, 1415, 1416, 1883, 1884] {
            assert!(LinkFault::start(port).await.is_err());
        }
    }
}
