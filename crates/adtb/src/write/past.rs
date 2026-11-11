//! Reads of the past: the history a database keeps for its retention window, the stores that
//! read at a safe snapshot marker, and the vacuum that moves versions into history.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use adtb_lang::types::RelId;

use crate::btree::Index;
use crate::exec::{self, Histories, PastReads, Reads, Store, StoredHeap, StoredIndex, Val};
use crate::heap::{Heap, HeapNo, Slots};
use crate::history::History;
use crate::key::{CtorId, Tid, Value};
use crate::mvcc::{
    Logged, Marker, Past, PastError, Pinned, Reader, ScanId, SerializationFailure, Transactions,
    Writers, Xid,
};
use crate::plan::{HeapId, IndexId};
use crate::runtime::Runtime;
use crate::storage::{Lsn, Pager, Pid, Position};
use crate::vacuum;

use super::reindex::VacuumHold;
use super::schema::Rel;
use super::{Database, Error};

const COMMIT_TIMES: u64 = 370;
const SAFE_SNAPSHOTS: u64 = 371;

/// The structures a database keeps history in, and the schema version each heap and each
/// migration's transaction belongs to.
#[derive(Debug)]
pub struct Retained {
    pub record: Past,
    /// The history heap of each heap.
    pub heaps: BTreeMap<HeapId, StoredHeap>,
    /// The history key index of each relation, by its key index.
    pub indexes: BTreeMap<IndexId, Index>,
    /// The transaction that applied each migration and the TID of the roots version recording
    /// it, by the version it made.
    pub migrations: BTreeMap<u64, (Xid, Tid)>,
    /// The version whose migration made each heap; the heaps of the engine's own declarations
    /// are absent.
    pub born: BTreeMap<HeapId, u64>,
    /// The version the database is at.
    pub version: u64,
}

/// What a read of the past sees: the versions the snapshot of a pinned marker sees, taking no
/// SIREAD locks.
struct AtMarker<R: Runtime> {
    reader: Reader<Logged<R>>,
    pinned: Pinned,
}

impl<R: Runtime> Reads for AtMarker<R> {
    fn saw(&self, (at, heap): (Pid, HeapNo), writers: &Writers) -> Result<bool, exec::Error> {
        let seen = self.reader.sees(writers);
        let at = (at, Slots::Versions(heap.0));
        let refuse = |e| Heap::refuse_unreserved(self.reader.pager(), at, (), e);
        Ok(seen.map_err(refuse)?.visible())
    }

    fn scanning(&self, _: HeapNo) -> Result<Option<ScanId>, SerializationFailure> {
        Ok(None)
    }

    fn scanned(&self, _: HeapNo, _: ScanId, _: Option<&[Pid]>) {}

    fn probed(
        &self,
        _: Pid,
        _: Bound<Vec<u8>>,
        _: Bound<Vec<u8>>,
        _: BTreeSet<CtorId>,
    ) -> Result<(), SerializationFailure> {
        Ok(())
    }

    fn fetched(&self, _: HeapNo, _: Tid) -> Result<(), SerializationFailure> {
        Ok(())
    }

    fn covers(&self, heap: HeapNo) -> bool {
        self.reader.covers(heap.0)
    }
}

/// The reads of the past of one statement, whose transaction reads commits up to `upto`.
pub(super) struct Pasts<R: Runtime> {
    pub(super) pager: Pager<R>,
    pub(super) txns: Arc<Transactions<R>>,
    pub(super) retained: Arc<Retained>,
    pub(super) heaps: BTreeMap<HeapId, StoredHeap>,
    pub(super) indexes: BTreeMap<IndexId, StoredIndex>,
    pub(super) upto: Position,
}

/// Returns the position `v`, a value of the prelude's `Position`, or `None` when no commit can
/// have it.
pub(crate) fn position(v: &Val) -> Result<Option<Position>, exec::Error> {
    let [Val::Int(epoch), Val::Int(lsn)] = v.fields()? else {
        return Err(exec::Broken::NotPosition(v.clone()).into());
    };
    Ok(u64::try_from(*epoch)
        .ok()
        .zip(u64::try_from(*lsn).ok())
        .map(|(epoch, lsn)| Position {
            epoch,
            lsn: Lsn(lsn),
        }))
}

/// Returns the wall time `micros` microseconds from the Unix epoch.
fn wall(micros: i64) -> SystemTime {
    let d = Duration::from_micros(micros.unsigned_abs());
    if micros < 0 {
        SystemTime::UNIX_EPOCH
            .checked_sub(d)
            .unwrap_or(SystemTime::UNIX_EPOCH)
    } else {
        SystemTime::UNIX_EPOCH + d
    }
}

