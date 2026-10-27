//! The operator log over the simulator: migrations after the first, their data maps, version
//! hashes, inverses and revert.

#[expect(dead_code, reason = "this binary uses part of the API helpers")]
#[path = "support/api.rs"]
mod api;
#[path = "support/harness.rs"]
mod harness;

use std::collections::BTreeSet;

use adtb::RejectReason;
use adtb::catalog::{self, Catalog, Error as CatalogError};
use adtb::mvcc::{Scope, Transactions};
use adtb::storage::Pager;
use adtb::verify::{self, Depth};
use adtb::{CallOptions, Database, Error, Log, OpenError, OpenOptions};
use adtb_sim::{Config, Sim, SimRuntime};

use api::{options, result, run, sorted};
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

fn open(rt: &SimRuntime, log: &[&str]) -> Result<Database<SimRuntime>, OpenError> {
    Database::open_in(
        rt.clone(),
        "app.adt",
        &Log::new(log.iter().copied()),
        &options(4),
    )
}

fn opened(rt: &SimRuntime, log: &[&str]) -> Database<SimRuntime> {
    open(rt, log).unwrap_or_else(|e| panic!("the database opens at {}: {e:?}", log.len()))
}

/// Returns each row of the one result of `text` run in `db`, printed and sorted.
fn query(db: &Database<SimRuntime>, text: &str) -> Vec<String> {
    sorted(result(db, text))
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
fn a_shipped_migration_that_does_not_resolve_diverges() {
    let mut sim = Sim::new(53, Config::default());
    run(&mut sim, |rt| drop(opened(&rt, &[SHOP, TAGS])));
    let adopted = "module shop\nadopt tzdb \"2026b\" from \"tz-2026b.tzif\"\n";
    run(&mut sim, |rt| match open(&rt, &[SHOP, adopted]) {
        Err(OpenError::Diverged { version }) => assert_eq!(2, version),
        got => panic!(
            "a migration 2 that does not resolve does not diverge: {:?}",
            got.err()
        ),
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
            let vips = "from c in customers select { name: c.name, vip: c.vip }";
            assert_eq!(
                vec![
                    r#"{ name: "ann", vip: False }"#,
                    r#"{ name: "bob", vip: False }"#
                ],
                query(&db, vips)
            );
            write(&db, r#"insert customers { name: "cy", vip: true }"#);
            assert_eq!(
                vec![
                    r#"{ name: "ann", vip: False }"#,
                    r#"{ name: "bob", vip: False }"#,
                    r#"{ name: "cy", vip: True }"#
                ],
                query(&db, vips)
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
fn removing_a_constructor_that_holds_rows_with_a_redefined_function_is_refused() {
    let define = "module shop\ndefine fn method (p: Payment) -> Text = match p\n  | Card c -> c.last4\n  | Invoice i -> i.due\n";
    let deprecate = "module shop\ndeprecate payments.Card\n";
    let remove = "module shop\nremove constructor Payment.Card\nredefine fn method (p: Payment) -> Text = match p\n  | Invoice i -> i.due\n";
    after(54, &[define, deprecate, remove], |db| {
        let (version, detail) = refusal(db);
        assert_eq!(4, version);
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

const INVOICE: &str = r#"transact
  let c = insert customers { name: "cy" }
  insert payments.Invoice { customer: key c, due: "2027-01" }"#;

/// Opens a database at `SHOP` with `CUSTOMERS` and `INVOICE`, then reopens it with `later`
/// appended to the log, and runs `body` over the result of that open.
fn after_invoice(
    seed: u64,
    later: &[&str],
    body: impl FnOnce(Result<Database<SimRuntime>, OpenError>),
) {
    let mut sim = Sim::new(seed, Config::default());
    run(&mut sim, |rt| {
        let db = opened(&rt, &[SHOP]);
        write(&db, CUSTOMERS);
        write(&db, INVOICE);
    });
    let mut log = vec![SHOP];
    log.extend_from_slice(later);
    run(&mut sim, |rt| body(open(&rt, &log)));
}

const NEST: &str = r"module shop
add type Method = Card { last4: Text } | Invoice { due: Text }
nest constructor Payment.(Card | Invoice) into Held { method: Method }
";

#[test]
fn a_nest_holds_each_row_as_a_value_of_the_nested_sum() {
    after_invoice(40, &[NEST], |db| {
        let db = db.expect("the nest applies");
        assert_eq!(
            vec![r#"Card { last4: "4242" }"#, r#"Invoice { due: "2027-01" }"#],
            query(&db, "from p in payments.Held select p.method")
        );
        assert_eq!(2, query(&db, "from p in payments select p.id").len());
    });
}

#[test]
fn an_unnest_lifts_each_held_value_to_its_constructor_keeping_the_key() {
    let unnest = "module shop\nunnest constructor Payment.Held\n";
    after_invoice(41, &[NEST, unnest], |db| {
        let db = db.expect("the unnest applies");
        assert_eq!(
            vec![r#""4242""#],
            query(&db, "from p in payments.Card select p.last4")
        );
        assert_eq!(
            vec![r#""2027-01""#],
            query(&db, "from p in payments.Invoice select p.due")
        );
        assert_eq!(
            vec![r#""ann""#],
            query(
                &db,
                "from p in payments.Card, c in customers where p.customer == key c select c.name"
            )
        );
    });
}

#[test]
fn moving_a_field_to_the_header_keeps_its_values_and_fills_the_other_constructors() {
    let up = "module shop\nmove field Payment.Card.last4 to Payment fill match\n  | Invoice i -> i.due\n";
    after_invoice(42, &[up], |db| {
        let db = db.expect("the move applies");
        assert_eq!(
            vec![r#""2027-01""#, r#""4242""#],
            query(&db, "from p in payments select p.last4")
        );
    });
}

#[test]
fn moving_a_header_field_down_keeps_its_values_in_the_listed_constructors() {
    let add = "module shop\nadd field Payment.note: Text fill \"n\"\n";
    let down = "module shop\nmove field Payment.note to Payment.(Card)\n";
    after_invoice(43, &[add, down], |db| {
        let db = db.expect("the move applies");
        assert_eq!(
            vec![r#""n""#],
            query(&db, "from p in payments.Card select p.note")
        );
        let got = fails(&db, "from p in payments.Invoice select p.note");
        assert!(matches!(got, Error::Rejected { .. }), "{got:?}");
    });
}

#[test]
fn a_merge_without_a_routing_maps_fields_by_name_and_fills_the_discriminator() {
    let rename = "module shop\nrename field Payment.Invoice.due to detail\nrename field Payment.Card.last4 to detail\n";
    let merge = r"module shop
add type Kind = Card | Invoice
merge constructor Payment.(Card | Invoice) into Any { kind: Kind, detail: Text }
";
    after_invoice(44, &[rename, merge], |db| {
        let db = db.expect("the merge applies");
        assert_eq!(
            vec![
                r#"{ detail: "2027-01", kind: Invoice }"#,
                r#"{ detail: "4242", kind: Card }"#
            ],
            query(
                &db,
                "from p in payments.Any select { detail: p.detail, kind: p.kind }"
            )
        );
    });
}

#[test]
fn a_rekey_carries_every_reference_to_the_new_key() {
    let rekey = r"module shop
rekey relation customers key name
  carry Payment.customer
drop field Customer.id
";
    after_invoice(45, &[rekey], |db| {
        let db = db.expect("the rekey applies");
        assert_eq!(
            vec![r#""ann""#, r#""cy""#],
            query(
                &db,
                "from p in payments, c in customers where p.customer == key c select c.name"
            )
        );
    });
}

#[test]
fn a_rekey_whose_new_key_two_rows_share_is_refused() {
    let same = r#"insert customers { name: "ann" }"#;
    let rekey = r"module shop
rekey relation customers key name
  carry Payment.customer
drop field Customer.id
";
    let mut sim = Sim::new(46, Config::default());
    run(&mut sim, |rt| {
        let db = opened(&rt, &[SHOP]);
        write(&db, CUSTOMERS);
        write(&db, same);
    });
    run(&mut sim, |rt| {
        let (version, detail) = refusal(open(&rt, &[SHOP, rekey]));
        assert_eq!(2, version);
        assert!(detail.contains("customers"), "{detail}");
    });
}

#[test]
fn an_added_check_existing_rows_violate_is_refused() {
    let check = "module shop\nadd field Customer.score: I32 fill 0\nadd check scored on customers (score > 0)\n";
    after(47, &[check], |db| {
        let (version, detail) = refusal(db);
        assert_eq!(2, version);
        assert!(detail.contains("scored"), "{detail}");
    });
}

#[test]
fn a_fill_that_breaks_an_existing_check_is_refused() {
    let check = "module shop\nadd check short on payments.Card (scalar_len(last4) == 4)\n";
    let split = r#"module shop
split constructor Payment.Card into Visa { last4: Text } | Other { last4: Text }
  by match
    | Card c -> Visa { last4: "12345" }
"#;
    after(48, &[check, split], |db| {
        let (version, detail) = refusal(db);
        assert_eq!(3, version);
        assert!(detail.contains("short"), "{detail}");
    });
}

#[test]
fn a_fill_whose_evaluation_faults_refuses_the_migration() {
    let add = "module shop\nadd field Customer.ratio: I64 fill 1 / 0\n";
    after(49, &[add], |db| {
        let (version, detail) = refusal(db);
        assert_eq!(2, version);
        assert!(detail.contains("DivisionByZero"), "{detail}");
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
fn a_shipped_migration_differing_only_in_an_arm_clause_diverges() {
    let shop = format!(
        "{SHOP}define fn method (p: Payment) -> Text = match p\n  | Card c -> c.last4\n  | Invoice i -> i.due\n"
    );
    let add = |arm: &str| {
        format!(
            "module shop\nadd constructor Payment.Wire {{ iban: Text }}\n  arm method: Wire w -> {arm}\n"
        )
    };
    let (first, other) = (add("w.iban"), add("\"wire\""));
    let mut sim = Sim::new(51, Config::default());
    run(&mut sim, |rt| drop(opened(&rt, &[&shop, &first])));
    run(&mut sim, |rt| match open(&rt, &[&shop, &other]) {
        Err(OpenError::Diverged { version }) => assert_eq!(2, version),
        got => panic!("a different arm clause does not diverge: {:?}", got.err()),
    });
}

#[test]
fn a_shipped_migration_differing_only_in_a_carried_field_diverges() {
    let shop = format!(
        "{SHOP}add type Note = {{ id: Key notes, about: Key customers, text: Text }}\nadd relation notes : Note key id generate sequence\n"
    );
    let rekey = "module shop\nrekey relation customers key name\n  carry Payment.customer, Note.about\ndrop field Customer.id\n";
    let reordered = "module shop\nrekey relation customers key name\n  carry Note.about, Payment.customer\ndrop field Customer.id\n";
    let mut sim = Sim::new(52, Config::default());
    run(&mut sim, |rt| drop(opened(&rt, &[&shop, rekey])));
    run(&mut sim, |rt| match open(&rt, &[&shop, reordered]) {
        Err(OpenError::Diverged { version }) => assert_eq!(2, version),
        got => panic!("a reordered carry clause does not diverge: {:?}", got.err()),
    });
}

#[test]
fn reverting_an_invertible_split_merges_the_rows_back() {
    let brand = "module shop\nadd type Brand = Visa | Other\nadd field Payment.Card.brand: Brand fill Visa\n";
    let split = r"module shop
split constructor Payment.Card into Visa { last4: Text } | Other { last4: Text }
  by match
    | Card { brand: Visa, last4 }  -> Visa { last4: last4 }
    | Card { brand: Other, last4 } -> Other { last4: last4 }
";
    let revert = "module shop\nrevert 0003\n";
    after_invoice(60, &[brand, split, revert], |db| {
        let db = db.expect("the revert applies");
        assert_eq!(
            vec![r#"{ brand: Visa, last4: "4242" }"#],
            query(
                &db,
                "from p in payments.Card select { brand: p.brand, last4: p.last4 }"
            )
        );
        assert_eq!(
            vec![r#""2027-01""#],
            query(&db, "from p in payments.Invoice select p.due")
        );
        let got = fails(&db, "from p in payments.Visa select p.last4");
        assert!(matches!(got, Error::Rejected { .. }), "{got:?}");
    });
}

#[test]
fn reverting_an_exact_merge_splits_the_rows_back_by_the_discriminator() {
    let merge = r"module shop
add type Kind = Card | Invoice
merge constructor Payment.(Card | Invoice) into Any { kind: Kind, last4: Text }
  by match
    | Card c    -> Any { kind: Card, last4: c.last4 }
    | Invoice i -> Any { kind: Invoice, last4: i.due }
";
    let revert = "module shop\nrevert 0002\n";
    after_invoice(61, &[merge, revert], |db| {
        let db = db.expect("the revert applies");
        assert_eq!(
            vec![r#""4242""#],
            query(&db, "from p in payments.Card select p.last4")
        );
        assert_eq!(
            vec![r#""2027-01""#],
            query(&db, "from p in payments.Invoice select p.due")
        );
    });
}

#[test]
fn reverting_a_nest_unnests_the_rows() {
    let revert = "module shop\nrevert 0002\n";
    after_invoice(62, &[NEST, revert], |db| {
        let db = db.expect("the revert applies");
        assert_eq!(
            vec![r#""4242""#],
            query(&db, "from p in payments.Card select p.last4")
        );
        assert_eq!(
            vec![r#""2027-01""#],
            query(&db, "from p in payments.Invoice select p.due")
        );
    });
}

#[test]
fn reverting_an_unnest_nests_the_rows_again() {
    let unnest = "module shop\nunnest constructor Payment.Held\n";
    let revert = "module shop\nrevert 0003\n";
    after_invoice(63, &[NEST, unnest, revert], |db| {
        let db = db.expect("the revert applies");
        assert_eq!(
            vec![r#"Card { last4: "4242" }"#, r#"Invoice { due: "2027-01" }"#],
            query(&db, "from p in payments.Held select p.method")
        );
    });
}

#[test]
fn reverting_a_move_to_the_header_moves_the_field_back_and_keeps_its_values() {
    let up = "module shop\nmove field Payment.Card.last4 to Payment fill match\n  | Invoice i -> i.due\n";
    let revert = "module shop\nrevert 0002\n";
    after_invoice(64, &[up, revert], |db| {
        let db = db.expect("the revert applies");
        assert_eq!(
            vec![r#""4242""#],
            query(&db, "from p in payments.Card select p.last4")
        );
        let got = fails(&db, "from p in payments.Invoice select p.last4");
        assert!(matches!(got, Error::Rejected { .. }), "{got:?}");
    });
}

#[test]
fn reverting_an_added_field_drops_it_and_keeps_the_rows() {
    let add = "module shop\nadd field Customer.vip: Bool fill false\n";
    let revert = "module shop\nrevert 0002\n";
    after(65, &[add, revert], |db| {
        let db = db.expect("the revert applies");
        assert_eq!(
            vec![r#""ann""#, r#""bob""#],
            query(&db, "from c in customers select c.name")
        );
        let got = fails(&db, "from c in customers select c.vip");
        assert!(matches!(got, Error::Rejected { .. }), "{got:?}");
    });
}

#[test]
fn reverting_a_revert_applies_the_migration_again_with_its_data_map() {
    let add = "module shop\nadd field Customer.vip: Bool fill false\n";
    let revert = "module shop\nrevert 0002\n";
    let again = "module shop\nrevert 0003\n";
    after(66, &[add, revert, again], |db| {
        let db = db.expect("the second revert applies");
        assert_eq!(
            vec![
                r#"{ name: "ann", vip: False }"#,
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
fn reverting_a_migration_of_several_operators_undoes_them_last_first() {
    let two = "module shop\nadd field Payment.Card.network: Text fill \"visa\"\nrename field Payment.Card.last4 to digits\n";
    let revert = "module shop\nrevert 0002\n";
    after(67, &[two, revert], |db| {
        let db = db.expect("the revert applies");
        assert_eq!(
            vec![r#""4242""#],
            query(&db, "from p in payments.Card select p.last4")
        );
        let got = fails(&db, "from p in payments.Card select p.network");
        assert!(matches!(got, Error::Rejected { .. }), "{got:?}");
    });
}

#[test]
fn reverting_a_lossy_split_is_refused() {
    let lossy = r#"module shop
split constructor Payment.Card into Visa { digits: Text } | Other { digits: Text }
  by match
    | Card c -> Visa { digits: "xxxx" }
"#;
    let revert = "module shop\nrevert 0002\n";
    after(68, &[lossy, revert], |db| {
        let (version, _) = refusal(db);
        assert_eq!(3, version);
    });
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

const UNIQUE_LAST4: &str = "module shop\nadd unique (last4) in payments.Card\n";

fn violated(e: &Error) -> String {
    match e {
        Error::Rejected {
            reason: RejectReason::Violated { constraint, .. },
        } => constraint.clone(),
        other => panic!("not a violation: {other:?}"),
    }
}

#[test]
fn a_nest_carries_a_unique_as_a_unique_over_the_path_into_the_nested_value() {
    after_invoice(70, &[UNIQUE_LAST4, NEST], |db| {
        let db = db.expect("the nest of a unique constructor applies");
        let e = fails(
            &db,
            r#"transact
  let c = insert customers { name: "dup" }
  insert payments.Held { customer: key c, method: Card { last4: "4242" } }"#,
        );
        assert_eq!(
            "shop::payments.unique(method.Card.last4) in Held",
            violated(&e)
        );
        write(
            &db,
            r#"transact
  let c = insert customers { name: "inv" }
  insert payments.Held { customer: key c, method: Invoice { due: "2027-01" } }"#,
        );
        assert_eq!(3, query(&db, "from p in payments select p.id").len());
    });
}

#[test]
fn an_unnest_of_a_path_unique_gives_back_the_unique_over_the_field() {
    let unnest = "module shop\nunnest constructor Payment.Held\n";
    after_invoice(71, &[UNIQUE_LAST4, NEST, unnest], |db| {
        let db = db.expect("the unnest applies");
        let e = fails(
            &db,
            r#"transact
  let c = insert customers { name: "dup" }
  insert payments.Card { customer: key c, last4: "4242" }"#,
        );
        assert_eq!("shop::payments.unique(last4) in Card", violated(&e));
    });
}

#[test]
fn an_added_path_unique_validates_the_rows() {
    let dup = r#"transact
  let c = insert customers { name: "dup" }
  insert payments.Held { customer: key c, method: Card { last4: "4242" } }"#;
    let add = "module shop\nadd unique (method.Card.last4) in payments.Held\n";
    let mut sim = Sim::new(72, Config::default());
    run(&mut sim, |rt| {
        let db = opened(&rt, &[SHOP]);
        write(&db, CUSTOMERS);
    });
    run(&mut sim, |rt| {
        let db = opened(&rt, &[SHOP, NEST]);
        write(&db, dup);
    });
    run(&mut sim, |rt| {
        let (version, detail) = refusal(open(&rt, &[SHOP, NEST, add]));
        assert_eq!(3, version);
        assert!(detail.contains("method.Card.last4"), "{detail}");
    });
}

#[test]
fn a_path_index_applies_and_leaves_query_results_unchanged() {
    let add = "module shop\nadd index (method.Invoice.due) in payments.Held\n";
    after_invoice(73, &[NEST, add], |db| {
        let db = db.expect("the path index applies");
        assert_eq!(
            vec![r#"Invoice { due: "2027-01" }"#],
            query(
                &db,
                "from p in payments.Held where match p.method | Invoice i -> i.due == \"2027-01\" | Card c -> false select p.method"
            )
        );
    });
}

const TAGGED: &str = r"module shop
add type Badge = { code: Text, label: Text }
add relation tags : Badge key code
add type Item = { id: Key items, tag: Key tags }
add relation items : Item key id generate sequence
";

#[test]
fn a_revert_of_a_rekey_carries_every_reference_back_to_the_old_key() {
    let rekey = "module shop\nrekey relation tags key label\n  carry Item.tag\n";
    let revert = "module shop\nrevert 0003\n";
    let mut sim = Sim::new(74, Config::default());
    run(&mut sim, |rt| {
        let db = opened(&rt, &[SHOP, TAGGED]);
        write(
            &db,
            r#"transact
  let a = insert tags { code: "a", label: "alpha" }
  let b = insert tags { code: "b", label: "beta" }
  insert items { tag: key a }
  insert items { tag: key b }
  insert items { tag: key b }"#,
        );
    });
    run(&mut sim, |rt| {
        let db = opened(&rt, &[SHOP, TAGGED, rekey, revert]);
        assert_eq!(
            vec![r#""a""#, r#""b""#],
            query(
                &db,
                "from i in items, t in tags where i.tag == key t select t.code"
            )
        );
        assert_eq!(
            3,
            query(
                &db,
                "from i in items, t in tags where i.tag == key t select { item: i.id, code: t.code }"
            )
            .len()
        );
        write(&db, r#"insert tags { code: "c", label: "alpha" }"#);
    });
}

#[test]
fn a_field_added_then_renamed_in_one_migration_applies() {
    let m = "module shop\nadd field Customer.nick: Text fill \"\"\nrename field Customer.nick to handle\n";
    after(75, &[m], |db| {
        let db = db.expect("the migration applies");
        assert_eq!(
            vec![r#""""#],
            query(&db, "from c in customers select c.handle")
        );
    });
}

const SHIPPING: &str = r"module shop
add type Address = { street: Text }
add type Shipment = { id: Key shipments, to: Address }
add relation shipments : Shipment key id generate sequence
";

#[test]
fn a_field_added_to_a_type_held_as_a_value_fills_every_stored_value() {
    let add = "module shop\nadd field Address.zip: Text fill \"none\"\n";
    let mut sim = Sim::new(76, Config::default());
    run(&mut sim, |rt| {
        let db = opened(&rt, &[SHOP, SHIPPING]);
        write(&db, r#"insert shipments { to: { street: "main" } }"#);
    });
    run(&mut sim, |rt| {
        let db = opened(&rt, &[SHOP, SHIPPING, add]);
        assert_eq!(
            vec![r#"Address { street: "main", zip: "none" }"#],
            query(&db, "from s in shipments select s.to")
        );
    });
}

#[test]
fn a_migration_validates_its_constraints_within_the_query_memory_of_the_open() {
    let check =
        "module shop\nadd constraint named = from c in customers where c.name == \"\" select c\n";
    let mut sim = Sim::new(77, Config::default());
    run(&mut sim, |rt| {
        let db = opened(&rt, &[SHOP]);
        write(&db, CUSTOMERS);
    });
    run(&mut sim, |rt| {
        let tight = OpenOptions {
            query_memory_bytes: Some(1),
            ..options(4)
        };
        let got = Database::open_in(rt, "app.adt", &Log::new([SHOP, check]), &tight);
        let (version, detail) = refusal(got);
        assert_eq!(2, version);
        assert!(detail.contains("query memory of 1 bytes"), "{detail}");
    });
    run(&mut sim, |rt| {
        opened(&rt, &[SHOP, check]);
    });
}

#[test]
fn reverting_a_constructor_a_keyed_record_gained_keeps_its_rows() {
    let vip = "module shop\nadd constructor Customer.Vip { tier: Text }\n";
    let revert = "module shop\nrevert 0002\n";
    after(74, &[vip, revert], |db| {
        let db = db.expect("the revert applies");
        write(&db, r#"insert customers { name: "cy" }"#);
        assert_eq!(
            vec![r#""ann""#, r#""bob""#, r#""cy""#],
            query(&db, "from c in customers select c.name")
        );
    });
}

#[test]
fn a_keyed_record_gains_a_constructor_keeping_its_rows_and_their_keys() {
    let vip = "module shop\nadd constructor Customer.Vip { tier: Text }\nenable customers.Vip\n";
    after(73, &[vip], |db| {
        let db = db.expect("the add applies");
        write(&db, r#"insert customers.Vip { tier: "gold" }"#);
        assert_eq!(
            vec![r#""ann""#, r#""bob""#],
            query(&db, "from c in customers.Customer select c.name")
        );
        assert_eq!(
            3,
            query(&db, "from c in customers select c.id").len(),
            "the keys stay distinct"
        );
        assert_eq!(
            vec![r#""4242""#],
            query(
                &db,
                "from p in payments.Card, c in customers.Customer where p.customer == key c and c.name == \"ann\" select p.last4"
            )
        );
    });
}

#[test]
fn reverting_a_split_by_a_bool_field_merges_the_rows_back_with_the_field() {
    let wide = "module shop\nadd field Payment.Card.wide: Bool fill true\n";
    let split = r"module shop
split constructor Payment.Card into Wide { last4: Text } | Narrow { last4: Text }
  by match
    | Card { wide: true, last4 } -> Wide { last4: last4 }
    | Card { wide: false, last4 } -> Narrow { last4: last4 }
";
    let revert = "module shop\nrevert 0003\n";
    after(70, &[wide, split, revert], |db| {
        let db = db.expect("the revert applies");
        assert_eq!(
            vec![r#"{ last4: "4242", wide: True }"#],
            query(
                &db,
                "from p in payments.Card select { wide: p.wide, last4: p.last4 }"
            )
        );
    });
}

#[test]
fn reverting_a_split_into_one_product_merges_the_rows_back() {
    let split = r"module shop
split constructor Payment.Card into Visa { digits: Text }
  by match
    | Card { last4 } -> Visa { digits: last4 }
";
    let revert = "module shop\nrevert 0002\n";
    after(71, &[split, revert], |db| {
        let db = db.expect("the revert applies");
        assert_eq!(
            vec![r#""4242""#],
            query(&db, "from p in payments.Card select p.last4")
        );
    });
}

#[test]
fn reverting_a_move_down_to_two_constructors_keeps_their_values_and_fills_the_others() {
    let wire = "module shop\nadd constructor Payment.Wire { iban: Text }\nenable payments.Wire\nadd field Payment.note: Text fill \"n\"\n";
    let down = "module shop\nmove field Payment.note to Payment.(Card | Invoice)\n";
    let revert =
        "module shop\nrevert 0003\n  fill Payment.Card.note = match\n    | Wire x -> \"w\"\n";
    let mut sim = Sim::new(72, Config::default());
    run(&mut sim, |rt| {
        let db = opened(&rt, &[SHOP]);
        write(&db, CUSTOMERS);
        write(&db, INVOICE);
    });
    run(&mut sim, |rt| {
        let db = opened(&rt, &[SHOP, wire]);
        write(
            &db,
            r#"insert payments.Wire { customer: customers.key(1), iban: "DE", note: "n" }"#,
        );
    });
    run(&mut sim, |rt| {
        let db = opened(&rt, &[SHOP, wire, down, revert]);
        for (ctor, note) in [
            ("Card", r#""n""#),
            ("Invoice", r#""n""#),
            ("Wire", r#""w""#),
        ] {
            assert_eq!(
                vec![note],
                query(&db, &format!("from p in payments.{ctor} select p.note")),
                "{ctor}"
            );
        }
    });
}

#[test]
fn an_added_default_fills_a_later_insert_and_its_revert_drops_it() {
    let default = "module shop\nadd default customers.name = \"anon\"\n";
    let revert = "module shop\nrevert 0002\n";
    after(73, &[default], |db| {
        let db = db.expect("the default applies");
        write(&db, "insert customers {}");
        assert_eq!(
            vec![r#""ann""#, r#""anon""#, r#""bob""#],
            query(&db, "from c in customers select c.name")
        );
    });
    after(74, &[default, revert], |db| {
        let db = db.expect("the revert applies");
        let got = fails(&db, "insert customers {}");
        assert!(matches!(got, Error::Rejected { .. }), "{got:?}");
    });
}

#[test]
fn reverting_a_move_down_moves_the_field_back_and_takes_the_fill_in_the_others() {
    let note = "module shop\nadd field Payment.note: Text fill \"n\"\n";
    let down = "module shop\nmove field Payment.note to Payment.(Invoice)\n";
    let revert =
        "module shop\nrevert 0003\n  fill Payment.Invoice.note = match\n    | Card x -> \"w\"\n";
    after_invoice(75, &[note, down, revert], |db| {
        let db = db.expect("the revert applies");
        assert_eq!(
            vec![r#""n""#, r#""w""#],
            query(&db, "from p in payments select p.note")
        );
    });
}

const SHOUT: &str = "module shop\nadd computed customers.shout: Text = self.name ++ \"!\"\n";

#[test]
fn a_dropped_default_no_longer_fills_an_insert_and_its_revert_restores_it() {
    let default = "module shop\nadd default customers.name = \"anon\"\n";
    let drop = "module shop\ndrop default customers.name\n";
    let revert = "module shop\nrevert 0003\n";
    after(77, &[default, drop], |db| {
        let db = db.expect("the drop applies");
        let got = fails(&db, "insert customers {}");
        assert!(matches!(got, Error::Rejected { .. }), "{got:?}");
    });
    after(78, &[default, drop, revert], |db| {
        let db = db.expect("the revert applies");
        write(&db, "insert customers {}");
        assert_eq!(
            vec![r#""ann""#, r#""anon""#, r#""bob""#],
            query(&db, "from c in customers select c.name")
        );
    });
}

#[test]
fn an_added_computed_field_reads_from_each_row() {
    after(79, &[SHOUT], |db| {
        let db = db.expect("the computed field applies");
        assert_eq!(
            vec![r#""ann!""#, r#""bob!""#],
            query(&db, "from c in customers select c.shout")
        );
    });
}

#[test]
fn a_redefined_computed_field_reads_its_new_expression_and_a_dropped_one_is_gone() {
    let redefine = "module shop\nredefine computed customers.shout: Text = self.name ++ \"?\"\n";
    after(80, &[SHOUT, redefine], |db| {
        let db = db.expect("the redefinition applies");
        assert_eq!(
            vec![r#""ann?""#, r#""bob?""#],
            query(&db, "from c in customers select c.shout")
        );
    });
    let drop = "module shop\ndrop computed customers.shout\n";
    after(81, &[SHOUT, drop], |db| {
        let db = db.expect("the drop applies");
        fails(&db, "from c in customers select c.shout");
    });
}

#[test]
fn reverting_an_added_computed_field_drops_it() {
    after(82, &[SHOUT, "module shop\nrevert 0002\n"], |db| {
        let db = db.expect("the revert applies");
        fails(&db, "from c in customers select c.shout");
    });
}

#[test]
fn a_computed_field_has_a_catalog_row_with_its_type_and_expression() {
    after(83, &[SHOUT], |db| {
        let db = db.expect("the computed field applies");
        assert_eq!(
            vec![r#"{ field_type: "Text", name: "shout", text: "self.name ++ \"!\"" }"#],
            query(
                &db,
                "from c in catalog::computed select { name: c.name, field_type: c.field_type, text: c.text }"
            )
        );
    });
}

const NAMED: &str = "module shop
define fn named () -> Set { id: Key customers, name: Text } =
  from c in customers select { id: c.id, name: c.name }
store named key id
";

#[test]
fn a_stored_view_reads_as_its_query_after_later_writes() {
    after(84, &[NAMED], |db| {
        let db = db.expect("the store applies");
        write(&db, "insert customers { name: \"cy\" }");
        assert_eq!(
            vec![r#""ann""#, r#""bob""#, r#""cy""#],
            query(&db, "from n in named() select n.name")
        );
    });
}

#[test]
fn a_stored_view_is_a_relation_of_kind_stored_and_a_base_relation_of_kind_base() {
    after(85, &[NAMED], |db| {
        let db = db.expect("the store applies");
        let kind = |name: &str| {
            query(
                &db,
                &format!(
                    "from r in catalog::relations where r.name == \"{name}\" select match r.kind
  | Base -> \"base\"
  | Stored -> \"stored\"
  | Catalog -> \"catalog\"
  | Volatile -> \"volatile\""
                ),
            )
        };
        assert_eq!(vec![r#""stored""#], kind("named"));
        assert_eq!(vec![r#""base""#], kind("customers"));
        assert_eq!(vec![r#""catalog""#], kind("migrations"));
    });
}

#[test]
fn unstoring_or_reverting_a_store_keeps_the_view_readable_and_drops_its_relation() {
    for (seed, last) in [
        (86, "module shop\nunstore named\n"),
        (87, "module shop\nrevert 0002\n"),
    ] {
        after(seed, &[NAMED, last], |db| {
            let db = db.unwrap_or_else(|e| panic!("seed {seed}: `{last}` applies: {e:?}"));
            assert_eq!(
                Vec::<String>::new(),
                query(
                    &db,
                    "from r in catalog::relations where r.name == \"named\" select r.name"
                ),
                "seed {seed}"
            );
        });
    }
    after(88, &[NAMED, "module shop\nunstore named\n"], |db| {
        let db = db.expect("the unstore applies");
        assert_eq!(
            vec![r#""ann""#, r#""bob""#],
            query(&db, "from n in named() select n.name")
        );
    });
}

#[test]
fn a_grant_and_its_revert_apply_and_log_the_grant() {
    let grant = "module shop\ngrant read customers to support with grant option\n";
    after(89, &[grant, "module shop\nrevert 0002\n"], |db| {
        let db = db.expect("the grant and its revert apply");
        assert_eq!(
            vec![r#""read shop::customers support True""#],
            query(
                &db,
                "from o in catalog::migration_ops where o.migration == catalog::migrations.key(2)
  select match o.op
    | Grant g -> g.privilege ++ \" \" ++ g.grantee ++ \" \" ++ print(g.grant_option)
    | _ -> \"\""
            )
        );
    });
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
    let (sim, start) = second_migration(seed, derived);
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
