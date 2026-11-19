//! A database on a real file under the host's runtime, with no simulator.

use std::collections::BTreeSet;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use adtb_testkit::scratch::Scratch;

use crate::catalog::Catalog;
use crate::runtime::FatalIo;
use crate::storage::{OpenError, Options, PageKind, Pager};
use crate::testing::Knobs;
use crate::{CallOptions, CreateOptions, Database, Log, OpenOptions, verify};
use adtb_lang::schema::ValueDef;
use adtb_lang::textual::print;
use adtb_sys::os::{Backend, Faults, Loss, OsRuntime};
use adtb_testkit::seeds::{assert_reached, sweep};

const SCHEMA: &str = r"module shop
add module shop
add type Customer = { id: Key customers, name: Text }
add relation customers : Customer key id generate sequence
add type Payment { id: Key payments, customer: Key customers }
  = Card { last4: Text }
  | Invoice { due: Text }
add relation payments : Payment key id generate uuidv7
";

fn options() -> Options {
    Options {
        page_size: 4096,
        log_block: 4096,
        segment_size: 256 * 1024,
        pool_frames: 64,
        atomic_page_writes: false,
        ..Options::new("shop.adt")
    }
}

fn scratch(name: &str) -> Scratch {
    Scratch::new("real_file", name)
}

fn runtime(dir: &Path) -> OsRuntime {
    OsRuntime::new(dir, Backend::Pool { threads: 4 })
}

fn faulty(dir: &Path, faults: Faults) -> OsRuntime {
    OsRuntime::with_faults(dir, Backend::Pool { threads: 4 }, faults)
}

fn no_faults(dir: &Path) -> OsRuntime {
    faulty(dir, Faults::none(4096))
}

/// Returns the files under `dir` this process holds open.
fn open_under(dir: &Path) -> Vec<PathBuf> {
    let dir = dir.canonicalize().expect("the directory");
    std::fs::read_dir("/proc/self/fd")
        .expect("the process's descriptors")
        .filter_map(|e| std::fs::read_link(e.expect("a descriptor").path()).ok())
        .filter(|p| p.starts_with(&dir))
        .collect()
}

fn has_relation(catalog: &Catalog, module: &str, name: &str) -> bool {
    let schema = catalog.schema();
    let m = schema.module_named(module).expect("the module");
    matches!(
        schema.module(m).values.get(name),
        Some(ValueDef::Relation(_))
    )
}

fn migrated_database_reopens(dir: &Path, runtime: fn(&Path) -> OsRuntime) {
    let dir = dir.to_path_buf();
    {
        let rt = runtime(&dir);
        let pager = Pager::create(rt.clone(), &options()).expect("create");
        let (mut catalog, txns) = Catalog::create(rt, &pager).expect("the catalog");
        catalog
            .migrate(&pager, &txns, SCHEMA)
            .expect("the migration applies");
    }
    assert!(dir.join("shop.adt").is_file());
    let segments = std::fs::read_dir(dir.join("shop.adt-log"))
        .expect("the log directory")
        .count();
    assert!(segments > 0, "the log has a segment");

    let rt = runtime(&dir);
    let (pager, _) = Pager::open(rt.clone(), &options()).expect("reopen");
    let (catalog, txns) = Catalog::open(rt, &pager).expect("the catalog opens");
    assert_eq!(1, catalog.source().version);
    assert!(has_relation(&catalog, "shop", "customers"));
    assert!(has_relation(&catalog, "shop", "payments"));
    let report = verify::verify(&pager, &catalog.structures().inventory(&txns.commit_log()));
    assert!(report.is_clean() && report.complete, "{report:#?}");
}

#[test]
fn a_migrated_database_reopens_from_its_files() {
    migrated_database_reopens(scratch("reopen").path(), runtime);
}

#[test]
fn a_migrated_database_reopens_from_its_files_beneath_the_fault_injector() {
    migrated_database_reopens(scratch("reopen-faulty").path(), no_faults);
}

