use adtb_simkit::api;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use adtb::runtime::{Clock, Done, Io, Op, Runtime, Threads};
use adtb::{Capability, Database, Error, Log, OpenOptions, Registration, RejectReason, Value};
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
        let got = refused(authority.grant_capability("absent", Capability::Schema, true));
        assert!(got.contains("`absent`"), "{got}");
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
                    reason: RejectReason::Invalid { detail },
                }) => {
                    assert_eq!("the grantee `web` is not a role", detail, "seed={seed}");
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

const SHOP: &str = "module shop
add module shop
add type Customer = { id: Key customers, name: Text }
add relation customers : Customer key id generate sequence
define fn names () -> Set Text = from c in customers select c.name
";

const OPTIONS: &str = "module shop
grant read customers to ann with grant option
grant read customers to bob with grant option
";

const RENAME: &str = "module shop\nrename type Customer to Client\n";

const BY_TEXT: &str = "import shop\nquery q (n: Text) -> Set Text = from c in customers where c.name == n select c.name\nquery r () -> Set Text = from c in customers select c.name\n";

const BY_INT: &str =
    "import shop\nquery q (n: I64) -> Set Text = from c in customers select c.name\n";

const BY_DATE: &str =
    "import shop\nquery q (n: Date) -> Set Text = from c in customers select c.name\n";

fn version_hash(log: &[&str]) -> String {
    let mut source = adtb_lang::evolve::Source::default();
    for text in log {
        let file = adtb_lang::parse::parse_migration(text, adtb_lang::parse::Options::default())
            .ok()
            .expect("the migration parses");
        source = source.migrate(&file, text).expect("the migration applies");
    }
    let schema = source.schema().expect("the source builds");
    adtb::catalog::version_hash(&schema, &source)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn bundle(name: &str, log: &[&str], body: &str) -> Registration {
    let text = format!(
        "bundle {name} pins {} ({}) edition \"2026-10\"\n{body}",
        log.len(),
        version_hash(log)
    );
    Registration::bundle(text).expect("the bundle parses")
}

fn at_log(rt: &SimRuntime, log: &[&str]) -> Database<SimRuntime> {
    Database::open_in(
        rt.clone(),
        "shop.adt",
        &Log::new(log.iter().copied()),
        &options(2),
    )
    .expect("the database opens")
}

/// Returns the database at `[SHOP, OPTIONS]` with the roles `ann`, `bob` and `cid`.
fn shop(rt: &SimRuntime) -> Database<SimRuntime> {
    let db = at_log(rt, &[SHOP]);
    for role in ["ann", "bob", "cid"] {
        db.role_authority().create_role(role).expect("the role");
    }
    db.close().expect("close");
    at_log(rt, &[SHOP, OPTIONS])
}

const AT_2: &[&str] = &[SHOP, OPTIONS];

fn register(db: &Database<SimRuntime>, role: &str, reg: &Registration) {
    let authority = db.role_authority();
    let session = authority.session(role).expect("the session");
    authority.register(&session, reg).expect("it registers");
}

fn grant(db: &Database<SimRuntime>, role: &str, text: &str) -> Result<(), Error> {
    let authority = db.role_authority();
    let session = authority.session(role).expect("the session");
    authority.grant(&session, text)
}

fn revoke(db: &Database<SimRuntime>, role: &str, text: &str) -> Result<(), Error> {
    let authority = db.role_authority();
    let session = authority.session(role).expect("the session");
    authority.revoke(&session, text)
}

fn retire(db: &Database<SimRuntime>, bundle: &str) {
    api::query(db, &format!("retire bundle {bundle}"));
}

/// Returns each call row as `id grantor>grantee option target`.
fn call_rows(db: &Database<SimRuntime>) -> Vec<String> {
    sorted(result(
        db,
        "from g in catalog::grants where g.privilege == \"call\" select { grantor: g.grantor, grantee: g.grantee, option: g.grant_option, target: g.target }",
    ))
}

/// Returns the id and target of the one call row of `grantee`.
fn one_row(db: &Database<SimRuntime>, grantee: &str) -> (String, String) {
    let got = sorted(result(
        db,
        &format!(
            "from g in catalog::grants where g.privilege == \"call\" and g.grantee == \"{grantee}\" select {{ id: g.id, target: g.target }}"
        ),
    ));
    assert_eq!(1, got.len(), "{got:?}");
    let row = &got[0];
    let (id, target) = row.split_once(", target: ").expect("a printed row");
    (id.to_owned(), target.to_owned())
}

fn audits(db: &Database<SimRuntime>) -> usize {
    api::query(db, "from a in catalog::audit select a.seq").len()
}

#[test]
fn call_grants_round_trip_through_their_public_text() {
    once(201, |rt| {
        let db = shop(&rt);
        register(&db, "ann", &bundle("web-1.0", AT_2, BY_TEXT));
        let before = audits(&db);
        grant(&db, "owner", "grant call web-1.0 to bob").unwrap();
        assert_eq!(before + 1, audits(&db), "one audit row");
        revoke(&db, "owner", "revoke call web-1.0 from bob").unwrap();
        grant(&db, "owner", "grant call web-1.0::q to bob").unwrap();
        revoke(&db, "owner", "revoke call web-1.0::q from bob").unwrap();

        grant(&db, "owner", "grant call web::q to bob").unwrap();
        let (_, target) = one_row(&db, "bob");
        assert!(
            target.contains("web::q at 2 (n: Text) -> Set Text"),
            "{target}"
        );
        revoke(&db, "owner", "revoke call web::q from bob").unwrap();

        let signed = "web::q at 2 (n: Text) -> Set Text";
        grant(&db, "owner", &format!("grant call {signed} to bob")).unwrap();
        revoke(&db, "owner", &format!("revoke call {signed} from bob")).unwrap();
        assert_eq!(Vec::<String>::new(), call_rows(&db));
    });
}

#[test]
fn a_nonowner_without_an_option_gets_the_same_denial_for_a_present_and_an_absent_target() {
    once(202, |rt| {
        let db = shop(&rt);
        register(&db, "ann", &bundle("web-1.0", AT_2, BY_TEXT));
        let audited = audits(&db);
        for (present, absent) in [
            ("web-1.0", "nope-1.0"),
            ("web-1.0::q", "nope-1.0::q"),
            ("web::q", "nope::q"),
        ] {
            let detail =
                |target: &str| match grant(&db, "bob", &format!("grant call {target} to cid")) {
                    Err(Error::Rejected {
                        reason: RejectReason::Denied { detail },
                    }) => detail,
                    other => panic!("{target}: {other:?}"),
                };
            assert_eq!(
                detail(present).replace(present, "T"),
                detail(absent).replace(absent, "T")
            );
        }
        assert_eq!(audited, audits(&db));
        assert_eq!(Vec::<String>::new(), call_rows(&db));
    });
}

#[test]
fn the_bundle_owner_the_family_owner_and_an_option_holder_grant() {
    once(203, |rt| {
        let db = shop(&rt);
        register(&db, "ann", &bundle("web-1.0", AT_2, BY_TEXT));
        register(&db, "owner", &bundle("web-1.1", AT_2, BY_TEXT));
        grant(&db, "ann", "grant call web-1.0::q to cid").unwrap();
        denied(grant(&db, "ann", "grant call web-1.1::q to cid"));
        grant(&db, "ann", "grant call web::q to cid").unwrap();

        denied(grant(&db, "bob", "grant call web-1.0::r to cid"));
        grant(&db, "ann", "grant call web-1.0 to bob with grant option").unwrap();
        grant(&db, "bob", "grant call web-1.0::r to cid").unwrap();

        denied(grant(&db, "bob", "grant call web-1.1::q to cid"));
        denied(grant(&db, "bob", "grant call web::q to cid"));
        grant(&db, "ann", "grant call web::q to bob with grant option").unwrap();
        grant(&db, "bob", "grant call web-1.1::q to cid").unwrap();
        grant(&db, "bob", "grant call web::q to cid").unwrap();
        denied(grant(&db, "bob", "grant call web::r to cid"));
    });
}

#[test]
fn an_absent_or_retired_target_is_refused_and_a_superseded_one_allowed() {
    once(204, |rt| {
        let db = shop(&rt);
        let imports = "import shop\nimport fn names\nquery q (n: Text) -> Set Text = from c in customers where c.name == n select c.name\n";
        register(&db, "owner", &bundle("web-1.0", AT_2, imports));
        register(&db, "owner", &bundle("web-1.1", AT_2, imports));
        register(&db, "owner", &bundle("old-1.0", AT_2, imports));
        for (text, names) in [
            ("grant call nope-1.0 to bob", "nope-1.0"),
            ("grant call nope-1.0::q to bob", "nope-1.0"),
            ("grant call web-1.0::absent to bob", "web-1.0::absent"),
            ("grant call web-1.0::names to bob", "grant call fn"),
            ("grant call nope::q to bob", "nope"),
            ("grant call web::names to bob", "web"),
            ("grant call web to bob", "web"),
            ("grant call fn shop::names to bob", "grant call fn"),
        ] {
            let got = refused(grant(&db, "owner", text));
            assert!(got.contains(names), "{text}: {got}");
        }
        retire(&db, "old-1.0");
        let got = refused(grant(&db, "owner", "grant call old-1.0 to bob"));
        assert!(got.contains("retired"), "{got}");
        let got = refused(grant(&db, "owner", "grant read catalog::roles to bob"));
        assert!(got.contains("system relation"), "{got}");
        assert_eq!(Vec::<String>::new(), call_rows(&db));

        api::query(&db, "supersede bundle web-1.0 by web-1.1");
        grant(&db, "owner", "grant call web-1.0 to bob").unwrap();
        grant(&db, "owner", "grant call web-1.0::q to bob").unwrap();
        assert_eq!(2, call_rows(&db).len());
    });
}

#[test]
fn a_family_grant_needs_every_live_bundle_to_share_one_signature() {
    once(205, |rt| {
        let db = shop(&rt);
        register(&db, "ann", &bundle("web-1.0", AT_2, BY_TEXT));
        register(&db, "ann", &bundle("web-2.0", AT_2, BY_INT));
        let got = refused(grant(
            &db,
            "ann",
            "grant call web::q to bob with grant option",
        ));
        assert!(got.contains("web-1.0") && got.contains("web-2.0"), "{got}");
        assert_eq!(Vec::<String>::new(), call_rows(&db));

        retire(&db, "web-1.0");
        grant(&db, "ann", "grant call web::q to bob with grant option").unwrap();
        let (_, target) = one_row(&db, "bob");
        assert!(target.contains("(n: I64)"), "{target}");

        register(&db, "ann", &bundle("web-3.0", AT_2, BY_TEXT));
        register(&db, "ann", &bundle("web-4.0", AT_2, BY_INT));
        denied(grant(&db, "bob", "grant call web-3.0::q to cid"));
        grant(&db, "bob", "grant call web-4.0::q to cid").unwrap();

        let got = refused(grant(
            &db,
            "ann",
            "grant call web::q at 2 (n: Text) -> Set Text to cid",
        ));
        assert!(got.contains("web-3.0") && got.contains("web-4.0"), "{got}");
    });
}

#[test]
fn a_written_signature_must_have_the_ids_the_live_bundles_share() {
    once(206, |rt| {
        let db = shop(&rt);
        register(&db, "ann", &bundle("web-1.0", AT_2, BY_TEXT));
        let got = refused(grant(
            &db,
            "ann",
            "grant call web::q at 2 (n: I64) -> Set Text to bob",
        ));
        assert!(got.contains("web::q at 2 (n: I64)"), "{got}");
        refused(grant(
            &db,
            "ann",
            "grant call web::q at 99 (n: Text) -> Set Text to bob",
        ));
        grant(
            &db,
            "ann",
            "grant call web::q at 1 (other: Text) -> Set Text to bob",
        )
        .unwrap();
        let (_, target) = one_row(&db, "bob");
        assert!(target.contains("at 2 (n: Text)"), "{target}");
    });
}

#[test]
fn a_regrant_of_equal_ids_keeps_the_row_and_of_other_ids_replaces_it() {
    let client: &[&str] = &[SHOP, OPTIONS, RENAME];
    let whole = "import shop\nquery q () -> Set Customer = from c in customers select c\n";
    let renamed = "import shop\nquery q () -> Set Client = from c in customers select c\n";
    let mut sim = Sim::new(207, Config::default());
    run(&mut sim, |rt| {
        let db = shop(&rt);
        register(&db, "ann", &bundle("web-1.0", AT_2, whole));
        grant(&db, "ann", "grant call web::q to bob").unwrap();
        let first = one_row(&db, "bob");
        assert!(
            first.1.contains("at 2 () -> Set shop::Customer"),
            "{first:?}"
        );
        grant(&db, "ann", "grant call web::q to bob").unwrap();
        assert_eq!(first, one_row(&db, "bob"));
        db.close().expect("close");
    });
    run(&mut sim, |rt| {
        let db = at_log(&rt, client);
        register(&db, "ann", &bundle("web-1.1", client, renamed));
        let first = one_row(&db, "bob");
        grant(&db, "ann", "grant call web::q to bob with grant option").unwrap();
        let again = one_row(&db, "bob");
        assert_eq!(first, again, "a renamed type counts as equal");
        grant(&db, "ann", "grant call web::q to bob").unwrap();
        assert!(
            call_rows(&db)[0].contains("option: True"),
            "the option stays"
        );

        revoke(
            &db,
            "ann",
            "revoke call web::q at 3 () -> Set shop::Client from bob",
        )
        .expect("a signed revoke finds the row across the rename");
        assert_eq!(Vec::<String>::new(), call_rows(&db));
    });
}

#[test]
fn a_regrant_of_other_ids_replaces_the_row_unless_it_uncovers_rows_given_under_it() {
    once(208, |rt| {
        let db = shop(&rt);
        register(&db, "ann", &bundle("web-1.0", AT_2, BY_TEXT));
        grant(&db, "ann", "grant call web::q to bob with grant option").unwrap();
        let first = one_row(&db, "bob");
        grant(&db, "bob", "grant call web-1.0::q to cid").unwrap();
        register(&db, "ann", &bundle("web-2.0", AT_2, BY_INT));
        api::query(&db, "supersede bundle web-1.0 by web-2.0");
        let got = refused(grant(&db, "ann", "grant call web::q to bob"));
        assert!(got.contains("web-1.0::q") && got.contains("cid"), "{got}");
        assert_eq!(first, one_row(&db, "bob"), "the refusal changes nothing");

        revoke(&db, "bob", "revoke call web-1.0::q from cid").unwrap();
        grant(&db, "ann", "grant call web::q to bob").unwrap();
        let second = one_row(&db, "bob");
        assert_ne!(first.0, second.0);
        assert!(second.1.contains("(n: I64)"), "{second:?}");
        assert!(
            call_rows(&db)[0].contains("option: True"),
            "a replaced row keeps its option"
        );
    });
}

#[test]
fn a_regrant_closing_a_cycle_is_refused() {
    once(209, |rt| {
        let db = shop(&rt);
        register(&db, "ann", &bundle("web-1.0", AT_2, BY_TEXT));
        grant(&db, "ann", "grant call web-1.0 to bob with grant option").unwrap();
        grant(&db, "bob", "grant call web-1.0 to cid with grant option").unwrap();
        let got = refused(grant(
            &db,
            "cid",
            "grant call web-1.0::q to bob with grant option",
        ));
        assert!(got.contains("cycle"), "{got}");
    });
}

#[test]
fn a_revoke_takes_a_family_option_only_from_rows_of_equal_ids() {
    once(210, |rt| {
        let db = shop(&rt);
        register(&db, "ann", &bundle("web-1.0", AT_2, BY_TEXT));
        grant(&db, "ann", "grant call web::q to bob with grant option").unwrap();
        grant(&db, "bob", "grant call web-1.0::q to cid").unwrap();
        let got = refused(revoke(&db, "ann", "revoke call web::q from bob"));
        assert!(got.contains("web-1.0::q"), "{got}");
        revoke(&db, "bob", "revoke call web-1.0::q from cid").unwrap();

        register(&db, "ann", &bundle("web-2.0", AT_2, BY_INT));
        api::query(&db, "supersede bundle web-2.0 by web-1.0");
        grant(&db, "ann", "grant call web-2.0::q to bob with grant option").unwrap();
        grant(&db, "bob", "grant call web-2.0::q to cid").unwrap();
        let got = refused(revoke(&db, "ann", "revoke call web-2.0::q from bob"));
        assert!(
            got.contains("web-2.0::q"),
            "the family row of another signature keeps nothing: {got}"
        );
        revoke(&db, "ann", "revoke call web::q from bob").expect("another signature");
        assert_eq!(2, call_rows(&db).len());
    });
}

#[test]
fn dropping_a_grantor_is_refused_and_dropping_a_grantee_removes_what_it_held() {
    once(211, |rt| {
        let db = shop(&rt);
        register(&db, "ann", &bundle("web-1.0", AT_2, BY_TEXT));
        let authority = db.role_authority();
        authority.create_role("dee").expect("dee");
        grant(&db, "ann", "grant call web-1.0 to dee with grant option").unwrap();
        grant(&db, "dee", "grant call web-1.0::q to cid").unwrap();
        let rows = call_rows(&db);
        let audited = audits(&db);
        let got = refused(authority.drop_role("dee"));
        assert!(got.contains("web-1.0::q") && got.contains("cid"), "{got}");
        assert_eq!(rows, call_rows(&db));
        assert_eq!(audited, audits(&db));
        let named = "from r in catalog::roles where r.name == \"dee\" select r.id";
        assert_eq!(1, api::query(&db, named).len());

        authority.drop_role("cid").unwrap();
        assert_eq!(1, call_rows(&db).len());
        authority.drop_role("dee").unwrap();
        assert_eq!(Vec::<String>::new(), call_rows(&db));
    });
}

#[test]
fn a_superseding_owner_learns_nothing_of_an_absent_successor() {
    once(212, |rt| {
        let db = shop(&rt);
        register(&db, "ann", &bundle("web-1.0", AT_2, BY_TEXT));
        register(&db, "bob", &bundle("api-1.0", AT_2, BY_TEXT));
        let authority = db.role_authority();
        authority
            .grant_capability("ann", Capability::Adhoc, false)
            .expect("adhoc");
        let ann = authority.session("ann").expect("ann");
        let shown = |by: &str| {
            format!(
                "{:?}",
                api::task(&rt, &ann, &format!("supersede bundle web-1.0 by {by}"))
            )
        };
        let other_family = shown("api-1.0");
        assert!(other_family.contains("Invalid"), "{other_family}");
        assert_eq!(other_family, shown("nope-1.0"));
    });
}

#[test]
fn call_rows_survive_a_reopen_and_a_family_row_reads_at_its_version() {
    let mut sim = Sim::new(213, Config::default());
    let rows = run(&mut sim, |rt| {
        let db = shop(&rt);
        register(&db, "ann", &bundle("web-1.0", AT_2, BY_TEXT));
        grant(&db, "ann", "grant call web::q to bob with grant option").unwrap();
        grant(&db, "ann", "grant call web-1.0::r to cid").unwrap();
        let rows = call_rows(&db);
        db.close().expect("close");
        rows
    });
    run(&mut sim, |rt| {
        let db = at_log(&rt, AT_2);
        assert_eq!(rows, call_rows(&db));
        grant(&db, "bob", "grant call web-1.0::q to cid").expect("the option still covers");
    });
}

#[test]
fn an_unpresented_bundle_with_callables_is_read_from_its_registration() {
    let client: &[&str] = &[SHOP, OPTIONS, RENAME];
    let mut sim = Sim::new(214, Config::default());
    run(&mut sim, |rt| {
        let db = shop(&rt);
        register(&db, "ann", &bundle("web-1.0", AT_2, BY_TEXT));
        db.close().expect("close");
    });
    run(&mut sim, |rt| {
        let db = at_log(&rt, client);
        grant(&db, "ann", "grant call web::q to bob").unwrap();
        let (_, target) = one_row(&db, "bob");
        assert!(target.contains("at 2 (n: Text)"), "{target}");
    });
}

#[test]
fn a_superseded_bundle_unserved_after_a_reopen_is_read_from_its_registration() {
    let mut sim = Sim::new(218, Config::default());
    run(&mut sim, |rt| {
        let db = shop(&rt);
        register(&db, "ann", &bundle("web-1.0", AT_2, BY_TEXT));
        register(&db, "ann", &bundle("web-1.1", AT_2, BY_TEXT));
        grant(&db, "ann", "grant call web::q to bob with grant option").unwrap();
        api::query(&db, "supersede bundle web-1.0 by web-1.1");
        db.close().expect("close");
    });
    run(&mut sim, |rt| {
        let db = at_log(&rt, AT_2);
        grant(&db, "bob", "grant call web-1.0::q to cid").expect("the option covers it");
        let rows = call_rows(&db);
        assert_eq!(2, rows.len(), "{rows:?}");
    });
}

#[test]
fn a_crash_cannot_separate_a_call_grant_from_its_audit_record() {
    let seed = 215;
    let mut sim = Sim::new(seed, Config::default());
    run(&mut sim, |rt| {
        let db = shop(&rt);
        register(&db, "ann", &bundle("web-1.0", AT_2, BY_TEXT));
    });
    let population = sim.files();
    let mut sim = Sim::with_files(seed, Config::default(), population);
    sim.record();
    let start = sim.recording().len();
    let audited = run(&mut sim, |rt| {
        let db = at_log(&rt, AT_2);
        let audited = audits(&db);
        grant(&db, "ann", "grant call web::q to bob").expect("the grant");
        audited
    });
    let recording = sim.recording();
    let end = recording.len();
    let (mut committed, mut absent) = (false, false);
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
            let db = at_log(&rt, AT_2);
            let granted = call_rows(&db).len();
            assert_eq!(
                audited + granted,
                audits(&db),
                "seed={seed} event={event}: the row and its audit go together"
            );
            committed |= granted == 1;
            absent |= granted == 0;
        });
    }
    assert!(committed && absent, "seed={seed}: {committed} {absent}");
}

