//! Logical dumps of a database and their restores into new files: the round trip, the refusals,
//! and dump and restore under a failed op or a crash.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use adtb::catalog::{Catalog, Dormant, version_hash};
use adtb::key::Value;
use adtb::runtime::{IoError, Op, OpKind, Runtime as _};
use adtb::storage::{self, Envelope, Lsn, Pager};
use adtb::{
    ArchiveTarget, CallOptions, Called, Capability, Database, Error, FatalReason, Log, OpenOptions,
    Position, Registration, RejectReason, RestorePoint, Restored, TxnCallOptions, TxnOptions,
    restore_in, restore_report_in,
};
use adtb_lang::evolve::Source;
use adtb_lang::parse::{Options, parse_migration};
use adtb_sim::{Action, Config, Match, Plan, Sim, SimRuntime};
use adtb_simkit::search::{self, Files};
use adtb_simkit::{api, grants};
use adtb_testkit::corpus;
use adtb_testkit::profile::{assert_reached, count, every_case};

const FILE: &str = "app.adt";
const DUMP: &str = "d";
const COPY: &str = "copy.adt";
const APP: &str = r"module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
add type Shape { name: Text } = Circle { r: I64 } | Square { side: I64 }
add relation shapes : Shape key name
";
const ROWS: &str = r#"transact
  insert users { name: "ann" }
  insert users { name: "bo" }
  insert users { name: "cy" }
  insert shapes.Circle { name: "c", r: 2 }
  insert shapes.Square { name: "s", side: 3 }"#;
const LATER: &str = "module app\nadd constructor Shape.Dot { at: I64 }\ndeprecate shapes.Square\n";

fn no_create() -> OpenOptions {
    OpenOptions {
        create: None,
        ..api::options(2)
    }
}

fn open(rt: &SimRuntime, name: &str, log: &[String]) -> Database<SimRuntime> {
    Database::open_in(rt.clone(), name, &Log::new(log), &api::options(2))
        .unwrap_or_else(|e| panic!("{name} opens: {e:?}"))
}

fn app_log() -> Vec<String> {
    vec![APP.to_owned(), LATER.to_owned()]
}

fn run<R: adtb::runtime::Runtime>(db: &Database<R>, text: &str) -> Result<(), Error> {
    db.query_text(text, &[], &CallOptions::default()).map(drop)
}

/// Returns what `text` answers on `db`: its rows printed and sorted, or its error.
fn answer(db: &Database<SimRuntime>, text: &str) -> String {
    match db.query_text(text, &[], &CallOptions::default()) {
        Ok(mut results) => {
            let Some(rows) = results.pop() else {
                return "no result".to_owned();
            };
            let form = rows.form().clone();
            let mut out: Vec<String> = rows
                .map(|r| adtb_lang::textual::print(&r.unwrap(), &form).unwrap())
                .collect();
            out.sort();
            out.join("\n")
        }
        Err(e) => format!("error {e:?}"),
    }
}

/// Returns the texts reading every stored relation of `db`, the catalog's included.
/// The catalog relations the log alone fills, which a restore of the log reproduces.
const LOGGED: [&str; 14] = [
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
];

/// The catalog relations of deployment state a dump carries.
const DEPLOYED: [&str; 10] = [
    "bundles",
    "registered",
    "feeds",
    "feed_members",
    "roles",
    "credentials",
    "grants",
    "peer_mappings",
    "settings",
    "audit",
];

/// Returns the texts reading every relation of a user module of `db`, of [`LOGGED`] and of
/// [`DEPLOYED`].
fn every_relation(db: &Database<SimRuntime>) -> Vec<String> {
    let s = db.schema();
    let user = s
        .relations
        .iter()
        .filter(|r| !r.volatile && !s.module(r.module).engine);
    let user = user.map(|r| format!("{}::{}", s.module(r.module).name, r.name));
    let catalog = LOGGED.iter().chain(&DEPLOYED);
    user.chain(catalog.map(|r| format!("catalog::{r}")))
        .collect()
}

fn dump<R: adtb::runtime::Runtime>(db: &Database<R>) -> Result<(), Error> {
    run(db, &format!("dump to \"{DUMP}\""))
}

fn restore(rt: &SimRuntime, to: RestorePoint, options: &OpenOptions) -> Result<(), Error> {
    let archive = ArchiveTarget::path(DUMP).unwrap();
    restore_in(rt.clone(), &archive, COPY, to, options).map(drop)
}

/// Give `FILE` roles `web` and `gone`, a credential and a peer mapping of `web`, a capability
/// granted to it, a setting and their audit rows, then drop `gone`.
fn deploy(rt: &SimRuntime) {
    let db = open(rt, FILE, &[APP.to_owned()]);
    let authority = db.role_authority();
    for role in ["web", "gone"] {
        authority.create_role(role).unwrap();
    }
    authority
        .grant_capability("web", Capability::Assert, false)
        .unwrap();
    authority.drop_role("gone").unwrap();
    run(&db, "set setting RetryLimit = { retries: 4 }").unwrap();
    db.close().unwrap();
    let credential = vec![
        ("id", Value::I64(1)),
        ("role", Value::I64(2)),
        ("salt", Value::Bytes(b"salt".to_vec())),
        ("iterations", Value::I32(4096)),
        ("stored_key", Value::Bytes(vec![7; 32])),
        ("server_key", Value::Bytes(vec![9; 32])),
    ];
    let mapping = vec![
        ("listener", Value::Text("local".to_owned())),
        ("ordinal", Value::I32(1)),
        ("pattern", Value::Text("svc".to_owned())),
        ("role", Value::Text("web".to_owned())),
    ];
    write_rows(
        rt,
        vec![
            ("credentials", "Credential", credential),
            ("peer_mappings", "PeerMapping", mapping),
        ],
    );
}

