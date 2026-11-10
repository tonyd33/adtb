//! The rows `load::rows_with_noise` loads through the public API: the input rows are what a later
//! snapshot reads, and every noise version it makes is in the heap and hidden from that snapshot.

use std::collections::{BTreeMap, BTreeSet};

use adtb::Value as Val;
use adtb::key::Value as Stored;
use adtb::testing::{Knobs, Pid};
use adtb::{Database, OpenOptions, TxnOptions};
use adtb_lang::schema::ValueDef;
use adtb_sim::{Config, Outcome, Sim, SimRuntime};
use adtb_simkit::{load, open};
use adtb_testkit::fixture::Fixture;

use crate::constraints::read;

const DECLS: &str = "type Item = { id: Key items, name: Text }
relation items : Item key id generate sequence
type Count = { id: Key counts, n: I64 }
relation counts : Count key id generate sequence";

fn options() -> OpenOptions {
    OpenOptions {
        testing: Knobs::without_background_vacuum(),
        ..open::options(1)
    }
}

fn read_only() -> TxnOptions {
    TxnOptions {
        read_only: true,
        ..TxnOptions::default()
    }
}

/// Returns the fixture of items 1 to `n`, item `i` named `item{i}` and a run of letters, and of
/// one count.
fn fixture(n: i64) -> Fixture {
    let mut f = Fixture::new(DECLS);
    let items: Vec<String> = (1..=n)
        .map(|i| format!(r#"Item {{ id: items.key({i}), name: "{}" }}"#, name(i)))
        .collect();
    for chunk in items.chunks(200) {
        f.rows("items", &format!("[{}]", chunk.join(", ")));
    }
    f.rows("counts", "[Count { id: counts.key(7), n: 5 }]");
    f
}

fn name(i: i64) -> String {
    format!("item{i}{}", "x".repeat(40))
}

/// Items enough to fill several heap pages with each kind of version.
const MANY: i64 = 2000;

/// Who sees a version: the snapshot after the load, the transaction the load leaves running,
/// or neither.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Seen {
    Later,
    Running,
    Neither,
}

/// Each version of a relation, its fields as `Debug` prints them, with who sees it, counted.
type Versions = BTreeMap<(String, Seen), usize>;

/// Returns each version of relation `name` of module `app`, its fields as `Debug` prints them,
/// with who sees it, counted.
fn versions(
    db: &Database<SimRuntime>,
    running: &adtb::AdHocTransaction<'_, SimRuntime>,
    name: &str,
) -> Versions {
    let schema = db.schema();
    let app = schema.module_named("app").expect("the module app");
    let Some(&ValueDef::Relation(rel)) = schema.module(app).values.get(name) else {
        panic!("app has no relation {name}")
    };
    let heap = db.testing().plan_catalog().heap(rel, 0);
    let reader = db.begin(&read_only()).expect("a reader");
    let later = db.testing().store(&reader).expect("the reader is open");
    let mine = db
        .testing()
        .store(running)
        .expect("the running transaction is open");
    let mut out = BTreeMap::new();
    let stored = &later.heaps[&heap].heap;
    stored
        .scan(&later.pager, |v| {
            let at = (Pid(v.tid.page()), stored.id());
            let seen = match (
                later
                    .reads
                    .saw(at, &v.header.writers)
                    .expect("a visibility check"),
                mine.reads
                    .saw(at, &v.header.writers)
                    .expect("a visibility check"),
            ) {
                (true, true) => panic!("both snapshots see {:?}", v.row),
                (true, false) => Seen::Later,
                (false, true) => Seen::Running,
                (false, false) => Seen::Neither,
            };
            *out.entry((format!("{:?}", v.row), seen)).or_default() += 1;
        })
        .expect("a scan of the heap");
    drop((later, mine));
    reader.rollback().expect("the reader ends");
    out
}

fn item(id: i64, name: &str) -> String {
    format!("{:?}", [Stored::I64(id), Stored::Text(name.into())])
}

fn run(n: i64, body: impl FnOnce(&Database<SimRuntime>, &Fixture)) {
    let f = fixture(n);
    let mut sim = Sim::new(0, Config::default());
    let outcome = sim.run(|rt| {
        let decls = f.decls().expect("the fixture has declarations");
        body(&load::open_with(&rt, decls, &options()), &f);
    });
    assert_eq!(Outcome::Finished, outcome);
}

#[test]
fn a_later_snapshot_reads_exactly_the_loaded_rows() {
    run(2, |db, f| {
        let _running = load::rows_with_noise(db, &f.schema, &f.db);
        let row = |id, v| Val::ctor(0, vec![Val::Int(id), v]);
        assert_eq!(
            BTreeSet::from([row(1, Val::text(&name(1))), row(2, Val::text(&name(2)))]),
            read(db, "from x in app::items select x")
        );
        assert_eq!(
            BTreeSet::from([row(7, Val::Int(5))]),
            read(db, "from x in app::counts select x")
        );
    });
}

/// Returns the versions of `n` items and one count that [`load::rows_with_noise`] leaves: each
/// item's row seen by a later snapshot, its running copy seen by the running transaction, and its
/// aborted copy, deleted copy and earlier value seen by neither; the count's row, and its aborted
/// and deleted copies, the row itself.
fn noise_of(n: i64) -> (Versions, Versions) {
    let mut items = BTreeMap::new();
    for id in 1..=n {
        let row = name(id);
        items.insert((item(id, &row), Seen::Later), 1);
        items.insert((item(id, &format!("{row}~r")), Seen::Running), 1);
        for shape in ["~a", "~d", "~e"] {
            items.insert((item(id, &format!("{row}{shape}")), Seen::Neither), 1);
        }
    }
    let count = format!("{:?}", [Stored::I64(7), Stored::I64(5)]);
    let counts = BTreeMap::from([
        ((count.clone(), Seen::Later), 1),
        ((count, Seen::Neither), 2),
    ]);
    (items, counts)
}

#[test]
fn every_noise_version_is_in_the_heap_and_hidden_from_a_later_snapshot() {
    run(2, |db, f| {
        let running = load::rows_with_noise(db, &f.schema, &f.db);
        let (items, counts) = noise_of(2);
        assert_eq!(items, versions(db, &running, "items"));
        assert_eq!(counts, versions(db, &running, "counts"));
    });
}

#[test]
fn every_committed_noise_version_over_several_pages_is_in_the_heap_and_hidden() {
    run(MANY, |db, f| {
        let running = load::rows_with_noise(db, &f.schema, &f.db);
        let heap = db
            .testing()
            .heap("app::items", "Item")
            .expect("the heap of items");
        let pages = db.testing().pages(heap).expect("the heap's pages");
        assert!(pages.len() > 4, "{} pages", pages.len());
        let (items, counts) = noise_of(MANY);
        let got = versions(db, &running, "items");
        let pruned: BTreeMap<_, _> = items
            .iter()
            .filter(|(k, _)| !got.contains_key(*k))
            .map(|(k, n)| (k.clone(), *n))
            .collect();
        let aborted: BTreeSet<String> = (1..=MANY)
            .map(|id| item(id, &format!("{}~a", name(id))))
            .collect();
        assert_eq!(
            None,
            pruned.keys().find(|(row, _)| !aborted.contains(row)),
            "only aborted copies are pruned"
        );
        let kept: BTreeMap<_, _> = items
            .into_iter()
            .filter(|(k, _)| !pruned.contains_key(k))
            .collect();
        assert_eq!(kept, got);
        assert_eq!(counts, versions(db, &running, "counts"));
    });
}
