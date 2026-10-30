//! Registration through a role session: the registering role's grants with grant option, the
//! family it comes to own, the recheck of its bundles at each migration, and their persistence
//! across reopen and crash.

use adtb_simkit::api;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use std::time::Duration;

use adtb::runtime::{Clock, Done, Io, Op, Runtime, Threads};
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

const HOUR: Duration = Duration::from_secs(3600);
const DAY: Duration = Duration::from_secs(86_400);

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

/// Returns the detail of a migration open refuses at `version`.
fn blocked(got: Result<Database<SimRuntime>, OpenError>, version: u64) -> String {
    match got {
        Err(OpenError::MigrationRefused {
            version: v,
            reason:
                Error::Rejected {
                    reason: RejectReason::SchemaRefused { operator, detail },
                },
        }) if v == version => {
            assert_eq!("", operator);
            detail
        }
        Err(e) => panic!("{e:?}"),
        Ok(_) => panic!("the migration applies"),
    }
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

const REVOKE: &str = "module shop
revoke read customers from web
revoke read customers from ops
";

const OPTION_ONLY: &str = "module shop
revoke read customers from web
grant read customers to web
";

/// Returns the database at the shop schema with `shop-1.0` registered by `web` and
/// `admin-1.0` by `ops`, each reading names, closed.
fn deployed(rt: &SimRuntime, clauses: &str) {
    let db = shop(rt);
    let pins = &[SHOP, GRANTS];
    register(&db, "web", &leased("shop-1.0", pins, clauses, NAMES)).expect("web registers");
    register(&db, "ops", &leased("admin-1.0", pins, clauses, NAMES)).expect("ops registers");
    db.close().expect("close");
}

#[test]
fn a_revoke_is_refused_naming_every_live_bundle_whose_owner_used_it_and_changes_nothing() {
    let mut sim = Sim::new(330, Config::default());
    run(&mut sim, |rt| {
        deployed(&rt, "");
        let detail = blocked(open(&rt, &[SHOP, GRANTS, REVOKE], Vec::new()), 3);
        let want: Vec<String> = [("admin-1.0", "ops"), ("shop-1.0", "web")]
            .iter()
            .flat_map(|(b, role)| {
                ["", " (id)", " (name)"].map(|f| {
                    format!(
                        "bundle {b} pins 2: names reads customers.Customer{f}; {role} lacks read with grant option"
                    )
                })
            })
            .collect();
        assert_eq!(want.join("\n"), detail);
        let db = opened(&rt, &[SHOP, GRANTS]);
        assert_eq!(
            vec!["2"],
            query(&db, "count(m for m in catalog::migrations)")
        );
        assert_eq!(Vec::<String>::new(), calls(&db, "shop-1.0"));
    });
}

#[test]
fn removing_only_the_grant_option_blocks_the_bundle_that_relied_on_it() {
    let mut sim = Sim::new(331, Config::default());
    run(&mut sim, |rt| {
        deployed(&rt, "");
        let detail = blocked(open(&rt, &[SHOP, GRANTS, OPTION_ONLY], Vec::new()), 3);
        assert!(detail.starts_with("bundle shop-1.0 pins 2: names reads customers.Customer; web lacks read with grant option"), "{detail}");
        assert!(!detail.contains("admin-1.0"), "{detail}");
    });
}

#[test]
fn retiring_superseding_or_expiring_the_blocking_bundles_lets_the_revoke_apply() {
    let mut sim = Sim::new(332, Config::default());
    run(&mut sim, |rt| {
        deployed(&rt, "");
        let db = opened(&rt, &[SHOP, GRANTS]);
        db.query_text("retire bundle shop-1.0", &[], &CallOptions::default())
            .expect("retire");
        db.close().expect("close");
        let detail = blocked(open(&rt, &[SHOP, GRANTS, REVOKE], Vec::new()), 3);
        assert!(!detail.contains("shop-1.0"), "{detail}");
        assert!(detail.contains("admin-1.0"), "{detail}");
        let db = opened(&rt, &[SHOP, GRANTS]);
        register(
            &db,
            "ops",
            &bundle("admin-1.1", &[SHOP, GRANTS], "import shop\n"),
        )
        .expect("a successor with no reads");
        db.query_text(
            "supersede bundle admin-1.0 by admin-1.1",
            &[],
            &CallOptions::default(),
        )
        .expect("supersede");
        db.close().expect("close");
        opened(&rt, &[SHOP, GRANTS, REVOKE]);
    });
    let mut sim = Sim::new(333, Config::default());
    run(&mut sim, |rt| {
        deployed(&rt, " lease 1d");
        sleep(&rt, DAY + 2 * HOUR);
        opened(&rt, &[SHOP, GRANTS, REVOKE]);
    });
}

#[test]
fn a_bundle_of_the_schema_owner_is_not_blocked_by_a_revoke() {
    let mut sim = Sim::new(334, Config::default());
    run(&mut sim, |rt| {
        let db = shop(&rt);
        register(&db, "owner", &bundle("shop-1.0", &[SHOP, GRANTS], NAMES)).expect("the owner");
        db.close().expect("close");
        let db = opened(&rt, &[SHOP, GRANTS, REVOKE]);
        assert_eq!(Vec::<String>::new(), calls(&db, "shop-1.0"));
    });
}

#[test]
fn a_role_owning_a_family_or_a_live_bundle_cannot_be_dropped() {
    let mut sim = Sim::new(340, Config::default());
    run(&mut sim, |rt| {
        let db = shop(&rt);
        let authority = db.role_authority();
        authority.create_role("spare").expect("spare");
        register(
            &db,
            "idle",
            &bundle("tools-0.1", &[SHOP, GRANTS], "import shop\n"),
        )
        .expect("idle registers");
        assert_eq!(
            "the role owns family tools",
            denied(authority.drop_role("idle"))
        );
        db.query_text("retire bundle tools-0.1", &[], &CallOptions::default())
            .expect("retire");
        assert_eq!(
            "the role owns family tools",
            denied(authority.drop_role("idle"))
        );
        authority
            .drop_role("spare")
            .expect("a role that owns nothing");
        db.close().expect("close");
        let db = opened(&rt, &[SHOP, GRANTS]);
        assert_eq!(
            "the role owns family tools",
            denied(db.role_authority().drop_role("idle"))
        );
    });
}

#[test]
fn the_schema_owner_reregistering_another_roles_bundle_is_checked_against_that_role() {
    let mut sim = Sim::new(350, Config::default());
    run(&mut sim, |rt| {
        deployed(&rt, " lease 1d");
        sleep(&rt, DAY + 2 * HOUR);
        let db = opened(&rt, &[SHOP, GRANTS, REVOKE]);
        let again = leased("shop-1.0", &[SHOP, GRANTS], " lease 1d", NAMES);
        let detail = denied(register(&db, "owner", &again));
        assert!(
            detail.starts_with("names reads customers.Customer; web lacks read with grant option"),
            "{detail}"
        );
        assert_eq!(Some(&"web".to_owned()), owners(&db).get("shop-1.0"));
        assert!(matches!(
            db.registered("shop-1.0"),
            Err(Error::Rejected {
                reason: RejectReason::BundleExpired { .. }
            })
        ));
        db.close().expect("close");
        opened(&rt, &[SHOP, GRANTS, REVOKE]);
    });
}

/// Returns the last renewal `catalog::bundles` records for `name`.
fn renewed(db: &Database<SimRuntime>, name: &str) -> Vec<String> {
    query(
        db,
        &format!("from b in catalog::bundles where b.name == \"{name}\" select b.renewed"),
    )
}

#[test]
fn an_open_never_renews_a_registered_bundle_it_was_not_given() {
    let mut sim = Sim::new(351, Config::default());
    run(&mut sim, |rt| {
        let db = shop(&rt);
        register(
            &db,
            "web",
            &leased("shop-1.0", &[SHOP, GRANTS], " lease 1d", NAMES),
        )
        .expect("web registers");
        let first = renewed(&db, "shop-1.0");
        db.close().expect("close");
        for _ in 0..3 {
            sleep(&rt, 20 * HOUR);
            let db = opened(&rt, &[SHOP, GRANTS]);
            assert_eq!(first, renewed(&db, "shop-1.0"));
            db.close().expect("close");
        }
        let db = opened(&rt, &[SHOP, GRANTS, REVOKE]);
        assert!(matches!(
            db.registered("shop-1.0"),
            Err(Error::Rejected {
                reason: RejectReason::BundleExpired { .. }
            })
        ));
        assert_eq!(first, renewed(&db, "shop-1.0"));
    });
}

const FNS: &str = "module shop
define fn all_names () -> Set Text = from c in customers select c.name
";

#[test]
fn an_imported_schema_function_needs_no_grant_of_the_owner_but_one_a_query_reaches_does() {
    let mut sim = Sim::new(352, Config::default());
    run(&mut sim, |rt| {
        shop(&rt).close().expect("close");
        let pins = &[SHOP, GRANTS, FNS];
        let db = opened(&rt, pins);
        register(
            &db,
            "idle",
            &bundle("fns-1.0", pins, "import shop\nimport fn all_names\n"),
        )
        .expect("an imported function alone");
        let reached = "import shop\nquery everyone () -> Set Text = all_names()\n";
        let detail = denied(register(&db, "idle", &bundle("fns-1.1", pins, reached)));
        assert!(
            detail.starts_with("everyone reads customers.Customer"),
            "{detail}"
        );
        register(
            &db,
            "web",
            &bundle("web-1.0", pins, "import shop\nimport fn all_names\n"),
        )
        .expect("web imports it");
        db.close().expect("close");
        opened(&rt, &[SHOP, GRANTS, FNS, REVOKE]);
    });
}

const RENAME: &str = "module shop\nrename field Customer.name to display\n";

#[test]
fn an_older_pin_is_checked_on_what_its_translation_reads() {
    let mut sim = Sim::new(353, Config::default());
    run(&mut sim, |rt| {
        shop(&rt).close().expect("close");
        let db = opened(&rt, &[SHOP, GRANTS, RENAME]);
        let old = bundle("shop-1.0", &[SHOP, GRANTS], NAMES);
        let detail = denied(register(&db, "app", &old));
        assert_eq!(
            [
                "names reads customers.Customer; app lacks read with grant option",
                "names reads customers.Customer (id); app lacks read with grant option",
                "names reads customers.Customer (display); app lacks read with grant option",
            ]
            .join("\n"),
            detail
        );
        register(&db, "web", &old).expect("the rename carries web's grant");
        assert_eq!(Vec::<String>::new(), calls(&db, "shop-1.0"));
    });
}

#[test]
fn only_its_owner_or_the_schema_owner_registers_a_registered_bundle_again() {
    let mut sim = Sim::new(354, Config::default());
    run(&mut sim, |rt| {
        shop(&rt).close().expect("close");
        let pins = &[SHOP, GRANTS, FNS];
        let db = opened(&rt, pins);
        register(
            &db,
            "idle",
            &bundle("shop-1.0", pins, "import shop\nimport fn all_names\n"),
        )
        .expect("idle claims family shop");
        db.close().expect("close");
        let two = bundle("shop-2.0", pins, NAMES);
        open(&rt, pins, vec![two.clone()])
            .expect("an open registers shop-2.0 as the schema owner")
            .close()
            .expect("close");
        let db = opened(&rt, pins);
        let before = renewed(&db, "shop-2.0");
        assert_eq!(
            "bundle shop-2.0 is owned by role owner",
            denied(register(&db, "idle", &two))
        );
        assert_eq!(before, renewed(&db, "shop-2.0"));
        assert!(matches!(
            db.registered("shop-2.0"),
            Err(Error::Rejected {
                reason: RejectReason::BundleSuperseded { .. }
            })
        ));
        register(&db, "owner", &two).expect("the schema owner registers it again");
        assert_eq!(Vec::<String>::new(), calls(&db, "shop-2.0"));
        let one = bundle("shop-1.0", pins, "import shop\nimport fn all_names\n");
        assert_eq!(
            "bundle shop-1.0 is owned by role idle",
            denied(register(&db, "web", &one))
        );
        register(&db, "idle", &one).expect("its owner registers it again");
    });
}
