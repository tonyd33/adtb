//! The damage mapper, of a failing read and of a commit, racing writes, reads, a checkpoint and
//! a second mapping at each boundary of either side, and at every pair of them under the deep
//! profile.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::Arc;

use adtb::runtime::Threads;
use adtb::testing::{Change, Checkpoint, Damage, Map, Pid, Plant, Quarantine, Step, StepFn};
use adtb::{Error, FatalReason};
use adtb_sim::{Config, Sim, SimRuntime};
use adtb_simkit::api;
use adtb_simkit::elle::{self, Elems, Kind, Reply, Req, Worker};
use adtb_simkit::racers::{Called, Txn};
use adtb_simkit::steps::{self, Boundaries, Pair, Reached, Second, Stepping};
use adtb_testkit::isolation::{self as iso, Outcome};
use adtb_testkit::process;
use adtb_testkit::profile::{self, sweep};

const SEEDS: Range<u64> = 0..1;

/// The list key the racers append to and read.
const KEY: i64 = 1;
/// The element the setup appends.
const FIRST: i64 = 10;
/// The element a racing append appends.
const RACED: i64 = 20;
/// The list key and element a committing mapper appends.
const COMMITTED: (i64, i64) = (2, 30);
/// The list key and element a second committing transaction appends.
const SECOND: (i64, i64) = (3, 40);

/// The boundaries a mapper reaches mapping one record whose holders it finds.
const MAPPED: [Map; 3] = [Map::Begun, Map::Walked, Map::Marked];

/// The step function that maps the damage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mapper {
    /// A read of the damaged counter, which fails and maps the damage.
    Read,
    /// The commit of an append, which maps the damage under the serialization lock.
    Commit,
}

impl Mapper {
    fn boundaries(self) -> Vec<Step> {
        let step = match self {
            Mapper::Read => Step::Map,
            Mapper::Commit => Step::CommitMap,
        };
        MAPPED.map(step).to_vec()
    }
}

/// What races the mapper.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Racer {
    /// A transaction appending to the list of [`KEY`] and committing.
    Append,
    /// A transaction updating an unindexed field of cell 1 and committing.
    Update,
    /// A transaction reading the list of [`KEY`] and committing.
    Read,
    Checkpoint,
    /// A second read of the damaged counter, which maps the damage again.
    Mapper,
    /// A transaction writing the damaged heap and committing.
    Damaged(Write),
    /// A second committing append, which maps under the serialization lock.
    Commit,
}

/// A write of the damaged heap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Write {
    /// An update of the damaged counter.
    Update,
    /// An insert of another counter.
    Insert,
    /// A delete of the damaged counter.
    Delete,
}

impl Write {
    fn text(self) -> &'static str {
        match self {
            Write::Update => "update c in counters where c.id == 1 set Counter { c with n: 1 }",
            Write::Insert => "insert counters { id: 2, n: 0 }",
            Write::Delete => "delete c in counters where c.id == 1",
        }
    }
}

const WRITES: [Write; 3] = [Write::Update, Write::Insert, Write::Delete];

impl Racer {
    /// Returns the boundaries it reaches run alone, `None` for a racer whose run depends on
    /// whether the damage is mapped.
    fn boundaries(self) -> Option<Vec<Step>> {
        Some(match self {
            Racer::Append => vec![Step::VersionRead, Step::WriteInserted],
            Racer::Update => vec![
                Step::VersionRead,
                Step::VersionRead,
                Step::VersionRead,
                Step::WriteChanged(Change::Update { heap_only: true }),
                Step::WriteChecked,
                Step::VersionRead,
            ],
            Racer::Read => vec![Step::VersionRead],
            Racer::Checkpoint => vec![
                Step::Checkpoint(Checkpoint::Folded),
                Step::Checkpoint(Checkpoint::Forced),
            ],
            Racer::Mapper | Racer::Damaged(_) | Racer::Commit => return None,
        })
    }

    /// Returns the boundaries it reaches stopped first, before the mapper runs. A write of the
    /// damaged heap meets the damage before any.
    fn first_boundaries(self) -> Vec<Step> {
        match self {
            Racer::Mapper => Mapper::Read.boundaries(),
            Racer::Commit => Mapper::Commit.boundaries(),
            Racer::Damaged(_) => vec![],
            racer => racer.boundaries().expect("a racer of a fixed run"),
        }
    }

    /// Returns whether it waits for the serialization lock a commit's mapping holds.
    fn serializes(self) -> bool {
        matches!(self, Racer::Read | Racer::Commit)
    }
}

