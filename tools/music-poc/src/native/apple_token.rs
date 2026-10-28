//! Short-lived Apple Music developer JWTs, signed exclusively from environment inputs.

use std::{
    env,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, anyhow};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use p256::{
    ecdsa::{Signature, SigningKey, signature::Signer},
    pkcs8::DecodePrivateKey,
};
use serde_json::json;
use zeroize::Zeroizing;

const MAX_PEM_BYTES: usize = 8192;
const LIFETIME_SECONDS: u64 = 900;

fn invalid_configuration() -> anyhow::Error {
    anyhow!("Invalid Apple Music signing configuration")
}

fn identifier(value: &str) -> bool {
    value.len() == 10 && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn sign(team_id: &str, key_id: &str, pem: &str, now: u64) -> Result<String> {
    if !identifier(team_id)
        || !identifier(key_id)
        || pem.trim().is_empty()
        || pem.len() > MAX_PEM_BYTES
        || pem.contains("{{secret:")
    {
        return Err(invalid_configuration());
    }
    let expires = now
        .checked_add(LIFETIME_SECONDS)
        .ok_or_else(invalid_configuration)?;
    let signing_key = SigningKey::from_pkcs8_pem(pem).map_err(|_| invalid_configuration())?;
    let header = serde_json::to_vec(&json!({"alg":"ES256", "kid":key_id, "typ":"JWT"}))
        .map_err(|_| invalid_configuration())?;
    let claims = serde_json::to_vec(&json!({"iss":team_id, "iat":now, "exp":expires}))
        .map_err(|_| invalid_configuration())?;
    let message = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header),
        URL_SAFE_NO_PAD.encode(claims)
    );
    // JOSE ES256 uses the fixed-width r || s encoding, not ASN.1 DER.
    let signature: Signature = signing_key.sign(message.as_bytes());
    Ok(format!(
        "{message}.{}",
        URL_SAFE_NO_PAD.encode(signature.to_bytes())
    ))
}

fn read_variable(name: &str) -> Result<Option<String>> {
    match env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(_)) => Err(invalid_configuration()),
    }
}

/// Returns no token when all signing inputs are absent; partial inputs are rejected.
pub(super) fn from_env() -> Result<Option<String>> {
    let pem = read_variable("APPLE_MUSIC_PRIVATE_KEY_P8")?;
    let team_id = read_variable("APPLE_MUSIC_TEAM_ID")?;
    let key_id = read_variable("APPLE_MUSIC_KEY_ID")?;
    match (pem, team_id, key_id) {
        (None, None, None) => Ok(None),
        (Some(pem), Some(team_id), Some(key_id)) => {
            let pem = Zeroizing::new(pem);
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| invalid_configuration())?
                .as_secs();
            sign(&team_id, &key_id, &pem, now).map(Some)
        }
        _ => Err(invalid_configuration()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::{
        ecdsa::{VerifyingKey, signature::Verifier},
        pkcs8::{EncodePrivateKey, LineEnding},
    };
    use serde_json::Value;

    fn fixture_key() -> SigningKey {
        SigningKey::from_slice(&[1_u8; 32]).unwrap()
    }

    fn fixture_pem() -> String {
        fixture_key()
            .to_pkcs8_pem(LineEnding::LF)
            .unwrap()
            .to_string()
    }

    #[test]
    fn signs_a_verifiable_jose_token_with_fifteen_minute_expiry() {
        let token = sign("TEAM123456", "KEY1234567", &fixture_pem(), 1_700_000_000).unwrap();
        let parts: Vec<_> = token.split('.').collect();
        assert_eq!(parts.len(), 3);
        assert!(parts.iter().all(|part| !part.contains('=')));
        let header: Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[0]).unwrap()).unwrap();
        let claims: Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
        assert_eq!(
            header,
            json!({"alg":"ES256", "kid":"KEY1234567", "typ":"JWT"})
        );
        assert_eq!(
            claims,
            json!({"iss":"TEAM123456", "iat":1_700_000_000_u64, "exp":1_700_000_900_u64})
        );
        let bytes = URL_SAFE_NO_PAD.decode(parts[2]).unwrap();
        assert_eq!(bytes.len(), 64);
        let signature = Signature::from_slice(&bytes).unwrap();
        let verifying_key = VerifyingKey::from(&fixture_key());
        let message = format!("{}.{}", parts[0], parts[1]);
        assert!(verifying_key.verify(message.as_bytes(), &signature).is_ok());
        assert!(verifying_key.verify(b"changed input", &signature).is_err());
    }

    #[test]
    fn rejects_invalid_keys_identifiers_placeholders_and_expiry_overflow() {
        let pem = fixture_pem();
        for (team, key, private, now) in [
            ("TEAM12345", "KEY1234567", pem.as_str(), 0),
            ("TEAM12345_", "KEY1234567", pem.as_str(), 0),
            ("TEAM123456", "KEY123456_", pem.as_str(), 0),
            ("TEAM123456", "KEY1234567", "", 0),
            ("TEAM123456", "KEY1234567", "invalid-private-key", 0),
            ("TEAM123456", "KEY1234567", "{{secret:private-key}}", 0),
            ("TEAM123456", "KEY1234567", pem.as_str(), u64::MAX),
        ] {
            let error = sign(team, key, private, now).unwrap_err();
            assert_eq!(
                error.to_string(),
                "Invalid Apple Music signing configuration"
            );
            assert!(!format!("{error:?}").contains("PRIVATE KEY"));
        }
        assert!(
            sign(
                "TEAM123456",
                "KEY1234567",
                &"x".repeat(MAX_PEM_BYTES + 1),
                0
            )
            .is_err()
        );
    }
}
