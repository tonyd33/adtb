mod arrow;
mod bundle;
mod command;
mod error;
#[cfg(target_os = "linux")]
mod file;
mod host;
#[cfg(all(target_os = "linux", debug_assertions, not(test)))]
mod instances;
mod load;
mod maintenance;
#[cfg(test)]
mod open_tests;
mod pinned;
mod reads;
mod registered;
mod reindex;
mod server;
mod settings;
mod statements;
mod stop;
mod text;
mod txn;
pub mod typed;
mod value;
mod verify;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::task::Waker;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use adtb_file::Disk;
use adtb_lang::check::{Bundles, Program};
use adtb_lang::evolve::{Logged, Source};
use adtb_lang::norm;
use adtb_lang::parse::{Options as ParseOptions, parse_migration};
use adtb_lang::schema::Schema;
use adtb_lang::types::Ty;
use adtb_sys::lock::{Lock, RwLock};

use crate::catalog::{self, Catalog};
use crate::exec::{self, Budget, Scheduler, Val};
use crate::mvcc::Limits;
use crate::mvcc::Transactions;
use crate::runtime::{Runtime, ThreadId};
use crate::storage::{self, LATCHES_PER_THREAD, Pager};
use crate::write::{self, Bindings, Bounds, Local, Setup, Txn};

pub use arrow::{ArrowSchema, ArrowStream, Field, RecordBatch};
pub use error::{Error, FatalReason, Fault, Limit, OpenError, RejectReason, RetryReason};
pub use host::{Determinism, HostFns};
pub use load::{Appender, Load, LoadOptions, LoadReport};
pub use registered::{Called, InTxn, Registered, Registration};
pub use server::{
    Capability, Completion, Format, Layout, Observation, ResultBatch, RoleAuthority, RoleSession,
    Task, TaskOptions,
};
pub use stop::CancelToken;
pub use text::DynRows;
pub use txn::{AdHocTransaction, Committed, TxnCallOptions, TxnOptions};
pub use typed::{BundleDef, Callable, CtorRelation, Key, Pending, Rows};

use settings::Settings;
use statements::{Failure, Halted};
use stop::Stops;
use text::Prepared;

use crate::catalog::OWNER_ROLE;

/// The id of the schema owner's row of `catalog::roles`.
const OWNER_ID: i64 = 1;

const PAGE_SIZE: u32 = 8192;
const DEFAULT_POOL_FRAMES: usize = 1024;
const DEFAULT_WORKERS: usize = 8;
const DEFAULT_QUERY_MEMORY: usize = 256 << 20;
const STATEMENT_GRANT: usize = 64 << 20;
/// The submission entries of the io_uring an open tries first.
#[cfg(target_os = "linux")]
const URING_ENTRIES: u32 = 256;

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
#[derive(Clone, Debug, Default)]
pub struct OpenOptions {
    /// Create the file when it does not exist; `None` fails open with `NotFound` instead.
    pub create: Option<CreateOptions>,
    pub pool_bytes: Option<u64>,
    pub query_memory_bytes: Option<u64>,
    pub workers: Option<usize>,
    /// The rounds a fixpoint may take before its statement is rejected for its limit.
    pub fix_rounds: Option<usize>,
    /// The directory a file scan resolves a relative path against; the working directory when
    /// `None`.
    pub file_root: Option<PathBuf>,
    /// What open registers once its migrations apply, all or none.
    pub registrations: Vec<Registration>,
    /// The bodies of the schema's host functions, bound before migrations run.
    pub host_fns: HostFns,
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

    /// Returns each setting as `sys::process_options` shows it.
    fn shown(&self) -> Vec<settings::Shown> {
        let shown = |name, value: String, given: bool| settings::Shown {
            name,
            value,
            derived: !given,
        };
        let pool = self.storage("").pool_frames as u64 * u64::from(PAGE_SIZE);
        let memory = self
            .query_memory_bytes
            .unwrap_or(DEFAULT_QUERY_MEMORY as u64);
        let rounds = self
            .fix_rounds
            .unwrap_or(exec::Options::default().fix_rounds);
        let root = self
            .file_root
            .as_ref()
            .map_or_else(|| ".".to_owned(), |p| p.display().to_string());
        vec![
            shown("pool_bytes", pool.to_string(), self.pool_bytes.is_some()),
            shown(
                "query_memory_bytes",
                memory.to_string(),
                self.query_memory_bytes.is_some(),
            ),
            shown(
                "workers",
                self.workers().to_string(),
                self.workers.is_some(),
            ),
            shown("fix_rounds", rounds.to_string(), self.fix_rounds.is_some()),
            shown("file_root", root, self.file_root.is_some()),
            shown("host_fns", self.host_fns.names().join(", "), true),
        ]
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
    /// An end-user or anonymous principal the call runs for, a value of the schema's
    /// `Principal`.
    pub assert: Option<Val>,
    /// The rounds a fixpoint of the call may take, else the limit the database was opened with.
    pub fix_rounds: Option<usize>,
    pub cancel: Option<CancelToken>,
}

impl CallOptions {
    fn stops(&self) -> Stops<'_> {
        Stops {
            cancel: self.cancel.as_ref(),
            deadline: self.deadline,
            limit: self.limit,
        }
    }
}

/// A handle on an open database file. Clones share one engine, which closes when the last
/// handle is dropped or when any handle is closed.
#[derive(Debug)]
pub struct Database<R: Runtime = adtb_sys::os::OsRuntime> {
    engine: Arc<Engine<R>>,
    handles: Arc<LastHandle<R>>,
}

