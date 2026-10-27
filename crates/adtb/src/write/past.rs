//! Reads of the past: the history a database keeps for its retention window, the stores that
//! read at a safe snapshot marker, and the vacuum that moves versions into history.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use adtb_lang::types::RelId;

use crate::btree::Index;
use crate::exec::{self, Histories, PastReads, Reads, Store, StoredHeap, StoredIndex, Val};
use crate::heap::{Header, HeapNo};
use crate::history::History;
use crate::key::{CtorId, Tid, Value};
use crate::mvcc::{Marker, Past, PastError, SerializationFailure, Transactions, Xid};
use crate::plan::{HeapId, IndexId};
use crate::runtime::Runtime;
use crate::storage::{Lsn, PageError, Pager, Pid, Position};
use crate::vacuum;

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
    /// The transaction that applied each migration, by the version it made.
    pub migrations: BTreeMap<u64, Xid>,
    /// The version whose migration made each heap; the heaps of the engine's own declarations
    /// are absent.
    pub born: BTreeMap<HeapId, u64>,
    /// The version the database is at.
    pub version: u64,
}

/// What a read of the past sees: the versions the snapshot of a held marker sees, taking no
/// SIREAD locks. Dropping it releases the marker.
struct AtMarker<R: Runtime> {
    pager: Pager<R>,
    txns: Arc<Transactions<R>>,
    marker: Marker,
}

impl<R: Runtime> Reads for AtMarker<R> {
    fn sees(&self, h: &Header) -> Result<bool, PageError> {
        self.marker
            .snapshot
            .sees(None, h.xmin, h.xmax, |x| self.txns.status(&self.pager, x))
    }

    fn scanned(&self, _: HeapNo) -> Result<(), SerializationFailure> {
        Ok(())
    }

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

    fn depends(&self, _: Xid) -> Result<(), exec::Error> {
        Ok(())
    }
}

impl<R: Runtime> Drop for AtMarker<R> {
    fn drop(&mut self) {
        self.txns.unpin(&self.marker);
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
fn position(v: &Val) -> Result<Option<Position>, exec::Error> {
    let [Val::Int(epoch), Val::Int(lsn)] = v.fields()? else {
        return Err(exec::Error::Internal(format!(
            "a position is an epoch and an LSN, found {v:?}"
        )));
    };
    Ok(u32::try_from(*epoch)
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
    /// Returns the marker `at` names, held.
    fn pin(&self, at: &Val, instant: bool) -> Result<Marker, exec::Error> {
        let pinned = match (instant, at) {
            (true, Val::Instant(t)) => self.txns.pin_time(&self.pager, wall(*t), self.upto),
            (true, other) => {
                return Err(exec::Error::Internal(format!(
                    "an `at instant` names an instant, found {other:?}"
                )));
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
            PastError::Damaged(d) => exec::Error::Corrupt(format!(
                "the record of the retention window is damaged: {d:?}"
            )),
        })
    }

    /// Returns whether the snapshot of `marker` sees the commit of `x`.
    fn sees_commit(&self, marker: &Marker, x: Xid) -> Result<bool, exec::Error> {
        Ok(marker
            .snapshot
            .counts_committed(x, |x| self.txns.status(&self.pager, x))?)
    }

    /// Returns the schema version at the state `marker` reads.
    fn version_at(&self, marker: &Marker) -> Result<u64, exec::Error> {
        for (&v, &x) in self.retained.migrations.iter().rev() {
            if self.sees_commit(marker, x)? {
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
            .map_err(|e| exec::Error::Corrupt(format!("{e:?}")))?;
        let mut oldest = self.upto;
        for m in markers
            .iter()
            .filter(|m| m.time >= cutoff && m.at <= self.upto)
        {
            match made {
                Some(x) if !self.sees_commit(m, x)? => break,
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
        let reads = AtMarker {
            pager: self.pager.clone(),
            txns: self.txns.clone(),
            marker: self.pin(at, instant)?,
        };
        let version = self.version_at(&reads.marker)?;
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
        if id != COMMIT_TIMES && id != SAFE_SNAPSHOTS {
            return Ok(self.volatile.get(&rel).cloned().unwrap_or_default());
        }
        let (Some(past), Some(cutoff)) = (self.txns.history(), self.txns.cutoff()) else {
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
    /// of the same constructors. Returns what the vacuum of the current heaps did.
    pub(super) fn vacuum_sum(
        &self,
        rel: &Rel,
        sum: &vacuum::Sum,
    ) -> Result<vacuum::Report, vacuum::Error> {
        let progress = vacuum::Progress::new();
        let kept = self
            .retained
            .as_ref()
            .filter(|_| self.txns.history().is_some());
        let Some(retained) = kept else {
            return vacuum::vacuum(&self.pager, &self.txns, sum, self.grant, &progress);
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
        let pasts: Vec<(CtorId, &crate::heap::Heap)> = sum
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
        let report =
            vacuum::vacuum_keeping(&self.pager, &self.txns, sum, &into, self.grant, &progress)?;
        let past = vacuum::Sum {
            heaps: pasts,
            indexes: vec![index],
        };
        vacuum::expire(&self.pager, &self.txns, &past, self.grant, &progress)?;
        Ok(report)
    }

    /// Returns the reads of the past of a statement reading commits up to `upto`, when the
    /// database keeps history structures.
    pub(super) fn pasts(&self, upto: Position) -> Option<Arc<dyn PastReads<R>>> {
        let retained = self.retained.clone()?;
        Some(Arc::new(Pasts {
            pager: self.pager.clone(),
            txns: self.txns.clone(),
            retained,
            heaps: self.heaps.clone(),
            indexes: self.indexes.clone(),
            upto,
        }))
    }

    /// Keep history for a retention window of `window` from now on: begin keeping it when it is
    /// not kept and `window` is not zero.
    ///
    /// Fails if the record of the window is damaged.
    pub fn set_retention(&self, window: Duration) -> Result<(), Error> {
        let _one = self.vacuuming.lock(&self.rt);
        if self.txns.history().is_some() {
            self.txns.set_window(window);
            return Ok(());
        }
        match &self.retained {
            Some(r) if !window.is_zero() => {
                self.txns.keep_history(&self.pager, r.record, window)?;
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
            Val::Int(i64::from(at.epoch)),
            Val::Int(i64::try_from(at.lsn.0).expect("commit LSN fits")),
        ],
    )
}
