//! The capability each operational command needs when a role session issues it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Wake, Waker};
use std::time::Duration;

use adtb::batch::{self, Batch, BatchBuilder, Type};
use adtb::catalog::version_hash;
use adtb::key::Scalar;
use adtb::runtime::{Clock, Io, Op, Runtime, ThreadId, Threads};
use adtb::storage::Quarantine;
use adtb::{
    CallOptions, Capability, Completion, Database, Determinism, Error, HostFns, Log, Observation,
    OpenOptions, Registration, RejectReason, RetryReason, RoleSession, Task, TaskOptions,
    TxnCallOptions, TxnOptions, Value,
};
use adtb_lang::evolve::Source;
use adtb_lang::parse::{Options as ParseOptions, parse_migration};
use adtb_lang::types::{Base, Ty};
use adtb_sim::{Config, Sim, SimRuntime};
use adtb_simkit::api::{once, options, query, results, run, task};

const SCHEMA: &str = "module example\nadd module example\nadd type Item = { id: Key items, value: I64 }\nadd relation items : Item key id\n";

fn opened_schema(rt: &SimRuntime) -> Database<SimRuntime> {
    Database::open_in(rt.clone(), "commands.adt", &Log::new([SCHEMA]), &options(2))
        .expect("schema database opens")
}

fn denied(got: Result<(), Error>) {
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

const CATALOG_SPEC: &str = include_str!("../../../docs/spec/catalog.md");

/// A row of the command table, by its first column: a statement for each form it runs in today
/// and the capabilities that decide it, or the statements of a form the engine does not run.
struct Row {
    spec: &'static str,
    capability: &'static str,
    forms: &'static [(&'static str, &'static [Capability])],
    unsupported: &'static [&'static str],
}

const COMMANDS: &[Row] = &[
    Row {
        spec: "`explain q`, `explain analyze s`",
        capability: "as for the statement",
        forms: &[],
        unsupported: &[],
    },
    Row {
        spec: "`explain [analyze] B::q(args)`",
        capability: "`call` on the query, and `monitor`",
        forms: &[],
        unsupported: &[],
    },
    Row {
        spec: "`pin plan p`, `unpin plan p`",
        capability: "`maintain`",
        forms: &[],
        unsupported: &[
            "pin plan catalog::plans.key(1)",
            "unpin plan catalog::plans.key(1)",
        ],
    },
    Row {
        spec: "`cancel call c`",
        capability: "own calls, or `cancel`",
        forms: &[],
        unsupported: &["cancel call sys::calls.key(1)"],
    },
    Row {
        spec: "`terminate session s`",
        capability: "`cancel`",
        forms: &[],
        unsupported: &["terminate session sys::sessions.key(1)"],
    },
    Row {
        spec: "`vacuum r`",
        capability: "`maintain`",
        forms: &[("vacuum items", &[Capability::Maintain])],
        unsupported: &[],
    },
    Row {
        spec: "`verify`, `verify r`",
        capability: "`maintain`",
        forms: &[
            ("verify", &[Capability::Maintain]),
            ("verify items", &[Capability::Maintain]),
        ],
        unsupported: &[],
    },
    Row {
        spec: "`reindex r`, `reindex r index (f) [in S]`",
        capability: "`maintain`",
        forms: &[("reindex items", &[Capability::Maintain])],
        unsupported: &[],
    },
    Row {
        spec: "`compact`",
        capability: "`maintain`",
        forms: &[],
        unsupported: &["compact"],
    },
    Row {
        spec: "`finalize upgrade`",
        capability: "`maintain`",
        forms: &[],
        unsupported: &["finalize upgrade"],
    },
    Row {
        spec: "`set setting S = v`",
        capability: "`configure`",
        forms: &[(
            "set setting RetryLimit = { retries: 3 }",
            &[Capability::Configure],
        )],
        unsupported: &[],
    },
    Row {
        spec: "`retire bundle B`",
        capability: "the bundle's owner or the schema owner",
        forms: &[],
        unsupported: &[],
    },
    Row {
        spec: "`supersede bundle B by B2`",
        capability: "`B`'s owner or the schema owner",
        forms: &[],
        unsupported: &[],
    },
    Row {
        spec: "`export (q) to \"path\"`",
        capability: "`write_files`, through a server",
        forms: &[],
        unsupported: &["export (from i in items select i) to \"x.csv\""],
    },
    Row {
        spec: "`register bundle \"file\"`",
        capability: "as registration requires",
        forms: &[],
        unsupported: &["register bundle \"x.adb\""],
    },
    Row {
        spec: "`apply migrations \"file\" ...`",
        capability: "`schema`",
        forms: &[],
        unsupported: &["apply migrations \"x\""],
    },
    Row {
        spec: "`print schema`",
        capability: "none beyond reading the catalog",
        forms: &[],
        unsupported: &["print schema"],
    },
    Row {
        spec: "`archive audit through n to \"path\"`",
        capability: "`maintain` and `write_files`",
        forms: &[],
        unsupported: &["archive audit through 1 to \"x\""],
    },
    Row {
        spec: "`rotate key kek \"uri\"`, `rotate key dek`",
        capability: "`configure`",
        forms: &[],
        unsupported: &["rotate key kek \"x\"", "rotate key dek"],
    },
    Row {
        spec: "`destroy key dek n`",
        capability: "`configure`",
        forms: &[],
        unsupported: &["destroy key dek 1"],
    },
    Row {
        spec: "`dump to \"path\" [encrypt key]`",
        capability: "`maintain` and `read` on every relation dumped, and `write_files` through a server; the security section only with `schema`",
        forms: &[],
        unsupported: &["dump to \"x\""],
    },
    Row {
        spec: "`restore from \"source\" to \"path\" [before version n \\| at instant t \\| at position p]`",
        capability: "in process or on a closed file only",
        forms: &[],
        unsupported: &["restore from \"a\" to \"b\""],
    },
    Row {
        spec: "`backup to \"target\" [incremental]`",
        capability: "`maintain`",
        forms: &[],
        unsupported: &["backup to \"x\""],
    },
];

/// Returns the first and last columns of each row of the catalog part's command table.
fn command_table() -> Vec<(String, String)> {
    let start = CATALOG_SPEC
        .find("| Command | Effect | Capability |")
        .expect("the catalog part has the command table");
    CATALOG_SPEC[start..]
        .lines()
        .skip(2)
        .take_while(|l| l.starts_with('|'))
        .map(|line| {
            let cells: Vec<&str> = line.trim_matches('|').split(" | ").map(str::trim).collect();
            (cells[0].to_owned(), cells[cells.len() - 1].to_owned())
        })
        .collect()
}

#[test]
fn the_table_of_commands_has_a_row_for_each_row_of_the_catalog_part() {
    let stated = command_table();
    let tested: Vec<(String, String)> = COMMANDS
        .iter()
        .map(|r| (r.spec.to_owned(), r.capability.to_owned()))
        .collect();
    assert_eq!(stated, tested);
}

fn audit_denied(db: &Database<SimRuntime>, role: &str) -> usize {
    query(
        db,
        &format!("from a in catalog::audit.Denied where a.role == \"{role}\" select a.seq"),
    )
    .len()
}

/// Assert `got` is `Denied` naming `needle`, and one more refusal of `role` is in the audit.
fn refused_and_audited(
    db: &Database<SimRuntime>,
    role: &str,
    text: &str,
    needle: &str,
    got: Result<(), Error>,
    before: usize,
) {
    match &got {
        Err(Error::Rejected {
            reason: RejectReason::Denied { detail },
        }) => assert!(detail.contains(needle), "{text}: {detail}"),
        other => panic!("{text}: {other:?}"),
    }
    assert_eq!(before + 1, audit_denied(db, role), "{text}");
}

#[test]
fn every_command_that_runs_checks_the_capability_the_catalog_part_names() {
    let mut sim = Sim::new(140, Config::default());
    run(&mut sim, |rt| {
        let db = opened_schema(&rt);
        let authority = db.role_authority();
        authority.create_role("none").expect("role");
        authority.create_role("adhoc").expect("role");
        authority.create_role("capable").expect("role");
        let none = authority.session("none").expect("session");
        let bare = authority.session("adhoc").expect("session");
        let capable = authority.session("capable").expect("session");
        let owner = authority.session("owner").expect("session");
        authority
            .grant_capability("adhoc", Capability::Adhoc, false)
            .expect("grant");
        authority
            .grant_capability("capable", Capability::Adhoc, false)
            .expect("grant");
        for row in COMMANDS {
            for &(text, capabilities) in row.forms {
                let before = audit_denied(&db, "none");
                refused_and_audited(&db, "none", text, "`adhoc`", task(&rt, &none, text), before);
                let before = audit_denied(&db, "adhoc");
                let needed = capabilities[0].name();
                refused_and_audited(&db, "adhoc", text, needed, task(&rt, &bare, text), before);
                for capability in capabilities {
                    authority
                        .grant_capability("capable", *capability, false)
                        .expect("grant");
                }
                let before = audit_denied(&db, "capable");
                task(&rt, &capable, text).unwrap_or_else(|e| panic!("{text}: {e:?}"));
                assert_eq!(before, audit_denied(&db, "capable"), "{text}");
                for capability in capabilities {
                    authority
                        .revoke_capability("capable", *capability)
                        .expect("revoke");
                }
                let got = task(&rt, &capable, text);
                refused_and_audited(&db, "capable", text, needed, got, before);
                task(&rt, &owner, text).unwrap_or_else(|e| panic!("owner {text}: {e:?}"));
            }
        }
    });
}