/// Which side an interleaving stops first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum First {
    Mapper,
    Racer,
}

#[derive(Clone, Copy, Debug)]
struct Case {
    mapper: Mapper,
    racer: Racer,
    first: First,
    seed: u64,
}

fn of_a_statement(s: Step) -> bool {
    matches!(s.function(), StepFn::Read | StepFn::Write)
}

fn of_the_mapper(s: Step) -> bool {
    s.function() == StepFn::Map
}

fn of_a_commit(s: Step) -> bool {
    s.function() == StepFn::CommitMap
}

fn of_a_checkpoint(s: Step) -> bool {
    s.function() == StepFn::Checkpoint
}

enum Side {
    Txn(Txn),
    Checkpoint(Called<()>),
}

impl Stepping for Side {
    fn boundaries(&self) -> Boundaries {
        match self {
            Side::Txn(t) => t.boundaries(),
            Side::Checkpoint(c) => c.boundaries(),
        }
    }

    fn start(&mut self, rt: &SimRuntime) {
        match self {
            Side::Txn(t) => t.start(rt),
            Side::Checkpoint(c) => c.start(rt),
        }
    }

    fn ended(&mut self) -> bool {
        match self {
            Side::Txn(t) => t.ended(),
            Side::Checkpoint(c) => c.ended(),
        }
    }
}

/// What one case left.
#[derive(Debug)]
struct Ran {
    mapper: Reached,
    racer: Reached,
    /// The replies of the mapper's transaction.
    mapped: Vec<Reply>,
    /// The replies of a racing transaction, and how it ended.
    raced: Vec<Reply>,
    raced_as: Option<Outcome>,
    /// Whether the damaged heap was marked when the mapper stopped first.
    marked_at_stop: Option<bool>,
    marks: BTreeSet<Quarantine>,
    /// The mark of the damaged heap.
    planted: Quarantine,
    /// The rows `verify` reports over the whole database, printed, when the case ran it.
    verified: Option<Vec<String>>,
    /// The page planted, and how.
    page: (Pid, Damage),
    refusals: BTreeMap<Pid, Damage>,
    anomalies: Vec<String>,
}

/// Commit a counter, a cell and an append of [`FIRST`] to [`KEY`], then refuse the counter's
/// page as failing its checksum. Returns the history's transaction of the setup, the page and
/// its damage, and the mark of the counters' heap.
fn damaged(rt: &SimRuntime, e: &Arc<Elems>) -> (iso::Txn, (Pid, Damage), Quarantine) {
    let db = e.database();
    let setup = Worker::start(rt, e, Kind::ReadWrite);
    let insert = Req::Text("insert counters { id: 1, n: 0 }".to_owned());
    setup.call(rt, insert).expect("a counter inserts");
    let cell = Req::Text("insert cells.Even { id: 1, n: 0, m: 0, pad: \"\" }".to_owned());
    setup.call(rt, cell).expect("a cell inserts");
    setup.call(rt, Req::Append(KEY, FIRST)).expect("an append");
    setup.call(rt, Req::Commit).expect("the setup commits");
    rt.join(setup.thread());
    let append = iso::Op::Append {
        key: KEY,
        elem: FIRST,
    };
    let heap = db
        .testing()
        .heap("elle::counters", "Counter")
        .expect("the counters' heap");
    let versions = db.testing().versions(heap, None).expect("its versions");
    let (page, damage) = (Pid(versions[0].tid.page()), Damage::Checksum);
    db.testing()
        .plant(Plant::Refuse { page, damage })
        .expect("the page is refused");
    let mark = db.testing().own_mark(heap).expect("its mark");
    let setup = iso::Txn::new(0, Outcome::Committed, vec![append]);
    (setup, (page, damage), mark)
}

fn read_counters() -> Req {
    Req::Text("from c in counters select c.n".to_owned())
}

