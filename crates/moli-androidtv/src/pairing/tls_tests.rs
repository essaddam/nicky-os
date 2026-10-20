//! Generated in-memory identities, loopback TLS, no stored private fixtures.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use moli_net::PinnedTls;
use ring::digest::{Context, SHA256};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject as _};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::time::{Instant, timeout};

use super::{Identity, PairingSession};
use crate::proto::{Writer, parse, varint};

static IDENTITIES: OnceLock<(Identity, Identity)> = OnceLock::new();

async fn frame<S: AsyncRead + Unpin>(stream: &mut S) -> Vec<u8> {
    let mut length = 0;
    for shift in (0..35).step_by(7) {
        let next = stream.read_u8().await.expect("client frame missing");
        length |= usize::from(next & 127) << shift;
        assert!(length <= 65536);
        if next < 128 {
            let mut body = vec![0; length];
            stream.read_exact(&mut body).await.unwrap();
            return body;
        }
    }
    panic!("invalid client length");
}

async fn respond<S: AsyncWrite + Unpin>(stream: &mut S, field: u32, body: Writer) {
    let body = Writer::new()
        .uint(1, 2)
        .uint(2, 200)
        .msg(field, body)
        .finish();
    let mut packet = Vec::new();
    varint(body.len() as u64, &mut packet);
    packet.extend_from_slice(&body);
    stream.write_all(&packet).await.unwrap();
    stream.flush().await.unwrap();
}

#[tokio::test]
async fn tls_pairing_binds_the_client_certificate_and_actual_peer_rsa_key() {
    let (client, server) = tokio::task::spawn_blocking(|| {
        IDENTITIES.get_or_init(|| (Identity::generate().unwrap(), Identity::generate().unwrap()))
    })
    .await
    .unwrap();
    let client_der = CertificateDer::from_pem_slice(client.certificate_pem().as_bytes()).unwrap();
    let server_der = CertificateDer::from_pem_slice(server.certificate_pem().as_bytes()).unwrap();
    let server_key = PrivateKeyDer::from_pem_slice(server.key_pem().as_bytes()).unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut roots = rustls::RootCertStore::empty();
    roots.add(client_der.clone()).unwrap();
    let verifier = rustls::server::WebPkiClientVerifier::builder_with_provider(
        Arc::new(roots),
        Arc::clone(&provider),
    )
    .build()
    .unwrap();
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_client_cert_verifier(verifier)
        .with_single_cert(vec![server_der], server_key)
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let client_key = client.public_key().unwrap();
    let server_key = server.public_key().unwrap();
    let mut digest = Context::new(&SHA256);
    for bytes in [
        &client_key.modulus,
        &client_key.exponent,
        &server_key.modulus,
        &server_key.exponent,
    ] {
        digest.update(bytes);
    }
    digest.update(&[0x12, 0x34]);
    let secret = digest.finish();
    let code = format!("{:02X}1234", secret.as_ref()[0]);
    let television = async {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut stream = acceptor
            .accept(tcp)
            .await
            .expect("mutual TLS must authenticate the generated client");
        assert_eq!(
            stream.get_ref().1.peer_certificates().unwrap()[0],
            client_der
        );
        for (request, reply) in [(10, 11), (20, 20), (30, 31)] {
            let raw = frame(&mut stream).await;
            assert!(parse(&raw).unwrap().msg(request).is_some());
            let body = if reply == 20 {
                Writer::new()
                    .msg(2, Writer::new().uint(1, 3).uint(2, 6))
                    .uint(3, 2)
            } else {
                Writer::new()
            };
            respond(&mut stream, reply, body).await;
        }
        let raw = frame(&mut stream).await;
        assert_eq!(
            parse(&raw).unwrap().msg(40).unwrap().bytes(1),
            Some(secret.as_ref())
        );
        respond(&mut stream, 41, Writer::new().bytes(1, b"opaque-ack")).await;
    };
    let remote = async {
        let tls =
            PinnedTls::with_client_cert(None, client.certificate_pem(), client.key_pem()).unwrap();
        let stream = tls.connect("127.0.0.1", address.port()).await.unwrap();
        let result =
            PairingSession::from_tls(stream, client, Instant::now() + Duration::from_secs(120))
                .await;
        assert!(
            result.is_ok(),
            "TLS pair must negotiate with the actual peer key"
        );
        let mut session = result.unwrap();
        assert_eq!(session.finish(&code).await, Ok(()));
    };
    timeout(
        Duration::from_secs(10),
        Box::pin(async {
            tokio::join!(television, remote);
        }),
    )
    .await
    .unwrap();
}
