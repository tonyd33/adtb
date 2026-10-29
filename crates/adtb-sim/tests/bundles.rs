//! Bundles in process: the bundle file and its header, registration and its checks at the
//! pinned version, immutability, supersession at open, leases, retirement and supersession by
//! command, and their persistence across reopen and crash.

use adtb_simkit::api;

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

use api::{once, options, printed, result, run, sorted};

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

fn query(db: &Database<SimRuntime>, text: &str) -> Vec<String> {
    sorted(result(db, text))
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
        let detail = invalid(
            open(&rt, &[V1], vec![bundle("web-1.0", &[V1], "", NAMES)]),
            "web-1.0",
        );
        assert!(detail.contains("retired"), "{detail}");
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
        epoch: u32::MAX,
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
