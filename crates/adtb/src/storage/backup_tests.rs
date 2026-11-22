//! Full backups of a database under writers, restores of them to new files, and the test restore
//! that proves one.

use crate::testing::Plant;
use adtb_testkit::process;
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::runtime::{Done, IoError, Op, OpKind, Runtime, Threads};
use crate::storage::{
    self, BackupError, BackupRun, Damage, Envelope, HEADER_LEN, Lsn, Manifest, Options, PageError,
    PageKind, Pager, Pid,
};
use crate::test_world::api;
use crate::test_world::load as open;
use crate::{
    ArchiveTarget, CallOptions, CancelToken, Capability, Database, Determinism, Error, FatalReason,
    HostFns, Log, OpenOptions, Position, Proof, RejectReason, RestorePoint, TxnCallOptions,
    TxnOptions, proof_in, prove_in, restore_in,
};
use adtb_sim::{Action, Config, Match, Plan, Sim, SimRuntime, Struck};
use adtb_sys::runtime::Faults;
use adtb_testkit::common;
use adtb_testkit::damage::DataFile;
use adtb_testkit::profile::{assert_reached, every_case, sweep};
use adtb_testkit::search::{self, Files};

const FILE: &str = "backup.adt";
const TARGET: &str = "bk";
const COPY: &str = "copy.adt";
const DB: DataFile = DataFile::new(FILE, 8192);
const APP: &str = r"module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
";

fn options() -> OpenOptions {
    open::options(2)
}

fn no_create() -> OpenOptions {
    OpenOptions {
        create: None,
        ..open::options(2)
    }
}

fn insert_text(id: i64) -> String {
    format!(r#"insert users {{ id: users.key({id}), name: "user {id}" }}"#)
}

fn insert(db: &Database<SimRuntime>, ids: std::ops::Range<i64>) {
    for id in ids {
        api::results(db, &insert_text(id));
    }
}

/// Commit the insert of `id` and return its position.
fn commit(db: &Database<SimRuntime>, id: i64) -> Position {
    let mut t = db.begin(&TxnOptions::default()).unwrap();
    t.query_text(&insert_text(id), &[], &TxnCallOptions::default())
        .unwrap();
    t.commit(None).unwrap()
}

fn ids(db: &Database<SimRuntime>) -> BTreeSet<i64> {
    api::query(db, "from u in users select u.id")
        .iter()
        .map(|row| {
            let digits: String = row.chars().filter(char::is_ascii_digit).collect();
            digits.parse().unwrap()
        })
        .collect()
}

fn envelope() -> Envelope {
    Envelope {
        release: "test".into(),
        edition: "2026-10".into(),
        schema_version: 1,
        schema_hash: "00".into(),
        catalog_version: 1,
    }
}

/// Back `db` up to `target`, calling `visit` with each page as it is first read.
fn db_backup<R: Runtime>(
    db: &Database<R>,
    target: &str,
    visit: &mut dyn FnMut(Pid, &mut [u8]) -> bool,
) -> Result<Manifest, BackupError> {
    db.testing().backup(target, envelope(), visit)
}

/// Back `pager` up to `target`, calling `visit` with each page as it is first read.
fn backup_with<R: Runtime>(
    pager: &Pager<R>,
    target: &str,
    visit: &mut dyn FnMut(Pid, &mut [u8]) -> bool,
) -> Result<Manifest, BackupError> {
    pager.backup(&mut BackupRun {
        target,
        envelope: envelope(),
        visit,
        name: &|q| format!("{q:?}"),
    })
}

fn archive() -> ArchiveTarget {
    ArchiveTarget::path(TARGET).unwrap()
}

fn restore(rt: &SimRuntime) -> Result<(), Error> {
    restore_in(
        rt.clone(),
        &archive(),
        COPY,
        RestorePoint::Latest,
        &no_create(),
    )
    .map(|_| ())
}

fn backup_text(db: &Database<SimRuntime>) -> Result<(), Error> {
    db.query_text(
        &format!("backup to \"{TARGET}\""),
        &[],
        &CallOptions::default(),
    )
    .map(|_| ())
}

/// Returns a file with `rows` users, closed cleanly, and a backup of it in `bk`.
fn backed_up(rows: i64) -> Files {
    let mut sim = Sim::new(1, Config::default());
    process::run(&mut sim, |rt| {
        let db = open::opened(&rt, FILE, &Log::new([APP]), &options());
        insert(&db, 0..rows);
        backup_text(&db).unwrap();
        db.close().unwrap();
    });
    sim.files()
}

fn restored_rows(files: Files) -> Result<BTreeSet<i64>, Error> {
    let mut sim = Sim::with_files(2, Config::default(), files);
    process::run(&mut sim, |rt| {
        restore(&rt)?;
        let db = open::open_in(&rt, COPY, &Log::new([APP]), &no_create()).unwrap();
        let rows = ids(&db);
        db.close().unwrap();
        Ok(rows)
    })
}

fn invalid(detail: &str) -> Result<(), Error> {
    Err(Error::Rejected {
        reason: RejectReason::Invalid {
            detail: detail.into(),
        },
    })
}

fn occupied() -> Result<(), Error> {
    invalid(&format!("the backup target `{TARGET}` holds files"))
}

fn exists(path: &str) -> Result<(), Error> {
    invalid(&format!("the restore path `{path}` exists"))
}

fn file_with_rows(rows: i64) -> Files {
    let mut sim = Sim::new(1, Config::default());
    process::run(&mut sim, |rt| {
        let db = open::opened(&rt, FILE, &Log::new([APP]), &options());
        insert(&db, 0..rows);
        db.close().unwrap();
    });
    sim.files()
}

/// Back up with `backup to` while a writer commits on another thread: the restore holds exactly the
/// commits acknowledged below the backup's end, and the writer committed during the backup and
/// past its end.
#[test]
fn a_backup_under_a_writer_restores_exactly_the_commits_acked_below_its_end() {
    let (mut during, mut past) = (0, 0);
    for seed in sweep(0..2) {
        let acked: Arc<Mutex<Vec<(i64, Position)>>> = Arc::default();
        let inside = Arc::new(AtomicU64::new(0));
        let mut sim = Sim::new(seed, Config::default());
        let (a, counted) = (acked.clone(), inside.clone());
        process::run(&mut sim, move |rt| {
            let db = open::opened(&rt, FILE, &Log::new([APP]), &options());
            let before: Vec<_> = (0..16).map(|id| (id, commit(&db, id))).collect();
            a.lock().unwrap().extend(before);
            let (copying, stop) = (
                Arc::new(AtomicBool::new(false)),
                Arc::new(AtomicBool::new(false)),
            );
            let (w, c, s, shared, clock) = (
                db.clone(),
                copying.clone(),
                stop.clone(),
                a.clone(),
                rt.clone(),
            );
            let writer = rt.spawn("writer", move || {
                let mut id = 16;
                while !s.load(Ordering::SeqCst) {
                    let began = c.load(Ordering::SeqCst);
                    let at = commit(&w, id);
                    shared.lock().unwrap().push((id, at));
                    if began && c.load(Ordering::SeqCst) {
                        counted.fetch_add(1, Ordering::SeqCst);
                    }
                    id += 1;
                    clock.yield_now();
                }
            });
            copying.store(true, Ordering::SeqCst);
            backup_text(&db).unwrap();
            copying.store(false, Ordering::SeqCst);
            stop.store(true, Ordering::SeqCst);
            rt.join(writer);
            a.lock().unwrap().push((1000, commit(&db, 1000)));
            db.close().unwrap();
        });
        let mut restored = Sim::with_files(seed, Config::default(), sim.files());
        let (end, rows) = process::run(&mut restored, |rt| {
            let (manifest, _) = storage::read_manifest(&rt, TARGET).unwrap();
            restore(&rt).unwrap();
            let db = open::open_in(&rt, COPY, &Log::new([APP]), &no_create()).unwrap();
            let rows = ids(&db);
            db.close().unwrap();
            (manifest.end, rows)
        });
        let acked = acked.lock().unwrap().clone();
        let below: BTreeSet<i64> = acked
            .iter()
            .filter(|(_, at)| at.lsn < end.lsn.0)
            .map(|(id, _)| *id)
            .collect();
        assert_eq!(below, rows, "seed {seed}: end {end:?}");
        during += inside.load(Ordering::SeqCst) as usize;
        past += usize::from(below.len() < acked.len());
    }
    assert_reached("a commit acknowledged while the backup copied", during, 1);
    assert_reached("a commit acknowledged past the backup's end", past, 1);
}

#[test]
fn restored_verify_clean_all_checks() {
    let mut sim = Sim::with_files(6, Config::default(), backed_up(40));
    process::run(&mut sim, |rt| {
        assert_eq!(
            Proof::Proven,
            prove_in(rt.clone(), &archive(), &Log::new([APP]), &no_create()).unwrap()
        );
        restore(&rt).unwrap();
        let db = open::open_in(&rt, COPY, &Log::new([APP]), &no_create()).unwrap();
        assert_eq!(0, db.testing().verify_holding().unwrap().count());
        let quiet = db.testing().hold_vacuum();
        assert!(db.testing().verify_pages().findings.is_empty());
        drop(quiet);
        db.close().unwrap();
    });
}

/// A proof opens with the file's own log, given no migrations.
#[test]
fn prove_reads_the_schema_from_the_file_without_the_callers_log() {
    let mut sim = Sim::with_files(6, Config::default(), backed_up(4));
    process::run(&mut sim, |rt| {
        assert_eq!(
            Proof::Proven,
            prove_in(
                rt,
                &archive(),
                &Log::new(Vec::<String>::new()),
                &no_create()
            )
            .unwrap()
        );
    });
}

#[test]
fn page_torn_mid_copy_retried_under_latch() {
    let mut sim = Sim::new(8, Config::default());
    let mut torn = 0;
    let mut got = None;
    process::run(&mut sim, |rt| {
        let db = open::opened(&rt, FILE, &Log::new([APP]), &options());
        insert(&db, 0..8);
        got = Some(
            db_backup(&db, TARGET, &mut |_, page| {
                if !page.is_empty() && torn == 0 && page.iter().any(|&b| b != 0) {
                    page[100] ^= 0xff;
                    torn += 1;
                }
                true
            })
            .unwrap(),
        );
        db.close().unwrap();
    });
    assert_eq!(1, torn);
    assert_eq!(Vec::<Pid>::new(), got.unwrap().damaged);
    assert_eq!(8, restored_rows(sim.files()).unwrap().len());
}

/// Flip a byte of the first page of `users` in `files`, past every full-page image the next open
/// replays, and return the page.
fn rot_users_page(files: &mut Files) -> Pid {
    let mut sim = Sim::with_files(9, Config::default(), files.clone());
    let pid = process::run(&mut sim, |rt| {
        let db = open::opened(&rt, FILE, &Log::new([APP]), &options());
        let t = db.testing();
        let pid = t.pages(t.heap("app::users", "User").unwrap()).unwrap()[0];
        db.close().unwrap();
        pid
    });
    *files = sim.files();
    DB.flip(files, pid, 200 - HEADER_LEN);
    pid
}

#[test]
fn rotted_page_listed_in_manifest_and_proof_failed() {
    let mut files = file_with_rows(4);
    let pid = rot_users_page(&mut files);
    let mut sim = Sim::with_files(10, Config::default(), files);
    process::run(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &Options::new(FILE)).unwrap();
        let got = backup_with(&pager, TARGET, &mut |_, _| true).unwrap();
        assert_eq!(vec![pid], got.damaged);
        pager.close();
        let got = prove_in(rt, &archive(), &Log::new([APP]), &no_create());
        let Ok(Proof::Failed(found)) = got else {
            panic!("{got:?}");
        };
        let timeless: Vec<String> = found
            .iter()
            .map(|f| match f.split_once(" at: ") {
                Some((head, tail)) => format!("{head} {}", tail.split_once(", ").unwrap().1),
                None => f.clone(),
            })
            .collect();
        let finding = format!(
            "Checksum {{ id: verify_findings.key(1), run: RunId(1), where: \"page {}: Checksum\" }}",
            pid.0
        );
        let damaged = format!("page {} was copied damaged", pid.0);
        assert_eq!(vec![damaged, finding], timeless);
    });
}

