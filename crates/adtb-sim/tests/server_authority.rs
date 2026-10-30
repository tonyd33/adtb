use adtb_simkit::api;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use adtb::runtime::Threads;
use adtb::{Capability, Database, Error, Log, OpenOptions, RejectReason, Value};
use adtb_sim::{Config, Outcome, Sim, SimRuntime};
use adtb_testkit::profile::{count, sweep};

use api::{once, options, result, run, sorted};

fn opened(rt: &SimRuntime) -> Database<SimRuntime> {
    Database::open_in(rt.clone(), "roles.adt", &Log::default(), &options(2))
        .expect("database opens")
}

const SCHEMA: &str = "module example\nadd module example\nadd type Item = { id: Key items, value: I64 }\nadd relation items : Item key id\n";

fn opened_schema(rt: &SimRuntime) -> Database<SimRuntime> {
    Database::open_in(rt.clone(), "roles.adt", &Log::new([SCHEMA]), &options(2))
        .expect("schema database opens")
}

fn denied<T: std::fmt::Debug>(got: Result<T, Error>) {
    assert!(
        matches!(
            got,
            Err(Error::Rejected {
                reason: RejectReason::Denied { .. }
            })
        ),
        "{got:?}"
    );
}

fn ids(db: &Database<SimRuntime>) -> Vec<Value> {
    result(
        db,
        "from r in catalog::roles where r.name == \"web\" select key r",
    )
    .collect::<Result<_, _>>()
    .expect("role ids read")
}

#[test]
fn the_embedded_owner_is_bootstrapped_once_and_has_every_capability() {
    let mut sim = Sim::new(101, Config::default());
    let before = run(&mut sim, |rt| {
        let db = opened(&rt);
        let authority = db.role_authority();
        let owner = authority.session("owner").expect("owner session");
        for capability in [
            Capability::Adhoc,
            Capability::Assert,
            Capability::Schema,
            Capability::Monitor,
            Capability::Cancel,
            Capability::Maintain,
            Capability::Configure,
            Capability::ReadFiles,
            Capability::WriteFiles,
        ] {
            authority
                .require(&owner, capability, true)
                .expect("owner is authorized");
        }
        let role = sorted(result(&db, "from r in catalog::roles select r"));
        assert_eq!(role.len(), 1);
        db.close().expect("close");
        role
    });
    run(&mut sim, |rt| {
        let db = opened(&rt);
        assert_eq!(
            before,
            sorted(result(&db, "from r in catalog::roles select r"))
        );
    });
}

#[test]
fn capabilities_and_grant_options_persist_without_changing_the_schema() {
    let mut sim = Sim::new(102, Config::default());
    run(&mut sim, |rt| {
        let db = opened_schema(&rt);
        let before = sorted(result(&db, "from m in catalog::migrations select m"));
        assert_eq!(1, before.len(), "the fixture has an actual schema hash");
        let authority = db.role_authority();
        authority.create_role("web").expect("create web");
        let session = authority.session("web").expect("web session");
        denied(authority.require(&session, Capability::Assert, false));
        authority
            .grant_capability("web", Capability::Assert, false)
            .expect("grant assert");
        authority
            .require(&session, Capability::Assert, false)
            .expect("assert authorized");
        denied(authority.require(&session, Capability::Assert, true));
        authority
            .grant_capability("web", Capability::Assert, true)
            .expect("grant option");
        authority
            .grant_capability("web", Capability::Assert, false)
            .expect("retain grant option");
        authority
            .require(&session, Capability::Assert, true)
            .expect("grant option retained");
        denied(authority.require(&session, Capability::Schema, false));
        assert_eq!(
            vec!["1"],
            sorted(result(&db, "count(g for g in catalog::grants)"))
        );
        assert_eq!(
            before,
            sorted(result(&db, "from m in catalog::migrations select m"))
        );
        assert_eq!(
            vec!["4"],
            sorted(result(&db, "count(a for a in catalog::audit)"))
        );
        let mut statements: Vec<_> = result(
            &db,
            "from a in catalog::audit.Grant select { seq: key a, statement: a.statement }",
        )
        .map(|row| match row.expect("audit row") {
            Value::Record(fields) => match fields.as_ref() {
                [Value::Int(_), Value::Text(statement)] => statement.to_string(),
                other => panic!("an audit observation has seq and statement: {other:?}"),
            },
            other => panic!("an audit observation is a record: {other:?}"),
        })
        .collect();
        statements.sort_unstable();
        assert_eq!(
            vec![
                "create role web",
                "grant assert to web",
                "grant assert to web",
                "grant assert to web with grant option",
            ],
            statements
        );
        assert_eq!(
            vec!["\"owner\""],
            sorted(result(&db, "from a in catalog::audit select a.role"))
        );
        db.close().expect("close");
    });
    run(&mut sim, |rt| {
        let db = opened_schema(&rt);
        let authority = db.role_authority();
        let session = authority.session("web").expect("persisted web session");
        authority
            .require(&session, Capability::Assert, true)
            .expect("persisted grant option");
        authority
            .revoke_capability("web", Capability::Assert)
            .expect("revoke assert");
        denied(authority.require(&session, Capability::Assert, false));
        assert_eq!(
            vec!["0"],
            sorted(result(&db, "count(g for g in catalog::grants)"))
        );
    });
}

