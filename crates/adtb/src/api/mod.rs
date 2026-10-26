mod error;
#[cfg(target_os = "linux")]
mod file;
mod text;
mod txn;

use std::sync::Arc;
use std::time::UNIX_EPOCH;

use adtb_lang::check::Program;
use adtb_lang::evolve::Logged;
use adtb_lang::norm;
use adtb_lang::parse::{Options as ParseOptions, parse_migration};
use adtb_lang::schema::Schema;
use adtb_lang::types::Ty;

use crate::catalog::{self, Catalog};
use crate::exec::{self, Budget, Val};
use crate::mvcc::{Limits, Xid};
use crate::runtime::Runtime;
use crate::storage::{self, LATCHES_PER_THREAD, Pager};
use crate::write::{self, Bindings, Setup, Txn};

pub use error::{Error, FatalReason, Fault, Limit, OpenError, RejectReason, RetryReason};
pub use text::DynRows;
pub use txn::{AdHocTransaction, Committed, TxnCallOptions, TxnOptions};

use text::Prepared;

/// The reruns of ad hoc text that writes, and of a `transact` block run in process, after a
/// serialization failure when the text states no `retry n`.
const RETRY_LIMIT: u64 = 5;

const PAGE_SIZE: u32 = 8192;
const DEFAULT_POOL_FRAMES: usize = 1024;
const DEFAULT_WORKERS: usize = 8;
const DEFAULT_QUERY_MEMORY: usize = 256 << 20;
const STATEMENT_GRANT: usize = 64 << 20;

/// The program's operator log: the text of each migration, in order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Log {
    migrations: Vec<String>,
}

impl Log {
    pub fn new<S: Into<String>>(migrations: impl IntoIterator<Item = S>) -> Log {
        Log {
            migrations: migrations.into_iter().map(Into::into).collect(),
        }
    }
}

/// What open creates a file with when none exists.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct CreateOptions {}

/// The settings fixed while a handle is open.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OpenOptions {
    /// Create the file when it does not exist; `None` fails open with `NotFound` instead.
    pub create: Option<CreateOptions>,
    pub pool_bytes: Option<u64>,
    pub query_memory_bytes: Option<u64>,
    pub workers: Option<usize>,
}

impl OpenOptions {
    fn workers(&self) -> usize {
        self.workers.unwrap_or(DEFAULT_WORKERS).max(1)
    }

    fn storage(&self, name: &str) -> storage::Options {
        let threads = self.workers();
        let frames = self.pool_bytes.map_or(DEFAULT_POOL_FRAMES, |b| {
            usize::try_from(b / u64::from(PAGE_SIZE)).unwrap_or(usize::MAX)
        });
        storage::Options {
            page_size: PAGE_SIZE,
            pool_frames: frames.max(threads * LATCHES_PER_THREAD),
            threads,
            ..storage::Options::new(name)
        }
    }
}

/// The place of a commit in the database's log.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Position {
    pub epoch: u32,
    pub lsn: u64,
}

impl From<storage::Position> for Position {
    fn from(p: storage::Position) -> Position {
        Position {
            epoch: p.epoch,
            lsn: p.lsn.0,
        }
    }
}

/// The options of a call outside an interactive transaction.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct CallOptions {}

/// A handle on an open database file. Clones share one engine, which closes when the last
/// handle is dropped.
#[derive(Debug)]
pub struct Database<R: Runtime = adtb_sys::os::OsRuntime> {
    engine: Arc<Engine<R>>,
}

impl<R: Runtime> Clone for Database<R> {
    fn clone(&self) -> Self {
        Database {
            engine: self.engine.clone(),
        }
    }
}

struct Engine<R: Runtime> {
    schema: Arc<Schema>,
    program: Program,
    pager: Arc<Pager<R>>,
    db: write::Database<R>,
    #[cfg(target_os = "linux")]
    held: Option<file::Held>,
}

impl<R: Runtime> std::fmt::Debug for Engine<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("db", &self.db)
            .finish_non_exhaustive()
    }
}

impl<R: Runtime> Drop for Engine<R> {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            self.pager.checkpoint();
        }
    }
}

