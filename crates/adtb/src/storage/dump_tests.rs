//! Logical dumps of a database and their restores into new files: the round trip, the refusals,
//! and dump and restore under a failed op or a crash.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use crate::catalog::{Catalog, Dormant, version_hash};
use crate::key::Value;
use crate::runtime::{IoError, Op, OpKind, Runtime as _};
use crate::storage::{self, Envelope, Lsn, Pager};
use crate::test_world::api;
use crate::test_world::grants;
use crate::test_world::load as open;
use crate::{
    ArchiveTarget, CallOptions, Called, Capability, Database, Error, FatalReason, Log, OpenOptions,
    Position, Registration, RejectReason, RestorePoint, TxnOptions, restore_in, restore_report_in,
};
use adtb_lang::evolve::{Privilege, Source};
use adtb_lang::parse::{Options, parse_migration};
use adtb_sim::{Action, Config, Match, Plan, Sim, SimRuntime, Struck};
use adtb_testkit::corpus;
use adtb_testkit::process;
use adtb_testkit::profile::{assert_reached, count, every_case, spread};
use adtb_testkit::search::{self, Files};

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
        ..open::options(2)
    }
}

fn app_log() -> Vec<String> {
    vec![APP.to_owned(), LATER.to_owned()]
}

fn run<R: crate::runtime::Runtime>(db: &Database<R>, text: &str) -> Result<(), Error> {
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

fn dump<R: crate::runtime::Runtime>(db: &Database<R>) -> Result<(), Error> {
    run(db, &format!("dump to \"{DUMP}\""))
}

fn restore(rt: &SimRuntime, to: RestorePoint, options: &OpenOptions) -> Result<(), Error> {
    let archive = ArchiveTarget::path(DUMP).unwrap();
    restore_in(rt.clone(), &archive, COPY, to, options).map(drop)
}

/// Give `FILE` roles `web` and `gone`, a credential and a peer mapping of `web`, a capability
/// granted to it, a setting and their audit rows, then drop `gone`.
fn deploy(rt: &SimRuntime) {
    let db = open::opened(rt, FILE, &Log::new(&[APP.to_owned()]), &open::options(2));
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
    let hash = version_hash(&source.schema().unwrap(), &source).expect("the log resolves");
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
    adtb_testkit::common::sleep(rt, &mut rt.io(), Duration::from_secs(3 * 3600));
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
    let at = process::run(&mut sim, |rt| {
        let db = open::opened(&rt, FILE, &Log::new(&[APP.to_owned()]), &open::options(2));
        run(&db, ROWS).unwrap();
        db.close().unwrap();
        deploy(&rt);
        let db = open::opened(&rt, FILE, &Log::new(app_log()), &open::options(2));
        let feed = register_bundles(&rt, &db);
        db.close().unwrap();
        write_rows(&rt, feed);
        let gui = OpenOptions {
            registrations: vec![bundle("gui-1.0", "", HELLO)],
            ..open::options(2)
        };
        let db = Database::open_in(rt.clone(), FILE, &Log::new(app_log()), &gui).unwrap();
        db.close().unwrap();
        let db = open::opened(&rt, FILE, &Log::new(app_log()), &open::options(2));
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
    let (src, copy) = (
        open::opened(rt, FILE, &Log::new(log), &open::options(2)),
        open::opened(rt, COPY, &Log::new(log), &open::options(2)),
    );
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
        process::run(&mut sim, |rt| {
            let db = open::opened(&rt, FILE, &Log::new(&log), &open::options(2));
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

/// Restore a dump holding a deprecated constructor's rows, a deleted user, roles, a credential
/// verifier, a peer mapping, a setting, audit rows and bundles: every relation answers as its
/// source's.
#[test]
fn a_restore_answers_every_relation_of_the_deployment_as_its_source() {
    let (files, _) = dumped();
    let mut sim = Sim::with_files(2, Config::default(), files);
    process::run(&mut sim, |rt| {
        restore(&rt, RestorePoint::Latest, &no_create()).unwrap();
        assert_same(&rt, &app_log(), &[], "deployed");
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
    process::run(&mut sim, |rt| {
        let given = OpenOptions {
            registrations: vec![bundle("new-1.0", "", HELLO)],
            ..no_create()
        };
        restore(&rt, RestorePoint::Latest, &given).unwrap();
        let (src, copy) = (
            open::opened(&rt, FILE, &Log::new(app_log()), &open::options(2)),
            open::opened(&rt, COPY, &Log::new(app_log()), &open::options(2)),
        );
        for (name, expected) in bundle_outcomes() {
            assert_eq!(expected, outcome(&src, name), "source {name}");
            assert_eq!(expected, outcome(&copy, name), "restored {name}");
        }
        let owned = "family tool is owned by role web".to_owned();
        for db in [&src, &copy] {
            let authority = db.role_authority();
            authority.create_role("ops").unwrap();
            let session = |role| authority.session(role).unwrap();
            let refused = authority.register(&session("ops"), &bundle("tool-2.0", "", HELLO));
            let denied = RejectReason::Denied {
                detail: owned.clone(),
            };
            assert_eq!(Err(Error::Rejected { reason: denied }), refused.map(drop));
            let new = bundle("new-1.0", "", HELLO);
            authority.register(&session("web"), &new).unwrap();
        }
        src.close().unwrap();
        copy.close().unwrap();
    });
}

const GRANTED: &str = "module app\ngrant read users to web\n";

fn granted_log() -> [String; 2] {
    [APP.to_owned(), GRANTED.to_owned()]
}

/// Returns a device holding `FILE` and its dump, whose log grants to a role its security holds,
/// after a deployment grant.
fn granted() -> Files {
    let mut sim = Sim::new(4, Config::default());
    process::run(&mut sim, |rt| {
        open::opened(&rt, FILE, &Log::new(&[APP.to_owned()]), &open::options(2))
            .close()
            .unwrap();
        deploy(&rt);
        let db = open::opened(&rt, FILE, &Log::new(granted_log()), &open::options(2));
        dump(&db).unwrap();
        db.close().unwrap();
    });
    sim.files()
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

/// Restore the dump at `from` to `to` on `rt`, asserting its report and `to` hold the log grant
/// of `FILE` dormant, and `to` no role but the schema owner when `owner` holds.
fn assert_dormant(rt: &SimRuntime, (from, to): (&str, &str), owner: bool) {
    let src = open::opened(rt, FILE, &Log::new(granted_log()), &open::options(2));
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
        privilege: Privilege::Read,
        target: "app::users frozen (id, name)".to_owned(),
    };
    assert_eq!(vec![dormant], got.unwrap().dormant, "{from}");
    let copy = open::opened(rt, to, &Log::new(granted_log()), &open::options(2));
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
fn a_dormant_grant_stays_dormant_through_a_dump_and_restore_once_its_role_exists() {
    let mut sim = Sim::with_files(7, Config::default(), owner_only(&granted()));
    process::run(&mut sim, |rt| {
        assert_dormant(&rt, (DUMP, COPY), true);
        let copy = open::opened(&rt, COPY, &Log::new(granted_log()), &open::options(2));
        copy.role_authority().create_role("web").unwrap();
        run(&copy, "dump to \"again\"").unwrap();
        copy.close().unwrap();
        assert_dormant(&rt, ("again", "third.adt"), false);
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

#[track_caller]
fn refused_invalid(got: Result<(), Error>, detail: &str) {
    let invalid = Error::Rejected {
        reason: RejectReason::Invalid {
            detail: detail.into(),
        },
    };
    adtb_testkit::refused_as(got, &invalid, detail);
}

const HOSTED: &str = r#"module app
add module app
add type Note = { id: Key notes, body: Text }
add relation notes : Note key id generate sequence
define fn slugify (t: Text) -> Text = host "slugify" version "1.0" deterministic
add constraint slugged = from n in notes where slugify(n.body) != n.body select n
"#;

#[test]
fn a_restore_given_a_host_function_at_another_version_refuses_naming_it() {
    let mut sim = Sim::new(6, Config::default());
    process::run(&mut sim, |rt| {
        let hosted = |version: &str| {
            let mut host_fns = crate::HostFns::new();
            host_fns.register("slugify", version, crate::Determinism::Deterministic, |b| {
                Ok(b.clone())
            });
            OpenOptions {
                host_fns,
                ..open::options(2)
            }
        };
        let db = Database::open_in(rt.clone(), FILE, &Log::new([HOSTED]), &hosted("1.0")).unwrap();
        run(&db, "insert notes { body: \"a\" }").unwrap();
        dump(&db).unwrap();
        db.close().unwrap();
        let other = OpenOptions {
            create: None,
            ..hosted("2.0")
        };
        refused_invalid(
            restore(&rt, RestorePoint::Latest, &other),
            "the dumped log does not open: host function slugify is bound at another type",
        );
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
        let got = process::run(&mut sim, |rt| {
            restore(&rt, RestorePoint::Latest, &no_create())
        });
        assert_eq!(*name, api::damaged_dump_part(got), "{name} at {at}");
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
    process::run(&mut sim, |rt| {
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

/// What a restore of a dump answers.
#[derive(Debug, PartialEq, Eq)]
enum Answer {
    Restored,
    Damaged(String),
    BackupDamaged(String),
    Invalid(String),
}

/// Returns what a restore of the dump in `files` answers and how many ops it issued that change a
/// path at the target, asserting a refused one changed no file.
fn answered(files: &Files) -> (Answer, u64) {
    let mut sim = Sim::with_files(18, Config::default(), files.clone());
    let (got, changed) = process::run(&mut sim, |rt| {
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
        Err(Error::Rejected {
            reason: RejectReason::Invalid { detail },
        }) => Answer::Invalid(detail),
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

/// Returns the refusal of a manifest of backup format `format`.
fn unread_format(format: &str) -> Answer {
    Answer::Invalid(format!("this engine does not read backup format {format}"))
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
        (false, true) if edited.contains(kind) => unread_format("2"),
        (false, _) => backup(),
    };
    let mut cases = Vec::new();
    for i in 0..lines.len() {
        let mut dropped = lines.to_vec();
        dropped.remove(i);
        let expect = match i {
            1 => unread_format("2"),
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
            unread_format("2")
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
    let value = |key: &str| lines[at(key)][key.len() + 1..].to_owned();
    let (edition, catalog) = (value("edition"), value("catalog_version"));
    let unread = |edition: &str, catalog: &str| {
        Answer::Invalid(format!(
            "this engine does not read a dump of edition {edition} and catalog version {catalog}"
        ))
    };
    [
        ("format", "0".to_owned(), unread_format("0")),
        ("format", "1".to_owned(), unread_format("1")),
        (
            "format",
            u32::MAX.to_string(),
            unread_format(&u32::MAX.to_string()),
        ),
        ("format", max.clone(), manifest_damaged()),
        ("schema_version", "0".to_owned(), log()),
        ("schema_version", max.clone(), log()),
        ("catalog_version", "0".to_owned(), Answer::Restored),
        ("catalog_version", max.clone(), unread(&edition, &max)),
        ("snapshot", format!("0/{lsn}"), manifest_damaged()),
        ("snapshot", format!("{max}/{lsn}"), manifest_damaged()),
        ("snapshot", format!("{epoch}/0"), Answer::Restored),
        ("snapshot", format!("{epoch}/{max}"), Answer::Restored),
        ("release", "0.0.1-other".to_owned(), Answer::Restored),
        ("edition", "1999".to_owned(), unread("1999", &catalog)),
        ("schema_hash", "ab".repeat(32), log()),
        ("kind", "dumps".to_owned(), unread_format("2")),
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
        let hostile_names = ["../ids", "..", "data/.", "ids/x", "data/../log", "/ids", ""];
        for hostile in hostile_names.into_iter().filter(|_| i == HEADER) {
            let mut edited = lines.to_vec();
            edited[i] = format!("part {hostile} {len} {hash}");
            cases.push((
                format!("name `{hostile}` for {name}"),
                edited,
                manifest_damaged(),
            ));
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
    let empty = "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";
    let mut with_empty = lines;
    with_empty.push(format!("part empty 0 {empty}"));
    with_empty[HEADER..].sort();
    let mut files = files;
    files.insert(format!("{DUMP}/empty"), Vec::new());
    let case = (
        "an empty part".to_owned(),
        with_empty,
        Answer::Damaged("empty".to_owned()),
    );
    assert_answers(&files, &[case]);
}

/// An edit of one line of a text part.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum LineEdit {
    Drop,
    Duplicate,
    SwapNext,
    Garble,
    Rekey,
    Revalue,
    Copy,
}

const LINE_EDITS: [LineEdit; 7] = [
    LineEdit::Drop,
    LineEdit::Duplicate,
    LineEdit::SwapNext,
    LineEdit::Garble,
    LineEdit::Rekey,
    LineEdit::Revalue,
    LineEdit::Copy,
];

/// Returns `line` with the number of its first key raised by 7.
fn rekeyed(line: &str) -> Option<String> {
    let at = line.find(".key(")? + ".key(".len();
    let end = at + line[at..].find(')')?;
    let key: u64 = line[at..end].parse().ok()?;
    Some(format!("{}{}{}", &line[..at], key + 7, &line[end..]))
}

/// Returns `line` with its first text value lengthened by one character.
fn revalued(line: &str) -> Option<String> {
    let open = line.find('"')? + 1;
    let close = open + line[open..].find('"')?;
    Some(format!("{}x{}", &line[..close], &line[close..]))
}

/// Returns `text` with `edit` made at its line `i`, or `None` when the edit has nothing to act on.
fn edited_line(text: &str, i: usize, edit: LineEdit) -> Option<String> {
    let mut lines: Vec<String> = text.split_inclusive('\n').map(str::to_owned).collect();
    match edit {
        LineEdit::Drop => drop(lines.remove(i)),
        LineEdit::Duplicate => lines.insert(i, lines[i].clone()),
        LineEdit::SwapNext if i + 1 < lines.len() => lines.swap(i, i + 1),
        LineEdit::SwapNext => return None,
        LineEdit::Garble => lines[i].insert(0, 'x'),
        LineEdit::Rekey => lines[i] = rekeyed(&lines[i])?,
        LineEdit::Revalue => lines[i] = revalued(&lines[i])?,
        LineEdit::Copy => {
            let copy = rekeyed(&lines[i])?;
            lines.insert(i + 1, copy);
        }
    }
    Some(lines.concat())
}

/// What a restore of a dump gave: its refusal and how many ops it issued that change a path at
/// the target before it removed what they made, or what the restored file answers for every
/// relation.
#[derive(Debug, PartialEq, Eq)]
enum Restore {
    Refused(Answer, u64),
    Answers(Vec<String>),
    Unopenable(String),
    Failed(String),
}

/// Returns what a restore of the dump in `files` gives, the restored file opened with `log`,
/// asserting a refused one changed no file.
fn restored_as(files: &Files, log: &[String]) -> Restore {
    let mut sim = Sim::with_files(18, Config::default(), files.clone());
    let got = process::run(&mut sim, |rt| {
        rt.arm(changes().prefixed(COPY));
        let got = restore_dump(&rt);
        let changed = rt.disarm();
        if got.is_err() {
            return (got, changed, Ok(Vec::new()));
        }
        let copy = match open::open_in(&rt, COPY, &Log::new(log), &no_create()) {
            Ok(copy) => copy,
            Err(e) => return (got, changed, Err(format!("{e:?}"))),
        };
        let answers = every_relation(&copy)
            .iter()
            .map(|text| answer(&copy, text))
            .collect();
        copy.close().unwrap();
        (got, changed, Ok(answers))
    });
    match got {
        (Ok(()), _, Ok(answers)) => Restore::Answers(answers),
        (Ok(()), _, Err(e)) => Restore::Unopenable(e),
        (Err(e), changed, _) => {
            assert_eq!(*files, sim.files(), "the refused restore wrote: {e:?}");
            match e {
                Error::Rejected {
                    reason: RejectReason::DumpDamaged { part },
                } => Restore::Refused(Answer::Damaged(part), changed),
                e => Restore::Failed(format!("{e:?}")),
            }
        }
    }
}

/// Returns whether the dump's contract fixes how a restore answers `edit` at line `i` of text part
/// `part`: a `data/` text is for review and never read; a garbled line, any edit of a part's first
/// line, which states its edition and schema version, a dropped row of `ids`, a rekeyed row of
/// `ids` whose key the log determines, and a changed field of a row whose key it does not damage
/// the part; rows of `ids` swapped within one relation restore, and a key the log does not
/// determine is data the dump states, so its row restores holding the new key.
fn stated(part: &str, lines: &[&str], i: usize, edit: LineEdit) -> bool {
    let row = |at: usize| at > 0 && at < lines.len() && !lines[at].starts_with("data ");
    match (part, edit) {
        (p, _) if p.starts_with("data/") => true,
        (_, LineEdit::Garble) => true,
        _ if i == 0 => true,
        ("ids", LineEdit::SwapNext) => row(i) && row(i + 1),
        ("ids", LineEdit::Drop | LineEdit::Rekey) => row(i),
        ("ids", LineEdit::Revalue | LineEdit::Copy) => row(i) && !log_determines(lines, i),
        _ => false,
    }
}

/// The kinds of stated cell the default profile edits, each once per part; of a `data/` text only a
/// duplicated row.
const DEFAULT_CELLS: [&str; 9] = [
    "Drop first",
    "Garble row",
    "Duplicate row",
    "Drop row",
    "SwapNext row",
    "Rekey determined",
    "Rekey data",
    "Revalue row",
    "Copy row",
];

/// Returns whether `got`, a restore of a dump whose text part `part` has `edit` made at line `i`
/// of its `lines`, answers as the contract states a stated cell, or else is refused naming `part`
/// or answers `unedited`.
fn held(
    got: &Restore,
    (part, lines, i, edit): (&str, &[&str], usize, LineEdit),
    unedited: &[String],
) -> bool {
    let refused = matches!(got, Restore::Refused(Answer::Damaged(p), _) if p == part);
    let same = *got == Restore::Answers(unedited.to_vec());
    let restores =
        part.starts_with("data/") || (part == "ids" && edit == LineEdit::SwapNext && i > 0);
    let data_key = part == "ids" && edit == LineEdit::Rekey && !log_determines(lines, i);
    match (stated(part, lines, i, edit), restores, data_key) {
        (false, _, _) => refused || same,
        (true, true, _) => same,
        (true, false, true) => *got == Restore::Answers(rekeyed_answers(unedited, lines[i])),
        (true, false, false) => refused,
    }
}

/// Returns the relation whose rows line `i` of a text part's `lines` is among, as its `data` line
/// names it.
fn section<'a>(lines: &[&'a str], i: usize) -> Option<&'a str> {
    lines[..=i]
        .iter()
        .rev()
        .find_map(|l| l.strip_prefix("data "))
        .map(str::trim_end)
}

/// Returns whether replaying the log determines the key of line `i` of `ids`: a row of a relation
/// of [`LOGGED`].
fn log_determines(lines: &[&str], i: usize) -> bool {
    section(lines, i)
        .and_then(|s| s.strip_prefix("catalog::"))
        .is_some_and(|r| LOGGED.contains(&r))
}

/// Returns the fields of `row`, a printed constructor value, as name and printed value.
fn fields(row: &str) -> Vec<(&str, &str)> {
    let row = row.trim_end();
    let Some(body) = row
        .find(" { ")
        .and_then(|at| row[at + 3..].strip_suffix(" }"))
    else {
        return Vec::new();
    };
    let (mut out, mut start, mut depth) = (Vec::new(), 0, 0);
    let (mut quoted, mut escaped) = (false, false);
    for (at, c) in body.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            '{' | '(' | '[' if !quoted => depth += 1,
            '}' | ')' | ']' if !quoted => depth -= 1,
            ',' if !quoted && depth == 0 => {
                out.push(&body[start..at]);
                start = at + 2;
            }
            _ => {}
        }
    }
    out.push(&body[start..]);
    out.into_iter().filter_map(|f| f.split_once(": ")).collect()
}

/// Returns the key fields of each catalog relation, by the name a `data` line gives it.
fn catalog_keys() -> BTreeMap<String, Vec<String>> {
    let schema = crate::catalog::engine_schema();
    schema
        .relations
        .iter()
        .filter(|r| schema.module(r.module).name == "catalog")
        .map(|r| (format!("catalog::{}", r.name), r.key.clone()))
        .collect()
}

/// Returns what line `i` of a text part's `lines` holds: a row of a relation of `keys` by its
/// relation and every field of its key, any other line by its text up to a parenthesis and, past
/// its first, which line of that text it is.
fn row_name(lines: &[&str], i: usize, keys: &BTreeMap<String, Vec<String>>) -> String {
    let text = |l: &str| {
        let l = l.trim_end();
        l.split(" (").next().unwrap_or(l).to_owned()
    };
    let line = lines[i].trim_end();
    let keyed = || {
        let relation = section(lines, i).filter(|_| !line.starts_with("data "))?;
        let fields = fields(line);
        let key: Option<Vec<String>> = keys
            .get(relation)?
            .iter()
            .map(|k| {
                let (name, value) = fields.iter().find(|(f, _)| f == k)?;
                Some(format!("{name}: {value}"))
            })
            .collect();
        Some(format!("{relation} {}", key?.join(", ")))
    };
    if i == 0 {
        return "the first line".to_owned();
    }
    keyed().unwrap_or_else(|| {
        let own = text(line);
        match lines[..i].iter().filter(|l| text(l) == own).count() {
            0 => format!("{own:?}"),
            n => format!("{own:?} {}", n + 1),
        }
    })
}

/// How many rows of each relation the profile `deep` edits beside its first and last.
const SPREAD: u64 = 1;

/// Returns the lines of text part `part` of `lines` the profile `deep` edits: every line of `log`
/// and `security`; of any other part its first line, of each relation, and of the lines before the
/// first, the first and last and [`SPREAD`] more evenly spread, the first row of each constructor
/// and, of `ids`, the first row of each module and every row whose key the log does not determine.
fn deep_rows(part: &str, lines: &[&str]) -> BTreeSet<usize> {
    if part == "log" || part == "security" {
        return (0..lines.len()).collect();
    }
    let heads: Vec<usize> = [0]
        .into_iter()
        .chain((1..lines.len()).filter(|&i| lines[i].starts_with("data ")))
        .chain([lines.len()])
        .collect();
    let mut out = BTreeSet::from([0]);
    for w in heads.windows(2) {
        let rows: Vec<usize> = (w[0] + 1..w[1]).collect();
        out.extend(spread(&rows, SPREAD + 2));
        let mut seen = BTreeSet::new();
        for &i in &rows {
            let ctor = lines[i].split(" {").next().unwrap_or_default();
            let module = fields(lines[i])
                .into_iter()
                .find(|(f, _)| *f == "module")
                .map(|(_, m)| m);
            let firsts = [Some(("ctor", ctor)), module.map(|m| ("module", m))];
            if firsts.into_iter().flatten().any(|first| seen.insert(first)) {
                out.insert(i);
            }
        }
    }
    if part == "ids" {
        out.extend(
            (1..lines.len())
                .filter(|&i| !lines[i].starts_with("data ") && !log_determines(lines, i)),
        );
    }
    out
}

/// Returns `answers` with the row `line` prints in place of the row it printed before [`rekeyed`]
/// raised its key, each answer's rows sorted again.
fn rekeyed_answers(answers: &[String], line: &str) -> Vec<String> {
    let edited = rekeyed(line).unwrap();
    let (from, to) = (line.trim_end(), edited.trim_end());
    let mut replaced = 0;
    let out = answers
        .iter()
        .map(|a| {
            let mut rows: Vec<String> = a
                .split('\n')
                .map(|r| {
                    if r == from {
                        replaced += 1;
                        to.to_owned()
                    } else {
                        r.to_owned()
                    }
                })
                .collect();
            rows.sort();
            rows.join("\n")
        })
        .collect();
    assert_eq!(1, replaced, "the unedited restore answers `{from}` once");
    out
}

/// The generated edits that restore to a state other than the unedited dump's, against the
/// contract; each label is the fixture, then the edit, the line as [`row_name`] names it and the
/// part.
const KNOWN_FAILURES: &[&str] = &[
    "dumped: Drop of catalog::audit seq: audit.key(1) of audit",
    "dumped: Rekey of catalog::audit seq: audit.key(1) of audit",
    "dumped: Revalue of catalog::audit seq: audit.key(1) of audit",
    "dumped: Copy of catalog::audit seq: audit.key(1) of audit",
    "dumped: Drop of catalog::audit seq: audit.key(2) of audit",
    "dumped: Rekey of catalog::audit seq: audit.key(2) of audit",
    "dumped: Revalue of catalog::audit seq: audit.key(2) of audit",
    "dumped: Copy of catalog::audit seq: audit.key(2) of audit",
    "dumped: Drop of catalog::audit seq: audit.key(3) of audit",
    "dumped: Rekey of catalog::audit seq: audit.key(3) of audit",
    "dumped: Revalue of catalog::audit seq: audit.key(3) of audit",
    "dumped: Copy of catalog::audit seq: audit.key(3) of audit",
    "dumped: Drop of catalog::audit seq: audit.key(4) of audit",
    "dumped: Rekey of catalog::audit seq: audit.key(4) of audit",
    "dumped: Revalue of catalog::audit seq: audit.key(4) of audit",
    "dumped: Copy of catalog::audit seq: audit.key(4) of audit",
    "dumped: Drop of catalog::audit seq: audit.key(65) of audit",
    "dumped: Rekey of catalog::audit seq: audit.key(65) of audit",
    "dumped: Revalue of catalog::audit seq: audit.key(65) of audit",
    "dumped: Copy of catalog::audit seq: audit.key(65) of audit",
    "dumped: Drop of catalog::registered id: registered.key(1) of bundles/app-0.9",
    "dumped: Rekey of catalog::registered id: registered.key(1) of bundles/app-0.9",
    "dumped: Revalue of catalog::registered id: registered.key(1) of bundles/app-0.9",
    "dumped: Copy of catalog::registered id: registered.key(1) of bundles/app-0.9",
    "dumped: SwapNext of \"bundle app-1.0 pins 2\" of bundles/app-1.0",
    "dumped: SwapNext of \"import app\" of bundles/app-1.0",
    "dumped: Drop of catalog::registered id: registered.key(2) of bundles/app-1.0",
    "dumped: Rekey of catalog::registered id: registered.key(2) of bundles/app-1.0",
    "dumped: Revalue of catalog::registered id: registered.key(2) of bundles/app-1.0",
    "dumped: Copy of catalog::registered id: registered.key(2) of bundles/app-1.0",
    "dumped: Revalue of catalog::feeds id: feeds.key(1) of bundles/app-1.0",
    "dumped: Copy of catalog::feeds id: feeds.key(1) of bundles/app-1.0",
    "dumped: Drop of catalog::feed_members feed: feeds.key(1), relation: relations.key(1026), ctor: constructors.key(1025) of bundles/app-1.0",
    "dumped: Drop of catalog::registered id: registered.key(6) of bundles/gui-1.0",
    "dumped: Rekey of catalog::registered id: registered.key(6) of bundles/gui-1.0",
    "dumped: Revalue of catalog::registered id: registered.key(6) of bundles/gui-1.0",
    "dumped: Copy of catalog::registered id: registered.key(6) of bundles/gui-1.0",
    "dumped: Drop of \"bundle app-0.9\" of bundles/index",
    "dumped: Drop of \"bundle app-1.0\" of bundles/index",
    "dumped: Drop of \"bundle old-1.0\" of bundles/index",
    "dumped: Drop of \"bundle short-1.0\" of bundles/index",
    "dumped: Drop of \"bundle tool-1.0\" of bundles/index",
    "dumped: Drop of \"bundle gui-1.0\" of bundles/index",
    "dumped: Drop of catalog::registered id: registered.key(3) of bundles/old-1.0",
    "dumped: Rekey of catalog::registered id: registered.key(3) of bundles/old-1.0",
    "dumped: Revalue of catalog::registered id: registered.key(3) of bundles/old-1.0",
    "dumped: Copy of catalog::registered id: registered.key(3) of bundles/old-1.0",
    "dumped: Drop of catalog::registered id: registered.key(4) of bundles/short-1.0",
    "dumped: Rekey of catalog::registered id: registered.key(4) of bundles/short-1.0",
    "dumped: Revalue of catalog::registered id: registered.key(4) of bundles/short-1.0",
    "dumped: Copy of catalog::registered id: registered.key(4) of bundles/short-1.0",
    "dumped: SwapNext of \"bundle tool-1.0 pins 2\" of bundles/tool-1.0",
    "dumped: Drop of catalog::registered id: registered.key(5) of bundles/tool-1.0",
    "dumped: Rekey of catalog::registered id: registered.key(5) of bundles/tool-1.0",
    "dumped: Revalue of catalog::registered id: registered.key(5) of bundles/tool-1.0",
    "dumped: Copy of catalog::registered id: registered.key(5) of bundles/tool-1.0",
    "dumped: Copy of catalog::roles id: roles.key(1) of security",
    "dumped: Drop of catalog::roles id: roles.key(2) of security",
    "dumped: Rekey of catalog::roles id: roles.key(2) of security",
    "dumped: Revalue of catalog::roles id: roles.key(2) of security",
    "dumped: Copy of catalog::roles id: roles.key(2) of security",
    "dumped: Drop of catalog::credentials id: credentials.key(1) of security",
    "dumped: Rekey of catalog::credentials id: credentials.key(1) of security",
    "dumped: Copy of catalog::credentials id: credentials.key(1) of security",
    "dumped: Drop of catalog::grants id: grants.key(1) of security",
    "dumped: Rekey of catalog::grants id: grants.key(1) of security",
    "dumped: Copy of catalog::grants id: grants.key(1) of security",
    "dumped: Drop of catalog::peer_mappings listener: \"local\", ordinal: 1 of security",
    "dumped: Revalue of catalog::peer_mappings listener: \"local\", ordinal: 1 of security",
    "dumped: Drop of catalog::settings id: settings.key(346) of settings",
    "dumped: Rekey of catalog::settings id: settings.key(346) of settings",
    "dumped: Copy of catalog::settings id: settings.key(346) of settings",
    "dumped: Drop of catalog::settings id: settings.key(451) of settings",
    "dumped: Rekey of catalog::settings id: settings.key(451) of settings",
    "dumped: Copy of catalog::settings id: settings.key(451) of settings",
];

/// Edit each text part of a dump, and `ids` of a dump whose log grants to a role, by dropping,
/// duplicating, swapping with the next, garbling, rekeying, revaluing and copying rekeyed each line.
/// The default profile edits each kind of `DEFAULT_CELLS` whose answer the contract states once per
/// part, a row of `ids` whose key the log does not determine among them; the profile `deep` edits
/// the lines [`deep_rows`] returns, of the dump whose log grants only the rows of `ids` whose key the
/// log does not determine, and every known failure. Plant parts the schema has
/// no relation for and a part of another constructor's rows. Each restore is refused naming the
/// edited part, writing nothing, or restores a file answering every relation as the unedited
/// dump's restore does; a stated cell answers as the contract states, a rekeyed row of `ids` whose
/// key the log determines refused naming `ids` and one whose key it does not restoring that key.
#[test]
fn every_edit_of_a_dump_text_part_is_refused_naming_it_or_restores_the_same() {
    let (files, _) = dump_only();
    let mut with_grant = granted();
    with_grant.retain(|p, _| p.starts_with("d/"));
    let text_parts = |files: &Files| -> Vec<(String, String)> {
        parts(files)
            .into_iter()
            .filter(|(name, _)| !name.ends_with(".arrows"))
            .map(|(name, len)| {
                let bytes = &files[&format!("{DUMP}/{name}")][..len as usize];
                (name, String::from_utf8(bytes.to_vec()).unwrap())
            })
            .collect()
    };
    let fixtures = [
        ("dumped", &files, app_log(), None),
        ("granted", &with_grant, granted_log().to_vec(), Some("ids")),
    ];
    let mut reached: BTreeSet<(String, String)> = BTreeSet::new();
    let (mut failed, mut passed_known) = (Vec::new(), Vec::new());
    let mut known_reached = BTreeSet::new();
    let keys = catalog_keys();
    for (fixture, dump, log, only) in &fixtures {
        let Restore::Answers(unedited) = restored_as(dump, log) else {
            panic!("{fixture}: the unedited dump does not restore");
        };
        for (part, text) in text_parts(dump) {
            if only.is_some_and(|o| o != part) {
                continue;
            }
            let lines: Vec<&str> = text.split_inclusive('\n').collect();
            let mut kinds = BTreeSet::new();
            let rows = deep_rows(&part, &lines);
            let mut edited = 0;
            for i in 0..lines.len() {
                let determined = log_determines(&lines, i);
                let deep = rows.contains(&i) && (only.is_none() || !determined);
                for edit in LINE_EDITS {
                    let is_stated = stated(&part, &lines, i, edit);
                    let kind = match (i, part == "ids" && edit == LineEdit::Rekey, determined) {
                        (0, _, _) => format!("{edit:?} first"),
                        (_, true, true) => format!("{edit:?} determined"),
                        (_, true, false) => format!("{edit:?} data"),
                        _ => format!("{edit:?} row"),
                    };
                    let sampled = if part.starts_with("data/") {
                        kind == "Duplicate row"
                    } else {
                        DEFAULT_CELLS.contains(&kind.as_str())
                    };
                    let label = format!(
                        "{fixture}: {edit:?} of {} of {part}",
                        row_name(&lines, i, &keys)
                    );
                    let known = KNOWN_FAILURES.contains(&label.as_str());
                    let first_of_kind = is_stated && sampled && kinds.insert(kind.clone());
                    if !first_of_kind && !(every_case() && (deep || known)) {
                        continue;
                    }
                    let Some(changed) = edited_line(&text, i, edit) else {
                        continue;
                    };
                    let bad = reissued(
                        dump,
                        |_| {},
                        |n, t| {
                            if n == part {
                                changed.clone()
                            } else {
                                t.to_owned()
                            }
                        },
                    );
                    let got = restored_as(&bad, log);
                    edited += 1;
                    let held = held(&got, (&part, &lines, i, edit), &unedited);
                    if known {
                        known_reached.insert(label.clone());
                    }
                    match (held, known) {
                        (false, false) => failed.push(format!("{label}, {kind}: {got:?}")),
                        (true, true) => passed_known.push(label),
                        _ => {}
                    }
                    if is_stated {
                        reached.insert((fixture.to_string(), kind));
                    }
                }
            }
            assert_reached(&format!("{fixture}: an edit of {part}"), edited, 1);
        }
    }
    assert!(failed.is_empty(), "against the contract: {failed:#?}");
    assert!(
        passed_known.is_empty(),
        "known failures now hold: {passed_known:#?}"
    );
    if every_case() {
        let unreached: Vec<&&str> = KNOWN_FAILURES
            .iter()
            .filter(|l| !known_reached.contains(**l))
            .collect();
        assert!(
            unreached.is_empty(),
            "known failures never edited: {unreached:#?}"
        );
    }
    for kind in [
        "Garble row",
        "Drop first",
        "Drop row",
        "SwapNext row",
        "Rekey determined",
    ] {
        let cell = ("dumped".to_owned(), kind.to_owned());
        assert!(reached.contains(&cell), "never reached: {cell:?}");
    }
    for kind in ["Rekey determined", "Rekey data", "Revalue row", "Copy row"] {
        let cell = ("granted".to_owned(), kind.to_owned());
        assert!(reached.contains(&cell), "never reached: {cell:?}");
    }
    let entry = |name: &str| {
        let (_, len) = parts(&files).into_iter().find(|(n, _)| n == name).unwrap();
        files[&format!("{DUMP}/{name}")][..len as usize].to_vec()
    };
    let circles = entry("data/app::shapes.Circle.arrows");
    let users = entry("data/app::users.User.arrows");
    let planted: [(&str, &[u8]); 5] = [
        ("data/app::shapes.Square.arrows", &circles),
        ("data/app::shapes.Dot.arrows", &users),
        ("data/app::zz.Z.txt", b"x"),
        ("data/app::zz.Z.arrows", b"x"),
        ("bundles/zz-1", b"x"),
    ];
    for (name, bytes) in planted {
        let bad = with_parts(&files, |_| {}, |_, t| t.to_owned(), &[(name, bytes)]);
        assert_eq!(
            Answer::Damaged(name.to_owned()),
            answered(&bad).0,
            "{name} planted"
        );
    }
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
    let got = process::run(&mut sim, |rt| {
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
    assert_eq!("ids", api::damaged_dump_part(got));
    let mut io_sim = sim;
    let dirs = process::run(&mut io_sim, |rt| {
        let mut io = rt.io();
        let listed = adtb_testkit::common::complete(
            &rt,
            &mut io,
            Op::List {
                dir: "keep".to_owned(),
            },
        );
        listed.result
    });
    let Ok(crate::runtime::Done::Listed(left)) = dirs else {
        panic!("{dirs:?}");
    };
    assert_eq!(vec!["keep/mine".to_owned()], left);
    assert_eq!(bad, io_sim.files());
}

#[test]
fn a_read_failure_creating_the_restored_file_is_fatal_and_leaves_nothing() {
    let (files, _) = dumped();
    let mut sim = Sim::with_files(21, Config::default(), files.clone());
    let read = || Match::kind(OpKind::Read).containing(".creating/");
    let (got, reads) = process::run(&mut sim, |rt| {
        let reads = rt.logged(read());
        rt.plan(Plan::new().first(read(), Action::Fail(IoError::Failed)));
        (restore_dump(&rt), reads.entries())
    });
    let failed = reads
        .first()
        .and_then(|r| r.path())
        .expect("the restore reads its file");
    let (aside, _) = failed.split_once(".creating/").unwrap();
    let detail = format!("`{aside}`: other error");
    assert_eq!(
        Err(Error::Fatal {
            reason: FatalReason::Storage { detail }
        }),
        got
    );
    assert_eq!(files, sim.files());
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
    process::run(&mut sim, |rt| assert_same(&rt, &app_log(), &[], label));
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

fn reads_the_created_file(op: &Struck) -> bool {
    op.kind() == OpKind::Read
        && op
            .path()
            .is_some_and(|p| p.starts_with(&format!("{COPY}.partial-")) && p.contains(".creating/"))
}

fn lists_the_restored_log(op: &Struck) -> bool {
    op.kind() == OpKind::List
        && op
            .path()
            .is_some_and(|p| p.starts_with(&format!("{COPY}.partial-")) && p.ends_with("-log"))
}

type Strikes = fn(&Struck) -> bool;

/// Failed ops the default profile always searches: a read while the new file is formatted, a read
/// of a catalog page while the new file first opens, and the listing of its log when it reopens.
const PINNED: [(u64, Strikes); 3] = [
    (63, reads_the_created_file),
    (74, reads_the_created_file),
    (1177, lists_the_restored_log),
];

#[test]
fn a_restore_with_any_op_failed_publishes_all_or_nothing() {
    let (base, _) = dumped();
    let workload = |rt: &SimRuntime| {
        rt.arm(searched());
        let got = restore_dump(rt);
        rt.disarm();
        got
    };
    let pinned = PINNED.map(|(at, _)| at);
    let mut struck = Vec::new();
    search::each_fatal_failure_at_every_name(
        &base,
        11,
        FAILURES,
        &pinned,
        &workload,
        &mut |failed| {
            let label = format!("op {} {}", failed.at, failed.struck);
            struck.push((
                failed.struck.kind(),
                failed.struck.path().unwrap_or_default().to_owned(),
            ));
            if let Some((_, strikes)) = PINNED.iter().find(|(at, _)| *at == failed.at) {
                assert!(strikes(&failed.struck), "{label}: the pinned op moved");
            }
            match &failed.out {
                Some(Ok(())) => assert!(
                    failed.files.contains_key(COPY),
                    "{label}: nothing published"
                ),
                Some(Err(e)) => {
                    match e {
                        Error::Fatal {
                            reason: FatalReason::Storage { .. },
                        } => {}
                        Error::Fatal {
                            reason: FatalReason::Quarantined { what },
                        } => assert_eq!("the database", what, "{label}"),
                        e => panic!("{label}: {e:?}"),
                    }
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
        },
    );
    assert_publish_failed(&struck);
}

/// Assert `struck`, the kind and path of each failed op of a restore in order, holds each step of
/// the restored file's publish after the rename out of its reserved directory: the two directory
/// forces, the marker's and the reserved directory's removal and the parent's force, then the
/// renames of the log and the file to their own names.
fn assert_publish_failed(struck: &[(OpKind, String)]) {
    let reserved = |p: &str| p.starts_with(&format!("{COPY}.partial-")) && p.ends_with(".creating");
    let at = |kind: OpKind, path: &dyn Fn(&str) -> bool| {
        struck
            .iter()
            .position(|(k, p)| *k == kind && path(p))
            .unwrap_or_else(|| panic!("no failed {kind:?} of the publish: {struck:#?}"))
    };
    let marker = at(OpKind::Remove, &|p| p.ends_with(".creating/id"));
    let parent = |p: &str| p.is_empty();
    let steps = [
        (OpKind::SyncDir, &reserved as &dyn Fn(&str) -> bool),
        (OpKind::SyncDir, &parent),
        (OpKind::Remove, &|p: &str| p.ends_with(".creating/id")),
        (OpKind::RemoveDir, &reserved),
        (OpKind::SyncDir, &parent),
    ];
    for (i, (kind, path)) in steps.iter().enumerate() {
        let (k, p) = &struck[marker + i - 2];
        assert!(*k == *kind && path(p), "publish step {i}: {k:?} of {p:?}");
    }
    at(OpKind::Rename, &|p| p == format!("{COPY}-log"));
    at(OpKind::Rename, &|p| p == COPY);
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
        Box::new(workload),
        &|image| image.contains_key(COPY),
        &mut |at, image| assert_restored_or_absent(&image, at as u64, &format!("event {at}")),
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
    process::run(&mut sim, |rt| {
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
    let db = Database::open_in(rt.clone(), FILE, &Log::new(app_log()), &open::options(2)).unwrap();
    rt.arm(searched().prefixed(DUMP));
    let got = dump(&db);
    rt.disarm();
    db.close().unwrap();
    got
}

#[test]
fn a_dump_with_any_op_failed_writes_all_or_nothing() {
    let base = undumped();
    search::each_failure(&base, 13, FAILURES, &armed_dump, &mut |failed| {
        let label = format!("op {} {}", failed.at, failed.struck);
        let whole = failed.files.contains_key(&format!("{DUMP}/manifest"));
        match &failed.out {
            Ok(()) => assert!(whole, "{label}: answered success with no manifest"),
            Err(e) => assert!(!whole, "{label}: a manifest after the failure {e:?}"),
        }
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
        Box::new(|rt| armed_dump(rt).unwrap()),
        &|image| image.contains_key(&format!("{DUMP}/manifest")),
        &mut |at, image| assert_dumped_or_absent(&image, at as u64, &format!("event {at}")),
    );
}
