//! A migration under each kind of mark: damage a read meets marks a constructor relation or an
//! index, or the database is marked whole, then an open applies the migration that drops `tags`.
//! The open is refused with the quarantine exactly when the migration reads a marked constructor
//! relation, and one that applies takes the mark of each index it drops with it.

use std::collections::BTreeSet;

use adtb::testing::{Damage, Plant, Quarantine};
use adtb::{CallOptions, Database, Error, FatalReason, OpenError};
use adtb_sim::{Config, Sim};
use adtb_testkit::process;

use super::world::{self, FILE, Rel};

/// What a case marks before the migration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Marked {
    Database,
    Heap(Rel),
    Index(Rel),
}

const CASES: [Marked; 5] = [
    Marked::Database,
    Marked::Heap(Rel::Tags),
    Marked::Index(Rel::Tags),
    Marked::Heap(Rel::Users),
    Marked::Index(Rel::Users),
];

/// The version of the migration that drops `tags`, which reads its heap to find it empty.
const UNTAG: u64 = 3;

fn quarantined(what: &str) -> Error {
    Error::Fatal {
        reason: FatalReason::Quarantined { what: what.into() },
    }
}

/// Returns the name a client sees of the heap of `rel`.
fn heap_name(rel: Rel) -> String {
    format!("{}.{}", rel.qualified(), rel.ctor())
}

/// Returns a file at the version that adds `tags`, with rows of `users` and a deleted row of
/// `tags`, and `marked` set by damage that a read of it meets, or planted for the database. Returns
/// the mark the file holds for `marked`, `None` when the read set none.
fn marked_file(seed: u64, marked: Marked) -> (Sim, Option<Quarantine>) {
    let mut sim = Sim::new(seed, Config::default());
    let set = process::run(&mut sim, |rt| {
        let db = Database::open_in(rt, FILE, &world::log(1), &world::options()).unwrap();
        let run = |text: &str| db.query_text(text, &[], &CallOptions::default()).map(drop);
        run(r#"insert users { id: users.key(1), name: "a" }"#).unwrap();
        run(r#"insert tags { id: tags.key(1), label: "t" }"#).unwrap();
        run("delete r in tags where key r == tags.key(1)").unwrap();
        let t = db.testing();
        let (object, read) = match marked {
            Marked::Database => {
                t.plant(Plant::Mark(Quarantine::Database)).unwrap();
                (None, None)
            }
            Marked::Heap(rel) => (t.heap(&rel.qualified(), rel.ctor()), Some(rel.read())),
            Marked::Index(rel) => (t.index(&rel.qualified(), "key id"), Some(rel.lookup(1))),
        };
        let mark = match (object, read) {
            (Some(object), Some(read)) => {
                let page = t.pages(object).unwrap()[0];
                t.plant(Plant::Refuse {
                    page,
                    damage: Damage::Checksum,
                })
                .unwrap();
                let named = match marked {
                    Marked::Index(rel) => format!("{} key id", rel.qualified()),
                    _ => heap_name(rel_of(marked)),
                };
                assert_eq!(Err(quarantined(&named)), run(&read), "{marked:?}");
                t.own_mark(object).unwrap()
            }
            _ => Quarantine::Database,
        };
        let set = t.probe().marks().contains(&mark).then_some(mark);
        db.close().unwrap();
        set
    });
    (sim, set)
}

fn rel_of(marked: Marked) -> Rel {
    match marked {
        Marked::Heap(rel) | Marked::Index(rel) => rel,
        Marked::Database => unreachable!("the database's mark names no relation"),
    }
}

/// Returns the refusal the rule states for a migration that drops `tags` under `marked`, `None`
/// when it applies.
fn refusal(marked: Marked) -> Option<Error> {
    match marked {
        Marked::Database => Some(quarantined("the database")),
        Marked::Heap(Rel::Tags) => Some(quarantined(&heap_name(Rel::Tags))),
        Marked::Heap(_) | Marked::Index(_) => None,
    }
}

#[test]
fn a_migration_is_refused_by_the_mark_of_what_it_reads_and_takes_the_mark_of_what_it_drops() {
    let mut reached = BTreeSet::new();
    for (seed, marked) in (1..).zip(CASES) {
        let (mut sim, mark) = marked_file(seed, marked);
        let Some(mark) = mark else {
            continue;
        };
        reached.insert(marked);
        let kept = marked == Marked::Database || rel_of(marked) != Rel::Tags;
        let applied = process::run(&mut sim, |rt| {
            let opened = Database::open_in(rt, FILE, &world::log(2), &world::options());
            match (refusal(marked), opened) {
                (
                    Some(reason),
                    Err(OpenError::MigrationRefused {
                        version,
                        reason: got,
                    }),
                ) => {
                    assert_eq!((UNTAG, reason), (version, got), "{marked:?}");
                    false
                }
                (None, Ok(db)) => {
                    let s = db.schema();
                    let tags = s.relations.iter().any(|r| r.name == Rel::Tags.name());
                    assert!(!tags, "{marked:?}: the migration dropped tags");
                    let marks = db.testing().probe().marks();
                    assert_eq!(kept, marks.contains(&mark), "{marked:?}: {marks:?}");
                    db.close().unwrap();
                    true
                }
                (want, got) => panic!("{marked:?}: expected {want:?}, found {:?}", got.map(drop)),
            }
        });
        if applied {
            process::run(&mut sim, |rt| {
                let db = Database::open_in(rt, FILE, &world::log(2), &world::options()).unwrap();
                let marks = db.testing().probe().marks();
                assert_eq!(
                    kept,
                    marks.contains(&mark),
                    "{marked:?} reopened: {marks:?}"
                );
                db.close().unwrap();
            });
        }
    }
    assert_eq!(BTreeSet::from(CASES), reached, "each mark a case sets");
}