#[test]
fn stale_and_foreign_sessions_cannot_gain_recreated_role_authority() {
    let mut sim = Sim::new(103, Config::default());
    let old_id = run(&mut sim, |rt| {
        let db = opened(&rt);
        let authority = db.role_authority();
        authority.create_role("web").expect("create web");
        authority
            .grant_capability("web", Capability::Assert, false)
            .expect("grant assert");
        let session = authority.session("web").expect("web session");
        let old = ids(&db);
        let other = Database::open_in(rt, "other.adt", &Log::default(), &options(2))
            .expect("other database");
        denied(
            other
                .role_authority()
                .require(&session, Capability::Assert, false),
        );
        authority.drop_role("web").expect("drop web");
        denied(authority.require(&session, Capability::Assert, false));
        authority.create_role("web").expect("recreate web");
        authority
            .grant_capability("web", Capability::Assert, false)
            .expect("grant new web");
        denied(authority.require(&session, Capability::Assert, false));
        assert!(ids(&db) > old, "new role id exceeds old identity");
        let old = ids_value(&ids(&db));
        authority.drop_role("web").expect("drop recreated web");
        db.close().expect("close");
        assert_eq!(
            Err(Error::Rejected {
                reason: RejectReason::Closed
            }),
            authority.require(&session, Capability::Assert, false)
        );
        old
    });
    run(&mut sim, |rt| {
        let db = opened(&rt);
        let authority = db.role_authority();
        authority.create_role("web").expect("create after restart");
        assert!(
            ids_value(&ids(&db)) > old_id,
            "restart does not reuse role identity"
        );
        assert_eq!(
            vec!["0"],
            sorted(result(&db, "count(g for g in catalog::grants)"))
        );
    });
}

fn ids_value(ids: &[Value]) -> i64 {
    match ids {
        [Value::Int(id)] => *id,
        _ => panic!("one role id: {ids:?}"),
    }
}

#[test]
fn principals_are_derived_from_the_role_and_assertions_need_the_current_grant() {
    once(104, |rt| {
        let db = opened(&rt);
        let authority = db.role_authority();
        authority.create_role("web").expect("create web");
        let session = authority.session("web").expect("web session");
        let principal = db.schema().ty(db.schema().principal);
        let service = Value::ctor(
            principal.ctor_index("Service").unwrap(),
            vec![Value::text("web")],
        );
        let anonymous = Value::ctor(principal.ctor_index("Anonymous").unwrap(), Vec::new());
        let bad = Value::ctor(
            principal.ctor_index("EndUser").unwrap(),
            vec![Value::Int(7)],
        );
        assert_eq!(
            service,
            authority
                .principal(&session, None)
                .expect("service derived")
        );
        denied(authority.principal(&session, Some(&anonymous)));
        authority
            .grant_capability("web", Capability::Assert, false)
            .expect("grant assert");
        assert_eq!(
            anonymous,
            authority
                .principal(&session, Some(&anonymous))
                .expect("assert anonymous")
        );
        denied(authority.principal(&session, Some(&service)));
        assert!(matches!(
            authority.principal(&session, Some(&bad)),
            Err(Error::Rejected {
                reason: RejectReason::Invalid { .. }
            })
        ));
        authority
            .revoke_capability("web", Capability::Assert)
            .expect("revoke assert");
        denied(authority.principal(&session, Some(&anonymous)));
    });
}

