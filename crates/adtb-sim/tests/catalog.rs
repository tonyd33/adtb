use std::collections::{BTreeMap, BTreeSet};

use adtb::catalog::{Catalog, Error, applied, declared, version_hash};
use adtb::key::{CtorId, Value};
use adtb::mvcc::{Scope, Transactions};
use adtb::plan::{Element, IndexId};
use adtb::storage::{Options, Pager, Pid};
use adtb::verify::{self, Report};
use adtb_lang::types::RelId;
use adtb_sim::{Config, Faults, Outcome, Sim, SimRuntime};

#[path = "support/harness.rs"]
mod harness;

use harness::{PAGE, finished, options};

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
            ..Faults::default()
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
    verify::verify(pager, &catalog.inventory(&clog))
}

/// Returns the rows of every relation of `catalog` that every committed transaction wrote.
fn committed(
    pager: &Pager<SimRuntime>,
    catalog: &Catalog,
    txns: &Transactions<SimRuntime>,
) -> BTreeMap<RelId, Vec<String>> {
    let (id, snapshot) = txns.take(Scope::Every).expect("a snapshot");
    let out = catalog
        .schema()
        .ids
        .relations
        .keys()
        .map(|&r| {
            let rows = catalog
                .rows(pager, txns, &snapshot, r)
                .expect("the rows read");
            (r, sorted(&rows))
        })
        .collect();
    txns.release(id);
    out
}

/// Returns the rows of `catalog::migrations` every committed transaction wrote.
fn migration_rows(
    pager: &Pager<SimRuntime>,
    catalog: &Catalog,
    txns: &Transactions<SimRuntime>,
) -> Vec<(u32, Vec<Value>)> {
    let (id, snapshot) = txns.take(Scope::Every).expect("a snapshot");
    let rows = catalog
        .rows(
            pager,
            txns,
            &snapshot,
            rel(catalog, "catalog", "migrations"),
        )
        .expect("the rows read");
    txns.release(id);
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
    let schema = catalog.schema();
    let got = committed(pager, catalog, txns);
    let migrations = rel(catalog, "catalog", "migrations");
    for (r, want) in declared(schema, catalog.source()) {
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
    assert!(report.is_clean() && report.complete, "{report:#?}");
    let commits: BTreeSet<u64> = report.commits.iter().copied().collect();
    assert_eq!(commits.len(), report.commits.len(), "{:?}", report.commits);
    assert_eq!(catalog.root(), pager.catalog_root());
}

/// Check that the catalog of `pager` opens, holds the rows its schema declares, and verifies
/// clean. Returns whether the header named a catalog before it opened, and the version the
/// catalog opened at.
fn check_reopened(rt: SimRuntime) -> (bool, u64) {
    let (pager, _) = Pager::open(rt.clone(), &options(FRAMES)).expect("reopen");
    let found = pager.catalog_root() != Pid(0);
    let (catalog, txns) = Catalog::open(rt, &pager).expect("the catalog opens");
    check_whole(&pager, &catalog, &txns);
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

#[test]
fn a_created_catalog_reopens_with_the_rows_the_engine_declares() {
    let (sim, _) = created(1);
    let recording = sim.recording();
    let files = recording.images(recording.len(), 1, 1).swap_remove(0);
    assert_eq!((true, 0), reopen(1, files));
}

#[test]
fn appended_log_rows_and_hashes_survive_reopen_refusal_and_revert() {
    let mut sim = Sim::new(41, Config::default());
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &options(FRAMES)).expect("create");
        let (mut catalog, txns) = Catalog::create(rt, &pager).expect("catalog");
        for text in [
            "module app\nadd module app\nadd type Row = { id: I64 }\nadd relation rows : Row key id\n",
            "module app\nadd field Row.x: I64 fill 7\n",
            "module app\nrename field Row.x to value\n",
        ] {
            catalog.migrate(&pager, &txns, text).expect("apply");
            check_whole(&pager, &catalog, &txns);
        }
    });
    finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &options(FRAMES)).expect("reopen");
        let (mut catalog, txns) = Catalog::open(rt, &pager).expect("catalog");
        let before = committed(&pager, &catalog, &txns);
        let hashes = migration_rows(&pager, &catalog, &txns);
        let refused = catalog.migrate(&pager, &txns, "module app\ndrop relation missing\n");
        assert!(matches!(refused, Err(Error::Refused(_))), "{refused:?}");
        assert_eq!(before, committed(&pager, &catalog, &txns));
        catalog
            .migrate(&pager, &txns, "module app\nrevert 0003\n")
            .expect("revert");
        check_whole(&pager, &catalog, &txns);
        let after = committed(&pager, &catalog, &txns);
        for name in ["migration_ops", "migration_clauses"] {
            let r = rel(&catalog, "catalog", name);
            for row in &before[&r] {
                assert!(after[&r].contains(row), "{name}: {row}");
            }
        }
        let stored = migration_rows(&pager, &catalog, &txns);
        assert_eq!(hashes.len() + 1, stored.len());
        for row in hashes {
            assert!(stored.contains(&row), "changed migration: {row:?}");
        }
    });
    finished(&mut sim, |rt| {
        assert_eq!((true, 4), check_reopened(rt));
    });
}

