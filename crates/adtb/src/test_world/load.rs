//! Open a database of the declarations of module `app` on the simulator and load rows into it
//! with statements, each row under versions no snapshot taken afterwards sees.

use std::cell::RefCell;
use std::collections::BTreeMap;

use adtb_harness::load::{self, Files, Made, Running, Session};
use adtb_lang::schema::Schema;
use adtb_lang::types::Ty;
use adtb_reference::Value;
use adtb_sim::SimRuntime;
use adtb_testkit::corpus::migration;

use crate::runtime::Runtime;
use crate::{
    AdHocTransaction, CreateOptions, Database, Log, OpenOptions, TxnCallOptions, TxnOptions,
};

use super::support::to_val;

/// The data file a test's database lives in.
pub const FILE: &str = "app.adt";

/// Returns the options that open the database with `workers` workers, creating it if it does not
/// exist.
pub fn options(workers: usize) -> OpenOptions {
    OpenOptions {
        create: Some(CreateOptions::default()),
        workers: Some(workers),
        ..OpenOptions::default()
    }
}

/// Open the database in [`FILE`] whose first migration adds module `app` with `decls`, creating
/// it if it does not exist.
pub fn open(rt: &SimRuntime, decls: &str) -> Database<SimRuntime> {
    open_with(rt, decls, &options(1))
}

/// Open the database as [`open`] does, with `options`.
pub fn open_with(rt: &SimRuntime, decls: &str, options: &OpenOptions) -> Database<SimRuntime> {
    Database::open_in(rt.clone(), FILE, &Log::new([migration(decls)]), options)
        .unwrap_or_else(|e| panic!("{FILE} opens: {e:?}"))
}

/// Load `rows`, the rows of a reference database over `schema`, into `db` by relation name, as
/// [`adtb_harness::load::load`] with [`Running::Left`] does. Returns the transaction left running.
pub fn rows_with_noise<'a, R: Runtime>(
    db: &'a Database<R>,
    schema: &Schema,
    rows: &adtb_reference::Database,
) -> AdHocTransaction<'a, R> {
    load::load(&Api(db), schema, rows, Running::Left).expect("the running transaction is returned")
}

/// Load `rows`, the rows of a reference database over `schema`, into `db` by relation name, as
/// [`adtb_harness::load::load`] does.
pub fn committed_noise<R: Runtime>(
    db: &Database<R>,
    schema: &Schema,
    rows: &adtb_reference::Database,
) {
    load::load(&Api(db), schema, rows, Running::None);
}

/// Begin a transaction over `db`, loaded with `rows` as [`committed_noise`] loads them, as
/// [`adtb_harness::load::running`] does. It rolls back when dropped.
pub fn running_noise<'a, R: Runtime>(
    db: &'a Database<R>,
    schema: &Schema,
    rows: &adtb_reference::Database,
) -> AdHocTransaction<'a, R> {
    load::running(&Api(db), schema, rows)
}

thread_local! {
    static MADE: Made = const { RefCell::new(BTreeMap::new()) };
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
    load::made_once(&MADE, decls, rows, |rt| {
        let db = open_with(rt, decls, options);
        committed_noise(&db, schema, rows);
        db.close().expect("the database closes");
    })
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

    fn run(&self, tx: &mut AdHocTransaction<'a, R>, text: &str, params: &[(&str, Ty, Value)]) {
        let schema = self.0.schema();
        let params: Vec<_> = params
            .iter()
            .map(|(n, t, v)| (*n, t.clone(), to_val(schema, t, v)))
            .collect();
        tx.query_text(text, &params, &TxnCallOptions::default())
            .unwrap_or_else(|e| panic!("`{text}` loads: {e:?}"));
    }

    fn commit(&self, tx: AdHocTransaction<'a, R>) {
        tx.commit(None).expect("a load commits");
    }

    fn rollback(&self, tx: AdHocTransaction<'a, R>) {
        tx.rollback().expect("a load rolls back");
    }
}