#[test]
fn refused_mutations_leave_roles_grants_and_audit_unchanged() {
    once(105, |rt| {
        let db = opened(&rt);
        let authority = db.role_authority();
        authority.create_role("web").expect("create web");
        let before = sorted(result(&db, "from a in catalog::audit select a"));
        denied(authority.drop_role("owner"));
        denied(authority.grant_capability("absent", Capability::Schema, true));
        assert!(matches!(
            authority.create_role("web"),
            Err(Error::Rejected {
                reason: RejectReason::Invalid { .. }
            })
        ));
        for name in [
            "",
            "two words",
            "web\ncreate role intruder",
            "schema",
            "Owner",
        ] {
            assert!(
                matches!(
                    authority.create_role(name),
                    Err(Error::Rejected {
                        reason: RejectReason::Invalid { .. }
                    })
                ),
                "name={name:?}"
            );
        }
        assert_eq!(
            before,
            sorted(result(&db, "from a in catalog::audit select a"))
        );
        assert_eq!(
            vec!["0"],
            sorted(result(&db, "count(g for g in catalog::grants)"))
        );
        assert_eq!(
            vec!["\"owner\"", "\"web\""],
            sorted(result(&db, "from r in catalog::roles select r.name"))
        );
    });
}

#[test]
fn operator_log_mentions_prevent_dropping_the_role_even_after_revoke() {
    once(106, |rt| {
        let grant = "module example\ngrant read items to web\n";
        let revoke = "module example\nrevoke read items from web\n";
        let db = opened_schema(&rt);
        db.role_authority()
            .create_role("web")
            .expect("create role the log names");
        db.close().expect("close");
        let db = Database::open_in(
            rt,
            "roles.adt",
            &Log::new([SCHEMA, grant, revoke]),
            &options(2),
        )
        .expect("database opens with historical grants");
        denied(db.role_authority().drop_role("web"));
    });
}

#[test]
fn concurrent_capability_mutations_and_role_replacement_cannot_leave_authority_behind() {
    for seed in sweep(107..109) {
        let mut sim = Sim::new(
            seed,
            Config {
                switch_rate: 0.3,
                ..Config::default()
            },
        );
        let result = sim.run(|rt| {
            let db = opened(&rt);
            let authority = db.role_authority();
            authority.create_role("web").expect("create web");
            let session = authority.session("web").expect("web session");
            let outcomes = Arc::new(Mutex::new(Vec::new()));
            let gate = Arc::new(AtomicUsize::new(0));
            let grant_authority = authority.clone();
            let done = outcomes.clone();
            let ready = gate.clone();
            let runtime = rt.clone();
            let a = rt.spawn("grant", move || {
                arrive(&runtime, &ready);
                let outcome = grant_authority.grant_capability("web", Capability::Schema, false);
                done.lock().unwrap().push(("grant", outcome));
            });
            let drop_authority = authority.clone();
            let done = outcomes.clone();
            let ready = gate.clone();
            let runtime = rt.clone();
            let b = rt.spawn("drop", move || {
                arrive(&runtime, &ready);
                let outcome = drop_authority.drop_role("web");
                done.lock().unwrap().push(("drop", outcome));
            });
            rt.join(a);
            rt.join(b);
            let outcomes = outcomes.lock().unwrap().clone();
            assert_eq!(2, outcomes.len(), "seed={seed}");
            let drop = &outcomes
                .iter()
                .find(|(actor, _)| *actor == "drop")
                .unwrap()
                .1;
            assert_eq!(&Ok(()), drop, "seed={seed}: {outcomes:?}");
            let grant = &outcomes
                .iter()
                .find(|(actor, _)| *actor == "grant")
                .unwrap()
                .1;
            match grant {
                Ok(()) => {}
                Err(Error::Rejected {
                    reason: RejectReason::Denied { detail },
                }) => {
                    assert_eq!(
                        "the authenticated role no longer exists", detail,
                        "seed={seed}"
                    );
                }
                other => panic!("seed={seed}: unexpected grant outcome {other:?}"),
            }
            assert_eq!(
                2,
                gate.load(Ordering::SeqCst),
                "seed={seed}: actors both reached the gate"
            );
            denied(authority.require(&session, Capability::Schema, false));
            assert_eq!(
                vec!["0"],
                sorted(result(&db, "count(g for g in catalog::grants)")),
                "seed={seed}"
            );
        });
        assert_eq!(Outcome::Finished, result, "seed={seed}");
    }
}

