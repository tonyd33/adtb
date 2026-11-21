//! Damage with a sound checksum on each version a statement wrote, planted at each point between
//! the write and its take-back, or on a row it targets once it began writing, quarantines the
//! relation and ends the transaction.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use adtb::key::Value;
use adtb::testing::{Damage, Edit, Object, Pid, Quarantine, Shape as PageShape, Step, Xid};
use adtb::{Database, Error, FatalReason, Log, TxnCallOptions, TxnOptions};
use adtb_sim::{Config, Sim, SimRuntime};
use adtb_simkit::{api, open};
use adtb_testkit::process::finished;
use adtb_testkit::profile::assert_reached;

use super::pool;

const FILE: &str = "taken.adt";
const APP: &str = r"module app
add module app
add type Cell { id: I64, n: I64, m: I64 } = Even | Odd
add relation cells : Cell key id index (m)
";

/// The value a statement writes into the row it inserts or changes, which no other row holds.
const WRITTEN: i64 = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Insert,
    /// An update of an unindexed field, which stays on its page.
    HotUpdate,
    /// An update of an indexed field.
    ColdUpdate,
    Retag,
    Delete,
    /// A delete of rows 1 and 2, in that order.
    DeleteBoth,
}

const KINDS: [Kind; 6] = [
    Kind::Insert,
    Kind::HotUpdate,
    Kind::ColdUpdate,
    Kind::Retag,
    Kind::Delete,
    Kind::DeleteBoth,
];

impl Kind {
    fn text(self) -> &'static str {
        match self {
            Kind::Insert => "insert cells.Even { id: 5, n: 7, m: 0 }",
            Kind::HotUpdate => "update c in cells.Even where c.id == 1 set Even { c with n: 7 }",
            Kind::ColdUpdate => "update c in cells.Even where c.id == 1 set Even { c with m: 7 }",
            Kind::Retag => "update c in cells.Even where c.id == 1 set Odd { c with n: 7 }",
            Kind::Delete => "delete c in cells where c.id == 1",
            Kind::DeleteBoth => "delete c in cells",
        }
    }

    /// Returns the versions the statement writes, in the order its take-back reaches them.
    fn versions(self) -> &'static [Written] {
        match self {
            Kind::Insert => &[Written::New],
            Kind::HotUpdate | Kind::ColdUpdate | Kind::Retag => &[Written::New, Written::Old],
            Kind::Delete | Kind::DeleteBoth => &[Written::Old],
        }
    }

    /// Returns whether the write supersedes a version, checking the SIREAD locks on it after.
    fn supersedes(self) -> bool {
        self != Kind::Insert
    }
}

/// A version the statement wrote.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Written {
    /// The version it inserted, whose take-back deletes it.
    New,
    /// The version it superseded, whose take-back clears its deleter.
    Old,
    /// The version of row 2, which a statement of [`Kind::DeleteBoth`] has yet to target.
    Target,
}

impl Written {
    /// Returns the constructor whose heap holds this version of a statement of `kind`.
    fn ctor(self, kind: Kind) -> &'static str {
        match (self, kind) {
            (Written::New, Kind::Retag) => "Odd",
            _ => "Even",
        }
    }

    /// Returns whether `row`, the row of a version of this version's heap, is this version's.
    fn is(self, row: &[Value]) -> bool {
        let written = row.contains(&Value::I64(WRITTEN));
        match self {
            Written::New => written,
            Written::Old => !written && row.contains(&Value::I64(1)),
            Written::Target => row.contains(&Value::I64(2)),
        }
    }
}

/// Where between a write and its take-back the damage lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Point {
    /// The heap write changed the version's page.
    Wrote,
    /// The write checked the SIREAD locks on the version it superseded.
    Checked,
    /// The take-back is to undo its change of this number, counted from 1 in the order it
    /// undoes them.
    TakeBack(usize),
}

impl Point {
    /// Returns whether `step` is this point, the number of take-back steps before it `taken`.
    fn at(self, step: Step, taken: usize) -> bool {
        match (self, step) {
            (Point::Wrote, Step::WriteInserted | Step::WriteChanged(_))
            | (Point::Checked, Step::WriteChecked) => true,
            (Point::TakeBack(n), Step::TakeBack) => n == taken + 1,
            _ => false,
        }
    }
}

/// How the damage leaves the version's page, its checksum sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    /// The version's slot is unused.
    Absent,
    /// The version names a committed transaction in the place of the statement's.
    Foreign,
}

const SHAPES: [Shape; 2] = [Shape::Absent, Shape::Foreign];

#[derive(Clone, Copy, Debug)]
struct Case {
    kind: Kind,
    written: Written,
    point: Point,
    shape: Shape,
}

/// Returns every case: each shape on each version of each kind at each point between its write
/// and its own take-back, and row 2 given a committed deleter once a delete of both rows checked
/// its write of row 1.
fn cases() -> Vec<Case> {
    let mut out = vec![Case {
        kind: Kind::DeleteBoth,
        written: Written::Target,
        point: Point::Checked,
        shape: Shape::Foreign,
    }];
    for kind in KINDS.into_iter().filter(|k| *k != Kind::DeleteBoth) {
        for (i, &written) in kind.versions().iter().enumerate() {
            let mut points = vec![Point::Wrote];
            if kind.supersedes() {
                points.push(Point::Checked);
            }
            points.extend((1..=i + 1).map(Point::TakeBack));
            for point in points {
                for shape in SHAPES {
                    out.push(Case {
                        kind,
                        written,
                        point,
                        shape,
                    });
                }
            }
        }
    }
    out
}