/// Run `case` with the mapper at its `i`th boundary and the racer at its `j`th, the side
/// `case.first` names stopped first.
fn run(case: Case, (i, j): (usize, usize)) -> Ran {
    let mut sim = Sim::with_files(case.seed, Config::default(), elle::created());
    process::run(&mut sim, |rt| {
        let e = Arc::new(Elems::open(&rt, &elle::options()).expect("the workload opens"));
        let db = e.database();
        let (setup, page, planted) = damaged(&rt, &e);
        let mut mapper = match case.mapper {
            Mapper::Read => Txn::start(
                &rt,
                &e,
                (1, Kind::ReadOnly),
                of_the_mapper,
                vec![read_counters()],
            ),
            Mapper::Commit => {
                let (k, v) = COMMITTED;
                let reqs = vec![Req::Append(k, v), Req::Commit];
                Txn::start(&rt, &e, (1, Kind::ReadWrite), of_a_commit, reqs)
            }
        };
        let txn = |on, reqs| Side::Txn(Txn::start(&rt, &e, (2, Kind::ReadWrite), on, reqs));
        let text = |t: &str| Req::Text(t.to_owned());
        let update = "update c in cells.Even where c.id == 1 set Even { c with n: 1 }";
        let mut racer = match case.racer {
            Racer::Append => txn(of_a_statement, vec![Req::Append(KEY, RACED), Req::Commit]),
            Racer::Update => txn(of_a_statement, vec![text(update), Req::Commit]),
            Racer::Read => txn(of_a_statement, vec![Req::Read(KEY), Req::Commit]),
            Racer::Checkpoint => {
                let checkpointing = db.clone();
                let on = ("checkpoint", of_a_checkpoint as fn(Step) -> bool);
                Side::Checkpoint(Called::spawn(&rt, on, move || {
                    checkpointing.testing().checkpoint();
                }))
            }
            Racer::Mapper => txn(of_the_mapper, vec![read_counters()]),
            Racer::Damaged(w) => txn(of_a_statement, vec![text(w.text()), Req::Commit]),
            Racer::Commit => {
                let (k, v) = SECOND;
                txn(of_a_commit, vec![Req::Append(k, v), Req::Commit])
            }
        };
        let mut marked_at_stop = None;
        let [mapper_reached, racer_reached] = match case.first {
            First::Mapper => {
                let probe = || marked_at_stop = Some(db.testing().probe().marked(planted));
                steps::interleave(
                    &rt,
                    db,
                    (&mut mapper, &mut racer),
                    (i, j),
                    (Box::new(probe), Box::new(|_| {})),
                )
            }
            First::Racer => {
                let [r, m] = steps::interleave(
                    &rt,
                    db,
                    (&mut racer, &mut mapper),
                    (j, i),
                    steps::nothing_between(),
                );
                [m, r]
            }
        };
        let mut history = vec![setup];
        if case.mapper == Mapper::Commit {
            history.push(mapper.history());
        }
        let (mut raced, mut raced_as) = (Vec::new(), None);
        if let Side::Txn(racer) = &mut racer {
            history.push(racer.history());
            raced = racer.replies().to_vec();
            raced_as = racer.outcome();
        }
        let keys = KEY..SECOND.0 + 1;
        let lists = e.lists(keys.clone());
        let last = keys
            .zip(lists)
            .map(|(key, list)| iso::Op::Read { key, list })
            .collect();
        history.push(iso::Txn::new(3, Outcome::Committed, last));
        let anomalies = match iso::check(&history) {
            Ok(found) => found.iter().map(|a| format!("{a:?}")).collect(),
            Err(malformed) => vec![format!("{malformed:?}")],
        };
        let writes = !matches!(case.racer, Racer::Read | Racer::Mapper);
        let verify = (case.first == First::Mapper && writes) || profile::every_case();
        let verified = verify.then(|| api::printed(db.verify(None).expect("verify runs")));
        let probe = db.testing().probe();
        Ran {
            mapper: mapper_reached,
            racer: racer_reached,
            mapped: mapper.replies().to_vec(),
            raced,
            raced_as,
            marked_at_stop,
            marks: probe.marks(),
            planted,
            verified,
            page,
            refusals: probe.refusals(),
            anomalies,
        }
    })
}

/// Returns `row`, a printed row of `verify_findings`, without the time of its run.
fn untimed(row: &str) -> String {
    match (row.find(", at: "), row.find(", where: ")) {
        (Some(at), Some(place)) => format!("{}{}", &row[..at], &row[place..]),
        _ => row.to_owned(),
    }
}

fn quarantined() -> Reply {
    Err(Error::Fatal {
        reason: FatalReason::Quarantined {
            what: "elle::counters.Counter".to_owned(),
        },
    })
}

