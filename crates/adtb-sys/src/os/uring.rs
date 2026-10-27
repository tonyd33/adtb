use std::collections::VecDeque;
use std::ffi::{CString, OsStr};
use std::fmt;
use std::fs::File;
use std::io::{self, ErrorKind};
use std::mem::MaybeUninit;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use io_uring::{IoUring, Probe, opcode, squeue, types};

use super::abort_on_panic;
use super::files::{self, Files, Outcome};
use super::pool::Pool;
use super::queue::Waiter;
use crate::runtime::{AlignedBuf, Done, IoError, Op};

/// Why the host refused an io_uring backend.
#[derive(Debug)]
pub enum Unavailable {
    /// Setting up or registering the ring failed, as on a kernel without io_uring or under a
    /// sandbox that denies `io_uring_setup`.
    Refused(io::Error),
    /// The kernel's io_uring lacks an operation the backend runs.
    Missing(&'static str),
}

impl fmt::Display for Unavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unavailable::Refused(e) => write!(f, "the host refused io_uring: {e}"),
            Unavailable::Missing(op) => write!(f, "the host's io_uring has no {op}"),
        }
    }
}

impl std::error::Error for Unavailable {}

const NEEDED: [(u8, &str); 8] = [
    (opcode::OpenAt::CODE, "openat"),
    (opcode::Read::CODE, "read"),
    (opcode::Write::CODE, "write"),
    (opcode::Statx::CODE, "statx"),
    (opcode::Fallocate::CODE, "fallocate"),
    (opcode::Fsync::CODE, "fsync"),
    (opcode::UnlinkAt::CODE, "unlinkat"),
    (opcode::MkDirAt::CODE, "mkdirat"),
];

/// File operations submitted to one io_uring, completed by one reaping thread.
///
/// `List`, which io_uring has no operation for, runs on a one-thread pool.
pub(crate) struct Uring {
    ring: IoUring,
    events: OwnedFd,
    state: Mutex<State>,
    files: Arc<Files>,
    pool: Arc<Pool>,
}

impl fmt::Debug for Uring {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Uring").finish_non_exhaustive()
    }
}

/// The ops submitted and not yet completed.
///
/// Invariant: an op in the ring keeps its slot, and the slot owns every buffer, path and file
/// the kernel reads or writes for it, until its completion is reaped.
#[derive(Default)]
struct State {
    slots: Vec<Option<Pending>>,
    free: Vec<usize>,
    waiting: VecDeque<usize>,
    in_ring: usize,
    stop: bool,
}

struct Pending {
    work: Work,
    waiter: Waiter,
}

enum Work {
    Open {
        path: CString,
        create: bool,
    },
    Read {
        file: Arc<File>,
        offset: u64,
        buf: AlignedBuf,
    },
    Write {
        file: Arc<File>,
        offset: u64,
        buf: AlignedBuf,
        written: usize,
    },
    Size {
        file: Arc<File>,
        stat: Box<MaybeUninit<libc::statx>>,
    },
    Allocate {
        file: Arc<File>,
        len: u64,
    },
    Sync {
        file: Arc<File>,
    },
    OpenDir {
        path: CString,
    },
    SyncDir {
        dir: File,
    },
    Remove {
        path: CString,
    },
    CreateDir {
        path: CString,
    },
}

enum Step {
    Complete(Outcome),
    Again(Work),
}

fn c_path(path: &Path) -> CString {
    CString::new(path.as_os_str().as_bytes()).expect("a database path has no NUL byte")
}

fn buf_len(len: usize) -> u32 {
    u32::try_from(len).expect("an I/O buffer is under 4 GiB")
}

pub(super) fn failure(errno: i32) -> IoError {
    match errno {
        libc::ENOENT => IoError::NotFound,
        libc::ENOSPC | libc::EDQUOT => IoError::Full,
        _ => IoError::Failed,
    }
}

fn retried(errno: i32) -> bool {
    errno == libc::EINTR || errno == libc::EAGAIN
}

/// Returns the open file the kernel handed back as descriptor `fd`.
fn adopt(fd: i32) -> File {
    // SAFETY: a successful openat completion returns a new descriptor no one else owns.
    unsafe { File::from_raw_fd(fd) }
}

