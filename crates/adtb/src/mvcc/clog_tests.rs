use crate::testing::TxnProbe;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::{Arc, Mutex};

use crate::mvcc::Fate as Verdict;
use crate::mvcc::{Begun, CommitLog, Hold, Rule, Scope, Status, StatusError, Transactions, Xid};

use super::writers;
use crate::storage::{Damage, Edit, HEADER_LEN, PageKind, Pager, Pid};
use crate::verify::Inventory;
use adtb_sim::{Config, Fate, Finding, Ledger, Recovered, Sim, SimRuntime};
use adtb_sys::runtime::Faults;
use adtb_testkit::Rng;

use crate::test_world as harness;
use crate::test_world::recovery;
use adtb_harness::crash;
use adtb_harness::draw;
use adtb_harness::seeds;

use adtb_harness::damage::{DataFile, Files};
use crash::{Reopened, Rounds, crash_loop};
use draw::below;
use harness::{PAGE, finished, options};
use recovery::verify_after_crash;
use seeds::{share, sweep};

const FRAMES: usize = 6;

/// Create a database with a commit log and `heap_pages` scratch pages, returning their ids.
fn setup(sim: &mut Sim, heap_pages: usize) -> (Pid, Vec<Pid>) {
    let mut out = None;
    finished(sim, |rt| {
        let pager = Pager::create(rt.clone(), &options(FRAMES)).expect("create");
        let txns = Transactions::create(rt, &pager).expect("commit log");
        let pages = (0..heap_pages)
            .map(|_| {
                pager
                    .allocate(1, PageKind::HeapSlotted, 7)
                    .expect("allocate")
            })
            .collect();
        pager.flush_log();
        out = Some((txns.commit_log().root(), pages));
    });
    out.unwrap()
}

fn with_open<T>(
    sim: &mut Sim,
    root: Pid,
    body: impl FnOnce(&mut Pager<SimRuntime>, &mut Transactions<SimRuntime>) -> T,
) -> T {
    let mut out = None;
    finished(sim, |rt| {
        let (mut pager, _) = Pager::open(rt.clone(), &options(FRAMES)).expect("open");
        let mut txns = Transactions::open(rt, &pager, root).expect("open the commit log");
        out = Some(body(&mut pager, &mut txns));
    });
    out.unwrap()
}

/// What the model knows of one transaction: when it ended, by event number, and how.
#[derive(Default)]
struct Model {
    ended: BTreeMap<Xid, (usize, Status)>,
}

impl Model {
    fn committed_before(&self, x: u64, event: usize) -> bool {
        self.ended
            .get(&Xid(x))
            .is_some_and(|&(at, s)| at < event && s == Status::Committed)
    }
}

struct Held {
    hold: Hold,
    taken: usize,
    reader: Option<Xid>,
    scope: Scope,
}

fn pick(rng: &mut Rng, set: &BTreeSet<Xid>) -> Xid {
    *set.iter()
        .nth(rng.below(set.len() as u64) as usize)
        .unwrap()
}