fn arrive(rt: &SimRuntime, gate: &AtomicUsize) {
    gate.fetch_add(1, Ordering::SeqCst);
    while gate.load(Ordering::SeqCst) < 2 {
        rt.yield_now();
    }
}

#[test]
fn a_crash_cannot_separate_a_capability_mutation_from_its_audit_record() {
    let seed = 109;
    let mut sim = Sim::new(seed, Config::default());
    sim.record();
    run(&mut sim, |rt| {
        let db = opened(&rt);
        db.role_authority().create_role("web").expect("create web");
    });
    let population = sim.files();
    let mut sim = Sim::with_files(seed, Config::default(), population);
    sim.record();
    let start = sim.recording().len();
    run(&mut sim, |rt| {
        let db = opened(&rt);
        db.role_authority()
            .grant_capability("web", Capability::Assert, true)
            .expect("grant assert");
    });
    let recording = sim.recording();
    let end = recording.len();
    let mut committed = false;
    let mut absent = false;
    let samples = usize::try_from(count(10)).expect("the crash sample count fits usize");
    let mut events: Vec<_> = (start..=end)
        .step_by((end - start).div_ceil(samples).max(1))
        .collect();
    if events.last() != Some(&end) {
        events.push(end);
    }
    for event in events {
        let files = recording
            .images(event, 1, seed + event as u64)
            .swap_remove(0);
        let mut recovered = Sim::with_files(seed + event as u64, Config::default(), files);
        run(&mut recovered, |rt| {
            let db = opened(&rt);
            let grants = sorted(result(&db, "count(g for g in catalog::grants)"));
            let audit = sorted(result(&db, "count(a for a in catalog::audit)"));
            assert!(
                grants == ["0"] || grants == ["1"],
                "seed={seed} event={event}: {grants:?}"
            );
            assert_eq!(
                if grants == ["0"] {
                    vec!["1"]
                } else {
                    vec!["2"]
                },
                audit,
                "seed={seed} event={event}"
            );
            committed |= grants == ["1"];
            absent |= grants == ["0"];
        });
    }
    assert!(
        committed,
        "seed={seed}: crash images include a committed grant"
    );
    assert!(
        absent,
        "seed={seed}: crash images include an uncommitted grant"
    );
}

#[test]
fn opening_an_empty_database_needs_no_statement_memory() {
    once(113, |rt| {
        let db = Database::open_in(
            rt,
            "tiny.adt",
            &Log::default(),
            &OpenOptions {
                query_memory_bytes: Some(0),
                ..options(2)
            },
        )
        .expect("native owner initialization needs no statement memory");
        let owner = db.role_authority().session("owner").expect("owner exists");
        assert_eq!("owner", owner.role_name());
    });
}

