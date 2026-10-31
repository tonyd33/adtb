//! Ad hoc text of a role: charged against its own grants, and revealing nothing it cannot read.

use std::ffi::{c_int, c_ulong};
use std::path::PathBuf;
use std::sync::Arc;
use std::task::{Wake, Waker};

use adtb::runtime::{ThreadId, Threads};
use adtb::{
    Capability, Completion, Database, Error, Log, Observation, OpenOptions, RejectReason,
    RoleSession, Task, TaskOptions, Value,
};
use adtb_lang::schema::ValueDef;
use adtb_lang::types::{Base, RelId, Ty, TypeId};
use adtb_sim::{Config, Sim, SimRuntime};
use adtb_simkit::api::{options, query, run};

const SCHEMA: &str = r#"module shop
add module shop
add type Doc { id: Key docs, owner: Text } = Draft { body: Text } | Sent { to: Text }
add relation docs : Doc key id generate sequence
add type Secret = { id: Key secrets, salary: I64 }
add relation secrets : Secret key id generate sequence
define fn salaries () -> Set I64 = from s in secrets select s.salary
define fn pure_answer () -> I64 = 42
define fn maybe (x: I64) -> Option I64 = if x > 0 then Some { value: x } else None
add relation doc_rejects : RejectedRow key (load, line)
add type Note = { id: Key notes, about: Key secrets, doc: Key docs.Draft }
add relation notes : Note key id generate sequence
add constraint no_blank = from d in docs.Draft where d.body == "" select d.owner
add constraint no_negative = from s in secrets where s.salary < 0 select s.id
"#;

const GRANTS: &str = "module shop
grant read docs.Draft (id, owner, body) to reader
grant read docs.Draft (owner) to narrow
grant insert docs.Draft to inserter
grant read docs.Draft to updater
grant update docs.Draft (body) to updater
grant read docs.Draft to frozen
grant read docs.Sent to deleter
grant delete docs.Sent to deleter
grant read docs to retagger
grant retag docs.Draft -> docs.Sent to retagger
grant read docs.Draft to writer
grant insert docs.Draft to writer
grant read secrets (id) to outer
grant insert secrets to payer
grant read docs to upserter
grant insert docs.Draft to upserter
grant update docs to upserter
grant retag docs.Sent -> docs.Draft to upserter
grant read docs.Draft to loader
grant insert secrets to loader
grant insert doc_rejects to loader
grant read doc_rejects to loader
grant call fn salaries to caller_fn
grant call fn pure_answer to caller_fn
grant call fn maybe to reader
grant read notes to noter
";

const ROLES: [&str; 17] = [
    "noter",
    "upserter",
    "loader",
    "reader",
    "narrow",
    "inserter",
    "updater",
    "frozen",
    "deleter",
    "retagger",
    "writer",
    "outer",
    "payer",
    "caller_fn",
    "bare",
    "other",
    "cataloged",
];

struct Unpark {
    rt: SimRuntime,
    thread: ThreadId,
}

impl Wake for Unpark {
    fn wake(self: Arc<Self>) {
        self.rt.unpark(self.thread);
    }
}

fn finish(rt: &SimRuntime, task: &Task<SimRuntime>) -> Result<Vec<String>, Error> {
    let waker = Waker::from(Arc::new(Unpark {
        rt: rt.clone(),
        thread: rt.current(),
    }));
    task.supply(usize::MAX);
    let mut rows = Vec::new();
    loop {
        match task.observe(&waker) {
            Observation::Pending => rt.park(),
            Observation::Layout(_) => {}
            Observation::Batch(b) => {
                let batch = b.rows().expect("a task of values");
                rows.extend(batch.iter().map(|v| format!("{v:?}")));
            }
            Observation::End => task.drained(),
            Observation::Done(Completion::Failed(e)) => return Err(e),
            Observation::Done(_) => return Ok(rows),
        }
    }
}

fn open_at(rt: &SimRuntime, opts: &OpenOptions, log: &[&str]) -> Database<SimRuntime> {
    Database::open_in(
        rt.clone(),
        "adhoc.adt",
        &Log::new(log.iter().copied()),
        opts,
    )
    .expect("the database opens")
}

/// Returns the database with every role holding `adhoc` and each its grants, and one row of each
/// constructor and one secret.
fn opened(rt: &SimRuntime, opts: &OpenOptions) -> Database<SimRuntime> {
    let db = open_at(rt, opts, &[SCHEMA]);
    let authority = db.role_authority();
    for role in ROLES {
        authority.create_role(role).expect("the role");
        authority
            .grant_capability(role, Capability::Adhoc, false)
            .expect("adhoc");
    }
    db.close().expect("close");
    let db = open_at(rt, opts, &[SCHEMA, GRANTS]);
    db.query_text(
        r#"insert docs.Draft { owner: "ann", body: "text" }
insert docs.Sent { owner: "bob", to: "cid" }
insert secrets { salary: 7 }"#,
        &[],
        &adtb::CallOptions::default(),
    )
    .expect("the rows");
    db
}

fn secrets_id(db: &Database<SimRuntime>) -> RelId {
    let schema = db.schema();
    let module = schema.module_named("shop").expect("the module");
    match schema.module(module).values.get("secrets") {
        Some(ValueDef::Relation(r)) => *r,
        other => panic!("{other:?}"),
    }
}

fn session(db: &Database<SimRuntime>, role: &str) -> RoleSession<SimRuntime> {
    db.role_authority().session(role).expect("the session")
}

fn send(
    rt: &SimRuntime,
    db: &Database<SimRuntime>,
    role: &str,
    text: &str,
) -> Result<Vec<String>, Error> {
    let task = session(db, role).submit_text(text, &[], TaskOptions::default())?;
    finish(rt, &task)
}

fn detail(got: Result<Vec<String>, Error>) -> String {
    match got {
        Err(Error::Rejected {
            reason: RejectReason::Denied { detail },
        }) => detail,
        other => panic!("not denied: {other:?}"),
    }
}

fn denied_rows(db: &Database<SimRuntime>, role: &str) -> usize {
    query(
        db,
        &format!("from a in catalog::audit.Denied where a.role == \"{role}\" select a.seq"),
    )
    .len()
}

fn call_rows(db: &Database<SimRuntime>, role: &str) -> usize {
    query(
        db,
        &format!("from a in catalog::audit.Call where a.role == \"{role}\" select a.seq"),
    )
    .len()
}

fn allowed(rt: &SimRuntime, db: &Database<SimRuntime>, role: &str, text: &str) -> Vec<String> {
    send(rt, db, role, text).unwrap_or_else(|e| panic!("{role}: `{text}`: {e:?}"))
}

fn scenario<T>(seed: u64, body: impl FnOnce(SimRuntime, Database<SimRuntime>) -> T) -> T {
    let mut sim = Sim::new(seed, Config::default());
    run(&mut sim, |rt| {
        let db = opened(&rt, &options(2));
        body(rt, db)
    })
}

/// The catalog relations a role reads to read the schema.
const CATALOG_READS: [&str; 15] = [
    "types",
    "constructors",
    "ctor_states",
    "value_states",
    "fields",
    "relations",
    "named_sets",
    "set_members",
    "indexes",
    "index_ctors",
    "constraints",
    "functions",
    "defaults",
    "computed",
    "grants",
];

const READ_DRAFT: &str = "from d in docs.Draft select d.body";
const READ_DOCS: &str = "from d in docs select d.owner";

#[test]
fn a_read_passes_only_within_constructors_and_fields_held() {
    scenario(601, |rt, db| {
        assert_eq!(1, allowed(&rt, &db, "reader", READ_DRAFT).len());
        detail(send(&rt, &db, "reader", READ_DOCS));
        detail(send(&rt, &db, "reader", "from d in docs.Sent select d.to"));
        assert_eq!(
            1,
            allowed(&rt, &db, "narrow", "from d in docs.Draft select d.owner").len()
        );
        detail(send(&rt, &db, "narrow", READ_DRAFT));
        detail(send(&rt, &db, "narrow", "from d in docs.Draft select d"));
    });
}

#[test]
fn a_text_it_may_not_run_is_refused_at_the_send() {
    scenario(630, |_, db| {
        let got = session(&db, "reader").submit_text(READ_DOCS, &[], TaskOptions::default());
        assert!(got.is_err(), "{got:?}");
    });
}

#[test]
fn an_insert_passes_with_its_grant_on_that_constructor_only() {
    scenario(602, |rt, db| {
        let draft = r#"insert docs.Draft { owner: "z", body: "b" }"#;
        allowed(&rt, &db, "inserter", draft);
        detail(send(
            &rt,
            &db,
            "inserter",
            r#"insert docs.Sent { owner: "z", to: "b" }"#,
        ));
        detail(send(&rt, &db, "reader", draft));
        assert_eq!(2, query(&db, "from d in docs.Draft select d.id").len());
    });
}

#[test]
fn an_update_passes_only_on_the_listed_field() {
    scenario(603, |rt, db| {
        let body = r#"update d in docs.Draft set Draft { d with body: "new" }"#;
        let owner = r#"update d in docs.Draft set Draft { d with owner: "new" }"#;
        allowed(&rt, &db, "updater", body);
        detail(send(&rt, &db, "updater", owner));
        detail(send(&rt, &db, "frozen", body));
        assert_eq!(
            vec!["new".to_owned()],
            query(&db, "from d in docs.Draft select d.owner")
                .iter()
                .map(|_| "new".to_owned())
                .collect::<Vec<_>>()
        );
    });
}

#[test]
fn a_delete_passes_with_its_grant_on_that_constructor_only() {
    scenario(604, |rt, db| {
        detail(send(&rt, &db, "deleter", "delete d in docs.Draft"));
        allowed(&rt, &db, "deleter", "delete d in docs.Sent");
        assert_eq!(0, query(&db, "from d in docs.Sent select d.id").len());
        assert_eq!(1, query(&db, "from d in docs.Draft select d.id").len());
    });
}

#[test]
fn a_retag_passes_only_along_the_pair_granted() {
    scenario(605, |rt, db| {
        let back = r#"update d in docs.Sent set Draft { body: "b" }"#;
        detail(send(&rt, &db, "retagger", back));
        allowed(
            &rt,
            &db,
            "retagger",
            r#"update d in docs.Draft set Sent { to: "t" }"#,
        );
        assert_eq!(2, query(&db, "from d in docs.Sent select d.id").len());
    });
}