const EVERY_CAPABILITY: [Capability; 9] = [
    Capability::Adhoc,
    Capability::Assert,
    Capability::Schema,
    Capability::Monitor,
    Capability::Cancel,
    Capability::Maintain,
    Capability::Configure,
    Capability::ReadFiles,
    Capability::WriteFiles,
];

fn grant_all(db: &Database<SimRuntime>, role: &str) {
    for capability in EVERY_CAPABILITY {
        db.role_authority()
            .grant_capability(role, capability, false)
            .expect("grant");
    }
}

/// The commands the engine does not run that no capability lets a role issue.
const OWNER_ONLY_UNSUPPORTED: &[&str] = &[
    "cancel call sys::calls.key(1)",
    "export (from i in items select i) to \"x.csv\"",
    "register bundle \"x.adb\"",
    "print schema",
    "dump to \"x\"",
    "restore from \"a\" to \"b\"",
];

#[test]
fn a_command_the_engine_does_not_run_is_refused_after_what_the_role_must_hold() {
    let mut sim = Sim::new(141, Config::default());
    run(&mut sim, |rt| {
        let db = opened_schema(&rt);
        let authority = db.role_authority();
        authority.create_role("web").expect("role");
        let web = authority.session("web").expect("session");
        let owner = authority.session("owner").expect("session");
        for text in COMMANDS.iter().flat_map(|r| r.unsupported) {
            denied(task(&rt, &web, text));
        }
        authority
            .grant_capability("web", Capability::Adhoc, false)
            .expect("grant");
        for text in COMMANDS.iter().flat_map(|r| r.unsupported) {
            let before = audit_denied(&db, "web");
            denied(task(&rt, &web, text));
            assert_eq!(before + 1, audit_denied(&db, "web"), "{text}");
            let got = task(&rt, &owner, text);
            assert!(
                format!("{got:?}").contains("does not run"),
                "{text}: {got:?}"
            );
        }
        grant_all(&db, "web");
        for text in COMMANDS.iter().flat_map(|r| r.unsupported) {
            let got = task(&rt, &web, text);
            if OWNER_ONLY_UNSUPPORTED.contains(text) {
                denied(got);
            } else {
                assert!(
                    format!("{got:?}").contains("does not run"),
                    "{text}: {got:?}"
                );
            }
        }
    });
}

const SECRETS: &str = "module example\nadd module example\nadd type Secret = { id: Key secrets, salary: I64 }\nadd relation secrets : Secret key id\n";

fn opened_secrets(rt: &SimRuntime) -> Database<SimRuntime> {
    Database::open_in(rt.clone(), "secrets.adt", &Log::new([SECRETS]), &options(2))
        .expect("schema database opens")
}

