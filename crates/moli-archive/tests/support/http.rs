//! The integration fixture's bounded HTTP response reader.
//! A socket read timeout can be `WouldBlock` on Unix, even in blocking mode.
//! A complete HTTP message must not need another read just to observe TCP EOF.
use std::io::{self, Read};

pub fn read_response(stream: &mut impl Read) -> io::Result<(u16, Vec<u8>)> {
    let mut stream = io::BufReader::new(stream);
    // HTTP message completion is independent from TCP close. In particular,
    // a descheduled peer must not turn an already complete reply into a timeout.
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        if header.len() == 16 * 1024 {
            return Err(invalid("HTTP headers too large"));
        }
        let mut byte = [0];
        stream.read_exact(&mut byte).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("HTTP headers ({} bytes): {error}", header.len()),
            )
        })?;
        header.push(byte[0]);
    }
    let headers =
        std::str::from_utf8(&header).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let mut lines = headers.split("\r\n");
    let code = lines
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| invalid("invalid HTTP status"))?;
    let mut length = None;
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| invalid("invalid HTTP header"))?;
        if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err(invalid("bounded JSON must use Content-Length"));
        }
        if name.eq_ignore_ascii_case("content-length") {
            if length.is_some() {
                return Err(invalid("duplicate Content-Length"));
            }
            let size = value
                .trim()
                .parse::<usize>()
                .map_err(|_| invalid("invalid Content-Length"))?;
            // Includes the 2 MiB catalogue and its JSON response envelope.
            if size > 3 * 1024 * 1024 {
                return Err(invalid("HTTP response body too large"));
            }
            length = Some(size);
        }
    }
    let mut body = vec![0; length.ok_or_else(|| invalid("missing Content-Length"))?];
    stream.read_exact(&mut body).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("HTTP body ({} bytes): {error}", body.len()),
        )
    })?;
    Ok((code, body))
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Write,
        net::{TcpListener, TcpStream},
        time::Duration,
    };

    fn sockets() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        client
            .set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        (client, server)
    }

    #[test]
    fn a_complete_response_does_not_wait_for_a_delayed_tcp_close() {
        let (mut client, mut server) = sockets();
        server
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
            .unwrap();
        // Keep the peer open until the read returns: no scheduler or real
        // machine load decides whether EOF arrives before the read timeout.
        let result = read_response(&mut client);
        drop(server);
        assert_eq!(result.unwrap(), (200, b"{}".to_vec()));
    }

    #[test]
    fn an_incomplete_response_is_not_accepted_when_the_peer_stalls() {
        let (mut client, mut server) = sockets();
        server
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n{")
            .unwrap();
        let result = read_response(&mut client);
        drop(server);
        let error = result.unwrap_err();
        assert!(matches!(
            error.kind(),
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
        ));
    }

    #[test]
    fn fragmented_headers_and_body_are_read_completely() {
        struct Fragmented(io::Cursor<&'static [u8]>);
        impl Read for Fragmented {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                let size = buffer.len().min(3);
                self.0.read(&mut buffer[..size])
            }
        }
        let mut response = Fragmented(io::Cursor::new(
            b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\n\r\n{\"a\":1}",
        ));
        assert_eq!(
            read_response(&mut response).unwrap(),
            (200, br#"{"a":1}"#.to_vec())
        );
    }

    #[test]
    fn truncated_or_unbounded_responses_are_rejected() {
        for response in [
            &b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n{}"[..],
            &b"HTTP/1.1 200 OK\r\n\r\n{}"[..],
            &b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Length: 2\r\n\r\n{}"[..],
            &b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n"[..],
            &b"HTTP/1.1 200 OK\r\nContent-Length: 3145729\r\n\r\n"[..],
        ] {
            assert!(read_response(&mut io::Cursor::new(response)).is_err());
        }
        assert!(read_response(&mut io::Cursor::new(vec![b'a'; 16 * 1024])).is_err());
    }
}
