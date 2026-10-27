mod change;
mod deploy;
mod ids;
mod keying;
pub mod layout;
mod moved;
mod oplog;
mod readers;
mod root;
mod rows;
mod validate;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::SystemTime;

use adtb_lang::ast::{CtorPath, CtorSel, CtorTarget, MigrationFile, Operator, OperatorKind};
use adtb_lang::check::Program;
use adtb_lang::evolve::{Code, Refusal, Source};
use adtb_lang::norm::{self, NormalProgram};
use adtb_lang::parse::{Options, parse_migration};
use adtb_lang::schema::{self, Schema, ValueDef};
use adtb_lang::types::{RelId, Ty};

use crate::btree::{self, BTree, Index};
use crate::exec::{self, StoredHeap, StoredIndex};
use crate::heap::{self, FieldType, Heap, LargeStore};
use crate::key::{CtorId, Dir, Entry, KeySchema, KeyType, OrderKey, Scalar, Value};
use crate::mvcc::{self, CommitLog, Scope, Snapshot, Transactions, Xid};
use crate::plan::{self, HeapId, IndexDesc, IndexId, RelationDesc};
use crate::runtime::Runtime;
use crate::storage::{self, Lsn, PageError, PageKind, Pager, Pid};
use crate::verify::{self, Depth, Inventory, Problem, Report, Sum};

pub use deploy::Rewrite;
pub use keying::{IndexKeys, element_types};
pub use layout::OrderKeys;
pub use oplog::{Hash, version_hash};
pub use readers::{PendingDrop, Read, Reader};
pub use rows::{IndexSpec, Rows, applied, declared, field_path, indexed_fields, indexes};

use change::Data;
use ids::{CatCtorId, CatRelId, ctor_id, ctor_index, rel_id, relation};
use readers::Readers;
use root::{Changing, Root};

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
    /// A stored page or row does not read as the catalog writes it; what is wrong.
    Corrupt(String),
    /// A stored migration does not parse or apply under this engine; its version and each
    /// reason.
    Unaccepted(u64, Vec<String>),
    /// A guarantee of the checker or planner does not hold; the one named.
    Internal(String),
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
    /// The checked program of `source`.
    program: Arc<Program>,
    root: Pid,
    large: LargeStore,
    roots: Arc<Heap>,
    heaps: BTreeMap<(RelId, u32), Arc<Heap>>,
    indexes: BTreeMap<u64, (IndexSpec, Index, Arc<IndexKeys>)>,
    keys: OrderKeys,
    next_heap: u32,
    sequences: Pid,
    /// The memory a migration's validation of its constraints may hold.
    query_memory: usize,
    readers: Readers,
}

/// The memory a migration's validation may hold until [`Catalog::set_query_memory`] sets it.
const DEFAULT_QUERY_MEMORY: usize = 256 << 20;

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

fn uint(v: &Value) -> Result<u64, Error> {
    match v {
        Value::I64(n) => u64::try_from(*n)
            .map_err(|_| Error::Corrupt(format!("a roots row holds negative id {n}"))),
        other => Err(Error::Corrupt(format!(
            "a roots row holds {other:?} for an id"
        ))),
    }
}

fn row_type(schema: &Schema, r: RelId, ctor: u32) -> Vec<FieldType> {
    layout::row(schema, r, ctor)
        .iter()
        .map(|(_, t)| layout::field_type(schema, t))
        .collect()
}

/// Fails with [`Error::Internal`] if an element is derived and `normal` is `None`, or a path
/// names no field or constructor.
fn key_schema(
    schema: &Schema,
    normal: Option<&NormalProgram>,
    keys: &OrderKeys,
    spec: &IndexSpec,
) -> Result<KeySchema, Error> {
    let types = element_types(schema, normal, spec.relation, spec.ctors[0], &spec.elements)
        .map_err(broken)?;
    Ok(KeySchema::new(
        types
            .iter()
            .map(|t| (layout::key_type(schema, keys, t), Dir::Asc))
            .collect(),
    ))
}

/// Returns `e`, a failure of the executor on a guarantee of the checker or of a stored row, as
/// the catalog's.
fn broken(e: exec::Error) -> Error {
    match e {
        exec::Error::Corrupt(why) => Error::Corrupt(why),
        exec::Error::Internal(why) => Error::Internal(why),
        other => Error::Internal(format!("an index key evaluates: {other:?}")),
    }
}

/// Returns the keying of index `spec` of `schema`, whose normal program is `normal`.
///
/// Fails with [`Error::Refused`] if the executor cannot compile an expression element, or with
/// [`Error::Internal`] or [`Error::Corrupt`] on a guarantee of the checker or of a stored row.
fn index_keys(
    schema: &Schema,
    normal: Option<&NormalProgram>,
    spec: &IndexSpec,
) -> Result<Arc<IndexKeys>, Error> {
    IndexKeys::new(schema, normal, spec.relation, &spec.ctors, &spec.elements)
        .map(Arc::new)
        .map_err(|e| match e {
            exec::Error::Internal(_) | exec::Error::Corrupt(_) => broken(e),
            e => Error::Refused(vec![format!(
                "index `{}` is not admitted: {e:?}",
                spec.clause
            )]),
        })
}

/// Returns the normal program of `schema`, whose checked program is `program`, when an index of
/// `specs` has an element other than a field.
fn index_program<'s>(
    schema: &Schema,
    program: &Program,
    mut specs: impl Iterator<Item = &'s IndexSpec>,
) -> Result<Option<NormalProgram>, String> {
    let derived = specs.any(|s| {
        s.elements
            .iter()
            .any(|e| matches!(e, plan::Element::Derived { .. }))
    });
    if !derived {
        return Ok(None);
    }
    norm::program(schema, program)
        .map(Some)
        .map_err(|e| format!("the schema normalises: {e:?}"))
}