#[test]
fn asserted_principals_use_the_shared_value_domain_validation() {
    once(109, |rt| {
        let log = Log::new([
            "module example\nadd module example\nadd type Principal = EndUser { level: I8 } | Service { name: Text } | Anonymous\n",
        ]);
        let db = Database::open_in(rt, "roles.adt", &log, &options(2)).unwrap();
        let authority = db.role_authority();
        authority.create_role("web").unwrap();
        authority
            .grant_capability("web", Capability::Assert, false)
            .unwrap();
        let session = authority.session("web").unwrap();
        let ctor = db
            .schema()
            .ty(db.schema().principal)
            .ctor_index("EndUser")
            .unwrap();
        let valid = Value::ctor(ctor, vec![Value::Int(127)]);
        assert_eq!(valid, authority.principal(&session, Some(&valid)).unwrap());
        for fields in [
            vec![Value::Int(256)],
            vec![Value::Int(1), Value::Int(2)],
            Vec::new(),
        ] {
            let invalid = Value::ctor(ctor, fields);
            assert!(matches!(
                authority.principal(&session, Some(&invalid)),
                Err(Error::Rejected {
                    reason: RejectReason::Invalid { .. }
                })
            ));
        }
    });
}

fn refused<T: std::fmt::Debug>(got: Result<T, Error>) -> String {
    match got {
        Err(Error::Rejected {
            reason: RejectReason::Invalid { detail },
        }) => detail,
        other => panic!("not Invalid: {other:?}"),
    }
}

/// Returns `grantee grantor grant_option` of each grant row of `monitor`.
fn monitors(db: &Database<SimRuntime>) -> Vec<String> {
    sorted(result(
        db,
        "from g in catalog::grants where g.privilege == \"monitor\"
         select g.grantee ++ \" \" ++ g.grantor ++ \" \" ++ print(g.grant_option)",
    ))
}