#[test]
fn a_crash_at_every_flush_during_create_reopens_to_a_whole_catalog() {
    let seed = 2;
    let (sim, start) = created(seed);
    let recording = sim.recording();
    let whole = recording.images(recording.len(), 1, seed).swap_remove(0);
    let (_, first) = reopened_commits(seed, whole);
    let [first] = first[..] else {
        panic!("one commit creates the catalog: {first:?}")
    };
    let flushes: Vec<usize> = recording
        .flushes()
        .into_iter()
        .filter(|&f| f >= start)
        .collect();
    assert!(flushes.len() > 4, "{} flushes", flushes.len());
    let mut found = BTreeSet::new();
    let mut redone = Vec::new();
    for at in flushes {
        let files = recording.images(at, 1, seed + at as u64).swap_remove(0);
        let (named, commits) = reopened_commits(seed + at as u64, files);
        found.insert(named);
        if named && commits != [first] {
            redone.push((at, commits));
        }
    }
    assert_eq!(BTreeSet::from([false, true]), found);
    assert!(
        !redone.is_empty(),
        "no crash point leaves a named catalog whose create commits under a new id"
    );
    for (at, commits) in redone {
        assert!(
            commits.iter().all(|&x| x > first),
            "the create redone after a crash at {at} commits {commits:?}, not after {first}"
        );
    }
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

/// Run `body` over a catalog created and then reopened from its durable state.
fn with_reopened(
    seed: u64,
    body: impl FnOnce(&Pager<SimRuntime>, &Catalog, &Transactions<SimRuntime>),
) {
    let (sim, _) = created(seed);
    let recording = sim.recording();
    let files = recording.images(recording.len(), 1, seed).swap_remove(0);
    let mut sim = Sim::with_files(seed, Config::default(), files);
    finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &options(FRAMES)).expect("reopen");
        let (catalog, txns) = Catalog::open(rt, &pager).expect("the catalog opens");
        body(&pager, &catalog, &txns);
    });
}

fn rel(catalog: &Catalog, module: &str, name: &str) -> RelId {
    let schema = catalog.schema();
    let m = schema.module_named(module).expect("the module");
    match schema.module(m).values.get(name) {
        Some(adtb_lang::schema::ValueDef::Relation(r)) => *r,
        _ => panic!("no relation `{module}::{name}`"),
    }
}

