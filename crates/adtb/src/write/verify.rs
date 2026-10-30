use std::collections::BTreeSet;
use std::ops::Bound;
use std::sync::Arc;

use adtb_lang::types::RelId;

use crate::btree::{self, Index};
use crate::exec::{AtSnapshot, Histories, Store};
use crate::heap::Heap;
use crate::key::{CtorId, Tid};
use crate::mvcc::Safety;
use crate::plan::IndexId;
use crate::runtime::Runtime;
use crate::storage::{self, Damage, PageError, Pid};
use crate::verify::{Finding, Problem};

use super::schema::{Indexed, Rel};
use super::{Database, Error, Txn};

/// What one run of `verify` covers, and so what it replaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerifyScope {
    Database,
    Relation(RelId),
    Scrub,
}

impl VerifyScope {
    fn covers(self, rel: RelId) -> bool {
        match self {
            VerifyScope::Database => true,
            VerifyScope::Relation(r) => r == rel,
            VerifyScope::Scrub => false,
        }
    }
}

/// One thing a run of `verify` found, each id a catalog id.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Found {
    Checksum {
        at: String,
        /// The page, unless an I/O failure rather than its bytes refused it.
        page: Option<Pid>,
    },
    StalePage {
        at: String,
        page: Pid,
    },
    Order {
        index: u64,
        at: String,
    },
    Orphan {
        index: u64,
        ctor: u64,
        at: String,
    },
    Missing {
        relation: u64,
        ctor: u64,
        index: u64,
        at: String,
    },
    Violation {
        constraint: u64,
        witness: String,
    },
    Structure {
        at: String,
    },
}

impl Found {
    fn of_damage(pid: Pid, damage: Damage) -> Found {
        let at = format!("page {}: {damage:?}", pid.0);
        match damage {
            Damage::Checksum => Found::Checksum {
                at,
                page: Some(pid),
            },
            Damage::Unreadable => Found::Checksum { at, page: None },
            Damage::Stale { .. } => Found::StalePage { at, page: pid },
        }
    }

    /// Returns the finding of `e`.
    pub fn of_page(e: PageError) -> Found {
        Found::of_damage(e.pid, e.damage)
    }

    /// Returns the finding of `f`, `None` for what a crash may leave on a sound file: an unsettled
    /// directory entry or an overcounted head.
    pub fn of_problem(f: &Finding) -> Option<Found> {
        match &f.problem {
            Problem::Damaged(d) => Some(Found::of_damage(f.pid, *d)),
            Problem::Unsettled(_) | Problem::Overcounted { .. } => None,
            other => Some(Found::Structure {
                at: format!("page {}: {other:?}", f.pid.0),
            }),
        }
    }
}

/// The transaction of a run of `verify`. Aborts it when dropped, a panic included.
#[derive(Debug)]
pub struct VerifySnapshot<'d, R: Runtime> {
    db: &'d Database<R>,
    tx: Option<Txn>,
}

impl<R: Runtime> VerifySnapshot<'_, R> {
    /// Returns the transaction.
    ///
    /// Panic if the snapshot has been dropped, which a borrow forbids.
    pub fn txn(&self) -> &Txn {
        self.tx
            .as_ref()
            .expect("a snapshot holds its transaction until dropped")
    }
}

impl<R: Runtime> Drop for VerifySnapshot<'_, R> {
    fn drop(&mut self) {
        if let Some(tx) = self.tx.take() {
            self.db.abort(tx).ok();
        }
    }
}

/// What one index holds, keyed as the rows of its constructors key it.
type Held = BTreeSet<(Vec<u8>, CtorId, Tid)>;

