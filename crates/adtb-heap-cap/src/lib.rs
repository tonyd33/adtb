//! A global allocator over `System` that aborts the process once live heap passes a cap:
//! [`DEFAULT_CAP`], or the decimal byte count in `ADTB_TEST_HEAP_CAP`, which overrides everything.
//! Each allocation is charged its malloc chunk. Freed memory malloc retains, page memory and C
//! allocations are not counted.
//!
//! The cap bounds a whole nextest run: slots times [`DEFAULT_CAP`]. A test that needs more calls
//! [`raise_cap`] and declares `threads-required` in `.config/nextest.toml`: its cap divided by
//! [`DEFAULT_CAP`], rounded up, so it holds the slots its cap covers.
//!
//! Linking this crate installs the allocator, except under Miri. Test builds link it through the
//! `test-heap-cap` feature of `adtb-lang` or `adtb-sys`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::ffi::CStr;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

pub const DEFAULT_CAP: usize = 3 << 29;

pub const MAX_RAISED_CAP: usize = 4 * DEFAULT_CAP;

const UNREAD: usize = 0;

const CHUNK_HEADER: usize = 8;
const CHUNK_ALIGN: usize = 16;
const MIN_CHUNK: usize = 32;

#[cfg_attr(not(miri), global_allocator)]
static HEAP: HeapCap = HeapCap::new(UNREAD);

/// Raise this process's cap to `bytes`. A no-op under `ADTB_TEST_HEAP_CAP`; never lowers the cap.
///
/// Preconditions: the calling test has `threads-required` of `slots_for(bytes)` in
/// `.config/nextest.toml`, and calls this with a literal in the test function's own body.
///
/// # Panics
///
/// When `bytes` passes [`MAX_RAISED_CAP`].
pub fn raise_cap(bytes: usize) {
    if env_cap().is_none() {
        HEAP.raise(bytes);
    }
}

/// Returns the nextest slots a test with a cap of `bytes` holds.
#[must_use]
pub const fn slots_for(bytes: usize) -> usize {
    bytes.div_ceil(DEFAULT_CAP)
}

/// Returns what malloc takes for a block: the size plus its header, rounded up to the chunk
/// alignment and minimum chunk, plus the alignment when it passes malloc's own.
const fn chunk_cost(size: usize, align: usize) -> usize {
    let chunk = size.saturating_add(CHUNK_HEADER + CHUNK_ALIGN - 1) & !(CHUNK_ALIGN - 1);
    let chunk = if chunk < MIN_CHUNK { MIN_CHUNK } else { chunk };
    if align > CHUNK_ALIGN {
        chunk.saturating_add(align)
    } else {
        chunk
    }
}

const fn layout_cost(layout: Layout) -> usize {
    chunk_cost(layout.size(), layout.align())
}

#[derive(Debug)]
pub struct HeapCap {
    live: AtomicUsize,
    cap: AtomicUsize,
}

impl HeapCap {
    /// Returns an allocator with `cap` bytes, or the environment's cap read at first use when
    /// `cap` is zero.
    #[must_use]
    pub const fn new(cap: usize) -> Self {
        Self {
            live: AtomicUsize::new(0),
            cap: AtomicUsize::new(cap),
        }
    }

    /// Returns the bytes currently charged.
    pub fn live(&self) -> usize {
        self.live.load(Relaxed)
    }

    fn cap(&self) -> usize {
        match self.cap.load(Relaxed) {
            UNREAD => {
                let cap = cap_from_environment();
                self.cap.store(cap, Relaxed);
                cap
            }
            cap => cap,
        }
    }

    fn raise(&self, bytes: usize) {
        assert!(
            bytes <= MAX_RAISED_CAP,
            "a raised cap of {bytes} bytes passes the maximum of {MAX_RAISED_CAP}"
        );
        self.cap();
        self.cap.fetch_max(bytes, Relaxed);
    }

    fn charge(&self, size: usize) {
        let live = self.live.fetch_add(size, Relaxed) + size;
        let cap = self.cap();
        if live > cap {
            abort_over_cap(live, cap);
        }
    }

