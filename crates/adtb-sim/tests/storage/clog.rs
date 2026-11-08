use adtb::testing::TxnProbe;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::{Arc, Mutex};

use adtb::mvcc::{CommitLog, Scope, Snapshot, SnapshotId, Status, Transactions, Xid};
use adtb::storage::{Edit, HEADER_LEN, PageKind, Pager, Pid};
use adtb::verify::Inventory;
use adtb_sim::{Config, Fate, Finding, Ledger, Recovered, Sim, SimRuntime};
use adtb_sys::runtime::Faults;
use adtb_testkit::Rng;

use adtb_harness::crash;
use adtb_harness::draw;
use adtb_harness::seeds;
use adtb_simkit::harness;
use adtb_simkit::recovery;

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
    id: SnapshotId,
    taken: usize,
    reader: Option<Xid>,
    snapshot: Snapshot,
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
        let mut checks = 0;
        let mut event = 0;
        while TxnProbe::of(txns).next_xid().0 < 3 * txns.commit_log().ids_per_page() {
            event += 1;
            match rng.below(16) {
                0..=3 => {
                    txns.begin(pager).expect("begin");
                }
                4..=7 if !txns.running().is_empty() => {
                    let x = pick(&mut rng, &txns.running());
                    if rng.below(3) == 0 {
                        txns.abort(pager, x).expect("abort");
                        model.ended.insert(x, (event, Status::Aborted));
                    } else {
                        txns.commit(pager, x).expect("commit");
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
                    let (id, snapshot) = txns.take(scope.clone()).unwrap();
                    held.push(Held {
                        id,
                        taken: event,
                        reader,
                        snapshot,
                        scope,
                    });
                }
                10 | 13 if !held.is_empty() => {
                    let h = held.swap_remove(rng.below(held.len() as u64) as usize);
                    txns.release(h.id);
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
                        let want = counts(xmin) && (xmax == 0 || !counts(xmax));
                        let got = txns
                            .sees(pager, &h.snapshot, h.reader, xmin, xmax)
                            .expect("sees");
                        assert_eq!(
                            want, got,
                            "version ({xmin}, {xmax}) at {:?} for {:?}",
                            h.snapshot, h.reader
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
                            w <= h.snapshot.xmin(),
                            "heap {heap}'s watermark {w:?} passes {:?}",
                            h.snapshot
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
    findings: Vec<adtb::storage::Finding>,
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
    for _ in 0..4000 {
        match below(&rt, 12) {
            0..=3 => {
                let x = txns.begin(&pager).expect("begin");
                let mut w = world.lock().unwrap();
                assert!(
                    w.assigned.last().is_none_or(|&l| l < x),
                    "{x:?} assigned after {:?}",
                    w.assigned.last()
                );
                w.assigned.push(x);
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
                if below(&rt, 4) == 0 {
                    txns.abort(&pager, x).expect("abort");
                    ledger.abort(x.0);
                } else {
                    txns.commit(&pager, x).expect("commit");
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
            let x = txns.begin(&pager).expect("begin");
            txns.commit(&pager, x).expect("commit");
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
            let x = txns.begin(&pager).expect("begin");
            txns.abort(&pager, x).expect("abort");
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
        let refused = adtb::mvcc::Error::Corrupt(damage);
        assert_eq!(Err(refused), txns.begin(&pager));
        assert_eq!(Some(refused), txns.take(Scope::Every).err());
        assert_eq!(Err(damage), txns.status(&pager, Xid(1)));
        assert_eq!(root, txns.commit_log().root());
        let clog = txns.commit_log();
        let inventory = Inventory {
            commit_log: Some(&clog),
            ..Inventory::default()
        };
        let report = adtb::verify::verify(&pager, &inventory);
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
