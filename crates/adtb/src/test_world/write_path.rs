//! A write path over the engine's catalog of a file of the simulator, outside a database.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Arc;

use adtb_harness::load::{self, Files, Made, Running, Session, convert};
use adtb_lang::check::{Checker, Context, Program, check_adhoc};
use adtb_lang::kernel::{Terms, Tm, Var};
use adtb_lang::norm;
use adtb_lang::parse::{Options, parse_adhoc, parse_type};
use adtb_lang::resolve::Visible;
use adtb_lang::schema::Schema;
use adtb_lang::types::Ty;
use adtb_reference::Value;
use adtb_sim::{Config, Sim};
use adtb_testkit::corpus::migration;
use adtb_testkit::fixture::Fixture;

use crate::catalog::Catalog;
use crate::exec::{self, Budget, Scheduler};
use crate::runtime::Runtime;
use crate::storage::Pager;
use crate::write::{self, Bindings, Setup, Txn};

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

/// Create the file of [`super::options`] with the engine's catalog, apply the migration adding
/// module `app` with the declarations of `f`, load its rows as
/// [`super::load::committed_noise`] does, and close the file.
fn create_loaded<R: Runtime>(rt: &R, f: &Fixture) {
    let decls = f.decls().expect("the fixture has declarations");
    let pager = Pager::create(rt.clone(), &super::options(256)).expect("the file is created");
    let (mut catalog, txns) = Catalog::create(rt.clone(), &pager).expect("the catalog");
    catalog
        .migrate(&pager, &txns, &migration(decls))
        .unwrap_or_else(|e| panic!("the declarations migrate: {e:?}"));
    catalog.counters(&pager).expect("the page of counters");
    pager.close();
    let path = WritePath::start(rt, None, None).expect("the migrated file opens");
    load::load(&path, &f.schema, &f.db, Running::None);
    let pager = path.db.pager().clone();
    drop(path);
    pager.close();
}

thread_local! {
    static MADE: Made = const { RefCell::new(BTreeMap::new()) };
}

/// Returns the files of a simulator after [`WritePath::of`] made its file for `f`. Made once per
/// thread for each declarations and rows.
pub fn write_files(f: &Fixture) -> Files {
    let decls = f.decls().expect("the fixture has declarations");
    load::made_once(&MADE, decls, &f.db, |rt| create_loaded(rt, f))
}

/// Returns a simulator of `seed` and `config` holding the files [`write_files`] returns for `f`.
pub fn sim(seed: u64, config: Config, f: &Fixture) -> Sim {
    Sim::with_files(seed, config, write_files(f))
}

impl<R: Runtime> WritePath<R> {
    /// Create the file of [`super::options`] with the engine's catalog, apply the migration
    /// adding module `app` with the declarations of `f`, and load its rows as
    /// [`super::load::committed_noise`] does. Returns the write path over the file, each
    /// constructor of `app` at the stage `f` gives it, its queries run on `pool` when it is given.
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
        let (pager, _) = Pager::open(rt.clone(), &super::options(256)).ok()?;
        let (catalog, txns) = Catalog::open(rt.clone(), &pager).expect("the catalog opens");
        let mut schema = catalog.schema().clone();
        if let Some(f) = staged_by {
            load::staged(&mut schema, &f.schema);
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
                structures: catalog.structures(),
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
        self.db.begin_read(false).expect("a reader begins")
    }

    fn run(&self, tx: &mut Txn, text: &str, params: &[(&str, Ty, Value)]) {
        let types: Vec<(&str, Ty)> = params.iter().map(|(n, t, _)| (*n, t.clone())).collect();
        let (terms, item, typed) = self.prepare_typed(text, &types);
        let values: Vec<_> = typed
            .iter()
            .zip(params)
            .map(|((var, t), (_, _, v))| (*var, convert(&self.schema, t, v)))
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
