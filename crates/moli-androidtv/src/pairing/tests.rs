//! Public synthetic RSA integers only. No private PEM, real TV or account.

use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, DuplexStream};
use tokio::sync::oneshot;
use tokio::time::Instant;

use super::{Identity, PairingError, PairingSession, PublicKey, proof};
use crate::proto::{Writer, parse};

const DIGEST: [u8; 32] = [
    0x03, 0x8e, 0x7a, 0x67, 0xe0, 0x40, 0xf7, 0xd1, 0x88, 0x06, 0x5c, 0x3c, 0x49, 0x85, 0x95, 0x77,
    0x78, 0xf3, 0xee, 0x79, 0x92, 0x39, 0x19, 0xfc, 0xa3, 0xad, 0x73, 0x55, 0x01, 0x87, 0xe0, 0x22,
];

// Failed setup is kept distinct from a reached contract assertion. This
// assertion explicitly observes that the real client emitted the next frame
// or produced the identity/session required by the contract.
fn required<T, E>(result: Result<T, E>) -> T {
    assert!(
        result.is_ok(),
        "required identity or Polo protocol step did not complete"
    );
    match result {
        Ok(value) => value,
        Err(_) => unreachable!("assertion above rejects the failed contract"),
    }
}

fn emitted_byte(result: &std::io::Result<u8>) -> u8 {
    assert!(
        result.is_ok(),
        "wire emission: client did not emit its required Polo frame"
    );
    match result {
        Ok(byte) => *byte,
        Err(_) => unreachable!("assertion above rejects the missing emission"),
    }
}

async fn eof_before_drop(
    peer: &mut DuplexStream,
    finished: oneshot::Receiver<()>,
    observed: oneshot::Sender<()>,
) {
    // The human-expiration case deliberately waits until 120 s before finish.
    assert!(
        tokio::time::timeout(Duration::from_secs(121), finished)
            .await
            .is_ok_and(|v| v.is_ok()),
        "terminal API: finish did not return"
    );
    let read = tokio::time::timeout(Duration::from_secs(1), peer.read_u8()).await;
    assert!(
        matches!(read, Ok(Err(ref error)) if error.kind() == std::io::ErrorKind::UnexpectedEof),
        "terminal stream: finish must close without emitting another byte before session Drop"
    );
    required(observed.send(()));
}

async fn keep_alive_after_finish(
    session: &mut PairingSession<DuplexStream>,
    finished: oneshot::Sender<()>,
    observed: oneshot::Receiver<()>,
) {
    required(finished.send(()));
    assert!(
        tokio::time::timeout(Duration::from_secs(2), observed)
            .await
            .is_ok_and(|v| v.is_ok()),
        "terminal API: peer must observe EOF while session remains alive"
    );
    // This use after the barrier prevents Drop from supplying the EOF oracle.
    assert!(session.finish("031234").await.is_err());
}

fn keys() -> (PublicKey, PublicKey) {
    (
        PublicKey {
            modulus: vec![0xa1; 256],
            exponent: vec![1, 0, 1],
        },
        PublicKey {
            modulus: vec![0xb3; 256],
            exponent: vec![1, 0, 1],
        },
    )
}

#[test]
fn proof_binds_client_server_public_keys_and_last_two_code_bytes() {
    let (client, server) = keys();
    assert_eq!(proof(&client, &server, "031234"), Ok(DIGEST));
    assert!(proof(&server, &client, "031234").is_err());
}

#[test]
fn invalid_code_does_not_leak_into_error_or_debug() {
    let (client, server) = keys();
    for code in [
        "", "03123", "0312345", "zz1234", " 31234", "03123é", "001234",
    ] {
        let error = proof(&client, &server, code).unwrap_err();
        assert_eq!(error, PairingError::InvalidCode);
        if !code.is_empty() {
            assert!(!error.to_string().contains(code));
            assert!(!format!("{error:?}").contains(code));
        }
    }
}

#[test]
fn unsigned_rsa_integers_are_bounded_before_hashing() {
    let (mut client, server) = keys();
    for modulus in [vec![], vec![0; 256], vec![0xa1; 8192]] {
        client.modulus = modulus;
        assert_eq!(
            proof(&client, &server, "031234"),
            Err(PairingError::InvalidKey)
        );
    }
}

#[test]
fn generated_identity_is_rsa_2048_and_debug_never_discloses_credentials() {
    let identity = required(Identity::generate());
    let public = required(identity.public_key());
    assert_eq!(public.modulus.len(), 256);
    assert_eq!(public.exponent, [1, 0, 1]);
    assert!(
        identity
            .certificate_pem()
            .starts_with("-----BEGIN CERTIFICATE-----")
    );
    let (_, pem) = x509_parser::pem::parse_x509_pem(identity.key_pem().as_bytes()).unwrap();
    assert_eq!(pem.label, "PRIVATE KEY");
    let debug = format!("{identity:?}");
    assert!(!debug.contains("BEGIN"));
    assert!(!debug.contains(identity.key_pem()));
}