/// A catalog relation, one of its constructors and the fields of a row of it.
type CatalogRow = (&'static str, &'static str, Vec<(&'static str, Value)>);

/// Write `rows` into the catalog of the closed `FILE`.
fn write_rows(rt: &SimRuntime, rows: Vec<CatalogRow>) {
    let options = storage::Options {
        page_size: 8192,
        ..storage::Options::new(FILE)
    };
    let (pager, _) = Pager::open(rt.clone(), &options).unwrap();
    let (catalog, txns) = Catalog::open(rt.clone(), &pager).unwrap();
    for (relation, ctor, fields) in rows {
        grants::deploy_fields(&catalog, &pager, &txns, (relation, ctor), fields);
    }
    drop((catalog, txns));
    pager.close();
}

const HELLO: &str = "query hello () -> Text = \"hi\"\n";
const READS: &str = "import app\nquery names () -> Set Text = from u in users select u.name\n";

/// Returns bundle `name` pinned at the last version of [`app_log`], with `clauses`, holding
/// `body`.
fn bundle(name: &str, clauses: &str, body: &str) -> Registration {
    let mut source = Source::default();
    for text in app_log() {
        let file = parse_migration(&text, Options::default()).ok().unwrap();
        source = source.migrate(&file, &text).unwrap();
    }
    let hash = version_hash(&source.schema().unwrap(), &source);
    let hash: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    let pins = format!("pins 2 ({hash}) edition \"2026-10\"");
    Registration::bundle(format!("bundle {name} {pins}{clauses}\n{body}")).unwrap()
}

/// The bundles of [`dumped`], each with the outcome of calling its first query.
fn bundle_outcomes() -> [(&'static str, Result<Vec<String>, RejectReason>); 6] {
    let names = Ok(vec!["\"ann\"".to_owned(), "\"bo\"".to_owned()]);
    let bundle = |name: &str| name.to_owned();
    [
        (
            "app-0.9",
            Err(RejectReason::BundleSuperseded {
                bundle: bundle("app-0.9"),
            }),
        ),
        ("app-1.0", names),
        (
            "gui-1.0",
            Err(RejectReason::BundleSuperseded {
                bundle: bundle("gui-1.0"),
            }),
        ),
        (
            "old-1.0",
            Err(RejectReason::Retired {
                bundle: bundle("old-1.0"),
            }),
        ),
        (
            "short-1.0",
            Err(RejectReason::BundleExpired {
                bundle: bundle("short-1.0"),
                query: String::new(),
            }),
        ),
        ("tool-1.0", Ok(vec!["\"hi\"".to_owned()])),
    ]
}

/// Register the bundles of [`bundle_outcomes`] on `db`, `tool-1.0` as `web`, the owner of its
/// family, then supersede `app-0.9`, retire `old-1.0` and latch the lease of `short-1.0` as
/// expired. Returns a feed of `app-1.0` over `app::users` and its member row.
fn register_bundles(rt: &SimRuntime, db: &Database<SimRuntime>) -> Vec<CatalogRow> {
    let authority = db.role_authority();
    for (name, _) in bundle_outcomes() {
        let (role, clauses, body) = match name {
            "tool-1.0" => ("web", "", HELLO),
            "gui-1.0" => continue,
            "short-1.0" => ("owner", " lease 1h", READS),
            _ => ("owner", "", READS),
        };
        let session = authority.session(role).unwrap();
        authority
            .register(&session, &bundle(name, clauses, body))
            .unwrap();
    }
    run(db, "supersede bundle app-0.9 by app-1.0").unwrap();
    run(db, "retire bundle old-1.0").unwrap();
    adtb_simkit::common::sleep(rt, &mut rt.io(), Duration::from_secs(3 * 3600));
    run(db, "set setting LeaseGrace = { grace: 2h }").unwrap();
    let key = |text: &str| {
        let key = answer(db, text);
        let digits = key.trim_end_matches(')').rsplit('(').next().unwrap();
        Value::I64(digits.parse().unwrap())
    };
    let relation = key("from r in catalog::relations where r.name == \"users\" select r.id");
    let ctor = key("from c in catalog::constructors where c.name == \"User\" select c.id");
    let feed = vec![
        ("id", Value::I64(1)),
        ("bundle", Value::I64(2)),
        ("name", Value::Text("users".to_owned())),
        ("ordinal", Value::I32(1)),
        ("relation", relation.clone()),
        ("set", Value::Text("app::users".to_owned())),
    ];
    let member = vec![
        ("feed", Value::I64(1)),
        ("relation", relation),
        ("ctor", ctor),
    ];
    vec![
        ("feeds", "Feed", feed),
        ("feed_members", "FeedMember", member),
    ]
}

/// Returns a device holding `FILE` with `ROWS`, the deployment state of [`deploy`], the bundles
/// of [`register_bundles`], the third user deleted, and its dump, and the position the dump's snapshot reads.
fn dumped() -> (Files, Position) {
    let mut sim = Sim::new(1, Config::default());
    let at = api::run(&mut sim, |rt| {
        let db = open(&rt, FILE, &[APP.to_owned()]);
        run(&db, ROWS).unwrap();
        db.close().unwrap();
        deploy(&rt);
        let db = open(&rt, FILE, &app_log());
        let feed = register_bundles(&rt, &db);
        db.close().unwrap();
        write_rows(&rt, feed);
        let gui = OpenOptions {
            registrations: vec![bundle("gui-1.0", "", HELLO)],
            ..api::options(2)
        };
        let db = Database::open_in(rt.clone(), FILE, &Log::new(app_log()), &gui).unwrap();
        db.close().unwrap();
        let db = open(&rt, FILE, &app_log());
        run(&db, "delete u in users where u.name == \"cy\"").unwrap();
        let t = db.begin(&TxnOptions::default()).unwrap();
        let at = t.commit(None).unwrap();
        dump(&db).unwrap();
        db.close().unwrap();
        at
    });
    (sim.files(), at)
}

/// Assert the restored `COPY` answers every relation and `queries` as `FILE` does.
fn assert_same(rt: &SimRuntime, log: &[String], queries: &[String], label: &str) {
    let (src, copy) = (open(rt, FILE, log), open(rt, COPY, log));
    for text in every_relation(&src).iter().chain(queries) {
        assert_eq!(answer(&src, text), answer(&copy, text), "{label}: `{text}`");
    }
    src.close().unwrap();
    copy.close().unwrap();
}

/// The conformance scripts the round trip restores by default; the profile `deep` restores all.
const SCRIPTS: u64 = 6;

#[test]
fn a_restored_dump_answers_every_relation_and_query_as_its_source() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance");
    let scripts = corpus::scripts(&dir);
    let n = if every_case() {
        scripts.len()
    } else {
        count(SCRIPTS) as usize
    };
    let mut restored = 0;
    for (seed, script) in (0u64..).zip((0..n).map(|i| &scripts[i * scripts.len() / n])) {
        let case = script.case(seed, 8);
        let log = vec![case.migration.clone()];
        let queries: Vec<String> = case
            .queries
            .iter()
            .map(adtb_testkit::cases::Query::text)
            .collect();
        let mut sim = Sim::new(seed, Config::default());
        api::run(&mut sim, |rt| {
            let db = open(&rt, FILE, &log);
            for w in &case.load {
                run(&db, w).unwrap_or_else(|e| panic!("{}: `{w}`: {e:?}", script.name));
            }
            dump(&db).unwrap_or_else(|e| panic!("{}: {e:?}", script.name));
            db.close().unwrap();
            restore(&rt, RestorePoint::Latest, &no_create())
                .unwrap_or_else(|e| panic!("{}: {e:?}", script.name));
            assert_same(&rt, &log, &queries, &script.name);
        });
        restored += 1;
    }
    assert_reached("a conformance script restored", restored, n);
}

#[test]
fn a_restore_keeps_deprecated_rows_ids_and_the_next_generated_key() {
    let (files, _) = dumped();
    let mut sim = Sim::with_files(2, Config::default(), files);
    api::run(&mut sim, |rt| {
        restore(&rt, RestorePoint::Latest, &no_create()).unwrap();
        assert_same(&rt, &app_log(), &[], "app");
        let copy = open(&rt, COPY, &app_log());
        assert_eq!("3", answer(&copy, "from s in shapes.Square select s.side"));
        let src = open(&rt, FILE, &app_log());
        let drawn = "from u in users where u.name == \"dee\" select u.id";
        for db in [&src, &copy] {
            run(db, "insert users { name: \"dee\" }").unwrap();
        }
        assert_eq!(answer(&src, drawn), answer(&copy, drawn));
        assert_ne!("users.key(3)", answer(&copy, drawn));
        src.close().unwrap();
        copy.close().unwrap();
    });
}

#[test]
fn a_restore_keeps_roles_capabilities_and_the_next_role_id() {
    let (files, _) = dumped();
    let mut sim = Sim::with_files(3, Config::default(), files);
    api::run(&mut sim, |rt| {
        restore(&rt, RestorePoint::Latest, &no_create()).unwrap();
        assert_same(&rt, &app_log(), &[], "deployed");
        let (src, copy) = (open(&rt, FILE, &app_log()), open(&rt, COPY, &app_log()));
        let authority = copy.role_authority();
        let web = authority.session("web").unwrap();
        authority.require(&web, Capability::Assert, false).unwrap();
        assert!(
            matches!(
                authority.require(&web, Capability::Schema, false),
                Err(Error::Rejected {
                    reason: RejectReason::Denied { .. }
                })
            ),
            "web holds only assert"
        );
        let drawn = "from r in catalog::roles where r.name == \"new\" select r.id";
        for db in [&src, &copy] {
            db.role_authority().create_role("new").unwrap();
        }
        assert_eq!(answer(&src, drawn), answer(&copy, drawn));
        assert_ne!("roles.key(3)", answer(&copy, drawn), "gone held 3");
        src.close().unwrap();
        copy.close().unwrap();
    });
}

/// Returns what calling the first query of bundle `name` on `db` answers: its rows or its refusal.
fn outcome(db: &Database<SimRuntime>, name: &str) -> Result<Vec<String>, RejectReason> {
    let called = db
        .registered(name)
        .and_then(|b| b.call(1, &[], &CallOptions::default()));
    match called {
        Ok(Called::Rows(rows)) => Ok(api::printed(rows)),
        Err(Error::Rejected { reason }) => Err(reason),
        other => panic!("{name}: {other:?}"),
    }
}

#[test]
fn each_restored_bundle_answers_or_refuses_its_calls_as_its_source() {
    let (files, _) = dumped();
    let mut sim = Sim::with_files(9, Config::default(), files);
    api::run(&mut sim, |rt| {
        let given = OpenOptions {
            registrations: vec![bundle("new-1.0", "", HELLO)],
            ..no_create()
        };
        restore(&rt, RestorePoint::Latest, &given).unwrap();
        let (src, copy) = (open(&rt, FILE, &app_log()), open(&rt, COPY, &app_log()));
        for (name, expected) in bundle_outcomes() {
            assert_eq!(expected, outcome(&src, name), "source {name}");
            assert_eq!(expected, outcome(&copy, name), "restored {name}");
        }
        run(&copy, "dump to \"again\"").unwrap();
        let owned = "family tool is owned by role web".to_owned();
        for db in [&src, &copy] {
            let authority = db.role_authority();
            authority.create_role("ops").unwrap();
            let session = |role| authority.session(role).unwrap();
            let refused = authority.register(&session("ops"), &bundle("tool-2.0", "", HELLO));
            let denied = |e: &Error| matches!(e, Error::Rejected { reason: RejectReason::Denied { detail } } if *detail == owned);
            assert!(refused.as_ref().is_err_and(denied), "{refused:?}");
            let new = bundle("new-1.0", "", HELLO);
            authority.register(&session("web"), &new).unwrap();
        }
        src.close().unwrap();
        copy.close().unwrap();
    });
    let files = sim.files();
    let bundles = files.iter().filter(|(p, _)| p.starts_with("d/bundles/"));
    let mut again = 0;
    for (path, bytes) in bundles {
        assert_eq!(
            Some(bytes),
            files.get(&path.replacen("d/", "again/", 1)),
            "{path}"
        );
        again += 1;
    }
    assert_eq!(7, again);
}

const GRANTED: &str = "module app\ngrant read users to web\n";

/// Returns the rows of `catalog::grants` of `db`, with their ids and without.
fn grant_rows(db: &Database<SimRuntime>) -> [String; 2] {
    let rows = answer(db, "from g in catalog::grants select g");
    let fields = "kind: g.kind, privilege: g.privilege, target: g.target, grantee: g.grantee, \
                  grantor: g.grantor, grant_option: g.grant_option, dormant: g.dormant";
    let hidden = answer(
        db,
        &format!("from g in catalog::grants select {{ {fields} }}"),
    );
    [rows, hidden]
}

fn granted_log() -> [String; 2] {
    [APP.to_owned(), GRANTED.to_owned()]
}

/// Returns a device holding `FILE` and its dump, whose log grants to a role its security holds,
/// after a deployment grant.
fn granted() -> Files {
    let mut sim = Sim::new(4, Config::default());
    api::run(&mut sim, |rt| {
        open(&rt, FILE, &[APP.to_owned()]).close().unwrap();
        deploy(&rt);
        let db = open(&rt, FILE, &granted_log());
        dump(&db).unwrap();
        db.close().unwrap();
    });
    sim.files()
}

fn report(rt: &SimRuntime) -> Result<Restored, Error> {
    let archive = ArchiveTarget::path(DUMP).unwrap();
    restore_report_in(
        rt.clone(),
        (&archive, None),
        COPY,
        RestorePoint::Latest,
        &no_create(),
    )
}

/// Returns the grants of the source and of the restore of [`granted`], and the restore's dormant
/// grants.
fn restored_grants() -> ([[String; 2]; 2], Vec<Dormant>) {
    let mut sim = Sim::with_files(4, Config::default(), granted());
    api::run(&mut sim, |rt| {
        let log = granted_log();
        let dormant = report(&rt).unwrap().dormant;
        let rows = [FILE, COPY].map(|name| {
            let db = open(&rt, name, &log);
            let rows = grant_rows(&db);
            db.close().unwrap();
            rows
        });
        (rows, dormant)
    })
}

#[test]
fn a_log_grant_to_a_restored_role_restores_active() {
    let ([src, copy], dormant) = restored_grants();
    assert_eq!(Vec::<Dormant>::new(), dormant);
    let active = src[1].contains("kind: Log") && src[1].contains("dormant: False");
    assert!(active, "{}", src[1]);
    assert_eq!(src[1], copy[1]);
}

#[test]
fn a_restore_keeps_the_id_of_each_log_grant() {
    let ([src, copy], _) = restored_grants();
    assert!(!src[0].contains("grants.key(2)"), "{}", src[0]);
    assert_eq!(src[0], copy[0]);
}

#[test]
fn log_grant_ids_that_do_not_pair_with_the_replayed_grants_are_refused_naming_ids() {
    let files = granted();
    let grants = |edit: fn(&str) -> String| {
        let ids = |n: &str, t: &str| match n {
            "ids" => {
                let at = t.find("data catalog::grants\n").unwrap() + "data catalog::grants\n".len();
                let row = t[at..].lines().next().unwrap();
                t.replacen(row, &edit(row), 1)
            }
            _ => t.to_owned(),
        };
        reissued(&files, |_| {}, ids)
    };
    let another = |row: &str| row.replacen("privilege: \"read\"", "privilege: \"insert\"", 1);
    let twice = |row: &str| {
        let key = row.find("grants.key(").unwrap();
        let end = key + row[key..].find(')').unwrap();
        format!("{row}\n{}grants.key(999){}", &row[..key], &row[end + 1..])
    };
    for (label, bad) in [
        ("another privilege", grants(another)),
        ("twice", grants(twice)),
        ("none", grants(|_| String::new())),
    ] {
        assert_eq!("ids", damaged_part(refused_with(bad)), "{label}");
    }
}

/// Returns the security of the dump in `files` holding only the schema owner.
fn owner_only(files: &Files) -> Files {
    let owner_only = |n: &str, t: &str| match n {
        "security" => t
            .split_inclusive('\n')
            .enumerate()
            .filter(|(i, l)| *i == 0 || l.starts_with("data ") || l.contains("roles.key(1),"))
            .map(|(_, l)| l)
            .collect(),
        _ => t.to_owned(),
    };
    reissued(files, |_| {}, owner_only)
}

/// Returns the dump in `files` as one written without `security`, whose `generators` hold no next
/// key of `catalog::roles` or `catalog::grants`.
fn unsecured(files: &Files) -> Files {
    let schema = adtb::catalog::engine_schema();
    let id = |n| schema.ids.relations[&adtb::catalog::values::relation(&schema, n)];
    let drawn = ["roles", "grants"].map(|n| format!("generator {} ", id(n)));
    let kept = |n: &str, t: &str| match n {
        "generators" => t
            .split_inclusive('\n')
            .filter(|l| !drawn.iter().any(|d| l.starts_with(d)))
            .collect(),
        _ => t.to_owned(),
    };
    without(&reissued(files, |_| {}, kept), "security")
}

/// Returns `files` with the dump at `DUMP` written again without its part `name`.
fn without(files: &Files, name: &str) -> Files {
    let (e, snapshot) = manifest_of(files);
    let entries: Vec<(String, Vec<u8>)> = parts(files)
        .into_iter()
        .filter(|(n, _)| n != name)
        .map(|(n, len)| {
            let bytes = files[&format!("{DUMP}/{n}")][..len as usize].to_vec();
            (n, bytes)
        })
        .collect();
    let mut kept = files.clone();
    kept.retain(|p, _| !p.starts_with("d/"));
    let mut sim = Sim::with_files(8, Config::default(), kept);
    api::run(&mut sim, |rt| {
        storage::write_dump(&rt, DUMP, e, snapshot, &entries, None).unwrap();
    });
    sim.files()
}

/// Restore the dump at `from` to `to` on `rt`, asserting its report and `to` hold the log grant
/// of `FILE` dormant, and `to` no role but the schema owner when `owner` holds.
fn assert_dormant(rt: &SimRuntime, (from, to): (&str, &str), owner: bool) {
    let src = open(rt, FILE, &granted_log());
    let read = "from g in catalog::grants where g.privilege == \"read\" select g.id";
    let id = answer(&src, read);
    src.close().unwrap();
    let archive = ArchiveTarget::path(from).unwrap();
    let got = restore_report_in(
        rt.clone(),
        (&archive, None),
        to,
        RestorePoint::Latest,
        &no_create(),
    );
    let dormant = Dormant {
        id: id["grants.key(".len()..id.len() - 1].parse().unwrap(),
        grantee: "web".to_owned(),
        privilege: "read".to_owned(),
        target: "app::users frozen (id, name)".to_owned(),
    };
    assert_eq!(vec![dormant], got.unwrap().dormant, "{from}");
    let copy = open(rt, to, &granted_log());
    assert_eq!(id, answer(&copy, read), "{from}");
    let rows = answer(&copy, "from g in catalog::grants select g.dormant");
    assert_eq!("True", rows, "{from}");
    if owner {
        let roles = answer(&copy, "from r in catalog::roles select r.name");
        assert_eq!("\"owner\"", roles, "{from}");
    }
    copy.close().unwrap();
}

#[test]
fn a_restore_without_the_role_of_a_log_grant_reports_it_dormant() {
    let mut sim = Sim::with_files(5, Config::default(), owner_only(&granted()));
    api::run(&mut sim, |rt| assert_dormant(&rt, (DUMP, COPY), true));
}

#[test]
fn a_dump_without_security_restores_the_owner_alone_and_log_grants_dormant() {
    let mut sim = Sim::with_files(6, Config::default(), unsecured(&granted()));
    api::run(&mut sim, |rt| assert_dormant(&rt, (DUMP, COPY), true));
}

#[test]
fn a_dormant_grant_stays_dormant_through_a_dump_and_restore_once_its_role_exists() {
    let mut sim = Sim::with_files(7, Config::default(), owner_only(&granted()));
    api::run(&mut sim, |rt| {
        assert_dormant(&rt, (DUMP, COPY), true);
        let copy = open(&rt, COPY, &granted_log());
        copy.role_authority().create_role("web").unwrap();
        run(&copy, "dump to \"again\"").unwrap();
        copy.close().unwrap();
        assert_dormant(&rt, ("again", "third.adt"), false);
    });
}

#[test]
fn a_restored_file_begins_an_epoch_past_the_dump_and_keeps_no_past() {
    let (files, at) = dumped();
    let mut sim = Sim::with_files(3, Config::default(), files);
    api::run(&mut sim, |rt| {
        restore(&rt, RestorePoint::Latest, &no_create()).unwrap();
        let copy = open(&rt, COPY, &app_log());
        let past = format!(
            "at commit Position {{ epoch: {}, lsn: {} }} {{ from u in users select u.name }}",
            at.epoch, at.lsn
        );
        let got = copy.query_text(&past, &[], &CallOptions::default());
        assert!(
            matches!(
                &got,
                Err(Error::Rejected {
                    reason: RejectReason::HistoryUnavailable { .. }
                })
            ),
            "{got:?}"
        );
        let mut t = copy.begin(&TxnOptions::default()).unwrap();
        t.query_text(
            "insert users { name: \"eve\" }",
            &[],
            &TxnCallOptions::default(),
        )
        .unwrap();
        let later = t.commit(None).unwrap();
        assert!(
            later.epoch > at.epoch && later.epoch <= at.epoch + (1 << 32),
            "{at:?} then {later:?}"
        );
        run(&copy, "backup to \"bk\"").unwrap();
        copy.close().unwrap();
    });
    let manifest = String::from_utf8_lossy(&sim.files()["bk/manifest"]).into_owned();
    let ended = format!("epoch_ended {} {}", at.epoch, at.lsn);
    assert!(manifest.lines().any(|l| l == ended), "{manifest}");
}

fn expired(bundle: &str) -> Result<Vec<String>, RejectReason> {
    let (bundle, query) = (bundle.to_owned(), String::new());
    Err(RejectReason::BundleExpired { bundle, query })
}

#[test]
fn a_lease_latched_in_the_dumping_session_stays_expired_after_restore() {
    let mut sim = Sim::new(17, Config::default());
    api::run(&mut sim, |rt| {
        let db = open(&rt, FILE, &app_log());
        run(&db, "set setting LeaseGrace = { grace: 60s }").unwrap();
        let owner = db.role_authority().session("owner").unwrap();
        let short = bundle("s-1.0", " lease 1h", HELLO);
        db.role_authority().register(&owner, &short).unwrap();
        adtb_simkit::common::sleep(&rt, &mut rt.io(), Duration::from_secs(2 * 3600));
        run(&db, "set setting LeaseGrace = { grace: 100h }").unwrap();
        dump(&db).unwrap();
        assert_eq!(expired("s-1.0"), outcome(&db, "s-1.0"), "source");
        restore(&rt, RestorePoint::Latest, &no_create()).unwrap();
        let copy = open(&rt, COPY, &app_log());
        assert_eq!(expired("s-1.0"), outcome(&copy, "s-1.0"), "restored");
        copy.close().unwrap();
        db.close().unwrap();
    });
}

#[test]
fn a_registration_after_the_dump_snapshot_leaves_its_family_unowned_in_the_restore() {
    let mut sim = Sim::new(18, Config::default());
    api::run(&mut sim, |rt| {
        let db = open(&rt, FILE, &app_log());
        let read_only = TxnOptions {
            read_only: true,
            ..TxnOptions::default()
        };
        let call = TxnCallOptions::default();
        let mut reader = db.begin(&read_only).unwrap();
        reader.query_text("users", &[], &call).unwrap();
        let authority = db.role_authority();
        authority.create_role("web").unwrap();
        authority
            .register(
                &authority.session("web").unwrap(),
                &bundle("zed-1.0", "", HELLO),
            )
            .unwrap();
        reader
            .query_text(&format!("dump to \"{DUMP}\""), &[], &call)
            .unwrap();
        reader.commit(None).unwrap();
        restore(&rt, RestorePoint::Latest, &no_create()).unwrap();
        let copy = open(&rt, COPY, &app_log());
        let authority = copy.role_authority();
        authority.create_role("ops").unwrap();
        let ops = authority.session("ops").unwrap();
        authority
            .register(&ops, &bundle("zed-2.0", "", HELLO))
            .unwrap();
        copy.close().unwrap();
        db.close().unwrap();
    });
}

#[test]
fn a_dump_in_a_transaction_that_wrote_is_refused_and_a_read_only_one_dumps_its_snapshot() {
    let mut sim = Sim::new(16, Config::default());
    api::run(&mut sim, |rt| {
        let db = open(&rt, FILE, &[APP.to_owned()]);
        run(&db, ROWS).unwrap();
        let mut wrote = db.begin(&TxnOptions::default()).unwrap();
        let call = TxnCallOptions::default();
        wrote
            .query_text("insert users { name: \"rolled\" }", &[], &call)
            .unwrap();
        let got = wrote.query_text(&format!("dump to \"{DUMP}\""), &[], &call);
        is_invalid_naming(
            &got.map(drop),
            "`dump` does not run in a transaction that has written",
        );
        wrote.rollback().unwrap();
        let mut reader = db
            .begin(&TxnOptions {
                read_only: true,
                ..TxnOptions::default()
            })
            .unwrap();
        reader.query_text("users", &[], &call).unwrap();
        run(&db, "insert users { name: \"later\" }").unwrap();
        reader
            .query_text(&format!("dump to \"{DUMP}\""), &[], &call)
            .unwrap();
        reader.commit(None).unwrap();
        restore(&rt, RestorePoint::Latest, &no_create()).unwrap();
        let copy = open(&rt, COPY, &[APP.to_owned()]);
        let names = "from u in users select u.name";
        assert_eq!("\"ann\"\n\"bo\"\n\"cy\"", answer(&copy, names));
        copy.close().unwrap();
        db.close().unwrap();
    });
}

#[test]
fn a_restore_point_other_than_the_snapshot_is_unavailable() {
    let (files, at) = dumped();
    let mut sim = Sim::with_files(4, Config::default(), files);
    api::run(&mut sim, |rt| {
        let other = Position {
            lsn: at.lsn + 1,
            ..at
        };
        for to in [RestorePoint::Position(other), RestorePoint::Version(1)] {
            let got = restore(&rt, to, &no_create());
            assert!(
                matches!(&got, Err(Error::Rejected {
                    reason: RejectReason::RestorePointUnavailable { oldest, newest }
                }) if *oldest == at && *newest == at),
                "{to:?}: {got:?}"
            );
        }
        restore(&rt, RestorePoint::Position(at), &no_create()).unwrap();
    });
}

/// Returns the match of the ops that change the names or bytes on the device.
fn changes() -> Match {
    Match::any_of([
        Match::kind(OpKind::Open).creating(),
        Match::kinds([
            OpKind::CreateNew,
            OpKind::CreateDir,
            OpKind::Write,
            OpKind::Allocate,
            OpKind::Remove,
            OpKind::RemoveDir,
            OpKind::Rename,
        ]),
    ])
}

fn is_invalid_naming(got: &Result<(), Error>, word: &str) {
    assert!(
        matches!(got, Err(e) if api::is_invalid(e) && e.to_string().contains(word)),
        "not invalid naming `{word}`: {got:?}"
    );
}

#[test]
fn a_restore_to_an_existing_path_and_a_dump_to_a_full_directory_are_refused() {
    let (mut files, _) = dumped();
    files.insert(COPY.to_owned(), b"mine".to_vec());
    let mut sim = Sim::with_files(5, Config::default(), files.clone());
    api::run(&mut sim, |rt| {
        rt.arm(changes().prefixed(COPY));
        is_invalid_naming(&restore_dump(&rt), "exists");
        assert_eq!(
            0,
            rt.disarm(),
            "the refused restore changed a path at the target"
        );
        let db =
            Database::open_in(rt.clone(), FILE, &Log::new(app_log()), &api::options(2)).unwrap();
        rt.arm(changes().prefixed(DUMP));
        is_invalid_naming(&dump(&db), "holds files");
        assert_eq!(
            0,
            rt.disarm(),
            "the refused dump changed a path at the target"
        );
        db.close().unwrap();
    });
    let now = sim.files();
    assert_eq!(Some(&b"mine".to_vec()), now.get(COPY));
    for (path, bytes) in files.iter().filter(|(p, _)| p.starts_with("d/")) {
        assert_eq!(Some(bytes), now.get(path), "{path}");
    }
}

const HOSTED: &str = r#"module app
add module app
add type Note = { id: Key notes, body: Text }
add relation notes : Note key id generate sequence
define fn slugify (t: Text) -> Text = host "slugify" version "1.0" deterministic
add constraint slugged = from n in notes where slugify(n.body) != n.body select n
"#;

#[test]
fn a_restore_lacking_a_host_function_refuses_naming_it_and_one_given_it_restores() {
    let mut sim = Sim::new(6, Config::default());
    api::run(&mut sim, |rt| {
        let mut host_fns = adtb::HostFns::new();
        host_fns.register("slugify", "1.0", adtb::Determinism::Deterministic, |b| {
            Ok(b.clone())
        });
        let options = OpenOptions {
            host_fns,
            ..api::options(2)
        };
        let db = Database::open_in(rt.clone(), FILE, &Log::new([HOSTED]), &options).unwrap();
        run(&db, "insert notes { body: \"a\" }").unwrap();
        dump(&db).unwrap();
        db.close().unwrap();
        let got = restore(&rt, RestorePoint::Latest, &no_create());
        assert!(
            matches!(&got, Err(Error::Fatal {
                reason: FatalReason::MissingFunction { function }
            }) if function == "slugify"),
            "{got:?}"
        );
        let mut io = rt.io();
        let listed = adtb_simkit::common::complete(&rt, &mut io, Op::List { dir: String::new() });
        let Ok(adtb::runtime::Done::Listed(top)) = listed.result else {
            panic!("{listed:?}");
        };
        let left: Vec<&String> = top.iter().filter(|p| p.starts_with(COPY)).collect();
        assert!(left.is_empty(), "{left:?}");
        let given = OpenOptions {
            create: None,
            ..options
        };
        restore(&rt, RestorePoint::Latest, &given).unwrap();
        let copy = Database::open_in(rt.clone(), COPY, &Log::new([HOSTED]), &given).unwrap();
        assert_eq!("\"a\"", answer(&copy, "from n in notes select n.body"));
        copy.close().unwrap();
    });
}

/// Returns the content of each part the dump's manifest in `files` lists, by name.
fn parts(files: &Files) -> Vec<(String, u64)> {
    let manifest = String::from_utf8_lossy(&files[&format!("{DUMP}/manifest")]).into_owned();
    manifest
        .lines()
        .filter_map(|l| l.strip_prefix("part "))
        .map(|l| {
            let mut w = l.rsplitn(3, ' ');
            let (_, len, name) = (w.next().unwrap(), w.next().unwrap(), w.next().unwrap());
            (name.to_owned(), len.parse().unwrap())
        })
        .collect()
}

fn damaged_part(got: Result<(), Error>) -> String {
    match got {
        Err(Error::Rejected {
            reason: RejectReason::DumpDamaged { part },
        }) => part,
        other => panic!("not DumpDamaged: {other:?}"),
    }
}

#[test]
fn a_flipped_byte_in_any_part_or_its_padding_is_named_and_nothing_is_written() {
    let (mut files, _) = dumped();
    files.retain(|p, _| p.starts_with("d/"));
    let mut names: Vec<(String, u64)> = parts(&files);
    let manifest_len = files[&format!("{DUMP}/manifest")]
        .iter()
        .position(|&b| b == 0);
    names.push(("manifest".to_owned(), manifest_len.unwrap() as u64));
    let padding = parts(&files).len();
    let within = names[..padding].iter().map(|(name, len)| (name, len / 2));
    let past = names[..padding].iter().map(|(name, len)| (name, *len));
    let (manifest, len) = &names[padding];
    let every = (0..*len).map(|at| (manifest, at));
    let file_len = |name: &str| files[&format!("{DUMP}/{name}")].len() as u64;
    let appended = names[..padding]
        .iter()
        .map(|(name, _)| (name, file_len(name)));
    let mut checked = 0;
    for (name, at) in within.chain(past).chain(appended).chain(every) {
        let mut bad = files.clone();
        let bytes = bad.get_mut(&format!("{DUMP}/{name}")).unwrap();
        if at as usize == bytes.len() {
            bytes.resize(bytes.len() + 4096, 0);
        }
        bytes[at as usize] ^= 0x20;
        let mut sim = Sim::with_files(7, Config::default(), bad.clone());
        let got = api::run(&mut sim, |rt| {
            restore(&rt, RestorePoint::Latest, &no_create())
        });
        assert_eq!(*name, damaged_part(got), "{name} at {at}");
        assert_eq!(bad, sim.files(), "{name}: the restore wrote");
        checked += 1;
    }
    assert_reached(
        "a dump part, its padding or a manifest byte damaged",
        checked,
        500,
    );
}

/// Returns the manifest's envelope and snapshot, read from its lines.
fn manifest_of(files: &Files) -> (Envelope, storage::Position) {
    let text = String::from_utf8_lossy(&files[&format!("{DUMP}/manifest")]).into_owned();
    let found = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix(' '))
            .unwrap()
            .to_owned()
    };
    let (epoch, lsn) = found("snapshot")
        .split_once('/')
        .map(|(e, l)| (e.parse().unwrap(), l.parse().unwrap()))
        .unwrap();
    let envelope = Envelope {
        release: found("release"),
        edition: found("edition"),
        schema_version: found("schema_version").parse().unwrap(),
        schema_hash: found("schema_hash"),
        catalog_version: found("catalog_version").parse().unwrap(),
    };
    (
        envelope,
        storage::Position {
            epoch,
            lsn: Lsn(lsn),
        },
    )
}

