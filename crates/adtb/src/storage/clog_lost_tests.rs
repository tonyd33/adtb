//! A status page that lost the write of a logged commit: the open settles the id from its commit
//! record in the retained log, aborts only an id the log holds no commit record of, and writes
//! nothing when the log it needs is not whole.

use crate::testing::{Knobs, Object, Plant, Probe, TxnProbe};
use adtb_harness::process;
use std::collections::{BTreeMap, BTreeSet};

use crate::key::{Entry, Tid, Value};
use crate::mvcc::{CommitLog, Status, Transactions, Xid};
use crate::runtime::Runtime;
use crate::storage::{self, Damage, HEADER_LEN, Lsn, PageError, PageKind, Pager, Pid, Quarantine};
use crate::test_world as harness;
use crate::test_world::api;
use crate::test_world::load as open;
use crate::{Database, Error, FatalReason, Log};
use adtb_harness::common;
use adtb_harness::damage::{DataFile, Files};
use adtb_sim::{Config, Sim, SimRuntime};
use adtb_testkit::profile::assert_reached;

const APP: &str = r"module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
";
const DB_PAGE: usize = 8192;
const DB: DataFile = DataFile::new(harness::NAME, DB_PAGE);
const PAGER: DataFile = DataFile::new(harness::NAME, harness::PAGE);
const FRAMES: usize = 6;

/// Returns `files` with page `pid` of `data` set to its bytes in `from`.
fn reverted(mut files: Files, from: &Files, pid: Pid, data: DataFile) -> Files {
    data.replace(&mut files, pid, data.page(from, pid));
    files
}

/// Returns the status of each of `ids` in the commit log of `db`.
fn statuses(db: &Database<SimRuntime>, ids: &[Xid]) -> Vec<Status> {
    ids.iter()
        .map(|&x| db.testing().status(x).unwrap())
        .collect()
}

/// Flip every byte past the header of `page`, as one logged change.
fn bump(db: &Database<SimRuntime>, page: Pid) {
    let t = db.testing();
    let bytes = t.probe().page(page).unwrap()[HEADER_LEN..]
        .iter()
        .map(|b| b ^ 1)
        .collect();
    t.plant(Plant::Write {
        page,
        at: HEADER_LEN,
        bytes,
    })
    .unwrap();
}

