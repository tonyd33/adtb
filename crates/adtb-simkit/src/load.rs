//! Open a database of the declarations of module `app` and load rows into it with statements,
//! each row under versions no snapshot taken afterwards sees.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use adtb::catalog::Catalog;
use adtb::exec::{self, Budget, Scheduler, Val};
use adtb::runtime::Runtime;
use adtb::storage::Pager;
use adtb::write::{self, Bindings, Setup, Txn};
use adtb::{AdHocTransaction, Database, Log, OpenOptions, TxnCallOptions, TxnOptions};
use adtb_lang::check::{Checker, Context, Program, check_adhoc};
use adtb_lang::kernel::{Terms, Tm, Var};
use adtb_lang::norm;
use adtb_lang::parse::{Options, parse_adhoc, parse_type};
use adtb_lang::resolve::Visible;
use adtb_lang::schema::{Schema, ValueDef};
use adtb_lang::types::{RelId, Ty};
use adtb_reference::Value;
use adtb_sim::{Config, Outcome, Sim, SimRuntime};
use adtb_testkit::corpus::migration;
use adtb_testkit::fixture::Fixture;

use crate::harness;
use crate::open::{self, FILE};
use crate::support::to_val;

/// Open the database in [`FILE`] whose first migration adds module `app` with `decls`, creating
/// it if it does not exist.
pub fn open(rt: &SimRuntime, decls: &str) -> Database<SimRuntime> {
    open_with(rt, decls, &open::options(1))
}

/// Open the database as [`open`] does, with `options`.
pub fn open_with(rt: &SimRuntime, decls: &str, options: &OpenOptions) -> Database<SimRuntime> {
    open::opened(rt, FILE, &Log::new([migration(decls)]), options)
}

/// Load `rows`, the rows of a reference database over `schema`, into `db` by relation name,
/// as [`committed_noise`] does, then begin the transaction [`running_noise`] returns, holding the
/// read-only transaction until its writes are done. Returns that transaction.
///
/// Postconditions: those of [`committed_noise`] and [`running_noise`], every committed noise
/// version present.
pub fn rows_with_noise<'a, R: Runtime>(
    db: &'a Database<R>,
    schema: &Schema,
    rows: &adtb_reference::Database,
) -> AdHocTransaction<'a, R> {
    load(&Api(db), schema, rows, Running::Left).expect("the running transaction is returned")
}

/// Load `rows`, the rows of a reference database over `schema`, into `db` by relation name,
/// holding a read-only transaction begun before the first write until the last ends.
///
/// Postconditions, for each row:
/// - the row is the one version of its key a snapshot begun afterwards sees;
/// - the file holds, hidden from such a snapshot, a version of the row's earlier value and one of
///   its deleted copy, each committed and deleted by a committed transaction;
/// - the file holds the row's aborted copy, inserted by a transaction that rolled back, unless a
///   write after it pruned that version from a full page;
/// - the earlier value, the deleted copy and the aborted copy each differ from the row and from
///   one another where a field outside the key is text or an instant; otherwise the copies are
///   the row and there is no earlier value.
pub fn committed_noise<R: Runtime>(
    db: &Database<R>,
    schema: &Schema,
    rows: &adtb_reference::Database,
) {
    load(&Api(db), schema, rows, Running::None);
}

/// Begin a transaction over `db`, loaded with `rows` as [`committed_noise`] loads them, and in
/// it replace each row that has an earlier value by its running copy and delete each other row.
/// Return it, not ended; it rolls back when dropped. The running copy differs from the row and
/// from its other copies as [`committed_noise`] states.
///
/// Postconditions:
/// - its writes may have pruned committed noise versions from the full pages they went to;
///   [`rows_with_noise`] keeps them.
pub fn running_noise<'a, R: Runtime>(
    db: &'a Database<R>,
    schema: &Schema,
    rows: &adtb_reference::Database,
) -> AdHocTransaction<'a, R> {
    let s = Api(db);
    let sets = Sets::of(s.schema(), schema, rows);
    let mut tx = s.begin();
    sets.run(&s, &mut tx, Write::Replace, Version::Running);
    tx
}

