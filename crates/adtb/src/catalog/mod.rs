//! Hold the catalog: relations, deployments, grants and their rows, and open and migrate them.

mod admit;
pub mod arrows;
mod broken;
mod change;
pub(crate) mod codec;
mod deploy;
mod deployment;
pub(crate) mod described;
mod grants;
mod history;
mod hosts;
mod ids;
mod insert;
mod kept;
mod keying;
pub use crate::exec::layout;
mod memo;
mod migration;
mod moved;
pub(crate) mod operators;
mod oplog;
mod pending;
#[cfg(any(test, feature = "testing"))]
mod probed;
mod readers;
mod root;
mod roots;
mod rot;
mod rows;
mod steps;
mod structures;
mod validate;
pub mod values;

pub use structures::Structures;
pub(crate) use structures::index_holder;
#[cfg(any(test, feature = "testing"))]
pub(crate) use structures::relation_holder;

#[cfg(test)]
mod admit_tests;
#[cfg(test)]
mod catalog_tests;
#[cfg(test)]
mod commit_point_tests;
#[cfg(test)]
mod fn_dependents_tests;
#[cfg(test)]
mod grant_rows_tests;
#[cfg(test)]
mod hash_corpus_tests;
#[cfg(test)]
mod index_text_tests;
#[cfg(test)]
mod memo_tests;
#[cfg(test)]
mod oplog_tests;
#[cfg(test)]
mod tests;

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, LazyLock, OnceLock};

use adtb_lang::check::Program;
use adtb_lang::evolve::{Privilege, Source};
use adtb_lang::norm::{self, NormalProgram};
use adtb_lang::parse::{Options, parse_migration};
use adtb_lang::schema::{self, Committed, Schema};
use adtb_lang::span::Diag;
use adtb_lang::types::{KeyForm, RelId, Ty};

use crate::btree::{self, BTree, Index};
use crate::exec::{self, IndexedRows, StoredHeap, StoredIndex};
use crate::faults;
use crate::heap::{self, FieldType, Heap, LargeStore};
use crate::key::{CtorId, Dir, KeySchema, OrderKey, Tid, Value};
use crate::mvcc::{self, DurableCommit, Hold, Scope, Transactions, Xid};
use crate::plan::{self, HeapId, IndexDesc, IndexId, RelationDesc};
use crate::runtime::Runtime;
use crate::storage::{self, Lsn, PageError, PageKind, Pager, Pid};
use crate::verify::{self, Depth, Problem, Report};

pub use broken::Broken;
use broken::refusal;
pub use deploy::Rewrite;
pub use deployment::{Deployed, Deployment, Fact, Mark, Origin};
pub use grants::Dormant;
pub use ids::engine_relation;
pub use kept::{KeptField, KeptRow};
pub use keying::{IndexKeys, element_types};
use keying::{broken, index_keys, row_key};
pub use layout::OrderKeys;
use memo::Memo;
#[cfg(any(test, feature = "testing"))]
pub use oplog::chained;
pub use oplog::{Hash, VersionHash, version_hash};
pub use readers::{Read, Reader, source_at, stale};
pub(crate) use root::Begun;
pub(crate) use root::Changing;
#[cfg(any(test, feature = "testing"))]
pub(crate) use root::Unbegun;
pub use rot::{Rot, detail};
pub use rows::{ElementTexts, IndexSpec, Rows, indexed_fields, indexes};
#[cfg(test)]
pub use rows::{applied, declared};
pub use validate::Denial;

use admit::{Admitted, Indexing};
use change::Data;
use history::{Kept, key_indexes};
use ids::{CatCtorId, CatRelId, ctor_id, ctor_index, held_id, rel_id, relation, stored};
use readers::{Pending, Readers};
use root::Root;
use roots::RootsRow;

/// The name of the schema owner's role, the grantor of every log grant.
pub use adtb_lang::evolve::OWNER_ROLE;

/// The heap id of the large-value store every heap shares.
const LARGE: u32 = 1;
/// The heap id of the heap that names every other heap's and index's root.
pub(crate) const ROOTS: u32 = 2;
const FIRST_HEAP: u32 = 3;

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
    /// The migration does not apply; the text of the first operator the data refuses, or of the
    /// operator whose step fails, and each reason.
    RefusedBy {
        operator: String,
        why: Vec<String>,
    },
    /// A stored page or row does not read as the catalog writes it; what is wrong.
    Corrupt(Rot),
    /// A stored migration does not parse or apply under this engine; its version and each
    /// reason.
    Unaccepted(u64, Vec<String>),
    /// A guarantee of the checker, planner, executor or catalog does not hold; the one named.
    Internal(Box<Broken>),
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
        match e {
            btree::Error::Key(e) => e.into(),
            e => Error::Index(e),
        }
    }
}