#[test]
fn a_role_without_adhoc_is_refused_the_same_whatever_it_names() {
    scenario(606, |rt, db| {
        let authority = db.role_authority();
        authority
            .revoke_capability("bare", Capability::Adhoc)
            .expect("revoke");
        let one = detail(send(&rt, &db, "bare", "from s in secrets select s"));
        let two = detail(send(&rt, &db, "bare", "from s in nosuch select s"));
        assert_eq!(one, two);
        assert_eq!(2, denied_rows(&db, "bare"));
    });
}

#[test]
fn a_relation_it_cannot_read_and_one_that_does_not_exist_answer_the_same() {
    fn doubling_over(field: &str) -> String {
        let mut text =
            format!("vacuum secrets\ntransact let a0 = (from s in secrets select s.{field})\n");
        for i in 1..=33 {
            let p = i - 1;
            text.push_str(&format!("let a{i} = a{p} union a{p}\n"));
        }
        text.push_str("return a33");
        text
    }
    let (over_salary, over_salry) = (doubling_over("salary"), doubling_over("salry"));
    scenario(607, |rt, db| {
        for role in ["bare", "reader"] {
            db.role_authority()
                .grant_capability(role, Capability::Maintain, false)
                .expect("maintain");
            for (present, absent) in [
                (
                    "vacuum secrets\ndelete s in secrets where s.salary == 1",
                    "vacuum secrets\ndelete s in secrets where s.salry == 1",
                ),
                (
                    "vacuum secrets\n(secrets.key(1))",
                    "vacuum secrets\n(secrets.key(\"x\"))",
                ),
                (
                    "vacuum secrets\n(secrets.key(id: 1))",
                    "vacuum secrets\n(secrets.key(idd: 1))",
                ),
                (
                    "vacuum notes\ntemp relation t : { n: Key notes.Note } key n",
                    "vacuum notes\ntemp relation t : { n: Key notes.Nosuch } key n",
                ),
                (over_salary.as_str(), over_salry.as_str()),
                (
                    "vacuum secrets\ncancel call secrets.key(1)",
                    "vacuum secrets\ncancel call secrets.key(\"x\")",
                ),
                ("cancel call shop::x.key(1)", "cancel call nomod::x.key(1)"),
                ("from s in secrets select s", "from s in nosuch select s"),
                (
                    "from s in secrets select s.salary",
                    "from s in secrets select s.nope",
                ),
                ("secrets.key(1)", "nosuch.key(1)"),
                (
                    "Secret { id: secrets.key(1), salary: 2 }",
                    "Nosuch { id: secrets.key(1), salary: 2 }",
                ),
                (
                    "(Secret { id: secrets.key(1), salary: 2 }).salary",
                    "(Secret { id: secrets.key(1), salary: 2 }).salry",
                ),
                (
                    "temp relation t : { n: Key secrets } key n",
                    "temp relation t : { n: Key nosuch } key n",
                ),
                (
                    "temp relation t : Secret key id",
                    "temp relation t : Nosuch key id",
                ),
                ("salaries()", "nosuchfn()"),
                ("no_negative", "nosuch"),
                ("delete s in secrets", "delete s in nosuch"),
                (
                    "explain from s in secrets select s",
                    "explain from s in nosuch select s",
                ),
                (
                    "export (from s in secrets select s) to \"x.csv\"",
                    "export (from s in nosuch select s) to \"x.csv\"",
                ),
            ] {
                if role == "bare" || !present.starts_with("export") {
                    let before = denied_rows(&db, role);
                    assert_eq!(
                        detail(send(&rt, &db, role, present)),
                        detail(send(&rt, &db, role, absent)),
                        "{role}: {present}"
                    );
                    assert_eq!(before + 2, denied_rows(&db, role), "{role}: {present}");
                }
            }
        }
        assert_eq!(0, call_rows(&db, "bare"));
    });
}

#[test]
fn a_text_that_fails_to_parse_answers_invalid() {
    scenario(608, |rt, db| {
        let got = format!("{:?}", send(&rt, &db, "bare", "from from"));
        assert!(got.contains("Invalid"), "{got}");
    });
}

#[test]
fn a_revoke_between_two_sends_is_seen_by_the_second() {
    let mut sim = Sim::new(609, Config::default());
    run(&mut sim, |rt| {
        let opts = options(2);
        let db = opened(&rt, &opts);
        allowed(&rt, &db, "reader", READ_DRAFT);
        let owner = session(&db, "owner");
        let authority = db.role_authority();
        authority
            .grant(&owner, "grant read catalog::roles to cataloged")
            .expect("grant");
        let roles = "from r in catalog::roles select r.name";
        allowed(&rt, &db, "cataloged", roles);
        authority
            .revoke(&owner, "revoke read catalog::roles from cataloged")
            .expect("revoke");
        detail(send(&rt, &db, "cataloged", roles));
        allowed(&rt, &db, "reader", READ_DRAFT);
        db.close().expect("close");
        let revoke = "module shop\nrevoke read docs.Draft (id, owner, body) from reader\n";
        let db = open_at(&rt, &opts, &[SCHEMA, GRANTS, revoke]);
        detail(send(&rt, &db, "reader", READ_DRAFT));
    });
}

#[test]
fn a_parameter_cannot_widen_what_is_read() {
    scenario(611, |rt, db| {
        let reader = session(&db, "reader");
        let key = Ty::Key {
            rel: secrets_id(&db),
            ctors: None,
        };
        let params = [("k", key, Value::Int(1))];
        let got = reader.submit_text(
            "k.salary",
            &[(params[0].0, params[0].1.clone(), params[0].2.clone())],
            TaskOptions::default(),
        );
        detail(got.and_then(|t| finish(&rt, &t)));
        let text = Ty::Base(Base::Text);
        for name in ["caller", "secrets", "salaries"] {
            let got = reader.submit_text(
                "from s in secrets select s.salary",
                &[(name, text.clone(), Value::text("x"))],
                TaskOptions::default(),
            );
            detail(got.and_then(|t| finish(&rt, &t)));
        }
    });
}

#[test]
fn a_parameter_of_a_type_the_schema_does_not_hold_is_refused_with_a_row() {
    scenario(629, |_, db| {
        let bogus = Ty::Key {
            rel: RelId(u32::MAX),
            ctors: None,
        };
        let got = session(&db, "reader")
            .submit_text("k", &[("k", bogus, Value::Int(1))], TaskOptions::default())
            .map(drop);
        let denied = format!("{got:?}");
        assert!(denied.contains("may not run this text"), "{denied}");
        assert_eq!(1, denied_rows(&db, "reader"));
        let mismatched = session(&db, "reader")
            .submit_text(
                "k",
                &[("k", Ty::Base(Base::I64), Value::text("no"))],
                TaskOptions::default(),
            )
            .map(drop);
        let shown = format!("{mismatched:?}");
        assert!(shown.contains("does not print"), "{shown}");
        assert_eq!(1, call_rows(&db, "reader"));
    });
}

#[test]
fn caller_names_the_session_s_role_whatever_a_parameter_says() {
    scenario(612, |rt, db| {
        let reader = session(&db, "reader");
        let got = reader
            .submit_text(
                "caller",
                &[("caller", Ty::Base(Base::Text), Value::text("owner"))],
                TaskOptions::default(),
            )
            .and_then(|t| finish(&rt, &t));
        let got = format!("{got:?}");
        assert!(got.contains("reader"), "{got}");
        assert!(!got.contains("owner"), "{got}");
    });
}

#[test]
fn a_transact_block_with_one_forbidden_item_runs_nothing() {
    scenario(613, |rt, db| {
        let text = r#"transact
  insert docs.Draft { owner: "x", body: "y" }
  delete s in secrets
"#;
        detail(send(&rt, &db, "writer", text));
        assert_eq!(1, query(&db, "from d in docs.Draft select d.id").len());
        assert_eq!(1, query(&db, "from s in secrets select s.id").len());
        assert_eq!(1, denied_rows(&db, "writer"));
    });
}

#[test]
fn several_items_with_one_forbidden_run_nothing() {
    scenario(614, |rt, db| {
        let text = "insert docs.Draft { owner: \"x\", body: \"y\" }\nfrom s in secrets select s";
        detail(send(&rt, &db, "writer", text));
        assert_eq!(1, query(&db, "from d in docs.Draft select d.id").len());
    });
}

#[test]
fn a_temp_relation_needs_no_grant_but_what_fills_it_does() {
    scenario(615, |rt, db| {
        let own = "temp relation stage : { n: I64 } key n\ninsert stage { n: 1 }\nfrom s in stage select s.n";
        assert_eq!(1, allowed(&rt, &db, "bare", own).len());
        let fill = "temp relation stage : { n: I64 } key n\ninsert stage from (from s in secrets select { n: s.salary })\nfrom s in stage select s.n";
        detail(send(&rt, &db, "bare", fill));
        let reading =
            "temp relation stage : { n: Key secrets } key n\nfrom s in stage select s.n.salary";
        detail(send(&rt, &db, "outer", reading));
    });
}

#[test]
fn a_schema_function_s_reads_are_charged_to_the_role_calling_it() {
    scenario(616, |rt, db| {
        detail(send(&rt, &db, "bare", "salaries()"));
        detail(send(&rt, &db, "caller_fn", "salaries()"));
    });
}

#[test]
fn a_violation_redacts_what_the_role_cannot_read() {
    scenario(617, |rt, db| {
        let blank = r#"insert docs.Draft { owner: "x", body: "" }"#;
        let seen = format!("{:?}", send(&rt, &db, "writer", blank));
        assert!(seen.contains("no_blank"), "{seen}");
        assert!(seen.contains("redacted: 0"), "{seen}");
        assert!(seen.contains('x'), "{seen}");
        let hidden = send(&rt, &db, "inserter", blank);
        let shown = format!("{hidden:?}");
        assert!(shown.contains("no_blank"), "{shown}");
        assert!(shown.contains("redacted: 1"), "{shown}");
        assert!(shown.contains("report: []"), "{shown}");
        let audit = query(
            &db,
            "from a in catalog::audit.Call where a.role == \"inserter\" select a.outcome",
        );
        assert_eq!(1, audit.len());
        assert!(audit[0].contains("redacted"), "{audit:?}");
        assert!(!audit[0].contains("\"x\""), "{audit:?}");
        let payer = send(&rt, &db, "payer", "insert secrets { salary: -1 }");
        assert!(format!("{payer:?}").contains("redacted: 1"), "{payer:?}");
    });
}

