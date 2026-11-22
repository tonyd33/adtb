use super::codec::{DecodeError, Reader};

const ONE_BYTE_LIMIT: u32 = 240;

/// Append `n` as an order-preserving, prefix-free varint.
pub fn encode(n: u32, out: &mut Vec<u8>) {
    match u8::try_from(n) {
        Ok(b) if u32::from(b) < ONE_BYTE_LIMIT => out.push(b),
        _ => {
            let be = n.to_be_bytes();
            let skip = n.leading_zeros() as usize / 8;
            out.push(239 + (4 - skip) as u8);
            out.extend_from_slice(&be[skip..]);
        }
    }
}

pub(super) fn decode(r: &mut Reader) -> Result<u32, DecodeError> {
    let marker = r.byte()?;
    if u32::from(marker) < ONE_BYTE_LIMIT {
        return Ok(u32::from(marker));
    }
    let len = usize::from(marker - 239);
    if len > 4 {
        return Err(DecodeError::BadVarint);
    }
    let mut be = [0; 4];
    for b in &mut be[4 - len..] {
        *b = r.byte()?;
    }
    let n = u32::from_be_bytes(be);
    let fewest = if len == 1 {
        n >= ONE_BYTE_LIMIT
    } else {
        be[4 - len] != 0
    };
    if fewest {
        Ok(n)
    } else {
        Err(DecodeError::BadVarint)
    }
}
