use std::ops::Add;
use std::time::{Duration, SystemTime};

pub use crate::buf::AlignedBuf;

/// A point on a process's monotonic clock, in nanoseconds from an origin the runtime picks.
///
/// An `Instant` is meaningful only within the process that read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Instant(u64);

impl Instant {
    pub const fn from_nanos(nanos: u64) -> Self {
        Instant(nanos)
    }

    pub const fn as_nanos(self) -> u64 {
        self.0
    }

    /// Returns the time from `earlier` to `self`, or zero if `earlier` is later.
    pub fn saturating_since(self, earlier: Instant) -> Duration {
        Duration::from_nanos(self.0.saturating_sub(earlier.0))
    }
}

impl Add<Duration> for Instant {
    type Output = Instant;

    fn add(self, d: Duration) -> Instant {
        let nanos = u64::try_from(d.as_nanos()).expect("instant overflows u64 nanoseconds");
        Instant(
            self.0
                .checked_add(nanos)
                .expect("instant overflows u64 nanoseconds"),
        )
    }
}

pub trait Clock {
    /// Returns the monotonic time. Successive calls within one process never go backwards.
    fn now(&self) -> Instant;

    /// Returns the wall-clock time, which may jump in either direction.
    fn wall(&self) -> SystemTime;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ThreadId(pub u64);

pub trait Threads {
    /// Start a thread running `body`.
    ///
    /// A panic in any thread crashes the process.
    fn spawn<F>(&self, name: &str, body: F) -> ThreadId
    where
        F: FnOnce() + Send + 'static;

    /// Start a thread that a process may end without.
    ///
    /// A simulator ends a run once only daemons are left and they are idle, and its timers neither
    /// keep a run alive nor stand in the way of reporting a deadlock.
    fn spawn_daemon<F>(&self, name: &str, body: F) -> ThreadId
    where
        F: FnOnce() + Send + 'static,
    {
        self.spawn(name, body)
    }

    /// Returns the calling thread's id.
    fn current(&self) -> ThreadId;

    /// Block until the calling thread's wake token is set, then clear it.
    ///
    /// May return spuriously; callers recheck their condition in a loop.
    fn park(&self);

    /// Set `thread`'s wake token, unblocking it if it is parked.
    fn unpark(&self, thread: ThreadId);

    /// Offer the processor to another thread.
    fn yield_now(&self);

    /// Block until `thread` has returned.
    fn join(&self, thread: ThreadId);
}

pub trait Entropy {
    fn next_u64(&self) -> u64;

    fn fill(&self, bytes: &mut [u8]) {
        for chunk in bytes.chunks_mut(8) {
            let word = self.next_u64().to_le_bytes();
            chunk.copy_from_slice(&word[..chunk.len()]);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ticket(pub u64);

/// A request to the I/O backend.
///
/// Paths are relative to the database directory and separated by `/`.
/// Offsets and lengths of `Read` and `Write` are multiples of the device's logical block size.
#[derive(Debug)]
pub enum Op {
    /// Open the file at `path`, creating it empty when `create` is set and it is absent.
    Open {
        path: String,
        create: bool,
    },
    /// Read up to `buf.len()` bytes at `offset`; a read past the end of the file is short.
    Read {
        file: FileId,
        offset: u64,
        buf: AlignedBuf,
    },
    /// Write all of `buf` at `offset`, extending the file if needed.
    Write {
        file: FileId,
        offset: u64,
        buf: AlignedBuf,
    },
    /// Returns the file's length, counting every write and allocation completed before this op
    /// was submitted.
    Size {
        file: FileId,
    },
    /// Extend the file to at least `len` bytes, the new bytes zero.
    Allocate {
        file: FileId,
        len: u64,
    },
    /// Force to the device every write to `file` completed before this op was submitted.
    Sync {
        file: FileId,
    },
    /// Force to the device every creation and removal of a file or directory in `dir` completed
    /// before this op was submitted.
    SyncDir {
        dir: String,
    },
    /// List the paths of the files in `dir`, in no particular order.
    List {
        dir: String,
    },
    Remove {
        path: String,
    },
    /// Create the directory `dir` if it is absent; its parent must exist.
    CreateDir {
        dir: String,
    },
    /// Release `file`. No op on it may be in flight, and its id is never used again.
    Close {
        file: FileId,
    },
    /// Complete once the clock has reached `at`.
    Timer {
        at: Instant,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub enum Done {
    Opened { file: FileId, len: u64 },
    Read(usize),
    Written,
    Size(u64),
    Allocated,
    Synced,
    Listed(Vec<String>),
    Removed,
    Created,
    Closed,
    Fired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IoError {
    NotFound,
    /// The device has no space for the op; a `Full` `Write` may have changed any of its blocks.
    Full,
    /// The device refused the op; a failed `Write` may have changed any of its blocks.
    Failed,
}

#[derive(Debug)]
pub struct Completion {
    pub ticket: Ticket,
    /// The buffer of a `Read` or `Write`, returned to the caller.
    pub buf: Option<AlignedBuf>,
    pub result: Result<Done, IoError>,
}

/// One submission and completion queue.
///
/// Postconditions:
///
/// - Every submitted op completes exactly once, in any order relative to other ops.
/// - The completion of an op unparks the thread that submitted it.
/// - A buffer handed over in an op is owned by the backend until its completion is polled.
pub trait Io: Send {
    fn submit(&mut self, op: Op) -> Ticket;

    /// Returns a completed op without blocking, if one is ready.
    fn poll(&mut self) -> Option<Completion>;
}

pub trait Runtime: Clock + Threads + Entropy + Clone + Send + Sync + 'static {
    type Io: Io;

    /// Returns a new, empty I/O queue.
    fn io(&self) -> Self::Io;
}
