//! Registration through a role session: the registering role's grants with grant option, the
//! family it comes to own, the recheck of its bundles at each migration, and their persistence
//! across reopen and crash.

use adtb_simkit::api;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use std::time::Duration;

use adtb::runtime::{Clock, Done, Io, Op, Runtime, Threads};
use adtb::{
    CallOptions, Capability, Database, Error, Log, OpenError, OpenOptions, Registration,
    RejectReason,
};
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
fn only_the_owner_of_a_bundle_or_the_schema_owner_retires_or_supersedes_it_as_a_role() {
    let mut sim = Sim::new(320, Config::default());
    run(&mut sim, |rt| {
        let db = shop(&rt);
        register(&db, "web", &bundle("shop-1.0", &[SHOP, GRANTS], NAMES)).expect("web registers");
        register(&db, "web", &bundle("shop-1.1", &[SHOP, GRANTS], NAMES)).expect("web registers");
        register(&db, "web", &bundle("shop-1.2", &[SHOP, GRANTS], NAMES)).expect("web registers");
        let authority = db.role_authority();
        for role in ["web", "ops"] {
            authority
                .grant_capability(role, Capability::Adhoc, false)
                .expect("adhoc");
        }
        let (web, ops) = (
            authority.session("web").expect("web"),
            authority.session("ops").expect("ops"),
        );
        let audited = || query(&db, "from a in catalog::audit.Denied select a.seq").len();
        denied(api::task(&rt, &ops, "retire bundle shop-1.0"));
        assert_eq!(1, audited());
        denied(api::task(
            &rt,
            &ops,
            "supersede bundle shop-1.0 by shop-1.1",
        ));
        assert_eq!(2, audited());
        api::task(&rt, &web, "supersede bundle shop-1.0 by shop-1.1").expect("the owner");
        api::task(&rt, &web, "retire bundle shop-1.1").expect("the owner");
        let owner = authority.session("owner").expect("owner");
        api::task(&rt, &owner, "retire bundle shop-1.2").expect("the schema owner");
        assert_eq!(2, audited());
    });
}