/// Write a whole dump at `DUMP` from the entries of the one in `files`, `envelope` editing its
/// envelope and `part` each entry's text by name.
fn reissued(
    files: &Files,
    envelope: impl Fn(&mut Envelope),
    part: impl Fn(&str, &str) -> String,
) -> Files {
    with_parts(files, envelope, part, &[])
}

/// Write a whole dump at `DUMP` as [`reissued`] does, `part` editing only text entries, with each
/// entry of `added` in place of the one of its name.
fn with_parts(
    files: &Files,
    envelope: impl Fn(&mut Envelope),
    part: impl Fn(&str, &str) -> String,
    added: &[(&str, &[u8])],
) -> Files {
    let (mut e, snapshot) = manifest_of(files);
    envelope(&mut e);
    let mut entries: BTreeMap<String, Vec<u8>> = parts(files)
        .into_iter()
        .map(|(name, len)| {
            let bytes = &files[&format!("{DUMP}/{name}")][..len as usize];
            if name.ends_with(".arrows") {
                return (name, bytes.to_vec());
            }
            let text = part(&name, std::str::from_utf8(bytes).unwrap());
            (name, text.into_bytes())
        })
        .collect();
    entries.extend(added.iter().map(|(n, b)| ((*n).to_owned(), b.to_vec())));
    let entries: Vec<(String, Vec<u8>)> = entries.into_iter().collect();
    let mut kept = files.clone();
    kept.retain(|p, _| !p.starts_with("d/"));
    let mut sim = Sim::with_files(8, Config::default(), kept);
    api::run(&mut sim, |rt| {
        storage::write_dump(&rt, DUMP, e, snapshot, &entries, None).unwrap();
    });
    sim.files()
}

