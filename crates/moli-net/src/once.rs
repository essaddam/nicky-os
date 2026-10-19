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
    mut request: Request<Full<Bytes>>,
    limit: Duration,
    max_body: usize,
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
        let tcp = TcpStream::connect((host, port))
            .await
            .map_err(|_| OnceError::BeforeSend)?;
        if tls {
            let name = ServerName::try_from(host.to_owned()).map_err(|_| OnceError::BeforeSend)?;
            let stream = TlsConnector::from(super::public_tls())
                .connect(name, tcp)
                .await
                .map_err(|_| OnceError::BeforeSend)?;
            exchange_once(stream, request, max_body, &sent).await
        } else {
            exchange_once(tcp, request, max_body, &sent).await
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
    stream: S,
    request: Request<Full<Bytes>>,
    max_body: usize,
    sent: &AtomicBool,
) -> Result<(StatusCode, HeaderMap, Bytes), OnceError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut sender, connection) = hyper::client::conn::http1::Builder::new()
        .max_headers(64)
        .max_buf_size(16 * 1024)
        .handshake(TokioIo::new(stream))
        .await
        .map_err(|_| OnceError::BeforeSend)?;
    let _connection = AbortOnDrop(tokio::spawn(async move {
        let _ = connection.await;
    }));
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
