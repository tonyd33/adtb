#[cfg(not(all(target_os = "linux", not(miri))))]
use std::alloc::{self, Layout};
use std::ptr::NonNull;

#[cfg(not(all(target_os = "linux", not(miri))))]
use super::ALIGN;
use crate::sync::AtomicU64;
#[cfg(not(all(target_os = "linux", not(miri))))]
use crate::sync::Ordering;

/// A zeroed range of memory reserved once and released only when dropped.
pub(super) struct Region {
    ptr: NonNull<u8>,
    len: usize,
}

impl Region {
    #[cfg(all(target_os = "linux", not(miri)))]
    pub(super) fn reserve(len: usize) -> Region {
        // SAFETY: an anonymous private mapping at an address the kernel picks touches no
        // existing memory.
        let raw = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_NORESERVE,
                -1,
                0,
            )
        };
        assert!(
            raw != libc::MAP_FAILED,
            "the kernel refused to reserve {len} bytes of page memory"
        );
        Region {
            ptr: NonNull::new(raw.cast()).expect("mmap returned null"),
            len,
        }
    }

    #[cfg(not(all(target_os = "linux", not(miri))))]
    pub(super) fn reserve(len: usize) -> Region {
        let layout = Layout::from_size_align(len, ALIGN).expect("page memory overflows a layout");
        // SAFETY: `layout` has a nonzero size.
        let raw = unsafe { alloc::alloc_zeroed(layout) };
        let Some(ptr) = NonNull::new(raw) else {
            alloc::handle_alloc_error(layout)
        };
        Region { ptr, len }
    }

    /// Returns the region as words.
    pub(super) fn words(&self) -> &[AtomicU64] {
        // SAFETY: the region is `len` zeroed bytes aligned to `ALIGN`, valid while `self` is,
        // and `AtomicU64` has the size and alignment of `u64`; atomics permit shared mutation.
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr().cast(), self.len / 8) }
    }

    /// Give the physical memory behind `offset..offset + len` back; it reads as zeros after.
    pub(super) fn release(&self, offset: usize, len: usize) {
        #[cfg(all(target_os = "linux", not(miri)))]
        {
            // SAFETY: `offset` lies in the mapping.
            let start = unsafe { self.ptr.as_ptr().add(offset) };
            // SAFETY: `start..start + len` lies in the mapping; MADV_DONTNEED on a private
            // anonymous mapping only replaces its contents with zeros.
            let r = unsafe { libc::madvise(start.cast(), len, libc::MADV_DONTNEED) };
            assert_eq!(0, r, "madvise refused to release page memory");
        }
        #[cfg(not(all(target_os = "linux", not(miri))))]
        for w in &self.words()[offset / 8..(offset + len) / 8] {
            w.store(0, Ordering::Relaxed);
        }
    }
}

impl Drop for Region {
    fn drop(&mut self) {
        #[cfg(all(target_os = "linux", not(miri)))]
        // SAFETY: the mapping was made by `reserve` with this length and is unmapped once.
        unsafe {
            libc::munmap(self.ptr.as_ptr().cast(), self.len);
        }
        #[cfg(not(all(target_os = "linux", not(miri))))]
        // SAFETY: `ptr` came from `alloc_zeroed` with this layout.
        unsafe {
            alloc::dealloc(
                self.ptr.as_ptr(),
                Layout::from_size_align(self.len, ALIGN).unwrap(),
            )
        }
    }
}

// SAFETY: the region is plain memory owned by the value; every shared access goes through
// atomics or through the latch protocol of `PageMemory`.
unsafe impl Send for Region {}
// SAFETY: as for `Send`.
unsafe impl Sync for Region {}