/// Shut the engine when the last handle goes, whatever else holds the engine, such as a rebuild
/// job whose thread must be stopped and joined to release the file.
#[derive(Debug)]
struct LastHandle<R: Runtime>(Arc<Engine<R>>);

impl<R: Runtime> Drop for LastHandle<R> {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            self.0.shut();
        }
    }
}

impl<R: Runtime> Clone for Database<R> {
    fn clone(&self) -> Self {
        Database {
            engine: self.engine.clone(),
            handles: self.handles.clone(),
        }
    }
}

struct Engine<R: Runtime> {
    rt: R,
    schema: Arc<Schema>,
    program: Arc<Program>,
    /// The source of the version the database is at.
    source: Source,
    /// The registered queries of the live bundles, which `explain B::q(args)` names.
    bundles: Lock<Arc<Bundles>>,
    registered: Lock<Vec<Arc<registered::Checked>>>,
    /// The rows of `catalog::bundles` and `catalog::registered`, the family owners and the
    /// successful registrations, as the latest commit holds them.
    deployed: Lock<Arc<bundle::Stored>>,
    deployment: catalog::Deployment,
    /// Held across every write of a row of `catalog::bundles`, so that the row a writer reads
    /// from `deployed` is the one its write replaces.
    bundle_writer: Lock<()>,
    /// Whether this process has drawn an audit sequence number past the highest row.
    audit_seeded: AtomicBool,
    /// The file settings, as the latest commit holds them.
    settings: Lock<Settings>,
    role_mentions: BTreeSet<String>,
    pager: Pager<R>,
    db: Arc<write::Database<R>>,
    catalog: Arc<Catalog>,
    verifier: Arc<verify::Verifier<R>>,
    /// The engine's background worker.
    background: Arc<maintenance::Background<R>>,
    /// Whether the engine is open. A call holds it shared while it runs; close holds it
    /// exclusively.
    open: RwLock<bool>,
    /// Held across a whole close, the files' release included.
    closing: Lock<()>,
    /// The close in progress, shared with every submitted call.
    close: Closes,
    /// The workers that run submitted tasks, started on the first submit.
    sched: Lock<Option<Arc<Scheduler<R>>>>,
    workers: usize,
    /// What the running rebuild jobs need of the engine.
    reindexing: reindex::Jobs<R>,
    #[cfg(target_os = "linux")]
    held: Lock<Option<file::Held>>,
}

/// A close in progress: the thread running it, and the wakers of the calls it stopped.
pub(super) struct Close {
    by: ThreadId,
    pub(super) kept: Vec<Waker>,
}

pub(super) type Closes = Arc<Lock<Option<Close>>>;

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
            self.shut();
        }
    }
}

impl<R: Runtime> Engine<R> {
    /// Returns the engine held open for one call, or refuses a call of a closed database.
    fn enter(&self) -> Result<std::sync::RwLockReadGuard<'_, bool>, Error> {
        let open = self.open.read(&self.rt);
        if *open {
            Ok(open)
        } else {
            Err(Error::closed())
        }
    }

    /// Close the engine once every running call has returned: stop every submitted task, write
    /// every changed page, take a checkpoint, close the files and release the lock on the
    /// database file. Every later call is refused.
    ///
    /// A close on a scheduler thread, or on the thread already closing the engine, returns at
    /// once while another close runs. The observers of the calls the close stops are woken once
    /// it has ended.
    fn shut(&self) {
        let me = self.rt.current();
        if self
            .close
            .lock(&self.rt)
            .as_ref()
            .is_some_and(|c| c.by == me)
        {
            return;
        }
        let on_worker = self
            .sched
            .lock(&self.rt)
            .as_ref()
            .is_some_and(|s| s.runs_on(me));
        let closing = if on_worker {
            let Some(closing) = self.closing.try_lock() else {
                return;
            };
            closing
        } else {
            self.closing.lock(&self.rt)
        };
        *self.close.lock(&self.rt) = Some(Close {
            by: me,
            kept: Vec::new(),
        });
        self.db.stop_rebuilds();
        self.background.stop();
        let was_open = std::mem::replace(&mut *self.open.write(&self.rt), false);
        let sched = self.sched.lock(&self.rt).clone();
        if let Some(s) = sched {
            s.shutdown();
        }
        self.stop_reindexing();
        if was_open {
            self.pager.close();
            #[cfg(target_os = "linux")]
            drop(self.held.lock(&self.rt).take());
        }
        let kept = self.close.lock(&self.rt).take().map(|c| c.kept);
        drop(closing);
        for w in kept.into_iter().flatten() {
            w.wake();
        }
    }

    /// Returns the scheduler that runs submitted tasks, starting it on the first call.
    fn scheduler(&self) -> Arc<Scheduler<R>> {
        self.sched
            .lock(&self.rt)
            .get_or_insert_with(|| Scheduler::start(self.rt.clone(), self.workers))
            .clone()
    }
}

impl<R: Runtime> Database<R> {
    fn of(engine: Engine<R>) -> Database<R> {
        let engine = Arc::new(engine);
        *engine.reindexing.me.lock(&engine.rt) = Arc::downgrade(&engine);
        engine.background.start();
        let handles = Arc::new(LastHandle(engine.clone()));
        Database { engine, handles }
    }

