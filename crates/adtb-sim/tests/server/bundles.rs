//! Bundles in process: the bundle file and its header, registration and its checks at the
//! pinned version, immutability, supersession at open, leases, retirement and supersession by
//! command, and their persistence across reopen and crash.

use adtb_simkit::api;

use std::collections::BTreeSet;
use std::time::Duration;

use adtb::catalog::version_hash;
use adtb::runtime::{Clock, Done, Io, Op, Runtime, Threads};
use adtb::{
    CallOptions, Called, Database, Error, Log, OpenError, OpenOptions, Registration, RejectReason,
    Value,
};
use adtb_lang::evolve::Source;
use adtb_lang::parse::{Options, parse_migration};
use adtb_sim::{Config, Outcome, Sim, SimRuntime};
use adtb_testkit::sections::{Section, expectations, sections};

use api::{once, options, printed, query, results, run};

const V1: &str = r"module auth
add module auth
add type Tier = Free | Paid
add type User = { id: Key users, name: Text, tier: Tier }
add relation users : User key id generate sequence
add type Identity { id: Key identities, user: Key users }
  = EmailPass { email: Text }
  | Google { gid: Text }
add relation identities : Identity key id generate sequence
add type Session = { id: Key sessions, who: Key identities }
add relation sessions : Session key id generate sequence
";

const NAMES: &str = "import auth
query names () -> Set Text = from u in users select u.name
txn add (n: Text) -> Key users =
  transact
    let u = insert users { name: n, tier: Free }
    return key u
";

fn log(migrations: &[&str]) -> Log {
    Log::new(migrations.iter().copied())
}

/// Returns the version hash of `migrations` as 64 hex digits.
fn hash(migrations: &[&str]) -> String {
    let mut source = Source::default();
    for text in migrations {
        let file = parse_migration(text, Options::default())
            .ok()
            .unwrap_or_else(|ds| panic!("`{text}` parses: {ds:?}"));
        source = source
            .migrate(&file, text)
            .unwrap_or_else(|r| panic!("`{text}` applies: {r:?}"));
    }
    let schema = source.schema().expect("the source builds");
    version_hash(&schema, &source)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Returns the text of bundle `name` pinned at the last version of `pins`, with `clauses` after
/// its edition, holding `body`.
fn file(name: &str, pins: &[&str], clauses: &str, body: &str) -> String {
    format!(
        "bundle {name} pins {} ({}) edition \"2026-10\"{clauses}\n{body}",
        pins.len(),
        hash(pins)
    )
}

fn bundle(name: &str, pins: &[&str], clauses: &str, body: &str) -> Registration {
    let text = file(name, pins, clauses, body);
    Registration::bundle(&text).unwrap_or_else(|e| panic!("`{text}` is a bundle file: {e:?}"))
}

fn with(regs: Vec<Registration>) -> OpenOptions {
    OpenOptions {
        registrations: regs,
        ..options(4)
    }
}

fn open(
    rt: &SimRuntime,
    migrations: &[&str],
    regs: Vec<Registration>,
) -> Result<Database<SimRuntime>, OpenError> {
    Database::open_in(rt.clone(), "app.adt", &log(migrations), &with(regs))
}

fn opened(rt: &SimRuntime, migrations: &[&str], regs: Vec<Registration>) -> Database<SimRuntime> {
    open(rt, migrations, regs).unwrap_or_else(|e| panic!("the database opens: {e:?}"))
}

fn reason(e: &Error) -> &RejectReason {
    match e {
        Error::Rejected { reason } => reason,
        other => panic!("not a rejection: {other:?}"),
    }
}

/// Returns the reason open refused the registration of `name` for.
fn refusal(got: Result<Database<SimRuntime>, OpenError>, name: &str) -> RejectReason {
    match got {
        Err(OpenError::RegistrationRefused { bundle, reason: e }) => {
            assert_eq!(name, bundle, "the refusal names the bundle");
            reason(&e).clone()
        }
        Err(other) => panic!("open fails otherwise: {other:?}"),
        Ok(_) => panic!("the registration of {name} is not refused"),
    }
}

/// Returns the detail of an `Invalid` refusal of the registration of `name`.
fn invalid(got: Result<Database<SimRuntime>, OpenError>, name: &str) -> String {
    match refusal(got, name) {
        RejectReason::Invalid { detail } => detail,
        other => panic!("the refusal is no Invalid: {other:?}"),
    }
}

fn call(db: &Database<SimRuntime>, name: &str, ordinal: u32, args: &[Value]) -> Called {
    db.registered(name)
        .unwrap_or_else(|e| panic!("{name} binds: {e:?}"))
        .call(ordinal, args, &CallOptions::default())
        .unwrap_or_else(|e| panic!("call {ordinal} of {name} fails: {e:?}"))
}

fn rows(called: Called) -> Vec<String> {
    match called {
        Called::Rows(r) => printed(r),
        other => panic!("the call returns no rows: {other:?}"),
    }
}

/// Returns why binding `name` is refused.
fn unbound(db: &Database<SimRuntime>, name: &str) -> RejectReason {
    match db.registered(name) {
        Ok(_) => panic!("{name} binds"),
        Err(e) => reason(&e).clone(),
    }
}

/// Returns the name and state of each row of `catalog::bundles`, sorted.
fn states(db: &Database<SimRuntime>) -> Vec<String> {
    query(
        db,
        r#"from b in catalog::bundles select b.name ++ " " ++ (match b.state
  | Active -> "active"
  | Superseded _ -> "superseded"
  | Retired _ -> "retired")"#,
    )
    .into_iter()
    .map(|s| s.trim_matches('"').to_owned())
    .collect()
}

/// Advance the simulated clock by `d`.
fn sleep(rt: &SimRuntime, d: Duration) {
    let mut io = rt.io();
    let ticket = io.submit(Op::Timer { at: rt.now() + d });
    loop {
        if let Some(c) = io.poll() {
            assert_eq!(ticket, c.ticket);
            assert_eq!(Ok(Done::Fired), c.result);
            return;
        }
        rt.park();
    }
}

const HOUR: Duration = Duration::from_secs(3600);
const DAY: Duration = Duration::from_secs(86_400);

fn write(db: &Database<SimRuntime>, text: &str) -> Result<(), Error> {
    db.query_text(text, &[], &CallOptions::default()).map(drop)
}

/// Returns the name and state of each bundle of the file `sim` holds, opened with no bundles.
fn reopened_states(seed: u64, files: std::collections::BTreeMap<String, Vec<u8>>) -> Vec<String> {
    let mut sim = Sim::with_files(seed, Config::default(), files);
    run(&mut sim, |rt| states(&opened(&rt, &[V1], Vec::new())))
}

#[test]
fn a_registration_and_a_retirement_survive_a_crash() {
    let mut sim = Sim::new(44, Config::default());
    run(&mut sim, |rt| {
        opened(&rt, &[V1], Vec::new());
    });
    sim.crash_at(sim.now() + Duration::from_secs(1));
    let outcome = sim.run(|rt| {
        let db = opened(&rt, &[V1], vec![bundle("web-1.0", &[V1], "", NAMES)]);
        db.query_text("retire bundle web-1.0", &[], &CallOptions::default())
            .expect("web-1.0 retires");
        sleep(&rt, Duration::from_secs(10));
        drop(db);
    });
    assert_eq!(Outcome::Crashed, outcome);
    run(&mut sim, |rt| {
        let db = opened(&rt, &[V1], Vec::new());
        assert_eq!(vec!["web-1.0 retired"], states(&db));
        assert_eq!(
            vec!["2"],
            query(&db, "count(r for r in catalog::registered)")
        );
    });
}

#[test]
fn a_crash_during_an_open_with_bundles_reopens_to_the_bundle_recorded_or_not_at_all() {
    let seed = 45;
    let mut sim = Sim::new(seed, Config::default());
    sim.record();
    run(&mut sim, |rt| {
        opened(&rt, &[V1], Vec::new());
    });
    let start = sim.recording().len();
    run(&mut sim, |rt| {
        let db = opened(&rt, &[V1], vec![bundle("web-1.0", &[V1], "", NAMES)]);
        db.query_text("retire bundle web-1.0", &[], &CallOptions::default())
            .expect("web-1.0 retires");
    });
    let recording = sim.recording();
    let n = recording.len();
    let mut seen = Vec::new();
    for at in (start..n).step_by((n - start).div_ceil(12).max(1)) {
        let files = recording.images(at, 1, seed + at as u64).swap_remove(0);
        let got = reopened_states(seed + at as u64, files);
        assert!(
            got.is_empty() || got == ["web-1.0 superseded"] || got == ["web-1.0 retired"],
            "at event {at}: {got:?}"
        );
        seen.push(got);
    }
    let files = recording.images(n, 1, seed).swap_remove(0);
    assert_eq!(vec!["web-1.0 retired"], reopened_states(seed, files));
    let rank = |s: &Vec<String>| match s.first().map(String::as_str) {
        None => 0,
        Some("web-1.0 superseded") => 1,
        Some(_) => 2,
    };
    assert!(
        seen.windows(2).all(|w| rank(&w[0]) <= rank(&w[1])),
        "a later event reopens to an earlier state: {seen:?}"
    );
}

#[test]
fn admission_principal_ordinal_and_argument_refusals_leave_renewal_unchanged() {
    once(61, |rt| {
        let db = opened(
            &rt,
            &[V1],
            vec![bundle("web-1.0", &[V1], " lease 1d", NAMES)],
        );
        let web = db.registered("web-1.0").expect("web binds");
        let renewed = || query(&db, "from b in catalog::bundles select b.renewed");
        let before = renewed();
        sleep(&rt, 4 * HOUR);
        for ordinal in [0, 3, u32::MAX] {
            let got = web.call(ordinal, &[], &CallOptions::default());
            assert!(
                matches!(
                    got,
                    Err(Error::Rejected {
                        reason: RejectReason::UnknownQuery
                    })
                ),
                "{got:?}"
            );
            assert_eq!(
                before,
                renewed(),
                "unknown ordinal {ordinal} writes no renewal"
            );
        }
        let service = db
            .schema()
            .ty(db.schema().principal)
            .ctor_index("Service")
            .expect("Service exists");
        for (assertion, denied) in [
            (Value::Int(1), false),
            (Value::ctor(service, vec![Value::text("owner")]), true),
        ] {
            let opts = CallOptions {
                assert: Some(assertion),
                ..CallOptions::default()
            };
            let got = web.call(1, &[], &opts);
            assert!(
                matches!(
                    got,
                    Err(Error::Rejected {
                        reason: RejectReason::Denied { .. }
                    })
                ) == denied,
                "{got:?}"
            );
            if !denied {
                assert!(
                    matches!(
                        got,
                        Err(Error::Rejected {
                            reason: RejectReason::Invalid { .. }
                        })
                    ),
                    "{got:?}"
                );
            }
            assert_eq!(before, renewed(), "a refused principal writes no renewal");
        }
        let got = web.call_arrow(1, &[Value::Int(1)], &CallOptions::default());
        assert!(
            matches!(
                got,
                Err(Error::Rejected {
                    reason: RejectReason::Invalid { .. }
                })
            ),
            "{got:?}"
        );
        assert_eq!(before, renewed(), "a refused Arrow call writes no renewal");
        let got = web.call_arrow(2, &[Value::Int(1)], &CallOptions::default());
        assert!(
            matches!(
                got,
                Err(Error::Rejected {
                    reason: RejectReason::Invalid { .. }
                })
            ),
            "{got:?}"
        );
        assert_eq!(before, renewed(), "a refused value writes no renewal");
        assert!(query(&db, "from u in auth::users select u.name").is_empty());
        web.call(1, &[], &CallOptions::default())
            .expect("a valid call runs");
        assert_ne!(before, renewed(), "a valid call renews the same handle");
    });
}

fn admission_options_refusal(opts: &CallOptions, expected: &RejectReason) {
    once(63, |rt| {
        let db = opened(
            &rt,
            &[V1],
            vec![bundle("web-1.0", &[V1], " lease 1d", NAMES)],
        );
        let web = db.registered("web-1.0").expect("web binds");
        let renewed = || query(&db, "from b in catalog::bundles select b.renewed");
        let before = renewed();
        sleep(&rt, 4 * HOUR);
        let got = web.call(2, &[Value::text("refused")], opts);
        assert_eq!(Some(expected), got.as_ref().err().map(reason), "{got:?}");
        assert_eq!(before, renewed(), "a refused call option writes no renewal");
        let got = web.call_arrow(2, &[Value::text("refused")], opts);
        assert_eq!(Some(expected), got.as_ref().err().map(reason), "{got:?}");
        assert_eq!(
            before,
            renewed(),
            "a refused Arrow option writes no renewal"
        );
        assert!(query(&db, "from u in auth::users select u.name").is_empty());
        web.call(1, &[], &CallOptions::default())
            .expect("valid call runs");
        assert_ne!(before, renewed(), "valid call renews");
    });
}

