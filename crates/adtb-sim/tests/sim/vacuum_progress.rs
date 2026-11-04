//! `sys::vacuums` while `vacuum r` runs, over relations of one to three constructors, up to two
//! secondary indexes, with and without history kept.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use adtb::runtime::{Runtime, Threads};
use adtb::{Database, Log, OpenOptions, Value};
use adtb_sim::SimRuntime;
use adtb_simkit::{api, common, seeds};

const WATCH: &str = "from v in sys::vacuums \
    select { i: v.id, r: v.relation, c: v.ctor, p: v.phase, d: v.done, n: v.dead_versions }";

const ROWS: usize = 100;

/// The simulated time between a watcher's polls.
const POLL: Duration = Duration::from_micros(100);

/// A pool small enough that a vacuum reads its pages from the file.
const POOL: u64 = 256 * 1024;

#[derive(Clone, Copy, Debug)]
struct Shape {
    ctors: usize,
    indexes: usize,
    history: bool,
}

impl Shape {
    fn of(seed: u64) -> Shape {
        Shape {
            ctors: 1 + (seed % 3) as usize,
            indexes: (seed / 2 % 3) as usize,
            history: seed % 2 == 1,
        }
    }

    fn schema(self) -> String {
        let ctors: Vec<String> = (0..self.ctors)
            .map(|c| format!("C{c} {{ f{c}: I64 }}"))
            .collect();
        let mut text = format!(
            "module app\nadd module app\n\
             add type Item {{ id: Key items, n: I64, m: I64 }}\n  = {}\n\
             add relation items : Item key id generate sequence\n",
            ctors.join("\n  | ")
        );
        for field in ["n", "m"].iter().take(self.indexes) {
            text.push_str(&format!("  index ({field})\n"));
        }
        text
    }
}

/// A row of `sys::vacuums`: id, relation, constructor, phase, done, dead versions.
type Row = (i64, i64, i64, String, f64, i64);

fn row(v: &Value) -> Row {
    let Value::Record(fs) = v else {
        panic!("a row of sys::vacuums is a record: {v:?}")
    };
    match fs.as_ref() {
        [
            Value::Int(c),
            Value::F64(d),
            Value::Int(i),
            Value::Int(n),
            Value::Text(p),
            Value::Int(r),
        ] => (*i, *r, *c, p.to_string(), d.get(), *n),
        other => panic!("the selected fields of sys::vacuums, by name: {other:?}"),
    }
}

fn rows(db: &Database<SimRuntime>) -> Vec<Row> {
    api::result(db, WATCH)
        .map(|r| row(&r.expect("a row")))
        .collect()
}

/// What a watcher saw across its polls.
#[derive(Default)]
struct Seen {
    polls: usize,
    /// The constructors of each poll that saw rows.
    ctors: BTreeSet<BTreeSet<i64>>,
    relations: BTreeSet<i64>,
    phases: BTreeSet<String>,
    dead: bool,
    done: BTreeMap<i64, f64>,
}

impl Seen {
    fn take(&mut self, got: &[Row], what: &str) {
        if got.is_empty() {
            return;
        }
        self.polls += 1;
        self.ctors.insert(got.iter().map(|r| r.2).collect());
        assert_eq!(
            got.len(),
            got.iter().map(|r| r.2).collect::<BTreeSet<_>>().len(),
            "{what}: one row per constructor relation: {got:?}"
        );
        for (id, rel, _, phase, done, dead) in got {
            assert!((0.0..=1.0).contains(done), "{what}: {got:?}");
            assert!(*dead >= 0, "{what}: {got:?}");
            let before = self.done.insert(*id, *done).unwrap_or(0.0);
            assert!(before <= *done, "{what}: done fell from {before}: {got:?}");
            if phase == "done" {
                assert_eq!((1.0, 0), (*done, *dead), "{what}: a finished row: {got:?}");
            }
            self.relations.insert(*rel);
            self.phases.insert(phase.clone());
            self.dead |= *dead > 0;
        }
    }
}

fn fill(db: &Database<SimRuntime>, shape: Shape) {
    let mut text = String::new();
    for k in 0..ROWS * shape.ctors {
        let c = k % shape.ctors;
        text.push_str(&format!(
            "insert items.C{c} {{ id: items.key({}), n: {k}, m: {}, f{c}: {k} }}\n",
            k + 1,
            k % 7
        ));
    }
    api::results(db, &text);
}

/// Returns the constructor keys a printed row of `vacuum` names.
fn vacuumed(printed: &[String]) -> BTreeSet<i64> {
    printed
        .iter()
        .map(|r| {
            let at = r.find("key(").expect("a vacuum row names its constructor") + 4;
            let end = at + r[at..].find(')').expect("the key closes");
            r[at..end].parse().expect("a constructor key is a number")
        })
        .collect()
}

fn watch(seed: u64) -> Seen {
    let shape = Shape::of(seed);
    let what = format!("seed {seed}, {shape:?}");
    api::once(seed, |rt| {
        let log = Log::new([shape.schema().as_str()]);
        let options = OpenOptions {
            pool_bytes: Some(POOL),
            ..api::options(2)
        };
        let db = Database::open_in(rt.clone(), "progress.adt", &log, &options)
            .expect("the database opens");
        if shape.history {
            api::results(&db, "set setting RetentionWindow = { length: 1h }");
        }
        fill(&db, shape);
        api::results(&db, "delete i in items where value(key i) % 3 != 0");
        let stop = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(Mutex::new(Seen::default()));
        let (watcher, stopping, into, at) = (db.clone(), stop.clone(), seen.clone(), what.clone());
        let clock = rt.clone();
        let t = rt.spawn("watcher", move || {
            let mut io = clock.io();
            while !stopping.load(Ordering::Relaxed) {
                let got = rows(&watcher);
                into.lock().unwrap().take(&got, &at);
                common::sleep(&clock, &mut io, POLL);
            }
        });
        let out = api::printed(api::results(&db, "vacuum items").remove(0));
        stop.store(true, Ordering::Relaxed);
        rt.join(t);
        assert_eq!(Vec::<Row>::new(), rows(&db), "{what}: no row once it ends");
        let mut seen = Arc::try_unwrap(seen)
            .ok()
            .expect("the watcher has ended")
            .into_inner()
            .unwrap();
        let ctors = vacuumed(&out);
        assert_eq!(shape.ctors, ctors.len(), "{what}: {out:?}");
        assert!(
            seen.ctors.iter().all(|c| *c == ctors),
            "{what}: rows name the relation's constructors {ctors:?}: {:?}",
            seen.ctors
        );
        assert!(seen.relations.len() <= 1, "{what}: {:?}", seen.relations);
        seen.ctors.clear();
        seen
    })
}

#[test]
fn a_running_vacuum_shows_one_row_per_constructor_relation() {
    let (mut polls, mut dead, mut late, mut finished) = (0, 0, 0, 0);
    for seed in seeds::sweep(0..6) {
        let seen = watch(seed);
        assert!(seen.polls > 0, "seed {seed}: the watcher saw the vacuum");
        polls += 1;
        dead += usize::from(seen.dead);
        late += usize::from(seen.phases.iter().any(|p| p != "waiting" && p != "scan"));
        finished += usize::from(seen.phases.contains("done"));
    }
    let floor = seeds::share(0..6, 3);
    seeds::assert_reached("a vacuum seen running", polls, floor);
    seeds::assert_reached("dead versions found and not yet removed", dead, floor);
    seeds::assert_reached("a phase past the scan", late, floor);
    seeds::assert_reached(
        "a finished constructor relation beside a running one",
        finished,
        seeds::share(0..6, 2),
    );
}
