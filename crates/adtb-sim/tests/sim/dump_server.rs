//! A dump issued by a role through a server: the authority it needs, the paths it may write and
//! what its restore holds.

use std::path::PathBuf;

use adtb::catalog::Dormant;
use adtb::storage::Quarantine;
use adtb::{
    ArchiveTarget, CallOptions, Capability, Database, Log, OpenOptions, RestorePoint,
    restore_report_in,
};
use adtb_sim::{Config, Sim, SimRuntime};
use adtb_simkit::api;
use adtb_testkit::refused_with;

const APP: &str = r"module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
add type Shape { name: Text } = Circle { r: I64 } | Square { side: I64 }
add relation shapes : Shape key name
";
const READS: &str = "module app\ngrant read users to ops\ngrant read shapes to ops\n";
const ROWS: &str = r#"transact
  insert users { name: "ann" }
  insert shapes.Circle { name: "c", r: 2 }
  insert shapes.Square { name: "s", side: 3 }"#;
/// The catalog relations a dump reads outside the printed schema.
const DUMPED: &str = "settings audit bundles registered feeds feed_members";
/// The constructor a refusal names for each of [`DUMPED`].
const NAMED: &str = "settings.StatementLimit audit.Login bundles.Bundle registered.Query feeds.Feed \
    feed_members.FeedMember";
/// The catalog relations the printed schema states.
const SCHEMA: &str = "types constructors ctor_states value_states fields relations named_sets \
    set_members indexes index_ctors constraints functions defaults computed grants";
const HIDDEN: &str = "denied: role `ops` may not run this text";
const OPS: [Capability; 2] = [Capability::Maintain, Capability::WriteFiles];
const DUMP: &str = "dump to \"out/d\"";

fn opts(root: PathBuf) -> OpenOptions {
    OpenOptions {
        file_root: Some(root),
        export_roots: vec!["out".into()],
        ..api::options(2)
    }
}

/// Returns a file named after `root` holding [`ROWS`] and role `ops` holding `adhoc`,
/// `capabilities`, `read` on [`DUMPED`], on both user relations when `reads` and on [`SCHEMA`]
/// when `schema`.
fn opened(
    rt: &SimRuntime,
    root: &str,
    capabilities: &[Capability],
    reads: bool,
    schema: bool,
) -> Database<SimRuntime> {
    let opts = opts(root.into());
    let file = format!("{}.adt", root.replace('/', "_"));
    let db = Database::open_in(rt.clone(), &file, &Log::new([APP]), &opts).unwrap();
    db.role_authority().create_role("ops").unwrap();
    db.close().unwrap();
    let log = if reads { vec![APP, READS] } else { vec![APP] };
    let db = Database::open_in(rt.clone(), &file, &Log::new(log), &opts).unwrap();
    db.query_text(ROWS, &[], &CallOptions::default()).unwrap();
    let authority = db.role_authority();
    for capability in [&[Capability::Adhoc][..], capabilities].concat() {
        authority
            .grant_capability("ops", capability, false)
            .unwrap();
    }
    let owner = authority.session("owner").unwrap();
    let schema = SCHEMA.split_whitespace().filter(|_| schema);
    for r in DUMPED.split_whitespace().chain(schema) {
        let text = format!("grant read catalog::{r} to ops");
        authority.grant(&owner, &text).unwrap();
    }
    db
}

fn ops(db: &Database<SimRuntime>) -> adtb::RoleSession<SimRuntime> {
    db.role_authority().session("ops").unwrap()
}

/// Returns each printed row of `text` on `db`, sorted.
fn rows(db: &Database<SimRuntime>, text: &str) -> Vec<String> {
    let mut out = api::query(db, text);
    out.sort();
    out
}

fn every(db: &Database<SimRuntime>) -> Vec<Vec<String>> {
    ["app::users", "app::shapes", "catalog::settings"]
        .iter()
        .map(|r| rows(db, r))
        .collect()
}

/// Assert no file of `sim` lies under an export root.
fn nothing_written(sim: &Sim) {
    let stray: Vec<_> = sim
        .files()
        .into_keys()
        .filter(|p| p.split('/').any(|c| c == "out"))
        .collect();
    assert!(stray.is_empty(), "{stray:?}");
}