/// Returns each way `r`, a run of `case` at `pair`, breaks what a mapping raced by a step
/// function leaves:
///
/// - the mapper and an unhindered racer each reach their boundaries;
/// - the damaged heap is marked at the mapper's stop exactly from `Marked` on;
/// - the read that meets the damage fails `Quarantined`, a committing mapper commits;
/// - a write of the damaged heap fails `Quarantined` at its statement, refused once the heap is
///   marked and meeting the damage itself before;
/// - the damaged heap alone is marked, the planted page alone refused, and `verify` reports it
///   alone, run on each case of a racer that writes with the mapper stopped first and, under the
///   deep profile, on every case;
/// - the history is serializable.
///
/// `stopped` says whether the side stopped first stopped; when it did not, it ran whole first,
/// and the other may find the damage mapped.
fn judged(case: Case, pair: (usize, usize), stopped: bool, r: &Ran) -> Vec<String> {
    let mut broken = Vec::new();
    let mut expect = |what: &str, ok: bool, got: &dyn std::fmt::Debug| {
        if !ok {
            broken.push(format!("{case:?} at {pair:?}: {what}: {got:?}"));
        }
    };
    let maps = matches!(case.racer, Racer::Mapper | Racer::Commit);
    if case.first == First::Mapper || stopped {
        let mapped = if r.mapper.blocked() && maps {
            vec![]
        } else {
            case.mapper.boundaries()
        };
        expect(
            "the mapper's boundaries",
            r.mapper.steps() == mapped,
            &r.mapper.steps(),
        );
    }
    if let Some(racer) = case.racer.boundaries()
        && !r.racer.blocked()
    {
        expect(
            "the racer's boundaries",
            r.racer.steps() == racer,
            &r.racer.steps(),
        );
    }
    let marked_from = matches!(
        r.mapper.steps().get(pair.0 - 1),
        Some(Step::Map(Map::Marked) | Step::CommitMap(Map::Marked))
    );
    if case.first == First::Mapper && stopped {
        expect(
            "the mark at the mapper's stop",
            r.marked_at_stop == Some(marked_from),
            &r.marked_at_stop,
        );
    }
    let mapped = match case.mapper {
        Mapper::Read => vec![quarantined()],
        Mapper::Commit => vec![Ok(vec![]), Ok(vec![])],
    };
    expect("the mapper's replies", r.mapped == mapped, &r.mapped);
    match case.racer {
        Racer::Damaged(_) => {
            let raced = vec![quarantined()];
            expect("the damaged write's replies", r.raced == raced, &r.raced);
        }
        Racer::Read if case.mapper == Mapper::Commit => {
            expect(
                "the read's end",
                r.raced_as == Some(Outcome::Committed),
                &r.raced_as,
            );
        }
        racer => {
            let raced = match racer {
                Racer::Append | Racer::Update | Racer::Commit => vec![Ok(vec![]), Ok(vec![])],
                Racer::Read => vec![Ok(vec![FIRST]), Ok(vec![])],
                Racer::Checkpoint => vec![],
                Racer::Mapper => vec![quarantined()],
                Racer::Damaged(_) => unreachable!("judged above"),
            };
            expect("the racer's replies", r.raced == raced, &r.raced);
        }
    }
    expect(
        "the marks",
        r.marks == BTreeSet::from([r.planted]),
        &r.marks,
    );
    let (pid, damage) = r.page;
    let refusals = BTreeMap::from([(pid, damage)]);
    expect("the refusals", r.refusals == refusals, &r.refusals);
    let found = vec![format!(
        "Checksum {{ id: verify_findings.key(1), run: RunId(1), where: \"page {}: Checksum\" }}",
        pid.0
    )];
    if let Some(verified) = &r.verified {
        let verified: Vec<String> = verified.iter().map(|v| untimed(v)).collect();
        expect("verify", verified == found, &r.verified);
    }
    expect("the history", r.anomalies.is_empty(), &r.anomalies);
    broken
}

/// Race the mapper against `racer` with each side stopped first in turn, on each seed, judging
/// every case: at each boundary of the side stopped first with the other run whole or blocked,
/// or under the deep profile at every pair. Check that the side stopped first stopped at each of
/// its boundaries, and that the other blocked exactly when it waits for the serialization lock a
/// stopped commit's mapping holds.
fn race(mapper: Mapper, racer: Racer) {
    let mut broken = Vec::new();
    for seed in sweep(SEEDS) {
        for first in [First::Mapper, First::Racer] {
            let case = Case {
                mapper,
                racer,
                first,
                seed,
            };
            let mut one = |(a, b): (usize, usize)| {
                let pair = match first {
                    First::Mapper => (a, b),
                    First::Racer => (b, a),
                };
                let r = run(case, pair);
                let reached = match first {
                    First::Mapper => [r.mapper.clone(), r.racer.clone()],
                    First::Racer => [r.racer.clone(), r.mapper.clone()],
                };
                broken.extend(judged(case, pair, reached[0].stopped(), &r));
                reached
            };
            let (stops, pairs) = if profile::every_case() {
                steps::every_pair(&mut one)
            } else {
                steps::every_boundary(&mut one)
            };
            let declared = match first {
                First::Mapper => mapper.boundaries(),
                First::Racer => racer.first_boundaries(),
            };
            assert_eq!(declared, stops, "{case:?}: the first side's boundaries");
            if !profile::every_case() {
                let blocked = match first {
                    First::Mapper => mapper == Mapper::Commit && racer.serializes(),
                    First::Racer => racer == Racer::Commit,
                };
                let second = if blocked {
                    Second::Blocked
                } else {
                    Second::Whole
                };
                let each: Vec<Pair> = declared.iter().map(|&a| (a, second)).collect();
                assert_eq!(each, pairs, "{case:?}: the pairs run");
            }
        }
    }
    assert_eq!(Vec::<String>::new(), broken);
}

