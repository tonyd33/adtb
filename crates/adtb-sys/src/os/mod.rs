//! The runtime over the host: its clocks, threads, entropy and files.

mod faults;
mod files;
mod pool;
mod queue;
mod timer;

#[cfg(test)]
mod tests;

use std::cell::OnceCell;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock, RwLockWriteGuard};
use std::thread::{self, JoinHandle, Thread};
use std::time::{Duration, SystemTime};

pub use self::faults::Faults;
use self::files::Files;
use self::pool::Pool;
use self::queue::{Ring, Waiter};
use self::timer::Timers;
use crate::runtime::{
    Clock, Completion, Entropy, Instant, Io, Op, Runtime, ThreadId, Threads, Ticket,
};

/// How an [`OsRuntime`] performs I/O.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// A pool of this many threads, each running one blocking system call at a time.
    Pool { threads: usize },
}

/// The runtime over the host's clocks, threads, entropy and the files under one directory.
///
/// Every file is opened for direct I/O. Dropping the last handle and the last I/O queue stops
/// the backend's threads once the ops already submitted have completed.
#[derive(Clone)]
pub struct OsRuntime {
    threads: Arc<Registry>,
    backend: Arc<Shared>,
}

impl fmt::Debug for OsRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OsRuntime").finish_non_exhaustive()
    }
}

#[derive(Debug)]
struct Shared {
    files: Arc<Files>,
    pool: Arc<Pool>,
    timers: Arc<Timers>,
}

impl Drop for Shared {
    fn drop(&mut self) {
        self.pool.stop();
        self.timers.stop();
    }
}

impl OsRuntime {
    /// Returns a runtime whose paths are relative to the directory `root`.
    ///
    /// Panics if `backend` asks for no threads.
    pub fn new(root: impl Into<PathBuf>, backend: Backend) -> OsRuntime {
        OsRuntime::start(Files::new(root.into(), None), backend)
    }

    /// Returns a runtime whose paths are relative to the directory `root`, injecting `faults`
    /// into every op on its files.
    ///
    /// Panics if `backend` asks for no threads, or if the atomic unit is not a whole number of
    /// sectors.
    pub fn with_faults(root: impl Into<PathBuf>, backend: Backend, faults: Faults) -> OsRuntime {
        OsRuntime::start(Files::new(root.into(), Some(faults)), backend)
    }

    fn start(files: Files, backend: Backend) -> OsRuntime {
        let files = Arc::new(files);
        let Backend::Pool { threads } = backend;
        OsRuntime {
            threads: Arc::default(),
            backend: Arc::new(Shared {
                pool: Pool::start(Arc::clone(&files), threads),
                files,
                timers: Timers::start(),
            }),
        }
    }

    /// Stop the files as a power loss would.
    ///
    /// Postconditions:
    ///
    /// - With `lose_unsynced`, any subset of the writes no completed `Sync` covers is on the
    ///   device, each whole or, with `torn_p`, in any subset of its atomic units; any prefix of
    ///   the creations and removals no completed `SyncDir` covers is.
    /// - Every op that runs after it completes with `Failed`.
    ///
    /// Panics unless the runtime was made by [`OsRuntime::with_faults`].
    pub fn crash(&self) {
        self.backend.files.crash();
    }

    /// Inject `faults` from now on, keeping what earlier faults left.
    ///
    /// Panics unless the runtime was made by [`OsRuntime::with_faults`], or if the atomic unit
    /// is not a whole number of sectors.
    pub fn set_faults(&self, faults: Faults) {
        self.backend.files.set_faults(faults);
    }
}

fn clock(id: libc::clockid_t) -> Duration {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid, writable timespec for the call's duration.
    let r = unsafe { libc::clock_gettime(id, &raw mut ts) };
    assert_eq!(0, r, "the host refused to read clock {id}");
    let secs = u64::try_from(ts.tv_sec).expect("the clock reads after its origin");
    let nanos = u32::try_from(ts.tv_nsec).expect("a timespec's nanoseconds fit a u32");
    Duration::new(secs, nanos)
}

/// Returns the host's monotonic time, from its boot.
fn monotonic() -> Instant {
    let d = clock(libc::CLOCK_MONOTONIC);
    Instant::from_nanos(u64::try_from(d.as_nanos()).expect("instant overflows u64 nanoseconds"))
}

impl Clock for OsRuntime {
    fn now(&self) -> Instant {
        monotonic()
    }

    fn wall(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH + clock(libc::CLOCK_REALTIME)
    }
}

impl Entropy for OsRuntime {
    fn next_u64(&self) -> u64 {
        let mut word = [0; 8];
        self.fill(&mut word);
        u64::from_le_bytes(word)
    }