fn index_ctors(schema: &Schema, spec: &IndexSpec) -> BTreeSet<CtorId> {
    spec.ctors
        .iter()
        .map(|&c| ctor_id(schema, spec.relation, c).stored())
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
/// Returns why each constructor relation of `schema` that `rewritten` names cannot store its
/// smallest row on a page of `page_size` bytes.
fn oversized(page_size: usize, schema: &Schema, rewritten: &BTreeSet<CatRelId>) -> Vec<String> {
    let mut why = Vec::new();
    for (r, c) in ctor_relations(schema) {
        if !rewritten.contains(&rel_id(schema, r)) {
            continue;
        }
        if let Err(heap::Error::TooLarge { len, limit }) =
            heap::fits(page_size, row_type(schema, r, c))
        {
            let rel = schema.relation(r);
            let ctor = &schema.ty(rel.elem_def).ctors()[c as usize].name;
            why.push(format!(
                "constructor relation {}.{ctor} stores its smallest row in {len} bytes, over the row limit of {limit}",
                rel.name
            ));
        }
    }
    why
}

fn ctor_relations(schema: &Schema) -> Vec<(RelId, u32)> {
    schema
        .ids
        .relations
        .keys()
        .flat_map(|&r| (0..ctor_count(schema, r)).map(move |c| (r, c)))
        .collect()
}

/// Returns the planner's description of every relation of `schema` over the heaps `heaps`, by
/// relation and constructor, and the indexes `specs`.
fn planned<'s>(
    schema: &Schema,
    specs: impl Iterator<Item = &'s IndexSpec> + Clone,
    heaps: &BTreeMap<(RelId, u32), HeapId>,
) -> plan::Catalog {
    let mut out = plan::Catalog::default();
    for &r in schema.ids.relations.keys() {
        let indexes = specs
            .clone()
            .filter(|s| s.relation == r)
            .map(|s| IndexDesc {
                id: IndexId(s.id),
                ctors: s.ctors.clone(),
                elements: s.elements.clone(),
                unique: s.unique,
                ordered: true,
            })
            .collect();
        let desc = RelationDesc {
            heaps: heaps
                .range((r, 0)..=(r, u32::MAX))
                .map(|((_, c), h)| (*c, *h))
                .collect(),
            key_index: IndexId(schema.ids.keys[&r]),
            indexes,
        };
        out.relations.insert(r, desc);
    }
    out
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
    heaps: BTreeMap<(CatRelId, CatCtorId), (u32, Pid)>,
    indexes: BTreeMap<u64, Pid>,
    keys: OrderKeys,
    /// The text of each migration, by version.
    migrations: BTreeMap<u64, String>,
}