#[test]
fn each_missing_authority_refuses_a_dump_and_writes_nothing() {
    let mut sim = Sim::new(541, Config::default());
    api::run(&mut sim, |rt| {
        for (lacking, reason) in [
            (Capability::Maintain, "denied: role `ops` lacks `maintain`"),
            (
                Capability::WriteFiles,
                "denied: role `ops` lacks `write_files`",
            ),
        ] {
            let held: Vec<_> = OPS.into_iter().filter(|c| *c != lacking).collect();
            let db = opened(&rt, lacking.name(), &held, true, true);
            refused_with(api::task(&rt, &ops(&db), DUMP), reason);
            db.close().unwrap();
        }
        let db = opened(&rt, "unread", &OPS, false, true);
        let reason = "denied: role `ops` reads users.User and lacks read";
        refused_with(api::task(&rt, &ops(&db), DUMP), reason);
        db.close().unwrap();
        let db = opened(&rt, "unschema", &OPS, true, false);
        refused_with(api::task(&rt, &ops(&db), DUMP), HIDDEN);
        db.close().unwrap();
        let db = opened(&rt, "data", &OPS, true, true);
        let authority = db.role_authority();
        let owner = authority.session("owner").unwrap();
        let mut got = Vec::new();
        for r in DUMPED.split_whitespace().chain(SCHEMA.split_whitespace()) {
            authority
                .revoke(&owner, &format!("revoke read catalog::{r} from ops"))
                .unwrap();
            got.push(api::task(&rt, &ops(&db), DUMP).unwrap_err().to_string());
            authority
                .grant(&owner, &format!("grant read catalog::{r} to ops"))
                .unwrap();
        }
        let lacked = |c: &str| format!("denied: role `ops` reads {c} and lacks read");
        let hidden = std::iter::repeat_n(HIDDEN.to_owned(), 15);
        let expected = NAMED.split_whitespace().map(lacked).chain(hidden);
        assert_eq!(expected.collect::<Vec<_>>(), got);
        db.close().unwrap();
    });
    nothing_written(&sim);
}

/// Returns what restoring the dump at `data/out/d` to `copy.adt` reports, and `copy.adt` open.
fn restored(rt: &SimRuntime) -> (adtb::Restored, Database<SimRuntime>) {
    let archive = ArchiveTarget::path("data/out/d").unwrap();
    let no_create = OpenOptions {
        create: None,
        ..api::options(2)
    };
    let report = restore_report_in(
        rt.clone(),
        &archive,
        "copy.adt",
        RestorePoint::Latest,
        &no_create,
    );
    let report = report.unwrap();
    let copy = Database::open_in(rt.clone(), "copy.adt", &Log::new([APP, READS]), &no_create);
    (report, copy.unwrap())
}

/// Returns the keys `text` selects on `db`, ascending.
fn keys(db: &Database<SimRuntime>, text: &str) -> Vec<i64> {
    let digits = |row: &String| row.chars().filter(char::is_ascii_digit).collect::<String>();
    let mut out: Vec<i64> = rows(db, text)
        .iter()
        .map(|r| digits(r).parse().unwrap())
        .collect();
    out.sort_unstable();
    out
}

const ROLES: &str = "from r in catalog::roles select r.name";

#[test]
fn a_role_holding_schema_dumps_what_restores_to_the_same_rows_and_roles() {
    let mut sim = Sim::new(542, Config::default());
    api::run(&mut sim, |rt| {
        let db = opened(
            &rt,
            "data",
            &[&OPS[..], &[Capability::Schema]].concat(),
            true,
            true,
        );
        let got = api::task_rows(&rt, &ops(&db), DUMP).unwrap();
        let (src, roles) = (every(&db), rows(&db, ROLES));
        db.close().unwrap();
        let (report, copy) = restored(&rt);
        assert_eq!(vec![format!("Record([Text({:?})])", report.manifest)], got);
        assert!(
            report.manifest.contains("\npart security "),
            "{}",
            report.manifest
        );
        assert_eq!(Vec::<Dormant>::new(), report.dormant);
        assert_eq!(src, every(&copy));
        assert_eq!(roles, rows(&copy, ROLES));
        copy.close().unwrap();
    });
}

