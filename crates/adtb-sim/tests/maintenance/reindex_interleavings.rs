//! A rebuild of an index racing writes and reads of its relation, a second rebuild, a vacuum, a
//! checkpoint and the damage mapper: at each boundary of either side, and at every pair of them
//! under the deep profile. One search stops at the rebuild's phases, one at the retirement of
//! the pages of the old tree, and one stops a read where its descent read a child's page id while
//! the retirement frees that page and an allocation takes it.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::{Arc, Mutex, PoisonError};

use adtb::runtime::Threads;
use adtb::testing::{
    Checkpoint, Damage, Knobs, Pid, Plant, Quarantine, Reindex, Retire, Step, StepFn, Vacuum,
};
use adtb::{CallOptions, Database, Error, FatalReason, OpenOptions};
use adtb_sim::{Config, Sim, SimRuntime};
use adtb_simkit::api;
use adtb_simkit::elle::{self, Elems, Kind, Reply, Req};
use adtb_simkit::racers::{Called, Txn};
use adtb_simkit::steps::{self, Boundaries, Pair, Reached, Second, Stepping};
use adtb_testkit::isolation::{self as iso, Outcome};
use adtb_testkit::process;
use adtb_testkit::profile::{self, sweep};

const SEEDS: Range<u64> = 0..1;

/// The list key the setup appends to, and the racers append to and read.
const KEY: i64 = 1;
/// The elements the setup appends to [`KEY`].
const FIRST: [i64; 3] = [10, 11, 12];
/// The element a racing append appends.
const RACED: i64 = 20;
/// The setup of the retirement searches appends one element to each of [`BULK`] lists from key
/// [`FILLED`], enough that the index takes several leaves, its last leaf holding none of [`KEY`].
const FILLED: i64 = 100;
const BULK: i64 = 300;

const RELATION: &str = "elle::elems";
const INDEX: &str = "unique (k, pos)";
const REINDEX: &str = "reindex elems index (k, pos)";

/// A read of the element of the last list the setup of the retirement searches fills.
const POINT: &str = "from x in elems where x.k == 399 and x.pos == 0 select x.e";

/// The boundaries of the retirement of the old tree's pages.
const RETIRE: [Step; 4] = [
    Step::Retire(Retire::Held),
    Step::Retire(Retire::Held),
    Step::Retire(Retire::Deleted),
    Step::Retire(Retire::Freed),
];
/// The lists from key [`ALLOCATED`] an allocation of [`Alloc::Elems`] appends a second element
/// to, enough to split the first leaf of the new tree, which its build filled to 90%.
const ALLOCATED: i64 = 110;
const ALLOCATIONS: i64 = 40;
/// The cells an allocation of [`Alloc::Cells`] inserts, each padded to a kilobyte.
const CELLS: i64 = 16;

/// The phases a rebuild begins.
const PHASES: [Reindex; 5] = [
    Reindex::Scan,
    Reindex::Build,
    Reindex::CatchUp,
    Reindex::Swap,
    Reindex::Reclaim,
];

/// What races the rebuild.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Racer {
    /// A transaction appending [`RACED`] to the list of [`KEY`] and committing.
    Append,
    /// A transaction reading the list of [`KEY`] and committing.
    Read,
    /// A second rebuild of the index.
    Reindex,
    /// A vacuum of the relation.
    Vacuum,
    Checkpoint,
    /// A transaction reading [`POINT`], through the damaged leaf while the old tree stands, and
    /// committing.
    Mapper,
    /// A transaction reading [`POINT`] and committing, stopped where a descent read a child's page
    /// id, while the rebuild frees the old tree's pages and the allocation goes on to take them.
    Descent(Alloc),
}

/// What allocates pages once the rebuild has freed one of the old tree's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Alloc {
    /// An append to each of [`ALLOCATIONS`] lists, splitting a leaf of the index's new tree.
    Elems,
    /// An insert of [`CELLS`] cells into another relation.
    Cells,
}

/// Which side an interleaving stops first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum First {
    Rebuild,
    Racer,
}