    fn refund(&self, size: usize) {
        self.live.fetch_sub(size, Relaxed);
    }
}

// SAFETY: every call forwards to `System` with the caller's arguments; the counters do not affect
// the returned memory.
unsafe impl GlobalAlloc for HeapCap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let cost = layout_cost(layout);
        self.charge(cost);
        // SAFETY: the caller's contract for `alloc` is `System`'s.
        let block = unsafe { System.alloc(layout) };
        if block.is_null() {
            self.refund(cost);
        }
        block
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let cost = layout_cost(layout);
        self.charge(cost);
        // SAFETY: the caller's contract for `alloc_zeroed` is `System`'s.
        let block = unsafe { System.alloc_zeroed(layout) };
        if block.is_null() {
            self.refund(cost);
        }
        block
    }

    unsafe fn dealloc(&self, block: *mut u8, layout: Layout) {
        // SAFETY: the caller's contract for `dealloc` is `System`'s.
        unsafe { System.dealloc(block, layout) };
        self.refund(layout_cost(layout));
    }

    unsafe fn realloc(&self, block: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let old_cost = layout_cost(layout);
        let new_cost = chunk_cost(new_size, layout.align());
        let grown = new_cost.saturating_sub(old_cost);
        self.charge(grown);
        // SAFETY: the caller's contract for `realloc` is `System`'s.
        let moved = unsafe { System.realloc(block, layout, new_size) };
        if moved.is_null() {
            self.refund(grown);
        } else {
            self.refund(old_cost.saturating_sub(new_cost));
        }
        moved
    }
}

fn cap_from_environment() -> usize {
    env_cap().unwrap_or(DEFAULT_CAP)
}

fn env_cap() -> Option<usize> {
    // SAFETY: the name is NUL terminated; no code in the test binaries calls `setenv` while
    // allocating threads run.
    let value = unsafe { libc::getenv(c"ADTB_TEST_HEAP_CAP".as_ptr()) };
    if value.is_null() {
        return None;
    }
    // SAFETY: a non-null `getenv` result is a NUL terminated string.
    let bytes = unsafe { CStr::from_ptr(value) }.to_bytes();
    parse_cap(bytes)
}

/// Returns the positive decimal number `bytes` spells.
fn parse_cap(bytes: &[u8]) -> Option<usize> {
    let mut cap = 0usize;
    for byte in bytes {
        let digit = byte.checked_sub(b'0').filter(|d| *d < 10)?;
        cap = cap.checked_mul(10)?.checked_add(usize::from(digit))?;
    }
    (cap != UNREAD).then_some(cap)
}

/// A line built on the stack.
struct Line {
    bytes: [u8; 128],
    len: usize,
}

impl Line {
    fn raw(&mut self, bytes: &[u8]) {
        let end = self.len + bytes.len();
        self.bytes[self.len..end].copy_from_slice(bytes);
        self.len = end;
    }

    fn text(&mut self, text: &str) {
        self.raw(text.as_bytes());
    }

    fn number(&mut self, mut n: usize) {
        let mut digits = [0u8; 20];
        let mut at = digits.len();
        loop {
            at -= 1;
            digits[at] = b'0' + (n % 10) as u8;
            n /= 10;
            if n == 0 {
                break;
            }
        }
        let end = self.len + digits.len() - at;
        self.bytes[self.len..end].copy_from_slice(&digits[at..]);
        self.len = end;
    }
}

fn abort_over_cap(live: usize, cap: usize) -> ! {
    let mut line = Line {
        bytes: [0; 128],
        len: 0,
    };
    line.text("adtb-heap-cap: thread ");
    let mut name: [libc::c_char; 16] = [0; 16];
    // SAFETY: the call takes no arguments and cannot fail.
    let me = unsafe { libc::pthread_self() };
    // SAFETY: the buffer holds the 16 bytes the length states; the call fills it NUL terminated
    // and does not allocate for the calling thread.
    let named = unsafe { libc::pthread_getname_np(me, name.as_mut_ptr(), name.len()) };
    if named == 0 {
        // SAFETY: a zero return leaves a NUL terminated name in the buffer.
        line.raw(unsafe { CStr::from_ptr(name.as_ptr()) }.to_bytes());
    }
    line.text(": ");
    line.number(live);
    line.text(" bytes of heap live, past the cap of ");
    line.number(cap);
    line.text("\n");
    // SAFETY: the pointer and length name initialised bytes of `line`.
    unsafe { libc::write(libc::STDERR_FILENO, line.bytes.as_ptr().cast(), line.len) };
    forgo_core_dump();
    std::process::abort()
}