#[test]
fn header_names_backup_checkpoint_not_later_one() {
    let mut sim = Sim::new(11, Config::default());
    let mut got = None;
    process::run(&mut sim, |rt| {
        let db = open::opened(&rt, FILE, &Log::new([APP]), &options());
        insert(&db, 0..4);
        let t = db.testing();
        let last = {
            let mut copied = 0;
            db_backup(&db, "probe", &mut |pid, _| {
                copied = copied.max(pid.0);
                true
            })
            .unwrap();
            copied
        };
        let mut id = 100;
        let manifest = db_backup(&db, TARGET, &mut |pid, _| {
            if pid.0 == last {
                commit(&db, id);
                id += 1;
                t.flush_pages(usize::MAX);
                t.checkpoint();
            }
            true
        })
        .unwrap();
        got = Some(manifest);
        db.close().unwrap();
    });
    let rows = restored_rows(sim.files()).unwrap();
    assert!(rows.contains(&100), "a commit after the page's copy");
    assert_eq!(5, rows.len());
    let m = got.unwrap();
    assert!(m.header.checkpoint < m.end.lsn);
}

#[test]
fn page_allocated_after_copy_rebuilt_from_log() {
    let mut sim = Sim::new(12, Config::default());
    let big = "x".repeat(2000);
    let mut pages_copied = 0;
    process::run(&mut sim, |rt| {
        let db = open::opened(&rt, FILE, &Log::new([APP]), &options());
        insert(&db, 0..2);
        let last = {
            let mut copied = 0;
            db_backup(&db, "probe", &mut |pid, _| {
                copied = copied.max(pid.0);
                true
            })
            .unwrap();
            copied
        };
        let manifest = db_backup(&db, TARGET, &mut |pid, _| {
            if pid.0 == last {
                for id in 2..60 {
                    let text = format!(r#"insert users {{ id: users.key({id}), name: "{big}" }}"#);
                    api::results(&db, &text);
                }
            }
            true
        })
        .unwrap();
        pages_copied = manifest
            .parts
            .iter()
            .filter(|p| p.name.starts_with("pages"))
            .count();
        db.close().unwrap();
    });
    assert_eq!(1, pages_copied);
    let mut restored = Sim::with_files(13, Config::default(), sim.files());
    process::run(&mut restored, |rt| {
        restore(&rt).unwrap();
        let db = open::open_in(&rt, COPY, &Log::new([APP]), &no_create()).unwrap();
        assert_eq!(60, ids(&db).len());
        assert!(db.testing().probe().refusals().is_empty());
        assert_eq!(0, db.testing().verify_holding().unwrap().count());
        insert(&db, 60..62);
        assert_eq!(62, ids(&db).len());
        db.close().unwrap();
    });
}

fn pager_options(name: &str, atomic: bool, segment: u64) -> Options {
    Options {
        page_size: 4096,
        segment_size: segment,
        pool_frames: 64,
        max_pages: 4096,
        atomic_page_writes: atomic,
        ..Options::new(name)
    }
}

#[test]
fn atomic_page_writes_backup_restores() {
    let mut sim = Sim::new(14, Config::default());
    process::run(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &pager_options(FILE, true, 1 << 16)).unwrap();
        let pid = pager.allocate(1, PageKind::HeapSlotted, 7).unwrap();
        pager.update(&[pid], |v| v[0][100] = 1).unwrap();
        let mut n = 2;
        backup_with(&pager, TARGET, &mut |at, _| {
            if at != Pid(0) {
                pager.update(&[pid], |v| v[0][100] = n).unwrap();
                n += 1;
            }
            true
        })
        .unwrap();
        let last = n - 1;
        pager.update(&[pid], |v| v[0][100] = 99).unwrap();
        pager.close();
        storage::restore_file(&rt, TARGET, COPY, &format!("{COPY}-log")).unwrap();
        let (copy, _) = Pager::open(rt, &pager_options(COPY, true, 1 << 16)).unwrap();
        assert_eq!(last, copy.read(pid).unwrap()[100]);
        assert!(copy.verify().findings.is_empty());
        copy.close();
    });
}

fn part_named(files: &Files, dir: &str) -> String {
    files
        .keys()
        .find_map(|k| k.strip_prefix(&format!("{TARGET}/")))
        .into_iter()
        .chain(
            files
                .keys()
                .filter_map(|k| k.strip_prefix(&format!("{TARGET}/"))),
        )
        .find(|k| k.starts_with(dir))
        .unwrap()
        .to_owned()
}

/// Returns `body` with the line of `key` holding `value`.
fn replaced(body: &str, key: &str, value: &str) -> String {
    let line = |l: &str| match l.strip_prefix(key) {
        Some(rest) if rest.starts_with(' ') => format!("{key} {value}\n"),
        _ => format!("{l}\n"),
    };
    body.lines().map(line).collect()
}

/// Damage a part, drop one, flip the manifest's bytes, or seal a manifest naming a format, a field
/// out of bounds, a part outside the backup or no log: a restore refuses each naming the first part
/// that fails, or the format it does not read, and writes nothing.
#[test]
fn every_damaged_or_resealed_backup_is_refused_naming_what_failed_and_writes_nothing() {
    let base = backed_up(4);
    let (pages, log) = (part_named(&base, "pages/"), part_named(&base, "log/"));
    let edited = |edit: &dyn Fn(&mut Files)| {
        let mut files = base.clone();
        edit(&mut files);
        files
    };
    let damaged = |part: &str| RejectReason::BackupDamaged { part: part.into() };
    let manifest = || damaged(storage::MANIFEST);
    let mut cases = vec![
        (
            format!("a flipped byte of {pages}"),
            edited(&|f| f.get_mut(&format!("{TARGET}/{pages}")).unwrap()[5000] ^= 1),
            damaged(&pages),
        ),
        (
            format!("{log} missing"),
            edited(&|f| drop(f.remove(&format!("{TARGET}/{log}")))),
            damaged(&log),
        ),
        (
            "a flipped byte of the manifest".into(),
            edited(&|f| {
                f.get_mut(&format!("{TARGET}/{}", storage::MANIFEST))
                    .unwrap()[30] ^= 1;
            }),
            manifest(),
        ),
        (
            "format 9".into(),
            resealed(base.clone(), |b| replaced(b, "format", "9")),
            RejectReason::Invalid {
                detail: "this engine does not read backup format 9".into(),
            },
        ),
        (
            "no log part".into(),
            resealed(base.clone(), |b| {
                let kept = b.lines().filter(|l| !l.starts_with("part log/"));
                kept.map(|l| format!("{l}\n")).collect()
            }),
            damaged("log"),
        ),
        (
            "a checkpoint naming no record".into(),
            resealed(base.clone(), |b| {
                let at = b
                    .lines()
                    .find_map(|l| l.strip_prefix("checkpoint "))
                    .unwrap();
                replaced(
                    b,
                    "checkpoint",
                    &(at.parse::<u64>().unwrap() + 1).to_string(),
                )
            }),
            damaged("log"),
        ),
    ];
    let max = u64::MAX.to_string();
    let out_of_bounds = [
        ("log_block", "0"),
        ("log_block", "1000"),
        ("segment_size", "0"),
        ("segment_size", "4097"),
        ("page_size", "0"),
        ("page_size", "5000"),
        ("page_formats", "2 1"),
        ("file_pages", "0"),
        ("catalog_root", &max),
        ("quarantine_page", &max),
        ("start", &format!("1/{max}")),
        ("end", "1/0"),
        ("checkpoint", "0"),
    ];
    for (key, value) in out_of_bounds {
        let files = resealed(base.clone(), |b| replaced(b, key, value));
        cases.push((format!("{key} {value}"), files, manifest()));
    }
    for line in [
        "part ../outside 0 00".to_owned(),
        "part pages/a/b 0 00".to_owned(),
        format!("damaged {max}"),
    ] {
        let files = resealed(base.clone(), |b| format!("{b}{line}\n"));
        cases.push((line, files, manifest()));
    }
    for (label, files, reason) in &cases {
        let mut sim = Sim::with_files(15, Config::default(), files.clone());
        let got = process::run(&mut sim, |rt| restore(&rt));
        let want = Error::Rejected {
            reason: reason.clone(),
        };
        assert_eq!(Err(want), got, "{label}");
        assert!(*files == sim.files(), "{label}: the refused restore wrote");
    }
    assert_reached("a damaged or resealed backup", cases.len(), 22);
}

#[test]
fn segments_covering_redo_retained_through_backup() {
    let mut sim = Sim::new(19, Config::default());
    process::run(&mut sim, |rt| {
        let options = pager_options(FILE, false, 1 << 14);
        let pager = Pager::create(rt.clone(), &options).unwrap();
        let pid = pager.allocate(1, PageKind::HeapSlotted, 7).unwrap();
        let other = pager.allocate(1, PageKind::HeapSlotted, 7).unwrap();
        for i in 0..64u8 {
            pager
                .update(&[pid, other], |v| {
                    v[0][100] = i;
                    v[1][100] = i;
                })
                .unwrap();
            pager.flush_log();
        }
        let manifest = backup_with(&pager, TARGET, &mut |_, _| {
            for _ in 0..8 {
                pager
                    .update(&[pid], |v| v[0][101] = v[0][101] % 200 + 1)
                    .unwrap();
                pager.flush_pages(usize::MAX);
            }
            pager.flush_log();
            true
        })
        .unwrap();
        let logs: Vec<&storage::Part> = manifest
            .parts
            .iter()
            .filter(|p| p.name.starts_with("log/"))
            .collect();
        let first = u64::from_str_radix(logs[0].name.strip_prefix("log/").unwrap(), 16).unwrap();
        assert!(first <= manifest.start.lsn.0);
        assert!(first > 0, "the backup holds no segment before redo's");
        assert!(logs.len() > 1, "the log crosses a segment during the copy");
        assert!(
            logs.iter().any(|p| p.len < options.segment_size),
            "the last part stops at end"
        );
        pager.close();
        storage::restore_file(&rt, TARGET, COPY, &format!("{COPY}-log")).unwrap();
        let (copy, opening) = Pager::open(rt, &pager_options(COPY, false, 1 << 14)).unwrap();
        let recovery = opening.recovery().cloned().expect("the open recovered");
        assert_eq!(manifest.end.lsn, recovery.end);
        assert_eq!(Vec::<(Pid, Damage)>::new(), recovery.damaged);
        let found = copy.verify().findings;
        assert!(found.is_empty(), "{found:?}");
        assert_ne!(0, copy.read(pid).unwrap()[101]);
        copy.close();
    });
}

