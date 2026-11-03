//! Bundles in process: the bundle file and its header, registration and its checks at the
//! pinned version, immutability, supersession at open, leases, retirement and supersession by
//! command, and their persistence across reopen and crash.

use adtb_simkit::api;

use std::collections::BTreeSet;
use std::time::Duration;

use adtb::catalog::version_hash;
use adtb::runtime::{Clock, Done, Io, Op, Runtime, Threads};
use adtb::{
    CallOptions, Called, Database, Error, Fault, Log, OpenError, OpenOptions, Registration,
    RejectReason, Value,
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

const ADD_APPLE: &str = "module auth\nadd constructor Identity.Apple { sub: Text }\n";
const ENABLE_APPLE: &str = "module auth\nenable identities.Apple\n";
const ADD_GOLD: &str = "module auth\nadd constructor Tier.Gold\n";
const ENABLE_GOLD: &str = "module auth\nenable Tier.Gold\n";
const RENAME_NAME: &str = "module auth\nrename field User.name to display\n";

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

#[test]
fn a_bundle_registers_under_its_header_name_and_calls_by_ordinal_from_one() {
    once(1, |rt| {
        let db = opened(&rt, &[V1], vec![bundle("web-1.0", &[V1], "", NAMES)]);
        let web = db.registered("web-1.0").expect("web-1.0 binds");
        assert_eq!("web-1.0", web.name());
        assert_eq!(Some(1), web.ordinal("names"));
        assert_eq!(Some(2), web.ordinal("add"));
        match web.call(2, &[Value::text("ann")], &CallOptions::default()) {
            Ok(Called::Committed(_)) => {}
            other => panic!("add commits nothing: {other:?}"),
        }
        assert_eq!(vec![r#""ann""#], rows(call(&db, "web-1.0", 1, &[])));
    });
}

#[test]
fn ordinal_zero_names_no_call() {
    once(2, |rt| {
        let db = opened(&rt, &[V1], vec![bundle("web-1.0", &[V1], "", NAMES)]);
        let web = db.registered("web-1.0").expect("web-1.0 binds");
        let got = web.call(0, &[], &CallOptions::default());
        assert!(
            matches!(
                got,
                Err(Error::Rejected {
                    reason: RejectReason::UnknownQuery
                })
            ),
            "{got:?}"
        );
    });
}

#[test]
fn registration_records_the_bundle_in_catalog_bundles() {
    once(3, |rt| {
        let db = opened(
            &rt,
            &[V1],
            vec![bundle(
                "web-app-2.4",
                &[V1],
                " lease 90d hide new constructors",
                NAMES,
            )],
        );
        assert_eq!(
            vec![
                r#"{ edition: "2026-10", family: "web-app", hides_new: True, name: "web-app-2.4", owner: "owner" }"#
            ],
            query(
                &db,
                "from b in catalog::bundles select { name: b.name, family: b.family, owner: b.owner, edition: b.edition, hides_new: b.hides_new }",
            )
        );
        assert_eq!(
            vec!["True"],
            query(
                &db,
                "from b in catalog::bundles, m in catalog::migrations where b.pins == key m select b.hash == m.hash",
            )
        );
        assert_eq!(
            vec!["True"],
            query(
                &db,
                "from b in catalog::bundles select (match b.lease | Some l -> l.value == 90d | None -> false)",
            )
        );
        assert_eq!(vec!["web-app-2.4 active"], states(&db));
    });
}

#[test]
fn a_bundle_with_no_lease_clause_records_no_lease() {
    once(4, |rt| {
        let db = opened(&rt, &[V1], vec![bundle("web-1.0", &[V1], "", NAMES)]);
        assert_eq!(
            vec!["False"],
            query(
                &db,
                "from b in catalog::bundles select (match b.lease | Some _ -> true | None -> false)",
            )
        );
    });
}

#[test]
fn registration_records_each_call_in_catalog_registered() {
    once(5, |rt| {
        let body = "import auth\nimport fn shown\nquery names () -> Set Text = from u in users select u.name\ntxn add (n: Text) -> Key users =\n  transact\n    let u = insert users { name: n, tier: Free }\n    return key u\n";
        let log = [
            V1,
            "module auth\ndefine fn shown (u: User) -> Text = u.name\n",
        ];
        let db = opened(&rt, &log, vec![bundle("web-1.0", &log, "", body)]);
        assert_eq!(
            vec![
                r#"{ kind: "fn", name: "shown", ordinal: 1 }"#,
                r#"{ kind: "query", name: "names", ordinal: 2 }"#,
                r#"{ kind: "txn", name: "add", ordinal: 3 }"#,
            ],
            query(
                &db,
                r#"from r in catalog::registered where r.bundle.name == "web-1.0" select { name: r.name, ordinal: r.ordinal, kind: match r | Query _ -> "query" | Txn _ -> "txn" | ImportedFn _ -> "fn" }"#,
            )
        );
    });
}

#[test]
fn a_bundle_pinned_at_a_version_the_log_lacks_refuses_open() {
    once(6, |rt| {
        let text = format!(
            "bundle web-1.0 pins 2 ({}) edition \"2026-10\"\n{NAMES}",
            hash(&[V1, ADD_APPLE])
        );
        let reg = Registration::bundle(&text).expect("a bundle file");
        let detail = invalid(open(&rt, &[V1], vec![reg]), "web-1.0");
        assert!(detail.contains("version 2"), "{detail}");
    });
}

#[test]
fn a_bundle_declaring_ad_hoc_priority_is_refused() {
    let text = file("web-1.0", &[V1], " priority adhoc", NAMES);
    match Registration::bundle(&text) {
        Err(Error::Rejected {
            reason: RejectReason::Invalid { detail },
        }) => assert!(detail.contains("adhoc"), "{detail}"),
        other => panic!("the registration is not refused as invalid: {other:?}"),
    }
}

#[test]
fn a_bundle_declaring_background_priority_registers() {
    once(102, |rt| {
        opened(
            &rt,
            &[V1],
            vec![bundle("web-1.0", &[V1], " priority background", NAMES)],
        );
    });
}

#[test]
fn a_bundle_whose_pinned_hash_differs_refuses_open_with_the_position_and_both_hashes() {
    once(7, |rt| {
        let wrong = hash(&[V1, ADD_APPLE]);
        let text = format!("bundle web-1.0 pins 1 ({wrong}) edition \"2026-10\"\n{NAMES}");
        let reg = Registration::bundle(&text).expect("a bundle file");
        let detail = invalid(open(&rt, &[V1], vec![reg]), "web-1.0");
        assert!(detail.contains("version 1"), "{detail}");
        assert!(detail.contains(&wrong), "{detail}");
        assert!(detail.contains(&hash(&[V1])), "{detail}");
    });
}

#[test]
fn a_bundle_of_an_edition_the_engine_does_not_implement_is_refused() {
    once(8, |rt| {
        let text = format!(
            "bundle web-1.0 pins 1 ({}) edition \"1999-01\"\n{NAMES}",
            hash(&[V1])
        );
        let reg = Registration::bundle(&text).expect("a bundle file");
        assert_eq!(
            RejectReason::UnknownEdition {
                edition: "1999-01".into()
            },
            refusal(open(&rt, &[V1], vec![reg]), "web-1.0")
        );
    });
}

#[test]
fn a_bundle_file_without_a_header_is_no_bundle() {
    assert!(Registration::bundle(NAMES).is_err());
    assert!(Registration::bundle("bundle web pins 1 (00) edition \"2026-10\"\n").is_err());
}

#[test]
fn a_bundle_that_fails_to_check_at_its_pin_refuses_open_naming_the_query() {
    once(9, |rt| {
        let apple =
            "import auth\nquery subs () -> Set Text = from a in identities.Apple select a.sub\n";
        let detail = invalid(
            open(
                &rt,
                &[V1, ADD_APPLE],
                vec![bundle("web-1.0", &[V1], "", apple)],
            ),
            "web-1.0",
        );
        assert!(detail.contains("subs"), "{detail}");
    });
}

#[test]
fn a_bundle_is_checked_at_its_pinned_version() {
    once(10, |rt| {
        let db = opened(
            &rt,
            &[V1, RENAME_NAME],
            vec![bundle("web-1.0", &[V1], "", NAMES)],
        );
        assert!(db.registered("web-1.0").is_ok());
        let detail = invalid(
            open(
                &rt,
                &[V1, RENAME_NAME],
                vec![bundle("web-2.0", &[V1, RENAME_NAME], "", NAMES)],
            ),
            "web-2.0",
        );
        assert!(detail.contains("names"), "{detail}");
    });
}

#[test]
fn a_call_of_a_bundle_pinned_before_a_rename_reads_and_writes_the_renamed_field() {
    once(11, |rt| {
        let db = opened(
            &rt,
            &[V1, RENAME_NAME],
            vec![bundle("web-1.0", &[V1], "", NAMES)],
        );
        let web = db.registered("web-1.0").expect("web-1.0 binds");
        let added = web.call(2, &[Value::text("ann")], &CallOptions::default());
        assert!(matches!(added, Ok(Called::Committed(_))), "{added:?}");
        assert_eq!(vec!["\"ann\""], rows(call(&db, "web-1.0", 1, &[])));
        assert_eq!(
            vec!["\"ann\""],
            query(&db, "from u in users select u.display")
        );
    });
}

#[test]
fn a_bundle_pinned_where_a_constructor_is_declared_registers_with_its_arm() {
    let covering = "import auth
query logins (u: Key users) -> Set Text =
  from i in identities where i.user == u
  select match i
    | EmailPass e -> e.email
    | Google g -> g.gid
    | Apple a -> a.sub
";
    let mut sim = Sim::new(12, Config::default());
    run(&mut sim, |rt| {
        let log = [V1, ADD_APPLE];
        let db = opened(&rt, &log, vec![bundle("web-2.5", &log, "", covering)]);
        assert!(db.registered("web-2.5").is_ok());
    });
    run(&mut sim, |rt| {
        let log = [V1, ADD_APPLE, ENABLE_APPLE];
        let db = opened(&rt, &log, vec![bundle("web-3.0", &log, "", covering)]);
        assert!(db.registered("web-3.0").is_ok());
    });
}

#[test]
fn registering_a_bundle_again_with_identical_content_is_a_no_op() {
    let mut sim = Sim::new(13, Config::default());
    run(&mut sim, |rt| {
        opened(&rt, &[V1], vec![bundle("web-1.0", &[V1], "", NAMES)]);
    });
    run(&mut sim, |rt| {
        let db = opened(&rt, &[V1], vec![bundle("web-1.0", &[V1], "", NAMES)]);
        assert_eq!(vec!["web-1.0 active"], states(&db));
        assert_eq!(
            vec!["2"],
            query(&db, "count(r for r in catalog::registered)")
        );
    });
}

#[test]
fn a_bundle_name_registered_again_with_different_content_is_refused() {
    let mut sim = Sim::new(14, Config::default());
    run(&mut sim, |rt| {
        opened(&rt, &[V1], vec![bundle("web-1.0", &[V1], "", NAMES)]);
    });
    run(&mut sim, |rt| {
        let other = NAMES.replace("u.name", "u.name ++ \"!\"");
        let detail = invalid(
            open(&rt, &[V1], vec![bundle("web-1.0", &[V1], "", &other)]),
            "web-1.0",
        );
        assert!(detail.contains("different content"), "{detail}");
    });
}

#[test]
fn a_bundle_with_another_lease_is_different_content() {
    let mut sim = Sim::new(15, Config::default());
    run(&mut sim, |rt| {
        opened(
            &rt,
            &[V1],
            vec![bundle("web-1.0", &[V1], " lease 2d", NAMES)],
        );
    });
    run(&mut sim, |rt| {
        invalid(
            open(
                &rt,
                &[V1],
                vec![bundle("web-1.0", &[V1], " lease 3d", NAMES)],
            ),
            "web-1.0",
        );
    });
}

#[test]
fn an_open_supersedes_every_bundle_of_an_earlier_open_it_is_not_given() {
    let mut sim = Sim::new(16, Config::default());
    run(&mut sim, |rt| {
        opened(&rt, &[V1], vec![bundle("web-1.0", &[V1], "", NAMES)]);
    });
    run(&mut sim, |rt| {
        let db = opened(&rt, &[V1], vec![bundle("web-1.1", &[V1], "", NAMES)]);
        assert_eq!(vec!["web-1.0 superseded", "web-1.1 active"], states(&db));
        assert_eq!(
            RejectReason::BundleSuperseded {
                bundle: "web-1.0".into()
            },
            unbound(&db, "web-1.0")
        );
        assert!(db.registered("web-1.1").is_ok());
    });
}

#[test]
fn a_superseded_bundle_registered_again_with_identical_content_is_live_again() {
    let mut sim = Sim::new(17, Config::default());
    run(&mut sim, |rt| {
        opened(&rt, &[V1], vec![bundle("web-1.0", &[V1], "", NAMES)]);
    });
    run(&mut sim, |rt| {
        opened(&rt, &[V1], vec![bundle("web-1.1", &[V1], "", NAMES)]);
    });
    run(&mut sim, |rt| {
        let db = opened(&rt, &[V1], vec![bundle("web-1.0", &[V1], "", NAMES)]);
        assert_eq!(vec!["web-1.0 active", "web-1.1 superseded"], states(&db));
        assert_eq!(Vec::<String>::new(), rows(call(&db, "web-1.0", 1, &[])));
    });
}

#[test]
fn supersession_commits_before_the_migrations_and_survives_a_refused_one() {
    let mut sim = Sim::new(18, Config::default());
    run(&mut sim, |rt| {
        opened(&rt, &[V1], vec![bundle("web-1.0", &[V1], "", NAMES)]);
    });
    run(&mut sim, |rt| {
        let bad = "module auth\ndrop relation nothing\n";
        let got = open(&rt, &[V1, bad], vec![bundle("web-1.1", &[V1], "", NAMES)]);
        assert!(
            matches!(got, Err(OpenError::MigrationRefused { version: 2, .. })),
            "{got:?}"
        );
    });
    run(&mut sim, |rt| {
        let db = opened(&rt, &[V1], Vec::new());
        assert_eq!(
            vec!["web-1.0 superseded", "web-1.1 superseded"],
            states(&db)
        );
        assert_eq!(
            vec!["0"],
            query(
                &db,
                r#"count(r for r in catalog::registered where r.bundle.name == "web-1.1")"#
            )
        );
    });
}

#[test]
fn the_registrations_of_an_open_take_effect_together_or_not_at_all() {
    let mut sim = Sim::new(19, Config::default());
    run(&mut sim, |rt| {
        opened(&rt, &[V1], Vec::new());
    });
    run(&mut sim, |rt| {
        let broken = "import auth\nquery bad () -> Set Text = from u in users select u.nope\n";
        let got = open(
            &rt,
            &[V1],
            vec![
                bundle("good-1.0", &[V1], "", NAMES),
                bundle("bad-1.0", &[V1], "", broken),
            ],
        );
        invalid(got, "bad-1.0");
    });
    run(&mut sim, |rt| {
        let db = opened(&rt, &[V1], Vec::new());
        assert_eq!(
            vec!["0"],
            query(&db, "count(r for r in catalog::registered)")
        );
        assert_eq!(
            RejectReason::BundleSuperseded {
                bundle: "good-1.0".into()
            },
            unbound(&db, "good-1.0")
        );
    });
}

#[test]
fn a_bundle_pinned_before_an_enabled_constructor_of_a_relation_it_reads_whole_is_stale() {
    once(20, |rt| {
        let whole =
            "import auth\nquery ids () -> Set Key users = from i in identities select i.user\n";
        let log = [V1, ADD_APPLE, ENABLE_APPLE];
        assert_eq!(
            RejectReason::BundleStale {
                bundle: "web-1.0".into(),
                ctor: "identities.Apple".into()
            },
            refusal(
                open(&rt, &log, vec![bundle("web-1.0", &[V1], "", whole)]),
                "web-1.0"
            )
        );
    });
}

#[test]
fn a_bundle_pinned_before_a_constructor_that_is_only_declared_is_not_stale() {
    once(21, |rt| {
        let whole =
            "import auth\nquery ids () -> Set Key users = from i in identities select i.user\n";
        let log = [V1, ADD_APPLE];
        let db = opened(&rt, &log, vec![bundle("web-1.0", &[V1], "", whole)]);
        assert!(db.registered("web-1.0").is_ok());
    });
}

#[test]
fn a_bundle_ranging_over_a_listed_constructor_set_is_not_stale() {
    once(22, |rt| {
        let google =
            "import auth\nquery gids () -> Set Text = from g in identities.Google select g.gid\n";
        let log = [V1, ADD_APPLE, ENABLE_APPLE];
        let db = opened(&rt, &log, vec![bundle("web-1.0", &[V1], "", google)]);
        assert!(db.registered("web-1.0").is_ok());
    });
}

#[test]
fn a_bundle_following_a_reference_that_admits_an_enabled_constructor_is_stale() {
    once(23, |rt| {
        let follows =
            "import auth\nquery whose () -> Set Key users = from s in sessions select s.who.user\n";
        let log = [V1, ADD_APPLE, ENABLE_APPLE];
        assert_eq!(
            RejectReason::BundleStale {
                bundle: "web-1.0".into(),
                ctor: "identities.Apple".into()
            },
            refusal(
                open(&rt, &log, vec![bundle("web-1.0", &[V1], "", follows)]),
                "web-1.0"
            )
        );
    });
}

const HIDING: &str = "import auth
query ids () -> Set Key users = from i in identities select i.user
query whose () -> Set Key users = from s in sessions select s.who.user
query whole () -> Set Identity = from s in sessions select (match s.who
  | EmailPass e -> e
  | Google g -> g)
query names () -> Set Text = from s in sessions select (match s.who
  | EmailPass e -> e.email
  | Google g -> g.gid)
";

/// Open `[V1, ADD_APPLE, ENABLE_APPLE]` with `HIDING` registered under `hide new constructors`,
/// holding a user, a Google identity, an Apple identity and a session of the Google one.
fn hiding(rt: &SimRuntime) -> Database<SimRuntime> {
    let db = opened(
        rt,
        &[V1, ADD_APPLE, ENABLE_APPLE],
        vec![bundle("web-1.0", &[V1], " hide new constructors", HIDING)],
    );
    write(
        &db,
        r#"insert users { id: users.key(1), name: "ann", tier: Free }
           insert identities.Google { id: identities.key(1), user: users.key(1), gid: "g-ann" }
           insert identities.Apple { id: identities.key(2), user: users.key(1), sub: "a-ann" }
           insert sessions { id: sessions.key(1), who: identities.key(1) }"#,
    )
    .expect("the rows insert");
    db
}

/// Returns the fault call `ordinal` of `web-1.0` ends in.
fn fault(db: &Database<SimRuntime>, ordinal: u32) -> Fault {
    let web = db.registered("web-1.0").expect("web-1.0 binds");
    match web.call(ordinal, &[], &CallOptions::default()) {
        Err(e) => match reason(&e) {
            RejectReason::Faulted { fault } => *fault,
            other => panic!("call {ordinal} is refused otherwise: {other:?}"),
        },
        Ok(got) => panic!("call {ordinal} does not fault: {got:?}"),
    }
}

#[test]
fn hide_new_constructors_exempts_whole_reads_and_references() {
    once(24, |rt| {
        let db = hiding(&rt);
        assert!(db.registered("web-1.0").is_ok());
        let ann = query(&db, "users.key(1)");
        assert_eq!(
            ann,
            rows(call(&db, "web-1.0", 1, &[])),
            "the Apple row is hidden"
        );
        assert_eq!(ann, rows(call(&db, "web-1.0", 2, &[])));
    });
}

#[test]
fn a_hide_new_constructors_bundle_reads_a_known_row_through_a_reference() {
    once(91, |rt| {
        let db = hiding(&rt);
        assert_eq!(
            query(&db, "identities.key(1).user"),
            rows(call(&db, "web-1.0", 2, &[]))
        );
        assert_eq!(
            query(&db, "from i in identities.Google select i"),
            rows(call(&db, "web-1.0", 3, &[]))
        );
        assert_eq!(vec![r#""g-ann""#], rows(call(&db, "web-1.0", 4, &[])));
    });
}

/// Returns the fault call `ordinal` of `web-1.0` ends in once a session references the Apple
/// identity.
fn hidden_fault(rt: &SimRuntime, ordinal: u32) -> Fault {
    let db = hiding(rt);
    write(
        &db,
        "insert sessions { id: sessions.key(2), who: identities.key(2) }",
    )
    .expect("the session of the Apple identity inserts");
    fault(&db, ordinal)
}

#[test]
fn a_hide_new_constructors_bundle_faults_reading_a_field_through_a_reference_to_a_hidden_row() {
    once(92, |rt| assert_eq!(Fault::NoSuchRow, hidden_fault(&rt, 2)));
}

#[test]
fn a_hide_new_constructors_bundle_faults_returning_a_hidden_row_whole() {
    once(93, |rt| assert_eq!(Fault::NoSuchRow, hidden_fault(&rt, 3)));
}

#[test]
fn a_hide_new_constructors_bundle_faults_matching_a_hidden_row() {
    once(94, |rt| assert_eq!(Fault::NoSuchRow, hidden_fault(&rt, 4)));
}

#[test]
fn a_bundle_reading_a_field_whose_type_reaches_an_enabled_value_constructor_is_stale() {
    let tiers = "import auth\nquery tiers () -> Set Tier = from u in users select u.tier\n";
    let log = [V1, ADD_GOLD, ENABLE_GOLD];
    once(25, |rt| {
        assert_eq!(
            RejectReason::BundleStale {
                bundle: "web-1.0".into(),
                ctor: "Tier.Gold".into()
            },
            refusal(
                open(&rt, &log, vec![bundle("web-1.0", &[V1], "", tiers)]),
                "web-1.0"
            )
        );
    });
    once(26, |rt| {
        let hiding = bundle("web-1.0", &[V1], " hide new constructors", tiers);
        assert!(matches!(
            refusal(open(&rt, &log, vec![hiding]), "web-1.0"),
            RejectReason::BundleStale { .. }
        ));
    });
}

#[test]
fn a_bundle_returning_whole_rows_reads_every_field() {
    once(27, |rt| {
        let whole = "import auth\nquery everyone () -> Set User = users\n";
        let log = [V1, ADD_GOLD, ENABLE_GOLD];
        assert!(matches!(
            refusal(
                open(&rt, &log, vec![bundle("web-1.0", &[V1], "", whole)]),
                "web-1.0"
            ),
            RejectReason::BundleStale { .. }
        ));
    });
}

#[test]
fn a_bundle_reading_no_field_that_reaches_the_value_constructor_is_not_stale() {
    once(28, |rt| {
        let log = [V1, ADD_GOLD, ENABLE_GOLD];
        let db = opened(&rt, &log, vec![bundle("web-1.0", &[V1], "", NAMES)]);
        assert!(db.registered("web-1.0").is_ok());
    });
}

#[test]
fn a_downgrade_whose_bundle_reads_a_constructor_enabled_since_its_pin_is_refused_and_changes_nothing()
 {
    let whole = "import auth\nquery ids () -> Set Key users = from i in identities select i.user\n";
    let log = [V1, ADD_APPLE, ENABLE_APPLE];
    let mut sim = Sim::new(29, Config::default());
    run(&mut sim, |rt| {
        opened(&rt, &log, vec![bundle("web-3.0", &log, "", whole)]);
    });
    run(&mut sim, |rt| {
        let got = open(&rt, &[V1], vec![bundle("web-1.0", &[V1], "", whole)]);
        match got {
            Err(OpenError::DowngradeRefused { cause }) => {
                assert!(cause.contains("identities.Apple"), "{cause}");
            }
            other => panic!("the downgrade is not refused: {other:?}"),
        }
    });
    run(&mut sim, |rt| {
        let db = opened(&rt, &log, vec![bundle("web-3.0", &log, "", whole)]);
        assert_eq!(vec!["web-3.0 active"], states(&db));
    });
}

#[test]
fn a_downgrade_whose_bundle_reads_nothing_enabled_since_opens_at_the_newer_version() {
    let mut sim = Sim::new(30, Config::default());
    run(&mut sim, |rt| {
        opened(&rt, &[V1, ADD_GOLD, ENABLE_GOLD], Vec::new());
    });
    run(&mut sim, |rt| {
        let db = opened(&rt, &[V1], vec![bundle("web-1.0", &[V1], "", NAMES)]);
        assert!(db.registered("web-1.0").is_ok());
    });
}

#[test]
fn a_bundle_whose_lease_and_grace_have_passed_is_expired() {
    once(31, |rt| {
        let db = opened(
            &rt,
            &[V1],
            vec![bundle("web-1.0", &[V1], " lease 1d", NAMES)],
        );
        let web = db.registered("web-1.0").expect("web-1.0 binds");
        sleep(&rt, DAY + 2 * HOUR);
        let expired = RejectReason::BundleExpired {
            bundle: "web-1.0".into(),
            query: String::new(),
        };
        assert_eq!(expired, unbound(&db, "web-1.0"));
        assert_eq!(
            "the lease of bundle `web-1.0` expired",
            expired.to_string(),
            "a bind names no query"
        );
        match web.call(1, &[], &CallOptions::default()) {
            Err(Error::Rejected {
                reason: reason @ RejectReason::BundleExpired { .. },
            }) => {
                assert_eq!(
                    RejectReason::BundleExpired {
                        bundle: "web-1.0".into(),
                        query: "names".into()
                    },
                    reason
                );
                assert_eq!(
                    "the lease of bundle `web-1.0` expired before `names`",
                    reason.to_string()
                );
            }
            other => panic!("the call is not refused expired: {other:?}"),
        }
    });
}

#[test]
fn a_bundle_within_its_grace_still_serves_but_its_lease_is_not_renewed() {
    once(32, |rt| {
        let db = opened(
            &rt,
            &[V1],
            vec![bundle("web-1.0", &[V1], " lease 1d", NAMES)],
        );
        sleep(&rt, DAY + HOUR / 2);
        assert_eq!(Vec::<String>::new(), rows(call(&db, "web-1.0", 1, &[])));
        sleep(&rt, HOUR * 2 / 3);
        assert!(matches!(
            unbound(&db, "web-1.0"),
            RejectReason::BundleExpired { .. }
        ));
    });
}

#[test]
fn a_call_renews_the_lease() {
    once(33, |rt| {
        let db = opened(
            &rt,
            &[V1],
            vec![bundle("web-1.0", &[V1], " lease 1d", NAMES)],
        );
        for _ in 0..3 {
            sleep(&rt, 20 * HOUR);
            assert_eq!(Vec::<String>::new(), rows(call(&db, "web-1.0", 1, &[])));
        }
    });
}

#[test]
fn a_bundle_with_no_lease_clause_holds_its_lease_thirty_days() {
    once(34, |rt| {
        let db = opened(&rt, &[V1], vec![bundle("web-1.0", &[V1], "", NAMES)]);
        sleep(&rt, 29 * DAY);
        assert!(db.registered("web-1.0").is_ok());
        sleep(&rt, 31 * DAY + 2 * HOUR);
        assert!(matches!(
            unbound(&db, "web-1.0"),
            RejectReason::BundleExpired { .. }
        ));
    });
}

#[test]
fn the_default_lease_is_the_file_setting() {
    let mut sim = Sim::new(46, Config::default());
    run(&mut sim, |rt| {
        let db = opened(&rt, &[V1], Vec::new());
        db.query_text(
            "set setting DefaultLease = { lease: 2d }",
            &[],
            &CallOptions::default(),
        )
        .expect("the setting");
    });
    run(&mut sim, |rt| {
        let db = opened(&rt, &[V1], vec![bundle("web-1.0", &[V1], "", NAMES)]);
        sleep(&rt, 2 * DAY + 2 * HOUR);
        assert!(matches!(
            unbound(&db, "web-1.0"),
            RejectReason::BundleExpired { .. }
        ));
    });
}

#[test]
fn the_lease_grace_is_the_file_setting_from_its_commit() {
    once(47, |rt| {
        let db = opened(
            &rt,
            &[V1],
            vec![bundle("web-1.0", &[V1], " lease 1d", NAMES)],
        );
        db.query_text(
            "set setting LeaseGrace = { grace: 3h }",
            &[],
            &CallOptions::default(),
        )
        .expect("the setting");
        sleep(&rt, DAY + 2 * HOUR);
        assert!(
            db.registered("web-1.0").is_ok(),
            "two hours past the lease is within a grace of three"
        );
        sleep(&rt, 2 * HOUR);
        assert!(matches!(
            unbound(&db, "web-1.0"),
            RejectReason::BundleExpired { .. }
        ));
    });
}

#[test]
fn a_renewal_is_recorded_and_survives_reopen() {
    let mut sim = Sim::new(35, Config::default());
    let renewed =
        |db: &Database<SimRuntime>| query(db, "from b in catalog::bundles select b.renewed");
    let first = run(&mut sim, |rt| {
        let db = opened(
            &rt,
            &[V1],
            vec![bundle("web-1.0", &[V1], " lease 1d", NAMES)],
        );
        let first = renewed(&db);
        sleep(&rt, 4 * HOUR);
        call(&db, "web-1.0", 1, &[]);
        let second = renewed(&db);
        assert_ne!(
            first, second,
            "a call past an eighth of the lease renews it"
        );
        second
    });
    run(&mut sim, |rt| {
        let db = opened(
            &rt,
            &[V1],
            vec![bundle("web-1.0", &[V1], " lease 1d", NAMES)],
        );
        assert_eq!(
            first,
            renewed(&db),
            "a reopen within an eighth of the lease writes nothing"
        );
    });
}

#[test]
fn a_call_refused_for_its_arguments_does_not_renew_the_lease() {
    once(37, |rt| {
        let db = opened(
            &rt,
            &[V1, RENAME_NAME],
            vec![bundle("web-1.0", &[V1], " lease 1d", NAMES)],
        );
        let web = db.registered("web-1.0").expect("web-1.0 binds");
        let renewed = || query(&db, "from b in catalog::bundles select b.renewed");
        let before = renewed();
        sleep(&rt, 4 * HOUR);
        let refused = web.call(2, &[], &CallOptions::default());
        assert!(
            matches!(
                refused.as_ref().map_err(reason),
                Err(RejectReason::Invalid { .. })
            ),
            "{refused:?}"
        );
        assert_eq!(before, renewed(), "a refused call writes no renewal");
    });
}

const IDENTITIES: &str = "import auth
query identities () -> I64 = count(i for i in identities)
";

#[test]
fn a_bundle_live_before_the_open_that_ships_it_again_blocks_an_enable_it_reads() {
    let mut sim = Sim::new(39, Config::default());
    run(&mut sim, |rt| {
        opened(&rt, &[V1], vec![bundle("web-1.0", &[V1], "", IDENTITIES)]);
    });
    run(&mut sim, |rt| {
        let got = open(
            &rt,
            &[V1, ADD_APPLE, ENABLE_APPLE],
            vec![bundle("web-1.0", &[V1], "", IDENTITIES)],
        );
        match got {
            Err(OpenError::MigrationRefused { version: 3, reason }) => {
                assert!(reason.to_string().contains("web-1.0"), "{reason}");
            }
            other => panic!("the enable is not refused for web-1.0: {other:?}"),
        }
    });
}

const NO_BLANK: &str =
    "module auth\nadd constraint no_blank = from u in users where u.name == \"\" select u\n";
const DROP_NO_BLANK: &str = "module auth\ndrop constraint no_blank\n";

#[test]
fn a_drop_stays_pending_while_a_bundle_live_before_the_open_reads_it() {
    let mut sim = Sim::new(40, Config::default());
    run(&mut sim, |rt| {
        opened(
            &rt,
            &[V1, NO_BLANK],
            vec![bundle("web-1.0", &[V1, NO_BLANK], "", NAMES)],
        );
    });
    run(&mut sim, |rt| {
        let db = opened(
            &rt,
            &[V1, NO_BLANK, DROP_NO_BLANK],
            vec![bundle("web-1.0", &[V1, NO_BLANK], "", NAMES)],
        );
        let blank = db.query_text(
            "insert users { name: \"\", tier: Free }",
            &[],
            &CallOptions::default(),
        );
        match blank {
            Err(Error::Rejected {
                reason: RejectReason::Violated { constraint, .. },
            }) => assert_eq!("auth::no_blank", constraint),
            other => panic!("the pending-dropped constraint is not enforced: {other:?}"),
        }
    });
}

/// Open a file at `[V1, NO_BLANK]` shipping `web-1.0` pinned there, then reopen it with
/// `DROP_NO_BLANK` applied while `web-1.0` holds it pending, and run `body` at a third open
/// shipping `shipped`.
fn third_open_after_a_pending_drop(
    seed: u64,
    shipped: Vec<Registration>,
    body: impl FnOnce(&Database<SimRuntime>),
) {
    let pinned = || bundle("web-1.0", &[V1, NO_BLANK], "", NAMES);
    let log = [V1, NO_BLANK, DROP_NO_BLANK];
    let mut sim = Sim::new(seed, Config::default());
    run(&mut sim, |rt| {
        opened(&rt, &[V1, NO_BLANK], vec![pinned()]);
    });
    run(&mut sim, |rt| {
        opened(&rt, &log, vec![pinned()]);
    });
    run(&mut sim, |rt| body(&opened(&rt, &log, shipped)));
}

fn insert_blank(db: &Database<SimRuntime>) -> Result<(), Error> {
    db.query_text(
        "insert users { name: \"\", tier: Free }",
        &[],
        &CallOptions::default(),
    )
    .map(drop)
}

#[test]
fn a_drop_stays_pending_at_a_later_open_that_ships_its_holder_again() {
    third_open_after_a_pending_drop(
        41,
        vec![bundle("web-1.0", &[V1, NO_BLANK], "", NAMES)],
        |db| match insert_blank(db) {
            Err(Error::Rejected {
                reason: RejectReason::Violated { constraint, .. },
            }) => assert_eq!("auth::no_blank", constraint),
            other => panic!("the pending-dropped constraint is not enforced: {other:?}"),
        },
    );
}

#[test]
fn a_drop_takes_effect_at_the_open_that_supersedes_its_last_holder() {
    third_open_after_a_pending_drop(42, Vec::new(), |db| {
        insert_blank(db).expect("the drop has taken effect");
    });
}

#[test]
fn a_drop_that_took_effect_stays_in_effect_when_its_holder_is_shipped_again() {
    let pinned = || bundle("web-1.0", &[V1, NO_BLANK], "", NAMES);
    let log = [V1, NO_BLANK, DROP_NO_BLANK];
    let mut sim = Sim::new(43, Config::default());
    run(&mut sim, |rt| {
        opened(&rt, &[V1, NO_BLANK], vec![pinned()]);
    });
    run(&mut sim, |rt| {
        opened(&rt, &log, vec![pinned()]);
    });
    run(&mut sim, |rt| {
        opened(&rt, &log, Vec::new());
    });
    run(&mut sim, |rt| {
        let db = opened(&rt, &log, vec![pinned()]);
        insert_blank(&db).expect("the drop stays in effect");
    });
}

/// An object added by migration `added` and dropped, pending while a bundle holds it, by
/// migration `dropped`: `first` writes a row and `again` a row the object refuses beside it.
struct Dropping {
    added: &'static str,
    dropped: &'static str,
    first: &'static str,
    again: &'static str,
}

const UNIQUE: Dropping = Dropping {
    added: "module auth\nadd unique (name) in users\n",
    dropped: "module auth\ndrop unique (name) in users\n",
    first: "insert users { name: \"ann\", tier: Free }",
    again: "insert users { name: \"ann\", tier: Free }",
};

const OLD_KEY: Dropping = Dropping {
    added: "module auth\nadd type Label = { name: Text, lang: Text }\nadd relation labels : Label key name\n",
    dropped: "module auth\nrekey relation labels key (name, lang)\n",
    first: "insert labels { name: \"hi\", lang: \"en\" }",
    again: "insert labels { name: \"hi\", lang: \"fr\" }",
};

fn write(db: &Database<SimRuntime>, text: &str) -> Result<(), Error> {
    db.query_text(text, &[], &CallOptions::default()).map(drop)
}

/// Open a file at `[V1, d.added]` holding the row `d.first` writes and shipping `web-1.0` pinned
/// there, then reopen it with `d.dropped` applied while `web-1.0` holds the drop pending, and run
/// `held` over that open and `third` over a third open shipping `shipped`.
fn after_a_pending_drop(
    seed: u64,
    d: &Dropping,
    shipped: impl FnOnce(&[&str]) -> Vec<Registration>,
    held: impl FnOnce(&Database<SimRuntime>),
    third: impl FnOnce(&Database<SimRuntime>),
) {
    let pins = [V1, d.added];
    let pinned = || bundle("web-1.0", &pins, "", NAMES);
    let log = [V1, d.added, d.dropped];
    let mut sim = Sim::new(seed, Config::default());
    run(&mut sim, |rt| {
        let db = opened(&rt, &pins, vec![pinned()]);
        write(&db, d.first).expect("the first row inserts");
    });
    run(&mut sim, |rt| held(&opened(&rt, &log, vec![pinned()])));
    run(&mut sim, |rt| third(&opened(&rt, &log, shipped(&pins))));
}

fn violates(object: &str, got: Result<(), Error>) {
    match got {
        Err(Error::Rejected {
            reason: RejectReason::Violated { constraint, .. },
        }) => assert!(constraint.contains(object), "{constraint}"),
        other => panic!("the pending-dropped {object} is not enforced: {other:?}"),
    }
}

fn enforced_while_held(seed: u64, d: &Dropping, object: &str) {
    after_a_pending_drop(
        seed,
        d,
        |pins| vec![bundle("web-1.0", pins, "", NAMES)],
        |db| violates(object, write(db, d.again)),
        |db| violates(object, write(db, d.again)),
    );
}

fn in_effect_once_superseded(seed: u64, d: &Dropping, object: &str) {
    after_a_pending_drop(
        seed,
        d,
        |_| Vec::new(),
        |db| violates(object, write(db, d.again)),
        |db| write(db, d.again).expect("the drop has taken effect"),
    );
}

#[test]
fn a_pending_dropped_unique_is_enforced_for_writers_while_a_bundle_holds_it() {
    enforced_while_held(47, &UNIQUE, "unique");
}

#[test]
fn a_pending_dropped_unique_takes_effect_at_the_open_that_supersedes_its_last_holder() {
    in_effect_once_superseded(48, &UNIQUE, "unique");
}

#[test]
fn a_relaxed_old_key_is_enforced_for_writers_while_a_bundle_holds_it() {
    enforced_while_held(49, &OLD_KEY, "key name");
}

#[test]
fn a_relaxed_old_key_takes_effect_at_the_open_that_supersedes_its_last_holder() {
    in_effect_once_superseded(50, &OLD_KEY, "key name");
}

#[test]
fn an_expired_bundle_registered_again_is_live_with_a_new_lease() {
    let mut sim = Sim::new(36, Config::default());
    run(&mut sim, |rt| {
        let db = opened(
            &rt,
            &[V1],
            vec![bundle("web-1.0", &[V1], " lease 1d", NAMES)],
        );
        sleep(&rt, 2 * DAY);
        assert!(matches!(
            unbound(&db, "web-1.0"),
            RejectReason::BundleExpired { .. }
        ));
    });
    run(&mut sim, |rt| {
        let db = opened(
            &rt,
            &[V1],
            vec![bundle("web-1.0", &[V1], " lease 1d", NAMES)],
        );
        assert_eq!(Vec::<String>::new(), rows(call(&db, "web-1.0", 1, &[])));
    });
}

#[test]
fn retire_bundle_refuses_binds_and_calls_from_its_commit() {
    once(37, |rt| {
        let db = opened(&rt, &[V1], vec![bundle("web-1.0", &[V1], "", NAMES)]);
        let web = db.registered("web-1.0").expect("web-1.0 binds");
        let retired = query(&db, "retire bundle web-1.0");
        assert_eq!(1, retired.len(), "{retired:?}");
        assert!(retired[0].contains(r#"name: "web-1.0""#), "{retired:?}");
        assert!(retired[0].contains("Retired"), "{retired:?}");
        let gone = RejectReason::Retired {
            bundle: "web-1.0".into(),
        };
        assert_eq!(gone, unbound(&db, "web-1.0"));
        match web.call(1, &[], &CallOptions::default()) {
            Err(e) => assert_eq!(&gone, reason(&e)),
            Ok(_) => panic!("a call on a retired bundle runs"),
        }
        assert_eq!(vec!["web-1.0 retired"], states(&db));
    });
}

#[test]
fn retiring_a_bundle_that_is_not_registered_is_refused() {
    once(38, |rt| {
        let db = opened(&rt, &[V1], Vec::new());
        match db.query_text("retire bundle web-9.9", &[], &CallOptions::default()) {
            Err(e) => assert_eq!(
                &RejectReason::UnknownBundle {
                    bundle: "web-9.9".into()
                },
                reason(&e)
            ),
            Ok(_) => panic!("an unknown bundle retires"),
        }
    });
}

#[test]
fn a_retired_bundle_is_never_registered_again_and_is_not_superseded() {
    let mut sim = Sim::new(39, Config::default());
    run(&mut sim, |rt| {
        let db = opened(&rt, &[V1], vec![bundle("web-1.0", &[V1], "", NAMES)]);
        db.query_text("retire bundle web-1.0", &[], &CallOptions::default())
            .expect("web-1.0 retires");
    });
    run(&mut sim, |rt| {
        let db = opened(&rt, &[V1], Vec::new());
        assert_eq!(vec!["web-1.0 retired"], states(&db));
    });
    run(&mut sim, |rt| {
        assert_eq!(
            RejectReason::Retired {
                bundle: "web-1.0".into()
            },
            refusal(
                open(&rt, &[V1], vec![bundle("web-1.0", &[V1], "", NAMES)]),
                "web-1.0"
            )
        );
    });
}

#[test]
fn supersede_bundle_by_a_live_bundle_of_its_family_releases_it() {
    once(40, |rt| {
        let db = opened(
            &rt,
            &[V1],
            vec![
                bundle("web-1.0", &[V1], "", NAMES),
                bundle("web-1.1", &[V1], "", NAMES),
            ],
        );
        let got = query(&db, "supersede bundle web-1.0 by web-1.1");
        assert_eq!(1, got.len(), "{got:?}");
        assert!(got[0].contains("Superseded"), "{got:?}");
        assert_eq!(
            RejectReason::BundleSuperseded {
                bundle: "web-1.0".into()
            },
            unbound(&db, "web-1.0")
        );
        assert!(db.registered("web-1.1").is_ok());
    });
}

#[test]
fn supersede_bundle_by_a_bundle_of_another_family_or_one_not_live_is_refused() {
    once(41, |rt| {
        let db = opened(
            &rt,
            &[V1],
            vec![
                bundle("web-1.0", &[V1], "", NAMES),
                bundle("ios-1.0", &[V1], "", NAMES),
            ],
        );
        for text in [
            "supersede bundle web-1.0 by ios-1.0",
            "supersede bundle web-1.0 by web-9.9",
            "supersede bundle web-1.0 by web-1.0",
        ] {
            let got = db.query_text(text, &[], &CallOptions::default());
            assert!(got.is_err(), "`{text}` supersedes");
        }
        assert!(db.registered("web-1.0").is_ok());
    });
}

#[test]
fn a_superseded_bundle_handle_refuses_calls() {
    once(42, |rt| {
        let db = opened(
            &rt,
            &[V1],
            vec![
                bundle("web-1.0", &[V1], "", NAMES),
                bundle("web-1.1", &[V1], "", NAMES),
            ],
        );
        let web = db.registered("web-1.0").expect("web-1.0 binds");
        db.query_text(
            "supersede bundle web-1.0 by web-1.1",
            &[],
            &CallOptions::default(),
        )
        .expect("web-1.0 is superseded");
        match web.call(1, &[], &CallOptions::default()) {
            Err(e) => assert!(
                matches!(reason(&e), RejectReason::BundleSuperseded { .. }),
                "{e:?}"
            ),
            Ok(_) => panic!("a call on a superseded bundle runs"),
        }
    });
}

#[test]
fn explain_of_a_registered_call_checks_against_the_registered_bundle() {
    once(43, |rt| {
        let db = opened(&rt, &[V1], vec![bundle("web-1.0", &[V1], "", NAMES)]);
        let known = db.query_text("explain web-1.0::names()", &[], &CallOptions::default());
        if let Err(e) = &known {
            assert!(!format!("{e:?}").contains("not registered"), "{e:?}");
        }
        let unknown = db.query_text("explain web-9.9::names()", &[], &CallOptions::default());
        assert!(
            format!("{unknown:?}").contains("not registered"),
            "{unknown:?}"
        );
    });
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
fn admission_refused_arity_does_not_renew_the_lease() {
    once(60, |rt| {
        let db = opened(
            &rt,
            &[V1],
            vec![bundle("web-1.0", &[V1], " lease 1d", NAMES)],
        );
        let web = db.registered("web-1.0").expect("web binds");
        let renewed = || query(&db, "from b in catalog::bundles select b.renewed");
        let before = renewed();
        sleep(&rt, 4 * HOUR);
        let got = web.call(1, &[Value::Int(1)], &CallOptions::default());
        assert!(
            matches!(
                got,
                Err(Error::Rejected {
                    reason: RejectReason::Invalid { .. }
                })
            ),
            "{got:?}"
        );
        assert_eq!(before, renewed(), "a refused call writes no renewal");
    });
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

#[test]
fn admission_interactive_refusals_leave_renewal_and_transaction_usable() {
    once(62, |rt| {
        let db = opened(
            &rt,
            &[V1],
            vec![bundle("web-1.0", &[V1], " lease 1d", NAMES)],
        );
        let web = db.registered("web-1.0").expect("web binds");
        let renewed = || query(&db, "from b in catalog::bundles select b.renewed");
        let before = renewed();
        sleep(&rt, 4 * HOUR);
        let mut t = db
            .begin(&adtb::TxnOptions {
                read_only: true,
                ..adtb::TxnOptions::default()
            })
            .expect("begin read-only");
        for (ordinal, args) in [
            (1, vec![Value::Int(1)]),
            (2, vec![Value::Int(1)]),
            (2, vec![Value::text("ann")]),
        ] {
            let got = t.call(&web, ordinal, &args, &adtb::TxnCallOptions::default());
            assert!(
                matches!(
                    got,
                    Err(Error::Rejected {
                        reason: RejectReason::Invalid { .. }
                    })
                ),
                "{got:?}"
            );
            assert_eq!(
                before,
                renewed(),
                "a refused interactive call writes no renewal"
            );
        }
        assert!(query(&db, "from u in auth::users select u.name").is_empty());
        let got = t
            .call(&web, 1, &[], &adtb::TxnCallOptions::default())
            .expect("a refused call leaves the transaction usable");
        let adtb::InTxn::Rows(rows) = got else {
            panic!("names returns rows")
        };
        assert!(printed(rows).is_empty());
        assert_ne!(before, renewed(), "a valid interactive call renews");
        t.commit(None).expect("the transaction commits");
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

const RENAME_PAID: &str = "module auth\nrename constructor Tier.Paid to Pro\n";
const ADD_VERIFIED: &str = "module auth\nadd field Identity.EmailPass.verified: Bool fill false\n";
const DEFAULT_VERIFIED: &str = "module auth\nadd default identities.EmailPass.verified = true\n";
const ADD_HASH: &str = "module auth\nadd field Identity.EmailPass.hash: Text fill \"none\"\n";
const DROP_HASH: &str = "module auth\ndrop field Identity.EmailPass.hash\n";
const DEPRECATE_GOOGLE: &str = "module auth\ndeprecate identities.Google\n";

/// The calls of a bundle over identities, in ordinal order: `join` 1, `link` 2, `reword` 3,
/// `convert` 4, `emails` 5, `logins` 6.
const IDS: &str = r"import auth
txn join (n: Text, e: Text) -> Key identities =
  transact
    let u = insert users { name: n, tier: Free }
    let i = insert identities.EmailPass { user: key u, email: e }
    return key i
txn link (n: Text, g: Text) -> Key identities =
  transact
    let u = insert users { name: n, tier: Free }
    let i = insert identities.Google { user: key u, gid: g }
    return key i
txn reword (e: Text, to: Text) -> Text =
  transact
    update i in identities.EmailPass where i.email == e set EmailPass { i with email: to }
    return to
txn convert (g: Text, e: Text) -> Text =
  transact
    update i in identities.Google where i.gid == g set EmailPass { email: e }
    return e
query emails () -> Set Text = from i in identities.EmailPass select i.email
query logins () -> Set Text =
  from i in identities
  select match i
    | EmailPass e -> e.email
    | Google g -> g.gid
";

const JOIN: u32 = 1;
const LINK: u32 = 2;
const REWORD: u32 = 3;
const CONVERT: u32 = 4;
const EMAILS: u32 = 5;
const LOGINS: u32 = 6;

fn texts(values: &[&str]) -> Vec<Value> {
    values.iter().map(|v| Value::text(v)).collect()
}

/// Call the transaction `ordinal` of `name` with the text arguments `args`.
fn wrote(db: &Database<SimRuntime>, name: &str, ordinal: u32, args: &[&str]) {
    match call(db, name, ordinal, &texts(args)) {
        Called::Committed(_) => {}
        other => panic!("call {ordinal} of {name} commits nothing: {other:?}"),
    }
}

/// Returns each email of `identities.EmailPass` with its `verified`, as `email:verified`.
fn verified(db: &Database<SimRuntime>) -> Vec<String> {
    query(
        db,
        r#"from i in identities.EmailPass select i.email ++ ":" ++ (if i.verified then "t" else "f")"#,
    )
    .into_iter()
    .map(|s| s.trim_matches('"').to_owned())
    .collect()
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

#[test]
fn a_pinned_call_takes_its_arguments_and_encodes_its_results_at_its_pin() {
    let body = "import auth
query tiered (t: Tier) -> Set Text = from u in users where u.tier == t select u.name
query tiers () -> Set Tier = from u in users select u.tier
query people () -> Set User = from u in users select u
";
    once(60, |rt| {
        let log = [V1, RENAME_PAID, RENAME_NAME];
        let db = opened(&rt, &log, vec![bundle("web-1.0", &[V1], "", body)]);
        write(&db, "insert users { display: \"ann\", tier: Pro }").expect("the write applies");
        write(&db, "insert users { display: \"bo\", tier: Free }").expect("the write applies");
        let paid = Value::tag(1);
        assert_eq!(vec!["\"ann\""], rows(call(&db, "web-1.0", 1, &[paid])));
        assert_eq!(
            vec!["Free", "Paid"],
            sorted_rows(call(&db, "web-1.0", 2, &[]))
        );
        assert_eq!(
            vec![
                r#"User { id: users.key(1), name: "ann", tier: Paid }"#,
                r#"User { id: users.key(2), name: "bo", tier: Free }"#,
            ],
            sorted_rows(call(&db, "web-1.0", 3, &[]))
        );
    });
}

fn sorted_rows(called: Called) -> Vec<String> {
    let mut r = rows(called);
    r.sort();
    r
}

#[test]
fn a_pinned_insert_takes_an_added_fields_default_else_its_fill() {
    let mut sim = Sim::new(61, Config::default());
    let pin = [V1];
    run(&mut sim, |rt| {
        let db = opened(
            &rt,
            &[V1, ADD_VERIFIED],
            vec![bundle("web-1.0", &pin, "", IDS)],
        );
        wrote(&db, "web-1.0", JOIN, &["ann", "a@x"]);
        assert_eq!(vec!["a@x:f"], verified(&db), "the fill, with no default");
        assert_eq!(vec!["\"a@x\""], rows(call(&db, "web-1.0", EMAILS, &[])));
    });
    run(&mut sim, |rt| {
        let log = [V1, ADD_VERIFIED, DEFAULT_VERIFIED];
        let db = opened(&rt, &log, vec![bundle("web-1.0", &pin, "", IDS)]);
        wrote(&db, "web-1.0", JOIN, &["bo", "b@x"]);
        assert_eq!(vec!["a@x:f", "b@x:t"], verified(&db), "the current default");
    });
}

#[test]
fn a_pinned_update_keeps_the_rows_added_field_and_a_retag_takes_the_default() {
    once(62, |rt| {
        let log = [V1, ADD_VERIFIED, DEFAULT_VERIFIED];
        let db = opened(&rt, &log, vec![bundle("web-1.0", &[V1], "", IDS)]);
        wrote(&db, "web-1.0", JOIN, &["ann", "a@x"]);
        write(
            &db,
            "update i in identities.EmailPass set EmailPass { i with verified: false }",
        )
        .expect("the write applies");
        wrote(&db, "web-1.0", REWORD, &["a@x", "a@y"]);
        assert_eq!(
            vec!["a@y:f"],
            verified(&db),
            "an update keeps the row's value"
        );
        wrote(&db, "web-1.0", LINK, &["bo", "g1"]);
        wrote(&db, "web-1.0", CONVERT, &["g1", "b@x"]);
        assert_eq!(
            vec!["a@y:f", "b@x:t"],
            verified(&db),
            "a retag from another constructor takes the default"
        );
    });
}

#[test]
fn a_pinned_retag_with_no_default_takes_the_fill() {
    once(63, |rt| {
        let log = [V1, ADD_VERIFIED];
        let db = opened(&rt, &log, vec![bundle("web-1.0", &[V1], "", IDS)]);
        wrote(&db, "web-1.0", LINK, &["bo", "g1"]);
        wrote(&db, "web-1.0", CONVERT, &["g1", "b@x"]);
        assert_eq!(vec!["b@x:f"], verified(&db));
    });
}

#[test]
fn a_pinned_query_reads_through_an_added_constructor() {
    once(64, |rt| {
        let log = [V1, ADD_APPLE];
        let db = opened(&rt, &log, vec![bundle("web-1.0", &[V1], "", IDS)]);
        wrote(&db, "web-1.0", JOIN, &["ann", "a@x"]);
        wrote(&db, "web-1.0", LINK, &["bo", "g1"]);
        assert_eq!(
            vec!["\"a@x\"", "\"g1\""],
            sorted_rows(call(&db, "web-1.0", LOGINS, &[]))
        );
    });
}

/// A bundle pinned where `EmailPass` has `hash`: `emails` does not read it, `hashes` does.
const HASHES: &str = "import auth
query emails () -> Set Text = from i in identities.EmailPass select i.email
query hashes () -> Set Text = from i in identities.EmailPass select i.hash
";

const EMAILS_ONLY: &str = "import auth
query emails () -> Set Text = from i in identities.EmailPass select i.email
txn join (n: Text, e: Text, h: Text) -> Key identities =
  transact
    let u = insert users { name: n, tier: Free }
    let i = insert identities.EmailPass { user: key u, email: e, hash: h }
    return key i
";

#[test]
fn a_pinned_bundle_runs_across_a_drop_of_a_field_it_does_not_read() {
    let mut sim = Sim::new(65, Config::default());
    let pin = [V1, ADD_HASH];
    run(&mut sim, |rt| {
        opened(&rt, &pin, vec![bundle("web-1.0", &pin, "", EMAILS_ONLY)]);
    });
    run(&mut sim, |rt| {
        let db = opened(
            &rt,
            &[V1, ADD_HASH, DROP_HASH],
            vec![bundle("web-1.0", &pin, "", EMAILS_ONLY)],
        );
        wrote(&db, "web-1.0", 2, &["ann", "a@x", "h"]);
        assert_eq!(vec!["\"a@x\""], rows(call(&db, "web-1.0", 1, &[])));
    });
}

#[test]
fn a_drop_of_a_field_a_live_bundle_reads_is_refused_naming_the_bundle_query_and_operator() {
    let mut sim = Sim::new(66, Config::default());
    let pin = [V1, ADD_HASH];
    run(&mut sim, |rt| {
        opened(&rt, &pin, vec![bundle("web-1.0", &pin, "", HASHES)]);
    });
    run(&mut sim, |rt| {
        let (operator, detail) = refused_migration(
            open(
                &rt,
                &[V1, ADD_HASH, DROP_HASH],
                vec![bundle("web-1.0", &pin, "", HASHES)],
            ),
            3,
        );
        assert_eq!("drop field Identity.EmailPass.hash", operator);
        assert!(
            detail.contains("web-1.0") && detail.contains("hashes"),
            "{detail}"
        );
    });
}

#[test]
fn a_bundle_pinned_before_a_drop_of_a_field_it_reads_is_refused_registration() {
    once(67, |rt| {
        let got = open(
            &rt,
            &[V1, ADD_HASH, DROP_HASH],
            vec![bundle("web-1.0", &[V1, ADD_HASH], "", HASHES)],
        );
        match refusal(got, "web-1.0") {
            RejectReason::Untranslatable { query, detail } => {
                assert_eq!("web-1.0::hashes", query);
                assert!(detail.contains("drop field"), "{detail}");
            }
            other => panic!("refused otherwise: {other:?}"),
        }
    });
}

#[test]
fn deprecating_a_constructor_a_live_bundle_inserts_into_is_refused() {
    let mut sim = Sim::new(68, Config::default());
    run(&mut sim, |rt| {
        opened(&rt, &[V1], vec![bundle("web-1.0", &[V1], "", IDS)]);
    });
    run(&mut sim, |rt| {
        let (operator, detail) = refused_migration(
            open(
                &rt,
                &[V1, DEPRECATE_GOOGLE],
                vec![bundle("web-1.0", &[V1], "", IDS)],
            ),
            2,
        );
        assert_eq!("deprecate identities.Google", operator);
        assert!(detail.contains("link"), "{detail}");
    });
}

#[test]
fn deprecating_a_constructor_a_live_bundle_only_reads_applies() {
    let mut sim = Sim::new(69, Config::default());
    run(&mut sim, |rt| {
        let db = opened(&rt, &[V1], vec![bundle("web-1.0", &[V1], "", HASHES_FREE)]);
        logins(&db);
    });
    run(&mut sim, |rt| {
        let db = opened(
            &rt,
            &[V1, DEPRECATE_GOOGLE],
            vec![bundle("web-1.0", &[V1], "", HASHES_FREE)],
        );
        assert_eq!(
            vec!["\"a@x\"", "\"g1\""],
            sorted_rows(call(&db, "web-1.0", 1, &[]))
        );
    });
}

/// Write a user with an `EmailPass` login `a@x` and one with a `Google` login `g1`.
fn logins(db: &Database<SimRuntime>) {
    write(
        db,
        r#"transact
  let u = insert users { name: "ann", tier: Free }
  insert identities.EmailPass { user: key u, email: "a@x" }
  insert identities.Google { user: key u, gid: "g1" }
  return 0"#,
    )
    .expect("the logins are written");
}

/// A bundle that reads identities and writes none.
const HASHES_FREE: &str = "import auth
query logins () -> Set Text =
  from i in identities
  select match i
    | EmailPass e -> e.email
    | Google g -> g.gid
";

#[test]
fn a_bundle_pinned_before_an_added_field_cannot_build_its_type_outside_a_write() {
    let body = "import auth
query probe (e: Text) -> Bool =
  exists i in identities.EmailPass where i == EmailPass { id: i.id, user: i.user, email: e }
";
    once(70, |rt| {
        let got = open(
            &rt,
            &[V1, ADD_VERIFIED],
            vec![bundle("web-1.0", &[V1], "", body)],
        );
        match refusal(got, "web-1.0") {
            RejectReason::Untranslatable { query, .. } => assert_eq!("web-1.0::probe", query),
            other => panic!("refused otherwise: {other:?}"),
        }
    });
}

#[test]
fn a_bundle_taking_a_parameter_of_a_type_that_gained_a_field_is_refused_registration() {
    let body = "import auth
query same (i: Identity) -> Set Text = from j in identities where key j == i.id select \"x\"
";
    once(71, |rt| {
        let ok = open(
            &rt,
            &[V1, RENAME_NAME],
            vec![bundle("web-1.0", &[V1], "", body)],
        );
        assert!(ok.is_ok(), "a rename carries the parameter: {ok:?}");
    });
    once(72, |rt| {
        let got = open(
            &rt,
            &[V1, ADD_VERIFIED],
            vec![bundle("web-1.0", &[V1], "", body)],
        );
        match refusal(got, "web-1.0") {
            RejectReason::Untranslatable { query, detail } => {
                assert_eq!("web-1.0::same", query);
                assert!(detail.contains("verified"), "{detail}");
            }
            other => panic!("refused otherwise: {other:?}"),
        }
    });
}

const SPLIT_PAID: &str = "module auth
split constructor Tier.Paid into Monthly | Yearly
  by match
    | Paid -> Monthly
";

#[test]
fn a_split_of_a_type_a_live_bundle_reaches_is_refused_and_one_it_does_not_reach_applies() {
    let mut sim = Sim::new(73, Config::default());
    run(&mut sim, |rt| {
        opened(
            &rt,
            &[V1],
            vec![
                bundle("web-1.0", &[V1], "", NAMES),
                bundle("ids-1.0", &[V1], "", HASHES_FREE),
            ],
        );
    });
    run(&mut sim, |rt| {
        let (operator, detail) = refused_migration(
            open(
                &rt,
                &[V1, SPLIT_PAID],
                vec![
                    bundle("web-1.0", &[V1], "", NAMES),
                    bundle("ids-1.0", &[V1], "", HASHES_FREE),
                ],
            ),
            2,
        );
        assert!(
            operator.starts_with("split constructor Tier.Paid"),
            "{operator}"
        );
        assert!(detail.contains("web-1.0"), "{detail}");
    });
    run(&mut sim, |rt| {
        let db = opened(
            &rt,
            &[V1, SPLIT_PAID],
            vec![bundle("ids-1.0", &[V1], "", HASHES_FREE)],
        );
        logins(&db);
        assert_eq!(
            vec!["\"a@x\"", "\"g1\""],
            sorted_rows(call(&db, "ids-1.0", 1, &[]))
        );
    });
}

const ADD_SEATS: &str = "module auth\nadd field Tier.Paid.seats: I32 fill 1\n";

/// A bundle that counts the tiers its users hold.
const TIER_COUNT: &str = "import auth
query n () -> I64 = count(t for t in (from u in users select u.tier))
";

/// A bundle that counts its users.
const USER_COUNT: &str = "import auth
query n () -> I64 = count(u for u in users)
";

#[test]
fn a_count_over_values_of_a_type_that_gained_a_field_is_refused_and_one_over_rows_runs() {
    let mut sim = Sim::new(74, Config::default());
    let regs = || {
        vec![
            bundle("tiers-1.0", &[V1], "", TIER_COUNT),
            bundle("users-1.0", &[V1], "", USER_COUNT),
        ]
    };
    run(&mut sim, |rt| {
        opened(&rt, &[V1], regs());
    });
    run(&mut sim, |rt| {
        let (operator, detail) = refused_migration(open(&rt, &[V1, ADD_SEATS], regs()), 2);
        assert_eq!("add field Tier.Paid.seats: I32 fill 1", operator);
        assert_eq!(
            "bundle tiers-1.0 pins 1: `n` has no translation: it counts the values of `Tier` \
             whole, and its field `seats` differs",
            detail
        );
    });
    run(&mut sim, |rt| {
        let db = opened(
            &rt,
            &[V1, ADD_SEATS],
            vec![bundle("users-1.0", &[V1], "", USER_COUNT)],
        );
        write(
            &db,
            r#"transact
  insert users { name: "ann", tier: Paid { seats: 1 } }
  insert users { name: "bo", tier: Free }
  insert users { name: "cy", tier: Paid { seats: 5 } }
  return 0"#,
        )
        .expect("the users are written");
        assert_eq!(vec!["3"], rows(call(&db, "users-1.0", 1, &[])));
    });
}

/// A bundle that returns its users whole.
const PEOPLE: &str = "import auth
query people () -> Set User = from u in users select u
";

#[test]
fn a_split_of_a_type_a_live_bundle_holds_inside_a_whole_row_is_refused() {
    let mut sim = Sim::new(75, Config::default());
    run(&mut sim, |rt| {
        opened(&rt, &[V1], vec![bundle("web-1.0", &[V1], "", PEOPLE)]);
    });
    run(&mut sim, |rt| {
        let (operator, detail) = refused_migration(
            open(
                &rt,
                &[V1, SPLIT_PAID],
                vec![bundle("web-1.0", &[V1], "", PEOPLE)],
            ),
            2,
        );
        assert_eq!(
            "split constructor Tier.Paid into Monthly | Yearly",
            operator
        );
        assert_eq!(
            "bundle web-1.0 pins 1: `people` has no translation: it reaches what the operator \
             changes, and this engine does not translate across it",
            detail
        );
    });
}

const DEPRECATE_FREE: &str = "module auth\ndeprecate Tier.Free\n";
const REMOVE_FREE: &str = "module auth\nremove constructor Tier.Free\n";

/// A bundle that takes a tier and compares tiers.
const TIERED: &str = "import auth
query tiered (t: Tier) -> Set Text = from u in users where u.tier == t select u.name
";

#[test]
fn removing_a_constructor_a_live_bundle_takes_or_compares_is_refused() {
    let mut sim = Sim::new(76, Config::default());
    let pin = [V1, ADD_GOLD, ENABLE_GOLD];
    run(&mut sim, |rt| {
        let db = opened(&rt, &pin, vec![bundle("web-1.0", &pin, "", TIERED)]);
        write(&db, r#"insert users { name: "ann", tier: Paid }"#).expect("the write applies");
    });
    run(&mut sim, |rt| {
        let (operator, detail) = refused_migration(
            open(
                &rt,
                &[V1, ADD_GOLD, ENABLE_GOLD, DEPRECATE_FREE, REMOVE_FREE],
                vec![bundle("web-1.0", &pin, "", TIERED)],
            ),
            5,
        );
        assert_eq!("remove constructor Tier.Free", operator);
        assert_eq!(
            "bundle web-1.0 pins 3: `tiered` has no translation: it takes a parameter of \
             `Tier.Free`, which the current version lacks",
            detail
        );
    });
}

#[test]
fn a_bundle_pinned_before_a_removed_constructor_it_takes_is_refused_registration() {
    let pin = [V1, ADD_GOLD, ENABLE_GOLD];
    once(77, |rt| {
        let got = open(
            &rt,
            &[V1, ADD_GOLD, ENABLE_GOLD, DEPRECATE_FREE, REMOVE_FREE],
            vec![bundle("web-1.0", &pin, "", TIERED)],
        );
        match refusal(got, "web-1.0") {
            RejectReason::Untranslatable { query, detail } => {
                assert_eq!("web-1.0::tiered", query);
                assert!(detail.contains("remove constructor Tier.Free"), "{detail}");
            }
            other => panic!("refused otherwise: {other:?}"),
        }
    });
}

/// A bundle that writes the login `identities.key(1)` of `users.key(1)` whole, by upsert.
const REPUT: &str = "import auth
txn reput (e: Text) -> Text =
  transact
    insert identities.EmailPass { id: identities.key(1), user: users.key(1), email: e }
      unless conflict on key
      else update old set new
    return e
";

#[test]
fn a_pinned_upsert_that_updates_keeps_the_rows_added_field() {
    once(78, |rt| {
        let log = [V1, ADD_VERIFIED, DEFAULT_VERIFIED];
        let db = opened(
            &rt,
            &log,
            vec![
                bundle("web-1.0", &[V1], "", IDS),
                bundle("put-1.0", &[V1], "", REPUT),
            ],
        );
        wrote(&db, "web-1.0", JOIN, &["ann", "a@x"]);
        write(
            &db,
            "update i in identities.EmailPass set EmailPass { i with verified: false }",
        )
        .expect("the write applies");
        wrote(&db, "put-1.0", 1, &["a@y"]);
        assert_eq!(
            vec!["a@y:f"],
            verified(&db),
            "an update keeps the row's value"
        );
    });
}

#[test]
fn a_pinned_upsert_that_inserts_takes_the_default() {
    once(79, |rt| {
        let log = [V1, ADD_VERIFIED, DEFAULT_VERIFIED];
        let db = opened(&rt, &log, vec![bundle("put-1.0", &[V1], "", REPUT)]);
        write(&db, r#"insert users { name: "ann", tier: Free }"#).expect("the write applies");
        wrote(&db, "put-1.0", 1, &["a@x"]);
        assert_eq!(vec!["a@x:t"], verified(&db));
    });
}

#[test]
fn an_expired_bundle_with_no_translation_registered_again_is_refused_expired() {
    let mut sim = Sim::new(80, Config::default());
    let pin = [V1, ADD_HASH];
    run(&mut sim, |rt| {
        opened(
            &rt,
            &pin,
            vec![bundle("web-1.0", &pin, " lease 1d", HASHES)],
        );
        sleep(&rt, 2 * DAY);
    });
    run(&mut sim, |rt| {
        let got = open(
            &rt,
            &[V1, ADD_HASH, DROP_HASH],
            vec![bundle("web-1.0", &pin, " lease 1d", HASHES)],
        );
        assert_eq!(
            RejectReason::BundleExpired {
                bundle: "web-1.0".into(),
                query: "web-1.0::hashes".into(),
            },
            refusal(got, "web-1.0")
        );
    });
}

const DISABLE_GOOGLE: &str = "module auth\ndisable identities.Google\n";

#[test]
fn disabling_a_constructor_a_live_bundle_inserts_into_is_refused() {
    let mut sim = Sim::new(81, Config::default());
    run(&mut sim, |rt| {
        opened(&rt, &[V1], vec![bundle("web-1.0", &[V1], "", IDS)]);
    });
    run(&mut sim, |rt| {
        let (operator, detail) = refused_migration(
            open(
                &rt,
                &[V1, DISABLE_GOOGLE],
                vec![bundle("web-1.0", &[V1], "", IDS)],
            ),
            2,
        );
        assert_eq!("disable identities.Google", operator);
        assert!(detail.contains("link"), "{detail}");
    });
}

#[test]
fn disabling_a_constructor_a_live_bundle_only_reads_applies() {
    let mut sim = Sim::new(82, Config::default());
    run(&mut sim, |rt| {
        let db = opened(&rt, &[V1], vec![bundle("web-1.0", &[V1], "", HASHES_FREE)]);
        write(
            &db,
            r#"transact
  let u = insert users { name: "ann", tier: Free }
  insert identities.EmailPass { user: key u, email: "a@x" }
  return 0"#,
        )
        .expect("the login is written");
    });
    run(&mut sim, |rt| {
        let db = opened(
            &rt,
            &[V1, DISABLE_GOOGLE],
            vec![bundle("web-1.0", &[V1], "", HASHES_FREE)],
        );
        assert_eq!(vec!["\"a@x\""], rows(call(&db, "web-1.0", 1, &[])));
    });
}

const REMOVE_GOOGLE: &str = "module auth\nremove constructor Identity.Google\n";

#[test]
fn a_pinned_bundle_reads_a_removed_constructor_as_empty_and_cannot_write_it() {
    let log = [V1, DEPRECATE_GOOGLE, REMOVE_GOOGLE];
    once(83, |rt| {
        let db = opened(&rt, &log, vec![bundle("web-1.0", &[V1], "", HASHES_FREE)]);
        write(
            &db,
            r#"transact
  let u = insert users { name: "ann", tier: Free }
  insert identities.EmailPass { user: key u, email: "a@x" }
  return 0"#,
        )
        .expect("the login is written");
        assert_eq!(vec!["\"a@x\""], rows(call(&db, "web-1.0", 1, &[])));
    });
    once(84, |rt| {
        match refusal(
            open(&rt, &log, vec![bundle("web-1.0", &[V1], "", IDS)]),
            "web-1.0",
        ) {
            RejectReason::Untranslatable { query, detail } => {
                assert_eq!("web-1.0::link", query);
                assert!(detail.contains("Identity.Google"), "{detail}");
            }
            other => panic!("refused otherwise: {other:?}"),
        }
    });
}

/// Queries reading the constructor relation of `Google` every way a query can, each empty once
/// the constructor is removed, in ordinal order.
const READS_GOOGLE: &str = r#"import auth
query body () -> Set Text = from g in identities.Google select g.gid
query header () -> Set (Key users) = from g in identities.Google select g.user
query keys () -> Set (Key identities) = from g in identities.Google select key g
query matched () -> Set Text = from g in identities.Google select match g
  | Google x -> x.gid
query joined () -> Set Text =
  from u in users, g in identities.Google where g.user == key u select u.name
query selfjoin () -> Set Text =
  from a in identities.Google, b in identities.Google where key a == key b select a.gid
query filtered () -> Set Text = from g in identities.Google where g.gid == "x" select g.gid
query nested () -> Set Text =
  from u in users
  where exists g in identities.Google where g.user == key u
  select u.name
query crossed () -> Set Text = from g in identities.Google, h in identities.Google select h.gid
"#;

#[test]
fn every_read_of_a_removed_constructor_pinned_before_it_is_empty() {
    let log = [V1, DEPRECATE_GOOGLE, REMOVE_GOOGLE];
    once(85, |rt| {
        let db = opened(&rt, &log, vec![bundle("web-1.0", &[V1], "", READS_GOOGLE)]);
        write(
            &db,
            r#"transact
  let u = insert users { name: "ann", tier: Free }
  insert identities.EmailPass { user: key u, email: "x" }
  return 0"#,
        )
        .expect("the login is written");
        for ordinal in 1..=9 {
            assert_eq!(
                Vec::<String>::new(),
                rows(call(&db, "web-1.0", ordinal, &[])),
                "query {ordinal}"
            );
        }
    });
}

#[test]
fn an_aggregate_and_a_test_of_a_removed_constructor_pinned_before_it_read_it_as_empty() {
    let log = [V1, DEPRECATE_GOOGLE, REMOVE_GOOGLE];
    let body = r#"import auth
query counted () -> I64 = count(g for g in identities.Google)
query present () -> Bool = exists g in identities.Google where g.gid == "x"
"#;
    once(86, |rt| {
        let db = opened(&rt, &log, vec![bundle("web-1.0", &[V1], "", body)]);
        write(
            &db,
            r#"transact
  let u = insert users { name: "ann", tier: Free }
  insert identities.EmailPass { user: key u, email: "x" }
  return 0"#,
        )
        .expect("the login is written");
        assert_eq!(vec!["0"], rows(call(&db, "web-1.0", 1, &[])));
        assert_eq!(vec!["False"], rows(call(&db, "web-1.0", 2, &[])));
    });
}

const ADD_NOTES: &str = "module auth
add type Note { id: Key notes } = Plain { body: Text }
add relation notes : Note key id generate sequence
";
const ADD_LINK: &str = "module auth\nadd constructor Note.Link { url: Text }\n";

/// What a pin sees of a sum a later migration grows, in ordinal order: its one constructor read
/// through the relation and by name, and a rewrite of each of its rows.
const READS_GROWN: &str = "import auth
query whole () -> Set Text = from n in notes select n.body
query named () -> Set Text = from n in notes.Plain select n.body
txn rewrite () -> I64 =
  transact
    insert notes.Plain from (from x in notes.Plain select { id: x.id, body: x.body })
      unless conflict on key else update old set new
    return 0
";

#[test]
fn a_pin_reads_the_constructors_it_saw_of_a_sum_grown_after_it() {
    let pin = [V1, ADD_NOTES];
    let log = [V1, ADD_NOTES, ADD_LINK];
    once(87, |rt| {
        let db = opened(&rt, &log, vec![bundle("web-1.0", &pin, "", READS_GROWN)]);
        write(&db, r#"insert notes.Plain { body: "a" }"#).expect("the note is written");
        assert_eq!(vec!["\"a\""], rows(call(&db, "web-1.0", 1, &[])));
        assert_eq!(vec!["\"a\""], rows(call(&db, "web-1.0", 2, &[])));
        call(&db, "web-1.0", 3, &[]);
        assert_eq!(vec!["\"a\""], rows(call(&db, "web-1.0", 1, &[])));
    });
}

const ENABLE_LINK: &str = "module auth\nenable notes.Link\n";
const ADD_LINK_BODY: &str = "module auth\nadd constructor Note.Link { url: Text, body: Text }\n";

/// Upserts whose `old` is read by field, in ordinal order: at key 1, then at key 2.
const UPSERTS: &str = r#"import auth
txn first () -> I64 =
  transact
    insert notes.Plain { id: notes.key(1), body: "x" }
      unless conflict on key else update old set Plain { old with body: old.body ++ "!" }
    return 0
txn second () -> I64 =
  transact
    insert notes.Plain { id: notes.key(2), body: "x" }
      unless conflict on key else update old set Plain { old with body: old.body ++ "!" }
    return 0
"#;

const NOTES: &str = "from n in notes select match n
  | Plain p -> p.body
  | Link l -> l.url";

#[test]
fn an_upsert_old_read_past_a_constructor_lacking_the_field_is_stale_unless_hidden() {
    let pin = [V1, ADD_NOTES];
    let log = [V1, ADD_NOTES, ADD_LINK, ENABLE_LINK];
    once(88, |rt| {
        assert_eq!(
            link_stale(),
            refusal(
                open(&rt, &log, vec![bundle("web-1.0", &pin, "", UPSERTS)]),
                "web-1.0"
            ),
        );
    });
    once(88, |rt| {
        let db = opened(
            &rt,
            &log,
            vec![bundle("web-1.0", &pin, " hide new constructors", UPSERTS)],
        );
        write(&db, r#"insert notes.Link { id: notes.key(1), url: "u" }"#)
            .expect("the link is written");
        write(&db, r#"insert notes.Plain { id: notes.key(2), body: "a" }"#)
            .expect("the note is written");
        let refused = db.registered("web-1.0").expect("the bundle binds").call(
            1,
            &[],
            &CallOptions::default(),
        );
        assert_eq!(
            Err(RejectReason::KeyViolation {
                relation: "auth::notes".into(),
                key: "notes.key(1)".into(),
            }),
            refused.map(drop).map_err(|e| reason(&e).clone()),
        );
        call(&db, "web-1.0", 2, &[]);
        let mut notes = query(&db, NOTES);
        notes.sort();
        assert_eq!(vec![r#""a!""#, r#""u""#], notes);
    });
}

#[test]
fn an_upsert_old_read_of_a_new_constructors_own_field_is_stale_unless_hidden() {
    let pin = [V1, ADD_NOTES];
    let log = [V1, ADD_NOTES, ADD_LINK_BODY, ENABLE_LINK];
    once(89, |rt| {
        assert_eq!(
            link_stale(),
            refusal(
                open(&rt, &log, vec![bundle("web-1.0", &pin, "", UPSERTS)]),
                "web-1.0"
            ),
        );
    });
    once(89, |rt| {
        let db = opened(
            &rt,
            &log,
            vec![bundle("web-1.0", &pin, " hide new constructors", UPSERTS)],
        );
        write(
            &db,
            r#"insert notes.Link { id: notes.key(1), url: "u", body: "b" }"#,
        )
        .expect("the link is written");
        let got = db
            .registered("web-1.0")
            .expect("the bundle binds")
            .call(1, &[], &CallOptions::default())
            .map(drop)
            .map_err(|e| reason(&e).clone());
        assert_eq!(
            (
                Err(RejectReason::KeyViolation {
                    relation: "auth::notes".into(),
                    key: "notes.key(1)".into(),
                }),
                vec![r#""u""#.to_owned()],
            ),
            (got, query(&db, NOTES)),
        );
    });
}

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
fn an_upsert_old_matched_past_a_constructor_is_stale() {
    let body = r#"import auth
txn put () -> I64 =
  transact
    insert notes.Plain { id: notes.key(1), body: "x" }
      unless conflict on key else update old set Plain { old with body: match old
        | Plain p -> p.body }
    return 0
"#;
    let pin = [V1, ADD_NOTES];
    let log = [V1, ADD_NOTES, ADD_LINK, ENABLE_LINK];
    once(90, |rt| {
        assert_eq!(
            link_stale(),
            refusal(
                open(&rt, &log, vec![bundle("web-1.0", &pin, "", body)]),
                "web-1.0"
            ),
        );
    });
}

#[test]
fn an_upsert_old_field_read_is_stale_once_a_value_constructor_is_enabled_even_with_a_same_named_field()
 {
    let add = "module auth
add type Tone = Ink { depth: I64 }
add type Mark = { id: Key marks, tone: Tone }
add relation marks : Mark key id generate sequence
";
    let grow = "module auth\nadd constructor Tone.Dye { hue: Text, depth: I64 }\n";
    let enable = "module auth\nenable Tone.Dye\n";
    let body = "import auth
txn put () -> I64 =
  transact
    insert marks { id: marks.key(1), tone: Ink { depth: 0 } }
      unless conflict on key else update old set { old with tone: Ink { depth: old.tone.depth } }
    return 0
";
    let pin = [V1, add];
    let log = [V1, add, grow, enable];
    once(91, |rt| {
        assert_eq!(
            RejectReason::BundleStale {
                bundle: "web-1.0".into(),
                ctor: "Tone.Dye".into(),
            },
            refusal(
                open(&rt, &log, vec![bundle("web-1.0", &pin, "", body)]),
                "web-1.0"
            ),
        );
    });
}

#[test]
fn a_hiding_pinned_upsert_conflicting_with_a_hidden_row_fails_by_key() {
    let body = r#"import auth
txn put () -> I64 =
  transact
    insert notes.Plain { id: notes.key(1), body: "x" }
      unless conflict on key else update old set new
    return 0
"#;
    let pin = [V1, ADD_NOTES];
    let log = [V1, ADD_NOTES, ADD_LINK, ENABLE_LINK];
    once(192, |rt| {
        let db = opened(
            &rt,
            &log,
            vec![bundle("web-1.0", &pin, " hide new constructors", body)],
        );
        write(&db, r#"insert notes.Link { id: notes.key(1), url: "u" }"#)
            .expect("the hidden link is written");
        let got = db
            .registered("web-1.0")
            .expect("the bundle binds")
            .call(1, &[], &CallOptions::default())
            .map(drop)
            .map_err(|e| reason(&e).clone());
        assert_eq!(
            (
                Err(RejectReason::KeyViolation {
                    relation: "auth::notes".into(),
                    key: "notes.key(1)".into(),
                }),
                vec![r#"Link { id: notes.key(1), url: "u" }"#.to_owned()],
            ),
            (got, query(&db, "from n in notes select n"))
        );
    });
}

#[test]
fn a_hiding_pin_refuses_hidden_conflicts_before_each_branch_and_binding() {
    let branches = [
        ("skip", "I64", "0"),
        ("delete old", "I64", "0"),
        (
            "update old set Plain { old with body: old.body ++ \"!\" }",
            "I64",
            "0",
        ),
        ("fail \"the conflict branch ran\"", "I64", "0"),
        (
            "match old | Plain p -> update p set Plain { p with body: \"matched\" }",
            "I64",
            "0",
        ),
        ("skip", "Note", "written"),
    ];
    let mut body = "import auth\n".to_owned();
    for (i, (branch, ty, result)) in branches.iter().enumerate() {
        let binding = if *result == "written" {
            "let written = "
        } else {
            ""
        };
        body.push_str(&format!(
            "txn put{i} () -> {ty} =\n  transact\n    {binding}insert notes.Plain {{ id: notes.key(1), body: \"x\" }}\n      unless conflict on key else {branch}\n    return {result}\n"
        ));
    }
    let pin = [V1, ADD_NOTES];
    let log = [V1, ADD_NOTES, ADD_LINK, ENABLE_LINK];
    once(195, |rt| {
        let db = opened(
            &rt,
            &log,
            vec![bundle("web-1.0", &pin, " hide new constructors", &body)],
        );
        write(&db, r#"insert notes.Link { id: notes.key(1), url: "u" }"#)
            .expect("the hidden row is written");
        let mut ran = 0;
        for ordinal in 1..=branches.len() as u32 {
            let got = db
                .registered("web-1.0")
                .expect("the bundle binds")
                .call(ordinal, &[], &CallOptions::default())
                .map(drop)
                .map_err(|e| reason(&e).clone());
            assert_eq!(
                Err(RejectReason::KeyViolation {
                    relation: "auth::notes".into(),
                    key: "notes.key(1)".into(),
                }),
                got,
                "branch {ordinal}"
            );
            assert_eq!(
                vec![r#"Link { id: notes.key(1), url: "u" }"#],
                query(&db, "from n in notes select n"),
                "branch {ordinal}"
            );
            ran += 1;
        }
        assert_eq!(6, ran);
    });
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
fn a_hidden_conflict_leaves_every_row_of_a_bulk_upsert_unapplied() {
    let body = r#"import auth
txn put () -> I64 =
  transact
    insert notes.Plain from [
      { id: notes.key(1), body: "x" },
      { id: notes.key(3), body: "new" }
    ] unless conflict on key else delete old
    return 0
"#;
    let pin = [V1, ADD_NOTES];
    let log = [V1, ADD_NOTES, ADD_LINK, ENABLE_LINK];
    once(197, |rt| {
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
                Err(RejectReason::KeyViolation {
                    relation: "auth::notes".into(),
                    key: "notes.key(1)".into(),
                }),
                vec![r#"Link { id: notes.key(1), url: "u" }"#.to_owned()],
            ),
            (got, query(&db, "from n in notes select n"))
        );
    });
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
fn a_hiding_pin_maps_conflict_visibility_across_rename_and_constructor_removal() {
    let add = "module auth\nadd type Note { id: Key notes } = Removed { body: Text } | Plain { body: Text }\nadd relation notes : Note key id generate sequence\n";
    let deprecate = "module auth\ndeprecate notes.Removed\n";
    let remove = "module auth\nremove constructor Note.Removed\n";
    let rename = "module auth\nrename constructor Note.Plain to Prose\n";
    let body = r#"import auth
txn first () -> I64 =
  transact
    insert notes.Plain { id: notes.key(1), body: "x" }
      unless conflict on key else update old set new
    return 0
txn second () -> I64 =
  transact
    insert notes.Plain { id: notes.key(2), body: "x" }
      unless conflict on key else update old set new
    return 0
"#;
    let pin = [V1, add];
    let log = [V1, add, deprecate, remove, rename, ADD_LINK, ENABLE_LINK];
    once(199, |rt| {
        let db = opened(
            &rt,
            &log,
            vec![bundle("web-1.0", &pin, " hide new constructors", body)],
        );
        write(&db, r#"insert notes.Prose { id: notes.key(1), body: "a" }"#)
            .expect("the known row is written");
        write(&db, r#"insert notes.Link { id: notes.key(2), url: "u" }"#)
            .expect("the hidden row is written");
        call(&db, "web-1.0", 1, &[]);
        let got = db
            .registered("web-1.0")
            .expect("the bundle binds")
            .call(2, &[], &CallOptions::default())
            .map(drop)
            .map_err(|e| reason(&e).clone());
        let mut rows = query(&db, "from n in notes select n");
        rows.sort();
        assert_eq!(
            (
                Err(RejectReason::KeyViolation {
                    relation: "auth::notes".into(),
                    key: "notes.key(2)".into(),
                }),
                vec![
                    r#"Link { id: notes.key(2), url: "u" }"#.to_owned(),
                    r#"Prose { id: notes.key(1), body: "x" }"#.to_owned(),
                ],
            ),
            (got, rows)
        );
    });
}

const PINNED_TONE_ADD: &str = "module auth
add type Tone = Ink { depth: I64 }
add type Mark = { id: Key marks, tone: Tone }
add relation marks : Mark key id generate sequence
";
const PINNED_TONE_GROW: &str = "module auth\nadd constructor Tone.Dye { hue: Text, depth: I64 }\n";
const PINNED_TONE_ENABLE: &str = "module auth\nenable Tone.Dye\n";
const PINNED_DEPTHS: &str =
    "import auth\nquery depths () -> Set I64 = from x in marks select x.tone.depth\n";

#[test]
fn a_hiding_pinned_nested_field_scan_refuses_an_enabled_value_constructor_at_registration() {
    let pin = [V1, PINNED_TONE_ADD];
    let log = [V1, PINNED_TONE_ADD, PINNED_TONE_GROW, PINNED_TONE_ENABLE];
    once(193, |rt| {
        assert_eq!(
            RejectReason::BundleStale {
                bundle: "web-1.0".into(),
                ctor: "Tone.Dye".into(),
            },
            refusal(
                open(
                    &rt,
                    &log,
                    vec![bundle(
                        "web-1.0",
                        &pin,
                        " hide new constructors",
                        PINNED_DEPTHS
                    )],
                ),
                "web-1.0",
            ),
        );
    });
}

#[test]
fn a_hiding_pinned_nested_field_scan_blocks_enabling_a_value_constructor_while_live() {
    let pin = [V1, PINNED_TONE_ADD];
    let before = [V1, PINNED_TONE_ADD, PINNED_TONE_GROW];
    let log = [V1, PINNED_TONE_ADD, PINNED_TONE_GROW, PINNED_TONE_ENABLE];
    let mut sim = Sim::new(194, Config::default());
    run(&mut sim, |rt| {
        opened(
            &rt,
            &before,
            vec![bundle(
                "web-1.0",
                &pin,
                " hide new constructors",
                PINNED_DEPTHS,
            )],
        );
    });
    run(&mut sim, |rt| {
        let (_operator, detail) = refused_migration(
            open(
                &rt,
                &log,
                vec![bundle(
                    "web-1.0",
                    &pin,
                    " hide new constructors",
                    PINNED_DEPTHS,
                )],
            ),
            4,
        );
        assert_eq!(
            "`enable Tone.Dye` is refused: bundle web-1.0, pinned at version 2, reads a field whose type reaches Tone; a bundle stops blocking only when it is released",
            detail,
        );
    });
}

#[test]
fn a_pinned_bundle_runs_across_operators_that_change_no_result() {
    let unchanged = [
        "module auth\nadd unique (email) in identities.EmailPass\n",
        "module auth\nadd index (gid) in identities.Google\n",
        "module auth\nadd constraint no_blank = from u in users where u.name == \"\" select u\n",
    ];
    let mut log = vec![V1];
    log.extend(unchanged);
    once(85, |rt| {
        let db = opened(&rt, &log, vec![bundle("web-1.0", &[V1], "", IDS)]);
        wrote(&db, "web-1.0", JOIN, &["ann", "a@x"]);
        wrote(&db, "web-1.0", LINK, &["bo", "g1"]);
        assert_eq!(vec!["\"a@x\""], rows(call(&db, "web-1.0", EMAILS, &[])));
        assert_eq!(
            vec!["\"a@x\"", "\"g1\""],
            sorted_rows(call(&db, "web-1.0", LOGINS, &[]))
        );
    });
}

#[test]
fn a_migration_refused_for_a_live_bundle_leaves_the_prior_version_after_reopen() {
    let mut sim = Sim::new(86, Config::default());
    let pin = [V1, ADD_HASH];
    run(&mut sim, |rt| {
        let db = opened(&rt, &pin, vec![bundle("web-1.0", &pin, "", HASHES)]);
        write(
            &db,
            r#"transact
  let u = insert users { name: "ann", tier: Free }
  insert identities.EmailPass { user: key u, email: "a@x", hash: "h" }
  return 0"#,
        )
        .expect("the login is written");
    });
    run(&mut sim, |rt| {
        refused_migration(
            open(
                &rt,
                &[V1, ADD_HASH, DROP_HASH],
                vec![bundle("web-1.0", &pin, "", HASHES)],
            ),
            3,
        );
    });
    run(&mut sim, |rt| {
        let db = opened(&rt, &pin, vec![bundle("web-1.0", &pin, "", HASHES)]);
        assert_eq!(vec!["\"h\""], rows(call(&db, "web-1.0", 2, &[])));
        assert_eq!(
            vec!["migrations.key(1)", "migrations.key(2)"],
            query(&db, "from m in catalog::migrations select m.version")
        );
    });
}

/// A bundle that pairs each Google login with each EmailPass login, returning both whole.
const PAIRS: &str = "import auth
query pairs () -> Set { a: Identity.Google, b: Identity.EmailPass } =
  from g in identities.Google, e in identities.EmailPass select { a: g, b: e }
";

#[test]
fn a_drop_of_a_field_of_a_constructor_a_live_bundle_returns_after_another_is_refused() {
    let mut sim = Sim::new(87, Config::default());
    let pin = [V1, ADD_HASH];
    run(&mut sim, |rt| {
        opened(&rt, &pin, vec![bundle("web-1.0", &pin, "", PAIRS)]);
    });
    run(&mut sim, |rt| {
        let (operator, detail) = refused_migration(
            open(
                &rt,
                &[V1, ADD_HASH, DROP_HASH],
                vec![bundle("web-1.0", &pin, "", PAIRS)],
            ),
            3,
        );
        assert_eq!("drop field Identity.EmailPass.hash", operator);
        assert_eq!(
            "bundle web-1.0 pins 2: `pairs` has no translation: it returns `Identity` whole, and \
             its field `hash` differs",
            detail
        );
    });
}

fn hashes_expired() -> RejectReason {
    RejectReason::BundleExpired {
        bundle: "web-1.0".into(),
        query: "web-1.0::hashes".into(),
    }
}

#[test]
fn a_superseded_bundle_with_no_translation_registered_again_is_refused_expired() {
    let pin = [V1, ADD_HASH];
    let mut sim = Sim::new(88, Config::default());
    run(&mut sim, |rt| {
        opened(&rt, &pin, vec![bundle("web-1.0", &pin, "", HASHES)]);
    });
    run(&mut sim, |rt| {
        opened(&rt, &pin, vec![bundle("web-2.0", &pin, "", NAMES)]);
    });
    run(&mut sim, |rt| {
        let got = open(
            &rt,
            &[V1, ADD_HASH, DROP_HASH],
            vec![bundle("web-1.0", &pin, "", HASHES)],
        );
        assert_eq!(hashes_expired(), refusal(got, "web-1.0"));
    });
}

#[test]
fn a_bundle_superseded_by_command_with_no_translation_registered_again_is_refused_expired() {
    let pin = [V1, ADD_HASH];
    let mut sim = Sim::new(89, Config::default());
    run(&mut sim, |rt| {
        let db = opened(
            &rt,
            &pin,
            vec![
                bundle("web-1.0", &pin, "", HASHES),
                bundle("web-1.1", &pin, "", NAMES),
            ],
        );
        query(&db, "supersede bundle web-1.0 by web-1.1");
    });
    run(&mut sim, |rt| {
        let got = open(
            &rt,
            &[V1, ADD_HASH, DROP_HASH],
            vec![
                bundle("web-1.0", &pin, "", HASHES),
                bundle("web-1.1", &pin, "", NAMES),
            ],
        );
        assert_eq!(hashes_expired(), refusal(got, "web-1.0"));
    });
}

#[test]
fn a_retired_bundle_with_no_translation_registered_again_is_refused_retired() {
    let pin = [V1, ADD_HASH];
    let mut sim = Sim::new(95, Config::default());
    run(&mut sim, |rt| {
        let db = opened(&rt, &pin, vec![bundle("web-1.0", &pin, "", HASHES)]);
        query(&db, "retire bundle web-1.0");
    });
    run(&mut sim, |rt| {
        let got = open(
            &rt,
            &[V1, ADD_HASH, DROP_HASH],
            vec![bundle("web-1.0", &pin, "", HASHES)],
        );
        assert_eq!(
            RejectReason::Retired {
                bundle: "web-1.0".into()
            },
            refusal(got, "web-1.0")
        );
    });
}

const SHOP: &str = r"module shop
add module shop
add type Price { sku: Text } = Priced { cents: I64 } | Gratis
add relation prices : Price key sku
add relation archive : Price key sku
";
const ADD_CURRENCY: &str = "module shop\nadd field Price.Priced.currency: Text fill \"USD\"\n";
const ARCHIVE_DEFAULT: &str = "module shop\nadd default archive.Priced.currency = \"ARC\"\n";

/// A bundle that copies prices into the archive whole: `archive_all` 1 every one, `keep` 2 the
/// one keyed `s` when it is priced.
const ARCHIVING: &str = "import shop
txn archive_all () -> I64 =
  transact
    insert archive from (from p in prices where p.sku != \"\" select p)
    return 0
txn keep (s: Text) -> Text =
  transact
    match prices.key(s)
      | Priced p -> insert archive.Priced p
      | Gratis g -> skip
    return s
";

#[test]
fn a_pinned_insert_of_a_copied_row_takes_the_target_relations_default() {
    once(90, |rt| {
        let log = [SHOP, ADD_CURRENCY, ARCHIVE_DEFAULT];
        let db = opened(&rt, &log, vec![bundle("shop-1.0", &[SHOP], "", ARCHIVING)]);
        write(
            &db,
            r#"insert prices.Priced { sku: "a", cents: 1, currency: "EUR" }"#,
        )
        .expect("the price is written");
        wrote(&db, "shop-1.0", 1, &[]);
        assert_eq!(vec!["\"a:ARC\""], archived(&db), "a row a `for` binds");
        write(
            &db,
            r#"insert prices.Priced { sku: "b", cents: 2, currency: "EUR" }"#,
        )
        .expect("the price is written");
        wrote(&db, "shop-1.0", 2, &["b"]);
        assert_eq!(
            vec!["\"a:ARC\"", "\"b:ARC\""],
            archived(&db),
            "a row a `match` binds"
        );
    });
}

/// Returns each row of `archive` as `sku:currency`, or its `sku` when it is no `Priced`.
fn archived(db: &Database<SimRuntime>) -> Vec<String> {
    query(
        db,
        r#"from a in archive select match a
  | Priced p -> p.sku ++ ":" ++ p.currency
  | Gratis g -> g.sku"#,
    )
}

const ITEMS: &str = r"module shop
add module shop
add type Item = { sku: Text, cents: I64 }
add relation items : Item key sku
add relation olds : Item key sku
";

#[test]
fn a_let_bound_value_inserted_into_two_relations_takes_each_relations_default() {
    let both = "import shop
txn both (s: Text, n: I64) -> Text =
  transact
    let i = Item { sku: s, cents: 10 / n }
    insert items i
    insert olds i
    return s
";
    once(96, |rt| {
        let log = [
            ITEMS,
            "module shop\nadd field Item.currency: Text fill \"USD\"\n",
            "module shop\nadd default items.currency = \"ITM\"\n",
            "module shop\nadd default olds.currency = \"OLD\"\n",
        ];
        let db = opened(&rt, &log, vec![bundle("shop-1.0", &[ITEMS], "", both)]);
        match call(&db, "shop-1.0", 1, &[Value::text("a"), Value::Int(2)]) {
            Called::Committed(_) => {}
            other => panic!("both commits nothing: {other:?}"),
        }
        assert_eq!(
            vec!["\"a:ITM\""],
            query(&db, r#"from i in items select i.sku ++ ":" ++ i.currency"#)
        );
        assert_eq!(
            vec!["\"a:OLD\""],
            query(&db, r#"from i in olds select i.sku ++ ":" ++ i.currency"#)
        );
    });
}

/// A bundle that reads the past.
const PAST: &str = "import auth
query past (t: Instant) -> Set Text = at instant t { from u in users select u.name }
";

#[test]
fn a_bundle_reading_the_past_registered_after_a_migration_is_refused_naming_the_at_block() {
    once(97, |rt| {
        let got = open(
            &rt,
            &[V1, ADD_VERIFIED],
            vec![bundle("web-1.0", &[V1], "", PAST)],
        );
        match refusal(got, "web-1.0") {
            RejectReason::Untranslatable { query, detail } => {
                assert_eq!("web-1.0::past", query);
                assert_eq!("it reads the past in an `at` block", detail);
            }
            other => panic!("refused otherwise: {other:?}"),
        }
    });
}

#[test]
fn a_migration_across_which_a_live_bundle_reads_the_past_is_refused_naming_the_at_block() {
    let mut sim = Sim::new(98, Config::default());
    run(&mut sim, |rt| {
        opened(&rt, &[V1], vec![bundle("web-1.0", &[V1], "", PAST)]);
    });
    run(&mut sim, |rt| {
        let (operator, detail) = refused_migration(
            open(
                &rt,
                &[V1, ADD_VERIFIED],
                vec![bundle("web-1.0", &[V1], "", PAST)],
            ),
            2,
        );
        assert_eq!("", operator);
        assert!(
            detail.contains("`past`") && detail.contains("`at` block"),
            "{detail}"
        );
    });
}

#[test]
fn a_migration_a_live_bundle_cannot_resolve_is_refused_naming_its_query() {
    let mut sim = Sim::new(99, Config::default());
    run(&mut sim, |rt| {
        opened(&rt, &[V1], vec![bundle("web-1.0", &[V1], "", NAMES)]);
    });
    run(&mut sim, |rt| {
        let adopted = "module auth\nadopt tzdb \"2026b\" from \"tz-2026b.tzif\"\n";
        let (operator, detail) = refused_migration(
            open(
                &rt,
                &[V1, adopted],
                vec![bundle("web-1.0", &[V1], "", NAMES)],
            ),
            2,
        );
        assert_eq!("adopt tzdb \"2026b\" from \"tz-2026b.tzif\"", operator);
        assert!(detail.contains("`names`"), "{detail}");
    });
}

#[test]
fn explain_of_a_call_of_a_translated_bundle_is_refused_as_not_registered() {
    let tiered = "import auth
query tiered (t: Tier) -> Set Text = from u in users where u.tier == t select u.name
";
    once(100, |rt| {
        let db = opened(
            &rt,
            &[V1, RENAME_PAID],
            vec![bundle("web-1.0", &[V1], "", tiered)],
        );
        for call in [
            "explain web-1.0::tiered(Pro)",
            "explain web-1.0::tiered(Paid)",
        ] {
            let explained = db.query_text(call, &[], &CallOptions::default());
            let shown = format!("{explained:?}");
            assert!(
                shown.contains("bundle `web-1.0` is not registered"),
                "{call}: {shown}"
            );
        }
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
        (2, 4, 6),
        runs,
        "every pin, every crossed operator and each named pair ran"
    );
    assert_eq!(
        expected.keys().collect::<Vec<_>>(),
        got.keys().collect::<Vec<_>>(),
        "every expectation is reached"
    );
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

#[test]
fn enabling_a_constructor_a_read_key_holds_is_refused_while_a_bundle_reads_the_key() {
    let pin = "module auth
add module auth
add type Tier = Free | Paid
add type P = { t: Tier, n: Text }
add relation p : P key (t, n)
add type Q = { id: Key qs, o: Key p }
add relation qs : Q key id generate sequence
";
    let grow = "module auth\nadd constructor Tier.Gold\n";
    let enable = "module auth\nenable Tier.Gold\n";
    let body = "import auth\nquery keys () -> Set (Key p) = from x in qs select x.o\n";
    let registered = || vec![bundle("web-1.0", &[pin], "", body)];
    let mut sim = Sim::new(212, Config::default());
    run(&mut sim, |rt| {
        opened(&rt, &[pin, grow], registered());
    });
    run(&mut sim, |rt| {
        let (_, detail) = refused_migration(open(&rt, &[pin, grow, enable], registered()), 3);
        assert_eq!(
            "`enable Tier.Gold` is refused: bundle web-1.0, pinned at version 1, reads a field whose type reaches Tier; a bundle stops blocking only when it is released",
            detail
        );
    });
}

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

#[test]
fn a_field_read_through_try_reads_that_field_and_a_returned_or_read_tier_blocks() {
    let log = [V1, ADD_GOLD, ENABLE_GOLD];
    let name_only = "import auth
query q () -> Set Text = from u in users select match try u
  | Ok o -> o.value.name
  | Err _ -> \"none\"
";
    once(29, |rt| {
        let db = opened(&rt, &log, vec![bundle("web-1.0", &[V1], "", name_only)]);
        results(&db, r#"insert users { name: "gil", tier: Gold }"#);
        assert_eq!(vec!["\"gil\""], rows(call(&db, "web-1.0", 1, &[])));
    });
    let tier = "import auth
query q () -> Set Text = from u in users select match try u
  | Ok o -> (match o.value.tier | Free -> \"f\" | Paid -> \"p\")
  | Err _ -> \"none\"
";
    let whole =
        "import auth\nquery q () -> Set (Result User Fault) = from u in users select try u\n";
    for (seed, body) in [(30, tier), (32, whole)] {
        once(seed, |rt| {
            assert_eq!(
                RejectReason::BundleStale {
                    bundle: "web-1.0".into(),
                    ctor: "Tier.Gold".into()
                },
                refusal(
                    open(&rt, &log, vec![bundle("web-1.0", &[V1], "", body)]),
                    "web-1.0"
                ),
                "{body}"
            );
        });
    }
}

#[test]
fn a_pinned_insert_built_under_a_selection_or_fix_is_refused_lacking_the_added_field() {
    let log = [V1, ADD_VERIFIED, DEFAULT_VERIFIED];
    let selected = "import auth
txn selected (e: Text) -> Text =
  transact
    insert identities from ([EmailPass { id: identities.key(1), user: users.key(1), email: e }].EmailPass)
    return e
";
    let fixed = "import auth
txn fixed (e: Text) -> Text =
  transact
    insert identities from (fix s = [EmailPass { id: identities.key(2), user: users.key(1), email: e }] in s)
    return e
";
    for (seed, name, body) in [(31, "selected", selected), (33, "fixed", fixed)] {
        once(seed, |rt| {
            assert_eq!(
                RejectReason::Untranslatable {
                    query: format!("web-1.0::{name}"),
                    detail: "across `add field Identity.EmailPass.verified: Bool fill false`: it builds `Identity.EmailPass` without field `verified`, outside a write".into(),
                },
                refusal(
                    open(&rt, &log, vec![bundle("web-1.0", &[V1], "", body)]),
                    "web-1.0"
                ),
            );
        });
    }
}

#[test]
fn a_pinned_tag_constructor_keeps_its_constructor_after_a_removal_moves_its_position() {
    let add = "module auth\nadd type T { id: Key ts } = A { n: I32 } | B { n: I32 } | C { n: I32 }\nadd relation ts : T key id generate sequence\n";
    let deprecate = "module auth\ndeprecate ts.A\n";
    let remove = "module auth\nremove constructor T.A\n";
    let log = [V1, add, deprecate, remove];
    let body = "import auth
query b () -> Tag T = B
query c () -> Tag T = C
fn f () -> Tag T = C
query called () -> Tag T = f()
query chosen () -> Tag T = if 1 == 1 then B else C
";
    once(35, |rt| {
        let db = opened(&rt, &log, vec![bundle("web-1.0", &[V1, add], "", body)]);
        let expected = ["B", "C", "C", "B"];
        for (ordinal, want) in (1..).zip(expected) {
            assert_eq!(
                vec![want],
                rows(call(&db, "web-1.0", ordinal, &[])),
                "{ordinal}"
            );
        }
    });
    once(36, |rt| {
        let body = "import auth\nquery a () -> Tag T = A\n";
        assert_eq!(
            RejectReason::Untranslatable {
                query: "web-1.0::a".into(),
                detail: "across `remove constructor T.A`: it builds `T.A`, which the current version lacks".into(),
            },
            refusal(
                open(&rt, &log, vec![bundle("web-1.0", &[V1, add], "", body)]),
                "web-1.0"
            ),
        );
    });
}

#[test]
fn a_pinned_selection_reads_its_constructors_after_a_removal_moves_their_positions() {
    let add = "module auth\nadd type T { id: Key ts } = A { n: I32 } | B { n: I32 } | C { n: I32 }\nadd relation ts : T key id generate sequence\n";
    let deprecate = "module auth\ndeprecate ts.A\n";
    let remove = "module auth\nremove constructor T.A\n";
    let body = "import auth
query built_b () -> Set I32 = from x in [B { id: ts.key(9), n: 7 }].B select x.n
query read_b () -> Set I32 = from x in (from r in ts select r).B select x.n
query named_b () -> Set I32 = from x in ts.B select x.n
query built_c () -> Set I32 = from x in [B { id: ts.key(9), n: 7 }, C { id: ts.key(8), n: 8 }].C select x.n
query read_a () -> Set I32 = from x in (from r in ts select r).A select x.n
query matched_a () -> Set I32 = from x in (from r in ts select r).A select (match x | A a -> a.n)
query named_a () -> Set I32 = from x in ts.A select (match x | A a -> a.n)
query matched_b () -> Set I32 = from x in ts.B select (match x | B b -> b.n)
query matched_all () -> Set I32 = from r in ts select (match r | A a -> a.n | B b -> b.n + 10 | C c -> c.n + 20)
";
    once(34, |rt| {
        let log = [V1, add, deprecate, remove];
        let db = opened(&rt, &log, vec![bundle("web-1.0", &[V1, add], "", body)]);
        results(
            &db,
            "insert ts from [B { id: ts.key(1), n: 2 }, C { id: ts.key(2), n: 3 }]",
        );
        let expected: [&[&str]; 9] = [
            &["7"],
            &["2"],
            &["2"],
            &["8"],
            &[],
            &[],
            &[],
            &["2"],
            &["12", "23"],
        ];
        for (ordinal, want) in (1..).zip(expected) {
            assert_eq!(want, rows(call(&db, "web-1.0", ordinal, &[])), "{ordinal}");
        }
    });
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

const FILLED: &str = "module auth\nadd type H { id: Key hs } = R { n: I32 }\nadd relation hs : H key id generate sequence\n";
const FILLED_INSERT: &str = "import auth\ntxn put () -> I32 =\n  transact\n    let h = insert hs.R { n: 1 }\n    return h.n\n";

#[test]
fn a_tag_in_a_fill_past_a_removal_keeps_its_constructor_or_refuses_by_its_name() {
    let deprecate = "module auth\ndeprecate P.X\n";
    let remove = "module auth\nremove constructor P.X\n";
    let pin = [V1, FILLED];
    let field = |c: &str| format!("module auth\nadd field H.R.t: Tag P fill {c}\n");
    let tys = "module auth\nadd type P = X | Y | Z\n";
    let seed = 980;
    let x = field("X");
    once(seed, |rt| {
        assert_eq!(
            RejectReason::Untranslatable {
                query: "web-1.0::put".into(),
                detail: "across `remove constructor P.X`: it builds `P.X`, which the current version lacks".into(),
            },
            refusal(
                open(
                    &rt,
                    &[V1, FILLED, tys, x.as_str(), deprecate, remove],
                    vec![bundle("web-1.0", &pin, "", FILLED_INSERT)],
                ),
                "web-1.0"
            ),
        );
    });
    for (c, seed) in [("Y", seed + 10), ("Z", seed + 20)] {
        let f = field(c);
        once(seed, |rt| {
            let db = opened(
                &rt,
                &[V1, FILLED, tys, f.as_str(), deprecate, remove],
                vec![bundle("web-1.0", &pin, "", FILLED_INSERT)],
            );
            call(&db, "web-1.0", 1, &[]);
            assert_eq!(
                vec![format!("R {{ id: hs.key(1), n: 1, t: {c} }}")],
                query(&db, "from h in hs select h")
            );
        });
    }
}

#[test]
fn a_newtype_over_a_declared_sum_is_refused() {
    let add = "module auth\nadd type T = A | B\nadd type U = new T\n";
    once(983, |rt| {
        let (_, detail) = refused_migration(open(&rt, &[V1, add], vec![]), 2);
        assert_eq!("a newtype is over a base type", detail);
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

#[test]
fn a_pinned_write_calls_the_function_a_fill_or_default_named_after_positions_move() {
    let host = |n: &str| {
        format!("define fn {n} (t: Text) -> Text = host \"{n}\" version \"1.0\" deterministic\n")
    };
    let base = format!(
        "module auth\n{}{}add type H {{ id: Key hs }} = R {{ n: Text }}\nadd relation hs : H key id generate sequence\n",
        host("g"),
        host("h")
    );
    let shift = format!("module auth\ndrop fn g\n{}", host("n"));
    let body = "import auth\ntxn put () -> I32 =\n  transact\n    let h = insert hs.R { n: \"s\" }\n    return 1\n";
    let pin = [V1, base.as_str()];
    let opening = |rt: &SimRuntime, late: &str| {
        let options = OpenOptions {
            host_fns: named_host_fns(&["g", "h", "n"]),
            ..with(vec![bundle("web-1.0", &pin, "", body)])
        };
        Database::open_in(
            rt.clone(),
            "app.adt",
            &log(&[V1, base.as_str(), shift.as_str(), late]),
            &options,
        )
    };
    let default = |f: &str| {
        format!("module auth\nadd field H.R.x: Text fill \"z\"\nadd default hs.R.x = {f}(\"a\")\n")
    };
    let fill = |f: &str| format!("module auth\nadd field H.R.x: Text fill {f}(\"a\")\n");
    for (seed, late) in [(984, fill("h")), (985, default("h"))] {
        once(seed, |rt| {
            let db = opening(&rt, &late).unwrap_or_else(|e| panic!("{late}: {e:?}"));
            call(&db, "web-1.0", 1, &[]);
            assert_eq!(
                vec!["R { id: hs.key(1), n: \"s\", x: \"h\" }".to_string()],
                query(&db, "from h in hs select h"),
                "{late}"
            );
        });
    }
    let n_fill = fill("n");
    let refused = "it calls `n`, whose code changed since the pin";
    for (seed, late, detail) in [
        (
            986,
            n_fill.as_str(),
            format!("across `add field H.R.x: Text fill n(\"a\")`: {refused}"),
        ),
        (987, default("n").as_str(), refused.to_string()),
    ] {
        once(seed, |rt| {
            assert_eq!(
                RejectReason::Untranslatable {
                    query: "web-1.0::put".into(),
                    detail,
                },
                refusal(opening(&rt, late), "web-1.0"),
            );
        });
    }
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

/// Returns a simulation whose database at the pin holds one row with `n` "old".
fn coded_sim(seed: u64) -> Sim {
    let mut sim = Sim::new(seed, Config::default());
    run(&mut sim, |rt| {
        let db = coded_open(&rt, &CODED_PIN, vec![]).expect("the pin opens");
        results(&db, "insert hs.R { n: \"old\" }");
    });
    sim
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

#[test]
fn a_pinned_fill_whose_function_went_to_other_code_and_back_is_refused_and_the_rows_keep_the_fill()
{
    let same_migration = format!(
        "{TO_B}add field H.R.x: Text fill f(\"s\")\nredefine fn f (t: Text) -> Text = host \"a\" version \"1.0\" deterministic\n"
    );
    let logs: [&[&str]; 2] = [
        &[V1, CODED, TO_B, FILL_F, TO_A],
        &[V1, CODED, same_migration.as_str()],
    ];
    for (seed, migrations) in (990..).zip(logs) {
        let mut sim = coded_sim(seed);
        run(&mut sim, |rt| {
            let db = coded_open(&rt, &CODED_PIN, coded_put("web-1.0")).expect("the pin opens");
            query(&db, "retire bundle web-1.0");
        });
        run(&mut sim, |rt| {
            assert_eq!(
                stale_fill("web-1.1", REDEFINED_B, "f"),
                refusal(coded_open(&rt, migrations, coded_put("web-1.1")), "web-1.1"),
                "{migrations:?}"
            );
        });
        run(&mut sim, |rt| {
            let db = coded_open(&rt, migrations, vec![]).expect("the log applies");
            assert_eq!(
                vec!["R { id: hs.key(1), n: \"old\", x: \"b\" }".to_string()],
                query(&db, "from h in hs select h"),
                "{migrations:?}"
            );
        });
    }
}

#[test]
fn a_redefine_of_a_host_function_is_refused_while_a_live_bundle_pins_another_code() {
    let version =
        "module auth\nredefine fn f (t: Text) -> Text = host \"a\" version \"2.0\" deterministic\n";
    let reads = "import auth\nquery q () -> Set Text = from h in hs select h.n\n";
    let cases = [
        (reads, TO_B, REDEFINED_B),
        (reads, version, version.lines().nth(1).expect("an operator")),
        (
            "import auth\nquery q () -> Set Text = from h in hs select f(h.n)\n",
            TO_B,
            REDEFINED_B,
        ),
        (
            "import auth\ntxn q () -> I32 =\n  transact\n    let h = insert hs.R { n: f(\"s\") }\n    return 1\n",
            TO_B,
            REDEFINED_B,
        ),
    ];
    for (seed, (body, to, operator)) in (994..).zip(cases) {
        let regs = || vec![bundle("web-1.0", &CODED_PIN, "", body)];
        let mut sim = Sim::new(seed, Config::default());
        run(&mut sim, |rt| {
            coded_open(&rt, &CODED_PIN, regs()).expect("the pin opens");
        });
        run(&mut sim, |rt| {
            let (refused_by, detail) =
                refused_migration(coded_open(&rt, &[V1, CODED, to], regs()), 3);
            assert_eq!(operator, refused_by, "{body}");
            assert_eq!(
                "bundle web-1.0 pins 2: its version holds another code of host or Wasm function `f`",
                detail,
                "{body}"
            );
        });
    }
}

const BARE: &str = "module auth\nadd module auth\n";
const BARE_PIN: [&str; 2] = [BARE, CODED];
const CODED_READS: &str = "import auth\nquery q () -> Set Text = from h in hs select h.n\n";
const TO_WASM: &str = "module auth\nredefine fn f (t: Text) -> Text = wasm \"m\" export \"e\"\n";
const REDEFINED_WASM: &str = "redefine fn f (t: Text) -> Text = wasm \"m\" export \"e\"";
const WASM_LINE: &str = "function `f` names Wasm module `m`, which no migration installs";
const DEFINE_WASM: &str = "module auth\ndefine fn w (t: Text) -> Text = wasm \"m\" export \"e\"\n";
const REVERT_3: &str = "module auth\nrevert 0003\n";
const INSTALL: &str = "module auth\ninstall geo \"1.2\" from \"geo.adtx\"\n";
const UNINSTALL: &str = "module auth\nuninstall geo\n";
const BLAMELESS: &str = "module auth\nadd type T = A | B\ndrop type T\nredefine fn f (t: Text) -> Text = host \"b\" version \"1.0\" deterministic\n";

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

/// Returns the `n` and `x` of each row of `hs`.
fn n_and_x(db: &Database<SimRuntime>) -> BTreeSet<String> {
    set(unquoted(query(
        db,
        r#"from h in hs select h.n ++ " " ++ h.x"#,
    )))
}

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

#[test]
fn a_live_pin_does_not_hold_back_code_it_does_not_pin() {
    let define_g =
        "module auth\ndefine fn g (t: Text) -> Text = host \"a\" version \"1.0\" deterministic\n";
    let move_g =
        "module auth\nredefine fn g (t: Text) -> Text = host \"b\" version \"1.0\" deterministic\n";
    let define_l = "module auth\ndefine fn l (t: Text) -> Text = t\n";
    let move_l = "module auth\nredefine fn l (t: Text) -> Text = t ++ t\n";
    let migrations = [BARE, CODED, TO_A, define_g, move_g, define_l, move_l];
    let mut sim = holding(1152, &BARE_PIN, Holder::LivePresented);
    run(&mut sim, |rt| {
        let regs = vec![bundle("web-1.0", &BARE_PIN, " lease 1d", CODED_READS)];
        let db = coded_open(&rt, &migrations, regs).unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(vec!["web-1.0 active"], states(&db));
        assert_eq!(vec!["a 1.0"], stored_host_f(&db));
        assert_eq!(
            vec!["b".to_string()],
            unquoted(query(
                &db,
                r#"from h in catalog::functions.Host where h.name == "g" select h.symbol"#,
            ))
        );
        assert_eq!(
            vec!["t ++ t".to_string()],
            unquoted(query(
                &db,
                r#"from h in catalog::functions.Language where h.name == "l" select h.body"#,
            ))
        );
    });
}

#[test]
fn a_bundle_refused_at_other_code_is_admitted_after_a_redefine_back_and_its_fill_stores_that_code()
{
    let mut sim = coded_sim(997);
    run(&mut sim, |rt| {
        assert_eq!(
            other_code("web-1.0::put"),
            refusal(
                coded_open(&rt, &[V1, CODED, TO_B], coded_put("web-1.0")),
                "web-1.0"
            ),
        );
    });
    run(&mut sim, |rt| {
        let db = coded_open(&rt, &[V1, CODED, TO_B, TO_A, FILL_F], coded_put("web-1.0"))
            .expect("every code matches the pin");
        call(&db, "web-1.0", 1, &[]);
        assert_eq!(set(["old a".to_string(), "s a".to_string()]), n_and_x(&db));
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
fn a_pinned_write_takes_a_default_with_the_current_code_when_it_is_the_pins() {
    let added = "module auth\nadd field H.R.x: Text fill \"z\"\nadd default hs.R.x = f(\"s\")\n";
    let mut sim = coded_sim(1000);
    run(&mut sim, |rt| {
        let db = coded_open(&rt, &[V1, CODED, TO_B, added, TO_A], coded_put("web-1.0"))
            .expect("the default's code at the pin is current");
        call(&db, "web-1.0", 1, &[]);
        assert_eq!(set(["old z".to_string(), "s a".to_string()]), n_and_x(&db));
    });
    once(1001, |rt| {
        assert_eq!(
            other_code("web-1.0::put"),
            refusal(
                coded_open(&rt, &[V1, CODED, TO_B, added], coded_put("web-1.0")),
                "web-1.0"
            ),
        );
    });
}

#[test]
fn a_tag_in_a_default_past_a_removal_keeps_its_constructor() {
    let types = "module auth\nadd type P = X | Y | Z\n";
    let add =
        |c: &str| format!("module auth\nadd field H.R.t: Tag P fill Y\nadd default hs.R.t = {c}\n");
    let pin = [V1, FILLED];
    let (z, x) = (add("Z"), add("X"));
    let deprecate = "module auth\ndeprecate P.X\n";
    let remove = "module auth\nremove constructor P.X\n";
    once(1002, |rt| {
        let db = opened(
            &rt,
            &[V1, FILLED, types, z.as_str(), deprecate, remove],
            vec![bundle("web-1.0", &pin, "", FILLED_INSERT)],
        );
        call(&db, "web-1.0", 1, &[]);
        assert_eq!(
            vec!["R { id: hs.key(1), n: 1, t: Z }".to_string()],
            query(&db, "from h in hs select h")
        );
    });
    once(1003, |rt| {
        let got = open(
            &rt,
            &[V1, FILLED, types, x.as_str(), deprecate, remove],
            vec![],
        );
        let (_, detail) = refused_migration(got, 6);
        assert_eq!("`X` is not a constructor of Tag P", detail);
    });
}

#[test]
fn a_pinned_default_folds_by_its_aggregate_after_a_drop_moves_its_position() {
    let agg = |n: &str, plus: u32| {
        format!(
            "define aggregate {n} (x: I64) -> I64 = state {{ n: I64 }} init {{ n: 0 }} step (s, x) -> {{ n: s.n + x }} merge (a, b) -> {{ n: a.n + b.n }} finish s -> s.n + {plus}\n"
        )
    };
    let base = format!(
        "module auth\n{}{}add type H {{ id: Key hs }} = R {{ n: I32 }}\nadd relation hs : H key id generate sequence\n",
        agg("other", 100),
        agg("total", 0),
    );
    let added = "module auth\nadd field H.R.x: Option I64 fill None\nadd default hs.R.x = total(v for v in [1, 2])\n";
    let pin = [V1, base.as_str()];
    let regs = || vec![bundle("web-1.0", &pin, "", FILLED_INSERT)];
    once(1004, |rt| {
        let drop = "module auth\ndrop aggregate other\n";
        let db = opened(&rt, &[V1, base.as_str(), drop, added], regs());
        call(&db, "web-1.0", 1, &[]);
        assert_eq!(
            vec!["R { id: hs.key(1), n: 1, x: Some { value: 3 } }".to_string()],
            query(&db, "from h in hs select h")
        );
    });
    let again = format!("module auth\ndrop aggregate total\n{}", agg("total", 0));
    once(1005, |rt| {
        assert_eq!(
            RejectReason::Untranslatable {
                query: "web-1.0::put".into(),
                detail: format!(
                    "across `{}`: it folds by an aggregate whose code changed since the pin",
                    "define aggregate total (x: I64) -> I64 = state { n: I64 } init { n: 0 } step (s, x) -> { n: s.n + x } merge (a, b) -> { n: a.n + b.n } finish s -> s.n + 0"
                ),
            },
            refusal(
                open(&rt, &[V1, base.as_str(), again.as_str(), added], regs()),
                "web-1.0"
            ),
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
