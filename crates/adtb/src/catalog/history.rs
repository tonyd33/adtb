//! The structures that keep the versions the retention window keeps: a history heap for each
//! constructor relation and a history key index for each relation, over a large-value store of
//! their own, and the record of the window.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use adtb_lang::schema::Schema;
use adtb_lang::types::RelId;

use crate::btree::{BTree, Index};
use crate::exec::StoredHeap;
use crate::heap::{Heap, LargeStore};
use crate::key::Tid;
use crate::mvcc::{Past, Scope, Transactions, Xid};
use crate::plan::{HeapId, IndexId};
use crate::runtime::Runtime;
use crate::storage::{Pager, Pid, Quarantine};
use crate::write::Retained;

use super::ids::{CatCtorId, CatRelId, ctor_id, ctor_index, held_id, rel_id, relation, stored};
use super::root::Root;
use super::roots::RootsRow;
use super::rot::Rot;
use super::structures::{Listed, index_holder, relation_holder};
use super::{Catalog, Error, IndexKeys, IndexSpec, Migrating, ROOTS, row_type};

/// The transaction of each migration and the roots version recording it, by the version it made,
/// and the version whose migration named each heap by heap id.
type Migrations = (BTreeMap<u64, (Xid, Tid)>, BTreeMap<u32, u64>);

/// The history structures of a catalog.
///
/// Invariants: `heaps` holds a heap for every constructor relation of the catalog, with the id
/// of the constructor's heap, and `indexes` an index for every relation with a key index, with
/// that index's key schema, constructors and page owner.
#[derive(Clone, Debug)]
pub(super) struct Kept {
    large: LargeStore,
    record: BTree,
    heaps: BTreeMap<(RelId, u32), (Arc<Heap>, u32)>,
    indexes: BTreeMap<RelId, (Index, u32)>,
}

/// What the rows of the roots heap name of the history structures.
#[derive(Debug, Default)]
pub(super) struct Names {
    heaps: BTreeMap<(CatRelId, CatCtorId), (u32, Pid)>,
    indexes: BTreeMap<CatRelId, (u32, Pid)>,
}

impl Names {
    /// Name history heap `at` of constructor relation `rc`.
    pub(super) fn heap(&mut self, rc: (CatRelId, CatCtorId), at: (u32, Pid)) {
        self.heaps.insert(rc, at);
    }

    /// Name history key index `at` of relation `rel`.
    pub(super) fn index(&mut self, rel: CatRelId, at: (u32, Pid)) {
        self.indexes.insert(rel, at);
    }
}

/// The catalog that a migration's build makes, as far as the history structures follow it.
pub(super) struct Rebuilding<'a, R: Runtime> {
    pub(super) m: &'a Migrating<'a, R>,
    pub(super) old: &'a Schema,
    pub(super) next: &'a Schema,
    pub(super) rewritten: &'a BTreeSet<CatRelId>,
    /// The key index of each relation of `next`.
    pub(super) keys: BTreeMap<RelId, &'a Index>,
}

impl Kept {
    /// Returns the history structures of a catalog that cannot be read: none, over no page.
    pub(super) fn unread() -> Kept {
        Kept {
            large: LargeStore::open(0, Pid(0)),
            record: BTree::open(Pid(0), 0),
            heaps: BTreeMap::new(),
            indexes: BTreeMap::new(),
        }
    }

    /// Allocate an empty large-value store with heap id `large` and an empty record whose pages
    /// carry `owner`, keeping history for no relation yet.
    pub(super) fn create<R: Runtime>(
        pager: &Pager<R>,
        large: u32,
        owner: u32,
    ) -> Result<Kept, Error> {
        Ok(Kept {
            large: LargeStore::create(pager, large)?,
            record: Past::create(pager, owner)?.tree(),
            heaps: BTreeMap::new(),
            indexes: BTreeMap::new(),
        })
    }

    /// Returns the history structures `root` and `names` name, for the constructor relations
    /// `heaps` holds and the relations `keys` gives a key index.
    ///
    /// Fail with [`Error::Missing`] when `names` names no history structure for one of them.
    pub(super) fn open<R: Runtime>(
        pager: &Pager<R>,
        schema: &Schema,
        root: &Root,
        names: &Names,
        heaps: &BTreeMap<(RelId, u32), Arc<Heap>>,
        keys: &BTreeMap<RelId, &Index>,
    ) -> Result<Kept, Error> {
        let large = LargeStore::open(root.history_large.0, root.history_large.1);
        let mut out = Kept {
            large,
            record: BTree::open(root.record.1, root.record.0),
            heaps: BTreeMap::new(),
            indexes: BTreeMap::new(),
        };
        for &(r, c) in heaps.keys() {
            let Some(&(id, at)) = names.heaps.get(&(rel_id(schema, r), ctor_id(schema, r, c)))
            else {
                let name = &schema.relation(r).name;
                return Err(Error::Missing(format!(
                    "history heap of {name} constructor {c}"
                )));
            };
            let heap = Heap::open(pager, id, at, row_type(schema, r, c), large)?;
            out.heaps
                .insert((r, c), (Arc::new(heap), heaps[&(r, c)].id().0));
        }
        for (&r, key) in keys {
            let Some(&(owner, at)) = names.indexes.get(&rel_id(schema, r)) else {
                let name = &schema.relation(r).name;
                return Err(Error::Missing(format!("history key index of {name}")));
            };
            let index = history_index(BTree::open(at, owner), key);
            out.indexes.insert(r, (index, key.tree().owner()));
        }
        Ok(out)
    }