/// Returns `files` with the text of the dump's manifest before its checksum edited by `edit`, and
/// sealed again.
fn resealed(files: &Files, edit: impl Fn(&str) -> String) -> Files {
    let path = format!("{DUMP}/manifest");
    let padded = files[&path].len();
    let text = String::from_utf8_lossy(&files[&path])
        .trim_end_matches('\0')
        .to_owned();
    let body = edit(&text[..text.rfind("checksum ").unwrap()]);
    let mut sealed = format!("{body}checksum {}\n", storage::manifest_hash(&body)).into_bytes();
    sealed.resize(padded, 0);
    let mut out = files.clone();
    out.insert(path, sealed);
    out
}

/// Returns what a restore of the dump in `files` answers, asserting it changed no file.
fn refused_with(files: Files) -> Result<(), Error> {
    let mut sim = Sim::with_files(15, Config::default(), files.clone());
    let got = api::run(&mut sim, |rt| {
        restore(&rt, RestorePoint::Latest, &no_create())
    });
    assert_eq!(files, sim.files(), "the refused restore wrote: {got:?}");
    got
}

#[test]
fn a_dump_that_decodes_wrong_is_refused_naming_its_part_and_writes_nothing() {
    let (files, _) = dumped();
    let same = |_: &mut Envelope| {};
    let users = "data/app::users.User.arrows";
    let (e, _) = manifest_of(&files);
    let renumbered = reissued(&files, same, |n, t| match n {
        "log" => t.replacen("migration 1 ", "migration 2 ", 1),
        _ => t.to_owned(),
    });
    let fake = "ab".repeat(32);
    let rehashed = reissued(
        &files,
        |e| e.schema_hash = fake.clone(),
        |_, t| t.replacen(&e.schema_hash, &fake, 1),
    );
    let cases = [
        ("log", renumbered),
        ("log", rehashed),
        (
            "manifest",
            resealed(&files, |b| b.replacen("data arrow", "data text", 1)),
        ),
        (
            "manifest",
            resealed(&files, |b| b.replacen("encrypted 0", "encrypted 1", 1)),
        ),
        (
            "manifest",
            resealed(&files, |b| b.replacen("part ids ", "part ../ids ", 1)),
        ),
        (
            "manifest",
            resealed(&files, |b| b.replacen("part ids ", "part .. ", 1)),
        ),
        (
            "manifest",
            resealed(&files, |b| b.replacen("part ids ", "part data/. ", 1)),
        ),
        (
            "manifest",
            resealed(&files, |b| b.replacen("part ids ", "part ids/x ", 1)),
        ),
        (
            "manifest",
            resealed(&files, |b| {
                b.replacen(&format!("part {users} "), "part data/../log ", 1)
            }),
        ),
    ];
    let generator = |edit: fn(&str) -> String| {
        reissued(&files, same, move |n, t| match n {
            "generators" => edit(t),
            _ => t.to_owned(),
        })
    };
    let header = |t: &str| t.lines().next().unwrap().to_owned() + "\n";
    let square = "data/app::shapes.Square.arrows";
    let entry = |name: &str| {
        let (_, len) = parts(&files).into_iter().find(|(n, _)| n == name).unwrap();
        files[&format!("{DUMP}/{name}")][..len as usize].to_vec()
    };
    let circles = entry("data/app::shapes.Circle.arrows");
    let dot = "data/app::shapes.Dot.arrows";
    let dot_users = entry(users);
    let more = [
        (
            "log",
            reissued(&files, same, |n, t| match n {
                "log" => {
                    let at = t.find("migration 2 ").unwrap();
                    let broken = "module app\ndeprecate shapes.Nope\n";
                    format!("{}migration 2 {}\n{broken}\n", &t[..at], broken.len())
                }
                _ => t.to_owned(),
            }),
        ),
        (
            square,
            with_parts(&files, same, |_, t| t.to_owned(), &[(square, &circles)]),
        ),
        (
            dot,
            with_parts(&files, same, |_, t| t.to_owned(), &[(dot, &dot_users)]),
        ),
        (
            "data/app::zz.Z.txt",
            with_parts(
                &files,
                same,
                |_, t| t.to_owned(),
                &[("data/app::zz.Z.txt", b"x")],
            ),
        ),
        (
            "data/app::zz.Z.arrows",
            with_parts(
                &files,
                same,
                |_, t| t.to_owned(),
                &[("data/app::zz.Z.arrows", b"x")],
            ),
        ),
        ("generators", generator(header)),
        (
            "generators",
            generator(|t| format!("{t}{}\n", t.lines().nth(1).unwrap())),
        ),
        (
            "generators",
            generator(|t| {
                let line = t.lines().nth(1).unwrap();
                let (id, next) = line["generator ".len()..].split_once(' ').unwrap();
                let other: u64 = id.parse::<u64>().unwrap() + 1;
                t.replacen(line, &format!("generator {other} {next}"), 1)
            }),
        ),
    ];
    for (part, bad) in cases.into_iter().chain(more) {
        assert_eq!(part, damaged_part(refused_with(bad)), "{part}");
    }
    let newer = reissued(&files, |e| e.catalog_version += 1, |_, t| t.to_owned());
    let version = format!("catalog version {}", e.catalog_version + 1);
    is_invalid_naming(&refused_with(newer), &version);
    let other = reissued(
        &files,
        |e| e.edition = "1999".to_owned(),
        |_, t| t.to_owned(),
    );
    is_invalid_naming(&refused_with(other), "edition 1999");
}