#[test]
fn visibility_through_the_commit_log_matches_a_model() {
    let mut sim = Sim::new(21, Config::default());
    let (root, _) = setup(&mut sim, 0);
    with_open(&mut sim, root, |pager, txns| {
        let mut rng = Rng::new(0x1234_5678_9abc_def1);
        let mut model = Model::default();
        let mut versions: Vec<(u64, u64)> = Vec::new();
        let mut held: Vec<Held> = Vec::new();
        let mut guards: BTreeMap<Xid, Begun> = BTreeMap::new();
        let mut checks = 0;
        let mut event = 0;
        while TxnProbe::of(txns).next_xid().0 < 3 * txns.commit_log().ids_per_page() {
            event += 1;
            match rng.below(16) {
                0..=3 => {
                    let begun = txns.begin(pager).expect("begin");
                    guards.insert(begun.xid(), begun);
                }
                4..=7 if !txns.running().is_empty() => {
                    let x = pick(&mut rng, &txns.running());
                    let begun = guards.remove(&x).expect("a running id has its guard");
                    if rng.below(3) == 0 {
                        txns.abort(pager, begun).expect("abort");
                        model.ended.insert(x, (event, Status::Aborted));
                    } else {
                        txns.commit(pager, begun).expect("commit");
                        model.ended.insert(x, (event, Status::Committed));
                    }
                }
                8 if !txns.running().is_empty() => {
                    let writer = pick(&mut rng, &txns.running());
                    if versions.is_empty() || rng.below(2) == 0 {
                        versions.push((writer.0, 0));
                    } else {
                        let v = rng.below(versions.len() as u64) as usize;
                        if versions[v].1 == 0 {
                            versions[v].1 = writer.0;
                        }
                    }
                }
                9 => {
                    let scope = if rng.below(3) == 0 {
                        Scope::Every
                    } else {
                        Scope::Heaps(BTreeSet::from([rng.below(4) as u32]))
                    };
                    let reader = (!txns.running().is_empty() && rng.below(2) == 0)
                        .then(|| pick(&mut rng, &txns.running()));
                    let hold = txns.take(scope.clone()).unwrap();
                    held.push(Held {
                        hold,
                        taken: event,
                        reader,
                        scope,
                    });
                }
                10 | 13 if !held.is_empty() => {
                    held.swap_remove(rng.below(held.len() as u64) as usize);
                }
                11 if rng.below(16) == 0 => {
                    pager.flush_log();
                    pager.flush_pages(2);
                }
                12 if !held.is_empty() && !versions.is_empty() => {
                    let h = &held[rng.below(held.len() as u64) as usize];
                    for _ in 0..8 {
                        let (xmin, xmax) = versions[rng.below(versions.len() as u64) as usize];
                        let counts =
                            |x: u64| Some(Xid(x)) == h.reader || model.committed_before(x, h.taken);
                        let concurrent = |x: u64| {
                            let aborted = model
                                .ended
                                .get(&Xid(x))
                                .is_some_and(|&(_, s)| s == Status::Aborted);
                            (!counts(x) && !aborted).then_some(Xid(x))
                        };
                        let visible = counts(xmin) && (xmax == 0 || !counts(xmax));
                        let before = match xmax {
                            _ if !counts(xmin) => concurrent(xmin),
                            0 => None,
                            _ => concurrent(xmax),
                        };
                        let reader = txns.reader(pager, &h.hold, h.reader);
                        let seen = reader.sees(&writers(xmin, xmax)).expect("sees");
                        let got = (seen.visible(), reader.before(seen).expect("before"));
                        assert_eq!(
                            (visible, before),
                            got,
                            "version ({xmin}, {xmax}) at {:?} for {:?}",
                            h.hold.snapshot(),
                            h.reader
                        );
                        checks += 1;
                    }
                }
                _ => {}
            }
            for heap in 0..4 {
                if event % 16 != 0 {
                    break;
                }
                let w = txns.watermark(heap);
                for h in &held {
                    if match &h.scope {
                        Scope::Every => true,
                        Scope::Heaps(hs) => hs.contains(&heap),
                    } {
                        assert!(
                            w <= h.hold.snapshot().xmin(),
                            "heap {heap}'s watermark {w:?} passes {:?}",
                            h.hold.snapshot()
                        );
                    }
                }
                assert!(w <= TxnProbe::of(txns).next_xid());
            }
        }
        assert!(
            txns.commit_log().status_pages().len() >= 3,
            "{:?}",
            txns.commit_log()
        );
        assert!(checks > 1000, "{checks} checks");
    });
}

#[test]
fn commit_log_pages_fill_and_page() {
    let mut sim = Sim::new(22, Config::default());
    let (root, _) = setup(&mut sim, 0);
    let mut resolved = BTreeMap::new();
    let (per, cap) = with_open(&mut sim, root, |pager, _| {
        let mut clog = CommitLog::open(pager, root).expect("open");
        let per = clog.ids_per_page();
        let cap = (PAGE - 40) / 16;
        while clog.directory_pages().len() < 3 {
            let from = clog.reserved();
            clog.reserve(pager, Xid(from.0 + per), from)
                .expect("reserve");
            for k in 0..3 {
                let x = Xid(from.0 + (k * 7919 + from.0 % 13) % per);
                if k == 2 {
                    clog.abort(pager, x).expect("abort");
                    resolved.insert(x, Status::Aborted);
                } else {
                    clog.commit(pager, x).expect("commit");
                    resolved.insert(x, Status::Committed);
                }
            }
            if from.0 % (per * 37) < per {
                pager.checkpoint();
            }
        }
        pager.flush_log();
        assert!(clog.status_pages().len() > 2 * cap);
        (per, cap)
    });
    let reserved = with_open(&mut sim, root, |pager, txns| {
        let clog = txns.commit_log();
        assert_eq!(3, clog.directory_pages().len());
        for (&x, &s) in &resolved {
            assert_eq!(s, clog.status(pager, x).expect("status"), "{x:?}");
        }
        clog.reserved()
    });
    let bound = Xid(per * (cap as u64 + 5) + 17);
    let freed = with_open(&mut sim, root, |pager, txns| {
        let before: BTreeSet<Pid> = txns
            .commit_log()
            .status_pages()
            .values()
            .chain(&txns.commit_log().directory_pages())
            .copied()
            .collect();
        txns.truncate(pager, bound).expect("truncate");
        pager.flush_log();
        let clog = txns.commit_log();
        assert_eq!(2, clog.directory_pages().len());
        assert_eq!(bound, clog.bound());
        let after: BTreeSet<Pid> = clog
            .status_pages()
            .values()
            .chain(&clog.directory_pages())
            .copied()
            .collect();
        before
            .difference(&after)
            .copied()
            .collect::<BTreeSet<Pid>>()
    });
    assert_eq!(cap + 5 + 1, freed.len());
    with_open(&mut sim, root, |pager, txns| {
        let clog = txns.commit_log();
        assert_eq!(bound, clog.bound());
        assert_eq!(reserved, clog.reserved());
        for (&x, &s) in &resolved {
            let want = if x < bound { Status::Committed } else { s };
            assert_eq!(want, clog.status(pager, x).expect("status"), "{x:?}");
        }
        let reused = pager
            .allocate(1, PageKind::HeapSlotted, 7)
            .expect("allocate");
        assert!(freed.contains(&reused), "{reused:?} is not a freed page");
        assert!(pager.verify().findings.is_empty());
    });
}

