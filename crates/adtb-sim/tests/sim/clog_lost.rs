//! A status page that lost the write of a logged commit: the open settles the id from its commit
//! record in the retained log, aborts only an id the log holds no commit record of, and writes
//! nothing when the log it needs is not whole.

use std::collections::BTreeMap;

use adtb::mvcc::{CommitLog, Status, Transactions, Xid};
use adtb::storage::{self, Damage, HEADER_LEN, Lsn, PageError, PageKind, Pager, Pid};
use adtb::{Database, Error, FatalReason, Log};
use adtb_sim::{Config, Sim, SimRuntime};
use adtb_simkit::{api, common, harness};

const APP: &str = r"module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
";
const DB_PAGE: usize = 8192;
const FRAMES: usize = 6;

type Files = BTreeMap<String, Vec<u8>>;

/// Returns `files` with page `pid` of the data file, of `page` bytes, set to its bytes in `from`.
fn reverted(mut files: Files, from: &Files, pid: Pid, page: usize) -> Files {
    let at = pid.0 as usize * page;
    files.get_mut(harness::NAME).unwrap()[at..at + page]
        .copy_from_slice(&from[harness::NAME][at..at + page]);
    files
}

fn open(rt: &SimRuntime) -> Database<SimRuntime> {
    Database::open_in(
        rt.clone(),
        harness::NAME,
        &Log::new([APP]),
        &api::options(2),
    )
    .unwrap()
}

/// Returns the status of each of `ids` in the commit log of `db`.
fn statuses(db: &Database<SimRuntime>, ids: &[Xid]) -> Vec<Status> {
    let w = db.write_path();
    let clog = w.txns().commit_log();
    ids.iter()
        .map(|&x| clog.status(w.pager(), x).unwrap())
        .collect()
}

/// Insert a user named `name` in one process. Returns the last status page and the ids the
/// process committed.
fn insert(sim: &mut Sim, name: &str) -> (Pid, Vec<Xid>) {
    api::run(sim, |rt| {
        let db = open(&rt);
        let first = db.write_path().txns().next();
        api::results(&db, &format!("insert users {{ name: \"{name}\" }}"));
        let ids: Vec<Xid> = (first.0..db.write_path().txns().next().0)
            .map(Xid)
            .collect();
        let done = statuses(&db, &ids);
        let committed = ids.into_iter().zip(done);
        let committed = committed.filter_map(|(x, s)| (s == Status::Committed).then_some(x));
        let txns = db.write_path().txns();
        let page = *txns
            .commit_log()
            .status_pages()
            .values()
            .next_back()
            .unwrap();
        let out = (page, committed.collect());
        db.close().unwrap();
        out
    })
}

fn quarantined() -> Error {
    Error::Fatal {
        reason: FatalReason::Quarantined {
            what: "the database".to_owned(),
        },
    }
}

/// Returns the constructor and place of each row of a `verify`.
fn findings(rows: adtb::DynRows) -> Vec<(String, String)> {
    let row = |r: String| {
        let (ctor, rest) = r.split_once(" {").unwrap();
        let (_, at) = rest.split_once("where: ").unwrap();
        (ctor.to_owned(), at.trim_end_matches(" }").to_owned())
    };
    api::sorted(rows).into_iter().map(row).collect()
}

#[test]
fn a_commit_whose_status_page_lost_the_write_reads_committed_until_verify_marks_the_database() {
    let mut sim = Sim::new(1, Config::default());
    insert(&mut sim, "ann");
    let before = sim.files();
    let (page, bo) = insert(&mut sim, "bo");
    assert!(!bo.is_empty(), "the insert commits");
    let lsn = |f: &Files| storage::lsn(&f[harness::NAME][page.0 as usize * DB_PAGE..]);
    let (expected, found) = (lsn(&sim.files()), lsn(&before));
    let lost = reverted(sim.files(), &before, page, DB_PAGE);
    assert_ne!(lost, sim.files(), "the revert loses a write");
    let mut sim = Sim::with_files(1, Config::default(), lost);
    let committed = vec![Status::Committed; bo.len()];
    api::run(&mut sim, |rt| {
        let db = open(&rt);
        let names = api::query(&db, "from u in app::users select u.name");
        assert_eq!(vec!["\"ann\"", "\"bo\""], names);
        db.close().unwrap();
    });
    api::run(&mut sim, |rt| {
        let db = open(&rt);
        assert_eq!(committed, statuses(&db, &bo));
        assert_eq!(Some(quarantined()), db.verify(None).err());
        let stale = format!(
            "\"page {}: Stale {{ expected: {expected:?}, found: {found:?} }}\"",
            page.0
        );
        let want = [
            ("StalePage", stale.as_str()),
            (
                "Structure",
                "\"the database: only the pages and the log were checked\"",
            ),
            (
                "Structure",
                "\"the database: quarantined whole, so the findings of this run are not kept\"",
            ),
        ];
        let want: Vec<(String, String)> = want.map(|(c, a)| (c.to_owned(), a.to_owned())).into();
        assert_eq!(want, findings(db.verify(None).unwrap()));
        db.close().unwrap();
    });
    api::run(&mut sim, |rt| {
        let db = open(&rt);
        assert_eq!(committed, statuses(&db, &bo));
        let names = api::failure(&db, "from u in app::users select u.name");
        assert_eq!(quarantined(), names);
    });
}