impl<R: Runtime> Database<R> {
    /// Open the database file `name` of runtime `rt`, recover it, and apply the migrations of
    /// `log` beyond the file's own.
    pub fn open_in(
        rt: R,
        name: &str,
        log: &Log,
        options: &OpenOptions,
    ) -> Result<Database<R>, OpenError> {
        Ok(Database {
            engine: Arc::new(Engine::open(rt, name, log, options)?),
        })
    }

    /// Returns the schema ad hoc text is checked against.
    pub fn schema(&self) -> &Schema {
        &self.engine.schema
    }

    /// Run ad hoc text as one transaction, each free name of it bound by `params` to a value of
    /// the given type. Returns the rows of each item that yields a result, in item order.
    ///
    /// Text that writes is rerun on a serialization failure up to its `retry n`, else up to the
    /// retry limit.
    pub fn query_text(
        &self,
        text: &str,
        params: &[(&str, Ty, Val)],
        _opts: &CallOptions,
    ) -> Result<Vec<DynRows>, Error> {
        let e = &*self.engine;
        let p = Prepared::new(&e.schema, &e.program, text, params)?;
        if p.deferrable {
            return Err(Error::invalid(
                "a read_only deferrable block does not run in this engine yet",
            ));
        }
        let reruns = p.retry.unwrap_or(RETRY_LIMIT);
        let mut ran = 0;
        loop {
            let tx = e.begin()?;
            match e.run(tx, &p) {
                Err(err) if err.is_serialization() && p.writes && ran < reruns => ran += 1,
                done => return done,
            }
        }
    }

    /// Close the handle. The engine closes, taking a checkpoint and releasing the file, once
    /// no other handle is open.
    pub fn close(self) -> Result<(), Error> {
        drop(self);
        Ok(())
    }
}

#[cfg(target_os = "linux")]
impl Database<adtb_sys::os::OsRuntime> {
    /// Open or create the database file at `path` under the host's runtime, locking it against
    /// every other open, and apply the migrations of `log` beyond the file's own.
    pub fn open(
        path: impl AsRef<std::path::Path>,
        log: &Log,
        options: &OpenOptions,
    ) -> Result<Database, OpenError> {
        let (held, dir, name) = file::Held::take(path.as_ref(), options.create.is_some())?;
        let rt = adtb_sys::os::OsRuntime::new(
            dir,
            adtb_sys::os::Backend::Pool {
                threads: options.workers(),
            },
        );
        let mut engine = Engine::open(rt, &name, log, options)?;
        engine.held = Some(held);
        Ok(Database {
            engine: Arc::new(engine),
        })
    }
}

/// Open or create the database file at `path`, as [`Database::open`] does.
#[cfg(target_os = "linux")]
pub fn open(
    path: impl AsRef<std::path::Path>,
    log: &Log,
    options: &OpenOptions,
) -> Result<Database, OpenError> {
    Database::open(path, log, options)
}

fn corrupt(detail: impl std::fmt::Debug) -> OpenError {
    OpenError::Corrupt {
        detail: format!("{detail:?}"),
    }
}

fn refused(version: u64, detail: String) -> OpenError {
    OpenError::MigrationRefused {
        version,
        reason: Error::rejected(RejectReason::SchemaRefused {
            operator: String::new(),
            detail,
        }),
    }
}

impl<R: Runtime> Engine<R> {
    fn open(rt: R, name: &str, log: &Log, options: &OpenOptions) -> Result<Engine<R>, OpenError> {
        let storage = options.storage(name);
        let pager = match Pager::open(rt.clone(), &storage) {
            Ok((p, _)) => p,
            Err(storage::OpenError::NotFound | storage::OpenError::NoHeader)
                if options.create.is_some() =>
            {
                Pager::create(rt.clone(), &storage).map_err(|e| match e {
                    storage::OpenError::Exists => {
                        corrupt("neither copy of the file header verifies")
                    }
                    other => corrupt(other),
                })?
            }
            Err(storage::OpenError::NotFound) => return Err(OpenError::NotFound),
            Err(e) => return Err(corrupt(e)),
        };
        let (mut catalog, txns) = Catalog::open(rt.clone(), &pager).map_err(corrupt)?;
        migrate(&mut catalog, &pager, &txns, log)?;
        let (schema, program) = catalog
            .source()
            .check()
            .map_err(|ds| corrupt(ds.into_iter().map(|d| d.diag.message).collect::<Vec<_>>()))?;
        let normal = norm::program(&schema, &program).map_err(corrupt)?;
        let plan = catalog.plan();
        let root = catalog.root();
        let sequences = catalog.sequences();
        let (heaps, indexes) = catalog.into_bindings();
        let pager = Arc::new(pager);
        let schema = Arc::new(schema);
        let query_memory = options
            .query_memory_bytes
            .map_or(DEFAULT_QUERY_MEMORY, |b| {
                usize::try_from(b).unwrap_or(usize::MAX)
            });
        let db = write::Database::new(
            rt,
            Setup {
                schema: schema.clone(),
                normal: Arc::new(normal),
                catalog: Arc::new(plan),
                pager: pager.clone(),
                txns: Arc::new(txns),
                bindings: Bindings { heaps, indexes },
                budget: Budget::new(query_memory),
                grant: STATEMENT_GRANT.min(query_memory),
                pool: None,
                options: exec::Options::default(),
                limits: Limits::default(),
                sequences,
            },
        )
        .map_err(corrupt)?;
        if sequences.is_none() {
            Catalog::name_sequences(&pager, root, db.sequences_root()).map_err(corrupt)?;
        }
        Ok(Engine {
            schema,
            program,
            pager,
            db,
            #[cfg(target_os = "linux")]
            held: None,
        })
    }

