#[cfg(all(test, adtb_loom))]
mod loom_tests;
#[cfg(not(all(test, adtb_loom)))]
mod region;
#[cfg(all(test, adtb_loom))]
mod region_loom;

use std::cell::{Cell, RefCell};
use std::fmt;
#[cfg(not(all(test, adtb_loom)))]
use std::ops::Deref;

#[cfg(not(all(test, adtb_loom)))]
use region::Region;
#[cfg(all(test, adtb_loom))]
use region_loom::Region;

use crate::runtime::Threads;
use crate::sync::{AtomicU64, Ordering, fence, thread_local};

const VERSION_BITS: u32 = 56;
const VERSION_MASK: u64 = (1 << VERSION_BITS) - 1;
const UNLOCKED: u64 = 0;
const MAX_SHARED: u64 = 252;
const EXCLUSIVE: u64 = 253;
const MARKED: u64 = 254;
const EVICTED: u64 = 255;

/// Stored words are the state XOR this, so zeroed memory reads as evicted at version 0.
const STORED_FLIP: u64 = EVICTED << VERSION_BITS;

const ALIGN: usize = 4096;

/// The latches the calling thread holds, by page memory and page, kept in debug builds and when
/// the simulator's tests enable `latch-checks`.
#[cfg(any(debug_assertions, feature = "latch-checks"))]
mod held {
    use std::cell::RefCell;

    use crate::sync::thread_local;

    thread_local! {
        static HELD: RefCell<Vec<(usize, u64, bool)>> = const { RefCell::new(Vec::new()) };
    }

    /// Panic if the calling thread already holds a latch on `pid`, exclusive only if
    /// `exclusive_only`.
    pub(super) fn refuse(mem: usize, pid: u64, exclusive_only: bool) {
        let held = HELD
            .try_with(|h| {
                h.borrow()
                    .iter()
                    .any(|&(m, p, x)| m == mem && p == pid && (x || !exclusive_only))
            })
            .unwrap_or(false);
        if held {
            panic!("page {pid} is already latched by this thread");
        }
    }

    pub(super) fn take(mem: usize, pid: u64, exclusive: bool) {
        HELD.try_with(|h| h.borrow_mut().push((mem, pid, exclusive)))
            .unwrap_or(());
    }

    pub(super) fn count(mem: usize) -> usize {
        HELD.try_with(|h| h.borrow().iter().filter(|&&(m, ..)| m == mem).count())
            .unwrap_or(0)
    }

    pub(super) fn release(mem: usize, pid: u64) {
        HELD.try_with(|h| {
            let mut h = h.borrow_mut();
            if let Some(i) = h.iter().position(|&(m, p, _)| m == mem && p == pid) {
                h.swap_remove(i);
            }
        })
        .unwrap_or(());
    }
}

#[cfg(not(any(debug_assertions, feature = "latch-checks")))]
mod held {
    pub(super) fn refuse(_: usize, _: u64, _: bool) {}
    pub(super) fn take(_: usize, _: u64, _: bool) {}
    pub(super) fn release(_: usize, _: u64) {}
    pub(super) fn count(_: usize) -> usize {
        0
    }
}

thread_local! {
    static SWITCH: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
}

thread_local! {
    /// The calling thread's buffer for [`Optimistic::copied`], out of the cell while in use so
    /// that a nested copy takes a fresh one.
    static COPY: Cell<Vec<u8>> = const { Cell::new(Vec::new()) };
}

/// Call `hook` at every switch point the calling thread passes in page memory, until the
/// returned guard drops: before it starts an optimistic read, takes a latch or upgrades to one,
/// halfway through an optimistic copy, and before it validates an optimistic read.
pub fn switch_points(hook: impl Fn() + 'static) -> SwitchPoints {
    let previous = SWITCH.with(|s| s.borrow_mut().replace(Box::new(hook)));
    SwitchPoints { previous }
}

/// The hook [`switch_points`] installed; dropping it restores the one before.
pub struct SwitchPoints {
    previous: Option<Box<dyn Fn()>>,
}

impl fmt::Debug for SwitchPoints {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SwitchPoints").finish_non_exhaustive()
    }
}