/// Returns the text of a `restore` of `TARGET` to `to` at `point`.
fn restore_at_text(to: &str, point: RestorePoint) -> String {
    let at = match point {
        RestorePoint::Latest => String::new(),
        RestorePoint::Version(v) => format!(" before version {v}"),
        RestorePoint::Position(p) => {
            format!(" at position {{ epoch: {}, lsn: {} }}", p.epoch, p.lsn)
        }
    };
    format!("restore from \"{TARGET}\" to \"{to}\"{at}")
}

/// Restore a backup at each point, through the API and the command: the latest and its end
/// restore, and any other point is unavailable naming the end as the oldest and newest. Each
/// restored file begins an epoch of its own: it holds the commits below the end, keeps them across
/// a reopen, and a position of the source past the end or of another restore is not on its
/// history.
#[test]
fn a_backup_restores_at_its_end_only_and_each_restore_begins_an_epoch() {
    let mut sim = Sim::new(21, Config::default());
    process::run(&mut sim, |rt| {
        let db = open::opened(&rt, FILE, &Log::new([APP]), &options());
        insert(&db, 0..2);
        let inside = commit(&db, 2);
        backup_text(&db).unwrap();
        let past = commit(&db, 3);
        let (manifest, _) = storage::read_manifest(&rt, TARGET).unwrap();
        let end = Position::from(manifest.end);
        assert!(past.lsn >= end.lsn, "{past:?} {end:?}");
        let moved = |lsn| RestorePoint::Position(Position { lsn, ..end });
        let unavailable = Err(Error::Rejected {
            reason: RejectReason::RestorePointUnavailable {
                oldest: end,
                newest: end,
            },
        });
        let points = [
            (RestorePoint::Latest, Ok(())),
            (RestorePoint::Position(end), Ok(())),
            (RestorePoint::Version(1), unavailable.clone()),
            (moved(end.lsn - 1), unavailable.clone()),
            (moved(end.lsn + 1), unavailable.clone()),
            (RestorePoint::Position(past), unavailable),
        ];
        for (i, (point, want)) in points.iter().enumerate() {
            let (by_api, by_text) = (format!("api{i}.adt"), format!("text{i}.adt"));
            let got = restore_in(rt.clone(), &archive(), &by_api, *point, &no_create());
            assert_eq!(*want, got.map(drop), "{point:?} by the API");
            let text = restore_at_text(&by_text, *point);
            let got = db.query_text(&text, &[], &CallOptions::default());
            assert_eq!(*want, got.map(drop), "{text}");
        }
        db.close().unwrap();
        let reopened =
            |name: &str| open::open_in(&rt, name, &Log::new([APP]), &no_create()).unwrap();
        let (a, b) = (reopened("api0.adt"), reopened("text1.adt"));
        let (at_a, at_b) = (commit(&a, 10), commit(&b, 20));
        assert!(
            at_a.epoch > inside.epoch && at_a.lsn > inside.lsn,
            "{at_a:?} {inside:?}"
        );
        assert_ne!(at_a.epoch, at_b.epoch);
        for (db, foreign) in [(&a, past), (&a, at_b), (&b, at_a)] {
            let after = CallOptions {
                after: Some(foreign),
                ..CallOptions::default()
            };
            let got = db.query_text("from u in users select u.id", &[], &after);
            let off = RejectReason::PositionNotOnHistory { position: foreign };
            assert_eq!(Err(Error::Rejected { reason: off }), got.map(drop));
        }
        let after = CallOptions {
            after: Some(inside),
            ..CallOptions::default()
        };
        let on: Vec<_> = a
            .query_text("from u in users select u.id", &[], &after)
            .unwrap();
        let on: Vec<_> = on.into_iter().map(api::sorted).collect();
        let want = vec![vec![
            "users.key(0)",
            "users.key(1)",
            "users.key(10)",
            "users.key(2)",
        ]];
        assert_eq!(want, on, "a position of the source below the end");
        a.close().unwrap();
        b.close().unwrap();
        let again = reopened("api0.adt");
        assert_eq!(BTreeSet::from([0, 1, 2, 10]), ids(&again));
        again.close().unwrap();
    });
}

/// Back up through a role's session: a role needs `maintain` and `write_files`, and its backup
/// runs inside the export roots and is refused outside them.
#[test]
fn a_role_backs_up_only_with_write_files_and_inside_the_export_roots() {
    process::once(26, |rt| {
        let opts = OpenOptions {
            file_root: Some("data".into()),
            export_roots: vec!["out".into()],
            ..options()
        };
        let db = Database::open_in(rt.clone(), FILE, &Log::new([APP]), &opts).unwrap();
        insert(&db, 0..2);
        let authority = db.role_authority();
        authority.create_role("ops").unwrap();
        for capability in [Capability::Adhoc, Capability::Maintain] {
            authority
                .grant_capability("ops", capability, false)
                .unwrap();
        }
        let ops = authority.session("ops").unwrap();
        let denied = |detail: &str| {
            Err(Error::Rejected {
                reason: RejectReason::Denied {
                    detail: detail.into(),
                },
            })
        };
        let lacking = "role `ops` lacks `write_files`";
        assert_eq!(
            denied(lacking),
            api::task(&rt, &ops, "backup to \"out/role-bk\"")
        );
        authority
            .grant_capability("ops", Capability::WriteFiles, false)
            .unwrap();
        api::task(&rt, &ops, "backup to \"out/role-bk\"").unwrap();
        storage::check_backup(&rt, "data/out/role-bk").unwrap();
        for path in ["role-bk", "out/../role-bk", "out"] {
            let text = format!("backup to \"{path}\"");
            let outside = format!("path `{path}` is outside export_roots");
            assert_eq!(denied(&outside), api::task(&rt, &ops, &text), "{path}");
        }
        db.close().unwrap();
    });
}

#[test]
fn backup_of_quarantined_heap_lists_it() {
    process::once(27, |rt| {
        let db = open::opened(&rt, FILE, &Log::new([APP]), &options());
        insert(&db, 0..2);
        let t = db.testing();
        let heap = t.own_mark(t.heap("app::users", "User").unwrap()).unwrap();
        t.plant(Plant::Mark(heap)).unwrap();
        backup_text(&db).unwrap();
        db.close().unwrap();
        let (manifest, _) = storage::read_manifest(&rt, TARGET).unwrap();
        assert_eq!(vec!["app::users.User".to_owned()], manifest.quarantined);
    });
}

