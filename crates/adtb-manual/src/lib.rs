//! Run the Rust examples of the user manual: each `rust` block of a page is a program whose
//! `main` a test calls, and the `text` block right after it, if any, is what it prints.

#[cfg(test)]
extern crate adtb_lang as _;

use std::fmt::Debug;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};

/// What the running example has printed, one line per `println!`.
static PRINTED: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Held while an example runs: an example opens files relative to the working directory.
static RUNNING: Mutex<()> = Mutex::new(());

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Record `line` as printed by the running example.
pub fn print(line: String) {
    lock(&PRINTED).extend(line.lines().map(str::to_owned));
}

/// What an example's `main` returns.
pub trait Outcome {
    /// Returns the error `main` failed with, as a reader would see it.
    fn failure(self) -> Option<String>;
}

impl Outcome for () {
    fn failure(self) -> Option<String> {
        None
    }
}

impl<E: Debug> Outcome for Result<(), E> {
    fn failure(self) -> Option<String> {
        self.err().map(|e| format!("{e:?}"))
    }
}

/// Run `main`, the example at `line` of manual page `page`, in the empty directory `dir`, and
/// assert that it succeeds and prints `shown`, the lines its page shows.
pub fn run<T: Outcome>(page: &str, line: usize, dir: &Path, shown: &str, main: fn() -> T) {
    let _running = lock(&RUNNING);
    match std::fs::remove_dir_all(dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => panic!("clear {dir:?}: {e}"),
        _ => {}
    }
    std::fs::create_dir_all(dir).unwrap_or_else(|e| panic!("create {dir:?}: {e}"));
    std::env::set_current_dir(dir).unwrap_or_else(|e| panic!("enter {dir:?}: {e}"));
    lock(&PRINTED).clear();
    if let Some(e) = main().failure() {
        panic!("{page}:{line}: the example fails: {e}");
    }
    let printed = std::mem::take(&mut *lock(&PRINTED));
    let shown: Vec<&str> = shown.lines().collect();
    assert_eq!(
        shown, printed,
        "{page}:{line}: the example prints what its page shows"
    );
}
