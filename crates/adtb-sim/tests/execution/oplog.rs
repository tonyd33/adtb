//! The operator log over the simulator: migrations after the first, their data maps, version
//! hashes, inverses and revert.

use adtb_simkit::api;
use adtb_simkit::harness;
use adtb_simkit::open;

use std::collections::BTreeSet;

use adtb::catalog::{self, Catalog, Error as CatalogError};
use adtb::mvcc::{Scope, Transactions};
use adtb::storage::Pager;
use adtb::verify::{self, Depth};
use adtb::{CallOptions, Database, Log, OpenError};
use adtb_sim::{Config, Sim, SimRuntime};

use adtb_simkit::grants;
use adtb_simkit::open::options;
use api::{query, run};
use harness::{finished, options as pager_options};

const SHOP: &str = r"module shop
add module shop
add type Customer = { id: Key customers, name: Text }
add relation customers : Customer key id generate sequence
add type Payment { id: Key payments, customer: Key customers }
  = Card { last4: Text }
  | Invoice { due: Text }
add relation payments : Payment
  key id generate sequence
  index (customer)
";

fn opened(rt: &SimRuntime, log: &[&str]) -> Database<SimRuntime> {
    open::open(rt, open::FILE, &Log::new(log.iter().copied()), &options(4))
        .unwrap_or_else(|e| panic!("the database opens at {}: {e:?}", log.len()))
}

fn write(db: &Database<SimRuntime>, text: &str) {
    db.query_text(text, &[], &CallOptions::default())
        .unwrap_or_else(|e| panic!("`{text}` fails: {e:?}"));
}

const CUSTOMERS: &str = r#"transact
  let c = insert customers { name: "ann" }
  insert payments.Card { customer: key c, last4: "4242" }
  insert customers { name: "bob" }"#;

/// Opens a database at `SHOP` with two customers and one card, then reopens it with `later`
/// appended to the log, and runs `body` over the result of that open.
fn after(seed: u64, later: &[&str], body: impl FnOnce(Result<Database<SimRuntime>, OpenError>)) {
    let mut sim = Sim::new(seed, Config::default());
    run(&mut sim, |rt| {
        let db = opened(&rt, &[SHOP]);
        write(&db, CUSTOMERS);
    });
    let mut log = vec![SHOP];
    log.extend_from_slice(later);
    run(&mut sim, |rt| {
        body(open::open(
            &rt,
            open::FILE,
            &Log::new(log.iter().copied()),
            &options(4),
        ));
    });
}

const TAGS: &str = "module shop\nadd type Label = { id: Key labels, name: Text }\nadd relation labels : Label key id generate sequence\n";

const FRAMES: usize = 64;

/// Applies `SHOP`, then inserts two customers and, recording, applies `second` through the
/// catalog. Returns the simulation and the number of events recorded when `second` started.
/// Returns a recorded simulation of `SHOP`, a role for each of `roles` and then `second`, and
/// the recorded write the second migration starts at.
fn second_migration(seed: u64, roles: &[&str], second: &str) -> (Sim, usize) {
    let mut sim = Sim::new(seed, Config::default());
    sim.record();
    let mut start = 0;
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &pager_options(FRAMES)).expect("create");
        let (mut catalog, txns) = Catalog::create(rt.clone(), &pager).expect("the catalog");
        catalog
            .migrate(&pager, &txns, SHOP)
            .expect("the first migration");
        for (id, role) in (2..).zip(roles) {
            grants::create_role(&catalog, &pager, &txns, id, role);
        }
        start = rt.recorded();
        catalog
            .migrate(&pager, &txns, second)
            .expect("the second migration applies");
        assert_clean(&pager, &catalog, &txns);
    });
    (sim, start)
}

fn assert_clean(pager: &Pager<SimRuntime>, catalog: &Catalog, txns: &Transactions<SimRuntime>) {
    let clog = txns.commit_log();
    let report = verify::verify_with(pager, &catalog.inventory(&clog), Depth::STRUCTURE);
    assert!(report.complete, "{:?}", report.findings);
    assert!(report.findings.is_empty(), "{:?}", report.findings);
}

