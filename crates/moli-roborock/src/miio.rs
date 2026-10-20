//! The miIO protocol (Xiaomi, devices paired in Mi Home): UDP 54321, a
//! 32-byte header (`0x2131`, length, 0, device id, stamp, MD5 checksum),
//! then JSON encrypted with AES-128-CBC. Key = MD5(token), IV =
//! MD5(key ‖ token); checksum = MD5(header[..16] ‖ token ‖ data). The
//! token (16 bytes) never leaves Moli: it only keys and signs.
//!
//! Pure: no socket here. Everything decoded comes from the network and is
//! checked (length, device, checksum) before a byte of it is decrypted.

use aes::Aes128;
use aes::cipher::generic_array::GenericArray;
use aes::cipher::{BlockDecrypt as _, BlockEncrypt as _, KeyInit as _};
use md5::{Digest as _, Md5};

pub const PORT: u16 = 54321;
const MAGIC: [u8; 2] = [0x21, 0x31];
const HEADER: usize = 32;
/// A status is a few hundred bytes: anything bigger is not an answer.
pub const MAX_PACKET: usize = 16 * 1024;

/// The discovery packet: the device answers with its id and stamp.
#[must_use]
pub fn hello() -> [u8; 32] {
    let mut packet = [0xff; 32];
    packet[..4].copy_from_slice(&[0x21, 0x31, 0x00, 0x20]);
    packet
}

/// A hello answer: the device's id and its stamp.
#[must_use]
pub fn parse_hello(packet: &[u8]) -> Option<([u8; 4], u32)> {
    if packet.len() != HEADER || packet[..2] != MAGIC || packet[2..4] != [0x00, 0x20] {
        return None;
    }
    let id: [u8; 4] = packet[8..12].try_into().ok()?;
    let stamp = u32::from_be_bytes(packet[12..16].try_into().ok()?);
    Some((id, stamp))
}

/// A device's token: 32 hexadecimal digits, as Mi Home and its tools show.
pub fn token(hex: &str) -> Option<[u8; 16]> {
    let hex = hex.trim();
    if hex.len() != 32 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

/// Key and IV, and nothing printed: the token opens the device.
pub struct Keys {
    token: [u8; 16],
    key: [u8; 16],
    iv: [u8; 16],
}

impl std::fmt::Debug for Keys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Keys").finish_non_exhaustive()
    }
}

fn md5(parts: &[&[u8]]) -> [u8; 16] {
    let mut hasher = Md5::new();
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize().into()
}

/// Why a packet was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    Malformed,
    OtherDevice,
    Checksum,
    Decrypt,
}

impl Keys {
    #[must_use]
    pub fn new(token: [u8; 16]) -> Self {
        let key = md5(&[&token]);
        let iv = md5(&[&key, &token]);
        Self { token, key, iv }
    }

    fn encrypt(&self, plain: &[u8]) -> Vec<u8> {
        let cipher = Aes128::new(GenericArray::from_slice(&self.key));
        let mut buf = plain.to_vec();
        let pad = 16 - buf.len() % 16;
        buf.extend(std::iter::repeat_n(u8::try_from(pad).unwrap_or(16), pad));
        let mut previous = self.iv;
        for block in buf.as_chunks_mut::<16>().0 {
            for (b, p) in block.iter_mut().zip(previous) {
                *b ^= p;
            }
            cipher.encrypt_block(GenericArray::from_mut_slice(block));
            previous = *block;
        }
        buf
    }

    fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>, Refused> {
        if data.is_empty() || !data.len().is_multiple_of(16) {
            return Err(Refused::Decrypt);
        }
        let cipher = Aes128::new(GenericArray::from_slice(&self.key));
        let mut buf = data.to_vec();
        let mut previous = self.iv;
        for block in buf.as_chunks_mut::<16>().0 {
            let encrypted = *block;
            cipher.decrypt_block(GenericArray::from_mut_slice(block));
            for (b, p) in block.iter_mut().zip(previous) {
                *b ^= p;
            }
            previous = encrypted;
        }
        let pad = usize::from(*buf.last().ok_or(Refused::Decrypt)?);
        if pad == 0
            || pad > 16
            || buf[buf.len() - pad..]
                .iter()
                .any(|&b| usize::from(b) != pad)
        {
            return Err(Refused::Decrypt);
        }
        buf.truncate(buf.len() - pad);
        Ok(buf)
    }