#[test]
fn a_family_grant_beside_a_registration_records_the_signature_it_saw_or_names_both() {
    for seed in sweep(216..218) {
        let mut sim = Sim::new(
            seed,
            Config {
                switch_rate: 0.3,
                ..Config::default()
            },
        );
        let finished = sim.run(|rt| {
            let db = shop(&rt);
            register(&db, "ann", &bundle("web-1.0", AT_2, BY_TEXT));
            let gate = Arc::new(AtomicUsize::new(0));
            let authority = db.role_authority();
            let ann = authority.session("ann").expect("ann");

            let (a, ready, runtime) = (authority.clone(), gate.clone(), rt.clone());
            let session = ann.clone();
            let reg = bundle("web-2.0", AT_2, BY_INT);
            let registered = Arc::new(Mutex::new(None));
            let out = registered.clone();
            let registering = rt.spawn("register", move || {
                arrive(&runtime, &ready);
                *out.lock().unwrap() = Some(a.register(&session, &reg).map(|_| ()));
            });

            let (a, ready, runtime) = (authority, gate, rt.clone());
            let granted = Arc::new(Mutex::new(None));
            let out = granted.clone();
            let granting = rt.spawn("grant", move || {
                arrive(&runtime, &ready);
                *out.lock().unwrap() = Some(a.grant(&ann, "grant call web::q to bob"));
            });
            rt.join(registering);
            rt.join(granting);

            assert_eq!(Some(Ok(())), *registered.lock().unwrap(), "seed={seed}");
            let granted = granted.lock().unwrap().take().expect("the grant ran");
            match granted {
                Ok(()) => {
                    let (_, target) = one_row(&db, "bob");
                    assert!(target.contains("(n: Text)"), "seed={seed}: {target}");
                }
                Err(e) => {
                    let got = format!("{e:?}");
                    assert!(
                        got.contains("web-1.0") && got.contains("web-2.0"),
                        "seed={seed}: {got}"
                    );
                    assert_eq!(Vec::<String>::new(), call_rows(&db), "seed={seed}");
                }
            }
        });
        assert_eq!(Outcome::Finished, finished, "seed={seed}");
    }
}

