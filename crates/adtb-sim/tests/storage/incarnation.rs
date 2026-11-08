//! A page freed and allocated again is a new incarnation: nothing the log or the damage records
//! hold of the incarnation before judges it.

use adtb::testing::Probe;
use std::collections::BTreeSet;

use adtb::mvcc::{CommitLog, Transactions, Xid};
use adtb::runtime::{IoError, OpKind, Runtime};
use adtb::storage::{
    self, Damage, Finding, HEADER_LEN, Options, PageError, PageKind, Pager, Pid, Quarantine,
};
use adtb_sim::{Action, Config, Match, Plan, Sim, SimRuntime};
use adtb_simkit::damage::{DataFile, Files};
use adtb_simkit::{common, harness};
use adtb_testkit::profile::{assert_reached, every_case};

const PAGER: DataFile = DataFile::new(harness::NAME, harness::PAGE);

const SEED: u64 = 23;
const OLD_OWNER: u32 = 7;
const NEW_OWNER: u32 = 9;

/// The kinds of page the engine frees.
const FREEABLE: [PageKind; 9] = [
    PageKind::HeapFixed,
    PageKind::HeapSlotted,
    PageKind::IndexLeaf,
    PageKind::IndexInner,
    PageKind::HashBucket,
    PageKind::FreeSpaceMap,
    PageKind::VisibilityMap,
    PageKind::CommitLog,
    PageKind::Catalog,
];

const SEGMENT: u64 = 64 * 1024;

fn options(atomic: bool) -> Options {
    Options {
        atomic_page_writes: atomic,
        segment_size: SEGMENT,
        ..harness::options(16)
    }
}

/// Recycle the log, then change `filler` until the log has grown by three segments.
fn recycle_and_fill(pager: &Pager<SimRuntime>, filler: Pid) {
    pager.recycle_log();
    let start = Probe::of(pager).appended().0;
    for n in (0..=u8::MAX).cycle() {
        if Probe::of(pager).appended().0 >= start + 3 * SEGMENT {
            return;
        }
        pager.update(&[filler], |v| v[0][HEADER_LEN] = n).unwrap();
        pager.flush_log();
    }
}

/// Returns `files` with page `pid` set to its bytes in `from`.
fn reverted(mut files: Files, from: &Files, pid: Pid) -> Files {
    PAGER.replace(&mut files, pid, PAGER.page(from, pid));
    files
}

/// What the test did to the page, in order. The page's expected state is read from these
/// alone, never from the engine.
#[derive(Clone, Copy, Debug)]
enum Op {
    Allocate { kind: PageKind, owner: u32 },
    Put(u8),
    Free,
}

#[derive(Debug, Default)]
struct Ops(Vec<Op>);

impl Ops {
    fn allocate<R: Runtime>(&mut self, pager: &Pager<R>, kind: PageKind, owner: u32) -> Pid {
        self.0.push(Op::Allocate { kind, owner });
        pager.allocate(1, kind, owner).unwrap()
    }

    fn put<R: Runtime>(&mut self, pager: &Pager<R>, pid: Pid, value: u8) {
        self.0.push(Op::Put(value));
        pager.update(&[pid], |p| p[0][HEADER_LEN] = value).unwrap();
    }

    fn free<R: Runtime>(&mut self, pager: &Pager<R>, pid: Pid) {
        self.0.push(Op::Free);
        pager.free(pid).unwrap();
    }

    /// Returns the kind, owner and first content byte the last allocation and the puts after it
    /// give the page, `None` once it is freed.
    fn current(&self) -> Option<(u16, u32, u8)> {
        self.0.iter().fold(None, |at, op| match *op {
            Op::Allocate { kind, owner } => Some((kind as u16, owner, 0)),
            Op::Put(v) => at.map(|(k, o, _)| (k, o, v)),
            Op::Free => None,
        })
    }
}

