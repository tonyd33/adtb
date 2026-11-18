use std::collections::{BTreeMap, BTreeSet};

use adtb::key::{CtorId, Entry, Value};
use adtb::testing::{Damage, Object, Pid, Plant, Quarantine, Shape};
use adtb::{CallOptions, Database, Error, FatalReason, Log};
use adtb_sim::{Config, Sim, SimRuntime};
use adtb_simkit::{api, open};
use adtb_testkit::process;

const APP: &str = r"module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
add type Identity { id: Key identities, user: Key users }
  = Google { gid: Text }
  | Apple  { sub: Text }
add relation identities : Identity key id generate sequence
  index (user) in (Google | Apple)
add type Label = { id: Key tags, owner: Key users }
add relation tags : Label key id generate sequence
add constraint no_apple_tags =
  from t in tags
  from a in identities.Apple
  where a.user == t.owner
  select t
";

const USER_INDEX: &str = "index (user) in (Google | Apple)";

/// A constructor id no index of the schema maps; it sorts before every mapped one.
const STRAY: CtorId = CtorId(1);

const PAST: &str = "from n in [identities.key(12)] select n.user";

/// Insert users 1 to 4, a Google identity `10 * i + 1` of each and an Apple identity `10 * i + 2`
/// of users 1 and 2.
fn seed(db: &Database<SimRuntime>) {
    for i in 1..=4 {
        api::results(
            db,
            &format!(
                r#"insert users {{ id: users.key({i}), name: "n{i}" }}
                   insert identities.Google {{ id: identities.key({}), user: users.key({i}), gid: "g{i}" }}"#,
                i * 10 + 1,
            ),
        );
    }
    for i in 1..=2 {
        api::results(
            db,
            &format!(
                r#"insert identities.Apple {{ id: identities.key({}), user: users.key({i}), sub: "a{i}" }}"#,
                i * 10 + 2,
            ),
        );
    }
}

fn runs(db: &Database<SimRuntime>, text: &str) {
    if let Err(e) = db.query_text(text, &[], &CallOptions::default()) {
        panic!("`{text}` runs: {e:?}");
    }
}

fn past_of(at: adtb::Position, text: &str) -> String {
    format!(
        "at commit Position {{ epoch: {}, lsn: {} }} {{ {text} }}",
        at.epoch, at.lsn
    )
}

/// Keep history and commit three updates of the Apple row of key 12, moving its old versions to
/// history. Returns the position after the first.
fn apple_history(db: &Database<SimRuntime>) -> adtb::Position {
    runs(db, "set setting RetentionWindow = { length: 2h }");
    let update = |user: i64| {
        format!(
            "update i in identities.Apple where key i == identities.key(12) \
             set Apple {{ i with user: users.key({user}) }}"
        )
    };
    let commit = |text: &str| {
        let mut t = db.begin(&adtb::TxnOptions::default()).unwrap();
        api::in_txn_results(&mut t, text);
        t.commit(None).unwrap()
    };
    let first = commit(&update(2));
    commit(&update(3));
    commit(&update(4));
    runs(db, "vacuum identities");
    first
}

/// A reader of an index entry's constructor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Via {
    /// A query probing the user index.
    Query,
    /// The incremental check of `no_apple_tags`, probing the user index for Apple rows.
    Constraint,
    /// The unique check of the key index on an insert.
    Unique,
    /// A read of the past, probing the history key index.
    Past,
    /// A vacuum, deleting the entries of the user index whose versions it frees.
    Vacuum,
}

const VIAS: [Via; 5] = [
    Via::Query,
    Via::Constraint,
    Via::Unique,
    Via::Past,
    Via::Vacuum,
];

/// Where a stray entry goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    User,
    Key,
    History,
}

impl Via {
    fn target(self) -> Target {
        match self {
            Via::Query | Via::Constraint | Via::Vacuum => Target::User,
            Via::Unique => Target::Key,
            Via::Past => Target::History,
        }
    }
}

/// The one leaf of the index holding a stray entry, the mark refusing that leaf sets, and the
/// name of the marked object.
struct Planted {
    leaf: Pid,
    mark: Quarantine,
    what: &'static str,
    /// The id of the index verify names: the key index for the history key index.
    named: u64,
    stray: Entry,
}

/// Prepare `db` for `target` and plant an entry of constructor [`STRAY`] in its index. Returns the
/// position a read of the past reads at, when `target` is the history key index.
fn plant(db: &Database<SimRuntime>, target: Target) -> (Planted, Option<adtb::Position>) {
    let t = db.testing();
    let key_index = t.index("app::identities", "key id").unwrap();
    let (index, key, at, what) = match target {
        Target::User => (
            t.index("app::identities", USER_INDEX).unwrap(),
            Value::I64(3),
            None,
            "app::identities index (user) in (Google | Apple)",
        ),
        Target::Key => (key_index, Value::I64(99), None, "app::identities key id"),
        Target::History => {
            let first = apple_history(db);
            (
                t.history_index("app::identities").unwrap(),
                Value::I64(12),
                Some(first),
                "app::identities key id",
            )
        }
    };
    let tid = t.all_entries(index).unwrap()[0].tid;
    let stray = Entry {
        key: vec![key],
        ctor: STRAY,
        tid,
    };
    t.plant(Plant::AddRawEntry {
        index,
        entry: stray.clone(),
    })
    .unwrap();
    let leaf = t
        .leaf_of(index, &stray)
        .unwrap()
        .expect("a leaf holds the stray");
    let marked = key_index_or(index, key_index);
    let Object::Index(id) = marked else {
        panic!("{marked:?} is an index");
    };
    let mark = t.own_mark(marked).unwrap();
    let planted = Planted {
        leaf,
        mark,
        what,
        named: id.0,
        stray,
    };
    (planted, at)
}

