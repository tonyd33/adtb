//! A database on a real file under the host's runtime, with no simulator.

use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use adtb::catalog::Catalog;
use adtb::storage::{OpenError, Options, PageKind, Pager, Pid};
use adtb::verify;
use adtb_lang::schema::ValueDef;
use adtb_sys::os::{Backend, Faults, OsRuntime};

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

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("real_file")
        .join(name);
    match std::fs::remove_dir_all(&dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => panic!("clear {dir:?}: {e}"),
        _ => {}
    }
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn runtime(dir: &Path) -> OsRuntime {
    OsRuntime::new(dir, Backend::Pool { threads: 4 })
}

fn faulty(dir: &Path, faults: Faults) -> OsRuntime {
    OsRuntime::with_faults(dir, Backend::Pool { threads: 4 }, faults)
}

fn no_faults(dir: &Path) -> OsRuntime {
    faulty(dir, Faults::default())
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
    let report = verify::verify(&pager, &catalog.inventory(&txns.commit_log()));
    assert!(report.is_clean() && report.complete, "{report:#?}");
}

#[test]
fn a_migrated_database_reopens_from_its_files() {
    migrated_database_reopens(&scratch("reopen"), runtime);
}

#[test]
fn a_migrated_database_reopens_from_its_files_beneath_the_fault_injector() {
    migrated_database_reopens(&scratch("reopen-faulty"), no_faults);
}

#[test]
fn a_database_is_created_once() {
    let dir = scratch("create-once");
    let rt = runtime(&dir);
    drop(Pager::create(rt.clone(), &options()).expect("create"));
    assert!(matches!(
        Pager::create(rt, &options()),
        Err(OpenError::Exists)
    ));
}

#[test]
fn a_missing_database_is_not_found() {
    let dir = scratch("missing");
    assert!(matches!(
        Pager::open(runtime(&dir), &options()),
        Err(OpenError::NotFound)
    ));
}

#[test]
fn a_closed_pager_holds_no_file_open_and_reopens_with_nothing_to_replay() {
    let dir = scratch("close");
    let rt = runtime(&dir);
    let pager = Pager::create(rt.clone(), &options()).expect("create");
    let pid = pager.allocate(1, PageKind::HeapSlotted, 7).expect("a page");
    pager.update(&[pid], |p| p[0][100] = 42).expect("an update");
    assert!(!open_under(&dir).is_empty());
    pager.close();
    assert_eq!(Vec::<PathBuf>::new(), open_under(&dir));
    drop(pager);

    let (pager, recovery) = Pager::open(rt, &options()).expect("reopen");
    assert_eq!(0, recovery.applied, "{recovery:?}");
    assert_eq!(42, pager.read(pid).expect("the page")[100]);
}

const PAGES: usize = 12;
const AT: usize = 100;

fn counter_options() -> Options {
    Options {
        pool_frames: 6,
        ..options()
    }
}

fn counter(pager: &Pager<OsRuntime>, pid: Pid) -> Result<u64, String> {
    let page = pager.read(pid).map_err(|e| format!("{e:?}"))?;
    Ok(u64::from_le_bytes(page[AT..AT + 8].try_into().unwrap()))
}

/// Create a database of `PAGES` counter pages on `rt`, with every allocation durable.
fn counters(rt: OsRuntime) -> (Pager<OsRuntime>, Vec<Pid>) {
    let pager = Pager::create(rt, &counter_options()).expect("create");
    let pids: Vec<Pid> = (0..PAGES)
        .map(|_| pager.allocate(1, PageKind::HeapSlotted, 7).expect("a page"))
        .collect();
    pager.checkpoint();
    (pager, pids)
}

/// Commit `steps` increments of counters `seed` picks, recording in `acked` each one acknowledged
/// and in `doubt` the page of a commit begun and not acknowledged.
fn increment(
    pager: &Pager<OsRuntime>,
    pids: &[Pid],
    seed: u64,
    steps: u64,
    acked: &mut [u64],
    doubt: &mut Option<usize>,
) {
    for step in 0..steps {
        let i = ((step * 7 + seed * 3) % PAGES as u64) as usize;
        *doubt = Some(i);
        let next = acked[i] + 1;
        let lsn = pager
            .commit(step + 1, &[pids[i]], |p| {
                p[0][AT..AT + 8].copy_from_slice(&next.to_le_bytes());
            })
            .expect("a commit");
        pager.wait_durable(lsn);
        acked[i] = next;
        *doubt = None;
    }
}

