use std::collections::BTreeMap;

use crate::runtime::{AlignedBuf, Completion, Done, FileId, Io, IoError, Op, Runtime, Ticket};

/// Blocking calls over one I/O queue, for a thread that waits on each op it submits.
pub(crate) struct Driver<R: Runtime> {
    pub(crate) rt: R,
    io: R::Io,
    ready: BTreeMap<Ticket, Completion>,
}

/// Crash the process on a failed write or force, which the engine never retries.
fn fatal(what: &str, e: IoError) -> ! {
    panic!("fatal I/O error: {what}: {e:?}")
}

impl<R: Runtime> Driver<R> {
    pub(crate) fn new(rt: R) -> Self {
        let io = rt.io();
        Driver {
            rt,
            io,
            ready: BTreeMap::new(),
        }
    }

    pub(crate) fn submit(&mut self, op: Op) -> Ticket {
        self.io.submit(op)
    }

    pub(crate) fn wait(&mut self, ticket: Ticket) -> Completion {
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

    fn run(&mut self, op: Op) -> Completion {
        let t = self.submit(op);
        self.wait(t)
    }

    pub(crate) fn open(&mut self, path: &str, create: bool) -> Result<(FileId, u64), IoError> {
        let path = path.to_owned();
        match self.run(Op::Open { path, create }).result? {
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
    ) -> Result<AlignedBuf, IoError> {
        let buf = AlignedBuf::zeroed(len);
        let c = self.run(Op::Read { file, offset, buf });
        c.result?;
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

    pub(crate) fn list(&mut self, dir: &str) -> Vec<String> {
        let dir = dir.to_owned();
        match self.run(Op::List { dir }).result {
            Ok(Done::Listed(paths)) => paths,
            Ok(other) => unreachable!("list completed as {other:?}"),
            Err(IoError::NotFound) => Vec::new(),
            Err(e) => fatal("list", e),
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

    pub(crate) fn close(&mut self, file: FileId) {
        if let Err(e) = self.run(Op::Close { file }).result {
            fatal("close", e);
        }
    }

    pub(crate) fn remove(&mut self, path: &str) {
        let path = path.to_owned();
        match self.run(Op::Remove { path }).result {
            Ok(_) | Err(IoError::NotFound) => {}
            Err(e) => fatal("remove", e),
        }
    }
}
