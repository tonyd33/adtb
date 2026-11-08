//! A restore writes the dumped rows unchecked, builds every index and checks every constraint
//! once: a dump that violates one is refused with the violation and leaves nothing at the target.

use std::sync::OnceLock;

use adtb::catalog::arrows;
use adtb::runtime::OpKind;
use adtb::storage;
use adtb::{
    ArchiveTarget, CallOptions, Database, Error, FatalReason, Log, OpenOptions, RejectReason,
    RestorePoint, Value, restore_in,
};
use adtb_lang::schema::Schema;
use adtb_lang::textual::{print_document, read_document};
use adtb_lang::types::RelId;
use adtb_sim::{Config, Match, Sim, SimRuntime};
use adtb_simkit::api;
use adtb_simkit::search::{self, Files};

const FILE: &str = "app.adt";
const DUMP: &str = "d";
const EDITED: &str = "e";
const COPY: &str = "copy.adt";
const APP: &str = r#"module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence unique (name)
add type Shape { name: Text } = Circle { r: I64 } | Square { side: I64 }
add relation shapes : Shape key name
add type Email = new Text where scalar_len(self) > 2
add type Pin = { id: I64, owner: Key users, email: Email }
add relation pins : Pin key id
add check round on shapes.Circle (r > 0)
add constraint named = from u in users where u.name == "" select u
add constraint kept = from e in deleted(shapes) select e
add constraint mixed =
  from u in users where u.name == "x" or exists d in deleted(users) where d.id == u.id select u
add constraint was = from p in prior(pins) where p.id > 100 select p
"#;
const ROWS: &str = r#"transact
  insert users { id: users.key(1), name: "ann" }
  insert users { id: users.key(2), name: "bo" }
  insert shapes.Circle { name: "c", r: 2 }
  insert shapes.Square { name: "s", side: 3 }
  insert pins { id: 1, owner: users.key(1), email: Email("a@b") }"#;
const USERS: &str = "data/app::users.User.arrows";
const CIRCLES: &str = "data/app::shapes.Circle.arrows";
const SQUARES: &str = "data/app::shapes.Square.arrows";
const PINS: &str = "data/app::pins.Pin.arrows";

fn log() -> Log {
    Log::new(vec![APP.to_owned()])
}

fn no_create() -> OpenOptions {
    OpenOptions {
        create: None,
        ..api::options(2)
    }
}

fn run(db: &Database<SimRuntime>, text: &str) -> Result<(), Error> {
    db.query_text(text, &[], &CallOptions::default()).map(drop)
}

/// Returns a device holding `FILE` with `ROWS` and its dump at `DUMP`.
fn dumped() -> Files {
    static DUMPED: OnceLock<Files> = OnceLock::new();
    let made = || {
        let mut sim = Sim::new(1, Config::default());
        api::run(&mut sim, |rt| {
            let db = Database::open_in(rt, FILE, &log(), &api::options(2)).unwrap();
            run(&db, ROWS).unwrap();
            run(&db, &format!("dump to \"{DUMP}\"")).unwrap();
            db.close().unwrap();
        });
        sim.files()
    };
    DUMPED.get_or_init(made).clone()
}

/// Returns the device of [`dumped`] with the dump written again at `EDITED`, each row of `added`
/// read into the Arrow entry it names and the text entry beside it.
fn edited(added: &[(&str, &str)]) -> Files {
    let mut sim = Sim::with_files(2, Config::default(), dumped());
    api::run(&mut sim, |rt| {
        let (manifest, mut entries) = storage::read_dump(&rt, DUMP, None).unwrap();
        for (part, row) in added {
            let named = part.strip_suffix(".arrows").unwrap();
            let text_part = format!("{named}.txt");
            let text = std::str::from_utf8(&entries[&text_part]).unwrap();
            let doc = read_document::<Value>(schema(), &format!("{text}{row}\n")).unwrap();
            let elem = schema().relation(doc.relation).elem_def;
            let (_, ctor) = named.rsplit_once('.').unwrap();
            let ctor = schema().ty(elem).ctor_index(ctor).unwrap();
            let stream =
                arrows::write_rows(schema(), doc.relation, ctor, &doc.rows, arrows::BATCH_LIMIT);
            entries.insert((*part).to_owned(), stream);
            let text = print_document(schema(), &doc).unwrap();
            entries.insert(text_part, text.into_bytes());
        }
        let entries: Vec<(String, Vec<u8>)> = entries.into_iter().collect();
        storage::write_dump(
            &rt,
            EDITED,
            manifest.envelope,
            manifest.snapshot,
            &entries,
            None,
        )
        .unwrap();
    });
    sim.files()
}