/// Keep the abort from writing the process's heap to a core file.
fn forgo_core_dump() {
    #[cfg(target_os = "linux")]
    // SAFETY: `PR_SET_DUMPABLE` takes an integer and touches no memory of the caller.
    unsafe {
        libc::prctl(libc::PR_SET_DUMPABLE, 0);
    }
}

/// Returns the paths of the test binaries of the workspace of `manifest` that do not abort at a
/// one byte cap, building them first.
///
/// # Panics
///
/// When `cargo nextest list` fails or lists no binary.
#[must_use]
pub fn uncapped_test_binaries(manifest: &Path) -> Vec<String> {
    let listed = list_command(manifest).output().expect("cargo nextest runs");
    let stdout = String::from_utf8_lossy(&listed.stdout);
    assert!(
        listed.status.success(),
        "cargo nextest list fails: {}",
        String::from_utf8_lossy(&listed.stderr)
    );
    let binaries: Vec<&str> = stdout
        .split("\"binary-path\":\"")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .collect();
    assert!(
        !binaries.is_empty(),
        "cargo nextest lists no binary: {stdout}"
    );
    binaries
        .into_iter()
        .filter(|binary| !aborts_at_one_byte(binary))
        .map(str::to_owned)
        .collect()
}

fn list_command(manifest: &Path) -> Command {
    let mut list = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    list.args([
        "nextest",
        "list",
        "--workspace",
        "--list-type",
        "binaries-only",
        "--message-format",
        "json",
        "--cargo-quiet",
        "--manifest-path",
    ])
    .arg(manifest)
    .env_remove("NEXTEST_PROFILE");
    list
}

