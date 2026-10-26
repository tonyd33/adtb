//! The runtime over the host: its clocks, threads, entropy and files.

mod files;
mod pool;
mod queue;
mod timer;

use std::cell::Cell;
use std::collections::HashMap;
use std::fmt;
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle, Thread};
use std::time::{Duration, SystemTime};

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
        let files = Arc::new(Files::new(root.into()));
        let Backend::Pool { threads } = backend;
        OsRuntime {
            threads: Arc::default(),
            backend: Arc::new(Shared {
                pool: Pool::start(files, threads),
                timers: Timers::start(),
            }),
        }
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

thread_local! {
    static CURRENT: Cell<Option<ThreadId>> = const { Cell::new(None) };
}

fn current_id() -> ThreadId {
    CURRENT.with(|c| match c.get() {
        Some(id) => id,
        None => {
            let id = ThreadId(NEXT_THREAD.fetch_add(1, Ordering::Relaxed));
            c.set(Some(id));
            id
        }
    })
}

/// The threads a runtime has seen, by id.
#[derive(Debug, Default)]
struct Registry(Mutex<HashMap<ThreadId, Entry>>);

#[derive(Debug)]
struct Entry {
    thread: Thread,
    handle: Option<JoinHandle<()>>,
}

impl Registry {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<ThreadId, Entry>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Threads for OsRuntime {
    fn spawn<F>(&self, name: &str, body: F) -> ThreadId
    where
        F: FnOnce() + Send + 'static,
    {
        let id = ThreadId(NEXT_THREAD.fetch_add(1, Ordering::Relaxed));
        #[expect(
            clippy::disallowed_methods,
            reason = "the real runtime's threads are host threads"
        )]
        let handle = thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                CURRENT.with(|c| c.set(Some(id)));
                abort_on_panic(body);
            })
            .expect("the host refused a thread");
        self.threads.lock().insert(
            id,
            Entry {
                thread: handle.thread().clone(),
                handle: Some(handle),
            },
        );
        id
    }

    fn current(&self) -> ThreadId {
        let id = current_id();
        self.threads.lock().entry(id).or_insert_with(|| Entry {
            thread: thread::current(),
            handle: None,
        });
        id
    }

    fn park(&self) {
        thread::park();
    }

    fn unpark(&self, thread: ThreadId) {
        self.threads
            .lock()
            .get(&thread)
            .unwrap_or_else(|| panic!("{thread:?} was neither spawned nor seen by this runtime"))
            .thread
            .unpark();
    }

    fn yield_now(&self) {
        thread::yield_now();
    }

    fn join(&self, thread: ThreadId) {
        let handle = self
            .threads
            .lock()
            .remove(&thread)
            .and_then(|e| e.handle)
            .unwrap_or_else(|| panic!("{thread:?} was not spawned by this runtime or is joined"));
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
