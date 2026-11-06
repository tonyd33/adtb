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
    unhex_all(text)?.try_into().ok()
}

pub(super) fn unhex_all(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len() / 2)
        .map(|i| u8::from_str_radix(text.get(i * 2..i * 2 + 2)?, 16).ok())
        .collect()
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