impl Named {
    /// Add what roots row `row` names.
    fn add(&mut self, row: &[Value]) -> Result<(), Error> {
        let [Value::Sum { ctor, fields }] = row else {
            return Err(Error::Corrupt(format!("a roots row is {row:?}")));
        };
        match (*ctor, fields.as_slice()) {
            (HEAP_ROOT, [rel, c, id, at]) => {
                let id = uint(id)?;
                let id = u32::try_from(id)
                    .map_err(|_| Error::Corrupt(format!("heap id {id} is past 32 bits")))?;
                self.heaps.insert(
                    (CatRelId(uint(rel)?), CatCtorId(uint(c)?)),
                    (id, Pid(uint(at)?)),
                );
            }
            (INDEX_ROOT, [id, at]) => {
                self.indexes.insert(uint(id)?, Pid(uint(at)?));
            }
            (ORDER_KEY, [c, Value::Bytes(key)]) => {
                let key = OrderKey::new(key.clone())
                    .ok_or_else(|| Error::Corrupt(format!("order key {key:?} is not valid")))?;
                self.keys.insert(uint(c)?, key);
            }
            (MIGRATION, [version, Value::Text(text)]) => {
                self.migrations.insert(uint(version)?, text.clone());
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
        out.add(&row)?;
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

/// Returns the source the migrations `texts` fold to, in version order from the empty schema,
/// and its schema and checked program.
fn fold(texts: &BTreeMap<u64, String>) -> Result<(Source, Schema, Program), Error> {
    let mut source = Source::default();
    for (&version, text) in texts {
        let refused = |why: Vec<String>| Error::Unaccepted(version, why);
        let file = parse_migration(text, Options::default())
            .ok()
            .map_err(|ds| refused(ds.into_iter().map(|d| d.message).collect()))?;
        source = source
            .migrate(&file, text)
            .map_err(|r| refused(reasons(r)))?;
    }
    let (schema, program) = source.check().map_err(|ds| {
        Error::Unaccepted(
            source.version,
            ds.into_iter().map(|d| d.diag.message).collect(),
        )
    })?;
    Ok((source, schema, program))
}

/// Returns whether `keys` holds an order key for each constructor of every declared sum of
/// `schema`, rising in declaration order.
fn check_order_keys(schema: &Schema, keys: &OrderKeys) -> Result<(), Error> {
    for cs in layout::declared_sums(schema) {
        let mut last: Option<&OrderKey> = None;
        for id in cs {
            let Some(key) = keys.get(&id) else {
                return Err(Error::Corrupt(format!(
                    "constructor {id} has no stored order key"
                )));
            };
            if last.is_some_and(|l| l >= key) {
                return Err(Error::Corrupt(format!(
                    "the order key of constructor {id} does not follow the one before it"
                )));
            }
            last = Some(key);
        }
    }
    Ok(())
}

/// Returns the message of each operator `r` refuses, then of each file its recheck refuses.
fn reasons(r: Refusal) -> Vec<String> {
    let ops = r.operators.into_iter().map(|d| d.message);
    ops.chain(r.recheck.into_iter().map(|d| d.diag.message))
        .collect()
}

/// Returns the failure that stops `report` finding every leaked page: the first damaged page,
/// or else each structure it could not read whole.
fn unfinished(report: &Report) -> Error {
    let damaged = report.findings.iter().find_map(|f| match f.problem {
        Problem::Damaged(damage) => Some(PageError { pid: f.pid, damage }),
        _ => None,
    });
    match damaged {
        Some(e) => Error::Page(e),
        None => Error::Corrupt(format!("{:?}", report.findings)),
    }
}

/// Returns why the catalog cannot record the last migration of `source`: its operators that do
/// not resolve, and its functions whose names do not resolve or whose code it does not hold.
fn recordable(source: &Source) -> Result<(), Vec<String>> {
    let mut why = Vec::new();
    if let Err(ds) = source.resolved(source.version) {
        why.extend(ds.into_iter().map(|d| d.message));
    }
    match source.functions() {
        Err(ds) => why.extend(ds.into_iter().map(|d| d.diag.message)),
        Ok(fs) => why.extend(fs.iter().filter_map(|f| match &f.code {
            Code::Wasm { module, .. } => Some(format!(
                "function `{}` names Wasm module `{module}`, which no migration installs",
                f.name
            )),
            _ => None,
        })),
    }
    if why.is_empty() { Ok(()) } else { Err(why) }
}

/// Returns `t` in microseconds since 1970-01-01T00:00:00Z, saturating.
fn micros(t: SystemTime) -> i64 {
    match t.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_micros()).unwrap_or(i64::MAX),
        Err(e) => i64::try_from(e.duration().as_micros()).map_or(i64::MIN, |n| -n),
    }
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
            changing: Changing::None,
            sequences: Pid(0),
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
        let (schema, program) = Source::default()
            .check()
            .map_err(|ds| Error::Internal(format!("the engine's declarations check: {ds:?}")))?;
        let program = Arc::new(program);
        let empty = Catalog {
            source: Source::default(),
            schema: engine_schema(),
            program: program.clone(),
            root: pid,
            large,
            roots: Arc::new(roots),
            heaps: BTreeMap::new(),
            indexes: BTreeMap::new(),
            keys: OrderKeys::new(),
            next_heap: FIRST_HEAP,
            sequences: Pid(0),
            query_memory: DEFAULT_QUERY_MEMORY,
            readers: Readers::default(),
        };
        let x = txns.begin(pager)?;
        let every = empty
            .schema
            .ids
            .relations
            .values()
            .map(|&id| CatRelId(id))
            .collect();
        let (id, snapshot) = txns.take(Scope::Every)?;
        let m = Migrating {
            pager,
            txns,
            x,
            snapshot: &snapshot,
        };
        let built = empty.build(
            &m,
            &Built {
                next: Checked {
                    source: Source::default(),
                    schema,
                    program,
                },
                data: &Data::new(),
                rewritten: &every,
                old: Rows::new(),
            },
        );
        txns.release(id);
        let catalog = built?;
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
    ///
    /// Fails with [`Error::Corrupt`] if a stored page or row does not read as the catalog writes
    /// it, and with [`Error::Unaccepted`] if a stored migration does not parse or apply under
    /// this engine. Fails with the damage found if a schema change that did not commit cannot
    /// have its pages found, leaving it for the next open to free.
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
            let found = (storage::kind(&page), storage::format(&page));
            if found != (PageKind::Catalog as u16, root::FORMAT) {
                return Err(Error::Corrupt(format!(
                    "the header's catalog root {pid:?} has kind and format {found:?}"
                )));
            }
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
        let (source, schema, program) = fold(&named.migrations)?;
        check_order_keys(&schema, &named.keys)?;
        let mut heaps = BTreeMap::new();
        for (r, c) in ctor_relations(&schema) {
            let rid = rel_id(&schema, r);
            let cid = ctor_id(&schema, r, c);
            let Some(&(id, at)) = named.heaps.get(&(rid, cid)) else {
                let name = &schema.relation(r).name;
                return Err(Error::Missing(format!("heap of {name} constructor {cid}")));
            };
            let heap = Heap::open(pager, id, at, row_type(&schema, r, c), large)?;
            heaps.insert((r, c), Arc::new(heap));
        }
        let specs = indexes(&schema);
        let normal = index_program(&schema, &program, specs.values()).map_err(Error::Corrupt)?;
        let mut stored = BTreeMap::new();
        for (id, spec) in specs {
            let Some(&at) = named.indexes.get(&id) else {
                return Err(Error::Missing(format!("index {id}")));
            };
            let index = Index::open(
                BTree::open(at, owner(id)),
                key_schema(&schema, normal.as_ref(), &named.keys, &spec)
                    .map_err(|e| Error::Corrupt(format!("{e:?}")))?,
                index_ctors(&schema, &spec),
            );
            let keys = index_keys(&schema, normal.as_ref(), &spec)
                .map_err(|e| Error::Corrupt(format!("{e:?}")))?;
            stored.insert(id, (spec, index, keys));
        }
        let catalog = Catalog {
            source,
            schema,
            program: Arc::new(program),
            root: pid,
            large,
            roots: Arc::new(roots),
            heaps,
            indexes: stored,
            keys: named.keys,
            next_heap: root.next_heap,
            sequences: root.sequences,
            query_memory: DEFAULT_QUERY_MEMORY,
            readers: Readers::default(),
        };
        if root.changing != Changing::None {
            catalog.free_leaked(pager, &txns)?;
            update_root(pager, pid, |r| r.changing = Changing::None)?;
        }
        Ok((catalog, txns))
    }

    /// Apply `text`, the migration numbered one past the schema's version, as one transaction:
    /// map the rows of every relation it changes, create the heaps and indexes the new schema
    /// stores them in, write the catalog rows of the new version, and keep its text. Returns once
    /// the migration is durable.
    ///
    /// Fails with [`Error::Refused`], changing nothing, if the migration does not apply to the
    /// schema, if a constraint of the result has no index-bounded incremental check, or if the
    /// data contradicts an operator: a constructor or relation it removes holds rows, a unique or
    /// key it adds finds a duplicate, a constructor it disables holds rows, a fill, routing or
    /// index expression faults on a row, or a constraint or check it adds, or one reading a
    /// relation it rewrites, has violating rows in the final state. Fails with
    /// [`Error::Internal`] when admitting or validating a constraint breaks a guarantee of the
    /// checker. On any other failure the transaction is aborted, and the catalog must be opened
    /// again.
    ///
    /// Postcondition: [`Catalog::open`] after a crash finds either the whole migration or
    /// nothing of it, and no page of the structures the migration replaced or created but did not
    /// commit.
    pub fn migrate<R: Runtime>(
        &mut self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        text: &str,
    ) -> Result<(), Error> {
        let refused = |why: Vec<String>| Err(Error::Refused(why));
        let file = match parse_migration(text, Options::default()).ok() {
            Ok(f) => f,
            Err(ds) => return refused(ds.into_iter().map(|d| d.message).collect()),
        };
        let next = match self.source.migrate(&file, text) {
            Ok(s) => s,
            Err(r) => return refused(reasons(r)),
        };
        let (schema, program) = match next.check() {
            Ok(checked) => checked,
            Err(ds) => return refused(ds.into_iter().map(|d| d.diag.message).collect()),
        };
        if let Err(why) = recordable(&next) {
            return refused(why);
        }
        let next = Checked {
            source: next,
            schema,
            program: Arc::new(program),
        };
        let readers = self.held(&file, text, &next)?;
        let rewritten = change::rewritten(&self.schema, &next.schema);
        let (schema, program) = readers
            .enforced()
            .map_or((&next.schema, &next.program), |e| (&e.schema, &e.program));
        self.admits(schema, program, &rewritten)?;
        let x = txns.begin(pager)?;
        update_root(pager, self.root, |r| r.changing = Changing::Schema(x))?;
        let (id, snapshot) = txns.take(Scope::Every)?;
        let m = Migrating {
            pager,
            txns,
            x,
            snapshot: &snapshot,
        };
        let built = self.replay(&m, &file, text, next, &rewritten, readers);
        txns.release(id);
        let built = built.and_then(|c| {
            update_root(pager, self.root, |r| r.next_heap = c.next_heap)?;
            Ok(c)
        });
        match built {
            Ok(c) => {
                txns.commit(pager, x)?;
                *self = c;
            }
            Err(e) => {
                txns.abort(pager, x)?;
                self.free_leaked(pager, txns)?;
                let lsn = update_root(pager, self.root, |r| r.changing = Changing::None)?;
                pager.wait_durable(lsn);
                return Err(e);
            }
        }
        self.free_leaked(pager, txns)?;
        let lsn = update_root(pager, self.root, |r| r.changing = Changing::None)?;
        pager.wait_durable(lsn);
        Ok(())
    }

    /// Refuses `schema`, whose checked program is `program`, when a constraint of it has no
    /// index-bounded incremental check over the heaps [`Catalog::heap_ids`] assigns a migration
    /// rewriting the relations `rewritten` names, and its indexes; each reason.
    fn admits(
        &self,
        schema: &Schema,
        program: &Program,
        rewritten: &BTreeSet<CatRelId>,
    ) -> Result<(), Error> {
        let normal = norm::program(schema, program)
            .map_err(|e| Error::Refused(vec![format!("the schema does not normalise: {e:?}")]))?;
        let (ids, _) = self.heap_ids(schema, rewritten)?;
        let heaps: BTreeMap<(RelId, u32), HeapId> = ids
            .into_iter()
            .map(|(rc, id)| (rc, HeapId(u64::from(id))))
            .collect();
        let specs = indexes(schema);
        for spec in specs.values() {
            index_keys(schema, Some(&normal), spec)?;
        }
        let catalog = planned(schema, specs.values(), &heaps);
        let why = crate::write::refusals(schema, &normal, &catalog).map_err(Error::Internal)?;
        if why.is_empty() {
            return Ok(());
        }
        Err(Error::Refused(
            why.into_iter()
                .map(|(c, detail)| format!("constraint {c} is not admitted: {detail}"))
                .collect(),
        ))
    }

    /// Returns the catalog of `next`, the source after migration `file`, as `m.x` writes it: the
    /// rows of every relation `rewritten` names mapped through each of the migration's operators
    /// in turn from the rows `m.snapshot` sees.
    fn replay<R: Runtime>(
        &self,
        m: &Migrating<'_, R>,
        file: &MigrationFile,
        text: &str,
        next: Checked,
        rewritten: &BTreeSet<CatRelId>,
        readers: Readers,
    ) -> Result<Catalog, Error> {
        let mut data = Data::new();
        for &rid in rewritten {
            let Some(r) = relation(&self.schema, rid) else {
                continue;
            };
            let mut ctors: BTreeMap<CatCtorId, Vec<Vec<Value>>> = (0..ctor_count(&self.schema, r))
                .map(|c| (ctor_id(&self.schema, r, c), Vec::new()))
                .collect();
            for (c, row) in self.rows(m.pager, m.txns, m.snapshot, r)? {
                ctors
                    .get_mut(&ctor_id(&self.schema, r, c))
                    .expect("a constructor of the relation")
                    .push(row);
            }
            data.insert(rid, ctors);
        }
        let module = &file.module.text;
        let mut at = self.source.clone();
        at.version += 1;
        at.expecting(module, &file.operators);
        let mut before = self.schema.clone();
        let mut why = Vec::new();
        for op in &file.operators {
            let mut after = at.clone();
            let steps = match after.steps(module, op, text) {
                Ok(s) => s,
                Err(ds) => {
                    return Err(Error::Refused(ds.into_iter().map(|d| d.message).collect()));
                }
            };
            why.extend(self.emptied(m, &before, &data, module, op)?);
            for step in steps {
                let Ok(schema) = step.after.schema() else {
                    continue;
                };
                let moved = match moved::moved(&before, &schema, &step.maps, &data) {
                    Ok(m) => m,
                    Err(Error::Refused(w)) => {
                        why.extend(w);
                        change::Moved::default()
                    }
                    Err(e) => return Err(e),
                };
                match change::step(&before, &schema, &mut data, moved) {
                    Ok(()) => {}
                    Err(Error::Refused(w)) => why.extend(w),
                    Err(e) => return Err(e),
                }
                before = schema;
            }
            at = after;
        }
        why.extend(oversized(m.pager.page_size(), &next.schema, rewritten));
        if !why.is_empty() {
            return Err(Error::Refused(why));
        }
        let mut built = self.build(
            m,
            &Built {
                next,
                data: &data,
                rewritten,
                old: declared(&self.schema, &self.source),
            },
        )?;
        built.readers = readers;
        let why = built.violations(m, self.checking().0, rewritten)?;
        if !why.is_empty() {
            return Err(Error::Refused(why));
        }
        Ok(built)
    }

    /// Returns why the data refuses `op`, applied to `schema` of a migration of `module`, beyond
    /// what its data map refuses: a constructor relation it disables holds rows, or a field
    /// holds a value of a constructor of a type it disables or removes.
    fn emptied<R: Runtime>(
        &self,
        m: &Migrating<'_, R>,
        schema: &Schema,
        data: &Data,
        module: &str,
        op: &Operator,
    ) -> Result<Vec<String>, Error> {
        let target = match &op.kind {
            OperatorKind::Disable(CtorTarget::Type(path)) => {
                return self.stored_in(m, schema, data, module, path, "disabled");
            }
            OperatorKind::RemoveConstructor(path) => {
                return self.stored_in(m, schema, data, module, path, "removed");
            }
            OperatorKind::Disable(target) => target,
            _ => return Ok(Vec::new()),
        };
        let mut why = Vec::new();
        for (r, c) in targeted(schema, module, target) {
            let rid = rel_id(schema, r);
            let cid = ctor_id(schema, r, c);
            let held = match data.get(&rid) {
                Some(ctors) => !ctors[&cid].is_empty(),
                None => self.holds(m, rid, cid)?,
            };
            if held {
                let rel = schema.relation(r);
                let ctor = &schema.ty(rel.elem_def).ctors()[c as usize].name;
                why.push(format!(
                    "constructor relation {}.{ctor} holds rows; it cannot be disabled",
                    rel.name
                ));
            }
        }
        Ok(why)
    }

    /// Returns why constructor `path` of a type, in a migration of `module` applied to
    /// `schema`, cannot be `verb`: each relation a field of whose rows holds a value of it.
    fn stored_in<R: Runtime>(
        &self,
        m: &Migrating<'_, R>,
        schema: &Schema,
        data: &Data,
        module: &str,
        path: &CtorPath,
        verb: &str,
    ) -> Result<Vec<String>, Error> {
        let Some(def) = readers::type_named(schema, module, &path.ty) else {
            return Ok(Vec::new());
        };
        let Some(c) = schema.ty(def).ctor_index(&path.ctor.text) else {
            return Ok(Vec::new());
        };
        let id = CatCtorId(schema.ids.ctors[&(def, c)]).stored();
        let mut why = Vec::new();
        for (&r, &rid) in &schema.ids.relations {
            let fields: Vec<Vec<(String, Ty)>> = (0..ctor_count(schema, r))
                .map(|k| layout::row(schema, r, k))
                .collect();
            let reached = fields
                .iter()
                .flatten()
                .any(|(_, t)| readers::reaches(schema, t, def, &mut BTreeSet::new()));
            if !reached {
                continue;
            }
            let types: Vec<Vec<FieldType>> = fields
                .iter()
                .map(|fs| {
                    fs.iter()
                        .map(|(_, t)| layout::field_type(schema, t))
                        .collect()
                })
                .collect();
            let rid = CatRelId(rid);
            let rows: Vec<(u32, Vec<Value>)> = match data.get(&rid) {
                Some(ctors) => ctors
                    .iter()
                    .flat_map(|(&cid, rows)| {
                        let k = ctor_index(schema, r, cid).expect("a constructor of the relation");
                        rows.iter().map(move |row| (k, row.clone()))
                    })
                    .collect(),
                None => match relation(&self.schema, rid) {
                    Some(old) => self.rows(m.pager, m.txns, m.snapshot, old)?,
                    None => continue,
                },
            };
            let held = rows.iter().any(|(k, row)| {
                types[*k as usize]
                    .iter()
                    .zip(row)
                    .any(|(t, v)| holds_ctor(t, v, id))
            });
            if held {
                why.push(format!(
                    "a field of {} holds a value of {}.{}; it cannot be {verb}",
                    schema.relation(r).name,
                    schema.ty(def).name,
                    path.ctor.text
                ));
            }
        }
        Ok(why)
    }

    /// Returns whether the heap of constructor `cid` of relation `rid` holds a row `m.snapshot`
    /// sees.
    fn holds<R: Runtime>(
        &self,
        m: &Migrating<'_, R>,
        rid: CatRelId,
        cid: CatCtorId,
    ) -> Result<bool, Error> {
        let Some(r) = relation(&self.schema, rid) else {
            return Ok(false);
        };
        let c = ctor_index(&self.schema, r, cid).expect("a constructor of the relation");
        Ok(!visible(m.pager, m.txns, m.snapshot, self.heap(r, c))?.is_empty())
    }

    /// Returns the catalog of `b.next` as transaction `m.x` writes it over this one: a heap for
    /// each constructor relation and an index for each index of the relations `b.rewritten`
    /// names, filled with the rows `b.data` gives them, the structures of every other relation
    /// kept, the roots rows of the replaced structures deleted and those of the new ones
    /// written, and every catalog row that `b.old` holds and the new version does not deleted
    /// and every other one written.
    fn build<R: Runtime>(&self, m: &Migrating<'_, R>, b: &Built) -> Result<Catalog, Error> {
        let pager = m.pager;
        let schema = b.next.schema.clone();
        let mut keys = self.keys.clone();
        let fresh = layout::new_order_keys(&schema, &keys);
        keys.extend(fresh.clone());
        let (heap_ids, next_heap) = self.heap_ids(&schema, b.rewritten)?;
        let mut next = Catalog {
            source: b.next.source.clone(),
            schema,
            program: b.next.program.clone(),
            root: self.root,
            large: self.large,
            roots: self.roots.clone(),
            heaps: BTreeMap::new(),
            indexes: BTreeMap::new(),
            keys,
            next_heap,
            sequences: self.sequences,
            query_memory: self.query_memory,
            readers: Readers::default(),
        };
        let specs = indexes(&next.schema);
        let normal =
            index_program(&next.schema, &next.program, specs.values()).map_err(Error::Internal)?;
        let mut keyed = BTreeMap::new();
        for (id, spec) in &specs {
            keyed.insert(*id, index_keys(&next.schema, normal.as_ref(), spec)?);
        }
        duplicates(&next.schema, &next.keys, normal.as_ref(), &specs, &keyed, b)?;
        let old: BTreeMap<(CatRelId, CatCtorId), (RelId, u32)> = self
            .heaps
            .keys()
            .map(|&(r, c)| {
                let id = (rel_id(&self.schema, r), ctor_id(&self.schema, r, c));
                (id, (r, c))
            })
            .collect();
        let mut named = Vec::new();
        let mut gone = Vec::new();
        for (r, c) in ctor_relations(&next.schema) {
            let rid = rel_id(&next.schema, r);
            let cid = ctor_id(&next.schema, r, c);
            if !b.rewritten.contains(&rid) {
                let kept = old
                    .get(&(rid, cid))
                    .unwrap_or_else(|| panic!("relation {rid} keeps constructor {cid}"));
                next.heaps.insert((r, c), self.heaps[kept].clone());
                continue;
            }
            let heap = Heap::create(
                pager,
                heap_ids[&(r, c)],
                row_type(&next.schema, r, c),
                self.large,
            )?;
            named.push((HEAP_ROOT, heap_root(rid, cid, &heap)));
            next.heaps.insert((r, c), Arc::new(heap));
        }
        for (&(rid, cid), rc) in &old {
            if b.rewritten.contains(&rid) {
                gone.push((HEAP_ROOT, heap_root(rid, cid, &self.heaps[rc])));
            }
        }
        for (id, spec) in specs {
            let rid = rel_id(&next.schema, spec.relation);
            let keys = keyed.remove(&id).expect("every index is keyed");
            if !b.rewritten.contains(&rid) {
                let (_, index, _) = &self.indexes[&id];
                let index = index.clone();
                next.indexes.insert(id, (spec, index, keys));
                continue;
            }
            let index = Index::create(
                pager,
                owner(id),
                key_schema(&next.schema, normal.as_ref(), &next.keys, &spec)?,
                index_ctors(&next.schema, &spec),
            )?;
            named.push((INDEX_ROOT, vec![int(id), int(index.tree().root().0)]));
            next.indexes.insert(id, (spec, index, keys));
        }
        for (&id, (spec, index, _)) in &self.indexes {
            if b.rewritten.contains(&rel_id(&self.schema, spec.relation)) {
                gone.push((INDEX_ROOT, vec![int(id), int(index.tree().root().0)]));
            }
        }
        for (c, key) in &fresh {
            named.push((
                ORDER_KEY,
                vec![int(*c), Value::Bytes(key.as_bytes().to_vec())],
            ));
        }
        if let Some(m) = next.source.log.last() {
            named.push((MIGRATION, vec![int(m.number), Value::Text(m.text.clone())]));
        }
        let gone: Vec<Vec<Value>> = gone
            .into_iter()
            .map(|(ctor, fields)| vec![Value::Sum { ctor, fields }])
            .collect();
        delete_rows(m, &self.roots, &gone)?;
        for (ctor, fields) in named {
            self.roots
                .insert(pager, m.x.0, &[Value::Sum { ctor, fields }])?;
        }
        for (&rid, ctors) in b.data {
            let r = relation(&next.schema, rid).expect("data holds relations of the result");
            for (&cid, rows) in ctors {
                let c = ctor_index(&next.schema, r, cid).expect("a constructor of the result");
                for row in rows {
                    next.insert(pager, m.x, r, c, row)?;
                }
            }
        }
        self.write_rows(m, &next, b)?;
        Ok(next)
    }

    /// Returns the heap id each constructor relation of `schema` has once a migration that
    /// rewrites the relations `rewritten` names is built over this catalog, and the id after
    /// the last one it gives: a kept relation's heap keeps its id, and every heap of a rewritten
    /// one takes the next free id in turn.
    fn heap_ids(
        &self,
        schema: &Schema,
        rewritten: &BTreeSet<CatRelId>,
    ) -> Result<(HeapIds, u32), Error> {
        let old: BTreeMap<(CatRelId, CatCtorId), u32> = self
            .heaps
            .iter()
            .map(|(&(r, c), h)| {
                let id = (rel_id(&self.schema, r), ctor_id(&self.schema, r, c));
                (id, h.id())
            })
            .collect();
        let mut next = self.next_heap;
        let mut out = BTreeMap::new();
        for (r, c) in ctor_relations(schema) {
            let rid = rel_id(schema, r);
            let cid = ctor_id(schema, r, c);
            let id = if rewritten.contains(&rid) {
                next += 1;
                next - 1
            } else {
                *old.get(&(rid, cid)).ok_or_else(|| {
                    Error::Internal(format!("relation {rid} keeps constructor {cid}"))
                })?
            };
            out.insert((r, c), id);
        }
        Ok((out, next))
    }

    /// Write the catalog rows of `next`'s version as `m.x` over this catalog's: delete each row
    /// `b.old` holds that the version does not, insert each other row of the version, and insert
    /// the row of its migration.
    fn write_rows<R: Runtime>(
        &self,
        m: &Migrating<'_, R>,
        next: &Catalog,
        b: &Built,
    ) -> Result<(), Error> {
        let pager = m.pager;
        let old: BTreeMap<CatRelId, Held> = b
            .old
            .iter()
            .map(|(r, rows)| (rel_id(&self.schema, *r), (*r, rows.as_slice())))
            .collect();
        for (r, rows) in declared(&next.schema, &next.source) {
            let rid = rel_id(&next.schema, r);
            let (was, before) = old.get(&rid).copied().unwrap_or((r, &[]));
            for (c, row) in &rows {
                if !before.contains(&(*c, row.clone())) {
                    next.insert(pager, m.x, r, *c, row)?;
                }
            }
            let mut gone: BTreeMap<u32, Vec<Vec<Value>>> = BTreeMap::new();
            for (c, row) in before {
                if !rows.contains(&(*c, row.clone())) {
                    gone.entry(*c).or_default().push(row.clone());
                }
            }
            for (c, rows) in gone {
                delete_rows(m, self.heap(was, c), &rows)?;
            }
        }
        for (r, rows) in applied(&next.schema, &next.source, micros(pager.wall())) {
            for (c, row) in rows {
                next.insert(pager, m.x, r, c, &row)?;
            }
        }
        Ok(())
    }

    /// Free every page the catalog's structures do not reach.
    ///
    /// Fails with the damage found, freeing nothing, if a damaged page keeps it from finding
    /// every structure's pages.
    fn free_leaked<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
    ) -> Result<(), Error> {
        let report = {
            let clog = txns.commit_log();
            verify::verify_with(pager, &self.inventory(&clog), Depth::STRUCTURE)
        };
        if !report.complete {
            return Err(unfinished(&report));
        }
        for p in report.leaked() {
            pager.free(p)?;
        }
        Ok(())
    }

