//! The integration fixture's response reader; not a production transport.
use std::io;
use tokio::io::{AsyncRead, AsyncReadExt};

pub async fn read_response(stream: &mut (impl AsyncRead + Unpin)) -> io::Result<Vec<u8>> {
    tokio::time::timeout(std::time::Duration::from_secs(5), receive(stream))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "HTTP test response timed out"))?
}

const MAX_HEADERS: usize = 16 * 1024;
const MAX_BODY: usize = 4 * 1024 * 1024;

enum Frame {
    Headers,
    Length(usize),
    Close(usize),
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn framing(header: &[u8]) -> io::Result<Frame> {
    let text = std::str::from_utf8(header).map_err(|_| invalid("invalid HTTP headers"))?;
    let mut lines = text.split("\r\n");
    let status = lines.next().unwrap_or_default();
    if !status.starts_with("HTTP/1.")
        || status
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse::<u16>().ok())
            .is_none()
    {
        return Err(invalid("invalid HTTP status"));
    }
    // Same strict delimitation as the Android TV test fix: one Content-Length,
    // no conflicting Transfer-Encoding, and exactly the advertised body.
    let mut length = None;
    let mut transfer = false;
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| invalid("invalid HTTP header"))?;
        if name.eq_ignore_ascii_case("content-length") {
            if length.is_some() {
                return Err(invalid("duplicate Content-Length"));
            }
            let value = value.trim();
            if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                return Err(invalid("invalid Content-Length"));
            }
            let size = value
                .parse::<usize>()
                .map_err(|_| invalid("invalid Content-Length"))?;
            if size > MAX_BODY {
                return Err(invalid("HTTP response body too large"));
            }
            length = Some(size);
        }
        if name.eq_ignore_ascii_case("transfer-encoding") {
            if transfer || !value.trim().eq_ignore_ascii_case("chunked") {
                return Err(invalid("unsupported Transfer-Encoding"));
            }
            transfer = true;
        }
    }
    if transfer && length.is_some() {
        return Err(invalid("ambiguous HTTP framing"));
    }
    Ok(length.map_or(Frame::Close(header.len()), |size| {
        Frame::Length(header.len() + size)
    }))
}

async fn receive(stream: &mut (impl AsyncRead + Unpin)) -> io::Result<Vec<u8>> {
    let mut raw = Vec::new();
    let mut frame = Frame::Headers;
    let mut buffer = [0; 8192];
    loop {
        let count = stream.read(&mut buffer).await?;
        if count == 0 {
            return if matches!(frame, Frame::Close(_)) {
                Ok(raw)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "incomplete HTTP response",
                ))
            };
        }
        raw.extend_from_slice(&buffer[..count]);
        if matches!(frame, Frame::Headers) {
            if let Some(end) = raw.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                if end + 4 > MAX_HEADERS {
                    return Err(invalid("HTTP headers too large"));
                }
                frame = framing(&raw[..end + 4])?;
            } else if raw.len() > MAX_HEADERS {
                return Err(invalid("HTTP headers too large"));
            }
        }
        match frame {
            Frame::Length(total) if raw.len() > total => {
                return Err(invalid("extra HTTP response bytes"));
            }
            Frame::Length(total) if raw.len() == total => return Ok(raw),
            Frame::Close(start) if raw.len() - start > MAX_BODY => {
                return Err(invalid("HTTP response body too large"));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        pin::Pin,
        task::{Context, Poll},
    };
    use tokio::io::ReadBuf;

    const COMPLETE: &[u8] = b"HTTP/1.1 403 Forbidden\r\nContent-Length: 2\r\n\r\n{}";
    enum Tail {
        Reset,
        Eof,
        Pending,
    }
    struct Script<'a> {
        bytes: &'a [u8],
        fragment: usize,
        tail: Tail,
    }
    impl AsyncRead for Script<'_> {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _: &mut Context<'_>,
            buffer: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            if self.bytes.is_empty() {
                return match self.tail {
                    Tail::Reset => Poll::Ready(Err(io::ErrorKind::ConnectionReset.into())),
                    Tail::Eof => Poll::Ready(Ok(())),
                    Tail::Pending => Poll::Pending,
                };
            }
            let n = self.bytes.len().min(buffer.remaining()).min(self.fragment);
            buffer.put_slice(&self.bytes[..n]);
            self.bytes = &self.bytes[n..];
            Poll::Ready(Ok(()))
        }
    }
    fn input(bytes: &[u8], tail: Tail) -> Script<'_> {
        Script {
            bytes,
            fragment: 8192,
            tail,
        }
    }

    #[tokio::test]
    async fn a_complete_length_delimited_reply_does_not_observe_a_later_reset() {
        assert_eq!(
            read_response(&mut input(COMPLETE, Tail::Reset))
                .await
                .unwrap(),
            COMPLETE
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_complete_reply_returns_while_the_peer_stays_open() {
        let mut response = input(COMPLETE, Tail::Pending);
        let reply = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            read_response(&mut response),
        )
        .await;
        assert_eq!(reply.unwrap().unwrap(), COMPLETE);
    }

    #[tokio::test]
    async fn a_reset_or_eof_never_hides_a_truncated_reply() {
        for bytes in [&COMPLETE[..10], &COMPLETE[..COMPLETE.len() - 1]] {
            for tail in [Tail::Reset, Tail::Eof] {
                assert!(read_response(&mut input(bytes, tail)).await.is_err());
            }
        }
    }

    #[tokio::test]
    async fn fragmented_headers_and_body_still_finish_before_reset() {
        let mut response = input(COMPLETE, Tail::Reset);
        response.fragment = 3;
        assert_eq!(read_response(&mut response).await.unwrap(), COMPLETE);
    }

    #[tokio::test]
    async fn ambiguous_or_excessive_lengths_and_headers_are_refused() {
        for bytes in [
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Length: 2\r\n\r\n{}".as_slice(),
            b"HTTP/1.1 200 OK\r\nContent-Length: invalid\r\n\r\n{}",
            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n{}",
            b"HTTP/1.1 200 OK\r\nContent-Length: 4194305\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Length: 2\r\n\r\n{}",
        ] {
            assert!(read_response(&mut input(bytes, Tail::Eof)).await.is_err());
        }
        assert!(
            read_response(&mut input(&vec![b'x'; 16385], Tail::Eof))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn existing_chunked_and_close_delimited_replies_still_require_eof() {
        for bytes in [
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\n\r\n".as_slice(),
            b"HTTP/1.1 204 No Content\r\n\r\n",
        ] {
            assert_eq!(
                read_response(&mut input(bytes, Tail::Eof)).await.unwrap(),
                bytes
            );
            assert!(read_response(&mut input(bytes, Tail::Reset)).await.is_err());
        }
    }
}