/// What the client knows, kept outside the simulated process so it survives crashes.
#[derive(Default)]
struct World {
    /// Every id the database handed out, in order.
    assigned: Vec<Xid>,
}

#[derive(Debug, Default)]
struct Report {
    statuses: BTreeMap<Xid, Status>,
    findings: Vec<crate::storage::Finding>,
    /// What the whole-file `verify` finds, leaked pages freed first.
    structure: Vec<String>,
    /// Each way the commit log disagrees with the log's commit records or the ids handed out.
    problems: Vec<String>,
}

impl Recovered for Report {
    fn committed(&self) -> BTreeSet<u64> {
        self.statuses
            .iter()
            .filter(|(_, s)| **s == Status::Committed)
            .map(|(x, _)| x.0)
            .collect()
    }

    fn verify(&self) -> Vec<Finding> {
        self.findings
            .iter()
            .map(|f| format!("{f:?}"))
            .chain(self.structure.iter().cloned())
            .map(|at| Finding {
                kind: adtb_sim::FindingKind::Structure,
                at,
            })
            .chain(self.problems.iter().map(|at| Finding {
                kind: adtb_sim::FindingKind::Logic,
                at: at.clone(),
            }))
            .collect()
    }
}

impl Reopened for Report {}

fn workload(rt: SimRuntime, root: Pid, pages: &[Pid], ledger: &Ledger, world: &Mutex<World>) {
    let Ok((pager, _)) = Pager::open(rt.clone(), &options(FRAMES)) else {
        return;
    };
    let txns = Transactions::open(rt.clone(), &pager, root).expect("open the commit log");
    let mut unacked: Vec<Xid> = Vec::new();
    let mut guards: BTreeMap<Xid, Begun> = BTreeMap::new();
    for _ in 0..4000 {
        match below(&rt, 12) {
            0..=3 => {
                let begun = txns.begin(&pager).expect("begin");
                let x = begun.xid();
                let mut w = world.lock().unwrap();
                assert!(
                    w.assigned.last().is_none_or(|&l| l < x),
                    "{x:?} assigned after {:?}",
                    w.assigned.last()
                );
                w.assigned.push(x);
                guards.insert(x, begun);
            }
            4 | 5 if !txns.running().is_empty() => {
                let running: Vec<Xid> = txns.running().iter().copied().collect();
                let x = running[below(&rt, running.len())];
                let pid = pages[below(&rt, pages.len())];
                let at = HEADER_LEN + below(&rt, 400) * 8;
                pager
                    .change(&[(
                        pid,
                        vec![Edit {
                            offset: at as u32,
                            bytes: x.0.to_le_bytes().to_vec(),
                        }],
                    )])
                    .expect("write");
            }
            6 | 7 if !txns.running().is_empty() => {
                let running: Vec<Xid> = txns.running().iter().copied().collect();
                let x = running[below(&rt, running.len())];
                ledger.submit(x.0);
                let begun = guards.remove(&x).expect("a running id has its guard");
                if below(&rt, 4) == 0 {
                    txns.abort(&pager, begun).expect("abort");
                    ledger.abort(x.0);
                } else {
                    txns.commit(&pager, begun).expect("commit");
                    unacked.push(x);
                }
            }
            8 => {
                pager.flush_log();
                for x in unacked.drain(..) {
                    ledger.acknowledge(x.0);
                }
            }
            9 => {
                pager.flush_pages(1 + below(&rt, 3));
            }
            10 if below(&rt, 4) == 0 => pager.checkpoint(),
            _ => {}
        }
    }
}