/// The transactions a load runs its statements in.
trait Session {
    type Txn;

    fn schema(&self) -> &Schema;
    fn begin(&self) -> Self::Txn;
    /// Begin a transaction that writes nothing and holds its snapshot until it ends.
    fn hold(&self) -> Self::Txn;
    fn run(&self, tx: &mut Self::Txn, text: &str, params: &[(&str, Ty, Val)]);
    fn commit(&self, tx: Self::Txn);
    fn rollback(&self, tx: Self::Txn);
}

/// Whether a load leaves a transaction running.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Running {
    Left,
    None,
}

fn load<S: Session>(
    s: &S,
    schema: &Schema,
    rows: &adtb_reference::Database,
    running: Running,
) -> Option<S::Txn> {
    let sets = Sets::of(s.schema(), schema, rows);
    let held = s.hold();
    let once = |write: Write, version: Version| {
        let mut tx = s.begin();
        sets.run(s, &mut tx, write, version);
        s.commit(tx);
    };
    let mut aborted = s.begin();
    sets.run(s, &mut aborted, Write::Insert, Version::Aborted);
    s.rollback(aborted);
    once(Write::Insert, Version::Deleted);
    let mut tx = s.begin();
    s.run(&mut tx, &sets.deletes(), &[]);
    s.commit(tx);
    once(Write::Insert, Version::Earlier);
    once(Write::Replace, Version::Row);
    let left = (running == Running::Left).then(|| {
        let mut tx = s.begin();
        sets.run(s, &mut tx, Write::Replace, Version::Running);
        tx
    });
    s.rollback(held);
    left
}

#[derive(Clone, Copy)]
enum Write {
    Insert,
    Replace,
}

/// Which value of each row a statement writes.
#[derive(Clone, Copy)]
enum Version {
    Row,
    /// Changed by `~a` or 1 µs where it can be, else the row.
    Aborted,
    /// Changed by `~d` or 2 µs where it can be, else the row.
    Deleted,
    /// Changed by `~e` or 3 µs; only rows that can be changed.
    Earlier,
    /// Changed by `~r` or 4 µs; only rows that can be changed.
    Running,
}

impl Version {
    /// Returns the text suffix and instant offset of the version, `None` for the row itself.
    fn change(self) -> Option<(&'static str, i64)> {
        match self {
            Version::Row => None,
            Version::Aborted => Some(("~a", 1)),
            Version::Deleted => Some(("~d", 2)),
            Version::Earlier => Some(("~e", 3)),
            Version::Running => Some(("~r", 4)),
        }
    }

    /// Whether a row with no field to change still has this version, as the row itself.
    fn of_unchanged(self) -> bool {
        matches!(self, Version::Row | Version::Aborted | Version::Deleted)
    }
}

/// One relation's rows, as the session's schema holds them.
struct Relation {
    name: String,
    elem: Ty,
    from: (RelId, Vec<Value>),
}

struct Sets<'a> {
    to: &'a Schema,
    from: &'a Schema,
    rels: Vec<Relation>,
}