#[derive(Clone, Copy, Debug)]
enum Op {
    Commit,
    Abort,
}

const SEQUENCE: [Op; 4] = [Op::Commit, Op::Abort, Op::Commit, Op::Commit];

#[derive(Clone, Copy, Debug)]
enum End {
    Close,
    Crash,
}

/// What one process of the sweep leaves: the commit log's root, its one status page, and each id
/// it assigned with the status it must read after a reopen.
type Session = (Pid, Pid, Vec<(Xid, Status)>);

/// Run `ops` on the commit log rooted at `root`, or a new one, leave one more id running, force the
/// log, which acknowledges each commit, write every page, and end as `end` says.
fn session(sim: &mut Sim, root: Option<Pid>, ops: &[Op], end: End) -> Session {
    let mut out = None;
    let mut body = |rt: &SimRuntime| {
        let options = harness::options(FRAMES);
        let (pager, txns) = match root {
            None => {
                let pager = Pager::create(rt.clone(), &options).unwrap();
                let txns = Transactions::create(rt.clone(), &pager).unwrap();
                (pager, txns)
            }
            Some(root) => {
                let (pager, _) = Pager::open(rt.clone(), &options).unwrap();
                let txns = Transactions::open(rt.clone(), &pager, root).unwrap();
                (pager, txns)
            }
        };
        let mut ids = Vec::new();
        for op in ops {
            let x = txns.begin(&pager).unwrap();
            let status = match op {
                Op::Commit => txns.commit(&pager, x).map(|_| Status::Committed),
                Op::Abort => txns.abort(&pager, x).map(|_| Status::Aborted),
            };
            ids.push((x, status.unwrap()));
        }
        ids.push((txns.begin(&pager).unwrap(), Status::Aborted));
        pager.flush_log();
        let clog = txns.commit_log();
        assert_eq!(1, clog.status_pages().len(), "one status page");
        out = Some((clog.root(), clog.status_pages()[&0], ids));
        drop(clog);
        match end {
            End::Close => pager.close(),
            End::Crash => {
                pager.flush_pages(usize::MAX);
                pager.checkpoint();
            }
        }
    };
    match end {
        End::Close => harness::finished(sim, |rt| body(&rt)),
        End::Crash => common::run_then_crash(sim, |rt, _| body(rt)),
    }
    out.unwrap()
}

/// Returns the status of each of `ids` after an open of the commit log rooted at `root` in `files`,
/// through the transactions layer and through the commit log alone.
fn reopened(seed: u64, files: &Files, root: Pid, ids: &[Xid]) -> [Vec<Status>; 2] {
    [false, true].map(|alone| {
        let mut sim = Sim::with_files(seed, Config::default(), files.clone());
        let mut out = None;
        harness::finished(&mut sim, |rt| {
            let (pager, _) = Pager::open(rt.clone(), &harness::options(FRAMES)).unwrap();
            out = Some(if alone {
                let clog = CommitLog::open(&pager, root).unwrap();
                ids.iter()
                    .map(|&x| clog.status(&pager, x).unwrap())
                    .collect()
            } else {
                let txns = Transactions::open(rt, &pager, root).unwrap();
                ids.iter()
                    .map(|&x| txns.status(&pager, x).unwrap())
                    .collect()
            });
        });
        out.unwrap()
    })
}

#[test]
fn every_point_a_written_status_change_is_lost_at_keeps_exactly_the_acknowledged_commits() {
    let mut reached = 0;
    for end in [End::Close, End::Crash] {
        for k in 0..SEQUENCE.len() {
            let mut sim = Sim::new(k as u64, Config::default());
            let (root, page, mut ids) = session(&mut sim, None, &SEQUENCE[..k], End::Close);
            let before = sim.files();
            let (_, _, later) = session(&mut sim, Some(root), &SEQUENCE[k..], end);
            ids.extend(later);
            let files = reverted(sim.files(), &before, page, harness::PAGE);
            assert_ne!(
                files,
                sim.files(),
                "{end:?}, op {k}: the revert loses a write"
            );
            let (xs, want): (Vec<Xid>, Vec<Status>) = ids.into_iter().unzip();
            let got = reopened(k as u64, &files, root, &xs);
            assert_eq!([want.clone(), want], got, "{end:?}, lost from op {k}");
            reached += 1;
        }
    }
    assert_eq!(2 * SEQUENCE.len(), reached);
}

