use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::root::{self, Changing, Root};
use crate::catalog::transaction::Change;
use crate::catalog::{Catalog, Error, KeptField, KeptRow, applied, declared};
use crate::key::Value;
use crate::mvcc::{CommitLog, Scope, Transactions, Xid};
use crate::storage::{Damage, HEADER_LEN, Options, Pager, Pid, Quarantine};
use crate::verify::{self, Finding, Problem, Report};
use adtb_lang::types::RelId;
use adtb_sim::{Config, Outcome, Sim, SimRuntime};
use adtb_sys::runtime::Faults;
use adtb_testkit::process::finished;
use adtb_testkit::profile::{every_case, spread};

use crate::test_world as harness;
use adtb_testkit::damage::DataFile;

use harness::{PAGE, options};

const FRAMES: usize = 64;

/// Returns the options of a pager whose pool is small enough that a migration's pages reach the
/// device before it commits.
fn small_pool() -> Options {
    Options {
        pool_frames: 16,
        ..options(FRAMES)
    }
}

fn crash_config() -> Config {
    Config {
        faults: Faults {
            torn_p: 0.5,
            ..Faults::none(512)
        },
        ..Config::default()
    }
}

/// Returns the rows of `rows` in an order that depends only on the rows.
fn sorted(rows: &[(u32, Vec<Value>)]) -> Vec<String> {
    let mut out: Vec<String> = rows.iter().map(|r| format!("{r:?}")).collect();
    out.sort();
    out
}

fn verify_catalog(
    pager: &Pager<SimRuntime>,
    catalog: &Catalog,
    txns: &Transactions<SimRuntime>,
) -> Report {
    let clog = txns.commit_log();
    verify::verify(pager, &catalog.structures().inventory(Some(&clog)))
}

/// Returns the rows of every relation of `catalog` that every committed transaction wrote.
fn committed(
    pager: &Pager<SimRuntime>,
    catalog: &Catalog,
    txns: &Transactions<SimRuntime>,
) -> BTreeMap<RelId, Vec<String>> {
    let hold = txns.take(Scope::Every).expect("a snapshot");
    let out = catalog
        .schema()
        .ids()
        .relation_ids()
        .map(|(r, _)| {
            let rows = catalog.rows(pager, txns, &hold, r).expect("the rows read");
            (r, sorted(&rows))
        })
        .collect();
    drop(hold);
    out
}

/// Returns the rows of `catalog::migrations` every committed transaction wrote.
fn migration_rows(
    pager: &Pager<SimRuntime>,
    catalog: &Catalog,
    txns: &Transactions<SimRuntime>,
) -> Vec<(u32, Vec<Value>)> {
    let hold = txns.take(Scope::Every).expect("a snapshot");
    let rows = catalog
        .rows(pager, txns, &hold, rel(catalog, "catalog", "migrations"))
        .expect("the rows read");
    drop(hold);
    rows
}

/// Returns `rows` of `catalog::migrations` without their `applied` field, checking that it is
/// an instant.
fn unapplied(rows: &[(u32, Vec<Value>)]) -> Vec<String> {
    rows.iter()
        .map(|(c, row)| {
            assert!(matches!(row[3], Value::Instant(_)), "{row:?}");
            format!("{:?}", (c, &row[..3]))
        })
        .collect()
}

/// Check that the catalog of `pager` holds the rows its schema declares and the row of its
/// latest migration, that no transaction
/// committed twice, and that it verifies clean.
fn check_whole(pager: &Pager<SimRuntime>, catalog: &Catalog, txns: &Transactions<SimRuntime>) {
    check_whole_but(pager, catalog, txns, None);
}

