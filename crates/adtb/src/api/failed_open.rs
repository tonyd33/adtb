use std::io::ErrorKind;

use crate::runtime::{IoError, Runtime};
use crate::storage::policy::{self, Phase};
use crate::storage::{self, Pager, Unpublished};

use super::{Engine, OpenError, corrupt};

/// Publish `new` once `opened` holds its engine, or discard it.
pub(super) fn finish_create<R: Runtime>(
    rt: R,
    pager: &Pager<R>,
    new: &Unpublished,
    opened: Result<Engine<R>, OpenError>,
    hold: storage::Hold<'_>,
) -> Result<Engine<R>, OpenError> {
    let published = opened.and_then(|mut engine| match new.publish(pager, hold) {
        Ok(()) => Ok(engine),
        Err(e) => {
            engine.archiver = None;
            Err(failed_create(e))
        }
    });
    if published.is_err() {
        new.discard(rt, hold).map_err(failed_create)?;
    }
    published
}

/// Returns the refusal a failure of a create or of publishing its database makes.
pub(super) fn failed_create(e: storage::OpenError) -> OpenError {
    match e {
        storage::OpenError::Locked => OpenError::Locked { pid: None },
        storage::OpenError::Io(e) => OpenError::Io(host_error(e)),
        storage::OpenError::SegmentSize(n) => OpenError::Io(std::io::Error::new(
            ErrorKind::InvalidInput,
            format!("segment size {n} is not a positive multiple of the log block"),
        )),
        other => corrupt(other),
    }
}

/// Returns the host's error of `e`'s kind.
pub(super) fn host_error(e: IoError) -> std::io::Error {
    std::io::Error::from(match e {
        IoError::NotFound => ErrorKind::NotFound,
        IoError::Exists => ErrorKind::AlreadyExists,
        IoError::NotADirectory => ErrorKind::NotADirectory,
        IoError::Full => ErrorKind::StorageFull,
        IoError::Failed => ErrorKind::Other,
    })
}

/// Returns the damage an open that failed may still answer `verify` over: a page that failed
/// verification on a database quarantined whole. Returns `None` while any page is unreadable,
/// but one whose damage holds the database for the open alone.
pub(super) fn open_damage<R: Runtime>(pager: &Pager<R>) -> Option<storage::PageError> {
    if !pager.database_marked() {
        return None;
    }
    let open_only = |pid: storage::Pid, damage: storage::Damage| {
        pager.held_unwalked(pid).is_some_and(|holder| {
            policy::mark(policy::Fault::Page(damage), holder, Phase::Opening).open_only()
        })
    };
    let damaged: Vec<_> = pager
        .damaged()
        .into_iter()
        .filter(|&(pid, damage)| !open_only(pid, damage))
        .collect();
    if damaged
        .iter()
        .any(|(_, d)| matches!(d, storage::Damage::Unreadable))
    {
        return None;
    }
    damaged
        .into_iter()
        .next()
        .map(|(pid, damage)| storage::PageError { pid, damage })
}
