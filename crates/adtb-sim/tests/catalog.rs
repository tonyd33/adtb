use std::collections::{BTreeMap, BTreeSet};

use adtb::catalog::{Catalog, Error, applied, declared, version_hash};
use adtb::key::{CtorId, Value};
use adtb::mvcc::{Scope, Transactions};
use adtb::plan::IndexId;
use adtb::storage::{Options, Pager, Pid};
use adtb::verify::{self, Report};
use adtb_lang::types::RelId;
use adtb_sim::{Config, Faults, Outcome, Sim, SimRuntime};

const PAGE: usize = 4096;

fn options() -> Options {
    Options {
        page_size: PAGE as u32,
        log_block: 512,
        segment_size: 256 * 1024,
        pool_frames: 64,
        atomic_page_writes: false,
        ..Options::new("app.adt")
    }
}

/// Returns the options of a pager whose pool is small enough that a migration's pages reach the
/// device before it commits.
fn small_pool() -> Options {
    Options {
        pool_frames: 16,
        ..options()
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

/// Check that the catalog of `pager` holds the rows its schema declares, that no transaction
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
    assert_eq!(
        want,
        unapplied(&migration_rows(pager, catalog, txns)),
        "rows of migrations"
    );
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
    let (pager, _) = Pager::open(rt.clone(), &options()).expect("reopen");
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
    let outcome = sim.run(|rt| {
        let pager = Pager::create(rt.clone(), &options()).expect("create");
        start = rt.recorded();
        Catalog::create(rt, &pager).expect("the catalog");
    });
    assert_eq!(Outcome::Finished, outcome);
    (sim, start)
}

fn reopen(seed: u64, files: BTreeMap<String, Vec<u8>>) -> (bool, u64) {
    let mut sim = Sim::with_files(seed, crash_config(), files);
    let mut found = (false, 0);
    let outcome = sim.run(|rt| found = check_reopened(rt));
    assert_eq!(Outcome::Finished, outcome);
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
fn a_crash_at_every_flush_during_create_reopens_to_a_whole_catalog() {
    let seed = 2;
    let (sim, start) = created(seed);
    let recording = sim.recording();
    let flushes: Vec<usize> = recording
        .flushes()
        .into_iter()
        .filter(|&f| f >= start)
        .collect();
    assert!(flushes.len() > 4, "{} flushes", flushes.len());
    let mut found = BTreeSet::new();
    for at in flushes {
        let files = recording.images(at, 1, seed + at as u64).swap_remove(0);
        found.insert(reopen(seed + at as u64, files).0);
    }
    assert_eq!(BTreeSet::from([false, true]), found);
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
    let outcome = sim.run(|rt| {
        let (pager, _) = Pager::open(rt.clone(), &options()).expect("reopen");
        let (catalog, txns) = Catalog::open(rt, &pager).expect("the catalog opens");
        body(&pager, &catalog, &txns);
    });
    assert_eq!(Outcome::Finished, outcome);
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
            .find(|i| i.fields == ["name"])
            .expect("the unique of roles");
        assert!(name.unique);
        assert_eq!(vec![0], name.ctors);
        let settings = &plan.relations[&rel(catalog, "catalog", "settings")];
        assert_eq!(19, settings.heaps.len());
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
        let outcome = sim.run(|rt| {
            let (pager, _) = Pager::open(rt.clone(), &options()).expect("reopen");
            let (catalog, _) = Catalog::open(rt, &pager).expect("the catalog opens");
            seen.push((catalog.root(), format!("{:?}", catalog.plan())));
            pager.checkpoint();
        });
        assert_eq!(Outcome::Finished, outcome);
    }
    assert_eq!(seen[0], seen[1]);
}

#[test]
fn a_header_naming_no_catalog_is_created_again() {
    let mut sim = Sim::new(7, Config::default());
    let outcome = sim.run(|rt| {
        let pager = Pager::create(rt, &options()).expect("create");
        adtb::heap::LargeStore::create(&pager, 1).expect("a stray page");
        pager.flush_log();
    });
    assert_eq!(Outcome::Finished, outcome);
    let outcome = sim.run(|rt| {
        let (pager, _) = Pager::open(rt.clone(), &options()).expect("reopen");
        assert_eq!(Pid(0), pager.catalog_root());
        drop(Catalog::open(rt, &pager).expect("the catalog is created"));
        assert_ne!(Pid(0), pager.catalog_root());
    });
    assert_eq!(Outcome::Finished, outcome);
    let outcome = sim.run(|rt| assert!(check_reopened(rt).0));
    assert_eq!(Outcome::Finished, outcome);
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
    let outcome = sim.run(|rt| {
        let pager = Pager::create(rt.clone(), &small_pool()).expect("create");
        let (mut catalog, txns) = Catalog::create(rt.clone(), &pager).expect("the catalog");
        start = rt.recorded();
        catalog
            .migrate(&pager, &txns, RUNNING)
            .expect("the migration applies");
        check_whole(&pager, &catalog, &txns);
    });
    assert_eq!(Outcome::Finished, outcome);
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
    let outcome = sim.run(|rt| {
        let (pager, _) = Pager::open(rt.clone(), &options()).expect("reopen");
        let (catalog, txns) = Catalog::open(rt, &pager).expect("the catalog opens");
        body(&pager, &catalog, &txns);
    });
    assert_eq!(Outcome::Finished, outcome);
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
        let uniques: BTreeSet<(Vec<String>, Vec<u32>)> = identities
            .indexes
            .iter()
            .filter(|i| i.id != identities.key_index)
            .map(|i| (i.fields.clone(), i.ctors.clone()))
            .collect();
        let field = |f: &str| vec![f.to_string()];
        assert_eq!(
            BTreeSet::from([
                (field("email"), vec![0]),
                (field("gid"), vec![1]),
                (field("sub"), vec![2]),
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
            (key.fields.clone(), key.ctors.clone(), key.unique)
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
        let outcome = sim.run(|rt| {
            let (pager, _) = Pager::open(rt.clone(), &options()).expect("reopen");
            let (catalog, _) = Catalog::open(rt, &pager).expect("the catalog opens");
            seen.push((
                catalog.order_keys().clone(),
                format!("{:?}", catalog.plan()),
            ));
            pager.checkpoint();
        });
        assert_eq!(Outcome::Finished, outcome);
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
        let outcome = sim.run(|rt| {
            let (pager, _) = Pager::open(rt.clone(), &options()).expect("reopen");
            let (mut catalog, txns) = Catalog::open(rt, &pager).expect("the catalog opens");
            if catalog.source().version == 0 {
                catalog
                    .migrate(&pager, &txns, RUNNING)
                    .expect("the migration applies");
                check_whole(&pager, &catalog, &txns);
                applied = true;
            }
        });
        assert_eq!(Outcome::Finished, outcome);
        if applied {
            let outcome = sim.run(|rt| assert_eq!((true, 1), check_reopened(rt)));
            assert_eq!(Outcome::Finished, outcome);
            return;
        }
    }
    panic!("no crash point reopens before the migration");
}

#[test]
fn only_a_first_migration_of_declarations_applies() {
    let mut sim = Sim::new(17, Config::default());
    let outcome = sim.run(|rt| {
        let pager = Pager::create(rt.clone(), &options()).expect("create");
        let (mut catalog, txns) = Catalog::create(rt, &pager).expect("the catalog");
        let refused = |r: Result<(), Error>| match r {
            Err(Error::Refused(why)) => why,
            other => panic!("not refused: {other:?}"),
        };
        let why = refused(catalog.migrate(&pager, &txns, "module auth\ndrop module auth\n"));
        assert_eq!(1, why.len(), "{why:?}");
        let why =
            refused(catalog.migrate(&pager, &txns, "module auth\nadd type T = { id: I64 }\n"));
        assert!(!why.is_empty());
        assert_eq!(0, catalog.source().version);
        catalog
            .migrate(&pager, &txns, RUNNING)
            .expect("the migration applies");
        let why =
            refused(catalog.migrate(&pager, &txns, "module auth\nadd type T = { id: I64 }\n"));
        assert_eq!(1, why.len(), "{why:?}");
        assert_eq!(1, catalog.source().version);
        check_whole(&pager, &catalog, &txns);
    });
    assert_eq!(Outcome::Finished, outcome);
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
            let (pager, _) = Pager::open(rt.clone(), &options()).expect("reopen");
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
    let outcome = sim.run(|rt| {
        let pager = Pager::create(rt.clone(), &options()).expect("create");
        let (mut catalog, txns) = Catalog::create(rt, &pager).expect("the catalog");
        let text =
            "module app\nadd module app\ndefine fn f (x: I64) -> I64 = wasm \"m\" export \"f\"\n";
        match catalog.migrate(&pager, &txns, text) {
            Err(Error::Refused(why)) => assert_eq!(1, why.len(), "{why:?}"),
            other => panic!("not refused: {other:?}"),
        }
        assert_eq!(0, catalog.source().version);
        check_whole(&pager, &catalog, &txns);
    });
    assert_eq!(Outcome::Finished, outcome);
}