/// Check as [`check_whole`] does, but with verify finding `listed`, a page whose checksum failure
/// is recorded, and nothing else.
fn check_whole_but(
    pager: &Pager<SimRuntime>,
    catalog: &Catalog,
    txns: &Transactions<SimRuntime>,
    listed: Option<Pid>,
) {
    let schema = catalog.schema();
    let got = committed(pager, catalog, txns);
    let migrations = rel(catalog, "catalog", "migrations");
    for (r, want) in declared(schema, catalog.source(), &catalog.texts) {
        if r == migrations {
            continue;
        }
        assert_eq!(
            sorted(&want),
            got[&r],
            "rows of {}",
            schema.relation(r).name
        );
    }
    let want: Vec<String> = applied(schema, catalog.source(), 0)
        .remove(&migrations)
        .unwrap_or_default()
        .iter()
        .map(|(c, row)| format!("{:?}", (c, &row[..3])))
        .collect();
    let version = Value::I64(i64::try_from(catalog.source().version).expect("a version"));
    let latest: Vec<(u32, Vec<Value>)> = migration_rows(pager, catalog, txns)
        .into_iter()
        .filter(|(_, row)| row[0] == version)
        .collect();
    assert_eq!(want, unapplied(&latest), "rows of migrations");
    let report = verify_catalog(pager, catalog, txns);
    let want: Vec<Finding> = listed
        .map(|pid| Finding {
            pid,
            problem: Problem::Damaged(Damage::Checksum),
        })
        .into_iter()
        .collect();
    assert_eq!(want, report.findings, "{report:#?}");
    assert!(report.log.is_empty() && report.complete, "{report:#?}");
    let commits: BTreeSet<u64> = report.commits.iter().copied().collect();
    assert_eq!(commits.len(), report.commits.len(), "{:?}", report.commits);
    assert_eq!(catalog.root(), pager.catalog_root());
}

/// Check that the catalog of `pager` opens, holds the rows its schema declares, and verifies
/// clean. Returns whether the header named a catalog before it opened, and the version the
/// catalog opened at.
fn check_reopened(rt: SimRuntime) -> (bool, u64) {
    check_reopened_but(rt, None)
}

/// Open as [`check_reopened`] does, checking as [`check_whole_but`] does with `listed`.
fn check_reopened_but(rt: SimRuntime, listed: Option<Pid>) -> (bool, u64) {
    let (pager, _) = Pager::open(rt.clone(), &options(FRAMES)).expect("reopen");
    let found = pager.catalog_root() != Pid(0);
    let (catalog, txns) = Catalog::open(rt, &pager).expect("the catalog opens");
    check_whole_but(&pager, &catalog, &txns, listed);
    (found, catalog.source().version)
}

/// Create a database and its catalog in a fresh simulation, recording what reaches the device.
/// Returns the simulation and the number of events recorded when the pager's create returned.
fn created(seed: u64) -> (Sim, usize) {
    let mut sim = Sim::new(seed, Config::default());
    sim.record();
    let mut start = 0;
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &options(FRAMES)).expect("create");
        start = rt.recorded();
        Catalog::create(rt, &pager).expect("the catalog");
    });
    (sim, start)
}

fn reopen(seed: u64, files: BTreeMap<String, Vec<u8>>) -> (bool, u64) {
    let mut sim = Sim::with_files(seed, crash_config(), files);
    let mut found = (false, 0);
    finished(&mut sim, |rt| found = check_reopened(rt));
    found
}

/// How many flushes of a create, evenly spread, the default profile crashes at; `deep` crashes
/// at every flush.
const CREATE_CRASHES: u64 = 8;