    /// Returns the schema and program constraint checks and validation run against: the
    /// version's, with every pending-dropped object kept.
    fn checking(&self) -> (&Schema, &Arc<Program>) {
        self.readers
            .enforced()
            .map_or((&self.schema, &self.program), |e| (&e.schema, &e.program))
    }

    /// Set the memory a migration's validation of its constraints may hold: the query memory
    /// budget of the open database.
    pub fn set_query_memory(&mut self, bytes: usize) {
        self.query_memory = bytes;
    }

    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// Returns the checked program of the catalog's source.
    pub fn program(&self) -> &Arc<Program> {
        &self.program
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

    /// Returns the write path's page of generated counters, once one is named.
    pub fn sequences(&self) -> Option<Pid> {
        (self.sequences != Pid(0)).then_some(self.sequences)
    }

    /// Mark the catalog whose root page is `root` as changing, before the write path allocates
    /// its page of generated counters, so an open after a crash frees that page until
    /// [`Catalog::name_sequences`] names it.
    pub fn naming_sequences<R: Runtime>(pager: &Pager<R>, root: Pid) -> Result<(), PageError> {
        update_root(pager, root, |r| r.changing = Changing::NamingSequences)?;
        Ok(())
    }

    /// Name `pid` as the write path's page of generated counters in the catalog whose root page
    /// is `root`, and end the change [`Catalog::naming_sequences`] began. Returns once the naming
    /// is durable.
    pub fn name_sequences<R: Runtime>(
        pager: &Pager<R>,
        root: Pid,
        pid: Pid,
    ) -> Result<(), PageError> {
        let lsn = update_root(pager, root, |r| {
            r.sequences = pid;
            r.changing = Changing::None;
        })?;
        pager.wait_durable(lsn);
        Ok(())
    }

    /// Returns the heap of every constructor relation and every index, as the executor and the
    /// write path bind them.
    pub fn bindings(&self) -> (BTreeMap<HeapId, StoredHeap>, BTreeMap<IndexId, StoredIndex>) {
        let schema = &self.schema;
        let heaps = self
            .heaps
            .iter()
            .map(|(&(r, c), heap)| {
                let stored = StoredHeap {
                    row: row_type(schema, r, c),
                    heap: heap.clone(),
                };
                (HeapId(u64::from(stored.heap.id())), stored)
            })
            .collect();
        let indexes = self
            .indexes
            .values()
            .map(|(spec, index, _)| {
                let ctors = spec
                    .ctors
                    .iter()
                    .map(|&c| (ctor_id(schema, spec.relation, c).stored(), c))
                    .collect();
                (
                    IndexId(spec.id),
                    StoredIndex {
                        index: index.clone(),
                        ctors,
                    },
                )
            })
            .collect();
        (heaps, indexes)
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
        let heaps: BTreeMap<(RelId, u32), HeapId> = self
            .heaps
            .iter()
            .map(|(rc, h)| (*rc, HeapId(u64::from(h.id()))))
            .collect();
        planned(
            &self.schema,
            self.indexes.values().map(|(s, _, _)| s),
            &heaps,
        )
    }

    /// Returns every structure of the file for `verify`: the commit log `clog`, the catalog's
    /// heaps and indexes, and its root page.
    pub fn inventory<'a>(&'a self, clog: &'a CommitLog) -> Inventory<'a> {
        let mut sums = vec![Sum {
            heaps: vec![(CtorId(0), self.roots.as_ref())],
            indexes: Vec::new(),
        }];
        for &r in self.schema.ids.relations.keys() {
            let heaps = self
                .heaps
                .range((r, 0)..=(r, u32::MAX))
                .map(|((_, c), h)| {
                    let id = ctor_id(&self.schema, r, *c).stored();
                    (id, h.as_ref())
                })
                .collect();
            let indexes = self
                .indexes
                .values()
                .filter(|(s, _, _)| s.relation == r)
                .map(|(_, i, _)| i)
                .collect();
            sums.push(Sum { heaps, indexes });
        }
        Inventory {
            commit_log: Some(clog),
            sums,
            trees: Vec::new(),
            pages: std::iter::once(self.root).chain(self.sequences()).collect(),
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
        let ctor = ctor_id(&self.schema, r, c).stored();
        for (spec, index, keys) in self.indexes.values() {
            if spec.relation != r {
                continue;
            }
            let Some(key) = row_key(keys, index.schema(), c, row)? else {
                continue;
            };
            let fresh = index.insert(pager, &Entry { key, ctor, tid })?;
            assert!(fresh, "a new row's entry is not in its index yet");
        }
        Ok(())
    }
}

/// A relation of the catalog before a migration, with the catalog rows it holds.
type Held<'a> = (RelId, &'a [(u32, Vec<Value>)]);

/// The heap id of each constructor relation, by relation and constructor.
type HeapIds = BTreeMap<(RelId, u32), u32>;

/// What [`Catalog::build`] makes a catalog of.
struct Built<'a> {
    next: Checked,
    data: &'a Data,
    /// The ids of the relations whose heaps and indexes are made again.
    rewritten: &'a BTreeSet<CatRelId>,
    /// The catalog rows of the catalog before the migration, by its relations.
    old: Rows,
}

