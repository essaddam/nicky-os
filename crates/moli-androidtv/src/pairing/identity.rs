//! A single client identity, persisted together by the driver's secret store.

use std::fmt;

use rand_core::OsRng;
use rcgen::{CertificateParams, DnType, ExtendedKeyUsagePurpose, KeyPair, KeyUsagePurpose};
use rsa::RsaPrivateKey;
use rsa::pkcs8::{DecodePrivateKey as _, EncodePrivateKey as _, LineEnding};
use rsa::traits::PublicKeyParts as _;
use x509_parser::public_key::PublicKey as ParsedPublicKey;

use super::{PairingError, PublicKey};

const MAX_CREDENTIAL: usize = 16 * 1024;

/// Client credentials. Debug output never contains the certificate or key.
pub struct Identity {
    certificate: String,
    key: String,
}

impl fmt::Debug for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Identity").finish_non_exhaustive()
    }
}

impl Identity {
    /// Generate RSA 2048 and a self-signed certificate, outside the async loop.
    ///
    /// # Errors
    /// Fails if secure key generation or certificate construction fails.
    pub fn generate() -> Result<Self, PairingError> {
        let private = RsaPrivateKey::new(&mut OsRng, 2048).map_err(|_| PairingError::InvalidKey)?;
        let key = private
            .to_pkcs8_pem(LineEnding::LF)
            .map_err(|_| PairingError::InvalidKey)?;
        let signing = KeyPair::from_pkcs8_pem_and_sign_algo(key.as_str(), &rcgen::PKCS_RSA_SHA256)
            .map_err(|_| PairingError::InvalidKey)?;
        let mut params =
            CertificateParams::new(Vec::<String>::new()).map_err(|_| PairingError::InvalidKey)?;
        params
            .distinguished_name
            .push(DnType::CommonName, "Moli Android TV");
        params.key_usages = vec![
            KeyUsagePurpose::DigitalSignature,
            KeyUsagePurpose::KeyEncipherment,
        ];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        let certificate = params
            .self_signed(&signing)
            .map_err(|_| PairingError::InvalidKey)?
            .pem();
        Self::from_pem(certificate, key.to_string())
    }

    /// Load one complete identity; certificate and private key must match.
    ///
    /// # Errors
    /// Malformed, oversized, non-RSA or mismatching credentials are rejected.
    pub fn from_pem(certificate: String, key: String) -> Result<Self, PairingError> {
        if certificate.len() > MAX_CREDENTIAL || key.len() > MAX_CREDENTIAL {
            return Err(PairingError::InvalidKey);
        }
        let candidate = Self { certificate, key };
        let public = candidate.public_key()?;
        let private =
            RsaPrivateKey::from_pkcs8_pem(&candidate.key).map_err(|_| PairingError::InvalidKey)?;
        if private.n().bits() != 2048
            || private.n().to_bytes_be() != public.modulus
            || private.e().to_bytes_be() != public.exponent
        {
            return Err(PairingError::InvalidKey);
        }
        private.validate().map_err(|_| PairingError::InvalidKey)?;
        Ok(candidate)
    }

    #[must_use]
    pub fn certificate_pem(&self) -> &str {
        &self.certificate
    }

    #[must_use]
    pub fn key_pem(&self) -> &str {
        &self.key
    }

    /// RSA integers actually carried by this identity's certificate.
    ///
    /// # Errors
    /// Rejects credentials whose public key cannot be used for Polo.
    pub fn public_key(&self) -> Result<PublicKey, PairingError> {
        let (rest, pem) = x509_parser::pem::parse_x509_pem(self.certificate.as_bytes())
            .map_err(|_| PairingError::InvalidKey)?;
        if pem.label != "CERTIFICATE" || !rest.iter().all(u8::is_ascii_whitespace) {
            return Err(PairingError::InvalidKey);
        }
        public_key_der(&pem.contents)
    }
}

pub(super) fn public_key_der(der: &[u8]) -> Result<PublicKey, PairingError> {
    if der.len() > 64 * 1024 {
        return Err(PairingError::InvalidKey);
    }
    let (rest, cert) =
        x509_parser::parse_x509_certificate(der).map_err(|_| PairingError::InvalidKey)?;
    if !rest.is_empty() {
        return Err(PairingError::InvalidKey);
    }
    let ParsedPublicKey::RSA(key) = cert
        .public_key()
        .parsed()
        .map_err(|_| PairingError::InvalidKey)?
    else {
        return Err(PairingError::InvalidKey);
    };
    let unsigned = |bytes: &[u8]| {
        bytes
            .iter()
            .copied()
            .skip_while(|byte| *byte == 0)
            .collect()
    };
    let key = PublicKey {
        modulus: unsigned(key.modulus),
        exponent: unsigned(key.exponent),
    };
    key.validate()?;
    Ok(key)
}