/// Assert that the page `pid` reads as `ops` leave it and that nothing reports it.
#[expect(clippy::disallowed_methods, reason = "until heap owns page checks")]
fn intact(pager: &Pager<SimRuntime>, pid: Pid, ops: &Ops, when: &str) {
    let (kind, owner, value) = ops.current().expect("the page is allocated");
    let read = pager
        .read(pid)
        .map(|p| (storage::kind(&p), storage::owner(&p), p[HEADER_LEN]));
    assert_eq!(Ok((kind, owner, value)), read, "{when}: read");
    let of_page =
        |f: &Finding| matches!(f, Finding::Page { pid: p, .. } | Finding::Orphan(p) if *p == pid);
    let verified: Vec<Finding> = pager
        .verify()
        .findings
        .into_iter()
        .filter(of_page)
        .collect();
    assert_eq!(Vec::<Finding>::new(), verified, "{when}: verify");
    let log = pager.read_log(true);
    assert_eq!(
        Vec::<Finding>::new(),
        pager.check_page(pid, &log),
        "{when}: scrub"
    );
    assert_eq!(
        None,
        Probe::of(pager).records().pages().get(&pid),
        "{when}: recorded"
    );
    assert_eq!(
        None,
        Probe::of(pager).refusals().get(&pid),
        "{when}: refused"
    );
    assert_eq!(
        BTreeSet::<Quarantine>::new(),
        Probe::of(pager).marks(),
        "{when}: marks"
    );
}

/// The evidence the old incarnation leaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Evidence {
    /// Its last write is lost while allocated, and it is freed unread.
    LostThenFreed,
    /// Its last write is lost while allocated, a read refuses it, and it is freed.
    ReadThenFreed,
    /// It is freed, then its last write is lost.
    LostWhileFree,
    /// It is freed, its last write is lost, and a stale read refuses it.
    ReadWhileFree,
    /// It is freed, and damage of it is recorded after the free.
    RecordedAfterFree,
    /// It is freed, the log that held the free is recycled, and damage of it is recorded and
    /// logged.
    RecordedAfterRecycle,
}

const EVIDENCE: [Evidence; 6] = [
    Evidence::LostThenFreed,
    Evidence::ReadThenFreed,
    Evidence::LostWhileFree,
    Evidence::ReadWhileFree,
    Evidence::RecordedAfterFree,
    Evidence::RecordedAfterRecycle,
];

#[derive(Clone, Copy, Debug)]
struct Case {
    old: PageKind,
    new: PageKind,
    evidence: Evidence,
    atomic: bool,
    /// Whether the allocation comes in an open after the one that leaves the evidence.
    reopened: bool,
}

/// Run `case`, asserting the new incarnation is judged by its own operations alone: in the open
/// that allocates it, after a crash, and after a close that folds the damage records. Returns
/// `case` once every step has run.
fn run(case: Case) -> Case {
    let Case {
        old,
        new,
        evidence,
        atomic,
        reopened,
    } = case;
    let options = options(atomic);
    let freed_first = matches!(
        evidence,
        Evidence::LostWhileFree
            | Evidence::ReadWhileFree
            | Evidence::RecordedAfterFree
            | Evidence::RecordedAfterRecycle
    );
    let mut ops = Ops::default();
    let mut sim = Sim::new(SEED, Config::default());
    let mut pids = None;
    harness::finished(&mut sim, |rt| {
        let pager = Pager::create(rt, &options).unwrap();
        let p = ops.allocate(&pager, old, OLD_OWNER);
        ops.put(&pager, p, 1);
        let filler = pager.allocate(1, PageKind::HeapSlotted, OLD_OWNER).unwrap();
        pids = Some((p, filler));
        pager.close();
    });
    let (p, filler) = pids.unwrap();
    let recycled = |sim: &mut Sim| {
        if evidence == Evidence::RecordedAfterRecycle {
            for _ in 0..2 {
                harness::finished(sim, |rt| {
                    let (pager, _) = Pager::open(rt, &options).unwrap();
                    recycle_and_fill(&pager, filler);
                    pager.close();
                });
            }
        }
    };
    recycled(&mut sim);
    let first = sim.files();
    harness::finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt, &options).unwrap();
        ops.put(&pager, p, 2);
        if freed_first {
            ops.free(&pager, p);
        }
        pager.close();
    });
    recycled(&mut sim);
    let second = sim.files();
    let lost = Damage::Stale {
        expected: storage::lsn(PAGER.page(&second, p)),
        found: storage::lsn(PAGER.page(&first, p)),
    };
    let files = match evidence {
        Evidence::RecordedAfterFree | Evidence::RecordedAfterRecycle => second,
        _ => {
            assert_ne!(
                storage::lsn(PAGER.page(&second, p)),
                storage::lsn(PAGER.page(&first, p)),
                "{case:?}: the second open wrote the page"
            );
            reverted(second, &first, p)
        }
    };
    let mut sim = Sim::with_files(SEED, Config::default(), files);
    let allocate = |pager: &Pager<SimRuntime>, ops: &mut Ops| {
        assert_eq!(p, ops.allocate(pager, new, NEW_OWNER), "{case:?}: reuse");
        ops.put(pager, p, 5);
        pager.flush_log();
        intact(pager, p, ops, &format!("{case:?}: allocated"));
    };
    common::run_then_crash(&mut sim, |rt, _| {
        let (pager, _) = Pager::open(rt.clone(), &options).unwrap();
        match evidence {
            Evidence::LostThenFreed => ops.free(&pager, p),
            Evidence::ReadThenFreed => {
                assert_eq!(
                    Err(PageError {
                        pid: p,
                        damage: lost
                    }),
                    pager.read(p).map(drop)
                );
                ops.free(&pager, p);
            }
            Evidence::ReadWhileFree => {
                assert_eq!(
                    Err(PageError {
                        pid: p,
                        damage: lost
                    }),
                    pager.read(p).map(drop)
                );
            }
            Evidence::RecordedAfterFree => drop(pager.damage(p, Damage::Checksum)),
            Evidence::RecordedAfterRecycle => {
                let _ = pager.damage(p, Damage::Checksum);
                assert_eq!(
                    Some(&Damage::Checksum),
                    Probe::of(&pager).records().pages().get(&p),
                    "{case:?}: the damage is recorded"
                );
            }
            Evidence::LostWhileFree => {}
        }
        if reopened {
            pager.close();
            let (pager, _) = Pager::open(rt.clone(), &options).unwrap();
            allocate(&pager, &mut ops);
        } else {
            allocate(&pager, &mut ops);
        }
    });
    let mut sim = Sim::with_files(SEED, Config::default(), sim.files());
    harness::finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt, &options).unwrap();
        intact(&pager, p, &ops, &format!("{case:?}: after a crash"));
        pager.close();
    });
    harness::finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt, &options).unwrap();
        intact(&pager, p, &ops, &format!("{case:?}: after a close"));
    });
    case
}