#[test]
fn identity_roundtrip_requires_the_matching_private_key() {
    let first = required(Identity::generate());
    let second = required(Identity::generate());
    let roundtrip = required(Identity::from_pem(
        first.certificate_pem().to_owned(),
        first.key_pem().to_owned(),
    ));
    assert_eq!(roundtrip.certificate_pem(), first.certificate_pem());
    assert!(
        Identity::from_pem(
            first.certificate_pem().to_owned(),
            second.key_pem().to_owned()
        )
        .is_err()
    );
    assert!(Identity::from_pem(String::new(), String::new()).is_err());
}

// An independent test peer: frame bytes are written one at a time so message
// fragmentation cannot be hidden by the duplex transport's buffering.
async fn reply(stream: &mut DuplexStream, field: u32, body: Writer) {
    let raw = Writer::new()
        .uint(1, 2)
        .uint(2, 200)
        .msg(field, body)
        .finish();
    let mut prefix = Vec::new();
    crate::proto::varint(raw.len() as u64, &mut prefix);
    for byte in prefix.into_iter().chain(raw) {
        stream.write_all(&[byte]).await.unwrap();
        tokio::task::yield_now().await;
    }
}

async fn request(stream: &mut DuplexStream) -> Vec<u8> {
    let mut length = 0usize;
    for shift in (0..35).step_by(7) {
        let byte = emitted_byte(&stream.read_u8().await);
        length |= usize::from(byte & 127) << shift;
        if byte < 128 {
            let mut frame = vec![0; length];
            stream.read_exact(&mut frame).await.unwrap();
            return frame;
        }
    }
    panic!("invalid frame emitted by client");
}

async fn negotiation(peer: &mut DuplexStream) {
    let frame = request(peer).await;
    let outer = parse(&frame).unwrap();
    assert_eq!(outer.uint(1), Some(2));
    assert_eq!(outer.uint(2), Some(200));
    assert_eq!(outer.msg(10).unwrap().str(1), Some("atvremote"));
    reply(peer, 11, Writer::new()).await;
    let frame = request(peer).await;
    let options = parse(&frame).unwrap().msg(20).unwrap();
    assert_eq!(options.uint(3), Some(1));
    assert_eq!(options.msg(1).unwrap().uint(1), Some(3));
    assert_eq!(options.msg(1).unwrap().uint(2), Some(6));
    reply(
        peer,
        20,
        Writer::new()
            .uint(3, 2)
            .msg(2, Writer::new().uint(1, 3).uint(2, 6)),
    )
    .await;
    let frame = request(peer).await;
    let config = parse(&frame).unwrap().msg(30).unwrap();
    assert_eq!(config.uint(2), Some(1));
    assert_eq!(config.msg(1).unwrap().uint(1), Some(3));
    assert_eq!(config.msg(1).unwrap().uint(2), Some(6));
    reply(peer, 31, Writer::new()).await;
}

#[tokio::test(start_paused = true)]
async fn fragmented_polo_negotiation_requires_secret_ack_after_the_proof() {
    let (stream, mut peer) = tokio::io::duplex(8192);
    let (client, server) = keys();
    let (finished_tx, finished_rx) = oneshot::channel();
    let (observed_tx, observed_rx) = oneshot::channel();
    let television = async {
        negotiation(&mut peer).await;
        let frame = request(&mut peer).await;
        let message = parse(&frame).unwrap();
        assert_eq!(message.msg(40).unwrap().bytes(1), Some(DIGEST.as_slice()));
        // The acknowledgement payload is opaque, not a documented proof echo.
        reply(&mut peer, 41, Writer::new().bytes(1, &[0xa5; 32])).await;
        eof_before_drop(&mut peer, finished_rx, observed_tx).await;
    };
    let remote = async {
        let mut session = required(
            PairingSession::negotiate(
                stream,
                client,
                server,
                Instant::now() + Duration::from_secs(120),
            )
            .await,
        );
        assert_eq!(session.finish("031234").await, Ok(()));
        keep_alive_after_finish(&mut session, finished_tx, observed_rx).await;
    };
    tokio::join!(television, remote);
}

#[tokio::test(start_paused = true)]
async fn invalid_code_sends_no_secret_and_closes_the_owned_stream() {
    let (stream, mut peer) = tokio::io::duplex(8192);
    let (client, server) = keys();
    let (finished_tx, finished_rx) = oneshot::channel();
    let (observed_tx, observed_rx) = oneshot::channel();
    let television = async {
        negotiation(&mut peer).await;
        eof_before_drop(&mut peer, finished_rx, observed_tx).await;
    };
    let remote = async {
        let mut session = required(
            PairingSession::negotiate(
                stream,
                client,
                server,
                Instant::now() + Duration::from_secs(120),
            )
            .await,
        );
        assert_eq!(
            session.finish("001234").await,
            Err(PairingError::InvalidCode)
        );
        keep_alive_after_finish(&mut session, finished_tx, observed_rx).await;
    };
    tokio::join!(television, remote);
}

