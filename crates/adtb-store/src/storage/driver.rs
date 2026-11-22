use std::collections::BTreeMap;

use crate::runtime::fatal;
use crate::runtime::{AlignedBuf, Completion, Done, FileId, Io, IoError, Op, Runtime, Ticket};

/// A failed storage operation.
///
/// Never derive `Debug`: without it no caller can `unwrap` or `expect` a failed operation.
#[expect(
    missing_debug_implementations,
    reason = "a failed operation would need another way to stop callers unwrapping it"
)]
pub struct Failed(IoError);

impl Failed {
    /// End the process with this failure as a fatal I/O payload.
    pub(crate) fn fatal(self, context: impl Into<String>) -> ! {
        fatal(context, self.0)
    }

    /// Returns the failure, for an operation whose failure is a stated refusal.
    pub fn cause(self) -> IoError {
        self.0
    }
}

/// Blocking calls over one I/O queue, for a thread that waits on each op it submits.
pub struct Driver<R: Runtime> {
    pub(crate) rt: R,
    io: R::Io,
    ready: BTreeMap<Ticket, Completion>,
}

impl<R: Runtime> Driver<R> {
    pub fn new(rt: R) -> Self {
        let io = rt.io();
        Driver {
            rt,
            io,
            ready: BTreeMap::new(),
        }
    }

    pub fn submit(&mut self, op: Op) -> Ticket {
        self.io.submit(op)
    }

    pub fn wait(&mut self, ticket: Ticket) -> Completion {
        loop {
            if let Some(c) = self.ready.remove(&ticket) {
                return c;
            }
            match self.io.poll() {
                Some(c) => {
                    self.ready.insert(c.ticket, c);
                }
                None => self.rt.park(),
            }
        }
    }

    /// Run `op` to completion and hand back its outcome, a failure included.
    pub(crate) fn attempt(&mut self, op: Op) -> Result<Done, Failed> {
        self.run(op).result.map_err(Failed)
    }

    fn run(&mut self, op: Op) -> Completion {
        let t = self.submit(op);
        self.wait(t)
    }

    pub(crate) fn create_new(&mut self, path: &str) -> Result<(FileId, u64), Failed> {
        let path = path.to_owned();
        match self.attempt(Op::CreateNew { path })? {
            Done::Opened { file, len } => Ok((file, len)),
            other => unreachable!("exclusive create completed as {other:?}"),
        }
    }

    pub fn open(&mut self, path: &str, create: bool) -> Result<(FileId, u64), Failed> {
        let path = path.to_owned();
        match self.attempt(Op::Open { path, create })? {
            Done::Opened { file, len } => Ok((file, len)),
            other => unreachable!("open completed as {other:?}"),
        }
    }

    /// Returns `len` bytes at `offset`, zero past the end of the file.
    pub(crate) fn read(
        &mut self,
        file: FileId,
        offset: u64,
        len: usize,
    ) -> Result<AlignedBuf, Failed> {
        let buf = AlignedBuf::zeroed(len);
        let c = self.run(Op::Read { file, offset, buf });
        c.result.map_err(Failed)?;
        Ok(c.buf.expect("a read returns its buffer"))
    }

    /// Write every buffer at its offset of `file`, all in flight at once.
    pub(crate) fn write_all(&mut self, writes: Vec<(FileId, u64, AlignedBuf)>) {
        let tickets: Vec<Ticket> = writes
            .into_iter()
            .map(|(file, offset, buf)| self.submit(Op::Write { file, offset, buf }))
            .collect();
        for t in tickets {
            if let Err(e) = self.wait(t).result {
                fatal("write", e);
            }
        }
    }

    pub(crate) fn sync_all(&mut self, files: &[FileId]) {
        let tickets: Vec<Ticket> = files
            .iter()
            .map(|&file| self.submit(Op::Sync { file }))
            .collect();
        for t in tickets {
            if let Err(e) = self.wait(t).result {
                fatal("sync", e);
            }
        }
    }

    pub(crate) fn sync_dir(&mut self, dir: &str) {
        let dir = dir.to_owned();
        if let Err(e) = self.run(Op::SyncDir { dir }).result {
            fatal("directory sync", e);
        }
    }

    pub(crate) fn allocate(&mut self, file: FileId, len: u64) {
        if let Err(e) = self.run(Op::Allocate { file, len }).result {
            fatal("allocate", e);
        }
    }

    /// Create the directory `dir` if it is absent.
    pub(crate) fn create_dir(&mut self, dir: &str) {
        let dir = dir.to_owned();
        if let Err(e) = self.run(Op::CreateDir { dir }).result {
            fatal("create directory", e);
        }
    }

    pub(crate) fn size(&mut self, file: FileId) -> u64 {
        match self.run(Op::Size { file }).result {
            Ok(Done::Size(len)) => len,
            Ok(other) => unreachable!("size completed as {other:?}"),
            Err(e) => fatal("size", e),
        }
    }

    pub fn close(&mut self, file: FileId) {
        if let Err(e) = self.run(Op::Close { file }).result {
            fatal("close", e);
        }
    }

    pub fn remove(&mut self, path: &str) {
        let path = path.to_owned();
        match self.run(Op::Remove { path }).result {
            Ok(_) | Err(IoError::NotFound) => {}
            Err(e) => fatal("remove", e),
        }
    }
}

impl<R: Runtime> std::fmt::Debug for Driver<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Driver")
    }
}