#[test]
fn admission_cancelled_call_does_not_renew() {
    let token = adtb::CancelToken::new();
    token.cancel();
    admission_options_refusal(
        &CallOptions {
            cancel: Some(token),
            ..CallOptions::default()
        },
        &RejectReason::Cancelled,
    );
}

#[test]
fn admission_expired_deadline_does_not_renew() {
    admission_options_refusal(
        &CallOptions {
            deadline: Some(std::time::SystemTime::UNIX_EPOCH),
            ..CallOptions::default()
        },
        &RejectReason::DeadlineExceeded,
    );
}

#[test]
fn admission_invalid_bookmark_does_not_renew() {
    let position = adtb::Position {
        epoch: u64::MAX,
        lsn: u64::MAX,
    };
    admission_options_refusal(
        &CallOptions {
            after: Some(position),
            ..CallOptions::default()
        },
        &RejectReason::PositionNotOnHistory { position },
    );
}

#[test]
fn admission_interactive_stops_do_not_renew() {
    once(64, |rt| {
        let db = opened(
            &rt,
            &[V1],
            vec![bundle("web-1.0", &[V1], " lease 1d", NAMES)],
        );
        let web = db.registered("web-1.0").expect("web binds");
        let renewed = || query(&db, "from b in catalog::bundles select b.renewed");
        let before = renewed();
        let mut t = db.begin(&adtb::TxnOptions::default()).expect("begin");
        sleep(&rt, 4 * HOUR);
        let token = adtb::CancelToken::new();
        token.cancel();
        for (opts, expected) in [
            (
                adtb::TxnCallOptions {
                    cancel: Some(token),
                    ..adtb::TxnCallOptions::default()
                },
                RejectReason::Cancelled,
            ),
            (
                adtb::TxnCallOptions {
                    deadline: Some(rt.wall()),
                    ..adtb::TxnCallOptions::default()
                },
                RejectReason::DeadlineExceeded,
            ),
        ] {
            let got = t.call(&web, 2, &[Value::text("refused")], &opts);
            assert_eq!(Some(&expected), got.as_ref().err().map(reason), "{got:?}");
            assert_eq!(
                before,
                renewed(),
                "a refused interactive option writes no renewal"
            );
        }
        let got = t
            .call(&web, 1, &[], &adtb::TxnCallOptions::default())
            .expect("a stopped call leaves the transaction open");
        let adtb::InTxn::Rows(rows) = got else {
            panic!("names returns rows")
        };
        assert!(printed(rows).is_empty());
        assert_ne!(before, renewed(), "a valid call renews");
        t.commit(None).expect("commit");
    });
}

#[test]
fn admission_expired_transaction_does_not_renew() {
    once(65, |rt| {
        let db = opened(
            &rt,
            &[V1],
            vec![bundle("web-1.0", &[V1], " lease 1d", NAMES)],
        );
        let web = db.registered("web-1.0").expect("web binds");
        let renewed = || query(&db, "from b in catalog::bundles select b.renewed");
        let before = renewed();
        let mut t = db
            .begin(&adtb::TxnOptions {
                deadline: Some(rt.wall() + HOUR),
                ..adtb::TxnOptions::default()
            })
            .expect("begin");
        sleep(&rt, 4 * HOUR);
        let got = t.call(
            &web,
            2,
            &[Value::text("refused")],
            &adtb::TxnCallOptions::default(),
        );
        assert_eq!(
            Some(&RejectReason::DeadlineExceeded),
            got.as_ref().err().map(reason),
            "{got:?}"
        );
        assert_eq!(
            before,
            renewed(),
            "an expired transaction writes no renewal"
        );
        assert!(query(&db, "from u in auth::users select u.name").is_empty());
        t.rollback().expect("rollback");
    });
}