#[test]
fn an_absent_bundle_is_refused_as_another_role_s_bundle_is_save_for_the_schema_owner() {
    let mut sim = Sim::new(321, Config::default());
    run(&mut sim, |rt| {
        let db = shop(&rt);
        register(&db, "web", &bundle("shop-1.0", &[SHOP, GRANTS], NAMES)).expect("web registers");
        let authority = db.role_authority();
        for role in ["web", "ops"] {
            authority
                .grant_capability(role, Capability::Adhoc, false)
                .expect("adhoc");
        }
        let (web, ops, owner) = (
            authority.session("web").expect("web"),
            authority.session("ops").expect("ops"),
            authority.session("owner").expect("owner"),
        );
        let audited = || query(&db, "from a in catalog::audit.Denied select a.seq").len();
        let held = denied(api::task(&rt, &ops, "retire bundle shop-1.0"));
        assert_eq!(1, audited());
        let absent = denied(api::task(&rt, &ops, "retire bundle nope-1.0"));
        assert_eq!(2, audited());
        assert_eq!(held.replace("shop-1.0", "nope-1.0"), absent);
        denied(api::task(&rt, &web, "retire bundle nope-1.0"));
        assert_eq!(3, audited());
        denied(api::task(
            &rt,
            &ops,
            "supersede bundle nope-1.0 by shop-1.0",
        ));
        assert_eq!(4, audited());
        let got = format!("{:?}", api::task(&rt, &owner, "retire bundle nope-1.0"));
        assert!(got.contains("UnknownBundle"), "{got}");
        assert_eq!(4, audited());
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
fn a_registered_bundle_not_presented_at_open_holds_its_host_code() {
    let host = |op: &str, symbol: &str| {
        format!("{op} fn f (t: Text) -> Text = host \"{symbol}\" version \"1.0\" deterministic")
    };
    let define = format!("module shop\n{}\n", host("define", "a"));
    let redefine = format!("module shop\n{}\n", host("redefine", "b"));
    let mut sim = Sim::new(331, Config::default());
    run(&mut sim, |rt| {
        let db = shop(&rt);
        db.close().expect("close");
        let pins = &[SHOP, GRANTS, define.as_str()];
        let db = opened(&rt, pins);
        register(&db, "web", &bundle("shop-1.0", pins, NAMES)).expect("web registers");
        db.close().expect("close");
        match open(
            &rt,
            &[SHOP, GRANTS, define.as_str(), redefine.as_str()],
            vec![],
        ) {
            Err(OpenError::MigrationRefused {
                version: 4,
                reason:
                    Error::Rejected {
                        reason: RejectReason::SchemaRefused { operator, detail },
                    },
            }) => {
                assert_eq!(host("redefine", "b"), operator);
                assert_eq!(
                    "bundle shop-1.0 pins 3: its version holds another code of host or Wasm function `f`",
                    detail
                );
            }
            other => panic!("the redefine is refused otherwise: {:?}", other.err()),
        }
    });
}

#[test]
fn a_bundle_registered_by_command_at_other_host_code_is_refused() {
    let host = |op: &str, symbol: &str| {
        format!(
            "module shop\n{op} fn f (t: Text) -> Text = host \"{symbol}\" version \"1.0\" deterministic\n"
        )
    };
    let (define, redefine) = (host("define", "a"), host("redefine", "b"));
    let pins = &[SHOP, GRANTS, define.as_str()];
    let mut sim = Sim::new(332, Config::default());
    run(&mut sim, |rt| {
        shop(&rt).close().expect("close");
        let db = opened(&rt, &[SHOP, GRANTS, define.as_str(), redefine.as_str()]);
        match register(&db, "web", &bundle("shop-1.0", pins, NAMES)) {
            Err(Error::Rejected {
                reason: RejectReason::Untranslatable { query, detail },
            }) => {
                assert_eq!("shop-1.0::names", query);
                assert_eq!(
                    "its version holds another code of host or Wasm function `f`",
                    detail
                );
            }
            other => panic!("the registration is refused otherwise: {:?}", other.err()),
        }
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

#[test]
fn a_retire_writes_one_audit_row_naming_the_role_that_retired() {
    let mut sim = Sim::new(201, Config::default());
    run(&mut sim, |rt| {
        let db = shop(&rt);
        let authority = db.role_authority();
        let web = authority.session("web").expect("session");
        register(&db, "web", &bundle("shop-1.0", &[SHOP, GRANTS], NAMES)).expect("web registers");
        register(&db, "web", &bundle("shop-1.1", &[SHOP, GRANTS], NAMES)).expect("web registers");
        authority
            .grant_capability("web", Capability::Adhoc, false)
            .expect("grant");
        let retires = "from a in catalog::audit.Retire select { role: a.role, bundle: a.bundle, who: a.principal }";
        api::task(&rt, &web, "retire bundle shop-1.0").expect("the owner retires");
        let rows = query(&db, retires);
        assert_eq!(1, rows.len(), "{rows:?}");
        assert!(rows[0].contains("shop-1.0"), "{rows:?}");
        assert!(rows[0].contains("role: \"web\""), "{rows:?}");
        assert!(rows[0].contains("Service"), "{rows:?}");
        api::task(&rt, &web, "retire bundle shop-1.0").expect("a second retire changes nothing");
        assert_eq!(1, query(&db, retires).len());
        db.query_text("retire bundle shop-1.1", &[], &CallOptions::default())
            .expect("the in-process owner");
        let rows = query(&db, retires);
        assert_eq!(2, rows.len(), "{rows:?}");
        assert!(
            rows.iter().any(|r| r.contains("role: \"owner\"")),
            "{rows:?}"
        );
    });
}

#[test]
fn a_bundle_owner_holding_monitor_registers_a_read_of_a_system_relation() {
    let mut sim = Sim::new(390, Config::default());
    run(&mut sim, |rt| {
        let db = shop(&rt);
        let body =
            "import shop\nquery files () -> Set Text = from f in sys::format select f.file\n";
        let reg = bundle("mon-1.0", &[SHOP, GRANTS], body);
        denied(register(&db, "idle", &reg));
        db.role_authority()
            .grant_capability("idle", Capability::Monitor, false)
            .expect("monitor");
        register(&db, "idle", &reg).expect("monitor stands for the read");
    });
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
