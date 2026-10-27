//! Calls through the code generated from the fixture shop, over the simulator: every typed result
//! agrees with the ad hoc text of the same query, and the generated ids and ordinals agree with
//! those the database assigns.

#[expect(dead_code, reason = "this binary uses part of the API helpers")]
#[path = "../../adtb-sim/tests/support/api.rs"]
mod api;

use std::sync::Arc;

use adtb::batch::BatchBuilder;
use adtb::key::Value as Stored;
use adtb::typed::{HostValue, Relation};
use adtb::{
    ArrowSchema, BundleDef, CallOptions, Database, Field, Key, LoadOptions, Log, OpenOptions,
    RecordBatch, RejectReason, TxnCallOptions, TxnOptions,
};
use adtb_codegen_tests::shop::{queries, schema};
use adtb_lang::textual::print;
use adtb_sim::SimRuntime;

use api::{once, options, printed, result, sorted};
use queries::Error;
use schema::{Customer, Customers, Payment, Tier};

fn log() -> Log {
    Log::new(schema::LOG.iter().copied())
}

fn open(rt: &SimRuntime) -> Database<SimRuntime> {
    let reg = queries::BUNDLE
        .registration()
        .expect("the generated bundle file registers");
    let opts = OpenOptions {
        registrations: vec![reg],
        ..options(4)
    };
    Database::open_in(rt.clone(), "shop.adt", &log(), &opts).expect("the database opens")
}

fn app(db: &Database<SimRuntime>) -> adtb::typed::Bundle<queries::Bundle, SimRuntime> {
    db.bundle(&queries::BUNDLE).expect("the bundle binds")
}

fn opts() -> CallOptions {
    CallOptions::default()
}

/// The customers ann (free, paying by card) and bob (paid, paying by card and invoice, with a
/// nick and a note), signed up through the generated transactions.
struct Seeded {
    ann: Key<Customers>,
    bob: Key<Customers>,
}

fn seed(db: &Database<SimRuntime>) -> Seeded {
    let app = app(db);
    let sign_up = |n: &str, t: Tier, last4: &str| {
        app.call(queries::sign_up, (n.into(), t, last4.into()), &opts())
            .unwrap_or_else(|e| panic!("{n} signs up: {e:?}"))
            .value
    };
    let ann = sign_up("ann", Tier::Free, "4242");
    let bob = sign_up("bob", Tier::Paid { level: 3 }, "1111");
    app.call(queries::invoice, (bob.clone(), 30), &opts())
        .expect("bob is invoiced");
    app.call(queries::set_nick, (bob.clone(), Some("b".into())), &opts())
        .expect("bob takes a nick");
    app.call(queries::note, (bob.clone(), "vip".into()), &opts())
        .expect("bob gets a note");
    Seeded { ann, bob }
}

/// Assert that `rows`, printed at the type of the result of ad hoc `text`, are its rows.
fn agree<T: HostValue>(
    db: &Database<SimRuntime>,
    rows: impl IntoIterator<Item = Result<T, Error>>,
    text: &str,
) {
    let adhoc = result(db, text);
    let form = adhoc.form().clone();
    let want = sorted(adhoc);
    let mut got: Vec<String> = rows
        .into_iter()
        .map(|r| {
            let v = r
                .unwrap_or_else(|e| panic!("a row of `{text}`: {e:?}"))
                .to_value();
            print(&v, &form).unwrap_or_else(|e| panic!("{v:?} prints: {e:?}"))
        })
        .collect();
    got.sort();
    assert_eq!(want, got, "the typed rows are those of `{text}`");
}

#[test]
fn the_bundle_pins_the_hash_of_the_version_it_names() {
    let pins = usize::try_from(queries::Bundle::PINS).expect("a small version");
    let source =
        adtb_lang::signature::replay(schema::LOG[..pins].iter().copied()).expect("the log replays");
    let schema = source.schema().expect("the source builds");
    let hash: String = adtb::catalog::version_hash(&schema, &source)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert!(
        queries::Bundle::TEXT.contains(&hash),
        "the fixture header states the hash {hash}"
    );
}