#[test]
fn a_crash_at_every_flush_during_create_reopens_to_a_whole_catalog() {
    let seed = 2;
    let (sim, start) = created(seed);
    let recording = sim.recording();
    let whole = recording.images(recording.len(), 1, seed).swap_remove(0);
    let (_, _, first) = reopened_commits(seed, whole);
    let [first] = first[..] else {
        panic!("one commit creates the catalog: {first:?}")
    };
    let flushes: Vec<usize> = recording
        .flushes()
        .into_iter()
        .filter(|&f| f >= start)
        .collect();
    assert!(flushes.len() > 4, "{} flushes", flushes.len());
    let flushes = if every_case() {
        flushes
    } else {
        spread(&flushes, CREATE_CRASHES)
    };
    let mut sides = BTreeSet::new();
    let mut assigned = 0;
    for at in flushes {
        let files = recording.images(at, 1, seed + at as u64).swap_remove(0);
        let (side, reserved, commits) = reopened_commits(seed + at as u64, files);
        sides.insert(side);
        if side == Side::Committed {
            assert_eq!(vec![first], commits, "a crash at {at}");
            continue;
        }
        let [x] = commits[..] else {
            panic!("the create redone after a crash at {at}, {side:?}, commits {commits:?}")
        };
        assert!(
            x >= reserved,
            "the create redone after a crash at {at} commits {x}, below the reserved {reserved}"
        );
        if reserved > first {
            assert!(
                x > first,
                "the create redone after a crash at {at} commits {x}"
            );
            assigned += 1;
        }
    }
    assert_eq!(
        BTreeSet::from([Side::Unnamed, Side::Uncommitted, Side::Committed]),
        sides
    );
    assert!(
        assigned > 0,
        "no crash before the commit point follows the create's begin"
    );
}

#[test]
fn a_crash_between_flushes_reopens_to_a_whole_catalog() {
    let seed = 3;
    let (sim, start) = created(seed);
    let recording = sim.recording();
    let n = recording.len();
    for at in (start..n).step_by((n - start).div_ceil(20)) {
        let files = recording.images(at, 1, seed + at as u64).swap_remove(0);
        reopen(seed + at as u64, files);
    }
}

fn rel(catalog: &Catalog, module: &str, name: &str) -> RelId {
    let schema = catalog.schema();
    let m = schema.module_named(module).expect("the module");
    match schema.module(m).values.get(name) {
        Some(adtb_lang::schema::ValueDef::Relation(r)) => *r,
        _ => panic!("no relation `{module}::{name}`"),
    }
}

/// The running schema, as the first migration of a database.
const RUNNING: &str = r"module auth
add module auth
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
add type Identity { id: Key identities, user: Key users }
  = EmailPass { email: Text, hash: Bytes }
  | Google    { gid: Text }
  | Apple     { sub: Text }
add relation identities : Identity
  key id generate uuidv7
  unique (email) in EmailPass
  unique (gid) in Google
  unique (sub) in Apple
add set OAuth of identities = Google | Apple
define fn display_name (i: Identity) -> Text = match i
  | EmailPass e -> e.email
  | Google g    -> g.gid
  | Apple a     -> a.sub
add constraint login_present =
  from i in identities
  where match i
    | EmailPass e -> scalar_len(e.email) == 0
    | Google g    -> scalar_len(g.gid) == 0
    | Apple a     -> scalar_len(a.sub) == 0
  select i
";

/// Create a database, its catalog and the running schema as its first migration in a fresh
/// simulation, recording what reaches the device. Returns the simulation and the number of
/// events recorded when the migration started.
fn migrated(seed: u64) -> (Sim, usize) {
    let mut sim = Sim::new(seed, Config::default());
    sim.record();
    let mut start = 0;
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &small_pool()).expect("create");
        let (mut catalog, txns) = Catalog::create(rt.clone(), &pager).expect("the catalog");
        start = rt.recorded();
        catalog
            .migrate(&pager, &txns, RUNNING)
            .expect("the migration applies");
        check_whole(&pager, &catalog, &txns);
    });
    (sim, start)
}