fn inspect(rt: SimRuntime, root: Pid, pages: &[Pid], assigned: &[Xid]) -> Report {
    let (pager, _) = Pager::open(rt.clone(), &options(FRAMES)).expect("open");
    let txns = Transactions::open(rt, &pager, root).expect("open the commit log");
    let statuses: BTreeMap<Xid, Status> = assigned
        .iter()
        .map(|&x| (x, txns.status(&pager, x).expect("status")))
        .collect();
    let v = pager.verify();
    let clog = txns.commit_log();
    let inventory = Inventory {
        commit_log: Some(&clog),
        pages: pages.to_vec(),
        ..Inventory::default()
    };
    let structure = verify_after_crash(&pager, &inventory);
    let logged: BTreeSet<u64> = v.commits.into_iter().collect();
    let mut problems = Vec::new();
    for (&x, &s) in &statuses {
        if s == Status::InProgress {
            problems.push(format!("{x:?} is still in progress"));
        }
        if (s == Status::Committed) != logged.contains(&x.0) {
            problems.push(format!(
                "{x:?} reads {s:?} against the log's commit records"
            ));
        }
    }
    let next = TxnProbe::of(&txns).next_xid();
    if let Some(&last) = assigned.last()
        && next <= last
    {
        problems.push(format!("next id {next:?} after {last:?}"));
    }
    Report {
        statuses,
        findings: v.findings,
        structure,
        problems,
    }
}

/// Returns how many of `rounds` runs of a random workload under `faults` crashed, and how many of
/// its commits the ledger acknowledged.
#[must_use]
fn crash_rounds(seed: u64, faults: Faults, rounds: usize) -> (usize, usize) {
    let mut sim = Sim::new(seed, Config::default());
    let (root, pages) = setup(&mut sim, 6);
    let world = Arc::new(Mutex::new(World::default()));
    let rounds = Rounds::new(rounds, 400, 40_000);
    let out = crash_loop(
        &mut sim,
        seed,
        faults,
        rounds,
        |r| {
            let (l, w) = (r.ledger().clone(), world.clone());
            r.crash(|rt| workload(rt, root, &pages, &l, &w));
        },
        |rt, _| {
            let assigned = world.lock().unwrap().assigned.clone();
            inspect(rt, root, &pages, &assigned)
        },
    );
    let committed = world
        .lock()
        .unwrap()
        .assigned
        .iter()
        .filter(|x| out.ledger().fate(x.0) == Some(Fate::Acknowledged))
        .count();
    (out.crashed(), committed)
}

#[test]
fn crashes_keep_commits_and_abort_what_was_running() {
    const SEEDS: Range<u64> = 0..10;
    let mut crashes = 0;
    let mut committed = 0;
    for seed in sweep(SEEDS) {
        let (c, n) = crash_rounds(
            seed,
            Faults {
                torn_p: 0.5,
                ..Faults::none(512)
            },
            5,
        );
        crashes += c;
        committed += n;
    }
    assert!(crashes >= share(SEEDS, 30), "{crashes} crashes");
    assert!(
        committed > share(SEEDS, 300),
        "{committed} acknowledged commits"
    );
}

#[test]
fn failed_writes_crash_and_recover_the_commit_log() {
    const SEEDS: Range<u64> = 100..106;
    let mut crashes = 0;
    for seed in sweep(SEEDS) {
        crashes += crash_rounds(
            seed,
            Faults {
                torn_p: 0.5,
                fail_p: 0.005,
                ..Faults::none(512)
            },
            4,
        )
        .0;
    }
    assert!(crashes >= share(SEEDS, 18), "{crashes} crashes");
}

/// Commit a few transactions, write every page to the data file, and return the database's
/// files with the root page of the commit log and its first status page.
fn written(seed: u64) -> (BTreeMap<String, Vec<u8>>, Pid, Pid) {
    let mut sim = Sim::new(seed, Config::default());
    let mut out = None;
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &options(FRAMES)).expect("create");
        let txns = Transactions::create(rt, &pager).expect("commit log");
        for _ in 0..3 {
            let begun = txns.begin(&pager).expect("begin");
            txns.commit(&pager, begun).expect("commit");
        }
        pager.flush_log();
        pager.flush_pages(usize::MAX);
        pager.checkpoint();
        let clog = txns.commit_log();
        out = Some((clog.root(), clog.status_pages()[&0]));
    });
    let (root, status) = out.unwrap();
    (sim.files(), root, status)
}

/// Returns `files` with a byte in the middle of page `pid` of the data file inverted.
fn flipped(mut files: Files, pid: Pid) -> Files {
    let options = options(FRAMES);
    DataFile::new(&options.name, options.page_size as usize).flip(
        &mut files,
        pid,
        PAGE / 2 - HEADER_LEN,
    );
    files
}