#[test]
fn ordinals_agree_with_the_registration() {
    once(0, |rt| {
        let db = open(&rt);
        let reg = db
            .registered(queries::Bundle::NAME)
            .expect("the bundle binds");
        let ordinal = |name: &str| {
            reg.ordinal(name)
                .unwrap_or_else(|| panic!("{name} registers"))
        };
        assert_eq!(1, ordinal("method"), "an imported function comes first");
        let generated = [
            ("method", <queries::method as adtb::Callable>::ORDINAL),
            ("names", <queries::names as adtb::Callable>::ORDINAL),
            ("customer", <queries::customer as adtb::Callable>::ORDINAL),
            (
                "payments_of",
                <queries::payments_of as adtb::Callable>::ORDINAL,
            ),
            ("tiers", <queries::tiers as adtb::Callable>::ORDINAL),
            ("nicks", <queries::nicks as adtb::Callable>::ORDINAL),
            ("summary", <queries::summary as adtb::Callable>::ORDINAL),
            ("total", <queries::total as adtb::Callable>::ORDINAL),
            ("sign_up", <queries::sign_up as adtb::Callable>::ORDINAL),
            ("note", <queries::note as adtb::Callable>::ORDINAL),
            ("invoice", <queries::invoice as adtb::Callable>::ORDINAL),
            ("set_nick", <queries::set_nick as adtb::Callable>::ORDINAL),
            ("tags_of", <queries::tags_of as adtb::Callable>::ORDINAL),
            ("tag", <queries::tag as adtb::Callable>::ORDINAL),
            (
                "open_ticket",
                <queries::open_ticket as adtb::Callable>::ORDINAL,
            ),
            ("statuses", <queries::statuses as adtb::Callable>::ORDINAL),
            (
                "with_status",
                <queries::with_status as adtb::Callable>::ORDINAL,
            ),
        ];
        for (name, o) in generated {
            assert_eq!(ordinal(name), o, "the ordinal of {name}");
        }
        assert_eq!(
            None,
            reg.ordinal("label"),
            "a bundle function takes no ordinal"
        );
    });
}

/// Returns the one id ad hoc `text` selects, as the digits of its printed form.
fn id(db: &Database<SimRuntime>, text: &str) -> u64 {
    let rows = printed(result(db, text));
    let [row] = rows.as_slice() else {
        panic!("`{text}` selects one id: {rows:?}")
    };
    let digits: String = row.chars().filter(char::is_ascii_digit).collect();
    digits
        .parse()
        .unwrap_or_else(|e| panic!("`{row}` holds an id: {e}"))
}

#[test]
fn ids_agree_with_the_catalog() {
    once(0, |rt| {
        let db = open(&rt);
        let ty = |name: &str| {
            id(
                &db,
                &format!(
                    "from t in catalog::types where t.module == \"shop\" and t.name == \"{name}\" \
                     select t.id"
                ),
            )
        };
        let ctor = |name: &str| {
            id(
                &db,
                &format!("from c in catalog::constructors where c.name == \"{name}\" select c.id"),
            )
        };
        let rel = |name: &str| {
            id(
                &db,
                &format!("from r in catalog::relations where r.name == \"{name}\" select r.id"),
            )
        };
        assert_eq!(ty("Tier"), Tier::TYPE_ID, "Tier");
        assert_eq!(
            [ctor("Free"), ctor("Paid")],
            Tier::CTOR_IDS,
            "Tier's constructors"
        );
        assert_eq!(ty("Customer"), Customer::TYPE_ID, "Customer");
        assert_eq!(
            [ctor("Customer")],
            Customer::CTOR_IDS,
            "Customer's constructor"
        );
        assert_eq!(ty("Payment"), Payment::TYPE_ID, "Payment");
        assert_eq!(
            [ctor("Card"), ctor("Invoice")],
            Payment::CTOR_IDS,
            "Payment's constructors"
        );
        assert_eq!(
            ty("Note"),
            schema::Note::TYPE_ID,
            "Note, of the second migration"
        );
        assert_eq!([ctor("Note")], schema::Note::CTOR_IDS, "Note's constructor");
        assert_eq!(rel("customers"), Customers::ID, "customers");
        assert_eq!(rel("payments"), schema::Payments::ID, "payments");
        assert_eq!(rel("notes"), schema::Notes::ID, "notes");
        assert_eq!(rel("tags"), schema::Tags::ID, "tags");
    });
}