fn denied_detail(got: Result<(), Error>) -> String {
    match got {
        Err(Error::Rejected {
            reason: RejectReason::Denied { detail },
        }) => detail,
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_role_with_only_adhoc_gets_no_checker_output_and_each_refusal_is_audited() {
    let mut sim = Sim::new(146, Config::default());
    run(&mut sim, |rt| {
        let db = opened_secrets(&rt);
        let authority = db.role_authority();
        authority.create_role("w").expect("role");
        authority
            .grant_capability("w", Capability::Adhoc, false)
            .expect("grant");
        let w = authority.session("w").expect("session");
        for text in [
            "from i in secrets select i.nope",
            "from i in secrets select i.salary",
            "from s in secrets select s.salary + \"x\"",
            "from i in nosuch select i",
            "vacuum nosuch",
            "vacuum secrets",
            "verify nosuch",
            "set setting Nope = 1",
            "set setting RetryLimit = { retries: \"x\" }",
            "explain from i in secrets select i.nope",
            "transact\n  delete i in secrets",
            "temp relation stage : Secret key id\nverify",
            "vacuum secrets\nvacuum nosuch",
            "grant read secrets (id) to w",
        ] {
            let before = audit_denied(&db, "w");
            denied_detail(task(&rt, &w, text));
            assert_eq!(before + 1, audit_denied(&db, "w"), "{text}");
        }
    });
}

#[test]
fn a_refusal_of_a_named_relation_reads_the_same_whether_or_not_it_exists() {
    let mut sim = Sim::new(147, Config::default());
    run(&mut sim, |rt| {
        let db = opened_secrets(&rt);
        let authority = db.role_authority();
        authority.create_role("w").expect("role");
        authority
            .grant_capability("w", Capability::Adhoc, false)
            .expect("grant");
        let w = authority.session("w").expect("session");
        for (present, absent) in [
            ("vacuum secrets", "vacuum nosuch"),
            ("verify secrets", "verify nosuch"),
            ("reindex secrets", "reindex nosuch"),
            (
                "from i in secrets select i.salary",
                "from i in secrets select i.nope",
            ),
            ("from i in secrets select i", "from i in nosuch select i"),
            (
                "set setting RetryLimit = { retries: 1 }",
                "set setting Nope = 1",
            ),
        ] {
            assert_eq!(
                denied_detail(task(&rt, &w, present)),
                denied_detail(task(&rt, &w, absent)),
                "{present} against {absent}"
            );
        }
    });
}

#[test]
fn a_capability_holder_gets_no_checker_reply_on_a_statement_inside_a_command() {
    let mut sim = Sim::new(148, Config::default());
    run(&mut sim, |rt| {
        let db = opened_secrets(&rt);
        let authority = db.role_authority();
        authority.create_role("w").expect("role");
        grant_all(&db, "w");
        let w = authority.session("w").expect("session");
        for text in [
            "set setting RetryLimit = { retries: count(i for i in secrets) }",
            "set setting RetryLimit = { retries: from i in secrets select i.nope }",
            "set setting RetryLimit = { retries: nosuch }",
            "pin plan (from i in secrets select i.nope)",
            "unpin plan (from i in secrets select i.nope)",
            "terminate session (from i in secrets select i.nope)",
            "archive audit through (from i in secrets select i.nope) to \"x\"",
            "reindex secrets index (from i in secrets select i.nope) in Secret",
            "explain from i in secrets select i.nope",
            "from i in secrets select i.nope",
        ] {
            let before = audit_denied(&db, "w");
            denied_detail(task(&rt, &w, text));
            assert_eq!(before + 1, audit_denied(&db, "w"), "{text}");
        }
        let got = format!(
            "{:?}",
            task(&rt, &w, "reindex secrets index (id) in Secret")
        );
        assert!(got.contains("Invalid"), "{got}");
    });
}

#[test]
fn a_parse_failure_answers_invalid_and_needs_adhoc_first() {
    let mut sim = Sim::new(149, Config::default());
    run(&mut sim, |rt| {
        let db = opened_secrets(&rt);
        let authority = db.role_authority();
        authority.create_role("w").expect("role");
        let w = authority.session("w").expect("session");
        let text = "syntax error here (((";
        denied_detail(task(&rt, &w, text));
        assert_eq!(1, audit_denied(&db, "w"));
        authority
            .grant_capability("w", Capability::Adhoc, false)
            .expect("grant");
        for text in [text, ""] {
            let got = format!("{:?}", task(&rt, &w, text));
            assert!(got.contains("Invalid"), "{got}");
            assert_eq!(1, audit_denied(&db, "w"));
        }
    });
}

#[test]
fn print_schema_runs_only_for_the_owner() {
    let mut sim = Sim::new(150, Config::default());
    run(&mut sim, |rt| {
        let db = opened_secrets(&rt);
        let authority = db.role_authority();
        authority.create_role("w").expect("role");
        grant_all(&db, "w");
        let w = authority.session("w").expect("session");
        let detail = denied_detail(task(&rt, &w, "print schema"));
        assert!(detail.contains("owner"), "{detail}");
        assert_eq!(1, audit_denied(&db, "w"));
    });
}

#[test]
fn a_refusal_on_a_dropped_role_session_is_audited() {
    let mut sim = Sim::new(151, Config::default());
    run(&mut sim, |rt| {
        let db = opened_secrets(&rt);
        let authority = db.role_authority();
        authority.create_role("w").expect("role");
        let w = authority.session("w").expect("session");
        authority.drop_role("w").expect("drop");
        let detail = denied_detail(task(&rt, &w, "verify"));
        assert!(detail.contains("no longer"), "{detail}");
        assert_eq!(1, audit_denied(&db, "w"));
    });
}

#[test]
fn a_statement_or_a_statement_inside_a_command_runs_only_for_the_owner() {
    let mut sim = Sim::new(142, Config::default());
    run(&mut sim, |rt| {
        let db = opened_schema(&rt);
        let authority = db.role_authority();
        authority.create_role("web").expect("role");
        let web = authority.session("web").expect("session");
        for capability in [
            Capability::Adhoc,
            Capability::Maintain,
            Capability::Configure,
            Capability::Monitor,
        ] {
            authority
                .grant_capability("web", capability, false)
                .expect("grant");
        }
        for text in [
            "from i in items select i.value",
            "delete i in items",
            "explain from i in items select i.value",
            "verify\nfrom i in items select i.value",
            "set setting RetryLimit = { retries: count(i for i in items) }",
        ] {
            let before = audit_denied(&db, "web");
            refused_and_audited(&db, "web", text, "owner", task(&rt, &web, text), before);
        }
    });
}

#[test]
fn a_role_without_adhoc_learns_nothing_of_text_the_checker_would_refuse() {
    let mut sim = Sim::new(143, Config::default());
    run(&mut sim, |rt| {
        let db = opened_schema(&rt);
        let authority = db.role_authority();
        authority.create_role("web").expect("role");
        let web = authority.session("web").expect("session");
        denied(task(&rt, &web, "from i in no_such_relation select i"));
        assert_eq!(1, audit_denied(&db, "web"));
    });
}

#[test]
fn an_asserted_principal_needs_assert_and_a_refusal_records_the_session_s_service() {
    let mut sim = Sim::new(144, Config::default());
    run(&mut sim, |rt| {
        let db = opened_schema(&rt);
        let authority = db.role_authority();
        authority.create_role("web").expect("role");
        authority
            .grant_capability("web", Capability::Adhoc, false)
            .expect("grant");
        authority
            .grant_capability("web", Capability::Maintain, false)
            .expect("grant");
        let web = authority.session("web").expect("session");
        let options = TaskOptions {
            call: CallOptions {
                assert: Some(Value::text("nobody")),
                ..CallOptions::default()
            },
            ..TaskOptions::default()
        };
        denied(web.submit_text("verify", &[], options).map(drop));
        let principal = query(&db, "from a in catalog::audit.Denied select a.principal");
        assert_eq!(1, principal.len());
        assert!(principal[0].contains("web"), "{principal:?}");
    });
}

#[test]
fn the_in_process_api_runs_each_command_with_no_role() {
    let mut sim = Sim::new(145, Config::default());
    run(&mut sim, |rt| {
        let db = opened_schema(&rt);
        for row in COMMANDS {
            for &(text, _) in row.forms {
                db.query_text(text, &[], &CallOptions::default())
                    .unwrap_or_else(|e| panic!("{text}: {e:?}"));
            }
        }
    });
}

/// The queries behind a held worker: a role's text waits in the queue until the gate opens.
const GATED: &str = "module example\nadd module example\ndefine fn gate (n: I64) -> I64 = host \"gate\" version \"1.0\" deterministic\n";

fn gated_options(rt: &SimRuntime, gate: &Arc<Gate>) -> OpenOptions {
    let mut host = HostFns::new();
    let (grt, g) = (rt.clone(), gate.clone());
    host.register(
        "gate",
        "1.0",
        Determinism::Deterministic,
        move |b: &Batch| {
            g.pass(&grt);
            let mut out = BatchBuilder::new(batch::Schema::new(vec![Type::Scalar(Scalar::I64)]));
            for i in 0..b.len() {
                out.push_row(&[b.value(0, i)]);
            }
            Ok(out.finish())
        },
    );
    OpenOptions {
        workers: Some(1),
        host_fns: host,
        ..options(2)
    }
}

/// Holds every call of the host function `gate` until released.
#[derive(Default)]
struct Gate {
    open: AtomicBool,
    entered: AtomicBool,
}

impl Gate {
    fn pass(&self, rt: &SimRuntime) {
        self.entered.store(true, Ordering::Release);
        while !self.open.load(Ordering::Acquire) {
            rt.yield_now();
        }
    }
}

struct Queued {
    db: Database<SimRuntime>,
    gate: Arc<Gate>,
    hold: Task<SimRuntime>,
}

/// Returns a database of one worker that the owner's text holds on the gate.
fn held(rt: &SimRuntime) -> Queued {
    let gate = Arc::new(Gate::default());
    let db = Database::open_in(
        rt.clone(),
        "queued.adt",
        &Log::new([GATED]),
        &gated_options(rt, &gate),
    )
    .expect("schema database opens");
    let owner = db.role_authority().session("owner").expect("session");
    let hold = owner
        .submit_text(
            "from n in [1, 2] select gate(n)",
            &[],
            TaskOptions::default(),
        )
        .expect("the holding text is admitted");
    hold.supply(usize::MAX);
    while !gate.entered.load(Ordering::Acquire) {
        rt.yield_now();
    }
    Queued { db, gate, hold }
}

struct Unpark {
    rt: SimRuntime,
    thread: ThreadId,
}

impl Wake for Unpark {
    fn wake(self: Arc<Self>) {
        self.rt.unpark(self.thread);
    }
}

fn finish(rt: &SimRuntime, task: &Task<SimRuntime>) -> Result<(), Error> {
    let waker = Waker::from(Arc::new(Unpark {
        rt: rt.clone(),
        thread: rt.current(),
    }));
    task.supply(usize::MAX);
    loop {
        match task.observe(&waker) {
            Observation::Pending => rt.park(),
            Observation::Layout(_) | Observation::Batch(_) => {}
            Observation::End => task.drained(),
            Observation::Done(Completion::Failed(e)) => return Err(e),
            Observation::Done(_) => return Ok(()),
        }
    }
}

fn queued_text(q: &Queued, session: &RoleSession<SimRuntime>, text: &str) -> Task<SimRuntime> {
    assert!(!q.gate.open.load(Ordering::Acquire), "the worker is held");
    session
        .submit_text(text, &[], TaskOptions::default())
        .expect("the text is admitted")
}

fn release(rt: &SimRuntime, q: &Queued) {
    q.gate.open.store(true, Ordering::Release);
    finish(rt, &q.hold).expect("the holding text ends");
}

#[test]
fn a_revoke_while_queued_stops_the_task_with_denied_and_its_audit_row() {
    let mut sim = Sim::new(160, Config::default());
    run(&mut sim, |rt| {
        let q = held(&rt);
        let authority = q.db.role_authority();
        authority.create_role("dba").expect("role");
        for capability in [Capability::Adhoc, Capability::Maintain] {
            authority
                .grant_capability("dba", capability, false)
                .expect("grant");
        }
        let dba = authority.session("dba").expect("session");
        let task = queued_text(&q, &dba, "verify");
        authority
            .revoke_capability("dba", Capability::Maintain)
            .expect("revoke");
        release(&rt, &q);
        denied(finish(&rt, &task));
        assert_eq!(1, audit_denied(&q.db, "dba"));
        assert_eq!(0, audit_calls(&q.db, "dba"));
    });
}

#[test]
fn a_revoke_of_adhoc_while_queued_stops_the_task() {
    let mut sim = Sim::new(161, Config::default());
    run(&mut sim, |rt| {
        let q = held(&rt);
        let authority = q.db.role_authority();
        authority.create_role("dba").expect("role");
        for capability in [Capability::Adhoc, Capability::Maintain] {
            authority
                .grant_capability("dba", capability, false)
                .expect("grant");
        }
        let dba = authority.session("dba").expect("session");
        let task = queued_text(&q, &dba, "verify");
        authority
            .revoke_capability("dba", Capability::Adhoc)
            .expect("revoke");
        release(&rt, &q);
        denied(finish(&rt, &task));
        assert_eq!(1, audit_denied(&q.db, "dba"));
    });
}

#[test]
fn a_role_dropped_while_queued_stops_the_task_with_its_audit_row() {
    let mut sim = Sim::new(162, Config::default());
    run(&mut sim, |rt| {
        let q = held(&rt);
        let authority = q.db.role_authority();
        authority.create_role("dba").expect("role");
        for capability in [Capability::Adhoc, Capability::Maintain] {
            authority
                .grant_capability("dba", capability, false)
                .expect("grant");
        }
        let dba = authority.session("dba").expect("session");
        let task = queued_text(&q, &dba, "verify");
        authority.drop_role("dba").expect("drop");
        release(&rt, &q);
        let detail = denied_detail(finish(&rt, &task));
        assert!(detail.contains("no longer"), "{detail}");
        assert_eq!(1, audit_denied(&q.db, "dba"));
    });
}

#[test]
fn a_grant_kept_while_queued_runs_the_task() {
    let mut sim = Sim::new(163, Config::default());
    run(&mut sim, |rt| {
        let q = held(&rt);
        let authority = q.db.role_authority();
        authority.create_role("dba").expect("role");
        for capability in [Capability::Adhoc, Capability::Maintain] {
            authority
                .grant_capability("dba", capability, false)
                .expect("grant");
        }
        let dba = authority.session("dba").expect("session");
        let task = queued_text(&q, &dba, "verify");
        release(&rt, &q);
        finish(&rt, &task).expect("the grant still holds");
        assert_eq!(0, audit_denied(&q.db, "dba"));
    });
}

#[test]
fn a_revoke_after_the_call_ran_does_not_reach_it() {
    let mut sim = Sim::new(164, Config::default());
    run(&mut sim, |rt| {
        let db = opened_secrets(&rt);
        let authority = db.role_authority();
        authority.create_role("dba").expect("role");
        for capability in [Capability::Adhoc, Capability::Maintain] {
            authority
                .grant_capability("dba", capability, false)
                .expect("grant");
        }
        let dba = authority.session("dba").expect("session");
        let task = dba
            .submit_text("verify", &[], TaskOptions::default())
            .expect("admitted");
        let got = finish(&rt, &task);
        authority
            .revoke_capability("dba", Capability::Maintain)
            .expect("revoke");
        got.expect("a revoke after the call ran does not reach it");
    });
}

fn audit_calls(db: &Database<SimRuntime>, role: &str) -> usize {
    query(
        db,
        &format!("from a in catalog::audit.Call where a.role == \"{role}\" select a.seq"),
    )
    .len()
}

fn adhoc_and(db: &Database<SimRuntime>, role: &str, capability: Capability) {
    let authority = db.role_authority();
    authority.create_role(role).expect("role");
    for c in [Capability::Adhoc, capability] {
        authority.grant_capability(role, c, false).expect("grant");
    }
}

fn retries(db: &Database<SimRuntime>) -> Vec<String> {
    query(
        db,
        "from s in catalog::settings.RetryLimit select s.retries",
    )
}

fn retries_after(
    rt: &SimRuntime,
    ops: &RoleSession<SimRuntime>,
    db: &Database<SimRuntime>,
) -> Vec<String> {
    task(rt, ops, "set setting RetryLimit = { retries: 5 }").expect("a literal value");
    retries(db)
}

fn with_param(
    rt: &SimRuntime,
    session: &RoleSession<SimRuntime>,
    text: &str,
    name: &str,
    value: i64,
) -> Result<(), Error> {
    let params = [(name, Ty::Base(Base::I64), Value::Int(value))];
    let task = session.submit_text(text, &params, TaskOptions::default())?;
    finish(rt, &task)
}

#[test]
fn a_holder_sets_a_setting_to_a_computed_value() {
    let mut sim = Sim::new(165, Config::default());
    run(&mut sim, |rt| {
        let db = opened_secrets(&rt);
        adhoc_and(&db, "ops", Capability::Configure);
        let ops = db.role_authority().session("ops").expect("session");
        task(&rt, &ops, "set setting RetryLimit = { retries: 1 + 2 }").expect("a computed value");
        assert_eq!(vec!["3"], retries(&db));
        assert_eq!(0, audit_denied(&db, "ops"));
    });
}

#[test]
fn a_holder_sets_a_setting_to_a_parameter() {
    let mut sim = Sim::new(166, Config::default());
    run(&mut sim, |rt| {
        let db = opened_secrets(&rt);
        adhoc_and(&db, "ops", Capability::Configure);
        let ops = db.role_authority().session("ops").expect("session");
        with_param(&rt, &ops, "set setting RetryLimit = { retries: n }", "n", 7)
            .expect("a parameter");
        assert_eq!(vec!["7"], retries(&db));
        assert_eq!(0, audit_denied(&db, "ops"));
    });
}

#[test]
fn a_nonholder_is_refused_a_computed_value_as_it_is_a_literal() {
    let mut sim = Sim::new(167, Config::default());
    run(&mut sim, |rt| {
        let db = opened_secrets(&rt);
        adhoc_and(&db, "ops", Capability::Maintain);
        let ops = db.role_authority().session("ops").expect("session");
        for text in [
            "set setting RetryLimit = { retries: 1 + 2 }",
            "set setting RetryLimit = { retries: 3 }",
        ] {
            let detail = denied_detail(task(&rt, &ops, text));
            assert!(detail.contains("`configure`"), "{text}: {detail}");
        }
        assert_eq!(2, audit_denied(&db, "ops"));
    });
}

#[test]
fn a_parameter_does_not_lift_a_read_inside_a_command_value() {
    let mut sim = Sim::new(168, Config::default());
    run(&mut sim, |rt| {
        let db = opened_secrets(&rt);
        adhoc_and(&db, "ops", Capability::Configure);
        let ops = db.role_authority().session("ops").expect("session");
        for text in [
            "set setting RetryLimit = { retries: count(i for i in secrets) + n }",
            "set setting RetryLimit = { retries: secrets + n }",
            "set setting RetryLimit = { retries: n.salary }",
        ] {
            let got = with_param(&rt, &ops, text, "n", 1);
            assert!(denied_detail(got).contains("owner"), "{text}");
        }
        assert_eq!(3, audit_denied(&db, "ops"));
    });
}

#[test]
fn a_parameter_named_for_a_schema_name_answers_as_an_absent_name() {
    let mut sim = Sim::new(169, Config::default());
    run(&mut sim, |rt| {
        let db = opened_secrets(&rt);
        adhoc_and(&db, "ops", Capability::Configure);
        adhoc_and(&db, "viewer", Capability::Monitor);
        let authority = db.role_authority();
        let ops = authority.session("ops").expect("session");
        let viewer = authority.session("viewer").expect("session");
        let shadowed = "set setting RetryLimit = { retries: secrets }";
        for (session, role) in [(&ops, "ops"), (&viewer, "viewer")] {
            let before = audit_denied(&db, role);
            let absent = denied_detail(task(
                &rt,
                session,
                "set setting RetryLimit = { retries: nosuch }",
            ));
            let relation = denied_detail(with_param(&rt, session, shadowed, "secrets", 5));
            assert_eq!(absent, relation, "{role}");
            let caller = denied_detail(with_param(
                &rt,
                session,
                "set setting RetryLimit = { retries: caller }",
                "caller",
                5,
            ));
            assert_eq!(absent, caller, "{role}");
            assert_eq!(before + 3, audit_denied(&db, role), "{role}");
        }
        assert_eq!(vec!["5"], retries_after(&rt, &ops, &db));
    });
}

#[test]
fn a_holders_absent_name_is_invalid_with_no_denied_row() {
    let mut sim = Sim::new(170, Config::default());
    run(&mut sim, |rt| {
        let db = opened_secrets(&rt);
        adhoc_and(&db, "dba", Capability::Maintain);
        let dba = db.role_authority().session("dba").expect("session");
        let got = format!("{:?}", task(&rt, &dba, "vacuum nosuch"));
        assert!(got.contains("Invalid"), "{got}");
        assert_eq!(0, audit_denied(&db, "dba"));
        assert_eq!(1, audit_calls(&db, "dba"));
    });
}

#[test]
fn a_nonholder_answer_and_audit_do_not_depend_on_the_name_existing() {
    let mut sim = Sim::new(171, Config::default());
    run(&mut sim, |rt| {
        let db = opened_secrets(&rt);
        adhoc_and(&db, "ops", Capability::Monitor);
        let ops = db.role_authority().session("ops").expect("session");
        let shape = "from a in catalog::audit.Denied where a.role == \"ops\" select a.seq";
        let principal =
            "from a in catalog::audit.Denied where a.role == \"ops\" select a.principal";
        for (present, absent) in [
            ("vacuum secrets", "vacuum nosuch"),
            ("verify secrets", "verify nosuch"),
            ("reindex secrets", "reindex nosuch"),
            (
                "set setting RetryLimit = { retries: 1 }",
                "set setting Nope = 1",
            ),
        ] {
            let rows_before = query(&db, shape).len();
            let one = denied_detail(task(&rt, &ops, present));
            assert_eq!(rows_before + 1, query(&db, shape).len(), "{present}");
            let other = denied_detail(task(&rt, &ops, absent));
            assert_eq!(rows_before + 2, query(&db, shape).len(), "{absent}");
            assert_eq!(one, other, "{present} against {absent}");
        }
        assert_eq!(1, query(&db, principal).len());
        assert_eq!(0, audit_calls(&db, "ops"));
    });
}

#[test]
fn an_adhoc_call_writes_one_row_with_its_outcome() {
    let mut sim = Sim::new(172, Config::default());
    run(&mut sim, |rt| {
        let db = opened_secrets(&rt);
        adhoc_and(&db, "dba", Capability::Maintain);
        let dba = db.role_authority().session("dba").expect("session");
        task(&rt, &dba, "verify").expect("verify");
        let rows = query(
            &db,
            "from a in catalog::audit.Call where a.role == \"dba\" select a",
        );
        assert_eq!(1, rows.len(), "{rows:?}");
        assert!(rows[0].contains("AdHoc"), "{rows:?}");
        assert!(rows[0].contains("Succeeded"), "{rows:?}");
        assert!(rows[0].contains("Service"), "{rows:?}");
        let got = format!("{:?}", task(&rt, &dba, "verify nosuch"));
        assert!(got.contains("Invalid"), "{got}");
        let failed = query(
            &db,
            "from a in catalog::audit.Call where a.role == \"dba\" select a.outcome",
        );
        assert_eq!(2, failed.len(), "{failed:?}");
        assert_eq!(
            1,
            failed.iter().filter(|o| o.contains("Failed")).count(),
            "{failed:?}"
        );
    });
}

#[test]
fn a_committed_adhoc_write_records_its_call_with_the_commit_and_a_failed_one_its_error() {
    let mut sim = Sim::new(173, Config::default());
    run(&mut sim, |rt| {
        let log = "module example\nadd module example\nadd type Item = { id: Key items, n: I64 }\nadd relation items : Item key id generate sequence\n";
        let db = Database::open_in(rt.clone(), "writes.adt", &Log::new([log]), &options(2))
            .expect("schema database opens");
        let owner = db.role_authority().session("owner").expect("session");
        task(&rt, &owner, "insert items { n: 1 }").expect("insert");
        let got = task(&rt, &owner, "insert items { n: 1 / 0 }");
        assert!(got.is_err(), "{got:?}");
        let rows = query(
            &db,
            "from a in catalog::audit.Call where a.role == \"owner\" select { seq: a.seq, outcome: a.outcome }",
        );
        assert_eq!(2, rows.len(), "{rows:?}");
        assert_eq!(
            1,
            rows.iter().filter(|o| o.contains("Succeeded")).count(),
            "{rows:?}"
        );
        assert!(
            rows.iter()
                .any(|o| o.contains("Failed") && o.contains("Rejected")),
            "the failed outcome holds the error value: {rows:?}"
        );
    });
}

#[test]
fn a_cancelled_queued_text_records_a_failed_call() {
    let mut sim = Sim::new(174, Config::default());
    run(&mut sim, |rt| {
        let q = held(&rt);
        let authority = q.db.role_authority();
        authority.create_role("dba").expect("role");
        for capability in [Capability::Adhoc, Capability::Maintain] {
            authority
                .grant_capability("dba", capability, false)
                .expect("grant");
        }
        let dba = authority.session("dba").expect("session");
        let task = queued_text(&q, &dba, "verify");
        task.cancel();
        release(&rt, &q);
        let got = format!("{:?}", finish(&rt, &task));
        assert!(got.contains("Cancelled"), "{got}");
        let rows = query(
            &q.db,
            "from a in catalog::audit.Call where a.role == \"dba\" select a.outcome",
        );
        assert_eq!(1, rows.len(), "{rows:?}");
        assert!(rows[0].contains("Cancelled"), "{rows:?}");
    });
}

#[test]
fn a_grant_by_a_role_records_that_role_and_its_service() {
    let mut sim = Sim::new(175, Config::default());
    run(&mut sim, |rt| {
        let db = opened_schema(&rt);
        let authority = db.role_authority();
        authority.create_role("ann").expect("role");
        authority.create_role("bob").expect("role");
        authority
            .grant_capability("ann", Capability::Monitor, true)
            .expect("grant");
        let ann = authority.session("ann").expect("session");
        authority
            .grant_capability_as(&ann, "bob", Capability::Monitor, false)
            .expect("grant");
        authority
            .revoke(&ann, "revoke monitor from bob")
            .expect("revoke");
        let rows = query(
            &db,
            "from a in catalog::audit.Grant where a.role == \"ann\" select { seq: a.seq, who: a.principal }",
        );
        assert_eq!(2, rows.len(), "{rows:?}");
        assert!(rows.iter().all(|p| p.contains("ann")), "{rows:?}");
        let owner = query(
            &db,
            "from a in catalog::audit.Grant where a.statement == \"grant monitor to bob\" select a.role",
        );
        assert_eq!(vec!["\"ann\""], owner);
    });
}

#[test]
fn nothing_audits_under_a_database_quarantined_whole() {
    let mut sim = Sim::new(176, Config::default());
    run(&mut sim, |rt| {
        let db = opened_secrets(&rt);
        let owner = db.role_authority().session("owner").expect("session");
        db.write_path()
            .pager()
            .quarantine(Quarantine::Database)
            .expect("mark");
        task(&rt, &owner, "verify").expect("verify runs and writes no row");
        let got = format!("{:?}", task(&rt, &owner, "from s in secrets select s"));
        assert!(got.contains("Quarantined"), "{got}");
        let got = format!("{:?}", task(&rt, &owner, "syntax error here ((("));
        assert!(got.contains("Quarantined"), "{got}");
    });
}

fn write(db: &Database<SimRuntime>, text: &str) {
    db.query_text(text, &[], &CallOptions::default())
        .expect("the write commits");
}

const GENERATED: &str = "module example\nadd module example\nadd type Secret = { id: Key secrets, salary: I64 }\nadd relation secrets : Secret key id generate sequence\n";

/// Each statement kind under a mark on the audit, and whether it answers.
const UNDER_AN_AUDIT_MARK: [(&str, bool); 11] = [
    ("verify", true),
    ("verify secrets", true),
    ("from s in secrets select s.salary", true),
    ("explain from s in secrets select s.salary", true),
    ("vacuum secrets", true),
    ("reindex secrets", true),
    ("delete s in secrets where s.salary == 99", true),
    ("explain analyze insert secrets { salary: 1 }", true),
    ("insert secrets { salary: 1 }", false),
    ("set setting RetryLimit = { retries: 7 }", false),
    ("from a in catalog::audit select a.seq", false),
];

fn quarantined(got: &Result<(), Error>) -> bool {
    format!("{got:?}").contains("Quarantined")
}

type Mark = fn(&Database<SimRuntime>) -> Quarantine;

fn audit_heap(ctor: &str) -> impl Fn(&Database<SimRuntime>) -> Quarantine {
    move |db| {
        let heap = db.write_path().heap_named("catalog::audit", ctor);
        Quarantine::Heap(heap.expect("the audit declares it").id().0)
    }
}

fn audit_key(db: &Database<SimRuntime>) -> Quarantine {
    let index = db.write_path().index_named("catalog::audit", "key seq");
    Quarantine::Index(index.expect("the audit has its key").tree().owner())
}

fn each_audit_mark(mut body: impl FnMut(u64, &SimRuntime, &Database<SimRuntime>)) {
    let call: Mark = |db| audit_heap("Call")(db);
    let denied: Mark = |db| audit_heap("Denied")(db);
    let key: Mark = audit_key;
    for (seed, mark) in [(180, call), (181, denied), (182, key)] {
        let mut sim = Sim::new(seed, Config::default());
        run(&mut sim, |rt| {
            let db = Database::open_in(
                rt.clone(),
                "audited.adt",
                &Log::new([GENERATED]),
                &options(2),
            )
            .expect("schema database opens");
            write(&db, "insert secrets { salary: 1 }");
            db.write_path().pager().quarantine(mark(&db)).expect("mark");
            body(seed, &rt, &db);
        });
    }
}

#[test]
fn an_audit_mark_stops_writes_and_lets_reads_refusals_and_verify_answer() {
    each_audit_mark(|seed, rt, db| {
        let owner = db.role_authority().session("owner").expect("session");
        for (text, answers) in UNDER_AN_AUDIT_MARK {
            let got = task(rt, &owner, text);
            if text.contains("catalog::audit") && seed == 182 {
                assert!(got.is_ok(), "seed {seed} {text}: {got:?}");
            } else if answers {
                assert!(!quarantined(&got), "seed {seed} {text}: {got:?}");
                assert!(got.is_ok(), "seed {seed} {text}: {got:?}");
            } else {
                assert!(quarantined(&got), "seed {seed} {text}: {got:?}");
            }
        }
        let refused = task(rt, &owner, "select (((");
        assert!(format!("{refused:?}").contains("Invalid"), "seed {seed}");
        assert_eq!(
            vec!["1"],
            query(db, "from s in secrets select s.salary"),
            "seed {seed}"
        );
        assert!(
            !query(
                db,
                "from s in catalog::settings.RetryLimit select s.retries"
            )
            .contains(&"7".to_owned()),
            "seed {seed}"
        );
    });
}

#[test]
fn an_audit_mark_writes_no_row_and_leaves_refusals_denied() {
    let mut sim = Sim::new(183, Config::default());
    run(&mut sim, |rt| {
        let db = opened_secrets(&rt);
        adhoc_and(&db, "ops", Capability::Monitor);
        let ops = db.role_authority().session("ops").expect("session");
        let owner = db.role_authority().session("owner").expect("session");
        db.write_path()
            .pager()
            .quarantine(audit_key(&db))
            .expect("mark");
        denied(task(&rt, &ops, "from s in secrets select s"));
        task(&rt, &owner, "verify").expect("verify");
        let rows = "from a in catalog::audit select a.seq";
        let audit = query(&db, rows);
        db.write_path()
            .pager()
            .release(audit_key(&db))
            .expect("release");
        assert_eq!(audit, query(&db, rows), "no row was written");
        assert_eq!(0, audit_denied(&db, "ops"));
    });
}

#[test]
fn a_grant_and_a_role_under_an_audit_mark_fail_and_leave_nothing() {
    each_audit_mark(|seed, _, db| {
        let authority = db.role_authority();
        let got = authority.create_role("late");
        assert!(format!("{got:?}").contains("Quarantined"), "seed {seed}");
        assert!(authority.session("late").is_err(), "seed {seed}");
    });
}

#[test]
fn a_write_failing_for_an_audit_mark_at_commit_is_undone() {
    each_audit_mark(|seed, rt, db| {
        let owner = db.role_authority().session("owner").expect("session");
        let got = task(rt, &owner, "insert secrets { salary: 1 }");
        assert!(quarantined(&got), "seed {seed}: {got:?}");
        assert_eq!(
            1,
            query(db, "from s in secrets select s.id").len(),
            "seed {seed}"
        );
    });
}

const COUNTED: &str = "module example\nadd module example\nadd type Item = { id: Key items, n: I64 }\nadd relation items : Item key id generate sequence\nadd type Log = { id: Key logs, n: I64 }\nadd relation logs : Log key id generate sequence\ndefine fn gate (n: I64) -> I64 = host \"gate\" version \"1.0\" deterministic\n";

fn completion(rt: &SimRuntime, task: &Task<SimRuntime>) -> Completion {
    let waker = Waker::from(Arc::new(Unpark {
        rt: rt.clone(),
        thread: rt.current(),
    }));
    task.supply(usize::MAX);
    loop {
        match task.observe(&waker) {
            Observation::Pending => rt.park(),
            Observation::Layout(_) | Observation::Batch(_) => {}
            Observation::End => task.drained(),
            Observation::Done(done) => return done,
        }
    }
}

#[test]
fn a_refused_call_forces_no_rerun_of_another_roles_write() {
    let mut sim = Sim::new(184, Config::default());
    run(&mut sim, |rt| {
        let gate = Arc::new(Gate::default());
        let options = OpenOptions {
            workers: Some(2),
            ..gated_options(&rt, &gate)
        };
        let db = Database::open_in(rt.clone(), "counted.adt", &Log::new([COUNTED]), &options)
            .expect("schema database opens");
        let authority = db.role_authority();
        authority.create_role("viewer").expect("role");
        authority
            .grant_capability("viewer", Capability::Adhoc, false)
            .expect("grant");
        let viewer = authority.session("viewer").expect("session");
        let owner = authority.session("owner").expect("session");
        task(&rt, &owner, "from i in items select i.n").expect("the audit is seeded");
        let gated = owner
            .submit_text(
                "insert logs { n: gate(count(i for i in items)) }",
                &[],
                TaskOptions::default(),
            )
            .expect("the write is admitted");
        while !gate.entered.load(Ordering::Acquire) {
            rt.yield_now();
        }
        write(&db, "insert items { n: 2 }");
        denied(task(&rt, &viewer, "vacuum items"));
        gate.open.store(true, Ordering::Release);
        let Completion::Committed { attempts, .. } = completion(&rt, &gated) else {
            panic!("the write commits");
        };
        assert_eq!(1, attempts);
        assert_eq!(1, audit_denied(&db, "viewer"));
    });
}

#[test]
fn a_parameter_that_does_not_print_as_its_type_refuses_with_no_row() {
    let mut sim = Sim::new(185, Config::default());
    run(&mut sim, |rt| {
        let db = opened_secrets(&rt);
        let owner = db.role_authority().session("owner").expect("session");
        let before = audit_calls(&db, "owner");
        let params = [("p", Ty::Base(Base::Text), Value::Int(1))];
        let got = owner.submit_text(
            "from s in secrets select p",
            &params,
            TaskOptions::default(),
        );
        let shown = format!("{:?}", got.map(drop));
        assert!(shown.contains("does not print"), "{shown}");
        assert_eq!(before, audit_calls(&db, "owner"));
    });
}

#[test]
fn a_task_stopped_between_its_commit_and_its_output_end_has_one_succeeded_row() {
    let mut sim = Sim::new(187, Config::default());
    run(&mut sim, |rt| {
        let log = "module example\nadd module example\nadd type Item = { id: Key items, n: I64 }\nadd relation items : Item key id generate sequence\n";
        let db = Database::open_in(rt.clone(), "output.adt", &Log::new([log]), &options(2))
            .expect("schema database opens");
        let owner = db.role_authority().session("owner").expect("session");
        let task = owner
            .submit_text(
                "insert items { n: 1 }\nfrom i in items select i.n",
                &[],
                TaskOptions {
                    call: CallOptions {
                        deadline: Some(rt.wall() + Duration::from_secs(5)),
                        ..CallOptions::default()
                    },
                    ..TaskOptions::default()
                },
            )
            .expect("the write is admitted");
        let waker = Waker::from(Arc::new(Unpark {
            rt: rt.clone(),
            thread: rt.current(),
        }));
        loop {
            match task.observe(&waker) {
                Observation::Pending => rt.park(),
                Observation::Layout(_) => break,
                other => panic!("the output waits for room: {other:?}"),
            }
        }
        let mut io = rt.io();
        let ticket = io.submit(Op::Timer {
            at: rt.now() + Duration::from_secs(10),
        });
        while io.poll().is_none_or(|c| c.ticket != ticket) {
            rt.park();
        }
        let done = completion(&rt, &task);
        assert!(
            matches!(
                done,
                Completion::Committed {
                    incomplete: true,
                    ..
                }
            ),
            "{done:?}"
        );
        let rows = query(
            &db,
            "from a in catalog::audit.Call where a.role == \"owner\" select a.outcome",
        );
        assert_eq!(1, rows.len(), "{rows:?}");
        assert!(rows[0].contains("Succeeded"), "{rows:?}");
    });
}

const TOUCH: &str = "update i in items set { i with value: i.value + 1 }";
const SETTING: &str = "set setting RetryLimit = { retries: 5 }";

/// Returns a database of `workers` workers with one row in `items`, of value 0.
const ITEMS_OF_VALUE: &str = "module example\nadd module example\nadd type Item = { id: Key items, value: I64 }\nadd relation items : Item key id generate sequence\n";

fn counted(rt: &SimRuntime, workers: usize) -> Database<SimRuntime> {
    let db = Database::open_in(
        rt.clone(),
        "counted.adt",
        &Log::new([ITEMS_OF_VALUE]),
        &options(workers),
    )
    .expect("schema database opens");
    results(&db, "insert items { value: 0 }");
    db
}

/// Returns the transaction ids taken so far, one of them by this call.
fn xids_taken(db: &Database<SimRuntime>) -> u64 {
    let write = db.write_path();
    let tx = write.begin().unwrap();
    let xid = tx.xid().unwrap().0;
    write.abort(tx).unwrap();
    xid
}

fn value(db: &Database<SimRuntime>) -> Vec<String> {
    query(db, "from i in items select i.value")
}

/// Run `text` alone, then again beside a transaction that has changed the row it updates, which
/// `end` ends once `text` waits on it. Returns the transaction ids the contended run took past
/// the lone one, what it completed with, and the row's value.
fn contended(
    text: &str,
    end: impl FnOnce(adtb::AdHocTransaction<'_, SimRuntime>),
) -> (u64, Completion, Vec<String>) {
    once(3, |rt| {
        let db = counted(&rt, 2);
        let session = db.role_authority().session("owner").expect("session");
        let before = xids_taken(&db);
        task(&rt, &session, text).expect("the text runs alone");
        let alone = xids_taken(&db) - before;
        let mut holder = db.begin(&TxnOptions::default()).unwrap();
        holder
            .query_text(TOUCH, &[], &TxnCallOptions::default())
            .unwrap();
        let before = xids_taken(&db);
        let t = session
            .submit_text(text, &[], TaskOptions::default())
            .expect("the text is admitted");
        for _ in 0..5000 {
            rt.yield_now();
        }
        let waker = Waker::from(Arc::new(Unpark {
            rt: rt.clone(),
            thread: rt.current(),
        }));
        assert!(
            matches!(t.observe(&waker), Observation::Pending),
            "the text waits on the holder"
        );
        end(holder);
        let done = completion(&rt, &t);
        let extra = xids_taken(&db) - before - alone;
        let values = value(&db);
        db.close().unwrap();
        (extra, done, values)
    })
}

#[test]
fn a_set_setting_before_a_waiting_write_changes_the_row_once() {
    let (extra, done, values) = contended(&format!("{SETTING}\n{TOUCH}"), |h| {
        h.rollback().unwrap();
    });
    assert!(matches!(done, Completion::Committed { .. }), "{done:?}");
    assert_eq!(0, extra);
    assert_eq!(vec!["2"], values);
}

#[test]
fn a_vacuum_never_precedes_a_write_in_one_text() {
    once(3, |rt| {
        let db = counted(&rt, 2);
        let session = db.role_authority().session("owner").expect("session");
        let refused = task(&rt, &session, &format!("vacuum items\n{TOUCH}"));
        assert!(
            matches!(
                refused,
                Err(Error::Rejected {
                    reason: RejectReason::Invalid { .. }
                })
            ),
            "{refused:?}"
        );
        assert_eq!(vec!["0"], value(&db));
        db.close().unwrap();
    });
}

#[test]
fn a_write_waiting_before_a_command_frees_its_worker_and_the_command_runs_once() {
    once(3, |rt| {
        let db = counted(&rt, 1);
        let session = db.role_authority().session("owner").expect("session");
        const READ: &str = "from i in items select i.value";
        let text = format!("{TOUCH}\n{SETTING}");
        let before = xids_taken(&db);
        task(&rt, &session, &text).expect("the text runs alone");
        task(&rt, &session, READ).expect("the read runs");
        let alone = xids_taken(&db) - before;
        let mut holder = db.begin(&TxnOptions::default()).unwrap();
        holder
            .query_text(TOUCH, &[], &TxnCallOptions::default())
            .unwrap();
        let before = xids_taken(&db);
        let t = session
            .submit_text(&text, &[], TaskOptions::default())
            .expect("the text is admitted");
        for _ in 0..5000 {
            rt.yield_now();
        }
        let read = session
            .submit_text(READ, &[], TaskOptions::default())
            .expect("the read is admitted");
        read.supply(usize::MAX);
        let waker = Waker::from(Arc::new(Unpark {
            rt: rt.clone(),
            thread: rt.current(),
        }));
        let mut read_done = false;
        for _ in 0..5000 {
            match read.observe(&waker) {
                Observation::End => read.drained(),
                Observation::Done(_) => {
                    read_done = true;
                    break;
                }
                Observation::Pending | Observation::Layout(_) | Observation::Batch(_) => {
                    rt.yield_now();
                }
            }
        }
        assert!(read_done, "the only worker runs a read beside the wait");
        holder.rollback().unwrap();
        finish(&rt, &t).expect("the text runs once the holder ends");
        assert_eq!(alone, xids_taken(&db) - before);
        assert_eq!(vec!["2"], value(&db));
        db.close().unwrap();
    });
}

#[test]
fn a_serialization_failure_before_any_command_reruns_the_text() {
    let (extra, done, values) = contended(&format!("{TOUCH}\n{SETTING}"), |h| {
        h.commit(None).unwrap();
    });
    let Completion::Committed { attempts, .. } = done else {
        panic!("the rerun commits: {done:?}")
    };
    assert_eq!(2, attempts);
    assert_eq!(
        1, extra,
        "the first attempt's transaction, the command in no attempt but the last"
    );
    assert_eq!(vec!["3"], values);
}

#[test]
fn a_serialization_failure_after_a_command_reaches_the_caller_with_no_second_effect() {
    let (extra, done, values) = contended(&format!("{SETTING}\n{TOUCH}"), |h| {
        h.commit(None).unwrap();
    });
    assert!(
        matches!(&done, Completion::Failed(e) if e.is_serialization()),
        "{done:?}"
    );
    assert_eq!(
        1, extra,
        "the audit row of the failure, and no second attempt"
    );
    assert_eq!(vec!["2"], values);
}

/// A text started on another thread or worker.
struct Started {
    /// Returns whether the text has yet to complete.
    pending: Box<dyn Fn() -> bool>,
    /// Returns what the text completes with.
    done: Box<dyn FnOnce() -> Result<(), Error>>,
}

fn as_task(rt: &SimRuntime, db: &Database<SimRuntime>, text: &str) -> Started {
    let session = db.role_authority().session("owner").expect("session");
    let t = session
        .submit_text(text, &[], TaskOptions::default())
        .expect("the text is admitted");
    let t = Arc::new(t);
    let (watched, rt, thread) = (t.clone(), rt.clone(), rt.current());
    let waker = Waker::from(Arc::new(Unpark {
        rt: rt.clone(),
        thread,
    }));
    Started {
        pending: Box::new(move || matches!(watched.observe(&waker), Observation::Pending)),
        done: Box::new(move || finish(&rt, &t)),
    }
}

fn embedded(rt: &SimRuntime, db: &Database<SimRuntime>, text: &str) -> Started {
    let done = Arc::new(Mutex::new(None));
    let caller = {
        let (db, text, done) = (db.clone(), text.to_owned(), done.clone());
        rt.spawn("caller", move || {
            let got = db.query_text(&text, &[], &CallOptions::default());
            *done.lock().unwrap() = Some(got.map(drop));
        })
    };
    let rt = rt.clone();
    let watched = done.clone();
    Started {
        pending: Box::new(move || watched.lock().unwrap().is_none()),
        done: Box::new(move || {
            rt.join(caller);
            done.lock()
                .unwrap()
                .take()
                .expect("the caller has finished")
        }),
    }
}

/// Run `text` beside a transaction that has changed the row it updates, which `end` ends after
/// `text` has run a while. Returns what `text` completed with and the row's value.
fn waited_on(
    text: &str,
    start: fn(&SimRuntime, &Database<SimRuntime>, &str) -> Started,
    end: impl FnOnce(adtb::AdHocTransaction<'_, SimRuntime>),
) -> (Result<(), Error>, Vec<String>) {
    once(3, |rt| {
        let db = counted(&rt, 2);
        let mut holder = db.begin(&TxnOptions::default()).unwrap();
        holder
            .query_text(TOUCH, &[], &TxnCallOptions::default())
            .unwrap();
        let started = start(&rt, &db, text);
        for _ in 0..5000 {
            rt.yield_now();
        }
        assert!((started.pending)(), "the text waits on the holder");
        end(holder);
        let got = (started.done)();
        let values = value(&db);
        db.close().unwrap();
        (got, values)
    })
}

/// Run `text`, which writes and ends in a statement reading `items` through `gate`, until it
/// holds at the gate, then doom it: a transaction reads the `logs` it wrote, another commits a
/// row of the `items` it read. Returns what `text` completed with and the rows of `logs`.
fn doomed_at_commit(
    text: &str,
    start: fn(&SimRuntime, &Database<SimRuntime>, &str) -> Started,
) -> (Result<(), Error>, Vec<String>) {
    once(3, |rt| {
        let gate = Arc::new(Gate::default());
        let options = OpenOptions {
            workers: Some(2),
            ..gated_options(&rt, &gate)
        };
        let db = Database::open_in(rt.clone(), "counted.adt", &Log::new([COUNTED]), &options)
            .expect("schema database opens");
        write(&db, "insert items { n: 1 }");
        let started = start(&rt, &db, text);
        while !gate.entered.load(Ordering::Acquire) {
            rt.yield_now();
        }
        let mut reader = db.begin(&TxnOptions::default()).unwrap();
        reader
            .query_text("from l in logs select l.n", &[], &TxnCallOptions::default())
            .unwrap();
        write(&db, "insert items { n: 2 }");
        gate.open.store(true, Ordering::Release);
        let got = (started.done)();
        reader.rollback().unwrap();
        let logs = query(&db, "from l in logs select l.n");
        db.close().unwrap();
        (got, logs)
    })
}

fn serialization(got: &Result<(), Error>) -> bool {
    matches!(got, Err(e) if e.is_serialization())
}

const LOGGED: &str = "insert logs { n: 1 }\nfrom i in items select gate(i.n)";

#[test]
fn a_failed_commit_after_a_command_reaches_the_caller() {
    let (got, logs) = doomed_at_commit(&format!("{SETTING}\n{LOGGED}"), as_task);
    assert!(serialization(&got), "{got:?}");
    assert!(logs.is_empty(), "{logs:?}");
}

#[test]
fn a_failed_commit_with_no_command_reruns_the_text() {
    let (got, logs) = doomed_at_commit(LOGGED, as_task);
    got.expect("the rerun commits");
    assert_eq!(vec!["1"], logs);
}

#[test]
fn an_embedded_serialization_failure_after_a_command_reaches_the_caller() {
    let (got, values) = waited_on(&format!("{SETTING}\n{TOUCH}"), embedded, |h| {
        h.commit(None).unwrap();
    });
    assert!(serialization(&got), "{got:?}");
    assert_eq!(vec!["1"], values);
}

#[test]
fn an_embedded_serialization_failure_before_any_command_reruns_the_text() {
    let (got, values) = waited_on(&format!("{TOUCH}\n{SETTING}"), embedded, |h| {
        h.commit(None).unwrap();
    });
    got.expect("the rerun commits");
    assert_eq!(vec!["2"], values);
}

#[test]
fn an_embedded_failed_commit_after_a_command_reaches_the_caller() {
    let (got, logs) = doomed_at_commit(&format!("{SETTING}\n{LOGGED}"), embedded);
    assert!(serialization(&got), "{got:?}");
    assert!(logs.is_empty(), "{logs:?}");
}

#[test]
fn an_embedded_failed_commit_with_no_command_reruns_the_text() {
    let (got, logs) = doomed_at_commit(LOGGED, embedded);
    got.expect("the rerun commits");
    assert_eq!(vec!["1"], logs);
}

/// Advance the simulated clock by `d`.
fn sleep(rt: &SimRuntime, d: Duration) {
    let mut io = rt.io();
    io.submit(Op::Timer { at: rt.now() + d });
    while io.poll().is_none() {
        rt.park();
    }
}

/// Returns what `task` completes with within a simulated second, its output taken.
fn settled(rt: &SimRuntime, task: &Task<SimRuntime>) -> Option<Completion> {
    let waker = Waker::from(Arc::new(Unpark {
        rt: rt.clone(),
        thread: rt.current(),
    }));
    task.supply(usize::MAX);
    for _ in 0..1000 {
        match task.observe(&waker) {
            Observation::Pending => sleep(rt, Duration::from_millis(1)),
            Observation::Layout(_) | Observation::Batch(_) => {}
            Observation::End => task.drained(),
            Observation::Done(done) => return Some(done),
        }
    }
    None
}

const AFTER_COMMAND: &str =
    "set setting RetryLimit = { retries: 5 }\nupdate i in items set { i with value: i.value + 1 }";

struct StoppedAtWait {
    /// Whether the text had yet to complete when `stop` ran.
    waited: bool,
    /// What the text completed with while the holder stayed open.
    done: Option<Completion>,
    setting: Vec<String>,
    values: Vec<String>,
}

/// Run a command and a write of the row a holder has changed, under `opts`, apply `stop` to it
/// once it has run a while, and end the holder only after it has settled or a simulated second
/// has passed.
fn stopped_at_wait(
    opts: impl FnOnce(&SimRuntime) -> CallOptions,
    stop: impl FnOnce(&SimRuntime, &Database<SimRuntime>, &Task<SimRuntime>),
) -> StoppedAtWait {
    once(3, |rt| {
        let db = counted(&rt, 2);
        let session = db.role_authority().session("owner").expect("session");
        let mut holder = db.begin(&TxnOptions::default()).unwrap();
        holder
            .query_text(TOUCH, &[], &TxnCallOptions::default())
            .unwrap();
        let options = TaskOptions {
            call: opts(&rt),
            ..TaskOptions::default()
        };
        let t = session
            .submit_text(AFTER_COMMAND, &[], options)
            .expect("the text is admitted");
        for _ in 0..5000 {
            rt.yield_now();
        }
        let waker = Waker::from(Arc::new(Unpark {
            rt: rt.clone(),
            thread: rt.current(),
        }));
        let waited = matches!(t.observe(&waker), Observation::Pending);
        stop(&rt, &db, &t);
        let done = settled(&rt, &t);
        let open = !matches!(
            &done,
            Some(Completion::Failed(Error::Rejected {
                reason: RejectReason::Closed
            }))
        );
        let (setting, values) = if open {
            (retries(&db), value(&db))
        } else {
            (Vec::new(), Vec::new())
        };
        holder.rollback().unwrap();
        db.close().unwrap();
        StoppedAtWait {
            waited,
            done,
            setting,
            values,
        }
    })
}

fn no_stop(_: &SimRuntime, _: &Database<SimRuntime>, _: &Task<SimRuntime>) {}

fn rejected(done: Option<Completion>, reason: impl Fn(&RejectReason) -> bool) {
    assert!(
        matches!(&done, Some(Completion::Failed(Error::Rejected { reason: r })) if reason(r)),
        "{done:?}"
    );
}

#[test]
fn a_cancel_stops_a_write_waiting_after_a_command() {
    let got = stopped_at_wait(|_| CallOptions::default(), |_, _, t| t.cancel());
    assert!(got.waited, "the text waits on the holder");
    rejected(got.done, |r| matches!(r, RejectReason::Cancelled));
    assert_eq!(vec!["0"], got.values);
    assert_eq!(1, got.setting.len(), "{:?}", got.setting);
}

#[test]
fn a_statement_limit_stops_a_write_waiting_after_a_command() {
    let got = stopped_at_wait(
        |_| CallOptions {
            limit: Some(Duration::from_micros(50)),
            ..CallOptions::default()
        },
        no_stop,
    );
    rejected(got.done, |r| {
        matches!(r, RejectReason::StatementLimit { .. })
    });
    assert_eq!(vec!["0"], got.values);
}

#[test]
fn a_deadline_stops_a_write_waiting_after_a_command() {
    let got = stopped_at_wait(
        |rt| CallOptions {
            deadline: Some(rt.wall() + Duration::from_millis(100)),
            ..CallOptions::default()
        },
        no_stop,
    );
    assert!(got.waited, "the text waits on the holder");
    rejected(got.done, |r| matches!(r, RejectReason::DeadlineExceeded));
    assert_eq!(vec!["0"], got.values);
}

#[test]
fn an_idle_expiry_stops_a_write_waiting_after_a_command() {
    let limit = Duration::from_secs(30);
    let got = stopped_at_wait(|_| CallOptions::default(), |_, _, t| t.expire_idle(limit));
    assert!(got.waited, "the text waits on the holder");
    assert_eq!(
        Some(Completion::Failed(Error::Retry {
            reason: RetryReason::StreamIdle { limit }
        })),
        got.done
    );
}

#[test]
fn closing_the_engine_stops_a_write_waiting_after_a_command() {
    let got = stopped_at_wait(
        |_| CallOptions::default(),
        |_, db, _| db.clone().close().unwrap(),
    );
    assert!(got.waited, "the text waits on the holder");
    rejected(got.done, |r| matches!(r, RejectReason::Closed));
}

const BUMP_FIRST: &str = "update i in items where i.id == items.key(1) set { i with n: i.n + 1 }";
const BUMP_SECOND: &str = "update i in items where i.id == items.key(2) set { i with n: i.n + 1 }";

#[test]
fn a_write_after_a_command_closing_a_wait_cycle_fails_and_frees_the_other_waiter() {
    once(3, |rt| {
        let gate = Arc::new(Gate::default());
        let options = OpenOptions {
            workers: Some(2),
            ..gated_options(&rt, &gate)
        };
        let db = Database::open_in(rt.clone(), "counted.adt", &Log::new([COUNTED]), &options)
            .expect("schema database opens");
        write(&db, "insert items { n: 1 }");
        write(&db, "insert items { n: 2 }");
        let holds = Arc::new(AtomicBool::new(false));
        let other = Arc::new(Mutex::new(None));
        let holder = {
            let (db, gate, holds, other, rt2) = (
                db.clone(),
                gate.clone(),
                holds.clone(),
                other.clone(),
                rt.clone(),
            );
            rt.spawn("holder", move || {
                let mut h = db.begin(&TxnOptions::default()).unwrap();
                h.query_text(BUMP_FIRST, &[], &TxnCallOptions::default())
                    .unwrap();
                holds.store(true, Ordering::Release);
                while !gate.entered.load(Ordering::Acquire) {
                    rt2.yield_now();
                }
                let got = h
                    .query_text(BUMP_SECOND, &[], &TxnCallOptions::default())
                    .map(drop);
                *other.lock().unwrap() = Some(got);
                h.rollback().unwrap();
            })
        };
        while !holds.load(Ordering::Acquire) {
            rt.yield_now();
        }
        let session = db.role_authority().session("owner").expect("session");
        let text =
            format!("{SETTING}\n{BUMP_SECOND}\nfrom i in items select gate(i.n)\n{BUMP_FIRST}");
        let t = session
            .submit_text(
                &text,
                &[],
                TaskOptions {
                    call: CallOptions {
                        limit: Some(Duration::from_secs(10)),
                        ..CallOptions::default()
                    },
                    ..TaskOptions::default()
                },
            )
            .expect("the text is admitted");
        while !gate.entered.load(Ordering::Acquire) {
            rt.yield_now();
        }
        for _ in 0..5000 {
            rt.yield_now();
        }
        gate.open.store(true, Ordering::Release);
        let got = finish(&rt, &t);
        assert!(serialization(&got), "{got:?}");
        rt.join(holder);
        let other = other.lock().unwrap().take().expect("the holder ran");
        other.expect("the other waiter proceeds once the text fails");
        assert_eq!(1, retries(&db).len());
        db.close().unwrap();
    });
}

/// Returns the text of bundle `name` pinned at `ITEMS_OF_VALUE`.
fn value_bundle(name: &str) -> Registration {
    let file = parse_migration(ITEMS_OF_VALUE, ParseOptions::default())
        .ok()
        .expect("the schema parses");
    let source = Source::default()
        .migrate(&file, ITEMS_OF_VALUE)
        .expect("the schema applies");
    let schema = source.schema().expect("the source builds");
    let pins: String = version_hash(&schema, &source)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let text = format!(
        "bundle {name} pins 1 ({pins}) edition \"2026-10\"\nimport example\nquery values () -> Set I64 = from i in items select i.value\n"
    );
    Registration::bundle(text).expect("a bundle file")
}

fn bundle_states(db: &Database<SimRuntime>) -> Vec<String> {
    query(
        db,
        r#"from b in catalog::bundles select b.name ++ " " ++ (match b.state
  | Active -> "active"
  | Superseded _ -> "superseded"
  | Retired _ -> "retired")"#,
    )
}

/// What a text did on a database of two bundles: the transaction ids it took, what it completed
/// with, the row's value and the bundles' states.
type BundleRun = (u64, Completion, Vec<String>, Vec<String>);

/// Run `text`, beside a holder of the row it updates that `end` ends once it waits, if given.
fn on_bundles(text: &str, end: Option<fn(adtb::AdHocTransaction<'_, SimRuntime>)>) -> BundleRun {
    once(3, |rt| {
        let options = OpenOptions {
            registrations: vec![value_bundle("web-1.0"), value_bundle("web-1.1")],
            ..options(2)
        };
        let db = Database::open_in(
            rt.clone(),
            "bundled.adt",
            &Log::new([ITEMS_OF_VALUE]),
            &options,
        )
        .expect("schema database opens");
        results(&db, "insert items { value: 0 }");
        let session = db.role_authority().session("owner").expect("session");
        let before = xids_taken(&db);
        let (done, xids) = match end {
            None => {
                let t = session
                    .submit_text(text, &[], TaskOptions::default())
                    .expect("the text is admitted");
                let done = completion(&rt, &t);
                (done, xids_taken(&db) - before)
            }
            Some(end) => {
                let mut holder = db.begin(&TxnOptions::default()).unwrap();
                holder
                    .query_text(TOUCH, &[], &TxnCallOptions::default())
                    .unwrap();
                let before = xids_taken(&db);
                let t = session
                    .submit_text(text, &[], TaskOptions::default())
                    .expect("the text is admitted");
                for _ in 0..5000 {
                    rt.yield_now();
                }
                let waker = Waker::from(Arc::new(Unpark {
                    rt: rt.clone(),
                    thread: rt.current(),
                }));
                assert!(
                    matches!(t.observe(&waker), Observation::Pending),
                    "the text waits on the holder"
                );
                end(holder);
                let done = completion(&rt, &t);
                (done, xids_taken(&db) - before)
            }
        };
        let values = value(&db);
        let states = bundle_states(&db);
        db.close().unwrap();
        (xids, done, values, states)
    })
}

fn a_bundle_command_before_a_write_takes_effect_once(command: &str, states: &[&str]) {
    let text = format!("{command}\n{TOUCH}");
    let (alone, done, values, seen) = on_bundles(&text, None);
    assert!(matches!(done, Completion::Committed { .. }), "{done:?}");
    assert_eq!(vec!["1"], values);
    assert_eq!(states, seen);

    let (xids, done, values, seen) = on_bundles(&text, Some(|h| h.rollback().unwrap()));
    assert!(matches!(done, Completion::Committed { .. }), "{done:?}");
    assert_eq!(alone, xids, "no second attempt");
    assert_eq!(vec!["1"], values);
    assert_eq!(states, seen);

    let (_, done, values, seen) = on_bundles(
        &text,
        Some(|h| {
            h.commit(None).unwrap();
        }),
    );
    assert!(
        matches!(&done, Completion::Failed(e) if e.is_serialization()),
        "{done:?}"
    );
    assert_eq!(vec!["1"], values);
    assert_eq!(states, seen);
}

#[test]
fn a_retire_bundle_before_a_waiting_write_takes_effect_once() {
    a_bundle_command_before_a_write_takes_effect_once(
        "retire bundle web-1.0",
        &["\"web-1.0 retired\"", "\"web-1.1 active\""],
    );
}

#[test]
fn a_supersede_bundle_before_a_waiting_write_takes_effect_once() {
    a_bundle_command_before_a_write_takes_effect_once(
        "supersede bundle web-1.0 by web-1.1",
        &["\"web-1.0 superseded\"", "\"web-1.1 active\""],
    );
}