/// A search: the boundaries of the rebuild it stops at, and the damage it plants first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Search {
    /// The phases of a rebuild of the sound index.
    Phases,
    /// The retirement of the old tree's pages, its last leaf failing its checksum.
    Retire,
    /// The retirement of the old tree's pages, its first leaf failing its checksum, so that the
    /// first page an allocation takes is that leaf and the next the last leaf.
    Stale,
}

impl Search {
    fn counts(self) -> fn(Step) -> bool {
        match self {
            Search::Phases => of_a_rebuild,
            Search::Retire | Search::Stale => of_a_retirement,
        }
    }
}

fn of_a_statement(s: Step) -> bool {
    matches!(s.function(), StepFn::Read | StepFn::Write)
}

fn of_a_descent(s: Step) -> bool {
    s.function() == StepFn::Descend
}

fn of_a_rebuild(s: Step) -> bool {
    s.function() == StepFn::Reindex
}

fn of_a_retirement(s: Step) -> bool {
    s.function() == StepFn::Retire
}

fn of_a_checkpoint(s: Step) -> bool {
    s.function() == StepFn::Checkpoint
}

fn of_a_vacuum(s: Step) -> bool {
    s.function() == StepFn::Vacuum
}

fn options() -> OpenOptions {
    OpenOptions {
        testing: Knobs::without_background_vacuum(),
        ..elle::options()
    }
}

type Files = BTreeMap<String, Vec<u8>>;

static FILES: [Mutex<Option<Files>>; 2] = [const { Mutex::new(None) }; 2];

/// Returns a simulation seeded `seed` over the workload's closed database holding [`FIRST`] in
/// the list of [`KEY`], and for the retirement searches [`BULK`] elements in that of [`FILLED`].
fn built(seed: u64, search: Search) -> Sim {
    let filled = search != Search::Phases;
    let mut files = FILES[usize::from(filled)]
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let files = files.get_or_insert_with(|| {
        let mut sim = Sim::with_files(0, Config::default(), elle::created());
        process::run(&mut sim, |rt| {
            let e = Elems::open(&rt, &options()).expect("the workload opens");
            let mut tx = e.begin(Kind::ReadWrite).expect("a transaction begins");
            for elem in FIRST {
                e.append(&mut tx, KEY, elem).expect("an append");
            }
            tx.commit(None).expect("the setup commits");
            if filled {
                let bulk: String = (0..BULK)
                    .map(|k| format!("insert elems {{ k: {}, pos: 0, e: {k} }}\n", FILLED + k))
                    .collect();
                api::results(e.database(), &bulk);
            }
            drop(e);
        });
        sim.files()
    });
    Sim::with_files(seed, Config::default(), files.clone())
}

/// A call's reply: the rows it returns, printed.
type Rows = Result<Vec<String>, Error>;

/// A step function of a case: a transaction, or a call on a thread of its own.
enum Side {
    Txn(Txn),
    Call(Called<Rows>),
}

impl Stepping for Side {
    fn boundaries(&self) -> Boundaries {
        match self {
            Side::Txn(t) => t.boundaries(),
            Side::Call(c) => c.boundaries(),
        }
    }

    fn start(&mut self, rt: &SimRuntime) {
        match self {
            Side::Txn(t) => t.start(rt),
            Side::Call(c) => c.start(rt),
        }
    }

    fn ended(&mut self) -> bool {
        match self {
            Side::Txn(t) => t.ended(),
            Side::Call(c) => c.ended(),
        }
    }
}

/// Returns a rebuild of the index by [`REINDEX`], whose boundaries are the steps `counts`
/// accepts on its job's thread, replying once its job has ended.
fn rebuild(rt: &SimRuntime, db: &Database<SimRuntime>, counts: fn(Step) -> bool) -> Called<Rows> {
    let (db2, rt2) = (db.clone(), rt.clone());
    Called::spawn(rt, ("reindex", counts), move || {
        let before = db2.testing().reindex_threads();
        let got = db2.query_text(REINDEX, &[], &CallOptions::default());
        for job in db2.testing().reindex_threads() {
            if !before.contains(&job) {
                rt2.join(job);
            }
        }
        got.map(|_| Vec::new())
    })
    .on_its_job()
}