#[test]
fn a_damaged_status_page_no_unresolved_id_lies_on_quarantines_the_database() {
    let mut sim = Sim::new(2, Config::default());
    let mut out = None;
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &options(FRAMES)).expect("create");
        let txns = Transactions::create(rt, &pager).expect("commit log");
        while TxnProbe::of(&txns).next_xid().0 < 2 * txns.commit_log().ids_per_page() {
            let begun = txns.begin(&pager).expect("begin");
            txns.abort(&pager, begun).expect("abort");
        }
        pager.flush_log();
        pager.flush_pages(usize::MAX);
        pager.checkpoint();
        let clog = txns.commit_log();
        out = Some((clog.root(), clog.status_pages()[&0]));
    });
    let (root, status) = out.unwrap();
    let mut sim = Sim::with_files(2, Config::default(), flipped(sim.files(), status));
    finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &options(FRAMES)).expect("open");
        let txns = Transactions::open(rt, &pager, root).expect("the commit log opens");
        assert_eq!(
            Some(status),
            TxnProbe::of(&txns).commit_log_damage().map(|e| e.pid)
        );
    });
}

#[test]
fn a_damaged_status_page_quarantines_the_database() {
    let (files, root, status) = written(1);
    let mut sim = Sim::with_files(1, Config::default(), flipped(files, status));
    finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &options(FRAMES)).expect("open");
        let txns = Transactions::open(rt, &pager, root).expect("the commit log opens");
        let damage = TxnProbe::of(&txns)
            .commit_log_damage()
            .expect("the database is quarantined");
        assert_eq!(status, damage.pid);
        let refused = crate::mvcc::Error::Corrupt(damage);
        assert_eq!(Some(refused), txns.begin(&pager).err());
        assert_eq!(Some(refused), txns.take(Scope::Every).err());
        assert_eq!(Err(StatusError::Page(damage)), txns.status(&pager, Xid(1)));
        assert_eq!(root, txns.commit_log().root());
        let clog = txns.commit_log();
        let inventory = Inventory {
            commit_log: Some(&clog),
            ..Inventory::default()
        };
        let report = crate::verify::verify(&pager, &inventory);
        assert!(
            !report.is_clean(),
            "verify reports the damaged status page: {report:?}"
        );
    });
}

#[test]
fn a_damaged_root_refuses_to_open_the_commit_log() {
    let (files, root, _) = written(2);
    let mut sim = Sim::with_files(2, Config::default(), flipped(files, root));
    finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &options(FRAMES)).expect("open");
        let e = Transactions::open(rt, &pager, root).expect_err("the root is damaged");
        assert_eq!(root, e.pid);
    });
}

/// What the fate model knows besides each transaction's end: the commit time the record of the
/// window holds for each committed transaction, and the snapshots held.
struct Histories<'a> {
    model: &'a Model,
    times: &'a BTreeMap<Xid, u64>,
    held: &'a [Held],
    versions: &'a [(u64, u64)],
}

/// The distinct versions met under each rule with each fate, and those the window kept that
/// `Current` and `Moving` judged apart.
#[derive(Default)]
struct Met {
    pairs: BTreeMap<(&'static str, &'static str), BTreeSet<usize>>,
    kept: BTreeSet<usize>,
}

