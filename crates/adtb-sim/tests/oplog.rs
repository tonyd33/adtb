//! The operator log over the simulator: migrations after the first, their data maps, version
//! hashes, inverses and revert.

#[path = "support/harness.rs"]
mod harness;

use std::collections::BTreeSet;

use adtb::catalog::{self, Catalog, Error as CatalogError};
use adtb::mvcc::{Scope, Transactions};
use adtb::storage::Pager;
use adtb::verify::{self, Depth};
use adtb::{CallOptions, Database, Error, Log, OpenError, OpenOptions};
use adtb::{CreateOptions, RejectReason};
use adtb_lang::textual::print;
use adtb_sim::{Config, Outcome, Sim, SimRuntime};

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

fn options() -> OpenOptions {
    OpenOptions {
        create: Some(CreateOptions::default()),
        workers: Some(4),
        ..OpenOptions::default()
    }
}

fn run<T>(sim: &mut Sim, body: impl FnOnce(SimRuntime) -> T) -> T {
    let mut got = None;
    let outcome = sim.run(|rt| got = Some(body(rt)));
    assert_eq!(Outcome::Finished, outcome);
    got.expect("the run returns")
}

fn open(rt: &SimRuntime, log: &[&str]) -> Result<Database<SimRuntime>, OpenError> {
    Database::open_in(
        rt.clone(),
        "app.adt",
        &Log::new(log.iter().copied()),
        &options(),
    )
}

fn opened(rt: &SimRuntime, log: &[&str]) -> Database<SimRuntime> {
    open(rt, log).unwrap_or_else(|e| panic!("the database opens at {}: {e:?}", log.len()))
}

/// Returns each row of the one result of `text` run in `db`, printed and sorted.
fn query(db: &Database<SimRuntime>, text: &str) -> Vec<String> {
    let mut out = db
        .query_text(text, &[], &CallOptions::default())
        .unwrap_or_else(|e| panic!("`{text}` fails: {e:?}"));
    assert_eq!(1, out.len(), "`{text}` has one result");
    let rows = out.remove(0);
    let form = rows.form().clone();
    let mut got: Vec<String> = rows
        .map(|r| print(&r.expect("a row"), &form).expect("a row prints"))
        .collect();
    got.sort();
    got
}

fn write(db: &Database<SimRuntime>, text: &str) {
    db.query_text(text, &[], &CallOptions::default())
        .unwrap_or_else(|e| panic!("`{text}` fails: {e:?}"));
}

fn fails(db: &Database<SimRuntime>, text: &str) -> Error {
    match db.query_text(text, &[], &CallOptions::default()) {
        Ok(rows) => panic!("`{text}` succeeds with {} results", rows.len()),
        Err(e) => e,
    }
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
    run(&mut sim, |rt| body(open(&rt, &log)));
}

fn refusal(got: Result<Database<SimRuntime>, OpenError>) -> (u64, String) {
    match got {
        Err(OpenError::MigrationRefused {
            version,
            reason:
                Error::Rejected {
                    reason: RejectReason::SchemaRefused { detail, .. },
                },
        }) => (version, detail),
        Err(other) => panic!("not a refused migration: {other:?}"),
        Ok(_) => panic!("the migration applies"),
    }
}

const TAGS: &str = "module shop\nadd type Label = { id: Key labels, name: Text }\nadd relation labels : Label key id generate sequence\n";

#[test]
fn a_second_migration_declaring_a_relation_applies_at_open() {
    after(1, &[TAGS], |db| {
        let db = db.expect("the second migration applies");
        assert!(query(&db, "from t in labels select t.name").is_empty());
        assert_eq!(2, query(&db, "from c in customers select c.name").len());
    });
}