#[test]
fn deployment_state_that_decodes_wrong_is_refused_naming_its_part_and_writes_nothing() {
    let (files, _) = dumped();
    let edited = |part: &'static str, edit: &dyn Fn(&str) -> String| {
        reissued(
            &files,
            |_| {},
            |n, t| if n == part { edit(t) } else { t.to_owned() },
        )
    };
    let section = |t: &str, relation: &str| {
        let at = t.find(&format!("data catalog::{relation}\n")).unwrap();
        let end = t[at + 5..].find("data ").map_or(t.len(), |e| at + 5 + e);
        format!("{}{}", &t[..at], &t[end..])
    };
    let text = |part: &str| String::from_utf8_lossy(&files[&format!("{DUMP}/{part}")]).into_owned();
    assert!(text("security").contains("\"web\""), "{}", text("security"));
    assert!(
        text("audit").contains("grant assert to web"),
        "{}",
        text("audit")
    );
    let cases = [
        edited("security", &|t| t.replacen("\"owner\"", "\"boss\"", 1)),
        edited("security", &|t| t.replacen("Deployment", "Log", 1)),
        edited("security", &|t| section(t, "peer_mappings")),
        edited("settings", &|t| {
            t.replacen("data catalog::settings", "data catalog::audit", 1)
        }),
        edited("audit", &|t| section(t, "audit")),
    ];
    let named = ["security", "security", "security", "settings", "audit"];
    for (part, bad) in named.into_iter().zip(cases) {
        assert_eq!(part, damaged_part(refused_with(bad)), "{part}");
    }
    let (index, app, tool) = ("bundles/index", "bundles/app-1.0", "bundles/tool-1.0");
    assert!(text(index).contains("family 2 tool\n"), "{}", text(index));
    assert!(text(index).contains("\nexpired 4 "), "{}", text(index));
    assert!(text(tool).contains("\nmark 2 command "), "{}", text(tool));
    assert!(text("bundles/gui-1.0").contains("\nmark 1 open "));
    let after = |t: &str, relation: &str, row: &str, key: &str| {
        let at = t.find(&format!("data catalog::{relation}\n")).unwrap();
        let moved = row.replacen(key, "key(7)", 1);
        assert!(t[at..].contains(row), "{relation}: {t}");
        format!("{}{}", &t[..at], t[at..].replacen(row, &moved, 1))
    };
    let twice = |t: &str| {
        let row = t.lines().find(|l| l.contains("bundles.key(2)")).unwrap();
        t.replacen(
            row,
            &format!("{row}\n{}", row.replacen("key(2)", "key(9)", 1)),
            1,
        )
    };
    let bundles = [
        (index, without(&files, index)),
        (app, without(&files, app)),
        (
            "bundles/zz-1",
            with_parts(
                &files,
                |_| {},
                |_, t| t.to_owned(),
                &[("bundles/zz-1", b"x")],
            ),
        ),
        (
            index,
            edited(index, &|t| t.replacen("family 2 ", "family two ", 1)),
        ),
        (
            index,
            edited(index, &|t| t.replacen("expired 4 ", "expired 4.0 ", 1)),
        ),
        (
            index,
            edited(index, &|t| t.replacen("expired 4 ", "expired 4 x", 1)),
        ),
        (index, edited(index, &|t| format!("{t}latched 4\n"))),
        (
            tool,
            edited(tool, &|t| t.replacen("mark 2 command ", "mark 2 shell ", 1)),
        ),
        (
            app,
            edited(app, &|t| t.replacen("\"app-1.0\"", "\"app-1.1\"", 1)),
        ),
        (app, edited(app, &twice)),
        (
            app,
            edited(app, &|t| {
                after(t, "registered", "bundle: bundles.key(2)", "key(2)")
            }),
        ),
        (
            app,
            edited(app, &|t| {
                after(t, "feeds", "bundle: bundles.key(2)", "key(2)")
            }),
        ),
        (
            app,
            edited(app, &|t| {
                after(t, "feed_members", "feed: feeds.key(1)", "key(1)")
            }),
        ),
        (tool, edited(tool, &|t| t.replacen("mark 2 ", "mark 9 ", 1))),
        (index, edited(index, &|t| format!("{t}bundle app-1.0\n"))),
        (
            index,
            edited(index, &|t| t.replacen("expired 4 ", "expired 99 ", 1)),
        ),
        (
            index,
            edited(index, &|t| t.replacen("family 2 tool", "family 2 zed", 1)),
        ),
        (
            index,
            edited(index, &|t| t.replacen("family 2 tool", "family 9 tool", 1)),
        ),
        (tool, without(&files, "security")),
    ];
    for (part, bad) in bundles {
        assert_eq!(part, damaged_part(refused_with(bad)), "{part}");
    }
}

