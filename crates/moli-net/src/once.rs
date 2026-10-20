//! A single bounded request with an explicit before/after-send boundary.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use http::{HeaderMap, Request, StatusCode};
use http_body_util::Full;
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::task::JoinHandle;
use tokio_rustls::TlsConnector;

/// Safe transport stages. No URL, request body or downstream error text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnceError {
    BeforeSend,
    AfterSend,
}

/// One request; never retries. An after-send failure may have had an effect.
///
/// The boundary is handing the request to HTTP, conservatively before any
/// confirmation of bytes sent. `limit` covers connection, TLS and the entire
/// response body. Response headers are bounded to 64 fields and a 16 KiB buffer.
/// Redirects are returned unchanged. Cancellation closes the connection task.
///
/// # Errors
/// Returns only the transport stage, without downstream error text.
pub async fn web_once(
    host: &str,
    port: u16,
    tls: bool,
    request: Request<Full<Bytes>>,
    limit: Duration,
    max_body: usize,
) -> Result<(StatusCode, HeaderMap, Bytes), OnceError> {
    web_once_guarded(host, port, tls, request, limit, max_body, || true).await
}

/// Like [`web_once`], with a synchronous permission check after connection
/// and HTTP handshake, immediately before handing the request to HTTP.
/// No await separates permission from handoff. Once handed off, permission
/// changes do not cancel acquisition of a potentially rotating response.
///
/// # Errors
/// A refused permission is `BeforeSend`; other errors follow [`web_once`].
pub async fn web_once_guarded(
    host: &str,
    port: u16,
    tls: bool,
    mut request: Request<Full<Bytes>>,
    limit: Duration,
    max_body: usize,
    allowed: impl Fn() -> bool,
) -> Result<(StatusCode, HeaderMap, Bytes), OnceError> {
    if host.is_empty() || host.len() > 253 || !host.is_ascii() || port == 0 {
        return Err(OnceError::BeforeSend);
    }
    if !request.headers().contains_key(http::header::HOST) {
        let authority = if host.contains(':') {
            format!("[{host}]")
        } else {
            host.to_owned()
        };
        let authority = if (tls && port == 443) || (!tls && port == 80) {
            authority
        } else {
            format!("{authority}:{port}")
        };
        request.headers_mut().insert(
            http::header::HOST,
            authority.parse().map_err(|_| OnceError::BeforeSend)?,
        );
    }
    let sent = AtomicBool::new(false);
    let exchange = async {
        let tcp = async {
            TcpStream::connect((host, port))
                .await
                .map_err(|_| OnceError::BeforeSend)
        };
        if tls {
            let stream = async {
                let tcp = tcp.await?;
                let name =
                    ServerName::try_from(host.to_owned()).map_err(|_| OnceError::BeforeSend)?;
                TlsConnector::from(super::public_tls())
                    .connect(name, tcp)
                    .await
                    .map_err(|_| OnceError::BeforeSend)
            };
            exchange_once(stream, request, max_body, &sent, &allowed).await
        } else {
            exchange_once(tcp, request, max_body, &sent, &allowed).await
        }
    };
    tokio::time::timeout(limit, exchange)
        .await
        .unwrap_or_else(|_| {
            Err(if sent.load(Ordering::Relaxed) {
                OnceError::AfterSend
            } else {
                OnceError::BeforeSend
            })
        })
}