impl Drop for SwitchPoints {
    fn drop(&mut self) {
        let previous = self.previous.take();
        SWITCH
            .try_with(|s| *s.borrow_mut() = previous)
            .unwrap_or(());
    }
}

fn switch_point() {
    SWITCH
        .try_with(|s| {
            if let Some(hook) = &*s.borrow() {
                hook();
            }
        })
        .unwrap_or(());
}

fn word(tag: u64, version: u64) -> u64 {
    (tag << VERSION_BITS) | (version & VERSION_MASK)
}

fn tag(state: u64) -> u64 {
    state >> VERSION_BITS
}

fn version(state: u64) -> u64 {
    state & VERSION_MASK
}

/// A page's state as one load of its state word saw it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageState {
    Unlocked,
    Shared(u8),
    Exclusive,
    /// Unlocked, and passed over once by the replacement clock since it was last used.
    Marked,
    Evicted,
}

/// The memory of every page of the file at `pid * page_size`, reserved once, with one state
/// word per page.
///
/// Invariants:
///
/// - The memory is never unmapped while the value lives; eviction only releases what backs it.
/// - Page bytes change only under an exclusive latch, and only through atomic stores, so an
///   optimistic reader's atomic loads never race a plain write.
/// - A shared or exclusive latch keeps its page resident.
pub struct PageMemory {
    page_size: usize,
    pages: u64,
    data: Region,
    states: Region,
}

impl fmt::Debug for PageMemory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PageMemory")
            .field("page_size", &self.page_size)
            .field("pages", &self.pages)
            .finish_non_exhaustive()
    }
}

impl PageMemory {
    /// Reserve memory for `pages` pages of `page_size` bytes, every page evicted.
    ///
    /// Panics if `page_size` is not a power of two from 4096, or the range cannot be reserved.
    pub fn reserve(page_size: usize, pages: u64) -> PageMemory {
        assert!(
            page_size.is_power_of_two() && page_size >= ALIGN,
            "page size {page_size} is not a power of two from {ALIGN}"
        );
        assert!(pages > 0, "page memory holds at least one page");
        let len = usize::try_from(pages)
            .ok()
            .and_then(|p| p.checked_mul(page_size))
            .expect("page memory overflows the address space");
        let state_len = (pages as usize * 8).next_multiple_of(ALIGN);
        PageMemory {
            page_size,
            pages,
            data: Region::reserve(len),
            states: Region::reserve(state_len),
        }
    }

    pub fn page_size(&self) -> usize {
        self.page_size
    }

    pub fn pages(&self) -> u64 {
        self.pages
    }

    fn slot(&self, pid: u64) -> &AtomicU64 {
        assert!(pid < self.pages, "page {pid} is outside the reserved range");
        &self.states.words()[pid as usize]
    }

    fn load(&self, pid: u64, order: Ordering) -> u64 {
        self.slot(pid).load(order) ^ STORED_FLIP
    }

    fn swap_state(&self, pid: u64, from: u64, to: u64) -> bool {
        self.slot(pid)
            .compare_exchange(
                from ^ STORED_FLIP,
                to ^ STORED_FLIP,
                Ordering::AcqRel,
                Ordering::Relaxed,
            )
            .is_ok()
    }

    fn store_state(&self, pid: u64, to: u64) {
        self.slot(pid).store(to ^ STORED_FLIP, Ordering::Release);
    }

    fn words(&self, pid: u64) -> &[AtomicU64] {
        let per = self.page_size / 8;
        let first = pid as usize * per;
        &self.data.words()[first..first + per]
    }

    /// Returns the bytes of page `pid`.
    ///
    /// Preconditions: the caller holds a latch on the page, so no other thread writes it.
    #[cfg(not(all(test, adtb_loom)))]
    fn bytes(&self, pid: u64) -> &[u8] {
        let words = self.words(pid);
        // SAFETY: the words are page memory valid while `self` is; under the caller's latch
        // nothing stores to them, and concurrent atomic loads do not race plain loads.
        unsafe { std::slice::from_raw_parts(words.as_ptr().cast(), self.page_size) }
    }