#[test]
fn a_dump_of_another_release_or_an_older_catalog_version_restores() {
    let (files, _) = dumped();
    let older = reissued(
        &files,
        |e| {
            e.release = "0.0.1-other".to_owned();
            e.catalog_version -= 1;
        },
        |_, t| t.to_owned(),
    );
    let mut sim = Sim::with_files(17, Config::default(), older);
    api::run(&mut sim, |rt| {
        restore(&rt, RestorePoint::Latest, &no_create()).unwrap();
        assert_same(&rt, &app_log(), &[], "another release");
    });
}

/// What a restore of a dump answers.
#[derive(Debug, PartialEq, Eq)]
enum Answer {
    Restored,
    Damaged(String),
    BackupDamaged(String),
    Invalid,
}

/// Returns what a restore of the dump in `files` answers and how many ops it issued that change a
/// path at the target, asserting a refused one changed no file.
fn answered(files: &Files) -> (Answer, u64) {
    let mut sim = Sim::with_files(18, Config::default(), files.clone());
    let (got, changed) = api::run(&mut sim, |rt| {
        rt.arm(changes().prefixed(COPY));
        let got = restore_dump(&rt);
        (got, rt.disarm())
    });
    if got.is_err() {
        assert_eq!(*files, sim.files(), "the refused restore wrote: {got:?}");
    }
    let answer = match got {
        Ok(()) => Answer::Restored,
        Err(Error::Rejected {
            reason: RejectReason::DumpDamaged { part },
        }) => Answer::Damaged(part),
        Err(Error::Rejected {
            reason: RejectReason::BackupDamaged { part },
        }) => Answer::BackupDamaged(part),
        Err(e) if api::is_invalid(&e) => Answer::Invalid,
        Err(e) => panic!("{e:?}"),
    };
    (answer, changed)
}

