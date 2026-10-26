mod layout;
mod root;
mod rows;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};

use adtb_lang::schema::{self, Schema};
use adtb_lang::types::RelId;

use crate::btree::{self, BTree, Index};
use crate::heap::{self, FieldType, Heap, LargeStore};
use crate::key::{CtorId, Dir, Entry, KeySchema, Scalar, Value};
use crate::mvcc::{self, CommitLog, Status, Transactions, Xid};
use crate::plan::{self, HeapId, IndexDesc, IndexId, RelationDesc};
use crate::runtime::Runtime;
use crate::storage::{self, PageError, PageKind, Pager, Pid};
use crate::verify::{Inventory, Sum};

pub use rows::{IndexSpec, Rows, declared, indexes};

use root::Root;

/// The heap id of the large-value store every catalog heap shares.
const LARGE: u32 = 1;
/// The heap id of the heap that names every other heap's and index's root.
const ROOTS: u32 = 2;
const FIRST_HEAP: u32 = 3;

/// The constructor of a row of the roots heap naming a heap.
const HEAP_ROOT: u32 = 0;
/// The constructor of a row of the roots heap naming an index.
const INDEX_ROOT: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Page(PageError),
    Heap(heap::Error),
    Index(btree::Error),
    Txn(mvcc::Error),
    /// The stored catalog has no root for a structure the engine declares; the one named.
    Missing(String),
}

impl From<PageError> for Error {
    fn from(e: PageError) -> Error {
        Error::Page(e)
    }
}

impl From<heap::Error> for Error {
    fn from(e: heap::Error) -> Error {
        Error::Heap(e)
    }
}

impl From<btree::Error> for Error {
    fn from(e: btree::Error) -> Error {
        Error::Index(e)
    }
}

impl From<mvcc::Error> for Error {
    fn from(e: mvcc::Error) -> Error {
        Error::Txn(e)
    }
}

/// The system and catalog relations of one database file, stored as heaps with their key and
/// unique indexes.
pub struct Catalog {
    schema: Schema,
    root: Pid,
    roots: Heap,
    heaps: BTreeMap<(RelId, u32), Heap>,
    indexes: BTreeMap<u64, (IndexSpec, Index)>,
}

impl std::fmt::Debug for Catalog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Catalog")
            .field("root", &self.root)
            .field("heaps", &self.heaps.len())
            .field("indexes", &self.indexes.len())
            .finish_non_exhaustive()
    }
}

/// Returns the schema of the engine's own declarations: the prelude, `catalog` and `sys`.
pub fn engine_schema() -> Schema {
    schema::build(&[]).expect("the engine's declarations build")
}

fn roots_row() -> Vec<FieldType> {
    let ids = |n: usize| vec![FieldType::Scalar(Scalar::I64); n];
    vec![FieldType::Sum(vec![
        heap::CtorType {
            id: CtorId(HEAP_ROOT),
            fields: ids(4),
        },
        heap::CtorType {
            id: CtorId(INDEX_ROOT),
            fields: ids(2),
        },
    ])]
}

fn int(n: u64) -> Value {
    Value::I64(i64::try_from(n).expect("an id fits 63 bits"))
}

fn uint(v: &Value) -> u64 {
    match v {
        Value::I64(n) => u64::try_from(*n).expect("a stored id is not negative"),
        other => panic!("a roots row holds ids, not {other:?}"),
    }
}

fn row_type(schema: &Schema, r: RelId, ctor: u32) -> Vec<FieldType> {
    layout::row(schema, r, ctor)
        .iter()
        .map(|(_, t)| layout::field_type(schema, t))
        .collect()
}

fn key_schema(schema: &Schema, spec: &IndexSpec) -> KeySchema {
    let row = layout::row(schema, spec.relation, spec.ctors[0]);
    KeySchema::new(
        spec.fields
            .iter()
            .map(|f| {
                let (_, t) = row
                    .iter()
                    .find(|(n, _)| n == f)
                    .unwrap_or_else(|| panic!("a row holds indexed field `{f}`"));
                (layout::key_type(schema, t), Dir::Asc)
            })
            .collect(),
    )
}