#[test]
fn every_call_of_a_role_has_its_row_with_hash_arguments_and_outcome() {
    scenario(618, |rt, db| {
        allowed(&rt, &db, "reader", READ_DRAFT);
        assert_eq!(1, call_rows(&db, "reader"));
        detail(send(&rt, &db, "reader", READ_DOCS));
        assert_eq!(1, denied_rows(&db, "reader"));
        assert_eq!(1, call_rows(&db, "reader"));
        let rows = query(
            &db,
            "from a in catalog::audit.Call where a.role == \"reader\" select { o: a.origin, x: a.args, y: a.outcome }",
        );
        assert!(rows[0].contains("AdHoc"), "{rows:?}");
        assert!(rows[0].contains("Succeeded"), "{rows:?}");
        let denied = query(
            &db,
            "from a in catalog::audit.Denied where a.role == \"reader\" select a.origin",
        );
        assert!(denied[0].contains("AdHoc"), "{denied:?}");
    });
}

#[test]
fn explain_is_as_for_the_statement_and_hides_what_needs_more() {
    scenario(619, |rt, db| {
        let plan = allowed(
            &rt,
            &db,
            "reader",
            "explain from d in docs.Draft select d.body",
        );
        assert!(
            !plan[0].contains("Sent"),
            "a constructor it cannot read: {plan:?}"
        );
        detail(send(
            &rt,
            &db,
            "reader",
            "explain from d in docs select d.owner",
        ));
        let analyzed = allowed(
            &rt,
            &db,
            "writer",
            "explain analyze insert docs.Draft { owner: \"o\", body: \"b\" }",
        );
        assert!(!analyzed[0].contains("actual"), "{analyzed:?}");
        assert_eq!(1, query(&db, "from d in docs.Draft select d.id").len());
        db.role_authority()
            .grant_capability("writer", Capability::Monitor, false)
            .expect("monitor");
        let analyzed = allowed(
            &rt,
            &db,
            "writer",
            "explain analyze insert docs.Draft { owner: \"o\", body: \"b\" }",
        );
        assert!(analyzed[0].contains("actual"), "{analyzed:?}");
    });
}

#[test]
fn explain_of_a_write_shows_no_constraint_of_a_relation_it_cannot_read() {
    scenario(620, |rt, db| {
        let plan = allowed(&rt, &db, "payer", "explain insert secrets { salary: 1 }");
        assert!(!plan[0].contains("salary\\\":7"), "{plan:?}");
        assert!(!plan[0].contains("docs"), "{plan:?}");
        assert!(!plan[0].contains("no_negative"), "{plan:?}");
    });
}

#[test]
fn print_schema_needs_a_read_of_the_catalog() {
    scenario(621, |rt, db| {
        detail(send(&rt, &db, "bare", "print schema"));
        let owner = session(&db, "owner");
        let authority = db.role_authority();
        for relation in CATALOG_READS {
            authority
                .grant(&owner, &format!("grant read catalog::{relation} to bare"))
                .expect("grant");
        }
        let printed = allowed(&rt, &db, "bare", "print schema");
        assert!(printed[0].contains("relation secrets"), "{printed:?}");
        assert!(printed[0].contains("version 2"), "{printed:?}");
        let got = format!(
            "{:?}",
            send(&rt, &db, "bare", "from s in secrets select s.nope")
        );
        assert!(
            got.contains("nope"),
            "a role reading the schema gets the checker: {got}"
        );
    });
}

#[test]
fn print_schema_runs_for_the_owner_in_process() {
    scenario(622, |_, db| {
        let printed = query(&db, "print schema");
        assert!(printed[0].contains("relation docs"), "{printed:?}");
    });
}

/// Returns a call of `session` that stays running until it is drained: nothing is supplied.
fn held(session: &RoleSession<SimRuntime>) -> Task<SimRuntime> {
    session
        .submit_text(READ_DRAFT, &[], TaskOptions::default())
        .expect("admitted")
}

fn cancelled(rt: &SimRuntime, task: &Task<SimRuntime>) {
    let got = finish(rt, task);
    assert!(
        matches!(
            got,
            Err(Error::Rejected {
                reason: RejectReason::Cancelled
            })
        ),
        "{got:?}"
    );
}

#[test]
fn cancel_call_of_another_session_needs_cancel_and_hides_whether_it_exists() {
    scenario(623, |rt, db| {
        let long = held(&session(&db, "reader"));
        let id = long.call().expect("a role's call has a key");
        let text = format!("cancel call sys::calls.key({id})");
        let unknown = "cancel call sys::calls.key(987654)";
        let refused = detail(send(&rt, &db, "bare", &text));
        assert_eq!(refused, detail(send(&rt, &db, "bare", unknown)));
        assert_eq!(2, denied_rows(&db, "bare"));
        db.role_authority()
            .grant_capability("bare", Capability::Cancel, false)
            .expect("cancel");
        assert_eq!(0, allowed(&rt, &db, "bare", unknown).len());
        assert_eq!(1, allowed(&rt, &db, "bare", &text).len());
        cancelled(&rt, &long);
    });
}

#[test]
fn a_session_cancels_its_own_call_and_no_other_session_of_its_role() {
    scenario(624, |rt, db| {
        let reader = session(&db, "reader");
        let long = held(&reader);
        let text = format!("cancel call sys::calls.key({})", long.call().expect("key"));
        detail(send(&rt, &db, "reader", &text));
        let task = reader
            .submit_text(&text, &[], TaskOptions::default())
            .expect("admitted");
        assert_eq!(1, finish(&rt, &task).expect("its own call").len());
        cancelled(&rt, &long);
    });
}

#[test]
fn verify_witnesses_need_a_read_of_what_the_constraint_reads() {
    scenario(625, |rt, db| {
        for role in ["reader", "bare"] {
            db.role_authority()
                .grant_capability(role, Capability::Maintain, false)
                .expect("maintain");
        }
        allowed(&rt, &db, "bare", "verify");
        allowed(&rt, &db, "reader", "verify");
    });
}

#[test]
fn export_writes_within_export_roots_and_needs_what_its_source_reads() {
    let dir = adtb_testkit::scratch::Scratch::new("adhoc-export", "x");
    let base = dir.path().to_path_buf();
    std::fs::create_dir_all(base.join("out")).expect("a directory");
    let mut sim = Sim::new(626, Config::default());
    run(&mut sim, |rt| {
        let opts = OpenOptions {
            file_root: Some(base.clone()),
            export_roots: vec![PathBuf::from("out")],
            ..options(2)
        };
        let db = opened(&rt, &opts);
        let text =
            "export (from d in docs.Draft select { o: d.owner, b: d.body }) to \"out/d.csv\"";
        detail(send(&rt, &db, "reader", text));
        assert!(!base.join("out/d.csv").exists());
        db.role_authority()
            .grant_capability("reader", Capability::WriteFiles, false)
            .expect("write_files");
        let got = allowed(&rt, &db, "reader", text);
        assert!(got[0].contains("out/d.csv"), "{got:?}");
        assert_eq!(
            "b,o\ntext,ann\n",
            std::fs::read_to_string(base.join("out/d.csv")).expect("the export")
        );
        let outside = "export (from d in docs.Draft select { b: d.body }) to \"d.csv\"";
        let before = denied_rows(&db, "reader");
        let why = detail(send(&rt, &db, "reader", outside));
        assert!(why.contains("export_roots"), "{why}");
        assert_eq!(before + 1, denied_rows(&db, "reader"));
        assert!(!base.join("d.csv").exists());
        let secret = "export (from s in secrets select { a: s.salary }) to \"out/s.csv\"";
        detail(send(&rt, &db, "reader", secret));
        assert!(!base.join("out/s.csv").exists());
    });
}

#[test]
fn a_definer_s_rights_do_not_reach_text_of_the_same_session() {
    scenario(627, |rt, db| {
        let owner = session(&db, "owner");
        assert_eq!(
            1,
            finish(
                &rt,
                &owner
                    .submit_text(
                        "from s in secrets select s.salary",
                        &[],
                        TaskOptions::default()
                    )
                    .expect("owner")
            )
            .expect("owner reads")
            .len()
        );
        detail(send(
            &rt,
            &db,
            "caller_fn",
            "from s in secrets select s.salary",
        ));
    });
}

#[test]
fn sys_relations_need_monitor_and_a_revealing_column_needs_every_read() {
    scenario(628, |rt, db| {
        let calls = "from c in sys::calls select c.id";
        detail(send(&rt, &db, "bare", calls));
        db.role_authority()
            .grant_capability("bare", Capability::Monitor, false)
            .expect("monitor");
        allowed(&rt, &db, "bare", calls);
        detail(send(&rt, &db, "bare", "from s in secrets select s.salary"));
        detail(send(
            &rt,
            &db,
            "bare",
            "from v in sys::field_values select v.value",
        ));
    });
}

fn secret_type(db: &Database<SimRuntime>) -> TypeId {
    let schema = db.schema();
    schema.relation(secrets_id(db)).elem_def
}

fn docs_id(db: &Database<SimRuntime>) -> RelId {
    let schema = db.schema();
    let module = schema.module_named("shop").expect("the module");
    match schema.module(module).values.get("docs") {
        Some(ValueDef::Relation(r)) => *r,
        other => panic!("{other:?}"),
    }
}

fn typed(db: &Database<SimRuntime>, role: &str, text: &str, ty: Ty) -> String {
    let got = session(db, role)
        .submit_text(text, &[("k", ty, Value::Int(1))], TaskOptions::default())
        .map(drop);
    format!("{got:?}")
}

#[test]
fn a_parameter_type_naming_what_the_role_holds_no_grant_on_answers_as_an_absent_one() {
    scenario(640, |_, db| {
        let secret = secret_type(&db);
        let key = Ty::Key {
            rel: secrets_id(&db),
            ctors: None,
        };
        let data = Ty::Data {
            def: secret,
            args: Vec::new(),
            ctors: None,
        };
        let absent_key = Ty::Key {
            rel: RelId(u32::MAX),
            ctors: None,
        };
        let absent_data = Ty::Data {
            def: TypeId(u32::MAX),
            args: Vec::new(),
            ctors: None,
        };
        for role in ["bare", "reader"] {
            let before = denied_rows(&db, role);
            let want = typed(&db, role, "k", absent_key.clone());
            assert!(want.contains("may not run this text"), "{want}");
            assert_eq!(want, typed(&db, role, "k", absent_data.clone()));
            assert_eq!(want, typed(&db, role, "k", key.clone()));
            assert_eq!(want, typed(&db, role, "k.salary", key.clone()));
            assert_eq!(want, typed(&db, role, "k", data.clone()));
            assert_eq!(want, typed(&db, role, "k.salary", data.clone()));
            assert_eq!(before + 6, denied_rows(&db, role));
        }
    });
}