#[test]
fn a_crash_at_every_flush_during_the_first_migration_reopens_to_all_or_none_of_it() {
    let seed = 14;
    let (sim, start) = migrated(seed);
    let recording = sim.recording();
    let flushes: Vec<usize> = recording
        .flushes()
        .into_iter()
        .filter(|&f| f >= start)
        .collect();
    assert!(flushes.len() > 2, "{} flushes", flushes.len());
    let mut versions = Vec::new();
    for at in flushes {
        let files = recording.images(at, 1, seed + at as u64).swap_remove(0);
        let (found, version) = reopen(seed + at as u64, files);
        assert!(found);
        versions.push(version);
    }
    assert_eq!(Some(&0), versions.first());
    assert_eq!(Some(&1), versions.last());
    assert!(
        versions.is_sorted(),
        "the version goes back to 0 after a later flush: {versions:?}"
    );
}

#[test]
fn a_crash_between_flushes_of_the_first_migration_reopens_to_all_or_none_of_it() {
    let seed = 15;
    let (sim, start) = migrated(seed);
    let recording = sim.recording();
    let n = recording.len();
    let mut versions = BTreeSet::new();
    for at in (start..n).step_by((n - start).div_ceil(20)) {
        let files = recording.images(at, 1, seed + at as u64).swap_remove(0);
        versions.insert(reopen(seed + at as u64, files).1);
    }
    assert_eq!(BTreeSet::from([0, 1]), versions);
}

#[test]
fn a_migration_after_a_crashed_one_applies() {
    let seed = 16;
    let (sim, start) = migrated(seed);
    let recording = sim.recording();
    let n = recording.len();
    let step = (n - start).div_ceil(20);
    let mut applied = false;
    for at in (start..n).rev().step_by(step) {
        let files = recording.images(at, 1, seed).swap_remove(0);
        let mut sim = Sim::with_files(seed, Config::default(), files);
        finished(&mut sim, |rt| {
            let (pager, _) = Pager::open(rt.clone(), &options(FRAMES)).expect("reopen");
            let (mut catalog, txns) = Catalog::open(rt, &pager).expect("the catalog opens");
            if catalog.source().version == 0 {
                catalog
                    .migrate(&pager, &txns, RUNNING)
                    .expect("the migration applies");
                check_whole(&pager, &catalog, &txns);
                applied = true;
            }
        });
        if applied {
            finished(&mut sim, |rt| assert_eq!((true, 1), check_reopened(rt)));
            return;
        }
    }
    panic!("no crash point reopens before the migration");
}

/// Returns the files of `files` once recovered, with every page written and checkpointed.
fn recovered(seed: u64, files: BTreeMap<String, Vec<u8>>) -> BTreeMap<String, Vec<u8>> {
    let mut sim = Sim::with_files(seed, Config::default(), files);
    finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt, &options(FRAMES)).expect("reopen");
        pager.flush_log();
        pager.flush_pages(usize::MAX);
        pager.checkpoint();
    });
    sim.files()
}

/// Returns whether opening the catalog of `files` frees pages an unfinished migration
/// allocated, with the root of the key index of `catalog::constructors`, which open does not
/// read.
fn frees_a_migration(seed: u64, files: BTreeMap<String, Vec<u8>>) -> (bool, Pid) {
    let mut sim = Sim::with_files(seed, Config::default(), files);
    let mut out = None;
    finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &options(FRAMES)).expect("reopen");
        let before = pager.allocated().expect("the bitmaps").len();
        let (catalog, _) = Catalog::open(rt, &pager).expect("the catalog opens");
        let after = pager.allocated().expect("the bitmaps").len();
        let constructors = rel(&catalog, "catalog", "constructors");
        let key = catalog.plan().relations()[&constructors].key_index;
        let frees = catalog.source().version == 0 && after < before;
        out = Some((frees, catalog.index_of(key.0).tree().root()));
    });
    out.expect("the run finished")
}