/// Returns what `vacuum elems` frees, each row past its constructor.
fn vacuumed(db: &Database<SimRuntime>) -> Rows {
    let mut rows = db.query_text("vacuum elems", &[], &CallOptions::default())?;
    let printed = api::printed(rows.remove(0)).into_iter();
    Ok(printed
        .map(|r| {
            r.split_once("freed: ")
                .expect("a vacuum's row")
                .1
                .to_owned()
        })
        .collect())
}

/// What one case left.
#[derive(Debug)]
struct Ran {
    rebuild: Reached,
    racer: Reached,
    rebuilt: Rows,
    raced: Vec<Reply>,
    /// What the racer replied, when it was a call.
    called: Option<Rows>,
    /// How many pages of the old tree were free when the rebuild stopped first.
    freed_at_stop: Option<usize>,
    /// Once an allocation ran, whether it took the old tree's last leaf, which [`POINT`] descends
    /// to, and whether that page is in the index's tree at the end.
    reused: Option<(bool, bool)>,
    /// The entries the index holds, and those a tree built fresh from every version of its
    /// relation holds.
    entries: [BTreeSet<String>; 2],
    marks: BTreeSet<Quarantine>,
    verified: Vec<String>,
    list: Vec<i64>,
    anomalies: Vec<String>,
}

/// Returns the entries of the index, and those a tree built fresh from every version of its
/// relation holds, each as its key and TID printed.
fn entries(db: &Database<SimRuntime>) -> [BTreeSet<String>; 2] {
    let t = db.testing();
    let index = t.index(RELATION, INDEX).expect("the index");
    let held = t.all_entries(index).expect("its entries");
    let held = held
        .into_iter()
        .map(|e| format!("{:?} {:?}", e.key, e.tid))
        .collect();
    let heap = t.heap(RELATION, "Elem").expect("the heap");
    let versions = t.versions(heap, None).expect("its versions");
    let fresh = versions
        .into_iter()
        .map(|v| format!("{:?} {:?}", [&v.row[1], &v.row[2]], v.tid))
        .collect();
    [held, fresh]
}

/// Returns the pages of the index's tree from its root, and refuse the leaf `search` names as
/// failing its checksum.
fn planted(db: &Database<SimRuntime>, search: Search) -> Vec<Pid> {
    let t = db.testing();
    let index = t.index(RELATION, INDEX).expect("the index");
    let pages = t.pages(index).expect("its pages");
    let leaf = match search {
        Search::Phases => None,
        Search::Retire => pages.last(),
        Search::Stale => pages.get(1),
    };
    if let Some(&page) = leaf {
        let damage = Damage::Checksum;
        t.plant(Plant::Refuse { page, damage })
            .expect("the leaf is refused");
    }
    pages
}

/// Run `alloc`, and return the pages of `old` free before it.
fn allocate(db: &Database<SimRuntime>, alloc: Alloc, old: &[Pid]) -> Vec<Pid> {
    let free = old
        .iter()
        .copied()
        .filter(|p| db.testing().is_free(*p).expect("the bitmap reads"))
        .collect();
    let text: String = match alloc {
        Alloc::Elems => (ALLOCATED..ALLOCATED + ALLOCATIONS)
            .map(|k| format!("insert elems {{ k: {k}, pos: 1, e: {k} }}\n"))
            .collect(),
        Alloc::Cells => {
            let pad = "x".repeat(1024);
            (0..CELLS)
                .map(|n| format!("insert cells.Even {{ id: {n}, n: {n}, m: 0, pad: \"{pad}\" }}\n"))
                .collect()
        }
    };
    api::results(db, &text);
    free
}