#[test]
fn a_database_is_created_once() {
    let held = scratch("create-once");
    let dir = held.path();
    let rt = runtime(dir);
    drop(Pager::create(rt.clone(), &options()).expect("create"));
    assert!(matches!(
        Pager::create(rt, &options()),
        Err(OpenError::Exists)
    ));
}

#[test]
fn a_missing_database_is_not_found() {
    let held = scratch("missing");
    let dir = held.path();
    assert!(matches!(
        Pager::open(runtime(dir), &options()),
        Err(OpenError::NotFound)
    ));
}

#[test]
fn a_closed_pager_holds_no_file_open_and_reopens_with_nothing_to_replay() {
    let held = scratch("close");
    let dir = held.path();
    let rt = runtime(dir);
    let pager = Pager::create(rt.clone(), &options()).expect("create");
    let pid = pager.allocate(1, PageKind::HeapSlotted, 7).expect("a page");
    pager.update(&[pid], |p| p[0][100] = 42).expect("an update");
    assert!(!open_under(dir).is_empty());
    pager.close();
    assert_eq!(Vec::<PathBuf>::new(), open_under(dir));
    drop(pager);

    let (pager, opening) = Pager::open(rt, &options()).expect("reopen");

    let recovery = opening.recovery().cloned().expect("the open recovered");
    assert_eq!(0, recovery.applied, "{recovery:?}");
    assert_eq!(42, pager.read(pid).expect("the page")[100]);
}

/// Returns the name a backend's directories begin with.
fn label(backend: Backend) -> &'static str {
    match backend {
        Backend::Pool { .. } => "pool",
        Backend::Uring { .. } => "uring",
    }
}

macro_rules! on_both_backends {
    ($($name:ident),* $(,)?) => {
        mod pool {
            $(
                #[test]
                fn $name() {
                    super::$name(adtb_sys::os::Backend::Pool { threads: 4 });
                }
            )*
        }
        mod uring {
            $(
                #[test]
                fn $name() {
                    if let Some(backend) = adtb_testkit::backends::uring(64) {
                        super::$name(backend);
                    }
                }
            )*
        }
    };
}

on_both_backends!(acknowledged_commits_survive_injected_failures_and_a_crash);

#[test]
fn a_failed_open_or_create_holds_no_file_open() {
    let held = scratch("failed-open");
    let dir = held.path();
    let rt = runtime(dir);
    std::fs::write(dir.join("shop.adt"), b"").expect("an empty file");
    assert!(matches!(
        Pager::open(rt.clone(), &options()),
        Err(OpenError::NoHeader)
    ));
    assert_eq!(Vec::<PathBuf>::new(), open_under(dir));
    std::fs::write(dir.join("shop.adt"), [1; 4096]).expect("a nonempty file");
    assert!(matches!(
        Pager::create(rt, &options()),
        Err(OpenError::Exists)
    ));
    assert_eq!(Vec::<PathBuf>::new(), open_under(dir));
}

const POOL_BYTES: u64 = 256 * 1024;
const NAME_PAD: usize = 1500;

const CUSTOMERS: &str = r"module shop
add module shop
add type Customer = { id: Key customers, name: Text }
add relation customers : Customer key id generate sequence
";

/// What one faulted workload left: the names of the commits acknowledged and of those refused,
/// how many refusals were fatal I/O failures, after which the process counts as crashed, and
/// what the crash dropped and tore.
#[derive(Default)]
struct Commits {
    acked: BTreeSet<String>,
    refused: BTreeSet<String>,
    storage: usize,
    loss: Loss,
}

fn answers(db: &Database<OsRuntime>, text: &str) -> BTreeSet<String> {
    let rows = db
        .query_text(text, &[], &CallOptions::default())
        .unwrap_or_else(|e| panic!("`{text}` fails: {e:?}"))
        .remove(0);
    let form = rows.form().clone();
    rows.map(|r| print(&r.expect("a row"), &form).expect("a row prints"))
        .collect()
}