#[test]
fn a_plain_regrant_keeping_its_option_cannot_close_a_cycle() {
    once(220, |rt| {
        let db = shop(&rt);
        register(&db, "owner", &bundle("web-1.0", AT_2, BY_TEXT));
        grant(&db, "owner", "grant call web::q to ann with grant option").unwrap();
        grant(&db, "ann", "grant call web::q to bob with grant option").unwrap();
        retire(&db, "web-1.0");
        register(&db, "owner", &bundle("web-2.0", AT_2, BY_INT));
        grant(&db, "owner", "grant call web::q to bob with grant option").unwrap();
        grant(&db, "bob", "grant call web::q to ann with grant option").unwrap();
        let before = call_rows(&db);
        let got = refused(grant(&db, "ann", "grant call web::q to bob"));
        assert!(got.contains("cycle"), "{got}");
        assert_eq!(before, call_rows(&db));
    });
}

#[test]
fn a_whole_bundle_row_neither_blocks_a_family_revoke_nor_closes_a_family_cycle() {
    once(221, |rt| {
        let db = shop(&rt);
        register(&db, "owner", &bundle("web-1.0", AT_2, BY_TEXT));
        register(&db, "ann", &bundle("mine-1.0", AT_2, BY_TEXT));
        grant(&db, "owner", "grant call web::q to ann with grant option").unwrap();
        grant(&db, "ann", "grant call mine-1.0 to cid").unwrap();
        revoke(&db, "owner", "revoke call web::q from ann").unwrap();
        assert_eq!(1, call_rows(&db).len());

        grant(&db, "ann", "grant call mine-1.0 to bob with grant option").unwrap();
        grant(&db, "owner", "grant call web::q to bob with grant option").unwrap();
        grant(&db, "bob", "grant call web::q to ann with grant option").unwrap();
    });
}