fn sync_entry(fd: RawFd, flags: types::FsyncFlags) -> squeue::Entry {
    opcode::Fsync::new(types::Fd(fd)).flags(flags).build()
}

impl Work {
    fn entry(&mut self) -> squeue::Entry {
        let cwd = types::Fd(libc::AT_FDCWD);
        match self {
            Work::Open { path, create } => {
                let create = if *create { libc::O_CREAT } else { 0 };
                opcode::OpenAt::new(cwd, path.as_ptr())
                    .flags(libc::O_RDWR | libc::O_DIRECT | libc::O_CLOEXEC | create)
                    .mode(0o666)
                    .build()
            }
            Work::Read { file, offset, buf } => opcode::Read::new(
                types::Fd(file.as_raw_fd()),
                buf.as_mut_ptr(),
                buf_len(buf.len()),
            )
            .offset(*offset)
            .build(),
            Work::Write {
                file,
                offset,
                buf,
                written,
            } => {
                let rest = &buf[*written..];
                opcode::Write::new(
                    types::Fd(file.as_raw_fd()),
                    rest.as_ptr(),
                    buf_len(rest.len()),
                )
                .offset(*offset + *written as u64)
                .build()
            }
            Work::Size { file, stat } => opcode::Statx::new(
                types::Fd(file.as_raw_fd()),
                c"".as_ptr(),
                stat.as_mut_ptr().cast(),
            )
            .flags(libc::AT_EMPTY_PATH)
            .mask(libc::STATX_SIZE)
            .build(),
            Work::Allocate { file, len } => {
                opcode::Fallocate::new(types::Fd(file.as_raw_fd()), *len).build()
            }
            Work::Sync { file } => sync_entry(file.as_raw_fd(), types::FsyncFlags::DATASYNC),
            Work::OpenDir { path } => opcode::OpenAt::new(cwd, path.as_ptr())
                .flags(libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC)
                .build(),
            Work::SyncDir { dir } => sync_entry(dir.as_raw_fd(), types::FsyncFlags::empty()),
            Work::Remove { path } => opcode::UnlinkAt::new(cwd, path.as_ptr()).build(),
            Work::CreateDir { path } => {
                opcode::MkDirAt::new(cwd, path.as_ptr()).mode(0o777).build()
            }
        }
    }

    /// Returns what the op does next, given the result `res` of its latest entry.
    fn finish(self, files: &Files, res: i32) -> Step {
        if res < 0 && retried(-res) {
            return Step::Again(self);
        }
        let ok = |d: Done| Step::Complete((None, Ok(d)));
        let Ok(n) = u32::try_from(res) else {
            return self.failed(-res);
        };
        match self {
            Work::Open { .. } => {
                let file = adopt(i32::try_from(n).expect("a descriptor fits an i32"));
                match file.metadata() {
                    Ok(m) => ok(Done::Opened {
                        len: m.len(),
                        file: files.insert(file),
                    }),
                    Err(_) => Step::Complete((None, Err(IoError::Failed))),
                }
            }
            Work::Read { buf, .. } => Step::Complete((Some(buf), Ok(Done::Read(n as usize)))),
            Work::Write {
                file,
                offset,
                buf,
                written,
            } => {
                let written = written + n as usize;
                if n == 0 {
                    Step::Complete((Some(buf), Err(IoError::Failed)))
                } else if written < buf.len() {
                    Step::Again(Work::Write {
                        file,
                        offset,
                        buf,
                        written,
                    })
                } else {
                    Step::Complete((Some(buf), Ok(Done::Written)))
                }
            }
            Work::Size { stat, .. } => {
                // SAFETY: a successful statx completion has filled the buffer.
                let stat = unsafe { stat.assume_init() };
                ok(Done::Size(stat.stx_size))
            }
            Work::Allocate { .. } => ok(Done::Allocated),
            Work::OpenDir { .. } => Step::Again(Work::SyncDir {
                dir: adopt(i32::try_from(n).expect("a descriptor fits an i32")),
            }),
            Work::Sync { .. } | Work::SyncDir { .. } => ok(Done::Synced),
            Work::Remove { .. } => ok(Done::Removed),
            Work::CreateDir { .. } => ok(Done::Created),
        }
    }