    pub fn state(&self, pid: u64) -> (PageState, u64) {
        let s = self.load(pid, Ordering::Acquire);
        let state = match tag(s) {
            UNLOCKED => PageState::Unlocked,
            EXCLUSIVE => PageState::Exclusive,
            MARKED => PageState::Marked,
            EVICTED => PageState::Evicted,
            n => PageState::Shared(n as u8),
        };
        (state, version(s))
    }

    /// Returns the number of latches the calling thread holds here, in debug builds; 0 otherwise.
    pub fn latched_here(&self) -> usize {
        held::count(self.id())
    }

    fn id(&self) -> usize {
        self as *const PageMemory as usize
    }

    fn exclusive_from(&self, pid: u64, s: u64) -> Option<Exclusive<'_>> {
        self.swap_state(pid, s, word(EXCLUSIVE, version(s)))
            .then(|| {
                fence(Ordering::Release);
                held::take(self.id(), pid, true);
                Exclusive {
                    mem: self,
                    pid,
                    version: version(s),
                    resident: tag(s) != EVICTED,
                    done: false,
                }
            })
    }

    /// Take the exclusive latch on `pid`, waiting through `threads` while another holds a latch.
    ///
    /// An evicted page is latched too; [`Exclusive::resident`] then says it must be filled.
    ///
    /// Panics, in debug builds, if the calling thread holds a latch on `pid` already.
    pub fn lock_exclusive(&self, pid: u64, threads: &impl Threads) -> Exclusive<'_> {
        held::refuse(self.id(), pid, false);
        switch_point();
        loop {
            let s = self.load(pid, Ordering::Acquire);
            if matches!(tag(s), UNLOCKED | MARKED | EVICTED)
                && let Some(g) = self.exclusive_from(pid, s)
            {
                return g;
            }
            threads.yield_now();
        }
    }

    /// Take the exclusive latch on resident, unlatched `pid` if no one holds a latch on it.
    pub fn try_lock_exclusive(&self, pid: u64) -> Option<Exclusive<'_>> {
        switch_point();
        let s = self.load(pid, Ordering::Acquire);
        match tag(s) {
            UNLOCKED | MARKED => self.exclusive_from(pid, s),
            _ => None,
        }
    }

    /// Take a shared latch on `pid`, waiting through `threads` while it is exclusive or its
    /// count is full. Returns `None` if the page is evicted.
    ///
    /// Panics, in debug builds, if the calling thread holds a latch on `pid` already.
    pub fn lock_shared(&self, pid: u64, threads: &impl Threads) -> Option<Shared<'_>> {
        held::refuse(self.id(), pid, false);
        switch_point();
        loop {
            let s = self.load(pid, Ordering::Acquire);
            let next = match tag(s) {
                UNLOCKED | MARKED => 1,
                EVICTED => return None,
                n if n < MAX_SHARED => n + 1,
                _ => {
                    threads.yield_now();
                    continue;
                }
            };
            if self.swap_state(pid, s, word(next, version(s))) {
                held::take(self.id(), pid, false);
                return Some(Shared { mem: self, pid });
            }
        }
    }

    /// Start an optimistic read of `pid`. Returns `None` while it is exclusive or evicted.
    ///
    /// Panics, in debug builds, if the calling thread holds the exclusive latch on `pid`.
    pub fn optimistic(&self, pid: u64) -> Option<Optimistic<'_>> {
        held::refuse(self.id(), pid, true);
        switch_point();
        let s = self.load(pid, Ordering::Acquire);
        match tag(s) {
            EXCLUSIVE | EVICTED => None,
            t => {
                if t == MARKED {
                    self.swap_state(pid, s, word(UNLOCKED, version(s)));
                }
                Some(Optimistic {
                    mem: self,
                    pid,
                    version: version(s),
                })
            }
        }
    }

    /// Mark `pid` for the replacement clock if it is unlatched. Returns whether it was marked
    /// already.
    pub fn mark(&self, pid: u64) -> bool {
        let s = self.load(pid, Ordering::Acquire);
        match tag(s) {
            MARKED => true,
            UNLOCKED => {
                self.swap_state(pid, s, word(MARKED, version(s)));
                false
            }
            _ => false,
        }
    }
}