#[tokio::test(start_paused = true)]
async fn silence_is_bounded_by_network_step_not_the_human_window() {
    let (stream, _silent_peer) = tokio::io::duplex(8192);
    let (client, server) = keys();
    let start = Instant::now();
    let result =
        PairingSession::negotiate(stream, client, server, start + Duration::from_secs(120)).await;
    assert_eq!(result.unwrap_err(), PairingError::Expired);
    assert!(start.elapsed() <= Duration::from_secs(4));
}

#[tokio::test(start_paused = true)]
async fn human_deadline_cannot_be_renewed_when_code_arrives() {
    let (stream, mut peer) = tokio::io::duplex(8192);
    let (client, server) = keys();
    let end = Instant::now() + Duration::from_secs(120);
    let (finished_tx, finished_rx) = oneshot::channel();
    let (observed_tx, observed_rx) = oneshot::channel();
    let television = async {
        negotiation(&mut peer).await;
        eof_before_drop(&mut peer, finished_rx, observed_tx).await;
    };
    let remote = async {
        let mut session = required(PairingSession::negotiate(stream, client, server, end).await);
        tokio::time::sleep_until(end).await;
        assert_eq!(session.finish("031234").await, Err(PairingError::Expired));
        keep_alive_after_finish(&mut session, finished_tx, observed_rx).await;
    };
    tokio::join!(television, remote);
}

#[tokio::test(start_paused = true)]
async fn wrong_version_refusal_and_out_of_order_reply_close_the_session() {
    for (version, status, field, expected) in [
        (1, 200, 11, PairingError::Protocol),
        (2, 402, 11, PairingError::Refused),
        (2, 200, 31, PairingError::Protocol),
    ] {
        let (stream, mut peer) = tokio::io::duplex(8192);
        let (client, server) = keys();
        let television = async {
            let _ = request(&mut peer).await;
            let raw = Writer::new()
                .uint(1, version)
                .uint(2, status)
                .msg(field, Writer::new())
                .finish();
            let mut framed = Vec::new();
            crate::proto::varint(raw.len() as u64, &mut framed);
            framed.extend_from_slice(&raw);
            peer.write_all(&framed).await.unwrap();
            let read = tokio::time::timeout(Duration::from_secs(1), peer.read_u8()).await;
            assert!(
                matches!(read, Ok(Err(ref error)) if error.kind() == std::io::ErrorKind::UnexpectedEof),
                "terminal stream: rejected negotiation must close its owned stream"
            );
        };
        let remote = async {
            let result = PairingSession::negotiate(
                stream,
                client,
                server,
                Instant::now() + Duration::from_secs(120),
            )
            .await;
            assert_eq!(result.unwrap_err(), expected);
        };
        tokio::join!(television, remote);
    }
}

#[tokio::test(start_paused = true)]
async fn oversized_frame_is_refused_from_its_prefix_without_reading_the_body() {
    let (stream, mut peer) = tokio::io::duplex(8192);
    let (client, server) = keys();
    let television = async {
        let _ = request(&mut peer).await;
        // 65537, varint; no body follows. Must fail before the step times out.
        peer.write_all(&[0x81, 0x80, 0x04]).await.unwrap();
        let read = tokio::time::timeout(Duration::from_secs(1), peer.read_u8()).await;
        assert!(
            matches!(read, Ok(Err(ref error)) if error.kind() == std::io::ErrorKind::UnexpectedEof),
            "terminal stream: rejected negotiation must close its owned stream"
        );
    };
    let remote = async {
        let start = Instant::now();
        let result =
            PairingSession::negotiate(stream, client, server, start + Duration::from_secs(120))
                .await;
        assert_eq!(result.unwrap_err(), PairingError::Protocol);
        assert!(start.elapsed() < Duration::from_secs(4));
    };
    tokio::join!(television, remote);
}

#[tokio::test(start_paused = true)]
async fn another_phase_cannot_acknowledge_a_secret_or_reuse_the_attempt() {
    let (stream, mut peer) = tokio::io::duplex(8192);
    let (client, server) = keys();
    let (finished_tx, finished_rx) = oneshot::channel();
    let (observed_tx, observed_rx) = oneshot::channel();
    let television = async {
        negotiation(&mut peer).await;
        let _ = request(&mut peer).await;
        reply(&mut peer, 31, Writer::new()).await;
        eof_before_drop(&mut peer, finished_rx, observed_tx).await;
    };
    let remote = async {
        let mut session = required(
            PairingSession::negotiate(
                stream,
                client,
                server,
                Instant::now() + Duration::from_secs(120),
            )
            .await,
        );
        assert_eq!(session.finish("031234").await, Err(PairingError::Protocol));
        keep_alive_after_finish(&mut session, finished_tx, observed_rx).await;
    };
    tokio::join!(television, remote);
}