#[test]
fn the_planner_catalog_describes_every_engine_relation() {
    with_reopened(4, |_, catalog, _| {
        let plan = catalog.plan();
        let schema = catalog.schema();
        assert_eq!(schema.ids.relations.len(), plan.relations.len());
        let mut heaps = BTreeSet::new();
        let mut indexes = BTreeSet::new();
        for (r, d) in &plan.relations {
            let n = schema.ty(schema.relation(*r).elem_def).ctors().len();
            assert_eq!(n, d.heaps.len());
            assert!(d.indexes.iter().any(|i| i.id == d.key_index && i.unique));
            heaps.extend(d.heaps.values().copied());
            indexes.extend(d.indexes.iter().map(|i| i.id));
        }
        assert_eq!(
            plan.relations
                .values()
                .map(|d| d.heaps.len())
                .sum::<usize>(),
            heaps.len(),
            "heap ids are distinct"
        );
        assert_eq!((0..61).map(IndexId).collect::<BTreeSet<_>>(), indexes);
        let roles = &plan.relations[&rel(catalog, "catalog", "roles")];
        let name = roles
            .indexes
            .iter()
            .find(|i| i.fields() == Some(vec!["name"]))
            .expect("the unique of roles");
        assert!(name.unique);
        assert_eq!(vec![0], name.ctors);
        let settings = &plan.relations[&rel(catalog, "catalog", "settings")];
        assert_eq!(20, settings.heaps.len());
    });
}

#[test]
fn the_key_index_of_constructors_finds_each_constructor() {
    with_reopened(5, |pager, catalog, _| {
        let constructors = rel(catalog, "catalog", "constructors");
        let key = catalog.plan().relations[&constructors].key_index;
        let index = catalog.index_of(key.0);
        let entries = index
            .probe(pager, &[Value::I64(121)])
            .expect("the probe reads");
        assert_eq!(1, entries.len(), "{entries:?}");
        assert_eq!(CtorId(121), entries[0].ctor);
        let row = catalog
            .heap(constructors, 0)
            .read(pager, entries[0].tid)
            .expect("the read")
            .expect("a version");
        assert_eq!(Value::Text("Constructor".into()), row.row[2]);
        let all = index
            .range(
                pager,
                std::ops::Bound::Unbounded,
                std::ops::Bound::Unbounded,
            )
            .expect("the scan");
        assert_eq!(catalog.schema().ids.ctors.len(), all.len());
    });
}

#[test]
fn a_reopened_catalog_names_the_same_structures() {
    let (sim, _) = created(6);
    let recording = sim.recording();
    let files = recording.images(recording.len(), 1, 6).swap_remove(0);
    let mut sim = Sim::with_files(6, Config::default(), files);
    let mut seen = Vec::new();
    for _ in 0..2 {
        finished(&mut sim, |rt| {
            let (pager, _) = Pager::open(rt.clone(), &options(FRAMES)).expect("reopen");
            let (catalog, _) = Catalog::open(rt, &pager).expect("the catalog opens");
            seen.push((catalog.root(), format!("{:?}", catalog.plan())));
            pager.checkpoint();
        });
    }
    assert_eq!(seen[0], seen[1]);
}

#[test]
fn a_header_naming_no_catalog_is_created_again() {
    let mut sim = Sim::new(7, Config::default());
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt, &options(FRAMES)).expect("create");
        adtb::heap::LargeStore::create(&pager, 1).expect("a stray page");
        pager.flush_log();
    });
    finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &options(FRAMES)).expect("reopen");
        assert_eq!(Pid(0), pager.catalog_root());
        drop(Catalog::open(rt, &pager).expect("the catalog is created"));
        assert_ne!(Pid(0), pager.catalog_root());
    });
    finished(&mut sim, |rt| assert!(check_reopened(rt).0));
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

/// Run `body` over a catalog migrated to the running schema and then reopened from its durable
/// state.
fn with_migrated(
    seed: u64,
    body: impl FnOnce(&Pager<SimRuntime>, &Catalog, &Transactions<SimRuntime>),
) {
    let (sim, _) = migrated(seed);
    let recording = sim.recording();
    let files = recording.images(recording.len(), 1, seed).swap_remove(0);
    let mut sim = Sim::with_files(seed, Config::default(), files);
    finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &options(FRAMES)).expect("reopen");
        let (catalog, txns) = Catalog::open(rt, &pager).expect("the catalog opens");
        body(&pager, &catalog, &txns);
    });
}