#[test]
fn prove_with_host_fn_is_unproven() {
    const LOG: &str = r#"module app
add module app
add type User = { id: Key users, name: Text }
define fn boom (t: Text) -> Text = host "boom" version "1.0" deterministic
add relation users : User key id generate sequence
  index (boom(name))
"#;
    let mut sim = Sim::new(29, Config::default());
    process::run(&mut sim, |rt| {
        let mut fns = HostFns::new();
        fns.register(
            "boom",
            "1.0",
            Determinism::Deterministic,
            |b: &crate::batch::Batch| Ok(b.clone()),
        );
        let opening = OpenOptions {
            host_fns: fns,
            ..open::options(2)
        };
        let log = Log::new([LOG]);
        let db = Database::open_in(rt.clone(), FILE, &log, &opening).unwrap();
        api::results(&db, r#"insert users { id: users.key(1), name: "a" }"#);
        backup_text(&db).unwrap();
        db.close().unwrap();
        match prove_in(rt.clone(), &archive(), &log, &no_create()).unwrap() {
            Proof::Unproven { function } => assert_eq!(Some("boom".to_owned()), function),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            Proof::Unproven {
                function: Some("boom".to_owned())
            },
            proof_in(&rt, &archive()).unwrap()
        );
    });
}

/// Cancel a `backup to` before it copies: it answers `Cancelled` and leaves nothing at its target.
#[test]
fn a_cancelled_backup_leaves_nothing() {
    let mut sim = Sim::new(30, Config::default());
    process::run(&mut sim, |rt| {
        let db = open::opened(&rt, FILE, &Log::new([APP]), &options());
        insert(&db, 0..2);
        let cancel = CancelToken::new();
        cancel.cancel();
        let cancelled = db.query_text(
            &format!("backup to \"{TARGET}\""),
            &[],
            &CallOptions {
                cancel: Some(cancel),
                ..CallOptions::default()
            },
        );
        let reason = RejectReason::Cancelled;
        assert_eq!(Err(Error::Rejected { reason }), cancelled.map(drop));
        assert!(!sim_has(&rt, TARGET), "the cancelled backup's target");
        backup_text(&db).unwrap();
        db.close().unwrap();
    });
    assert_eq!(2, restored_rows(sim.files()).unwrap().len());
}

/// Returns `files` with the backup's manifest rewritten by `edit` and sealed again.
fn resealed(mut files: Files, edit: impl FnOnce(&str) -> String) -> Files {
    let at = format!("{TARGET}/manifest");
    let bytes = files.remove(&at).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    let body = text
        .trim_end_matches('\0')
        .rsplit_once("checksum ")
        .unwrap()
        .0;
    let body = edit(body);
    let sealed = format!("{body}checksum {}\n", storage::manifest_hash(&body));
    let mut bytes = sealed.into_bytes();
    bytes.resize(bytes.len().next_multiple_of(4096), 0);
    files.insert(at, bytes);
    files
}

fn manifest_absent<R: Runtime>(rt: &R) {
    let got = storage::read_manifest(rt, TARGET).map(drop);
    assert_eq!(Err(BackupError::Absent(TARGET.into())), got);
}

/// Refuse a backup whose log range has lost a segment.
#[test]
fn a_hole_in_the_log_fails_the_backup() {
    let mut sim = Sim::new(40, Config::default());
    process::run(&mut sim, |rt| {
        let options = pager_options(FILE, false, 1 << 14);
        let pager = Pager::create(rt.clone(), &options).unwrap();
        let pid = pager.allocate(1, PageKind::HeapSlotted, 7).unwrap();
        let mut io = rt.io();
        let mut removed = None;
        let got = backup_with(&pager, TARGET, &mut |_, _| {
            if removed.is_none() {
                for i in 0..200u32 {
                    pager
                        .update(&[pid], |v| v[0][100] = (i % 200) as u8 + 1)
                        .unwrap();
                    pager.flush_pages(usize::MAX);
                }
                pager.flush_log();
                let dir = options.log_dir();
                let listed = common::complete(&rt, &mut io, Op::List { dir }).result;
                let Ok(Done::Listed(mut segments)) = listed else {
                    panic!("{listed:?}");
                };
                segments.sort();
                assert!(segments.len() > 2, "{segments:?}");
                let path = segments[segments.len() - 2].clone();
                removed = path.rsplit('/').next().map(str::to_owned);
                common::complete(&rt, &mut io, Op::Remove { path })
                    .result
                    .unwrap();
            }
            true
        });
        let hole = BackupError::Io {
            what: format!("log segment {}", removed.unwrap()),
            error: IoError::NotFound,
        };
        assert_eq!(Err(hole), got.map(drop));
        manifest_absent(&rt);
    });
}

/// Return the pages a manifest lists as copied damaged from a restore that succeeds.
#[test]
fn restore_returns_the_listed_damage() {
    let mut files = file_with_rows(4);
    let pid = rot_users_page(&mut files);
    let mut sim = Sim::with_files(42, Config::default(), files);
    process::run(&mut sim, |rt| {
        let db = open::opened(&rt, FILE, &Log::new([APP]), &options());
        backup_text(&db).unwrap();
        db.close().unwrap();
        let got = restore_in(rt, &archive(), COPY, RestorePoint::Latest, &no_create());
        assert_eq!(vec![pid.0], got.unwrap());
    });
}

/// Withdraw an earlier proof when a part has rotted since.
#[test]
fn a_part_rotted_after_a_proof_withdraws_it() {
    let mut files = backed_up(2);
    let mut sim = Sim::with_files(43, Config::default(), files.clone());
    process::run(&mut sim, |rt| {
        let log = Log::new([APP]);
        assert_eq!(
            Proof::Proven,
            prove_in(rt.clone(), &archive(), &log, &no_create()).unwrap()
        );
        assert_eq!(Proof::Proven, proof_in(&rt, &archive()).unwrap());
    });
    files = sim.files();
    assert!(
        !files.keys().any(|name| name.contains("scratch")),
        "a proof keeps no scratch database"
    );
    let part = part_named(&files, "pages/");
    files.get_mut(&format!("{TARGET}/{part}")).unwrap()[5000] ^= 1;
    let mut sim = Sim::with_files(44, Config::default(), files);
    process::run(&mut sim, |rt| {
        let got = prove_in(rt.clone(), &archive(), &Log::new([APP]), &no_create());
        let reason = RejectReason::BackupDamaged { part: part.clone() };
        assert_eq!(Err(Error::Rejected { reason }), got);
        assert_eq!(
            Proof::Unproven { function: None },
            proof_in(&rt, &archive()).unwrap()
        );
    });
}

/// Returns a file of one used page and the pids of that page and of its group's bitmap.
fn pager_with_bitmap(options: &Options) -> (Files, Pid, Pid) {
    let mut sim = Sim::new(45, Config::default());
    let (used, bitmap) = process::run(&mut sim, |rt| {
        let pager = Pager::create(rt, options).unwrap();
        let used = pager.allocate(1, PageKind::HeapSlotted, 7).unwrap();
        pager.update(&[used], |v| v[0][100] = 9).unwrap();
        let bitmap = Pid::FIRST_DATA;
        assert_eq!(
            PageKind::AllocationBitmap as u16,
            storage::kind(&pager.read(bitmap).unwrap())
        );
        pager.close();
        (used, bitmap)
    });
    (sim.files(), used, bitmap)
}

/// List a corrupt page of a group whose bitmap cannot be read.
#[test]
fn a_corrupt_page_under_an_unreadable_bitmap_is_listed() {
    let options = pager_options(FILE, false, 1 << 20);
    let (mut files, used, bitmap) = pager_with_bitmap(&options);
    let data = DataFile::new(&options.name, options.page_size as usize);
    data.flip(&mut files, bitmap, 200 - HEADER_LEN);
    data.flip(&mut files, used, 200 - HEADER_LEN);
    let mut sim = Sim::with_files(46, Config::default(), files);
    process::run(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt, &options).unwrap();
        let manifest = backup_with(&pager, TARGET, &mut |_, _| true).unwrap();
        assert!(manifest.damaged.contains(&used), "{:?}", manifest.damaged);
        assert!(manifest.damaged.contains(&bitmap), "{:?}", manifest.damaged);
        pager.close();
    });
}

/// Leave out of the list a corrupt page the bitmap calls free.
#[test]
fn a_corrupt_free_page_is_not_listed() {
    let options = pager_options(FILE, false, 1 << 20);
    let (mut files, _, _) = pager_with_bitmap(&options);
    let mut sim = Sim::with_files(47, Config::default(), files.clone());
    let free = process::run(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt, &options).unwrap();
        let free = pager.allocate(1, PageKind::HeapSlotted, 7).unwrap();
        pager.free(free).unwrap();
        pager.close();
        free
    });
    files = sim.files();
    DataFile::new(&options.name, options.page_size as usize).flip(
        &mut files,
        free,
        200 - HEADER_LEN,
    );
    let mut sim = Sim::with_files(48, Config::default(), files);
    process::run(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt, &options).unwrap();
        assert!(pager.is_free(free).unwrap());
        let manifest = backup_with(&pager, TARGET, &mut |_, _| true).unwrap();
        assert!(!manifest.damaged.contains(&free), "{:?}", manifest.damaged);
        pager.close();
    });
}

/// Copy the pool's changed version of a page whose stored bytes have gone bad during the backup.
#[test]
fn a_changed_page_with_a_corrupt_store_is_copied_from_the_pool() {
    let options = pager_options(FILE, false, 1 << 20);
    let mut sim = Sim::new(49, Config::default());
    process::run(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &options).unwrap();
        let first = pager.allocate(1, PageKind::HeapSlotted, 7).unwrap();
        let page = pager.allocate(1, PageKind::HeapSlotted, 7).unwrap();
        assert!(first < page);
        let mut io = rt.io();
        let mut spoiled = false;
        let manifest = backup_with(&pager, TARGET, &mut |at, _| {
            if !spoiled && at.0 < page.0 {
                spoiled = true;
                pager.update(&[page], |v| v[0][100] = 7).unwrap();
                let file = common::open(&rt, &mut io, FILE, false).unwrap().0;
                common::write(
                    &rt,
                    &mut io,
                    file,
                    DataFile::new(&options.name, options.page_size as usize)
                        .range(page)
                        .start as u64,
                    &vec![0xee; 4096],
                )
                .unwrap();
            }
            true
        })
        .unwrap();
        assert!(spoiled);
        assert!(!manifest.damaged.contains(&page), "{:?}", manifest.damaged);
        pager.close();
        storage::restore_file(&rt, TARGET, COPY, &format!("{COPY}-log")).unwrap();
        let (copy, _) = Pager::open(rt, &pager_options(COPY, false, 1 << 20)).unwrap();
        assert_eq!(7, copy.read(page).unwrap()[100]);
        copy.close();
    });
}

#[test]
fn a_page_that_lost_a_write_before_the_open_is_listed() {
    let mut before = file_with_rows(4);
    let pid = rot_users_page(&mut before);
    DB.flip(&mut before, pid, 200 - HEADER_LEN);
    let mut sim = Sim::with_files(10, Config::default(), before.clone());
    process::run(&mut sim, |rt| {
        let db = open::opened(&rt, FILE, &Log::new([APP]), &options());
        api::results(
            &db,
            r#"update u in users where key u == users.key(0) set User { u with name: "later" }"#,
        );
        db.close().unwrap();
    });
    let mut files = sim.files();
    DB.replace(&mut files, pid, DB.page(&before, pid));
    let mut sim = Sim::with_files(10, Config::default(), files);
    process::run(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt, &Options::new(FILE)).unwrap();
        let got = backup_with(&pager, TARGET, &mut |_, _| true).unwrap();
        assert_eq!(vec![pid], got.damaged);
        pager.close();
    });
}

/// List a page the device refuses to read, and show the visit an empty page.
#[test]
fn an_unreadable_page_is_listed() {
    let options = pager_options(FILE, false, 1 << 20);
    let mut sim = Sim::new(50, Config::default());
    process::run(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &options).unwrap();
        let page = pager.allocate(1, PageKind::HeapSlotted, 7).unwrap();
        pager.update(&[page], |v| v[0][100] = 7).unwrap();
        pager.flush_log();
        pager.flush_pages(usize::MAX);
        let reads = Match::kind(OpKind::Read).offset(
            DataFile::new(&options.name, options.page_size as usize)
                .range(page)
                .start as u64,
        );
        rt.plan(Plan::new().rule(reads, Action::Fail(IoError::Failed)));
        let mut empty = false;
        let manifest = backup_with(&pager, TARGET, &mut |at, bytes| {
            empty |= at == page && bytes.is_empty();
            true
        })
        .unwrap();
        assert!(empty);
        assert!(manifest.damaged.contains(&page), "{:?}", manifest.damaged);
    });
}

/// Close every file a stopped or failed backup or restore opened.
#[test]
fn early_returns_close_their_files() {
    let options = pager_options(FILE, false, 1 << 20);
    let mut sim = Sim::new(61, Config::default());
    process::run(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &options).unwrap();
        for _ in 0..700 {
            pager.allocate(1, PageKind::HeapSlotted, 7).unwrap();
        }
        pager.flush_log();
        pager.flush_pages(usize::MAX);
        let open = rt.files_open();
        let got = backup_with(&pager, TARGET, &mut |at, _| at.0 < 600);
        assert_eq!(Some(BackupError::Stopped), got.err());
        assert_eq!(open, rt.files_open(), "a stop among the pages");
        let got = backup_with(&pager, TARGET, &mut |at, _| at != Pid(0));
        assert_eq!(Some(BackupError::Stopped), got.err(), "a stop in the log");
        assert_eq!(open, rt.files_open(), "a stop in the log");
        manifest_absent(&rt);
        backup_with(&pager, TARGET, &mut |_, _| true).unwrap();
        rt.arm(Match::kind(OpKind::Write));
        rt.plan(Plan::new().at(2, Action::Fail(IoError::Full)));
        let got = storage::restore_file(&rt, TARGET, COPY, &format!("{COPY}-log"));
        assert!(matches!(got, Err(BackupError::Io { .. })), "{got:?}");
        assert_eq!(open, rt.files_open(), "a failed restore");
    });
}

/// Keep the epochs a header holds, forget the oldest past its capacity, and refuse an epoch it never
/// held.
#[test]
fn history_truncates_at_capacity() {
    let files = resealed(backed_up(1), |body| {
        let lines: String = (1..=storage::HISTORY_CAPACITY as u64)
            .map(|k| format!("epoch_ended {k} {}\n", k * 1000))
            .collect();
        let end = body.lines().find(|l| l.starts_with("end ")).unwrap();
        let lsn = end.rsplit_once('/').unwrap().1;
        let raised = format!("end 100000/{lsn}");
        format!("{}{lines}", body.replace(end, &raised))
    });
    let mut sim = Sim::with_files(62, Config::default(), files);
    process::run(&mut sim, |rt| {
        restore(&rt).unwrap();
        let db = open::open_in(&rt, COPY, &Log::new([APP]), &no_create()).unwrap();
        let t = db.testing();
        let on = |epoch, lsn| {
            t.probe().on_history(storage::Position {
                epoch,
                lsn: Lsn(lsn),
            })
        };
        assert!(on(2, 2000), "the oldest kept epoch");
        assert!(!on(2, 2001), "past that epoch's end");
        assert!(on(1, 2000), "a forgotten epoch up to the oldest end");
        assert!(!on(1, 2001), "a forgotten epoch past the oldest end");
        assert!(!on(500, 1), "an epoch the history never held");
        db.close().unwrap();
    });
}