/// A source, the schema it builds and its checked program.
#[derive(Debug)]
struct Checked {
    source: Source,
    schema: Schema,
    program: Arc<Program>,
}

/// The transaction `x` of one migration, and what it reads the catalog before it at.
struct Migrating<'a, R: Runtime> {
    pager: &'a Pager<R>,
    txns: &'a Transactions<R>,
    x: Xid,
    snapshot: &'a Snapshot,
}

/// Returns the fields of the roots row naming `heap`, constructor `cid` of relation `rid`.
fn heap_root(rid: CatRelId, cid: CatCtorId, heap: &Heap) -> Vec<Value> {
    vec![
        int(rid.0),
        int(cid.0),
        int(u64::from(heap.id())),
        int(heap.root().0),
    ]
}

/// Delete, as `m.x`, one version `m.snapshot` sees of each row of `rows` from `heap`.
fn delete_rows<R: Runtime>(
    m: &Migrating<'_, R>,
    heap: &Heap,
    rows: &[Vec<Value>],
) -> Result<(), Error> {
    let pager = m.pager;
    if rows.is_empty() {
        return Ok(());
    }
    let mut left: Vec<&Vec<Value>> = rows.iter().collect();
    let mut versions = Vec::new();
    heap.scan(pager, |v| versions.push(v))?;
    for v in versions {
        let Some(at) = left.iter().position(|r| **r == v.row) else {
            continue;
        };
        if m.txns
            .sees(pager, m.snapshot, None, v.header.xmin, v.header.xmax)?
        {
            heap.delete(pager, v.tid, m.x.0)?;
            left.swap_remove(at);
        }
    }
    assert!(left.is_empty(), "every row deleted is visible: {left:?}");
    Ok(())
}