#[test]
fn migrations_apply_in_order_each_producing_a_version_that_survives_reopening() {
    let mut sim = Sim::new(2, Config::default());
    let rename = "module shop\nrename relation customers to clients\n";
    run(&mut sim, |rt| {
        let db = opened(&rt, &[SHOP]);
        write(&db, CUSTOMERS);
    });
    run(&mut sim, |rt| {
        let db = opened(&rt, &[SHOP, TAGS, rename]);
        write(&db, r#"insert labels { name: "x" }"#);
    });
    run(&mut sim, |rt| {
        let db = opened(&rt, &[SHOP, TAGS, rename]);
        assert_eq!(
            vec![r#""ann""#, r#""bob""#],
            query(&db, "from c in clients select c.name")
        );
        assert_eq!(vec![r#""x""#], query(&db, "from t in labels select t.name"));
    });
}

#[test]
fn a_refused_migration_stops_the_set_and_keeps_the_versions_before_it() {
    let mut sim = Sim::new(3, Config::default());
    let bad = "module shop\ndrop relation nothing\n";
    run(&mut sim, |rt| drop(opened(&rt, &[SHOP])));
    run(&mut sim, |rt| {
        let (version, _) = refusal(open(&rt, &[SHOP, TAGS, bad, TAGS]));
        assert_eq!(3, version);
    });
    run(&mut sim, |rt| {
        let db = opened(&rt, &[SHOP, TAGS]);
        assert!(query(&db, "from t in labels select t.name").is_empty());
    });
}

#[test]
fn a_shipped_migration_differing_only_in_layout_has_the_same_hash() {
    let mut sim = Sim::new(4, Config::default());
    run(&mut sim, |rt| drop(opened(&rt, &[SHOP, TAGS])));
    let spaced = "module shop\n\nadd type Label = { id: Key labels,   name: Text }\nadd relation labels : Label   key id generate sequence\n";
    run(&mut sim, |rt| drop(opened(&rt, &[SHOP, spaced])));
    let other = "module shop\nadd type Label = { id: Key labels, title: Text }\nadd relation labels : Label key id generate sequence\n";
    run(&mut sim, |rt| match open(&rt, &[SHOP, other]) {
        Err(OpenError::Diverged { version }) => assert_eq!(2, version),
        got => panic!("a different migration 2 does not diverge: {:?}", got.err()),
    });
}

#[test]
fn renames_keep_rows_and_values() {
    let renames = "module shop
rename relation customers to clients
rename field Customer.name to full_name
rename type Customer to Client
rename constructor Payment.Card to Debit
";
    after(5, &[renames], |db| {
        let db = db.expect("the renames apply");
        assert_eq!(
            vec![r#""ann""#, r#""bob""#],
            query(&db, "from c in clients select c.full_name")
        );
        assert_eq!(
            vec![r#""4242""#],
            query(&db, "from p in payments.Debit select p.last4")
        );
    });
}

#[test]
fn a_dropped_field_loses_its_values_and_keeps_its_rows() {
    after(6, &["module shop\ndrop field Payment.Card.last4\n"], |db| {
        let db = db.expect("the drop applies");
        assert_eq!(
            1,
            query(&db, "from p in payments.Card select p.customer").len()
        );
        let got = fails(&db, "from p in payments.Card select p.last4");
        assert!(matches!(got, Error::Rejected { .. }), "{got:?}");
    });
}

#[test]
fn an_added_field_takes_its_fill_in_every_existing_row() {
    after(
        7,
        &["module shop\nadd field Customer.vip: Bool fill false\n"],
        |db| {
            let db = db.expect("the add applies");
            assert_eq!(
                vec!["false", "false"],
                query(&db, "from c in customers select c.vip")
            );
            write(&db, r#"insert customers { name: "cy", vip: true }"#);
            assert_eq!(
                vec!["false", "false", "true"],
                query(&db, "from c in customers select c.vip")
            );
        },
    );
}

#[test]
fn an_added_field_fill_may_match_on_the_row() {
    let add = r"module shop
add field Payment.note: Text fill match
  | Card c    -> c.last4
  | Invoice i -> i.due
";
    after(8, &[add], |db| {
        let db = db.expect("the add applies");
        assert_eq!(
            vec![r#""4242""#],
            query(&db, "from p in payments select p.note")
        );
    });
}

#[test]
fn an_added_constructor_takes_rows_once_enabled() {
    let add = "module shop\nadd constructor Payment.Wire { iban: Text }\n";
    let enable = "module shop\nenable payments.Wire\n";
    let insert = r#"transact
  let c = insert customers { name: "cy" }
  insert payments.Wire { customer: key c, iban: "DE00" }"#;
    let mut sim = Sim::new(9, Config::default());
    run(&mut sim, |rt| {
        let db = opened(&rt, &[SHOP]);
        write(&db, CUSTOMERS);
    });
    run(&mut sim, |rt| {
        let db = opened(&rt, &[SHOP, add]);
        let got = fails(&db, insert);
        assert!(
            matches!(
                got,
                Error::Rejected {
                    reason: RejectReason::NotOpen { .. }
                }
            ),
            "an insert into a declared constructor is not refused: {got:?}"
        );
    });
    run(&mut sim, |rt| {
        let db = opened(&rt, &[SHOP, add, enable]);
        write(&db, insert);
        assert_eq!(
            vec![r#""DE00""#],
            query(&db, "from p in payments.Wire select p.iban")
        );
    });
}

#[test]
fn removing_a_constructor_that_holds_rows_is_refused() {
    let deprecate = "module shop\ndeprecate payments.Card\n";
    let remove = "module shop\nremove constructor Payment.Card\n";
    after(10, &[deprecate, remove], |db| {
        let (version, detail) = refusal(db);
        assert_eq!(3, version);
        assert!(detail.contains("payments.Card"), "{detail}");
    });
}

#[test]
fn removing_an_empty_deprecated_constructor_applies() {
    let deprecate = "module shop\ndeprecate payments.Invoice\n";
    let remove = "module shop\nremove constructor Payment.Invoice\n";
    after(11, &[deprecate, remove], |db| {
        let db = db.expect("the remove applies");
        assert_eq!(1, query(&db, "from p in payments select p.customer").len());
    });
}

#[test]
fn dropping_a_relation_that_holds_rows_is_refused_and_an_empty_one_drops() {
    let drop_payments = "module shop\ndrop relation payments\ndrop type Payment\n";
    after(12, &[drop_payments], |db| {
        let (version, detail) = refusal(db);
        assert_eq!(2, version);
        assert!(detail.contains("payments"), "{detail}");
    });
    let drop_labels = "module shop\ndrop relation labels\ndrop type Label\n";
    after(13, &[TAGS, drop_labels], |db| {
        let db = db.expect("dropping an empty relation applies");
        assert!(matches!(
            fails(&db, "from t in labels select t.name"),
            Error::Rejected { .. }
        ));
    });
}

#[test]
fn disabling_a_constructor_that_holds_rows_is_refused() {
    let add = "module shop\nadd constructor Payment.Wire { iban: Text }\nenable payments.Wire\n";
    let disable_card = "module shop\ndisable payments.Card\n";
    after(14, &[add, disable_card], |db| {
        let (version, detail) = refusal(db);
        assert_eq!(3, version);
        assert!(detail.contains("payments.Card"), "{detail}");
    });
    let disable_wire = "module shop\ndisable payments.Wire\n";
    after(15, &[add, disable_wire], |db| {
        db.expect("disabling an empty constructor applies");
    });
}

#[test]
fn an_added_unique_that_existing_rows_break_is_refused() {
    let dup = r#"insert customers { name: "ann" }"#;
    let mut sim = Sim::new(16, Config::default());
    run(&mut sim, |rt| {
        let db = opened(&rt, &[SHOP]);
        write(&db, CUSTOMERS);
        write(&db, dup);
    });
    let unique = "module shop\nadd unique (name) in customers\n";
    run(&mut sim, |rt| {
        let (version, detail) = refusal(open(&rt, &[SHOP, unique]));
        assert_eq!(2, version);
        assert!(detail.contains("customers"), "{detail}");
    });
    run(&mut sim, |rt| {
        let db = opened(&rt, &[SHOP]);
        assert_eq!(3, query(&db, "from c in customers select c").len());
    });
}

#[test]
fn an_added_unique_holds_for_later_writes() {
    let unique = "module shop\nadd unique (name) in customers\n";
    after(17, &[unique], |db| {
        let db = db.expect("the unique applies");
        let got = fails(&db, r#"insert customers { name: "ann" }"#);
        assert!(matches!(got, Error::Rejected { .. }), "{got:?}");
    });
}

#[test]
fn an_added_constraint_existing_rows_violate_is_refused() {
    let paid = "module shop
add constraint paid =
  from c in customers
  where not exists p in payments where p.customer == key c
  select c
";
    after(18, &[paid], |db| {
        let (version, detail) = refusal(db);
        assert_eq!(2, version);
        assert!(detail.contains("paid"), "{detail}");
    });
}

#[test]
fn an_index_drop_and_add_leave_a_file_that_verifies_clean() {
    let drop_index = "module shop\ndrop index (customer) in payments\n";
    let add_index = "module shop\nadd index (customer) in payments\n";
    after(19, &[drop_index, add_index], |db| {
        let db = db.expect("the index changes apply");
        assert_eq!(
            vec![r#""4242""#],
            query(
                &db,
                r#"from c in customers, p in payments.Card where c.name == "ann" and p.customer == key c select p.last4"#
            )
        );
        db.close().expect("the database closes");
    });
}

#[test]
fn a_split_routes_each_row_to_its_new_constructor() {
    let split = r"module shop
split constructor Payment.Card into Visa { digits: Text } | Other { digits: Text }
  by match
    | Card { last4 } -> Visa { digits: last4 }
";
    after(20, &[split], |db| {
        let db = db.expect("the split applies");
        assert_eq!(
            vec![r#""4242""#],
            query(&db, "from p in payments.Visa select p.digits")
        );
        assert!(query(&db, "from p in payments.Other select p.digits").is_empty());
    });
}

#[test]
fn a_merge_routes_every_source_row_into_the_merged_constructor() {
    let merge = r"module shop
add type Method = Card | Invoice
merge constructor Payment.(Card | Invoice) into Any { method: Method, detail: Text }
  by match
    | Card c    -> Any { method: Card, detail: c.last4 }
    | Invoice i -> Any { method: Invoice, detail: i.due }
";
    after(21, &[merge], |db| {
        let db = db.expect("the merge applies");
        assert_eq!(
            vec![r#""4242""#],
            query(&db, "from p in payments.Any select p.detail")
        );
    });
}

#[test]
fn reverting_renames_restores_the_names_and_keeps_the_rows() {
    let rename = "module shop\nrename relation customers to clients\nrename field Customer.name to full_name\n";
    let revert = "module shop\nrevert 0002\n";
    after(22, &[rename, revert], |db| {
        let db = db.expect("the revert applies");
        assert_eq!(
            vec![r#""ann""#, r#""bob""#],
            query(&db, "from c in customers select c.name")
        );
    });
}

#[test]
fn reverting_a_dropped_field_takes_the_fill_the_revert_states() {
    let drop_field = "module shop\ndrop field Payment.Card.last4\n";
    let revert = "module shop\nrevert 0002\n  fill Payment.Card.last4 = \"0000\"\n";
    after(23, &[drop_field, revert], |db| {
        let db = db.expect("the revert applies");
        assert_eq!(
            vec![r#""0000""#],
            query(&db, "from p in payments.Card select p.last4")
        );
    });
}

#[test]
fn reverting_an_added_constructor_removes_it() {
    let add = "module shop\nadd constructor Payment.Wire { iban: Text }\n";
    let revert = "module shop\nrevert 0002\n";
    after(24, &[add, revert], |db| {
        let db = db.expect("the revert applies");
        let got = fails(&db, "from p in payments.Wire select p.iban");
        assert!(matches!(got, Error::Rejected { .. }), "{got:?}");
    });
}

const FRAMES: usize = 64;

/// Applies `SHOP`, then inserts two customers and, recording, applies `second` through the
/// catalog. Returns the simulation and the number of events recorded when `second` started.
fn second_migration(seed: u64, second: &str) -> (Sim, usize) {
    let mut sim = Sim::new(seed, Config::default());
    sim.record();
    let mut start = 0;
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &pager_options(FRAMES)).expect("create");
        let (mut catalog, txns) = Catalog::create(rt.clone(), &pager).expect("the catalog");
        catalog
            .migrate(&pager, &txns, SHOP)
            .expect("the first migration");
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
    let (sim, start) = second_migration(seed, rename);
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
    let (sim, start) = second_migration(seed, "module shop\ndrop index (customer) in payments\n");
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
