pub mod layout;
mod root;
mod rows;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};

use adtb_lang::ast::OperatorKind;
use adtb_lang::evolve::Source;
use adtb_lang::parse::{Options, parse_migration};
use adtb_lang::schema::{self, Schema};
use adtb_lang::types::RelId;

use crate::btree::{self, BTree, Index};
use crate::heap::{self, FieldType, Heap, LargeStore};
use crate::key::{CtorId, Dir, Entry, KeySchema, OrderKey, Scalar, Value};
use crate::mvcc::{self, CommitLog, Scope, Snapshot, Status, Transactions, Xid};
use crate::plan::{self, HeapId, IndexDesc, IndexId, RelationDesc};
use crate::runtime::Runtime;
use crate::storage::{self, Lsn, PageError, PageKind, Pager, Pid};
use crate::verify::{self, Depth, Inventory, Sum};

pub use layout::OrderKeys;
pub use rows::{IndexSpec, Rows, declared, indexed_fields, indexes};

use root::Root;

/// The heap id of the large-value store every heap shares.
const LARGE: u32 = 1;
/// The heap id of the heap that names every other heap's and index's root.
const ROOTS: u32 = 2;
const FIRST_HEAP: u32 = 3;

/// The constructor of a row of the roots heap naming a heap.
const HEAP_ROOT: u32 = 0;
/// The constructor of a row of the roots heap naming an index.
const INDEX_ROOT: u32 = 1;
/// The constructor of a row of the roots heap holding a constructor's order key.
const ORDER_KEY: u32 = 2;
/// The constructor of a row of the roots heap holding a migration's text.
const MIGRATION: u32 = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Page(PageError),
    Heap(heap::Error),
    Index(btree::Error),
    Txn(mvcc::Error),
    /// The stored catalog has no root for a structure the engine declares; the one named.
    Missing(String),
    /// The migration does not apply; each reason.
    Refused(Vec<String>),
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

/// The relations of one database file, the system and catalog relations and those its
/// migrations declare, stored as heaps with their key, unique and plain indexes.
pub struct Catalog {
    source: Source,
    schema: Schema,
    root: Pid,
    large: LargeStore,
    roots: Heap,
    heaps: BTreeMap<(RelId, u32), Heap>,
    indexes: BTreeMap<u64, (IndexSpec, Index)>,
    keys: OrderKeys,
    next_heap: u32,
}