fn restore_text(db: &Database<SimRuntime>) -> Result<(), Error> {
    db.query_text(
        &format!("restore from \"{TARGET}\" to \"{COPY}\""),
        &[],
        &CallOptions::default(),
    )
    .map(|_| ())
}

fn storage_failure(detail: &str) -> Error {
    Error::Fatal {
        reason: FatalReason::Storage {
            detail: detail.into(),
        },
    }
}

/// Fail every read once a restore opens the manifest, a prove opens a part, a read of the proof
/// opens it, or a restore by a running source opens the restored log: each answers a storage
/// failure, never damage, and changes nothing. The proof stays, the source keeps taking writes,
/// and a retry restores.
#[test]
fn a_failed_read_of_a_backup_is_a_storage_failure_and_changes_nothing() {
    let mut sim = Sim::with_files(60, Config::default(), backed_up(1));
    process::run(&mut sim, |rt| {
        let open_op = |path: &str| Match::kind(OpKind::Open).path(path);
        let log = Log::new([APP]);
        rt.plan(fail_reads_after_open(&rt, open_op("bk/manifest")));
        let got = restore(&rt);
        assert_eq!(Err(storage_failure("`bk/manifest`: Failed")), got);
        rt.plan(Plan::new());
        let (manifest, _) = storage::read_manifest(&rt, TARGET).unwrap();
        prove_in(rt.clone(), &archive(), &log, &no_create()).unwrap();
        let pages = Match::kind(OpKind::Open).prefixed("bk/pages/");
        rt.plan(fail_reads_after_open(&rt, pages));
        let got = prove_in(rt.clone(), &archive(), &log, &no_create());
        let pages = manifest.parts.iter().find(|p| p.name.starts_with("pages/"));
        let part = format!("`{TARGET}/{}`: Failed", pages.unwrap().name);
        assert_eq!(Err(storage_failure(&part)), got);
        rt.plan(Plan::new());
        assert_eq!(Proof::Proven, proof_in(&rt, &archive()).unwrap());
        rt.plan(fail_reads_after_open(&rt, open_op("bk/proof")));
        let got = proof_in(&rt, &archive());
        assert_eq!(Err(storage_failure("`bk/proof`: Failed")), got);
        rt.plan(Plan::new());
        let db = open::open_in(&rt, FILE, &log, &no_create()).unwrap();
        let restored_log = Match::kind(OpKind::Open).prefixed("copy.adt-log");
        rt.plan(fail_reads_after_open(&rt, restored_log));
        let got = restore_text(&db);
        let start = manifest.start.lsn.0;
        let first = start - start % u64::from(manifest.header.log_block);
        let block = format!("`log block at {first}`: Failed");
        assert_eq!(Err(storage_failure(&block)), got);
        rt.plan(Plan::new());
        insert(&db, 2..3);
        assert!(!sim_has(&rt, COPY), "the file of the failed restore");
        assert!(
            !sim_has(&rt, "copy.adt-log"),
            "the log of the failed restore"
        );
        restore_text(&db).unwrap();
        db.close().unwrap();
    });
}

/// Returns whether an entry named `name` is in the top directory.
fn sim_has(rt: &SimRuntime, name: &str) -> bool {
    let mut io = rt.io();
    let listed = common::complete(rt, &mut io, Op::List { dir: String::new() }).result;
    match listed {
        Ok(Done::Listed(paths)) => paths.iter().any(|p| p == name),
        other => panic!("{other:?}"),
    }
}

/// Returns the entries of directory `dir`.
fn listed(rt: &SimRuntime, dir: &str) -> Vec<String> {
    let mut io = rt.io();
    let dir = dir.to_owned();
    match common::complete(rt, &mut io, Op::List { dir }).result {
        Ok(Done::Listed(paths)) => paths,
        other => panic!("{other:?}"),
    }
}

/// Fail a backup to a target it creates, to an empty target it finds, and a restore to a nested
/// path: each removes what it created and nothing it found. Run each whole: it forces the
/// directories it creates and no directory above them.
#[test]
fn a_backup_or_restore_removes_and_forces_only_the_directories_it_creates() {
    let mut sim = Sim::new(74, Config::default());
    process::run(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &pager_options(FILE, false, 1 << 20)).unwrap();
        let mut io = rt.io();
        for dir in ["up", "up/mid", "up/found"] {
            let dir = dir.to_owned();
            common::complete(&rt, &mut io, Op::CreateDir { dir })
                .result
                .unwrap();
        }
        for target in ["up/mid/bk", "up/found"] {
            let got = backup_with(&pager, target, &mut |_, _| false);
            assert_eq!(Some(BackupError::Stopped), got.err(), "{target}");
        }
        let mut up = listed(&rt, "up");
        up.sort();
        assert_eq!(vec!["up/found".to_owned(), "up/mid".to_owned()], up);
        assert_eq!(Vec::<String>::new(), listed(&rt, "up/mid"));
        let forces = rt.logged(Match::kind(OpKind::SyncDir));
        let refused = |dir| Match::kind(OpKind::SyncDir).path(dir);
        rt.plan(
            Plan::new()
                .rule(refused("up"), Action::Fail(IoError::Failed))
                .rule(refused(""), Action::Fail(IoError::Failed)),
        );
        backup_with(&pager, "up/mid/bk", &mut |_, _| true).unwrap();
        storage::restore_file(&rt, "up/mid/bk", "up/mid/new/copy.adt", "up/mid/new/log").unwrap();
        let forced: BTreeSet<String> = forces
            .entries()
            .iter()
            .filter_map(|e| e.path().map(str::to_owned))
            .collect();
        for dir in ["up/mid", "up/mid/new"] {
            assert!(forced.contains(dir), "{dir}: {forced:?}");
        }
        rt.plan(Plan::new().rule(Match::kind(OpKind::Write), Action::Fail(IoError::Full)));
        let got = storage::restore_file(&rt, "up/mid/bk", "deep/er/copy.adt", "deep/er/log");
        let (manifest, _) = storage::read_manifest(&rt, "up/mid/bk").unwrap();
        let pages = manifest.parts.iter().find(|p| p.name.starts_with("pages/"));
        let full = BackupError::Io {
            what: format!("up/mid/bk/{}", pages.unwrap().name),
            error: IoError::Full,
        };
        assert_eq!(Err(full), got.map(drop));
        rt.plan(Plan::new());
        assert_eq!(Vec::<String>::new(), listed(&rt, "deep/er"));
        storage::restore_file(&rt, "up/mid/bk", "deep/er/copy.adt", "deep/er/log").unwrap();
        pager.close();
    });
}

/// Restore a backup taken under a rotted bitmap: the copy opens and lists the damage.
#[test]
fn a_restore_under_a_rotted_bitmap_opens_and_lists_it() {
    let options = pager_options(FILE, false, 1 << 20);
    let (mut files, used, bitmap) = pager_with_bitmap(&options);
    DataFile::new(&options.name, options.page_size as usize).flip(
        &mut files,
        bitmap,
        200 - HEADER_LEN,
    );
    let mut sim = Sim::with_files(79, Config::default(), files);
    process::run(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &options).unwrap();
        backup_with(&pager, TARGET, &mut |_, _| true).unwrap();
        pager.close();
        let manifest = storage::restore_file(&rt, TARGET, COPY, "copy.adt-log").unwrap();
        assert!(manifest.damaged.contains(&bitmap), "{:?}", manifest.damaged);
        let copy = pager_options(COPY, false, 1 << 20);
        let (pager, _) = Pager::open(rt, &copy).unwrap();
        assert_eq!(
            Some(PageError {
                pid: bitmap,
                damage: Damage::Checksum
            }),
            pager.read(bitmap).err(),
            "the bitmap stays rotted"
        );
        assert_eq!(
            storage::kind(&pager.read(used).unwrap()),
            PageKind::HeapSlotted as u16
        );
        pager.close();
    });
}

/// Restore a page this open refused, but whose stored bytes verify, as copied and unlisted, unless
/// it was refused as stale.
#[test]
fn a_refused_page_with_sound_bytes_restores_as_copied_unless_stale() {
    let options = pager_options(FILE, false, 1 << 20);
    let (files, used, _) = pager_with_bitmap(&options);
    let stale = Damage::Stale {
        expected: Lsn(1 << 40),
        found: Lsn(1),
    };
    let cases = [
        (Damage::Unreadable, Ok(9), vec![]),
        (Damage::Checksum, Ok(9), vec![]),
        (stale, Err(Damage::Checksum), vec![used]),
    ];
    for (damage, want, listed) in cases {
        let mut sim = Sim::with_files(51, Config::default(), files.clone());
        let got = process::run(&mut sim, |rt| {
            let (pager, _) = Pager::open(rt.clone(), &options).unwrap();
            pager.damage_tested(used, damage);
            let manifest = backup_with(&pager, TARGET, &mut |_, _| true).unwrap();
            assert_eq!(listed, manifest.damaged, "{damage:?}");
            pager.close();
            storage::restore_file(&rt, TARGET, COPY, &format!("{COPY}-log")).unwrap();
            let (copy, _) = Pager::open(rt, &pager_options(COPY, false, 1 << 20)).unwrap();
            let got = copy.read(used).map(|p| p[100]).map_err(|e| e.damage);
            copy.close();
            got
        });
        assert_eq!(want, got, "{damage:?}");
    }
}

/// Where a page last flushed is when the backup copies it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Flushed {
    Evicted,
    /// Evicted, and read during the backup after its copy.
    ReadAfterCopy,
    Resident,
}

/// What a backup of a page whose byte 100 was set to 7 and flushed made of it.
#[derive(Debug, PartialEq)]
struct FlushedCopy {
    listed: Vec<Pid>,
    restored: Result<u8, Damage>,
    source: Result<u8, Damage>,
}

