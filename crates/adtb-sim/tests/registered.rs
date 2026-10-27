//! Registered queries and transactions in process: registration at open, calls by ordinal, the
//! principal a call runs for, and the recheck of a registration when the schema changes.

#[expect(dead_code, reason = "this binary uses part of the API helpers")]
#[path = "support/api.rs"]
mod api;

use adtb::{
    CallOptions, Called, Database, Error, InTxn, Log, OpenError, OpenOptions, Registration,
    RejectReason, TxnCallOptions, TxnOptions, Value,
};
use adtb_lang::textual::{self, Form};
use adtb_lang::types::Ty;
use adtb_sim::{Config, Sim, SimRuntime};

use api::{once, options, printed, result, run};

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

const METHOD: u32 = 1;
const WHOSE: u32 = 2;
const NAMES: u32 = 3;
const METHODS: u32 = 4;
const SIGN_UP: u32 = 5;
const ME: u32 = 6;
const MINE: u32 = 7;

/// The value the schema owner's handle runs as.
const OWNER: &str = r#"Service { name: "owner" }"#;

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

/// Returns `printed`, a value of the schema's `Principal` in the textual syntax.
fn principal(db: &Database<SimRuntime>, printed: &str) -> Value {
    let schema = db.schema();
    let ty = Ty::Data {
        def: schema.principal,
        args: Vec::new(),
        ctors: None,
    };
    textual::parse(printed, &Form::of(schema, &ty))
        .unwrap_or_else(|e| panic!("`{printed}` is a principal: {e}"))
}

fn asserting(p: Value) -> CallOptions {
    CallOptions {
        assert: Some(p),
        ..CallOptions::default()
    }
}

/// Returns the rows of the call `ordinal` of `app` with `args`, printed in order.
fn rows(
    db: &Database<SimRuntime>,
    ordinal: u32,
    args: &[Value],
    opts: &CallOptions,
) -> Vec<String> {
    let app = db.registered("app").expect("app is registered");
    match app.call(ordinal, args, opts) {
        Ok(Called::Rows(r)) => printed(r),
        other => panic!("call {ordinal} returns no rows: {other:?}"),
    }
}

fn sign_up(db: &Database<SimRuntime>, n: &str) -> Value {
    let app = db.registered("app").expect("app is registered");
    match app.call(SIGN_UP, &[text(n), text("4242")], &CallOptions::default()) {
        Ok(Called::Committed(c)) => {
            let mut v: Vec<_> = c.value.collect();
            assert_eq!(1, v.len(), "a transaction returns one value");
            v.remove(0).expect("the key")
        }
        other => panic!("sign_up commits nothing: {other:?}"),
    }
}

fn reason(e: &Error) -> &RejectReason {
    match e {
        Error::Rejected { reason } => reason,
        other => panic!("not a rejection: {other:?}"),
    }
}

/// Returns the reason a registration named `name` was refused at open, its detail text.
fn refused(got: Result<Database<SimRuntime>, OpenError>, name: &str) -> String {
    match got {
        Err(OpenError::RegistrationRefused { bundle, reason }) => {
            assert_eq!(name, bundle, "the refusal names the registration");
            match reason {
                Error::Rejected {
                    reason: RejectReason::Invalid { detail },
                } => detail,
                other => panic!("the refusal is no Invalid: {other:?}"),
            }
        }
        Err(other) => panic!("open fails otherwise: {other:?}"),
        Ok(_) => panic!("the registration is not refused"),
    }
}