#[test]
fn a_migrated_catalog_reopens_with_the_running_schema() {
    with_migrated(11, |pager, catalog, txns| {
        assert_eq!(1, catalog.source().version);
        check_whole(pager, catalog, txns);
        let plan = catalog.plan();
        assert_eq!(catalog.schema().ids.relations.len(), plan.relations.len());
        let users = &plan.relations[&rel(catalog, "auth", "users")];
        assert_eq!(1, users.heaps.len());
        assert_eq!(1, users.indexes.len());
        let identities = &plan.relations[&rel(catalog, "auth", "identities")];
        assert_eq!(
            vec![0, 1, 2],
            identities.heaps.keys().copied().collect::<Vec<_>>()
        );
        let uniques: BTreeSet<(Vec<Element>, Vec<u32>)> = identities
            .indexes
            .iter()
            .filter(|i| i.id != identities.key_index)
            .map(|i| (i.elements.clone(), i.ctors.clone()))
            .collect();
        let field = |f: &str| vec![Element::Field(f.to_string())];
        assert_eq!(
            BTreeSet::from([
                (field("email"), vec![0]),
                (field("gid"), vec![1]),
                (field("sub"), vec![2]),
                (field("user"), vec![0, 1, 2]),
            ]),
            uniques
        );
        let key = identities
            .indexes
            .iter()
            .find(|i| i.id == identities.key_index)
            .expect("the key index");
        assert_eq!(
            (field("id"), vec![0, 1, 2], true),
            (key.elements.clone(), key.ctors.clone(), key.unique)
        );
        let mut heaps = BTreeSet::new();
        for d in plan.relations.values() {
            heaps.extend(d.heaps.values().copied());
        }
        let count: usize = plan.relations.values().map(|d| d.heaps.len()).sum();
        assert_eq!(count, heaps.len(), "heap ids are distinct");
    });
}

#[test]
fn a_migrated_catalog_names_its_objects() {
    with_migrated(12, |pager, catalog, txns| {
        let rows = committed(pager, catalog, txns);
        let has = |rel_name: &str, text: &str| {
            let r = rel(catalog, "catalog", rel_name);
            rows[&r].iter().any(|row| row.contains(text))
        };
        assert!(has("types", "Text(\"Identity\")"));
        assert!(has("constructors", "Text(\"Apple\")"));
        assert!(has("fields", "Text(\"gid\")"));
        assert!(has("relations", "Text(\"identities\")"));
        assert!(has("indexes", "Text(\"unique (sub) in Apple\")"));
        assert!(has("named_sets", "Text(\"OAuth\")"));
        assert!(has("constraints", "Text(\"login_present\")"));
        assert!(has("defaults", "Text(\"generate sequence\")"));
        assert!(has("defaults", "Text(\"generate uuidv7\")"));
    });
}

#[test]
fn a_migrated_catalog_reopens_to_the_same_structures() {
    let (sim, _) = migrated(13);
    let recording = sim.recording();
    let files = recording.images(recording.len(), 1, 13).swap_remove(0);
    let mut sim = Sim::with_files(13, Config::default(), files);
    let mut seen = Vec::new();
    for _ in 0..2 {
        finished(&mut sim, |rt| {
            let (pager, _) = Pager::open(rt.clone(), &options(FRAMES)).expect("reopen");
            let (catalog, _) = Catalog::open(rt, &pager).expect("the catalog opens");
            seen.push((
                catalog.order_keys().clone(),
                format!("{:?}", catalog.plan()),
            ));
            pager.checkpoint();
        });
    }
    assert_eq!(seen[0], seen[1]);
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

#[test]
fn a_migration_that_does_not_apply_to_the_schema_is_refused_and_a_later_one_applies() {
    let mut sim = Sim::new(17, Config::default());
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &options(FRAMES)).expect("create");
        let (mut catalog, txns) = Catalog::create(rt, &pager).expect("the catalog");
        let refused = |r: Result<(), Error>| match r {
            Err(Error::Refused(why)) => why,
            other => panic!("not refused: {other:?}"),
        };
        let why =
            refused(catalog.migrate(&pager, &txns, "module auth\nadd type T = { id: I64 }\n"));
        assert_eq!(
            vec!["no module `auth`; a migration of a new module starts with `add module auth`"],
            why
        );
        assert_eq!(0, catalog.source().version);
        catalog
            .migrate(&pager, &txns, RUNNING)
            .expect("the migration applies");
        catalog
            .migrate(&pager, &txns, "module auth\nadd type T = { id: I64 }\n")
            .expect("a later migration applies");
        assert_eq!(2, catalog.source().version);
        check_whole(&pager, &catalog, &txns);
    });
}