/// Returns the key stored row `row` of constructor `c` has in an index keyed by `keys` whose key
/// schema is `schema`, or `None` when the index holds no entry for it.
///
/// Fails with [`Error::Refused`] when an expression element faults on the row.
fn row_key(
    keys: &IndexKeys,
    schema: &KeySchema,
    c: u32,
    row: &[Value],
) -> Result<Option<Vec<Value>>, Error> {
    let types: Vec<KeyType> = schema.components().iter().map(|(t, _)| t.clone()).collect();
    keys.key(c, row, &types).map_err(|e| match e {
        exec::Error::Fault(f) => Error::Refused(vec![format!(
            "an index expression faults with {} on a row",
            f.name()
        )]),
        other => broken(other),
    })
}

/// Refuses the rows `b.data` gives the relations `b.rewritten` names when two of them share a key
/// of a unique of `specs`, each keyed by `keyed`: each unique and the keys rows share.
fn duplicates(
    schema: &Schema,
    keys: &OrderKeys,
    normal: Option<&NormalProgram>,
    specs: &BTreeMap<u64, IndexSpec>,
    keyed: &BTreeMap<u64, Arc<IndexKeys>>,
    b: &Built,
) -> Result<(), Error> {
    let mut why = Vec::new();
    for (id, spec) in specs {
        let rid = rel_id(schema, spec.relation);
        let Some(ctors) = b
            .data
            .get(&rid)
            .filter(|_| spec.unique && b.rewritten.contains(&rid))
        else {
            continue;
        };
        let ks = key_schema(schema, normal, keys, spec)?;
        let mut seen = BTreeSet::new();
        let mut shared = Vec::new();
        for &c in &spec.ctors {
            for row in &ctors[&ctor_id(schema, spec.relation, c)] {
                if let Some(key) = row_key(&keyed[id], &ks, c, row)?
                    && !seen.insert(ks.encode(&key))
                {
                    shared.push(shown_key(&key));
                }
            }
        }
        if !shared.is_empty() {
            why.push(format!(
                "rows of {} share the key {} of `{}`",
                schema.relation(spec.relation).name,
                shared.join(", "),
                spec.clause
            ));
        }
    }
    if why.is_empty() {
        Ok(())
    } else {
        Err(Error::Refused(why))
    }
}