impl<'a> Sets<'a> {
    /// Returns the rows of `rows`, a reference database over `from`, by the relation of `to` of
    /// the same module and name.
    fn of(to: &'a Schema, from: &'a Schema, rows: &adtb_reference::Database) -> Sets<'a> {
        let mut out: BTreeMap<RelId, Relation> = BTreeMap::new();
        for ((r, _), vs) in rows {
            let target = relation_of(to, from, *r);
            let rel = out.entry(target).or_insert_with(|| Relation {
                name: qualified(from, *r),
                elem: to.relation(target).elem.clone(),
                from: (*r, Vec::new()),
            });
            rel.from.1.extend(vs.iter().cloned());
        }
        Sets {
            to,
            from,
            rels: out.into_values().collect(),
        }
    }

    /// Returns `version` of every row of `rel` that has one, as the session's schema holds it.
    fn values(&self, rel: &Relation, version: Version) -> BTreeSet<Val> {
        let (r, rows) = &rel.from;
        rows.iter()
            .filter_map(|v| match version.change() {
                None => Some(v.clone()),
                Some(change) => match changed(self.from, *r, v, change) {
                    Some(c) => Some(c),
                    None => version.of_unchanged().then(|| v.clone()),
                },
            })
            .map(|v| to_val(self.to, &rel.elem, &v))
            .collect()
    }

    /// Returns the statement deleting every row of every relation.
    fn deletes(&self) -> String {
        let writes: Vec<String> = self
            .rels
            .iter()
            .map(|r| format!("delete x in {} where true", r.name))
            .collect();
        tuple(&writes)
    }

    /// Run in `tx` the statement inserting `version` of every row, replacing every row first
    /// when `write` is a replace.
    fn run<S: Session>(&self, s: &S, tx: &mut S::Txn, write: Write, version: Version) {
        let mut writes = Vec::new();
        let mut params = Vec::new();
        let names: Vec<String> = (0..self.rels.len()).map(|i| format!("rows{i}")).collect();
        for (r, name) in self.rels.iter().zip(&names) {
            if let Write::Replace = write {
                writes.push(format!("delete x in {} where true", r.name));
            }
            writes.push(format!("insert {} from ({name})", r.name));
            params.push((
                name.as_str(),
                Ty::Set(Box::new(r.elem.clone())),
                Val::set(self.values(r, version)),
            ));
        }
        if !writes.is_empty() {
            s.run(tx, &tuple(&writes), &params);
        }
    }
}

/// Returns `rows`, a reference database over `from`, with each relation's rows under the
/// relation of `to` of the same module and name.
pub fn rows_of(
    to: &Schema,
    from: &Schema,
    rows: &adtb_reference::Database,
) -> adtb_reference::Database {
    rows.iter()
        .map(|((r, c), vs)| ((relation_of(to, from, *r), *c), vs.clone()))
        .collect()
}

fn qualified(schema: &Schema, rel: RelId) -> String {
    let def = schema.relation(rel);
    format!("{}::{}", schema.module(def.module).name, def.name)
}

/// Returns the relation of `to` of the module and name of relation `rel` of `from`.
pub fn relation_of(to: &Schema, from: &Schema, rel: RelId) -> RelId {
    let def = from.relation(rel);
    to.module_named(&from.module(def.module).name)
        .and_then(|m| match to.module(m).values.get(&def.name) {
            Some(&ValueDef::Relation(t)) => Some(t),
            _ => None,
        })
        .unwrap_or_else(|| panic!("the database has no relation {}", qualified(from, rel)))
}

/// Set in `to` the stage of each constructor of a module that is not the engine's that `from`
/// stages, by the names of its relation or type and constructor.
fn staged(to: &mut Schema, from: &Schema) {
    for (&(r, c), &stage) in &from.stages {
        if !from.module(from.relation(r).module).engine {
            to.stages.insert((relation_of(to, from, r), c), stage);
        }
    }
    for (&(t, c), &stage) in &from.value_stages {
        let def = from.ty(t);
        let module = &from.module(def.module);
        if !module.engine {
            let there = to
                .module_named(&module.name)
                .and_then(|m| to.module(m).types.get(&def.name).copied())
                .unwrap_or_else(|| {
                    panic!("the database has no type {}::{}", module.name, def.name)
                });
            to.value_stages.insert((there, c), stage);
        }
    }
}

/// Returns the one statement of `writes`.
fn tuple(writes: &[String]) -> String {
    match writes {
        [one] => one.clone(),
        _ => format!("({})", writes.join(",\n")),
    }
}