impl Histories<'_> {
    fn ended(&self, x: u64) -> Option<Status> {
        self.model.ended.get(&Xid(x)).map(|&(_, s)| s)
    }

    /// Returns whether the window bounded by `cutoff` keeps a version once it is dead: its
    /// inserter committed, and another transaction deleted it and committed at or after `cutoff`.
    fn window_keeps(&self, (xmin, xmax): (u64, u64), cutoff: Option<u64>) -> bool {
        xmax != 0
            && xmax != xmin
            && self.ended(xmin) == Some(Status::Committed)
            && self.deleted_within((xmin, xmax), cutoff)
    }

    /// Returns whether the version's deleter committed at or after `cutoff`.
    fn deleted_within(&self, (_, xmax): (u64, u64), cutoff: Option<u64>) -> bool {
        let time = self.times.get(&Xid(xmax));
        cutoff.is_some_and(|c| time.is_some_and(|&t| t >= c))
    }

    fn live(&self, heap: u32) -> impl Iterator<Item = &Held> {
        self.held.iter().filter(move |h| match &h.scope {
            Scope::Every => true,
            Scope::Heaps(hs) => hs.contains(&heap),
        })
    }

    /// Check what `fate` claims of a version of heap `heap` against the snapshots that hold it
    /// and the transactions' ends.
    fn check_visibility(&self, fate: Verdict, (xmin, xmax): (u64, u64), heap: u32, at: &str) {
        let sees = |h: &Held| {
            let counts =
                |x: u64| Some(Xid(x)) == h.reader || self.model.committed_before(x, h.taken);
            counts(xmin) && (xmax == 0 || !counts(xmax))
        };
        match fate {
            Verdict::Remove => {
                assert!(
                    self.live(heap).all(|h| !sees(h)),
                    "{at} is removed, yet a live snapshot sees it"
                );
                assert!(
                    self.ended(xmin) == Some(Status::Aborted)
                        || (xmax != 0 && self.ended(xmax) == Some(Status::Committed)),
                    "{at} is removed, yet a later snapshot may see it"
                );
            }
            Verdict::AllVisible => {
                assert!(
                    self.live(heap).all(sees),
                    "{at} is all-visible, yet a live snapshot misses it"
                );
                assert!(
                    xmax == 0 && self.ended(xmin) == Some(Status::Committed),
                    "{at} is all-visible, yet a later snapshot may miss it"
                );
            }
            Verdict::Clear => assert!(
                xmax != 0 && self.ended(xmax) == Some(Status::Aborted),
                "{at} is cleared, yet its deleter did not abort"
            ),
            Verdict::Keep => {}
        }
    }

    /// Judge every version of heap `heap` under `Current`, `Moving` and `Past` with `cutoff`, and
    /// by the horizon on-access pruning takes now, and check each verdict against the model.
    fn check(
        &self,
        pager: &Pager<SimRuntime>,
        txns: &Transactions<SimRuntime>,
        heap: u32,
        cutoff: Option<u64>,
        event: usize,
        met: &mut Met,
    ) {
        let now = txns.horizon_now(heap);
        let now_cutoff = txns.cutoff();
        let current = txns.horizon_fixed(heap, cutoff, Rule::Current);
        let moving = txns.horizon_fixed(heap, cutoff, Rule::Moving);
        let past = txns.horizon_fixed(heap, cutoff, Rule::Past);
        let oldest = txns.running().first().copied();
        let watermark = self
            .live(heap)
            .map(|h| h.hold.snapshot().xmin())
            .chain(oldest)
            .fold(TxnProbe::of(txns).next_xid(), Xid::min);
        for (n, &v) in self.versions.iter().enumerate() {
            let (xmin, xmax) = v;
            let at = format!("event {event}: version ({xmin}, {xmax}) in heap {heap}");
            let fate = |h| txns.fate(pager, &writers(xmin, xmax), h).expect("fate");
            let (f_now, f_current, f_moving, f_past) =
                (fate(&now), fate(&current), fate(&moving), fate(&past));
            for f in [f_now, f_current, f_moving] {
                self.check_visibility(f, v, heap, &at);
            }
            for (f, c) in [(f_now, now_cutoff), (f_current, cutoff)] {
                assert!(
                    f != Verdict::Remove || !self.window_keeps(v, c),
                    "{at} is removed under Current, yet the window bounded by {c:?} keeps it"
                );
            }
            let dead = self.ended(xmin) == Some(Status::Committed)
                && xmax != 0
                && self.ended(xmax) == Some(Status::Committed)
                && Xid(xmax) < watermark;
            let kept = dead && self.window_keeps(v, cutoff);
            assert_eq!(
                kept,
                f_current != f_moving,
                "{at}: Current judged {f_current:?} and Moving {f_moving:?} under {cutoff:?}"
            );
            if kept {
                assert_eq!(
                    (Verdict::Keep, Verdict::Remove),
                    (f_current, f_moving),
                    "{at}"
                );
                met.kept.insert(n);
            }
            assert_eq!(
                self.deleted_within(v, cutoff),
                f_past == Verdict::Keep,
                "{at}: Past judged {f_past:?} under {cutoff:?}"
            );
            for (rule, f) in [
                ("current", f_current),
                ("moving", f_moving),
                ("past", f_past),
            ] {
                met.pairs.entry((rule, name(f))).or_default().insert(n);
            }
        }
    }
}

fn name(f: Verdict) -> &'static str {
    match f {
        Verdict::Remove => "remove",
        Verdict::AllVisible => "all-visible",
        Verdict::Clear => "clear",
        Verdict::Keep => "keep",
    }
}

