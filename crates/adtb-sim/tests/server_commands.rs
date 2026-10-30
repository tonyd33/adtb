//! The capability each operational command needs when a role session issues it.

use adtb::{CallOptions, Capability, Database, Error, Log, RejectReason, TaskOptions, Value};
use adtb_sim::{Config, Sim, SimRuntime};
use adtb_simkit::api::{options, query, run, task};

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

/// How a command of the table behaves for a role that may issue it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Ran {
    /// It runs to completion.
    Done,
    /// The check passes, then the engine reports it does not run the command yet.
    Later,
}

/// A row of the command table, by its first column: a statement for each form it runs in today
/// and the capabilities that decide it, or the statements of a form the engine does not run.
struct Row {
    spec: &'static str,
    capability: &'static str,
    forms: &'static [(&'static str, &'static [Capability], Ran)],
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
        forms: &[("vacuum items", &[Capability::Maintain], Ran::Done)],
        unsupported: &[],
    },
    Row {
        spec: "`verify`, `verify r`",
        capability: "`maintain`",
        forms: &[
            ("verify", &[Capability::Maintain], Ran::Done),
            ("verify items", &[Capability::Maintain], Ran::Done),
        ],
        unsupported: &[],
    },
    Row {
        spec: "`reindex r`, `reindex r index (f) in S`",
        capability: "`maintain`",
        forms: &[("reindex items", &[Capability::Maintain], Ran::Later)],
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
            Ran::Done,
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
            for &(text, capabilities, ran) in row.forms {
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
                let got = task(&rt, &capable, text);
                match ran {
                    Ran::Done => got.unwrap_or_else(|e| panic!("{text}: {e:?}")),
                    Ran::Later => {
                        assert!(
                            format!("{got:?}").contains("does not run"),
                            "{text}: {got:?}"
                        );
                    }
                }
                assert_eq!(before, audit_denied(&db, "capable"), "{text}");
                for capability in capabilities {
                    authority
                        .revoke_capability("capable", *capability)
                        .expect("revoke");
                }
                let got = task(&rt, &capable, text);
                refused_and_audited(&db, "capable", text, needed, got, before);
                if ran == Ran::Done {
                    task(&rt, &owner, text).unwrap_or_else(|e| panic!("owner {text}: {e:?}"));
                }
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
            for &(text, _, ran) in row.forms {
                if ran == Ran::Done {
                    db.query_text(text, &[], &CallOptions::default())
                        .unwrap_or_else(|e| panic!("{text}: {e:?}"));
                }
            }
        }
    });
}