fn ctor_id(schema: &Schema, r: RelId, c: u32) -> u64 {
    schema.ids.ctors[&(schema.relation(r).elem_def, c)]
}

fn index_ctors(schema: &Schema, spec: &IndexSpec) -> BTreeSet<CtorId> {
    spec.ctors
        .iter()
        .map(|&c| CtorId(layout::ctor_id(ctor_id(schema, spec.relation, c))))
        .collect()
}

fn owner(index: u64) -> u32 {
    u32::try_from(index).expect("an index id fits 32 bits")
}

/// Returns whether a version written by `xmin` and deleted by `xmax` is committed and not
/// deleted by a committed transaction.
fn committed<R: Runtime>(
    txns: &Transactions<R>,
    pager: &Pager<R>,
    header: &heap::Header,
) -> Result<bool, PageError> {
    if txns.status(pager, Xid(header.xmin))? != Status::Committed {
        return Ok(false);
    }
    Ok(header.xmax == 0 || txns.status(pager, Xid(header.xmax))? != Status::Committed)
}

impl Catalog {
    /// Create the system and catalog relations of a new database in `pager`, fill them with the
    /// rows [`declared`] gives for the engine's own declarations, commit, and point the file
    /// header at the catalog.
    ///
    /// Precondition: `pager` holds a file no catalog was created in, with no page allocated.
    ///
    /// Postcondition: once it returns, [`Catalog::open`] after a crash finds the catalog. A
    /// crash before that leaves a file whose header names no catalog.
    pub fn create<R: Runtime>(
        rt: R,
        pager: &Pager<R>,
    ) -> Result<(Catalog, Transactions<R>), Error> {
        assert_eq!(
            Pid(0),
            pager.catalog_root(),
            "a catalog exists in this file"
        );
        let schema = engine_schema();
        let txns = Transactions::create(rt, pager)?;
        let large = LargeStore::create(pager, LARGE)?;
        let roots = Heap::create(pager, ROOTS, roots_row(), large)?;
        let mut heaps = BTreeMap::new();
        let mut next = FIRST_HEAP;
        for &r in schema.ids.relations.keys() {
            let n = schema.ty(schema.relation(r).elem_def).ctors().len() as u32;
            for c in 0..n {
                let heap = Heap::create(pager, next, row_type(&schema, r, c), large)?;
                heaps.insert((r, c), heap);
                next += 1;
            }
        }
        let mut stored = BTreeMap::new();
        for (id, spec) in indexes(&schema) {
            let index = Index::create(
                pager,
                owner(id),
                key_schema(&schema, &spec),
                index_ctors(&schema, &spec),
            )?;
            stored.insert(id, (spec, index));
        }
        let mut catalog = Catalog {
            schema,
            root: Pid(0),
            roots,
            heaps,
            indexes: stored,
        };
        let x = txns.begin(pager)?;
        catalog.write_roots(pager, x)?;
        for (r, rows) in declared(&catalog.schema) {
            for (c, row) in rows {
                catalog.insert(pager, x, r, c, &row)?;
            }
        }
        txns.commit(pager, x)?;
        let pid = pager.allocate(root::FORMAT, PageKind::Catalog, 0)?;
        let root = Root {
            commit_log: txns.commit_log().root(),
            large: (large.id(), large.root()),
            roots: (catalog.roots.id(), catalog.roots.root()),
        };
        pager.update(&[pid], |p| root.write(p[0]))?;
        pager.set_catalog_root(pid);
        catalog.root = pid;
        Ok((catalog, txns))
    }