/// Returns the components of index key `key`, printed.
fn shown_key(key: &[Value]) -> String {
    let parts: Vec<String> = key.iter().map(shown_value).collect();
    match parts.as_slice() {
        [one] => one.clone(),
        _ => format!("({})", parts.join(", ")),
    }
}

fn shown_value(v: &Value) -> String {
    match v {
        Value::Text(t) => format!("{t:?}"),
        Value::I8(x) => x.to_string(),
        Value::I16(x) => x.to_string(),
        Value::I32(x) => x.to_string(),
        Value::I64(x) => x.to_string(),
        Value::Record(vs) => shown_key(vs),
        other => format!("{other:?}"),
    }
}

/// Returns whether `v`, stored as `t`, holds a value of the constructor stored as `id`.
fn holds_ctor(t: &FieldType, v: &Value, id: CtorId) -> bool {
    match (t, v) {
        (FieldType::Sum(cs), Value::Sum { ctor, fields }) => {
            cs.get(*ctor as usize).is_some_and(|c| {
                c.id == id
                    || c.fields
                        .iter()
                        .zip(fields)
                        .any(|(t, v)| holds_ctor(t, v, id))
            })
        }
        (FieldType::Record(ts), Value::Record(vs)) => {
            ts.iter().zip(vs).any(|(t, v)| holds_ctor(t, v, id))
        }
        _ => false,
    }
}

/// Returns the constructor relations `target`, of an `enable`, `disable` or `deprecate` in a
/// migration of `module`, names in `schema`: none for a constructor of a type.
fn targeted(schema: &Schema, module: &str, target: &CtorTarget) -> Vec<(RelId, u32)> {
    let CtorTarget::Relation(on) = target else {
        return Vec::new();
    };
    let m = on
        .relation
        .module
        .as_ref()
        .map_or(module, |m| m.text.as_str());
    let Some(ValueDef::Relation(r)) = schema
        .module_named(m)
        .and_then(|mid| schema.module(mid).values.get(&on.relation.name.text))
    else {
        return Vec::new();
    };
    let t = schema.ty(schema.relation(*r).elem_def);
    let names: Vec<&str> = match &on.sel {
        Some(CtorSel::One(c)) => vec![c.text.as_str()],
        Some(CtorSel::Many(cs)) => cs.iter().map(|c| c.text.as_str()).collect(),
        None => Vec::new(),
    };
    names
        .into_iter()
        .filter_map(|n| t.ctor_index(n))
        .map(|c| (*r, c))
        .collect()
}