/// Insert one customer per commit into a new database on `backend` under failing writes and
/// forces until a fatal I/O failure, then crash it.
fn commit_under_faults(dir: &Path, backend: Backend, seed: u64) -> Commits {
    let rt = OsRuntime::with_faults(
        dir,
        backend,
        Faults {
            seed,
            ..Faults::none(4096)
        },
    );
    let create = OpenOptions {
        create: Some(CreateOptions::default()),
        workers: Some(2),
        pool_bytes: Some(POOL_BYTES),
        testing: Knobs::without_background_vacuum(),
        ..OpenOptions::default()
    };
    let db = Database::open_in(rt.clone(), "shop.adt", &Log::new([CUSTOMERS]), &create)
        .unwrap_or_else(|e| panic!("seed {seed}: create: {e:?}"));
    rt.set_faults(Faults {
        seed,
        fail_p: 0.05,
        torn_p: 0.5,
        ..Faults::none(4096)
    });
    let mut commits = Commits::default();
    for i in 0..40 {
        let name = format!("\"c{i}{}\"", "x".repeat(NAME_PAD));
        let insert = format!("insert customers {{ name: {name} }}");
        assert_eq!(
            0,
            db.testing().take_ends_by_drop(),
            "seed {seed}: a transaction before commit {i} was dropped"
        );
        let run = panic::catch_unwind(AssertUnwindSafe(|| {
            db.query_text(&insert, &[], &CallOptions::default())
        }));
        match run {
            Ok(Ok(_)) => {
                commits.acked.insert(name);
            }
            Ok(Err(_)) => {
                commits.refused.insert(name);
            }
            Err(payload) if payload.is::<FatalIo>() => {
                let unwound = db.testing().take_ends_by_drop();
                assert!(
                    unwound <= 1,
                    "seed {seed}: commit {i}'s fatal I/O failure ended {unwound} transactions by drop"
                );
                commits.storage += 1;
                commits.refused.insert(name);
                break;
            }
            Err(payload) => panic::resume_unwind(payload),
        }
    }
    commits.loss = rt.crash();
    if let Err(payload) = panic::catch_unwind(AssertUnwindSafe(|| drop(db)))
        && !payload.is::<FatalIo>()
    {
        let message = payload.downcast_ref::<String>().map(String::as_str);
        panic!(
            "seed {seed}: the close fails: {}",
            message.unwrap_or("a non-text panic")
        );
    }
    commits
}

fn acknowledged_commits_survive_injected_failures_and_a_crash(backend: Backend) {
    let (mut acked, mut storage, mut loss) = (0, 0, Loss::default());
    for seed in sweep(0..4) {
        let held = scratch(&format!("db-faults-{}-{seed}", label(backend)));
        let dir = held.path();
        let commits = commit_under_faults(dir, backend, seed);
        let db = Database::open_in(
            OsRuntime::new(dir, backend),
            "shop.adt",
            &Log::new([CUSTOMERS]),
            &OpenOptions::default(),
        )
        .unwrap_or_else(|e| panic!("seed {seed}: reopen: {e:?}"));
        let got = answers(&db, "from c in customers select c.name");
        let missing: Vec<_> = commits.acked.difference(&got).collect();
        assert!(
            missing.is_empty(),
            "seed {seed}: acknowledged and lost: {missing:?}"
        );
        let unknown: Vec<_> = got
            .iter()
            .filter(|n| !commits.acked.contains(*n) && !commits.refused.contains(*n))
            .collect();
        assert!(
            unknown.is_empty(),
            "seed {seed}: never committed: {unknown:?}"
        );
        let findings = db
            .verify(None)
            .unwrap_or_else(|e| panic!("seed {seed}: verify: {e:?}"));
        assert_eq!(0, findings.count(), "seed {seed}: verify finds damage");
        acked += commits.acked.len();
        storage += commits.storage;
        loss.lost += commits.loss.lost;
        loss.torn += commits.loss.torn;
    }
    assert_reached("an injected storage failure inside a commit", storage, 1);
    assert_reached("an acknowledged commit read back", acked, 1);
    assert_reached("an unforced write a crash dropped", loss.lost, 1);
    assert_reached("an unforced write a crash tore", loss.torn, 1);
}