/// Run `search` with the rebuild stopped at its `i`th boundary and `racer` at its `j`th, the side
/// `first` names stopped first.
fn run(seed: u64, (search, racer, first): (Search, Racer, First), (i, j): (usize, usize)) -> Ran {
    process::run(&mut built(seed, search), |rt| {
        let e = Arc::new(Elems::open(&rt, &options()).expect("the workload opens"));
        let db = e.database();
        let old = planted(db, search);
        let mut rebuilding = rebuild(&rt, db, search.counts());
        let txn_on = |boundaries, reqs| {
            Side::Txn(Txn::start(&rt, &e, (1, Kind::ReadWrite), boundaries, reqs))
        };
        let txn = |reqs| txn_on(of_a_statement as fn(Step) -> bool, reqs);
        let db2 = db.clone();
        let mut side = match racer {
            Racer::Append => txn(vec![Req::Append(KEY, RACED), Req::Commit]),
            Racer::Read => txn(vec![Req::Read(KEY), Req::Commit]),
            Racer::Mapper => txn(vec![Req::Text(POINT.to_owned()), Req::Commit]),
            Racer::Descent(_) => {
                txn_on(of_a_descent, vec![Req::Text(POINT.to_owned()), Req::Commit])
            }
            Racer::Reindex => Side::Call(rebuild(&rt, db, of_a_rebuild)),
            Racer::Vacuum => {
                let on = ("vacuum", of_a_vacuum as fn(Step) -> bool);
                Side::Call(Called::spawn(&rt, on, move || vacuumed(&db2)))
            }
            Racer::Checkpoint => {
                let on = ("checkpoint", of_a_checkpoint as fn(Step) -> bool);
                Side::Call(Called::spawn(&rt, on, move || {
                    db2.testing().checkpoint();
                    Ok(Vec::new())
                }))
            }
        };
        let mut freed_at_stop = None;
        let alloc = match racer {
            Racer::Descent(alloc) => Some(alloc),
            _ => None,
        };
        let mut was_free = None;
        let freed = Step::Retire(Retire::Freed);
        let [rebuild_reached, racer_reached] = match first {
            First::Rebuild => {
                let free = |p: &&Pid| db.testing().is_free(**p).expect("the bitmap reads");
                let probe = || {
                    freed_at_stop = Some(old.iter().filter(free).count());
                    if let Some(alloc) = alloc
                        && RETIRE.get(i - 1) == Some(&freed)
                    {
                        was_free = Some(allocate(db, alloc, &old));
                    }
                };
                let sides = (&mut rebuilding, &mut side);
                steps::interleave(&rt, db, sides, (i, j), (Box::new(probe), Box::new(|_| {})))
            }
            First::Racer => {
                let sides = (&mut side, &mut rebuilding);
                let then = |at: Second| {
                    if let Some(alloc) = alloc
                        && matches!(at, Second::Whole) | matches!(at, Second::At(s) if s == freed)
                    {
                        was_free = Some(allocate(db, alloc, &old));
                    }
                };
                let between = (Box::new(|| {}) as Box<dyn FnOnce()>, Box::new(then) as _);
                let [r, b] = steps::interleave(&rt, db, sides, (j, i), between);
                [b, r]
            }
        };
        let reused = was_free.map(|was: Vec<Pid>| {
            let t = db.testing();
            let leaf = *old.last().expect("a leaf");
            let taken = was.contains(&leaf) && !t.is_free(leaf).expect("the bitmap reads");
            let index = t.index(RELATION, INDEX).expect("the index");
            (taken, t.pages(index).expect("its pages").contains(&leaf))
        });
        let ops = FIRST
            .iter()
            .map(|&elem| iso::Op::Append { key: KEY, elem })
            .collect();
        let mut history = vec![iso::Txn::new(0, Outcome::Committed, ops)];
        let (mut raced, mut called) = (Vec::new(), None);
        match &mut side {
            Side::Txn(t) => {
                history.push(t.history());
                raced = t.replies().to_vec();
            }
            Side::Call(c) => called = Some(c.take()),
        }
        let list = e.lists(KEY..KEY + 1).remove(0);
        let last = vec![iso::Op::Read {
            key: KEY,
            list: list.clone(),
        }];
        history.push(iso::Txn::new(2, Outcome::Committed, last));
        let anomalies = match iso::check(&history) {
            Ok(found) => found.iter().map(|a| format!("{a:?}")).collect(),
            Err(malformed) => vec![format!("{malformed:?}")],
        };
        Ran {
            rebuild: rebuild_reached,
            racer: racer_reached,
            rebuilt: rebuilding.take(),
            raced,
            called,
            freed_at_stop,
            reused,
            entries: entries(db),
            marks: db.testing().probe().marks(),
            verified: e.verify(),
            list,
            anomalies,
        }
    })
}