#[test]
fn a_revoke_deletes_the_issuers_rows_and_the_schema_owners_deletes_every_grantors() {
    once(201, |rt| {
        let db = opened(&rt);
        let authority = db.role_authority();
        for role in ["ann", "bob", "cal"] {
            authority.create_role(role).expect("create");
        }
        authority
            .grant_capability("ann", Capability::Schema, false)
            .expect("ann");
        let ann = authority.session("ann").expect("ann session");
        let cal = authority.session("cal").expect("cal session");
        let owner = authority.session("owner").expect("owner session");
        authority
            .grant_capability("bob", Capability::Monitor, false)
            .expect("owner gives");
        authority
            .grant_capability_as(&ann, "bob", Capability::Monitor, false)
            .expect("ann gives");
        assert_eq!(
            vec![r#""bob ann False""#, r#""bob owner False""#],
            monitors(&db),
            "one row per grantor"
        );
        let detail = refused(authority.revoke(&cal, "revoke monitor from bob"));
        assert!(detail.contains("revoke monitor from bob"), "{detail}");
        authority
            .revoke(&ann, "revoke monitor from bob")
            .expect("ann revokes its own");
        assert_eq!(vec![r#""bob owner False""#], monitors(&db));
        let detail = refused(authority.revoke(&ann, "revoke monitor from bob"));
        assert!(detail.contains("revoke monitor from bob"), "{detail}");
        authority
            .grant_capability_as(&ann, "bob", Capability::Monitor, false)
            .expect("ann gives again");
        authority
            .revoke(&owner, "revoke monitor from bob")
            .expect("the owner revokes all");
        assert_eq!(Vec::<String>::new(), monitors(&db));
    });
}

#[test]
fn only_a_schema_holder_or_an_option_holder_gives_a_capability() {
    once(202, |rt| {
        let db = opened(&rt);
        let authority = db.role_authority();
        authority.create_role("ann").expect("create");
        authority.create_role("bob").expect("create");
        let ann = authority.session("ann").expect("session");
        denied(authority.grant_capability_as(&ann, "bob", Capability::Monitor, false));
        authority
            .grant_capability("ann", Capability::Monitor, false)
            .expect("no option");
        denied(authority.grant_capability_as(&ann, "bob", Capability::Monitor, false));
        authority
            .grant_capability("ann", Capability::Monitor, true)
            .expect("option");
        authority
            .grant_capability_as(&ann, "bob", Capability::Monitor, false)
            .expect("ann grants on");
    });
}

#[test]
fn a_revoke_that_orphans_rows_granted_under_the_option_is_refused_naming_them() {
    once(203, |rt| {
        let db = opened(&rt);
        let authority = db.role_authority();
        authority.create_role("ann").expect("create");
        authority.create_role("bob").expect("create");
        authority
            .grant_capability("ann", Capability::Monitor, true)
            .expect("option");
        let ann = authority.session("ann").expect("session");
        let owner = authority.session("owner").expect("session");
        authority
            .grant_capability_as(&ann, "bob", Capability::Monitor, false)
            .expect("ann grants on");
        let before = monitors(&db);
        let detail = refused(authority.revoke(&owner, "revoke monitor from ann"));
        assert!(detail.contains("`monitor`"), "{detail}");
        assert!(detail.contains("`ann` to `bob`"), "{detail}");
        assert_eq!(before, monitors(&db), "a refused revoke changes nothing");
        refused(authority.revoke_capability("ann", Capability::Monitor));
        authority
            .revoke(&ann, "revoke monitor from bob")
            .expect("ann revokes bob");
        authority
            .revoke(&owner, "revoke monitor from ann")
            .expect("nothing relies on the option now");
        assert_eq!(Vec::<String>::new(), monitors(&db));
    });
}

#[test]
fn a_revoke_of_several_privileges_changes_nothing_when_one_matches_no_row() {
    once(204, |rt| {
        let db = opened(&rt);
        let authority = db.role_authority();
        authority.create_role("bob").expect("create");
        authority
            .grant_capability("bob", Capability::Monitor, false)
            .expect("grant");
        let owner = authority.session("owner").expect("session");
        let detail = refused(authority.revoke(&owner, "revoke monitor, cancel from bob"));
        assert!(
            detail.contains("revoke monitor, cancel from bob"),
            "{detail}"
        );
        assert_eq!(vec![r#""bob owner False""#], monitors(&db));
    });
}

#[test]
fn an_operator_grant_named_as_a_deployment_revoke_is_refused() {
    once(205, |rt| {
        let db = opened(&rt);
        let authority = db.role_authority();
        let owner = authority.session("owner").expect("session");
        let detail = refused(authority.revoke(&owner, "revoke call fn f from bob"));
        assert!(detail.contains("schema operator"), "{detail}");
        let detail = refused(authority.revoke(&owner, "select 1"));
        assert!(
            detail.contains("expected") || detail.contains("not one revoke"),
            "{detail}"
        );
    });
}

#[test]
fn a_revoke_leaving_another_option_keeps_the_rows_it_covers() {
    once(206, |rt| {
        let db = opened(&rt);
        let authority = db.role_authority();
        for role in ["a", "b", "x"] {
            authority.create_role(role).expect("create");
        }
        authority
            .grant_capability("x", Capability::Schema, false)
            .expect("x");
        let x = authority.session("x").expect("x session");
        let a = authority.session("a").expect("a session");
        authority
            .grant_capability("a", Capability::Monitor, true)
            .expect("owner gives");
        authority
            .grant_capability_as(&x, "a", Capability::Monitor, true)
            .expect("x gives");
        authority
            .grant_capability_as(&a, "b", Capability::Monitor, false)
            .expect("a gives");
        authority
            .revoke(&x, "revoke monitor from a")
            .expect("the owner's option remains");
        assert_eq!(vec![r#""a owner True""#, r#""b a False""#], monitors(&db));
    });
}

#[test]
fn a_row_held_without_an_option_supports_nothing_a_revoke_could_orphan() {
    once(207, |rt| {
        let db = opened(&rt);
        let authority = db.role_authority();
        authority.create_role("a").expect("create");
        authority.create_role("b").expect("create");
        authority
            .grant_capability("a", Capability::Schema, false)
            .expect("schema");
        authority
            .grant_capability("a", Capability::Monitor, false)
            .expect("monitor");
        let a = authority.session("a").expect("session");
        let owner = authority.session("owner").expect("session");
        authority
            .grant_capability_as(&a, "b", Capability::Monitor, false)
            .expect("a gives under schema");
        authority
            .revoke(&owner, "revoke monitor from a")
            .expect("a held no option");
        assert_eq!(vec![r#""b a False""#], monitors(&db));
    });
}

#[test]
fn a_grant_closing_a_cycle_of_options_is_refused_and_leaves_revoke_cleanly() {
    once(208, |rt| {
        let db = opened(&rt);
        let authority = db.role_authority();
        for role in ["ann", "bob", "x"] {
            authority.create_role(role).expect("create");
        }
        let (ann, bob) = (
            authority.session("ann").expect("ann"),
            authority.session("bob").expect("bob"),
        );
        let owner = authority.session("owner").expect("owner");
        authority
            .grant_capability("ann", Capability::Monitor, true)
            .expect("owner gives ann");
        authority
            .grant_capability_as(&ann, "bob", Capability::Monitor, true)
            .expect("ann gives bob");
        let detail = refused(authority.grant_capability_as(&bob, "ann", Capability::Monitor, true));
        assert!(detail.contains("cycle"), "{detail}");
        refused(authority.grant_capability_as(&ann, "ann", Capability::Monitor, true));
        authority
            .grant_capability_as(&bob, "ann", Capability::Monitor, false)
            .expect("no option closes no cycle");
        authority
            .revoke(&bob, "revoke monitor from ann")
            .expect("bob revokes its row");
        authority
            .revoke(&ann, "revoke monitor from bob")
            .expect("the leaf goes");
        authority
            .revoke(&owner, "revoke monitor from ann")
            .expect("then its grantor");
        assert_eq!(Vec::<String>::new(), monitors(&db));

        authority
            .grant_capability("x", Capability::Schema, false)
            .expect("x");
        let x = authority.session("x").expect("x session");
        authority
            .grant_capability_as(&x, "ann", Capability::Monitor, true)
            .expect("x gives a");
        authority
            .grant_capability_as(&ann, "bob", Capability::Monitor, true)
            .expect("a gives b");
        refused(authority.grant_capability_as(&bob, "ann", Capability::Monitor, true));
        refused(authority.revoke(&x, "revoke monitor from ann"));
        authority
            .revoke(&ann, "revoke monitor from bob")
            .expect("the leaf goes");
        authority
            .revoke(&x, "revoke monitor from ann")
            .expect("then its grantor");
        assert_eq!(Vec::<String>::new(), monitors(&db));
    });
}

#[test]
fn a_stale_session_neither_grants_nor_revokes() {
    once(209, |rt| {
        let db = opened(&rt);
        let authority = db.role_authority();
        authority.create_role("web").expect("create");
        authority.create_role("bob").expect("create");
        let stale = authority.session("web").expect("session");
        authority.drop_role("web").expect("drop");
        authority.create_role("web").expect("recreate");
        authority
            .grant_capability("web", Capability::Schema, false)
            .expect("schema");
        let web = authority.session("web").expect("session");
        denied(authority.grant_capability_as(&stale, "bob", Capability::Monitor, false));
        authority
            .grant_capability_as(&web, "bob", Capability::Monitor, false)
            .expect("the new role gives");
        denied(authority.revoke(&stale, "revoke monitor from bob"));
        assert_eq!(vec![r#""bob web False""#], monitors(&db));
    });
}

#[test]
fn a_session_of_another_database_neither_grants_nor_revokes() {
    once(210, |rt| {
        let db = opened(&rt);
        let other = Database::open_in(rt, "other.adt", &Log::default(), &options(2))
            .expect("the other database opens");
        let authority = db.role_authority();
        let foreign = other.role_authority().session("owner").expect("session");
        authority.create_role("bob").expect("create");
        denied(authority.grant_capability_as(&foreign, "bob", Capability::Monitor, false));
        authority
            .grant_capability("bob", Capability::Monitor, false)
            .expect("grant");
        denied(authority.revoke(&foreign, "revoke monitor from bob"));
        assert_eq!(vec![r#""bob owner False""#], monitors(&db));
    });
}