/// Returns `files` with the dump's manifest holding `lines`, sealed.
fn sealed_with(files: &Files, lines: &[String]) -> Files {
    resealed(files, |_| lines.iter().map(|l| format!("{l}\n")).collect())
}

/// A manifest edit: what it is, the manifest's lines after it, and what a restore answers.
type Edit = (String, Vec<String>, Answer);

/// The lines of a manifest before its checksum.
const HEADER: usize = 11;

/// Returns the device holding only the dump of [`dumped`], and its manifest's lines before the
/// checksum.
fn dump_only() -> (Files, Vec<String>) {
    let (mut files, _) = dumped();
    files.retain(|p, _| p.starts_with("d/"));
    let text = String::from_utf8_lossy(&files[&format!("{DUMP}/manifest")]).into_owned();
    let lines: Vec<String> = text
        .trim_end_matches('\0')
        .lines()
        .filter(|l| !l.starts_with("checksum "))
        .map(str::to_owned)
        .collect();
    let header = lines.iter().take_while(|l| !l.starts_with("part ")).count();
    assert_eq!(HEADER, header);
    (files, lines)
}

fn manifest_damaged() -> Answer {
    Answer::Damaged("manifest".to_owned())
}

fn part_name(line: &str) -> String {
    let words: Vec<&str> = line.rsplitn(3, ' ').collect();
    words[2]["part ".len()..].to_owned()
}

/// Returns each line of `lines` dropped, duplicated, swapped with the next and its key renamed.
fn line_edits(lines: &[String]) -> Vec<Edit> {
    let kind = &lines[1];
    let backup = || Answer::BackupDamaged("manifest".to_owned());
    let judged = |edited: &[String]| match (edited[1] == *kind, edited[0] == lines[0]) {
        (true, _) => manifest_damaged(),
        (false, true) if edited.contains(kind) => Answer::Invalid,
        (false, _) => backup(),
    };
    let mut cases = Vec::new();
    for i in 0..lines.len() {
        let mut dropped = lines.to_vec();
        dropped.remove(i);
        let expect = match i {
            1 => Answer::Invalid,
            i if i < HEADER => judged(&dropped),
            i if part_name(&lines[i]) == "security" => {
                Answer::Damaged("bundles/tool-1.0".to_owned())
            }
            i if part_name(&lines[i]).ends_with(".txt") => Answer::Restored,
            i => Answer::Damaged(part_name(&lines[i])),
        };
        cases.push((format!("drop {i}"), dropped, expect));
        let mut twice = lines.to_vec();
        twice.insert(i, lines[i].clone());
        let expect = judged(&twice);
        cases.push((format!("duplicate {i}"), twice, expect));
        if i + 1 < lines.len() {
            let mut swapped = lines.to_vec();
            swapped.swap(i, i + 1);
            let expect = judged(&swapped);
            cases.push((format!("swap {i}"), swapped, expect));
        }
    }
    for i in 0..HEADER {
        let mut renamed = lines.to_vec();
        renamed[i] = format!("x{}", lines[i]);
        let expect = if i == 1 {
            Answer::Invalid
        } else {
            manifest_damaged()
        };
        cases.push((format!("rename key {i}"), renamed, expect));
    }
    cases
}

/// Returns each field of the header of `lines` set to edge values.
fn field_edits(lines: &[String]) -> Vec<Edit> {
    let at = |key: &str| {
        lines
            .iter()
            .position(|l| l.starts_with(&format!("{key} ")))
            .unwrap()
    };
    let field = |key: &str, value: &str| {
        let mut edited = lines.to_vec();
        edited[at(key)] = format!("{key} {value}");
        edited
    };
    let max = u64::MAX.to_string();
    let (epoch, lsn) = lines[at("snapshot")]["snapshot ".len()..]
        .split_once('/')
        .unwrap();
    let log = || Answer::Damaged("log".to_owned());
    [
        ("format", "0".to_owned(), Answer::Invalid),
        ("format", "1".to_owned(), Answer::Invalid),
        ("format", u32::MAX.to_string(), Answer::Invalid),
        ("format", max.clone(), manifest_damaged()),
        ("schema_version", "0".to_owned(), log()),
        ("schema_version", max.clone(), log()),
        ("catalog_version", "0".to_owned(), Answer::Restored),
        ("catalog_version", max.clone(), Answer::Invalid),
        ("snapshot", format!("0/{lsn}"), manifest_damaged()),
        ("snapshot", format!("{max}/{lsn}"), manifest_damaged()),
        ("snapshot", format!("{epoch}/0"), Answer::Restored),
        ("snapshot", format!("{epoch}/{max}"), Answer::Restored),
        ("release", "0.0.1-other".to_owned(), Answer::Restored),
        ("edition", "1999".to_owned(), Answer::Invalid),
        ("schema_hash", "ab".repeat(32), log()),
        ("kind", "dumps".to_owned(), Answer::Invalid),
        ("data", "text".to_owned(), manifest_damaged()),
        ("encrypted", "1".to_owned(), manifest_damaged()),
    ]
    .into_iter()
    .map(|(key, value, expect)| (format!("{key} {value}"), field(key, &value), expect))
    .collect()
}

/// Returns each part line of `lines` with its length, hash and name edited.
fn part_edits(lines: &[String]) -> Vec<Edit> {
    let mut cases = Vec::new();
    for i in HEADER..lines.len() {
        let w: Vec<&str> = lines[i].rsplitn(3, ' ').collect();
        let (name, len, hash) = (part_name(&lines[i]), w[1], w[0]);
        let past = len.parse::<u64>().unwrap() + 4096;
        let zero = "0".repeat(64);
        for (what, line, damaged) in [
            ("len 0", format!("part {name} 0 {hash}"), name.clone()),
            (
                "len past the file",
                format!("part {name} {past} {hash}"),
                name.clone(),
            ),
            (
                "len max",
                format!("part {name} {} {hash}", u64::MAX),
                name.clone(),
            ),
            ("hash 0", format!("part {name} {len} {zero}"), name.clone()),
            (
                "name",
                format!("part {name}x {len} {hash}"),
                format!("{name}x"),
            ),
        ] {
            let mut edited = lines.to_vec();
            edited[i] = line;
            cases.push((format!("{what} {name}"), edited, Answer::Damaged(damaged)));
        }
    }
    cases
}

/// Assert a restore of each of `cases` over `files` answers as it states, and that one refused
/// before a data part issued no op changing a path at the target.
fn assert_answers(files: &Files, cases: &[Edit]) -> usize {
    for (label, edited, expect) in cases {
        let (got, changed) = answered(&sealed_with(files, edited));
        assert_eq!(*expect, got, "{label}");
        let built = match expect {
            Answer::Restored => true,
            Answer::Damaged(part) => part.starts_with("data/") || part == "empty",
            _ => false,
        };
        assert!(
            built || changed == 0,
            "{label}: {changed} ops at the target before refusing"
        );
    }
    cases.len()
}

#[test]
fn every_edit_of_a_dump_manifest_is_refused_naming_what_it_damaged_or_restores_as_stated() {
    let (files, lines) = dump_only();
    let cases: Vec<Edit> = [line_edits(&lines), field_edits(&lines), part_edits(&lines)]
        .into_iter()
        .flatten()
        .collect();
    let checked = assert_answers(&files, &cases);
    assert_reached("a manifest edit restored", checked, 100);
}

#[test]
fn an_empty_part_is_read_and_named_and_a_dump_read_of_another_kind_is_damage() {
    let (mut files, lines) = dump_only();
    let empty = "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";
    let mut with_empty = lines.clone();
    with_empty.push(format!("part empty 0 {empty}"));
    with_empty[HEADER..].sort();
    files.insert(format!("{DUMP}/empty"), Vec::new());
    let case = (
        "an empty part".to_owned(),
        with_empty,
        Answer::Damaged("empty".to_owned()),
    );
    assert_answers(&files, &[case]);
    let mut not_kind = lines;
    not_kind[1] = "kind backup".to_owned();
    let mut sim = Sim::with_files(22, Config::default(), sealed_with(&files, &not_kind));
    let read = api::run(&mut sim, |rt| storage::read_dump(&rt, DUMP, None).map(drop));
    assert!(
        matches!(&read, Err(storage::BackupError::Damaged { part }) if part == "manifest"),
        "{read:?}"
    );
}