fn quarantined() -> Reply {
    Err(Error::Fatal {
        reason: FatalReason::Quarantined {
            what: format!("{RELATION} {INDEX}"),
        },
    })
}

/// Returns each way `r`, a run of `racer` in `search`, breaks what a rebuild raced by a step
/// function leaves. `refused` says whether a statement through the index meets the old tree's
/// damaged leaf, and `stop` is the rebuild's boundary when it stopped first.
///
/// - the rebuild replies as it does alone, and so does the racer, but for a statement that meets
///   the damaged leaf, which fails as quarantined;
/// - when the rebuild stopped first, the pages of the old tree free at its stop are the damaged
///   leaf once the sound pages are marked deleted, and one more for each free it reached;
/// - the index holds an entry for every version of its relation and no other, as one built fresh
///   does;
/// - the list holds the setup's elements and the racer's append;
/// - nothing is marked and `verify` finds nothing;
/// - the history is serializable.
fn judged(
    racer: Racer,
    (refused, stop): (bool, Option<usize>),
    what: &str,
    r: &Ran,
) -> Vec<String> {
    let mut broken = Vec::new();
    let mut expect = |about: &str, ok: bool, got: &dyn std::fmt::Debug| {
        if !ok {
            broken.push(format!("{what}: {about}: {got:?}"));
        }
    };
    expect("the rebuild's reply", r.rebuilt == Ok(vec![]), &r.rebuilt);
    let done = vec![Ok(vec![]), Ok(vec![])];
    let mut list = FIRST.to_vec();
    match racer {
        Racer::Append | Racer::Mapper if refused => {
            expect(
                "the statement's replies",
                r.raced == [quarantined()],
                &r.raced,
            );
        }
        Racer::Append => {
            expect("the append's replies", r.raced == done, &r.raced);
            list.push(RACED);
        }
        Racer::Read => {
            let read = vec![Ok(FIRST.to_vec()), Ok(vec![])];
            expect("the read's replies", r.raced == read, &r.raced);
        }
        Racer::Mapper => {
            let read = vec![Ok(vec![BULK - 1]), Ok(vec![])];
            expect("the point read's replies", r.raced == read, &r.raced);
        }
        Racer::Descent(alloc) => {
            let read = vec![Ok(vec![BULK - 1]), Ok(vec![])];
            expect("the point read's replies", r.raced == read, &r.raced);
            if r.reused.is_some() {
                let taken = Some((true, alloc == Alloc::Elems));
                expect(
                    "the old leaf the allocation took",
                    r.reused == taken,
                    &r.reused,
                );
            }
        }
        Racer::Reindex | Racer::Checkpoint => {
            expect("the call's reply", r.called == Some(Ok(vec![])), &r.called);
        }
        Racer::Vacuum => {
            let nothing = "\"0 pages, 0 out-of-line chunks of the relation\", removed: 0 }";
            let got = Some(Ok(vec![nothing.to_owned()]));
            expect("the vacuum's reply", r.called == got, &r.called);
        }
    }
    if let Some(i) = stop {
        let reached = &r.rebuild.steps()[..i];
        let scrapped = usize::from(reached.contains(&Step::Retire(Retire::Deleted)));
        let frees = scrapped
            + reached
                .iter()
                .filter(|s| **s == Step::Retire(Retire::Freed))
                .count();
        expect(
            "the pages of the old tree free at the stop",
            r.freed_at_stop == Some(frees),
            &r.freed_at_stop,
        );
    }
    let [held, fresh] = &r.entries;
    expect("the index's entries", held == fresh, &(held, fresh));
    expect("the list", r.list == list, &r.list);
    expect("the marks", r.marks.is_empty(), &r.marks);
    expect("verify", r.verified.is_empty(), &r.verified);
    expect("the history", r.anomalies.is_empty(), &r.anomalies);
    broken
}