#[test]
fn admission_ended_transaction_does_not_renew() {
    once(66, |rt| {
        let db = opened(
            &rt,
            &[V1],
            vec![bundle("web-1.0", &[V1], " lease 1d", NAMES)],
        );
        let web = db.registered("web-1.0").expect("web binds");
        web.call(2, &[Value::text("first")], &CallOptions::default())
            .expect("seed a row");
        let renewed = || query(&db, "from b in catalog::bundles select b.renewed");
        let before = renewed();
        let mut t = db.begin(&adtb::TxnOptions::default()).expect("begin");
        let updated = db
            .query_text(
                r#"update u in auth::users set auth::User { u with name: "new" }"#,
                &[],
                &CallOptions::default(),
            )
            .expect("the concurrent writer commits");
        assert!(updated.is_empty(), "an update returns no result");
        let failed = t
            .query_text(
                r#"update u in auth::users set auth::User { u with name: "old" }"#,
                &[],
                &adtb::TxnCallOptions::default(),
            )
            .expect_err("the old writer fails serialization");
        assert!(
            matches!(
                failed,
                Error::Retry {
                    reason: adtb::RetryReason::Serialization { .. }
                }
            ),
            "{failed:?}"
        );
        sleep(&rt, 4 * HOUR);
        let got = t.call(&web, 1, &[], &adtb::TxnCallOptions::default());
        assert_eq!(
            Some(&failed),
            got.as_ref().err(),
            "the transaction retains its original error"
        );
        assert_eq!(before, renewed(), "an ended transaction writes no renewal");
        assert_eq!(
            vec![r#""new""#],
            query(&db, "from u in auth::users select u.name")
        );
        t.rollback().expect("rollback");
    });
}

#[test]
fn admission_closed_database_refuses_interactive_call_before_renewal() {
    once(67, |rt| {
        let db = opened(
            &rt,
            &[V1],
            vec![bundle("web-1.0", &[V1], " lease 1d", NAMES)],
        );
        let web = db.registered("web-1.0").expect("web binds");
        let mut t = db.begin(&adtb::TxnOptions::default()).expect("begin");
        sleep(&rt, 4 * HOUR);
        db.clone().close().expect("close the database");
        let got = t.call(&web, 1, &[], &adtb::TxnCallOptions::default());
        assert_eq!(
            Some(&RejectReason::Closed),
            got.as_ref().err().map(reason),
            "{got:?}"
        );
        t.rollback().expect("rollback after close");
    });
}

/// Returns the detail and operator of the refusal of the last migration of `migrations`.
fn refused_migration(
    got: Result<Database<SimRuntime>, OpenError>,
    version: u64,
) -> (String, String) {
    match got {
        Err(OpenError::MigrationRefused { version: v, reason }) => {
            assert_eq!(version, v, "the refused migration");
            match reason {
                Error::Rejected {
                    reason: RejectReason::SchemaRefused { operator, detail },
                } => (operator, detail),
                other => panic!("the migration is refused otherwise: {other:?}"),
            }
        }
        Err(other) => panic!("open fails otherwise: {other:?}"),
        Ok(_) => panic!("migration {version} applies"),
    }
}

const ADD_NOTES: &str = "module auth
add type Note { id: Key notes } = Plain { body: Text }
add relation notes : Note key id generate sequence
";
const ADD_LINK: &str = "module auth\nadd constructor Note.Link { url: Text }\n";

const ENABLE_LINK: &str = "module auth\nenable notes.Link\n";
const ADD_LINK_BODY: &str = "module auth\nadd constructor Note.Link { url: Text, body: Text }\n";

fn link_stale() -> RejectReason {
    RejectReason::BundleStale {
        bundle: "web-1.0".into(),
        ctor: "notes.Link".into(),
    }
}

/// Returns the migration adding `Link` of the fixture's `kind`, the write of a `Link` and its print.
fn stored_link(kind: &str) -> (&'static str, &'static str, &'static str) {
    match kind {
        "url" => (
            ADD_LINK,
            r#"insert notes.Link { id: notes.key(1), url: "u" }"#,
            r#"Link { id: notes.key(1), url: "u" }"#,
        ),
        "body" => (
            ADD_LINK_BODY,
            r#"insert notes.Link { id: notes.key(1), url: "u", body: "theirs" }"#,
            r#"Link { id: notes.key(1), url: "u", body: "theirs" }"#,
        ),
        other => panic!("a link is url or body, not {other}"),
    }
}

/// The branches a default run calls under `hide new constructors`.
const ALWAYS_HIDDEN: [&str; 6] = [
    "delete",
    "new",
    "target-only",
    "filter-on-new",
    "with-copy",
    "kept-returned",
];

/// Assert each conflict branch and use of its result, with and without `hide new constructors`,
/// while live and at registration: a branch naming `old`, or a use of a result that can hold the
/// stored row, in a bundle that does not hide refuses `enable notes.Link` and is stale past it; any
/// other opens and runs over a stored `Link` to its result and written rows.
#[test]
fn a_conflict_branch_naming_old_reads_the_relation_unless_new_constructors_are_hidden() {
    let refused = "`enable notes.Link` is refused: bundle web-1.0, pinned at version 3, reads notes whole; a bundle stops blocking only when it is released";
    const ADD_REJECTED: &str =
        "module auth\nadd relation rejected : RejectedRow key (load, line)\n";
    let cases = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/pinned-readers/conflict-branches.txt"),
    )
    .expect("the conflict branches are readable");
    let pin = [V1, ADD_NOTES, ADD_REJECTED];
    let mut reached = BTreeSet::new();
    let every = adtb_testkit::profile::every_case();
    let mut seed = 600;
    for line in cases.lines() {
        let [
            name,
            uses,
            branch,
            after,
            link,
            bound,
            source,
            ty,
            rest,
            value,
        ] = line.split('\t').collect::<Vec<_>>()[..]
        else {
            panic!(
                "a branch is name, use, text, rows, link, binding, source, type, rest, value: {line}"
            );
        };
        let (add, stored, link) = stored_link(link);
        let before = [V1, ADD_NOTES, ADD_REJECTED, add];
        let log = [V1, ADD_NOTES, ADD_REJECTED, add, ENABLE_LINK];
        let binding = match bound {
            "-" => String::new(),
            b => format!("let {b} = "),
        };
        let source = match source {
            "-" => r#"{ id: notes.key(1), body: "new" }"#,
            s => s,
        };
        let rest = rest.replace("; ", "\n    ");
        let text = format!(
            "import auth\nfn pass (n: Note) -> Note = n\ntxn put () -> {ty} =\n  transact\n    {binding}insert notes.Plain {source}\n      unless conflict on key else {branch}\n    {rest}\n"
        );
        let blocks = match uses {
            "uses" => true,
            "free" => false,
            other => panic!("{name}: use is uses or free, not {other}"),
        };
        for (clauses, hides) in [("", false), (" hide new constructors", true)] {
            if hides && !every && !ALWAYS_HIDDEN.contains(&name) {
                continue;
            }
            let registered = || vec![bundle("web-1.0", &pin, clauses, &text)];
            let runs = |db: &Database<SimRuntime>| {
                write(db, stored).expect("the link is written");
                write(db, r#"insert users { name: "a", tier: Free }"#)
                    .expect("the source row is written");
                let got = db
                    .registered("web-1.0")
                    .expect("the bundle binds")
                    .call(1, &[], &CallOptions::default())
                    .map(|c| match c {
                        Called::Rows(r) => printed(r),
                        Called::Committed(c) => printed(c.value),
                    })
                    .map_err(|e| reason(&e).clone());
                let expected = if hides {
                    (
                        Err(RejectReason::KeyViolation {
                            relation: "auth::notes".into(),
                            key: "notes.key(1)".into(),
                        }),
                        vec![link.to_owned()],
                    )
                } else {
                    (
                        value.strip_prefix('!').map_or_else(
                            || Ok(value.split(';').map(str::to_owned).collect()),
                            |message| {
                                Err(RejectReason::UserError {
                                    message: message.to_owned(),
                                })
                            },
                        ),
                        after
                            .split(';')
                            .filter(|r| !r.is_empty())
                            .map(str::to_owned)
                            .collect(),
                    )
                };
                assert_eq!(
                    expected,
                    (got, query(db, "from n in notes select n")),
                    "{name}{clauses}"
                );
            };
            seed += 1;
            if every || bound == "-" || name.ends_with("-returned") {
                let mut sim = Sim::new(seed, Config::default());
                run(&mut sim, |rt| {
                    opened(&rt, &before, registered());
                });
                run(&mut sim, |rt| {
                    let got = open(&rt, &log, registered());
                    if blocks && !hides {
                        assert_eq!(
                            refused,
                            refused_migration(got, 5).1,
                            "{name}{clauses} while live"
                        );
                        reached.insert((name, hides, "refused"));
                    } else {
                        runs(&got.unwrap_or_else(|e| {
                            panic!("{name}{clauses}: enable passes while live: {e:?}")
                        }));
                        reached.insert((name, hides, "ran"));
                    }
                });
            }
            seed += 1;
            once(seed, |rt| {
                let got = open(&rt, &log, registered());
                if blocks && !hides {
                    assert_eq!(link_stale(), refusal(got, "web-1.0"), "{name}{clauses}");
                    reached.insert((name, hides, "stale"));
                } else {
                    runs(&got.unwrap_or_else(|e| {
                        panic!("{name}{clauses}: the registration passes: {e:?}")
                    }));
                    reached.insert((name, hides, "registered"));
                }
            });
        }
    }
    let (using, free) = (
        cases.lines().filter(|l| l.contains("\tuses\t")).count(),
        cases.lines().filter(|l| l.contains("\tfree\t")).count(),
    );
    assert_eq!((36, 3), (using, free), "the branches of the fixture");
    assert_eq!(
        if every { 156 } else { 66 },
        reached.len(),
        "each selected branch reached its two outcomes under each header"
    );
}

#[test]
fn a_hiding_pin_handles_known_unique_conflicts_and_refuses_hidden_ones() {
    let add = "module auth\nadd type Note { id: Key notes, slug: Text } = Plain { body: Text }\nadd relation notes : Note key id generate sequence unique (slug)\n";
    let body = r#"import auth
txn put () -> I64 =
  transact
    insert notes.Plain { id: notes.key(1), slug: "same", body: "x" }
      unless conflict on (slug) else update old set Plain { old with body: new.body }
    return 0
"#;
    let pin = [V1, add];
    let log = [V1, add, ADD_LINK, ENABLE_LINK];
    let mut ran = 0;
    for hidden in [false, true] {
        once(196, |rt| {
            let db = opened(
                &rt,
                &log,
                vec![bundle("web-1.0", &pin, " hide new constructors", body)],
            );
            let (insert, expected_rows) = if hidden {
                (
                    r#"insert notes.Link { id: notes.key(9), slug: "same", url: "u" }"#,
                    vec![r#"Link { id: notes.key(9), slug: "same", url: "u" }"#],
                )
            } else {
                (
                    r#"insert notes.Plain { id: notes.key(9), slug: "same", body: "a" }"#,
                    vec![r#"Plain { id: notes.key(9), slug: "same", body: "x" }"#],
                )
            };
            write(&db, insert).expect("the conflicting row is written");
            let mut got = db
                .registered("web-1.0")
                .expect("the bundle binds")
                .call(1, &[], &CallOptions::default())
                .map(drop)
                .map_err(|e| reason(&e).clone());
            if let Err(RejectReason::Violated { report, .. }) = &mut got {
                report.sort();
            }
            let expected = if hidden {
                let plain = Value::ctor(
                    0,
                    vec![Value::Int(1), Value::text("same"), Value::text("x")],
                );
                let link = Value::ctor(
                    1,
                    vec![Value::Int(9), Value::text("same"), Value::text("u")],
                );
                let mut report = vec![
                    Value::record(vec![plain.clone(), link.clone()]),
                    Value::record(vec![link, plain]),
                ];
                report.sort();
                Err(RejectReason::Violated {
                    constraint: "auth::notes.unique(slug)".into(),
                    report,
                    redacted: 0,
                })
            } else {
                Ok(())
            };
            assert_eq!(expected, got, "hidden {hidden}");
            assert_eq!(
                expected_rows,
                query(&db, "from n in notes select n"),
                "hidden {hidden}"
            );
        });
        ran += 1;
    }
    assert_eq!(2, ran);
}

#[test]
fn a_hidden_conflict_uses_the_ordinary_least_constraint_refusal() {
    let check =
        "module auth\nadd constraint aaa = from n in notes.Plain where n.body == \"x\" select n\n";
    let body = r#"import auth
txn put () -> I64 =
  transact
    insert notes.Plain { id: notes.key(1), body: "x" }
      unless conflict on key else fail "the conflict branch ran"
    return 0
"#;
    let pin = [V1, ADD_NOTES];
    let log = [V1, ADD_NOTES, ADD_LINK, ENABLE_LINK, check];
    once(198, |rt| {
        let db = opened(
            &rt,
            &log,
            vec![bundle("web-1.0", &pin, " hide new constructors", body)],
        );
        write(&db, r#"insert notes.Link { id: notes.key(1), url: "u" }"#)
            .expect("the hidden row is written");
        let got = db
            .registered("web-1.0")
            .expect("the bundle binds")
            .call(1, &[], &CallOptions::default())
            .map(drop)
            .map_err(|e| reason(&e).clone());
        assert_eq!(
            (
                Err(RejectReason::Violated {
                    constraint: "auth::aaa".into(),
                    report: vec![Value::ctor(0, vec![Value::Int(1), Value::text("x")])],
                    redacted: 0,
                }),
                vec![r#"Link { id: notes.key(1), url: "u" }"#.to_owned()],
            ),
            (got, query(&db, "from n in notes select n"))
        );
    });
}

#[test]
fn pinned_fields_follow_each_surviving_owner_identity() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/pinned-fields");
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .expect("the pinned field cases are readable")
        .map(|entry| entry.expect("a case entry is readable").path())
        .collect();
    paths.sort();
    assert_eq!(paths.len(), 6, "the owner matrix is complete");
    let mut ran = 0;
    for path in paths {
        let text = std::fs::read_to_string(&path).expect("the case is readable");
        let parts: Vec<_> = text.trim_end().split("\n===\n").collect();
        assert_eq!(parts.len(), 5, "{}: all case sections", path.display());
        let pin = [parts[0]];
        let mut log = pin.to_vec();
        log.extend(parts[1].split("\n+++\n"));
        once(211, |rt| {
            let db = opened(&rt, &pin, vec![]);
            write(&db, parts[3]).expect("the pinned rows are written before evolution");
            drop(db);
            let db = open(&rt, &log, vec![bundle("fields-1.0", &pin, "", parts[2])])
                .unwrap_or_else(|e| {
                    panic!("{}: the evolved database opens: {e:?}", path.display())
                });
            let mut got = rows(call(&db, "fields-1.0", 1, &[]));
            got.sort();
            let expected: Vec<_> = parts[4].lines().collect();
            assert_eq!(got, expected, "{}: pinned field values", path.display());
        });
        ran += 1;
    }
    assert_eq!(ran, 6, "every owner case ran");
}

/// Write each row of the `rows` part of `s`.
fn key_rows(s: &Section, db: &Database<SimRuntime>, case: &str) {
    for line in s.part("rows").lines() {
        write(db, line).unwrap_or_else(|e| panic!("{case}: `{line}` writes: {e:?}"));
    }
}

fn key_file(name: &str) -> String {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../adtb-lang/tests/pinned-keys");
    std::fs::read_to_string(dir.join(name)).expect("the pinned key search is readable")
}

/// Returns what a call of callable `ordinal` of `web-1.0` gives, then what `observe` reads.
fn key_answer(db: &Database<SimRuntime>, ordinal: u32, observe: &str) -> String {
    let called = db
        .registered("web-1.0")
        .and_then(|r| r.call(ordinal, &[], &CallOptions::default()));
    let answer = match called {
        Ok(Called::Rows(r)) => format!("call {:?}", printed(r)),
        Ok(Called::Committed(c)) => format!("call {:?}", printed(c.value)),
        Err(e) => format!("fails {:?}", reason(&e)),
    };
    format!("{answer} rows {:?}", query(db, observe))
}

/// Returns the answer of each of `bodies`, called in turn, over the rows of `rel` written at its
/// pin, then after `op` with its rows when one is given; or the refusal of `op`.
fn key_answers(seed: u64, rel: &Section, op: Option<&Section>, bodies: &[&Section]) -> Vec<String> {
    let pin = [rel.text.as_str()];
    let text: String = bodies.iter().map(|b| b.text.as_str()).collect();
    let bundled = || vec![bundle("web-1.0", &pin, "", &format!("import auth\n{text}"))];
    let case = format!("{} seed {seed}", rel.name[0]);
    let mut sim = Sim::new(seed, Config::default());
    let pinned = run(&mut sim, |rt| {
        let db = opened(&rt, &pin, bundled());
        key_rows(rel, &db, &case);
        op.is_none().then(|| {
            (1..=bodies.len() as u32)
                .map(|i| key_answer(&db, i, rel.part("observe")))
                .collect()
        })
    });
    if let Some(answers) = pinned {
        return answers;
    }
    let op = op.expect("an operator follows the pin");
    let log = [rel.text.as_str(), op.text.as_str()];
    run(&mut sim, |rt| match open(&rt, &log, bundled()) {
        Ok(db) => {
            key_rows(op, &db, &case);
            (1..=bodies.len() as u32)
                .map(|i| key_answer(&db, i, rel.part("observe")))
                .collect()
        }
        got => {
            let (operator, detail) = refused_migration(got, 2);
            vec![format!("refused {operator:?} {detail:?}")]
        }
    })
}

/// The answers of the pinned key search through the engine: every body at its pin, every body
/// after each operator that translates them all, and the pairs `answers.txt` names after an
/// operator that refuses some.
#[test]
fn a_pinned_key_answers_at_its_pin_and_after_each_operator_it_crosses() {
    let sections = sections(&key_file("search.txt"));
    let translations = expectations(&key_file("translations.txt"));
    let expected = expectations(&key_file("answers.txt"));
    let mut got = std::collections::BTreeMap::new();
    let mut seed = 400;
    let mut runs = (0, 0, 0);
    for rel in sections.iter().filter(|s| s.kind == "relation") {
        let name = &rel.name[0];
        let of = |kind: &str| -> Vec<&Section> {
            sections
                .iter()
                .filter(|s| s.kind == kind && s.name[0] == *name)
                .collect()
        };
        let bodies = of("body");
        seed += 1;
        for (b, answer) in bodies.iter().zip(key_answers(seed, rel, None, &bodies)) {
            got.insert(format!("{name} pin {}", b.name[1]), answer);
        }
        runs.0 += 1;
        for op in of("operator") {
            let pair = |b: &Section| format!("{name} {} {}", op.name[1], b.name[1]);
            seed += 1;
            if bodies.iter().all(|b| {
                translations
                    .get(&pair(b))
                    .is_some_and(|t| t == "translates")
            }) {
                for (b, answer) in bodies.iter().zip(key_answers(seed, rel, Some(op), &bodies)) {
                    got.insert(pair(b), answer);
                }
                runs.1 += 1;
                continue;
            }
            for b in bodies.iter().filter(|b| expected.contains_key(&pair(b))) {
                let answer = key_answers(seed, rel, Some(op), &[b]).remove(0);
                got.insert(pair(b), answer);
                runs.2 += 1;
            }
        }
    }
    let differs: Vec<String> = got
        .iter()
        .filter(|(k, v)| expected.get(*k) != Some(v))
        .map(|(k, v)| format!("{k} = {v}"))
        .collect();
    assert!(
        differs.is_empty(),
        "answers differ:\n{}",
        differs.join("\n")
    );
    assert_eq!(
        (2, 6, 6),
        runs,
        "every pin, every crossed operator and each named pair ran"
    );
    assert_eq!(
        expected.keys().collect::<Vec<_>>(),
        got.keys().collect::<Vec<_>>(),
        "every expectation is reached"
    );
}

fn revert_file(name: &str) -> String {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../adtb-lang/tests/pinned-revert");
    std::fs::read_to_string(dir.join(name)).expect("the pinned revert search is readable")
}

/// The bodies of one bundle, pinned after operator `op` of the revert search.
struct Pinned<'s> {
    base: &'s Section,
    op: &'s Section,
    bodies: Vec<&'s Section>,
}

impl Pinned<'_> {
    /// Returns the migrations of the pin, `then` after them.
    fn log(&self, then: &[&str]) -> Vec<String> {
        let mut log = vec![self.base.text.clone()];
        let op = migrations(&self.op.text);
        log.extend(op.iter().chain(then).map(|m| format!("module auth\n{m}\n")));
        log
    }

    fn bundled(&self) -> Vec<Registration> {
        let text: String = self.bodies.iter().map(|b| b.text.as_str()).collect();
        let pin = self.log(&[]);
        let pin: Vec<&str> = pin.iter().map(String::as_str).collect();
        vec![bundle("web-1.0", &pin, "", &format!("import auth\n{text}"))]
    }

    /// Returns the files of a database at the pin holding the operator's rows.
    fn files(&self, seed: u64) -> std::collections::BTreeMap<String, Vec<u8>> {
        let mut sim = Sim::new(seed, Config::default());
        run(&mut sim, |rt| {
            let case = format!("{} seed {seed}", self.op.name[0]);
            let pin = self.log(&[]);
            let pin: Vec<&str> = pin.iter().map(String::as_str).collect();
            key_rows(self.op, &opened(&rt, &pin, self.bundled()), &case);
        });
        sim.files()
    }

    /// Returns the answer of each body called once `then`, migrations each without its
    /// `module auth` line, applies to `files`.
    fn crossed(
        &self,
        seed: u64,
        files: std::collections::BTreeMap<String, Vec<u8>>,
        then: &[&str],
    ) -> Vec<String> {
        let log = self.log(then);
        let log: Vec<&str> = log.iter().map(String::as_str).collect();
        let mut sim = Sim::with_files(seed, Config::default(), files);
        run(&mut sim, |rt| {
            let db = opened(&rt, &log, self.bundled());
            (1..=self.bodies.len() as u32)
                .map(|i| key_answer(&db, i, self.op.part("observe")))
                .collect()
        })
    }
}

/// Every body pinned after an operator the search observes, across the revert of that operator
/// and across its inverse written as migrations, or the stand-in an `oracle` part marks: the same
/// answer.
#[test]
fn a_pinned_call_answers_across_a_revert_as_across_its_inverse_or_a_marked_stand_in() {
    let sections = sections(&revert_file("search.txt"));
    let translations = expectations(&revert_file("translations.txt"));
    let expected = expectations(&revert_file("answers.txt"));
    let base = sections
        .iter()
        .find(|s| s.kind == "base")
        .expect("the search has a base");
    let mut got = std::collections::BTreeMap::new();
    let mut seed = 600;
    for op in sections.iter().filter(|s| s.kind == "operator") {
        let name = &op.name[0];
        let bodies: Vec<&Section> = sections
            .iter()
            .filter(|s| s.kind == "body" && s.name[0] == *name)
            .collect();
        if op.part("observe").is_empty() {
            continue;
        }
        for b in &bodies {
            let pair = format!("{name} {}", b.name[1]);
            assert_eq!(
                Some("translates"),
                translations.get(&pair).map(String::as_str),
                "{pair}"
            );
        }
        let pinned = Pinned { base, op, bodies };
        let reverted = format!("revert {:04}", pinned.log(&[]).len());
        let revert = match op.part("revert") {
            "" => reverted.as_str(),
            given => given,
        };
        seed += 1;
        let files = pinned.files(seed);
        for (side, then) in [
            ("inverse", migrations(op.part("inverse"))),
            ("revert", migrations(revert)),
        ] {
            let answers = pinned.crossed(seed, files.clone(), &then);
            for (b, answer) in pinned.bodies.iter().zip(answers) {
                got.insert(format!("{name} {} {side}", b.name[1]), answer);
            }
        }
    }
    let differs: Vec<String> = got
        .iter()
        .filter(|(k, v)| {
            let pair = k.rsplit_once(' ').expect("a side ends the key").0;
            expected.get(pair) != Some(*v)
        })
        .map(|(k, v)| format!("{k} = {v}"))
        .collect();
    assert!(
        differs.is_empty(),
        "answers differ:\n{}",
        differs.join("\n")
    );
    let reached: Vec<String> = expected
        .keys()
        .flat_map(|k| [format!("{k} inverse"), format!("{k} revert")])
        .collect();
    assert_eq!(
        reached,
        got.keys().cloned().collect::<Vec<_>>(),
        "every pair is reached on both sides"
    );
    assert_eq!(2, expected.len(), "every observed pair has an answer");
}

/// Returns the migrations of `text`, each without its `module auth` line.
fn migrations(text: &str) -> Vec<&str> {
    text.split("module auth\n")
        .filter(|m| !m.is_empty())
        .collect()
}

const READER_ADD: &str = "module auth
add type Tone = Ink { depth: I64 }
add type Mark = { id: Key marks, note: Text, tone: Tone }
add relation marks : Mark key id generate sequence
add type Pad = { id: Key pads, mark: Key marks }
add relation pads : Pad key id generate sequence
";
const READER_GROW: &str = "module auth\nadd constructor Tone.Dye { hue: Text }\n";
const READER_ENABLE: &str = "module auth\nenable Tone.Dye\n";
const READER_REFUSED: &str = "`enable Tone.Dye` is refused: bundle web-1.0, pinned at version 2, reads a field whose type reaches Tone; a bundle stops blocking only when it is released";
const READER_TONES: &str =
    r#"from m in marks select match m.tone | Ink i -> "ink" | Dye d -> d.hue"#;

/// Write the mark and pad every pinned reader case reads, its mark's tone being `tone`.
fn reader_rows(db: &Database<SimRuntime>, tone: &str) {
    write(
        db,
        &format!(r#"insert marks {{ id: marks.key(1), note: "n", tone: {tone} }}"#),
    )
    .expect("the mark is written");
    write(db, "insert pads { id: pads.key(1), mark: marks.key(1) }").expect("the pad is written");
}

/// Returns what a call of the bundle's only callable gives, printed.
fn reader_called(db: &Database<SimRuntime>) -> Vec<String> {
    match call(db, "web-1.0", 1, &[]) {
        Called::Rows(r) => printed(r),
        Called::Committed(c) => printed(c.value),
    }
}

/// What a generated reader gives over the row it reads.
#[derive(Clone, Copy)]
struct ReaderOutcome<'a> {
    called: &'a str,
    stored: &'a str,
}

/// Assert that `body`, which reads a field reaching `Tone`, gives `at_pin` over a stored `Ink` at
/// its pin and is stale once `Dye` is enabled. With `live`, also assert it blocks `enable Tone.Dye`.
fn assert_reader_blocks(
    seed: u64,
    clauses: &str,
    body: &str,
    at_pin: Option<ReaderOutcome>,
    live: bool,
) {
    let pin = [V1, READER_ADD];
    let before = [V1, READER_ADD, READER_GROW];
    let log = [V1, READER_ADD, READER_GROW, READER_ENABLE];
    let text = format!("import auth\n{body}\n");
    let registered = || vec![bundle("web-1.0", &pin, clauses, &text)];
    if let Some(at_pin) = at_pin {
        once(seed, |rt| {
            let db = opened(&rt, &pin, registered());
            reader_rows(&db, "Ink { depth: 7 }");
            assert_eq!(vec![at_pin.called], reader_called(&db), "{clauses}: {body}");
            assert_eq!(
                vec![at_pin.stored],
                query(&db, "from m in marks select m.tone.depth"),
                "{clauses}: {body}",
            );
        });
    }
    if live {
        let mut sim = Sim::new(seed, Config::default());
        run(&mut sim, |rt| {
            opened(&rt, &before, registered());
        });
        run(&mut sim, |rt| {
            let (_, detail) = refused_migration(open(&rt, &log, registered()), 4);
            assert_eq!(READER_REFUSED, detail, "{clauses}: {body}");
        });
    }
    once(seed, |rt| {
        assert_eq!(
            RejectReason::BundleStale {
                bundle: "web-1.0".into(),
                ctor: "Tone.Dye".into(),
            },
            refusal(open(&rt, &log, registered()), "web-1.0"),
            "{clauses}: {body}",
        );
    });
}

/// Assert that `body`, which reads no field reaching `Tone`, lets `enable Tone.Dye` pass while live
/// and then runs over a stored `Dye` to `outcome`, with `Dye`'s hue still stored.
fn assert_reader_runs(seed: u64, clauses: &str, body: &str, outcome: Option<ReaderOutcome>) {
    let pin = [V1, READER_ADD];
    let log = [V1, READER_ADD, READER_GROW, READER_ENABLE];
    let text = format!("import auth\n{body}\n");
    let registered = || vec![bundle("web-1.0", &pin, clauses, &text)];
    let mut sim = Sim::new(seed, Config::default());
    run(&mut sim, |rt| {
        opened(&rt, &pin, registered());
    });
    run(&mut sim, |rt| {
        let db = opened(&rt, &log, registered());
        reader_rows(&db, r#"Dye { hue: "red" }"#);
        let got = reader_called(&db);
        if let Some(outcome) = outcome {
            assert_eq!(vec![outcome.called], got, "{clauses}: {body}");
            assert_eq!(
                vec![outcome.stored],
                query(&db, "from m in marks select m.note"),
                "{clauses}: {body}",
            );
        }
        assert_eq!(
            vec![r#""red""#],
            query(&db, READER_TONES),
            "{clauses}: {body}"
        );
    });
}

fn reader_fixtures(dir: &str, extension: &str) -> Vec<(String, Vec<String>)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/pinned-readers")
        .join(dir);
    let mut paths: Vec<_> = std::fs::read_dir(root)
        .expect("the pinned reader fixtures are readable")
        .map(|entry| entry.expect("a fixture entry is readable").path())
        .filter(|path| path.extension().is_some_and(|e| e == extension))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path).expect("the fixture is readable");
            let name = path
                .file_stem()
                .expect("a fixture has a name")
                .to_string_lossy()
                .into_owned();
            let parts = text
                .trim_end()
                .split("\n===\n")
                .map(str::to_owned)
                .collect();
            (name, parts)
        })
        .collect()
}

