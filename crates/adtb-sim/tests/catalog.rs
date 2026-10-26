use std::collections::{BTreeMap, BTreeSet};

use adtb::catalog::{Catalog, declared, engine_schema};
use adtb::key::{CtorId, Value};
use adtb::mvcc::Transactions;
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

/// Check that the catalog of `pager` opens, holds the rows the engine declares, and verifies
/// clean. Returns whether the header named a catalog before it opened.
fn check_reopened(rt: SimRuntime) -> bool {
    let (pager, _) = Pager::open(rt.clone(), &options()).expect("reopen");
    let found = pager.catalog_root() != Pid(0);
    let (catalog, txns) = Catalog::open(rt, &pager).expect("the catalog opens");
    let schema = engine_schema();
    for (r, want) in declared(&schema) {
        let got = catalog
            .committed_rows(&pager, &txns, r)
            .expect("the rows read");
        assert_eq!(
            sorted(&want),
            sorted(&got),
            "rows of {}",
            schema.relation(r).name
        );
    }
    let report = verify_catalog(&pager, &catalog, &txns);
    assert!(report.is_clean() && report.complete, "{report:#?}");
    assert_eq!(catalog.root(), pager.catalog_root());
    found
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

fn reopen(seed: u64, files: BTreeMap<String, Vec<u8>>) -> bool {
    let mut sim = Sim::with_files(seed, crash_config(), files);
    let mut found = false;
    let outcome = sim.run(|rt| found = check_reopened(rt));
    assert_eq!(Outcome::Finished, outcome);
    found
}

#[test]
fn a_created_catalog_reopens_with_the_rows_the_engine_declares() {
    let (sim, _) = created(1);
    let recording = sim.recording();
    let files = recording.images(recording.len(), 1, 1).swap_remove(0);
    assert!(reopen(1, files));
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
        found.insert(reopen(seed + at as u64, files));
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
        let pager = Pager::create(rt.clone(), &options()).expect("create");
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
    let outcome = sim.run(|rt| assert!(check_reopened(rt)));
    assert_eq!(Outcome::Finished, outcome);
}