#[test]
fn a_registered_query_is_called_by_ordinal_with_its_arguments() {
    once(0, |rt| {
        let db = open(&rt);
        assert!(rows(&db, NAMES, &[], &CallOptions::default()).is_empty());
        let k = sign_up(&db, "ann");
        sign_up(&db, "bob");
        let mut names = rows(&db, NAMES, &[], &CallOptions::default());
        names.sort();
        assert_eq!(vec![r#""ann""#, r#""bob""#], names);
        assert_eq!(
            vec![r#""4242""#],
            rows(&db, METHODS, &[k], &CallOptions::default()),
            "a parameter selects the customer, and a schema function the query reaches runs"
        );
    });
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
fn an_imported_schema_function_is_called_by_ordinal() {
    once(3, |rt| {
        let db = open(&rt);
        sign_up(&db, "ann");
        let card = result(&db, "from p in payments select p");
        let p: Vec<Value> = card.map(|r| r.expect("a payment")).collect();
        assert_eq!(1, p.len());
        assert_eq!(
            vec![r#""4242""#],
            rows(&db, METHOD, &p, &CallOptions::default()),
            "a function whose result is not a set returns its value as one row"
        );
    });
}

#[test]
fn an_unregistered_name_or_ordinal_is_unknown() {
    once(4, |rt| {
        let db = open(&rt);
        let got = db.registered("web");
        assert!(
            matches!(
                got.as_ref().map(|_| ()).map_err(reason),
                Err(RejectReason::UnknownBundle { bundle }) if bundle == "web"
            ),
            "{:?}",
            got.err()
        );
        let app = db.registered("app").expect("app is registered");
        let got = app.call(99, &[], &CallOptions::default());
        assert!(
            matches!(
                got.as_ref().map_err(reason),
                Err(RejectReason::UnknownQuery)
            ),
            "{got:?}"
        );
    });
}

#[test]
fn a_call_with_the_wrong_number_of_arguments_is_invalid() {
    once(5, |rt| {
        let db = open(&rt);
        let app = db.registered("app").expect("app is registered");
        let got = app.call(SIGN_UP, &[text("ann")], &CallOptions::default());
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
fn a_registration_that_fails_to_check_refuses_open_naming_the_query() {
    once(6, |rt| {
        let bad = Registration::new(
            "bad",
            "import shop\nquery ok () -> Set Text = from c in customers select c.name\n\
             query broken () -> Set Text = from c in customers select c.nickname\n",
        );
        let got = Database::open_in(rt, "app.adt", &log(), &with(vec![bad]));
        let detail = refused(got, "bad");
        assert!(detail.contains("broken"), "{detail}");
        assert!(detail.contains("nickname"), "{detail}");
    });
}

#[test]
fn a_registration_that_misses_a_constructor_refuses_open() {
    once(7, |rt| {
        let bad = Registration::new(
            "partial",
            "import shop\nquery cards () -> Set Text =\n  from p in payments select match p\n    | Card c -> c.last4\n",
        );
        let got = Database::open_in(rt, "app.adt", &log(), &with(vec![bad]));
        let detail = refused(got, "partial");
        assert!(detail.contains("cards"), "{detail}");
    });
}

#[test]
fn registrations_take_effect_together_or_not_at_all() {
    let mut sim = Sim::new(8, Config::default());
    run(&mut sim, |rt| {
        let bad = Registration::new("bad", "import shop\nquery q () -> Set Text = nope\n");
        let got = Database::open_in(rt, "app.adt", &log(), &with(vec![app(), bad]));
        refused(got, "bad");
    });
    run(&mut sim, |rt| {
        let db = Database::open_in(rt, "app.adt", &log(), &options(4))
            .expect("the migration of the failed open stays applied");
        let got = db.registered("app");
        assert!(
            matches!(
                got.as_ref().map(|_| ()).map_err(reason),
                Err(RejectReason::UnknownBundle { .. })
            ),
            "an open that fails registers nothing"
        );
    });
}

#[test]
fn two_registrations_of_one_name_refuse_open() {
    once(9, |rt| {
        let got = Database::open_in(rt, "app.adt", &log(), &with(vec![app(), app()]));
        refused(got, "app");
    });
}

#[test]
fn two_calls_of_one_name_in_a_registration_refuse_open() {
    once(10, |rt| {
        let twice = Registration::new(
            "twice",
            "import shop\nquery q () -> Set Text = from c in customers select c.name\n\
             query q () -> Set Text = from c in customers select c.name\n",
        );
        let got = Database::open_in(rt, "app.adt", &log(), &with(vec![twice]));
        let detail = refused(got, "twice");
        assert!(detail.contains('q'), "{detail}");
    });
}

#[test]
fn a_call_with_no_assertion_runs_for_the_schema_owner() {
    once(11, |rt| {
        let db = open(&rt);
        assert_eq!(vec![OWNER], rows(&db, ME, &[], &CallOptions::default()));
        assert_eq!(
            vec![r#""owner""#],
            rows(&db, WHOSE, &[], &CallOptions::default()),
            "a schema function sees the caller of the call that evaluates it"
        );
    });
}

#[test]
fn an_asserted_end_user_is_the_caller() {
    once(12, |rt| {
        let db = open(&rt);
        sign_up(&db, "ann");
        sign_up(&db, "bob");
        let ann = principal(&db, r#"EndUser { name: "ann" }"#);
        assert_eq!(
            vec![r#"EndUser { name: "ann" }"#],
            rows(&db, ME, &[], &asserting(ann.clone()))
        );
        assert_eq!(
            vec![r#""ann""#],
            rows(&db, MINE, &[], &asserting(ann.clone())),
            "a definer query returns the rows it selects for the caller"
        );
        assert_eq!(vec![r#""ann""#], rows(&db, WHOSE, &[], &asserting(ann)));
        assert!(
            rows(&db, MINE, &[], &CallOptions::default()).is_empty(),
            "the schema owner is no end user"
        );
    });
}

#[test]
fn an_asserted_anonymous_principal_is_the_caller() {
    once(13, |rt| {
        let db = open(&rt);
        let anon = principal(&db, "Anonymous");
        assert_eq!(
            vec!["Anonymous"],
            rows(&db, ME, &[], &asserting(anon.clone()))
        );
        assert_eq!(vec![r#""nobody""#], rows(&db, WHOSE, &[], &asserting(anon)));
    });
}

#[test]
fn asserting_a_service_principal_is_denied() {
    once(14, |rt| {
        let db = open(&rt);
        let svc = principal(&db, r#"Service { name: "owner" }"#);
        let app = db.registered("app").expect("app is registered");
        let got = app.call(ME, &[], &asserting(svc.clone()));
        assert!(
            matches!(
                got.as_ref().map_err(reason),
                Err(RejectReason::Denied { .. })
            ),
            "{got:?}"
        );
        let got = db.query_text("caller", &[], &asserting(svc.clone()));
        assert!(
            matches!(
                got.as_ref().map_err(reason),
                Err(RejectReason::Denied { .. })
            ),
            "ad hoc text: {got:?}"
        );
        let got = db.begin(&TxnOptions {
            assert: Some(svc),
            ..TxnOptions::default()
        });
        assert!(
            matches!(
                got.as_ref().map(|_| ()).map_err(reason),
                Err(RejectReason::Denied { .. })
            ),
            "a transaction: {:?}",
            got.err()
        );
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
fn ad_hoc_text_runs_for_the_handle_or_the_asserted_principal() {
    once(16, |rt| {
        let db = open(&rt);
        assert_eq!(vec![OWNER], printed(result(&db, "caller")));
        let ann = principal(&db, r#"EndUser { name: "ann" }"#);
        let mut out = db
            .query_text("caller", &[], &asserting(ann))
            .expect("ad hoc text reads caller");
        assert_eq!(vec![r#"EndUser { name: "ann" }"#], printed(out.remove(0)));
        assert_eq!(
            vec![r#""owner""#],
            printed(result(&db, "shop::whose()")),
            "a schema function called in ad hoc text sees its caller"
        );
    });
}

#[test]
fn an_interactive_transaction_runs_every_call_for_its_asserted_principal() {
    once(17, |rt| {
        let db = open(&rt);
        let app = db.registered("app").expect("app is registered");
        let ann = principal(&db, r#"EndUser { name: "ann" }"#);
        let mut t = db
            .begin(&TxnOptions {
                assert: Some(ann),
                ..TxnOptions::default()
            })
            .expect("begin");
        let mut out = t
            .query_text("caller", &[], &TxnCallOptions::default())
            .expect("caller");
        assert_eq!(vec![r#"EndUser { name: "ann" }"#], printed(out.remove(0)));
        match t.call(&app, ME, &[], &TxnCallOptions::default()) {
            Ok(InTxn::Rows(r)) => assert_eq!(vec![r#"EndUser { name: "ann" }"#], printed(r)),
            other => panic!("me returns no rows: {other:?}"),
        }
        t.rollback().expect("rollback");
    });
}

#[test]
fn a_registered_transaction_in_an_interactive_transaction_commits_with_it() {
    once(18, |rt| {
        let db = open(&rt);
        let app = db.registered("app").expect("app is registered");
        let mut t = db.begin(&TxnOptions::default()).expect("begin");
        let done = t.call(
            &app,
            SIGN_UP,
            &[text("ann"), text("4242")],
            &TxnCallOptions::default(),
        );
        assert!(matches!(done, Ok(InTxn::Pending(_))), "{done:?}");
        match t.call(&app, NAMES, &[], &TxnCallOptions::default()) {
            Ok(InTxn::Rows(r)) => assert_eq!(
                vec![r#""ann""#],
                printed(r),
                "a later call reads the pending change"
            ),
            other => panic!("names returns no rows: {other:?}"),
        }
        assert!(
            rows(&db, NAMES, &[], &CallOptions::default()).is_empty(),
            "nothing is visible before the commit"
        );
        t.commit(None).expect("commit");
        assert_eq!(
            vec![r#""ann""#],
            rows(&db, NAMES, &[], &CallOptions::default())
        );
    });
}

#[test]
fn a_rolled_back_interactive_transaction_discards_its_registered_transaction() {
    once(19, |rt| {
        let db = open(&rt);
        let app = db.registered("app").expect("app is registered");
        let mut t = db.begin(&TxnOptions::default()).expect("begin");
        t.call(
            &app,
            SIGN_UP,
            &[text("ann"), text("4242")],
            &TxnCallOptions::default(),
        )
        .expect("sign_up runs");
        t.rollback().expect("rollback");
        assert!(rows(&db, NAMES, &[], &CallOptions::default()).is_empty());
    });
}

#[test]
fn a_read_only_transaction_refuses_a_registered_transaction() {
    once(20, |rt| {
        let db = open(&rt);
        let app = db.registered("app").expect("app is registered");
        let mut t = db
            .begin(&TxnOptions {
                read_only: true,
                ..TxnOptions::default()
            })
            .expect("begin");
        let got = t.call(
            &app,
            SIGN_UP,
            &[text("ann"), text("4242")],
            &TxnCallOptions::default(),
        );
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
fn a_registration_is_rechecked_when_a_later_open_changes_the_schema() {
    let mut sim = Sim::new(21, Config::default());
    run(&mut sim, |rt| {
        let db = open(&rt);
        sign_up(&db, "ann");
    });
    run(&mut sim, |rt| {
        let next = Log::new([SHOP, "module shop\ndrop field Note.tag\n"]);
        let cards = Registration::new(
            "cards",
            "import shop\nquery tags () -> Set Text = from n in notes select n.tag\n",
        );
        let got = Database::open_in(rt, "app.adt", &next, &with(vec![cards]));
        let detail = refused(got, "cards");
        assert!(detail.contains("tags"), "{detail}");
    });
    run(&mut sim, |rt| {
        let next = Log::new([SHOP, "module shop\ndrop field Note.tag\n"]);
        let db = Database::open_in(rt, "app.adt", &next, &options(4))
            .expect("the migration before the refused registration stays applied");
        assert_eq!(
            vec![r#""ann""#],
            printed(result(&db, "from c in customers select c.name"))
        );
    });
}

#[test]
fn a_registration_survives_a_migration_it_does_not_read() {
    let mut sim = Sim::new(22, Config::default());
    run(&mut sim, |rt| sign_up(&open(&rt), "ann"));
    run(&mut sim, |rt| {
        let next = Log::new([
            SHOP,
            "module shop\nadd type Label = { id: Key tags, name: Text }\nadd relation tags : Label key id generate sequence\n",
        ]);
        let db = Database::open_in(rt, "app.adt", &next, &with(vec![app()]))
            .expect("the registration checks at the new version");
        assert_eq!(
            vec![r#""ann""#],
            rows(&db, NAMES, &[], &CallOptions::default())
        );
    });
}

#[test]
fn a_bundle_function_is_called_by_its_queries() {
    once(23, |rt| {
        let reg = Registration::new(
            "helpers",
            "import shop\nfn shout (t: Text) -> Text = t ++ \"!\"\n\
             query loud () -> Set Text = from c in customers select shout(c.name)\n",
        );
        let db = Database::open_in(rt, "app.adt", &log(), &with(vec![app(), reg]))
            .expect("a bundle function registers with the queries that call it");
        sign_up(&db, "ann");
        let helpers = db.registered("helpers").expect("helpers is registered");
        assert_eq!(
            None,
            helpers.ordinal("shout"),
            "a client may not call a bundle function"
        );
        match helpers.call(1, &[], &CallOptions::default()) {
            Ok(Called::Rows(r)) => assert_eq!(vec![r#""ann!""#], printed(r)),
            other => panic!("loud returns no rows: {other:?}"),
        }
    });
}

#[test]
fn a_bundle_function_named_as_a_schema_function_is_refused() {
    once(24, |rt| {
        let reg = Registration::new(
            "clash",
            "import shop\nfn method (t: Text) -> Text = t\n\
             query q () -> Set Text = from c in customers select method(c.name)\n",
        );
        let got = Database::open_in(rt, "app.adt", &log(), &with(vec![reg]));
        let detail = refused(got, "clash");
        assert!(detail.contains("method"), "{detail}");
    });
}

#[test]
fn a_registration_imports_a_module_it_names() {
    once(25, |rt| {
        let reg = Registration::new(
            "lost",
            "import nowhere\nquery q () -> Set Text = from c in customers select c.name\n",
        );
        let got = Database::open_in(rt, "app.adt", &log(), &with(vec![reg]));
        let detail = refused(got, "lost");
        assert!(detail.contains("nowhere"), "{detail}");
    });
}

#[test]
fn a_bundle_function_belongs_to_its_registration() {
    once(26, |rt| {
        let helpers = Registration::new(
            "helpers",
            "import shop\nfn shout (t: Text) -> Text = t ++ \"!\"\n\
             query loud () -> Set Text = from c in customers select shout(c.name)\n",
        );
        let other = Registration::new(
            "other",
            "import shop\nquery louder () -> Set Text = from c in customers select shout(c.name)\n",
        );
        let got = Database::open_in(rt, "app.adt", &log(), &with(vec![helpers, other]));
        let detail = refused(got, "other");
        assert!(detail.contains("shout"), "{detail}");
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

const MEMBERS: &str = r"module club
add module club
add type Member = { id: Key members, name: Text }
add relation members : Member key id generate sequence
add type Principal = EndUser { member: Key members } | Service { name: Text } | Anonymous
";

#[test]
fn a_declared_principal_is_what_calls_run_for() {
    once(28, |rt| {
        let reg = Registration::new(
            "club",
            "import club\nquery me () -> Principal = caller\n\
             query my_name () -> Set Text =\n  from m in members\n  where match caller\n          \
             | EndUser u -> key m == u.member\n          | Service _ -> false\n          \
             | Anonymous -> false\n  select m.name\n",
        );
        let db = Database::open_in(rt, "club.adt", &Log::new([MEMBERS]), &with(vec![reg]))
            .expect("the club opens");
        db.query_text(
            r#"insert members { name: "ann" }"#,
            &[],
            &CallOptions::default(),
        )
        .expect("ann joins");
        let ann = result(
            &db,
            r#"from m in members where m.name == "ann" select key m"#,
        )
        .next()
        .expect("ann's key")
        .expect("a key");
        let club = db.registered("club").expect("club is registered");
        let me = |opts: &CallOptions| match club.call(1, &[], opts) {
            Ok(Called::Rows(r)) => printed(r),
            other => panic!("me returns no rows: {other:?}"),
        };
        assert_eq!(vec![OWNER], me(&CallOptions::default()));
        let schema = db.schema();
        let end_user = schema
            .ty(schema.principal)
            .ctor_index("EndUser")
            .expect("the declared principal has an end user");
        let asserted = asserting(Value::ctor(end_user, vec![ann]));
        match club.call(2, &[], &asserted) {
            Ok(Called::Rows(r)) => assert_eq!(vec![r#""ann""#], printed(r)),
            other => panic!("my_name returns no rows: {other:?}"),
        }
    });
}