/// Assert a reader of a row each binder binds, through each access, is stale once `Dye` is enabled.
#[test]
fn each_binder_with_each_access_of_tone_is_stale_after_enable() {
    let accesses = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/pinned-readers/accesses.txt"),
    )
    .expect("the accesses are readable");
    let accesses: Vec<(&str, &str)> = accesses
        .lines()
        .map(|l| l.split_once('\t').expect("an access is name, tab, text"))
        .collect();
    let helpers = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/pinned-readers/helpers.txt"),
    )
    .expect("the helpers are readable");
    let binders = reader_fixtures("binders", "binder");
    let every = adtb_testkit::profile::every_case();
    let mut reached = BTreeSet::new();
    let mut seed = 400;
    for (name, parts) in &binders {
        let [
            row,
            reading,
            copying,
            read_called,
            copy_called,
            read_stored,
            copy_stored,
        ] = parts.as_slice()
        else {
            panic!("{name}: a binder has seven sections");
        };
        let at_pin = ReaderOutcome {
            called: read_called,
            stored: read_stored,
        };
        let over_dye = ReaderOutcome {
            called: copy_called,
            stored: copy_stored,
        };
        let headers: &[&str] = if !every || name == "deref" {
            &[""]
        } else {
            &["", " hide new constructors"]
        };
        for clauses in headers {
            for (access, text) in &accesses {
                if !every && !both_paths(name) && *access != "direct" {
                    continue;
                }
                seed += 1;
                let reading = reading.replace("@A", &text.replace("@R", row));
                let body = format!("{helpers}{reading}");
                assert_reader_blocks(seed, clauses, &body, Some(at_pin), every);
                reached.insert((name.as_str(), *access));
            }
            seed += 1;
            assert_reader_runs(seed, clauses, copying, Some(over_dye));
        }
    }
    let pairs = if every {
        binders.len() * accesses.len()
    } else {
        binders.len() + (accesses.len() - 1) * binders.iter().filter(|(n, _)| both_paths(n)).count()
    };
    assert_eq!(pairs, reached.len(), "every selected pair ran");
    assert_eq!(
        vec![
            "deref",
            "rel",
            "replace",
            "upsert-old",
            "upsert-result-let",
            "upsert-result",
        ],
        binders.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
    );
    assert_eq!(
        vec![
            "direct",
            "fn-row",
            "fn-field",
            "case",
            "for",
            "for-capture",
            "fn-capture",
            "with-read",
            "whole-compare",
            "sole-case",
            "known-case",
            "unused-arg",
            "unused-fn",
            "other-field",
            "dead-if",
            "dead-arm",
            "singleton-source",
            "empty-source",
        ],
        accesses.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
    );
}

/// Returns whether a default run takes every access through binder `name`.
fn both_paths(name: &str) -> bool {
    name == "rel" || name == "upsert-old"
}

#[test]
fn a_sole_access_shape_blocks_enable_only_when_it_reads_the_field() {
    let shapes = reader_fixtures("shapes", "shape");
    let mut ran = BTreeSet::new();
    let mut seed = 900;
    for (name, parts) in &shapes {
        let [outcome, body] = parts.as_slice() else {
            panic!("{name}: a shape has two sections");
        };
        seed += 1;
        match outcome.as_str() {
            "stale" => assert_reader_blocks(seed, "", body, None, false),
            "stale-live" => assert_reader_blocks(seed, "", body, None, true),
            "accept" => assert_reader_runs(seed, "", body, None),
            "transition" => once(seed, |rt| {
                let text = format!("import auth\n{body}\n");
                let pin = [V1, READER_ADD];
                assert_eq!(
                    "query `run`: `inserted` reads the change of a statement, only in a constraint",
                    invalid(
                        open(&rt, &pin, vec![bundle("web-1.0", &pin, "", &text)]),
                        "web-1.0"
                    ),
                );
            }),
            other => panic!("{name}: unknown outcome {other}"),
        }
        ran.insert(name.as_str());
    }
    assert_eq!(
        BTreeSet::from([
            "deref-path",
            "field-transition",
            "outer-capture",
            "retained-let",
            "returned-with-copy",
            "selected-pure",
            "sole-upsert-old-depth",
            "unused-let-field",
            "unused-let-fn",
            "with-copy",
            "with-copy-then-note",
        ]),
        ran,
    );
}

