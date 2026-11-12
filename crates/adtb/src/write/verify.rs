use std::collections::BTreeSet;
use std::ops::Bound;
use std::sync::Arc;

use adtb_lang::ids::held_id;
use adtb_lang::types::RelId;

use crate::btree::{self, Index};
#[cfg(any(test, feature = "testing"))]
use crate::catalog::relation_holder;
use crate::exec::Store;
use crate::heap::Heap;
use crate::key::{CtorId, Tid};
use crate::mvcc::Safety;
#[cfg(any(test, feature = "testing"))]
use crate::plan::HeapId;
use crate::plan::IndexId;
use crate::runtime::Runtime;
#[cfg(any(test, feature = "testing"))]
use crate::storage::policy::Holder;
use crate::storage::{self, Damage, PageError, Pid};
use crate::verify::{Finding, Problem};

use super::schema::{Indexed, Rel};
use super::{Database, Error, HoldWait, Txn, VacuumHold};

/// What one run of `verify` covers, and so what it replaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerifyScope {
    Database,
    Relation(RelId),
}

impl VerifyScope {
    fn covers(self, rel: RelId) -> bool {
        match self {
            VerifyScope::Database => true,
            VerifyScope::Relation(r) => r == rel,
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
    /// A commit log status page settled from the log. It marks nothing.
    Settled {
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
            Damage::Checksum | Damage::Shape(_) => Found::Checksum {
                at,
                page: Some(pid),
            },
            Damage::Unreadable => Found::Checksum { at, page: None },
            Damage::Stale { .. } => Found::StalePage { at, page: pid },
            Damage::Settled { .. } => Found::Settled { at, page: pid },
            Damage::LogUnproven { .. } => Found::Structure { at },
        }
    }

    fn of_storage(f: storage::Finding) -> Found {
        match f {
            storage::Finding::Page { pid, damage } => Found::of_damage(pid, damage),
            storage::Finding::Orphan(pid) => Found::Structure {
                at: format!("page {}: no record of the retained log wrote it", pid.0),
            },
            storage::Finding::Log(l) => Found::Structure {
                at: format!("log at {}: {:?}", l.lsn.0, l.problem),
            },
            storage::Finding::Unlisted => Found::Structure {
                at: "the database: the damage list is full".to_owned(),
            },
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

    /// Returns the store reading the whole database at the safe snapshot.
    pub fn store(&self) -> Arc<Store<R>> {
        let db = self.db;
        let reader = (db.session.txns).reader(&db.pager, self.txn().hold(), None);
        Arc::new(Store::safe(reader, db.bindings.clone()))
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
        super::outside_writer();
        let mut tx = self.session.begin_reader(true)?;
        loop {
            let safety = self.session.ser.lock(&self.rt).safety(tx.sid);
            match safety {
                Safety::Safe => {
                    return Ok(VerifySnapshot {
                        db: self,
                        tx: Some(tx),
                    });
                }
                Safety::Unsafe => {
                    self.abort(tx)?;
                    tx = self.session.begin_reader(true)?;
                }
                Safety::Pending(writers) => match writers.first() {
                    Some(w) => self.session.waits.wait(None, *w)?,
                    None => self.rt.yield_now(),
                },
            }
        }
    }

    /// Check the pages of `scope` and, of the whole database, the retained log, while writers run.
    pub fn check_pages(&self, scope: VerifyScope) -> Result<Vec<Found>, Error> {
        let v = self.pager.verify_live();
        let mut out = Vec::new();
        let within = match scope {
            VerifyScope::Relation(rel) => Some(self.pages_of(rel, &mut out)?),
            VerifyScope::Database => None,
        };
        for f in v.findings {
            let page = match f {
                storage::Finding::Page { pid, .. }
                | storage::Finding::Orphan(pid)
                | storage::Finding::Log(storage::LogFinding {
                    problem: storage::LogProblem::Unlogged { pid, .. },
                    ..
                }) => Some(pid),
                storage::Finding::Log(_) | storage::Finding::Unlisted => None,
            };
            let wanted = match (&within, page) {
                (None, _) => true,
                (Some(w), Some(pid)) => w.contains(&pid),
                (Some(_), None) => false,
            };
            if wanted {
                out.push(Found::of_storage(f));
            }
        }
        Ok(out)
    }

    /// Returns what the retained log says of every page, for a scrub to check pages against.
    pub fn scrub_log(&self) -> storage::RetainedLog {
        self.pager.read_log(true)
    }

    /// Returns what the check of one page against `log` finds, while writers run.
    pub fn scrub_page(&self, pid: Pid, log: &storage::RetainedLog) -> Vec<Found> {
        let found = self.pager.check_page(pid, log);
        found.into_iter().map(Found::of_storage).collect()
    }

    fn pages_of(&self, rel: RelId, out: &mut Vec<Found>) -> Result<BTreeSet<Pid>, Error> {
        let mut pages = BTreeSet::new();
        let Some(r) = self.rels.get(&rel) else {
            return Ok(pages);
        };
        for c in &r.ctors {
            let heap = &self.bindings.heaps()[&c.heap].heap;
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
            match self.bindings.indexes()[id].index.tree().pages(&self.pager) {
                Ok(p) => pages.extend(p),
                Err(btree::Error::Page(e)) => out.push(Found::of_page(e)),
                Err(_) => {}
            }
        }
        Ok(pages)
    }

    /// Check each index of `scope`: its structure, every row version `tx` sees has its entry, and
    /// every entry names a row version of its key. Holds off vacuum across each index, since an
    /// entry read must keep naming the slot it named, stopping as [`Database::hold_vacuum`] says
    /// while another holds it off. Adds what it finds to `out`, a failure keeping what it found
    /// before.
    pub fn check_indexes(
        &self,
        tx: &Txn,
        scope: VerifyScope,
        waiting: HoldWait<'_>,
        out: &mut Vec<Found>,
    ) -> Result<(), Error> {
        self.index_checks(tx, scope, || self.hold_vacuum(waiting).map(Some), out)
    }

    /// Check each index of `scope` as [`Database::check_indexes`] does, under `_held`, the vacuum
    /// lock the caller holds.
    pub fn check_indexes_held(
        &self,
        tx: &Txn,
        scope: VerifyScope,
        _held: &VacuumHold<'_, R>,
        out: &mut Vec<Found>,
    ) -> Result<(), Error> {
        self.index_checks(tx, scope, || Ok(None), out)
    }

    fn index_checks<'a>(
        &'a self,
        tx: &Txn,
        scope: VerifyScope,
        hold: impl Fn() -> Result<Option<VacuumHold<'a, R>>, Error>,
        out: &mut Vec<Found>,
    ) -> Result<(), Error> {
        for r in self.rels.values() {
            if scope.covers(r.id) {
                for (id, ix) in &r.indexes {
                    let _no_vacuum = hold()?;
                    self.check_index(tx, r, *id, ix, out)?;
                }
            }
        }
        Ok(())
    }

    /// Check the links between the versions on each heap page of `scope`.
    pub fn check_links(&self, scope: VerifyScope) -> Vec<Found> {
        let mut out = Vec::new();
        for r in self.rels.values() {
            if scope.covers(r.id) {
                for c in &r.ctors {
                    self.links_of(&self.bindings.heaps()[&c.heap].heap, &mut out);
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
        let stored = &self.bindings.indexes()[&id];
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
            let heap = &self.bindings.heaps()[&r.ctors[c as usize].heap].heap;
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
                Err(Error::Exec(crate::exec::Error::Page(e))) => out.push(Found::of_page(e)),
                Err(e) => return Err(e),
            }
            held.insert((encoded, e.ctor, e.tid));
        }
        for (&ctor, &c) in &stored.ctors {
            let heap = self.bindings.heaps()[&r.ctors[c as usize].heap]
                .heap
                .clone();
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
        let reader = self.session.txns.reader(&self.pager, tx.hold(), None);
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
                let seen = reader.sees(&v.header.writers);
                if !seen.map_err(|e| v.refuse(&self.pager, e))?.visible() {
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
                        relation: held_id(self.schema.ids.relation_id(r.id)).raw(),
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
        let def = self.schema.relation(r.id).elem_def;
        let ctor = self.schema.ty(def).ctor_at(c);
        held_id(ctor.and_then(|c| self.schema.ids.ctor_id(c))).raw()
    }
}

#[cfg(any(test, feature = "testing"))]
impl<R: Runtime> Database<R> {
    /// Returns the heap of constructor `ctor` of relation `relation`, by qualified name.
    pub(crate) fn heap_named(&self, relation: &str, ctor: &str) -> Option<HeapId> {
        let r = self.rels.values().find(|r| r.qualified == relation)?;
        Some(r.ctors.iter().find(|c| c.name == ctor)?.heap)
    }

    /// Returns the key index of relation `relation`, by qualified name.
    pub(crate) fn key_index_named(&self, relation: &str) -> Option<IndexId> {
        Some(
            self.rels
                .values()
                .find(|r| r.qualified == relation)?
                .key_index,
        )
    }

    /// Returns the index of relation `relation`, by qualified name, of clause `clause`.
    pub(crate) fn index_named(&self, relation: &str, clause: &str) -> Option<IndexId> {
        let r = self.rels.values().find(|r| r.qualified == relation)?;
        let (id, _) = r.indexes.iter().find(|(_, ix)| ix.clause == clause)?;
        Some(*id)
    }

    /// Returns the relation index `id` belongs to.
    pub(crate) fn relation_of_index(&self, id: IndexId) -> Option<RelId> {
        let r = self
            .rels
            .values()
            .find(|r| r.indexes.iter().any(|(i, _)| *i == id))?;
        Some(r.id)
    }

    /// Returns the constructors key index `id` routes, `None` when `id` is no relation's key
    /// index.
    pub(crate) fn key_routes(&self, id: IndexId) -> Option<crate::exec::Routes> {
        let r = self.rels.values().find(|r| r.key_index == id)?;
        Some(r.routes(r.all_ctors()))
    }

    pub(crate) fn heap_of(&self, id: HeapId) -> Option<Arc<Heap>> {
        Some(self.bindings.heaps().get(&id)?.heap.clone())
    }

    /// Returns the history heap of heap `id`.
    pub(crate) fn history_heap_of(&self, id: HeapId) -> Option<Arc<Heap>> {
        Some(self.retained.as_ref()?.heaps.get(&id)?.heap.clone())
    }

    pub(crate) fn index_of(&self, id: IndexId) -> Option<Index> {
        Some(self.bindings.indexes().get(&id)?.index.clone())
    }

    /// Returns the history key index of the relation whose key index is `key`.
    pub(crate) fn history_index_of(&self, key: IndexId) -> Option<Index> {
        self.retained.as_ref()?.indexes.get(&key).cloned()
    }

    /// Returns the holder of the pages of heap `id` and its history heap.
    pub(crate) fn heap_holder(&self, id: HeapId) -> Option<Holder> {
        let r = self
            .rels
            .values()
            .find(|r| r.ctors.iter().any(|c| c.heap == id))?;
        let object = storage::Quarantine::Heap(self.bindings.heaps().get(&id)?.heap.id().0);
        Some(relation_holder(&self.schema, r.id, object))
    }

    /// Returns the heap of every constructor of every relation.
    pub(crate) fn heap_ids(&self) -> Vec<HeapId> {
        let ctors = self.rels.values().flat_map(|r| &r.ctors);
        ctors.map(|c| c.heap).collect()
    }

    /// Returns every index of every relation.
    pub(crate) fn index_ids(&self) -> Vec<IndexId> {
        let indexes = self.rels.values().flat_map(|r| &r.indexes);
        indexes.map(|(id, _)| *id).collect()
    }

    /// Returns the heap of the catalog's rows.
    pub(crate) fn kept_store(&self) -> Option<Arc<Heap>> {
        self.structures.rows()
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
        for problem in [
            Problem::Unlogged,
            Problem::Leaked,
            Problem::Unallocated,
            Problem::Shared("a".into(), "b".into()),
            Problem::Structure("a".into()),
        ] {
            assert!(
                matches!(found(problem.clone()), Some(Found::Structure { .. })),
                "{problem:?}"
            );
        }
        assert!(matches!(
            found(Problem::Damaged(Damage::Checksum)),
            Some(Found::Checksum { .. })
        ));
    }
}
