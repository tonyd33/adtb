mod change;
mod deploy;
mod history;
mod hosts;
mod ids;
mod keying;
pub mod layout;
mod migration;
mod moved;
mod oplog;
mod pending;
mod readers;
mod root;
mod rows;
mod steps;
mod validate;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use adtb_lang::check::Program;
use adtb_lang::evolve::{Refusal, Source};
use adtb_lang::norm::{self, NormalProgram};
use adtb_lang::parse::{Options, parse_migration};
use adtb_lang::schema::{self, Schema};
use adtb_lang::span::Diag;
use adtb_lang::types::RelId;

use crate::btree::{self, BTree, Index};
use crate::exec::{self, StoredHeap, StoredIndex};
use crate::heap::{self, FieldType, Heap, HeapNo, LargeStore};
use crate::key::{CtorId, Dir, Entry, KeySchema, OrderKey, Scalar, Value};
use crate::mvcc::{self, CommitLog, Scope, Snapshot, Transactions, Xid};
use crate::plan::{self, HeapId, IndexDesc, IndexId, RelationDesc};
use crate::runtime::Runtime;
use crate::storage::{self, Lsn, PageError, PageKind, Pager, Pid};
use crate::verify::{self, Depth, Inventory, Problem, Report, Sum};

pub use deploy::Rewrite;
pub use keying::{IndexKeys, element_types};
use keying::{broken, index_keys, row_key};
pub use layout::OrderKeys;
pub use oplog::{Hash, try_version_hash, version_hash};
pub use readers::{PendingDrop, Read, Reader};
pub use rows::{IndexSpec, Rows, applied, declared, field_path, indexed_fields, indexes};

use change::Data;
use history::{HISTORY_HEAP, HISTORY_INDEX, Kept, key_indexes};
use ids::{CatCtorId, CatRelId, ctor_id, ctor_index, rel_id, relation};
use pending::{HELD_CHECK, HELD_CONSTRAINT};
use readers::{Pending, Readers};
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
    /// The migration reached a host function, by its name, with no body bound.
    MissingFunction(String),
    /// The migration reached a host function whose body is bound at another version than its
    /// declaration names; the name the body is registered under.
    OtherVersion(String),
    /// The checker refuses a schema the catalog builds and guarantees it admits; what was built
    /// and each diagnostic.
    Unchecked(String, Vec<Diag>),
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
    hosts: Arc<exec::Hosts>,
    kept: Kept,
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
    let held = || {
        let text = FieldType::Scalar(Scalar::Text);
        vec![
            FieldType::Scalar(Scalar::I64),
            text.clone(),
            text.clone(),
            text,
        ]
    };
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
        ctor(HISTORY_HEAP, ids(4)),
        ctor(HISTORY_INDEX, ids(3)),
        ctor(HELD_CONSTRAINT, held()),
        ctor(HELD_CHECK, held()),
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
    history: history::Names,
    /// The pending drops and the bundles holding each.
    pending: Vec<Pending>,
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
            (HISTORY_HEAP | HISTORY_INDEX, fields) => self.history.add(*ctor, fields)?,
            (HELD_CONSTRAINT | HELD_CHECK, fields) => {
                pending::add(&mut self.pending, *ctor, fields)?;
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
            history_large: (0, Pid(0)),
            record: (0, Pid(0)),
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
        let kept = Kept::create(pager, FIRST_HEAP, FIRST_HEAP + 1)?;
        let named = (roots.id().0, roots.root());
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
            next_heap: FIRST_HEAP + 2,
            sequences: Pid(0),
            query_memory: DEFAULT_QUERY_MEMORY,
            readers: Readers::default(),
            hosts: Arc::default(),
            kept,
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
                operators: &[],
            },
        );
        txns.release(id);
        let catalog = built?;
        txns.commit(pager, x)?;
        let lsn = update_root(pager, pid, |r| {
            r.large = (large.id(), large.root());
            r.roots = named;
            r.next_heap = catalog.next_heap;
            catalog.kept.name(r);
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
        let readers = Readers::reopened(&source, named.pending)?;
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
            let keys = index_keys(&schema, normal.as_ref(), &spec, &Arc::default())
                .map_err(|e| Error::Corrupt(format!("{e:?}")))?;
            stored.insert(id, (spec, index, keys));
        }
        let kept = Kept::open(
            pager,
            &schema,
            &root,
            &named.history,
            &heaps,
            &key_indexes(&schema, &stored),
        )?;
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
            readers,
            hosts: Arc::default(),
            kept,
        };
        if root.changing != Changing::None {
            catalog.free_leaked(pager, &txns)?;
            update_root(pager, pid, |r| r.changing = Changing::None)?;
        }
        Ok((catalog, txns))
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
    pub fn checking(&self) -> (&Schema, &Arc<Program>) {
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
                    sum: sum_of(&self.heaps, r),
                };
                (HeapId(u64::from(stored.heap.id().0)), stored)
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
            .map(|(rc, h)| (*rc, HeapId(u64::from(h.id().0))))
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
        let (past, record) = self.kept.inventory(&self.schema);
        sums.extend(past);
        Inventory {
            commit_log: Some(clog),
            sums,
            trees: vec![record],
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
        let ctor = ctor_id(&self.schema, r, c).stored();
        let mut entries = Vec::new();
        for (spec, index, keys) in self.indexes.values() {
            if spec.relation != r {
                continue;
            }
            let Some(key) = row_key(keys, index.schema(), c, row)? else {
                continue;
            };
            index.check_key(pager.page_size(), &key, ctor).map_err(|e| match e {
                btree::Error::TooLong { len, max } => {
                    let rel = self.schema.relation(r);
                    let module = &self.schema.module(rel.module).name;
                    let size = index.schema().encode(&key).len();
                    let limit = max - (len - size);
                    Error::Refused(vec![format!(
                        "index `{module}::{}.{}` needs a key payload of {size} bytes; the limit is {limit} bytes",
                        rel.name, spec.clause
                    )])
                }
                e => e.into(),
            })?;
            entries.push((index, key));
        }
        let tid = self.heap(r, c).insert(pager, x.0, row)?;
        for (index, key) in entries {
            let fresh = index.insert(pager, &Entry { key, ctor, tid })?;
            assert!(fresh, "a new row's entry is not in its index yet");
        }
        Ok(())
    }
}

/// What [`Catalog::build`] makes a catalog of.
struct Built<'a> {
    next: Checked,
    data: &'a Data,
    /// The ids of the relations whose heaps and indexes are made again.
    rewritten: &'a BTreeSet<CatRelId>,
    /// The catalog rows of the catalog before the migration, by its relations.
    old: Rows,
    operators: &'a [oplog::OperatorRow],
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

/// Returns the id of the heap of every constructor of relation `r` among `heaps`.
fn sum_of(heaps: &BTreeMap<(RelId, u32), Arc<Heap>>, r: RelId) -> Vec<HeapNo> {
    heaps
        .range((r, 0)..=(r, u32::MAX))
        .map(|(_, h)| h.id())
        .collect()
}

/// Returns the fields of the roots row naming `heap`, constructor `cid` of relation `rid`.
fn heap_root(rid: CatRelId, cid: CatCtorId, heap: &Heap) -> Vec<Value> {
    vec![
        int(rid.0),
        int(cid.0),
        int(u64::from(heap.id().0)),
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
            heap.delete(pager, heap::WriteTarget::checked(v.tid, v.header), m.x.0)?;
            left.swap_remove(at);
        }
    }
    assert!(left.is_empty(), "every row deleted is visible: {left:?}");
    Ok(())
}