impl From<crate::key::EncodeError> for Error {
    fn from(e: crate::key::EncodeError) -> Error {
        Broken::IndexKey(e.into()).into()
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
    /// The hash of the version `source` is at.
    hash: VersionHash,
    schema: Committed,
    /// The checked program of `source`.
    program: Arc<Program>,
    texts: ElementTexts,
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
    /// The grantee, privilege and target of each grant [`Catalog::replay_holding`] keeps dormant.
    held: BTreeSet<(String, Privilege, String)>,
    /// The catalog rows of the objects of `source`, once read.
    objects: Arc<OnceLock<Rows>>,
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
pub fn engine_schema() -> Committed {
    static SCHEMA: LazyLock<Committed> =
        LazyLock::new(|| schema::build(&[]).expect("the engine's declarations build"));
    SCHEMA.clone()
}

/// Returns `schema`, the schema of a settled source.
fn committed(schema: Schema) -> Result<Committed, Error> {
    Committed::new(schema).ok_or_else(|| Error::from(Broken::Unsettled))
}

fn int(n: u64) -> Value {
    Value::I64(i64::try_from(n).expect("an id fits 63 bits"))
}

fn row_type(schema: &Schema, r: RelId, ctor: u32) -> Vec<FieldType> {
    layout::row(schema, r, ctor)
        .iter()
        .map(|(_, t)| layout::field_type(schema, t))
        .collect()
}

/// Errors:
/// - [`Error::Internal`] if an element is derived and `normal` is `None`, or a path
///   names no field or constructor.
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
) -> Result<Option<NormalProgram>, Box<Broken>> {
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
        .map_err(|e| Box::new(Broken::StoredNotNormal(e)))
}

fn index_ctors(schema: &Schema, spec: &IndexSpec) -> BTreeSet<CtorId> {
    spec.ctors
        .iter()
        .map(|&c| stored(ctor_id(schema, spec.relation, c)))
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
        .ids()
        .relation_ids()
        .flat_map(|(r, _)| (0..ctor_count(schema, r)).map(move |c| (r, c)))
        .collect()
}

/// Returns the planner's description of every relation of `schema` over the heaps `heaps`, by
/// relation and constructor, and the indexes `specs`.
fn planned<'s>(
    schema: &Schema,
    specs: impl Iterator<Item = &'s IndexSpec> + Clone,
    heaps: &BTreeMap<(RelId, u32), HeapId>,
) -> plan::Catalog {
    let mut out = BTreeMap::new();
    for (r, _) in schema.ids().relation_ids() {
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
            key_index: IndexId(held_id(schema.ids().key_id(r)).raw()),
            indexes,
        };
        out.insert(r, desc);
    }
    plan::Catalog::new(out)
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
    /// The roles whose log grants became dormant at each version.
    dormant: BTreeMap<u64, BTreeSet<String>>,
}

impl Named {
    /// Add what roots row `row` names.
    fn add(&mut self, row: &[Value]) -> Result<(), Error> {
        match RootsRow::decode(row)? {
            RootsRow::HeapRoot {
                rel,
                ctor,
                heap,
                at,
            } => {
                self.heaps.insert((rel, ctor), (heap, at));
            }
            RootsRow::IndexRoot { index, at } => {
                self.indexes.insert(index, at);
            }
            RootsRow::OrderKey { ctor, key } => {
                self.keys.insert(ctor, key);
            }
            RootsRow::Migration { version, text } => {
                self.migrations.insert(version, text);
            }
            RootsRow::Dormant { version, role } => {
                self.dormant.entry(version).or_default().insert(role);
            }
            RootsRow::HistoryHeap {
                rel,
                ctor,
                heap,
                at,
            } => self.history.heap((rel, ctor), (heap, at)),
            RootsRow::HistoryIndex { rel, owner, at } => self.history.index(rel, (owner, at)),
            RootsRow::Held {
                dropped,
                module,
                name,
                holder,
                kind,
            } => pending::add(&mut self.pending, dropped, module, name, holder, kind)?,
            RootsRow::Deployment(_) | RootsRow::Kept { .. } => {}
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
    let rows = if let Some(damage) = pager.whole_cause() {
        latest_rows(pager, txns, roots, damage)?
    } else {
        let hold = txns.take(Scope::Heaps(BTreeSet::from([ROOTS])))?;
        let rows = visible(pager, txns, &hold, roots);
        drop(hold);
        rows?
    };
    let mut out = Named::default();
    for row in rows {
        out.add(&row)?;
    }
    Ok(out)
}

/// Returns the rows of `heap` in the latest state of a database whose commit log has `damage`.
///
/// Fail with `damage` when a version's writer or deleter has a damaged status page and no commit
/// record in the retained log, so no uncommitted row reads as committed.
fn latest_rows<R: Runtime>(
    pager: &Pager<R>,
    txns: &Transactions<R>,
    heap: &Heap,
    damage: PageError,
) -> Result<Vec<Vec<Value>>, PageError> {
    let logged = pager.logged_commits();
    'scan: loop {
        let mut versions = Vec::new();
        heap.scan(pager, |v| versions.push(v))?;
        let mut rows = Vec::new();
        for v in versions {
            let live = txns.latest_live(pager, &v.header.writers, &logged);
            let Some(live) = v.looked_up(pager, live)? else {
                continue 'scan;
            };
            if live.ok_or(damage)? {
                rows.push(v.row);
            }
        }
        return Ok(rows);
    }
}

/// Returns the rows of the versions of `heap` visible at the snapshot of `hold`, in heap order.
fn visible<R: Runtime>(
    pager: &Pager<R>,
    txns: &Transactions<R>,
    hold: &Hold,
    heap: &Heap,
) -> Result<Vec<Vec<Value>>, PageError> {
    let reader = txns.reader(pager, hold, None);
    'scan: loop {
        let mut versions = Vec::new();
        heap.scan(pager, |v| versions.push(v))?;
        let mut out = Vec::new();
        for v in versions {
            let Some(seen) = v.looked_up(pager, reader.sees(&v.header.writers))? else {
                continue 'scan;
            };
            if seen.visible() {
                out.push(v.row);
            }
        }
        return Ok(out);
    }
}

