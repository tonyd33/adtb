//! Hex text of bytes, and text sealed by a closing line holding its hash.

use std::fmt::Write as _;

pub(super) fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        write!(out, "{b:02x}").expect("a string takes any text");
    }
    out
}

pub(super) fn unhex<const N: usize>(text: &str) -> Option<[u8; N]> {
    let mut out = [0; N];
    if text.len() != N * 2 {
        return None;
    }
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(text.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

/// Returns `body` followed by a line holding the hash of `body`.
pub(super) fn sealed(mut body: String) -> String {
    let sum = hex(blake3::hash(body.as_bytes()).as_bytes());
    writeln!(body, "checksum {sum}").expect("a string takes any text");
    body
}

/// Returns what `text` says before its checksum line, when that line holds its hash.
pub(super) fn unsealed(text: &str) -> Option<&str> {
    let body = text.strip_suffix('\n')?;
    let at = body.rfind('\n')? + 1;
    let sum = body[at..].strip_prefix("checksum ")?;
    let lines = &text[..at];
    (hex(blake3::hash(lines.as_bytes()).as_bytes()) == sum).then_some(lines)
}
