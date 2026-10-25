use std::alloc::{self, Layout};
use std::fmt;
use std::ops::{Deref, DerefMut};
use std::ptr::NonNull;

/// A zero-initialised byte buffer whose address and length suit direct I/O.
///
/// Invariants:
///
/// - The address is a multiple of [`AlignedBuf::ALIGN`].
/// - The length is fixed at allocation and is never zero.
pub struct AlignedBuf {
    ptr: NonNull<u8>,
    len: usize,
}

// SAFETY: the buffer owns its allocation exclusively, like a `Box<[u8]>`.
unsafe impl Send for AlignedBuf {}
// SAFETY: shared access only reads through `&[u8]`.
unsafe impl Sync for AlignedBuf {}

impl AlignedBuf {
    pub const ALIGN: usize = 4096;

    /// Returns a buffer of `len` zero bytes.
    ///
    /// Panics if `len` is zero or the allocation fails.
    pub fn zeroed(len: usize) -> Self {
        assert!(len > 0, "an I/O buffer is never empty");
        let layout = Self::layout(len);
        // SAFETY: `layout` has a nonzero size.
        let raw = unsafe { alloc::alloc_zeroed(layout) };
        let Some(ptr) = NonNull::new(raw) else {
            alloc::handle_alloc_error(layout)
        };
        AlignedBuf { ptr, len }
    }

    /// Returns a buffer holding a copy of `bytes`.
    pub fn from_slice(bytes: &[u8]) -> Self {
        let mut buf = Self::zeroed(bytes.len());
        buf.copy_from_slice(bytes);
        buf
    }

    fn layout(len: usize) -> Layout {
        Layout::from_size_align(len, Self::ALIGN).expect("I/O buffer length overflows a layout")
    }
}

impl Deref for AlignedBuf {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        // SAFETY: `ptr` points to `len` initialised bytes owned by `self`.
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }
}

impl DerefMut for AlignedBuf {
    fn deref_mut(&mut self) -> &mut [u8] {
        // SAFETY: `ptr` points to `len` initialised bytes owned exclusively by `self`.
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len) }
    }
}

impl Drop for AlignedBuf {
    fn drop(&mut self) {
        // SAFETY: `ptr` came from `alloc_zeroed` with this same layout.
        unsafe { alloc::dealloc(self.ptr.as_ptr(), Self::layout(self.len)) }
    }
}

impl fmt::Debug for AlignedBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AlignedBuf")
            .field("len", &self.len)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::AlignedBuf;

    #[test]
    fn zeroed_is_aligned_and_zero() {
        let buf = AlignedBuf::zeroed(512);
        assert_eq!(buf.as_ptr() as usize % AlignedBuf::ALIGN, 0);
        assert_eq!(512, buf.len());
        assert!(buf.iter().all(|&b| b == 0));
    }

    #[test]
    fn from_slice_copies() {
        let buf = AlignedBuf::from_slice(&[1, 2, 3]);
        assert_eq!(&[1, 2, 3], &buf[..]);
    }

    #[test]
    #[should_panic(expected = "never empty")]
    fn empty_is_refused() {
        AlignedBuf::zeroed(0);
    }
}