/// Returns the schema of a database of `APP`.
fn schema() -> &'static Schema {
    static SCHEMA: OnceLock<Schema> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        let mut sim = Sim::new(4, Config::default());
        api::run(&mut sim, |rt| {
            let db = Database::open_in(rt, FILE, &log(), &api::options(2)).unwrap();
            let schema = db.schema().clone();
            db.close().unwrap();
            schema
        })
    })
}

/// Returns the rows of `text` as rows of `relation` of `APP`.
fn rows(relation: &str, text: &[&str]) -> Vec<Value> {
    let doc = format!("data {relation}\n{}\n", text.join("\n"));
    let rows = read_document::<Value>(schema(), &doc).unwrap().rows;
    rows.into_iter().collect()
}

fn restore(rt: &SimRuntime) -> Result<Vec<u64>, Error> {
    let archive = ArchiveTarget::path(EDITED).unwrap();
    restore_in(
        rt.clone(),
        &archive,
        COPY,
        RestorePoint::Latest,
        &no_create(),
    )
}

/// Restore `EDITED` of `files` to `COPY`. Returns the reason it is refused with, once the device
/// is asserted unchanged and `COPY` to open as a path that never held a database does.
fn refused(files: &Files) -> RejectReason {
    let mut sim = Sim::with_files(3, Config::default(), files.clone());
    let reason = api::run(&mut sim, |rt| match restore(&rt) {
        Err(Error::Rejected { reason }) => reason,
        other => panic!("not refused: {other:?}"),
    });
    assert_eq!(files, &sim.files(), "the refused restore wrote");
    api::run(&mut sim, |rt| {
        let open = |name: &str| {
            let got = Database::open_in(rt.clone(), name, &log(), &no_create()).map(drop);
            got.map_err(|e| format!("{e:?}").replace(name, "PATH"))
        };
        let Err(never) = open("never.adt") else {
            panic!("a path that never held a database opens");
        };
        assert_eq!(Err(never), open(COPY));
    });
    reason
}

/// A label, the rows an edit adds by data part, and the refusal they give.
type Case<'a> = (&'a str, Vec<(&'a str, &'a str)>, RejectReason);

fn violated(constraint: &str, report: Vec<Value>) -> RejectReason {
    RejectReason::Violated {
        constraint: constraint.to_owned(),
        report,
        redacted: 0,
    }
}

fn key_violation(relation: &str, key: &str) -> RejectReason {
    RejectReason::KeyViolation {
        relation: relation.to_owned(),
        key: key.to_owned(),
    }
}

const TWIN: &str = r#"User { id: users.key(3), name: "ann" }"#;