/// Returns `row` of relation `rel` with `suffix` appended to each field outside the key that
/// is text and `micros` added to each that is an instant, or `None` when it has no such field.
fn changed(
    schema: &Schema,
    rel: RelId,
    row: &Value,
    (suffix, micros): (&str, i64),
) -> Option<Value> {
    let Value::Ctor(c, fs) = row else {
        panic!("a row is a constructor value")
    };
    let key = &schema.relation(rel).key;
    let mut any = false;
    let fs = fs
        .iter()
        .map(|(n, v)| {
            let w = match v {
                _ if key.contains(n) => v.clone(),
                Value::Text(t) => Value::Text(format!("{t}{suffix}")),
                Value::Instant(i) => Value::Instant(i + micros),
                other => other.clone(),
            };
            any |= &w != v;
            (n.clone(), w)
        })
        .collect();
    any.then_some(Value::Ctor(*c, fs))
}

struct Api<'a, R: Runtime>(&'a Database<R>);

impl<'a, R: Runtime> Session for Api<'a, R> {
    type Txn = AdHocTransaction<'a, R>;

    fn schema(&self) -> &Schema {
        self.0.schema()
    }

    fn begin(&self) -> AdHocTransaction<'a, R> {
        self.0
            .begin(&TxnOptions::default())
            .expect("a transaction begins")
    }

    fn hold(&self) -> AdHocTransaction<'a, R> {
        self.0
            .begin(&TxnOptions {
                read_only: true,
                ..TxnOptions::default()
            })
            .expect("a reader begins")
    }

    fn run(&self, tx: &mut AdHocTransaction<'a, R>, text: &str, params: &[(&str, Ty, Val)]) {
        tx.query_text(text, params, &TxnCallOptions::default())
            .unwrap_or_else(|e| panic!("`{text}` loads: {e:?}"));
    }

    fn commit(&self, tx: AdHocTransaction<'a, R>) {
        tx.commit(None).expect("a load commits");
    }

    fn rollback(&self, tx: AdHocTransaction<'a, R>) {
        tx.rollback().expect("a load rolls back");
    }
}

/// A write path over the engine's catalog of a file of the simulator, outside a database.
pub struct WritePath<R: Runtime> {
    db: write::Database<R>,
    schema: Arc<Schema>,
    program: Arc<Program>,
}

impl<R: Runtime> std::fmt::Debug for WritePath<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WritePath").finish_non_exhaustive()
    }
}

/// The contents of a simulator's files, by name.
pub type Files = BTreeMap<String, Vec<u8>>;

/// Which files a cache entry holds: the public database's or the write path's.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Made {
    Database,
    WritePath,
}

type Cache = BTreeMap<(Made, String, adtb_reference::Database), Files>;

thread_local! {
    static MADE: RefCell<Cache> = const { RefCell::new(BTreeMap::new()) };
}

/// Returns the files `make` leaves in a simulator of seed 0, made once per thread for each
/// `made`, declarations and rows.
fn made_once(
    made: Made,
    decls: &str,
    rows: &adtb_reference::Database,
    make: impl FnOnce(&SimRuntime),
) -> Files {
    let key = (made, decls.to_owned(), rows.clone());
    if let Some(files) = MADE.with_borrow(|m| m.get(&key).cloned()) {
        return files;
    }
    let mut sim = Sim::new(0, Config::default());
    let outcome = sim.run(|rt| make(&rt));
    assert_eq!(
        Outcome::Finished,
        outcome,
        "the files of `{decls}` are made"
    );
    let files = sim.files();
    MADE.with_borrow_mut(|m| m.insert(key, files.clone()));
    files
}

/// Returns the files of a simulator after [`open_with`] created the database of `decls` with
/// `options`, [`committed_noise`] loaded `rows`, rows of a reference database over `schema`,
/// and the database closed. Made once per thread for each `decls` and `rows`.
///
/// Preconditions:
/// - `options` start no background vacuum, and are the same at each call with `decls` and `rows`.
pub fn files(
    decls: &str,
    schema: &Schema,
    rows: &adtb_reference::Database,
    options: &OpenOptions,
) -> Files {
    made_once(Made::Database, decls, rows, |rt| {
        let db = open_with(rt, decls, options);
        committed_noise(&db, schema, rows);
        db.close().expect("the database closes");
    })
}