#[test]
fn an_unfinished_migration_over_a_damaged_page_is_freed_once_the_page_reads() {
    let seed = 19;
    let (sim, start) = migrated(seed);
    let recording = sim.recording();
    let n = recording.len();
    for at in (start..n).rev().step_by((n - start).div_ceil(20)) {
        let files = recovered(seed, recording.images(at, 1, seed).swap_remove(0));
        let (frees, pid) = frees_a_migration(seed, files.clone());
        if !frees {
            continue;
        }
        let data = DataFile::new(harness::NAME, PAGE);
        let mut damaged = files.clone();
        data.flip(&mut damaged, pid, PAGE / 2 - HEADER_LEN);
        let mut sim = Sim::with_files(seed, Config::default(), damaged);
        let outcome = sim.run(|rt| {
            let (pager, _) = Pager::open(rt.clone(), &options(FRAMES)).expect("reopen");
            Catalog::open(rt, &pager).expect("the open leaves the sweep for a later open");
            pager.close();
        });
        assert_eq!(Outcome::Finished, outcome, "at {at}");
        let mut mended = sim.files();
        data.replace(&mut mended, pid, data.page(&files, pid));
        let mut sim = Sim::with_files(seed, Config::default(), mended);
        let reopened = |rt| assert_eq!((true, 0), check_reopened_but(rt, Some(pid)), "at {at}");
        let outcome = sim.run(reopened);
        assert_eq!(Outcome::Finished, outcome, "at {at}");
        return;
    }
    panic!("no crash point leaves a migration to free");
}

/// Where a crash during a create left the file, as its recovered header and root page say.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Side {
    /// The header names no catalog.
    Unnamed,
    /// The header names a root page that names no roots heap.
    Uncommitted,
    /// The root page names the roots heap.
    Committed,
}

/// Returns where the file `files` recovers to, the first id its commit log had not reserved before
/// the catalog opens, and the transaction of every commit record its log holds once it opens.
fn reopened_commits(seed: u64, files: BTreeMap<String, Vec<u8>>) -> (Side, u64, Vec<u64>) {
    let mut sim = Sim::with_files(seed, crash_config(), files);
    let mut out = (Side::Unnamed, Xid::FIRST.0, Vec::new());
    finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &options(FRAMES)).expect("reopen");
        let pid = pager.catalog_root();
        let (side, reserved) = if pid == Pid(0) {
            (Side::Unnamed, Xid::FIRST)
        } else {
            let root = Root::read(&pager.read(pid).expect("the root page"));
            let clog = CommitLog::load(&pager, root.commit_log).expect("the commit log");
            let side = if root.roots.1 == Pid(0) {
                Side::Uncommitted
            } else {
                Side::Committed
            };
            (side, clog.reserved())
        };
        let (catalog, txns) = Catalog::open(rt, &pager).expect("the catalog opens");
        check_whole(&pager, &catalog, &txns);
        out = (
            side,
            reserved.0,
            verify_catalog(&pager, &catalog, &txns).commits,
        );
    });
    out
}

#[test]
fn a_kept_row_written_into_a_full_roots_page_of_dead_versions_reuses_it() {
    let mut sim = Sim::new(4, Config::default());
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &options(FRAMES)).expect("create");
        let (catalog, txns) = Catalog::create(rt, &pager).expect("the catalog");
        let deployment = catalog.deployment();
        let roots = deployment.roots();
        let row = KeptRow {
            relation: 339,
            group: "database".into(),
            ctor: None,
            fields: vec![KeptField::Text("k".repeat(400))],
        };
        let keep = || {
            let begun = txns.begin(&pager).expect("a transaction");
            let x = begun.xid();
            catalog
                .replace_kept(
                    &pager,
                    &txns,
                    x,
                    (339, "database"),
                    std::slice::from_ref(&row),
                )
                .expect("the kept row is written");
            txns.commit(&pager, begun).expect("the commit").done();
        };
        keep();
        let pages = roots.pages(&pager).expect("the roots pages");
        let allocated = pager.allocated().expect("the allocated pages");
        for _ in 0..30 {
            keep();
        }
        assert_eq!(pages, roots.pages(&pager).expect("the roots pages"));
        assert_eq!(allocated, pager.allocated().expect("the allocated pages"));
    });
}