/// Returns the violation of the unique on users' names by the first user and [`TWIN`].
fn twins() -> RejectReason {
    let [ann, twin] = [r#"User { id: users.key(1), name: "ann" }"#, TWIN]
        .map(|r| rows("app::users", &[r]).remove(0));
    let pair = |a: &Value, b: &Value| Value::record(vec![a.clone(), b.clone()]);
    violated(
        "app::users.unique(name)",
        vec![pair(&ann, &twin), pair(&twin, &ann)],
    )
}

#[test]
fn a_restored_dump_holds_every_index_so_its_constraints_hold_against_later_writes() {
    let mut sim = Sim::with_files(5, Config::default(), edited(&[]));
    api::run(&mut sim, |rt| {
        restore(&rt).unwrap();
        let db = Database::open_in(rt, COPY, &log(), &no_create()).unwrap();
        let refusal = |text: &str| match run(&db, text) {
            Err(Error::Rejected { reason }) => reason,
            other => panic!("`{text}` not refused: {other:?}"),
        };
        assert_eq!(
            twins(),
            refusal(r#"insert users { id: users.key(3), name: "ann" }"#)
        );
        assert_eq!(
            key_violation("app::shapes", r#"shapes.key("c")"#),
            refusal(r#"insert shapes.Square { name: "c", side: 1 }"#)
        );
        let pin = r#"Pin { id: 1, owner: users.key(1), email: Email("a@b") }"#;
        assert_eq!(
            violated("app::pins.Pin.owner", rows("app::pins", &[pin])),
            refusal("delete u in users where u.id == users.key(1)")
        );
        db.close().unwrap();
    });
}

#[test]
fn a_dump_violating_any_constraint_form_is_refused_with_its_violation_and_leaves_nothing() {
    let stray = r#"Pin { id: 2, owner: users.key(9), email: Email("a@b") }"#;
    let short = r#"Pin { id: 2, owner: users.key(1), email: Email("x") }"#;
    let (zero, blank) = (
        r#"Circle { name: "z", r: 0 }"#,
        r#"User { id: users.key(4), name: "" }"#,
    );
    let (x, late) = (
        r#"User { id: users.key(5), name: "x" }"#,
        r#"Pin { id: 101, owner: users.key(1), email: Email("a@b") }"#,
    );
    let long = format!(
        "User {{ id: users.key(6), name: \"{}\" }}",
        "y".repeat(20_000)
    );
    let one = |part, row| vec![(part, row)];
    let cases: Vec<Case<'_>> = vec![
        (
            "a key held by two constructors",
            one(SQUARES, r#"Square { name: "c", side: 1 }"#),
            key_violation("app::shapes", r#"shapes.key("c")"#),
        ),
        (
            "a key held by two rows of one constructor",
            one(USERS, r#"User { id: users.key(2), name: "cy" }"#),
            key_violation("app::users", "users.key(2)"),
        ),
        ("a unique", one(USERS, TWIN), twins()),
        (
            "a reference",
            one(PINS, stray),
            violated("app::pins.Pin.owner", rows("app::pins", &[stray])),
        ),
        (
            "a domain",
            one(PINS, short),
            violated("app::Email.domain", rows("app::pins", &[short])),
        ),
        (
            "a check",
            one(CIRCLES, zero),
            violated("app::round", rows("app::shapes", &[zero])),
        ),
        (
            "a constraint whose state part a row violates",
            one(USERS, x),
            violated("app::mixed", rows("app::users", &[x])),
        ),
        (
            "a constraint reading prior",
            one(PINS, late),
            violated("app::was", rows("app::pins", &[late])),
        ),
        (
            "an index key too large",
            one(USERS, long.as_str()),
            RejectReason::KeyTooLarge {
                relation: "app::users".to_owned(),
                ctor: "User".to_owned(),
                index: "unique (name)".to_owned(),
                size: 20_002,
                limit: 1607,
            },
        ),
        (
            "a constraint query",
            one(USERS, blank),
            violated("app::named", rows("app::users", &[blank])),
        ),
        (
            "a reference and a unique, the reference's name least",
            vec![(PINS, stray), (USERS, TWIN)],
            violated("app::pins.Pin.owner", rows("app::pins", &[stray])),
        ),
    ];
    for (label, added, expected) in cases {
        assert_eq!(expected, refused(&edited(&added)), "{label}");
    }
}

#[test]
fn a_stream_of_another_row_shape_in_a_part_is_refused_as_damage() {
    let files = edited(&[]);
    let mut sim = Sim::with_files(6, Config::default(), files);
    let moved = api::run(&mut sim, |rt| {
        let (manifest, mut entries) = storage::read_dump(&rt, EDITED, None).unwrap();
        let circles = entries[CIRCLES].clone();
        entries.insert(SQUARES.to_owned(), circles);
        let entries: Vec<(String, Vec<u8>)> = entries.into_iter().collect();
        storage::write_dump(
            &rt,
            "m",
            manifest.envelope,
            manifest.snapshot,
            &entries,
            None,
        )
        .unwrap();
        let archive = ArchiveTarget::path("m").unwrap();
        restore_in(rt, &archive, COPY, RestorePoint::Latest, &no_create())
    });
    assert!(
        matches!(&moved, Err(Error::Rejected { reason: RejectReason::DumpDamaged { part } }) if part == SQUARES),
        "{moved:?}"
    );
}

#[test]
fn a_violating_restore_with_any_op_failed_publishes_nothing() {
    let stray = r#"Pin { id: 2, owner: users.key(9), email: Email("a@b") }"#;
    let base = edited(&[(PINS, stray)]);
    let expected = violated("app::pins.Pin.owner", rows("app::pins", &[stray]));
    let workload = |rt: &SimRuntime| {
        rt.arm(
            Match::any()
                .except(Match::kinds([OpKind::Close, OpKind::Timer]))
                .except(Match::kind(OpKind::Rename).from_containing(".creating/")),
        );
        let archive = ArchiveTarget::path(EDITED).unwrap();
        let got = restore_in(
            rt.clone(),
            &archive,
            COPY,
            RestorePoint::Latest,
            &no_create(),
        );
        rt.disarm();
        got.map(drop)
    };
    let mut reached = 0;
    search::each_fatal_failure(&base, 13, 4, &[], workload, |failed| {
        let label = format!("op {} {}", failed.at, failed.struck);
        match &failed.out {
            Some(Err(Error::Rejected { reason })) => assert_eq!(&expected, reason, "{label}"),
            Some(Err(Error::Fatal {
                reason: FatalReason::Storage { .. },
            })) => reached += 1,
            None => {}
            other => panic!("{label}: {other:?}"),
        }
        assert!(!failed.files.contains_key(COPY), "{label}: published");
    });
    assert!(reached > 0, "no failure reached the restore");
}

#[test]
fn a_restore_resumes_a_generator_past_every_restored_key() {
    let mut sim = Sim::with_files(7, Config::default(), dumped());
    let users = schema().relations.iter().position(|r| r.name == "users");
    let id = schema().ids.relations[&RelId(u32::try_from(users.unwrap()).unwrap())];
    let dee = api::run(&mut sim, |rt| {
        let (manifest, mut entries) = storage::read_dump(&rt, DUMP, None).unwrap();
        let text = std::str::from_utf8(&entries["generators"]).unwrap();
        let at = format!("generator {id} 3\n");
        assert!(text.contains(&at), "{text}");
        let text = text.replace(&at, &format!("generator {id} 1\n"));
        entries.insert("generators".to_owned(), text.into_bytes());
        let entries: Vec<(String, Vec<u8>)> = entries.into_iter().collect();
        storage::write_dump(
            &rt,
            EDITED,
            manifest.envelope,
            manifest.snapshot,
            &entries,
            None,
        )
        .unwrap();
        restore(&rt).unwrap();
        let db = Database::open_in(rt, COPY, &log(), &no_create()).unwrap();
        run(&db, r#"insert users { name: "dee" }"#).unwrap();
        let read = r#"from u in users where u.name == "dee" select u"#;
        let mut got = db.query_text(read, &[], &CallOptions::default()).unwrap();
        let dee: Vec<Value> = got.pop().unwrap().map(Result::unwrap).collect();
        db.close().unwrap();
        dee
    });
    let expected = rows("app::users", &[r#"User { id: users.key(3), name: "dee" }"#]);
    assert_eq!(expected, dee);
}