    /// Name the large-value store and the record in `root`.
    pub(super) fn name(&self, root: &mut Root) {
        root.history_large = (self.large.id(), self.large.root());
        root.record = (self.record.owner(), self.record.root());
    }

    /// Returns the history structures of the catalog `b` builds: those of every relation it
    /// does not rewrite kept, and new ones for each relation it rewrites, with structure ids from
    /// `b.m`, given the heap of each constructor relation of `b.next` in `heaps`. Adds the roots row of each new structure to `named` and of each
    /// replaced one to `gone`, each as its constructor and fields.
    pub(super) fn rebuilt<R: Runtime>(
        &self,
        b: &Rebuilding<'_, R>,
        heaps: &BTreeMap<(RelId, u32), Arc<Heap>>,
        named: &mut Vec<RootsRow>,
        gone: &mut Vec<RootsRow>,
    ) -> Result<Kept, Error> {
        let mut out = Kept {
            large: self.large,
            record: self.record,
            heaps: BTreeMap::new(),
            indexes: BTreeMap::new(),
        };
        for (&(r, c), current) in heaps {
            let (rid, cid) = (rel_id(b.next, r), ctor_id(b.next, r, c));
            let of = current.id().0;
            if !b.rewritten.contains(&rid) {
                out.heaps
                    .insert((r, c), (self.heap_of(b.old, rid, cid), of));
                continue;
            }
            let heap = b.m.create_heap(row_type(b.next, r, c), self.large)?;
            named.push(heap_row(rid, cid, &heap));
            out.heaps.insert((r, c), (heap, of));
        }
        for (&r, key) in &b.keys {
            let rid = rel_id(b.next, r);
            if !b.rewritten.contains(&rid) {
                let was = relation(b.old, rid).expect("a relation kept has a history key index");
                let (index, _) = self.indexes[&was].clone();
                out.indexes.insert(r, (index, key.tree().owner()));
                continue;
            }
            let index = history_index(b.m.create_tree()?, key);
            named.push(index_row(rid, index.tree()));
            out.indexes.insert(r, (index, key.tree().owner()));
        }
        for (&(r, c), (heap, _)) in &self.heaps {
            let rid = rel_id(b.old, r);
            if b.rewritten.contains(&rid) {
                gone.push(heap_row(rid, ctor_id(b.old, r, c), heap));
            }
        }
        for (&r, (index, _)) in &self.indexes {
            let rid = rel_id(b.old, r);
            if b.rewritten.contains(&rid) {
                gone.push(index_row(rid, index.tree()));
            }
        }
        Ok(out)
    }

    /// Returns the large-value store of every history heap.
    pub(super) fn large(&self) -> LargeStore {
        self.large
    }

    /// Returns every history heap.
    pub(super) fn heaps(&self) -> impl Iterator<Item = &Arc<Heap>> {
        self.heaps.values().map(|(heap, _)| heap)
    }

    /// Returns the heap id or page owner of each history structure of a relation `of` holds.
    #[cfg(any(test, feature = "testing"))]
    pub(super) fn ids(&self, of: impl Fn(RelId) -> bool) -> Vec<u32> {
        let heaps = self.heaps.iter().filter(|((r, _), _)| of(*r));
        let indexes = self.indexes.iter().filter(|(r, _)| of(**r));
        heaps
            .map(|(_, (h, _))| h.id().0)
            .chain(indexes.map(|(_, (i, _))| i.tree().owner()))
            .collect()
    }

    fn heap_of(&self, old: &Schema, rid: CatRelId, cid: CatCtorId) -> Arc<Heap> {
        let r = relation(old, rid).expect("a relation kept has history heaps");
        let c = ctor_index(old, r, cid).expect("a constructor kept has a history heap");
        self.heaps[&(r, c)].0.clone()
    }

    /// Returns the history structures of `schema`'s relations, each history heap held as its
    /// constructor's heap and each history key index as its key index, and the record's tree.
    pub(super) fn structures(&self, schema: &Schema) -> (Vec<Listed>, BTree) {
        let mut sums: BTreeMap<RelId, Listed> = BTreeMap::new();
        for (&(r, c), (past, of)) in &self.heaps {
            let holder = relation_holder(schema, r, Quarantine::Heap(*of));
            let sum = sums.entry(r).or_default();
            sum.heap(stored(ctor_id(schema, r, c)), holder, past.clone());
        }
        for (r, (past, owner)) in &self.indexes {
            let holder = index_holder(schema, *r, *owner);
            sums.entry(*r).or_default().index(holder, past.clone());
        }
        (sums.into_values().collect(), self.record)
    }
}

