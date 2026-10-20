//! Fixed-host UPnP RenderingControl: no URL discovery, no credentials.
use moli_runtime::CommandRequest;

pub(crate) async fn call(
    host: &str,
    port: u16,
    action: &str,
    value: Option<&str>,
    command: Option<&CommandRequest>,
) -> anyhow::Result<String> {
    let desired = match (action, value) {
        ("GetVolume" | "GetMute", None) => String::new(),
        ("SetVolume", Some(v)) if v.parse::<u8>().is_ok_and(|n| n <= 100) => {
            format!("<DesiredVolume>{v}</DesiredVolume>")
        }
        ("SetMute", Some(v @ ("0" | "1"))) => format!("<DesiredMute>{v}</DesiredMute>"),
        _ => anyhow::bail!("invalid audio action"),
    };
    let service = "urn:schemas-upnp-org:service:RenderingControl:1";
    let body = format!(
        r#"<?xml version="1.0"?><s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"><s:Body><u:{action} xmlns:u="{service}"><InstanceID>0</InstanceID><Channel>Master</Channel>{desired}</u:{action}></s:Body></s:Envelope>"#
    );
    let request = http::Request::builder()
        .method(http::Method::POST)
        .uri("/upnp/control/RenderingControl1")
        .header("host", format!("{host}:{port}"))
        .header("content-type", "text/xml; charset=utf-8")
        .header("soapaction", format!("\"{service}#{action}\""))
        .body(http_body_util::Full::new(body.into()))?;
    let (status, bytes) = moli_net::plain_limited_if(host, port, request, 64 * 1024, || {
        command.is_none_or(|c| !c.abandoned())
    })
    .await?;
    let text = std::str::from_utf8(&bytes)?;
    anyhow::ensure!(
        status.is_success()
            && !text.contains(":Fault")
            && !text.contains("<Fault")
            && text.contains(&format!("{action}Response")),
        "audio refused"
    );
    Ok(text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    #[tokio::test]
    async fn audio_uses_fixed_control_path_and_rejects_faults_even_on_http_success() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let mut captured = Vec::new();
            for reply in [
                "<SetVolumeResponse/>",
                "<s:Fault><errorCode>501</errorCode></s:Fault>",
            ] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let mut chunk = [0; 4096];
                loop {
                    let n = socket.read(&mut chunk).await.unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&chunk[..n]);
                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                        let len: usize = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length: "))
                            .unwrap()
                            .parse()
                            .unwrap();
                        if bytes.len() >= end + 4 + len {
                            break;
                        }
                    }
                }
                captured.push(String::from_utf8(bytes).unwrap());
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len()).as_bytes()).await.unwrap();
            }
            captured
        });
        assert!(
            call("127.0.0.1", port, "SetVolume", Some("42"), None)
                .await
                .is_ok()
        );
        assert!(
            call("127.0.0.1", port, "SetMute", Some("1"), None)
                .await
                .is_err()
        );
        let requests = server.await.unwrap();
        assert!(requests[0].starts_with("POST /upnp/control/RenderingControl1 HTTP/1.1"));
        assert!(requests[0].contains("<DesiredVolume>42</DesiredVolume>"));
        assert!(requests[1].contains("<DesiredMute>1</DesiredMute>"));
        assert!(!requests.iter().any(|r| r.contains("token")));
    }
}