    /// Open the database file `name` of runtime `rt`, recover it, and apply the migrations of
    /// `log` beyond the file's own.
    pub fn open_in(
        rt: R,
        name: &str,
        log: &Log,
        options: &OpenOptions,
    ) -> Result<Database<R>, OpenError> {
        Ok(Database::of(Engine::open(rt, name, log, options)?))
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
        let e = &*self.engine;
        let _open = e.enter()?;
        let caller = e.principal(opts.assert.as_ref())?;
        let p = Prepared::new(&e.schema, &e.program, &e.explained(), text, params, caller);
        let p = e.admit_text(p)?;
        e.call(&p, opts, || {}).map(|done| done.value)
    }

    /// Run `verify` over relation variable `relation`, or the whole database, while the database
    /// serves calls. Returns the rows of `sys::verify_findings` it wrote.
    pub fn verify(&self, relation: Option<&str>) -> Result<DynRows, Error> {
        let text = relation.map_or_else(|| "verify".to_owned(), |r| format!("verify {r}"));
        self.query_text(&text, &[], &CallOptions::default())?
            .pop()
            .ok_or_else(|| {
                Error::rejected(RejectReason::Internal {
                    what: "a command returns its rows".into(),
                })
            })
    }

    /// Run `verify` over the whole database with the checks of the allocation bitmap, the heaps
    /// and the large values that hold only with no writer. Returns the rows it wrote.
    ///
    /// Precondition: no other call runs on this database while it runs.
    #[doc(hidden)]
    pub fn verify_holding(&self) -> Result<DynRows, Error> {
        let e = &*self.engine;
        let _open = e.enter()?;
        e.verified_holding(&e.schema)
            .map_err(|w| e.write_error(TxnId(0), w))
    }

    #[doc(hidden)]
    pub fn on_reindex_phase(&self, hook: reindex::PhaseHook) {
        *self.engine.reindexing.hook.lock(&self.engine.rt) = Some(hook);
    }

    /// Returns how many job threads are not yet joined.
    #[doc(hidden)]
    pub fn reindex_threads(&self) -> usize {
        self.engine.reindexing.threads.lock(&self.engine.rt).len()
    }

    /// Panic the background worker's next read of a page with `text`.
    #[doc(hidden)]
    pub fn fail_background(&self, text: &'static str) {
        self.engine.background.fail(text);
    }

    /// Panic the next write of kept findings with `text`, after its first replacement.
    #[doc(hidden)]
    pub fn fail_kept_write(&self, text: &'static str) {
        *self.engine.verifier.fail_kept.lock(&self.engine.rt) = Some(text);
    }

    /// Returns the next page of the background scrub's pass as last saved in the file: the page,
    /// `None` between passes, and `None` of the outer option when nothing has been saved.
    #[doc(hidden)]
    pub fn saved_scrub_page(&self) -> Option<Option<u64>> {
        let cursor = self.engine.verifier.cursor()?;
        Some(cursor.next.map(|p| p.0))
    }

    /// Returns the rows of `sys::verify_findings` the engine holds, which a read of a database
    /// quarantined whole cannot show.
    #[doc(hidden)]
    pub fn held_findings(&self) -> usize {
        self.engine.verifier.held_findings()
    }

    /// Refuse page `pid` as if it failed as `damage` says. Plants a fault for a test of quarantine.
    #[doc(hidden)]
    pub fn damage_page(&self, pid: storage::Pid, damage: storage::Damage) {
        self.engine.pager.damage(pid, damage);
    }

    /// Refuse the pages of the store of kept rows. Plants a fault for a test of `verify`.
    #[doc(hidden)]
    pub fn damage_kept_store(&self) {
        self.engine.damage_kept_store();
    }

    /// Returns the write path, for tests that plant faults beneath it.
    #[doc(hidden)]
    pub fn write_path(&self) -> &write::Database<R> {
        &self.engine.db
    }

    /// Returns each incremental check the constraint or check `name`, by qualified name, is
    /// admitted with, or an invalid error when the schema has no constraint or check `name`.
    pub fn explain_checks(&self, name: &str) -> Result<Vec<write::CheckPlan>, Error> {
        let e = &*self.engine;
        let _open = e.enter()?;
        e.db.checks(name).ok_or_else(|| {
            Error::invalid(format!("the schema has no constraint or check `{name}`"))
        })
    }

    /// Close the database once every running call has returned: write every changed page, take
    /// a checkpoint, close the files and release the lock on the database file. Interactive
    /// transactions still open end uncommitted, and every later call through any handle on the
    /// database, a clone or a registration, is refused as the database is closed.
    ///
    /// A close on a thread that runs submitted tasks, or on the thread already closing the
    /// database, returns at once while another close runs, before the file is released.
    pub fn close(self) -> Result<(), Error> {
        self.engine.shut();
        Ok(())
    }
}