/// The stored forms a removal sweep writes for constructor `c`, as a text builds each.
struct Stored<'a> {
    c: &'a str,
    /// A payload a text field stores out of line.
    var: &'a str,
    /// A fixed payload the largest of whose sum is `A`'s.
    fixed: &'a str,
    /// A payload past the largest a row stores whole when `A` is present.
    large: &'a str,
}

type Shown = fn(&Stored<'_>) -> String;

const STORED: [Stored<'_>; 3] = [
    Stored {
        c: "A",
        var: "A { a: 1 }",
        fixed: "A { a: 1 }",
        large: "A { x1: 1, x2: 2, x3: 3, x4: 4, x5: 5, x6: 6, x7: 7, x8: 8, x9: 9 }",
    },
    Stored {
        c: "B",
        var: "B { b: 2 }",
        fixed: "B { b: 2 }",
        large: "B { b: 2 }",
    },
    Stored {
        c: "C",
        var: "C { c: \"x\" }",
        fixed: "C { c: 3 }",
        large: "C { c: 3 }",
    },
];

const STORED_TYPES: &str = "module auth
add type T = A { a: I64 } | B { b: I32 } | C { c: Text }
add type F = A { a: I64 } | B { b: I32 } | C { c: I16 }
add type G = A { x1: I64, x2: I64, x3: I64, x4: I64, x5: I64, x6: I64, x7: I64, x8: I64, x9: I64 } | B { b: I32 } | C { c: I16 }
add type E = A0 | B0 | C0
add type U { id: Key us } = A { n: I32 } | B { n: I32 } | C { n: I32 }
add relation us : U key id generate sequence
add type Rec = { t: T, n: I32 }
add type H = { id: Key hs, tag: Tag T, val: T, f: F, g: G, en: E, opt: Option T, rec: Rec, rtag: Tag U }
add relation hs : H key id generate sequence
  index (val)
  index (tag)
  index (f)
  index (en)
  index (rtag)
";

const STORED_READS: &str = "import auth
query tags () -> Set (Tag T) = from h in hs select h.tag
query vals () -> Set T = from h in hs select h.val
query fs () -> Set F = from h in hs select h.f
query gs () -> Set G = from h in hs select h.g
query ens () -> Set E = from h in hs select h.en
query opts () -> Set (Option T) = from h in hs select h.opt
query recs () -> Set Rec = from h in hs select h.rec
query rtags () -> Set (Tag U) = from h in hs select h.rtag
";

fn stored_deprecate(c: &str) -> String {
    format!(
        "module auth\ndeprecate T.{c}\ndeprecate F.{c}\ndeprecate G.{c}\ndeprecate E.{c}0\ndeprecate us.{c}\n"
    )
}

fn stored_remove(c: &str) -> String {
    format!(
        "module auth\nremove constructor T.{c}\nremove constructor F.{c}\nremove constructor G.{c}\nremove constructor E.{c}0\nremove constructor U.{c}\n"
    )
}

fn stored_insert(s: &Stored<'_>, id: u32) -> String {
    let Stored {
        c,
        var,
        fixed,
        large,
    } = s;
    format!(
        "insert hs {{ id: hs.key({id}), tag: {c}, val: {var}, f: {fixed}, g: {large}, en: {c}0, opt: Some {{ value: {var} }}, rec: {{ t: {var}, n: {id} }}, rtag: {c} }}"
    )
}

fn stored_printed(s: &Stored<'_>, id: u32) -> String {
    let Stored {
        c,
        var,
        fixed,
        large,
    } = s;
    format!(
        "H {{ id: hs.key({id}), tag: {c}, val: {var}, f: {fixed}, g: {large}, en: {c}0, opt: Some {{ value: {var} }}, rec: Rec {{ t: {var}, n: {id} }}, rtag: {c} }}"
    )
}

fn set(rows: impl IntoIterator<Item = String>) -> BTreeSet<String> {
    rows.into_iter().collect()
}

/// Returns the position of constructor `ctor` of type `ty` after `migrations`.
fn ctor_position(migrations: &[&str], ty: &str, ctor: &str) -> usize {
    let mut source = Source::default();
    for text in migrations {
        let file = parse_migration(text, Options::default())
            .ok()
            .unwrap_or_else(|ds| panic!("`{text}` parses: {ds:?}"));
        source = source
            .migrate(&file, text)
            .unwrap_or_else(|r| panic!("`{text}` applies: {r:?}"));
    }
    let schema = source.schema().expect("the source builds");
    let def = schema
        .types
        .iter()
        .find(|t| t.name == ty)
        .unwrap_or_else(|| panic!("a type `{ty}`"));
    def.ctors()
        .iter()
        .position(|c| c.name == ctor)
        .unwrap_or_else(|| panic!("a constructor `{ctor}` of `{ty}`"))
}

#[test]
fn a_stored_value_keeps_its_constructor_when_a_removal_moves_its_position() {
    for (seed, removed) in (960..).zip(&STORED[..2]) {
        let (deprecate, remove) = (stored_deprecate(removed.c), stored_remove(removed.c));
        let survivors: Vec<&Stored<'_>> = STORED.iter().filter(|s| s.c != removed.c).collect();
        let writes = format!(
            "import auth\ntxn put () -> I32 =\n  transact\n    {}\n    return 1\n",
            stored_insert(survivors[1], 21)
        );
        let pin = [V1, STORED_TYPES];
        let log = [V1, STORED_TYPES, deprecate.as_str(), remove.as_str()];
        assert_eq!(
            1,
            ctor_position(&pin, "T", survivors[1].c) - ctor_position(&log, "T", survivors[1].c),
            "removed {}",
            removed.c
        );
        let regs = || {
            vec![
                bundle("read-1.0", &pin, "", STORED_READS),
                bundle("write-1.0", &pin, "", &writes),
            ]
        };
        let mut sim = Sim::new(seed, Config::default());
        run(&mut sim, |rt| {
            let db = opened(&rt, &[V1, STORED_TYPES, deprecate.as_str()], vec![]);
            results(&db, &stored_insert(survivors[0], 1));
            results(&db, &stored_insert(survivors[1], 2));
        });
        run(&mut sim, |rt| {
            let db = opened(&rt, &log, regs());
            results(&db, &stored_insert(survivors[0], 11));
            results(&db, &stored_insert(survivors[1], 12));
            call(&db, "write-1.0", 1, &[]);
        });
        run(&mut sim, |rt| {
            let db = opened(&rt, &log, regs());
            let written = [(survivors[0], vec![1, 11]), (survivors[1], vec![2, 12, 21])];
            let all = written
                .iter()
                .flat_map(|(s, ids)| ids.iter().map(|id| stored_printed(s, *id)));
            assert_eq!(
                set(all),
                set(query(&db, "from h in hs select h")),
                "removed {}",
                removed.c
            );
            let columns: [(u32, Shown); 6] = [
                (1, |s| s.c.to_string()),
                (2, |s| s.var.to_string()),
                (3, |s| s.fixed.to_string()),
                (4, |s| s.large.to_string()),
                (5, |s| format!("{}0", s.c)),
                (6, |s| format!("Some {{ value: {} }}", s.var)),
            ];
            for (ordinal, shown) in columns {
                assert_eq!(
                    set(survivors.iter().map(|s| shown(s))),
                    set(rows(call(&db, "read-1.0", ordinal, &[]))),
                    "removed {} query {ordinal}",
                    removed.c
                );
            }
            let recs = written.iter().flat_map(|(s, ids)| {
                ids.iter()
                    .map(|id| format!("Rec {{ t: {}, n: {id} }}", s.var))
            });
            assert_eq!(
                set(recs),
                set(rows(call(&db, "read-1.0", 7, &[]))),
                "removed {}",
                removed.c
            );
            assert_eq!(
                set(survivors.iter().map(|s| s.c.to_string())),
                set(rows(call(&db, "read-1.0", 8, &[]))),
                "removed {}",
                removed.c
            );
            for (s, ids) in &written {
                let keys = set(ids.iter().map(|id| format!("hs.key({id})")));
                let c = s.c;
                let probes = [
                    format!("h.val == {}", s.var),
                    format!("h.tag == {c}"),
                    format!("h.f == {}", s.fixed),
                    format!("h.en == {c}0"),
                    format!("h.rtag == {c}"),
                ];
                for probe in &probes {
                    let text = format!("from h in hs where {probe} select h.id");
                    assert_eq!(
                        keys,
                        set(query(&db, &text)),
                        "removed {} {probe}",
                        removed.c
                    );
                }
            }
            let plan = query(
                &db,
                &format!(
                    "explain from h in hs where h.val == {} select h.id",
                    survivors[0].var
                ),
            );
            assert!(plan[0].contains("hs (val)"), "the index answers: {plan:?}");
        });
    }
}

#[test]
fn a_removal_is_refused_while_a_stored_field_of_each_shape_holds_the_constructor() {
    let [a, b, _] = &STORED;
    let shapes = |removed: &Stored<'_>| {
        let some = format!("Some {{ value: {} }}", removed.var);
        let rec = format!("{{ t: {}, n: 1 }}", removed.var);
        [
            ("T", "val", removed.var.to_string()),
            ("T", "tag", removed.c.to_string()),
            ("F", "f", removed.fixed.to_string()),
            ("G", "g", removed.large.to_string()),
            ("E", "en", format!("{}0", removed.c)),
            ("T", "opt", some),
            ("T", "rec", rec),
            ("U", "rtag", removed.c.to_string()),
        ]
    };
    let row = |kept: &Stored<'_>, field: &str, value: &str| {
        let fields = [
            ("tag", kept.c.to_string()),
            ("val", kept.var.to_string()),
            ("f", kept.fixed.to_string()),
            ("g", kept.large.to_string()),
            ("en", format!("{}0", kept.c)),
            ("opt", "None".to_string()),
            ("rec", format!("{{ t: {}, n: 1 }}", kept.var)),
            ("rtag", kept.c.to_string()),
        ]
        .map(|(n, v)| format!("{n}: {}", if n == field { value } else { &v }));
        format!("insert hs {{ id: hs.key(1), {} }}", fields.join(", "))
    };
    let at_first = shapes(a);
    let [val_b, ..] = shapes(b);
    let cases = at_first.iter().map(|s| (a, b, s)).chain([(b, a, &val_b)]);
    for (seed, (removed, kept, (held, field, value))) in (970..).zip(cases) {
        let (deprecate, remove) = (stored_deprecate(removed.c), stored_remove(removed.c));
        let name = if *held == "E" {
            format!("{}0", removed.c)
        } else {
            removed.c.to_string()
        };
        let insert = row(kept, field, value);
        let log = [V1, STORED_TYPES, deprecate.as_str(), remove.as_str()];
        let mut sim = Sim::new(seed, Config::default());
        run(&mut sim, |rt| {
            let db = opened(&rt, &[V1, STORED_TYPES], vec![]);
            results(&db, &insert);
        });
        run(&mut sim, |rt| {
            let (operator, detail) = refused_migration(open(&rt, &log, vec![]), 4);
            assert_eq!(format!("remove constructor {held}.{name}"), operator);
            assert_eq!(
                format!("a field of hs holds a value of {held}.{name}; it cannot be removed"),
                detail.lines().next().expect("a reason"),
                "{insert}"
            );
        });
    }
    let (deprecate, remove) = (stored_deprecate(a.c), stored_remove(a.c));
    let log = [V1, STORED_TYPES, deprecate.as_str(), remove.as_str()];
    let mut sim = Sim::new(980, Config::default());
    run(&mut sim, |rt| {
        let db = opened(&rt, &[V1, STORED_TYPES], vec![]);
        results(&db, &row(b, "val", a.var));
        results(&db, "delete h in hs where h.id == hs.key(1)");
        results(&db, &row(b, "", ""));
    });
    run(&mut sim, |rt| {
        let db = opened(&rt, &log, vec![]);
        assert_eq!(
            vec!["hs.key(1)".to_string()],
            query(&db, "from h in hs select h.id")
        );
    });
}

