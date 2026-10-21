//! Limited DNS decoding: packet offsets remain authoritative, never text pointers.
use std::collections::BTreeSet;
use std::ops::Range;

pub(super) struct Record {
    pub name: String,
    pub kind: u16,
    pub data: Range<usize>,
}
pub(super) fn word(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}
pub(super) fn name(bytes: &[u8], cursor: &mut usize) -> Option<String> {
    let mut at = *cursor;
    let mut consumed = None;
    let mut seen = BTreeSet::new();
    let mut parts = Vec::new();
    let mut length = 0;
    for _ in 0..64 {
        if !seen.insert(at) {
            return None;
        }
        let size = *bytes.get(at)?;
        at += 1;
        if size & 0xc0 == 0xc0 {
            let pointer = (usize::from(size & 0x3f) << 8) | usize::from(*bytes.get(at)?);
            if pointer >= at - 1 {
                return None;
            }
            consumed.get_or_insert(at + 1);
            at = pointer;
            continue;
        }
        if size & 0xc0 != 0 {
            return None;
        }
        if size == 0 {
            *cursor = consumed.unwrap_or(at);
            return Some(parts.join(".").to_ascii_lowercase());
        }
        let label = bytes.get(at..at + usize::from(size))?;
        if !label
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(*b, b'-' | b'_'))
        {
            return None;
        }
        length += usize::from(size) + 1;
        if length > 253 {
            return None;
        }
        parts.push(std::str::from_utf8(label).ok()?.to_owned());
        at += usize::from(size);
    }
    None
}
pub(super) fn records(bytes: &[u8]) -> Option<Vec<Record>> {
    if !(12..=9000).contains(&bytes.len()) {
        return None;
    }
    let flags = word(bytes, 2)?;
    if flags & 0x8000 == 0 || flags & 0x7a0f != 0 {
        return None;
    }
    let questions = word(bytes, 4)?;
    let count = word(bytes, 6)?
        .checked_add(word(bytes, 8)?)?
        .checked_add(word(bytes, 10)?)?;
    if questions > 16 || count > 128 {
        return None;
    }
    let mut cursor = 12;
    for _ in 0..questions {
        name(bytes, &mut cursor)?;
        cursor = cursor.checked_add(4)?;
        bytes.get(..cursor)?;
    }
    let mut result = Vec::new();
    for _ in 0..count {
        let owner = name(bytes, &mut cursor)?;
        let kind = word(bytes, cursor)?;
        let class = word(bytes, cursor + 2)? & 0x7fff;
        let ttl = u32::from_be_bytes(bytes.get(cursor + 4..cursor + 8)?.try_into().ok()?);
        let length = usize::from(word(bytes, cursor + 8)?);
        cursor += 10;
        let end = cursor.checked_add(length)?;
        bytes.get(cursor..end)?;
        if class == 1 && ttl > 0 {
            result.push(Record {
                name: owner,
                kind,
                data: cursor..end,
            });
        }
        cursor = end;
    }
    (cursor == bytes.len()).then_some(result)
}
