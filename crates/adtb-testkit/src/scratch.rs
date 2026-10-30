use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A directory for a test that needs real files.
///
/// Dropping it removes the directory, unless the thread is panicking: a failed test keeps its files.
#[derive(Debug)]
pub struct Scratch {
    path: PathBuf,
}

impl Scratch {
    /// Create a new directory under `target/tmp/GROUP`, unique across processes and calls.
    pub fn new(group: &str, label: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let parent = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/tmp")
            .join(group);
        std::fs::create_dir_all(&parent).expect("create scratch parent");
        loop {
            let nonce = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = parent.join(format!("{label}-{}-{nonce}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => return Self { path },
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("create scratch directory: {error}"),
            }
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            std::fs::remove_dir_all(&self.path).expect("remove scratch directory");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Scratch;

    #[test]
    fn a_passing_owner_removes_the_directory() {
        let scratch = Scratch::new("scratch-tests", "passing");
        let path = scratch.path().to_owned();
        std::fs::write(path.join("file"), b"x").unwrap();
        drop(scratch);
        assert!(!path.exists());
    }

    #[test]
    fn a_panicking_owner_keeps_the_directory() {
        let mut path = None;
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let scratch = Scratch::new("scratch-tests", "panicking");
            path = Some(scratch.path().to_owned());
            panic!("the owner fails");
        }));
        assert!(outcome.is_err());
        let path = path.unwrap();
        assert!(path.is_dir());
        std::fs::remove_dir_all(path).unwrap();
    }
}