/// What refuses the database whole when its catalog opens.
#[derive(Clone, Copy, Debug)]
enum Refused {
    No,
    /// The database's mark, planted with no damage.
    Marked,
    /// A status page failing its checksum, which refuses every transaction and is not yet mapped
    /// to a mark.
    StatusPage,
}

const REFUSALS: [Refused; 3] = [Refused::No, Refused::Marked, Refused::StatusPage];

/// Returns a file holding a created catalog whose root page `change` changed, and a status page
/// of its commit log.
fn with_root(seed: u64, change: fn(&Pager<SimRuntime>, Pid)) -> (BTreeMap<String, Vec<u8>>, Pid) {
    let mut sim = Sim::new(seed, Config::default());
    let mut status = None;
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &options(FRAMES)).expect("create");
        let (catalog, txns) = Catalog::create(rt, &pager).expect("the catalog");
        status = txns.commit_log().status_pages().values().next().copied();
        change(&pager, catalog.root());
        pager.flush_log();
    });
    (sim.files(), status.expect("a status page"))
}

/// What an open of the catalog answered, with the root's change in progress and the marks after
/// it, and the pages allocated before and after it.
type RefusedCatalog = (
    Result<(), Error>,
    Option<Changing>,
    BTreeSet<Quarantine>,
    Vec<Pid>,
    Vec<Pid>,
);

/// Returns what [`Catalog::open`] of `files` answers when `refused` refuses the database whole,
/// with `status` a status page of its commit log.
fn refused_catalog(
    seed: u64,
    files: BTreeMap<String, Vec<u8>>,
    status: Pid,
    refused: Refused,
) -> RefusedCatalog {
    let mut sim = Sim::with_files(seed, Config::default(), files);
    let mut out = None;
    finished(&mut sim, |rt| {
        let (pager, opening) = Pager::open(rt.clone(), &options(FRAMES)).expect("reopen");
        match refused {
            Refused::No => {}
            Refused::Marked => pager
                .plant_mark(Quarantine::Database, &opening)
                .expect("the mark"),
            Refused::StatusPage => drop(pager.plant_damage(status, Damage::Checksum)),
        }
        let before = pager.allocated().expect("the bitmaps");
        let opened = Catalog::open(rt, &pager).map(drop);
        let root = pager.catalog_root();
        let changing = (root != Pid(0))
            .then(|| Root::read(&pager.read(root).expect("the root page")).changing());
        out = Some((
            opened,
            changing,
            pager.quarantined(),
            before,
            pager.allocated().expect("the bitmaps"),
        ));
    });
    out.expect("the run finished")
}

#[test]
fn an_open_refused_whole_by_a_mark_or_its_commit_log_leaves_a_change_in_progress() {
    let (files, status) = with_root(31, |pager, root| {
        let rebuild = root::begin(pager, root, Changing::Reindexing);
        drop(rebuild.expect("the rebuild begins"));
    });
    let got = REFUSALS.map(|refused| {
        let (opened, changing, marks, _, _) = refused_catalog(31, files.clone(), status, refused);
        (opened, changing, marks)
    });
    let database = || BTreeSet::from([Quarantine::Database]);
    let want = [
        (Ok(()), Some(Changing::None), BTreeSet::new()),
        (Ok(()), Some(Changing::Reindexing), database()),
        (Ok(()), Some(Changing::Reindexing), BTreeSet::new()),
    ];
    assert_eq!(want, got);
}

