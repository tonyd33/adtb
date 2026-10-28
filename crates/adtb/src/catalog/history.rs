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
use crate::key::Value;
use crate::mvcc::{Past, Scope, Transactions, Xid};
use crate::plan::{HeapId, IndexId};
use crate::runtime::Runtime;
use crate::storage::{Pager, Pid};
use crate::verify::Sum;
use crate::write::Retained;

use super::ids::{CatCtorId, CatRelId, ctor_id, ctor_index, rel_id, relation};
use super::root::Root;
use super::{
    Catalog, Error, HEAP_ROOT, IndexKeys, IndexSpec, MIGRATION, ROOTS, int, row_type, uint,
};

/// The transaction of each migration by the version it made, and the version whose migration
/// named each heap by heap id.
type Migrations = (BTreeMap<u64, Xid>, BTreeMap<u32, u64>);

/// The constructor of a row of the roots heap naming a history heap.
pub(super) const HISTORY_HEAP: u32 = 4;
/// The constructor of a row of the roots heap naming a history key index.
pub(super) const HISTORY_INDEX: u32 = 5;

/// The history structures of a catalog.
///
/// Invariants: `heaps` holds a heap for every constructor relation of the catalog, and `indexes`
/// an index for every relation with a key index, with that index's key schema and constructors.
#[derive(Clone, Debug)]
pub(super) struct Kept {
    large: LargeStore,
    record: BTree,
    heaps: BTreeMap<(RelId, u32), Arc<Heap>>,
    indexes: BTreeMap<RelId, Index>,
}

/// What the rows of the roots heap name of the history structures.
#[derive(Debug, Default)]
pub(super) struct Names {
    heaps: BTreeMap<(CatRelId, CatCtorId), (u32, Pid)>,
    indexes: BTreeMap<CatRelId, (u32, Pid)>,
}

fn heap_id(v: &Value) -> Result<u32, Error> {
    let id = uint(v)?;
    u32::try_from(id).map_err(|_| Error::Corrupt(format!("heap id {id} is past 32 bits")))
}

impl Names {
    /// Add what roots row `fields` of constructor `ctor`, one of [`HISTORY_HEAP`] and
    /// [`HISTORY_INDEX`], names.
    pub(super) fn add(&mut self, ctor: u32, fields: &[Value]) -> Result<(), Error> {
        match (ctor, fields) {
            (HISTORY_HEAP, [rel, c, id, at]) => {
                let at = (heap_id(id)?, Pid(uint(at)?));
                self.heaps
                    .insert((CatRelId(uint(rel)?), CatCtorId(uint(c)?)), at);
            }
            (HISTORY_INDEX, [rel, owner, at]) => {
                let at = (heap_id(owner)?, Pid(uint(at)?));
                self.indexes.insert(CatRelId(uint(rel)?), at);
            }
            (ctor, fields) => {
                return Err(Error::Corrupt(format!(
                    "a roots row of constructor {ctor} holds {fields:?}"
                )));
            }
        }
        Ok(())
    }
}

/// The catalog that a migration's build makes, as far as the history structures follow it.
pub(super) struct Rebuilding<'a, R: Runtime> {
    pub(super) pager: &'a Pager<R>,
    pub(super) old: &'a Schema,
    pub(super) next: &'a Schema,
    pub(super) rewritten: &'a BTreeSet<CatRelId>,
    /// The key index of each relation of `next`.
    pub(super) keys: BTreeMap<RelId, &'a Index>,
}