fn aborts_at_one_byte(binary: &str) -> bool {
    Command::new(binary)
        .arg("--list")
        .env("ADTB_TEST_HEAP_CAP", "1")
        .output()
        .expect("the test binary starts")
        .status
        .signal()
        == Some(libc::SIGABRT)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAYOUT: Layout = Layout::new::<[u8; 64]>();

    #[test]
    fn alloc_and_dealloc_balance() {
        let heap = HeapCap::new(1 << 20);
        // SAFETY: the layout has a non-zero size and the block is freed with it.
        let block = unsafe { heap.alloc(LAYOUT) };
        assert_eq!(heap.live(), 80);
        // SAFETY: `block` came from `heap` with `LAYOUT`.
        unsafe { heap.dealloc(block, LAYOUT) };
        assert_eq!(heap.live(), 0);
    }

    #[test]
    fn alloc_zeroed_charges_and_zeroes() {
        let heap = HeapCap::new(1 << 20);
        // SAFETY: the layout has a non-zero size and the block is freed with it.
        let block = unsafe { heap.alloc_zeroed(LAYOUT) };
        assert_eq!(heap.live(), 80);
        // SAFETY: `block` holds 64 initialised bytes.
        assert_eq!(unsafe { std::slice::from_raw_parts(block, 64) }, [0; 64]);
        // SAFETY: `block` came from `heap` with `LAYOUT`.
        unsafe { heap.dealloc(block, LAYOUT) };
        assert_eq!(heap.live(), 0);
    }

    #[test]
    fn realloc_charges_the_growth_and_refunds_the_shrink() {
        let heap = HeapCap::new(1 << 20);
        // SAFETY: the layout has a non-zero size.
        let block = unsafe { heap.alloc(LAYOUT) };
        // SAFETY: `block` came from `heap` with `LAYOUT`.
        let grown = unsafe { heap.realloc(block, LAYOUT, 4096) };
        assert_eq!(heap.live(), 4112);
        let large = Layout::from_size_align(4096, LAYOUT.align()).expect("a valid layout");
        // SAFETY: `grown` holds 4096 bytes under `large`.
        let shrunk = unsafe { heap.realloc(grown, large, 16) };
        assert_eq!(heap.live(), 32);
        let small = Layout::from_size_align(16, LAYOUT.align()).expect("a valid layout");
        // SAFETY: `shrunk` holds 16 bytes under `small`.
        unsafe { heap.dealloc(shrunk, small) };
        assert_eq!(heap.live(), 0);
    }

    #[test]
    fn a_chunk_costs_its_header_rounded_up_to_the_minimum_and_alignment() {
        assert_eq!(chunk_cost(1, 1), 32);
        assert_eq!(chunk_cost(8, 8), 32);
        assert_eq!(chunk_cost(24, 8), 32);
        assert_eq!(chunk_cost(25, 8), 48);
        assert_eq!(chunk_cost(40, 8), 48);
        assert_eq!(chunk_cost(41, 8), 64);
        assert_eq!(chunk_cost(4096, 16), 4112);
        assert_eq!(chunk_cost(64, 4096), 80 + 4096);
        assert_eq!(chunk_cost(usize::MAX, 1), usize::MAX & !15);
    }

    #[test]
    fn a_small_realloc_within_one_chunk_charges_nothing_new() {
        let heap = HeapCap::new(1 << 20);
        let one = Layout::from_size_align(1, 1).expect("a valid layout");
        // SAFETY: the layout has a non-zero size.
        let block = unsafe { heap.alloc(one) };
        assert_eq!(heap.live(), 32);
        // SAFETY: `block` came from `heap` with `one`.
        let same = unsafe { heap.realloc(block, one, 24) };
        assert_eq!(heap.live(), 32);
        let full = Layout::from_size_align(24, 1).expect("a valid layout");
        // SAFETY: `same` holds 24 bytes under `full`.
        unsafe { heap.dealloc(same, full) };
        assert_eq!(heap.live(), 0);
    }

    #[test]
    fn raising_lifts_the_cap_and_never_lowers_it() {
        let heap = HeapCap::new(1 << 20);
        heap.raise(1 << 30);
        assert_eq!(heap.cap(), 1 << 30);
        heap.raise(1 << 21);
        assert_eq!(heap.cap(), 1 << 30);
    }

    #[test]
    #[should_panic(expected = "passes the maximum")]
    fn raising_past_the_maximum_panics() {
        HeapCap::new(1 << 20).raise(MAX_RAISED_CAP + 1);
    }

    #[test]
    fn a_raised_test_takes_the_slots_its_cap_covers() {
        assert_eq!(slots_for(DEFAULT_CAP), 1);
        assert_eq!(slots_for(DEFAULT_CAP + 1), 2);
        assert_eq!(slots_for(MAX_RAISED_CAP), 4);
    }

    #[test]
    fn the_nested_list_drops_the_callers_profile() {
        let list = list_command(Path::new("Cargo.toml"));
        let profile = list.get_envs().find(|(name, _)| *name == "NEXTEST_PROFILE");
        assert_eq!(
            profile,
            Some((std::ffi::OsStr::new("NEXTEST_PROFILE"), None))
        );
    }

    #[test]
    fn a_failed_allocation_is_refunded() {
        let heap = HeapCap::new(usize::MAX);
        let huge = Layout::from_size_align(isize::MAX as usize - 4096, 1).expect("a valid layout");
        // SAFETY: the layout has a non-zero size; a null result needs no free.
        let block = unsafe { heap.alloc(huge) };
        assert!(std::hint::black_box(block).is_null());
        assert_eq!(heap.live(), 0);
    }

    #[test]
    fn caps_parse_as_positive_decimals() {
        assert_eq!(parse_cap(b"4096"), Some(4096));
        assert_eq!(parse_cap(b"0"), None);
        assert_eq!(parse_cap(b""), None);
        assert_eq!(parse_cap(b"6G"), None);
        assert_eq!(parse_cap(b"99999999999999999999999"), None);
    }
}
