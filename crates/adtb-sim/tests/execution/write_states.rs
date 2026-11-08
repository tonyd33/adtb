//! Write states of constructors, readers first, pending drops and staged migrations over the
//! simulator.

use adtb_harness::process::run;
use adtb_simkit::api;
use adtb_simkit::open;

use adtb::RejectReason;

use adtb::{CallOptions, Database, Error, Log, OpenError};
use adtb_sim::{Config, Sim, SimRuntime};

use adtb_simkit::open::options;
use api::query;

fn opened(rt: &SimRuntime, log: &[&str]) -> Database<SimRuntime> {
    open::open(rt, open::FILE, &Log::new(log.iter().copied()), &options(4))
        .unwrap_or_else(|e| panic!("the database opens at {}: {e:?}", log.len()))
}

fn write(db: &Database<SimRuntime>, text: &str) {
    db.query_text(text, &[], &CallOptions::default())
        .unwrap_or_else(|e| panic!("`{text}` fails: {e:?}"));
}

#[must_use]
fn fails(db: &Database<SimRuntime>, text: &str) -> Error {
    match db.query_text(text, &[], &CallOptions::default()) {
        Ok(rows) => panic!("`{text}` succeeds with {} results", rows.len()),
        Err(e) => e,
    }
}

/// Assert that `text` is refused as a write into a constructor that is not open, naming `ctor`
/// in state `stage`.
fn not_open(db: &Database<SimRuntime>, text: &str, ctor: &str, stage: &str) {
    match fails(db, text) {
        Error::Rejected {
            reason: RejectReason::NotOpen { ctor: c, stage: s },
        } => assert_eq!((ctor, stage), (c.as_str(), s.as_str()), "`{text}`"),
        other => panic!("`{text}` is not refused as not open: {other:?}"),
    }
}

/// Returns the version and the reason of a migration refused at open.
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
add type Badge { name: Text } = Plain | Color { rgb: I32 }
add relation post_badges : Badge key name
add relation user_badges : Badge key name
add set Simple of post_badges = Plain
add type Provider = Local | Google
add type Account = { id: Key accounts, provider: Provider, note: Text }
add relation accounts : Account key id generate sequence
add type Hold { id: Key holds } = Live { payment: Key payments on delete retag Lapsed {} } | Lapsed
add relation holds : Hold key id generate sequence
";

const ROWS: &str = r#"transact
  let c = insert customers { name: "ann" }
  insert payments.Card { customer: key c, last4: "4242" }
  insert payments.Invoice { customer: key c, due: "2027-01" }
  insert customers { name: "bob" }
  insert post_badges.Plain { name: "new" }
  insert accounts { provider: Local, note: "a" }"#;