/// The texts of a stored log's migrations and the roles each makes dormant, by version.
type Log = (BTreeMap<u64, String>, BTreeMap<u64, BTreeSet<String>>);

/// The source a log folds to, its schema and its checked program.
type Folded = (Source, Committed, Arc<Program>, VersionHash);

/// The most logs [`FOLDED`] holds: the distinct logs a process reopens while it works. A server
/// on one database reopens 1 or 2; a test sweep reopens one for each schema it sweeps.
const FOLDED_LOGS: usize = 8;

/// The logs this process folded most recently, and what each folded to. A fold depends on nothing
/// but its log.
static FOLDED: Memo<Log, Folded> = Memo::new(FOLDED_LOGS);

/// Returns the source the migrations `texts` fold to, in version order from the empty schema,
/// and its schema and checked program. Each grant of a role `dormant` names at a version is
/// dormant once the migration of that version applies.
fn fold(
    texts: &BTreeMap<u64, String>,
    dormant: &BTreeMap<u64, BTreeSet<String>>,
) -> Result<Folded, Error> {
    let folded = FOLDED.get_or_try_insert((texts.clone(), dormant.clone()), || {
        fold_from_empty(texts, dormant)
    })?;
    Ok((*folded).clone())
}

fn fold_from_empty(
    texts: &BTreeMap<u64, String>,
    dormant: &BTreeMap<u64, BTreeSet<String>>,
) -> Result<Folded, Error> {
    let mut source = Source::default();
    for (&version, text) in texts {
        let refused = |why: Vec<String>| Error::Unaccepted(version, why);
        let file = parse_migration(text, Options::default())
            .ok()
            .map_err(|ds| refused(ds.into_iter().map(|d| d.message).collect()))?;
        source = source
            .migrate(&file, text)
            .map_err(|r| refusal(r, refused))?;
        if let Some(roles) = dormant.get(&version) {
            for g in &mut source.grants {
                g.dormant |= roles.contains(&g.role);
            }
        }
    }
    let (schema, program) = source.check().map_err(|ds| {
        Error::Unaccepted(
            source.version,
            ds.into_iter().map(|d| d.diag.message).collect(),
        )
    })?;
    Ok((
        source,
        committed(schema)?,
        Arc::new(program),
        VersionHash::default(),
    ))
}