#[test]
fn a_family_row_overlaps_a_whole_bundle_of_its_family_declaring_the_query() {
    once(222, |rt| {
        let db = shop(&rt);
        register(&db, "owner", &bundle("web-1.0", AT_2, BY_TEXT));
        grant(&db, "owner", "grant call web::q to bob with grant option").unwrap();
        grant(&db, "bob", "grant call web::q to cid with grant option").unwrap();
        grant(&db, "owner", "grant call web-1.0 to cid with grant option").unwrap();
        let got = refused(grant(
            &db,
            "cid",
            "grant call web-1.0 to bob with grant option",
        ));
        assert!(got.contains("cycle"), "{got}");
    });
    once(223, |rt| {
        let db = shop(&rt);
        register(&db, "owner", &bundle("web-1.0", AT_2, BY_TEXT));
        grant(&db, "owner", "grant call web-1.0 to bob with grant option").unwrap();
        grant(&db, "bob", "grant call web-1.0 to cid with grant option").unwrap();
        grant(&db, "owner", "grant call web::q to cid with grant option").unwrap();
        let got = refused(grant(
            &db,
            "cid",
            "grant call web::q to bob with grant option",
        ));
        assert!(got.contains("cycle"), "{got}");
    });
    once(224, |rt| {
        let db = shop(&rt);
        let other = "import shop\nquery r (n: Text) -> Set Text = from c in customers where c.name == n select c.name\n";
        register(&db, "owner", &bundle("web-1.0", AT_2, BY_TEXT));
        register(&db, "owner", &bundle("web-1.1", AT_2, other));
        grant(&db, "owner", "grant call web::q to bob with grant option").unwrap();
        grant(&db, "bob", "grant call web::q to cid with grant option").unwrap();
        grant(&db, "owner", "grant call web-1.1 to cid with grant option").unwrap();
        grant(&db, "cid", "grant call web-1.1 to bob with grant option").unwrap();
    });
}