/// Returns the version the catalog of `files` opens at, after checking it verifies clean.
fn reopened(seed: u64, files: std::collections::BTreeMap<String, Vec<u8>>) -> u64 {
    let mut sim = Sim::with_files(seed, Config::default(), files);
    let mut version = None;
    finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &pager_options(FRAMES)).expect("reopen");
        let (catalog, txns) = Catalog::open(rt, &pager).expect("the catalog opens");
        assert_clean(&pager, &catalog, &txns);
        version = Some(catalog.source().version);
    });
    version.expect("the catalog opened")
}

#[test]
fn a_crash_at_every_flush_of_a_second_migration_reopens_to_all_or_none_of_it() {
    let seed = 30;
    let rename = "module shop\nrename relation customers to clients\nadd index (name) in clients\n";
    let (sim, start) = second_migration(seed, &[], rename);
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
        versions.push(reopened(seed + at as u64, files));
    }
    assert_eq!(Some(&1), versions.first());
    assert_eq!(Some(&2), versions.last());
    assert!(
        versions.is_sorted(),
        "the version goes back after a later flush: {versions:?}"
    );
}

#[test]
fn a_crash_between_flushes_of_a_second_migration_reopens_to_all_or_none_of_it() {
    let seed = 31;
    let (sim, start) = second_migration(
        seed,
        &[],
        "module shop\ndrop index (customer) in payments\n",
    );
    let recording = sim.recording();
    let n = recording.len();
    let mut versions = BTreeSet::new();
    for at in (start..n).step_by((n - start).div_ceil(20)) {
        let files = recording.images(at, 1, seed + at as u64).swap_remove(0);
        versions.insert(reopened(seed + at as u64, files));
    }
    assert_eq!(BTreeSet::from([1, 2]), versions);
}

#[test]
fn the_migrations_rows_hold_the_chained_version_hash_of_each_version() {
    let mut sim = Sim::new(32, Config::default());
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &pager_options(FRAMES)).expect("create");
        let (mut catalog, txns) = Catalog::create(rt, &pager).expect("the catalog");
        let mut hashes = Vec::new();
        for text in [SHOP, TAGS, "module shop\nrename relation labels to tags\n"] {
            catalog
                .migrate(&pager, &txns, text)
                .expect("the migration applies");
            hashes.push(catalog::version_hash(catalog.schema(), catalog.source()));
        }
        let distinct: BTreeSet<_> = hashes.iter().collect();
        assert_eq!(3, distinct.len());
        let (id, snapshot) = txns.take(Scope::Every).expect("a snapshot");
        let m = catalog
            .schema()
            .module_named("catalog")
            .expect("the catalog module");
        let Some(&adtb_lang::schema::ValueDef::Relation(r)) =
            catalog.schema().module(m).values.get("migrations")
        else {
            panic!("catalog::migrations")
        };
        let rows = catalog.rows(&pager, &txns, &snapshot, r).expect("the rows");
        txns.release(id);
        let stored: BTreeSet<Vec<u8>> = rows
            .iter()
            .filter_map(|(_, row)| {
                row.iter().find_map(|v| match v {
                    adtb::key::Value::Bytes(b) if b.len() == 32 => Some(b.clone()),
                    _ => None,
                })
            })
            .collect();
        let expected: BTreeSet<Vec<u8>> = hashes.iter().map(|h| h.to_vec()).collect();
        assert_eq!(expected, stored);
    });
}

#[test]
fn a_migration_refused_by_its_data_changes_nothing_and_the_next_applies() {
    let mut sim = Sim::new(33, Config::default());
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &pager_options(FRAMES)).expect("create");
        let (mut catalog, txns) = Catalog::create(rt, &pager).expect("the catalog");
        catalog
            .migrate(&pager, &txns, SHOP)
            .expect("the first migration");
        let got = catalog.migrate(&pager, &txns, "module shop\ndrop relation nothing\n");
        assert!(matches!(got, Err(CatalogError::Refused(_))), "{got:?}");
        assert_eq!(1, catalog.source().version);
        assert_clean(&pager, &catalog, &txns);
        catalog
            .migrate(&pager, &txns, TAGS)
            .expect("the next migration applies");
        assert_eq!(2, catalog.source().version);
        assert_clean(&pager, &catalog, &txns);
    });
}

