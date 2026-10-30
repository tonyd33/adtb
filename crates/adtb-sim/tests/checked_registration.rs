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

/// Returns the text of bundle `name` pinned at the last version of `pins`, holding `body`.
fn file(name: &str, pins: &[&str], body: &str) -> String {
    format!(
        "bundle {name} pins {} ({}) edition \"2026-10\"\n{body}",
        pins.len(),
        hash(pins)
    )
}

fn bundle(name: &str, pins: &[&str], body: &str) -> Registration {
    Registration::bundle(file(name, pins, body)).expect("the bundle file parses")
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
fn a_role_holding_every_grant_with_grant_option_registers_and_owns_the_bundle() {
    let mut sim = Sim::new(301, Config::default());
    run(&mut sim, |rt| {
        let db = shop(&rt);
        db.query_text(
            "insert customers { name: \"ann\", email: \"a@x\" }",
            &[],
            &CallOptions::default(),
        )
        .expect("a customer");
        register(&db, "web", &bundle("shop-1.0", &[SHOP, GRANTS], NAMES)).expect("web registers");
        assert_eq!(Some(&"web".to_owned()), owners(&db).get("shop-1.0"));
        assert_eq!(vec!["\"ann\""], calls(&db, "shop-1.0"));
    });
}

#[test]
fn a_grant_without_grant_option_refuses_listing_every_missing_grant_and_writes_nothing() {
    let mut sim = Sim::new(302, Config::default());
    run(&mut sim, |rt| {
        let db = shop(&rt);
        let both = format!("{NAMES}{}", EMAILS.trim_start_matches("import shop\n"));
        let reg = bundle("shop-1.0", &[SHOP, GRANTS], &both);
        let detail = denied(register(&db, "app", &reg));
        assert_eq!(
            [
                "names reads customers.Customer; app lacks read with grant option",
                "names reads customers.Customer (id); app lacks read with grant option",
                "names reads customers.Customer (name); app lacks read with grant option",
                "emails reads customers.Customer; app lacks read with grant option",
                "emails reads customers.Customer (id); app lacks read with grant option",
                "emails reads customers.Customer (email); app lacks read with grant option",
            ]
            .join("\n"),
            detail
        );
        denied(register(&db, "idle", &reg));
        assert!(owners(&db).is_empty());
        unknown(db.registered("shop-1.0"));
        register(&db, "ops", &reg).expect("the family is unclaimed");
    });
}

#[test]
fn the_first_successful_registration_claims_the_family_even_with_no_callables() {
    let mut sim = Sim::new(303, Config::default());
    run(&mut sim, |rt| {
        let db = shop(&rt);
        let pins = &[SHOP, GRANTS];
        register(&db, "idle", &bundle("tools-0.1", pins, "import shop\n")).expect("idle registers");
        assert_eq!(
            "family tools is owned by role idle",
            denied(register(&db, "web", &bundle("tools-0.2", pins, NAMES)))
        );
        register(&db, "web", &bundle("shop-1.0", pins, NAMES)).expect("web registers");
        assert_eq!(
            "family shop is owned by role web",
            denied(register(&db, "ops", &bundle("shop-1.1", pins, NAMES)))
        );
        register(&db, "owner", &bundle("shop-1.1", pins, NAMES)).expect("the schema owner");
        register(&db, "web", &bundle("shop-1.2", pins, EMAILS)).expect("web again");
        let got = owners(&db);
        assert_eq!(Some(&"owner".to_owned()), got.get("shop-1.1"));
        assert_eq!(Some(&"web".to_owned()), got.get("shop-1.2"));
        db.close().expect("close");
        let db = opened(&rt, &[SHOP, GRANTS]);
        assert_eq!(
            "family shop is owned by role web",
            denied(register(&db, "ops", &bundle("shop-1.3", pins, NAMES)))
        );
        assert_eq!(
            "family tools is owned by role idle",
            denied(register(&db, "web", &bundle("tools-0.2", pins, NAMES)))
        );
    });
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

#[test]
fn reopen_serves_a_registered_bundle_unpresented_and_supersedes_an_opened_one() {
    let mut sim = Sim::new(306, Config::default());
    run(&mut sim, |rt| {
        let db = shop(&rt);
        db.close().expect("close");
        let pins = &[SHOP, GRANTS];
        let db = open(&rt, pins, vec![bundle("legacy-1.0", pins, NAMES)]).expect("open");
        register(&db, "web", &bundle("shop-1.0", pins, NAMES)).expect("web registers");
        db.close().expect("close");
        let db = opened(&rt, pins);
        assert!(matches!(
            db.registered("legacy-1.0"),
            Err(Error::Rejected {
                reason: RejectReason::BundleSuperseded { .. }
            })
        ));
        assert_eq!(Vec::<String>::new(), calls(&db, "shop-1.0"));
    });
}

#[test]
fn an_open_presenting_a_registered_bundle_keeps_its_owner() {
    let mut sim = Sim::new(307, Config::default());
    run(&mut sim, |rt| {
        let db = shop(&rt);
        let pins = &[SHOP, GRANTS];
        let reg = bundle("shop-1.0", pins, NAMES);
        register(&db, "web", &reg).expect("web registers");
        db.close().expect("close");
        let db = open(&rt, pins, vec![reg]).expect("the open presents it");
        assert_eq!(Some(&"web".to_owned()), owners(&db).get("shop-1.0"));
        assert_eq!(Vec::<String>::new(), calls(&db, "shop-1.0"));
        db.close().expect("close");
        let db = opened(&rt, pins);
        assert_eq!(Vec::<String>::new(), calls(&db, "shop-1.0"));
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
