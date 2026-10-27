mod error;
#[cfg(target_os = "linux")]
mod file;
mod text;
mod txn;

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use adtb_lang::check::Program;
use adtb_lang::evolve::Logged;
use adtb_lang::norm;
use adtb_lang::parse::{Options as ParseOptions, parse_migration};
use adtb_lang::schema::Schema;
use adtb_lang::types::Ty;

use crate::catalog::{self, Catalog};
use crate::exec::{self, Budget, Val};
use crate::mvcc::Limits;
use crate::runtime::Runtime;
use crate::storage::{self, LATCHES_PER_THREAD, Pager};
use crate::write::{self, Bindings, Bounds, Setup, Txn};

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
    /// The rounds a fixpoint may take before its statement is rejected for its limit.
    pub fix_rounds: Option<usize>,
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

/// The id of a transaction: a random positive 63-bit integer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TxnId(pub i64);

/// The durability a commit acknowledgement promises.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Durability {
    #[default]
    Local,
    Quorum,
}

/// The options of a call outside an interactive transaction.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CallOptions {
    pub deadline: Option<SystemTime>,
    pub limit: Option<Duration>,
    pub priority: Option<u8>,
    pub durability: Durability,
    /// Read no snapshot older than this commit position.
    pub after: Option<Position>,
    /// The most reruns of text that writes on a serialization failure, in place of its
    /// `retry n` or the retry limit.
    pub retry: Option<u32>,
    /// The rounds a fixpoint of the call may take, else the limit the database was opened with.
    pub fix_rounds: Option<usize>,
}

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
    rt: R,
    schema: Arc<Schema>,
    program: Arc<Program>,
    pager: Pager<R>,
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
            self.pager.close();
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
    /// Text that only reads runs as a read-only transaction, waiting for a safe snapshot when a
    /// block of it is `read_only deferrable`. Text that writes is rerun on a serialization
    /// failure up to `opts.retry`, else its `retry n`, else the retry limit.
    ///
    /// A database without replicas commits at `local` whatever level the options name.
    pub fn query_text(
        &self,
        text: &str,
        params: &[(&str, Ty, Val)],
        opts: &CallOptions,
    ) -> Result<Vec<DynRows>, Error> {
        if opts.deadline.is_some() || opts.limit.is_some() {
            return Err(unsupported("a deadline or statement limit on a call"));
        }
        let e = &*self.engine;
        e.reads_after(opts.after)?;
        let p = Prepared::new(&e.schema, &e.program, text, params)?;
        let bounds = Bounds {
            fix_rounds: opts.fix_rounds,
        };
        if !p.writes {
            let id = e.txn_id();
            let tx = e.begin_read(id, p.deferrable)?;
            return e.run(id, tx, &p, bounds);
        }
        let reruns = opts.retry.map_or(p.retry.unwrap_or(RETRY_LIMIT), u64::from);
        let mut ran = 0;
        loop {
            let id = e.txn_id();
            let tx = e.begin(id)?;
            match e.run(id, tx, &p, bounds) {
                Err(err) if err.is_serialization() && ran < reruns => ran += 1,
                done => return done,
            }
        }
    }

    /// Close the handle. The engine closes, writing every changed page, taking a checkpoint,
    /// closing its files and releasing the lock on the database file, once no other handle is
    /// open.
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

fn unsupported(what: &str) -> Error {
    Error::invalid(format!("{what} does not run in this engine yet"))
}

fn corrupt(detail: impl std::fmt::Debug) -> OpenError {
    OpenError::Corrupt {
        detail: format!("{detail:?}"),
    }
}