#[test]
fn fate_matches_a_model_of_random_histories() {
    let mut sim = Sim::new(23, Config::default());
    let (root, _) = setup(&mut sim, 0);
    with_open(&mut sim, root, |pager, txns| {
        let past = crate::mvcc::Past::create(pager, 9).expect("record of the window");
        let window = std::time::Duration::from_millis(5);
        txns.keep_history(pager, past, window)
            .expect("keep history");
        let mut rng = Rng::new(0x0f1e_2d3c_4b5a_6978);
        let mut model = Model::default();
        let mut times: BTreeMap<Xid, u64> = BTreeMap::new();
        let mut versions: Vec<(u64, u64)> = Vec::new();
        let mut held: Vec<Held> = Vec::new();
        let mut guards: BTreeMap<Xid, Begun> = BTreeMap::new();
        let mut met = Met::default();
        for event in 1..6000 {
            match rng.below(16) {
                0..=3 => {
                    let begun = txns.begin(pager).expect("begin");
                    guards.insert(begun.xid(), begun);
                }
                4..=7 if !txns.running().is_empty() => {
                    let x = pick(&mut rng, &txns.running());
                    let begun = guards.remove(&x).expect("a running id has its guard");
                    if rng.below(3) == 0 {
                        txns.abort(pager, begun).expect("abort");
                        model.ended.insert(x, (event, Status::Aborted));
                    } else {
                        txns.commit(pager, begun).expect("commit");
                        model.ended.insert(x, (event, Status::Committed));
                        let record = txns.history().expect("history is kept");
                        let time = record.committed_at(pager, x).expect("commit time");
                        times.insert(x, time.expect("a commit records its time"));
                    }
                    held.retain(|h| h.reader != Some(x));
                }
                8 | 9 if !txns.running().is_empty() => {
                    let writer = pick(&mut rng, &txns.running());
                    if versions.is_empty() || rng.below(2) == 0 {
                        versions.push((writer.0, 0));
                    } else {
                        let v = rng.below(versions.len() as u64) as usize;
                        if versions[v].1 == 0 {
                            versions[v].1 = writer.0;
                        }
                    }
                }
                10 | 11 => {
                    let scope = if rng.below(3) == 0 {
                        Scope::Every
                    } else {
                        Scope::Heaps(BTreeSet::from([rng.below(4) as u32]))
                    };
                    let reader = (!txns.running().is_empty() && rng.below(2) == 0)
                        .then(|| pick(&mut rng, &txns.running()));
                    let hold = txns.take(scope.clone()).unwrap();
                    held.push(Held {
                        hold,
                        taken: event,
                        reader,
                        scope,
                    });
                }
                12 | 13 if !held.is_empty() => {
                    held.swap_remove(rng.below(held.len() as u64) as usize);
                }
                14 if !versions.is_empty() => {
                    let cutoff = match rng.below(4) {
                        0 => None,
                        _ if times.is_empty() => txns.cutoff(),
                        _ => {
                            let pick = rng.below(times.len() as u64) as usize;
                            times.values().nth(pick).copied()
                        }
                    };
                    let histories = Histories {
                        model: &model,
                        times: &times,
                        held: &held,
                        versions: &versions,
                    };
                    let heap = rng.below(4) as u32;
                    histories.check(pager, txns, heap, cutoff, event, &mut met);
                }
                _ => {}
            }
        }
        let distinct: BTreeSet<u64> = times.values().copied().collect();
        assert!(distinct.len() > 10, "commit times {distinct:?}");
        let fates = ["remove", "clear", "keep", "all-visible"];
        let pairs = fates
            .iter()
            .flat_map(|&f| [("current", f), ("moving", f)])
            .chain([("past", "remove"), ("past", "keep")]);
        for (rule, f) in pairs {
            let n = met.pairs.get(&(rule, f)).map_or(0, BTreeSet::len);
            assert!(n >= 5, "{n} distinct versions judged {f} under {rule}");
        }
        assert_eq!(10, met.pairs.len(), "{:?}", met.pairs.keys());
        assert!(
            !met.kept.is_empty(),
            "no version met was kept by the window"
        );
    });
}

#[test]
fn a_dropped_hold_releases_its_heaps() {
    let mut sim = Sim::new(5, Config::default());
    let (root, _) = setup(&mut sim, 0);
    with_open(&mut sim, root, |pager, txns| {
        let begun = txns.begin(pager).expect("begin");
        let x = begun.xid();
        let hold = txns
            .take(Scope::Heaps(BTreeSet::from([1])))
            .expect("a snapshot");
        txns.commit(pager, begun).expect("commit");
        let next = TxnProbe::of(txns).next_xid();
        assert_eq!(x, txns.watermark(1));
        drop(hold);
        assert_eq!(next, txns.watermark(1));
    });
}

#[test]
fn a_reader_keeps_the_hold_it_was_built_from_until_it_drops() {
    let mut sim = Sim::new(5, Config::default());
    let (root, _) = setup(&mut sim, 0);
    with_open(&mut sim, root, |pager, txns| {
        let begun = txns.begin(pager).expect("begin");
        let x = begun.xid();
        let hold = txns
            .take(Scope::Heaps(BTreeSet::from([1])))
            .expect("a snapshot");
        let reader = txns.reader(pager, &hold, None);
        txns.commit(pager, begun).expect("commit");
        let next = TxnProbe::of(txns).next_xid();
        drop(hold);
        let kept = txns.watermark(1);
        drop(reader);
        assert_eq!((x, next), (kept, txns.watermark(1)));
    });
}