#[test]
fn a_dump_by_a_role_without_schema_restores_the_owner_alone_and_its_log_grants_dormant() {
    let mut sim = Sim::new(543, Config::default());
    api::run(&mut sim, |rt| {
        let db = opened(&rt, "data", &OPS, true, true);
        api::task(&rt, &ops(&db), DUMP).unwrap();
        let src = every(&db);
        db.close().unwrap();
        let (report, copy) = restored(&rt);
        assert!(
            !report.manifest.contains("\npart security "),
            "{}",
            report.manifest
        );
        let dormant: Vec<_> = report
            .dormant
            .iter()
            .map(|d| (d.grantee.as_str(), d.privilege.as_str(), d.target.as_str()))
            .collect();
        assert_eq!(
            vec![
                ("ops", "read", "app::users frozen (id, name)"),
                (
                    "ops",
                    "read",
                    "app::shapes frozen (name, Circle.r, Square.side)"
                ),
            ],
            dormant
        );
        assert_eq!(src, every(&copy));
        assert_eq!(vec!["\"owner\"".to_owned()], rows(&copy, ROLES));
        let roles = keys(&copy, "from r in catalog::roles select r.id");
        let grants = keys(&copy, "from g in catalog::grants select g.id");
        let authority = copy.role_authority();
        authority.create_role("web").unwrap();
        authority
            .grant_capability("web", Capability::Adhoc, false)
            .unwrap();
        let after = keys(&copy, "from r in catalog::roles select r.id");
        assert!(
            after[1] > roles[roles.len() - 1],
            "{roles:?} then {after:?}"
        );
        let after = keys(&copy, "from g in catalog::grants select g.id");
        assert!(
            after[after.len() - 1] > grants[grants.len() - 1],
            "{grants:?} then {after:?}"
        );
        copy.close().unwrap();
    });
}

#[test]
fn a_dump_path_outside_the_export_roots_or_existing_is_refused() {
    let mut sim = Sim::new(544, Config::default());
    api::run(&mut sim, |rt| {
        let db = opened(&rt, "data", &OPS, true, true);
        for path in ["d", "out/../d", "out", "/d"] {
            let reason = format!("denied: path `{path}` is outside export_roots");
            refused_with(
                api::task(&rt, &ops(&db), &format!("dump to \"{path}\"")),
                &reason,
            );
        }
        api::task(&rt, &ops(&db), DUMP).unwrap();
        let in_process = db.query_text(DUMP, &[], &CallOptions::default()).map(drop);
        let reason = "the backup target `data/out/d` holds files";
        refused_with(in_process, reason);
        refused_with(api::task(&rt, &ops(&db), DUMP), reason);
        db.close().unwrap();
    });
}

/// The refusal is the evidence: the simulator's write never reaches the host directory.
#[test]
fn a_dump_through_a_symlink_out_of_the_export_roots_is_refused() {
    let dir = adtb_testkit::scratch::Scratch::new("dump-server-link", "x");
    let base = dir.path().to_path_buf();
    std::fs::create_dir_all(base.join("out")).unwrap();
    std::fs::create_dir_all(base.join("outside")).unwrap();
    std::os::unix::fs::symlink(base.join("outside"), base.join("out/link")).unwrap();
    let mut sim = Sim::new(545, Config::default());
    api::run(&mut sim, |rt| {
        let root = base.to_str().unwrap();
        let db = opened(&rt, root, &OPS, true, true);
        let reason = "denied: path `out/link/d` is outside export_roots";
        refused_with(api::task(&rt, &ops(&db), "dump to \"out/link/d\""), reason);
        db.close().unwrap();
    });
    nothing_written(&sim);
}

#[test]
fn a_dump_through_a_server_is_refused_as_in_process() {
    let mut sim = Sim::new(546, Config::default());
    api::run(&mut sim, |rt| {
        let db = opened(&rt, "data", &OPS, true, true);
        let encrypt = "dump to \"out/d\" encrypt \"k\"";
        let reason = "`dump` with `encrypt` does not run in this engine yet";
        refused_with(db.query_text(encrypt, &[], &CallOptions::default()), reason);
        refused_with(api::task(&rt, &ops(&db), encrypt), reason);
        db.close().unwrap();
    });
    nothing_written(&sim);
}

#[test]
fn a_file_quarantined_whole_dumps_through_a_server_and_refuses_every_other_text() {
    let mut sim = Sim::new(547, Config::default());
    api::run(&mut sim, |rt| {
        let db = opened(&rt, "data", &OPS, true, true);
        let src = every(&db);
        let pager = db.write_path().pager();
        pager.quarantine(Quarantine::Database).unwrap();
        let reason = "fatal: the database is quarantined";
        for text in ["app::users", "print schema", "dump to \"out/e\"\nverify"] {
            refused_with(api::task(&rt, &ops(&db), text), reason);
        }
        api::task(&rt, &ops(&db), DUMP).unwrap();
        db.close().unwrap();
        let (_, copy) = restored(&rt);
        assert_eq!(src, every(&copy));
        copy.close().unwrap();
    });
}