#[test]
fn a_role_names_what_its_grants_reach_and_the_prelude() {
    scenario(641, |rt, db| {
        for text in [
            "temp relation t : Doc key id",
            "temp relation t : { n: Option I64 } key n",
            "from d in docs.Draft select { o: d.owner, b: d.body }",
            r#"Draft { id: docs.key(1), owner: "a", body: "b" }"#,
            "lower(\"A\")",
        ] {
            let got = send(&rt, &db, "reader", text);
            assert!(
                !format!("{got:?}").contains("may not run this text"),
                "{text}: {got:?}"
            );
        }
    });
}

#[test]
fn a_key_parameter_may_name_what_a_field_of_a_held_type_references() {
    scenario(642, |_, db| {
        let docs = Ty::Key {
            rel: docs_id(&db),
            ctors: None,
        };
        let got = typed(&db, "reader", "k", docs);
        assert!(!got.contains("may not run this text"), "{got}");
    });
}

#[test]
fn a_hidden_declaration_of_the_same_name_does_not_make_a_held_name_ambiguous() {
    let shadow = r"module shop
add module shop
add type Doc = { id: Key docs, n: I64 }
add relation docs : Doc key id generate sequence
";
    let vault = r"module vault
add module vault
add type Doc = { id: Key docs, n: I64 }
add relation docs : Doc key id generate sequence
";
    let grant = "module shop\ngrant read docs to reader\n";
    let mut sim = Sim::new(643, Config::default());
    run(&mut sim, |rt| {
        let opts = options(2);
        let db = open_at(&rt, &opts, &[shadow, vault]);
        let authority = db.role_authority();
        authority.create_role("reader").expect("role");
        authority
            .grant_capability("reader", Capability::Adhoc, false)
            .expect("adhoc");
        db.close().expect("close");
        let db = open_at(&rt, &opts, &[shadow, vault, grant]);
        assert!(allowed(&rt, &db, "reader", "from d in docs select d.n").is_empty());
        let owner = db.role_authority().session("owner").expect("session");
        let got = owner
            .submit_text("from d in docs select d.n", &[], TaskOptions::default())
            .map(drop);
        assert!(format!("{got:?}").contains("qualify"), "{got:?}");
    });
}

#[test]
fn a_role_reading_the_schema_is_told_what_it_names_wrong() {
    scenario(644, |rt, db| {
        let owner = session(&db, "owner");
        let authority = db.role_authority();
        for relation in [
            "types",
            "constructors",
            "ctor_states",
            "value_states",
            "fields",
            "relations",
            "named_sets",
            "set_members",
            "indexes",
            "index_ctors",
            "constraints",
            "functions",
            "defaults",
            "computed",
            "grants",
        ] {
            authority
                .grant(
                    &owner,
                    &format!("grant read catalog::{relation} to cataloged"),
                )
                .expect("grant");
        }
        let got = format!(
            "{:?}",
            send(&rt, &db, "cataloged", "from s in nosuch select s")
        );
        assert!(got.contains("Invalid"), "{got}");
        assert!(got.contains("nosuch"), "{got}");
        let why = detail(send(
            &rt,
            &db,
            "cataloged",
            "from s in secrets select s.salary",
        ));
        assert!(why.contains("salary") || why.contains("secrets"), "{why}");
        let why = detail(send(&rt, &db, "cataloged", "pure_answer()"));
        assert!(why.contains("pure_answer"), "{why}");
        let bad = typed(
            &db,
            "cataloged",
            "k",
            Ty::Key {
                rel: RelId(u32::MAX),
                ctors: None,
            },
        );
        assert!(bad.contains("no type of the schema"), "{bad}");
        assert_eq!(
            1,
            call_rows(&db, "cataloged") - 1,
            "the absent name and the bad parameter"
        );
    });
}

#[test]
fn a_function_needs_its_call_grant_even_when_it_reads_nothing() {
    scenario(645, |rt, db| {
        let want = detail(send(&rt, &db, "bare", "nosuchfn()"));
        assert_eq!(want, detail(send(&rt, &db, "bare", "pure_answer()")));
        assert_eq!(
            vec!["Int(42)"],
            allowed(&rt, &db, "caller_fn", "pure_answer()")
        );
    });
}

#[test]
fn an_out_of_range_constructor_in_a_parameter_type_panics_nothing() {
    scenario(646, |_, db| {
        let docs = docs_id(&db);
        let def = db.schema().relation(docs).elem_def;
        let bad = [
            Ty::Key {
                rel: docs,
                ctors: Some(vec![99]),
            },
            Ty::Key {
                rel: docs,
                ctors: Some(Vec::new()),
            },
            Ty::Key {
                rel: docs,
                ctors: Some(vec![1, 0]),
            },
            Ty::Data {
                def,
                args: Vec::new(),
                ctors: Some(vec![99]),
            },
            Ty::Data {
                def,
                args: vec![Ty::Base(Base::I64)],
                ctors: None,
            },
            Ty::Sum {
                ctors: vec![("A".to_owned(), Vec::new())],
                sel: Some(vec![5]),
            },
        ];
        for ty in bad {
            let shown = format!("{ty:?}");
            for text in ["k.owner", "k.A"] {
                let got = typed(&db, "reader", text, ty.clone());
                assert!(got.contains("may not run this text"), "{shown}: {got}");
                let got = session(&db, "owner")
                    .submit_text(
                        text,
                        &[("k", ty.clone(), Value::Int(1))],
                        TaskOptions::default(),
                    )
                    .map(drop);
                let got = format!("{got:?}");
                assert!(got.contains("no type of the schema"), "{shown}: {got}");
            }
        }
    });
}

#[test]
fn a_call_with_a_value_that_does_not_print_hashes_its_parameter_names() {
    scenario(647, |_, db| {
        let hashes_after = |name: &str| {
            let reader = session(&db, "reader");
            let got = reader
                .submit_text(
                    "1",
                    &[(name, Ty::Base(Base::I64), Value::text("no"))],
                    TaskOptions::default(),
                )
                .map(drop);
            assert!(format!("{got:?}").contains("does not print"), "{got:?}");
            let distinct = "from a in catalog::audit.Call where a.role == \"reader\" select a.args";
            query(&db, distinct).len()
        };
        assert_eq!(1, hashes_after("a"));
        assert_eq!(1, hashes_after("a"));
        assert_eq!(2, hashes_after("b"));
    });
}

/// Returns a text that doubles what it binds `n` times, by `kind`.
fn doubling(kind: &str, n: usize) -> String {
    let (start, step): (&str, fn(usize) -> String) = match kind {
        "join" => ("(from d in docs.Draft select d.owner)", |p| {
            format!("from x in a{p}, y in a{p} select x")
        }),
        "record" => ("1", |p| format!("{{ l: a{p}, r: a{p} }}")),
        "set" => ("1", |p| format!("[a{p}, a{p}]")),
        "union" => ("[1]", |p| format!("a{p} union a{p}")),
        "sum" => ("1", |p| format!("a{p} + a{p}")),
        "lambda" => ("fn (x: I64) -> x + 1", |p| {
            format!("fn (x: I64) -> a{p}(a{p}(x))")
        }),
        "match_both" => ("1", |i| {
            format!("match maybe(p) | None -> a{i} | Some s -> a{i}")
        }),
        "match_on_match" => ("maybe(p)", |i| {
            format!("match a{i} | None -> a{i} | Some s -> maybe(s.value + p)")
        }),
        other => panic!("{other}"),
    };
    let mut text = format!("transact let a0 = {start}\n");
    for i in 1..=n {
        text.push_str(&format!("let a{i} = {}\n", step(i - 1)));
    }
    if kind == "lambda" {
        text.push_str(&format!("return a{n}(1)"));
    } else {
        text.push_str(&format!("return a{n}"));
    }
    text
}

/// Returns what `who` gets for the doubling text of `kind`.
fn doubled(seed: u64, kind: &str, n: usize, who: &str) -> String {
    let mut sim = Sim::new(seed, Config::default());
    let text = doubling(kind, n);
    run(&mut sim, |rt| {
        let db = opened(&rt, &options(2));
        let got = session(&db, who)
            .submit_text(
                &text,
                &[("p", Ty::Base(Base::I64), Value::Int(1))],
                TaskOptions::default(),
            )
            .and_then(|task| finish(&rt, &task));
        format!("{got:?}")
    })
}

/// Returns a text deepening its value `n` times by `kind`, with parameter `p` an I64.
fn deepening(kind: &str, n: usize) -> String {
    let (start, step): (&str, fn(usize) -> String) = match kind {
        "lambda" => ("fn (x: I64) -> x + p", |i| {
            format!("fn (x: I64) -> a{i}(x)")
        }),
        "lambda_set" => ("fn (x: I64) -> [x + p]", |i| {
            format!("fn (x: I64) -> from y in a{i}(x) select y + 1")
        }),
        "prim" => ("p", |i| format!("a{i} + p")),
        "if" => ("p", |i| format!("if p > 0 then a{i} else 0")),
        "match" => ("p", |i| {
            format!("match maybe(p) | None -> 0 | Some s -> a{i} + s.value")
        }),
        "record" => ("p", |i| format!("{{ v: a{i} }}")),
        "set" => ("p", |i| format!("[a{i}]")),
        "from" => ("[p]", |i| format!("from y in a{i} select y + 1")),
        "arms" => ("p", |_| String::new()),
        other => panic!("{other}"),
    };
    if kind == "arms" {
        let arms: String = (0..n).map(|i| format!("| {i} -> {i} ")).collect();
        return format!("match p {arms}| _ -> 0");
    }
    let mut text = format!("transact let a0 = {start}\n");
    for i in 1..=n {
        text.push_str(&format!("let a{i} = {}\n", step(i - 1)));
    }
    if kind.starts_with("lambda") {
        text.push_str(&format!("return a{n}(1)"));
    } else {
        text.push_str(&format!("return a{n}"));
    }
    text
}