/// Set byte 100 of a page to 7 and flush it, losing the write when `lost`, leave it `at`, back
/// it up and restore the backup. Returns the copy, the page, and the stale damage a lost write is
/// refused with.
fn backup_after_flush(seed: u64, lost: bool, at: Flushed) -> (FlushedCopy, Pid, Damage) {
    let mut sim = Sim::new(seed, Config::default());
    process::run(&mut sim, |rt| {
        let (pager, pages, found, expected) = flushed(&rt, false, lost);
        let (victim, last) = (pages[0], pages[15]);
        let evicting = if at == Flushed::Resident { 0 } else { 3 };
        for _ in 0..evicting {
            for &pid in &pages[1..] {
                drop(pager.read(pid).unwrap());
            }
        }
        let byte = |p: &Pager<SimRuntime>| p.read(victim).map(|g| g[100]).map_err(|e| e.damage);
        let mut during = None;
        let manifest = backup_with(&pager, TARGET, &mut |pid, _| {
            if at == Flushed::ReadAfterCopy && pid == last && during.is_none() {
                during = Some(byte(&pager));
            }
            true
        })
        .unwrap();
        let stale = Damage::Stale { expected, found };
        let source = byte(&pager);
        if at == Flushed::ReadAfterCopy {
            assert_eq!(Some(source), during, "{at:?}");
        }
        pager.close();
        storage::restore_file(&rt, TARGET, COPY, &format!("{COPY}-log")).unwrap();
        let (copy, _) = Pager::open(rt, &pager_options(COPY, false, 1 << 20)).unwrap();
        let restored = byte(&copy);
        copy.close();
        let listed = manifest.damaged;
        let copy = FlushedCopy {
            listed,
            restored,
            source,
        };
        (copy, victim, stale)
    })
}

/// Back up a page whose last flush was lost or landed, with the page evicted, read during the
/// backup after its copy, or resident: a lost write is listed and its copy restores refused, and a
/// landed one restores with its value.
#[test]
fn a_backup_lists_a_lost_write_wherever_the_pool_holds_its_page() {
    let places = [Flushed::Evicted, Flushed::ReadAfterCopy, Flushed::Resident];
    let mut reached = 0;
    for (lost, seed) in [(true, 53), (false, 56)] {
        for at in places {
            let (copy, pid, stale) = backup_after_flush(seed, lost, at);
            let source = match (lost, at) {
                (true, Flushed::Evicted | Flushed::ReadAfterCopy) => Err(stale),
                (true, Flushed::Resident) | (false, _) => Ok(7),
            };
            let want = FlushedCopy {
                listed: if lost { vec![pid] } else { vec![] },
                restored: if lost { Err(Damage::Checksum) } else { Ok(7) },
                source,
            };
            assert_eq!(want, copy, "lost {lost}, {at:?}");
            reached += 1;
        }
    }
    assert_reached("a flush and a place of its page", reached, 6);
}

/// Create a pager of 8 frames, allocate 16 pages and flush them, then set byte 100 of the first to
/// 7 and flush it, losing the write when `lost`. Returns the pager, the pages and the first's LSN
/// before and after the change.
fn flushed(rt: &SimRuntime, atomic: bool, lost: bool) -> (Pager<SimRuntime>, Vec<Pid>, Lsn, Lsn) {
    let options = Options {
        pool_frames: 8,
        ..pager_options(FILE, atomic, 1 << 20)
    };
    let pager = Pager::create(rt.clone(), &options).unwrap();
    let pages: Vec<Pid> = (0..16)
        .map(|_| pager.allocate(1, PageKind::HeapSlotted, 7).unwrap())
        .collect();
    pager.flush_log();
    pager.flush_pages(usize::MAX);
    let found = storage::lsn(&pager.read(pages[0]).unwrap());
    let expected = pager.update(&[pages[0]], |v| v[0][100] = 7).unwrap();
    pager.flush_log();
    let lost_write_p = if lost { 1.0 } else { 0.0 };
    rt.set_faults(Faults {
        lost_write_p,
        ..Faults::none(512)
    });
    pager.flush_pages(usize::MAX);
    rt.set_faults(Faults::none(512));
    (pager, pages, found, expected)
}

/// Copy from the pool a lost write that a writer changes again during the backup, before its copy.
#[test]
fn a_lost_write_changed_again_before_its_copy_restores_both_changes() {
    let mut sim = Sim::new(57, Config::default());
    let got = process::run(&mut sim, |rt| {
        let (pager, pages, _, _) = flushed(&rt, true, true);
        let victim = pages[0];
        pager.checkpoint();
        let bytes = |p: &Pager<SimRuntime>| {
            p.read(victim)
                .map(|g| (g[100], g[101]))
                .map_err(|e| e.damage)
        };
        let manifest = backup_with(&pager, TARGET, &mut |pid, _| {
            if pid == victim {
                pager.update(&[victim], |v| v[0][101] = 8).unwrap();
            }
            true
        })
        .unwrap();
        let source = bytes(&pager);
        pager.close();
        storage::restore_file(&rt, TARGET, COPY, &format!("{COPY}-log")).unwrap();
        let (copy, _) = Pager::open(rt, &pager_options(COPY, true, 1 << 20)).unwrap();
        let restored = bytes(&copy);
        copy.close();
        (manifest.damaged, source, restored)
    });
    assert_eq!((vec![], Ok((7, 8)), Ok((7, 8))), got);
}

/// Heal in the restored file a listed page that a full-page image in the backup's log overwrites.
#[test]
fn a_listed_page_a_replayed_image_overwrites_restores_healed() {
    let options = pager_options(FILE, false, 1 << 20);
    let (mut files, used, _) = pager_with_bitmap(&options);
    DataFile::new(&options.name, options.page_size as usize).flip(
        &mut files,
        used,
        200 - HEADER_LEN,
    );
    let mut sim = Sim::with_files(52, Config::default(), files);
    process::run(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &options).unwrap();
        let from = pager.allocate(1, PageKind::HeapSlotted, 7).unwrap();
        pager.update(&[from], |v| v[0][100] = 5).unwrap();
        assert!(used < from);
        let refused = pager.read(used).map(|_| ()).map_err(|e| e.damage);
        assert_eq!(Err(Damage::Checksum), refused);
        let mut replaced = false;
        let manifest = backup_with(&pager, TARGET, &mut |at, _| {
            if at == from && !replaced {
                replaced = true;
                pager.replace_contents(used, from, None).unwrap();
            }
            true
        })
        .unwrap();
        assert!(replaced);
        assert_eq!(vec![used], manifest.damaged);
        pager.close();
        storage::restore_file(&rt, TARGET, COPY, &format!("{COPY}-log")).unwrap();
        let (copy, _) = Pager::open(rt, &pager_options(COPY, false, 1 << 20)).unwrap();
        assert_eq!(5, copy.read(used).unwrap()[100]);
        copy.close();
    });
}

const SEARCH_ROWS: i64 = 3;
const LATER_ROW: i64 = 100;
const FAILURES: u64 = 4;
const CRASH_POINTS: u64 = 4;
const CRASH_IMAGES: usize = 2;
const STARTS: u64 = 2;

fn rows_upto(rows: i64) -> BTreeSet<i64> {
    (0..rows).collect()
}

fn is_aside(path: &str) -> bool {
    path.contains(".partial-")
}

/// Returns `files` outside every private name.
fn final_files(files: &Files) -> Files {
    let finals = files.iter().filter(|(path, _)| !is_aside(path));
    finals.map(|(p, b)| (p.clone(), b.clone())).collect()
}

/// Accept every op of a run, and of the source only those a failure may answer without ending it.
fn searched() -> Match {
    let source = Match::any_of([
        Match::any().path(FILE),
        Match::any().prefixed("backup.adt-log"),
    ]);
    let answered = Match::kinds([OpKind::Read, OpKind::Open, OpKind::List]);
    Match::any()
        .except(Match::kinds([OpKind::Close, OpKind::Timer]))
        .except(source.except(answered))
}

/// Returns `ops` limited to those on the database a test restore opened, but its renames.
fn on_the_test_database(ops: Match) -> Match {
    ops.containing("scratch.partial-")
        .containing("/restored.adt")
        .except(Match::any().containing("/restored.adt.partial-"))
        .except(Match::any().containing("/restored.adt-log.partial-"))
        .except(Match::kind(OpKind::Rename))
}

/// Returns whether a failure of the op `struck` may leave a private name behind.
fn stops_a_cleanup(struck: &Struck) -> bool {
    matches!(
        struck.kind(),
        OpKind::Remove | OpKind::RemoveDir | OpKind::List
    )
}

/// Returns whether `struck` is a read of the source's data file or log.
fn reads_the_source(struck: &Struck) -> bool {
    struck.kind() == OpKind::Read
        && struck
            .path()
            .is_some_and(|p| p == FILE || p.starts_with(&format!("{FILE}-log")))
}

/// Returns a plan failing every read once an open `opening` matches has come.
fn fail_reads_after_open(rt: &SimRuntime, opening: Match) -> Plan {
    let planner = rt.clone();
    let then = Action::Run(Box::new(move || {
        let reads = Match::kind(OpKind::Read);
        planner.plan(Plan::new().rule(reads, Action::Fail(IoError::Failed)));
    }));
    Plan::new().first(opening, then)
}

fn asides_at_top(rt: &SimRuntime) -> Vec<String> {
    let mut io = rt.io();
    let listed = common::complete(rt, &mut io, Op::List { dir: String::new() }).result;
    match listed {
        Ok(Done::Listed(paths)) => paths.into_iter().filter(|p| is_aside(p)).collect(),
        other => panic!("{other:?}"),
    }
}

struct BackupOutcome {
    got: Result<(), Error>,
    published: bool,
    retried: Result<(), Error>,
    asides: Vec<String>,
}

/// Assert the retry's answer and what the published backup restores to.
fn assert_backup_outcome(published: bool, retried: &Result<(), Error>, files: Files, label: &str) {
    let mut expected = rows_upto(SEARCH_ROWS);
    if published {
        assert_eq!(&occupied(), retried, "{label}: the retry");
    } else {
        assert_eq!(&Ok(()), retried, "{label}: the retry");
        expected.insert(LATER_ROW);
    }
    assert_eq!(expected, restored_rows(files).unwrap(), "{label}");
}

/// Fail each op of a backup: the source lives, the target is absent or whole, and a retry answers
/// by which. After a failed read of the source, the source reopens before it takes a write and the
/// retry.
#[test]
fn a_backup_with_any_op_failed() {
    let workload = |rt: &SimRuntime| {
        let db = Database::open_in(rt.clone(), FILE, &Log::new([APP]), &no_create()).unwrap();
        let files_open = rt.files_open();
        rt.arm(searched());
        let got = backup_text(&db);
        rt.disarm();
        assert_eq!(files_open, rt.files_open(), "{:?}", rt.numbered().struck());
        let published = sim_has(rt, TARGET);
        let asides = asides_at_top(rt);
        let db = if rt.numbered().struck().is_some_and(reads_the_source) {
            db.close().unwrap();
            Database::open_in(rt.clone(), FILE, &Log::new([APP]), &no_create()).unwrap()
        } else {
            db
        };
        insert(&db, LATER_ROW..LATER_ROW + 1);
        let retried = backup_text(&db);
        db.close().unwrap();
        BackupOutcome {
            got,
            published,
            retried,
            asides,
        }
    };
    search::each_failure(
        &file_with_rows(SEARCH_ROWS),
        90,
        FAILURES,
        &workload,
        &mut |failed| {
            let label = format!("op {} {}", failed.at, failed.struck);
            let out = failed.out;
            if out.got == Ok(()) {
                assert!(
                    out.published,
                    "{label}: answered success with nothing published"
                );
            }
            if !stops_a_cleanup(&failed.struck) && out.got.is_err() {
                assert!(out.asides.is_empty(), "{label}: {:?}", out.asides);
            }
            assert_backup_outcome(out.published, &out.retried, failed.files, &label);
        },
    );
}