/// Returns the cases: each freeable kind with each evidence, the atomic writes and the reopen
/// varied across kinds, or every combination under the deep profile.
fn cases() -> Vec<Case> {
    let mut out = Vec::new();
    for (i, &old) in FREEABLE.iter().enumerate() {
        let new = FREEABLE[(i + 1) % FREEABLE.len()];
        for (e, &evidence) in EVIDENCE.iter().enumerate() {
            let varied = [((i + e) % 2 == 0, (i / 2 + e) % 2 == 0)];
            let all = [(false, false), (false, true), (true, false), (true, true)];
            let shapes: &[(bool, bool)] = if every_case() { &all } else { &varied };
            for &(atomic, reopened) in shapes {
                out.push(Case {
                    old,
                    new,
                    evidence,
                    atomic,
                    reopened,
                });
            }
        }
    }
    out
}

#[test]
fn no_consumer_judges_a_reallocated_page_by_the_evidence_of_its_last_incarnation() {
    let ran: Vec<Case> = cases().into_iter().map(run).collect();
    for kind in FREEABLE {
        let reached = ran.iter().filter(|c| c.old == kind || c.new == kind);
        assert_reached(&format!("{kind:?}"), reached.count(), 2);
    }
    for evidence in EVIDENCE {
        for shape in [(false, false), (false, true), (true, false), (true, true)] {
            let reached = ran
                .iter()
                .filter(|c| c.evidence == evidence && (c.atomic, c.reopened) == shape);
            assert_reached(&format!("{evidence:?} {shape:?}"), reached.count(), 1);
        }
    }
}

/// Returns a file whose status page 0 lost the write of the open that aborted its ids, the commit log's root,
/// the page and its ids per page.
fn lost_aborts() -> (Files, Pid, Pid, u64) {
    let options = harness::options(16);
    let mut sim = Sim::new(SEED, Config::default());
    let mut made = None;
    harness::finished(&mut sim, |rt| {
        let pager = Pager::create(rt, &options).unwrap();
        let mut clog = CommitLog::create(&pager).unwrap();
        let per = clog.ids_per_page();
        clog.reserve(&pager, Xid(per), Xid::FIRST).unwrap();
        made = Some((clog.root(), clog.status_pages()[&0], per));
        pager.close();
    });
    let (root, status, per) = made.unwrap();
    let before = sim.files();
    harness::finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt, &options).unwrap();
        CommitLog::open(&pager, root).unwrap();
        pager.close();
    });
    (reverted(sim.files(), &before, status), root, status, per)
}