/// Returns whether `keys` holds an order key for each constructor of every declared sum of
/// `schema`, rising in declaration order.
fn check_order_keys(schema: &Schema, keys: &OrderKeys) -> Result<(), Error> {
    for cs in layout::declared_sums(schema) {
        let mut last: Option<&OrderKey> = None;
        for id in cs {
            let Some(key) = keys.get(&id) else {
                return Err(Rot::NoOrderKey(id).into());
            };
            if last.is_some_and(|l| l >= key) {
                return Err(Rot::OrderKeyOrder(id).into());
            }
            last = Some(key);
        }
    }
    Ok(())
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
        None => Rot::Unfinished(report.findings.clone()).into(),
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
        let root = Root::created(txns.commit_log().root(), FIRST_HEAP);
        pager.update(&[pid], |p| root.write(p[0]))?;
        pager.set_catalog_root(pid);
        let catalog = Catalog::fill(pager, &txns, pid)?;
        Ok((catalog, txns))
    }

    /// Create the engine's structures and rows in a file whose root page `pid` names the commit
    /// log of `txns` and nothing else, and name them in the root page.
    ///
    /// Commit the transaction that writes the rows and name the roots heap in the root page in
    /// the log batch of its commit record, the create's commit point. A failure fails the call; an
    /// open that finds no roots heap named frees what this made and fills the file again.
    fn fill<R: Runtime>(
        pager: &Pager<R>,
        txns: &Transactions<R>,
        pid: Pid,
    ) -> Result<Catalog, Error> {
        let large = LargeStore::create(pager, LARGE)?;
        let roots = Heap::create(pager, ROOTS, roots::row_type(), large)?;
        let kept = Kept::create(pager, FIRST_HEAP, FIRST_HEAP + 1)?;
        let named = (roots.id().0, roots.root());
        let next = Checked::engine()?;
        let empty = Catalog {
            source: Source::default(),
            hash: VersionHash::default(),
            schema: engine_schema(),
            program: next.program.clone(),
            texts: next.texts.clone(),
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
            held: BTreeSet::new(),
            objects: Arc::default(),
        };
        let begun = txns.begin(pager)?;
        let x = begun.xid();
        let every = empty
            .schema
            .ids()
            .relation_ids()
            .map(|(_, id)| id)
            .collect();
        let hold = txns.take(Scope::Every)?;
        let m = Migrating {
            pager,
            txns,
            x,
            hold: &hold,
            made: Default::default(),
            top: Default::default(),
        };
        let built = Indexing::new(&empty, &next, None).and_then(|indexing| {
            let data = admit::admit(pager, &next.schema, &every, &indexing, Data::new())?;
            empty.build(
                &m,
                &Built {
                    next: &next,
                    indexing: &indexing,
                    data: &data,
                    rewritten: &every,
                    old: &Rows::new(),
                    operators: &[],
                },
            )
        });
        drop(hold);
        let catalog = match built {
            Ok(catalog) => catalog,
            Err(e) => {
                txns.abandon_failed(pager, begun)?;
                return Err(e);
            }
        };
        txns.commit_changing(pager, begun, pid, |p| {
            let mut root = Root::read(p);
            root.large = (large.id(), large.root());
            root.roots = named;
            root.next_heap = catalog.next_heap;
            catalog.kept.name(&mut root);
            root.write(p);
        })?;
        Ok(catalog)
    }

    /// Open the catalog the file header of a recovered `pager` names, and the commit log it
    /// names, with every committed migration applied.
    ///
    /// A file whose create did not commit has every page but its commit log's and its root
    /// page freed, and its catalog created again; a file whose header names no catalog has
    /// every page freed first. A schema change left on the root has every page no structure
    /// reaches freed, and every large-value head lowered to the versions that point at it.
    ///
    /// Errors:
    /// - [`Error::Corrupt`] if a stored page or row does not read as the catalog writes it.
    /// - [`Error::Unaccepted`] if a stored migration does not parse or apply under this engine.
    ///
    /// A schema change that did not commit and whose pages damage hides stays on the root for the
    /// next open to free.
    pub fn open<R: Runtime>(rt: R, pager: &Pager<R>) -> Result<(Catalog, Transactions<R>), Error> {
        let (mut catalog, txns) = Catalog::load(rt, pager)?;
        if pager.refused_whole() {
            return Ok((catalog, txns));
        }
        let changing = Root::read(&pager.read(catalog.root)?).changing();
        if changing != Changing::None {
            let freed = catalog.free_leaked(pager, &txns, &[]);
            match freed.and_then(|f| catalog.lower_overcounted(pager, &f.report)) {
                Ok(()) => {
                    root::end_found(pager, catalog.root)?;
                }
                Err(Error::Page(e)) if e.damage != storage::Damage::Unreadable => {}
                Err(e) => return Err(e),
            }
        }
        catalog.tag_grants(pager, &txns)?;
        Ok((catalog, txns))
    }

    /// Returns the catalog the file header names and its commit log, reading no row of a heap but
    /// the roots heap's. Leaves what a crashed schema change allocated, and every grant untagged.
    pub fn load<R: Runtime>(rt: R, pager: &Pager<R>) -> Result<(Catalog, Transactions<R>), Error> {
        let pid = pager.catalog_root();
        let refused = || Error::from(Rot::MarkedNoCatalog);
        if pid == Pid(0) {
            if pager.refused_whole() {
                return Err(refused());
            }
            for p in pager.allocated()? {
                pager.free(p)?;
            }
            return Catalog::create(rt, pager);
        }
        let root = {
            let page = pager.read(pid)?;
            let found = (storage::kind(&page), storage::format(&page));
            if found != (PageKind::Catalog as u16, root::FORMAT) {
                return Err(Rot::RootPage { pid, found }.into());
            }
            Root::read(&page)
        };
        let txns = Transactions::open(rt, pager, root.commit_log)?;
        let marked = pager.refused_whole();
        if root.roots.1 == Pid(0) {
            if marked {
                return Err(refused());
            }
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
        let roots = Heap::open(pager, root.roots.0, root.roots.1, roots::row_type(), large)?;
        let named = named(pager, &txns, &roots)?;
        let (source, schema, program, hash) = fold(&named.migrations, &named.dormant)?;
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
        let texts = ElementTexts::of(&source, &schema)?;
        let (held, held_program, held_texts) = readers
            .enforced()
            .map_or((&schema, &*program, &texts), |e| {
                (&e.schema, &*e.program, &e.texts)
            });
        let specs =
            pending::kept_indexes(held, held_texts, &readers.pending).map_err(Rot::KeptIndexes)?;
        let normal = index_program(held, held_program, specs.values()).map_err(Rot::KeptIndexes)?;
        let mut stored = BTreeMap::new();
        for (id, spec) in specs {
            let Some(&at) = named.indexes.get(&id) else {
                return Err(Error::Missing(format!("index {id}")));
            };
            let index = Index::open(
                BTree::open(at, owner(id)),
                key_schema(held, normal.as_ref(), &named.keys, &spec)
                    .map_err(|e| Rot::IndexKeying(Box::new(e)))?,
                index_ctors(held, &spec),
            );
            let keys = index_keys(held, normal.as_ref(), &spec, &Arc::default())
                .map_err(|e| Rot::IndexKeying(Box::new(e)))?;
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
            hash,
            schema,
            program,
            texts,
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
            held: BTreeSet::new(),
            objects: Arc::default(),
        };
        Ok((catalog, txns))
    }

    /// Returns the catalog of a file whose own cannot be read: the engine's declarations with no
    /// heap, index or page behind them.
    pub fn unread<R: Runtime>(pager: &Pager<R>) -> Result<Catalog, Error> {
        let (schema, program) = Source::default()
            .check()
            .map_err(|ds| Error::from(Broken::Declarations(ds)))?;
        let large = LargeStore::open(0, Pid(0));
        Ok(Catalog {
            texts: ElementTexts::of(&Source::default(), &schema)?,
            source: Source::default(),
            hash: VersionHash::default(),
            schema: committed(schema)?,
            program: Arc::new(program),
            root: pager.catalog_root(),
            large,
            roots: Arc::new(Heap::open(pager, ROOTS, Pid(0), roots::row_type(), large)?),
            heaps: BTreeMap::new(),
            indexes: BTreeMap::new(),
            keys: OrderKeys::default(),
            next_heap: FIRST_HEAP,
            sequences: Pid(0),
            query_memory: DEFAULT_QUERY_MEMORY,
            readers: Readers::default(),
            hosts: Arc::default(),
            kept: Kept::unread(),
            held: BTreeSet::new(),
            objects: Arc::default(),
        })
    }

    /// Returns whether this is a catalog [`Catalog::unread`] made.
    pub fn is_unread(&self) -> bool {
        self.heaps.is_empty()
    }

    /// Free every page the catalog's structures do not reach, first counting each version on a
    /// page of a heap of `gone` off the large values it points at. A page whose versions it
    /// could not count off is freed with its heads left overcounted.
    ///
    /// Errors:
    /// - The damage found, freeing nothing, if a damaged page keeps it from finding
    ///   every structure's pages.
    ///
    /// Precondition: no structure reaches a heap of `gone`, no transaction reads one, and no
    /// call counted its versions off before.
    fn free_leaked<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        gone: &[Arc<Heap>],
    ) -> Result<Freed, Error> {
        let report = {
            let clog = txns.commit_log();
            let structures = self.structures();
            verify::verify_with(pager, &structures.inventory(&clog), Depth::STRUCTURE)
        };
        if !report.complete {
            return Err(unfinished(&report));
        }
        let gone: BTreeMap<u32, &Heap> = gone.iter().map(|h| (h.id().0, h.as_ref())).collect();
        let mut counted = true;
        for p in report.leaked() {
            let owner = match pager.read(p) {
                Err(e) if e.damage != storage::Damage::Unreadable => {
                    pager.scrap(p)?;
                    continue;
                }
                Err(_) => None,
                Ok(page) => Some(page.owner()),
            };
            if let Some(heap) = owner.and_then(|o| gone.get(&o)) {
                counted &= heap.release_page(pager, p);
            }
            pager.free(p)?;
        }
        Ok(Freed { report, counted })
    }

    /// Lower each head `report` finds counting more versions than point at it to the number
    /// that do, leaving a head whose change is abandoned as it is.
    ///
    /// Precondition: `report` is complete, found over this catalog's structures, and no write
    /// ran since.
    fn lower_overcounted<R: Runtime>(
        &self,
        pager: &Pager<R>,
        report: &Report,
    ) -> Result<(), Error> {
        let stores = [self.large, self.kept.large()];
        for f in &report.findings {
            let Problem::Overcounted { slot, versions, .. } = f.problem else {
                continue;
            };
            let owner = pager.read(f.pid)?.owner();
            let store = stores
                .iter()
                .find(|s| s.id() == owner)
                .expect("an overcounted head lies in a store the catalog names");
            let head = Tid::new(f.pid.0, slot).expect("a page id fits 48 bits");
            match store.recount(pager, head, versions) {
                Ok(()) | Err(heap::Refused::Gone) => {}
                Err(heap::Refused::Page(e)) => return Err(e.into()),
            }
        }
        Ok(())
    }

    /// Returns every heap of this catalog, current and history.
    fn every_heap(&self) -> impl Iterator<Item = &Arc<Heap>> {
        self.heaps.values().chain(self.kept.heaps())
    }

    /// Returns each heap of this catalog, current or history, that `next` does not hold.
    fn heaps_gone_in(&self, next: &Catalog) -> Vec<Arc<Heap>> {
        let held: BTreeSet<u32> = next.every_heap().map(|h| h.id().0).collect();
        self.every_heap()
            .filter(|h| !held.contains(&h.id().0))
            .cloned()
            .collect()
    }

    /// Free every page a schema change left unreached once it is `committed`, counting the
    /// versions of the heaps it took out of the catalog, `gone`, off their large values; then end
    /// `marked`, or with none the change the root records, and wait until the end is durable.
    /// Leave the change on the root for the next open when the free fails or leaves a version
    /// not counted off.
    #[track_caller]
    fn end_committed<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        marked: Option<Begun>,
        committed: &DurableCommit,
        gone: &[Arc<Heap>],
    ) {
        let freed = committed.then(faults::Step::FreeLeaked, || {
            self.free_leaked(pager, txns, gone)
        });
        if !freed.is_some_and(|f| f.counted) {
            return;
        }
        let ended = committed.then(faults::Step::EndChange, || {
            root::end_freed(pager, self.root, marked)
        });
        if let Some(lsn) = ended {
            pager.wait_durable(lsn);
        }
    }

    /// Returns the schema and program constraint checks and validation run against: the
    /// version's, with every pending-dropped object kept.
    pub fn checking(&self) -> (&Committed, &Arc<Program>) {
        self.readers
            .enforced()
            .map_or((&self.schema, &self.program), |e| (&e.schema, &e.program))
    }

    /// Returns the catalog rows of the objects of the catalog's version.
    fn objects(&self) -> &Rows {
        self.objects
            .get_or_init(|| rows::objects(&self.schema, &self.source, &self.texts))
    }

    /// Returns the index element texts of the schema [`Catalog::checking`] returns.
    fn checking_texts(&self) -> &ElementTexts {
        self.readers.enforced().map_or(&self.texts, |e| &e.texts)
    }

    /// Returns every index of the schema [`Catalog::checking`] returns, by index id.
    pub fn declared_indexes(&self) -> BTreeMap<u64, IndexSpec> {
        indexes(self.checking().0, self.checking_texts())
    }

    /// Set the memory a migration's validation of its constraints may hold: the query memory
    /// budget of the open database.
    pub fn set_query_memory(&mut self, bytes: usize) {
        self.query_memory = bytes;
    }

    pub fn schema(&self) -> &Committed {
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

    /// Returns the hash of the version the catalog is at, made at most once for every holder.
    pub fn version_hash(&self) -> VersionHash {
        self.hash.clone()
    }

    /// Returns the page the file header names as the catalog's root.
    #[cfg(test)]
    pub fn root(&self) -> Pid {
        self.root
    }

    pub fn heap(&self, r: RelId, ctor: u32) -> &Heap {
        self.heaps
            .get(&(r, ctor))
            .unwrap_or_else(|| panic!("relation {r:?} has a heap for constructor {ctor}"))
    }

    #[cfg(test)]
    pub fn index_of(&self, id: u64) -> &Index {
        &self
            .indexes
            .get(&id)
            .unwrap_or_else(|| panic!("the catalog holds index {id}"))
            .1
    }

    /// Returns the pages the schema and the visibility of every row are read from: the catalog's
    /// root, its roots heap, each heap of a relation of an engine module, the counters page and
    /// every page of the commit log of `txns`.
    pub fn read_from<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
    ) -> Result<BTreeSet<Pid>, PageError> {
        let clog = txns.commit_log();
        let mut pages = BTreeSet::from([self.root, clog.root()]);
        pages.extend(clog.status_pages().values());
        pages.extend(clog.directory_pages());
        pages.extend(self.sequences());
        pages.extend(self.roots.owned_pages(pager)?);
        for (&(r, _), heap) in &self.heaps {
            if self.schema.module(self.schema.relation(r).module).engine {
                pages.extend(heap.owned_pages(pager)?);
            }
        }
        Ok(pages)
    }

    /// Returns the write path's page of generated counters, once one is named.
    pub fn sequences(&self) -> Option<Pid> {
        (self.sequences != Pid(0)).then_some(self.sequences)
    }

    /// Mark the catalog as rebuilding indexes, so an open after a crash frees the pages no
    /// structure names. Returns the change begun, or `None`, marking nothing, when the root page
    /// already records a change in progress, which an open after a crash frees as well.
    ///
    /// Precondition: the database is open, so no schema change is running.
    pub fn begin_reindexing<R: Runtime>(&self, pager: &Pager<R>) -> Result<Option<Begun>, Error> {
        Ok(root::begin_or_join(pager, self.root, Changing::Reindexing)?)
    }

    /// End the change [`Catalog::begin_reindexing`] began.
    ///
    /// Fail with [`Broken::ChangeMoved`], writing nothing, when the root page records another
    /// change in progress.
    pub fn end_reindexing<R: Runtime>(&self, pager: &Pager<R>, begun: Begun) -> Result<(), Error> {
        root::end(pager, begun, |_| {})?;
        Ok(())
    }

    /// Mark the catalog whose root page is `root` as changing, before the write path allocates
    /// its page of generated counters, so an open after a crash frees that page until
    /// [`Catalog::name_sequences`] names it.
    ///
    /// Fail with [`Unbegun::Kept`], marking nothing, when the root page records a change in
    /// progress.
    #[cfg(any(test, feature = "testing"))]
    pub fn naming_sequences<R: Runtime>(pager: &Pager<R>, root: Pid) -> Result<Begun, Unbegun> {
        root::begin(pager, root, Changing::NamingSequences)
    }

    /// Name `pid` as the write path's page of generated counters in the catalog whose root page
    /// is `root`, ending `naming` in the same write. With no `naming`, the change in progress
    /// stays for the next open to free. Returns once the naming is durable.
    ///
    /// Fail with [`Broken::ChangeMoved`], naming nothing, when the root page records another
    /// change in progress than `naming`.
    pub fn name_sequences<R: Runtime>(
        pager: &Pager<R>,
        root: Pid,
        pid: Pid,
        naming: Option<Begun>,
    ) -> Result<(), Error> {
        let lsn = match naming {
            Some(begun) => root::end(pager, begun, |r| r.sequences = pid)?,
            None => update_root(pager, root, |r| r.sequences = pid)?,
        };
        pager.wait_durable(lsn);
        Ok(())
    }

    /// Returns the page of the database's id counters. When the root page names none, allocate
    /// one, advance the counter of each relation whose single key field is generated by
    /// `sequence` past every key the relation holds, and name the page, durably. A change in
    /// progress the root page already records stays there for the next open to free.
    ///
    /// Precondition: no `write::Database` holds the page of id counters.
    pub fn counters<R: Runtime>(&mut self, pager: &Pager<R>) -> Result<Pid, Error> {
        if let Some(pid) = self.sequences() {
            return Ok(pid);
        }
        let naming = root::begin_or_join(pager, self.root, Changing::NamingSequences)?;
        let ids = &self.schema.ids();
        let pid = crate::write::counter_page(pager, &self.schema, |r| {
            ids.key_id(r).map(|k| &self.indexes[&k.raw()].1)
        })
        .map_err(|e| match e {
            crate::write::Error::Exec(exec::Error::Page(e)) => Error::Page(e),
            other => Error::from(Broken::KeyCounter(other)),
        })?;
        Catalog::name_sequences(pager, self.root, pid, naming)?;
        self.sequences = pid;
        Ok(pid)
    }

    /// Returns the heap of every constructor relation and every index, as the executor and the
    /// write path bind them.
    pub fn bindings(&self) -> exec::Bindings {
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
            .map(|(spec, index, keys)| {
                let ctors = spec
                    .ctors
                    .iter()
                    .map(|&c| (stored(ctor_id(schema, spec.relation, c)), c))
                    .collect();
                let heaps = spec
                    .ctors
                    .iter()
                    .map(|&c| {
                        let heap = &self.heaps[&(spec.relation, c)];
                        (c, HeapId(u64::from(heap.id().0)))
                    })
                    .collect();
                (
                    IndexId(spec.id),
                    StoredIndex {
                        index: index.clone(),
                        ctors,
                        rows: Some(IndexedRows {
                            keys: keys.clone(),
                            heaps,
                        }),
                    },
                )
            })
            .collect();
        exec::Bindings::new(Arc::new(heaps), Arc::new(indexes))
    }

    /// Returns the rows of relation `r` visible at the snapshot of `hold`, each with its
    /// constructor index, by constructor and then in heap order.
    fn rows<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        hold: &Hold,
        r: RelId,
    ) -> Result<Vec<(u32, Vec<Value>)>, PageError> {
        let mut out = Vec::new();
        for ((_, c), heap) in self.heaps.range((r, 0)..=(r, u32::MAX)) {
            out.extend(
                visible(pager, txns, hold, heap)?
                    .into_iter()
                    .map(|row| (*c, row)),
            );
        }
        Ok(out)
    }

    /// Returns the planner's description of every relation.
    pub fn plan(&self) -> plan::Catalog {
        if self.is_unread() {
            return plan::Catalog::default();
        }
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
}