/// Check every counter after a restart: an acknowledged increment is present, and only the one
/// in doubt may be past it.
fn check_counters(dir: &Path, pids: &[Pid], acked: &[u64], doubt: Option<usize>, seed: u64) {
    let (pager, _) = Pager::open(runtime(dir), &counter_options()).expect("reopen");
    for (i, &pid) in pids.iter().enumerate() {
        let got = counter(&pager, pid).unwrap_or_else(|e| panic!("seed {seed}: page {i}: {e}"));
        let ok = got == acked[i] || (doubt == Some(i) && got == acked[i] + 1);
        assert!(
            ok,
            "seed {seed}: page {i} holds {got}, acknowledged {}",
            acked[i]
        );
    }
    let findings = pager.verify().findings;
    assert!(findings.is_empty(), "seed {seed}: {findings:?}");
}

#[test]
fn acknowledged_commits_survive_crashes_with_torn_and_lost_unforced_writes() {
    for seed in 0..6 {
        let dir = scratch(&format!("crash-{seed}"));
        let rt = faulty(
            &dir,
            Faults {
                seed,
                torn_p: 0.5,
                atomic_unit: 4096,
                ..Faults::default()
            },
        );
        let (pager, pids) = counters(rt.clone());
        let mut acked = vec![0; PAGES];
        let mut doubt = None;
        increment(&pager, &pids, seed, 20 + seed * 7, &mut acked, &mut doubt);
        pager.flush_pages(seed as usize);
        rt.crash();
        drop(pager);
        check_counters(&dir, &pids, &acked, doubt, seed);
    }
}

#[test]
fn a_failed_write_or_force_crashes_the_engine_and_recovery_keeps_every_acknowledged_commit() {
    for seed in 0..6 {
        let dir = scratch(&format!("fail-{seed}"));
        let rt = faulty(
            &dir,
            Faults {
                seed,
                ..Faults::default()
            },
        );
        let (pager, pids) = counters(rt.clone());
        rt.set_faults(Faults {
            seed,
            fail_p: 0.05,
            torn_p: 0.5,
            ..Faults::default()
        });
        let mut acked = vec![0; PAGES];
        let mut doubt = None;
        let run = panic::catch_unwind(AssertUnwindSafe(|| {
            increment(&pager, &pids, seed, 400, &mut acked, &mut doubt);
        }));
        assert!(run.is_err(), "seed {seed}: some write or force fails");
        rt.crash();
        drop(pager);
        check_counters(&dir, &pids, &acked, doubt, seed);
    }
}

#[test]
fn rot_latent_errors_and_misdirected_and_lost_writes_are_refused_never_returned() {
    let (mut returned, mut refused) = (0, 0);
    for seed in 0..6 {
        let dir = scratch(&format!("damage-{seed}"));
        let rt = faulty(
            &dir,
            Faults {
                seed,
                ..Faults::default()
            },
        );
        let (pager, pids) = counters(rt.clone());
        let mut acked = [0u64; PAGES];
        rt.set_faults(Faults {
            seed,
            corrupt_p: 0.02,
            latent_p: 0.02,
            misdirect_p: 0.02,
            lost_write_p: 0.02,
            lose_unsynced: false,
            ..Faults::default()
        });
        for step in 0..60u64 {
            let i = ((step * 7 + seed * 3) % PAGES as u64) as usize;
            let next = acked[i] + 1;
            let commit = pager.commit(step + 1, &[pids[i]], |p| {
                p[0][AT..AT + 8].copy_from_slice(&next.to_le_bytes());
            });
            if let Ok(lsn) = commit {
                pager.wait_durable(lsn);
                acked[i] = next;
            }
        }
        pager.flush_pages(usize::MAX);
        for (i, &pid) in pids.iter().enumerate() {
            match counter(&pager, pid) {
                Ok(got) => {
                    returned += 1;
                    assert_eq!(
                        acked[i], got,
                        "seed {seed}: page {i} returned wrong contents"
                    );
                }
                Err(_) => refused += 1,
            }
        }
    }
    assert!(
        returned > 0 && refused > 0,
        "{returned} returned, {refused} refused"
    );
}

#[test]
fn a_failed_open_or_create_holds_no_file_open() {
    let dir = scratch("failed-open");
    let rt = runtime(&dir);
    std::fs::write(dir.join("shop.adt"), b"").expect("an empty file");
    assert!(matches!(
        Pager::open(rt.clone(), &options()),
        Err(OpenError::NoHeader)
    ));
    assert_eq!(Vec::<PathBuf>::new(), open_under(&dir));
    std::fs::write(dir.join("shop.adt"), [1; 4096]).expect("a nonempty file");
    assert!(matches!(
        Pager::create(rt, &options()),
        Err(OpenError::Exists)
    ));
    assert_eq!(Vec::<PathBuf>::new(), open_under(&dir));
}