/// The exclusive latch on one page: no other latch is held on it, and only this guard changes it.
pub struct Exclusive<'a> {
    mem: &'a PageMemory,
    pid: u64,
    version: u64,
    resident: bool,
    done: bool,
}

impl fmt::Debug for Exclusive<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Exclusive")
            .field("pid", &self.pid)
            .field("resident", &self.resident)
            .finish()
    }
}

impl Exclusive<'_> {
    pub fn pid(&self) -> u64 {
        self.pid
    }

    /// Returns whether the page holds contents; an evicted page reads as zeros until filled.
    pub fn resident(&self) -> bool {
        self.resident
    }

    /// Write `bytes` at `offset` of the page through atomic stores, making it resident.
    ///
    /// Panics if the bytes run past the end of the page.
    pub fn write(&mut self, offset: usize, bytes: &[u8]) {
        let end = offset + bytes.len();
        assert!(end <= self.mem.page_size, "write past the end of the page");
        let words = self.mem.words(self.pid);
        let mut at = offset;
        while at < end {
            let w = at / 8;
            let lo = w * 8;
            let from = at - lo;
            let n = (8 - from).min(end - at);
            let src = &bytes[at - offset..at - offset + n];
            let value = match <[u8; 8]>::try_from(src) {
                Ok(whole) => whole,
                Err(_) => {
                    let mut buf = words[w].load(Ordering::Relaxed).to_le_bytes();
                    buf[from..from + n].copy_from_slice(src);
                    buf
                }
            };
            words[w].store(u64::from_le_bytes(value), Ordering::Relaxed);
            #[cfg(all(test, adtb_loom))]
            self.mem
                .data
                .written(self.pid as usize * (self.mem.page_size / 8) + w);
            at += n;
        }
        self.resident = true;
    }

    /// Release the memory behind the page and leave it evicted.
    pub fn evict(mut self) {
        let size = self.mem.page_size;
        self.mem.data.release(self.pid as usize * size, size);
        self.mem
            .store_state(self.pid, word(EVICTED, self.version + 1));
        held::release(self.mem.id(), self.pid);
        self.done = true;
    }
}

impl<'a> Exclusive<'a> {
    /// Exchange the exclusive latch for a shared one, with no other writer in between.
    ///
    /// Panics if the page is not resident.
    pub fn share(mut self) -> Shared<'a> {
        assert!(self.resident, "a shared latch needs a resident page");
        self.mem.store_state(self.pid, word(1, self.version + 1));
        held::release(self.mem.id(), self.pid);
        held::take(self.mem.id(), self.pid, false);
        self.done = true;
        Shared {
            mem: self.mem,
            pid: self.pid,
        }
    }
}

#[cfg(not(all(test, adtb_loom)))]
impl Deref for Exclusive<'_> {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        self.mem.bytes(self.pid)
    }
}

impl Drop for Exclusive<'_> {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        let tag = if self.resident { UNLOCKED } else { EVICTED };
        self.mem.store_state(self.pid, word(tag, self.version + 1));
        held::release(self.mem.id(), self.pid);
    }
}

/// A shared latch on one page: the page is resident and no one changes it.
pub struct Shared<'a> {
    mem: &'a PageMemory,
    pid: u64,
}

impl fmt::Debug for Shared<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Shared").field("pid", &self.pid).finish()
    }
}

impl Shared<'_> {
    pub fn pid(&self) -> u64 {
        self.pid
    }
}

#[cfg(not(all(test, adtb_loom)))]
impl Deref for Shared<'_> {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        self.mem.bytes(self.pid)
    }
}

impl Drop for Shared<'_> {
    fn drop(&mut self) {
        loop {
            let s = self.mem.load(self.pid, Ordering::Relaxed);
            let n = tag(s);
            debug_assert!(
                (1..=MAX_SHARED).contains(&n),
                "shared release of state {s:#x}"
            );
            let next = if n == 1 { UNLOCKED } else { n - 1 };
            if self.mem.swap_state(self.pid, s, word(next, version(s))) {
                held::release(self.mem.id(), self.pid);
                return;
            }
        }
    }
}