#[test]
fn the_mapper_of_a_read_racing_an_append_an_update_or_a_read_marks_only_the_damaged_heap() {
    for racer in [Racer::Append, Racer::Update, Racer::Read] {
        race(Mapper::Read, racer);
    }
}

#[test]
fn the_mapper_of_a_read_racing_a_checkpoint_or_a_second_mapping_marks_only_the_damaged_heap() {
    for racer in [Racer::Checkpoint, Racer::Mapper] {
        race(Mapper::Read, racer);
    }
}

#[test]
fn a_write_of_the_heap_a_mapping_marks_is_refused_at_each_boundary_of_the_mapping() {
    for write in WRITES {
        race(Mapper::Read, Racer::Damaged(write));
    }
}

#[test]
fn the_mapper_of_a_commit_racing_a_read_a_checkpoint_or_a_second_commit_marks_only_the_damaged_heap()
 {
    for racer in [Racer::Read, Racer::Checkpoint, Racer::Commit] {
        race(Mapper::Commit, racer);
    }
}

/// A walk of the damaged page's holders that meets the heap's unreadable directory is
/// incomplete: the mapping marks nothing and leaves the record unmapped, and a later commit's
/// mapping leaves the record out.
#[test]
fn an_incomplete_walk_marks_nothing_and_leaves_its_record_to_a_later_mapping() {
    let mut sim = Sim::with_files(0, Config::default(), elle::created());
    process::run(&mut sim, |rt| {
        let e = Arc::new(Elems::open(&rt, &elle::options()).expect("the workload opens"));
        let db = e.database();
        let (_, (page, damage), _) = damaged(&rt, &e);
        let heap = db
            .testing()
            .heap("elle::counters", "Counter")
            .expect("the heap");
        let directory = db.testing().root(heap).expect("its directory");
        let unreadable = Damage::Unreadable;
        db.testing()
            .plant(Plant::Refuse {
                page: directory,
                damage: unreadable,
            })
            .expect("the directory is refused");
        let reqs = vec![read_counters()];
        let mut mapper = Txn::start(&rt, &e, (1, Kind::ReadOnly), of_the_mapper, reqs);
        let (k, v) = COMMITTED;
        let reqs = vec![Req::Append(k, v), Req::Commit];
        let mut commit = Txn::start(&rt, &e, (2, Kind::ReadWrite), of_a_commit, reqs);
        let whole = (usize::MAX, usize::MAX);
        let [m, c] = steps::interleave(
            &rt,
            db,
            (&mut mapper, &mut commit),
            whole,
            steps::nothing_between(),
        );
        let walked = [Map::Begun, Map::Incomplete, Map::Marked].map(Step::Map);
        assert_eq!(walked, m.steps(), "the failing read's mapping");
        let committed = [Map::Begun, Map::Marked].map(Step::CommitMap);
        assert_eq!(committed, c.steps(), "the commit's mapping");
        let failed = Err(Error::Fatal {
            reason: FatalReason::Storage {
                detail: format!(
                    "PageError {{ pid: Pid({}), damage: Unreadable }}",
                    directory.0
                ),
            },
        });
        assert_eq!(vec![failed], mapper.replies(), "the read");
        assert_eq!(vec![Ok(vec![]), Ok(vec![])], commit.replies(), "the commit");
        let probe = db.testing().probe();
        assert_eq!(BTreeSet::new(), probe.marks());
        assert_eq!(None, probe.mapped_to(page));
        let refused = BTreeMap::from([(page, damage), (directory, unreadable)]);
        assert_eq!(refused, probe.refusals());
    });
}
