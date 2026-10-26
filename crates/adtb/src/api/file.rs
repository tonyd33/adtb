use std::collections::BTreeSet;
use std::fs::{File, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use super::OpenError;

/// The canonical path of every database file this process holds open.
static OPEN: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());

/// The exclusive lock on a database file, and its place among the files this process holds.
#[derive(Debug)]
pub struct Held {
    path: PathBuf,
    file: File,
}

impl Held {
    /// Lock the database file at `path`, creating it empty first when `create` is set. Returns
    /// the lock, the file's directory and its name in that directory.
    pub fn take(path: &Path, create: bool) -> Result<(Held, PathBuf, String), OpenError> {
        let file = File::options()
            .read(true)
            .write(true)
            .create(create)
            .truncate(false)
            .open(path)
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => OpenError::NotFound,
                _ => OpenError::Io(e),
            })?;
        let canonical = path.canonicalize().map_err(OpenError::Io)?;
        let (Some(dir), Some(name)) = (
            canonical.parent(),
            canonical.file_name().and_then(|n| n.to_str()),
        ) else {
            return Err(OpenError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("{} names no file in a directory", path.display()),
            )));
        };
        let (dir, name) = (dir.to_path_buf(), name.to_owned());
        if !OPEN
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(canonical.clone())
        {
            return Err(OpenError::AlreadyOpen);
        }
        let held = Held {
            path: canonical,
            file,
        };
        match held.file.try_lock() {
            Ok(()) => Ok((held, dir, name)),
            Err(TryLockError::WouldBlock) => Err(OpenError::Locked { pid: None }),
            Err(TryLockError::Error(e)) => Err(OpenError::Io(e)),
        }
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        OPEN.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.path);
    }
}