/// An optimistic read of one page: copies are trusted only once [`Optimistic::validate`] holds.
pub struct Optimistic<'a> {
    mem: &'a PageMemory,
    pid: u64,
    version: u64,
}

impl fmt::Debug for Optimistic<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Optimistic")
            .field("pid", &self.pid)
            .field("version", &self.version)
            .finish()
    }
}

impl<'a> Optimistic<'a> {
    pub fn pid(&self) -> u64 {
        self.pid
    }

    /// Copy the page's bytes from `offset` into `out` through atomic loads.
    ///
    /// Panics if the range runs past the end of the page.
    pub fn read(&self, offset: usize, out: &mut [u8]) {
        assert!(
            offset + out.len() <= self.mem.page_size,
            "read past the end of the page"
        );
        let (first, second) = out.split_at_mut(out.len() / 2);
        self.copy(offset, first);
        switch_point();
        self.copy(offset + first.len(), second);
    }

    /// Returns what `f` computes from a copy of `len` bytes of the page from `offset`, taken as
    /// [`Optimistic::read`] takes it.
    ///
    /// The copy may be torn: `f` bounds-checks what it reads, and the caller uses its result only
    /// once [`Optimistic::validate`] holds.
    ///
    /// Panic if the range runs past the end of the page.
    pub fn copied<T>(&self, offset: usize, len: usize, f: impl FnOnce(&[u8]) -> T) -> T {
        let mut copy = COPY.try_with(Cell::take).unwrap_or_default();
        copy.resize(len, 0);
        self.read(offset, &mut copy);
        let out = f(&copy);
        COPY.try_with(|c| c.set(copy)).unwrap_or(());
        out
    }

    fn copy(&self, offset: usize, out: &mut [u8]) {
        let end = offset + out.len();
        let words = self.mem.words(self.pid);
        if offset.is_multiple_of(8) && out.len().is_multiple_of(8) {
            for (chunk, w) in out
                .as_chunks_mut::<8>()
                .0
                .iter_mut()
                .zip(&words[offset / 8..])
            {
                *chunk = w.load(Ordering::Relaxed).to_le_bytes();
            }
            return;
        }
        let mut at = offset;
        while at < end {
            let w = at / 8;
            let buf = words[w].load(Ordering::Relaxed).to_le_bytes();
            let from = at - w * 8;
            let n = (8 - from).min(end - at);
            out[at - offset..at - offset + n].copy_from_slice(&buf[from..from + n]);
            at += n;
        }
    }

    /// Returns whether no exclusive latch was taken, and no eviction happened, since the read
    /// began.
    pub fn validate(&self) -> bool {
        switch_point();
        fence(Ordering::Acquire);
        let s = self.mem.load(self.pid, Ordering::Relaxed);
        !matches!(tag(s), EXCLUSIVE | EVICTED) && version(s) == self.version
    }

    /// Take the exclusive latch if the page is unlatched and unchanged since the read began.
    pub fn upgrade(self) -> Option<Exclusive<'a>> {
        switch_point();
        for t in [UNLOCKED, MARKED] {
            if let Some(g) = self.mem.exclusive_from(self.pid, word(t, self.version)) {
                return Some(g);
            }
        }
        None
    }
}

#[cfg(all(test, not(adtb_loom)))]
mod tests {
    use super::{PageMemory, PageState};
    use crate::runtime::{ThreadId, Threads};

    /// A thread interface for one thread, which never has to wait.
    struct Alone;