#[test]
fn a_crash_at_every_flush_of_a_split_reopens_to_the_split_with_every_row_routed() {
    let seed = 50;
    let split = r"module shop
split constructor Payment.Card into Visa { digits: Text } | Other { digits: Text }
  by match
    | Card { last4 } -> Visa { digits: last4 }
";
    let mut sim = Sim::new(seed, Config::default());
    sim.record();
    let mut start = 0;
    run(&mut sim, |rt| {
        let db = opened(&rt, &[SHOP]);
        write(&db, CUSTOMERS);
        db.close().expect("the database closes");
        start = rt.recorded();
        let db = opened(&rt, &[SHOP, split]);
        db.close().expect("the database closes");
    });
    let recording = sim.recording();
    let flushes: Vec<usize> = recording
        .flushes()
        .into_iter()
        .filter(|&f| f >= start)
        .collect();
    assert!(flushes.len() > 2, "{} flushes", flushes.len());
    for at in flushes {
        let files = recording.images(at, 1, seed + at as u64).swap_remove(0);
        let mut sim = Sim::with_files(seed + at as u64, Config::default(), files);
        run(&mut sim, |rt| {
            let db = opened(&rt, &[SHOP, split]);
            assert_eq!(
                vec![r#""4242""#],
                query(&db, "from p in payments.Visa select p.digits"),
                "after a crash at event {at}"
            );
            assert_eq!(
                2,
                query(&db, "from c in customers select c.name").len(),
                "after a crash at event {at}"
            );
        });
    }
}

#[test]
fn a_crash_at_every_flush_of_a_revert_with_a_fill_reopens_to_the_revert_with_its_fill() {
    let seed = 69;
    let drop_field = "module shop\ndrop field Payment.Card.last4\n";
    let revert = "module shop\nrevert 0002\n  fill Payment.Card.last4 = \"0000\"\n";
    let mut sim = Sim::new(seed, Config::default());
    sim.record();
    let mut start = 0;
    run(&mut sim, |rt| {
        let db = opened(&rt, &[SHOP]);
        write(&db, CUSTOMERS);
        db.close().expect("the database closes");
        let db = opened(&rt, &[SHOP, drop_field]);
        db.close().expect("the database closes");
        start = rt.recorded();
        let db = opened(&rt, &[SHOP, drop_field, revert]);
        db.close().expect("the database closes");
    });
    let recording = sim.recording();
    let flushes: Vec<usize> = recording
        .flushes()
        .into_iter()
        .filter(|&f| f >= start)
        .collect();
    assert!(flushes.len() > 2, "{} flushes", flushes.len());
    for at in flushes {
        let files = recording.images(at, 1, seed + at as u64).swap_remove(0);
        let mut sim = Sim::with_files(seed + at as u64, Config::default(), files);
        run(&mut sim, |rt| {
            let db = opened(&rt, &[SHOP, drop_field, revert]);
            assert_eq!(
                vec![r#""0000""#],
                query(&db, "from p in payments.Card select p.last4"),
                "after a crash at event {at}"
            );
        });
    }
}

#[test]
#[ignore = "the engine does not run a script yet"]
fn a_script_writes_on_the_state_the_operators_before_it_produce() {
    let script = "module shop
add field Customer.vip: Bool fill False
script
  update c in customers where c.name == \"ann\" set { c with vip: True }
";
    after(90, &[script], |db| {
        let db = db.expect("the script applies");
        assert_eq!(
            vec![
                r#"{ name: "ann", vip: True }"#,
                r#"{ name: "bob", vip: False }"#
            ],
            query(
                &db,
                "from c in customers select { name: c.name, vip: c.vip }"
            )
        );
    });
}

#[test]
fn a_crash_at_every_flush_of_a_migration_of_derived_operators_reopens_to_all_or_none_of_it() {
    let seed = 91;
    let derived = "module shop
add default customers.name = \"anon\"
add computed customers.shout: Text = self.name ++ \"!\"
define fn named () -> Set { id: Key customers, name: Text } =
  from c in customers select { id: c.id, name: c.name }
store named key id
grant read customers to support
";
    let (sim, start) = second_migration(seed, &["support"], derived);
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
        versions.push(reopened(seed + at as u64, files));
    }
    assert_eq!(Some(&1), versions.first());
    assert_eq!(Some(&2), versions.last());
    assert!(
        versions.is_sorted(),
        "the version goes back after a later flush: {versions:?}"
    );
}
