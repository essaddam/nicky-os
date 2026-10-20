//! Human-initiated Polo v2 pairing. No code, key or proof is logged.

use std::fmt;
use std::fmt::Write as _;
use std::time::Duration;

use ring::digest::{Context, SHA256};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::Instant;

mod identity;
mod wire;
pub use identity::Identity;

pub const PORT: u16 = 6467;
const STEP: Duration = Duration::from_secs(4);
const HUMAN_WINDOW: Duration = Duration::from_secs(120);

/// Public RSA integers, unsigned big-endian, without DER sign padding.
#[derive(Clone, Debug)]
pub struct PublicKey {
    pub modulus: Vec<u8>,
    pub exponent: Vec<u8>,
}

impl PublicKey {
    fn validate(&self) -> Result<(), PairingError> {
        if !(256..=1024).contains(&self.modulus.len())
            || self.modulus.first().is_none_or(|byte| *byte == 0)
            || self.modulus.last().is_none_or(|byte| byte & 1 == 0)
            || self.exponent.is_empty()
            || self.exponent.len() > 8
            || self.exponent[0] == 0
            || self.exponent.last().is_none_or(|byte| byte & 1 == 0)
            || self.exponent == [1]
        {
            return Err(PairingError::InvalidKey);
        }
        Ok(())
    }
}

/// Errors deliberately carry no peer-controlled text or entered code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PairingError {
    InvalidCode,
    InvalidKey,
    Protocol,
    Refused,
    Expired,
    Disconnected,
}

impl fmt::Display for PairingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidCode => "invalid pairing code",
            Self::InvalidKey => "invalid pairing public key",
            Self::Protocol => "invalid pairing response",
            Self::Refused => "pairing refused",
            Self::Expired => "pairing expired",
            Self::Disconnected => "pairing disconnected",
        })
    }
}

impl std::error::Error for PairingError {}

/// The first code byte checks SHA-256 before any secret is sent.
///
/// # Errors
/// Rejects malformed keys, codes and mismatched check bytes without echoing them.
pub fn proof(client: &PublicKey, server: &PublicKey, code: &str) -> Result<[u8; 32], PairingError> {
    client.validate()?;
    server.validate()?;
    if code.len() != 6 || !code.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(PairingError::InvalidCode);
    }
    let mut bytes = [0u8; 3];
    for (out, pair) in bytes.iter_mut().zip(code.as_bytes().as_chunks::<2>().0) {
        let digit = |byte: u8| match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            _ => byte - b'A' + 10,
        };
        *out = digit(pair[0]) * 16 + digit(pair[1]);
    }
    let mut digest = Context::new(&SHA256);
    for part in [
        &client.modulus,
        &client.exponent,
        &server.modulus,
        &server.exponent,
    ] {
        digest.update(part);
    }
    digest.update(&bytes[1..]);
    let digest = digest.finish();
    if digest.as_ref()[0] != bytes[0] {
        return Err(PairingError::InvalidCode);
    }
    let mut result = [0; 32];
    result.copy_from_slice(digest.as_ref());
    Ok(result)
}

/// Owns the pairing stream until completion, cancellation or deadline.
pub struct PairingSession<S> {
    stream: Option<S>,
    client: PublicKey,
    server: PublicKey,
    deadline: Instant,
    fingerprint: Option<String>,
}