#[test]
fn a_refused_restore_to_a_nested_path_removes_the_directories_it_made_and_no_other() {
    let (mut files, _) = dumped();
    files.insert("keep/mine".to_owned(), b"mine".to_vec());
    let bad = reissued(
        &files,
        |_| {},
        |n, t| match n {
            "ids" => t.replacen("types.key(1024)", "types.key(1030)", 1),
            _ => t.to_owned(),
        },
    );
    let mut sim = Sim::with_files(19, Config::default(), bad.clone());
    let got = api::run(&mut sim, |rt| {
        let archive = ArchiveTarget::path(DUMP).unwrap();
        restore_in(
            rt,
            &archive,
            "keep/a/b/copy.adt",
            RestorePoint::Latest,
            &no_create(),
        )
        .map(drop)
    });
    assert_eq!("ids", damaged_part(got));
    let mut io_sim = sim;
    let dirs = api::run(&mut io_sim, |rt| {
        let mut io = rt.io();
        let listed = adtb_simkit::common::complete(
            &rt,
            &mut io,
            Op::List {
                dir: "keep".to_owned(),
            },
        );
        listed.result
    });
    let Ok(adtb::runtime::Done::Listed(left)) = dirs else {
        panic!("{dirs:?}");
    };
    assert_eq!(vec!["keep/mine".to_owned()], left);
    assert_eq!(bad, io_sim.files());
}

#[test]
fn a_storage_failure_building_the_restored_file_is_fatal_and_leaves_nothing() {
    let (files, _) = dumped();
    let mut sim = Sim::with_files(21, Config::default(), files.clone());
    let got = api::run(&mut sim, |rt| {
        let partial = Match::kind(OpKind::CreateNew).containing(".partial");
        rt.plan(Plan::new().rule(partial, Action::Fail(IoError::Failed)));
        restore_dump(&rt)
    });
    assert!(
        matches!(
            &got,
            Err(Error::Fatal {
                reason: FatalReason::Storage { .. }
            })
        ),
        "{got:?}"
    );
    assert_eq!(files, sim.files());
}

#[test]
fn a_restore_to_a_free_path_with_an_occupied_log_directory_is_refused_naming_it() {
    let (mut files, _) = dumped();
    files.insert(format!("{COPY}-log/x"), b"x".to_vec());
    let mut sim = Sim::with_files(20, Config::default(), files.clone());
    let got = api::run(&mut sim, |rt| {
        restore(&rt, RestorePoint::Latest, &no_create())
    });
    is_invalid_naming(&got, &format!("`{COPY}-log` exists"));
    assert_eq!(files, sim.files());
}

#[test]
fn ids_are_compared_as_rows_not_as_text() {
    let (files, _) = dumped();
    let ids_only = |edit: fn(&str) -> String| {
        move |n: &str, t: &str| if n == "ids" { edit(t) } else { t.to_owned() }
    };
    let reordered = reissued(
        &files,
        |_| {},
        ids_only(|ids| {
            let mut lines: Vec<&str> = ids.lines().collect();
            let types = lines
                .iter()
                .position(|l| l.starts_with("data catalog::types"))
                .unwrap();
            let end = types
                + lines[types + 1..]
                    .iter()
                    .position(|l| l.starts_with("data "))
                    .unwrap();
            lines[types + 1..=end].reverse();
            lines.join("\n") + "\n"
        }),
    );
    let one_off = reissued(
        &files,
        |_| {},
        ids_only(|ids| ids.replacen("types.key(1024)", "types.key(1030)", 1)),
    );
    let mut sim = Sim::with_files(9, Config::default(), reordered);
    api::run(&mut sim, |rt| {
        restore(&rt, RestorePoint::Latest, &no_create()).unwrap();
    });
    assert_eq!("ids", damaged_part(refused_with(one_off)));
}

const FAILURES: u64 = 4;
const CRASH_POINTS: u64 = 4;
const CRASH_IMAGES: usize = 2;

fn is_aside(path: &str) -> bool {
    path.contains(".partial-")
}

/// Accept every op but a close, a timer and the renames of the restored database's create, whose
/// undo renames a log directory into a child, which the simulator does not model.
fn searched() -> Match {
    Match::any()
        .except(Match::kinds([OpKind::Close, OpKind::Timer]))
        .except(Match::kind(OpKind::Rename).from_containing(".creating/"))
}

/// Assert `files` holds no part of a restore at `COPY`, or a whole one answering as its source.
fn assert_restored_or_absent(files: &Files, seed: u64, label: &str) {
    let finals: Vec<&String> = files.keys().filter(|p| !is_aside(p)).collect();
    let has_file = finals.iter().any(|p| *p == COPY);
    let has_log = finals
        .iter()
        .any(|p| p.starts_with(&format!("{COPY}-log/")));
    if !has_file {
        return;
    }
    assert!(has_log, "{label}: a file without its log");
    let mut sim = Sim::with_files(seed, Config::default(), files.clone());
    api::run(&mut sim, |rt| assert_same(&rt, &app_log(), &[], label));
}

fn restore_dump(rt: &SimRuntime) -> Result<(), Error> {
    let archive = ArchiveTarget::path(DUMP).unwrap();
    restore_in(
        rt.clone(),
        &archive,
        COPY,
        RestorePoint::Latest,
        &no_create(),
    )
    .map(drop)
}

#[test]
fn a_restore_with_any_op_failed_publishes_all_or_nothing() {
    let (base, _) = dumped();
    let workload = |rt: &SimRuntime| {
        rt.arm(searched());
        let got = restore_dump(rt);
        rt.disarm();
        got
    };
    search::each_fatal_failure(&base, 11, FAILURES, &[], workload, |failed| {
        let label = format!("op {} {}", failed.at, failed.struck);
        match &failed.out {
            Some(Ok(())) => assert!(
                failed.files.contains_key(COPY),
                "{label}: nothing published"
            ),
            Some(Err(e)) => {
                assert!(
                    matches!(
                        e,
                        Error::Fatal {
                            reason: FatalReason::Storage { .. }
                        }
                    ),
                    "{label}: {e:?}"
                );
                assert!(
                    !failed.files.contains_key(COPY)
                        || (failed.struck.kind(), failed.struck.path())
                            == (OpKind::SyncDir, Some("")),
                    "{label}: published, {e:?}"
                );
            }
            None => {}
        }
        assert_restored_or_absent(&failed.files, 11, &label);
    });
}

#[test]
fn a_restore_crashed_anywhere_leaves_nothing_or_the_whole_file() {
    let (base, _) = dumped();
    let workload = |rt: &SimRuntime| {
        rt.arm(searched());
        restore_dump(rt).unwrap();
        rt.disarm();
    };
    search::each_crash(
        &base,
        12,
        CRASH_POINTS,
        CRASH_IMAGES,
        workload,
        |image| image.contains_key(COPY),
        |at, image| assert_restored_or_absent(&image, at as u64, &format!("event {at}")),
    );
}

/// Assert `files` holds no dump at `DUMP`, or a whole one that restores.
fn assert_dumped_or_absent(files: &Files, seed: u64, label: &str) {
    let manifest = format!("{DUMP}/manifest");
    let finals: BTreeSet<&String> = files.keys().filter(|p| p.starts_with("d/")).collect();
    if !files.contains_key(&manifest) {
        assert!(finals.is_empty(), "{label}: {finals:?}");
        return;
    }
    let mut sim = Sim::with_files(seed, Config::default(), files.clone());
    api::run(&mut sim, |rt| {
        restore(&rt, RestorePoint::Latest, &no_create())
            .unwrap_or_else(|e| panic!("{label}: {e:?}"));
        assert_same(&rt, &app_log(), &[], label);
    });
}

/// Returns the device of [`dumped`] before its dump, with the database closed.
fn undumped() -> Files {
    let (mut files, _) = dumped();
    files.retain(|p, _| !p.starts_with("d/"));
    files
}

fn armed_dump(rt: &SimRuntime) -> Result<(), Error> {
    let db = Database::open_in(rt.clone(), FILE, &Log::new(app_log()), &api::options(2)).unwrap();
    rt.arm(searched().prefixed(DUMP));
    let got = dump(&db);
    rt.disarm();
    db.close().unwrap();
    got
}

#[test]
fn a_dump_with_any_op_failed_writes_all_or_nothing() {
    let base = undumped();
    search::each_failure(&base, 13, FAILURES, armed_dump, |failed| {
        let label = format!("op {} {}", failed.at, failed.struck);
        let whole = failed.files.contains_key(&format!("{DUMP}/manifest"));
        assert_eq!(failed.out.is_ok(), whole, "{label}: {:?}", failed.out);
        assert_dumped_or_absent(&failed.files, 13, &label);
    });
}

#[test]
fn a_dump_crashed_anywhere_leaves_nothing_or_the_whole_dump() {
    let base = undumped();
    search::each_crash(
        &base,
        14,
        CRASH_POINTS,
        CRASH_IMAGES,
        |rt| armed_dump(rt).unwrap(),
        |image| image.contains_key(&format!("{DUMP}/manifest")),
        |at, image| assert_dumped_or_absent(&image, at as u64, &format!("event {at}")),
    );
}