#[test]
fn a_migrated_catalog_reopens_with_its_log_rows_and_version_hash() {
    with_migrated(18, |pager, catalog, txns| {
        let schema = catalog.schema();
        let rows = migration_rows(pager, catalog, txns);
        let [(_, row)] = rows.as_slice() else {
            panic!("one migrations row: {rows:?}")
        };
        assert_eq!(Value::I64(1), row[0]);
        let hash = version_hash(schema, catalog.source());
        assert_ne!([0; 32], hash);
        assert_eq!(Value::Bytes(hash.to_vec()), row[1]);
        assert_eq!(Value::Text(RUNNING.into()), row[2]);
        let got = committed(pager, catalog, txns);
        let ops = &got[&rel(catalog, "catalog", "migration_ops")];
        assert_eq!(8, ops.len(), "{ops:?}");
        let clauses = &got[&rel(catalog, "catalog", "migration_clauses")];
        assert_eq!(2, clauses.len(), "the two members of OAuth: {clauses:?}");
        let functions = &got[&rel(catalog, "catalog", "functions")];
        assert_eq!(1, functions.len(), "{functions:?}");
        assert!(
            functions[0].contains("Text(\"display_name\")"),
            "{functions:?}"
        );
    });
}

#[test]
fn the_log_rows_survive_a_crash_after_the_migration_commits() {
    let (sim, _) = migrated(19);
    let recording = sim.recording();
    for (k, files) in recording
        .images(recording.len(), 4, 19)
        .into_iter()
        .enumerate()
    {
        let mut sim = Sim::with_files(19, Config::default(), files);
        let outcome = sim.run(|rt| {
            let (pager, _) = Pager::open(rt.clone(), &options(FRAMES)).expect("reopen");
            let (catalog, txns) = Catalog::open(rt, &pager).expect("the catalog opens");
            assert_eq!(1, catalog.source().version, "image {k}");
            check_whole(&pager, &catalog, &txns);
            assert_eq!(
                1,
                migration_rows(&pager, &catalog, &txns).len(),
                "image {k}"
            );
        });
        assert_eq!(Outcome::Finished, outcome, "image {k}");
    }
}

#[test]
fn a_function_naming_a_wasm_module_no_migration_installs_is_refused() {
    let mut sim = Sim::new(20, Config::default());
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &options(FRAMES)).expect("create");
        let (mut catalog, txns) = Catalog::create(rt, &pager).expect("the catalog");
        let text =
            "module app\nadd module app\ndefine fn f (x: I64) -> I64 = wasm \"m\" export \"f\"\n";
        match catalog.migrate(&pager, &txns, text) {
            Err(Error::Refused(why)) => assert_eq!(
                vec!["function `f` names Wasm module `m`, which no migration installs"],
                why
            ),
            other => panic!("not refused: {other:?}"),
        }
        assert_eq!(0, catalog.source().version);
        check_whole(&pager, &catalog, &txns);
    });
}

