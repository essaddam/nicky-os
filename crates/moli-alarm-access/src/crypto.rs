use crate::Code;

pub(crate) fn sign(key: &[u8; 32], code: &Code<'_>) -> Option<Vec<u8>> {
    if !code.valid() {
        return None;
    }
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key);
    Some(ring::hmac::sign(&key, code.0?.as_bytes()).as_ref().to_vec())
}
pub(crate) fn matches(key: &[u8; 32], code: &Code<'_>, tag: &[u8]) -> bool {
    code.valid()
        && code.0.is_some_and(|value| {
            let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key);
            ring::hmac::verify(&key, value.as_bytes(), tag).is_ok()
        })
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}
pub(crate) fn unhex(value: &str) -> Option<Vec<u8>> {
    if !value.len().is_multiple_of(2) || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    (0..value.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&value[i..i + 2], 16).ok())
        .collect()
}
pub(crate) fn random_key() -> Result<[u8; 32], crate::Error> {
    use ring::rand::SecureRandom;
    let mut key = [0; 32];
    ring::rand::SystemRandom::new()
        .fill(&mut key)
        .map_err(|_| crate::Error::Unavailable)?;
    Ok(key)
}
pub(crate) fn random_id() -> Result<String, crate::Error> {
    Ok(hex(&random_key()?[..16]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hmac_matches_only_the_same_code_and_key_without_trimming() {
        let key = [19; 32];
        let code = Code::provided("70710937");
        let tag = sign(&key, &code).unwrap();
        assert_eq!(tag.len(), 32);
        assert!(matches(&key, &code, &tag));
        assert!(!matches(&[20; 32], &code, &tag));
        assert!(!matches(&key, &Code::provided("70710938"), &tag));
        assert!(!matches(&key, &Code::provided(" 70710937"), &tag));
        assert!(sign(&key, &Code::missing()).is_none());
    }
}