#[cfg(target_os = "linux")]
impl Database<adtb_sys::os::OsRuntime> {
    /// Open or create the database file at `path` under the host's runtime, locking it against
    /// every other open, and apply the migrations of `log` beyond the file's own.
    ///
    /// The runtime runs on io_uring where the host allows it, else on a pool of threads. An open
    /// that creates the file and then fails removes the file and its log, leaving nothing behind.
    pub fn open(
        path: impl AsRef<std::path::Path>,
        log: &Log,
        options: &OpenOptions,
    ) -> Result<Database, OpenError> {
        let path = path.as_ref();
        let fresh = options.create.is_some() && !path.exists();
        let (held, dir, name) = file::Held::take(path, options.create.is_some())?;
        let rt = os_runtime(&dir, options.workers());
        match Engine::open(rt, &name, log, options) {
            Ok(engine) => {
                *engine.held.lock(&engine.rt) = Some(held);
                Ok(Database::of(engine))
            }
            Err(e) => {
                drop(held);
                if fresh {
                    let log_dir = dir.join(options.storage(&name).log_dir());
                    remove(&dir.join(&name), &log_dir)?;
                }
                Err(e)
            }
        }
    }
}

/// Returns the host's runtime over directory `dir`: on io_uring where the host allows it, else
/// on a pool of `workers` threads.
#[cfg(target_os = "linux")]
fn os_runtime(dir: &std::path::Path, workers: usize) -> adtb_sys::os::OsRuntime {
    use adtb_sys::os::{Backend, OsRuntime};
    OsRuntime::try_new(
        dir,
        Backend::Uring {
            entries: URING_ENTRIES,
        },
    )
    .unwrap_or_else(|_| OsRuntime::new(dir, Backend::Pool { threads: workers }))
}

/// Remove the database file `file` and its log directory `log`, whichever exist.
#[cfg(target_os = "linux")]
fn remove(file: &std::path::Path, log: &std::path::Path) -> Result<(), OpenError> {
    let gone = |r: std::io::Result<()>| match r {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(OpenError::Io(e)),
        _ => Ok(()),
    };
    gone(std::fs::remove_file(file))?;
    gone(std::fs::remove_dir_all(log))
}

/// Open the database file at `path` as the one process that holds it and run `verify` over it,
/// with the checks that hold only with no writer. Returns the rows it wrote.
#[cfg(target_os = "linux")]
pub fn verify_file(
    path: impl AsRef<std::path::Path>,
    log: &Log,
    options: &OpenOptions,
) -> Result<DynRows, VerifyFileError> {
    let db = Database::open(path, log, options).map_err(VerifyFileError::Open)?;
    let rows = db.verify_holding().map_err(VerifyFileError::Run)?;
    db.close().map_err(VerifyFileError::Run)?;
    Ok(rows)
}

/// Why [`verify_file`] did not finish.
#[derive(Debug)]
pub enum VerifyFileError {
    Open(OpenError),
    Run(Error),
}

impl std::fmt::Display for VerifyFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VerifyFileError::Open(e) => e.fmt(f),
            VerifyFileError::Run(e) => e.fmt(f),
        }
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

/// Mark what holds each page that failed verification while the database opened. A page of the
/// catalog or commit log marks the database. A page nothing maps marks nothing. A failure to
/// write a mark is left to the next open, which finds the same damage.
fn mark_open_damage<R: Runtime>(rt: R, pager: &Pager<R>) {
    let damaged: Vec<storage::Pid> = pager
        .damaged()
        .into_iter()
        .filter(|(_, d)| !matches!(d, storage::Damage::Unreadable))
        .map(|(pid, _)| pid)
        .collect();
    if damaged.is_empty() {
        return;
    }
    let objects: BTreeSet<storage::Quarantine> = match Catalog::load(rt, pager) {
        Ok((catalog, txns)) => damaged
            .iter()
            .flat_map(|&pid| catalog.holders(pager, &txns, pid))
            .collect(),
        Err(_) => damaged
            .iter()
            .filter(|&&pid| Catalog::opens_from(pager, pid))
            .map(|_| storage::Quarantine::Database)
            .collect(),
    };
    for object in objects {
        pager.quarantine(object).ok();
    }
}

/// Returns the damage an open that failed may still answer `verify` over: a page that failed
/// verification on a database quarantined whole. Returns `None` while any page is unreadable.
fn open_damage<R: Runtime>(pager: &Pager<R>) -> Option<storage::PageError> {
    if !pager.is_quarantined(storage::Quarantine::Database) {
        return None;
    }
    let damaged = pager.damaged();
    if damaged
        .values()
        .any(|d| matches!(d, storage::Damage::Unreadable))
    {
        return None;
    }
    damaged
        .into_iter()
        .next()
        .map(|(pid, damage)| storage::PageError { pid, damage })
}

/// What an engine holds besides its file and write path.
struct Parts {
    schema: Arc<Schema>,
    program: Arc<Program>,
    registered: Vec<registered::Checked>,
    bundles: Bundles,
    deployed: bundle::Stored,
    settings: Settings,
    findings: verify::Findings,
}

fn query_memory(options: &OpenOptions) -> usize {
    options
        .query_memory_bytes
        .map_or(DEFAULT_QUERY_MEMORY, |b| {
            usize::try_from(b).unwrap_or(usize::MAX)
        })
}

/// Returns the setup of the write path over `catalog`.
fn setup_of<R: Runtime>(
    catalog: &Catalog,
    schema: Arc<Schema>,
    normal: norm::NormalProgram,
    pager: &Pager<R>,
    txns: Transactions<R>,
    options: &OpenOptions,
) -> Setup<R> {
    let memory = query_memory(options);
    let (heaps, indexes) = catalog.bindings();
    let defaults = exec::Options::default();
    Setup {
        schema,
        normal: Arc::new(normal),
        catalog: Arc::new(catalog.plan()),
        pager: pager.clone(),
        txns: Arc::new(txns),
        bindings: Bindings { heaps, indexes },
        budget: Budget::new(memory),
        grant: STATEMENT_GRANT.min(memory),
        pool: None,
        options: exec::Options {
            fix_rounds: options.fix_rounds.unwrap_or(defaults.fix_rounds),
            ..defaults
        },
        limits: Limits::default(),
        sequences: catalog.sequences().unwrap_or(storage::Pid(0)),
    }
}