impl std::fmt::Debug for Catalog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Catalog")
            .field("version", &self.source.version)
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
    let ctor = |id: u32, fields: Vec<FieldType>| heap::CtorType {
        id: CtorId(id),
        fields,
    };
    vec![FieldType::Sum(vec![
        ctor(HEAP_ROOT, ids(4)),
        ctor(INDEX_ROOT, ids(2)),
        ctor(
            ORDER_KEY,
            vec![
                FieldType::Scalar(Scalar::I64),
                FieldType::Scalar(Scalar::Bytes),
            ],
        ),
        ctor(
            MIGRATION,
            vec![
                FieldType::Scalar(Scalar::I64),
                FieldType::Scalar(Scalar::Text),
            ],
        ),
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

fn key_schema(schema: &Schema, keys: &OrderKeys, spec: &IndexSpec) -> KeySchema {
    let row = layout::row(schema, spec.relation, spec.ctors[0]);
    KeySchema::new(
        spec.fields
            .iter()
            .map(|f| {
                let (_, t) = row
                    .iter()
                    .find(|(n, _)| n == f)
                    .unwrap_or_else(|| panic!("a row holds indexed field `{f}`"));
                (layout::key_type(schema, keys, t), Dir::Asc)
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

fn ctor_count(schema: &Schema, r: RelId) -> u32 {
    schema.ty(schema.relation(r).elem_def).ctors().len() as u32
}

/// Returns the constructor relations of `schema` that have catalog ids, by relation and
/// constructor index.
fn ctor_relations(schema: &Schema) -> Vec<(RelId, u32)> {
    schema
        .ids
        .relations
        .keys()
        .flat_map(|&r| (0..ctor_count(schema, r)).map(move |c| (r, c)))
        .collect()
}

/// Update the root page `pid` with `f`.
fn update_root<R: Runtime>(
    pager: &Pager<R>,
    pid: Pid,
    f: impl FnOnce(&mut Root),
) -> Result<Lsn, PageError> {
    pager.update(&[pid], |p| {
        let mut root = Root::read(p[0]);
        f(&mut root);
        root.write(p[0]);
    })
}

/// What the rows of the roots heap visible at a snapshot name.
#[derive(Default)]
struct Named {
    /// The heap id and root of each constructor relation, by relation id and constructor id.
    heaps: BTreeMap<(u64, u64), (u32, Pid)>,
    indexes: BTreeMap<u64, Pid>,
    keys: OrderKeys,
    /// The text of each migration, by version.
    migrations: BTreeMap<u64, String>,
}

/// Returns what the rows of `roots` that every committed transaction wrote name.
fn named<R: Runtime>(
    pager: &Pager<R>,
    txns: &Transactions<R>,
    roots: &Heap,
) -> Result<Named, Error> {
    let (id, snapshot) = txns.take(Scope::Heaps(BTreeSet::from([ROOTS])))?;
    let rows = visible(pager, txns, &snapshot, roots);
    txns.release(id);
    let mut out = Named::default();
    for row in rows? {
        let [Value::Sum { ctor, fields }] = row.as_slice() else {
            panic!("a roots row is one sum value")
        };
        match (*ctor, fields.as_slice()) {
            (HEAP_ROOT, [rel, c, id, at]) => {
                let id = u32::try_from(uint(id)).expect("a heap id fits 32 bits");
                out.heaps.insert((uint(rel), uint(c)), (id, Pid(uint(at))));
            }
            (INDEX_ROOT, [id, at]) => {
                out.indexes.insert(uint(id), Pid(uint(at)));
            }
            (ORDER_KEY, [c, Value::Bytes(key)]) => {
                let key = OrderKey::new(key.clone()).expect("a stored order key is valid");
                out.keys.insert(uint(c), key);
            }
            (MIGRATION, [version, Value::Text(text)]) => {
                out.migrations.insert(uint(version), text.clone());
            }
            _ => panic!("a roots row names a heap, an index, an order key or a migration"),
        }
    }
    Ok(out)
}

/// Returns the rows of the versions of `heap` visible at `snapshot`, in heap order.
fn visible<R: Runtime>(
    pager: &Pager<R>,
    txns: &Transactions<R>,
    snapshot: &Snapshot,
    heap: &Heap,
) -> Result<Vec<Vec<Value>>, PageError> {
    let mut versions = Vec::new();
    heap.scan(pager, |v| versions.push(v))?;
    let mut out = Vec::new();
    for v in versions {
        if txns.sees(pager, snapshot, None, v.header.xmin, v.header.xmax)? {
            out.push(v.row);
        }
    }
    Ok(out)
}

/// Returns the source the migrations `texts` fold to, in version order from the empty schema.
fn fold(texts: &BTreeMap<u64, String>) -> Source {
    let mut source = Source::default();
    for (version, text) in texts {
        let file = parse_migration(text, Options::default())
            .ok()
            .unwrap_or_else(|ds| panic!("stored migration {version} does not parse: {ds:?}"));
        source = source
            .migrate(&file, text)
            .unwrap_or_else(|r| panic!("stored migration {version} does not apply: {r:?}"));
    }
    source
}

/// Returns whether a first migration may hold `op`: it declares an object and needs no data.
fn declares(op: &OperatorKind) -> bool {
    matches!(
        op,
        OperatorKind::AddModule(_)
            | OperatorKind::AddType(_)
            | OperatorKind::AddRelation(_)
            | OperatorKind::AddSet(_)
            | OperatorKind::AddUnique(..)
            | OperatorKind::AddIndex { .. }
            | OperatorKind::DefineFn(_)
            | OperatorKind::DefineAggregate(_)
            | OperatorKind::AddConstraint(_)
            | OperatorKind::AddCheck(_)
    )
}

impl Catalog {
    /// Create the system and catalog relations of a new database in `pager`, fill them with the
    /// rows [`declared`] gives for the engine's own declarations, commit, and point the file
    /// header at the catalog.
    ///
    /// Precondition: `pager` holds a file no catalog was created in, with no page allocated.
    ///
    /// Postcondition: once it returns, [`Catalog::open`] after a crash finds the catalog. A
    /// crash before that leaves a file whose header names no catalog, or names a root page
    /// that names no roots heap.
    pub fn create<R: Runtime>(
        rt: R,
        pager: &Pager<R>,
    ) -> Result<(Catalog, Transactions<R>), Error> {
        assert_eq!(
            Pid(0),
            pager.catalog_root(),
            "a catalog exists in this file"
        );
        let txns = Transactions::create(rt, pager)?;
        let pid = pager.allocate(root::FORMAT, PageKind::Catalog, 0)?;
        let root = Root {
            commit_log: txns.commit_log().root(),
            large: (0, Pid(0)),
            roots: (0, Pid(0)),
            next_heap: FIRST_HEAP,
            changing: 0,
        };
        pager.update(&[pid], |p| root.write(p[0]))?;
        pager.set_catalog_root(pid);
        let catalog = Catalog::fill(pager, &txns, pid)?;
        Ok((catalog, txns))
    }

    /// Create the engine's structures and rows in a file whose root page `pid` names the commit
    /// log of `txns` and nothing else, and name them in the root page.
    fn fill<R: Runtime>(
        pager: &Pager<R>,
        txns: &Transactions<R>,
        pid: Pid,
    ) -> Result<Catalog, Error> {
        let large = LargeStore::create(pager, LARGE)?;
        let roots = Heap::create(pager, ROOTS, roots_row(), large)?;
        let named = (roots.id(), roots.root());
        let mut catalog = Catalog {
            source: Source::default(),
            schema: engine_schema(),
            root: pid,
            large,
            roots,
            heaps: BTreeMap::new(),
            indexes: BTreeMap::new(),
            keys: OrderKeys::new(),
            next_heap: FIRST_HEAP,
        };
        let x = txns.begin(pager)?;
        let engine = catalog.source.clone();
        catalog.extend(pager, x, engine, Rows::new())?;
        txns.commit(pager, x)?;
        let lsn = update_root(pager, pid, |r| {
            r.large = (large.id(), large.root());
            r.roots = named;
            r.next_heap = catalog.next_heap;
        })?;
        pager.wait_durable(lsn);
        Ok(catalog)
    }

    /// Open the catalog the file header of a recovered `pager` names, and the commit log it
    /// names, with every committed migration applied.
    ///
    /// A file whose create did not commit has every page but its commit log's and its root
    /// page freed, and its catalog created again; a file whose header names no catalog has
    /// every page freed first. A schema change that did not commit has every page it allocated
    /// freed.
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
        if root.roots.1 == Pid(0) {
            let mut kept = BTreeSet::from([pid]);
            {
                let clog = txns.commit_log();
                kept.insert(clog.root());
                kept.extend(clog.status_pages().values().copied());
                kept.extend(clog.directory_pages());
            }
            for p in pager.allocated()? {
                if !kept.contains(&p) {
                    pager.free(p)?;
                }
            }
            let catalog = Catalog::fill(pager, &txns, pid)?;
            return Ok((catalog, txns));
        }
        let large = LargeStore::open(root.large.0, root.large.1);
        let roots = Heap::open(pager, root.roots.0, root.roots.1, roots_row(), large)?;
        let named = named(pager, &txns, &roots)?;
        let source = fold(&named.migrations);
        let schema = source.schema().expect("a folded log builds");
        let mut heaps = BTreeMap::new();
        for (r, c) in ctor_relations(&schema) {
            let rid = schema.ids.relations[&r];
            let cid = ctor_id(&schema, r, c);
            let Some(&(id, at)) = named.heaps.get(&(rid, cid)) else {
                let name = &schema.relation(r).name;
                return Err(Error::Missing(format!("heap of {name} constructor {cid}")));
            };
            let heap = Heap::open(pager, id, at, row_type(&schema, r, c), large)?;
            heaps.insert((r, c), heap);
        }
        let mut stored = BTreeMap::new();
        for (id, spec) in indexes(&schema) {
            let Some(&at) = named.indexes.get(&id) else {
                return Err(Error::Missing(format!("index {id}")));
            };
            let index = Index::open(
                BTree::open(at, owner(id)),
                key_schema(&schema, &named.keys, &spec),
                index_ctors(&schema, &spec),
            );
            stored.insert(id, (spec, index));
        }
        let catalog = Catalog {
            source,
            schema,
            root: pid,
            large,
            roots,
            heaps,
            indexes: stored,
            keys: named.keys,
            next_heap: root.next_heap,
        };
        if root.changing != 0 {
            if txns.status(pager, Xid(root.changing))? != Status::Committed {
                let report = {
                    let clog = txns.commit_log();
                    verify::verify_with(pager, &catalog.inventory(&clog), Depth::STRUCTURE)
                };
                if report.complete {
                    for p in report.leaked() {
                        pager.free(p)?;
                    }
                }
            }
            update_root(pager, pid, |r| r.changing = 0)?;
        }
        Ok((catalog, txns))
    }

    /// Apply `text`, the first migration of the database, as one transaction: create a heap for
    /// each constructor of every relation it declares and each of their indexes, write the
    /// catalog rows of every object it declares, and keep its text. Returns once the migration
    /// is durable.
    ///
    /// Fails with [`Error::Refused`], changing nothing, if a migration was applied before, if
    /// `text` holds an operator other than one that declares an object, or if the migration does
    /// not apply to the empty schema. On any other failure the transaction is aborted, and the
    /// catalog must be opened again.
    ///
    /// Postcondition: [`Catalog::open`] after a crash finds either the whole migration or
    /// nothing of it, and no page it allocated unless it committed.
    pub fn migrate<R: Runtime>(
        &mut self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        text: &str,
    ) -> Result<(), Error> {
        let refused = |why: Vec<String>| Err(Error::Refused(why));
        if self.source.version != 0 {
            return refused(vec![format!(
                "the schema is at version {}; only a first migration applies",
                self.source.version
            )]);
        }
        let file = match parse_migration(text, Options::default()).ok() {
            Ok(f) => f,
            Err(ds) => return refused(ds.into_iter().map(|d| d.message).collect()),
        };
        let others: Vec<String> = file
            .operators
            .iter()
            .filter(|op| !declares(&op.kind))
            .map(|op| format!("`{}` declares nothing", &text[op.span.range()]))
            .collect();
        if !others.is_empty() {
            return refused(others);
        }
        let next = match self.source.migrate(&file, text) {
            Ok(s) => s,
            Err(r) => {
                let ops = r.operators.into_iter().map(|d| d.message);
                return refused(
                    ops.chain(r.recheck.into_iter().map(|d| d.diag.message))
                        .collect(),
                );
            }
        };
        let schema = next.schema().expect("an applied migration builds");
        let over: Vec<String> = schema
            .relations
            .iter()
            .flat_map(|r| &r.indexes)
            .filter(|d| indexed_fields(d).is_none())
            .map(|_| "an index over an expression other than a field".to_string())
            .collect();
        if !over.is_empty() {
            return refused(over);
        }
        let x = txns.begin(pager)?;
        let added = ctor_relations(&schema)
            .into_iter()
            .filter(|rc| !self.heaps.contains_key(rc))
            .count() as u32;
        let next_heap = self.next_heap + added;
        update_root(pager, self.root, |r| {
            r.changing = x.0;
            r.next_heap = next_heap;
        })?;
        let old = declared(&self.schema, &self.source);
        if let Err(e) = self.extend(pager, x, next, old) {
            txns.abort(pager, x)?;
            return Err(e);
        }
        txns.commit(pager, x)?;
        let lsn = update_root(pager, self.root, |r| r.changing = 0)?;
        pager.wait_durable(lsn);
        Ok(())
    }

    /// Make the catalog that of `source` as transaction `x`: create the structures of every
    /// constructor relation and index it has and the catalog lacks, name them and the new order
    /// keys in the roots heap, insert every row [`declared`] gives that `old` lacks, and keep
    /// the text of `source`'s last migration.
    fn extend<R: Runtime>(
        &mut self,
        pager: &Pager<R>,
        x: Xid,
        source: Source,
        old: Rows,
    ) -> Result<(), Error> {
        let schema = source.schema().expect("the source builds");
        let mut named = Vec::new();
        for (r, c) in ctor_relations(&schema) {
            if self.heaps.contains_key(&(r, c)) {
                continue;
            }
            let heap = Heap::create(pager, self.next_heap, row_type(&schema, r, c), self.large)?;
            self.next_heap += 1;
            let fields = vec![
                int(schema.ids.relations[&r]),
                int(ctor_id(&schema, r, c)),
                int(u64::from(heap.id())),
                int(heap.root().0),
            ];
            named.push((HEAP_ROOT, fields));
            self.heaps.insert((r, c), heap);
        }
        let keys = layout::new_order_keys(&schema, &self.keys);
        for (c, key) in &keys {
            named.push((
                ORDER_KEY,
                vec![int(*c), Value::Bytes(key.as_bytes().to_vec())],
            ));
        }
        self.keys.extend(keys);
        for (id, spec) in indexes(&schema) {
            if self.indexes.contains_key(&id) {
                continue;
            }
            let index = Index::create(
                pager,
                owner(id),
                key_schema(&schema, &self.keys, &spec),
                index_ctors(&schema, &spec),
            )?;
            named.push((INDEX_ROOT, vec![int(id), int(index.tree().root().0)]));
            self.indexes.insert(id, (spec, index));
        }
        if let Some(m) = source.log.last() {
            named.push((MIGRATION, vec![int(m.number), Value::Text(m.text.clone())]));
        }
        for (ctor, fields) in named {
            self.roots
                .insert(pager, x.0, &[Value::Sum { ctor, fields }])?;
        }
        self.schema = schema;
        self.source = source;
        for (r, rows) in declared(&self.schema, &self.source) {
            let before = old.get(&r).map_or(&[][..], Vec::as_slice);
            for (c, row) in rows {
                if !before.contains(&(c, row.clone())) {
                    self.insert(pager, x, r, c, &row)?;
                }
            }
        }
        Ok(())
    }

    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// Returns the source the catalog's migrations fold to.
    pub fn source(&self) -> &Source {
        &self.source
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

    /// Returns the order key of each constructor of every declared sum.
    pub fn order_keys(&self) -> &OrderKeys {
        &self.keys
    }

    /// Returns the rows of relation `r` visible at `snapshot`, each with its constructor index,
    /// by constructor and then in heap order.
    pub fn rows<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        snapshot: &Snapshot,
        r: RelId,
    ) -> Result<Vec<(u32, Vec<Value>)>, PageError> {
        let mut out = Vec::new();
        for ((_, c), heap) in self.heaps.range((r, 0)..=(r, u32::MAX)) {
            out.extend(
                visible(pager, txns, snapshot, heap)?
                    .into_iter()
                    .map(|row| (*c, row)),
            );
        }
        Ok(out)
    }

    /// Returns the planner's description of every relation.
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
        let tid = self.heap(r, c).insert(pager, x.0, row)?;
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