#[test]
fn a_record_and_its_key_round_trip() {
    once(0, |rt| {
        let db = open(&rt);
        let s = seed(&db);
        let app = app(&db);
        let rows: Vec<Customer> = app
            .call(queries::customer, (s.ann.clone(),), &opts())
            .expect("the customer")
            .collect::<Result<_, _>>()
            .expect("every row is a Customer");
        let want = Customer {
            id: s.ann.clone(),
            name: "ann".into(),
            tier: Tier::Free,
            nick: None,
        };
        assert_eq!(vec![want], rows);
        let rows = app
            .call(queries::customer, (s.bob,), &opts())
            .expect("the customer");
        agree(
            &db,
            rows,
            "from c in customers where c.name == \"bob\" select c",
        );
    });
}

#[test]
fn a_sum_of_several_constructors_agrees_with_ad_hoc_text() {
    once(0, |rt| {
        let db = open(&rt);
        let s = seed(&db);
        let rows = app(&db)
            .call(queries::payments_of, (s.bob,), &opts())
            .expect("bob's payments");
        let rows: Vec<Result<Payment, Error>> = rows.collect();
        assert_eq!(2, rows.len(), "bob pays by card and invoice");
        agree(
            &db,
            rows,
            "from p in payments, c in customers where p.customer == key c and c.name == \"bob\" \
             select p",
        );
    });
}

#[test]
fn value_level_sums_and_options_agree_with_ad_hoc_text() {
    once(0, |rt| {
        let db = open(&rt);
        seed(&db);
        let app = app(&db);
        let tiers = app.call(queries::tiers, (), &opts()).expect("the tiers");
        agree(&db, tiers, "from c in customers select c.tier");
        let nicks = app.call(queries::nicks, (), &opts()).expect("the nicks");
        agree(&db, nicks, "from c in customers select c.nick");
        let mut got: Vec<Option<String>> = app
            .call(queries::nicks, (), &opts())
            .expect("the nicks")
            .collect::<Result<_, _>>()
            .expect("every row is an Option Text");
        got.sort();
        assert_eq!(vec![None, Some("b".to_owned())], got);
    });
}

#[test]
fn nested_results_agree_with_ad_hoc_text() {
    once(0, |rt| {
        let db = open(&rt);
        seed(&db);
        let rows: Vec<Result<queries::SummaryRow, Error>> = app(&db)
            .call(queries::summary, (), &opts())
            .expect("the summary")
            .collect();
        let bob = rows
            .iter()
            .flatten()
            .find(|r| r.name == "bob")
            .expect("bob is summarised");
        assert_eq!(vec!["vip".to_owned()], bob.notes);
        assert_eq!(2, bob.methods.len(), "bob pays two ways");
        assert!(
            bob.methods
                .contains(&queries::SummaryRowMethods::ByInvoice { due: 30 }),
            "{:?}",
            bob.methods
        );
        agree(
            &db,
            rows,
            "from c in customers
             select {
               name: c.name,
               notes: from n in notes where n.customer == key c select n.body,
               methods: from p in payments where p.customer == key c
                 select match p
                   | Card x -> ByCard { last4: x.last4 }
                   | Invoice i -> ByInvoice { due: i.due }
             }",
        );
    });
}

#[test]
fn a_result_that_is_no_set_is_the_value_itself() {
    once(0, |rt| {
        let db = open(&rt);
        seed(&db);
        let total: i64 = app(&db)
            .call(queries::total, (), &opts())
            .expect("the total");
        assert_eq!(2, total);
    });
}