    fn fill(&self, bytes: &mut [u8]) {
        let mut at = 0;
        while at < bytes.len() {
            let rest = &mut bytes[at..];
            // SAFETY: `rest` is a valid, writable buffer of `rest.len()` bytes.
            let n = unsafe { libc::getrandom(rest.as_mut_ptr().cast(), rest.len(), 0) };
            match usize::try_from(n) {
                Ok(n) => at += n,
                Err(_) => {
                    let e = std::io::Error::last_os_error();
                    assert_eq!(
                        Some(libc::EINTR),
                        e.raw_os_error(),
                        "the host refused entropy: {e}"
                    );
                }
            }
        }
    }
}

/// Run `body`, aborting the process if it panics.
fn abort_on_panic(body: impl FnOnce()) {
    if panic::catch_unwind(AssertUnwindSafe(body)).is_err() {
        std::process::abort();
    }
}

static NEXT_THREAD: AtomicU64 = AtomicU64::new(0);

/// Every live thread that has an id, by id.
///
/// A thread's entry is inserted before its id is handed out and removed when it exits.
static LIVE: RwLock<BTreeMap<ThreadId, Thread>> = RwLock::new(BTreeMap::new());

fn live() -> RwLockWriteGuard<'static, BTreeMap<ThreadId, Thread>> {
    LIVE.write().unwrap_or_else(PoisonError::into_inner)
}

/// The calling thread's id, which removes the thread from [`LIVE`] when the thread exits.
struct Current(ThreadId);

impl Drop for Current {
    fn drop(&mut self) {
        live().remove(&self.0);
    }
}

thread_local! {
    static CURRENT: OnceCell<Current> = const { OnceCell::new() };
}

fn fresh_id() -> ThreadId {
    ThreadId(NEXT_THREAD.fetch_add(1, Ordering::Relaxed))
}

fn current_id() -> ThreadId {
    CURRENT.with(|c| {
        c.get_or_init(|| {
            let id = fresh_id();
            live().insert(id, thread::current());
            Current(id)
        })
        .0
    })
}

/// The threads a runtime spawned and has not joined, by id.
#[derive(Debug, Default)]
struct Registry(Mutex<HashMap<ThreadId, JoinHandle<()>>>);

impl Registry {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<ThreadId, JoinHandle<()>>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Threads for OsRuntime {
    fn spawn<F>(&self, name: &str, body: F) -> ThreadId
    where
        F: FnOnce() + Send + 'static,
    {
        let id = fresh_id();
        // The thread removes its entry under this lock when it exits.
        let mut live = live();
        #[expect(
            clippy::disallowed_methods,
            reason = "the real runtime's threads are host threads"
        )]
        let handle = thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                CURRENT.with(|c| {
                    assert!(
                        c.set(Current(id)).is_ok(),
                        "a new host thread has no id yet"
                    );
                });
                abort_on_panic(body);
            })
            .expect("the host refused a thread");
        live.insert(id, handle.thread().clone());
        drop(live);
        self.threads.lock().insert(id, handle);
        id
    }

    fn current(&self) -> ThreadId {
        current_id()
    }

    fn park(&self) {
        thread::park();
    }

    fn unpark(&self, thread: ThreadId) {
        assert!(
            thread.0 < NEXT_THREAD.load(Ordering::Relaxed),
            "{thread:?} was never handed out"
        );
        // An exited thread has no entry.
        if let Some(t) = LIVE
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&thread)
        {
            t.unpark();
        }
    }

    fn yield_now(&self) {
        thread::yield_now();
    }

    fn join(&self, thread: ThreadId) {
        let handle =
            self.threads.lock().remove(&thread).unwrap_or_else(|| {
                panic!("{thread:?} was not spawned by this runtime or is joined")
            });
        handle
            .join()
            .expect("a panicking thread aborts the process before it is joined");
    }
}

/// One I/O queue of an [`OsRuntime`].
#[derive(Debug)]
pub struct OsIo {
    backend: Arc<Shared>,
    ring: Arc<Ring>,
    next: u64,
}

impl Io for OsIo {
    fn submit(&mut self, op: Op) -> Ticket {
        let ticket = Ticket(self.next);
        self.next += 1;
        let waiter = Waiter {
            ring: Arc::clone(&self.ring),
            ticket,
            thread: thread::current(),
        };
        match op {
            Op::Timer { at } => self.backend.timers.add(at, waiter),
            op => self.backend.pool.push(op, waiter),
        }
        ticket
    }

    fn poll(&mut self) -> Option<Completion> {
        self.ring.pop()
    }
}

impl Runtime for OsRuntime {
    type Io = OsIo;

    fn io(&self) -> OsIo {
        OsIo {
            backend: Arc::clone(&self.backend),
            ring: Arc::default(),
            next: 0,
        }
    }
}
