//! Registered queries and transactions in process: registration at open, calls by ordinal, the
//! principal a call runs for, and the recheck of a registration when the schema changes.

use adtb_simkit::api;

use adtb::{
    CallOptions, Called, Database, Error, InTxn, Log, OpenOptions, Registration, RejectReason,
    TxnCallOptions, TxnOptions, Value,
};
use adtb_sim::SimRuntime;

use api::{once, options, printed, result};

const SHOP: &str = r#"module shop
add module shop
add type Customer = { id: Key customers, name: Text }
add relation customers : Customer key id generate sequence
add type Note = { id: Key notes, body: Text, tag: Text }
add relation notes : Note key id generate sequence
add type Payment { id: Key payments, customer: Key customers }
  = Card { last4: Text }
  | Invoice { due: Text }
add relation payments : Payment
  key id generate sequence
  index (customer)
define fn method (p: Payment) -> Text = match p
  | Card c -> c.last4
  | Invoice i -> i.due
define fn whose () -> Text = match caller
  | EndUser u -> u.name
  | Service s -> s.name
  | Anonymous -> "nobody"
"#;

/// The calls of the shop, in ordinal order: `method` 1, `whose` 2, then the declarations.
const APP: &str = r"import shop
import fn method
import fn whose
query names () -> Set Text = from c in customers select c.name
query methods (k: Key customers) -> Set Text =
  from p in payments where p.customer == k select method(p)
txn sign_up (n: Text, last4: Text) -> Key customers =
  transact
    let c = insert customers { name: n }
    insert payments.Card { customer: key c, last4: last4 }
    return key c
query me () -> Principal = caller
query mine () -> Set Text =
  from c in customers
  where match caller
          | EndUser u -> c.name == u.name
          | Service _ -> false
          | Anonymous -> false
  select c.name
";

const NAMES: u32 = 3;
const SIGN_UP: u32 = 5;
const ME: u32 = 6;

fn log() -> Log {
    Log::new([SHOP])
}

fn app() -> Registration {
    Registration::new("app", APP)
}

fn with(regs: Vec<Registration>) -> OpenOptions {
    OpenOptions {
        registrations: regs,
        ..options(4)
    }
}

fn open(rt: &SimRuntime) -> Database<SimRuntime> {
    Database::open_in(rt.clone(), "app.adt", &log(), &with(vec![app()]))
        .expect("the database opens with its registration")
}

fn text(t: &str) -> Value {
    Value::text(t)
}

fn asserting(p: Value) -> CallOptions {
    CallOptions {
        assert: Some(p),
        ..CallOptions::default()
    }
}

fn reason(e: &Error) -> &RejectReason {
    match e {
        Error::Rejected { reason } => reason,
        other => panic!("not a rejection: {other:?}"),
    }
}