/// What [`Catalog::build`] makes a catalog of.
struct Built<'a> {
    next: &'a Checked,
    indexing: &'a Indexing<'a>,
    data: &'a Admitted,
    /// The ids of the relations whose heaps and indexes are made again.
    rewritten: &'a BTreeSet<CatRelId>,
    /// The catalog rows of the catalog before the migration, by its relations.
    old: &'a Rows,
    operators: &'a [oplog::OperatorRow],
}

/// A source, the schema it builds and its checked program.
#[derive(Clone, Debug)]
struct Checked {
    source: Source,
    schema: Committed,
    program: Arc<Program>,
    texts: ElementTexts,
    objects: Arc<OnceLock<Rows>>,
}

impl Checked {
    /// Returns `source` with its schema and program. Fail with [`Broken::Unsettled`] when the
    /// schema takes a relation as added later, and with [`Error::Refused`] when an index element
    /// is a set.
    fn new(source: Source, schema: Schema, program: Program) -> Result<Checked, Error> {
        let texts = ElementTexts::of(&source, &schema)?;
        let sets: Vec<String> = program
            .index_fields
            .iter()
            .filter(|(_, f)| {
                matches!(program.terms.ty(f.value), Ty::Fn(_, t) if t.key_form() == Some(KeyForm::Entries))
            })
            .map(|(&(r, i, j), _)| {
                format!(
                    "index element `{}` of `{}` is a set: an index with an entry for each element of a set does not run in this engine yet",
                    texts.get(r, i, j),
                    schema.relation(r).name
                )
            })
            .collect();
        if !sets.is_empty() {
            return Err(Error::Refused(sets));
        }
        Ok(Checked {
            source,
            schema: committed(schema)?,
            program: Arc::new(program),
            texts,
            objects: Arc::default(),
        })
    }

