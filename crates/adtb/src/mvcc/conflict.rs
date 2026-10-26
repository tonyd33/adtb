use super::{Cause, SerializationFailure, Snapshot, Status, Xid};

fn later(x: Xid) -> SerializationFailure {
    SerializationFailure::new(Cause::LaterCommit, vec![x])
}

/// What a uniqueness probe does about one version it found under the probed key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Probe {
    /// The version does not hold the key in the latest committed state.
    Skip,
    /// The version holds the key: the insert is a key violation.
    Conflict,
    /// Wait for the transaction to end, then probe the version again.
    Wait(Xid),
    Fail(SerializationFailure),
}

/// What a write does about the version it updates, deletes or retags.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Write {
    Proceed,
    /// Wait for the transaction to end, then check the version again.
    Wait(Xid),
    Fail(SerializationFailure),
}

/// Returns what an insert by `me`, reading at `snapshot`, does about the version whose header
/// holds `xmin` and `xmax`, found under the key it inserts into a unique index. `status` gives
/// the latest commit log status.
///
/// A version `me` inserted and has not deleted is a conflict; whether it came from the same
/// statement is the caller's, from its command ids.
pub fn probe_unique<E>(
    me: Xid,
    snapshot: &Snapshot,
    xmin: u64,
    xmax: u64,
    mut status: impl FnMut(Xid) -> Result<Status, E>,
) -> Result<Probe, E> {
    let (xmin, xmax) = (Xid(xmin), Xid(xmax));
    let inserter = if xmin == me {
        Status::Committed
    } else {
        status(xmin)?
    };
    if inserter == Status::Aborted || xmax == me {
        return Ok(Probe::Skip);
    }
    let in_snapshot = |x: Xid, status: &mut dyn FnMut(Xid) -> Result<Status, E>| {
        Ok::<bool, E>(x == me || snapshot.counts_committed(x, status)?)
    };
    if xmax != Xid::NONE {
        match status(xmax)? {
            Status::Committed if in_snapshot(xmax, &mut status)? => return Ok(Probe::Skip),
            Status::Committed => return Ok(Probe::Fail(later(xmax))),
            Status::InProgress if inserter == Status::InProgress => {
                return Ok(Probe::Wait(xmin));
            }
            Status::InProgress if in_snapshot(xmin, &mut status)? => {
                return Ok(Probe::Wait(xmax));
            }
            Status::InProgress => return Ok(Probe::Fail(later(xmin))),
            Status::Aborted => {}
        }
    }
    Ok(match inserter {
        Status::InProgress => Probe::Wait(xmin),
        _ if in_snapshot(xmin, &mut status)? => Probe::Conflict,
        _ => Probe::Fail(later(xmin)),
    })
}

/// Returns what a write by `me` does about the version it found visible at its snapshot, whose
/// header now holds `xmax`. `status` gives the latest commit log status.
pub fn check_write<E>(
    me: Xid,
    xmax: u64,
    status: impl FnOnce(Xid) -> Result<Status, E>,
) -> Result<Write, E> {
    let xmax = Xid(xmax);
    if xmax == Xid::NONE || xmax == me {
        return Ok(Write::Proceed);
    }
    Ok(match status(xmax)? {
        Status::Aborted => Write::Proceed,
        Status::InProgress => Write::Wait(xmax),
        Status::Committed => Write::Fail(later(xmax)),
    })
}

/// Returns the concurrent writer that a read by `reader` at `snapshot`, of the version whose
/// header holds `xmin` and `xmax`, reads before: the inserter of a version the snapshot misses,
/// or the deleter of a version it sees, when that writer has not aborted and the snapshot does
/// not count it as committed.
pub fn read_before<E>(
    reader: Option<Xid>,
    snapshot: &Snapshot,
    xmin: u64,
    xmax: u64,
    mut status: impl FnMut(Xid) -> Result<Status, E>,
) -> Result<Option<Xid>, E> {
    let (xmin, xmax) = (Xid(xmin), Xid(xmax));
    if Some(xmin) != reader && !snapshot.counts_committed(xmin, &mut status)? {
        return Ok((status(xmin)? != Status::Aborted).then_some(xmin));
    }
    if xmax == Xid::NONE || Some(xmax) == reader || snapshot.counts_committed(xmax, &mut status)? {
        return Ok(None);
    }
    Ok((status(xmax)? != Status::Aborted).then_some(xmax))
}