fn deep_answer(
    rt: &SimRuntime,
    db: &Database<SimRuntime>,
    who: &str,
    kind: &str,
    n: usize,
) -> Result<Vec<String>, Error> {
    let task = session(db, who).submit_text(
        &deepening(kind, n),
        &[("p", Ty::Base(Base::I64), Value::Int(1))],
        TaskOptions::default(),
    )?;
    finish(rt, &task)
}

const PLANS: &str = "the text expands past what the engine plans";
const NORMALISES: &str = "the text expands past what the engine normalises";
const TYPE_TOO_LARGE: &str = "the type of this expression is too large";

/// The stack the deepening tests run on, in bytes. Stands for under 2 MiB in a dev build, where a
/// path through planning and running takes up to 11.4 times what it takes in the test profile.
const REDUCED_STACK: &str = "174080";

/// The stack of a chain of functions, a nesting of fixpoints or fused loops, in bytes. Stands for
/// under 2 MiB in a dev build, where each takes up to 5.7 times what it takes in the test profile.
const FUNCTION_CHAIN_STACK: &str = "368640";

const PR_SET_PDEATHSIG: c_int = 1;
const SIGKILL: c_ulong = 9;

unsafe extern "C" {
    fn prctl(option: c_int, ...) -> c_int;
}

/// Set one prctl option of the calling process.
fn set_process_option(option: c_int, value: c_ulong) -> std::io::Result<()> {
    // SAFETY: both options take one integer argument and change only the calling process.
    match unsafe { prctl(option, value) } {
        0 => Ok(()),
        _ => Err(std::io::Error::last_os_error()),
    }
}

/// Run `body` of the test `name` in a child of this test binary on threads of `REDUCED_STACK`.
fn on_reduced_stack(name: &str, body: impl FnOnce()) {
    on_stack(REDUCED_STACK, name, body);
}