#[test]
fn an_open_refused_whole_of_a_catalog_whose_create_did_not_commit_fails_and_frees_nothing() {
    let (files, status) = with_root(32, |pager, root| {
        super::update_root(pager, root, |r| r.roots.1 = Pid(0)).expect("the root changes");
    });
    let got = REFUSALS.map(|refused| {
        let (opened, _, marks, before, after) = refused_catalog(32, files.clone(), status, refused);
        match opened {
            Ok(()) => Ok(()),
            Err(e) => Err((e, marks, before == after)),
        }
    });
    let refused = |marks| {
        Err((
            Error::Corrupt(super::rot::Rot::MarkedNoCatalog),
            marks,
            true,
        ))
    };
    let database = BTreeSet::from([Quarantine::Database]);
    assert_eq!([Ok(()), refused(database), refused(BTreeSet::new())], got);
}

#[test]
fn a_marked_header_naming_no_catalog_fails_and_frees_nothing() {
    let mut sim = Sim::new(33, Config::default());
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt, &options(FRAMES)).expect("create");
        crate::heap::LargeStore::create(&pager, 1).expect("a stray page");
        pager.flush_log();
    });
    let (opened, changing, _, before, after) =
        refused_catalog(33, sim.files(), Pid(0), Refused::Marked);
    let refused = Err(Error::Corrupt(super::rot::Rot::MarkedNoCatalog));
    assert_eq!((refused, None, before), (opened, changing, after));
}

/// Returns the files of a created catalog that crashed naming its page of counters, past the
/// allocation of the page, with that page and the root of the key index of
/// `catalog::constructors`, which open does not read.
fn crashed_naming(seed: u64) -> (BTreeMap<String, Vec<u8>>, Pid, Pid) {
    let mut sim = Sim::new(seed, Config::default());
    let mut made = None;
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &options(FRAMES)).expect("create");
        let (catalog, _txns) = Catalog::create(rt, &pager).expect("the catalog");
        let naming = Catalog::naming_sequences(&pager, catalog.root()).expect("the naming begins");
        drop(naming);
        let orphan = crate::write::full_counter_page(&pager, 0).expect("a page of counters");
        let constructors = rel(&catalog, "catalog", "constructors");
        let key = catalog.plan().relations()[&constructors].key_index;
        made = Some((orphan, catalog.index_of(key.0).tree().root()));
        pager.flush_log();
    });
    let (orphan, index) = made.expect("the run finished");
    (recovered(seed, sim.files()), orphan, index)
}

#[test]
fn naming_the_counters_over_a_crashed_naming_the_open_kept_leaves_its_page_to_the_next_open() {
    let seed = 34;
    let (files, orphan, index) = crashed_naming(seed);
    let data = DataFile::new(harness::NAME, PAGE);
    let mut damaged = files.clone();
    data.flip(&mut damaged, index, PAGE / 2 - HEADER_LEN);
    let mut sim = Sim::with_files(seed, Config::default(), damaged);
    let mut kept = None;
    finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &options(FRAMES)).expect("reopen");
        let (mut catalog, _txns) = Catalog::open(rt, &pager).expect("the catalog opens");
        let before = Root::read(&pager.read(catalog.root()).expect("the root"));
        let named = catalog.counters(&pager).expect("the counters are named");
        let after = Root::read(&pager.read(catalog.root()).expect("the root"));
        kept = Some((
            before.changing(),
            after.changing(),
            after.sequences == named,
            named,
        ));
        pager.close();
    });
    let mut mended = sim.files();
    data.replace(&mut mended, index, data.page(&files, index));
    let named = kept
        .map(|(_, _, _, named)| named)
        .expect("the first open ran");
    let mut sim = Sim::with_files(seed, Config::default(), mended);
    let mut freed = None;
    finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &options(FRAMES)).expect("reopen");
        let (catalog, _txns) = Catalog::open(rt, &pager).expect("the catalog opens");
        let root = Root::read(&pager.read(catalog.root()).expect("the root"));
        let allocated = pager.allocated().expect("the bitmaps");
        freed = Some((
            root.changing(),
            allocated.contains(&orphan),
            root.sequences == named,
            allocated.contains(&named),
        ));
    });
    assert_eq!(
        (
            Some((
                Changing::NamingSequences,
                Changing::NamingSequences,
                true,
                named
            )),
            Some((Changing::None, false, true, true))
        ),
        (kept, freed)
    );
}