/// Returns the host functions `names`, each answering its own name.
fn named_host_fns(names: &[&str]) -> adtb::HostFns {
    let mut fns = adtb::HostFns::new();
    for name in names {
        let reply = name.to_string();
        fns.register(
            name,
            "1.0",
            adtb::Determinism::Deterministic,
            move |b: &adtb::batch::Batch| {
                let mut out = adtb::batch::BatchBuilder::new(adtb::batch::Schema::new(vec![
                    adtb::batch::Type::Scalar(adtb::key::Scalar::Text),
                ]));
                for _ in 0..b.len() {
                    out.push_row(&[adtb::key::Value::Text(reply.clone())]);
                }
                Ok(out.finish())
            },
        );
    }
    fns
}

const CODED: &str = "module auth\ndefine fn f (t: Text) -> Text = host \"a\" version \"1.0\" deterministic\nadd type H { id: Key hs } = R { n: Text }\nadd relation hs : H key id generate sequence\n";
const CODED_PUT: &str = "import auth\ntxn put () -> I32 =\n  transact\n    let h = insert hs.R { n: \"s\" }\n    return 1\n";
const FILL_F: &str = "module auth\nadd field H.R.x: Text fill f(\"s\")\n";
const TO_A: &str =
    "module auth\nredefine fn f (t: Text) -> Text = host \"a\" version \"1.0\" deterministic\n";
const TO_B: &str =
    "module auth\nredefine fn f (t: Text) -> Text = host \"b\" version \"1.0\" deterministic\n";
const REDEFINED_B: &str =
    "redefine fn f (t: Text) -> Text = host \"b\" version \"1.0\" deterministic";
const CODED_PIN: [&str; 2] = [V1, CODED];

fn coded_open(
    rt: &SimRuntime,
    migrations: &[&str],
    regs: Vec<Registration>,
) -> Result<Database<SimRuntime>, OpenError> {
    let options = OpenOptions {
        host_fns: named_host_fns(&["a", "b", "c"]),
        ..with(regs)
    };
    Database::open_in(rt.clone(), "app.adt", &log(migrations), &options)
}

fn coded_put(name: &str) -> Vec<Registration> {
    vec![bundle(name, &CODED_PIN, "", CODED_PUT)]
}

fn stale_fill(name: &str, operator: &str, f: &str) -> RejectReason {
    RejectReason::Untranslatable {
        query: format!("{name}::put"),
        detail: format!("across `{operator}`: it calls `{f}`, whose code changed since the pin"),
    }
}

const BARE: &str = "module auth\nadd module auth\n";
const BLAMELESS: &str = "module auth\nadd type T = A | B\ndrop type T\nredefine fn f (t: Text) -> Text = host \"b\" version \"1.0\" deterministic\n";
const BARE_PIN: [&str; 2] = [BARE, CODED];
const CODED_READS: &str = "import auth\nquery q () -> Set Text = from h in hs select h.n\n";
const TO_WASM: &str = "module auth\nredefine fn f (t: Text) -> Text = wasm \"m\" export \"e\"\n";
const REDEFINED_WASM: &str = "redefine fn f (t: Text) -> Text = wasm \"m\" export \"e\"";
const WASM_LINE: &str = "function `f` names Wasm module `m`, which no migration installs";
const DEFINE_WASM: &str = "module auth\ndefine fn w (t: Text) -> Text = wasm \"m\" export \"e\"\n";
const REVERT_3: &str = "module auth\nrevert 0003\n";
const INSTALL: &str = "module auth\ninstall geo \"1.2\" from \"geo.adtx\"\n";
const UNINSTALL: &str = "module auth\nuninstall geo\n";
/// What the last open of a case gives bundle web-1.0, and what its state is by then.
#[derive(Clone, Copy, Debug)]
enum Holder {
    LivePresented,
    SupersededByThisOpen,
    SupersededOmitted,
    SupersededPresented,
    RetiredOmitted,
    RetiredPresented,
    ExpiredOmitted,
    ExpiredPresented,
}

impl Holder {
    fn live(self) -> bool {
        matches!(self, Holder::LivePresented)
    }

    fn presented(self) -> bool {
        matches!(
            self,
            Holder::LivePresented
                | Holder::SupersededPresented
                | Holder::RetiredPresented
                | Holder::ExpiredPresented
        )
    }

    /// Returns the state of web-1.0 after an open this holder admits.
    fn state(self) -> &'static str {
        match self {
            Holder::LivePresented | Holder::SupersededPresented | Holder::ExpiredPresented => {
                "web-1.0 active"
            }
            Holder::SupersededByThisOpen | Holder::SupersededOmitted | Holder::ExpiredOmitted => {
                "web-1.0 superseded"
            }
            Holder::RetiredOmitted | Holder::RetiredPresented => "web-1.0 retired",
        }
    }
}

const HOLDERS: [Holder; 6] = [
    Holder::LivePresented,
    Holder::SupersededByThisOpen,
    Holder::SupersededOmitted,
    Holder::RetiredOmitted,
    Holder::RetiredPresented,
    Holder::ExpiredOmitted,
];

fn unquoted(texts: Vec<String>) -> Vec<String> {
    texts
        .into_iter()
        .map(|s| s.trim_matches('"').to_owned())
        .collect()
}

/// Returns the symbol and version `catalog::functions` holds for host function `f`.
fn stored_host_f(db: &Database<SimRuntime>) -> Vec<String> {
    unquoted(query(
        db,
        r#"from h in catalog::functions.Host where h.name == "f" select h.symbol ++ " " ++ h.version"#,
    ))
}

/// Returns a simulation holding bundle web-1.0, pinned at `pin` with a lease of a day, in the
/// state `holder` names.
fn holding(seed: u64, pin: &[&str], holder: Holder) -> Sim {
    let mut sim = Sim::new(seed, Config::default());
    let regs = || vec![bundle("web-1.0", pin, " lease 1d", CODED_READS)];
    run(&mut sim, |rt| {
        let db = coded_open(&rt, pin, regs()).expect("the pin opens");
        match holder {
            Holder::RetiredOmitted | Holder::RetiredPresented => {
                query(&db, "retire bundle web-1.0");
            }
            Holder::ExpiredOmitted | Holder::ExpiredPresented => {
                sleep(&rt, DAY + 2 * HOUR);
            }
            Holder::LivePresented
            | Holder::SupersededByThisOpen
            | Holder::SupersededOmitted
            | Holder::SupersededPresented => {}
        }
    });
    if matches!(
        holder,
        Holder::SupersededOmitted | Holder::SupersededPresented
    ) {
        run(&mut sim, |rt| {
            coded_open(&rt, pin, vec![]).expect("the pin opens without the bundle");
        });
    }
    sim
}

/// Assert that the migration at the end of `migrations`, opened with web-1.0 held as `holder`
/// and pinned at the log before it, is refused by the host rule when web-1.0 is live, naming
/// `operator` and leaving `stored_before` in the catalog, and otherwise applied, leaving
/// `stored_after`.
fn holds_or_admits(
    seed: u64,
    migrations: &[&str],
    holder: Holder,
    operator: &str,
    stored_before: &str,
    stored_after: &str,
) {
    let (migration, pin) = migrations.split_last().expect("a migration");
    let mut sim = holding(seed, pin, holder);
    let regs = || {
        if holder.presented() {
            vec![bundle("web-1.0", pin, " lease 1d", CODED_READS)]
        } else {
            vec![]
        }
    };
    let held = pin.len() as u64 + 1;
    let was = std::cell::Cell::new(Vec::new());
    run(&mut sim, |rt| {
        let got = coded_open(&rt, migrations, regs());
        if holder.live() {
            let (refused_by, detail) = refused_migration(got, held);
            let line = format!(
                "bundle web-1.0 pins {}: its version holds another code of host or Wasm function `f`",
                pin.len()
            );
            assert_eq!(
                (operator, line.as_str()),
                (refused_by.as_str(), detail.as_str()),
                "{holder:?} {migration}"
            );
            return;
        }
        if matches!(holder, Holder::RetiredPresented) {
            assert_eq!(
                RejectReason::Retired {
                    bundle: "web-1.0".into()
                },
                refusal(got, "web-1.0"),
                "{migration}"
            );
            return;
        }
        let db = got.unwrap_or_else(|e| panic!("{holder:?} {migration}: {e:?}"));
        assert_eq!(vec![holder.state()], states(&db), "{holder:?} {migration}");
        was.set(stored_host_f(&db));
    });
    if holder.live() {
        run(&mut sim, |rt| {
            let db = coded_open(&rt, pin, regs()).expect("the log before the refused migration");
            was.set(stored_host_f(&db));
        });
        assert_eq!(vec![stored_before.to_string()], was.take(), "{migration}");
    } else if matches!(holder, Holder::RetiredPresented) {
        run(&mut sim, |rt| {
            let db = coded_open(&rt, migrations, vec![]).expect("the log applies");
            was.set(stored_host_f(&db));
        });
        assert_eq!(vec![stored_after.to_string()], was.take(), "{migration}");
    } else {
        assert_eq!(
            vec![stored_after.to_string()],
            was.take(),
            "{holder:?} {migration}"
        );
    }
}

#[test]
fn a_change_of_host_code_is_refused_for_a_live_pin_and_applies_under_any_other_holder() {
    let revert: &[&str] = &[BARE, CODED, TO_B, REVERT_3];
    let cases = HOLDERS
        .map(|h| (h, &[BARE, CODED, TO_B][..], REDEFINED_B, "a 1.0", "b 1.0"))
        .into_iter()
        .chain(
            [
                Holder::LivePresented,
                Holder::SupersededByThisOpen,
                Holder::RetiredOmitted,
                Holder::ExpiredOmitted,
            ]
            .map(|h| (h, revert, "revert 0003", "b 1.0", "a 1.0")),
        );
    for (seed, (holder, migrations, operator, before, after)) in (1100..).zip(cases) {
        holds_or_admits(seed, migrations, holder, operator, before, after);
    }
}

fn other_code(query: &str) -> RejectReason {
    RejectReason::Untranslatable {
        query: query.into(),
        detail: "its version holds another code of host or Wasm function `f`".into(),
    }
}

#[test]
fn a_bundle_going_live_is_refused_at_other_host_code_and_admitted_at_the_same() {
    let regs = || vec![bundle("web-1.0", &BARE_PIN, " lease 1d", CODED_READS)];
    let expired = RejectReason::BundleExpired {
        bundle: "web-1.0".into(),
        query: "web-1.0::q".into(),
    };
    let held: [(Option<Holder>, RejectReason); 3] = [
        (None, other_code("web-1.0::q")),
        (Some(Holder::ExpiredPresented), expired.clone()),
        (Some(Holder::SupersededPresented), expired),
    ];
    for (seed, (holder, refused)) in (1170..).step_by(2).zip(held) {
        let sim = |seed| match holder {
            Some(holder) => holding(seed, &BARE_PIN, holder),
            None => {
                let mut sim = Sim::new(seed, Config::default());
                run(&mut sim, |rt| {
                    coded_open(&rt, &BARE_PIN, vec![]).expect("the pin opens");
                });
                sim
            }
        };
        let mut other = sim(seed);
        run(&mut other, |rt| {
            let got = coded_open(&rt, &[BARE, CODED, TO_B], regs());
            assert_eq!(refused, refusal(got, "web-1.0"), "{holder:?}");
        });
        run(&mut other, |rt| {
            let db = coded_open(&rt, &[BARE, CODED, TO_B], vec![]).expect("the migration applied");
            assert_eq!(vec!["b 1.0"], stored_host_f(&db), "{holder:?}");
        });
        let mut same = sim(seed + 1);
        run(&mut same, |rt| {
            let db = coded_open(&rt, &[BARE, CODED, TO_A], regs())
                .unwrap_or_else(|e| panic!("{holder:?}: {e:?}"));
            assert_eq!(vec!["web-1.0 active"], states(&db), "{holder:?}");
            assert_eq!(vec!["a 1.0"], stored_host_f(&db), "{holder:?}");
        });
    }
}