/// Insert a user named `name` in one process. Returns the last status page and the ids the
/// process committed.
fn insert(sim: &mut Sim, name: &str) -> (Pid, Vec<Xid>) {
    process::run(sim, |rt| {
        let db = open::opened(&rt, harness::NAME, &Log::new([APP]), &open::options(2));
        let first = db.testing().next_xid();
        api::results(&db, &format!("insert users {{ name: \"{name}\" }}"));
        let ids: Vec<Xid> = (first.0..db.testing().next_xid().0).map(Xid).collect();
        let done = statuses(&db, &ids);
        let committed = ids.into_iter().zip(done);
        let committed = committed.filter_map(|(x, s)| (s == Status::Committed).then_some(x));
        let page = *db
            .testing()
            .commit_log()
            .status()
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
fn findings(rows: Vec<String>) -> Vec<(String, String)> {
    let row = |r: String| {
        let (ctor, rest) = r.split_once(" {").unwrap();
        let (_, at) = rest.split_once("where: ").unwrap();
        (ctor.to_owned(), at.trim_end_matches(" }").to_owned())
    };
    rows.into_iter().map(row).collect()
}

/// Returns the constructor and place of a row of a `verify` a task answers in its debug form.
fn task_finding(row: &str) -> (String, String) {
    const CTORS: [&str; 7] = [
        "Checksum",
        "StalePage",
        "Order",
        "Orphan",
        "Missing",
        "Violation",
        "Structure",
    ];
    let (ctor, rest) = row["Ctor(".len()..].split_once(", ").unwrap();
    let (_, at) = rest.rsplit_once("Text(").unwrap();
    let at = at.strip_suffix(")])").unwrap();
    (
        CTORS[ctor.parse::<usize>().unwrap()].to_owned(),
        at.to_owned(),
    )
}

/// Returns a simulation whose last status page lost the write of a logged commit, the ids of that
/// commit, read committed so far, and the findings of a verify of the file.
fn lost_write() -> (Sim, Vec<Xid>, Vec<(String, String)>) {
    lost_write_in(Sim::new(1, Config::default()))
}

/// Returns what [`lost_write`] does, from `sim`.
fn lost_write_in(mut sim: Sim) -> (Sim, Vec<Xid>, Vec<(String, String)>) {
    insert(&mut sim, "ann");
    let before = sim.files();
    let (page, bo) = insert(&mut sim, "bo");
    assert!(!bo.is_empty(), "the insert commits");
    let lsn = |f: &Files| storage::lsn(DB.page(f, page));
    let (expected, found) = (lsn(&sim.files()), lsn(&before));
    let lost = reverted(sim.files(), &before, page, DB);
    assert_ne!(lost, sim.files(), "the revert loses a write");
    let mut sim = Sim::with_files(1, Config::default(), lost);
    process::run(&mut sim, |rt| {
        let db = open::opened(&rt, harness::NAME, &Log::new([APP]), &open::options(2));
        let names = api::query(&db, "from u in app::users select u.name");
        assert_eq!(vec!["\"ann\"", "\"bo\""], names);
        assert_eq!(vec![Status::Committed; bo.len()], statuses(&db, &bo));
        db.close().unwrap();
    });
    let settled = format!(
        "\"page {}: Settled {{ expected: {expected:?}, found: {found:?} }}\"",
        page.0
    );
    let want = [("StalePage", settled.as_str())];
    let want = want.map(|(c, a)| (c.to_owned(), a.to_owned())).into();
    (sim, bo, want)
}

#[test]
fn a_settled_status_page_whose_log_is_recycled_is_a_finding_of_verify_that_marks_nothing() {
    let segment = 1 << 16;
    let options = storage::Options {
        page_size: DB_PAGE as u32,
        segment_size: segment,
        threads: 2,
        ..storage::Options::new(harness::NAME)
    };
    let mut sim = Sim::new(1, Config::default());
    process::run(&mut sim, |rt| Pager::create(rt, &options).unwrap().close());
    let (mut sim, bo, _) = lost_write_in(sim);
    let settled = process::run(&mut sim, |rt| {
        let db = open::opened(&rt, harness::NAME, &Log::new([APP]), &open::options(2));
        let probe = db.testing().probe();
        let recorded = probe.records().pages().clone();
        let settled = recorded
            .into_iter()
            .find(|(_, d)| matches!(d, Damage::Settled { .. }));
        let name = "x".repeat(2000);
        let (start, mut n) = (probe.appended(), 0);
        while probe.appended().0 - start.0 <= 3 * segment {
            api::results(&db, &format!("insert users {{ name: \"{name}{n}\" }}"));
            n += 1;
        }
        assert_eq!(BTreeSet::new(), probe.marks(), "the open and the commits");
        db.close().unwrap();
        settled
    });
    let recycled = process::run(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt, &options).unwrap();
        let recycled = pager.recycle_log();
        pager.close();
        recycled
    });
    assert_reached("segments recycled", recycled, 1);
    let Some((page, damage)) = settled else {
        panic!("the open settles a status page");
    };
    process::run(&mut sim, |rt| {
        let db = open::opened(&rt, harness::NAME, &Log::new([APP]), &open::options(2));
        assert_eq!(vec![Status::Committed; bo.len()], statuses(&db, &bo));
        let rows = findings(api::sorted(db.verify(None).unwrap()));
        let of_page = format!("\"page {}: ", page.0);
        let rows: Vec<_> = rows
            .into_iter()
            .filter(|(_, a)| a.starts_with(&of_page))
            .collect();
        let want = ("StalePage".to_owned(), format!("{of_page}{damage:?}\""));
        assert_eq!(vec![want], rows);
        assert_eq!(BTreeSet::new(), db.testing().probe().marks());
        db.close().unwrap();
    });
}

/// Assert a reopen of `sim` reads `bo` committed and refuses a query as quarantined whole.
/// Assert a reopen of `sim` reads `bo` committed, keeps `marks` and serves the relation.
fn served_on_reopen(sim: &mut Sim, bo: &[Xid], marks: &BTreeSet<Quarantine>) {
    process::run(sim, |rt| {
        let db = open::opened(&rt, harness::NAME, &Log::new([APP]), &open::options(2));
        assert_eq!(vec![Status::Committed; bo.len()], statuses(&db, bo));
        assert_eq!(*marks, db.testing().probe().marks());
        let names = api::query(&db, "from u in app::users select u.name");
        assert_eq!(vec!["\"ann\"", "\"bo\""], names);
    });
}

#[test]
fn the_first_verify_of_a_settled_status_page_reports_its_lost_write_once_and_marks_nothing() {
    let (mut sim, bo, want) = lost_write();
    process::run(&mut sim, |rt| {
        let db = open::opened(&rt, harness::NAME, &Log::new([APP]), &open::options(2));
        assert_eq!(want, findings(api::sorted(db.verify(None).unwrap())));
        assert_eq!(BTreeSet::new(), db.testing().probe().marks());
        let names = api::query(&db, "from u in app::users select u.name");
        assert_eq!(vec!["\"ann\"", "\"bo\""], names);
        db.close().unwrap();
    });
    served_on_reopen(&mut sim, &bo, &BTreeSet::new());
}

#[test]
fn a_lost_write_verify_finds_on_a_settled_status_page_at_other_lsns_marks_the_database() {
    let (mut sim, _, _) = lost_write();
    process::run(&mut sim, |rt| {
        let db = open::opened(&rt, harness::NAME, &Log::new([APP]), &open::options(2));
        let t = db.testing();
        let recorded = t.probe().records().pages().clone();
        let settled = recorded
            .into_iter()
            .find(|(_, d)| matches!(d, Damage::Settled { .. }));
        let (page, _) = settled.expect("the open settles a status page");
        let mut io = rt.io();
        let (file, _) = common::open(&rt, &mut io, harness::NAME, false).unwrap();
        let at = DB.range(page).start as u64;
        t.flush();
        t.checkpoint();
        let mut copies = Vec::new();
        for name in ["cy", "dee"] {
            api::results(&db, &format!("insert users {{ name: \"{name}\" }}"));
            t.flush();
            copies.push(common::read(&rt, &mut io, file, at, DB_PAGE).unwrap());
        }
        let (found, expected) = (storage::lsn(&copies[0]), storage::lsn(&copies[1]));
        assert!(
            found < expected,
            "the second insert rewrites the status page"
        );
        common::write(&rt, &mut io, file, at, &copies[0]).unwrap();
        common::sync(&rt, &mut io, file).unwrap();
        for p in 0..4096 {
            t.probe()
                .page(Pid(1 << 16 | p))
                .expect("a page never written");
        }
        let stale = Damage::Stale { expected, found };
        assert!(matches!(
            t.probe().records().pages()[&page],
            Damage::Settled { .. }
        ));
        let checked = t.check_page(page);
        let finding = format!("Page {{ pid: {page:?}, damage: {stale:?} }}");
        assert!(format!("{checked:?}").contains(&finding), "{checked:?}");
        let rows = findings(api::sorted(db.verify(None).unwrap()));
        let row = (
            "StalePage".to_owned(),
            format!("\"page {}: {stale:?}\"", page.0),
        );
        assert!(rows.contains(&row), "{rows:?}");
        let database = BTreeSet::from([Quarantine::Database]);
        assert_eq!(database, t.probe().marks());
        let names = api::failure(&db, "from u in app::users select u.name");
        assert_eq!(quarantined(), names);
        db.close().unwrap();
    });
}

#[test]
fn verify_of_one_relation_beside_a_settled_status_page_marks_nothing() {
    let (mut sim, bo, _) = lost_write();
    process::run(&mut sim, |rt| {
        let db = open::opened(&rt, harness::NAME, &Log::new([APP]), &open::options(2));
        let found = db.verify(Some("app::users")).unwrap();
        assert_eq!(Vec::<(String, String)>::new(), findings(api::sorted(found)));
        assert_eq!(BTreeSet::new(), db.testing().probe().marks());
        db.close().unwrap();
    });
    served_on_reopen(&mut sim, &bo, &BTreeSet::new());
}

#[test]
fn an_index_finding_beside_a_settled_status_page_marks_only_its_index() {
    let (mut sim, bo, _) = lost_write();
    let index = process::run(&mut sim, |rt| {
        let db = open::opened(&rt, harness::NAME, &Log::new([APP]), &open::options(2));
        let t = db.testing();
        let ix = t.index("app::users", "key id").unwrap();
        let page = t.pages(t.heap("app::users", "User").unwrap()).unwrap()[0];
        let orphan = Entry {
            key: vec![Value::I64(1000)],
            ctor: *t.ctors(ix).unwrap().iter().next().unwrap(),
            tid: Tid::new(page.0, 400).unwrap(),
        };
        t.plant(Plant::AddEntry {
            index: ix,
            entry: orphan,
        })
        .unwrap();
        let found = findings(api::sorted(db.verify(Some("app::users")).unwrap()));
        let at = format!(
            "\"page {} slot 400: holds no version of the entry's key\"",
            page.0
        );
        assert_eq!(vec![("Orphan".to_owned(), at)], found);
        let index = BTreeSet::from([t.mark_of(ix).unwrap()]);
        assert_eq!(index, db.testing().probe().marks());
        db.close().unwrap();
        index
    });
    served_on_reopen(&mut sim, &bo, &index);
}

#[test]
fn verify_meeting_an_unreadable_status_page_fails_and_marks_nothing() {
    let mut sim = Sim::new(3, Config::default());
    let (status, _) = insert(&mut sim, "ann");
    process::run(&mut sim, |rt| {
        let db = open::opened(&rt, harness::NAME, &Log::new([APP]), &open::options(2));
        db.testing()
            .plant(Plant::Refuse {
                page: status,
                damage: Damage::Unreadable,
            })
            .unwrap();
        let unreadable = PageError {
            pid: status,
            damage: Damage::Unreadable,
        };
        let detail = format!("{unreadable:?}");
        let want = Error::Fatal {
            reason: FatalReason::Storage { detail },
        };
        assert_eq!(Some(want), db.verify(None).err());
        assert_eq!(BTreeSet::new(), db.testing().probe().marks());
        db.close().unwrap();
    });
}

#[test]
fn the_first_verify_through_a_server_of_a_settled_status_page_reports_it_once() {
    let (mut sim, bo, want) = lost_write();
    process::run(&mut sim, |rt| {
        let db = open::opened(&rt, harness::NAME, &Log::new([APP]), &open::options(2));
        let owner = db.role_authority().session("owner").unwrap();
        let rows = api::task_rows(&rt, &owner, "verify").unwrap();
        assert_eq!(
            want,
            rows.iter().map(|r| task_finding(r)).collect::<Vec<_>>()
        );
        assert_eq!(BTreeSet::new(), db.testing().probe().marks());
        api::task(&rt, &owner, "app::users").unwrap();
        db.close().unwrap();
    });
    served_on_reopen(&mut sim, &bo, &BTreeSet::new());
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
        End::Close => process::finished(sim, |rt| body(&rt)),
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
        process::finished(&mut sim, |rt| {
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
            let files = reverted(sim.files(), &before, page, PAGER);
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
    process::finished(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &options).unwrap();
        let txns = Transactions::create(rt, &pager).unwrap();
        let scratch = pager.allocate(1, PageKind::HeapSlotted, 7).unwrap();
        let x = txns.begin(&pager).unwrap();
        txns.commit(&pager, x).unwrap();
        while Probe::of(&pager).appended().0 <= options.segment_size {
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
    process::finished(&mut sim, |rt| {
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
    (reverted(sim.files(), &before, page, PAGER), root, page, ids)
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
    process::finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &harness::options(FRAMES)).unwrap();
        assert_eq!(Some(damage), CommitLog::open(&pager, root).err());
        let txns = Transactions::open(rt, &pager, root).unwrap();
        assert_eq!(Some(damage), TxnProbe::of(&txns).commit_log_damage());
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
    let quiet = crate::OpenOptions {
        testing: Knobs::without_background_vacuum(),
        ..open::options(2)
    };
    let open = |rt: &SimRuntime| {
        Database::open_in(rt.clone(), harness::NAME, &Log::new([APP]), &quiet).unwrap()
    };
    let mut sim = Sim::new(3, Config::default());
    let page = process::run(&mut sim, |rt| {
        let db = open(&rt);
        let page = *db
            .testing()
            .commit_log()
            .status()
            .values()
            .next_back()
            .unwrap();
        db.close().unwrap();
        page
    });
    let before = sim.files();
    let segment = storage::Options::new(harness::NAME).segment_size;
    process::run(&mut sim, |rt| {
        let db = open(&rt);
        let t = db.testing();
        let scratch = t
            .plant(Plant::Orphan {
                kind: PageKind::HeapSlotted,
                owner: 7,
            })
            .unwrap()
            .page()
            .unwrap();
        let pad = |to: u64| {
            while t.probe().appended().0 < to {
                bump(&db, scratch);
                t.flush_log();
            }
        };
        pad(segment);
        api::results(&db, "insert users { name: \"bo\" }");
        pad(2 * segment);
        db.close().unwrap();
    });
    let lost = reverted(sim.files(), &before, page, DB);
    let mut sim = Sim::with_files(3, Config::default(), without_segment(lost, segment));
    process::run(&mut sim, |rt| {
        let db = open(&rt);
        let damage = db.testing().commit_log_damage();
        assert!(
            matches!(damage, Some(PageError { pid, damage: Damage::LogUnproven { .. } }) if pid == page),
            "{damage:?}"
        );
        db.close().unwrap();
    });
    process::run(&mut sim, |rt| {
        let db = open(&rt);
        assert_eq!(quarantined(), api::failure(&db, "dump to \"d\""));
    });
}

/// Returns what each page of the commit log, the sequence counters and `app::users` is, and the
/// mark its damage takes.
fn kinds(db: &Database<SimRuntime>) -> BTreeMap<Pid, (&'static str, Quarantine)> {
    let t = db.testing();
    let clog = t.commit_log();
    let mut kinds = BTreeMap::new();
    kinds.insert(clog.root(), ("root", Quarantine::Database));
    let counters = t.root(Object::Sequences).unwrap();
    kinds.insert(counters, ("counters", Quarantine::Database));
    for &p in clog.status().values() {
        kinds.insert(p, ("status", Quarantine::Database));
    }
    for &p in clog.directory() {
        kinds.insert(p, ("directory", Quarantine::Database));
    }
    let heap = t.heap("app::users", "User").unwrap();
    for p in t.owned(heap).unwrap() {
        kinds.insert(p, ("heap", t.mark_of(heap).unwrap()));
    }
    let index = t.index("app::users", "key id").unwrap();
    for p in t.pages(index).unwrap() {
        kinds.insert(p, ("index", t.mark_of(index).unwrap()));
    }
    kinds
}

/// Revert each page `work` changes in the files of `sim`, and assert its next read is refused
/// and the first verify reports it and marks its holder. A status page is settled by the open of
/// the commit log, read after it, reported settled and marks nothing. Returns how many pages of
/// each kind it reverted.
fn each_lost_write_refused(
    sim: &mut Sim,
    work: impl FnOnce(&Database<SimRuntime>),
) -> BTreeMap<&'static str, usize> {
    let before = sim.files();
    let kinds = process::run(sim, |rt| {
        let db = open::opened(&rt, harness::NAME, &Log::new([APP]), &open::options(2));
        work(&db);
        let kinds = kinds(&db);
        db.close().unwrap();
        kinds
    });
    let after = sim.files();
    let bytes = |f: &Files, p: Pid| DB.page(f, p).to_vec();
    let lsn = |f: &Files, p: Pid| storage::lsn(&bytes(f, p));
    let pages = (before[harness::NAME].len() / DB_PAGE) as u64;
    let changed = (Pid::FIRST_DATA.0..pages).map(Pid);
    let changed = changed.filter(|&p| lsn(&before, p) != lsn(&after, p));
    let mut reached = BTreeMap::new();
    for page in changed {
        let by_kind = match storage::kind(&bytes(&after, page)) {
            k if k == PageKind::AllocationBitmap as u16 => "bitmap",
            k if k == PageKind::Quarantine as u16 => "marks",
            _ => "other",
        };
        let (kind, mark) = *kinds.get(&page).unwrap_or(&(by_kind, Quarantine::Database));
        *reached.entry(kind).or_insert(0) += 1;
        let lost = reverted(after.clone(), &before, page, DB);
        let settled = kind == "status";
        let stale = format!(
            "\"page {}: {} {{ expected: {:?}, found: {:?} }}\"",
            page.0,
            if settled { "Settled" } else { "Stale" },
            lsn(&after, page),
            lsn(&before, page)
        );
        let damage = Damage::Stale {
            expected: lsn(&after, page),
            found: lsn(&before, page),
        };
        let read = process::run(
            &mut Sim::with_files(2, Config::default(), lost.clone()),
            |rt| {
                let db = open::opened(&rt, harness::NAME, &Log::new([APP]), &open::options(2));
                let read = db.testing().probe().page(page).map(drop);
                db.close().unwrap();
                read
            },
        );
        let refused = if kind == "status" {
            Ok(())
        } else {
            Err(PageError { pid: page, damage })
        };
        assert_eq!(refused, read, "{kind} page {page:?}");
        process::run(&mut Sim::with_files(2, Config::default(), lost), |rt| {
            let db = open::opened(&rt, harness::NAME, &Log::new([APP]), &open::options(2));
            let found = findings(api::sorted(db.verify(None).unwrap()));
            let want = ("StalePage".to_owned(), stale.clone());
            assert!(found.contains(&want), "{kind} page {page:?}: {found:?}");
            let marks = db.testing().probe().marks();
            let want = if settled {
                BTreeSet::new()
            } else {
                BTreeSet::from([mark])
            };
            assert_eq!(want, marks, "{kind} page {page:?}");
            db.close().unwrap();
        });
    }
    reached
}

#[test]
fn a_lost_write_of_each_page_an_insert_changes_is_refused_on_read_and_reported_by_verify() {
    let mut sim = Sim::new(2, Config::default());
    insert(&mut sim, "ann");
    let reached = each_lost_write_refused(&mut sim, |db| {
        api::results(db, r#"insert users { name: "bo" }"#);
    });
    let want = [
        ("counters", 1),
        ("heap", 2),
        ("index", 1),
        ("root", 1),
        ("status", 1),
    ];
    assert_eq!(BTreeMap::from(want), reached);
}

#[test]
fn a_lost_write_of_each_page_an_allocating_insert_and_a_mark_change_is_refused_on_read() {
    let mut sim = Sim::new(2, Config::default());
    insert(&mut sim, "ann");
    let reached = each_lost_write_refused(&mut sim, |db| {
        let digit = "[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]";
        api::results(
            db,
            &format!(
                "insert users from (from a in {digit}, b in {digit}, c in {digit} select {{ name: \"n\" }})"
            ),
        );
        let t = db.testing();
        t.plant(Plant::Mark(Quarantine::Index(u32::MAX))).unwrap();
        t.plant(Plant::Unmark(Quarantine::Index(u32::MAX))).unwrap();
    });
    assert_eq!(Some(&1), reached.get("marks"), "{reached:?}");
    assert!(
        reached.get("bitmap").is_some_and(|&n| n >= 1),
        "{reached:?}"
    );
}

#[test]
fn an_open_whose_log_cannot_settle_a_status_page_writes_no_mark() {
    let options = crate::OpenOptions {
        create: Some(crate::CreateOptions::default()),
        testing: Knobs::default().with_segment_size(64 << 10),
        ..open::options(2)
    };
    let open =
        |rt: SimRuntime| Database::open_in(rt, harness::NAME, &Log::new([APP]), &options).unwrap();
    let mut sim = Sim::new(6, Config::default());
    let page = process::run(&mut sim, |rt| {
        let db = open(rt);
        api::results(&db, r#"insert users { name: "ann" }"#);
        let page = *db
            .testing()
            .commit_log()
            .status()
            .values()
            .next_back()
            .unwrap();
        db.close().unwrap();
        page
    });
    let before = sim.files();
    process::run(&mut sim, |rt| {
        let db = open(rt);
        for i in 0..200 {
            api::results(&db, &format!(r#"insert users {{ name: "{i}" }}"#));
        }
        db.close().unwrap();
    });
    let after = sim.files();
    let old = storage::lsn(DB.page(&before, page));
    let segments: Vec<(&String, u64)> = after
        .keys()
        .filter_map(|k| Some((k, u64::from_str_radix(k.rsplit_once('/')?.1, 16).ok()?)))
        .collect();
    let (gap, base) = segments[..segments.len() - 1]
        .iter()
        .find(|(_, base)| *base > old.0)
        .copied()
        .expect("a segment after the status page's old write and before the last");
    let mut lost = reverted(after.clone(), &before, page, DB);
    lost.remove(gap).unwrap();
    let mut sim = Sim::with_files(6, Config::default(), lost);
    process::run(&mut sim, |rt| {
        let db = open(rt);
        let found = db
            .testing()
            .commit_log_damage()
            .expect("the commit log is quarantined");
        assert_eq!(page, found.pid);
        let Damage::LogUnproven { at } = found.damage else {
            panic!("{found:?}");
        };
        assert!(
            old < at && at.0 <= base,
            "{at:?} between {old:?} and {base}"
        );
        assert_eq!(
            BTreeSet::from([Quarantine::Database]),
            db.testing().probe().marks()
        );
        db.close().unwrap();
    });
    let mut restored = reverted(sim.files(), &after, page, DB);
    restored.insert(gap.clone(), after[gap].clone());
    let mut sim = Sim::with_files(6, Config::default(), restored);
    let marks = process::run(&mut sim, |rt| {
        let db = open(rt);
        let marks = db.testing().probe().marks();
        db.close().unwrap();
        marks
    });
    assert_eq!(BTreeSet::new(), marks);
}

const ARCHIVE: &str = "store";

/// How the second process of [`resolved_then_lost`] resolves its id.
#[derive(Clone, Copy, Debug)]
enum Resolve {
    Commit,
    Abort,
}

/// A database whose last status page lost the write of an id the log resolved.
struct Lost {
    files: Files,
    page: Pid,
    ids: Vec<Xid>,
    stale: Damage,
}

fn engine_options(archived: bool) -> crate::OpenOptions {
    crate::OpenOptions {
        create: Some(crate::CreateOptions::default()),
        log_archive: archived.then(|| ARCHIVE.into()),
        testing: Knobs::without_background_vacuum().with_segment_size(64 << 10),
        ..open::options(2)
    }
}

/// Returns the base of each log segment of `files`.
fn segment_bases(files: &Files) -> Vec<u64> {
    let dir = format!("{}/", storage::Options::new(harness::NAME).log_dir());
    let names = files.keys().filter_map(|k| k.strip_prefix(&dir));
    names.map(|n| u64::from_str_radix(n, 16).unwrap()).collect()
}

/// Returns a database whose first process inserts a row, whose second resolves ids as `resolve`
/// says, inserting a row for a commit, then logs past three segments, and, when `recycled`, whose
/// third open recycles the log of the second; with every write of the last status page after the
/// first process lost, and an archive set when `archived`.
fn resolved_then_lost(resolve: Resolve, recycled: bool, archived: bool) -> Lost {
    let options = engine_options(archived);
    let open =
        |rt: SimRuntime| Database::open_in(rt, harness::NAME, &Log::new([APP]), &options).unwrap();
    let root = Files::from([(format!("{ARCHIVE}/keep"), Vec::new())]);
    let mut sim = Sim::with_files(7, Config::default(), root);
    let page = process::run(&mut sim, |rt| {
        let db = open(rt);
        api::results(&db, r#"insert users { name: "ann" }"#);
        let pages = db.testing().commit_log().status().clone();
        db.close().unwrap();
        *pages.values().next_back().unwrap()
    });
    let before = sim.files();
    let (ids, resolved) = process::run(&mut sim, |rt| {
        let db = open(rt);
        let t = db.testing();
        let ids = match resolve {
            Resolve::Commit => {
                let first = t.next_xid();
                api::results(&db, r#"insert users { name: "bo" }"#);
                let ids: Vec<Xid> = (first.0..t.next_xid().0).map(Xid).collect();
                let done = statuses(&db, &ids).into_iter().zip(ids);
                done.filter_map(|(s, x)| (s == Status::Committed).then_some(x))
                    .collect()
            }
            Resolve::Abort => vec![t.take_xid().unwrap()],
        };
        let resolved = t.probe().appended();
        let segment = 64 << 10;
        let scratch = t
            .plant(Plant::Orphan {
                kind: PageKind::HeapSlotted,
                owner: 7,
            })
            .unwrap()
            .page()
            .unwrap();
        while t.probe().appended().0 < resolved.0 + 3 * segment {
            bump(&db, scratch);
            t.flush_log();
        }
        db.close().unwrap();
        (ids, resolved)
    });
    assert!(
        !ids.is_empty(),
        "{resolve:?}: the second process resolves an id"
    );
    if recycled {
        process::run(&mut sim, |rt| open(rt).close().unwrap());
        let first = segment_bases(&sim.files())[0];
        assert!(
            first >= resolved.0,
            "{resolve:?}, archived {archived}: the recycle removes the resolution, first segment {first}"
        );
    }
    let lsn = |f: &Files| storage::lsn(DB.page(f, page));
    let stale = Damage::Stale {
        expected: lsn(&sim.files()),
        found: lsn(&before),
    };
    let files = reverted(sim.files(), &before, page, DB);
    Lost {
        files,
        page,
        ids,
        stale,
    }
}

/// Assert an open of a status page that lost the write of an id resolved as `resolve`, whose log
/// is recycled, records the page's damage and quarantines the database, at that open and the next.
fn refused_unproven(resolve: Resolve, archived: bool) {
    let lost = resolved_then_lost(resolve, true, archived);
    let options = engine_options(archived);
    let damage = PageError {
        pid: lost.page,
        damage: lost.stale,
    };
    let whole = BTreeSet::from([Quarantine::Database]);
    let mut sim = Sim::with_files(7, Config::default(), lost.files);
    for open in 0..2 {
        process::run(&mut sim, |rt| {
            let db = Database::open_in(rt, harness::NAME, &Log::new([APP]), &options).unwrap();
            let at = format!("{resolve:?}, archived {archived}, open {open}");
            assert_eq!(Some(damage), db.testing().commit_log_damage(), "{at}");
            assert_eq!(
                Some(&lost.stale),
                db.testing().probe().records().pages().get(&lost.page),
                "{at}"
            );
            assert_eq!(whole, db.testing().probe().marks(), "{at}");
            let names = api::failure(&db, "from u in app::users select u.name");
            assert_eq!(quarantined(), names, "{at}");
            db.close().unwrap();
        });
    }
}

#[test]
fn a_status_page_that_lost_a_commit_whose_record_is_recycled_quarantines_the_database() {
    refused_unproven(Resolve::Commit, false);
}

#[test]
fn a_status_page_that_lost_a_commit_whose_record_is_recycled_past_an_archive_quarantines_the_database()
 {
    refused_unproven(Resolve::Commit, true);
}

#[test]
fn a_status_page_that_lost_an_abort_whose_record_is_recycled_quarantines_the_database() {
    refused_unproven(Resolve::Abort, false);
}

#[test]
fn a_status_page_that_lost_a_commit_whose_record_is_retained_settles_it_committed() {
    let lost = resolved_then_lost(Resolve::Commit, false, false);
    let mut sim = Sim::with_files(7, Config::default(), lost.files);
    process::run(&mut sim, |rt| {
        let db = open::opened(&rt, harness::NAME, &Log::new([APP]), &open::options(2));
        assert_eq!(
            vec![Status::Committed; lost.ids.len()],
            statuses(&db, &lost.ids)
        );
        let Damage::Stale { expected, found } = lost.stale else {
            panic!("{:?} is a lost write", lost.stale);
        };
        let settled = Damage::Settled { expected, found };
        assert_eq!(
            Some(&settled),
            db.testing().probe().records().pages().get(&lost.page)
        );
        assert_eq!(BTreeSet::new(), db.testing().probe().marks());
        let names = api::query(&db, "from u in app::users select u.name");
        assert_eq!(vec!["\"ann\"", "\"bo\""], names);
        db.close().unwrap();
    });
}

#[test]
fn a_status_page_reverted_to_an_older_page_at_its_pid_is_refused() {
    let options = harness::options(FRAMES);
    let mut sim = Sim::new(8, Config::default());
    let mut first = None;
    process::finished(&mut sim, |rt| {
        let pager = Pager::create(rt, &options).unwrap();
        let mut clog = CommitLog::create(&pager).unwrap();
        let per = clog.ids_per_page();
        clog.reserve(&pager, Xid(per), Xid::FIRST).unwrap();
        for x in 1..per {
            clog.abort(&pager, Xid(x)).unwrap();
        }
        clog.reserve(&pager, Xid(2 * per), Xid(per)).unwrap();
        let old = clog.status_pages()[&0];
        clog.truncate(&pager, Xid(per)).unwrap();
        first = Some((clog.root(), old, per));
        pager.close();
    });
    let (root, old, per) = first.unwrap();
    let before = sim.files();
    let committed = Xid(2 * per + 5);
    process::finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt, &options).unwrap();
        let mut clog = CommitLog::open(&pager, root).unwrap();
        clog.reserve(&pager, Xid(3 * per), Xid(per)).unwrap();
        assert_eq!(
            old,
            clog.status_pages()[&2],
            "the new status page reuses the freed pid"
        );
        clog.commit(&pager, committed).unwrap();
        clog.reserve(&pager, Xid(4 * per), Xid(per)).unwrap();
        pager.close();
    });
    let lsn = |f: &Files| storage::lsn(PAGER.page(f, old));
    let damage = PageError {
        pid: old,
        damage: Damage::Stale {
            expected: lsn(&sim.files()),
            found: lsn(&before),
        },
    };
    let files = reverted(sim.files(), &before, old, PAGER);
    let mut sim = Sim::with_files(8, Config::default(), files);
    process::finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &options).unwrap();
        assert_eq!(Some(damage), CommitLog::open(&pager, root).err());
        let txns = Transactions::open(rt, &pager, root).unwrap();
        assert_eq!(Some(damage), TxnProbe::of(&txns).commit_log_damage());
        assert_eq!(
            Some(&damage.damage),
            Probe::of(&pager).records().pages().get(&old)
        );
        let intact = Xid(3 * per + 1);
        let status = txns.commit_log().status(&pager, intact);
        assert_eq!(Ok(Status::InProgress), status, "{intact:?} on page 3");
    });
}
