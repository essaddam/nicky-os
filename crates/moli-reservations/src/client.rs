//! The human-selected calendar endpoint. Errors and Debug never expose it.

use std::time::Duration;

#[derive(Debug, thiserror::Error)]
#[error("{}", moli_i18n::tr!("pilotes.reservations.lecture_impossible"))]
pub struct FetchError;

pub struct Client {
    endpoint: moli_net::WebUrl,
    limit: Duration,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CalendarClient").finish_non_exhaustive()
    }
}

impl Client {
    pub fn new(url: &str) -> Result<Self, FetchError> {
        if url.len() > 8192 || url.contains('#') || url.chars().any(char::is_whitespace) {
            return Err(FetchError);
        }
        let endpoint = moli_net::WebUrl::parse(url).map_err(|_| FetchError)?;
        let uri: http::Uri = url.parse().map_err(|_| FetchError)?;
        if !endpoint.tls || endpoint.port == 0 || uri.host() != Some(endpoint.host.as_str()) {
            return Err(FetchError);
        }
        Ok(Self {
            endpoint,
            limit: Duration::from_secs(10),
        })
    }

    pub async fn fetch(&self) -> Result<String, FetchError> {
        let request = moli_net::empty(
            http::Request::builder()
                .method(http::Method::GET)
                .uri(&self.endpoint.path)
                .header(http::header::ACCEPT, "text/calendar")
                .header(http::header::ACCEPT_ENCODING, "identity"),
        )
        .map_err(|_| FetchError)?;
        let (status, headers, body) = moli_net::web_with_headers(
            &self.endpoint.host,
            self.endpoint.port,
            self.endpoint.tls,
            request,
            self.limit,
            1024 * 1024,
        )
        .await
        .map_err(|_| FetchError)?;
        if status != http::StatusCode::OK {
            return Err(FetchError);
        }
        if let Some(encoding) = headers.get(http::header::CONTENT_ENCODING)
            && encoding != "identity"
        {
            return Err(FetchError);
        }
        if let Some(content_type) = headers.get(http::header::CONTENT_TYPE) {
            let mime = content_type
                .to_str()
                .map_err(|_| FetchError)?
                .split(';')
                .next()
                .ok_or(FetchError)?
                .trim();
            if !mime.eq_ignore_ascii_case("text/calendar") {
                return Err(FetchError);
            }
        }
        String::from_utf8(body.to_vec()).map_err(|_| FetchError)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn server(response: Vec<u8>) -> (Client, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut byte = [0];
                socket.read_exact(&mut byte).await.unwrap();
                request.push(byte[0]);
                if request.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            // An oversized response may be refused before it is fully sent.
            let _ = socket.write_all(&response).await;
            String::from_utf8(request).unwrap()
        });
        let client = Client {
            endpoint: moli_net::WebUrl {
                tls: false,
                host: "127.0.0.1".into(),
                port,
                path: "/calendar?token=fixture-only".into(),
            },
            limit: Duration::from_millis(500),
        };
        (client, task)
    }

    #[test]
    fn only_a_bounded_https_url_is_accepted_and_debug_is_redacted() {
        let client =
            Client::new("https://calendar.example.com/private?token=fixture-only").unwrap();
        let debug = format!("{client:?}");
        assert!(!debug.contains("fixture-only"));
        assert!(!debug.contains("calendar.example.com"));
        for url in [
            "http://calendar.example.com/private",
            "https://user:password@calendar.example.com/private",
            "https://calendar.example.com/private#fragment",
            "https://calendar.example.com/\r\nHeader: injected",
            "https://calendar.example.com:0/private",
        ] {
            let error = Client::new(url).unwrap_err();
            assert!(!format!("{error:?} {error}").contains(url));
        }
        assert!(
            Client::new(&format!(
                "https://calendar.example.com/{}",
                "a".repeat(8192)
            ))
            .is_err()
        );
    }

    #[tokio::test]
    async fn one_get_reads_the_calendar_and_never_loads_its_embedded_links() {
        let body = "BEGIN:VCALENDAR\r\nURL:https://ignored.example.com/\r\nEND:VCALENDAR\r\n";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/calendar; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let (client, server) = server(response.into_bytes()).await;
        assert_eq!(client.fetch().await.unwrap(), body);
        let request = tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
        assert!(request.starts_with("GET /calendar?token=fixture-only HTTP/1.1\r\n"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("accept-encoding: identity")
        );
    }

    #[tokio::test]
    async fn redirects_non_calendar_answers_and_invalid_utf8_are_refused() {
        for response in [
            b"HTTP/1.1 302 Found\r\nLocation: https://ignored.example.com/\r\nContent-Length: 0\r\n\r\n".to_vec(),
            b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 3\r\n\r\nbad".to_vec(),
            b"HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: 3\r\n\r\nbad".to_vec(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\n\xff".to_vec(),
        ] {
            let (client, server) = server(response).await;
            let error = client.fetch().await.unwrap_err();
            assert!(!format!("{error:?} {error}").contains("fixture-only"));
            tokio::time::timeout(Duration::from_secs(2), server).await.unwrap().unwrap();
        }
    }

    #[tokio::test]
    async fn the_complete_body_is_bounded_in_bytes_and_time() {
        let body = "x".repeat(1024 * 1024 + 1);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let (client, server) = server(response.into_bytes()).await;
        assert!(client.fetch().await.is_err());
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = Client {
            endpoint: moli_net::WebUrl {
                tls: false,
                host: "127.0.0.1".into(),
                port: listener.local_addr().unwrap().port(),
                path: "/fixture-only".into(),
            },
            limit: Duration::from_millis(25),
        };
        let hanging = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nx")
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_secs(2)).await;
        });
        assert!(
            tokio::time::timeout(Duration::from_secs(1), client.fetch())
                .await
                .unwrap()
                .is_err()
        );
        hanging.abort();
        let _ = hanging.await;
        client.limit = Duration::ZERO;
        assert!(client.fetch().await.is_err());
    }
}