/// Returns the key index of each relation of `schema` among `indexes`, by index id.
pub(super) fn key_indexes<'a>(
    schema: &Schema,
    indexes: &'a BTreeMap<u64, (IndexSpec, Index, Arc<IndexKeys>)>,
) -> BTreeMap<RelId, &'a Index> {
    schema
        .ids()
        .key_ids()
        .filter_map(|(r, id)| Some((r, &indexes.get(&id.raw())?.1)))
        .collect()
}

/// Returns an index over `tree` keyed and constructed as the key index `key` is.
fn history_index(tree: BTree, key: &Index) -> Index {
    Index::open(tree, key.schema().clone(), key.ctors().clone())
}

fn heap_row(rid: CatRelId, cid: CatCtorId, heap: &Heap) -> RootsRow {
    RootsRow::HistoryHeap {
        rel: rid,
        ctor: cid,
        heap: heap.id().0,
        at: heap.root(),
    }
}

fn index_row(rid: CatRelId, tree: BTree) -> RootsRow {
    RootsRow::HistoryIndex {
        rel: rid,
        owner: tree.owner(),
        at: tree.root(),
    }
}

impl Catalog {
    /// Returns the history structures of every relation, bound as the write path reads them,
    /// with the version each migration made and the version whose migration made each heap, as
    /// the latest commit holds them.
    pub fn retained<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
    ) -> Result<Retained, Error> {
        let schema = &self.schema;
        let pasts: BTreeMap<(RelId, u32), Arc<Heap>> = (self.kept.heaps.iter())
            .map(|(&rc, (past, _))| (rc, past.clone()))
            .collect();
        let heaps = self
            .kept
            .heaps
            .iter()
            .map(|(&(r, c), (past, of))| {
                let id = HeapId(u64::from(*of));
                let stored = StoredHeap {
                    row: row_type(schema, r, c),
                    heap: past.clone(),
                    sum: super::sum_of(&pasts, r),
                };
                (id, stored)
            })
            .collect();
        let indexes = self
            .kept
            .indexes
            .iter()
            .map(|(&r, (index, _))| {
                (
                    IndexId(held_id(schema.ids().key_id(r)).raw()),
                    index.clone(),
                )
            })
            .collect();
        let (migrations, made) = self.migrations(pager, txns)?;
        let born = self
            .heaps
            .values()
            .map(|h| {
                let id = h.id().0;
                let v = made.get(&id).ok_or(Rot::Nameless(id))?;
                Ok((HeapId(u64::from(id)), *v))
            })
            .collect::<Result<_, Error>>()?;
        Ok(Retained {
            record: Past::open(self.kept.record),
            heaps,
            indexes,
            migrations,
            born,
            version: self.source.version,
        })
    }

    /// Returns the transaction that applied each migration, by the version it made, and the
    /// version whose migration named each heap, by heap id, from the roots rows the latest commit
    /// holds: 0 for a heap the create named.
    ///
    /// Fail with damage when a heap is named by a transaction that applied no migration and is
    /// not the create's: one transaction, earlier than every migration's.
    fn migrations<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
    ) -> Result<Migrations, Error> {
        let hold = txns.take(Scope::Heaps(BTreeSet::from([ROOTS])))?;
        let rows = self.roots.rows(pager, txns, (&hold, None));
        drop(hold);
        let rows = rows?;
        let mut migrations = BTreeMap::new();
        let mut named = Vec::new();
        for (x, tid, row) in rows {
            match row {
                RootsRow::Migration { version, .. } => {
                    migrations.insert(version, (x, tid));
                }
                RootsRow::HeapRoot { heap, .. } => named.push((heap, x)),
                RootsRow::IndexRoot { .. }
                | RootsRow::OrderKey { .. }
                | RootsRow::HistoryHeap { .. }
                | RootsRow::HistoryIndex { .. }
                | RootsRow::Held { .. }
                | RootsRow::Dormant { .. }
                | RootsRow::Deployment(_)
                | RootsRow::Kept { .. } => {}
            }
        }
        let by_xid: BTreeMap<Xid, u64> = migrations.iter().map(|(v, (x, _))| (*x, *v)).collect();
        let first = by_xid.keys().next().copied();
        let mut created = None;
        let mut made = BTreeMap::new();
        for (heap, x) in named {
            let version = match by_xid.get(&x) {
                Some(v) => *v,
                // The create's transaction names heaps before any migration commits.
                None if created.is_none_or(|c| c == x) && first.is_none_or(|f| x < f) => {
                    created = Some(x);
                    0
                }
                None => return Err(Rot::Unborn(heap).into()),
            };
            made.insert(heap, version);
        }
        Ok((migrations, made))
    }
}