#[test]
fn an_imported_function_takes_its_arguments_as_host_values() {
    once(0, |rt| {
        let db = open(&rt);
        let s = seed(&db);
        let card = Payment::Card {
            id: Key(1),
            customer: s.ann,
            last4: "4242".into(),
        };
        let shown: String = app(&db)
            .call(queries::method, (card,), &opts())
            .expect("the method");
        assert_eq!("4242", shown);
    });
}

#[test]
fn calls_inside_an_interactive_transaction_commit_with_it() {
    once(0, |rt| {
        let db = open(&rt);
        let app = app(&db);
        let mut t = app.begin(&TxnOptions::default()).expect("a transaction");
        let in_txn = TxnCallOptions::default();
        let k = t
            .call(
                queries::sign_up,
                ("cy".into(), Tier::Free, "9".into()),
                &in_txn,
            )
            .expect("cy signs up")
            .value;
        t.call(queries::note, (k.clone(), "new".into()), &in_txn)
            .expect("cy gets a note");
        let seen: Vec<String> = t
            .call(queries::names, (), &in_txn)
            .expect("the names")
            .collect::<Result<_, _>>()
            .expect("every name is Text");
        assert_eq!(
            vec!["cy".to_owned()],
            seen,
            "the transaction reads its own writes"
        );
        t.commit(None).expect("the transaction commits");
        let rows = app.call(queries::customer, (k,), &opts()).expect("cy");
        assert_eq!(1, rows.count(), "cy is committed");
    });
}

#[test]
fn transact_reruns_a_closure_and_returns_its_commit() {
    once(0, |rt| {
        let db = open(&rt);
        let app = app(&db);
        let done = app
            .transact(&TxnOptions::default(), 3, |t| {
                let k = t
                    .call(
                        queries::sign_up,
                        ("dee".into(), Tier::Free, "7".into()),
                        &TxnCallOptions::default(),
                    )?
                    .value;
                Ok(k)
            })
            .expect("the closure commits");
        assert_eq!(1, done.attempts);
        let names: Vec<String> = app
            .call(queries::names, (), &opts())
            .expect("the names")
            .collect::<Result<_, _>>()
            .expect("every name is Text");
        assert_eq!(vec!["dee".to_owned()], names);
    });
}

#[test]
fn a_tag_takes_the_constructors_in_declaration_order() {
    once(0, |rt| {
        let db = open(&rt);
        let app = app(&db);
        let open = app
            .call(queries::open_ticket, (schema::Status::Open,), &opts())
            .expect("a ticket opens")
            .value;
        let tags: Vec<queries::StatusTag> = app
            .call(queries::statuses, (), &opts())
            .expect("the statuses")
            .collect::<Result<_, _>>()
            .expect("every row is a StatusTag");
        assert_eq!(vec![queries::StatusTag::Open], tags);
        agree(
            &db,
            tags.into_iter().map(Ok),
            "from t in tickets select tag(t.status)",
        );
        let keys: Vec<Key<schema::Tickets>> = app
            .call(queries::with_status, (queries::StatusTag::Open,), &opts())
            .expect("the open tickets")
            .collect::<Result<_, _>>()
            .expect("every row is a key");
        assert_eq!(vec![open], keys, "the tag argument names Open");
    });
}

#[test]
fn transact_reruns_a_closure_whose_call_meets_a_serialization_failure() {
    once(0, |rt| {
        let db = open(&rt);
        let s = seed(&db);
        let app = app(&db);
        let mut runs = 0;
        let done = app
            .transact(&TxnOptions::default(), 3, |t| {
                runs += 1;
                t.call(
                    queries::customer,
                    (s.ann.clone(),),
                    &TxnCallOptions::default(),
                )?;
                if runs == 1 {
                    app.call(
                        queries::set_nick,
                        (s.ann.clone(), Some("x".into())),
                        &opts(),
                    )?;
                }
                let nick = t
                    .call(
                        queries::set_nick,
                        (s.ann.clone(), Some("y".into())),
                        &TxnCallOptions::default(),
                    )?
                    .value;
                Ok(nick)
            })
            .expect("the second run commits");
        assert_eq!(2, runs, "the closure runs again");
        assert_eq!(2, done.attempts);
        assert_eq!(Some("y".to_owned()), done.value);
    });
}