/// Returns the bundles and settings the file holds, none when the commit log's damage leaves
/// their rows unreadable.
fn deployed_state<R: Runtime>(
    catalog: &Catalog,
    pager: &Pager<R>,
    txns: &Transactions<R>,
    clog_damaged: bool,
) -> Result<(bundle::Stored, Settings), OpenError> {
    if clog_damaged {
        return Ok((bundle::Stored::default(), Settings::default()));
    }
    Ok((
        bundle::stored(catalog, pager, txns)?,
        Settings::stored(catalog, pager, txns)?,
    ))
}

/// Returns what `verify` holds of the file and the rows of the volatile relations, with no kept
/// rows when the commit log's damage leaves them unreadable.
fn held_rows<R: Runtime>(
    catalog: &Catalog,
    pager: &Pager<R>,
    txns: &Transactions<R>,
    schema: &Schema,
    options: &OpenOptions,
    clog_damaged: bool,
) -> Result<
    (
        verify::Findings,
        BTreeMap<adtb_lang::types::RelId, BTreeSet<Val>>,
    ),
    OpenError,
> {
    let kept = if clog_damaged {
        Vec::new()
    } else {
        catalog.kept(pager, txns).map_err(corrupt)?
    };
    let table = command::engine_relation(schema, "sys", "verify_findings").map_err(corrupt)?;
    let findings = verify::Findings::of(kept, schema.ids.relations[&table])?;
    let mut volatile = settings::process_options(schema, &options.shown());
    volatile.insert(table, findings.vals());
    Ok((findings, volatile))
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
        Engine::on(rt.clone(), pager.clone(), log, options).or_else(|e| {
            mark_open_damage(rt.clone(), &pager);
            let opened = open_damage(&pager).map_or(Err(e), |damage| {
                Engine::unread(rt, pager.clone(), options, damage)
            });
            if opened.is_err() {
                pager.close();
            }
            opened
        })
    }

    /// Returns the engine of a file quarantined whole whose catalog cannot be trusted: the
    /// engine's own declarations, the mark, and `verify` of the pages and the log.
    fn unread(
        rt: R,
        pager: Pager<R>,
        options: &OpenOptions,
        damage: storage::PageError,
    ) -> Result<Engine<R>, OpenError> {
        let catalog = Catalog::unread(&pager).map_err(corrupt)?;
        let txns = Transactions::refusing(rt.clone(), &pager, damage);
        let (schema, program) = catalog.checking();
        let (schema, program) = (Arc::new(schema.clone()), program.clone());
        let normal = norm::program(&schema, &program).map_err(corrupt)?;
        let (findings, volatile) = held_rows(&catalog, &pager, &txns, &schema, options, true)?;
        let setup = setup_of(&catalog, schema.clone(), normal, &pager, txns, options);
        let db = write::Database::new(rt.clone(), setup)
            .map_err(corrupt)?
            .with_volatile(volatile);
        Ok(Engine::of(
            rt,
            Parts {
                schema,
                program,
                registered: Vec::new(),
                bundles: Bundles::default(),
                deployed: bundle::Stored::default(),
                settings: Settings::default(),
                findings,
            },
            pager,
            db,
            catalog,
            options,
        ))
    }

    /// Returns the engine over the opened `db`.
    fn of(
        rt: R,
        parts: Parts,
        pager: Pager<R>,
        db: write::Database<R>,
        catalog: Catalog,
        options: &OpenOptions,
    ) -> Engine<R> {
        let role_mentions = server::mentioned(catalog.source());
        let db = Arc::new(db);
        let catalog = Arc::new(catalog);
        let verifier = Arc::new(verify::Verifier {
            rt: rt.clone(),
            pager: pager.clone(),
            db: db.clone(),
            catalog: catalog.clone(),
            findings: Lock::new(parts.findings),
            fail_kept: Lock::new(None),
        });
        let background =
            maintenance::Background::new(verifier.clone(), parts.schema.clone(), &parts.settings);
        Engine {
            rt,
            schema: parts.schema,
            program: parts.program,
            source: catalog.source().clone(),
            bundles: Lock::new(Arc::new(parts.bundles)),
            registered: Lock::new(parts.registered.into_iter().map(Arc::new).collect()),
            deployed: Lock::new(Arc::new(parts.deployed)),
            deployment: catalog.deployment(),
            bundle_writer: Lock::new(()),
            settings: Lock::new(parts.settings),
            role_mentions,
            pager,
            db,
            catalog,
            verifier,
            background,
            open: RwLock::new(true),
            audit_seeded: AtomicBool::new(false),
            closing: Lock::new(()),
            close: Arc::default(),
            sched: Lock::new(None),
            workers: options.workers(),
            reindexing: reindex::Jobs::default(),
            #[cfg(target_os = "linux")]
            held: Lock::new(None),
        }
    }

    /// Returns `p`, the checked text of a call. On a database quarantined whole, fail with
    /// `Quarantined` whether or not it checked, unless it is a run of `verify` alone.
    fn admit_text(&self, p: Result<Prepared, Error>) -> Result<Prepared, Error> {
        let Some(refused) = self.refusal() else {
            return p;
        };
        match p {
            Ok(p) if self.lone_verify(&p) => Ok(p),
            _ => Err(refused),
        }
    }

    /// Returns the failure of every call but `verify` on a database quarantined whole.
    fn refusal(&self) -> Option<Error> {
        let refused = self.db.admit_database().err()?;
        Some(self.write_error(TxnId(0), refused))
    }

    /// Returns whether `p` is one run of `verify` this file can answer: over a relation only when
    /// its catalog was read.
    fn lone_verify(&self, p: &Prepared) -> bool {
        use adtb_lang::check::Command::Verify;
        matches!(
            (p.items(), p.commands()),
            ([], [c]) if matches!(c.command, Verify(None))
                || (matches!(c.command, Verify(Some(_))) && !self.catalog.is_unread())
        )
    }

    /// Open the catalog of the recovered `pager`, apply the migrations of `log` beyond the
    /// file's own, and bind the write path to it.
    fn on(
        rt: R,
        pager: Pager<R>,
        log: &Log,
        options: &OpenOptions,
    ) -> Result<Engine<R>, OpenError> {
        let (mut catalog, txns) = Catalog::open(rt.clone(), &pager).map_err(corrupt)?;
        let clog_damaged = txns.quarantined().is_some();
        if clog_damaged {
            pager.quarantine(storage::Quarantine::Database).ok();
        }
        let writable = !pager.is_quarantined(storage::Quarantine::Database);
        catalog.set_query_memory(query_memory(options));
        if writable {
            diverged(&catalog, log)?;
        }
        let (mut stored, settings) = deployed_state(&catalog, &pager, &txns, clog_damaged)?;
        let opened = micros(pager.wall());
        let regs = &commanded(&options.registrations, &stored, opened, &settings)?;
        if writable {
            registered::downgrade(&catalog, regs)?;
        }
        let live = stored.live(opened, &settings);
        let headers: Vec<&bundle::Header> = regs.iter().filter_map(Registration::header).collect();
        if writable {
            bundle::record(&catalog, &pager, &txns, &mut stored, &headers, opened)?;
        }
        let live = if writable {
            let roles = catalog.committed_roles(&pager, &txns).map_err(corrupt)?;
            let live = registered::readers(&mut catalog, regs, &live, &stored, &roles)?;
            catalog.settle(&pager, &txns).map_err(corrupt)?;
            let applied = catalog.source().log.len();
            options
                .host_fns
                .check(catalog.schema(), &host::pending(log, applied))?;
            live
        } else {
            Vec::new()
        };
        let hosts = Arc::new(options.host_fns.hosts());
        catalog.set_hosts(hosts.clone());
        if writable {
            catalog.counters(&pager).map_err(corrupt)?;
            catalog.record_grants(&pager, &txns).map_err(corrupt)?;
            migrate(&mut catalog, &pager, &txns, log, &live)?;
        }
        let (schema, program) = catalog.checking();
        let (schema, program) = (schema.clone(), program.clone());
        let normal = norm::program(&schema, &program).map_err(corrupt)?;
        if writable {
            options.host_fns.bound(&schema, &normal)?;
        }
        let schema = Arc::new(schema);
        let now = micros(pager.wall());
        let over = registered::Over {
            source: catalog.source(),
            schema: &schema,
            program: &program,
        };
        let registered = if writable {
            registered::register(&over, regs, &stored, now, &settings)?
        } else {
            registered::register_marked(&over, regs, &stored, now, &settings)
        };
        let given = &options.registrations;
        let bundled: Vec<_> = registered
            .iter()
            .filter(|c| given.iter().any(|g| g.name() == c.name()))
            .filter_map(registered::Checked::passed)
            .collect();
        let deployed = if writable {
            let roles = catalog.committed_roles(&pager, &txns).map_err(corrupt)?;
            registered::definers(&registered, &stored, catalog.source(), &roles)?;
            let deployed =
                bundle::register(&catalog, &pager, &txns, stored, &bundled, now, &settings)?;
            server::initialize(&catalog, &pager, &txns)?;
            deployed
        } else {
            stored
        };
        let mut bundles = Bundles::default();
        for c in &registered {
            c.explained(&mut bundles);
        }
        let (findings, volatile) =
            held_rows(&catalog, &pager, &txns, &schema, options, clog_damaged)?;
        let retained = if writable {
            Some(catalog.retained(&pager, &txns).map_err(corrupt)?)
        } else {
            None
        };
        let setup = setup_of(&catalog, schema.clone(), normal, &pager, txns, options);
        let db = write::Database::new(rt.clone(), setup)
            .map_err(corrupt)?
            .with_files(options.file_root.clone().map(Disk::new).unwrap_or_default())
            .with_hosts(hosts)
            .with_volatile(volatile)
            .with_kept_store(catalog.kept_store());
        let db = match retained {
            Some(retained) => db
                .with_history(retained, settings.window)
                .map_err(corrupt)?,
            None => db,
        };
        Ok(Engine::of(
            rt,
            Parts {
                schema,
                program,
                registered,
                bundles,
                deployed,
                settings,
                findings,
            },
            pager,
            db,
            catalog,
            options,
        ))
    }

    /// Returns the registered queries `explain B::q(args)` names.
    fn explained(&self) -> Arc<Bundles> {
        self.bundles.lock(&self.rt).clone()
    }

    /// Returns a new transaction id.
    fn txn_id(&self) -> TxnId {
        TxnId(i64::try_from((self.rt.next_u64() >> 1).max(1)).expect("63 bits fit an i64"))
    }

    fn write_error(&self, txn: TxnId, e: write::Error) -> Error {
        Error::of_write(&self.schema, txn, self.db.quarantining(e))
    }

    /// Returns the client error of `e`, met writing catalog rows.
    fn catalog_error(&self, e: catalog::Error) -> Error {
        match e {
            catalog::Error::Page(p) | catalog::Error::Heap(crate::heap::Error::Page(p)) => {
                self.write_error(TxnId(0), write::Error::Page(p))
            }
            other => Error::of_catalog_write(other),
        }
    }

    /// Run the items of `p` as statements of `tx`, the transaction `id`, over `local`. Returns
    /// the rows of each item that yields a result, or the failure.
    fn statements(
        &self,
        id: TxnId,
        tx: &Txn,
        p: &Prepared,
        local: &mut Local,
        bounds: Bounds,
    ) -> Result<Vec<DynRows>, Failure> {
        self.resumable(id, tx, p, local, bounds)
            .map_err(|f| match f {
                Halted::Blocked { on, rerun } => Failure {
                    error: Error::of_write(p.schema(), id, write::Error::Blocked { on, rerun }),
                    ends: false,
                    commanded: false,
                },
                Halted::Failed(failure) => failure,
            })
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
        micros(self.pager.wall())
    }

    /// Run `p` as one transaction of its own under `opts`: read-only when `p` only reads, else
    /// rerun on a serialization failure before any command ran, up to `opts.retry`, else `p`'s
    /// `retry n`, else the file setting `RetryLimit`. Returns the rows of each item that yields a
    /// result, with the commit position and the attempts made.
    ///
    /// A call whose token is cancelled, or whose deadline passes, before it commits fails with
    /// `Cancelled` or `DeadlineExceeded` and has no effect.
    /// Invoke `on_ready` once after checking the call options, before beginning execution.
    fn call(
        &self,
        p: &Prepared,
        opts: &CallOptions,
        on_ready: impl FnOnce(),
    ) -> Result<Committed<Vec<DynRows>>, Error> {
        let stops = opts.stops();
        stops.check(&self.rt)?;
        self.reads_after(opts.after)?;
        on_ready();
        let bounds = stops.bounds(&self.rt, opts.fix_rounds);
        if let Some(rows) = self.verify_without_transaction(p) {
            return rows.map(|rows| Committed {
                value: vec![rows],
                position: Position::from(self.pager.position(self.pager.durable())),
                attempts: 1,
            });
        }
        if !p.writes {
            let id = self.txn_id();
            let tx = self.begin_read(id, p.deferrable)?;
            return self
                .run(id, tx, p, bounds)
                .map(|(value, position)| Committed {
                    value,
                    position,
                    attempts: 1,
                })
                .map_err(|f| f.error);
        }
        let mut attempts = 1;
        loop {
            stops.check(&self.rt)?;
            let id = self.txn_id();
            let tx = self.begin(id)?;
            match self.run(id, tx, p, bounds.clone()) {
                Err(f) if self.reruns_after(p, opts.retry, attempts, &f.error, f.commanded) => {
                    attempts += 1;
                }
                done => {
                    return done
                        .map(|(value, position)| Committed {
                            value,
                            position,
                            attempts,
                        })
                        .map_err(|f| f.error);
                }
            }
        }
    }

    /// Returns the run of `verify` that `p` is, when `p` is nothing else and the commit log's
    /// damage refuses every transaction.
    pub(super) fn verify_without_transaction(
        &self,
        p: &Prepared,
    ) -> Option<Result<DynRows, Error>> {
        self.db.txns().quarantined()?;
        let ([], [c]) = (p.items(), p.commands()) else {
            return None;
        };
        let adtb_lang::check::Command::Verify(rel) = c.command else {
            return None;
        };
        Some(
            self.verified(p.schema(), rel, write::HoldWait::unbounded())
                .map_err(|e| self.write_error(TxnId(0), e)),
        )
    }

    /// Returns how often a call of `p` reruns on a serialization failure: `retry`, else `p`'s
    /// `retry n`, else the file setting `RetryLimit`.
    fn reruns(&self, p: &Prepared, retry: Option<u32>) -> u64 {
        retry.map_or_else(
            || p.retry.unwrap_or(self.settings.lock(&self.rt).retries),
            u64::from,
        )
    }

    /// Returns whether a call of `p` makes another attempt after `attempts`, the last failing
    /// with `err`: a serialization failure, no command having run, within its reruns.
    fn reruns_after(
        &self,
        p: &Prepared,
        retry: Option<u32>,
        attempts: u32,
        err: &Error,
        commanded: bool,
    ) -> bool {
        err.is_serialization() && !commanded && u64::from(attempts) <= self.reruns(p, retry)
    }

    /// Run `p` in `tx`, the transaction `id`, bounded by `bounds`, and commit it, or abort it on
    /// any failure. Returns the rows of each item that yields a result and the commit position.
    fn run(
        &self,
        id: TxnId,
        tx: Txn,
        p: &Prepared,
        bounds: Bounds,
    ) -> Result<(Vec<DynRows>, Position), Failure> {
        let (cancel, deadline) = (bounds.cancel.clone(), bounds.deadline);
        let past_commands = |error| Failure {
            error,
            ends: true,
            commanded: !p.commands().is_empty(),
        };
        let rows = self
            .statements(id, &tx, p, &mut Local::default(), bounds)
            .and_then(|rows| {
                if cancel.is_some_and(|c| c.load(std::sync::atomic::Ordering::Acquire)) {
                    Err(past_commands(Error::rejected(RejectReason::Cancelled)))
                } else if deadline.is_some_and(|d| self.rt.now() >= d) {
                    Err(past_commands(Error::rejected(
                        RejectReason::DeadlineExceeded,
                    )))
                } else {
                    Ok(rows)
                }
            });
        match rows {
            Ok(rows) => {
                let at = self
                    .db
                    .commit(tx)
                    .map_err(|e| past_commands(self.write_error(id, e)))?;
                Ok((rows, Position::from(at)))
            }
            Err(f) => {
                self.abort(id, tx).map_err(past_commands)?;
                Err(f)
            }
        }
    }

    /// Returns the principal a call runs for: `assert`, an end user or the anonymous
    /// principal, else the schema owner's service principal. Refuses a service principal with
    /// `Denied` and a value of another type as invalid.
    fn principal(&self, assert: Option<&Val>) -> Result<Val, Error> {
        let def = &self.schema.types[self.schema.principal.0 as usize];
        let service = def
            .ctor_index("Service")
            .expect("the schema's `Principal` has a `Service` constructor");
        let Some(v) = assert else {
            return Ok(Val::ctor(service, vec![Val::text(OWNER_ROLE)]));
        };
        let arity = match v {
            Val::Ctor(c, fields) => def
                .ctors()
                .get(*c as usize)
                .filter(|ctor| ctor.fields.len() == fields.len())
                .map(|_| *c),
            _ => None,
        };
        let principal = Ty::Data {
            def: self.schema.principal,
            args: Vec::new(),
            ctors: None,
        };
        match arity {
            Some(c) if c == service => Err(Error::rejected(RejectReason::Denied {
                detail: "a service principal is the role the handle acts as; it is never asserted"
                    .into(),
            })),
            Some(_) if value::valid(&self.schema, &principal, v) => Ok(v.clone()),
            Some(_) | None => Err(Error::invalid(
                "an asserted principal is a value of the schema's `Principal`",
            )),
        }
    }
}