/// Create the file of [`harness::options`] with the engine's catalog, apply the migration
/// adding module `app` with the declarations of `f`, load its rows as [`committed_noise`] does,
/// and close the file.
fn create_loaded<R: Runtime>(rt: &R, f: &Fixture) {
    let decls = f.decls().expect("the fixture has declarations");
    let pager = Pager::create(rt.clone(), &harness::options(256)).expect("the file is created");
    let (mut catalog, txns) = Catalog::create(rt.clone(), &pager).expect("the catalog");
    catalog
        .migrate(&pager, &txns, &migration(decls))
        .unwrap_or_else(|e| panic!("the declarations migrate: {e:?}"));
    catalog.counters(&pager).expect("the page of counters");
    pager.close();
    let path = WritePath::start(rt, None, None).expect("the migrated file opens");
    load(&path, &f.schema, &f.db, Running::None);
    let pager = path.db.pager().clone();
    drop(path);
    pager.close();
}

/// Returns the files of a simulator after [`WritePath::of`] made its file for `f`. Made once per
/// thread for each declarations and rows.
pub fn write_files(f: &Fixture) -> Files {
    let decls = f.decls().expect("the fixture has declarations");
    made_once(Made::WritePath, decls, &f.db, |rt| create_loaded(rt, f))
}

/// Returns a simulator of `seed` and `config` holding the files [`write_files`] returns for `f`.
pub fn sim(seed: u64, config: Config, f: &Fixture) -> Sim {
    Sim::with_files(seed, config, write_files(f))
}

impl<R: Runtime> WritePath<R> {
    /// Create the file of [`harness::options`] with the engine's catalog, apply the migration
    /// adding module `app` with the declarations of `f`, and load its rows as
    /// [`committed_noise`] does. Returns the write path over the file, each constructor of `app`
    /// at the stage `f` gives it, its queries run on `pool` when it is given.
    pub fn of(rt: &R, f: &Fixture, pool: Option<Arc<Scheduler<R>>>) -> WritePath<R> {
        create_loaded(rt, f);
        WritePath::start(rt, Some(f), pool).expect("the file opens")
    }

    /// Returns the write path [`WritePath::of`] returns, over the file [`write_files`] made.
    ///
    /// Preconditions:
    /// - the runtime's files are those `write_files` returns for `f`.
    pub fn loaded(rt: &R, f: &Fixture, pool: Option<Arc<Scheduler<R>>>) -> WritePath<R> {
        WritePath::start(rt, Some(f), pool).expect("the loaded file opens")
    }

    /// Open the file [`WritePath::of`] made after a crash, recovering it. Returns `None` when
    /// the pager cannot open the file.
    pub fn reopen(rt: &R, pool: Option<Arc<Scheduler<R>>>) -> Option<WritePath<R>> {
        WritePath::start(rt, None, pool)
    }

    fn start(
        rt: &R,
        staged_by: Option<&Fixture>,
        pool: Option<Arc<Scheduler<R>>>,
    ) -> Option<WritePath<R>> {
        let (pager, _) = Pager::open(rt.clone(), &harness::options(256)).ok()?;
        let (catalog, txns) = Catalog::open(rt.clone(), &pager).expect("the catalog opens");
        let mut schema = catalog.schema().clone();
        if let Some(f) = staged_by {
            staged(&mut schema, &f.schema);
        }
        let program = catalog.program().clone();
        let normal = norm::program(&schema, &program).expect("the schema normalises");
        let (heaps, indexes) = catalog.bindings();
        let schema = Arc::new(schema);
        let db = write::Database::new(
            rt.clone(),
            Setup {
                schema: schema.clone(),
                normal: Arc::new(normal),
                catalog: Arc::new(catalog.plan()),
                pager,
                txns: Arc::new(txns),
                bindings: Bindings { heaps, indexes },
                budget: Budget::new(1 << 30),
                grant: 64 << 20,
                pool,
                options: exec::Options::default(),
                limits: Default::default(),
                sequences: catalog.sequences().expect("the page of counters"),
            },
        )
        .expect("a write path over the catalog");
        Some(WritePath {
            db,
            schema,
            program,
        })
    }