#[test]
fn a_bundle_going_live_is_admitted_at_a_pin_of_a_language_function_changed_since() {
    let define_l = "module auth\ndefine fn l (t: Text) -> Text = t\n";
    let move_l = "module auth\nredefine fn l (t: Text) -> Text = t ++ t\n";
    let pin = [BARE, CODED, define_l];
    once(1178, |rt| {
        let db = coded_open(
            &rt,
            &[BARE, CODED, define_l, move_l],
            vec![bundle("web-1.0", &pin, " lease 1d", CODED_READS)],
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(vec!["web-1.0 active"], states(&db));
    });
}

#[test]
fn a_bundle_pinned_ahead_of_the_file_is_refused_at_other_host_code() {
    let define_g =
        "module auth\ndefine fn g (t: Text) -> Text = host \"a\" version \"1.0\" deterministic\n";
    let pin = [BARE, CODED, define_g];
    let mut sim = Sim::new(1176, Config::default());
    run(&mut sim, |rt| {
        coded_open(&rt, &BARE_PIN, vec![]).expect("the file opens behind the pin");
    });
    run(&mut sim, |rt| {
        assert_eq!(
            other_code("web-1.0::q"),
            refusal(
                coded_open(
                    &rt,
                    &[BARE, CODED, define_g, TO_B],
                    vec![bundle("web-1.0", &pin, " lease 1d", CODED_READS)]
                ),
                "web-1.0"
            )
        );
    });
}

/// A bundle whose first registration is refused at `log`, and a migration past `log` that a
/// reader of that bundle would block.
struct NeverRegistered {
    seed: u64,
    pin: &'static [&'static str],
    log: &'static [&'static str],
    body: &'static str,
    refused: RejectReason,
    further: &'static str,
    applied: fn(&Database<SimRuntime>) -> Vec<String>,
    after: &'static [&'static str],
}

const ADD_X: &str = "module auth\nadd field H.R.x: Text fill \"s\"\n";
const DROP_X: &str = "module auth\ndrop field H.R.x\n";
const TO_C: &str =
    "module auth\nredefine fn f (t: Text) -> Text = host \"c\" version \"1.0\" deterministic\n";
const ADD_TS: &str = "module auth\nadd type T { id: Key ts } = S { n: Text }\nadd relation ts : T key id generate sequence\n";

fn never_registered() -> [NeverRegistered; 2] {
    [
        NeverRegistered {
            seed: 1190,
            pin: &BARE_PIN,
            log: &[BARE, CODED, TO_B],
            body: CODED_READS,
            refused: other_code("web-1.0::q"),
            further: TO_C,
            applied: stored_host_f,
            after: &["c 1.0"],
        },
        NeverRegistered {
            seed: 1192,
            pin: &[BARE, CODED, ADD_X],
            log: &[BARE, CODED, ADD_X, DROP_X],
            body: "import auth\nquery q () -> Set Text = from h in hs select h.x\n",
            refused: RejectReason::Untranslatable {
                query: "web-1.0::q".into(),
                detail: "across `drop field H.R.x`: it reads field `x` of `H`, which the current version lacks".into(),
            },
            further: ADD_TS,
            applied: |db| query(db, "from t in ts select t.n"),
            after: &[],
        },
    ]
}

impl NeverRegistered {
    fn regs(&self) -> Vec<Registration> {
        vec![bundle("web-1.0", self.pin, " lease 1d", self.body)]
    }

    fn further_log(&self) -> Vec<&'static str> {
        let mut log = self.log.to_vec();
        log.push(self.further);
        log
    }

    /// Returns a simulation whose file holds the recorded row of the refused first registration.
    fn recorded(&self, seed: u64) -> Sim {
        let mut sim = Sim::new(seed, Config::default());
        run(&mut sim, |rt| {
            coded_open(&rt, self.pin, vec![]).expect("the file opens at the pin");
        });
        run(&mut sim, |rt| {
            let got = coded_open(&rt, self.log, self.regs());
            assert_eq!(self.refused, refusal(got, "web-1.0"), "seed {seed}");
        });
        sim
    }
}

#[test]
fn a_bundle_never_registered_holds_back_no_migration_and_is_refused_at_registration() {
    for case in never_registered() {
        let mut sim = case.recorded(case.seed);
        run(&mut sim, |rt| {
            let got = coded_open(&rt, &case.further_log(), case.regs());
            assert_eq!(case.refused, refusal(got, "web-1.0"), "seed {}", case.seed);
        });
        run(&mut sim, |rt| {
            let db = coded_open(&rt, case.log, vec![]).expect("the file is ahead of this log");
            assert_eq!(case.after, (case.applied)(&db), "seed {}", case.seed);
        });
    }
}

#[test]
fn a_bundle_never_registered_and_not_presented_again_holds_back_nothing_and_stays_not_live() {
    for case in never_registered() {
        let mut sim = case.recorded(case.seed + 1);
        run(&mut sim, |rt| {
            let db = coded_open(&rt, &case.further_log(), vec![])
                .unwrap_or_else(|e| panic!("seed {}: {e:?}", case.seed));
            assert_eq!(case.after, (case.applied)(&db), "seed {}", case.seed);
            assert_eq!(
                vec!["web-1.0 superseded"],
                states(&db),
                "seed {}",
                case.seed
            );
            assert_eq!(
                RejectReason::BundleSuperseded {
                    bundle: "web-1.0".into()
                },
                unbound(&db, "web-1.0"),
                "seed {}",
                case.seed
            );
        });
    }
}

#[test]
fn the_refusal_of_a_host_change_names_its_operator_past_others_in_the_migration() {
    let add = "module auth\nadd type T = A | B\n";
    let redefine = format!("{add}{}", TO_B.trim_start_matches("module auth\n"));
    let revert = format!("{add}revert 0003\n");
    let cases: [(&[&str], &str, &str, &str); 2] = [
        (
            &[BARE, CODED, redefine.as_str()],
            REDEFINED_B,
            "a 1.0",
            "b 1.0",
        ),
        (
            &[BARE, CODED, TO_B, revert.as_str()],
            "revert 0003",
            "b 1.0",
            "a 1.0",
        ),
    ];
    for (seed, (migrations, operator, before, after)) in (1120..).zip(cases) {
        holds_or_admits(
            seed,
            migrations,
            Holder::LivePresented,
            operator,
            before,
            after,
        );
    }
}

#[test]
fn wasm_and_extensions_change_no_pinned_code_because_the_engine_refuses_them() {
    let cases: [(&[&str], &str, &str); 4] = [
        (&[BARE, CODED, TO_WASM], "", WASM_LINE),
        (
            &[BARE, CODED, DEFINE_WASM],
            "",
            "function `w` names Wasm module `m`, which no migration installs",
        ),
        (
            &[BARE, CODED, INSTALL],
            "",
            "the package `geo.adtx` is not available",
        ),
        (
            &[BARE, CODED, UNINSTALL],
            "",
            "`geo` is not an installed extension",
        ),
    ];
    let mut sim = holding(1130, &BARE_PIN, Holder::SupersededByThisOpen);
    run(&mut sim, |rt| {
        for (migrations, operator, detail) in cases {
            let (refused_by, why) = refused_migration(coded_open(&rt, migrations, vec![]), 3);
            assert_eq!((operator, detail), (refused_by.as_str(), why.as_str()));
        }
    });
    let mut sim = holding(1150, &BARE_PIN, Holder::LivePresented);
    run(&mut sim, |rt| {
        let (refused_by, detail) = refused_migration(
            coded_open(
                &rt,
                &[BARE, CODED, TO_WASM],
                vec![bundle("web-1.0", &BARE_PIN, " lease 1d", CODED_READS)],
            ),
            3,
        );
        assert_eq!(REDEFINED_WASM, refused_by);
        assert_eq!(
            "bundle web-1.0 pins 2: its version holds another code of host or Wasm function `f`",
            detail
        );
    });
}

#[test]
fn a_function_dropped_and_added_again_is_a_fresh_function_a_pinned_fill_refuses() {
    let again = "module auth\ndrop fn f\ndefine fn f (t: Text) -> Text = host \"a\" version \"1.0\" deterministic\n";
    for (seed, migrations, operator) in [
        (
            998,
            [V1, CODED, again, FILL_F],
            "add field H.R.x: Text fill f(\"s\")",
        ),
        (999, [V1, CODED, FILL_F, again], "drop fn f"),
    ] {
        once(seed, |rt| {
            assert_eq!(
                stale_fill("web-1.0", operator, "f"),
                refusal(
                    coded_open(&rt, &migrations, coded_put("web-1.0")),
                    "web-1.0"
                ),
            );
        });
    }
}

#[test]
fn a_host_redefine_under_a_live_pin_is_refused_when_its_migration_does_not_resolve() {
    let mut sim = holding(1151, &BARE_PIN, Holder::LivePresented);
    run(&mut sim, |rt| {
        let (_, detail) = refused_migration(
            coded_open(
                &rt,
                &[BARE, CODED, BLAMELESS],
                vec![bundle("web-1.0", &BARE_PIN, " lease 1d", CODED_READS)],
            ),
            3,
        );
        assert_eq!(
            "bundle web-1.0 pins 2: its version holds another code of host or Wasm function `f`",
            detail
        );
    });
}

const CONTACT: &str = "module auth
add type Contact = Mail { addr: Text } | Phone { num: Text }
add type Handle = new Text
add type Tagged = { nick: Handle, n: Text }
add relation tags : Tagged key nick
add type Pair = { a: Text, b: Text }
add relation pairs : Pair key (a, b)
add type Pt = { x: Text }
";

/// The operators after `CONTACT` a print or parse may meet.
const TEXT_OPS: [&str; 8] = [
    "rename constructor Contact.Mail to Email",
    "rename field Contact.Mail.addr to address",
    "add constructor Contact.Post { box: Text }",
    "add field Contact.Mail.verified: Bool fill false",
    "rename type Handle to Nick",
    "rename relation users to people",
    "rename field Pair.a to x",
    "rename type Pt to Q",
];

/// A bundle body pinned after `CONTACT` that prints or parses a value of a user type, what each
/// of its queries answers at the pin, and the refusal of its first query across each operator
/// of `TEXT_OPS`, by position, that changes the text.
struct Textual {
    body: &'static str,
    answers: &'static [&'static str],
    refused: &'static [(usize, &'static str)],
}