impl<R: Runtime> Database<R> {
    /// Begin the read-only transaction of a run of `verify`, waiting until its snapshot is safe.
    ///
    /// Precondition: the calling thread holds no transaction with an id.
    pub fn begin_verify(&self) -> Result<VerifySnapshot<'_, R>, Error> {
        let mut tx = self.begin_reader(true)?;
        loop {
            let safety = self.ser.lock(&self.rt).safety(tx.sid);
            match safety {
                Safety::Safe => {
                    return Ok(VerifySnapshot {
                        db: self,
                        tx: Some(tx),
                    });
                }
                Safety::Unsafe => {
                    self.abort(tx)?;
                    tx = self.begin_reader(true)?;
                }
                Safety::Pending(writers) => match writers.first() {
                    Some(w) => self.waits.wait(None, *w)?,
                    None => self.rt.yield_now(),
                },
            }
        }
    }

    /// Returns the store reading the whole database at the snapshot of `tx`.
    pub fn store_at(&self, tx: &Txn) -> Arc<Store<R>> {
        Arc::new(Store {
            pager: self.pager.clone(),
            reads: Arc::new(AtSnapshot {
                rt: self.rt.clone(),
                pager: self.pager.clone(),
                txns: self.txns.clone(),
                snapshot: tx.snapshot.clone(),
                reader: None,
                serial: None,
            }),
            heaps: self.heaps.clone(),
            indexes: self.indexes.clone(),
            history: Histories::default(),
            past: None,
        })
    }

    /// Check the pages of `scope` and, of the whole database, the retained log, while writers run.
    pub fn check_pages(&self, scope: VerifyScope) -> Result<Vec<Found>, Error> {
        let v = self.pager.verify_live();
        let mut out = Vec::new();
        let within = match scope {
            VerifyScope::Relation(rel) => Some(self.pages_of(rel, &mut out)?),
            VerifyScope::Database | VerifyScope::Scrub => None,
        };
        for f in v.findings {
            match f {
                storage::Finding::Page { pid, damage } => {
                    if within.as_ref().is_none_or(|w| w.contains(&pid)) {
                        out.push(Found::of_damage(pid, damage));
                    }
                }
                storage::Finding::Orphan(pid) => {
                    if within.as_ref().is_none_or(|w| w.contains(&pid)) {
                        out.push(Found::Structure {
                            at: format!("page {}: no record of the retained log wrote it", pid.0),
                        });
                    }
                }
                storage::Finding::Log(l) => {
                    if within.is_none() {
                        out.push(Found::Structure {
                            at: format!("log at {}: {:?}", l.lsn.0, l.problem),
                        });
                    }
                }
            }
        }
        Ok(out)
    }

    fn pages_of(&self, rel: RelId, out: &mut Vec<Found>) -> Result<BTreeSet<Pid>, Error> {
        let mut pages = BTreeSet::new();
        let Some(r) = self.rels.get(&rel) else {
            return Ok(pages);
        };
        for c in &r.ctors {
            let heap = &self.heaps[&c.heap].heap;
            for owned in [
                heap.owned_pages(&self.pager),
                heap.large().owned_pages(&self.pager),
            ] {
                match owned {
                    Ok(p) => pages.extend(p),
                    Err(e) => out.push(Found::of_page(e)),
                }
            }
        }
        for (id, _) in &r.indexes {
            match self.indexes[id].index.tree().pages(&self.pager) {
                Ok(p) => pages.extend(p),
                Err(btree::Error::Page(e)) => out.push(Found::of_page(e)),
                Err(_) => {}
            }
        }
        Ok(pages)
    }

    /// Check each index of `scope`: its structure, every row version `tx` sees has its entry, and
    /// every entry names a row version of its key. Holds off vacuum across each index, since an
    /// entry read must keep naming the slot it named.
    pub fn check_indexes(&self, tx: &Txn, scope: VerifyScope) -> Result<Vec<Found>, Error> {
        let mut out = Vec::new();
        for r in self.rels.values() {
            if scope.covers(r.id) {
                for (id, ix) in &r.indexes {
                    let _no_vacuum = self.vacuuming.lock(&self.rt);
                    self.check_index(tx, r, *id, ix, &mut out)?;
                }
            }
        }
        Ok(out)
    }

    /// Check the links between the versions on each heap page of `scope`.
    pub fn check_links(&self, scope: VerifyScope) -> Vec<Found> {
        let mut out = Vec::new();
        for r in self.rels.values() {
            if scope.covers(r.id) {
                for c in &r.ctors {
                    self.links_of(&self.heaps[&c.heap].heap, &mut out);
                }
            }
        }
        out
    }

    fn links_of(&self, heap: &Heap, out: &mut Vec<Found>) {
        let pages = match heap.pages(&self.pager) {
            Ok(p) => p,
            Err(e) => return out.push(Found::of_page(e)),
        };
        for pid in pages {
            match heap.broken_links(&self.pager, pid) {
                Ok(faults) => out.extend(faults.into_iter().map(|what| Found::Structure {
                    at: format!("page {}: {what}", pid.0),
                })),
                Err(e) => out.push(Found::of_page(e)),
            }
        }
    }

    fn check_index(
        &self,
        tx: &Txn,
        r: &Rel,
        id: IndexId,
        ix: &Indexed,
        out: &mut Vec<Found>,
    ) -> Result<(), Error> {
        let stored = &self.indexes[&id];
        let index = id.0;
        let tree = stored.index.tree();
        let tree_fault = |e: btree::Error, out: &mut Vec<Found>| match e {
            btree::Error::Page(e) => out.push(Found::of_page(e)),
            e => out.push(Found::Order {
                index,
                at: format!("root page {}: {e:?}", tree.root().0),
            }),
        };
        let before = out.len();
        match tree.check_live(&self.pager) {
            Ok(found) => out.extend(found.into_iter().map(|(pid, what)| Found::Order {
                index,
                at: format!("page {}: {what}", pid.0),
            })),
            Err(e) => tree_fault(e, out),
        }
        if out.len() > before {
            return Ok(());
        }
        let entries = match stored
            .index
            .range(&self.pager, Bound::Unbounded, Bound::Unbounded)
        {
            Ok(e) => e,
            Err(e) => {
                tree_fault(e, out);
                return Ok(());
            }
        };
        let schema = stored.index.schema();
        let types: Vec<_> = schema.components().iter().map(|(t, _)| t.clone()).collect();
        let mut held = Held::new();
        for e in entries {
            let encoded = schema.encode(&e.key);
            let at = format!("page {} slot {}", e.tid.page(), e.tid.slot());
            let Some(&c) = stored.ctors.get(&e.ctor) else {
                out.push(Found::Orphan {
                    index,
                    ctor: u64::from(e.ctor.0),
                    at: format!("{at}: a constructor outside the index"),
                });
                continue;
            };
            let heap = &self.heaps[&r.ctors[c as usize].heap].heap;
            let ok = (|| -> Result<bool, Error> {
                if !heap.holds(&self.pager, e.tid)? {
                    return Ok(false);
                }
                let Some(rows) = heap.chain_rows(&self.pager, e.tid)? else {
                    return Ok(true);
                };
                for row in &rows {
                    let key = ix.keys.key(c, row, &types)?;
                    if key.is_some_and(|k| schema.encode(&k) == encoded) {
                        return Ok(true);
                    }
                }
                Ok(rows.is_empty())
            })();
            match ok {
                Ok(true) => {}
                Ok(false) => out.push(Found::Orphan {
                    index,
                    ctor: self.ctor_id(r, c),
                    at: format!("{at}: holds no version of the entry's key"),
                }),
                Err(Error::Page(e)) => out.push(Found::of_page(e)),
                Err(e) => return Err(e),
            }
            held.insert((encoded, e.ctor, e.tid));
        }
        for (&ctor, &c) in &stored.ctors {
            let heap = self.heaps[&r.ctors[c as usize].heap].heap.clone();
            self.check_rows(
                tx,
                r,
                (index, &stored.index),
                ix,
                (ctor, c),
                &heap,
                &held,
                out,
            )?;
        }
        Ok(())
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "one call site; removing it needs a struct of the index under check"
    )]
    fn check_rows(
        &self,
        tx: &Txn,
        r: &Rel,
        (index, stored): (u64, &Index),
        ix: &Indexed,
        (ctor, c): (CtorId, u32),
        heap: &Heap,
        held: &Held,
        out: &mut Vec<Found>,
    ) -> Result<(), Error> {
        let schema = stored.schema();
        let types: Vec<_> = schema.components().iter().map(|(t, _)| t.clone()).collect();
        let pages = match heap.pages(&self.pager) {
            Ok(p) => p,
            Err(e) => {
                out.push(Found::of_page(e));
                return Ok(());
            }
        };
        for pid in pages {
            let mut versions = Vec::new();
            if let Err(e) = heap.scan_rooted(&self.pager, pid, |v, roots| versions.push((v, roots)))
            {
                out.push(Found::of_page(e));
                continue;
            }
            for (v, roots) in versions {
                let seen = self.txns.sees(
                    &self.pager,
                    &tx.snapshot,
                    None,
                    v.header.xmin,
                    v.header.xmax,
                )?;
                if !seen {
                    continue;
                }
                let Some(key) = ix.keys.key(c, &v.row, &types)? else {
                    continue;
                };
                let encoded = schema.encode(&key);
                if !roots
                    .into_iter()
                    .any(|root| held.contains(&(encoded.clone(), ctor, root)))
                {
                    out.push(Found::Missing {
                        relation: self.schema.ids.relations[&r.id],
                        ctor: self.ctor_id(r, c),
                        index,
                        at: format!("page {} slot {}", v.tid.page(), v.tid.slot()),
                    });
                }
            }
        }
        Ok(())
    }

    fn ctor_id(&self, r: &Rel, c: u32) -> u64 {
        self.schema.ids.ctors[&(self.schema.relation(r.id).elem_def, c)]
    }

    /// Returns the id of relation `relation`, by qualified name.
    pub fn relation_named(&self, relation: &str) -> Option<RelId> {
        self.rels
            .values()
            .find(|r| r.qualified == relation)
            .map(|r| r.id)
    }

    /// Returns the heap of constructor `ctor` of relation `relation`, by qualified name.
    pub fn heap_named(&self, relation: &str, ctor: &str) -> Option<Arc<Heap>> {
        let r = self.rels.values().find(|r| r.qualified == relation)?;
        let c = r.ctors.iter().find(|c| c.name == ctor)?;
        Some(self.heaps[&c.heap].heap.clone())
    }

    /// Returns the key index of relation `relation`, by qualified name, and its constructors.
    pub fn key_probe(&self, relation: &str) -> Option<(IndexId, BTreeSet<u32>)> {
        let r = self.rels.values().find(|r| r.qualified == relation)?;
        Some((r.key_index, r.all_ctors().into_iter().collect()))
    }

    /// Returns the history heap of constructor `ctor` of relation `relation`, by qualified name.
    pub fn history_heap_named(&self, relation: &str, ctor: &str) -> Option<Arc<Heap>> {
        let r = self.rels.values().find(|r| r.qualified == relation)?;
        let c = r.ctors.iter().find(|c| c.name == ctor)?;
        Some(self.retained.as_ref()?.heaps.get(&c.heap)?.heap.clone())
    }

    /// Returns the history key index of relation `relation`, by qualified name.
    pub fn history_index_named(&self, relation: &str) -> Option<Index> {
        let r = self.rels.values().find(|r| r.qualified == relation)?;
        self.retained.as_ref()?.indexes.get(&r.key_index).cloned()
    }

    /// Returns the index of relation `relation`, by qualified name, of clause `clause`.
    pub fn index_named(&self, relation: &str, clause: &str) -> Option<Index> {
        let r = self.rels.values().find(|r| r.qualified == relation)?;
        let (id, _) = r.indexes.iter().find(|(_, ix)| ix.clause == clause)?;
        Some(self.indexes[id].index.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(problem: Problem) -> Option<Found> {
        Found::of_problem(&Finding {
            pid: Pid(9),
            problem,
        })
    }

    #[test]
    fn what_a_crash_may_leave_is_no_finding() {
        assert_eq!(None, found(Problem::Unsettled("a directory".into())));
        let over = Problem::Overcounted {
            slot: 1,
            count: 2,
            versions: 1,
        };
        assert_eq!(None, found(over));
    }

    #[test]
    fn every_other_problem_is_a_finding() {
        assert!(matches!(
            found(Problem::Leaked),
            Some(Found::Structure { .. })
        ));
        assert!(matches!(
            found(Problem::Unallocated),
            Some(Found::Structure { .. })
        ));
        assert!(matches!(
            found(Problem::Damaged(Damage::Checksum)),
            Some(Found::Checksum { .. })
        ));
    }
}