    /// Returns the engine's own declarations, checked once per process.
    fn engine() -> Result<Checked, Error> {
        static ENGINE: OnceLock<Checked> = OnceLock::new();
        if let Some(checked) = ENGINE.get() {
            return Ok(checked.clone());
        }
        let (schema, program) = Source::default()
            .check()
            .map_err(|ds| Error::from(Broken::Declarations(ds)))?;
        let checked = Checked::new(Source::default(), schema, program)?;
        Ok(ENGINE.get_or_init(|| checked).clone())
    }

    /// Returns the catalog rows of the objects of the version.
    fn objects(&self) -> &Rows {
        self.objects
            .get_or_init(|| rows::objects(&self.schema, &self.source, &self.texts))
    }
}

/// The transaction `x` of one migration, and what it reads the catalog before it at.
struct Migrating<'a, R: Runtime> {
    pager: &'a Pager<R>,
    txns: &'a Transactions<R>,
    x: Xid,
    hold: &'a Hold,
    /// Every heap the transaction created.
    made: RefCell<Vec<Arc<Heap>>>,
    /// One past the highest heap id the transaction's structures took.
    top: Cell<u32>,
}

/// What [`Catalog::free_leaked`] did.
struct Freed {
    /// The report the leaked pages were found by.
    report: Report,
    /// Whether every version on a page of a heap it was given was counted off.
    counted: bool,
}

