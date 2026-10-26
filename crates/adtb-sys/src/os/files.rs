use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};

use super::faults::{Faults, Injector};
use crate::runtime::{AlignedBuf, Done, FileId, IoError, Op};

/// The files one process has open, under its database directory.
#[derive(Debug)]
pub(crate) struct Files {
    root: PathBuf,
    open: RwLock<Vec<Option<Arc<File>>>>,
    faults: Option<Mutex<Injector>>,
}

pub(crate) type Outcome = (Option<AlignedBuf>, Result<Done, IoError>);

fn error(e: &io::Error) -> IoError {
    match e.kind() {
        ErrorKind::NotFound => IoError::NotFound,
        _ => IoError::Failed,
    }
}

fn done<T>(r: io::Result<T>, d: impl FnOnce(T) -> Done) -> Result<Done, IoError> {
    r.map(d).map_err(|e| error(&e))
}

impl Files {
    pub(crate) fn new(root: PathBuf, faults: Option<Faults>) -> Files {
        Files {
            root,
            open: RwLock::new(Vec::new()),
            faults: faults.map(|f| Mutex::new(Injector::new(f))),
        }
    }

    pub(crate) fn path(&self, relative: &str) -> PathBuf {
        if relative.is_empty() {
            self.root.clone()
        } else {
            self.root.join(relative)
        }
    }

    pub(crate) fn file(&self, id: FileId) -> Arc<File> {
        let open = self.open.read().unwrap_or_else(PoisonError::into_inner);
        open.get(id.0 as usize)
            .and_then(Option::clone)
            .unwrap_or_else(|| panic!("{id:?} is not open in this process"))
    }

    /// Run `op` to completion on the calling thread, with the faults injected if there are any.
    ///
    /// Panics on a `Timer`, which no file operation completes.
    pub(crate) fn run(&self, op: Op) -> Outcome {
        match &self.faults {
            Some(f) => f
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .run(self, op),
            None => self.perform(op),
        }
    }

    /// Panics if the files inject no faults.
    fn injector(&self) -> MutexGuard<'_, Injector> {
        self.faults
            .as_ref()
            .expect("the runtime was made with faults")
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Stop the device as a power loss would.
    pub(crate) fn crash(&self) {
        self.injector().crash(self);
    }

    pub(crate) fn set_faults(&self, faults: Faults) {
        self.injector().set(faults);
    }

    /// Run `op` to completion on the calling thread.
    pub(crate) fn perform(&self, op: Op) -> Outcome {
        match op {
            Op::Open { path, create } => (None, self.open(&path, create)),
            Op::Read { file, offset, buf } => self.read(file, offset, buf),
            Op::Write { file, offset, buf } => {
                let r = self.file(file).write_all_at(&buf, offset);
                (Some(buf), done(r, |()| Done::Written))
            }
            Op::Size { file } => (
                None,
                done(self.file(file).metadata(), |m| Done::Size(m.len())),
            ),
            Op::Allocate { file, len } => (
                None,
                done(allocate(&self.file(file), len), |()| Done::Allocated),
            ),
            Op::Sync { file } => (None, done(self.file(file).sync_data(), |()| Done::Synced)),
            Op::SyncDir { dir } => (None, done(sync_dir(&self.path(&dir)), |()| Done::Synced)),
            Op::List { dir } => (None, done(self.list(&dir), Done::Listed)),
            Op::Remove { path } => (
                None,
                done(fs::remove_file(self.path(&path)), |()| Done::Removed),
            ),
            Op::CreateDir { dir } => (None, done(create_dir(&self.path(&dir)), |()| Done::Created)),
            Op::Close { file } => {
                let mut open = self.open.write().unwrap_or_else(PoisonError::into_inner);
                let slot = open.get_mut(file.0 as usize).and_then(Option::take);
                assert!(slot.is_some(), "{file:?} is not open in this process");
                (None, Ok(Done::Closed))
            }
            Op::Timer { .. } => unreachable!("a timer is not a file operation"),
        }
    }

    fn open(&self, path: &str, create: bool) -> Result<Done, IoError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(create)
            .custom_flags(libc::O_DIRECT)
            .open(self.path(path))
            .map_err(|e| error(&e))?;
        let len = file.metadata().map_err(|e| error(&e))?.len();
        let mut open = self.open.write().unwrap_or_else(PoisonError::into_inner);
        let id = FileId(u32::try_from(open.len()).expect("a process opens fewer than 2^32 files"));
        open.push(Some(Arc::new(file)));
        Ok(Done::Opened { file: id, len })
    }

    /// Fill `buf` from `offset`, stopping at the end of the file.
    ///
    /// A regular file's `pread` returns short only at its end.
    fn read(&self, file: FileId, offset: u64, mut buf: AlignedBuf) -> Outcome {
        let file = self.file(file);
        let r = loop {
            match file.read_at(&mut buf, offset) {
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                r => break r,
            }
        };
        (Some(buf), done(r, Done::Read))
    }

    fn list(&self, dir: &str) -> io::Result<Vec<String>> {
        let mut out = Vec::new();
        for entry in fs::read_dir(self.path(dir))? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            out.push(if dir.is_empty() {
                name.to_owned()
            } else {
                format!("{dir}/{name}")
            });
        }
        Ok(out)
    }
}

fn create_dir(path: &Path) -> io::Result<()> {
    match fs::create_dir(path) {
        Err(e) if e.kind() == ErrorKind::AlreadyExists && path.is_dir() => Ok(()),
        r => r,
    }
}

fn sync_dir(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

fn allocate(file: &File, len: u64) -> io::Result<()> {
    if file.metadata()?.len() >= len {
        return Ok(());
    }
    let end = libc::off_t::try_from(len).map_err(|_| io::Error::from(ErrorKind::InvalidInput))?;
    // SAFETY: the descriptor belongs to `file`, which outlives the call.
    let r = unsafe { libc::fallocate(file.as_raw_fd(), 0, 0, end) };
    if r == 0 {
        return Ok(());
    }
    let e = io::Error::last_os_error();
    if e.raw_os_error() == Some(libc::EOPNOTSUPP) {
        return file.set_len(len);
    }
    Err(e)
}
