use std::collections::BTreeSet;
use std::fs::{File, TryLockError};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use super::OpenError;

/// The opens one [`Held::take`] tries while each finds its path no longer names the file it locked.
const TAKE_ATTEMPTS: usize = 8;

/// The canonical path of every database file this process holds open.
static OPEN: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());

/// The exclusive lock on a database file, and its place among the files this process holds.
#[derive(Debug)]
pub struct Held {
    path: PathBuf,
    file: File,
    created: bool,
}

/// A lock, the file's directory and its name in that directory.
type Taken = (Held, PathBuf, String);

/// Open the file at `path`, creating it when `create` is set. Returns the file and whether this
/// call's exclusive create made it.
fn open_file(path: &Path, create: bool) -> std::io::Result<(File, bool)> {
    let mut opening = File::options();
    opening.read(true).write(true);
    if create {
        match opening.clone().create_new(true).open(path) {
            Ok(file) => return Ok((file, true)),
            Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => return Err(e),
            Err(_) => {}
        }
    }
    let file = opening.create(create).truncate(false).open(path)?;
    Ok((file, false))
}

/// Returns whether `path` names the inode `file` has open.
fn names(path: &Path, file: &File) -> std::io::Result<bool> {
    let named = match std::fs::metadata(path) {
        Ok(named) => named,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    let open = file.metadata()?;
    Ok(named.dev() == open.dev() && named.ino() == open.ino())
}

/// Remove what a failed open made: the log directory, when it claimed one, and the file. A name
/// that cannot be removed stays, and does not stop the other's removal.
pub fn remove_created(log_dir: Option<&Path>, file: &Path) {
    if let Some(log_dir) = log_dir {
        drop(std::fs::remove_dir_all(log_dir));
    }
    drop(std::fs::remove_file(file));
}

impl Held {
    /// Lock the database file at `path`, creating it empty first when `create` is set. Returns
    /// the lock, the file's directory and its name in that directory.
    ///
    /// Postconditions:
    /// - The name returned names the locked inode, and keeps naming it for as long as every
    ///   other opener that renames or unlinks it does so holding the lock.
    pub fn take(path: &Path, create: bool) -> Result<Taken, OpenError> {
        Held::take_opened(path, create, || {})
    }

    /// Run [`Held::take`], calling `opened` after each open of the file.
    fn take_opened(
        path: &Path,
        create: bool,
        mut opened: impl FnMut(),
    ) -> Result<Taken, OpenError> {
        for _ in 0..TAKE_ATTEMPTS {
            if let Some(taken) = Held::attempt(path, create, &mut opened)? {
                return Ok(taken);
            }
        }
        Err(OpenError::Io(std::io::Error::new(
            std::io::ErrorKind::ResourceBusy,
            format!(
                "{} was replaced during each of {TAKE_ATTEMPTS} attempts to lock it",
                path.display()
            ),
        )))
    }

    /// Open the file and lock it. Returns `None` when `path` stopped naming the file on the way.
    fn attempt(
        path: &Path,
        create: bool,
        opened: &mut impl FnMut(),
    ) -> Result<Option<Taken>, OpenError> {
        let (file, created) = open_file(path, create).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => OpenError::NotFound,
            _ => OpenError::Io(e),
        })?;
        opened();
        let canonical = match path.canonicalize() {
            Ok(canonical) => canonical,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(OpenError::Io(e)),
        };
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
            created,
        };
        match held.file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Err(OpenError::Locked { pid: None }),
            Err(TryLockError::Error(e)) => return Err(OpenError::Io(e)),
        }
        let still = names(&held.path, &held.file).map_err(OpenError::Io)?;
        Ok(still.then_some((held, dir, name)))
    }

    /// Returns whether this lock's [`Held::take`] made the file.
    pub fn created(&self) -> bool {
        self.created
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        OPEN.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Scratch {
            let dir = std::env::temp_dir().join(format!("adtb-held-{}-{name}", std::process::id()));
            std::fs::create_dir_all(&dir).expect("the scratch directory");
            Scratch(dir.canonicalize().expect("the scratch directory's path"))
        }

        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }

        /// Rename a new file onto `path`.
        fn replace(&self, path: &Path) {
            let aside = self.path("aside");
            File::create(&aside).expect("the replacement");
            std::fs::rename(&aside, path).expect("the rename");
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).expect("remove the scratch directory");
        }
    }

    #[test]
    fn names_tells_the_locked_inode_from_a_replaced_and_an_unlinked_path() {
        let scratch = Scratch::new("names");
        let path = scratch.path("db");
        let file = File::create(&path).expect("the file");
        assert!(names(&path, &file).expect("same"));
        scratch.replace(&path);
        assert!(!names(&path, &file).expect("replaced"));
        std::fs::remove_file(&path).expect("the unlink");
        assert!(!names(&path, &file).expect("unlinked"));
    }

    #[test]
    fn a_path_replaced_before_the_lock_is_opened_again() {
        let scratch = Scratch::new("replaced");
        let path = scratch.path("db");
        File::create(&path).expect("the file");
        let mut opens = 0;
        let (held, _, name) = Held::take_opened(&path, false, || {
            opens += 1;
            if opens == 1 {
                scratch.replace(&path);
            }
        })
        .expect("the second open locks the replacement");
        assert_eq!(2, opens);
        assert_eq!("db", name);
        assert!(names(&path, &held.file).expect("the locked inode"));
        assert!(!held.created());
    }

    #[test]
    fn a_path_unlinked_before_the_lock_is_created_again() {
        let scratch = Scratch::new("unlinked");
        let path = scratch.path("db");
        let mut opens = 0;
        let (held, _, _) = Held::take_opened(&path, true, || {
            opens += 1;
            if opens == 1 {
                std::fs::remove_file(&path).expect("the unlink");
            }
        })
        .expect("the second open makes the file");
        assert_eq!(2, opens);
        assert!(names(&path, &held.file).expect("the locked inode"));
        assert!(held.created());
    }

    #[test]
    fn a_path_unlinked_before_the_lock_without_create_is_not_found() {
        let scratch = Scratch::new("gone");
        let path = scratch.path("db");
        File::create(&path).expect("the file");
        let got = Held::take_opened(&path, false, || {
            std::fs::remove_file(&path).expect("the unlink");
        });
        assert!(matches!(got, Err(OpenError::NotFound)), "{got:?}");
    }

    #[test]
    fn a_path_replaced_on_every_attempt_is_refused_and_leaves_no_hold() {
        let scratch = Scratch::new("every");
        let path = scratch.path("db");
        File::create(&path).expect("the file");
        let mut opens = 0;
        let got = Held::take_opened(&path, false, || {
            opens += 1;
            scratch.replace(&path);
        });
        assert_eq!(TAKE_ATTEMPTS, opens);
        let Err(OpenError::Io(e)) = got else {
            panic!("{got:?}");
        };
        assert_eq!(std::io::ErrorKind::ResourceBusy, e.kind());
        Held::take(&path, false).expect("no attempt left the path held");
    }

    #[test]
    fn a_log_directory_that_cannot_be_removed_leaves_the_file_removed() {
        let scratch = Scratch::new("cleanup");
        let path = scratch.path("db");
        let blocker = scratch.path("blocker");
        File::create(&path).expect("the file");
        File::create(&blocker).expect("the blocker");
        let log_dir = blocker.join("log");
        std::fs::remove_dir_all(&log_dir).expect_err("a path under a file cannot be removed");
        remove_created(Some(&log_dir), &path);
        assert!(!path.exists());
    }
}