/// Assert `text` answers `ann`, holding the family option at another signature, as it answers
/// `cid`, holding nothing.
fn denied_as_without_an_option(db: &Database<SimRuntime>, text: &str) {
    let detail = |role| match grant(db, role, text) {
        Err(Error::Rejected {
            reason: RejectReason::Denied { detail },
        }) => detail,
        other => panic!("{text} as {role}: not Denied: {other:?}"),
    };
    assert_eq!(detail("cid").replace("cid", "ann"), detail("ann"), "{text}");
}

#[test]
fn an_option_at_another_signature_learns_nothing_of_the_live_bundles() {
    once(225, |rt| {
        let db = shop(&rt);
        register(&db, "owner", &bundle("web-1.0", AT_2, BY_TEXT));
        grant(&db, "owner", "grant call web::q to ann with grant option").unwrap();
        retire(&db, "web-1.0");
        denied_as_without_an_option(&db, "grant call web::q to bob");

        register(&db, "owner", &bundle("web-2.0", AT_2, BY_INT));
        denied_as_without_an_option(&db, "grant call web::q to bob");
        denied_as_without_an_option(&db, "grant call web::q at 2 (n: Date) -> Set Text to bob");

        register(&db, "owner", &bundle("web-3.0", AT_2, BY_DATE));
        denied_as_without_an_option(&db, "grant call web::q to bob");

        let got = refused(grant(&db, "owner", "grant call web::q to bob"));
        assert!(got.contains("web-2.0") && got.contains("web-3.0"), "{got}");
    });
}