const TEXTUAL: [Textual; 10] = [
    Textual {
        body: r#"query q () -> Text = print(Mail { addr: "a" })"#,
        answers: &[r#""Mail { addr: \"a\" }""#],
        refused: &[
            (
                0,
                "it prints `Mail`, which the current version names otherwise",
            ),
            (
                1,
                "it prints `addr`, which the current version names otherwise",
            ),
            (
                3,
                "it prints `Contact` whole, and its field `verified` differs",
            ),
        ],
    },
    Textual {
        body: r#"query q () -> Option Contact = Contact.parse("Mail { addr: \"a\" }")
query post () -> Option Contact = Contact.parse("Post { box: \"b\" }")"#,
        answers: &["Some { value: Mail { addr: \"a\" } }", "None"],
        refused: &[
            (
                0,
                "it parses `Mail`, which the current version names otherwise",
            ),
            (
                1,
                "it parses `addr`, which the current version names otherwise",
            ),
            (
                3,
                "it parses `Contact` whole, and its field `verified` differs",
            ),
        ],
    },
    Textual {
        body: "query q () -> Text = print(users.key(1))",
        answers: &[r#""users.key(1)""#],
        refused: &[(
            5,
            "it prints `users`, which the current version names otherwise",
        )],
    },
    Textual {
        body: r#"query q () -> Text = print(Handle("h"))"#,
        answers: &[r#""Handle(\"h\")""#],
        refused: &[(
            4,
            "it prints `Handle`, which the current version names otherwise",
        )],
    },
    Textual {
        body: r#"query q () -> Option Handle = Handle.parse("Handle(\"h\")")"#,
        answers: &[r#"Some { value: Handle("h") }"#],
        refused: &[(
            4,
            "it parses `Handle`, which the current version names otherwise",
        )],
    },
    Textual {
        body: r#"query q () -> Text = print(tags.key(Handle("h")))"#,
        answers: &[r#""tags.key(Handle(\"h\"))""#],
        refused: &[(
            4,
            "it prints `Handle`, which the current version names otherwise",
        )],
    },
    Textual {
        body: r#"query q () -> Text = print(pairs.key(a: "1", b: "2"))"#,
        answers: &[r#""pairs.key(a: \"1\", b: \"2\")""#],
        refused: &[(
            6,
            "it prints `a`, which the current version names otherwise",
        )],
    },
    Textual {
        body: r#"query q () -> Option (Key pairs) = (Key pairs).parse("pairs.key(a: \"1\", b: \"2\")")"#,
        answers: &[r#"Some { value: pairs.key(a: "1", b: "2") }"#],
        refused: &[(
            6,
            "it parses `a`, which the current version names otherwise",
        )],
    },
    Textual {
        body: r#"query q () -> Text = print(Pt { x: "1" })"#,
        answers: &[r#""Pt { x: \"1\" }""#],
        refused: &[(
            7,
            "it prints `Pt`, which the current version names otherwise",
        )],
    },
    Textual {
        body: r#"query q () -> Option Pt = Pt.parse("Pt { x: \"1\" }")"#,
        answers: &[r#"Some { value: Pt { x: "1" } }"#],
        refused: &[(
            7,
            "it parses `Pt`, which the current version names otherwise",
        )],
    },
];

/// Returns how many bodies of `TEXTUAL` translated and how many were refused across the operators
/// of `TEXT_OPS` at positions `ops`, each applied alone after the pin, asserting each answer and
/// refusal.
fn textual_across(ops: std::ops::Range<usize>) -> (usize, usize) {
    let mut reached = (0, 0);
    for (o, op) in TEXT_OPS.iter().enumerate().take(ops.end).skip(ops.start) {
        let later = format!("module auth\n{op}\n");
        once(1000 + o as u64, |rt| {
            let db = opened(&rt, &[V1, CONTACT, &later], Vec::new());
            let authority = db.role_authority();
            let owner = authority.session("owner").expect("the owner's session");
            let mut kept = String::from("import auth\n");
            let mut answers = Vec::new();
            for (b, t) in TEXTUAL.iter().enumerate() {
                let body = t.body.replace("query q ", &format!("query q{b} "));
                let body = body.replace("query post ", &format!("query post{b} "));
                let Some(&(_, want)) = t.refused.iter().find(|(i, _)| *i == o) else {
                    kept.push_str(&format!("{body}\n"));
                    answers.extend(t.answers.iter().map(|a| (b, *a)));
                    reached.0 += 1;
                    continue;
                };
                let case = format!("body {b} across `{op}`");
                let name = format!("text{b}-1.0");
                let body = format!("import auth\n{body}\n");
                let reg = bundle(&name, &[V1, CONTACT], "", &body);
                let e = authority
                    .register(&owner, &reg)
                    .err()
                    .unwrap_or_else(|| panic!("{case}: registers"));
                let RejectReason::Untranslatable { query, detail } = reason(&e) else {
                    panic!("{case}: refused otherwise: {e:?}");
                };
                assert_eq!(&format!("{name}::q{b}"), query, "{case}");
                assert_eq!(&format!("across `{op}`: {want}"), detail, "{case}");
                reached.1 += 1;
            }
            let reg = bundle("kept-1.0", &[V1, CONTACT], "", &kept);
            authority
                .register(&owner, &reg)
                .unwrap_or_else(|e| panic!("the bodies kept across `{op}` register: {e:?}"));
            for (ordinal, (b, answer)) in (1..).zip(&answers) {
                let got = rows(call(&db, "kept-1.0", ordinal, &[]));
                assert_eq!(vec![*answer], got, "body {b} across `{op}`");
            }
        });
    }
    reached
}

#[test]
fn a_pinned_print_or_parse_answers_as_at_its_pin_or_is_refused_across_a_change_of_a_sum() {
    assert_eq!((34, 6), textual_across(0..4));
}

#[test]
fn a_pinned_print_or_parse_answers_as_at_its_pin_or_is_refused_across_a_rename_elsewhere() {
    assert_eq!((32, 8), textual_across(4..8));
}

/// Each composite key's fields, and each rename of one field `f` to `g` as `fg`, marked `!`
/// where it reorders the key's fields by name: to a name before, between and after the others.
const KEY_RENAMES: [(&str, &str); 2] = [
    ("b d", "ba bc be! da! dc de"),
    ("b d f", "ba bc be! bg! da! dc de dg! fa! fc! fe fg"),
];

/// One rename of `KEY_RENAMES`: the key's fields in name order, the field renamed, its new name
/// and whether it reorders the key's fields.
struct KeyRename {
    fields: Vec<&'static str>,
    from: &'static str,
    to: &'static str,
    reorders: bool,
}

fn key_renames((fields, renames): (&'static str, &'static str)) -> Vec<KeyRename> {
    renames
        .split(' ')
        .map(|r| KeyRename {
            fields: fields.split(' ').collect(),
            from: &r[..1],
            to: &r[1..2],
            reorders: r.ends_with('!'),
        })
        .collect()
}

/// Returns the operators that declare relation `p{i}` keyed by the fields of rename `i`, its key
/// declared in reverse at an odd `i`, and `n{i}` keyed by a key of `p{i}` and a text.
fn key_schema(i: usize, k: &KeyRename) -> String {
    let fields: Vec<String> = k.fields.iter().map(|f| format!("{f}: Text")).collect();
    let mut key: Vec<&str> = k.fields.clone();
    if i % 2 == 1 {
        key.reverse();
    }
    format!(
        "add type P{i} = {{ {} }}\nadd relation p{i} : P{i} key ({})\n\
         add type N{i} = {{ o: Key p{i}, m: Text }}\nadd relation n{i} : N{i} key (o, m)\n",
        fields.join(", "),
        key.join(", ")
    )
}

/// Returns `p{i}.key(..)` of rename `i`, its field `j` in name order holding `"{j + 1}"`, or
/// `"9"` for every field when `top`.
fn key_text(i: usize, k: &KeyRename, top: bool) -> String {
    let args: Vec<String> = (1..)
        .zip(&k.fields)
        .map(|(j, f)| format!("{f}: \"{}\"", if top { 9 } else { j }))
        .collect();
    format!("p{i}.key({})", args.join(", "))
}

/// The number of bodies `key_reads` returns.
const KEY_READS: usize = 7;

/// Returns the queries of rename `i` that read `p{i}` and `n{i}`, whose key holds a reference
/// to `p{i}`.
fn key_reads(i: usize, k: &KeyRename) -> String {
    let key = key_text(i, k, false);
    let cat = |x: &str| {
        k.fields
            .iter()
            .map(|f| format!("{x}.{f}"))
            .collect::<Vec<_>>()
            .join(" ++ ")
    };
    format!(
        "query eq{i} () -> Set Text = from p in p{i} where key p == {key} select {}\n\
         query given{i} (k: Key p{i}) -> Set Text = from p in p{i} where key p == k select {}\n\
         query follow{i} () -> Set Text = from x in n{i} select {}\n\
         query tos{i} () -> Set (Key p{i}) = from x in n{i} select x.o\n\
         query whole{i} () -> Set {{ {} }} = from p in p{i} select value(key p)\n\
         query nested{i} () -> Set (Key n{i}) = from x in n{i} select key x\n\
         query ngiven{i} (k: Key n{i}) -> Set Text = from x in n{i} where key x == k select x.m\n",
        cat("p"),
        cat("p"),
        cat("x.o"),
        k.fields
            .iter()
            .map(|f| format!("{f}: Text"))
            .collect::<Vec<_>>()
            .join(", "),
    )
}

fn committed(called: Result<Called, Error>, case: &str) -> Vec<String> {
    match called {
        Ok(Called::Committed(c)) => printed(c.value),
        other => panic!("{case}: commits: {other:?}"),
    }
}

/// Returns how many (rename, body) pairs answered as at the pin, asserting each answer: a
/// transaction inserting into `p{i}` and returning the key, one inserting into every `n{i}` a
/// row keyed by a key of `p{i}` it builds, the reads of `key_reads`, and, across a rename that keeps
/// the key's order, a query comparing keys ordered.
fn key_order_search(renames: &[KeyRename]) -> usize {
    let n = renames.len();
    let mut schema = String::from("module auth\nadd module auth\n");
    let mut later = String::from("module auth\n");
    let mut puts = String::from("import auth\n");
    let mut reads = String::new();
    let mut sorted = String::from("import auth\n");
    let mut nested = String::new();
    for (i, k) in renames.iter().enumerate() {
        schema.push_str(&key_schema(i, k));
        later.push_str(&format!("rename field P{i}.{} to {}\n", k.from, k.to));
        let row: Vec<String> = (1..)
            .zip(&k.fields)
            .map(|(j, f)| format!("{f}: \"{j}\""))
            .collect();
        puts.push_str(&format!(
            "txn put{i} () -> Key p{i} =\n  transact\n    let p = insert p{i} {{ {} }}\n    return key p\n",
            row.join(", ")
        ));
        nested.push_str(&format!(
            "    let n{i} = insert n{i} {{ o: {}, m: \"z\" }}\n",
            key_text(i, k, false)
        ));
        reads.push_str(&key_reads(i, k));
        if !k.reorders {
            sorted.push_str(&format!(
                "query lt{i} () -> Set Text = from p in p{i} where key p < {} select \"below\"\n",
                key_text(i, k, true)
            ));
        }
    }
    let body =
        format!("{puts}txn link () -> Key n0 =\n  transact\n{nested}    return key n0\n{reads}");
    let mut reached = 0;
    once(31, |rt| {
        let regs = vec![
            bundle("keys-1.0", &[&schema], "", &body),
            bundle("sorted-1.0", &[&schema], "", &sorted),
        ];
        let db = opened(&rt, &[&schema, &later], regs);
        let keys = db.registered("keys-1.0").expect("keys-1.0 binds");
        let run = |ordinal: usize, args: &[Value]| {
            keys.call(ordinal as u32, args, &CallOptions::default())
        };
        let case = |i: usize| {
            let k = &renames[i];
            format!(
                "`rename field P{i}.{} to {}` of {:?}",
                k.from, k.to, k.fields
            )
        };
        for (i, k) in renames.iter().enumerate() {
            let got = committed(run(i + 1, &[]), &case(i));
            assert_eq!(vec![key_text(i, k, false)], got, "put across {}", case(i));
            reached += 1;
        }
        let got = committed(run(n + 1, &[]), "link");
        let n0 = format!("n0.key(m: \"z\", o: {})", key_text(0, &renames[0], false));
        assert_eq!(vec![n0], got, "link");
        reached += n;
        let mut lt = 0;
        for (i, k) in renames.iter().enumerate() {
            let case = case(i);
            let key = key_text(i, k, false);
            let cat: String = (1..=k.fields.len()).map(|j| j.to_string()).collect();
            let cat = format!("{cat:?}");
            let given: Vec<Value> = (1..=k.fields.len())
                .map(|j| Value::text(&j.to_string()))
                .collect();
            let whole: Vec<String> = (1..)
                .zip(&k.fields)
                .map(|(j, f)| format!("{f}: \"{j}\""))
                .collect();
            let whole = format!("{{ {} }}", whole.join(", "));
            let nested = format!("n{i}.key(m: \"z\", o: {key})");
            let z = "\"z\"".to_string();
            for (j, args, want) in [
                (0, Vec::new(), &cat),
                (1, vec![Value::record(given.clone())], &cat),
                (2, Vec::new(), &cat),
                (3, Vec::new(), &key),
                (4, Vec::new(), &whole),
                (5, Vec::new(), &nested),
                (
                    6,
                    vec![Value::record(vec![
                        Value::text("z"),
                        Value::record(given.clone()),
                    ])],
                    &z,
                ),
            ] {
                let got = run(n + 2 + i * KEY_READS + j, &args)
                    .unwrap_or_else(|e| panic!("read {j} across {case}: {e:?}"));
                assert_eq!(vec![want.clone()], rows(got), "read {j} across {case}");
                reached += 1;
            }
            if !k.reorders {
                lt += 1;
                let got = rows(call(&db, "sorted-1.0", lt, &[]));
                assert_eq!(vec!["\"below\""], got, "lt across {case}");
                reached += 1;
            }
        }
    });
    reached
}

#[test]
fn a_pinned_composite_key_reads_and_writes_by_field_across_every_rename_of_a_key_field() {
    let reached: Vec<usize> = KEY_RENAMES
        .iter()
        .map(|&shape| key_order_search(&key_renames(shape)))
        .collect();
    assert_eq!(
        vec![6 * (2 + KEY_READS) + 4, 12 * (2 + KEY_READS) + 6],
        reached
    );
}

#[test]
fn a_pinned_key_value_reads_by_field_across_two_renames_that_reorder_its_fields() {
    let schema = "module auth\nadd module auth\nadd type P = { b: Text, d: Text, f: Text }\nadd relation p : P key (b, d, f)\n";
    let renames = [
        "module auth\nrename field P.b to e\n",
        "module auth\nrename field P.d to a\n",
    ];
    let body = r#"import auth
txn put () -> Key p =
  transact
    let x = insert p { b: "1", d: "2", f: "3" }
    return key x
query whole () -> Set { b: Text, d: Text, f: Text } = from x in p select value(key x)
"#;
    once(32, |rt| {
        let regs = vec![bundle("whole-1.0", &[schema], "", body)];
        let db = opened(&rt, &[schema, renames[0], renames[1]], regs);
        let put = committed(Ok(call(&db, "whole-1.0", 1, &[])), "put");
        assert_eq!(vec![r#"p.key(b: "1", d: "2", f: "3")"#], put);
        let got = rows(call(&db, "whole-1.0", 2, &[]));
        assert_eq!(vec![r#"{ b: "1", d: "2", f: "3" }"#], got);
    });
}