    /// Open the catalog the file header of a recovered `pager` names, and the commit log it
    /// names. A file whose header names no catalog, one whose create did not finish, has every
    /// page freed and its catalog created again.
    pub fn open<R: Runtime>(rt: R, pager: &Pager<R>) -> Result<(Catalog, Transactions<R>), Error> {
        let pid = pager.catalog_root();
        if pid == Pid(0) {
            for p in pager.allocated()? {
                pager.free(p)?;
            }
            return Catalog::create(rt, pager);
        }
        let root = {
            let page = pager.read(pid)?;
            assert_eq!(
                (PageKind::Catalog as u16, root::FORMAT),
                (storage::kind(&page), storage::format(&page)),
                "the header's catalog root {pid:?} is a catalog root page"
            );
            Root::read(&page)
        };
        let txns = Transactions::open(rt, pager, root.commit_log)?;
        let large = LargeStore::open(root.large.0, root.large.1);
        let roots = Heap::open(pager, root.roots.0, root.roots.1, roots_row(), large)?;
        let mut versions = Vec::new();
        roots.scan(pager, |v| versions.push(v))?;
        let mut heap_roots = BTreeMap::new();
        let mut index_roots = BTreeMap::new();
        for v in versions {
            if !committed(&txns, pager, &v.header)? {
                continue;
            }
            let [Value::Sum { ctor, fields }] = v.row.as_slice() else {
                panic!("a roots row is one sum value")
            };
            match (*ctor, fields.as_slice()) {
                (HEAP_ROOT, [rel, c, id, at]) => {
                    let id = u32::try_from(uint(id)).expect("a heap id fits 32 bits");
                    heap_roots.insert((uint(rel), uint(c)), (id, Pid(uint(at))));
                }
                (INDEX_ROOT, [id, at]) => {
                    index_roots.insert(uint(id), Pid(uint(at)));
                }
                _ => panic!("a roots row names a heap or an index"),
            }
        }
        let schema = engine_schema();
        let mut heaps = BTreeMap::new();
        for (&r, &rid) in &schema.ids.relations {
            let n = schema.ty(schema.relation(r).elem_def).ctors().len() as u32;
            for c in 0..n {
                let cid = ctor_id(&schema, r, c);
                let Some(&(id, at)) = heap_roots.get(&(rid, cid)) else {
                    let name = &schema.relation(r).name;
                    return Err(Error::Missing(format!("heap of {name} constructor {cid}")));
                };
                let heap = Heap::open(pager, id, at, row_type(&schema, r, c), large)?;
                heaps.insert((r, c), heap);
            }
        }
        let mut stored = BTreeMap::new();
        for (id, spec) in indexes(&schema) {
            let Some(&at) = index_roots.get(&id) else {
                return Err(Error::Missing(format!("index {id}")));
            };
            let index = Index::open(
                BTree::open(at, owner(id)),
                key_schema(&schema, &spec),
                index_ctors(&schema, &spec),
            );
            stored.insert(id, (spec, index));
        }
        let catalog = Catalog {
            schema,
            root: pid,
            roots,
            heaps,
            indexes: stored,
        };
        Ok((catalog, txns))
    }

    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// Returns the page the file header names as the catalog's root.
    pub fn root(&self) -> Pid {
        self.root
    }

    pub fn heap(&self, r: RelId, ctor: u32) -> &Heap {
        self.heaps
            .get(&(r, ctor))
            .unwrap_or_else(|| panic!("relation {r:?} has a heap for constructor {ctor}"))
    }

    pub fn index_of(&self, id: u64) -> &Index {
        &self
            .indexes
            .get(&id)
            .unwrap_or_else(|| panic!("the catalog holds index {id}"))
            .1
    }