impl Kept {
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
            out.heaps.insert((r, c), Arc::new(heap));
        }
        for (&r, key) in keys {
            let Some(&(owner, at)) = names.indexes.get(&rel_id(schema, r)) else {
                let name = &schema.relation(r).name;
                return Err(Error::Missing(format!("history key index of {name}")));
            };
            out.indexes
                .insert(r, history_index(BTree::open(at, owner), key));
        }
        Ok(out)
    }

    /// Name the large-value store and the record in `root`.
    pub(super) fn name(&self, root: &mut Root) {
        root.history_large = (self.large.id(), self.large.root());
        root.record = (self.record.owner(), self.record.root());
    }

    /// Returns the history structures of the catalog `b` builds: those of every relation it
    /// does not rewrite kept, and new ones for each relation it rewrites, with heap ids and owners
    /// from `next_heap` on. Adds the roots row of each new structure to `named` and of each
    /// replaced one to `gone`, each as its constructor and fields.
    pub(super) fn rebuilt<R: Runtime>(
        &self,
        b: &Rebuilding<'_, R>,
        next_heap: &mut u32,
        named: &mut Vec<(u32, Vec<Value>)>,
        gone: &mut Vec<(u32, Vec<Value>)>,
    ) -> Result<Kept, Error> {
        let mut out = Kept {
            large: self.large,
            record: self.record,
            heaps: BTreeMap::new(),
            indexes: BTreeMap::new(),
        };
        let mut fresh = || {
            *next_heap += 1;
            *next_heap - 1
        };
        for (r, c) in super::ctor_relations(b.next) {
            let (rid, cid) = (rel_id(b.next, r), ctor_id(b.next, r, c));
            if !b.rewritten.contains(&rid) {
                out.heaps.insert((r, c), self.heap_of(b.old, rid, cid));
                continue;
            }
            let heap = Heap::create(b.pager, fresh(), row_type(b.next, r, c), self.large)?;
            named.push((HISTORY_HEAP, heap_row(rid, cid, &heap)));
            out.heaps.insert((r, c), Arc::new(heap));
        }
        for (&r, key) in &b.keys {
            let rid = rel_id(b.next, r);
            if !b.rewritten.contains(&rid) {
                let was = relation(b.old, rid).expect("a relation kept has a history key index");
                out.indexes.insert(r, self.indexes[&was].clone());
                continue;
            }
            let index = history_index(BTree::create(b.pager, fresh())?, key);
            named.push((HISTORY_INDEX, index_row(rid, index.tree())));
            out.indexes.insert(r, index);
        }
        for (&(r, c), heap) in &self.heaps {
            let rid = rel_id(b.old, r);
            if b.rewritten.contains(&rid) {
                gone.push((HISTORY_HEAP, heap_row(rid, ctor_id(b.old, r, c), heap)));
            }
        }
        for (&r, index) in &self.indexes {
            let rid = rel_id(b.old, r);
            if b.rewritten.contains(&rid) {
                gone.push((HISTORY_INDEX, index_row(rid, index.tree())));
            }
        }
        Ok(out)
    }

    fn heap_of(&self, old: &Schema, rid: CatRelId, cid: CatCtorId) -> Arc<Heap> {
        let r = relation(old, rid).expect("a relation kept has history heaps");
        let c = ctor_index(old, r, cid).expect("a constructor kept has a history heap");
        self.heaps[&(r, c)].clone()
    }

    /// Returns the sums of the history structures of `schema`'s relations for `verify`, and the
    /// record's tree.
    pub(super) fn inventory<'a>(&'a self, schema: &Schema) -> (Vec<Sum<'a>>, BTree) {
        let mut sums: BTreeMap<RelId, Sum<'a>> = BTreeMap::new();
        for (&(r, c), heap) in &self.heaps {
            let sum = sums.entry(r).or_insert_with(|| Sum {
                heaps: Vec::new(),
                indexes: Vec::new(),
            });
            sum.heaps
                .push((ctor_id(schema, r, c).stored(), heap.as_ref()));
        }
        for (r, index) in &self.indexes {
            if let Some(sum) = sums.get_mut(r) {
                sum.indexes.push(index);
            }
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
        .ids
        .keys
        .iter()
        .filter_map(|(&r, id)| Some((r, &indexes.get(id)?.1)))
        .collect()
}

/// Returns an index over `tree` keyed and constructed as the key index `key` is.
fn history_index(tree: BTree, key: &Index) -> Index {
    Index::open(tree, key.schema().clone(), key.ctors().clone())
}

fn heap_row(rid: CatRelId, cid: CatCtorId, heap: &Heap) -> Vec<Value> {
    vec![
        int(rid.0),
        int(cid.0),
        int(u64::from(heap.id().0)),
        int(heap.root().0),
    ]
}

fn index_row(rid: CatRelId, tree: BTree) -> Vec<Value> {
    vec![int(rid.0), int(u64::from(tree.owner())), int(tree.root().0)]
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
        let heaps = self
            .kept
            .heaps
            .iter()
            .map(|(&(r, c), past)| {
                let id = HeapId(u64::from(self.heap(r, c).id().0));
                let stored = StoredHeap {
                    row: row_type(schema, r, c),
                    heap: past.clone(),
                    sum: super::sum_of(&self.kept.heaps, r),
                };
                (id, stored)
            })
            .collect();
        let indexes = self
            .kept
            .indexes
            .iter()
            .map(|(r, index)| (IndexId(schema.ids.keys[r]), index.clone()))
            .collect();
        let (migrations, made) = self.migrations(pager, txns)?;
        let born = self
            .heaps
            .values()
            .filter_map(|h| {
                let v = made.get(&h.id().0)?;
                Some((HeapId(u64::from(h.id().0)), *v))
            })
            .collect();
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
    /// holds.
    fn migrations<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
    ) -> Result<Migrations, Error> {
        let (id, snapshot) = txns.take(Scope::Heaps(BTreeSet::from([ROOTS])))?;
        let mut versions = Vec::new();
        let scanned = self.roots.scan(pager, |v| versions.push(v));
        let mut rows = Vec::new();
        let seen = scanned.map_err(Error::from).and_then(|()| {
            for v in versions {
                if txns.sees(pager, &snapshot, None, v.header.xmin, v.header.xmax)? {
                    rows.push((Xid(v.header.xmin), v.row));
                }
            }
            Ok(())
        });
        txns.release(id);
        seen?;
        let mut migrations = BTreeMap::new();
        let mut named = Vec::new();
        for (x, row) in rows {
            let [Value::Sum { ctor, fields }] = row.as_slice() else {
                return Err(Error::Corrupt(format!("a roots row is {row:?}")));
            };
            match (*ctor, fields.as_slice()) {
                (MIGRATION, [version, _]) => {
                    migrations.insert(uint(version)?, x);
                }
                (HEAP_ROOT, [_, _, heap, _]) => named.push((heap_id(heap)?, x)),
                _ => {}
            }
        }
        let by_xid: BTreeMap<Xid, u64> = migrations.iter().map(|(v, x)| (*x, *v)).collect();
        let made = named
            .into_iter()
            .filter_map(|(heap, x)| Some((heap, *by_xid.get(&x)?)))
            .collect();
        Ok((migrations, made))
    }
}