#[test]
fn a_dropped_guard_aborts_its_id_and_holds_back_no_later_snapshot() {
    let mut sim = Sim::new(6, Config::default());
    let (root, _) = setup(&mut sim, 0);
    with_open(&mut sim, root, |pager, txns| {
        let begun = txns.begin(pager).expect("begin");
        let x = begun.xid();
        drop(begun);
        let hold = txns
            .take(Scope::Heaps(BTreeSet::from([1])))
            .expect("a snapshot");
        let next = TxnProbe::of(txns).next_xid();
        let got = (
            txns.running(),
            txns.watermark(1),
            txns.recorded(pager, x),
            txns.unrecorded_aborts(),
        );
        assert_eq!((BTreeSet::new(), next, Ok(Status::Aborted), 0), got);
        drop(hold);
    });
}

/// Returns the status page of the id `begun` holds and the id's index on it.
fn status_slot(txns: &Transactions<SimRuntime>, begun: &Begun) -> (Pid, usize) {
    let clog = txns.commit_log();
    let (n, i) = super::clog::locate(begun.xid(), clog.ids_per_page());
    (clog.status_pages()[&n], i)
}

#[test]
fn a_guard_dropped_when_its_abort_record_fails_takes_its_id_out_of_running() {
    let mut sim = Sim::new(7, Config::default());
    let (root, _) = setup(&mut sim, 0);
    let (status, i) = with_open(&mut sim, root, |pager, txns| {
        let begun = txns.begin(pager).expect("begin");
        let (status, i) = status_slot(txns, &begun);
        pager.flush_log();
        pager.flush_pages(usize::MAX);
        let damage = pager.plant_damage(status, Damage::Checksum);
        drop(begun);
        let next = TxnProbe::of(txns).next_xid();
        let got = (
            txns.running(),
            txns.watermark(1),
            txns.unrecorded_aborts(),
            pager.whole_cause(),
        );
        assert_eq!((BTreeSet::new(), next, 1, Some(damage)), got);
        (status, i)
    });
    let files = sim.files();
    let page = DataFile::new(&options(FRAMES).name, PAGE).page(&files, status);
    assert_eq!(Status::InProgress, super::clog::status_at(page, i));
}

#[test]
fn a_guard_dropped_in_a_panic_bounds_the_next_reservation_and_its_id_recovers_as_aborted() {
    let mut sim = Sim::new(9, Config::default());
    let (root, _) = setup(&mut sim, 0);
    let (x, before) = with_open(&mut sim, root, |pager, txns| {
        let begun = txns.begin(pager).expect("begin");
        let x = begun.xid();
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _held = begun;
            panic!("the holder of the guard panics");
        }));
        let panicked = unwound.is_err();
        let got = (
            panicked,
            txns.running(),
            txns.recorded(pager, x),
            txns.unrecorded_aborts(),
        );
        let reserved = txns.commit_log().reserved();
        while TxnProbe::of(txns).next_xid() <= reserved {
            let later = txns.begin(pager).expect("begin");
            txns.commit(pager, later).expect("commit");
        }
        pager.flush_log();
        let page = pager.read(root).expect("the root page");
        (x, (got, super::clog::Root::read(&page).resolved))
    });
    let unresolved = (true, BTreeSet::new(), Ok(Status::InProgress), 1);
    assert_eq!((unresolved, x), before);
    let after = with_open(&mut sim, root, |pager, txns| txns.recorded(pager, x));
    assert_eq!(Ok(Status::Aborted), after);
}

#[test]
fn a_guard_dropped_after_its_pager_closes_releases_its_id_and_writes_nothing() {
    let mut sim = Sim::new(10, Config::default());
    let (root, scratch) = setup(&mut sim, 2 * FRAMES);
    finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &options(FRAMES)).expect("open");
        let txns = Transactions::open(rt.clone(), &pager, root).expect("open the commit log");
        let begun = txns.begin(&pager).expect("begin");
        let (status, _) = status_slot(&txns, &begun);
        pager.flush_pages(usize::MAX);
        for &pid in &scratch {
            drop(pager.read(pid).expect("a scratch page"));
        }
        assert!(
            pager.try_optimistic(status).is_none(),
            "the status page is evicted"
        );
        pager.close();
        let closed = rt.files();
        drop(begun);
        let got = (
            txns.running(),
            txns.unrecorded_aborts(),
            rt.files() == closed,
        );
        assert_eq!((BTreeSet::new(), 1, true), got);
    });
}

#[test]
fn a_failed_commit_takes_its_id_out_of_running_and_returns_the_commit_failure() {
    let mut sim = Sim::new(8, Config::default());
    let (root, _) = setup(&mut sim, 0);
    with_open(&mut sim, root, |pager, txns| {
        let begun = txns.begin(pager).expect("begin");
        let (status, _) = status_slot(txns, &begun);
        let damage = pager.plant_damage(status, Damage::Checksum);
        let committed = txns.commit(pager, begun);
        let got = (committed, txns.running(), txns.unrecorded_aborts());
        let refused = crate::mvcc::Error::Corrupt(damage);
        assert_eq!((Err(refused), BTreeSet::new(), 1), got);
    });
}