    fn write_error(&self, txn: Xid, e: write::Error) -> Error {
        Error::of_write(&self.schema, txn, e)
    }

    fn begin(&self) -> Result<Txn, Error> {
        self.db.begin().map_err(|e| self.write_error(Xid(0), e))
    }

    fn abort(&self, tx: Txn) -> Result<(), Error> {
        let xid = tx.xid();
        self.db.abort(tx).map_err(|e| self.write_error(xid, e))
    }

    /// Returns the instant every `generate clock` of a statement starting now takes.
    fn clock(&self) -> i64 {
        let micros = |d: std::time::Duration| i64::try_from(d.as_micros()).unwrap_or(i64::MAX);
        match self.pager.wall().duration_since(UNIX_EPOCH) {
            Ok(d) => micros(d),
            Err(e) => -micros(e.duration()),
        }
    }

    /// Run `p` in `tx` and commit it, or abort it on any failure.
    fn run(&self, tx: Txn, p: &Prepared) -> Result<Vec<DynRows>, Error> {
        let xid = tx.xid();
        let ran = self
            .db
            .statements(&tx, p.terms(), p.items(), p.params(), self.clock());
        let rows = match ran {
            Ok(outcomes) => p.results(&self.schema, outcomes),
            Err(f) => Err(self.write_error(xid, f.error)),
        };
        match rows {
            Ok(rows) => {
                self.db.commit(tx).map_err(|e| self.write_error(xid, e))?;
                Ok(rows)
            }
            Err(e) => {
                self.abort(tx)?;
                Err(e)
            }
        }
    }
}

/// Apply the migrations of `log` beyond the file's own, each as its own schema change, stopping
/// at the first refused one. Refuses a log whose migrations at or below the file's version differ
/// from the file's by hash, naming the first that does.
fn migrate<R: Runtime>(
    catalog: &mut Catalog,
    pager: &Pager<R>,
    txns: &crate::mvcc::Transactions<R>,
    log: &Log,
) -> Result<(), OpenError> {
    let stored = &catalog.source().log;
    if let Some((m, _)) = stored
        .iter()
        .zip(&log.migrations)
        .find(|(m, shipped)| &m.text != *shipped && !same_hash(m, shipped))
    {
        return Err(OpenError::Diverged { version: m.number });
    }
    for text in log.migrations.iter().skip(stored.len()) {
        let version = catalog.source().version + 1;
        catalog.migrate(pager, txns, text).map_err(|e| match e {
            catalog::Error::Refused(why) => refused(version, why.join("\n")),
            other => corrupt(other),
        })?;
    }
    Ok(())
}

/// Returns whether `text`, applied where migration `m` of the log was, gives the version hash
/// `m` gives.
fn same_hash(m: &Logged, text: &str) -> bool {
    let hash = |text: &str| {
        let file = parse_migration(text, ParseOptions::default()).ok().ok()?;
        let source = m.before.migrate(&file, text).ok()?;
        let schema = source.schema().ok()?;
        Some(catalog::version_hash(&schema, &source))
    };
    hash(text).is_some_and(|h| Some(h) == hash(&m.text))
}