    /// A request to `device` at `stamp`: the JSON, a `NUL` after it (as
    /// the devices expect), encrypted and signed.
    #[must_use]
    pub fn encode(&self, device: [u8; 4], stamp: u32, json: &[u8]) -> Vec<u8> {
        let mut plain = json.to_vec();
        plain.push(0);
        let data = self.encrypt(&plain);
        let length = u16::try_from(HEADER + data.len()).unwrap_or(u16::MAX);
        let mut packet = Vec::with_capacity(HEADER + data.len());
        packet.extend_from_slice(&MAGIC);
        packet.extend_from_slice(&length.to_be_bytes());
        packet.extend_from_slice(&[0; 4]);
        packet.extend_from_slice(&device);
        packet.extend_from_slice(&stamp.to_be_bytes());
        let checksum = md5(&[&packet[..16], &self.token, &data]);
        packet.extend_from_slice(&checksum);
        packet.extend_from_slice(&data);
        packet
    }

    /// An answer from `device`: checked (length, device, checksum), then
    /// decrypted, trailing `NUL`s removed. Returns the stamp and the JSON.
    pub fn decode(&self, device: [u8; 4], packet: &[u8]) -> Result<(u32, Vec<u8>), Refused> {
        if packet.len() <= HEADER || packet.len() > MAX_PACKET || packet[..2] != MAGIC {
            return Err(Refused::Malformed);
        }
        if usize::from(u16::from_be_bytes([packet[2], packet[3]])) != packet.len() {
            return Err(Refused::Malformed);
        }
        if packet[8..12] != device {
            return Err(Refused::OtherDevice);
        }
        let data = &packet[HEADER..];
        if md5(&[&packet[..16], &self.token, data])[..] != packet[16..32] {
            return Err(Refused::Checksum);
        }
        let stamp = u32::from_be_bytes([packet[12], packet[13], packet[14], packet[15]]);
        let mut plain = self.decrypt(data)?;
        while plain.last() == Some(&0) {
            plain.pop();
        }
        Ok((stamp, plain))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Built apart with OpenSSL (`aes-128-cbc`) and Python's `hashlib`:
    /// token 00…0f, device 01020304, stamp 100,
    /// `{"id":1,"method":"get_status","params":[]}` and its `NUL`.
    const VECTOR: &str = "2131005000000000010203040000006486eb35f1d75fadf7943718270493ca842c9dfc761522a115e1c2ce9e6e74852298529077f0e7a3646807304c72a9c33e3241789615c97ef9a47a639af66fec14";
    const REQUEST: &[u8] = br#"{"id":1,"method":"get_status","params":[]}"#;
    const DEVICE: [u8; 4] = [1, 2, 3, 4];

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    fn keys() -> Keys {
        Keys::new(token("000102030405060708090a0b0c0d0e0f").unwrap())
    }

    #[test]
    fn packets_match_an_independent_implementation() {
        let keys = keys();
        assert_eq!(keys.encode(DEVICE, 100, REQUEST), unhex(VECTOR));
        assert_eq!(
            keys.decode(DEVICE, &unhex(VECTOR)).unwrap(),
            (100, REQUEST.to_vec())
        );
        assert!(!format!("{keys:?}").contains("0001"), "never printed");
    }

    #[test]
    fn hostile_packets_are_refused_before_decrypting() {
        let keys = keys();
        let good = unhex(VECTOR);
        assert_eq!(keys.decode([9, 9, 9, 9], &good), Err(Refused::OtherDevice));
        let mut tampered = good.clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert_eq!(keys.decode(DEVICE, &tampered), Err(Refused::Checksum));
        let other = Keys::new([7; 16]);
        assert_eq!(other.decode(DEVICE, &good), Err(Refused::Checksum));
        assert_eq!(keys.decode(DEVICE, &good[..40]), Err(Refused::Malformed));
        assert_eq!(keys.decode(DEVICE, &hello()), Err(Refused::Malformed));
        assert_eq!(keys.decode(DEVICE, &[0; 64]), Err(Refused::Malformed));
    }

    #[test]
    fn hello_and_tokens() {
        let mut answer = hello();
        answer[4..16].copy_from_slice(&[0, 0, 0, 0, 1, 2, 3, 4, 0, 0, 0x30, 0x39]);
        assert_eq!(parse_hello(&answer), Some((DEVICE, 12345)));
        assert_eq!(parse_hello(&unhex(VECTOR)), None);
        assert_eq!(parse_hello(&answer[..31]), None);
        assert_eq!(token("FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"), Some([0xff; 16]));
        for bad in ["", "0011", &"g".repeat(32), &"0".repeat(33)] {
            assert_eq!(token(bad), None, "{bad:?}");
        }
    }
}
