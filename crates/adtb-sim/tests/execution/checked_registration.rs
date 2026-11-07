//! Registration through a role session: the registering role's grants with grant option, the
//! family it comes to own, the recheck of its bundles at each migration, and their persistence
//! across reopen and crash.

use adtb_simkit::api;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use adtb::runtime::Threads;
use adtb::{CallOptions, Database, Error, Log, OpenError, OpenOptions, Registration, RejectReason};
use adtb_lang::evolve::Source;
use adtb_lang::parse::{Options, parse_migration};
use adtb_sim::{Config, Outcome, Sim, SimRuntime};
use adtb_testkit::profile::sweep;

use api::{options, query, run};

const SHOP: &str = r"module shop
add module shop
add type Customer = { id: Key customers, name: Text, email: Text }
add relation customers : Customer key id generate sequence
";

const GRANTS: &str = "module shop
grant read customers to web with grant option
grant read customers to ops with grant option
grant read customers to app
";

const NAMES: &str = "import shop
query names () -> Set Text = from c in customers select c.name
";

const EMAILS: &str = "import shop
query emails () -> Set Text = from c in customers select c.email
";

const ROLES: [&str; 4] = ["web", "ops", "app", "idle"];

fn open(
    rt: &SimRuntime,
    log: &[&str],
    regs: Vec<Registration>,
) -> Result<Database<SimRuntime>, OpenError> {
    Database::open_in(
        rt.clone(),
        "shop.adt",
        &Log::new(log.iter().copied()),
        &OpenOptions {
            registrations: regs,
            ..options(2)
        },
    )
}

fn opened(rt: &SimRuntime, log: &[&str]) -> Database<SimRuntime> {
    open(rt, log, Vec::new()).unwrap_or_else(|e| panic!("the database opens: {e:?}"))
}

