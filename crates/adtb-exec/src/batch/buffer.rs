use std::fmt;
use std::ops::Range;

/// The alignment of every buffer's first byte and the multiple its padded length rounds up to.
pub const ALIGN: usize = 64;

/// A growable byte buffer in Arrow's layout.
///
/// Invariants:
/// - The first byte's address is a multiple of [`ALIGN`].
/// - The bytes from the length up to the next multiple of [`ALIGN`] are allocated and zero.
pub struct Buffer {
    bytes: Vec<u8>,
    start: usize,
    len: usize,
}

impl Buffer {
    pub fn new() -> Buffer {
        Buffer {
            bytes: Vec::new(),
            start: 0,
            len: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.bytes[self.start..self.start + self.len]
    }

    /// Returns the bytes followed by their zero padding up to a multiple of [`ALIGN`].
    pub fn padded(&self) -> &[u8] {
        &self.bytes[self.start..self.start + self.len.next_multiple_of(ALIGN)]
    }

    /// Returns the allocation and the range of it the buffer's bytes occupy.
    pub fn into_parts(self) -> (Vec<u8>, Range<usize>) {
        (self.bytes, self.start..self.start + self.len)
    }

    pub(super) fn extend(&mut self, bytes: &[u8]) {
        self.reserve(bytes.len());
        let at = self.start + self.len;
        self.bytes[at..at + bytes.len()].copy_from_slice(bytes);
        self.len += bytes.len();
    }

    pub(super) fn last_mut(&mut self) -> &mut u8 {
        &mut self.bytes[self.start + self.len - 1]
    }

    fn reserve(&mut self, extra: usize) {
        let need = (self.len + extra).next_multiple_of(ALIGN);
        if self.start + need <= self.bytes.len() {
            return;
        }
        let room = need.max(2 * self.len.next_multiple_of(ALIGN));
        let mut bytes = vec![0; room + ALIGN - 1];
        let addr = bytes.as_ptr().addr();
        let start = addr.next_multiple_of(ALIGN) - addr;
        bytes[start..start + self.len].copy_from_slice(self.as_slice());
        self.bytes = bytes;
        self.start = start;
    }
}

impl Default for Buffer {
    fn default() -> Buffer {
        Buffer::new()
    }
}

impl Clone for Buffer {
    fn clone(&self) -> Buffer {
        let mut b = Buffer::new();
        b.extend(self.as_slice());
        b
    }
}

impl PartialEq for Buffer {
    fn eq(&self, other: &Buffer) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl Eq for Buffer {}

impl fmt::Debug for Buffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Buffer").field("len", &self.len).finish()
    }
}