/// Crash a backup at each device event: the source recovers and takes commits, and the target is
/// absent or whole.
#[test]
fn a_backup_crashed_anywhere() {
    let workload = |rt: &SimRuntime| {
        let db = Database::open_in(rt.clone(), FILE, &Log::new([APP]), &no_create()).unwrap();
        rt.arm(searched());
        backup_text(&db).unwrap();
        rt.disarm();
        db.close().unwrap();
    };
    let files = file_with_rows(SEARCH_ROWS);
    search::each_crash(
        &files,
        91,
        CRASH_POINTS,
        CRASH_IMAGES,
        Box::new(workload),
        &|image| image.keys().any(|p| p.starts_with("bk/")),
        &mut |at, image| {
            let label = format!("event {at}");
            let published = image.keys().any(|p| p.starts_with("bk/"));
            let mut sim = Sim::with_files(at as u64, Config::default(), image);
            let retried = process::run(&mut sim, |rt| {
                let db = open::open_in(&rt, FILE, &Log::new([APP]), &no_create()).unwrap();
                assert_eq!(rows_upto(SEARCH_ROWS), ids(&db), "{label}");
                assert!(
                    db.testing().verify_holding().unwrap().next().is_none(),
                    "{label}"
                );
                if published {
                    storage::check_backup(&rt, TARGET).expect("what is published is whole");
                }
                insert(&db, LATER_ROW..LATER_ROW + 1);
                let retried = backup_text(&db);
                db.close().unwrap();
                retried
            });
            assert_backup_outcome(published, &retried, sim.files(), &label);
        },
    );
}

/// Start a second backup to one target at each op of a first: one wins, whole, and the loser leaves
/// nothing.
#[test]
fn two_backups_to_one_target() {
    let workload = |rt: &SimRuntime, at: Option<u64>| {
        let db = Database::open_in(rt.clone(), FILE, &Log::new([APP]), &no_create()).unwrap();
        let second = Arc::new(Mutex::new(None));
        let thread = Arc::new(Mutex::new(None));
        if let Some(at) = at {
            let (db, second, thread, spawner) =
                (db.clone(), second.clone(), thread.clone(), rt.clone());
            let body = move || {
                let run = move || {
                    let got = db_backup(&db, TARGET, &mut |_, _| true);
                    *second.lock().unwrap() = Some(got.map(|_| ()));
                };
                *thread.lock().unwrap() = Some(spawner.spawn("second backup", run));
            };
            rt.plan(Plan::new().at(at, Action::Run(Box::new(body))));
        }
        rt.arm(searched());
        let first = db_backup(&db, TARGET, &mut |_, _| true).map(|_| ());
        rt.disarm();
        if at.is_some() {
            let joined = thread.lock().unwrap().take();
            let joined = joined.expect("the second run started");
            rt.join(joined);
            let second = second.lock().unwrap().take().expect("the thread ended");
            let wins = usize::from(first.is_ok()) + usize::from(second.is_ok());
            assert_eq!(1, wins, "op {at:?}: {first:?} {second:?}");
            let loser = first.err().or(second.err());
            let occupied = BackupError::Occupied(TARGET.into());
            assert_eq!(Some(occupied), loser, "op {at:?}");
        }
        db.close().unwrap();
    };
    let files = file_with_rows(SEARCH_ROWS);
    search::each_start(&files, 92, STARTS, &workload, &mut |at, files| {
        assert!(!files.keys().any(|p| is_aside(p)), "op {at}");
        assert_eq!(
            rows_upto(SEARCH_ROWS),
            restored_rows(files).unwrap(),
            "op {at}"
        );
    });
}

/// Assert `files` holds `base` and nothing else, or that plus a whole restore, and that a retry
/// answers by which. A crash alone may leave the final log directory without the file.
fn assert_restore_state(base: &Files, files: &Files, crashed: bool, seed: u64, label: &str) {
    let finals = final_files(files);
    for (path, bytes) in final_files(base) {
        assert!(finals.get(&path) == Some(&bytes), "{label}: changed {path}");
    }
    let log_dir = format!("{COPY}-log/");
    for path in finals.keys().filter(|p| !base.contains_key(*p)) {
        assert!(
            path == COPY || path.starts_with(&log_dir),
            "{label}: stray {path}"
        );
    }
    let has_file = finals.contains_key(COPY);
    let has_log = finals.keys().any(|p| p.starts_with(&log_dir));
    assert!(!has_file || has_log, "{label}: a file without its log");
    assert!(
        has_file || !has_log || crashed,
        "{label}: a log directory after an answered failure"
    );
    let mut sim = Sim::with_files(seed, Config::default(), files.clone());
    process::run(&mut sim, |rt| {
        if has_file || has_log {
            let got = restore(&rt);
            let named = got == exists(COPY) || got == exists("copy.adt-log");
            assert!(named, "{label}: the retry: {got:?}");
        } else {
            assert_eq!(Ok(()), restore(&rt), "{label}: the retry");
        }
        if has_log && !has_file {
            return;
        }
        let db = open::open_in(&rt, COPY, &Log::new([APP]), &no_create()).unwrap();
        assert_eq!(rows_upto(SEARCH_ROWS), ids(&db), "{label}");
        assert!(
            db.testing().verify_holding().unwrap().next().is_none(),
            "{label}"
        );
        commit(&db, LATER_ROW);
        db.close().unwrap();
    });
}

fn restore_copy(rt: &SimRuntime) -> Result<(), Error> {
    let got = restore_in(
        rt.clone(),
        &archive(),
        COPY,
        RestorePoint::Latest,
        &no_create(),
    );
    got.map(|_| ())
}

/// Fail each op of a restore: nothing shows, or all of it, and a retry answers by which.
#[test]
fn a_restore_with_any_op_failed() {
    let base = backed_up(SEARCH_ROWS);
    let workload = |rt: &SimRuntime| {
        rt.arm(searched());
        let got = restore_copy(rt);
        rt.disarm();
        (got, rt.files_open())
    };
    search::each_failure(&base, 93, FAILURES, &workload, &mut |failed| {
        let label = format!("op {} {}", failed.at, failed.struck);
        let (got, files_open) = failed.out;
        assert_eq!(0, files_open, "{label}");
        match &got {
            Ok(()) => assert!(
                failed.files.contains_key(COPY),
                "{label}: answered success with nothing published"
            ),
            Err(Error::Fatal {
                reason: FatalReason::Storage { .. },
            }) => {}
            Err(e) => panic!("{label}: a failed op answered {e:?}, not a storage failure"),
        }
        if !stops_a_cleanup(&failed.struck) && got.is_err() {
            let left: Vec<_> = failed.files.keys().filter(|p| is_aside(p)).collect();
            assert!(left.is_empty(), "{label}: {left:?}");
        }
        assert_restore_state(&base, &failed.files, false, 93, &label);
    });
}

/// Crash a restore at each device event: the file's name is the commit point. An open with
/// `create` of a path whose log the crash left with no file is refused and leaves the log as it
/// was.
#[test]
fn a_restore_crashed_anywhere() {
    let base = backed_up(SEARCH_ROWS);
    let workload = |rt: &SimRuntime| {
        rt.arm(searched());
        restore_copy(rt).unwrap();
        rt.disarm();
    };
    let log_dir = format!("{COPY}-log");
    let mut between = 0;
    search::each_crash(
        &base,
        94,
        CRASH_POINTS,
        CRASH_IMAGES,
        Box::new(workload),
        &|image| image.contains_key(COPY),
        &mut |at, image| {
            assert_restore_state(&base, &image, true, at as u64, &format!("event {at}"));
            let segments = segments_of(&image, &log_dir);
            if final_files(&image).contains_key(COPY) || segments.is_empty() {
                return;
            }
            between += 1;
            let mut sim = Sim::with_files(at as u64, Config::default(), image);
            let opened = process::run(&mut sim, |rt| {
                Database::open_in(rt, COPY, &Log::new([APP]), &options()).map(Database::close)
            });
            let Err(crate::OpenError::Corrupt { detail }) = opened else {
                panic!("event {at}: {opened:?}");
            };
            let foreign = "the log directory holds a log that no file here wrote";
            assert_eq!(foreign, detail, "event {at}");
            assert!(
                segments == segments_of(&sim.files(), &log_dir),
                "event {at}: the refusal changed the log"
            );
        },
    );
    assert_reached("a crash between the renames", between, 1);
}

fn segments_of(files: &Files, log_dir: &str) -> Files {
    let log_dir = format!("{log_dir}/");
    let mut held = final_files(files);
    held.retain(|path, _| path.starts_with(&log_dir));
    held
}

/// Open with `create` through `open_in` at each op of a restore: one of them wins, and nobody
/// loses a log they were told is theirs.
#[test]
fn an_open_at_each_op_of_a_restore() {
    let (restore_won, open_won) = (AtomicU64::new(0), AtomicU64::new(0));
    let workload = |rt: &SimRuntime, at: Option<u64>| {
        let opened = Arc::new(Mutex::new(None));
        let thread = Arc::new(Mutex::new(None));
        if let Some(at) = at {
            let (other, opened, thread, spawner) =
                (rt.clone(), opened.clone(), thread.clone(), rt.clone());
            let body = move || {
                let run = move || {
                    let log = Log::new([APP]);
                    let got = Database::open_in(other, COPY, &log, &options())
                        .map(|db| db.close().unwrap());
                    *opened.lock().unwrap() = Some(got);
                };
                *thread.lock().unwrap() = Some(spawner.spawn("opener", run));
            };
            rt.plan(Plan::new().at(at, Action::Run(Box::new(body))));
        }
        rt.arm(searched());
        let restored = restore_copy(rt);
        rt.disarm();
        if at.is_some() {
            let joined = thread.lock().unwrap().take();
            let joined = joined.expect("the second run started");
            rt.join(joined);
            let opened = opened.lock().unwrap().take().expect("the thread ended");
            match (&restored, &opened) {
                (Ok(()), _) | (_, Ok(())) => {}
                _ => panic!("op {at:?}: neither wins: {restored:?} {opened:?}"),
            }
            let tally = if restored.is_ok() {
                &restore_won
            } else {
                &open_won
            };
            tally.fetch_add(1, Ordering::Relaxed);
            let db = open::open_in(rt, COPY, &Log::new([APP]), &no_create())
                .expect("the winner's database opens");
            let expected = if restored.is_ok() {
                rows_upto(SEARCH_ROWS)
            } else {
                BTreeSet::new()
            };
            assert_eq!(expected, ids(&db), "op {at:?}");
            db.close().unwrap();
        }
    };
    let base = backed_up(SEARCH_ROWS);
    search::each_start(&base, 95, STARTS, &workload, &mut |at, files| {
        assert!(!files.keys().any(|p| is_aside(p)), "op {at}");
    });
    let (restore, open) = (
        restore_won.load(Ordering::Relaxed) as usize,
        open_won.load(Ordering::Relaxed) as usize,
    );
    assert_reached("the restore winning", restore, 1);
    assert_reached("the open winning", open, 1);
}