impl<R: Runtime> Migrating<'_, R> {
    /// Insert `row` into `heap` as `self.x`, as [`insert_row`] does.
    fn insert(&self, heap: &Heap, row: &[Value]) -> Result<Tid, Error> {
        insert_row(self.pager, self.txns, heap, self.x, row)
    }

    /// Insert the rows of `rows` into `heap` as `self.x`, in order, as
    /// [`Heap::insert_all_pruning`] does, calling `placed` with each row and its TID.
    fn insert_all<V: AsRef<[Value]>>(
        &self,
        heap: &Heap,
        rows: impl IntoIterator<Item = Result<V, Error>>,
        placed: impl FnMut(V, Tid) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let mut judge = self.txns.judge_now(self.pager, heap.id().0);
        heap.insert_all_pruning(
            self.pager,
            self.x,
            rows,
            &mut judge,
            placed,
            &Abandoning(()),
        )
    }

    /// Create an empty heap with id `id` whose rows have fields `row`, out-of-line values in
    /// `large`, and add it to `made`.
    fn create_heap(
        &self,
        id: u32,
        row: Vec<FieldType>,
        large: LargeStore,
    ) -> Result<Arc<Heap>, Error> {
        let heap = Arc::new(Heap::create(self.pager, id, row, large)?);
        self.made.borrow_mut().push(heap.clone());
        self.took(id);
        Ok(heap)
    }

    /// Create an empty tree owned by heap id `id`.
    fn create_tree(&self, id: u32) -> Result<BTree, Error> {
        let tree = BTree::create(self.pager, id)?;
        self.took(id);
        Ok(tree)
    }

    fn took(&self, id: u32) {
        self.top.set(self.top.get().max(id + 1));
    }
}

