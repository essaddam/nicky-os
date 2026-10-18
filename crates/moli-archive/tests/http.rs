//! The same authenticated protocol can target a separate container through ARCHIVE_TEST_URL.
use flate2::{Compression, write::GzEncoder};
use std::{
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
const INSTALL: &str = "00000000000000000000000000000002";
const TOKEN: &str = "fixture-archive-token-for-tests";
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Server {
    root: Option<PathBuf>,
    address: SocketAddr,
    process: Option<std::process::Child>,
}
impl Server {
    fn start() -> Self {
        if let Ok(url) = std::env::var("ARCHIVE_TEST_URL") {
            let address = url
                .strip_prefix("http://")
                .expect("test URL HTTP")
                .to_socket_addrs()
                .unwrap()
                .next()
                .unwrap();
            wait_ready(address);
            return Self {
                root: None,
                address,
                process: None,
            };
        }
        let root = std::env::temp_dir().join(format!(
            "moli-http-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let token = root.join("token");
        fs::write(&token, TOKEN).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let process = std::process::Command::new(env!("CARGO_BIN_EXE_moli-archive"))
            .args([
                "serve",
                "--installation",
                INSTALL,
                "--listen",
                &address.to_string(),
                "--token-file",
                token.to_str().unwrap(),
                "--data",
                root.join("data").to_str().unwrap(),
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .unwrap();
        let server = Self {
            root: Some(root),
            address,
            process: Some(process),
        };
        wait_ready(address);
        server
    }
    fn request(
        &self,
        method: &str,
        path: &str,
        token: &str,
        body: &[u8],
        encoding: Option<&str>,
    ) -> (u16, serde_json::Value) {
        let begin = Instant::now();
        loop {
            let response = self.request_once(method, path, token, body, encoding);
            if response.0 != 503 {
                return response;
            }
            assert!(begin.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    fn request_once(
        &self,
        method: &str,
        path: &str,
        token: &str,
        body: &[u8],
        encoding: Option<&str>,
    ) -> (u16, serde_json::Value) {
        let mut stream = TcpStream::connect(self.address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let extra = encoding.map_or_else(String::new, |e| format!("Content-Encoding: {e}\r\n"));
        write!(stream,"{method} /v1/installations/{INSTALL}/{path} HTTP/1.1\r\nHost: archive\r\nAuthorization: Bearer {token}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra}\r\n",body.len()).unwrap();
        stream.write_all(body).unwrap();
        let mut reply = Vec::new();
        stream.read_to_end(&mut reply).unwrap();
        let boundary = reply.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let headers = std::str::from_utf8(&reply[..boundary]).unwrap();
        let code = headers.split_whitespace().nth(1).unwrap().parse().unwrap();
        assert!(
            !headers
                .to_ascii_lowercase()
                .contains("transfer-encoding: chunked"),
            "bounded JSON has a content length"
        );
        (
            code,
            serde_json::from_slice(&reply[boundary + 4..]).unwrap(),
        )
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        if let Some(p) = &mut self.process {
            let _ = p.kill();
            let _ = p.wait();
        }
        if let Some(root) = &self.root {
            let _ = fs::remove_dir_all(root);
        }
    }
}
fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut g = GzEncoder::new(Vec::new(), Compression::fast());
    g.write_all(bytes).unwrap();
    g.finish().unwrap()
}
#[test]
fn authenticated_d13_transport_handles_replay_conflict_gap_and_read() {
    let server = Server::start();
    assert_eq!(server.request("GET", "status", "wrong", b"", None).0, 401);
    let (code, status) = server.request("GET", "status", TOKEN, b"", None);
    assert_eq!(code, 200);
    assert_eq!(status.as_object().unwrap().len(), 3);
    let first = status["durable_upto"].as_i64().unwrap() + 1;
    let line = format!(
        "{{\"type\":\"energy_hour\",\"seq\":{first},\"meter\":\"demo.meter\",\"hour\":0,\"kwh\":1.25,\"cost\":null}}\n"
    );
    let body = gzip(line.as_bytes());
    let path = format!("batches/{first}-{first}");
    assert_eq!(server.request("POST", &path, TOKEN, &body, None).0, 415);
    let (code, ack) = server.request("POST", &path, TOKEN, &body, Some("gzip"));
    assert_eq!(code, 200);
    assert_eq!(ack["durable_upto"], first);
    wait_published(&server, first);
    assert_eq!(
        server.request("POST", &path, TOKEN, &body, Some("gzip")).0,
        200
    );
    let conflict = gzip(line.replace("1.25", "2.5").as_bytes());
    assert_eq!(
        server
            .request("POST", &path, TOKEN, &conflict, Some("gzip"))
            .0,
        409
    );
    let gap = format!("batches/{}-{}", first + 2, first + 2);
    let gap_body = gzip(
        line.replace(
            &format!("\"seq\":{first}"),
            &format!("\"seq\":{}", first + 2),
        )
        .as_bytes(),
    );
    assert_eq!(
        server
            .request("POST", &gap, TOKEN, &gap_body, Some("gzip"))
            .0,
        409
    );
    let query=br#"{"source":"energy","points":["demo.meter"],"from":0,"to":3600000,"resolution":"raw","max_points":200}"#;
    let (code, response) = server.request("POST", "query", TOKEN, query, None);
    assert_eq!(code, 200);
    assert_eq!(response["series"][0]["values"].as_array().unwrap().len(), 1);
    let corrected = first + 1;
    let correction=gzip(format!("{{\"type\":\"energy_hour\",\"seq\":{corrected},\"meter\":\"demo.meter\",\"hour\":0,\"kwh\":1.5,\"cost\":0.3}}\n").as_bytes());
    assert_eq!(
        server
            .request(
                "POST",
                &format!("batches/{corrected}-{corrected}"),
                TOKEN,
                &correction,
                Some("gzip")
            )
            .0,
        200
    );
    wait_published(&server, corrected);
    let (_, repeat) = server.request("POST", &path, TOKEN, &body, Some("gzip"));
    assert_eq!(repeat["published_upto"], corrected);
    assert_energy_query(&server);
    let oversized = gzip(&vec![b' '; 8 * 1024 * 1024 + 1]);
    assert_eq!(
        server
            .request(
                "POST",
                &format!("batches/{}-{}", corrected + 1, corrected + 1),
                TOKEN,
                &oversized,
                Some("gzip")
            )
            .0,
        413
    );
    println!(
        "{}",
        serde_json::json!({"phase":"http_proof","generation":status["generation"],"published_upto":corrected})
    );
}

fn wait_ready(address: SocketAddr) {
    let start = Instant::now();
    while TcpStream::connect(address).is_err() {
        assert!(start.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn wait_published(server: &Server, target: i64) {
    let start = Instant::now();
    loop {
        let (code, status) = server.request("GET", "status", TOKEN, b"", None);
        assert_eq!(code, 200);
        if status["published_upto"] == target {
            return;
        }
        assert!(start.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn assert_energy_query(server: &Server) {
    let query=br#"{"source":"energy","points":["demo.meter"],"from":0,"to":3600000,"resolution":"raw","max_points":200}"#;
    let (code, reply) = server.request("POST", "query", TOKEN, query, None);
    assert_eq!(code, 200);
    assert_eq!(reply["series"][0]["values"].as_array().unwrap().len(), 1);
    assert_eq!(reply["series"][0]["values"][0]["kwh"], 1.5);
    assert_eq!(reply["series"][0]["values"][0]["cost"], 0.3);
    let query=br#"{"source":"energy","points":["demo.meter"],"from":0,"to":2678400000,"resolution":"month","max_points":200}"#;
    let (code, reply) = server.request("POST", "query", TOKEN, query, None);
    assert_eq!(code, 200);
    assert_eq!(
        reply["series"][0]["values"][0]["stat"]["aggregate"]["total"],
        1.5
    );
    assert_eq!(reply["series"][0]["values"][0]["known_cost_sum"], 0.3);
    assert_eq!(reply["series"][0]["values"][0]["coverage"], "unknown");
}