impl<R: Runtime> Pasts<R> {
    /// Returns the marker `at` names, pinned.
    fn pin(&self, at: &Val, instant: bool) -> Result<Pinned, exec::Error> {
        let pinned = match (instant, at) {
            (true, Val::Instant(t)) => self.txns.pin_time(&self.pager, wall(*t), self.upto),
            (true, other) => {
                return Err(exec::Broken::NotInstant(other.clone()).into());
            }
            (false, p) => match position(p)? {
                Some(p) => self.txns.pin(&self.pager, p, self.upto),
                None => Err(PastError::Unavailable { oldest: None }),
            },
        };
        pinned.map_err(|e| match e {
            PastError::Unavailable { oldest } => exec::Error::HistoryUnavailable {
                oldest: oldest.unwrap_or(self.upto),
            },
            PastError::Damaged(crate::btree::Error::Page(p)) => exec::Error::Page(p),
            PastError::Damaged(d) => exec::Rot::RetentionWindow(d).into(),
        })
    }

    /// Returns whether the snapshot of `marker` sees the commit of `x`, the writer of the roots
    /// version at `at`.
    fn sees_commit(&self, marker: &Marker, (x, at): (Xid, Tid)) -> Result<bool, exec::Error> {
        let at = (Pid(at.page()), Slots::Versions(crate::catalog::ROOTS));
        let refuse = |e| Heap::refuse_unreserved(&self.pager, at, (), e);
        let counted = self.txns.counts_committed(&self.pager, &marker.snapshot, x);
        Ok(counted.map_err(refuse)?)
    }

    /// Returns the schema version at the state `marker` reads.
    fn version_at(&self, marker: &Marker) -> Result<u64, exec::Error> {
        for (&v, &made) in self.retained.migrations.iter().rev() {
            if self.sees_commit(marker, made)? {
                return Ok(v);
            }
        }
        Ok(0)
    }

    /// Returns the oldest position a read of the past may name and see the migration that made
    /// version `v`, or the reader's own when none.
    fn oldest_since(&self, v: u64) -> Result<Position, exec::Error> {
        let (Some(record), Some(cutoff)) = (self.txns.history(), self.txns.cutoff()) else {
            return Ok(self.upto);
        };
        let made = self.retained.migrations.get(&v).copied();
        let markers = record
            .markers(&self.pager)
            .map_err(|e| exec::Error::from(exec::Rot::Markers(e)))?;
        let mut oldest = self.upto;
        for m in markers
            .iter()
            .filter(|m| m.time >= cutoff && m.at <= self.upto)
        {
            match made {
                Some(made) if !self.sees_commit(m, made)? => break,
                _ => oldest = m.at,
            }
        }
        Ok(oldest)
    }
}

impl<R: Runtime> PastReads<R> for Pasts<R> {
    fn store(
        &self,
        at: &Val,
        instant: bool,
        pins: Option<u64>,
        heaps: &BTreeSet<HeapId>,
    ) -> Result<Arc<Store<R>>, exec::Error> {
        let retained = &self.retained;
        if let Some(v) = pins.filter(|v| *v > retained.version) {
            return Err(exec::Error::Unsupported(format!(
                "an `at` block pinned at version {v}, after the database's version {}",
                retained.version
            )));
        }
        let pinned = self.pin(at, instant)?;
        let reads = AtMarker {
            reader: self.txns.reader_at(&self.pager, &pinned),
            pinned,
        };
        let version = self.version_at(reads.pinned.marker())?;
        let typed = pins.unwrap_or(version);
        let made = heaps
            .iter()
            .filter_map(|h| retained.born.get(h))
            .max()
            .copied()
            .unwrap_or(0);
        if made > version && made > typed {
            return Err(exec::Error::HistoryUnavailable {
                oldest: self.oldest_since(made)?,
            });
        }
        if made > version.min(typed) {
            return Err(exec::Error::WrongVersion { version });
        }
        Ok(Arc::new(Store {
            pager: self.pager.clone(),
            reads: Arc::new(reads),
            heaps: self.heaps.clone(),
            indexes: self.indexes.clone(),
            history: Histories {
                heaps: retained.heaps.clone(),
                indexes: retained.indexes.clone(),
            },
            past: None,
        }))
    }
}