/// The writer of a catalog transaction, which abandons it whole on failure.
#[derive(Clone, Copy, Debug)]
pub struct Abandoning(());

impl heap::Abandons for Abandoning {}

impl heap::UncheckedWriter for Abandoning {}

/// Insert `row` into `heap` as transaction `x`, pruning a page found without room before
/// looking further. Returns its TID.
fn insert_row<R: Runtime>(
    pager: &Pager<R>,
    txns: &Transactions<R>,
    heap: &Heap,
    x: Xid,
    row: &[Value],
) -> Result<Tid, Error> {
    let mut judge = txns.judge_now(pager, heap.id().0);
    let put = heap.insert_pruning(pager, x, row, &mut judge);
    Ok(put.kept(&Abandoning(()))?)
}

/// Returns the heap of every constructor of relation `r` among `heaps`.
fn sum_of(heaps: &BTreeMap<(RelId, u32), Arc<Heap>>, r: RelId) -> Vec<Arc<Heap>> {
    heaps
        .range((r, 0)..=(r, u32::MAX))
        .map(|(_, h)| h.clone())
        .collect()
}

/// Returns the roots row naming `heap`, constructor `cid` of relation `rid`.
fn heap_root(rid: CatRelId, cid: CatCtorId, heap: &Heap) -> RootsRow {
    RootsRow::HeapRoot {
        rel: rid,
        ctor: cid,
        heap: heap.id().0,
        at: heap.root(),
    }
}

/// Delete, as `m.x`, one version the snapshot of `m.hold` sees of each row of `rows` from `heap`.
fn delete_rows<R: Runtime>(
    m: &Migrating<'_, R>,
    heap: &Heap,
    rows: &[Vec<Value>],
) -> Result<(), Error> {
    delete_keyed(m, heap, rows.to_vec(), <[Value]>::to_vec)
}

/// Delete, as `m.x`, one version the snapshot of `m.hold` sees from `heap` of each row whose
/// image under `key` is one of `keys`.
fn delete_keyed<R: Runtime>(
    m: &Migrating<'_, R>,
    heap: &Heap,
    mut keys: Vec<Vec<Value>>,
    key: impl Fn(&[Value]) -> Vec<Value>,
) -> Result<(), Error> {
    let pager = m.pager;
    if keys.is_empty() {
        return Ok(());
    }
    let reader = m.txns.reader(pager, m.hold, None);
    'scan: loop {
        let mut versions = Vec::new();
        heap.scan(pager, |v| versions.push(v))?;
        for v in versions {
            let Some(at) = keys.iter().position(|k| *k == key(&v.row)) else {
                continue;
            };
            let Some(seen) = v.looked_up(pager, reader.sees(&v.header.writers))? else {
                continue 'scan;
            };
            if seen.visible() {
                heap.delete(pager, heap::WriteTarget::checked(v.tid, v.header), m.x)
                    .kept(&Abandoning(()))?
                    .unchecked(&Abandoning(()));
                keys.swap_remove(at);
            }
        }
        assert!(keys.is_empty(), "every row deleted is visible: {keys:?}");
        return Ok(());
    }
}