#[test]
fn a_registered_transaction_commits_and_returns_its_result_and_position() {
    once(1, |rt| {
        let db = open(&rt);
        let app = db.registered("app").expect("app is registered");
        let commit =
            |n: &str| match app.call(SIGN_UP, &[text(n), text("1111")], &CallOptions::default()) {
                Ok(Called::Committed(c)) => c,
                other => panic!("a registered transaction commits: {other:?}"),
            };
        let first = commit("ann");
        let second = commit("bob");
        assert!(
            second.position > first.position,
            "the later commit comes later in the log"
        );
        assert_eq!(1, first.attempts, "an uncontended transaction runs once");
        let key: Vec<String> = printed(first.value);
        assert_eq!(
            printed(result(
                &db,
                r#"from c in customers where c.name == "ann" select key c"#
            )),
            key,
            "its result is the declared key"
        );
        assert_eq!(
            vec![r#""ann""#, r#""bob""#],
            printed(result(
                &db,
                "from c in customers select c.name order by c.name"
            )),
            "ad hoc text reads what the registered transaction wrote"
        );
    });
}

#[test]
fn ordinals_follow_declaration_order() {
    once(2, |rt| {
        let db = open(&rt);
        let app = db.registered("app").expect("app is registered");
        let order = [
            "method", "whose", "names", "methods", "sign_up", "me", "mine",
        ];
        for (i, name) in order.iter().enumerate() {
            assert_eq!(
                Some(u32::try_from(i + 1).expect("few calls")),
                app.ordinal(name),
                "{name}"
            );
        }
        assert_eq!(None, app.ordinal("nothing"));
    });
}

#[test]
fn an_assertion_that_is_no_principal_is_invalid() {
    once(15, |rt| {
        let db = open(&rt);
        let app = db.registered("app").expect("app is registered");
        let got = app.call(ME, &[], &asserting(text("ann")));
        assert!(
            matches!(
                got.as_ref().map_err(reason),
                Err(RejectReason::Invalid { .. })
            ),
            "{got:?}"
        );
    });
}

#[test]
fn a_registration_calls_only_in_a_transaction_of_its_own_database() {
    once(27, |rt| {
        let db = open(&rt);
        let other = Database::open_in(rt, "other.adt", &log(), &with(vec![app()]))
            .expect("a second database opens");
        let app = other.registered("app").expect("app is registered");
        let mut t = db.begin(&TxnOptions::default()).expect("begin");
        let got = t.call(&app, NAMES, &[], &TxnCallOptions::default());
        assert!(
            matches!(
                got.as_ref().map_err(reason),
                Err(RejectReason::Invalid { .. })
            ),
            "{got:?}"
        );
    });
}

#[test]
fn admission_checks_unused_parameters_before_running_a_write() {
    once(30, |rt| {
        let reg = Registration::new(
            "app",
            r#"import shop
 txn unused (n: I8) -> I64 =
   transact
     insert customers { name: "written" }
     return 1
 "#,
        );
        let db = Database::open_in(rt, "app.adt", &log(), &with(vec![reg]))
            .expect("the admission fixture opens");
        let app = db.registered("app").expect("app binds");
        let got = app.call(1, &[Value::Int(128)], &CallOptions::default());
        assert!(
            matches!(
                got,
                Err(Error::Rejected {
                    reason: RejectReason::Invalid { .. }
                })
            ),
            "{got:?}"
        );
        assert!(printed(result(&db, "from c in shop::customers select c.name")).is_empty());
        let got = app.call_arrow(1, &[Value::Int(128)], &CallOptions::default());
        assert!(
            matches!(
                got,
                Err(Error::Rejected {
                    reason: RejectReason::Invalid { .. }
                })
            ),
            "{got:?}"
        );
        let mut t = db.begin(&TxnOptions::default()).expect("begin");
        let good = t
            .call(&app, 1, &[Value::Int(127)], &TxnCallOptions::default())
            .expect("a valid call stays pending");
        assert!(matches!(good, InTxn::Pending(_)));
        let bad = t.call(&app, 1, &[Value::Int(128)], &TxnCallOptions::default());
        assert!(
            matches!(
                bad,
                Err(Error::Rejected {
                    reason: RejectReason::Invalid { .. }
                })
            ),
            "{bad:?}"
        );
        assert_eq!(
            vec!["1"],
            api::in_txn(&mut t, "count(c for c in shop::customers)")
        );
        t.commit(None)
            .expect("a refused call preserves the pending write");
        assert_eq!(
            vec![r#""written""#],
            printed(result(&db, "from c in shop::customers select c.name"))
        );
    });
}

const ADMISSION_SCHEMA: &str = r"module shop
add type Positive = new I8 where self > 0
add type Item = { id: Key items, number: Positive }
add relation items : Item key id generate sequence
add type Envelope a { code: I16 } = Wrap { value: Option a }
define fn ignored (n: I8) -> I64 = 1
";

const ADMISSION_CALLS: &str = r"import shop
import fn ignored
query narrow (n: I8) -> I8 = n
query envelope (v: Envelope I8) -> I64 = 1
query nested (v: { choice: Option I8, words: Set Text }) -> I64 = 1
query restricted (v: Payment.Card) -> I64 = 1
query key_value (v: Key customers) -> I64 = 1
query numbers (v: Set I8) -> I64 = 1
query positive (v: Positive) -> I64 = 1
query uuid (v: Uuid) -> I64 = 1
query time (v: Time) -> I64 = 1
query decimal (v: Decimal 2) -> I64 = 1
query unicode (v: Text) -> I64 = 1
query unit (v: {}) -> I64 = 1
query date (v: Date) -> I64 = 1
";

type AdmissionCase = (&'static str, Vec<Value>, Vec<Value>);

fn admission_cases() -> Vec<AdmissionCase> {
    vec![
        (
            "ignored",
            vec![Value::Int(-128), Value::Int(127)],
            vec![Value::Int(-129), Value::Int(128), text("bad")],
        ),
        (
            "narrow",
            vec![Value::Int(-128), Value::Int(127)],
            vec![Value::Int(-129), Value::Int(128)],
        ),
        (
            "envelope",
            vec![Value::ctor(
                0,
                vec![Value::Int(32767), Value::some(Value::Int(127))],
            )],
            vec![
                Value::ctor(0, vec![Value::Int(32768), Value::none()]),
                Value::ctor(0, vec![Value::Int(1), Value::some(Value::Int(128))]),
                Value::ctor(0, vec![Value::Int(1)]),
                Value::ctor(1, vec![Value::Int(1), Value::none()]),
            ],
        ),
        (
            "nested",
            vec![Value::record(vec![
                Value::some(Value::Int(127)),
                Value::set([text("é")].into()),
            ])],
            vec![
                Value::record(vec![Value::some(Value::Int(128)), Value::empty_set()]),
                Value::record(vec![Value::none()]),
                Value::record(vec![Value::none(), Value::set([Value::Int(1)].into())]),
            ],
        ),
        (
            "restricted",
            vec![Value::ctor(
                0,
                vec![Value::Int(999), Value::Int(888), text("4242")],
            )],
            vec![
                Value::ctor(1, vec![Value::Int(999), Value::Int(888), text("due")]),
                Value::ctor(0, vec![text("bad"), Value::Int(888), text("4242")]),
            ],
        ),
        ("key_value", vec![Value::Int(999)], vec![text("bad")]),
        (
            "numbers",
            vec![
                Value::empty_set(),
                Value::set([Value::Int(-128), Value::Int(127)].into()),
            ],
            vec![
                Value::set([Value::Int(128)].into()),
                Value::List(std::sync::Arc::from([Value::Int(1)])),
            ],
        ),
        (
            "positive",
            vec![Value::Int(-1), Value::Int(0), Value::Int(127)],
            vec![Value::Int(128)],
        ),
        (
            "uuid",
            vec![
                Value::Bytes(std::sync::Arc::from([])),
                Value::Bytes(std::sync::Arc::from([1])),
                Value::Bytes(std::sync::Arc::from([1; 16])),
            ],
            vec![text("bad")],
        ),
        (
            "time",
            vec![Value::Time(0), Value::Time(86_399_999_999)],
            vec![Value::Time(-1), Value::Time(86_400_000_000)],
        ),
        (
            "decimal",
            vec![Value::Decimal(10_i128.pow(38) - 1)],
            vec![Value::Decimal(10_i128.pow(38)), Value::Int(1)],
        ),
        (
            "unicode",
            vec![text("é")],
            vec![Value::Text(std::sync::Arc::from("e\u{301}"))],
        ),
        (
            "unit",
            vec![Value::record(vec![])],
            vec![Value::record(vec![Value::Int(1)]), Value::tag(0)],
        ),
        (
            "date",
            vec![Value::Date(0)],
            vec![Value::Date(i32::MIN), Value::Date(i32::MAX)],
        ),
    ]
}

#[test]
fn admission_validates_all_representations_at_each_public_call_boundary() {
    once(31, |rt| {
        let db = Database::open_in(
            rt,
            "app.adt",
            &Log::new([SHOP, ADMISSION_SCHEMA]),
            &with(vec![Registration::new("app", ADMISSION_CALLS)]),
        )
        .expect("the representation fixture opens");
        let app = db.registered("app").expect("app binds");
        let mut t = db.begin(&TxnOptions::default()).expect("begin");
        for (name, good, bad) in admission_cases() {
            let ordinal = app.ordinal(name).expect("the callable exists");
            for arg in bad {
                let args = [arg];
                let got = app.call(ordinal, &args, &CallOptions::default());
                assert!(
                    matches!(
                        got,
                        Err(Error::Rejected {
                            reason: RejectReason::Invalid { .. }
                        })
                    ),
                    "{name} {args:?}: {got:?}"
                );
                let got = app.call_arrow(ordinal, &args, &CallOptions::default());
                assert!(
                    matches!(
                        got,
                        Err(Error::Rejected {
                            reason: RejectReason::Invalid { .. }
                        })
                    ),
                    "Arrow {name} {args:?}: {got:?}"
                );
                let got = t.call(&app, ordinal, &args, &TxnCallOptions::default());
                assert!(
                    matches!(
                        got,
                        Err(Error::Rejected {
                            reason: RejectReason::Invalid { .. }
                        })
                    ),
                    "interactive {name} {args:?}: {got:?}"
                );
            }
            for arg in good {
                let expected = if name == "narrow" {
                    arg.clone()
                } else {
                    Value::Int(1)
                };
                let args = [arg];
                let Called::Rows(rows) = app
                    .call(ordinal, &args, &CallOptions::default())
                    .expect("valid standalone argument")
                else {
                    panic!("a query returns rows")
                };
                assert_eq!(
                    vec![expected.clone()],
                    rows.collect::<Result<Vec<_>, _>>().expect("valid rows"),
                    "{name} {args:?}"
                );
                let arrow = app
                    .call_arrow(ordinal, &args, &CallOptions::default())
                    .expect("valid Arrow argument");
                assert_eq!(
                    1,
                    arrow
                        .map(|batch| batch.expect("a batch").num_rows())
                        .sum::<usize>(),
                    "{name} {args:?}"
                );
                let InTxn::Rows(rows) = t
                    .call(&app, ordinal, &args, &TxnCallOptions::default())
                    .expect("valid interactive argument")
                else {
                    panic!("a query returns rows")
                };
                assert_eq!(
                    vec![expected],
                    rows.collect::<Result<Vec<_>, _>>().expect("valid rows"),
                    "{name} {args:?}"
                );
            }
        }
        t.commit(None)
            .expect("refusals leave the transaction usable");
        let got = db.query_text(
            "insert shop::items { number: shop::Positive(-1) }",
            &[],
            &CallOptions::default(),
        );
        assert!(
            matches!(
                got,
                Err(Error::Rejected {
                    reason: RejectReason::Violated { .. }
                })
            ),
            "a domain still rejects a stored value: {got:?}"
        );
        assert_eq!(
            vec!["0"],
            printed(result(&db, "count(i for i in shop::items)"))
        );
    });
}