#[test]
fn a_family_whose_name_reads_as_a_bundle_is_refused_at_first_registration() {
    once(226, |rt| {
        let db = shop(&rt);
        let authority = db.role_authority();
        let session = authority.session("ann").expect("the session");
        let got = refused(authority.register(&session, &bundle("web-2-1.0", AT_2, BY_TEXT)));
        assert!(got.contains("`web-2`"), "{got}");
        assert_eq!(
            0,
            api::query(&db, "from b in catalog::bundles select b.name").len()
        );
        register(&db, "ann", &bundle("web-two-1.0", AT_2, BY_TEXT));
        grant(&db, "ann", "grant call web-two::q to bob").unwrap();
    });
}

#[test]
fn an_absent_grantee_is_invalid_for_a_role_that_may_grant_and_denied_otherwise() {
    once(227, |rt| {
        let db = shop(&rt);
        register(&db, "ann", &bundle("web-1.0", AT_2, BY_TEXT));
        for target in ["web-1.0", "web-1.0::q", "web::q"] {
            let text = format!("grant call {target} to nobody");
            let got = refused(grant(&db, "ann", &text));
            assert!(got.contains("`nobody`"), "{text}: {got}");
            denied(grant(&db, "bob", &text));
        }
        let got = refused(grant(&db, "owner", "grant monitor to nobody"));
        assert!(got.contains("`nobody`"), "{got}");
        denied(grant(&db, "bob", "grant monitor to nobody"));
    });
}