fn with_rows_of_seed(seed: u64, rows: i64) -> Files {
    let mut sim = Sim::new(seed, Config::default());
    process::run(&mut sim, |rt| {
        let db = open::opened(&rt, FILE, &Log::new([APP]), &options());
        insert(&db, 0..rows);
        db.close().unwrap();
    });
    sim.files()
}

/// Refuse to publish a backup when the live log holds a block another file wrote.
#[test]
fn a_backup_refuses_a_block_of_another_file_in_the_log() {
    const BLOCK: usize = 512;
    let segment = format!("{FILE}-log/");
    let ours = with_rows_of_seed(7, 8);
    let theirs = with_rows_of_seed(8, 40);
    let (name, bytes) = ours
        .iter()
        .find(|(path, _)| path.starts_with(&segment))
        .expect("a segment");
    let last = bytes
        .chunks(BLOCK)
        .rposition(|block| block.iter().any(|&b| b != 0))
        .expect("a written block")
        * BLOCK;
    let foreign = theirs[name.as_str()][last..last + BLOCK].to_vec();
    let mut sim = Sim::with_files(10, Config::default(), ours.clone());
    process::run(&mut sim, |rt| {
        let db = open::open_in(&rt, FILE, &Log::new([APP]), &no_create()).unwrap();
        let mut io = rt.io();
        let file = common::open(&rt, &mut io, name, false).unwrap().0;
        common::write(&rt, &mut io, file, last as u64, &foreign).unwrap();
        let text = format!("backup to {TARGET:?}");
        let at = super::parse_segment(name).expect("a segment name") + last as u64;
        let detail = format!("`log block at {at}`: Failed");
        adtb_testkit::refused_as(
            db.query_text(&text, &[], &CallOptions::default()),
            &Error::Fatal {
                reason: FatalReason::Storage {
                    detail: detail.clone(),
                },
            },
            &format!("fatal: storage failure: {detail}"),
        );
        assert_eq!(
            Some(BackupError::Absent(TARGET.into())),
            storage::read_manifest(&rt, TARGET).err(),
            "nothing was published"
        );
        drop(db);
    });
}

/// Start a second restore to one path at each op of a first: one wins, whole.
#[test]
fn two_restores_to_one_path() {
    let workload = |rt: &SimRuntime, at: Option<u64>| {
        let second = Arc::new(Mutex::new(None));
        let thread = Arc::new(Mutex::new(None));
        if let Some(at) = at {
            let (other, second, thread, spawner) =
                (rt.clone(), second.clone(), thread.clone(), rt.clone());
            let body = move || {
                let run = move || {
                    *second.lock().unwrap() = Some(restore_copy(&other));
                };
                *thread.lock().unwrap() = Some(spawner.spawn("second restore", run));
            };
            rt.plan(Plan::new().at(at, Action::Run(Box::new(body))));
        }
        rt.arm(searched());
        let first = restore_copy(rt);
        rt.disarm();
        if at.is_some() {
            let joined = thread.lock().unwrap().take();
            let joined = joined.expect("the second run started");
            rt.join(joined);
            let second = second.lock().unwrap().take().expect("the thread ended");
            match (&first, &second) {
                (Ok(()), Err(_)) | (Err(_), Ok(())) => {}
                _ => panic!("op {at:?}: not exactly one wins: {first:?} {second:?}"),
            }
            let lost = first.and(second);
            assert!(
                lost == exists(COPY) || lost == exists("copy.adt-log"),
                "op {at:?}: {lost:?}"
            );
        }
    };
    let base = backed_up(SEARCH_ROWS);
    search::each_start(&base, 95, STARTS, &workload, &mut |at, files| {
        assert!(!files.keys().any(|p| is_aside(p)), "op {at}");
        assert_restore_state(&base, &files, false, at, &format!("op {at}"));
    });
}

/// Assert `files` holds `base`, with a whole proof or none, and a retry proves it and it restores.
fn assert_prove_state(base: &Files, files: &Files, seed: u64, label: &str) {
    let finals = final_files(files);
    let proof = format!("{TARGET}/{}", storage::PROOF);
    let mut without = finals;
    let had_proof = without.remove(&proof).is_some();
    assert!(
        without == final_files(base),
        "{label}: a change outside the proof: {:?}",
        without.keys().collect::<Vec<_>>()
    );
    let mut sim = Sim::with_files(seed, Config::default(), files.clone());
    process::run(&mut sim, |rt| {
        let expected = if had_proof {
            Proof::Proven
        } else {
            Proof::Unproven { function: None }
        };
        assert_eq!(expected, proof_in(&rt, &archive()).unwrap(), "{label}");
        let log = Log::new([APP]);
        let got = prove_in(rt.clone(), &archive(), &log, &no_create());
        assert_eq!(Ok(Proof::Proven), got, "{label}: the retry");
        restore(&rt).unwrap();
    });
}

fn prove_archive(rt: &SimRuntime) -> Result<Proof, Error> {
    prove_in(rt.clone(), &archive(), &Log::new([APP]), &no_create())
}

/// Fail each op of a prove: the backup stays, and no proof but a true one shows.
#[test]
fn a_prove_with_any_op_failed() {
    let base = backed_up(SEARCH_ROWS);
    let workload = |rt: &SimRuntime| {
        rt.arm(searched().except(on_the_test_database(Match::any())));
        let got = prove_archive(rt);
        rt.disarm();
        (got, rt.files_open())
    };
    search::each_failure(&base, 96, FAILURES, &workload, &mut |failed| {
        let label = format!("op {} {}", failed.at, failed.struck);
        let (got, files_open) = failed.out;
        assert_eq!(0, files_open, "{label}");
        if !stops_a_cleanup(&failed.struck) && got.is_err() {
            let left: Vec<_> = failed.files.keys().filter(|p| is_aside(p)).collect();
            assert!(left.is_empty(), "{label}: {left:?}");
        }
        assert_prove_state(&base, &failed.files, 96, &label);
    });
}

/// Answer a failed read of the test database as a storage failure, and record no verdict.
#[test]
#[ignore = "an open answers an unreadable log as `Corrupt` with text only, so a prove records a failed proof; a typed cause of `Corrupt` removes this"]
fn a_failed_read_of_the_test_database_records_no_verdict() {
    let base = backed_up(SEARCH_ROWS);
    let workload = |rt: &SimRuntime| {
        rt.arm(on_the_test_database(Match::kind(OpKind::Read)));
        let got = prove_archive(rt);
        rt.disarm();
        got
    };
    search::each_failure(&base, 96, FAILURES, &workload, &mut |failed| {
        let label = format!("op {} {}", failed.at, failed.struck);
        if let Ok(proof) = &failed.out {
            assert_eq!(&Proof::Proven, proof, "{label}: a false verdict");
        }
    });
}

/// Crash a prove at each device event: a whole proof or none, and the backup untouched.
#[test]
fn a_prove_crashed_anywhere() {
    let base = backed_up(SEARCH_ROWS);
    let workload = |rt: &SimRuntime| {
        rt.arm(searched());
        assert_eq!(Ok(Proof::Proven), prove_archive(rt));
        rt.disarm();
    };
    let proof = format!("{TARGET}/{}", storage::PROOF);
    let mut retried = BTreeSet::new();
    let mut with_aside = 0;
    search::each_crash(
        &base,
        97,
        CRASH_POINTS,
        CRASH_IMAGES,
        Box::new(workload),
        &|image| image.contains_key(&proof),
        &mut |at, image| {
            let names: BTreeSet<String> = image.keys().cloned().collect();
            if retried.insert(names.clone()) || every_case() {
                if names.iter().any(|p| p.contains(".partial-")) {
                    with_aside += 1;
                }
                assert_prove_state(&base, &image, at as u64, &format!("event {at}"));
            }
        },
    );
    assert_reached("an image retried", retried.len(), 4);
    assert_reached("an image holding a scratch", with_aside, 2);
}

/// Start a second prove at each op of a first: neither sees the other's scratch, and both prove it.
#[test]
fn two_proves_of_one_backup() {
    let workload = |rt: &SimRuntime, at: Option<u64>| {
        let second = Arc::new(Mutex::new(None));
        let thread = Arc::new(Mutex::new(None));
        if let Some(at) = at {
            let (other, second, thread, spawner) =
                (rt.clone(), second.clone(), thread.clone(), rt.clone());
            let body = move || {
                let run = move || {
                    *second.lock().unwrap() = Some(prove_archive(&other));
                };
                *thread.lock().unwrap() = Some(spawner.spawn("second prove", run));
            };
            rt.plan(Plan::new().at(at, Action::Run(Box::new(body))));
        }
        rt.arm(searched());
        let first = prove_archive(rt);
        rt.disarm();
        assert_eq!(Ok(Proof::Proven), first, "op {at:?}");
        if at.is_some() {
            let joined = thread.lock().unwrap().take();
            let joined = joined.expect("the second run started");
            rt.join(joined);
            let second = second.lock().unwrap().take().expect("the thread ended");
            assert_eq!(Ok(Proof::Proven), second, "op {at:?}");
        }
        assert_eq!(Proof::Proven, proof_in(rt, &archive()).unwrap());
    };
    let base = backed_up(SEARCH_ROWS);
    search::each_start(&base, 98, STARTS, &workload, &mut |at, files| {
        assert!(!files.keys().any(|p| is_aside(p)), "op {at}");
        let mut finals = final_files(&files);
        finals.remove(&format!("{TARGET}/{}", storage::PROOF));
        assert!(
            finals == final_files(&base),
            "op {at}: a change outside the proof"
        );
    });
}

/// Show no restored file under its final name while the restore can still fail.
#[test]
fn a_restore_that_fails_its_check_never_shows_a_file() {
    let mut sim = Sim::with_files(80, Config::default(), backed_up(2));
    process::run(&mut sim, |rt| {
        let names_the_file = Match::any_of([
            Match::kind(OpKind::Open).creating(),
            Match::kinds([OpKind::CreateNew, OpKind::Rename]),
        ]);
        let shown = rt.counted(names_the_file.path(COPY));
        rt.plan(fail_reads_after_open(
            &rt,
            Match::kind(OpKind::Open).prefixed("copy.adt-log"),
        ));
        let (manifest, _) = storage::read_manifest(&rt, TARGET).unwrap();
        let start = manifest.start.lsn.0;
        let first = start - start % u64::from(manifest.header.log_block);
        let detail = format!("`log block at {first}`: Failed");
        adtb_testkit::refused_as(
            restore_in(rt, &archive(), COPY, RestorePoint::Latest, &no_create()),
            &Error::Fatal {
                reason: FatalReason::Storage {
                    detail: detail.clone(),
                },
            },
            &format!("fatal: storage failure: {detail}"),
        );
        assert_eq!(
            0,
            shown.get(),
            "an op put a file another thread could open at {COPY}"
        );
    });
}