fn refused(version: u64, reason: RejectReason) -> OpenError {
    OpenError::MigrationRefused {
        version,
        reason: Error::rejected(reason),
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
        Engine::on(rt, pager.clone(), log, options).inspect_err(|_| pager.close())
    }

    /// Open the catalog of the recovered `pager`, apply the migrations of `log` beyond the
    /// file's own, and bind the write path to it.
    fn on(
        rt: R,
        pager: Pager<R>,
        log: &Log,
        options: &OpenOptions,
    ) -> Result<Engine<R>, OpenError> {
        let query_memory = options
            .query_memory_bytes
            .map_or(DEFAULT_QUERY_MEMORY, |b| {
                usize::try_from(b).unwrap_or(usize::MAX)
            });
        let (mut catalog, txns) = Catalog::open(rt.clone(), &pager).map_err(corrupt)?;
        catalog.set_query_memory(query_memory);
        migrate(&mut catalog, &pager, &txns, log)?;
        let schema = catalog.schema().clone();
        let program = catalog.program().clone();
        let normal = norm::program(&schema, &program).map_err(corrupt)?;
        let plan = catalog.plan();
        let root = catalog.root();
        let sequences = catalog.sequences();
        let (heaps, indexes) = catalog.bindings();
        let schema = Arc::new(schema);
        let defaults = exec::Options::default();
        if sequences.is_none() {
            Catalog::naming_sequences(&pager, root).map_err(corrupt)?;
        }
        let db = write::Database::new(
            rt.clone(),
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
                options: exec::Options {
                    fix_rounds: options.fix_rounds.unwrap_or(defaults.fix_rounds),
                    ..defaults
                },
                limits: Limits::default(),
                sequences,
            },
        )
        .map_err(corrupt)?;
        if sequences.is_none() {
            Catalog::name_sequences(&pager, root, db.sequences_root()).map_err(corrupt)?;
        }
        Ok(Engine {
            rt,
            schema,
            program,
            pager,
            db,
            #[cfg(target_os = "linux")]
            held: None,
        })
    }

    /// Returns a new transaction id.
    fn txn_id(&self) -> TxnId {
        TxnId(i64::try_from((self.rt.next_u64() >> 1).max(1)).expect("63 bits fit an i64"))
    }

    fn write_error(&self, txn: TxnId, e: write::Error) -> Error {
        Error::of_write(&self.schema, txn, e)
    }

    fn begin(&self, id: TxnId) -> Result<Txn, Error> {
        self.db.begin().map_err(|e| self.write_error(id, e))
    }

    fn begin_read(&self, id: TxnId, deferrable: bool) -> Result<Txn, Error> {
        self.db
            .begin_read(deferrable)
            .map_err(|e| self.write_error(id, e))
    }

    fn abort(&self, id: TxnId, tx: Txn) -> Result<(), Error> {
        self.db.abort(tx).map_err(|e| self.write_error(id, e))
    }

    /// Refuses a commit position past every commit of the database's history.
    fn reads_after(&self, after: Option<Position>) -> Result<(), Error> {
        let end = Position::from(self.pager.position(self.pager.durable()));
        match after {
            Some(position) if position > end => {
                Err(Error::rejected(RejectReason::PositionNotOnHistory {
                    position,
                }))
            }
            _ => Ok(()),
        }
    }

    /// Returns the instant every `generate clock` of a statement starting now takes.
    fn clock(&self) -> i64 {
        let micros = |d: Duration| i64::try_from(d.as_micros()).unwrap_or(i64::MAX);
        match self.pager.wall().duration_since(UNIX_EPOCH) {
            Ok(d) => micros(d),
            Err(e) => -micros(e.duration()),
        }
    }

    /// Run `p` in `tx`, the transaction `id`, bounded by `bounds`, and commit it, or abort it on
    /// any failure.
    fn run(&self, id: TxnId, tx: Txn, p: &Prepared, bounds: Bounds) -> Result<Vec<DynRows>, Error> {
        let ran = self
            .db
            .statements(&tx, p.terms(), p.items(), p.params(), self.clock(), bounds);
        let rows = match ran {
            Ok(outcomes) => p.results(&self.schema, outcomes),
            Err(f) => Err(self.write_error(id, f.error)),
        };
        match rows {
            Ok(rows) => {
                self.db.commit(tx).map_err(|e| self.write_error(id, e))?;
                Ok(rows)
            }
            Err(e) => {
                self.abort(id, tx)?;
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
            catalog::Error::Refused(why) => refused(
                version,
                RejectReason::SchemaRefused {
                    operator: String::new(),
                    detail: why.join("\n"),
                },
            ),
            catalog::Error::Internal(what) => refused(version, RejectReason::Internal { what }),
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