/// Returns the files of a commit log whose status page was last written in the first log segment,
/// then `commits` commits, each forced alone, and one id left running, with every write of the
/// status page after the first segment lost; the root, the status page and each id with the status
/// it must read.
fn lost_past_the_first_segment(commits: usize) -> (Files, Pid, Pid, Vec<(Xid, Status)>) {
    let options = harness::options(FRAMES);
    let mut sim = Sim::new(5, Config::default());
    let mut out = None;
    harness::finished(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &options).unwrap();
        let txns = Transactions::create(rt, &pager).unwrap();
        let scratch = pager.allocate(1, PageKind::HeapSlotted, 7).unwrap();
        let x = txns.begin(&pager).unwrap();
        txns.commit(&pager, x).unwrap();
        while pager.appended().0 <= options.segment_size {
            pager.update(&[scratch], |p| p[0][HEADER_LEN] ^= 1).unwrap();
            pager.flush_log();
        }
        let clog = txns.commit_log();
        out = Some((clog.root(), clog.status_pages()[&0]));
        drop(clog);
        pager.close();
    });
    let (root, page) = out.unwrap();
    let before = sim.files();
    let mut ids = Vec::new();
    harness::finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &options).unwrap();
        let txns = Transactions::open(rt, &pager, root).unwrap();
        for _ in 0..commits {
            let x = txns.begin(&pager).unwrap();
            txns.commit(&pager, x).unwrap();
            pager.flush_log();
            ids.push((x, Status::Committed));
        }
        ids.push((txns.begin(&pager).unwrap(), Status::Aborted));
        pager.close();
    });
    (
        reverted(sim.files(), &before, page, harness::PAGE),
        root,
        page,
        ids,
    )
}

/// Returns `files` without the log segment starting at `base`.
fn without_segment(mut files: Files, base: u64) -> Files {
    let path = format!("{}/{base:016x}", harness::options(FRAMES).log_dir());
    assert!(files.remove(&path).is_some(), "{path} is a segment");
    files
}

#[test]
fn a_segment_recycled_before_the_lost_writes_records_leaves_the_status_page_to_settle_from_its_log()
{
    let (files, root, _, ids) = lost_past_the_first_segment(3);
    let files = without_segment(files, 0);
    let (xs, want): (Vec<Xid>, Vec<Status>) = ids.into_iter().unzip();
    assert_eq!([want.clone(), want], reopened(5, &files, root, &xs));
}

#[test]
fn a_segment_missing_after_the_status_pages_last_write_quarantines_the_database_and_writes_nothing()
{
    let segment = harness::options(FRAMES).segment_size;
    let commits = (segment / 512) as usize + 16;
    let (files, root, page, ids) = lost_past_the_first_segment(commits);
    let files = without_segment(files, segment);
    let damage = PageError {
        pid: page,
        damage: Damage::LogUnproven { at: Lsn(segment) },
    };
    let mut sim = Sim::with_files(5, Config::default(), files);
    harness::finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &harness::options(FRAMES)).unwrap();
        assert_eq!(Some(damage), CommitLog::open(&pager, root).err());
        let txns = Transactions::open(rt, &pager, root).unwrap();
        assert_eq!(Some(damage), txns.quarantined());
        let clog = txns.commit_log();
        let got: Vec<Status> = ids
            .iter()
            .map(|&(x, _)| clog.status(&pager, x).unwrap())
            .collect();
        assert_eq!(vec![Status::InProgress; ids.len()], got);
    });
}

#[test]
fn a_dump_is_refused_once_the_log_cannot_prove_a_status_page_that_lost_its_writes() {
    let quiet = adtb::OpenOptions {
        no_background_vacuum: true,
        ..api::options(2)
    };
    let open = |rt: &SimRuntime| {
        Database::open_in(rt.clone(), harness::NAME, &Log::new([APP]), &quiet).unwrap()
    };
    let mut sim = Sim::new(3, Config::default());
    let page = api::run(&mut sim, |rt| {
        let db = open(&rt);
        let txns = db.write_path().txns();
        let page = *txns
            .commit_log()
            .status_pages()
            .values()
            .next_back()
            .unwrap();
        db.close().unwrap();
        page
    });
    let before = sim.files();
    let segment = storage::Options::new(harness::NAME).segment_size;
    api::run(&mut sim, |rt| {
        let db = open(&rt);
        let pager = db.write_path().pager();
        let scratch = pager.allocate(1, PageKind::HeapSlotted, 7).unwrap();
        let pad = |to: u64| {
            while pager.appended().0 < to {
                let bump =
                    |p: &mut [&mut [u8]]| p[0][HEADER_LEN..].iter_mut().for_each(|b| *b ^= 1);
                pager.update(&[scratch], bump).unwrap();
                pager.flush_log();
            }
        };
        pad(segment);
        api::results(&db, "insert users { name: \"bo\" }");
        pad(2 * segment);
        db.close().unwrap();
    });
    let lost = reverted(sim.files(), &before, page, DB_PAGE);
    let mut sim = Sim::with_files(3, Config::default(), without_segment(lost, segment));
    api::run(&mut sim, |rt| {
        let db = open(&rt);
        let damage = db.write_path().txns().quarantined();
        assert!(
            matches!(damage, Some(PageError { pid, damage: Damage::LogUnproven { .. } }) if pid == page),
            "{damage:?}"
        );
        db.close().unwrap();
    });
    api::run(&mut sim, |rt| {
        let db = open(&rt);
        assert_eq!(quarantined(), api::failure(&db, "dump to \"d\""));
    });
}
