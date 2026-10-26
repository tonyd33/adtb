use super::{Status, Xid};

/// The transactions a reader counts as committed: those below `xmax`, not in `in_progress`, that
/// the commit log says committed.
///
/// Invariants: `in_progress` is sorted, and every id in it lies in `[xmin, xmax)`; every id below
/// `xmin` had resolved when the snapshot was taken.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    xmin: Xid,
    xmax: Xid,
    in_progress: Vec<Xid>,
}

impl Snapshot {
    /// Panics if `in_progress` breaks the invariants.
    pub fn new(xmin: Xid, xmax: Xid, in_progress: Vec<Xid>) -> Snapshot {
        assert!(xmin <= xmax, "snapshot xmin {xmin:?} is past xmax {xmax:?}");
        assert!(
            in_progress.is_sorted_by(|a, b| a < b)
                && in_progress.iter().all(|x| (xmin..xmax).contains(x)),
            "snapshot in-progress set {in_progress:?} is not sorted within [{xmin:?}, {xmax:?})"
        );
        Snapshot {
            xmin,
            xmax,
            in_progress,
        }
    }

    pub fn xmin(&self) -> Xid {
        self.xmin
    }

    pub fn xmax(&self) -> Xid {
        self.xmax
    }

    pub fn in_progress(&self) -> &[Xid] {
        &self.in_progress
    }

    /// Returns the snapshot as a safe snapshot marker carries it: `xmin` and `xmax` as
    /// little-endian `u64`s, then each in-progress id the same way.
    pub fn encode(&self) -> Vec<u8> {
        [self.xmin, self.xmax]
            .iter()
            .chain(&self.in_progress)
            .flat_map(|x| x.0.to_le_bytes())
            .collect()
    }

    /// Returns the snapshot `bytes` encodes, or `None` if they encode none.
    pub fn decode(bytes: &[u8]) -> Option<Snapshot> {
        if bytes.len() < 16 || !bytes.len().is_multiple_of(8) {
            return None;
        }
        let ids: Vec<Xid> = bytes
            .as_chunks::<8>()
            .0
            .iter()
            .map(|c| Xid(u64::from_le_bytes(*c)))
            .collect();
        let (xmin, xmax, in_progress) = (ids[0], ids[1], ids[2..].to_vec());
        let valid = xmin <= xmax
            && in_progress.is_sorted_by(|a, b| a < b)
            && in_progress.iter().all(|x| (xmin..xmax).contains(x));
        valid.then(|| Snapshot::new(xmin, xmax, in_progress))
    }

    /// Returns whether the writer `x` counts as committed, consulting `status` only for an id
    /// below `xmax` and not in progress.
    pub fn counts_committed<E>(
        &self,
        x: Xid,
        status: impl FnOnce(Xid) -> Result<Status, E>,
    ) -> Result<bool, E> {
        if x >= self.xmax || (x >= self.xmin && self.in_progress.binary_search(&x).is_ok()) {
            return Ok(false);
        }
        Ok(status(x)? == Status::Committed)
    }

    /// Returns whether the version whose header holds `xmin` and `xmax` is visible to `reader`
    /// reading at this snapshot. `reader` sees its own writes, whatever their command.
    pub fn sees<E>(
        &self,
        reader: Option<Xid>,
        xmin: u64,
        xmax: u64,
        mut status: impl FnMut(Xid) -> Result<Status, E>,
    ) -> Result<bool, E> {
        let mut counts = |x: Xid| {
            if Some(x) == reader {
                Ok(true)
            } else {
                self.counts_committed(x, &mut status)
            }
        };
        if !counts(Xid(xmin))? {
            return Ok(false);
        }
        Ok(Xid(xmax) == Xid::NONE || !counts(Xid(xmax))?)
    }
}