#[test]
fn a_status_page_settled_freed_and_reused_is_reported_by_nothing() {
    let (files, root, status, per) = lost_aborts();
    let options = harness::options(16);
    let mut sim = Sim::with_files(SEED, Config::default(), files);
    let of_status = |f: &Finding| matches!(f, Finding::Page { pid, .. } if *pid == status);
    harness::finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &options).unwrap();
        let txns = Transactions::open(rt, &pager, root).unwrap();
        let settled = Probe::of(&pager).records().pages().get(&status).copied();
        assert!(
            matches!(settled, Some(Damage::Settled { .. })),
            "the open settles the status page: {settled:?}"
        );
        txns.truncate(&pager, Xid(per)).unwrap();
        drop(txns);
        let mut clog = CommitLog::open(&pager, root).unwrap();
        clog.reserve(&pager, Xid(2 * per), Xid(per)).unwrap();
        assert_eq!(
            status,
            clog.status_pages()[&1],
            "the new status page reuses the pid"
        );
        let found: Vec<Finding> = pager
            .verify()
            .findings
            .into_iter()
            .filter(of_status)
            .collect();
        assert_eq!(Vec::<Finding>::new(), found, "after the reuse");
        pager.close();
    });
    harness::finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt, &options).unwrap();
        CommitLog::open(&pager, root).unwrap();
        let found: Vec<Finding> = pager
            .verify()
            .findings
            .into_iter()
            .filter(of_status)
            .collect();
        assert_eq!(Vec::<Finding>::new(), found, "after a reopen");
        assert_eq!(BTreeSet::<Quarantine>::new(), Probe::of(&pager).marks());
    });
}

#[test]
fn allocation_skips_a_free_page_the_device_could_not_read() {
    let options = options(false);
    let mut sim = Sim::new(SEED, Config::default());
    let mut freed = None;
    harness::finished(&mut sim, |rt| {
        let pager = Pager::create(rt, &options).unwrap();
        let p = pager.allocate(1, PageKind::HeapSlotted, OLD_OWNER).unwrap();
        pager.free(p).unwrap();
        freed = Some(p);
        pager.close();
    });
    let p = freed.unwrap();
    harness::finished(&mut sim, |rt| {
        let reads = Match::kind(OpKind::Read).offset(PAGER.range(p).start as u64);
        rt.plan(Plan::new().rule(reads, Action::Fail(IoError::Failed)));
        let (pager, _) = Pager::open(rt, &options).unwrap();
        let unreadable = PageError {
            pid: p,
            damage: Damage::Unreadable,
        };
        assert_eq!(Err(unreadable), pager.read(p).map(drop));
        let next = Pid(p.0 + 1);
        assert_eq!(
            Ok(next),
            pager.allocate(1, PageKind::HeapSlotted, NEW_OWNER)
        );
        pager.close();
    });
}

/// Returns what a read of a free page finds after an allocation of it latched its blank frame
/// and then failed on a refused page, the free page refused before it when `refused`.
fn read_after_a_failed_allocation(refused: bool) -> (Pid, Result<u8, PageError>) {
    let options = options(false);
    let mut sim = Sim::new(SEED, Config::default());
    let mut pages = None;
    harness::finished(&mut sim, |rt| {
        let pager = Pager::create(rt, &options).unwrap();
        let p = pager.allocate(1, PageKind::HeapSlotted, OLD_OWNER).unwrap();
        let q = pager.allocate(1, PageKind::HeapSlotted, OLD_OWNER).unwrap();
        pager.update(&[p], |v| v[0][HEADER_LEN] = 7).unwrap();
        pager.free(p).unwrap();
        pages = Some((p, q));
        pager.close();
    });
    let (p, q) = pages.unwrap();
    let mut read = None;
    harness::finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt, &options).unwrap();
        if refused {
            pager.damage(p, Damage::Checksum);
        }
        pager.damage(q, Damage::Checksum);
        let mut new = pager.reserve();
        assert_eq!(Ok(p), new.page(1, PageKind::HeapSlotted, NEW_OWNER));
        let failed = pager.update_with(new, &[p, q], |v| v[0][HEADER_LEN] = 1);
        let refusal = PageError {
            pid: q,
            damage: Damage::Checksum,
        };
        assert_eq!(Err(refusal), failed);
        read = Some(pager.read(p).map(|g| g[HEADER_LEN]));
        pager.close();
    });
    (p, read.unwrap())
}

#[test]
fn a_failed_allocation_leaves_its_free_page_as_the_file_holds_it() {
    assert_eq!(Ok(7), read_after_a_failed_allocation(false).1);
}

#[test]
fn a_failed_allocation_leaves_its_free_page_refused_as_before() {
    let (p, read) = read_after_a_failed_allocation(true);
    let refusal = PageError {
        pid: p,
        damage: Damage::Checksum,
    };
    assert_eq!(Err(refusal), read);
}