/// Returns `t` in microseconds since 1970-01-01T00:00:00Z.
fn micros(t: SystemTime) -> i64 {
    let micros = |d: Duration| i64::try_from(d.as_micros()).unwrap_or(i64::MAX);
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => micros(d),
        Err(e) => -micros(e.duration()),
    }
}

/// Returns `given` with the registration of each live bundle at `now`, under the file settings
/// `s`, that a role registered by the command and `given` does not name.
fn commanded(
    given: &[Registration],
    stored: &bundle::Stored,
    now: i64,
    s: &Settings,
) -> Result<Vec<Registration>, OpenError> {
    let mut out = given.to_vec();
    for (name, text) in stored.commanded(now, s) {
        if given.iter().all(|r| r.name() != name) {
            out.push(Registration::bundle(text).map_err(corrupt)?);
        }
    }
    Ok(out)
}

/// Refuse a log whose migrations at or below the file's version differ from the file's by
/// hash, naming the first that does.
fn diverged(catalog: &Catalog, log: &Log) -> Result<(), OpenError> {
    let stored = &catalog.source().log;
    match stored
        .iter()
        .zip(&log.migrations)
        .find(|(m, shipped)| &m.text != *shipped && !same_hash(m, shipped))
    {
        Some((m, _)) => Err(OpenError::Diverged { version: m.number }),
        None => Ok(()),
    }
}