#[test]
fn a_migration_over_a_kept_change_applies_and_frees_what_it_left() {
    let mut sim = Sim::new(35, Config::default());
    let mut got = None;
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &options(FRAMES)).expect("create");
        let (mut catalog, txns) = Catalog::create(rt, &pager).expect("the catalog");
        let naming = Catalog::naming_sequences(&pager, catalog.root()).expect("the naming begins");
        drop(naming);
        let orphan = crate::write::full_counter_page(&pager, 0).expect("a page of counters");
        let migrated = catalog.migrate(&pager, &txns, RUNNING).map(drop);
        let root = Root::read(&pager.read(catalog.root()).expect("the root"));
        let allocated = pager.allocated().expect("the bitmaps");
        got = Some((
            migrated,
            root.changing(),
            catalog.source().version,
            allocated.contains(&orphan),
        ));
    });
    assert_eq!(Some((Ok(()), Changing::None, 1, false)), got);
}

#[test]
fn a_replayed_log_folds_a_revert_with_a_fill_no_inverse_takes() {
    let mut sim = Sim::new(43, Config::default());
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &options(FRAMES)).expect("create");
        let (mut catalog, txns) = Catalog::create(rt, &pager).expect("catalog");
        let revert = "module app\nrevert 0002\n  fill T.A.x = \"q\"\n  fill T.A.x = \"r\"\n  fill T.B.y = \"s\"\n";
        for text in [
            "module app\nadd module app\nadd type T { id: Key ts } = A { x: Text } | B { y: Text }\nadd relation ts : T key id generate sequence\n",
            "module app\ndrop field T.A.x\n",
        ] {
            catalog.migrate(&pager, &txns, text).expect("apply");
        }
        assert_eq!(
            Err(Error::Refused(vec![
                "the revert fills `T.A.x` more than once".to_string()
            ])),
            catalog.migrate(&pager, &txns, revert)
        );
        assert_eq!(Ok(Vec::new()), catalog.replay(&pager, &txns, revert));
        check_whole(&pager, &catalog, &txns);
        assert_eq!(3, catalog.source().log.len());
    });
}

#[test]
fn a_rebuild_joins_only_a_change_no_running_transaction_holds() {
    let mut sim = Sim::new(409, Config::default());
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &options(FRAMES)).expect("create");
        let (catalog, txns) = Catalog::create(rt, &pager).expect("the catalog");
        let rebuild = |p: &Pager<SimRuntime>| catalog.begin_reindexing(p, &txns);
        let change = Change::begin(&pager, &txns, Scope::Every)
            .expect("a transaction")
            .mark(catalog.root)
            .expect("the change is marked");
        let reason = format!(
            "the schema change of transaction {} is in progress",
            change.writes().x.0
        );
        adtb_testkit::refused_as(
            rebuild(&pager),
            &Error::Refused(vec![reason.clone()]),
            &reason,
        );
        drop(change);
        assert!(
            rebuild(&pager).expect("the rebuild joins").is_none(),
            "a change no transaction runs is joined"
        );
        root::end_found(&pager, catalog.root).expect("the change ends");
        let naming = root::begin(&pager, catalog.root, Changing::NamingSequences).expect("marked");
        assert!(rebuild(&pager).expect("the rebuild joins").is_none());
        root::end(&pager, naming, |_| {}).expect("the naming ends");
        let first = rebuild(&pager).expect("the rebuild begins");
        let first = first.expect("an unmarked root takes the rebuild's mark");
        assert!(rebuild(&pager).expect("the rebuild joins").is_none());
        catalog
            .end_reindexing(&pager, first)
            .expect("the rebuild ends");
        drop(catalog);
        drop(txns);
        pager.close();
    });
}