    impl Threads for Alone {
        fn spawn<F: FnOnce() + Send + 'static>(&self, _: &str, _: F) -> ThreadId {
            unreachable!()
        }
        fn current(&self) -> ThreadId {
            ThreadId(0)
        }
        fn park(&self) {}
        fn unpark(&self, _: ThreadId) {}
        fn yield_now(&self) {
            panic!("a lone thread waited on a latch");
        }
        fn join(&self, _: ThreadId) {}
    }

    /// A thread interface whose waits yield the host thread.
    struct Spin;

    impl Threads for Spin {
        fn spawn<F: FnOnce() + Send + 'static>(&self, _: &str, _: F) -> ThreadId {
            unreachable!()
        }
        fn current(&self) -> ThreadId {
            unreachable!()
        }
        fn park(&self) {
            unreachable!()
        }
        fn unpark(&self, _: ThreadId) {
            unreachable!()
        }
        fn yield_now(&self) {
            std::thread::yield_now();
        }
        fn join(&self, _: ThreadId) {
            unreachable!()
        }
    }

    fn memory() -> PageMemory {
        PageMemory::reserve(4096, 16)
    }

    #[test]
    fn pages_start_evicted() {
        let m = memory();
        assert_eq!((PageState::Evicted, 0), m.state(3));
        assert!(m.lock_shared(3, &Alone).is_none());
        assert!(m.optimistic(3).is_none());
        assert!(m.try_lock_exclusive(3).is_none());
    }

    #[test]
    fn an_exclusive_latch_fills_a_page_and_bumps_the_version() {
        let m = memory();
        let mut g = m.lock_exclusive(3, &Alone);
        assert!(!g.resident());
        g.write(4093, &[1, 2, 3]);
        g.write(5, &[9; 20]);
        assert!(g.resident());
        drop(g);
        assert_eq!((PageState::Unlocked, 1), m.state(3));
        let s = m.lock_shared(3, &Alone).unwrap();
        assert_eq!([1, 2, 3], s[4093..]);
        assert_eq!([9; 20], s[5..25]);
        assert_eq!(0, s[4]);
    }

    #[test]
    fn an_unfilled_page_stays_evicted() {
        let m = memory();
        drop(m.lock_exclusive(2, &Alone));
        assert_eq!((PageState::Evicted, 1), m.state(2));
    }

    #[test]
    fn writers_shared_latches_and_optimistic_reads_share_a_page_across_threads() {
        let m = memory();
        m.lock_exclusive(1, &Alone).write(0, &[0; 64]);
        std::thread::scope(|s| {
            s.spawn(|| {
                for fill in 1..=3 {
                    m.lock_exclusive(1, &Spin).write(0, &[fill; 64]);
                }
            });
            s.spawn(|| {
                for _ in 0..3 {
                    let g = m.lock_shared(1, &Spin).unwrap();
                    assert!(
                        g[..64].iter().all(|&b| b == g[0]),
                        "a shared latch saw a torn page"
                    );
                }
            });
            s.spawn(|| {
                for _ in 0..3 {
                    if let Some(r) = m.optimistic(1) {
                        let mut out = [9; 64];
                        r.read(0, &mut out);
                        assert!(
                            !r.validate() || out.iter().all(|&b| b == out[0]),
                            "a validated read saw a torn page"
                        );
                    }
                }
            });
        });
        assert_eq!([3; 64], m.lock_shared(1, &Alone).unwrap()[..64]);
    }

    #[test]
    fn shared_latches_count_and_exclude_writers() {
        let m = memory();
        m.lock_exclusive(1, &Alone).write(0, &[7]);
        let a = m.lock_shared(1, &Alone).unwrap();
        std::thread::scope(|s| {
            s.spawn(|| {
                let b = m.lock_shared(1, &Alone).unwrap();
                assert_eq!(PageState::Shared(2), m.state(1).0);
                assert!(m.try_lock_exclusive(1).is_none());
                drop(b);
            });
        });
        assert_eq!(PageState::Shared(1), m.state(1).0);
        drop(a);
        assert_eq!(PageState::Unlocked, m.state(1).0);
        assert!(m.try_lock_exclusive(1).is_some());
    }

    #[test]
    fn an_optimistic_read_fails_validation_after_a_writer() {
        let m = memory();
        m.lock_exclusive(1, &Alone).write(0, &[7; 8]);
        let r = m.optimistic(1).unwrap();
        let mut out = [0; 4];
        r.read(2, &mut out);
        assert_eq!([7; 4], out);
        assert!(r.validate());
        let shared = m.lock_shared(1, &Alone).unwrap();
        assert!(r.validate(), "a shared latch changes nothing");
        drop(shared);
        m.lock_exclusive(1, &Alone).write(0, &[8]);
        assert!(!r.validate());
        assert!(r.upgrade().is_none());
        let fresh = m.optimistic(1).unwrap();
        let g = fresh.upgrade().expect("unchanged since the read began");
        assert_eq!(8, g[0]);
    }

    #[test]
    fn eviction_zeroes_the_page_and_fails_readers() {
        let m = memory();
        m.lock_exclusive(5, &Alone).write(100, &[1; 100]);
        let r = m.optimistic(5).unwrap();
        m.try_lock_exclusive(5).unwrap().evict();
        assert_eq!(PageState::Evicted, m.state(5).0);
        assert!(!r.validate());
        let g = m.lock_exclusive(5, &Alone);
        assert!(!g.resident());
        assert!(g.iter().all(|&b| b == 0));
    }

    #[test]
    fn the_clock_marks_unlatched_pages_and_use_clears_the_mark() {
        let m = memory();
        m.lock_exclusive(4, &Alone).write(0, &[1]);
        assert!(!m.mark(4));
        assert!(m.mark(4));
        assert_eq!(PageState::Marked, m.state(4).0);
        let r = m.optimistic(4).unwrap();
        assert_eq!(PageState::Unlocked, m.state(4).0);
        assert!(r.validate());
    }

    #[test]
    fn share_keeps_the_page_and_admits_readers() {
        let m = memory();
        let mut g = m.lock_exclusive(6, &Alone);
        g.write(0, &[3]);
        let s = g.share();
        assert_eq!(PageState::Shared(1), m.state(6).0);
        assert_eq!(3, s[0]);
        assert!(m.optimistic(6).is_some());
    }

    #[test]
    #[should_panic(expected = "page 2 is already latched by this thread")]
    fn a_second_shared_latch_on_one_thread_panics() {
        let m = memory();
        m.lock_exclusive(2, &Alone).write(0, &[1]);
        let _a = m.lock_shared(2, &Alone).unwrap();
        m.lock_shared(2, &Alone);
    }

    #[test]
    #[should_panic(expected = "page 2 is already latched by this thread")]
    fn an_exclusive_latch_over_a_shared_one_panics() {
        let m = memory();
        m.lock_exclusive(2, &Alone).write(0, &[1]);
        let _a = m.lock_shared(2, &Alone).unwrap();
        m.lock_exclusive(2, &Alone);
    }

    #[test]
    #[should_panic(expected = "page 2 is already latched by this thread")]
    fn an_optimistic_read_under_an_exclusive_latch_panics() {
        let m = memory();
        let _g = m.lock_exclusive(2, &Alone);
        m.optimistic(2);
    }

    #[test]
    fn released_latches_are_forgotten() {
        let m = memory();
        let mut g = m.lock_exclusive(2, &Alone);
        g.write(0, &[1]);
        let s = g.share();
        assert!(
            m.optimistic(2).is_some(),
            "an optimistic read beside a shared latch is fine"
        );
        drop(s);
        m.try_lock_exclusive(2).unwrap().evict();
        drop(m.lock_exclusive(2, &Alone));
        let other = memory();
        let _held = m.lock_exclusive(3, &Alone);
        drop(other.lock_exclusive(3, &Alone));
    }

    #[test]
    fn switch_points_call_the_hook_until_the_guard_drops() {
        use std::cell::Cell;
        use std::rc::Rc;

        let m = memory();
        m.lock_exclusive(1, &Alone).write(0, &[1]);
        let calls = Rc::new(Cell::new(0));
        let c = calls.clone();
        let guard = super::switch_points(move || c.set(c.get() + 1));
        drop(m.lock_shared(1, &Alone));
        let r = m.optimistic(1).unwrap();
        r.read(0, &mut [0; 64]);
        assert!(r.validate());
        assert!(r.upgrade().is_some());
        assert_eq!(5, calls.get());
        drop(guard);
        drop(m.lock_exclusive(1, &Alone));
        assert_eq!(5, calls.get());
    }

    /// Returns a page whose byte at each offset is that offset, truncated.
    fn counting() -> PageMemory {
        let m = memory();
        let bytes: Vec<u8> = (0..4096).map(|i| i as u8).collect();
        m.lock_exclusive(1, &Alone).write(0, &bytes);
        m
    }

    #[test]
    fn a_read_returns_the_bytes_of_any_range() {
        let m = counting();
        let r = m.optimistic(1).unwrap();
        for offset in [0, 1, 7, 8, 9, 16, 2040, 4080, 4095] {
            for len in [0, 1, 7, 8, 9, 16, 17, 32, 4096 - offset] {
                if offset + len > 4096 {
                    continue;
                }
                let mut out = vec![0xAA; len];
                r.read(offset, &mut out);
                let want: Vec<u8> = (offset..offset + len).map(|i| i as u8).collect();
                assert_eq!(want, out, "offset {offset}, length {len}");
            }
        }
    }

    #[test]
    fn a_read_leaves_the_bytes_around_its_range() {
        let m = counting();
        let r = m.optimistic(1).unwrap();
        let mut out = [0xAA; 32];
        r.read(8, &mut out[8..24]);
        assert_eq!([0xAA; 8], out[..8]);
        assert_eq!([0xAA; 8], out[24..]);
    }

    #[test]
    fn a_copy_holds_the_range_it_asked_for() {
        let m = counting();
        let r = m.optimistic(1).unwrap();
        for (offset, len) in [(0, 4096), (8, 16), (3, 5), (0, 0), (4088, 8)] {
            let want: Vec<u8> = (offset..offset + len).map(|i| i as u8).collect();
            r.copied(offset, len, |copy| {
                assert_eq!(want, copy, "offset {offset}, length {len}");
            });
        }
    }

    #[test]
    fn a_copy_holds_nothing_of_the_copy_before_it() {
        let m = memory();
        m.lock_exclusive(1, &Alone).write(0, &[1; 4096]);
        m.lock_exclusive(2, &Alone).write(0, &[2; 4096]);
        m.optimistic(1)
            .unwrap()
            .copied(0, 4096, |copy| assert_eq!([1; 4096], copy));
        m.optimistic(2)
            .unwrap()
            .copied(0, 4096, |copy| assert_eq!([2; 4096], copy));
        m.optimistic(1).unwrap().copied(0, 16, |copy| {
            assert_eq!([1; 16], copy, "a shorter copy has its own length");
        });
        m.optimistic(2)
            .unwrap()
            .copied(0, 4096, |copy| assert_eq!([2; 4096], copy));
    }

    #[test]
    fn a_copy_nested_in_a_copy_keeps_each_its_bytes() {
        let m = memory();
        m.lock_exclusive(1, &Alone).write(0, &[1; 64]);
        m.lock_exclusive(2, &Alone).write(0, &[2; 64]);
        let (outer, inner) = (m.optimistic(1).unwrap(), m.optimistic(2).unwrap());
        outer.copied(0, 64, |a| {
            inner.copied(0, 64, |b| assert_eq!([2; 64], b));
            assert_eq!([1; 64], a);
        });
    }

    #[test]
    fn a_panic_in_a_copy_leaves_later_copies_whole() {
        let m = memory();
        m.lock_exclusive(1, &Alone).write(0, &[1; 64]);
        let r = m.optimistic(1).unwrap();
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            r.copied(0, 64, |_| panic!("the closure gave up"));
        }));
        assert!(panicked.is_err());
        r.copied(0, 64, |copy| assert_eq!([1; 64], copy));
    }

    #[test]
    fn a_copy_passes_the_switch_point_of_a_read() {
        use std::cell::Cell;
        use std::rc::Rc;

        let m = memory();
        m.lock_exclusive(1, &Alone).write(0, &[1]);
        let r = m.optimistic(1).unwrap();
        let calls = Rc::new(Cell::new(0));
        let c = calls.clone();
        let guard = super::switch_points(move || c.set(c.get() + 1));
        r.copied(0, 64, |_| {});
        drop(guard);
        assert_eq!(1, calls.get());
    }

    #[test]
    #[should_panic(expected = "read past the end of the page")]
    fn a_copy_past_the_end_of_the_page_is_refused() {
        let m = memory();
        m.lock_exclusive(1, &Alone).write(0, &[1]);
        m.optimistic(1).unwrap().copied(8, 4096, |_| {});
    }

    #[test]
    #[should_panic(expected = "outside the reserved range")]
    fn pages_past_the_range_are_refused() {
        memory().state(16);
    }
}