/// Run `body` of the test `name` in a child of this test binary on threads of `stack` bytes.
/// The child is killed when the test thread that started it ends.
fn on_stack(stack: &str, name: &str, body: impl FnOnce()) {
    const CHILD: &str = "ADTB_ON_REDUCED_STACK";
    if let Ok(parent) = std::env::var(CHILD) {
        set_process_option(PR_SET_PDEATHSIG, SIGKILL).expect("die with the parent");
        if parent != std::os::unix::process::parent_id().to_string() {
            std::process::exit(1);
        }
        body();
        return;
    }
    let out = adtb_heap_cap::test_child(std::env::current_exe().expect("this test binary"), 1)
        .args(["--exact", name, "--nocapture"])
        .env(CHILD, std::process::id().to_string())
        .env("RUST_MIN_STACK", stack)
        .output()
        .expect("the child starts");
    assert!(
        out.status.success(),
        "{name}: {:?}\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("1 passed"), "{name}: {stdout}");
}

fn answer_of(detail: &str) -> String {
    format!("Err(Rejected {{ reason: Invalid {{ detail: {detail:?} }} }})")
}

fn fixed_denial(who: &str) -> String {
    let detail = format!("role `{who}` may not run this text");
    format!("Err(Rejected {{ reason: Denied {{ detail: {detail:?} }} }})")
}

/// Assert each role runs `kind` at `limit` and refuses it one level on, an owner with `refusal`
/// and a role that does not read the schema with it unless `hidden`.
fn runs_to_its_limit(name: &str, seed: u64, kind: &str, limit: usize, refusal: &str, hidden: bool) {
    let stack = if kind.starts_with("lambda") {
        FUNCTION_CHAIN_STACK
    } else {
        REDUCED_STACK
    };
    on_stack(stack, name, || {
        let mut sim = Sim::new(seed, Config::default());
        run(&mut sim, |rt| {
            let db = opened(&rt, &options(2));
            for who in ["reader", "owner"] {
                let at = deep_answer(&rt, &db, who, kind, limit);
                assert!(at.is_ok(), "{kind} {who} at {limit}: {at:?}");
                let expected = if hidden && who == "reader" {
                    fixed_denial(who)
                } else {
                    answer_of(refusal)
                };
                let past = deep_answer(&rt, &db, who, kind, limit + 1).map(drop);
                assert_eq!(expected, format!("{past:?}"), "{kind} {who} past {limit}");
            }
        });
    });
}

#[test]
fn a_function_chain_runs_to_its_limit() {
    runs_to_its_limit(
        "a_function_chain_runs_to_its_limit",
        700,
        "lambda",
        124,
        NORMALISES,
        true,
    );
}

#[test]
fn a_function_chain_over_sets_runs_to_its_limit() {
    runs_to_its_limit(
        "a_function_chain_over_sets_runs_to_its_limit",
        701,
        "lambda_set",
        61,
        NORMALISES,
        true,
    );
}

#[test]
fn an_operator_chain_runs_to_its_limit() {
    runs_to_its_limit(
        "an_operator_chain_runs_to_its_limit",
        702,
        "prim",
        62,
        PLANS,
        false,
    );
}

#[test]
fn an_if_chain_runs_to_its_limit() {
    runs_to_its_limit("an_if_chain_runs_to_its_limit", 703, "if", 62, PLANS, false);
}

#[test]
fn a_match_chain_runs_to_its_limit() {
    runs_to_its_limit(
        "a_match_chain_runs_to_its_limit",
        704,
        "match",
        60,
        PLANS,
        false,
    );
}

#[test]
fn a_record_nesting_runs_to_its_limit() {
    runs_to_its_limit(
        "a_record_nesting_runs_to_its_limit",
        705,
        "record",
        63,
        PLANS,
        false,
    );
}

#[test]
fn a_set_nesting_runs_to_its_limit() {
    runs_to_its_limit(
        "a_set_nesting_runs_to_its_limit",
        706,
        "set",
        63,
        PLANS,
        false,
    );
}

#[test]
fn a_comprehension_chain_runs_to_its_limit() {
    runs_to_its_limit(
        "a_comprehension_chain_runs_to_its_limit",
        707,
        "from",
        60,
        PLANS,
        false,
    );
}

#[test]
fn a_chain_of_match_arms_runs_to_its_limit() {
    runs_to_its_limit(
        "a_chain_of_match_arms_runs_to_its_limit",
        708,
        "arms",
        63,
        PLANS,
        false,
    );
}

#[test]
fn a_type_one_level_past_its_limit_is_refused_for_its_height() {
    on_reduced_stack(
        "a_type_one_level_past_its_limit_is_refused_for_its_height",
        || {
            scenario(709, |rt, db| {
                for kind in ["record", "set"] {
                    let past = deep_answer(&rt, &db, "owner", kind, 65).map(drop);
                    assert_eq!(answer_of(TYPE_TOO_LARGE), format!("{past:?}"), "{kind}");
                    let past = deep_answer(&rt, &db, "reader", kind, 65).map(drop);
                    assert_eq!(fixed_denial("reader"), format!("{past:?}"), "{kind}");
                }
            });
        },
    );
}

/// Returns a set of `copies` of a binary nesting of `levels` sets, its written out size set by both.
fn wide_doubling(levels: usize, copies: usize) -> String {
    let mut text = String::from("transact let a0 = 1\n");
    for i in 1..=levels {
        let p = i - 1;
        text.push_str(&format!("let a{i} = [a{p}, a{p}]\n"));
    }
    let items = vec![format!("a{levels}"); copies].join(", ");
    text.push_str(&format!("return [{items}]"));
    text
}

#[test]
fn the_written_out_size_runs_up_to_its_limit_and_no_further() {
    scenario(710, |rt, db| {
        let at = send(&rt, &db, "owner", &wide_doubling(10, 64)).map(drop);
        assert!(at.is_ok(), "{at:?}");
        let past = send(&rt, &db, "owner", &wide_doubling(10, 65)).map(drop);
        assert_eq!(answer_of(PLANS), format!("{past:?}"));
    });
}

/// Returns `depth` fixpoints, each nested in the seed of the next.
fn nested_fix(depth: usize) -> String {
    let mut text = String::from("[1]");
    for i in 0..depth {
        text = format!("(fix x{i} = {text} union (from y in x{i} select y) in x{i})");
    }
    text
}

#[test]
fn a_nesting_of_fixpoints_runs_to_its_limit_and_no_further() {
    scenario(711, |rt, db| {
        for who in ["reader", "owner"] {
            let at = send(&rt, &db, who, &nested_fix(8)).map(drop);
            assert!(at.is_ok(), "{who}: {at:?}");
        }
        let past = send(&rt, &db, "owner", &nested_fix(9)).map(drop);
        let refusal = "this text nests too many `fix` expressions";
        assert_eq!(answer_of(refusal), format!("{past:?}"));
        let past = send(&rt, &db, "reader", &nested_fix(9)).map(drop);
        assert_eq!(fixed_denial("reader"), format!("{past:?}"));
    });
}

/// Assert each role is refused the doubling text of `kind` at 33 doublings: an owner with
/// `refusal`, and a role that does not read the schema with it unless `hidden`.
fn doubled_is_refused(seed: u64, kind: &str, refusal: &str, hidden: bool) {
    let owner = doubled(seed, kind, 33, "owner");
    assert_eq!(answer_of(refusal), owner, "{kind}");
    let role = doubled(seed, kind, 33, "reader");
    let expected = if hidden {
        fixed_denial("reader")
    } else {
        answer_of(refusal)
    };
    assert_eq!(expected, role, "{kind}");
}

#[test]
fn a_doubling_set_within_the_height_bound_is_refused_for_its_size() {
    assert_eq!(answer_of(PLANS), doubled(666, "set", 28, "owner"));
}

#[test]
fn a_very_deep_type_is_refused_on_a_default_stack() {
    on_reduced_stack("a_very_deep_type_is_refused_on_a_default_stack", || {
        let mut sim = Sim::new(667, Config::default());
        run(&mut sim, |rt| {
            let db = opened(&rt, &options(2));
            for kind in ["record", "set"] {
                let got = deep_answer(&rt, &db, "owner", kind, 6000).map(drop);
                assert_eq!(answer_of(TYPE_TOO_LARGE), format!("{got:?}"), "{kind}");
            }
        });
    });
}

#[test]
fn a_doubling_join_is_refused() {
    doubled_is_refused(648, "join", NORMALISES, true);
}

#[test]
fn a_doubling_record_is_refused() {
    doubled_is_refused(649, "record", TYPE_TOO_LARGE, true);
}

#[test]
fn a_doubling_set_is_refused() {
    doubled_is_refused(650, "set", PLANS, false);
}

#[test]
fn a_doubling_union_is_refused() {
    doubled_is_refused(651, "union", PLANS, false);
}

#[test]
fn a_doubling_function_is_refused() {
    doubled_is_refused(653, "lambda", NORMALISES, true);
}

#[test]
fn a_doubling_sum_answers_or_is_refused_and_stops() {
    for who in ["reader", "owner"] {
        let got = doubled(652, "sum", 33, who);
        assert_eq!("Ok([\"Int(8589934592)\"])", got);
    }
}

#[test]
fn a_doubling_match_stops() {
    let got = doubled(669, "match_both", 33, "owner");
    assert_eq!("Ok([\"Int(1)\"])", got);
}

#[test]
fn a_match_over_a_doubling_match_stops() {
    let got = doubled(670, "match_on_match", 33, "owner");
    assert_eq!("Ok([\"Ctor(1, [Int(34)])\"])", got);
}

#[test]
fn a_capability_command_names_any_relation_and_a_statement_beside_it_does_not_read_it() {
    scenario(665, |rt, db| {
        db.role_authority()
            .grant_capability("bare", Capability::Maintain, false)
            .expect("maintain");
        for text in ["vacuum secrets", "verify secrets", "reindex secrets"] {
            allowed(&rt, &db, "bare", text);
        }
        let absent = format!("{:?}", send(&rt, &db, "bare", "verify nosuch"));
        assert!(absent.contains("Invalid"), "{absent}");
        let beside = "vacuum secrets\nfrom s in secrets select s.salary";
        detail(send(&rt, &db, "bare", beside));
        let beside = "vacuum nosuch\nfrom s in nosuch select s";
        detail(send(&rt, &db, "bare", beside));
    });
}

#[test]
fn a_chain_of_doubling_functions_is_refused() {
    let mut sim = Sim::new(654, Config::default());
    run(&mut sim, |rt| {
        let mut log = SCHEMA.to_owned();
        log.push_str("define fn d0 (x: I64) -> I64 = x + 1\n");
        for i in 0..40 {
            let n = i + 1;
            log.push_str(&format!(
                "define fn d{n} (x: I64) -> I64 = d{i}(x) + d{i}(x)\n"
            ));
        }
        let db = open_at(&rt, &options(2), &[&log]);
        let got = format!("{:?}", send(&rt, &db, "owner", "d40(1)"));
        assert!(got.contains("expands past"), "{got}");
    });
}

/// The most generators of `fused_loops` that pass the run bound.
const FUSED_LOOPS_THAT_RUN: usize = 61;

/// The longest list of additions a role's text may have within the permission check's steps.
const STEPS_THAT_CHECK: usize = 209_715;

/// Returns a text whose normal form nests `loops` generators, each `let` fusing into the one before.
fn fused_loops(loops: usize) -> String {
    let mut text = String::from("transact let a1 = (from d in docs.Draft select d.owner)\n");
    for i in 2..=loops {
        let p = i - 1;
        text.push_str(&format!(
            "let a{i} = from x in a{p}, y in docs.Draft select x\n"
        ));
    }
    text.push_str(&format!("return a{loops}"));
    text
}

#[test]
fn a_normal_form_nesting_loops_past_its_limit_is_refused() {
    scenario(668, |rt, db| {
        let at = send(&rt, &db, "owner", &fused_loops(64)).map(drop);
        assert_eq!(answer_of(PLANS), format!("{at:?}"));
        let past = send(&rt, &db, "owner", &fused_loops(65)).map(drop);
        assert_eq!(answer_of(NORMALISES), format!("{past:?}"));
    });
}

#[test]
fn a_nesting_of_loops_within_the_bound_runs_on_a_default_stack() {
    scenario(655, |rt, db| {
        let text = doubling("join", 3);
        assert_eq!(1, allowed(&rt, &db, "reader", &text).len());
    });
}

#[test]
fn every_write_form_is_charged_to_the_role() {
    scenario(656, |rt, db| {
        let upsert = r#"insert docs.Draft { id: docs.key(1), owner: "o", body: "b" }
  unless conflict on key
  else update old set Draft { old with body: "c" }"#;
        detail(send(&rt, &db, "reader", upsert));
        detail(send(&rt, &db, "inserter", upsert));
        allowed(&rt, &db, "upserter", upsert);
        let rejects = "insert secrets from (from d in docs.Draft select { salary: 1 }) \
                       rejects into doc_rejects";
        detail(send(&rt, &db, "reader", rejects));
        detail(send(&rt, &db, "payer", rejects));
        allowed(&rt, &db, "loader", rejects);
    });
}

#[test]
fn a_recursive_query_reading_past_its_grants_is_refused() {
    scenario(657, |rt, db| {
        let body = "fix r = (from s in secrets select s.salary) union \
                    (from x in r, y in r select x) in r";
        detail(send(&rt, &db, "reader", body));
        detail(send(&rt, &db, "outer", body));
        let open =
            r"fix r = (from d in docs.Draft select d.owner) union (from x in r select x) in r";
        assert_eq!(vec!["Text(\"ann\")"], allowed(&rt, &db, "reader", open));
    });
}

#[test]
fn ordering_by_a_field_reads_it() {
    scenario(658, |rt, db| {
        let by_body = "from d in docs.Draft select d order by d.body";
        detail(send(&rt, &db, "narrow", by_body));
        assert_eq!(1, allowed(&rt, &db, "reader", by_body).len());
    });
}

#[test]
fn prior_stays_outside_ad_hoc_text() {
    scenario(659, |rt, db| {
        for role in ["reader", "owner"] {
            let got = format!(
                "{:?}",
                send(&rt, &db, role, "from d in prior(docs) select d.owner")
            );
            assert!(
                got.contains("Invalid") || got.contains("Denied"),
                "{role}: {got}"
            );
            assert!(!got.contains("ann"), "{role}: {got}");
        }
    });
}

#[test]
fn a_dropped_role_runs_nothing() {
    scenario(660, |rt, db| {
        let authority = db.role_authority();
        let held = session(&db, "other");
        authority.drop_role("other").expect("drop");
        let got = held.submit_text("1", &[], TaskOptions::default()).map(drop);
        assert!(got.is_err(), "{got:?}");
        assert!(send(&rt, &db, "reader", "1").is_ok());
    });
}

#[test]
fn a_checked_text_that_does_not_run_answers_invalid_not_denied() {
    scenario(661, |rt, db| {
        let text = "insert docs.Draft { owner: \"x\", body: \"y\" }\nprint schema";
        let got = format!("{:?}", send(&rt, &db, "writer", text));
        assert!(got.contains("Invalid"), "{got}");
        assert!(got.contains("inside a transaction"), "{got}");
    });
}

fn exporter(rt: &SimRuntime, base: &std::path::Path) -> Database<SimRuntime> {
    let opts = OpenOptions {
        file_root: Some(base.to_path_buf()),
        export_roots: vec![PathBuf::from("out")],
        ..options(2)
    };
    let db = opened(rt, &opts);
    let authority = db.role_authority();
    for role in ["reader", "owner"] {
        authority
            .grant_capability(role, Capability::WriteFiles, false)
            .expect("write_files");
    }
    db
}

const EXPORT: &str = "export (from d in docs.Draft select { o: d.owner })";

#[test]
fn an_export_does_not_follow_a_link_or_replace_what_exists() {
    let dir = adtb_testkit::scratch::Scratch::new("adhoc-export-link", "x");
    let base = dir.path().to_path_buf();
    std::fs::create_dir_all(base.join("out")).expect("a directory");
    std::fs::create_dir_all(base.join("outside")).expect("a directory");
    std::fs::write(base.join("outside/victim"), "kept").expect("the victim");
    std::os::unix::fs::symlink(base.join("outside/victim"), base.join("out/a.csv.partial"))
        .expect("the link");
    std::os::unix::fs::symlink(base.join("outside/gone"), base.join("out/dangling.csv"))
        .expect("the link");
    std::fs::write(base.join("out/there.csv"), "kept").expect("the target");
    std::fs::create_dir_all(base.join("out/isdir.csv")).expect("a directory");
    let mut sim = Sim::new(662, Config::default());
    run(&mut sim, |rt| {
        let db = exporter(&rt, &base);
        allowed(&rt, &db, "reader", &format!("{EXPORT} to \"out/a.csv\""));
        assert_eq!(
            "kept",
            std::fs::read_to_string(base.join("outside/victim")).expect("the victim")
        );
        assert!(base.join("out/a.csv").is_file());
        for target in ["there.csv", "dangling.csv", "isdir.csv", "no/dir.csv"] {
            let got = format!(
                "{:?}",
                send(&rt, &db, "reader", &format!("{EXPORT} to \"out/{target}\""))
            );
            assert!(got.contains("BadInput"), "{target}: {got}");
            assert!(
                !got.contains("does not run in this engine"),
                "{target}: {got}"
            );
        }
        assert_eq!(
            "kept",
            std::fs::read_to_string(base.join("out/there.csv")).expect("the target")
        );
        assert!(!base.join("outside/gone").exists());
        let mut left: Vec<String> = std::fs::read_dir(base.join("out"))
            .expect("the directory")
            .map(|e| {
                e.expect("an entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        left.sort();
        assert_eq!(
            vec![
                "a.csv",
                "a.csv.partial",
                "dangling.csv",
                "isdir.csv",
                "there.csv"
            ],
            left
        );
    });
}

#[test]
fn an_owner_session_of_a_server_is_confined_to_the_export_roots() {
    let dir = adtb_testkit::scratch::Scratch::new("adhoc-export-owner", "x");
    let base = dir.path().to_path_buf();
    std::fs::create_dir_all(base.join("out")).expect("a directory");
    let mut sim = Sim::new(663, Config::default());
    run(&mut sim, |rt| {
        let db = exporter(&rt, &base);
        let why = detail(send(
            &rt,
            &db,
            "owner",
            &format!("{EXPORT} to \"loose.csv\""),
        ));
        assert!(why.contains("export_roots"), "{why}");
        assert!(!base.join("loose.csv").exists());
        allowed(&rt, &db, "owner", &format!("{EXPORT} to \"out/held.csv\""));
        assert!(base.join("out/held.csv").is_file());
    });
}

#[test]
fn a_long_chain_of_operators_answers_and_does_not_take_the_stack() {
    on_reduced_stack(
        "a_long_chain_of_operators_answers_and_does_not_take_the_stack",
        || {
            scenario(664, |rt, db| {
                let chain = vec!["d.body == \"text\""; 5000].join(" or ");
                let text = format!("from d in docs.Draft where {chain} select d.owner");
                for who in ["reader", "owner"] {
                    let got = format!("{:?}", send(&rt, &db, who, &text));
                    assert_eq!("Ok([\"Text(\\\"ann\\\")\"])", got, "{who}");
                }
            });
        },
    );
}

#[test]
fn a_module_a_role_names_nothing_in_answers_as_an_absent_one() {
    let vault = "module vault\nadd module vault\nadd type V = { id: Key vs }\nadd relation vs : V key id generate sequence\n";
    let mut sim = Sim::new(671, Config::default());
    run(&mut sim, |rt| {
        let opts = options(2);
        let db = open_at(&rt, &opts, &[SCHEMA, vault]);
        let authority = db.role_authority();
        for role in ["only_adhoc", "maintainer"] {
            authority.create_role(role).expect("role");
            authority
                .grant_capability(role, Capability::Adhoc, false)
                .expect("adhoc");
        }
        authority
            .grant_capability("maintainer", Capability::Maintain, false)
            .expect("maintain");
        for (present, absent) in [
            ("cancel call vault::x.key(1)", "cancel call nomod::x.key(1)"),
            (
                "from x in vault::vs select x",
                "from x in nomod::vs select x",
            ),
            ("vacuum vault::x", "vacuum nomod::x"),
        ] {
            for role in ["only_adhoc", "maintainer"] {
                let one = format!("{:?}", send(&rt, &db, role, present));
                let two = format!("{:?}", send(&rt, &db, role, absent));
                let two = two.replace("nomod", "vault");
                assert_eq!(one, two, "{role}: {present}");
            }
        }
        detail(send(&rt, &db, "only_adhoc", "cancel call nomod::x.key(1)"));
        detail(send(&rt, &db, "only_adhoc", "vacuum nomod::x"));
        detail(send(&rt, &db, "maintainer", "cancel call nomod::x.key(1)"));
    });
}

#[test]
fn a_capability_command_on_a_name_of_two_modules_answers_ambiguous() {
    let vault = "module vault\nadd module vault\nadd type V = { id: Key secrets }\nadd relation secrets : V key id generate sequence\n";
    let mut sim = Sim::new(672, Config::default());
    run(&mut sim, |rt| {
        let db = open_at(&rt, &options(2), &[SCHEMA, vault]);
        let authority = db.role_authority();
        authority.create_role("maintainer").expect("role");
        for capability in [Capability::Adhoc, Capability::Maintain] {
            authority
                .grant_capability("maintainer", capability, false)
                .expect("capability");
        }
        let got = format!("{:?}", send(&rt, &db, "maintainer", "vacuum secrets"));
        assert!(got.contains("qualify"), "{got}");
        allowed(&rt, &db, "maintainer", "vacuum vault::secrets");
    });
}

#[test]
fn reindex_by_fields_names_fields_to_a_maintain_role() {
    scenario(673, |rt, db| {
        db.role_authority()
            .grant_capability("bare", Capability::Maintain, false)
            .expect("maintain");
        let got = format!(
            "{:?}",
            send(&rt, &db, "bare", "reindex secrets index (salry)")
        );
        assert!(got.contains("Invalid") && got.contains("no index"), "{got}");
    });
}

#[test]
fn a_key_parameter_of_a_referenced_relation_shows_only_the_selection_a_field_states() {
    scenario(674, |_, db| {
        let key = |rel, ctors| Ty::Key { rel, ctors };
        let absent = typed(&db, "noter", "k", Ty::Base(Base::I64));
        assert!(!absent.contains("may not run this text"), "{absent}");
        let (secrets, docs) = (secrets_id(&db), docs_id(&db));
        for held in [
            key(secrets, None),
            key(docs, None),
            key(docs, Some(vec![0])),
        ] {
            let got = typed(&db, "noter", "k", held.clone());
            assert!(!got.contains("may not run this text"), "{held:?}: {got}");
        }
        let fixed = typed(&db, "noter", "k", key(secrets, Some(vec![5])));
        assert!(fixed.contains("may not run this text"), "{fixed}");
        for shown in [
            key(secrets, Some(vec![0])),
            key(docs, Some(vec![1])),
            key(docs, Some(vec![0, 1])),
        ] {
            assert_eq!(fixed, typed(&db, "noter", "k", shown.clone()), "{shown:?}");
        }
    });
}

fn invalid(got: &Result<Vec<String>, Error>) -> bool {
    matches!(
        got,
        Err(Error::Rejected {
            reason: RejectReason::Invalid { .. }
        })
    )
}

#[test]
fn a_text_past_the_normaliser_s_limit_answers_invalid() {
    let mut sim = Sim::new(675, Config::default());
    run(&mut sim, |rt| {
        let db = opened(&rt, &options(2));
        let got = deep_answer(&rt, &db, "owner", "lambda", 200).map(drop);
        assert_eq!(answer_of(NORMALISES), format!("{got:?}"));
        let got = deep_answer(&rt, &db, "reader", "lambda", 200).map(drop);
        assert_eq!(fixed_denial("reader"), format!("{got:?}"));
    });
}

#[test]
fn a_text_past_the_permission_check_s_limit_answers_invalid_with_its_row() {
    let step = vec!["x"; 60].join(" + ");
    let schema = format!(
        "{SCHEMA}define aggregate deep (x: I64) -> I64 = state {{ n: I64 }} init {{ n: 0 }} \
         step (s, x) -> {{ n: s.n + {step} }} merge (a, b) -> {{ n: a.n + b.n }} finish s -> s.n\n"
    );
    let mut sim = Sim::new(676, Config::default());
    run(&mut sim, |rt| {
        let db = open_at(&rt, &options(2), &[&schema]);
        let authority = db.role_authority();
        authority.create_role("reader").expect("role");
        authority
            .grant_capability("reader", Capability::Adhoc, false)
            .expect("adhoc");
        let owner = session(&db, "owner");
        for relation in CATALOG_READS {
            authority
                .grant(&owner, &format!("grant read catalog::{relation} to reader"))
                .expect("the catalog grant");
        }
        let mut text = String::from("deep(y for y in [1])");
        for _ in 0..30 {
            text = format!("[{text}]");
        }
        let got = format!("{:?}", send(&rt, &db, "reader", &text).map(drop));
        let refusal = "the text asks more of the permission check than it allows";
        assert_eq!(answer_of(refusal), got);
        assert_eq!(1, call_rows(&db, "reader"));
    });
}

#[test]
fn a_doubling_concatenation_is_held_to_the_memory_grant() {
    let literal = "x".repeat(64 << 10);
    let mut text = format!("transact let a0 = \"{literal}\"\n");
    for i in 1..=13 {
        let p = i - 1;
        text.push_str(&format!("let a{i} = a{p} ++ a{p}\n"));
    }
    text.push_str("return a13");
    scenario(677, |rt, db| {
        let got = send(&rt, &db, "owner", &text);
        assert!(
            matches!(
                got,
                Err(Error::Rejected {
                    reason: RejectReason::StatementLimit { .. }
                })
            ),
            "{:?}",
            got.map(|rows| rows.len())
        );
    });
}

#[test]
fn a_long_chain_of_lets_answers_and_does_not_take_the_stack() {
    let mut text = String::from("transact let a0 = 1\n");
    for i in 1..=40_000 {
        let p = i - 1;
        text.push_str(&format!("let a{i} = a{p}\n"));
    }
    text.push_str("return a40000");
    on_reduced_stack(
        "a_long_chain_of_lets_answers_and_does_not_take_the_stack",
        || {
            scenario(678, |rt, db| {
                for who in ["reader", "owner"] {
                    let got = send(&rt, &db, who, &text);
                    assert_eq!(1, got.expect(who).len(), "{who}");
                }
            });
        },
    );
}

/// Returns the text of `n` statements inserting a draft each, then `last`.
fn inserts(n: usize, last: &str) -> String {
    let mut text = String::from("transact\n");
    for i in 0..n {
        text.push_str(&format!(
            "insert docs.Draft {{ owner: \"o{i}\", body: \"b\" }}\n"
        ));
    }
    text.push_str(last);
    text
}

#[test]
fn a_transact_of_a_thousand_inserts_runs_and_explains_on_the_reduced_stack() {
    on_reduced_stack(
        "a_transact_of_a_thousand_inserts_runs_and_explains_on_the_reduced_stack",
        || {
            scenario(800, |rt, db| {
                let text = inserts(1000, "return 1");
                for who in ["writer", "owner"] {
                    let got = send(&rt, &db, who, &text).map(|r| r.len());
                    assert_eq!("Ok(1)", format!("{got:?}"), "{who}");
                }
                for how in ["explain", "explain analyze"] {
                    let got = send(&rt, &db, "owner", &format!("{how} {text}")).map(|r| r.len());
                    assert_eq!("Ok(1)", format!("{got:?}"), "{how}");
                }
            });
        },
    );
}

#[test]
fn a_chain_of_ifs_just_under_the_normal_form_height_is_normalised() {
    scenario(817, |rt, db| {
        let at = deep_answer(&rt, &db, "owner", "if", 127).map(drop);
        assert_eq!(answer_of(PLANS), format!("{at:?}"));
        let past = deep_answer(&rt, &db, "owner", "if", 128).map(drop);
        assert_eq!(answer_of(NORMALISES), format!("{past:?}"));
    });
}

#[test]
fn a_chain_of_different_ors_runs_on_the_reduced_stack() {
    on_reduced_stack("a_chain_of_different_ors_runs_on_the_reduced_stack", || {
        let chain: Vec<String> = (0..5000).map(|i| format!("d.body == \"t{i}\"")).collect();
        let text = format!(
            "from d in docs.Draft where {} select d.owner",
            chain.join(" or ")
        );
        scenario(818, |rt, db| {
            assert_eq!(0, allowed(&rt, &db, "owner", &text).len());
        });
    });
}

#[test]
fn a_chain_of_conditions_past_the_height_limit_is_refused_by_the_normaliser() {
    scenario(801, |rt, db| {
        let arms: String = (0..200).map(|i| format!("| {i} -> skip ")).collect();
        let text = format!("transact let n = 1 match n {arms}| _ -> skip");
        let got = send(&rt, &db, "owner", &text).map(drop);
        assert_eq!(answer_of(NORMALISES), format!("{got:?}"));
    });
}

#[test]
fn a_directly_nested_text_stops_for_every_kind() {
    let kinds: Vec<fn(&str) -> String> = vec![
        |i| format!("covers(span(1, 2), {i})"),
        |i| format!("overlaps(span(1, 2), {i})"),
        |i| format!("overlaps({i}, span(1, 2))"),
        |i| format!("({i}) + 1"),
        |i| format!("1 + ({i})"),
        |i| format!("({i}) == 1"),
        |i| format!("1 == ({i})"),
        |i| format!("if {i} then 1 else 2"),
        |i| format!("if true then {i} else 2"),
        |i| format!("if true then 1 else {i}"),
        |i| format!("match maybe(1) | None -> 0 | Some s -> {i}"),
        |i| format!("{{ v: {i} }}"),
        |i| format!("[{i}]"),
        |i| format!("[1, {i}]"),
        |i| format!("from y in {i} select y"),
        |i| format!("from y in [1] where {i} select y"),
        |i| format!("from y in [1] select {i}"),
        |i| format!("maybe({i})"),
        |i| format!("({i}) union [1]"),
        |i| format!("[1] union ({i})"),
        |i| format!("Some {{ value: {i} }}"),
        |i| format!("sum(y for y in {i})"),
        |i| format!("({i}).v"),
    ];
    let nested = |kind: fn(&str) -> String, depth: usize, bottom: &str| {
        (0..depth).fold(bottom.to_string(), |t, _| kind(&t))
    };
    let absent =
        answer_of("`nosuch` is a parameter of this text, but the call supplies no value for it");
    scenario(802, |rt, db| {
        for kind in kinds {
            let got = send(&rt, &db, "owner", &nested(kind, 20, "nosuch")).map(drop);
            assert_eq!(absent, format!("{got:?}"));
            let got = send(&rt, &db, "reader", &nested(kind, 20, "nosuch")).map(drop);
            assert_eq!(fixed_denial("reader"), format!("{got:?}"));
            let got = send(&rt, &db, "owner", &nested(kind, 20, "1"));
            assert!(got.is_ok() || invalid(&got), "{got:?}");
            let got = format!(
                "{:?}",
                send(&rt, &db, "owner", &nested(kind, 30, "nosuch")).map(drop)
            );
            assert!(got == absent || got.contains("nests more than 64"), "{got}");
        }
    });
}

/// Returns a text of type doubled 13 times, the most one expression may have, returning `copies`.
fn large_type_copies(copies: usize) -> String {
    let mut text = String::from("transact let a0 = 1\n");
    for i in 1..=13 {
        let p = i - 1;
        text.push_str(&format!("let a{i} = {{ l: a{p}, r: a{p} }}\n"));
    }
    let items = vec!["a13"; copies].join(", ");
    text.push_str(&format!("return [{items}]"));
    text
}

#[test]
fn the_largest_type_of_an_expression_is_accepted() {
    scenario(803, |rt, db| {
        let got = send(&rt, &db, "owner", &large_type_copies(1)).map(|r| r.len());
        assert_eq!("Ok(1)", format!("{got:?}"));
    });
}

#[test]
fn references_to_a_large_type_are_charged_for_the_whole_text() {
    let refusal = "the types of this text are too large";
    scenario(804, |rt, db| {
        let text = large_type_copies(13_000);
        let got = send(&rt, &db, "owner", &text).map(drop);
        assert_eq!(answer_of(refusal), format!("{got:?}"));
        let got = send(&rt, &db, "reader", &text).map(drop);
        assert_eq!(fixed_denial("reader"), format!("{got:?}"));
        let got = send(&rt, &db, "owner", &large_type_copies(10)).map(|r| r.len());
        assert_eq!("Ok(1)", format!("{got:?}"));
    });
}

#[test]
fn reindex_answers_one_way_for_what_the_index_does_not_declare() {
    let pairs = [
        (
            "reindex secrets index (salary == \"x\") in Secret",
            "reindex secrets index (salary == 1) in Secret",
            "secrets",
        ),
        (
            "reindex docs index (owner) in Sent",
            "reindex docs index (owner) in Nosuch",
            "docs",
        ),
    ];
    scenario(805, |rt, db| {
        db.role_authority()
            .grant_capability("bare", Capability::Maintain, false)
            .expect("maintain");
        for (one, other, relation) in pairs {
            let role = format!("{:?}", send(&rt, &db, "bare", one).map(drop));
            let same = format!("{:?}", send(&rt, &db, "bare", other).map(drop));
            assert_eq!(role, same, "{one}");
            let none = answer_of(&format!(
                "`{relation}` declares no index or unique with these fields in this constructor set"
            ));
            for text in [one, other] {
                let owner = format!("{:?}", send(&rt, &db, "owner", text).map(drop));
                assert_eq!(none, owner, "{text}");
            }
        }
    });
}

#[test]
fn a_nesting_of_fixpoints_at_its_limit_runs_on_the_reduced_stack() {
    on_stack(
        FUNCTION_CHAIN_STACK,
        "a_nesting_of_fixpoints_at_its_limit_runs_on_the_reduced_stack",
        || {
            scenario(806, |rt, db| {
                for who in ["reader", "owner"] {
                    allowed(&rt, &db, who, &nested_fix(8));
                }
            });
        },
    );
}

#[test]
fn fused_loops_under_the_run_bound_run_on_the_reduced_stack() {
    on_stack(
        FUNCTION_CHAIN_STACK,
        "fused_loops_under_the_run_bound_run_on_the_reduced_stack",
        || {
            scenario(807, |rt, db| {
                allowed(&rt, &db, "owner", &fused_loops(FUSED_LOOPS_THAT_RUN));
                let past =
                    send(&rt, &db, "owner", &fused_loops(FUSED_LOOPS_THAT_RUN + 1)).map(drop);
                assert_eq!(answer_of(PLANS), format!("{past:?}"));
            });
        },
    );
}

#[test]
fn a_wide_doubling_at_its_limit_runs_on_the_reduced_stack() {
    on_reduced_stack(
        "a_wide_doubling_at_its_limit_runs_on_the_reduced_stack",
        || {
            scenario(808, |rt, db| {
                allowed(&rt, &db, "owner", &wide_doubling(10, 64));
            });
        },
    );
}

/// Returns `base` with `n` additions of one.
fn added(base: &str, n: usize) -> String {
    let mut text = base.to_string();
    for _ in 0..n {
        text.push_str(" + 1");
    }
    text
}

#[test]
fn an_aggregate_at_the_height_limit_runs_on_the_reduced_stack() {
    on_reduced_stack(
        "an_aggregate_at_the_height_limit_runs_on_the_reduced_stack",
        || {
            scenario(809, |rt, db| {
                let text = format!("sum({} for y in [1])", added("y", 60));
                for who in ["reader", "owner"] {
                    assert_eq!(1, allowed(&rt, &db, who, &text).len(), "{who}");
                }
            });
        },
    );
}

#[test]
fn a_write_at_the_height_limit_checks_its_constraint_on_the_reduced_stack() {
    on_reduced_stack(
        "a_write_at_the_height_limit_checks_its_constraint_on_the_reduced_stack",
        || {
            scenario(810, |rt, db| {
                let text = format!("insert secrets {{ salary: {} }}", added("1", 60));
                allowed(&rt, &db, "payer", &text);
                allowed(&rt, &db, "owner", &text);
                let refused = format!("insert secrets {{ salary: {} }}", added("-100", 60));
                let got = format!("{:?}", send(&rt, &db, "owner", &refused).map(drop));
                assert!(got.contains("no_negative"), "{got}");
            });
        },
    );
}

#[test]
fn explain_at_the_height_limit_runs_on_the_reduced_stack() {
    on_reduced_stack(
        "explain_at_the_height_limit_runs_on_the_reduced_stack",
        || {
            scenario(811, |rt, db| {
                let read = format!("from y in [1] select {}", added("y", 60));
                let write = format!("insert secrets {{ salary: {} }}", added("1", 60));
                for how in ["explain", "explain analyze"] {
                    for text in [&read, &write] {
                        allowed(&rt, &db, "owner", &format!("{how} {text}"));
                    }
                }
            });
        },
    );
}

#[test]
fn an_export_at_the_height_limit_runs_on_the_reduced_stack() {
    let dir = adtb_testkit::scratch::Scratch::new("adhoc-deep-export", "x");
    let base = dir.path().to_path_buf();
    on_reduced_stack(
        "an_export_at_the_height_limit_runs_on_the_reduced_stack",
        || {
            std::fs::create_dir_all(base.join("out")).expect("the export root");
            let mut sim = Sim::new(812, Config::default());
            run(&mut sim, |rt| {
                let opts = OpenOptions {
                    file_root: Some(base.clone()),
                    export_roots: vec![PathBuf::from("out")],
                    ..options(2)
                };
                let db = opened(&rt, &opts);
                let text = format!(
                    "export (from y in [1] select {{ n: {} }}) to \"out/deep.csv\"",
                    added("y", 55)
                );
                allowed(&rt, &db, "owner", &text);
            });
        },
    );
}

const STATEMENT_LIMIT: &str =
    "Err(Rejected { reason: StatementLimit { limit: Memory { bytes: 67108864 } } })";

#[test]
fn a_list_of_more_terms_than_the_base_spend_is_planned() {
    let items = vec!["1"; 300_000].join(", ");
    scenario(813, |rt, db| {
        let got = send(&rt, &db, "owner", &format!("[{items}]")).map(drop);
        assert_eq!(STATEMENT_LIMIT, format!("{got:?}"));
    });
}

#[test]
fn a_list_just_under_the_permission_step_limit_is_checked() {
    let list = |n: usize| format!("[{}]", vec!["1 + 1"; n].join(", "));
    let refusal = "the text asks more of the permission check than it allows";
    scenario(814, |rt, db| {
        let at = send(&rt, &db, "reader", &list(STEPS_THAT_CHECK)).map(drop);
        assert_eq!(STATEMENT_LIMIT, format!("{at:?}"));
        let past = send(&rt, &db, "reader", &list(STEPS_THAT_CHECK + 1)).map(drop);
        assert_eq!(answer_of(refusal), format!("{past:?}"));
    });
}