/// Returns `index`, or `key_index` for the history key index, which marks its key index.
fn key_index_or(index: Object, key_index: Object) -> Object {
    match index {
        Object::HistoryIndex(_) => key_index,
        other => other,
    }
}

/// The statement `via` runs.
fn statement(via: Via, past: Option<adtb::Position>) -> String {
    match via {
        Via::Query => {
            "from i in identities.Google where i.user == users.key(3) select i.gid".into()
        }
        Via::Constraint => "insert tags { id: tags.key(1), owner: users.key(3) }".into(),
        Via::Unique => {
            r#"insert identities.Google { id: identities.key(99), user: users.key(4), gid: "g" }"#
                .into()
        }
        Via::Past => past_of(past.expect("history is kept"), PAST),
        Via::Vacuum => "vacuum identities".into(),
    }
}

/// What a reader did with the stray entry.
#[derive(Debug, PartialEq, Eq)]
struct Seen {
    got: Result<(), Error>,
    recorded: BTreeMap<Pid, Damage>,
    marks: BTreeSet<Quarantine>,
}

fn read(seed_no: u64, via: Via) -> (Seen, Planted) {
    let mut sim = Sim::new(seed_no, Config::default());
    process::run(&mut sim, |rt| {
        let db = open::opened(&rt, open::FILE, &Log::new([APP]), &open::options(2));
        seed_for(&db, via);
        let (planted, past) = plant(&db, via.target());
        let got = db
            .query_text(&statement(via, past), &[], &CallOptions::default())
            .map(|_| ());
        let probe = db.testing().probe();
        let seen = Seen {
            got,
            recorded: probe.records().pages().clone(),
            marks: probe.marks(),
        };
        (seen, planted)
    })
}

fn seed_for(db: &Database<SimRuntime>, via: Via) {
    seed(db);
    if via == Via::Vacuum {
        runs(
            db,
            "delete i in identities.Google where key i == identities.key(41)",
        );
    }
}

fn refused(what: &str) -> Error {
    Error::Fatal {
        reason: FatalReason::Quarantined {
            what: what.to_owned(),
        },
    }
}

#[test]
fn every_reader_of_an_entry_naming_an_unmapped_constructor_refuses_its_index_leaf() {
    let mut reached = BTreeSet::new();
    let mut seen = Vec::new();
    for (via, seed) in VIAS.into_iter().zip(140..) {
        let (got, planted) = read(seed, via);
        let expected = Seen {
            got: Err(refused(planted.what)),
            recorded: BTreeMap::from([(planted.leaf, Damage::Shape(Shape::Stray))]),
            marks: BTreeSet::from([planted.mark]),
        };
        if got == expected {
            reached.insert(via);
        }
        seen.push((via, seed, got, expected));
    }
    assert_eq!(BTreeSet::from(VIAS), reached, "{seen:#?}");
}

#[test]
fn a_query_reads_its_rows_from_the_heaps_once_the_stray_entry_refused_its_index() {
    let mut sim = Sim::new(150, Config::default());
    process::run(&mut sim, |rt| {
        let db = open::opened(&rt, open::FILE, &Log::new([APP]), &open::options(2));
        seed(&db);
        let (planted, _) = plant(&db, Target::User);
        let text = statement(Via::Query, None);
        assert_eq!(refused(planted.what), api::failure(&db, &text));
        assert_eq!(vec![r#""g3""#.to_owned()], api::query(&db, &text));
    });
}

/// Returns verify's findings over a database whose `target` holds a stray entry, each as its
/// index, constructor and place.
fn verified(seed_no: u64, target: Target) -> (Vec<String>, BTreeSet<Quarantine>, Planted) {
    let mut sim = Sim::new(seed_no, Config::default());
    process::run(&mut sim, |rt| {
        let db = open::opened(&rt, open::FILE, &Log::new([APP]), &open::options(2));
        seed(&db);
        let (planted, _) = plant(&db, target);
        let found = api::printed(db.verify(None).expect("verify runs"));
        assert_eq!(1, found.len(), "{target:?}: {found:?}");
        let fields = ["index", "ctor", "where"]
            .into_iter()
            .flat_map(|field| {
                api::printed(api::result(
                    &db,
                    &format!("from f in sys::verify_findings.Orphan select f.{field}"),
                ))
            })
            .collect();
        (fields, db.testing().probe().marks(), planted)
    })
}

#[test]
fn verify_finds_an_entry_naming_an_unmapped_constructor_as_an_orphan() {
    for (target, seed) in [Target::User, Target::Key, Target::History]
        .into_iter()
        .zip(160..)
    {
        let (found, marks, planted) = verified(seed, target);
        let index = match target {
            Target::History => "history key index",
            Target::User | Target::Key => "index",
        };
        let tid = planted.stray.tid;
        let place = format!(
            "page {} slot {}: a constructor outside the {index}",
            tid.page(),
            tid.slot()
        );
        assert_eq!(
            vec![
                format!("indexes.key({})", planted.named),
                format!("constructors.key({})", STRAY.0),
                format!("{place:?}"),
            ],
            found,
            "{target:?}"
        );
        assert_eq!(BTreeSet::from([planted.mark]), marks, "{target:?}");
    }
}