#[test]
fn a_refusal_prints_a_target_as_text() {
    once(228, |rt| {
        let db = shop(&rt);
        register(&db, "owner", &bundle("web-1.0", AT_2, BY_TEXT));
        grant(&db, "owner", "grant call web-1.0 to ann with grant option").unwrap();
        grant(&db, "ann", "grant call web-1.0::q to bob").unwrap();
        let got = refused(revoke(&db, "owner", "revoke call web-1.0 from ann"));
        assert!(
            got.contains("`call` on `web-1.0::q` given by `ann` to `bob`"),
            "{got}"
        );
        grant(&db, "owner", "grant monitor to ann with grant option").unwrap();
        grant(&db, "ann", "grant monitor to bob").unwrap();
        let got = refused(revoke(&db, "owner", "revoke monitor from ann"));
        assert!(got.contains("`monitor` given by `ann` to `bob`"), "{got}");
    });
}

#[test]
fn a_bundle_live_again_with_an_earlier_signature_is_not_covered() {
    once(229, |rt| {
        let db = shop(&rt);
        let first = bundle("web-1.0", AT_2, BY_TEXT);
        register(&db, "owner", &first);
        register(&db, "owner", &bundle("web-2.0", AT_2, BY_INT));
        api::query(&db, "supersede bundle web-1.0 by web-2.0");
        grant(&db, "owner", "grant call web::q to bob with grant option").unwrap();
        grant(&db, "bob", "grant call web-2.0::q to cid").unwrap();
        register(&db, "owner", &first);
        denied(grant(&db, "bob", "grant call web-1.0::q to cid"));
    });
}

#[test]
fn an_expired_bundle_stays_grantable_and_declares_nothing_for_its_family() {
    once(230, |rt| {
        let db = shop(&rt);
        let text = format!(
            "bundle web-1.0 pins 2 ({}) edition \"2026-10\" lease 1d\n{BY_TEXT}",
            version_hash(AT_2)
        );
        let leased = Registration::bundle(text).expect("the bundle parses");
        register(&db, "owner", &leased);
        let mut io = rt.io();
        let at = rt.now() + std::time::Duration::from_secs(3 * 86_400);
        let ticket = io.submit(Op::Timer { at });
        loop {
            if let Some(c) = io.poll() {
                assert_eq!(ticket, c.ticket);
                assert_eq!(Ok(Done::Fired), c.result);
                break;
            }
            rt.park();
        }
        grant(&db, "owner", "grant call web-1.0 to bob").unwrap();
        grant(&db, "owner", "grant call web-1.0::q to bob").unwrap();
        assert_eq!(2, call_rows(&db).len());
        let got = refused(grant(&db, "owner", "grant call web::q to bob"));
        assert!(got.contains("no live bundle"), "{got}");
    });
}