/// Open a database at `SHOP` holding `ROWS`, then reopen it with `later` appended to the log,
/// and run `body` over the result of that open.
fn after(seed: u64, later: &[&str], body: impl FnOnce(Result<Database<SimRuntime>, OpenError>)) {
    let mut sim = Sim::new(seed, Config::default());
    run(&mut sim, |rt| {
        let db = opened(&rt, &[SHOP]);
        write(&db, ROWS);
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

const ADD_WIRE: &str = "module shop\nadd constructor Payment.Wire { iban: Text }\n";

const INSERT_WIRE: &str = r#"transact
  let c = insert customers { name: "cy" }
  insert payments.Wire { customer: key c, iban: "DE00" }"#;

#[test]
fn a_declared_constructor_refuses_inserts_and_retags_into_it_and_reads_as_empty() {
    after(1, &[ADD_WIRE], |db| {
        let db = db.expect("the add applies");
        not_open(&db, INSERT_WIRE, "payments.Wire", "Declared");
        not_open(
            &db,
            r#"update p in payments.Card where p.last4 == "4242" set Wire { iban: "DE01" }"#,
            "payments.Wire",
            "Declared",
        );
        assert!(query(&db, "from p in payments.Wire select p.iban").is_empty());
        assert_eq!(2, query(&db, "from p in payments select p.id").len());
    });
}

#[test]
fn a_deprecated_constructor_refuses_entry_and_allows_reads_updates_retags_out_and_deletes() {
    let deprecate = "module shop\ndeprecate payments.Card\n";
    after(2, &[deprecate], |db| {
        let db = db.expect("the deprecate applies");
        not_open(
            &db,
            r#"transact
  let c = insert customers { name: "cy" }
  insert payments.Card { customer: key c, last4: "1111" }"#,
            "payments.Card",
            "Deprecated",
        );
        not_open(
            &db,
            r#"update p in payments.Invoice where p.due == "2027-01" set Card { last4: "1" }"#,
            "payments.Card",
            "Deprecated",
        );
        assert_eq!(
            vec![r#""4242""#],
            query(&db, "from p in payments.Card select p.last4")
        );
        write(
            &db,
            r#"update p in payments.Card where p.last4 == "4242" set Card { p with last4: "4243" }"#,
        );
        assert_eq!(
            vec![r#""4243""#],
            query(&db, "from p in payments.Card select p.last4")
        );
        write(
            &db,
            r#"update p in payments.Card where p.last4 == "4243" set Invoice { due: "2028-01" }"#,
        );
        assert!(query(&db, "from p in payments.Card select p.last4").is_empty());
        write(
            &db,
            r#"delete p in payments.Invoice where p.due == "2028-01""#,
        );
        assert_eq!(1, query(&db, "from p in payments select p.customer").len());
    });
}

#[test]
fn enabling_a_deprecated_constructor_makes_it_writable_and_its_revert_deprecates_it_again() {
    let deprecate = "module shop\ndeprecate payments.Card\n";
    let enable = "module shop\nenable payments.Card\n";
    let revert = "module shop\nrevert 0003\n";
    let insert = r#"transact
  let c = insert customers { name: "cy" }
  insert payments.Card { customer: key c, last4: "1111" }"#;
    after(3, &[deprecate, enable], |db| {
        let db = db.expect("the enable applies");
        write(&db, insert);
    });
    after(4, &[deprecate, enable, revert], |db| {
        let db = db.expect("the revert applies");
        not_open(&db, insert, "payments.Card", "Deprecated");
    });
}

#[test]
fn disabling_an_empty_enabled_constructor_refuses_inserts_into_it_again() {
    let enable = "module shop\nenable payments.Wire\n";
    let disable = "module shop\ndisable payments.Wire\n";
    after(5, &[ADD_WIRE, enable, disable], |db| {
        let db = db.expect("the disable applies");
        not_open(&db, INSERT_WIRE, "payments.Wire", "Declared");
    });
}

#[test]
fn a_constructor_a_delete_action_retags_into_is_neither_deprecated_nor_disabled() {
    for (seed, op) in [(6, "deprecate"), (7, "disable")] {
        let text = format!("module shop\n{op} holds.Lapsed\n");
        after(seed, &[&text], |db| {
            let (version, detail) = refusal(db);
            assert_eq!(2, version, "seed {seed}");
            assert!(detail.contains("holds.Lapsed"), "seed {seed}: {detail}");
        });
    }
}

#[test]
fn deprecate_applies_only_to_an_enabled_constructor() {
    let deprecate = "module shop\ndeprecate payments.Wire\n";
    after(8, &[ADD_WIRE, deprecate], |db| {
        let (version, detail) = refusal(db);
        assert_eq!(3, version);
        assert!(detail.contains("payments.Wire"), "{detail}");
    });
}

#[test]
fn write_states_are_per_relation_variable() {
    let add =
        "module shop\nadd constructor Badge.Emoji { glyph: Text }\nenable post_badges.Emoji\n";
    after(9, &[add], |db| {
        let db = db.expect("the add applies");
        write(
            &db,
            r#"insert post_badges.Emoji { name: "smile", glyph: ":)" }"#,
        );
        not_open(
            &db,
            r#"insert user_badges.Emoji { name: "smile", glyph: ":)" }"#,
            "user_badges.Emoji",
            "Declared",
        );
    });
}

#[test]
fn a_relation_added_after_a_declared_constructor_starts_it_declared() {
    let add = "module shop\nadd constructor Badge.Emoji { glyph: Text }\n";
    let team = "module shop\nadd relation team_badges : Badge key name\n";
    after(10, &[add, team], |db| {
        let db = db.expect("the relation applies");
        write(&db, r#"insert team_badges.Plain { name: "p" }"#);
        not_open(
            &db,
            r#"insert team_badges.Emoji { name: "smile", glyph: ":)" }"#,
            "team_badges.Emoji",
            "Declared",
        );
    });
}

#[test]
fn the_products_of_a_split_take_the_state_of_their_source() {
    let deprecate = "module shop\ndeprecate payments.Card\n";
    let split = r"module shop
split constructor Payment.Card into Visa { digits: Text } | Other { digits: Text }
  by match
    | Card { last4 } -> Visa { digits: last4 }
";
    after(11, &[deprecate, split], |db| {
        let db = db.expect("the split applies");
        assert_eq!(
            vec![r#""4242""#],
            query(&db, "from p in payments.Visa select p.digits")
        );
        not_open(
            &db,
            r#"transact
  let c = insert customers { name: "cy" }
  insert payments.Visa { customer: key c, digits: "1" }"#,
            "payments.Visa",
            "Deprecated",
        );
    });
}

const MERGE: &str = r"module shop
add type Method = Card | Invoice
merge constructor Payment.(Card | Invoice) into Any { method: Method, detail: Text }
  by match
    | Card c    -> Any { method: Card, detail: c.last4 }
    | Invoice i -> Any { method: Invoice, detail: i.due }
";

#[test]
fn merging_a_deprecated_constructor_is_refused() {
    let deprecate = "module shop\ndeprecate payments.Card\n";
    after(12, &[deprecate, MERGE], |db| {
        let (version, detail) = refusal(db);
        assert_eq!(3, version);
        assert!(detail.contains("Card"), "{detail}");
    });
}

#[test]
fn a_merge_of_only_deprecated_sources_keeps_their_rows_and_refuses_entry_and_reverts() {
    let deprecate = "module shop\ndeprecate payments.Card\ndeprecate payments.Invoice\n";
    let entry = r#"transact
  let c = insert customers { name: "cy" }
  insert payments.Any { customer: key c, method: Card, detail: "1" }"#;
    after(14, &[deprecate, MERGE], |db| {
        let db = db.expect("the merge applies");
        not_open(&db, entry, "payments.Any", "Deprecated");
        assert_eq!(
            2,
            query(&db, "from p in payments.Any select p.detail").len()
        );
    });
    after(
        15,
        &[deprecate, MERGE, "module shop\nrevert 0003\n"],
        |db| {
            let db = db.expect("the revert applies");
            assert_eq!(
                vec![r#""4242""#],
                query(&db, "from p in payments.Card select p.last4")
            );
            not_open(
                &db,
                r#"transact
  let c = insert customers { name: "cy" }
  insert payments.Card { customer: key c, last4: "1111" }"#,
                "payments.Card",
                "Deprecated",
            );
        },
    );
}

#[test]
fn a_merge_with_a_declared_source_starts_declared() {
    let merge = r"module shop
add type Method = Invoice | Wire
merge constructor Payment.(Invoice | Wire) into Billed { method: Method, detail: Text }
  by match
    | Invoice i -> Billed { method: Invoice, detail: i.due }
    | Wire w    -> Billed { method: Wire, detail: w.iban }
";
    after(13, &[ADD_WIRE, merge], |db| {
        let db = db.expect("the merge applies");
        not_open(
            &db,
            r#"transact
  let c = insert customers { name: "cy" }
  insert payments.Billed { customer: key c, method: Wire, detail: "DE00" }"#,
            "payments.Billed",
            "Declared",
        );
    });
}

const ADD_OKTA: &str = "module shop\nadd constructor Provider.Okta\n";

#[test]
fn a_declared_value_constructor_is_never_stored_until_enabled() {
    let enable = "module shop\nenable Provider.Okta\n";
    let store = r#"insert accounts { provider: Okta, note: "o" }"#;
    after(14, &[ADD_OKTA], |db| {
        let db = db.expect("the add applies");
        not_open(&db, store, "Provider.Okta", "Declared");
        not_open(
            &db,
            r#"update a in accounts where a.note == "a" set Account { a with provider: Okta }"#,
            "Provider.Okta",
            "Declared",
        );
    });
    after(15, &[ADD_OKTA, enable], |db| {
        let db = db.expect("the enable applies");
        write(&db, store);
        assert_eq!(
            vec!["Local", "Okta"],
            query(&db, "from a in accounts select a.provider")
        );
    });
}

#[test]
fn a_row_holding_a_deprecated_value_may_be_updated_while_it_keeps_the_value() {
    let enable = "module shop\nenable Provider.Okta\n";
    let deprecate = "module shop\ndeprecate Provider.Okta\n";
    let mut sim = Sim::new(16, Config::default());
    run(&mut sim, |rt| {
        let db = opened(&rt, &[SHOP, ADD_OKTA, enable]);
        write(&db, ROWS);
        write(&db, r#"insert accounts { provider: Okta, note: "o" }"#);
    });
    run(&mut sim, |rt| {
        let db = opened(&rt, &[SHOP, ADD_OKTA, enable, deprecate]);
        write(
            &db,
            r#"update a in accounts where a.note == "o" set Account { a with note: "kept" }"#,
        );
        not_open(
            &db,
            r#"update a in accounts where a.note == "a" set Account { a with provider: Okta }"#,
            "Provider.Okta",
            "Deprecated",
        );
        not_open(
            &db,
            r#"insert accounts { provider: Okta, note: "p" }"#,
            "Provider.Okta",
            "Deprecated",
        );
    });
}

#[test]
fn a_value_constructor_a_stored_value_holds_is_neither_disabled_nor_removed() {
    let enable = "module shop\nenable Provider.Okta\n";
    let disable = "module shop\ndisable Provider.Okta\n";
    let deprecate = "module shop\ndeprecate Provider.Okta\n";
    let remove = "module shop\nremove constructor Provider.Okta\n";
    let stored = |seed: u64, later: &[&str]| {
        let mut sim = Sim::new(seed, Config::default());
        run(&mut sim, |rt| {
            let db = opened(&rt, &[SHOP, ADD_OKTA, enable]);
            write(&db, r#"insert accounts { provider: Okta, note: "o" }"#);
        });
        let mut log = vec![SHOP, ADD_OKTA, enable];
        log.extend_from_slice(later);
        run(&mut sim, |rt| {
            refusal(open::open(
                &rt,
                open::FILE,
                &Log::new(log.iter().copied()),
                &options(4),
            ))
        })
    };
    let (version, detail) = stored(17, &[disable]);
    assert_eq!(4, version);
    assert!(detail.contains("Okta"), "{detail}");
    let (version, detail) = stored(18, &[deprecate, remove]);
    assert_eq!(5, version);
    assert!(detail.contains("Okta"), "{detail}");
}

/// Open a database at `SHOP` and `before`, write `stored`, then return the refusal of
/// reopening it with `later` appended.
#[must_use]
fn refused_after(seed: u64, before: &[&str], stored: &str, later: &[&str]) -> (u64, String) {
    let mut sim = Sim::new(seed, Config::default());
    let mut log = vec![SHOP];
    log.extend_from_slice(before);
    run(&mut sim, |rt| {
        let db = opened(&rt, &log);
        write(&db, stored);
    });
    log.extend_from_slice(later);
    run(&mut sim, |rt| {
        refusal(open::open(
            &rt,
            open::FILE,
            &Log::new(log.iter().copied()),
            &options(4),
        ))
    })
}

#[test]
fn a_revert_of_an_enable_on_a_constructor_that_holds_rows_is_refused() {
    let enable = "module shop\nenable payments.Wire\n";
    let revert = "module shop\nrevert 0003\n";
    let (version, detail) = refused_after(104, &[ADD_WIRE, enable], INSERT_WIRE, &[revert]);
    assert_eq!(4, version);
    assert!(
        detail.contains("constructor relation payments.Wire holds rows"),
        "{detail}"
    );
}

#[test]
fn a_revert_that_disables_or_removes_a_stored_value_constructor_is_refused() {
    let enable = "module shop\nenable Provider.Okta\n";
    let store = r#"insert accounts { provider: Okta, note: "o" }"#;
    let (version, detail) = refused_after(
        105,
        &[ADD_OKTA, enable],
        store,
        &["module shop\nrevert 0003\n"],
    );
    assert_eq!(4, version);
    assert!(
        detail.contains("holds a value of Provider.Okta; it cannot be disabled"),
        "{detail}"
    );
    let both = "module shop\nadd constructor Provider.Okta\nenable Provider.Okta\n";
    let (version, detail) = refused_after(106, &[both], store, &["module shop\nrevert 0002\n"]);
    assert_eq!(3, version);
    assert!(
        detail.contains("holds a value of Provider.Okta"),
        "{detail}"
    );
}

#[test]
fn removing_an_unused_deprecated_value_constructor_applies() {
    let enable = "module shop\nenable Provider.Okta\n";
    let deprecate = "module shop\ndeprecate Provider.Okta\n";
    let remove = "module shop\nremove constructor Provider.Okta\n";
    after(19, &[ADD_OKTA, enable, deprecate, remove], |db| {
        let db = db.expect("the remove applies");
        assert_eq!(
            vec!["Local"],
            query(&db, "from a in accounts select a.provider")
        );
    });
}

#[test]
fn a_constructor_deprecated_in_the_same_migration_is_not_removed() {
    let both = "module shop\ndeprecate payments.Invoice\nremove constructor Payment.Invoice\n";
    after(20, &[both], |db| {
        let (version, detail) = refusal(db);
        assert_eq!(2, version);
        assert!(detail.contains("Invoice"), "{detail}");
    });
}

#[test]
fn a_constructor_deprecated_in_only_some_relation_variables_is_not_removed() {
    let deprecate = "module shop\ndeprecate post_badges.Color\n";
    let remove = "module shop\nremove constructor Badge.Color\n";
    after(21, &[deprecate, remove], |db| {
        let (version, detail) = refusal(db);
        assert_eq!(3, version);
        assert!(detail.contains("user_badges"), "{detail}");
    });
}

#[test]
fn removing_the_last_constructor_of_a_named_set_is_refused() {
    let deprecate = "module shop\ndeprecate post_badges.Plain\ndeprecate user_badges.Plain\n";
    let remove = "module shop\nremove constructor Badge.Plain\n";
    let mut sim = Sim::new(22, Config::default());
    run(&mut sim, |rt| {
        opened(&rt, &[SHOP, deprecate]);
    });
    run(&mut sim, |rt| {
        let (version, detail) = refusal(open::open(
            &rt,
            open::FILE,
            &Log::new([SHOP, deprecate, remove].iter().copied()),
            &options(4),
        ));
        assert_eq!(3, version);
        assert!(detail.contains("Simple"), "{detail}");
    });
}

#[test]
fn removing_a_constructor_a_reference_type_names_in_a_set_is_refused() {
    let refer = r"module shop
add type Charge = { id: Key charges, bill: Key payments.Invoice }
add relation charges : Charge key id generate sequence
";
    let deprecate = "module shop\ndeprecate payments.Invoice\n";
    let remove = "module shop\nremove constructor Payment.Invoice\n";
    let mut sim = Sim::new(23, Config::default());
    run(&mut sim, |rt| {
        let db = opened(&rt, &[SHOP, refer, deprecate]);
        write(
            &db,
            r#"delete p in payments.Invoice where p.due == "2027-01""#,
        );
    });
    run(&mut sim, |rt| {
        let (version, detail) = refusal(open::open(
            &rt,
            open::FILE,
            &Log::new([SHOP, refer, deprecate, remove].iter().copied()),
            &options(4),
        ));
        assert_eq!(4, version);
        assert!(
            detail.contains("charges") || detail.contains("Charge"),
            "{detail}"
        );
    });
}

const NAMED: &str =
    "module shop\nadd constraint named = from c in customers where c.name == \"\" select c\n";
const DROP_NAMED: &str = "module shop\ndrop constraint named\n";

#[test]
fn an_embedded_open_holds_back_no_drop() {
    after(24, &[NAMED, DROP_NAMED], |db| {
        let db = db.expect("the drop applies");
        write(&db, r#"insert customers { name: "" }"#);
        assert_eq!(3, query(&db, "from c in customers select c.name").len());
    });
}

#[test]
fn a_staged_migration_that_finds_a_duplicate_is_discarded_naming_the_operator_and_the_rows() {
    let unique = "module shop\nadd unique (name) in customers\n";
    let mut sim = Sim::new(25, Config::default());
    run(&mut sim, |rt| {
        let db = opened(&rt, &[SHOP]);
        write(&db, ROWS);
        write(&db, r#"insert customers { name: "ann" }"#);
    });
    run(&mut sim, |rt| {
        let (version, detail) = refusal(open::open(
            &rt,
            open::FILE,
            &Log::new([SHOP, unique].iter().copied()),
            &options(4),
        ));
        assert_eq!(2, version);
        assert!(detail.contains("unique (name)"), "{detail}");
        assert!(
            detail.contains(r#""ann""#),
            "the report names no row: {detail}"
        );
    });
    run(&mut sim, |rt| {
        let db = opened(&rt, &[SHOP]);
        assert_eq!(3, query(&db, "from c in customers select c.id").len());
        assert_eq!(
            vec!["migrations.key(1)"],
            query(&db, "from m in catalog::migrations select m.version")
        );
    });
}

#[test]
fn a_disable_of_a_constructor_that_holds_rows_is_refused_naming_the_disable() {
    let both = "module shop\nadd constructor Payment.Wire { iban: Text }\ndisable payments.Card\n";
    after(973, &[both], |got| match got {
        Err(OpenError::MigrationRefused {
            version,
            reason:
                Error::Rejected {
                    reason: RejectReason::SchemaRefused { operator, detail },
                },
        }) => {
            assert_eq!(2, version);
            assert_eq!("disable payments.Card", operator);
            assert!(
                detail.contains("constructor relation payments.Card holds rows"),
                "{detail}"
            );
        }
        Err(other) => panic!("not a refused migration: {other:?}"),
        Ok(_) => panic!("the disable applies"),
    });
}