struct AbortOnDrop(JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn exchange_once<S>(
    connect: impl std::future::Future<Output = Result<S, OnceError>>,
    request: Request<Full<Bytes>>,
    max_body: usize,
    sent: &AtomicBool,
    allowed: &impl Fn() -> bool,
) -> Result<(StatusCode, HeaderMap, Bytes), OnceError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let stream = connect.await?;
    let (mut sender, connection) = hyper::client::conn::http1::Builder::new()
        .max_headers(64)
        .max_buf_size(16 * 1024)
        .handshake(TokioIo::new(stream))
        .await
        .map_err(|_| OnceError::BeforeSend)?;
    let _connection = AbortOnDrop(tokio::spawn(async move {
        let _ = connection.await;
    }));
    if !allowed() {
        return Err(OnceError::BeforeSend);
    }
    sent.store(true, Ordering::Relaxed);
    let response = sender
        .send_request(request)
        .await
        .map_err(|_| OnceError::AfterSend)?;
    let (head, body) = response.into_parts();
    let body = super::read_body(body, max_body)
        .await
        .map_err(|_| OnceError::AfterSend)?;
    Ok((head.status, head.headers, body))
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    use super::*;

    fn request() -> Request<Full<Bytes>> {
        Request::builder()
            .method("POST")
            .uri("/oauth2/token")
            .body(Full::new(Bytes::new()))
            .unwrap()
    }

    #[tokio::test(start_paused = true)]
    async fn permission_is_checked_after_a_pending_connection_before_any_post() {
        for mode in 0..3 {
            let (client, mut server) = tokio::io::duplex(4096);
            let entered = std::sync::Arc::new(tokio::sync::Notify::new());
            let release = std::sync::Arc::new(tokio::sync::Notify::new());
            let live = std::sync::Arc::new(AtomicBool::new(true));
            let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
            let reader = tokio::spawn(async move {
                let mut head = Vec::new();
                loop {
                    let mut bytes = [0; 256];
                    let size = server.read(&mut bytes).await.unwrap();
                    if size == 0 {
                        return head;
                    }
                    head.extend_from_slice(&bytes[..size]);
                    assert!(head.len() < 4096);
                    if head.windows(4).any(|w| w == b"\r\n\r\n") {
                        server
                            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
                            .await
                            .unwrap();
                        return head;
                    }
                }
            });
            let connection_entered = entered.clone();
            let connection_release = release.clone();
            let permitted = live.clone();
            let exchange = tokio::spawn(async move {
                let connect = async {
                    connection_entered.notify_one();
                    connection_release.notified().await;
                    Ok(client)
                };
                let sent = AtomicBool::new(false);
                let allowed =
                    || permitted.load(Ordering::SeqCst) && tokio::time::Instant::now() < deadline;
                let result = exchange_once(connect, request(), 100, &sent, &allowed).await;
                (result, sent.load(Ordering::Relaxed))
            });
            entered.notified().await;
            match mode {
                0 => live.store(false, Ordering::SeqCst),
                1 => tokio::time::advance(Duration::from_secs(20)).await,
                _ => {}
            }
            release.notify_one();
            let (result, sent) = exchange.await.unwrap();
            let head = reader.await.unwrap();
            if mode == 2 {
                assert_eq!(result.unwrap().0, StatusCode::OK);
                assert!(sent);
                assert!(head.starts_with(b"POST /oauth2/token HTTP/1.1\r\n"));
                assert_eq!(head.windows(5).filter(|w| *w == b"POST ").count(), 1);
            } else {
                assert_eq!(result, Err(OnceError::BeforeSend));
                assert!(!sent);
                assert!(head.is_empty());
            }
        }
    }

    async fn read_head(stream: &mut TcpStream) -> Vec<u8> {
        let mut head = Vec::new();
        while !head.windows(4).any(|window| window == b"\r\n\r\n") {
            assert!(head.len() <= 8192);
            let mut bytes = [0; 256];
            let size = stream.read(&mut bytes).await.unwrap();
            assert!(size > 0, "request ended before its headers");
            head.extend_from_slice(&bytes[..size]);
        }
        head
    }

    #[tokio::test]
    async fn connection_failure_is_before_send() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        assert_eq!(
            web_once(
                "127.0.0.1",
                port,
                false,
                request(),
                Duration::from_secs(1),
                100
            )
            .await,
            Err(OnceError::BeforeSend)
        );
    }

    #[tokio::test]
    async fn tls_failure_is_before_send_and_never_sends_http() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = [0; 4096];
            let size = stream.read(&mut bytes).await.unwrap();
            assert!(size > 0);
            assert!(!bytes[..size].starts_with(b"POST "));
            stream.write_all(b"not a TLS server").await.unwrap();
        });
        assert_eq!(
            web_once(
                "127.0.0.1",
                port,
                true,
                request(),
                Duration::from_secs(1),
                100
            )
            .await,
            Err(OnceError::BeforeSend)
        );
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn disconnect_after_request_is_ambiguous() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let head = read_head(&mut stream).await;
            assert!(head.starts_with(b"POST /oauth2/token HTTP/1.1\r\n"));
        });
        assert_eq!(
            web_once(
                "127.0.0.1",
                port,
                false,
                request(),
                Duration::from_secs(1),
                100
            )
            .await,
            Err(OnceError::AfterSend)
        );
        task.await.unwrap();
    }

    #[tokio::test]
    async fn bounded_body_and_redirect_are_not_retried() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            read_head(&mut stream).await;
            stream.write_all(b"HTTP/1.1 302 Found\r\nLocation: https://example.com/\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}").await.unwrap();
        });
        let result = web_once(
            "127.0.0.1",
            port,
            false,
            request(),
            Duration::from_secs(1),
            100,
        )
        .await
        .unwrap();
        assert_eq!(result.0, StatusCode::FOUND);
        assert_eq!(result.2.as_ref(), b"{}");
        task.await.unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            read_head(&mut stream).await;
            stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n8\r\n12345678\r\n0\r\n\r\n").await.unwrap();
        });
        assert_eq!(
            web_once(
                "127.0.0.1",
                port,
                false,
                request(),
                Duration::from_secs(1),
                4
            )
            .await,
            Err(OnceError::AfterSend)
        );
        task.await.unwrap();
    }
}