/// How the second side of a case runs once the first stops at one of its boundaries: the
/// boundaries it reaches, and where it is when the first goes on.
type Runs = (Vec<Step>, Second);

/// Returns the pairs a search of the first side's `stops` runs, `runs` giving how the second
/// runs at each, counted from 1: each boundary of the second it reaches and then its end under
/// the deep profile, else its end alone.
fn declared(stops: &[Step], runs: &dyn Fn(usize) -> Runs) -> Vec<Pair> {
    let mut pairs = Vec::new();
    for (i, &stop) in stops.iter().enumerate() {
        let (reached, end) = runs(i + 1);
        if profile::every_case() {
            pairs.extend(reached.into_iter().map(|s| (stop, Second::At(s))));
        }
        pairs.push((stop, end));
    }
    pairs
}

/// Returns the boundaries `racer` reaches run alone in `search`. A read through the damaged leaf
/// meets the damage before any.
fn alone(search: Search, racer: Racer) -> Vec<Step> {
    let read = Step::VersionRead;
    match racer {
        Racer::Mapper if search != Search::Phases => vec![],
        Racer::Append => vec![read, read, read, Step::WriteInserted],
        Racer::Read => vec![read; 3],
        Racer::Mapper => vec![read],
        Racer::Descent(_) => vec![Step::Descended],
        Racer::Reindex => PHASES.map(Step::Reindex).to_vec(),
        Racer::Vacuum => [Vacuum::Scan, Vacuum::Prune].map(Step::Vacuum).to_vec(),
        Racer::Checkpoint => [Checkpoint::Folded, Checkpoint::Forced]
            .map(Step::Checkpoint)
            .to_vec(),
    }
}

/// Search `racer` against the rebuild in `search` with the rebuild stopped first, and under the
/// deep profile each side stopped first in turn, judging every case, and check that each side stopped first stopped at its declared boundaries, the
/// rebuild's `stops`, and that the pairs run are those `runs` declares for the side stopped
/// first and its boundary. `refused` says whether a statement run with the rebuild stopped at
/// its `i`th boundary meets the old tree's damaged leaf.
fn race(
    (search, racer): (Search, Racer),
    stops: &[Step],
    runs: &dyn Fn(First, usize) -> Runs,
    refused: &dyn Fn(First, usize) -> bool,
) -> usize {
    let mut broken = Vec::new();
    let firsts: &[First] = match racer {
        _ if profile::every_case() => &[First::Rebuild, First::Racer],
        Racer::Descent(_) => &[First::Racer],
        _ => &[First::Rebuild],
    };
    let mut stale = 0;
    for seed in sweep(SEEDS) {
        for &first in firsts {
            let what = format!("{search:?} {racer:?} {first:?} seed {seed}");
            let mut one = |(a, b): (usize, usize)| {
                let pair = match first {
                    First::Rebuild => (a, b),
                    First::Racer => (b, a),
                };
                let r = run(seed, (search, racer, first), pair);
                let stopped = first == First::Rebuild && r.rebuild.stopped();
                let stop = stopped.then_some(a).filter(|_| search != Search::Phases);
                let at = format!("{what} at {pair:?}");
                let refused = match first {
                    First::Rebuild => stopped && refused(first, a),
                    First::Racer => refused(first, b),
                };
                broken.extend(judged(racer, (refused, stop), &at, &r));
                if first == First::Racer && r.racer.stopped() && r.reused.is_some() {
                    stale += 1;
                }
                match first {
                    First::Rebuild => [r.rebuild, r.racer],
                    First::Racer => [r.racer, r.rebuild],
                }
            };
            let (reached, pairs) = if profile::every_case() {
                steps::every_pair(&mut one)
            } else {
                steps::every_boundary(&mut one)
            };
            let stops = match first {
                First::Rebuild => stops.to_vec(),
                First::Racer => alone(search, racer),
            };
            assert_eq!(stops, reached, "{what}: the first side's boundaries");
            let expected = declared(&stops, &|i| runs(first, i));
            assert_eq!(expected, pairs, "{what}: the pairs run");
        }
    }
    assert_eq!(Vec::<String>::new(), broken);
    stale
}