impl<R: Runtime> Database<R> {
    pub(super) fn volatile_rows(&self, rel: RelId) -> Result<BTreeSet<Val>, Error> {
        let id = self.schema.ids.relations[&rel];
        if id == super::vacuums::VACUUMS {
            return Ok(self.vacuum_rows());
        }
        if id != COMMIT_TIMES && id != SAFE_SNAPSHOTS {
            return Ok(self
                .volatile
                .lock(&self.rt)
                .get(&rel)
                .cloned()
                .unwrap_or_default());
        }
        let (Some(past), Some(cutoff)) = (self.session.txns.history(), self.session.txns.cutoff())
        else {
            return Ok(BTreeSet::new());
        };
        if id == COMMIT_TIMES {
            Ok(past
                .commits(&self.pager)?
                .into_iter()
                .filter(|c| c.time >= cutoff)
                .map(|c| {
                    Val::ctor(
                        0,
                        vec![
                            position_value(c.at),
                            Val::Instant(i64::try_from(c.time / 1000).expect("commit time fits")),
                        ],
                    )
                })
                .collect())
        } else {
            Ok(past
                .markers(&self.pager)?
                .into_iter()
                .filter(|m| m.time >= cutoff)
                .map(|m| Val::ctor(0, vec![position_value(m.at)]))
                .collect())
        }
    }

    /// Vacuum the heaps of `sum`, of constructors of `rel`, moving each version the retention
    /// window keeps to its history heap when history is kept, and then expire the history heaps
    /// of the same constructors. Returns what the vacuum of the current heaps did, and whether
    /// the history heaps keep a page.
    pub(super) fn vacuum_sum(
        &self,
        rel: &Rel,
        sum: &vacuum::Sum,
        pace: &mut dyn FnMut() -> bool,
    ) -> Result<(vacuum::Report, bool), vacuum::Error> {
        let progress = &self.vacuums.progress;
        let kept = self
            .retained
            .as_ref()
            .filter(|_| self.session.txns.history().is_some());
        let Some(retained) = kept else {
            let report = vacuum::vacuum(
                &self.pager,
                &self.session.txns,
                sum,
                self.grant,
                progress,
                pace,
            )?;
            return Ok((report, false));
        };
        let key_index = rel.key_index;
        let ctors = &self.indexes[&key_index].ctors;
        let keyed = &rel
            .indexes
            .iter()
            .find(|(i, _)| *i == key_index)
            .expect("a relation's key index is among its indexes")
            .1
            .keys;
        let types: Vec<_> = self.indexes[&key_index]
            .index
            .schema()
            .components()
            .iter()
            .map(|(t, _)| t.clone())
            .collect();
        let key = |ctor: CtorId, row: &[Value]| -> Vec<Value> {
            let c = ctors[&ctor];
            keyed.key(c, row, &types).ok().flatten().unwrap_or_else(|| {
                panic!("a stored row of constructor {c} has the key it had at its insert")
            })
        };
        let pasts: Vec<(CtorId, &Heap)> = sum
            .heaps
            .iter()
            .map(|(id, h)| (*id, &*retained.heaps[&HeapId(u64::from(h.id().0))].heap))
            .collect();
        let index = &retained.indexes[&key_index];
        let into = History {
            heaps: pasts.clone(),
            index,
            key: &key,
        };
        let report = vacuum::vacuum_keeping(
            &self.pager,
            &self.session.txns,
            sum,
            &into,
            self.grant,
            progress,
            pace,
        )?;
        let past = vacuum::Sum {
            heaps: pasts,
            indexes: vec![index],
        };
        let left = vacuum::expire(
            &self.pager,
            &self.session.txns,
            &past,
            self.grant,
            progress,
            pace,
        )?;
        Ok((report, left.scanned + left.skipped > left.removed))
    }

    /// Returns the reads of the past of a statement reading commits up to `upto`, when the
    /// database keeps history structures.
    pub(super) fn pasts(&self, upto: Position) -> Option<Arc<dyn PastReads<R>>> {
        let retained = self.retained.clone()?;
        Some(Arc::new(Pasts {
            pager: self.pager.clone(),
            txns: self.session.txns.clone(),
            retained,
            heaps: self.heaps.clone(),
            indexes: self.indexes.clone(),
            upto,
        }))
    }

    /// Keep history for a retention window of `window` from now on: begin keeping it when it is
    /// not kept and `window` is not zero.
    ///
    /// Fail if the record of the window is damaged.
    pub fn set_retention(&self, window: Duration, _held: &VacuumHold<'_, R>) -> Result<(), Error> {
        if self.session.txns.history().is_some() {
            self.session.txns.set_window(window);
            return Ok(());
        }
        match &self.retained {
            Some(r) if !window.is_zero() => {
                self.session
                    .txns
                    .keep_history(&self.pager, r.record, window)?;
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

fn position_value(at: Position) -> Val {
    Val::ctor(
        0,
        vec![
            Val::Int(i64::try_from(at.epoch).expect("a restore keeps the epoch within an I64")),
            Val::Int(i64::try_from(at.lsn.0).expect("commit LSN fits")),
        ],
    )
}