/// Returns `files` with page `pid` of the data file damaged.
fn damage(mut files: BTreeMap<String, Vec<u8>>, pid: Pid) -> BTreeMap<String, Vec<u8>> {
    let data = files.get_mut("app.adt").expect("the data file");
    data[pid.0 as usize * PAGE + PAGE / 2] ^= 0xff;
    files
}

/// Returns `files` with page `pid` of the data file as `from` holds it.
fn restore(
    mut files: BTreeMap<String, Vec<u8>>,
    from: &BTreeMap<String, Vec<u8>>,
    pid: Pid,
) -> BTreeMap<String, Vec<u8>> {
    let at = pid.0 as usize * PAGE..(pid.0 as usize + 1) * PAGE;
    let data = files.get_mut("app.adt").expect("the data file");
    data[at.clone()].copy_from_slice(&from["app.adt"][at]);
    files
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
        let key = catalog.plan().relations[&constructors].key_index;
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
        let mut sim = Sim::with_files(seed, Config::default(), damage(files.clone(), pid));
        let outcome = sim.run(|rt| {
            let (pager, _) = Pager::open(rt.clone(), &options(FRAMES)).expect("reopen");
            let e = Catalog::open(rt, &pager).expect_err("the migration's pages are not found");
            let Error::Page(e) = e else {
                panic!("at {at}: the open failed by {e:?}")
            };
            assert_eq!(pid, e.pid, "at {at}");
        });
        assert_eq!(Outcome::Finished, outcome, "at {at}");
        let mended = restore(sim.files(), &files, pid);
        let mut sim = Sim::with_files(seed, Config::default(), mended);
        let outcome = sim.run(|rt| assert_eq!((true, 0), check_reopened(rt), "at {at}"));
        assert_eq!(Outcome::Finished, outcome, "at {at}");
        return;
    }
    panic!("no crash point leaves a migration to free");
}

/// The kind of an implied index as a row of `catalog::indexes` shows it.
const IMPLIED: &str = "Sum { ctor: 4, fields: [] }, Text(\"index (";

/// Returns the rows of `catalog::indexes` over `auth::identities` after `migration` applies as
/// the first migration of a fresh database.
fn identity_indexes(seed: u64, migration: &str) -> Vec<String> {
    let mut sim = Sim::new(seed, Config::default());
    let mut out = Vec::new();
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &small_pool()).expect("create");
        let (mut catalog, txns) = Catalog::create(rt, &pager).expect("the catalog");
        catalog
            .migrate(&pager, &txns, migration)
            .expect("the migration applies");
        let rows = committed(&pager, &catalog, &txns);
        let identities = catalog.schema().ids.relations[&rel(&catalog, "auth", "identities")];
        let relation = format!("I64({identities}), ");
        out = rows[&rel(&catalog, "catalog", "indexes")]
            .iter()
            .filter(|row| row.contains(&relation))
            .cloned()
            .collect();
    });
    out
}

#[test]
fn a_reference_field_with_no_declared_index_has_an_implied_one_listed_in_the_catalog() {
    let rows = identity_indexes(31, RUNNING);
    let implied: Vec<&String> = rows
        .iter()
        .filter(|r| r.contains("Text(\"index (user)\")"))
        .collect();
    assert_eq!(1, implied.len(), "{rows:?}");
    assert!(implied[0].contains(IMPLIED), "{implied:?}");
    with_migrated(32, |_, catalog, _| {
        let plan = catalog.plan();
        let identities = &plan.relations[&rel(catalog, "auth", "identities")];
        assert!(
            identities
                .indexes
                .iter()
                .any(|i| i.fields() == Some(vec!["user"]) && i.ctors == [0, 1, 2] && !i.unique),
            "{:?}",
            identities.indexes
        );
    });
}

#[test]
fn a_declared_index_leading_with_a_reference_field_replaces_its_implied_one() {
    let declared = RUNNING.replacen(
        "  unique (sub) in Apple\n",
        "  unique (sub) in Apple\n  index (user, id)\n",
        1,
    );
    let rows = identity_indexes(33, &declared);
    assert!(rows.iter().all(|r| !r.contains(IMPLIED)), "{rows:?}");
    assert!(
        rows.iter()
            .any(|r| r.contains("Text(\"index (user, id)\")")),
        "{rows:?}"
    );
}