    /// Returns the committed rows of relation `r` no committed transaction deleted, each with
    /// its constructor index, by constructor and then in heap order.
    pub fn committed_rows<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        r: RelId,
    ) -> Result<Vec<(u32, Vec<Value>)>, PageError> {
        let mut out = Vec::new();
        for ((_, c), heap) in self.heaps.range((r, 0)..=(r, u32::MAX)) {
            let mut versions = Vec::new();
            heap.scan(pager, |v| versions.push(v))?;
            for v in versions {
                if committed(txns, pager, &v.header)? {
                    out.push((*c, v.row));
                }
            }
        }
        Ok(out)
    }

    /// Returns the planner's description of every system and catalog relation.
    pub fn plan(&self) -> plan::Catalog {
        let mut out = plan::Catalog::default();
        for &r in self.schema.ids.relations.keys() {
            let heaps = self
                .heaps
                .range((r, 0)..=(r, u32::MAX))
                .map(|((_, c), h)| (*c, HeapId(u64::from(h.id()))))
                .collect();
            let indexes = self
                .indexes
                .values()
                .filter(|(s, _)| s.relation == r)
                .map(|(s, _)| IndexDesc {
                    id: IndexId(s.id),
                    ctors: s.ctors.clone(),
                    fields: s.fields.clone(),
                    unique: s.unique,
                    ordered: true,
                })
                .collect();
            let desc = RelationDesc {
                heaps,
                key_index: IndexId(self.schema.ids.keys[&r]),
                indexes,
            };
            out.relations.insert(r, desc);
        }
        out
    }

    /// Returns every structure of the file for `verify`: the commit log `clog`, the catalog's
    /// heaps and indexes, and its root page.
    pub fn inventory<'a>(&'a self, clog: &'a CommitLog) -> Inventory<'a> {
        let mut sums = vec![Sum {
            heaps: vec![(CtorId(0), &self.roots)],
            indexes: Vec::new(),
        }];
        for &r in self.schema.ids.relations.keys() {
            let heaps = self
                .heaps
                .range((r, 0)..=(r, u32::MAX))
                .map(|((_, c), h)| (CtorId(layout::ctor_id(ctor_id(&self.schema, r, *c))), h))
                .collect();
            let indexes = self
                .indexes
                .values()
                .filter(|(s, _)| s.relation == r)
                .map(|(_, i)| i)
                .collect();
            sums.push(Sum { heaps, indexes });
        }
        Inventory {
            commit_log: Some(clog),
            sums,
            trees: Vec::new(),
            pages: vec![self.root],
        }
    }

    fn write_roots<R: Runtime>(&self, pager: &Pager<R>, x: Xid) -> Result<(), Error> {
        let mut rows = Vec::new();
        for ((r, c), h) in &self.heaps {
            let rel = self.schema.ids.relations[r];
            let fields = vec![
                int(rel),
                int(ctor_id(&self.schema, *r, *c)),
                int(u64::from(h.id())),
                int(h.root().0),
            ];
            rows.push((HEAP_ROOT, fields));
        }
        for (id, (_, index)) in &self.indexes {
            rows.push((INDEX_ROOT, vec![int(*id), int(index.tree().root().0)]));
        }
        for (ctor, fields) in rows {
            self.roots
                .insert(pager, x.0, &[Value::Sum { ctor, fields }])?;
        }
        Ok(())
    }

    /// Insert `row` of constructor `c` into relation `r` as transaction `x`, with an entry in
    /// every index over `c`.
    fn insert<R: Runtime>(
        &self,
        pager: &Pager<R>,
        x: Xid,
        r: RelId,
        c: u32,
        row: &[Value],
    ) -> Result<(), Error> {
        let heap = self
            .heaps
            .get(&(r, c))
            .unwrap_or_else(|| panic!("relation {r:?} has a heap for constructor {c}"));
        let tid = heap.insert(pager, x.0, row)?;
        let names = layout::row(&self.schema, r, c);
        let ctor = CtorId(layout::ctor_id(ctor_id(&self.schema, r, c)));
        for (spec, index) in self.indexes.values() {
            if spec.relation != r || !spec.ctors.contains(&c) {
                continue;
            }
            let key = spec
                .fields
                .iter()
                .map(|f| {
                    let at = names
                        .iter()
                        .position(|(n, _)| n == f)
                        .unwrap_or_else(|| panic!("a row holds indexed field `{f}`"));
                    row[at].clone()
                })
                .collect();
            let fresh = index.insert(pager, &Entry { key, ctor, tid })?;
            assert!(fresh, "a new row's entry is not in its index yet");
        }
        Ok(())
    }
}