impl<S> fmt::Debug for PairingSession<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PairingSession")
            .field("active", &self.stream.is_some())
            .finish_non_exhaustive()
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> PairingSession<S> {
    /// The actual TLS peer fingerprint, absent on a generic test transport.
    #[must_use]
    pub fn peer_fingerprint(&self) -> Option<&str> {
        self.fingerprint.as_deref()
    }

    /// Negotiates the six-character hexadecimal input role on an authenticated
    /// transport. The TLS constructor supplies the keys from the actual peer.
    ///
    /// # Errors
    /// Rejects an invalid sequence, a refused response, silence or disconnection.
    pub async fn negotiate(
        mut stream: S,
        client: PublicKey,
        server: PublicKey,
        deadline: Instant,
    ) -> Result<Self, PairingError> {
        use crate::proto::{Writer, parse};
        client.validate()?;
        server.validate()?;
        // Even an overly generous caller cannot extend the human window.
        let deadline = deadline.min(Instant::now() + HUMAN_WINDOW);
        wire::exchange(
            &mut stream,
            10,
            Writer::new().str(1, "atvremote").str(2, "Moli"),
            11,
            deadline,
        )
        .await?;
        let options = wire::exchange(
            &mut stream,
            20,
            Writer::new()
                .msg(1, Writer::new().uint(1, 3).uint(2, 6))
                .uint(3, 1),
            20,
            deadline,
        )
        .await?;
        let options = parse(&options).ok_or(PairingError::Protocol)?;
        if !options
            .all_msgs(2)
            .any(|encoding| encoding.uint(1) == Some(3) && encoding.uint(2) == Some(6))
        {
            return Err(PairingError::Protocol);
        }
        wire::exchange(
            &mut stream,
            30,
            Writer::new()
                .msg(1, Writer::new().uint(1, 3).uint(2, 6))
                .uint(2, 1),
            31,
            deadline,
        )
        .await?;
        Ok(Self {
            stream: Some(stream),
            client,
            server,
            deadline,
            fingerprint: None,
        })
    }

    /// Validates a human code and waits for the matching secret acknowledgement.
    ///
    /// # Errors
    /// The original human deadline is never renewed. Any terminal failure closes
    /// the stream; a new attempt requires a new explicit human action.
    pub async fn finish(&mut self, code: &str) -> Result<(), PairingError> {
        use crate::proto::Writer;
        // Take ownership BEFORE awaiting: dropping this future also closes the
        // stream; cancellation cannot leave an active attempt behind.
        let mut stream = self.stream.take().ok_or(PairingError::Protocol)?;
        if Instant::now() >= self.deadline {
            return Err(PairingError::Expired);
        }
        let secret = proof(&self.client, &self.server, code)?;
        wire::exchange(
            &mut stream,
            40,
            Writer::new().bytes(1, &secret),
            41,
            self.deadline,
        )
        .await?;
        // The SecretAck payload is opaque. Only its status, type and order are
        // established; it is not an undocumented echo of our digest.
        Ok(())
    }
}

impl PairingSession<tokio_rustls::client::TlsStream<tokio::net::TcpStream>> {
    /// Connect to the fixed pairing port using this identity and the supplied
    /// certificate pin. Identity material is never inferred from caller text.
    ///
    /// # Errors
    /// TLS, peer-key, protocol and deadline failures close the owned stream.
    pub async fn start(
        tls: &moli_net::PinnedTls,
        host: &str,
        identity: &Identity,
    ) -> Result<Self, PairingError> {
        let deadline = Instant::now() + HUMAN_WINDOW;
        let tls = moli_net::PinnedTls::with_client_cert(
            tls.pinned_fingerprint(),
            identity.certificate_pem(),
            identity.key_pem(),
        )
        .map_err(|_| PairingError::InvalidKey)?;
        let stream =
            tokio::time::timeout_at(deadline.min(Instant::now() + STEP), tls.connect(host, PORT))
                .await
                .map_err(|_| PairingError::Expired)?
                .map_err(|_| PairingError::Disconnected)?;
        Self::from_tls(stream, identity, deadline).await
    }

    async fn from_tls(
        stream: tokio_rustls::client::TlsStream<tokio::net::TcpStream>,
        identity: &Identity,
        deadline: Instant,
    ) -> Result<Self, PairingError> {
        let cert = stream
            .get_ref()
            .1
            .peer_certificates()
            .and_then(|chain| chain.first())
            .ok_or(PairingError::InvalidKey)?;
        let server = identity::public_key_der(cert.as_ref())?;
        let mut fingerprint = String::with_capacity(64);
        for byte in ring::digest::digest(&SHA256, cert.as_ref()).as_ref() {
            let _ = write!(fingerprint, "{byte:02x}");
        }
        let mut session = Self::negotiate(stream, identity.public_key()?, server, deadline).await?;
        session.fingerprint = Some(fingerprint);
        Ok(session)
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tls_tests;