    fn failed(self, errno: i32) -> Step {
        let fail = |buf| Step::Complete((buf, Err(failure(errno))));
        match self {
            Work::Read { buf, .. } | Work::Write { buf, .. } => fail(Some(buf)),
            Work::Allocate { file, len } if errno == libc::EOPNOTSUPP => {
                let r = files::allocate(&file, len).map(|()| Done::Allocated);
                Step::Complete((None, r.map_err(|e| files::error(&e))))
            }
            Work::CreateDir { path } if errno == libc::EEXIST => {
                let is_dir = Path::new(OsStr::from_bytes(path.as_bytes())).is_dir();
                Step::Complete((
                    None,
                    if is_dir {
                        Ok(Done::Created)
                    } else {
                        Err(IoError::Failed)
                    },
                ))
            }
            _ => fail(None),
        }
    }
}

fn eventfd() -> io::Result<OwnedFd> {
    // SAFETY: eventfd takes no pointers.
    let fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a successful eventfd returns a new descriptor no one else owns.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

impl Uring {
    /// Returns a ring of `entries` submission entries over `files`, with its reaping thread
    /// started.
    ///
    /// Panics if `entries` is zero.
    pub(crate) fn start(files: Arc<Files>, entries: u32) -> Result<Arc<Uring>, Unavailable> {
        assert!(entries > 0, "an io_uring has at least one entry");
        let ring = IoUring::new(entries).map_err(Unavailable::Refused)?;
        let mut probe = Probe::new();
        ring.submitter()
            .register_probe(&mut probe)
            .map_err(Unavailable::Refused)?;
        if let Some((_, name)) = NEEDED.iter().find(|(op, _)| !probe.is_supported(*op)) {
            return Err(Unavailable::Missing(name));
        }
        let events = eventfd().map_err(Unavailable::Refused)?;
        ring.submitter()
            .register_eventfd(events.as_raw_fd())
            .map_err(Unavailable::Refused)?;
        let uring = Arc::new(Uring {
            ring,
            events,
            state: Mutex::default(),
            pool: Pool::start(Arc::clone(&files), 1),
            files,
        });
        let run = Arc::clone(&uring);
        #[expect(
            clippy::disallowed_methods,
            reason = "the real runtime's I/O threads are host threads"
        )]
        std::thread::Builder::new()
            .name("adtb-uring".to_owned())
            .spawn(move || abort_on_panic(|| run.reap()))
            .expect("the host refused an I/O thread");
        Ok(uring)
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Start `op`, completing it through `waiter`.
    ///
    /// Panics on a `Timer`, which no file operation completes.
    pub(crate) fn push(&self, op: Op, waiter: Waiter) {
        let path = |p: &str| c_path(&self.files.path(p));
        let work = match op {
            Op::Open { path: p, create } => Work::Open {
                path: path(&p),
                create,
            },
            Op::Read { file, offset, buf } => Work::Read {
                file: self.files.file(file),
                offset,
                buf,
            },
            Op::Write { file, offset, buf } => Work::Write {
                file: self.files.file(file),
                offset,
                buf,
                written: 0,
            },
            Op::Size { file } => Work::Size {
                file: self.files.file(file),
                stat: Box::new(MaybeUninit::uninit()),
            },
            Op::Allocate { file, len } => Work::Allocate {
                file: self.files.file(file),
                len,
            },
            Op::Sync { file } => Work::Sync {
                file: self.files.file(file),
            },
            Op::SyncDir { dir } => Work::OpenDir { path: path(&dir) },
            Op::Remove { path: p } => Work::Remove { path: path(&p) },
            Op::CreateDir { dir } => Work::CreateDir { path: path(&dir) },
            op @ Op::List { .. } => return self.pool.push(op, waiter),
            op @ Op::Close { .. } => {
                let (buf, result) = self.files.perform(op);
                return waiter.complete(buf, result);
            }
            Op::Timer { .. } => unreachable!("a timer is not a file operation"),
        };
        let mut st = self.lock();
        let slot = st.insert(Pending { work, waiter });
        self.enqueue(&mut st, slot);
    }

    fn enqueue(&self, st: &mut State, slot: usize) {
        st.waiting.push_back(slot);
        self.refill(st);
    }

    fn capacity(&self) -> usize {
        self.ring.params().sq_entries() as usize
    }

    /// Submit `entry` to the kernel.
    ///
    /// Preconditions: the caller holds the state lock, and fewer than `capacity` entries are in
    /// the ring, so the submission queue has room.
    fn submit(&self, st: &mut State, entry: &squeue::Entry) {
        {
            // SAFETY: the state lock, held by the caller, is the only path to the submission
            // queue.
            let mut sq = unsafe { self.ring.submission_shared() };
            // SAFETY: the entry's slot owns everything it points to until its completion is
            // reaped.
            let pushed = unsafe { sq.push(entry) };
            pushed.expect("the submission queue has room for every entry in the ring");
        }
        st.in_ring += 1;
        loop {
            match self.ring.submit() {
                Ok(_) => return,
                Err(e)
                    if matches!(
                        e.raw_os_error(),
                        Some(libc::EINTR | libc::EAGAIN | libc::EBUSY)
                    ) =>
                {
                    std::thread::yield_now();
                }
                Err(e) => panic!("the kernel refused a submission to a ring with room: {e}"),
            }
        }
    }

    /// Stop the reaping thread once every op already pushed has completed.
    pub(crate) fn stop(&self) {
        self.lock().stop = true;
        self.pool.stop();
        self.signal();
    }

    fn signal(&self) {
        let one = 1u64.to_ne_bytes();
        // SAFETY: `one` is a valid buffer of 8 bytes for the call's duration.
        let n = unsafe { libc::write(self.events.as_raw_fd(), one.as_ptr().cast(), one.len()) };
        assert_eq!(
            Ok(one.len()),
            usize::try_from(n),
            "the host refused to signal an eventfd"
        );
    }

    fn wait(&self) {
        let mut count = [0u8; 8];
        loop {
            // SAFETY: `count` is a valid, writable buffer of 8 bytes for the call's duration.
            let n = unsafe {
                libc::read(
                    self.events.as_raw_fd(),
                    count.as_mut_ptr().cast(),
                    count.len(),
                )
            };
            if n >= 0 {
                return;
            }
            let e = io::Error::last_os_error();
            assert_eq!(
                ErrorKind::Interrupted,
                e.kind(),
                "the host refused to read an eventfd: {e}"
            );
        }
    }

    fn reap(&self) {
        loop {
            self.wait();
            // SAFETY: this thread is the only one that reads the completion queue.
            let done: Vec<(u64, i32)> = unsafe { self.ring.completion_shared() }
                .map(|c| (c.user_data(), c.result()))
                .collect();
            for (slot, res) in done {
                self.complete(
                    usize::try_from(slot).expect("a slot index fits a usize"),
                    res,
                );
            }
            let st = self.lock();
            if st.stop && st.in_ring == 0 && st.waiting.is_empty() {
                return;
            }
        }
    }

    fn complete(&self, slot: usize, res: i32) {
        let Pending { work, waiter } = {
            let mut st = self.lock();
            st.in_ring -= 1;
            st.slots[slot]
                .take()
                .expect("a completion's slot holds the op it completes")
        };
        match work.finish(&self.files, res) {
            Step::Again(work) => {
                let mut st = self.lock();
                st.slots[slot] = Some(Pending { work, waiter });
                self.enqueue(&mut st, slot);
            }
            Step::Complete((buf, result)) => {
                let mut st = self.lock();
                st.free.push(slot);
                self.refill(&mut st);
                drop(st);
                waiter.complete(buf, result);
            }
        }
    }

    fn refill(&self, st: &mut State) {
        while st.in_ring < self.capacity() {
            let Some(slot) = st.waiting.pop_front() else {
                return;
            };
            let entry = st.entry(slot);
            self.submit(st, &entry);
        }
    }
}

impl State {
    fn insert(&mut self, pending: Pending) -> usize {
        match self.free.pop() {
            Some(slot) => {
                self.slots[slot] = Some(pending);
                slot
            }
            None => {
                self.slots.push(Some(pending));
                self.slots.len() - 1
            }
        }
    }

    /// Returns the next submission entry of the op in `slot`.
    fn entry(&mut self, slot: usize) -> squeue::Entry {
        self.slots[slot]
            .as_mut()
            .expect("a queued slot holds its op")
            .work
            .entry()
            .user_data(slot as u64)
    }
}