/// Returns the database at the shop schema with every role of `ROLES` and the grants of
/// `GRANTS`.
fn shop(rt: &SimRuntime) -> Database<SimRuntime> {
    let db = opened(rt, &[SHOP]);
    for role in ROLES {
        db.role_authority().create_role(role).expect("the role");
    }
    db.close().expect("close");
    opened(rt, &[SHOP, GRANTS])
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
    adtb::catalog::version_hash(&schema, &source)
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

fn bundle(name: &str, pins: &[&str], body: &str) -> Registration {
    leased(name, pins, "", body)
}

fn leased(name: &str, pins: &[&str], clauses: &str, body: &str) -> Registration {
    Registration::bundle(file(name, pins, clauses, body)).expect("the bundle file parses")
}

/// Register `reg` as `role`.
fn register(db: &Database<SimRuntime>, role: &str, reg: &Registration) -> Result<(), Error> {
    let authority = db.role_authority();
    let session = authority.session(role).expect("the session");
    authority.register(&session, reg).map(|_| ())
}

fn denied(got: Result<(), Error>) -> String {
    match got {
        Err(Error::Rejected {
            reason: RejectReason::Denied { detail },
        }) => detail,
        other => panic!("{other:?}"),
    }
}

/// Returns the owner of each bundle, by name.
fn owners(db: &Database<SimRuntime>) -> BTreeMap<String, String> {
    query(
        db,
        "from b in catalog::bundles select { name: b.name, owner: b.owner }",
    )
    .into_iter()
    .map(|row| {
        let parts: Vec<&str> = row.split('"').collect();
        (parts[1].to_owned(), parts[3].to_owned())
    })
    .collect()
}

fn calls(db: &Database<SimRuntime>, name: &str) -> Vec<String> {
    let reg = db
        .registered(name)
        .unwrap_or_else(|e| panic!("{name} is served: {e:?}"));
    let rows = reg
        .call(1, &[], &CallOptions::default())
        .unwrap_or_else(|e| panic!("{name} calls: {e:?}"));
    match rows {
        adtb::Called::Rows(rows) => api::sorted(rows),
        other => panic!("{other:?}"),
    }
}

fn unknown<T: std::fmt::Debug>(got: Result<T, Error>) {
    assert!(
        matches!(
            got,
            Err(Error::Rejected {
                reason: RejectReason::UnknownBundle { .. }
            })
        ),
        "{got:?}"
    );
}

#[test]
fn a_name_taken_by_other_content_is_refused() {
    let mut sim = Sim::new(304, Config::default());
    run(&mut sim, |rt| {
        let db = shop(&rt);
        let pins = &[SHOP, GRANTS];
        register(&db, "web", &bundle("shop-1.0", pins, NAMES)).expect("web registers");
        register(&db, "web", &bundle("shop-1.0", pins, NAMES)).expect("the same content again");
        assert!(matches!(
            register(&db, "web", &bundle("shop-1.0", pins, EMAILS)),
            Err(Error::Rejected {
                reason: RejectReason::Invalid { .. }
            })
        ));
        assert!(matches!(
            register(&db, "web", &Registration::new("loose", NAMES)),
            Err(Error::Rejected {
                reason: RejectReason::Invalid { .. }
            })
        ));
    });
}

#[test]
fn a_header_an_open_recorded_and_failed_claims_nothing() {
    let mut sim = Sim::new(305, Config::default());
    run(&mut sim, |rt| {
        let db = opened(&rt, &[SHOP]);
        for role in ROLES {
            db.role_authority().create_role(role).expect("the role");
        }
        db.close().expect("close");
        let early = bundle("shop-1.0", &[SHOP, GRANTS], NAMES);
        assert!(matches!(
            open(&rt, &[SHOP], vec![early.clone()]),
            Err(OpenError::RegistrationRefused { .. })
        ));
        let db = opened(&rt, &[SHOP, GRANTS]);
        assert_eq!(Some(&"owner".to_owned()), owners(&db).get("shop-1.0"));
        register(&db, "web", &early).expect("web claims the failed header");
        assert_eq!(Some(&"web".to_owned()), owners(&db).get("shop-1.0"));
        db.close().expect("close");
        let db = opened(&rt, &[SHOP, GRANTS]);
        assert_eq!(
            "family shop is owned by role web",
            denied(register(
                &db,
                "ops",
                &bundle("shop-1.1", &[SHOP, GRANTS], NAMES)
            ))
        );
    });
}

fn arrive(rt: &SimRuntime, gate: &AtomicUsize) {
    gate.fetch_add(1, Ordering::SeqCst);
    while gate.load(Ordering::SeqCst) < 2 {
        rt.yield_now();
    }
}

#[test]
fn two_roles_racing_for_one_family_leave_exactly_one_owner() {
    for seed in sweep(310..314) {
        let mut sim = Sim::new(
            seed,
            Config {
                switch_rate: 0.3,
                ..Config::default()
            },
        );
        let got = sim.run(|rt| {
            let db = shop(&rt);
            let outcomes = Arc::new(Mutex::new(Vec::new()));
            let gate = Arc::new(AtomicUsize::new(0));
            let mut joins = Vec::new();
            for (role, name) in [("web", "shop-1.0"), ("ops", "shop-2.0")] {
                let db = db.clone();
                let done = outcomes.clone();
                let ready = gate.clone();
                let runtime = rt.clone();
                let reg = bundle(name, &[SHOP, GRANTS], NAMES);
                joins.push(rt.spawn(role, move || {
                    arrive(&runtime, &ready);
                    let outcome = register(&db, role, &reg);
                    done.lock().unwrap().push((role, outcome));
                }));
            }
            for j in joins {
                rt.join(j);
            }
            let outcomes = outcomes.lock().unwrap().clone();
            let won: Vec<&str> = outcomes
                .iter()
                .filter(|(_, o)| o.is_ok())
                .map(|(r, _)| *r)
                .collect();
            assert_eq!(1, won.len(), "seed={seed}: {outcomes:?}");
            let lost = outcomes.iter().find(|(_, o)| o.is_err()).unwrap();
            assert_eq!(
                format!("family shop is owned by role {}", won[0]),
                denied(lost.1.clone()),
                "seed={seed}"
            );
            assert_eq!(1, owners(&db).len(), "seed={seed}");
        });
        assert_eq!(Outcome::Finished, got, "seed={seed}");
    }
}

#[test]
fn a_crash_at_every_flush_of_a_registration_keeps_all_of_it_or_none() {
    let seed = 320;
    let mut sim = Sim::new(seed, Config::default());
    sim.record();
    let mut start = 0;
    run(&mut sim, |rt| {
        let db = shop(&rt);
        start = rt.recorded();
        register(&db, "web", &bundle("shop-1.0", &[SHOP, GRANTS], NAMES)).expect("web registers");
    });
    let recording = sim.recording();
    let flushes: Vec<usize> = recording
        .flushes()
        .into_iter()
        .filter(|&f| f >= start)
        .collect();
    assert!(!flushes.is_empty(), "seed {seed}: the registration flushes");
    let mut kept = false;
    for at in flushes {
        let at_seed = seed + at as u64;
        let files = recording.images(at, 1, at_seed).swap_remove(0);
        let mut sim = Sim::with_files(at_seed, Config::default(), files);
        run(&mut sim, |rt| {
            let db = opened(&rt, &[SHOP, GRANTS]);
            let ops = register(&db, "ops", &bundle("shop-1.1", &[SHOP, GRANTS], NAMES));
            match owners(&db).get("shop-1.0") {
                Some(owner) => {
                    kept = true;
                    assert_eq!("web", owner, "seed {at_seed}");
                    assert_eq!(
                        Vec::<String>::new(),
                        calls(&db, "shop-1.0"),
                        "seed {at_seed}"
                    );
                    assert_eq!(
                        "family shop is owned by role web",
                        denied(ops),
                        "seed {at_seed}"
                    );
                }
                None => {
                    unknown(db.registered("shop-1.0"));
                    ops.unwrap_or_else(|e| {
                        panic!("seed {at_seed}: the family is unclaimed: {e:?}")
                    });
                }
            }
        });
    }
    assert!(kept, "seed {seed}: the last flush keeps the registration");
}

/// Assert that a bundle importing the last function of `fns` is refused when it registers.
fn import_is_refused(seed: u64, fns: &str, last: &str) {
    let mut sim = Sim::new(seed, Config::default());
    run(&mut sim, |rt| {
        let pins = &[SHOP, GRANTS, fns];
        shop(&rt).close().expect("close");
        let db = opened(&rt, pins);
        let body = format!("import shop\nimport fn {last}\n");
        let got = register(&db, "owner", &bundle("deep-1.0", pins, &body));
        let detail = match got {
            Err(Error::Rejected {
                reason: RejectReason::Invalid { detail },
            }) => detail,
            other => panic!("{other:?}"),
        };
        assert_eq!("the call's body expands past what the engine plans", detail);
    });
}

/// Returns a module defining `d0` to `d{last}`, each two operators above the one before.
fn deepening_fns(last: usize) -> String {
    let mut fns = String::from("module shop\ndefine fn d0 (x: I64) -> I64 = x + 1\n");
    for n in 1..=last {
        let p = n - 1;
        fns.push_str(&format!(
            "define fn d{n} (x: I64) -> I64 = (d{p}(x) + 1) + 1\n"
        ));
    }
    fns
}

#[test]
fn a_function_too_deep_to_run_is_refused_at_its_definition() {
    let mut sim = Sim::new(380, Config::default());
    run(&mut sim, |rt| {
        shop(&rt).close().expect("close");
        let fns = deepening_fns(42);
        opened(&rt, &[SHOP, GRANTS, fns.as_str()])
            .close()
            .expect("the deepest that is defined");
    });
    let mut sim = Sim::new(381, Config::default());
    run(&mut sim, |rt| {
        shop(&rt).close().expect("close");
        let fns = deepening_fns(43);
        match open(&rt, &[SHOP, GRANTS, fns.as_str()], Vec::new()) {
            Err(OpenError::MigrationRefused { version: 3, reason }) => {
                let got = format!("{reason:?}");
                assert!(
                    got.contains("the text expands past what the engine normalises"),
                    "{got}"
                );
            }
            Err(other) => panic!("{other:?}"),
            Ok(_) => panic!("d43 is defined"),
        }
    });
}

#[test]
fn an_imported_function_past_the_run_bound_is_refused() {
    let mut sim = Sim::new(382, Config::default());
    run(&mut sim, |rt| {
        let fns = deepening_fns(31);
        let pins = &[SHOP, GRANTS, fns.as_str()];
        shop(&rt).close().expect("close");
        let db = opened(&rt, pins);
        let body = "import shop\nimport fn d31\n";
        register(&db, "owner", &bundle("deep-1.0", pins, body)).expect("the deepest that runs");
    });
    import_is_refused(383, &deepening_fns(32), "d32");
    import_is_refused(384, &deepening_fns(42), "d42");
}

#[test]
fn an_imported_function_that_doubles_is_refused() {
    let mut fns = String::from("module shop\ndefine fn d0 (x: I64) -> Set I64 = [x]\n");
    for n in 1..=40 {
        let p = n - 1;
        fns.push_str(&format!(
            "define fn d{n} (x: I64) -> Set I64 = d{p}(x) union d{p}(x)\n"
        ));
    }
    import_is_refused(385, &fns, "d40");
}

/// Returns the migration that adds `body`, calling `d{k}`, over the function chain to `d{k}`.
fn planned_body(k: usize, body: &str) -> String {
    let fns = deepening_fns(k);
    let body = body.replace("DEEP", &format!("d{k}"));
    format!("{fns}{body}\n")
}

/// Each kind of body the engine plans, calling `DEEP`, with the deepest `d{k}` it may call.
const PLANNED_BODIES: [(&str, usize); 5] = [
    (
        "add constraint deep = from c in customers where DEEP(1) == 0 select c.id",
        30,
    ),
    (
        "add type T = { id: Key ts, n: I64 }\nadd relation ts : T key id generate sequence\n  default n = DEEP(1)",
        31,
    ),
    (
        "add type T = { id: Key ts, n: I64 }\nadd relation ts : T key id generate sequence\nadd computed ts.c: I64 = DEEP(self.n)",
        31,
    ),
    (
        "add type T = { id: Key ts, n: I64 }\nadd relation ts : T key id generate sequence\n  index (DEEP(n))",
        31,
    ),
    (
        "define aggregate deep (x: I64) -> I64 = state { n: I64 } init { n: 0 } step (s, x) -> { n: DEEP(x) } merge (a, b) -> { n: a.n + b.n } finish s -> s.n",
        31,
    ),
];

#[test]
fn a_planned_body_past_the_run_bound_is_refused_at_its_definition() {
    for (body, deepest) in PLANNED_BODIES {
        for (k, passes) in [(deepest, true), (deepest + 1, false)] {
            let mut sim = Sim::new(386, Config::default());
            run(&mut sim, |rt| {
                shop(&rt).close().expect("close");
                let migration = planned_body(k, body);
                match open(&rt, &[SHOP, GRANTS, migration.as_str()], Vec::new()) {
                    Ok(_) => assert!(passes, "{body} at d{k} is defined"),
                    Err(OpenError::MigrationRefused { reason, .. }) => {
                        let got = format!("{reason:?}");
                        assert!(!passes, "{body} at d{k}: {got}");
                        assert!(
                            got.contains("the text expands past what the engine plans"),
                            "{got}"
                        );
                    }
                    Err(other) => panic!("{other:?}"),
                }
            });
        }
    }
}

#[test]
fn a_registered_transaction_of_a_thousand_inserts_registers_and_runs() {
    let mut body = String::from("import shop\ntxn many () -> I64 =\n  transact\n");
    for i in 0..1000 {
        body.push_str(&format!(
            "    insert customers {{ name: \"n{i}\", email: \"e{i}\" }}\n"
        ));
    }
    body.push_str("    return 1\n");
    let mut sim = Sim::new(387, Config::default());
    run(&mut sim, |rt| {
        let pins = &[SHOP, GRANTS];
        shop(&rt).close().expect("close");
        let db = opened(&rt, pins);
        register(&db, "owner", &bundle("many-1.0", pins, &body)).expect("it registers");
        let served = db.registered("many-1.0").expect("it is served");
        let called = served.call(1, &[], &CallOptions::default());
        assert!(
            matches!(called, Ok(adtb::Called::Committed(_))),
            "{:?}",
            called.map(drop)
        );
        assert_eq!(1000, query(&db, "from c in customers select c.id").len());
    });
}

#[test]
fn a_tall_function_chain_is_defined_up_to_its_limit() {
    let chain = |last: usize| {
        let mut fns = String::from("module shop\ndefine fn d0 (x: I64) -> I64 = x + 1\n");
        for n in 1..=last {
            let p = n - 1;
            fns.push_str(&format!(
                "define fn d{n} (x: I64) -> I64 = d{p}(x) + 1 + 1 + 1\n"
            ));
        }
        fns
    };
    let opens = |seed: u64, fns: String| {
        let mut sim = Sim::new(seed, Config::default());
        run(&mut sim, |rt| {
            shop(&rt).close().expect("close");
            open(&rt, &[SHOP, GRANTS, fns.as_str()], Vec::new()).map(drop)
        })
    };
    opens(388, chain(31)).expect("the tallest chain");
    let got = format!("{:?}", opens(389, chain(32)));
    assert!(
        got.contains("the text expands past what the engine normalises"),
        "{got}"
    );
}