    pub fn db(&self) -> &write::Database<R> {
        &self.db
    }

    pub fn schema(&self) -> &Arc<Schema> {
        &self.schema
    }

    /// Returns the write path's database, ending the write path.
    pub fn into_db(self) -> write::Database<R> {
        self.db
    }

    pub fn program(&self) -> &Arc<Program> {
        &self.program
    }

    /// Returns the type `text` names in the write path's schema.
    pub fn ty(&self, text: &str) -> Ty {
        let parsed = parse_type(text, Options::default())
            .ok()
            .unwrap_or_else(|_| panic!("`{text}` is not a type"));
        Checker::new(&self.schema, Visible::all(&self.schema), Context::Adhoc)
            .resolve_type(&parsed)
            .unwrap_or_else(|d| panic!("`{text}`: {}", d.message))
    }

    /// Returns ad hoc `text` over `params`, each given as name and the text of its type, as the
    /// write path runs it.
    pub fn prepare(&self, text: &str, params: &[(&str, &str)]) -> Prepared {
        let typed: Vec<(&str, Ty)> = params.iter().map(|(n, t)| (*n, self.ty(t))).collect();
        self.prepare_typed(text, &typed)
    }

    fn prepare_typed(&self, text: &str, params: &[(&str, Ty)]) -> Prepared {
        let items = parse_adhoc(text, Options::default())
            .ok()
            .unwrap_or_else(|ds| panic!("`{text}` does not parse: {ds:?}"));
        let tys = params
            .iter()
            .map(|(n, t)| ((*n).to_owned(), t.clone()))
            .collect();
        let adhoc = check_adhoc(&self.schema, &self.program, &items, &tys)
            .unwrap_or_else(|ds| panic!("`{text}` does not check: {ds:?}"));
        let normal = norm::adhoc(&self.schema, &self.program, &adhoc)
            .unwrap_or_else(|e| panic!("`{text}` has no normal form: {e:?}"));
        let [item] = normal.items.as_slice() else {
            panic!("`{text}` holds one statement")
        };
        let params = params
            .iter()
            .map(|(n, t)| (normal.params[*n], t.clone()))
            .collect();
        (normal.terms, *item, params)
    }
}

/// A statement ready for a write path: its terms, its item, and the variable and type of each
/// parameter in the order they were given.
pub type Prepared = (Terms, Tm, Vec<(Var, Ty)>);

impl<R: Runtime> Session for WritePath<R> {
    type Txn = Txn;

    fn schema(&self) -> &Schema {
        &self.schema
    }

    fn begin(&self) -> Txn {
        self.db.begin().expect("a transaction begins")
    }

    fn hold(&self) -> Txn {
        self.db.begin().expect("a transaction begins")
    }

    fn run(&self, tx: &mut Txn, text: &str, params: &[(&str, Ty, Val)]) {
        let types: Vec<(&str, Ty)> = params.iter().map(|(n, t, _)| (*n, t.clone())).collect();
        let (terms, item, typed) = self.prepare_typed(text, &types);
        let values: Vec<_> = typed
            .iter()
            .zip(params)
            .map(|((var, _), (_, _, v))| (*var, v.clone()))
            .collect();
        self.db
            .statement(tx, &terms, item, &values, 0)
            .unwrap_or_else(|e| panic!("`{text}` loads: {e:?}"));
    }

    fn commit(&self, tx: Txn) {
        self.db.commit(tx).expect("a load commits");
    }

    fn rollback(&self, tx: Txn) {
        self.db.abort(tx).expect("a load rolls back");
    }
}