/// What a case's plant reached: the version it damaged, by its heap's constructor and mark and
/// its page.
type Planted = Option<(&'static str, Quarantine, Pid)>;

/// What a call met by the damage left.
#[derive(Debug, PartialEq)]
struct Ended {
    reply: Result<(), Error>,
    later: Result<(), Error>,
    commit: Result<(), Error>,
    marks: BTreeSet<Quarantine>,
    refusals: BTreeMap<Pid, Damage>,
}

/// Plant `case.shape` on `case.written` as `case.point` is reached in `v`'s heap.
fn plant(db: &Database<SimRuntime>, case: Case, committed: Xid) -> Planted {
    let ctor = case.written.ctor(case.kind);
    let heap = db.testing().heap("app::cells", ctor).expect("the heap");
    let versions = db.testing().versions(heap, None).expect("its versions");
    let v = versions
        .iter()
        .find(|v| case.written.is(&v.row))
        .expect("the version the statement wrote");
    let pid = Pid(v.tid.page());
    let slot = usize::from(v.tid.slot());
    let edit = match (case.shape, case.written) {
        (Shape::Absent, _) => Edit::Vacate(slot),
        (Shape::Foreign, Written::New) => Edit::Inserter {
            slot,
            by: committed,
        },
        (Shape::Foreign, Written::Old | Written::Target) => Edit::Deleter {
            slot,
            by: committed,
        },
    };
    db.testing()
        .scribble(pid, edit)
        .expect("the version is edited");
    let mark = db.testing().own_mark(heap).expect("its mark");
    Some((ctor, mark, pid))
}

/// Returns the transaction that inserted the rows of `heap`.
fn committed(db: &Database<SimRuntime>, heap: Object) -> Xid {
    let versions = db.testing().versions(heap, None).expect("its versions");
    versions[0].header.writers.inserter()
}

/// Run `case`: the statement of its kind and then a failing one, in one call of a transaction,
/// with the damage planted at its point. Returns what the call left and what the plant reached.
fn run(case: Case) -> (Ended, Planted) {
    let mut sim = Sim::new(1, Config::default());
    let mut out = None;
    finished(&mut sim, |rt| {
        let db = open::opened(&rt, FILE, &Log::new([APP]), &open::options(2));
        api::results(&db, "insert cells.Even { id: 1, n: 0, m: 0 }");
        api::results(&db, "insert cells.Even { id: 2, n: 0, m: 0 }");
        let even = db.testing().heap("app::cells", "Even").expect("the heap");
        let committed = committed(&db, even);
        let planted: Arc<Mutex<(usize, Planted)>> = Arc::default();
        {
            let (db, planted) = (db.clone(), planted.clone());
            db.clone().testing().on_step(move |step| {
                let mut p = planted.lock().unwrap();
                if p.1.is_none() && case.point.at(step, p.0) {
                    p.1 = plant(&db, case, committed);
                }
                if step == Step::TakeBack {
                    p.0 += 1;
                }
            });
        }
        let opts = TxnCallOptions::default();
        let mut txn = db.begin(&TxnOptions::default()).expect("a transaction");
        let call = format!("{}\nfail \"no\"", case.kind.text());
        let reply = txn.query_text(&call, &[], &opts).map(drop);
        let later = txn.query_text("1", &[], &opts).map(drop);
        let commit = txn.commit(None).map(drop);
        db.testing().on_step(|_| {});
        let probe = db.testing().probe();
        let ended = Ended {
            reply,
            later,
            commit,
            marks: probe.marks(),
            refusals: probe.refusals(),
        };
        let planted = planted.lock().unwrap().1;
        db.close().expect("the database closes");
        out = Some((ended, planted));
    });
    out.expect("the case ran")
}

/// Returns what the call leaves when it meets the damage on page `pid` of the heap of
/// constructor `ctor`, whose mark is `mark`: the relation quarantined, and the page refused as
/// out of frame.
fn ended_by_damage(ctor: &str, mark: Quarantine, pid: Pid) -> Ended {
    let ended = Err(Error::Fatal {
        reason: FatalReason::Quarantined {
            what: format!("app::cells.{ctor}"),
        },
    });
    Ended {
        reply: ended.clone(),
        later: ended.clone(),
        commit: ended,
        marks: BTreeSet::from([mark]),
        refusals: BTreeMap::from([(pid, Damage::Shape(PageShape::Frame))]),
    }
}

#[test]
fn damage_to_a_version_a_statement_wrote_or_targets_quarantines_its_relation() {
    let cases = cases();
    let ran = pool::map(&cases, |c| format!("{c:?}"), |c| run(*c));
    let mut reached = 0;
    for (case, (ended, planted)) in cases.iter().zip(ran) {
        let Some((ctor, mark, pid)) = planted else {
            panic!("{case:?}: the plant is never reached");
        };
        reached += 1;
        assert_eq!(ended_by_damage(ctor, mark, pid), ended, "{case:?}");
    }
    assert_reached("a plant on a written or targeted version", reached, 53);
}