/// Apply the migrations of `log` beyond the file's own, each as its own schema change, stopping
/// at the first refused one or the first across which a bundle of `live` has no translation.
fn migrate<R: Runtime>(
    catalog: &mut Catalog,
    pager: &Pager<R>,
    txns: &Transactions<R>,
    log: &Log,
    live: &[registered::Live],
) -> Result<(), OpenError> {
    let stored = catalog.source().log.len();
    for text in log.migrations.iter().skip(stored) {
        let version = catalog.source().version + 1;
        registered::recheck(catalog, text, live).map_err(|reason| refused(version, reason))?;
        catalog.migrate(pager, txns, text).map_err(|e| match e {
            catalog::Error::Refused(why) => refused(
                version,
                RejectReason::SchemaRefused {
                    operator: String::new(),
                    detail: why.join("\n"),
                },
            ),
            catalog::Error::RefusedBy { operator, why } => refused(
                version,
                RejectReason::SchemaRefused {
                    operator,
                    detail: why.join("\n"),
                },
            ),
            catalog::Error::Internal(what) => refused(version, RejectReason::Internal { what }),
            catalog::Error::MissingFunction(name) => OpenError::HostFnMissing { name },
            catalog::Error::OtherVersion(name) => OpenError::HostFnMismatch { name },
            catalog::Error::Unchecked(built, ds) => {
                let what = ds.into_iter().map(|d| d.message).collect::<Vec<_>>();
                let what = format!("{built} does not check: {}", what.join("; "));
                refused(version, RejectReason::Internal { what })
            }
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
        catalog::try_version_hash(&schema, &source).ok()
    };
    hash(text).is_some_and(|h| Some(h) == hash(&m.text))
}