#[test]
fn a_refused_call_returns_the_bundle_error_type() {
    once(0, |rt| {
        let db = open(&rt);
        let got = app(&db).call(queries::note, (Key(99), "lost".into()), &opts());
        assert!(
            matches!(got, Err(Error::Rejected { .. })),
            "a note of no customer is refused: {got:?}"
        );
    });
}

/// A definition of the fixture bundle that pins another version than its file.
struct Elsewhere;

impl BundleDef for Elsewhere {
    type Error = Error;
    const NAME: &'static str = queries::Bundle::NAME;
    const TEXT: &'static str = queries::Bundle::TEXT;
    const PINS: u64 = 1;
}

#[test]
fn binding_a_bundle_of_another_pin_is_refused() {
    once(0, |rt| {
        let db = open(&rt);
        let got = db.bundle(&Elsewhere).map(drop);
        assert!(
            matches!(
                got,
                Err(Error::Rejected {
                    reason: RejectReason::VersionMismatch { .. }
                })
            ),
            "{got:?}"
        );
    });
}

#[test]
fn a_call_returns_arrow_batches_of_its_result() {
    once(0, |rt| {
        let db = open(&rt);
        seed(&db);
        let stream = app(&db)
            .call_arrow(queries::names, (), &opts())
            .expect("the names as batches");
        let names: Vec<String> = stream
            .schema()
            .fields
            .iter()
            .map(|f| f.name.clone())
            .collect();
        assert_eq!(vec!["value".to_owned()], names);
        let rows: usize = stream.map(|b| b.expect("a batch").num_rows()).sum();
        assert_eq!(2, rows);
    });
}

/// Returns a batch of `rows` over the fields of `full` named in `names`, in that order.
fn batch(full: &ArrowSchema, names: &[&str], rows: &[Vec<Stored>]) -> RecordBatch {
    let fields: Vec<Field> = names
        .iter()
        .map(|n| {
            let f = full.fields.iter().find(|f| f.name == *n);
            f.unwrap_or_else(|| panic!("the row has field {n}")).clone()
        })
        .collect();
    let schema = Arc::new(ArrowSchema { fields });
    let mut b = BatchBuilder::new(schema.batch_schema());
    for r in rows {
        b.push_row(r);
    }
    RecordBatch::new(schema, b.finish()).expect("the batch is of its schema")
}

#[test]
fn a_load_appends_to_a_generated_constructor_relation() {
    once(0, |rt| {
        let db = open(&rt);
        let s = seed(&db);
        let mut load = db.load(&LoadOptions::default()).expect("a load");
        let mut cards = load
            .appender(schema::payments::Card)
            .expect("the appender of payments.Card");
        let k = s.ann.0;
        let b = batch(
            cards.schema(),
            &["customer", "last4"],
            &[vec![Stored::I64(k), Stored::Text("5555".into())]],
        );
        cards.push(&b).expect("the batch stages");
        let report = load.commit().expect("the load commits");
        assert_eq!(1, report.rows);
        let rows = app(&db)
            .call(queries::payments_of, (s.ann,), &opts())
            .expect("ann's payments");
        assert_eq!(2, rows.count(), "ann pays by two cards");
        assert_eq!("shop::customers", Customers::NAME);
    });
}

#[test]
fn a_composite_key_is_a_record_of_its_key_fields() {
    once(0, |rt| {
        let db = open(&rt);
        let s = seed(&db);
        let app = app(&db);
        let k = app
            .call(queries::tag, (s.ann.clone(), "early".into()), &opts())
            .expect("ann is tagged")
            .value;
        let want = schema::TagsKey {
            customer: s.ann.clone(),
            label: "early".into(),
        };
        assert_eq!(Key(want), k);
        let rows = app
            .call(queries::tags_of, (s.ann,), &opts())
            .expect("ann's tags");
        agree(&db, rows, "from t in tags select t");
    });
}
