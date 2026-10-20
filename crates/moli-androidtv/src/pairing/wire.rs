//! Bounded Polo frames; an exchange owns one absolute four-second deadline.

use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use tokio::time::{Instant, timeout_at};

use super::{PairingError, STEP};
use crate::proto::{Writer, parse, varint};

const MAX_FRAME: usize = 64 * 1024;

pub(super) async fn exchange<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    field: u32,
    body: Writer,
    expected: u32,
    human_deadline: Instant,
) -> Result<Vec<u8>, PairingError> {
    let deadline = human_deadline.min(Instant::now() + STEP);
    if Instant::now() >= deadline {
        return Err(PairingError::Expired);
    }
    timeout_at(deadline, async {
        let frame = Writer::new()
            .uint(1, 2)
            .uint(2, 200)
            .msg(field, body)
            .finish();
        let mut packet = Vec::new();
        varint(frame.len() as u64, &mut packet);
        packet.extend_from_slice(&frame);
        stream
            .write_all(&packet)
            .await
            .map_err(|_| PairingError::Disconnected)?;
        stream
            .flush()
            .await
            .map_err(|_| PairingError::Disconnected)?;
        let frame = read(stream).await?;
        let outer = parse(&frame).ok_or(PairingError::Protocol)?;
        if outer.uint(1) != Some(2) || outer.uint(2).is_none() {
            return Err(PairingError::Protocol);
        }
        if outer.uint(2) != Some(200) {
            return Err(PairingError::Refused);
        }
        let bodies = [10, 11, 20, 30, 31, 40, 41];
        if bodies.into_iter().filter(|key| outer.has(*key)).count() != 1
            || outer.all_bytes(expected).count() != 1
            || outer.msg(expected).is_none()
        {
            return Err(PairingError::Protocol);
        }
        Ok(outer
            .bytes(expected)
            .ok_or(PairingError::Protocol)?
            .to_vec())
    })
    .await
    .map_err(|_| PairingError::Expired)?
}

async fn read<S: AsyncRead + Unpin>(stream: &mut S) -> Result<Vec<u8>, PairingError> {
    let mut length = 0u32;
    for shift in (0..35).step_by(7) {
        let byte = stream
            .read_u8()
            .await
            .map_err(|_| PairingError::Disconnected)?;
        if shift == 28 && byte & 0xf0 != 0 {
            return Err(PairingError::Protocol);
        }
        length |= u32::from(byte & 127) << shift;
        if length as usize > MAX_FRAME {
            return Err(PairingError::Protocol);
        }
        if byte & 128 == 0 {
            if length == 0 {
                return Err(PairingError::Protocol);
            }
            let mut frame = vec![0; length as usize];
            stream
                .read_exact(&mut frame)
                .await
                .map_err(|_| PairingError::Disconnected)?;
            return Ok(frame);
        }
    }
    Err(PairingError::Protocol)
}