#[test]
fn a_declared_index_over_fewer_constructors_than_hold_the_field_keeps_the_implied_one() {
    let declared = RUNNING.replacen(
        "  unique (sub) in Apple\n",
        "  unique (sub) in Apple\n  index (user) in Google\n",
        1,
    );
    let rows = identity_indexes(34, &declared);
    assert_eq!(
        1,
        rows.iter().filter(|r| r.contains(IMPLIED)).count(),
        "{rows:?}"
    );
}

/// Returns whether the header of the file `files` named a catalog, and the transaction of every
/// commit record its log holds once the catalog opens.
fn reopened_commits(seed: u64, files: BTreeMap<String, Vec<u8>>) -> (bool, Vec<u64>) {
    let mut sim = Sim::with_files(seed, crash_config(), files);
    let mut out = (false, Vec::new());
    finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &options(FRAMES)).expect("reopen");
        let found = pager.catalog_root() != Pid(0);
        let (catalog, txns) = Catalog::open(rt, &pager).expect("the catalog opens");
        check_whole(&pager, &catalog, &txns);
        out = (found, verify_catalog(&pager, &catalog, &txns).commits);
    });
    out
}

#[test]
fn a_migration_after_a_reopen_gives_its_new_heaps_ids_no_heap_had() {
    let (sim, _) = migrated(42);
    let recording = sim.recording();
    let files = recording.images(recording.len(), 1, 42).swap_remove(0);
    let mut sim = Sim::with_files(42, Config::default(), files);
    finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &options(FRAMES)).expect("reopen");
        let (mut catalog, txns) = Catalog::open(rt, &pager).expect("the catalog opens");
        let heaps = |c: &Catalog| -> Vec<u64> {
            let plan = c.plan();
            plan.relations
                .values()
                .flat_map(|d| d.heaps.values().map(|h| h.0))
                .collect()
        };
        let before: BTreeSet<u64> = heaps(&catalog).into_iter().collect();
        catalog
            .migrate(
                &pager,
                &txns,
                "module auth\nadd type Label = { id: Key tags, label: Text }\n\
                 add relation tags : Label key id generate sequence\n",
            )
            .expect("the migration applies");
        let tags = &catalog.plan().relations[&rel(&catalog, "auth", "tags")];
        let new: Vec<u64> = tags.heaps.values().map(|h| h.0).collect();
        assert!(
            new.iter().all(|h| !before.contains(h)),
            "tags is stored in {new:?}, among {before:?}"
        );
        let after = heaps(&catalog);
        let distinct: BTreeSet<u64> = after.iter().copied().collect();
        assert_eq!(
            after.len(),
            distinct.len(),
            "heap ids are distinct: {after:?}"
        );
    });
}

#[test]
fn a_snapshot_taken_before_a_migration_reads_the_catalog_rows_of_the_version_before() {
    let mut sim = Sim::new(43, Config::default());
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &small_pool()).expect("create");
        let (mut catalog, txns) = Catalog::create(rt, &pager).expect("the catalog");
        let relations = rel(&catalog, "catalog", "relations");
        let (id, before) = txns.take(Scope::Every).expect("a snapshot");
        catalog
            .migrate(&pager, &txns, RUNNING)
            .expect("the migration applies");
        let names = |snapshot| {
            let rows = catalog
                .rows(&pager, &txns, snapshot, relations)
                .expect("the rows read");
            sorted(&rows)
                .into_iter()
                .filter(|r| r.contains("Text(\"identities\")"))
                .count()
        };
        assert_eq!(0, names(&before), "the snapshot before sees no identities");
        txns.release(id);
        let (id, after) = txns.take(Scope::Every).expect("a snapshot");
        assert_eq!(1, names(&after), "a snapshot after sees identities");
        txns.release(id);
    });
}