/// The phases of a rebuild of `elems (k, pos)`, raced at each by an append, a read, a second
/// rebuild, a vacuum and a checkpoint, leave the index equal to one built fresh, every racer's
/// reply as it is alone, nothing marked and a serializable history. A second rebuild and a vacuum
/// wait for the vacuum lock the stopped side holds, whichever is stopped first.
#[test]
fn a_rebuild_raced_at_each_phase_leaves_the_index_one_built_fresh_would() {
    let phases = PHASES.map(Step::Reindex);
    for racer in [
        Racer::Append,
        Racer::Read,
        Racer::Reindex,
        Racer::Vacuum,
        Racer::Checkpoint,
    ] {
        let waits = matches!(racer, Racer::Reindex | Racer::Vacuum);
        let runs = |first, _| -> Runs {
            match (first, waits) {
                (_, true) => (vec![], Second::Blocked),
                (First::Rebuild, false) => (alone(Search::Phases, racer), Second::Whole),
                (First::Racer, false) => (phases.to_vec(), Second::Whole),
            }
        };
        race((Search::Phases, racer), &phases, &runs, &|_, _| false);
    }
}

/// The retirement of the pages of an old tree with a damaged leaf, raced at each of its
/// boundaries by a read through that leaf, an append and a read: a sound page of the old tree is
/// free only once the rebuild freed it, a read through the leaf before the rebuild met it fails
/// as quarantined, a write while the rebuild's mark stands before the swap fails as quarantined,
/// every other statement runs as alone, and the swap leaves nothing marked. A write waits for
/// the index's write lock the sweep of the old tree holds.
#[test]
fn a_retired_tree_raced_at_each_boundary_frees_its_pages_after_the_swap_and_leaves_no_mark() {
    let stops = RETIRE;
    for racer in [Racer::Mapper, Racer::Append, Racer::Read] {
        let runs = |first, i| -> Runs {
            match (first, racer, i) {
                (First::Rebuild, Racer::Append, 1) => (vec![], Second::Whole),
                (First::Rebuild, Racer::Append, 2) => {
                    (alone(Search::Retire, racer), Second::Blocked)
                }
                (First::Rebuild, Racer::Mapper, _) => (alone(Search::Phases, racer), Second::Whole),
                (First::Rebuild, _, _) => (alone(Search::Retire, racer), Second::Whole),
                (First::Racer, _, _) => (stops.to_vec(), Second::Whole),
            }
        };
        let refused = |first, i| match (first, racer) {
            (First::Rebuild, Racer::Append) => i == 1,
            (First::Racer, Racer::Mapper) => true,
            _ => false,
        };
        race((Search::Retire, racer), &stops, &runs, &refused);
    }
}

/// A read of a sound leaf of the old tree, stopped at each point its descent read a child's page
/// id from the parent, while the rebuild retires and frees the old tree's pages and an append to
/// the relation or an insert into another takes them, answers as alone, and leaves `verify`
/// finding nothing. The stale page id a stopped read holds names a page of the new tree or of
/// another relation.
#[test]
fn a_read_holding_a_page_id_of_a_retired_tree_freed_and_taken_again_answers_as_alone() {
    for alloc in [Alloc::Elems, Alloc::Cells] {
        let racer = Racer::Descent(alloc);
        let runs = |first, _| -> Runs {
            match first {
                First::Rebuild => (alone(Search::Stale, racer), Second::Whole),
                First::Racer => (RETIRE.to_vec(), Second::Whole),
            }
        };
        let stale = race((Search::Stale, racer), &RETIRE, &runs, &|_, _| false);
        assert_ne!(
            0, stale,
            "{alloc:?}: no read stopped while the allocation took a page"
        );
    }
}
