//! Full backups of a database under writers, restores of them to new files, and the test restore
//! that proves one.

use adtb::testing::Plant;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use adtb::runtime::{Done, IoError, Op, OpKind, Runtime, Threads};
use adtb::storage::{
    self, BackupError, BackupRun, Damage, Envelope, Lsn, Manifest, Options, PageKind, Pager, Pid,
    Quarantine,
};
use adtb::{
    ArchiveTarget, CallOptions, CancelToken, Capability, Database, Determinism, Error, FatalReason,
    HostFns, Log, OpenOptions, Position, Proof, RejectReason, RestorePoint, TxnCallOptions,
    TxnOptions, proof_in, prove_in, restore_in,
};
use adtb_sim::{Action, Config, Match, Outcome, Plan, Sim, SimRuntime, Struck};
use adtb_simkit::search::{self, Files};
use adtb_simkit::{api, common};
use adtb_sys::runtime::Faults;
use adtb_testkit::profile::{assert_reached, every_case, sweep};

const FILE: &str = "backup.adt";
const TARGET: &str = "bk";
const COPY: &str = "copy.adt";
const APP: &str = r"module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
";

fn options() -> OpenOptions {
    api::options(2)
}

fn no_create() -> OpenOptions {
    OpenOptions {
        create: None,
        ..api::options(2)
    }
}

fn open(rt: SimRuntime) -> Database<SimRuntime> {
    Database::open_in(rt, FILE, &Log::new([APP]), &options()).unwrap()
}

fn open_at(rt: SimRuntime, name: &str) -> Result<Database<SimRuntime>, adtb::OpenError> {
    Database::open_in(rt, name, &Log::new([APP]), &no_create())
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
    api::run(&mut sim, |rt| {
        let db = open(rt);
        insert(&db, 0..rows);
        backup_text(&db).unwrap();
        db.close().unwrap();
    });
    sim.files()
}

fn restored_rows(files: Files) -> Result<BTreeSet<i64>, Error> {
    let mut sim = Sim::with_files(2, Config::default(), files);
    api::run(&mut sim, |rt| {
        restore(&rt)?;
        let db = open_at(rt, COPY).unwrap();
        let rows = ids(&db);
        db.close().unwrap();
        Ok(rows)
    })
}

fn damaged_part(got: Result<(), Error>) -> String {
    match got {
        Err(Error::Rejected {
            reason: RejectReason::BackupDamaged { part },
        }) => part,
        other => panic!("not BackupDamaged: {other:?}"),
    }
}

fn is_invalid_naming(got: &Result<(), Error>, word: &str) {
    match got {
        Err(e) if api::is_invalid(e) && e.to_string().contains(word) => {}
        other => panic!("not an invalid naming {word}: {other:?}"),
    }
}

fn file_with_rows(rows: i64) -> Files {
    let mut sim = Sim::new(1, Config::default());
    api::run(&mut sim, |rt| {
        let db = open(rt);
        insert(&db, 0..rows);
        db.close().unwrap();
    });
    sim.files()
}

#[test]
fn backup_under_writers_restores_every_commit_acked_before_start_and_none_after_end() {
    for seed in sweep(0..2) {
        let acked: Arc<Mutex<Vec<(i64, Position)>>> = Arc::default();
        let manifest: Arc<Mutex<Option<Manifest>>> = Arc::default();
        let mut sim = Sim::new(seed, Config::default());
        let (a, m) = (acked.clone(), manifest.clone());
        api::run(&mut sim, move |rt| {
            let db = open(rt.clone());
            let before: Vec<_> = (0..16).map(|id| (id, commit(&db, id))).collect();
            a.lock().unwrap().extend(before);
            let stop = Arc::new(AtomicBool::new(false));
            let (w, s, shared, clock) = (db.clone(), stop.clone(), a.clone(), rt.clone());
            let writer = rt.spawn("writer", move || {
                let mut id = 16;
                while !s.load(Ordering::Relaxed) {
                    let at = commit(&w, id);
                    shared.lock().unwrap().push((id, at));
                    id += 1;
                    clock.yield_now();
                }
            });
            let got = db_backup(&db, TARGET, &mut |_, _| {
                rt.yield_now();
                true
            })
            .unwrap();
            stop.store(true, Ordering::Relaxed);
            rt.join(writer);
            *m.lock().unwrap() = Some(got);
            db.close().unwrap();
        });
        let end = manifest.lock().unwrap().clone().unwrap().end;
        let rows = restored_rows(sim.files()).unwrap();
        for (id, at) in acked.lock().unwrap().iter() {
            assert_eq!(
                at.lsn < end.lsn.0,
                rows.contains(id),
                "seed {seed}: commit {id} at {at:?}, end {end:?}"
            );
        }
    }
}

#[test]
fn restored_equals_ledger_at_end_position() {
    let mut sim = Sim::new(5, Config::default());
    let mut ledger = BTreeMap::new();
    let mut end = None;
    api::run(&mut sim, |rt| {
        let db = open(rt);
        for id in 0..20 {
            ledger.insert(id, commit(&db, id));
        }
        let mut next = 20;
        let got = db_backup(&db, TARGET, &mut |_, _| {
            ledger.insert(next, commit(&db, next));
            next += 1;
            true
        })
        .unwrap();
        end = Some(got.end);
        for id in next..next + 5 {
            ledger.insert(id, commit(&db, id));
        }
        db.close().unwrap();
    });
    let end = end.unwrap();
    let expected: BTreeSet<i64> = ledger
        .iter()
        .filter(|(_, at)| at.lsn < end.lsn.0)
        .map(|(id, _)| *id)
        .collect();
    assert!(expected.len() > 20, "commits ran during the copy");
    assert!(expected.len() < ledger.len(), "commits ran past the end");
    assert_eq!(expected, restored_rows(sim.files()).unwrap());
}

#[test]
fn restored_verify_clean_all_checks() {
    let mut sim = Sim::with_files(6, Config::default(), backed_up(40));
    api::run(&mut sim, |rt| {
        assert_eq!(
            Proof::Proven,
            prove_in(rt.clone(), &archive(), &Log::new([APP]), &no_create()).unwrap()
        );
        restore(&rt).unwrap();
        let db = open_at(rt, COPY).unwrap();
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
    api::run(&mut sim, |rt| {
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
fn commits_not_blocked_during_backup() {
    let mut sim = Sim::new(7, Config::default());
    api::run(&mut sim, |rt| {
        let db = open(rt);
        insert(&db, 0..8);
        let mut next = 8;
        db_backup(&db, TARGET, &mut |_, _| {
            commit(&db, next);
            next += 1;
            true
        })
        .unwrap();
        assert!(next > 8, "the copy visited a page");
        db.close().unwrap();
    });
}

#[test]
fn page_torn_mid_copy_retried_under_latch() {
    let mut sim = Sim::new(8, Config::default());
    let mut torn = 0;
    let mut got = None;
    api::run(&mut sim, |rt| {
        let db = open(rt);
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
    let pid = api::run(&mut sim, |rt| {
        let db = open(rt);
        let t = db.testing();
        let pid = t.pages(t.heap("app::users", "User").unwrap()).unwrap()[0];
        db.close().unwrap();
        pid
    });
    *files = sim.files();
    let data = files.get_mut(FILE).unwrap();
    data[pid.0 as usize * 8192 + 200] ^= 0xff;
    pid
}

#[test]
fn rotted_page_listed_in_manifest_and_proof_failed() {
    let mut files = file_with_rows(4);
    let pid = rot_users_page(&mut files);
    let mut sim = Sim::with_files(10, Config::default(), files);
    api::run(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &Options::new(FILE)).unwrap();
        let got = backup_with(&pager, TARGET, &mut |_, _| true).unwrap();
        assert_eq!(vec![pid], got.damaged);
        pager.close();
        match prove_in(rt, &archive(), &Log::new([APP]), &no_create()).unwrap() {
            Proof::Failed(found) => {
                assert!(found[0].contains(&format!("page {}", pid.0)), "{found:?}");
            }
            other => panic!("{other:?}"),
        }
    });
}

#[test]
fn header_names_backup_checkpoint_not_later_one() {
    let mut sim = Sim::new(11, Config::default());
    let mut got = None;
    api::run(&mut sim, |rt| {
        let db = open(rt);
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
    api::run(&mut sim, |rt| {
        let db = open(rt);
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
    api::run(&mut restored, |rt| {
        restore(&rt).unwrap();
        let db = open_at(rt, COPY).unwrap();
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
    api::run(&mut sim, |rt| {
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

#[test]
fn torn_part_refused_naming_part() {
    let mut files = backed_up(4);
    let part = part_named(&files, "pages/");
    files.get_mut(&format!("{TARGET}/{part}")).unwrap()[5000] ^= 1;
    let mut sim = Sim::with_files(15, Config::default(), files);
    api::run(&mut sim, |rt| {
        assert_eq!(part, damaged_part(restore(&rt)));
        assert_eq!(
            Err(adtb::OpenError::NotFound.to_string()),
            open_at(rt, COPY).map(|_| ()).map_err(|e| e.to_string()),
            "a refused restore writes nothing"
        );
    });
}

#[test]
fn missing_part() {
    let mut files = backed_up(4);
    let part = part_named(&files, "log/");
    files.remove(&format!("{TARGET}/{part}"));
    let mut sim = Sim::with_files(16, Config::default(), files);
    api::run(&mut sim, |rt| assert_eq!(part, damaged_part(restore(&rt))));
}

#[test]
fn bad_manifest_checksum() {
    let mut files = backed_up(4);
    let at = format!("{TARGET}/{}", storage::MANIFEST);
    files.get_mut(&at).unwrap()[30] ^= 1;
    let mut sim = Sim::with_files(17, Config::default(), files);
    api::run(&mut sim, |rt| {
        assert_eq!(storage::MANIFEST, damaged_part(restore(&rt)));
    });
}

#[test]
fn unknown_format() {
    let mut files = backed_up(4);
    let at = format!("{TARGET}/{}", storage::MANIFEST);
    let text = String::from_utf8(files[&at].clone()).unwrap();
    let text = text.trim_end_matches('\0');
    let body = text
        .rsplit_once("checksum ")
        .unwrap()
        .0
        .replace("format 1\n", "format 9\n");
    let sealed = format!("{body}checksum {}\n", storage::manifest_hash(&body));
    files.insert(at, sealed.into_bytes());
    let mut sim = Sim::with_files(18, Config::default(), files);
    api::run(&mut sim, |rt| is_invalid_naming(&restore(&rt), "format 9"));
}

#[test]
fn segments_covering_redo_retained_through_backup() {
    let mut sim = Sim::new(19, Config::default());
    api::run(&mut sim, |rt| {
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
        let (copy, recovery) = Pager::open(rt, &pager_options(COPY, false, 1 << 14)).unwrap();
        assert_eq!(manifest.end.lsn, recovery.end);
        assert_eq!(Vec::<(Pid, Damage)>::new(), recovery.damaged);
        let found = copy.verify().findings;
        assert!(found.is_empty(), "{found:?}");
        assert_ne!(0, copy.read(pid).unwrap()[101]);
        copy.close();
    });
}

#[test]
fn restore_bumps_epoch_and_commit_continues() {
    let mut sim = Sim::new(20, Config::default());
    api::run(&mut sim, |rt| {
        let db = open(rt.clone());
        insert(&db, 0..4);
        let before = commit(&db, 4);
        backup_text(&db).unwrap();
        db.close().unwrap();
        restore(&rt).unwrap();
        let copy = open_at(rt.clone(), COPY).unwrap();
        let after = commit(&copy, 5);
        let step = after.epoch - before.epoch;
        assert!((1..=1 << 32).contains(&step), "{step}");
        assert!(after.lsn > before.lsn);
        assert_eq!(6, ids(&copy).len());
        copy.close().unwrap();
        let copy = open_at(rt, COPY).unwrap();
        assert_eq!(6, ids(&copy).len());
        copy.close().unwrap();
    });
}

#[test]
fn old_position_past_end_refused() {
    let mut sim = Sim::new(21, Config::default());
    api::run(&mut sim, |rt| {
        let db = open(rt.clone());
        insert(&db, 0..2);
        let inside = commit(&db, 2);
        let manifest = db_backup(&db, TARGET, &mut |_, _| true).unwrap();
        let lost = commit(&db, 3);
        assert!(lost.lsn >= manifest.end.lsn.0);
        db.close().unwrap();
        restore(&rt).unwrap();
        let copy = open_at(rt, COPY).unwrap();
        let read_after = |after| {
            copy.query_text(
                "from u in users select u.id",
                &[],
                &CallOptions {
                    after: Some(after),
                    ..CallOptions::default()
                },
            )
        };
        assert!(read_after(inside).is_ok(), "the position is on the history");
        match read_after(lost) {
            Err(Error::Rejected {
                reason: RejectReason::PositionNotOnHistory { position },
            }) => assert_eq!(lost, position),
            other => panic!("{other:?}"),
        }
        copy.close().unwrap();
    });
}

#[test]
fn restore_at_point_refused_with_range() {
    let mut sim = Sim::with_files(22, Config::default(), backed_up(2));
    api::run(&mut sim, |rt| {
        let (manifest, _) = storage::read_manifest(&rt, TARGET).unwrap();
        let end = Position::from(manifest.end);
        let refused = |got: Result<(), Error>| match got {
            Err(Error::Rejected {
                reason: RejectReason::RestorePointUnavailable { oldest, newest },
            }) => assert_eq!((end, end), (oldest, newest)),
            other => panic!("{other:?}"),
        };
        let refused_at = |point| {
            refused(restore_in(rt.clone(), &archive(), COPY, point, &no_create()).map(|_| ()));
        };
        refused_at(RestorePoint::Version(1));
        refused_at(RestorePoint::Position(Position {
            lsn: end.lsn - 1,
            ..end
        }));
        let db = open_at(rt, FILE).unwrap();
        for text in [
            format!("restore from \"{TARGET}\" to \"{COPY}\" before version 1"),
            format!("restore from \"{TARGET}\" to \"{COPY}\" at position {{ epoch: 1, lsn: 1 }}"),
        ] {
            refused(
                db.query_text(&text, &[], &CallOptions::default())
                    .map(|_| ()),
            );
        }
        db.close().unwrap();
    });
}

#[test]
fn s3_refused_by_name() {
    api::once(23, |rt| {
        let db = open(rt);
        for text in [
            "backup to \"s3://bucket/app\"",
            "restore from \"s3://bucket/app\" to \"x.adt\"",
        ] {
            let got = db
                .query_text(text, &[], &CallOptions::default())
                .map(|_| ());
            is_invalid_naming(&got, "s3");
        }
        is_invalid_naming(&ArchiveTarget::path("s3://bucket/app").map(|_| ()), "s3");
        db.close().unwrap();
    });
}

#[test]
fn restore_over_existing_refused() {
    let mut sim = Sim::with_files(24, Config::default(), backed_up(2));
    api::run(&mut sim, |rt| {
        restore(&rt).unwrap();
        is_invalid_naming(&restore(&rt), COPY);
        let over_file = restore_in(rt, &archive(), FILE, RestorePoint::Latest, &no_create());
        is_invalid_naming(&over_file.map(|_| ()), FILE);
    });
}

#[test]
fn backup_to_nonempty_refused() {
    let mut sim = Sim::with_files(25, Config::default(), backed_up(2));
    api::run(&mut sim, |rt| {
        let db = open_at(rt, FILE).unwrap();
        is_invalid_naming(&backup_text(&db), TARGET);
        db.close().unwrap();
    });
}

/// A role needs `maintain` and `write_files`, and a role's target stays inside the export roots.
#[test]
fn backup_needs_write_files_over_server() {
    api::once(26, |rt| {
        let db = open(rt.clone());
        insert(&db, 0..2);
        let authority = db.role_authority();
        authority.create_role("ops").unwrap();
        authority
            .grant_capability("ops", Capability::Adhoc, false)
            .unwrap();
        authority
            .grant_capability("ops", Capability::Maintain, false)
            .unwrap();
        let ops = authority.session("ops").unwrap();
        let denied = |got: Result<(), Error>| match got {
            Err(Error::Rejected {
                reason: RejectReason::Denied { detail },
            }) => detail,
            other => panic!("{other:?}"),
        };
        let lacking = denied(api::task(&rt, &ops, "backup to \"role-bk\""));
        assert!(!lacking.contains("export_roots"), "{lacking}");
        authority
            .grant_capability("ops", Capability::WriteFiles, false)
            .unwrap();
        let outside = denied(api::task(&rt, &ops, "backup to \"role-bk\""));
        assert!(outside.contains("export_roots"), "{outside}");
        db.close().unwrap();
    });
}

/// A role's backup runs inside the export roots and is refused outside them.
#[test]
fn a_role_backs_up_inside_the_export_roots() {
    api::once(26, |rt| {
        let opts = OpenOptions {
            file_root: Some("data".into()),
            export_roots: vec!["out".into()],
            ..options()
        };
        let db = Database::open_in(rt.clone(), FILE, &Log::new([APP]), &opts).unwrap();
        insert(&db, 0..2);
        let authority = db.role_authority();
        authority.create_role("ops").unwrap();
        for capability in [
            Capability::Adhoc,
            Capability::Maintain,
            Capability::WriteFiles,
        ] {
            authority
                .grant_capability("ops", capability, false)
                .unwrap();
        }
        let ops = authority.session("ops").unwrap();
        api::task(&rt, &ops, "backup to \"out/role-bk\"").unwrap();
        storage::check_backup(&rt, "data/out/role-bk").unwrap();
        let outside = api::task(&rt, &ops, "backup to \"role-bk\"");
        assert!(
            matches!(&outside, Err(Error::Rejected { reason: RejectReason::Denied { detail } })
                if detail.contains("export_roots")),
            "{outside:?}"
        );
        let beyond = api::task(&rt, &ops, "backup to \"out/../role-bk\"");
        assert!(beyond.is_err(), "{beyond:?}");
        let root = api::task(&rt, &ops, "backup to \"out\"");
        assert!(
            matches!(&root, Err(Error::Rejected { reason: RejectReason::Denied { detail } })
                if detail.contains("export_roots")),
            "{root:?}"
        );
        db.close().unwrap();
    });
}

/// A read-only check answers a log it cannot list as a finding, with no panic.
#[test]
fn a_check_reports_a_log_it_cannot_read() {
    let mut sim = Sim::with_files(28, Config::default(), file_with_rows(2));
    api::run(&mut sim, |rt| {
        let db = Database::open_in(rt.clone(), FILE, &Log::new([APP]), &no_create()).unwrap();
        let lists = Match::kind(OpKind::List).prefixed(FILE);
        rt.plan(Plan::new().rule(lists, Action::Fail(IoError::Failed)));
        let t = db.testing();
        assert!(t.probe().logged_commits().is_empty());
        let found = t.verify_pages().findings;
        assert!(
            found.iter().any(|f| matches!(
                f,
                storage::Finding::Log(l) if l.problem == storage::LogProblem::Unreadable
            )),
            "{found:?}"
        );
        rt.plan(Plan::new());
        db.close().unwrap();
    });
}

#[test]
fn backup_of_quarantined_heap_lists_it() {
    api::once(27, |rt| {
        let db = open(rt);
        insert(&db, 0..2);
        let t = db.testing();
        let heap = t.mark_of(t.heap("app::users", "User").unwrap()).unwrap();
        t.plant(Plant::Mark(heap)).unwrap();
        let rows = db
            .query_text("backup to \"bk\"", &[], &CallOptions::default())
            .unwrap();
        let shown: Vec<String> = rows.into_iter().flat_map(api::printed).collect();
        assert!(shown[0].contains("quarantined app::users"), "{shown:?}");
        db.close().unwrap();
    });
}

#[test]
fn backup_of_quarantined_database_refused() {
    api::once(28, |rt| {
        let db = open(rt);
        db.testing()
            .plant(Plant::Mark(Quarantine::Database))
            .unwrap();
        match backup_text(&db) {
            Err(Error::Fatal {
                reason: FatalReason::Quarantined { .. },
                ..
            }) => {}
            other => panic!("{other:?}"),
        }
        db.close().unwrap();
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
    api::run(&mut sim, |rt| {
        let mut fns = HostFns::new();
        fns.register(
            "boom",
            "1.0",
            Determinism::Deterministic,
            |b: &adtb::batch::Batch| Ok(b.clone()),
        );
        let opening = OpenOptions {
            host_fns: fns,
            ..api::options(2)
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
        assert!(matches!(
            proof_in(&rt, &archive()).unwrap(),
            Proof::Unproven { function: Some(_) }
        ));
    });
}

#[test]
fn cancel_leaves_no_manifest() {
    let mut sim = Sim::new(30, Config::default());
    api::run(&mut sim, |rt| {
        let db = open(rt.clone());
        insert(&db, 0..2);
        let cancel = CancelToken::new();
        cancel.cancel();
        let cancelled = db.query_text(
            "backup to \"early\"",
            &[],
            &CallOptions {
                cancel: Some(cancel),
                ..CallOptions::default()
            },
        );
        assert!(
            matches!(
                cancelled,
                Err(Error::Rejected {
                    reason: RejectReason::Cancelled
                })
            ),
            "{cancelled:?}"
        );
        let mut seen = 0;
        let got = db_backup(&db, TARGET, &mut |_, _| {
            seen += 1;
            seen < 2
        });
        assert_eq!(Some(BackupError::Stopped), got.err());
        assert!(matches!(
            storage::check_backup(&rt, TARGET),
            Err(BackupError::Absent(_))
        ));
        assert!(
            !sim_has(&rt, TARGET),
            "a stop removes the parts and their directories"
        );
        backup_text(&db).expect("a stopped backup's target takes another");
        db.close().unwrap();
    });
    assert_eq!(2, restored_rows(sim.files()).unwrap().len());
}

/// Keep every commit acknowledged under checkpoints and page flushes of other threads across a
/// crash.
#[test]
fn first_checkpoint_under_live_writers_keeps_every_acked_commit() {
    for seed in sweep(40..42) {
        let acked: Arc<Mutex<Vec<i64>>> = Arc::default();
        let mut sim = Sim::new(
            seed,
            Config {
                faults: Faults {
                    torn_p: 0.5,
                    ..Faults::none(512)
                },
                ..Config::default()
            },
        );
        api::run(&mut sim, |rt| open(rt).close().unwrap());
        sim.crash_at(sim.now() + std::time::Duration::from_millis(300));
        let shared = acked.clone();
        let outcome = sim.run(move |rt| {
            let db = open(rt.clone());
            let threads: Vec<_> = (0..2)
                .map(|w| {
                    let (db, acked) = (db.clone(), shared.clone());
                    rt.spawn("writer", move || {
                        let mut id = w;
                        loop {
                            commit(&db, id);
                            acked.lock().unwrap().push(id);
                            id += 2;
                        }
                    })
                })
                .collect();
            assert_eq!(2, threads.len());
            let other = db.clone();
            rt.spawn("flusher", move || {
                loop {
                    other.testing().flush_pages(usize::MAX);
                }
            });
            loop {
                db.testing().flush_pages(usize::MAX);
                db.testing().checkpoint();
                rt.yield_now();
            }
        });
        assert_eq!(Outcome::Crashed, outcome, "seed {seed}");
        let acked: Vec<i64> = acked.lock().unwrap().clone();
        assert!(!acked.is_empty(), "seed {seed}: no commit before the crash");
        api::run(&mut sim, |rt| {
            let db = open_at(rt, FILE).unwrap();
            let rows = ids(&db);
            for id in &acked {
                assert!(rows.contains(id), "seed {seed}: commit {id} lost");
            }
            assert_eq!(
                0,
                db.testing().verify_holding().unwrap().count(),
                "seed {seed}"
            );
            db.close().unwrap();
        });
    }
}

/// Keep a page changed and written while another thread's force of the data file runs, through a
/// checkpoint whose header reaches the device ahead of that page.
#[test]
fn page_written_during_a_force_stays_dirty_through_a_checkpoint() {
    let options = pager_options(FILE, false, 1 << 20);
    let mut sim = Sim::new(31, Config::default());
    sim.record();
    sim.crash_at(sim.now() + std::time::Duration::from_secs(1));
    let mut changed = None;
    let updated = Arc::new(AtomicBool::new(false));
    let published = sim.logged(Match::kind(OpKind::Rename).path(FILE));
    let mut logs = None;
    let outcome = sim.run(|rt| {
        let pager = Arc::new(Pager::create(rt.clone(), &options).unwrap());
        let data = published.entries()[0].from().unwrap().to_owned();
        let pid = pager.allocate(1, PageKind::HeapSlotted, 7).unwrap();
        changed = Some(pid);
        pager.update(&[pid], |v| v[0][100] = 1).unwrap();
        pager.flush_log();
        let second = pager.clone();
        let (spawner, first, marked, seen) =
            (rt.clone(), rt.clone(), updated.clone(), updated.clone());
        let during = Action::Run(Box::new(move || {
            let (waker, waiter) = (first.clone(), first.current());
            spawner.spawn("second force", move || {
                second.update(&[pid], |v| v[0][100] = 2).unwrap();
                marked.store(true, Ordering::SeqCst);
                waker.unpark(waiter);
                second.flush_log();
                second.flush_pages(usize::MAX);
            });
            while !seen.load(Ordering::SeqCst) {
                first.park();
            }
        }));
        let data_forces = Match::kind(OpKind::Sync).path(&data);
        let page = pid.0 * u64::from(options.page_size);
        logs = Some((
            rt.logged(data_forces.clone()),
            rt.logged(Match::kind(OpKind::Write).path(&data).covers(page)),
            page,
        ));
        rt.plan(
            Plan::new()
                .first(data_forces.clone(), during)
                .rule(data_forces, Action::Hold),
        );
        rt.spawn("first force", move || {
            pager.flush_pages(usize::MAX);
            pager.checkpoint();
        });
        let mut io = rt.io();
        common::sleep(&rt, &mut io, std::time::Duration::from_secs(10));
        unreachable!("the crash comes first");
    });
    assert_eq!(Outcome::Crashed, outcome);
    assert!(
        updated.load(Ordering::SeqCst),
        "the page changed during the first force"
    );
    let (forces, writes, page) = logs.expect("the run planned");
    let forces = forces.entries();
    assert!(
        forces.len() >= 2,
        "a later force of the data file was held: {forces:?}"
    );
    let first = &forces[0];
    let changed_write = writes
        .entries()
        .into_iter()
        .find(|w| {
            let at = (page - w.offset().unwrap()) as usize;
            w.bytes().unwrap()[at + 100] == 2
        })
        .expect("the changed page was written");
    let completed = first.completed().expect("the first force completed");
    assert!(
        first.ticket() < changed_write.ticket() && changed_write.at() < completed,
        "the page's write came during the first force: {first:?}, {changed_write:?}"
    );
    let pid = changed.unwrap();
    let recording = sim.recording();
    let images = recording.images(recording.len(), 64, 31);
    for (i, image) in images.into_iter().enumerate() {
        let mut after = Sim::with_files(32, Config::default(), image);
        api::run(&mut after, |rt| {
            let (copy, _) = Pager::open(rt, &options).unwrap();
            assert_eq!(2, copy.read(pid).unwrap()[100], "image {i}");
            copy.close();
        });
    }
}

/// Give two restores of one backup different epochs, and refuse a position of one on the other.
#[test]
fn two_restores_of_a_backup_share_no_positions() {
    for seed in sweep(33..35) {
        let mut sim = Sim::with_files(seed, Config::default(), backed_up(2));
        api::run(&mut sim, |rt| {
            let restored = |name: &str| {
                restore_in(
                    rt.clone(),
                    &archive(),
                    name,
                    RestorePoint::Latest,
                    &no_create(),
                )
                .unwrap();
                open_at(rt.clone(), name).unwrap()
            };
            let (a, b) = (restored("a.adt"), restored("b.adt"));
            let (at_a, at_b) = (commit(&a, 10), commit(&b, 20));
            assert_ne!(at_a.epoch, at_b.epoch, "seed {seed}");
            for (db, foreign) in [(&a, at_b), (&b, at_a)] {
                let got = db.query_text(
                    "from u in users select u.id",
                    &[],
                    &CallOptions {
                        after: Some(foreign),
                        ..CallOptions::default()
                    },
                );
                assert!(
                    matches!(
                        got,
                        Err(Error::Rejected {
                            reason: RejectReason::PositionNotOnHistory { .. }
                        })
                    ),
                    "seed {seed}: {got:?}"
                );
            }
            a.close().unwrap();
            b.close().unwrap();
        });
    }
}

/// Restore at the backup's end, through the API and through the command, the point that a refusal
/// names as reachable.
#[test]
fn restore_at_the_backups_end_is_reachable() {
    let mut sim = Sim::with_files(36, Config::default(), backed_up(3));
    api::run(&mut sim, |rt| {
        let (manifest, _) = storage::read_manifest(&rt, TARGET).unwrap();
        let end = Position::from(manifest.end);
        restore_in(
            rt.clone(),
            &archive(),
            COPY,
            RestorePoint::Position(end),
            &no_create(),
        )
        .unwrap();
        let copy = open_at(rt.clone(), COPY).unwrap();
        assert_eq!(3, ids(&copy).len());
        copy.close().unwrap();
        let db = open_at(rt.clone(), FILE).unwrap();
        let text = format!(
            "restore from \"{TARGET}\" to \"again.adt\" at position {{ epoch: {}, lsn: {} }}",
            end.epoch, end.lsn
        );
        db.query_text(&text, &[], &CallOptions::default()).unwrap();
        let past = format!(
            "restore from \"{TARGET}\" to \"past.adt\" at position {{ epoch: {}, lsn: {} }}",
            end.epoch,
            end.lsn + 1
        );
        assert!(matches!(
            db.query_text(&past, &[], &CallOptions::default()),
            Err(Error::Rejected {
                reason: RejectReason::RestorePointUnavailable { .. }
            })
        ));
        db.close().unwrap();
        let again = open_at(rt, "again.adt").unwrap();
        assert_eq!(3, ids(&again).len());
        again.close().unwrap();
    });
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
    let got = storage::read_manifest(rt, TARGET);
    assert!(matches!(got, Err(BackupError::Absent(_))), "{got:?}");
}

/// Refuse a backup whose log range has lost a segment.
#[test]
fn a_hole_in_the_log_fails_the_backup() {
    let mut sim = Sim::new(40, Config::default());
    api::run(&mut sim, |rt| {
        let options = pager_options(FILE, false, 1 << 14);
        let pager = Pager::create(rt.clone(), &options).unwrap();
        let pid = pager.allocate(1, PageKind::HeapSlotted, 7).unwrap();
        let mut io = rt.io();
        let mut holed = false;
        let got = backup_with(&pager, TARGET, &mut |_, _| {
            if !holed {
                holed = true;
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
                common::complete(&rt, &mut io, Op::Remove { path })
                    .result
                    .unwrap();
            }
            true
        });
        match got {
            Err(BackupError::Io { what, .. }) => assert!(what.contains("log segment"), "{what}"),
            other => panic!("{other:?}"),
        }
        manifest_absent(&rt);
    });
}

/// Refuse a restore whose recovery stops short of the manifest's end, and leave no file.
#[test]
fn a_restore_short_of_the_end_is_refused() {
    let files = resealed(backed_up(2), |body| {
        body.lines()
            .filter(|l| !l.starts_with("part log/"))
            .map(|l| format!("{l}\n"))
            .collect()
    });
    let mut sim = Sim::with_files(41, Config::default(), files);
    api::run(&mut sim, |rt| {
        assert_eq!("log", damaged_part(restore(&rt)));
        assert!(open_at(rt, COPY).is_err());
    });
}

/// Return the pages a manifest lists as copied damaged from a restore that succeeds.
#[test]
fn restore_returns_the_listed_damage() {
    let mut files = file_with_rows(4);
    let pid = rot_users_page(&mut files);
    let mut sim = Sim::with_files(42, Config::default(), files);
    api::run(&mut sim, |rt| {
        let db = open(rt.clone());
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
    api::run(&mut sim, |rt| {
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
    api::run(&mut sim, |rt| {
        let got = prove_in(rt.clone(), &archive(), &Log::new([APP]), &no_create());
        assert!(matches!(
            got,
            Err(Error::Rejected {
                reason: RejectReason::BackupDamaged { .. }
            })
        ));
        assert_eq!(
            Proof::Unproven { function: None },
            proof_in(&rt, &archive()).unwrap()
        );
    });
}

/// Returns a file of one used page and the pids of that page and of its group's bitmap.
fn pager_with_bitmap(options: &Options) -> (Files, Pid, Pid) {
    let mut sim = Sim::new(45, Config::default());
    let (used, bitmap) = api::run(&mut sim, |rt| {
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
    let data = files.get_mut(FILE).unwrap();
    data[bitmap.0 as usize * 4096 + 200] ^= 0xff;
    data[used.0 as usize * 4096 + 200] ^= 0xff;
    let mut sim = Sim::with_files(46, Config::default(), files);
    api::run(&mut sim, |rt| {
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
    let free = api::run(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt, &options).unwrap();
        let free = pager.allocate(1, PageKind::HeapSlotted, 7).unwrap();
        pager.free(free).unwrap();
        pager.close();
        free
    });
    files = sim.files();
    files.get_mut(FILE).unwrap()[free.0 as usize * 4096 + 200] ^= 0xff;
    let mut sim = Sim::with_files(48, Config::default(), files);
    api::run(&mut sim, |rt| {
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
    api::run(&mut sim, |rt| {
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
                common::write(&rt, &mut io, file, page.0 * 4096, &vec![0xee; 4096]).unwrap();
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
    let byte = pid.0 as usize * 8192 + 200;
    before.get_mut(FILE).unwrap()[byte] ^= 0xff;
    let mut sim = Sim::with_files(10, Config::default(), before.clone());
    api::run(&mut sim, |rt| {
        let db = open(rt);
        api::results(
            &db,
            r#"update u in users where key u == users.key(0) set User { u with name: "later" }"#,
        );
        db.close().unwrap();
    });
    let mut files = sim.files();
    let page = pid.0 as usize * 8192..(pid.0 as usize + 1) * 8192;
    files.get_mut(FILE).unwrap()[page.clone()].copy_from_slice(&before[FILE][page]);
    let mut sim = Sim::with_files(10, Config::default(), files);
    api::run(&mut sim, |rt| {
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
    api::run(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &options).unwrap();
        let page = pager.allocate(1, PageKind::HeapSlotted, 7).unwrap();
        pager.update(&[page], |v| v[0][100] = 7).unwrap();
        pager.flush_log();
        pager.flush_pages(usize::MAX);
        let reads = Match::kind(OpKind::Read).offset(page.0 * 4096);
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

/// Fail a proof of a backup with listed damage even when a host function is missing.
#[test]
fn listed_damage_outranks_a_missing_function() {
    const LOG: &str = r#"module app
add module app
add type User = { id: Key users, name: Text }
define fn boom (t: Text) -> Text = host "boom" version "1.0" deterministic
add relation users : User key id generate sequence
  index (boom(name))
"#;
    let opening = || {
        let mut fns = HostFns::new();
        fns.register(
            "boom",
            "1.0",
            Determinism::Deterministic,
            |b: &adtb::batch::Batch| Ok(b.clone()),
        );
        OpenOptions {
            host_fns: fns,
            ..api::options(2)
        }
    };
    let log = Log::new([LOG]);
    let mut sim = Sim::new(51, Config::default());
    let pid = api::run(&mut sim, |rt| {
        let db = Database::open_in(rt, FILE, &log, &opening()).unwrap();
        api::results(&db, r#"insert users { id: users.key(1), name: "a" }"#);
        let t = db.testing();
        let pid = t.pages(t.heap("app::users", "User").unwrap()).unwrap()[0];
        db.close().unwrap();
        pid
    });
    let mut files = sim.files();
    files.get_mut(FILE).unwrap()[pid.0 as usize * 8192 + 200] ^= 0xff;
    let mut sim = Sim::with_files(52, Config::default(), files);
    api::run(&mut sim, |rt| {
        let db = Database::open_in(rt.clone(), FILE, &log, &opening()).unwrap();
        backup_text(&db).unwrap();
        db.close().unwrap();
        match prove_in(rt, &archive(), &log, &no_create()).unwrap() {
            Proof::Failed(found) => assert!(found[0].contains("copied damaged"), "{found:?}"),
            other => panic!("{other:?}"),
        }
    });
}

/// Treat a subdirectory at a target or a restore path as occupied.
#[test]
fn a_directory_at_a_target_or_path_is_occupied() {
    let mut sim = Sim::with_files(53, Config::default(), backed_up(1));
    api::run(&mut sim, |rt| {
        let mut io = rt.io();
        for dir in ["taken", "taken/sub", COPY] {
            common::complete(
                &rt,
                &mut io,
                Op::CreateDir {
                    dir: dir.to_owned(),
                },
            )
            .result
            .unwrap();
        }
        is_invalid_naming(&restore(&rt), "exists");
        let db = open_at(rt, FILE).unwrap();
        let got = db
            .query_text("backup to \"taken\"", &[], &CallOptions::default())
            .map(|_| ());
        is_invalid_naming(&got, "holds files");
        db.close().unwrap();
    });
}

/// Answer a restore of nothing as invalid, naming the path, and a failed read as a storage failure.
#[test]
fn an_absent_source_and_an_unreadable_manifest_differ() {
    let mut sim = Sim::with_files(60, Config::default(), backed_up(1));
    api::run(&mut sim, |rt| {
        let elsewhere = ArchiveTarget::path("nowhere").unwrap();
        let got = restore_in(
            rt.clone(),
            &elsewhere,
            COPY,
            RestorePoint::Latest,
            &no_create(),
        );
        is_invalid_naming(&got.map(|_| ()), "nowhere");
        rt.plan(fail_reads_after_open(
            &rt,
            Match::kind(OpKind::Open).suffixed("/manifest"),
        ));
        let got = restore_in(rt, &archive(), COPY, RestorePoint::Latest, &no_create());
        assert!(
            matches!(
                got,
                Err(Error::Fatal {
                    reason: FatalReason::Storage { .. }
                })
            ),
            "{got:?}"
        );
    });
}

/// Close every file a stopped or failed backup or restore opened.
#[test]
fn early_returns_close_their_files() {
    let options = pager_options(FILE, false, 1 << 20);
    let mut sim = Sim::new(61, Config::default());
    api::run(&mut sim, |rt| {
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
    api::run(&mut sim, |rt| {
        restore(&rt).unwrap();
        let db = open_at(rt, COPY).unwrap();
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

/// Let a commit finish while a `backup to` statement copies.
#[test]
fn a_commit_finishes_during_a_backup_statement() {
    let mut sim = Sim::new(63, Config::default());
    api::run(&mut sim, |rt| {
        let db = Database::open_in(rt.clone(), FILE, &Log::new([APP]), &options()).unwrap();
        insert(&db, 0..2);
        let committed = Arc::new(AtomicBool::new(false));
        let started = Arc::new(AtomicBool::new(false));
        let (done, began, committer) = (committed.clone(), started.clone(), db.clone());
        let begins = Match::kind(OpKind::CreateNew).prefixed("bk.partial-");
        let begin = Action::Run(Box::new(move || began.store(true, Ordering::SeqCst)));
        rt.plan(Plan::new().first(begins, begin));
        let waiter = rt.clone();
        let during = started.clone();
        let thread = rt.spawn("committer", move || {
            while !during.load(Ordering::SeqCst) {
                waiter.yield_now();
            }
            let text = insert_text(50);
            committer
                .query_text(&text, &[], &CallOptions::default())
                .unwrap();
            done.store(true, Ordering::SeqCst);
        });
        let target = format!("backup to \"{TARGET}\"");
        db.query_text(&target, &[], &CallOptions::default())
            .unwrap();
        rt.join(thread);
        assert!(committed.load(Ordering::SeqCst));
        assert!(started.load(Ordering::SeqCst));
        db.close().unwrap();
    });
}

/// Returns a running source with a backup of it in `bk`.
fn backed_up_source(rt: &SimRuntime) -> Database<SimRuntime> {
    let db = Database::open_in(rt.clone(), FILE, &Log::new([APP]), &options()).unwrap();
    insert(&db, 0..2);
    db.query_text(
        &format!("backup to \"{TARGET}\""),
        &[],
        &CallOptions::default(),
    )
    .unwrap();
    db
}

fn restore_text(db: &Database<SimRuntime>) -> Result<(), Error> {
    db.query_text(
        &format!("restore from \"{TARGET}\" to \"{COPY}\""),
        &[],
        &CallOptions::default(),
    )
    .map(|_| ())
}

/// Answer an unreadable target log as a storage failure, leave nothing, and keep the source alive.
#[test]
fn a_failed_read_of_the_restored_log_leaves_the_source_alive() {
    let mut sim = Sim::new(70, Config::default());
    api::run(&mut sim, |rt| {
        let db = backed_up_source(&rt);
        rt.plan(fail_reads_after_open(
            &rt,
            Match::kind(OpKind::Open).prefixed("copy.adt-log"),
        ));
        let got = restore_text(&db);
        assert!(
            matches!(
                got,
                Err(Error::Fatal {
                    reason: FatalReason::Storage { .. }
                })
            ),
            "{got:?}"
        );
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

/// Remove the target a failed backup created, and keep the empty one it found.
#[test]
fn a_failed_backup_removes_a_target_it_created_only() {
    let mut sim = Sim::new(74, Config::default());
    api::run(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &pager_options(FILE, false, 1 << 20)).unwrap();
        let got = backup_with(&pager, TARGET, &mut |_, _| false);
        assert_eq!(Some(BackupError::Stopped), got.err());
        assert!(!sim_has(&rt, TARGET), "the target this run created");
        let mut io = rt.io();
        let dir = TARGET.to_owned();
        common::complete(&rt, &mut io, Op::CreateDir { dir })
            .result
            .unwrap();
        let got = backup_with(&pager, TARGET, &mut |_, _| false);
        assert_eq!(Some(BackupError::Stopped), got.err());
        assert!(sim_has(&rt, TARGET), "the target this run found");
        backup_with(&pager, TARGET, &mut |_, _| true).unwrap();
        pager.close();
    });
}

/// Treat a file at a target, or at a restore's log directory, as occupied.
#[test]
fn a_file_at_a_target_or_log_directory_is_occupied() {
    let mut sim = Sim::with_files(76, Config::default(), backed_up(1));
    api::run(&mut sim, |rt| {
        let mut io = rt.io();
        for path in ["taken", "copy.adt-log"] {
            let (path, create) = (path.to_owned(), true);
            common::complete(&rt, &mut io, Op::Open { path, create })
                .result
                .unwrap();
        }
        is_invalid_naming(&restore(&rt), "exists");
        assert!(!sim_has(&rt, COPY));
        let db = open_at(rt, FILE).unwrap();
        for target in ["taken", "taken/sub"] {
            let got = db
                .query_text(
                    &format!("backup to \"{target}\""),
                    &[],
                    &CallOptions::default(),
                )
                .map(|_| ());
            is_invalid_naming(&got, "holds files");
        }
        db.close().unwrap();
    });
}

/// Force the directories a backup and a restore create and no directory above them.
#[test]
fn only_directories_a_run_creates_are_forced() {
    let mut sim = Sim::new(77, Config::default());
    api::run(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &pager_options(FILE, false, 1 << 20)).unwrap();
        let mut io = rt.io();
        for dir in ["up", "up/mid"] {
            let dir = dir.to_owned();
            common::complete(&rt, &mut io, Op::CreateDir { dir })
                .result
                .unwrap();
        }
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
        rt.plan(Plan::new());
        pager.close();
    });
}

/// Restore a backup taken under a rotted bitmap: the copy opens and lists the damage.
#[test]
fn a_restore_under_a_rotted_bitmap_opens_and_lists_it() {
    let options = pager_options(FILE, false, 1 << 20);
    let (mut files, used, bitmap) = pager_with_bitmap(&options);
    let data = files.get_mut(FILE).unwrap();
    data[bitmap.0 as usize * 4096 + 200] ^= 0xff;
    let mut sim = Sim::with_files(79, Config::default(), files);
    api::run(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &options).unwrap();
        backup_with(&pager, TARGET, &mut |_, _| true).unwrap();
        pager.close();
        let manifest = storage::restore_file(&rt, TARGET, COPY, "copy.adt-log").unwrap();
        assert!(manifest.damaged.contains(&bitmap), "{:?}", manifest.damaged);
        let copy = pager_options(COPY, false, 1 << 20);
        let (pager, _) = Pager::open(rt, &copy).unwrap();
        assert!(pager.read(bitmap).is_err(), "the bitmap stays rotted");
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
        let got = api::run(&mut sim, |rt| {
            let (pager, _) = Pager::open(rt.clone(), &options).unwrap();
            pager.damage(used, damage);
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
    api::run(&mut sim, |rt| {
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

/// Spoil and list the copy of a page whose flush was lost before the pool evicted it, no reader
/// having refused it before the backup.
#[test]
fn a_lost_write_evicted_before_the_backup_restores_refused() {
    let (copy, pid, stale) = backup_after_flush(53, true, Flushed::Evicted);
    let want = FlushedCopy {
        listed: vec![pid],
        restored: Err(Damage::Checksum),
        source: Err(stale),
    };
    assert_eq!(want, copy);
}
/// Spoil and list the copy of a page whose flush was lost and that a reader refuses during the
/// backup, after its copy.
#[test]
fn a_lost_write_refused_after_its_copy_restores_refused() {
    let (copy, pid, stale) = backup_after_flush(54, true, Flushed::ReadAfterCopy);
    let want = FlushedCopy {
        listed: vec![pid],
        restored: Err(Damage::Checksum),
        source: Err(stale),
    };
    assert_eq!(want, copy);
}

/// Spoil and list the copy of a page whose flush was lost while the pool keeps the page.
#[test]
fn a_lost_write_still_resident_restores_refused() {
    let (copy, pid, _) = backup_after_flush(55, true, Flushed::Resident);
    let want = FlushedCopy {
        listed: vec![pid],
        restored: Err(Damage::Checksum),
        source: Ok(7),
    };
    assert_eq!(want, copy);
}

/// Copy a page whose flush reached the file as it is, evicted or resident.
#[test]
fn a_flushed_write_restores_with_its_value() {
    for at in [Flushed::Evicted, Flushed::Resident] {
        let (copy, _, _) = backup_after_flush(56, false, at);
        let want = FlushedCopy {
            listed: vec![],
            restored: Ok(7),
            source: Ok(7),
        };
        assert_eq!(want, copy, "{at:?}");
    }
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
    let got = api::run(&mut sim, |rt| {
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
    files.get_mut(FILE).unwrap()[used.0 as usize * 4096 + 200] ^= 0xff;
    let mut sim = Sim::with_files(52, Config::default(), files);
    api::run(&mut sim, |rt| {
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
        is_invalid_naming(retried, "holds files");
    } else {
        assert!(retried.is_ok(), "{label}: the retry: {retried:?}");
        expected.insert(LATER_ROW);
    }
    assert_eq!(expected, restored_rows(files).unwrap(), "{label}");
}

/// Fail each op of a backup: the source lives, the target is absent or whole, and a retry answers
/// by which.
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
        workload,
        |failed| {
            let label = format!("op {} {}", failed.at, failed.struck);
            let out = failed.out;
            assert!(
                out.got.is_err() || out.published,
                "{label}: answered success with nothing published"
            );
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
        workload,
        |image| image.keys().any(|p| p.starts_with("bk/")),
        |at, image| {
            let label = format!("event {at}");
            let published = image.keys().any(|p| p.starts_with("bk/"));
            let mut sim = Sim::with_files(at as u64, Config::default(), image);
            let retried = api::run(&mut sim, |rt| {
                let db = open_at(rt.clone(), FILE).unwrap();
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
            assert!(
                matches!(loser, Some(BackupError::Occupied(_))),
                "op {at:?}: {loser:?}"
            );
        }
        db.close().unwrap();
    };
    let files = file_with_rows(SEARCH_ROWS);
    search::each_start(&files, 92, STARTS, workload, |at, files| {
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
    api::run(&mut sim, |rt| {
        if has_file || has_log {
            is_invalid_naming(&restore(&rt), "exists");
        } else {
            let got = restore(&rt);
            assert!(got.is_ok(), "{label}: the retry: {got:?}");
        }
        if has_log && !has_file {
            return;
        }
        let db = open_at(rt, COPY).unwrap();
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
    search::each_failure(&base, 93, FAILURES, workload, |failed| {
        let label = format!("op {} {}", failed.at, failed.struck);
        let (got, files_open) = failed.out;
        assert_eq!(0, files_open, "{label}");
        assert!(
            got.is_err() || failed.files.contains_key(COPY),
            "{label}: answered success with nothing published"
        );
        if !stops_a_cleanup(&failed.struck) && got.is_err() {
            let left: Vec<_> = failed.files.keys().filter(|p| is_aside(p)).collect();
            assert!(left.is_empty(), "{label}: {left:?}");
        }
        assert_restore_state(&base, &failed.files, false, 93, &label);
    });
}

/// Crash a restore at each device event: the file's name is the commit point.
#[test]
fn a_restore_crashed_anywhere() {
    let base = backed_up(SEARCH_ROWS);
    let workload = |rt: &SimRuntime| {
        rt.arm(searched());
        restore_copy(rt).unwrap();
        rt.disarm();
    };
    search::each_crash(
        &base,
        94,
        CRASH_POINTS,
        CRASH_IMAGES,
        workload,
        |image| image.contains_key(COPY),
        |at, image| {
            assert_restore_state(&base, &image, true, at as u64, &format!("event {at}"));
        },
    );
}

fn segments_of(files: &Files, log_dir: &str) -> Files {
    let log_dir = format!("{log_dir}/");
    let mut held = final_files(files);
    held.retain(|path, _| path.starts_with(&log_dir));
    held
}

/// Crash a restore at each device event and open its path with `create`: a log with no file is
/// refused and left as it was.
#[test]
fn an_open_after_a_restore_crashed_between_its_renames_refuses() {
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
        workload,
        |image| image.contains_key(COPY),
        |at, image| {
            let finals = final_files(&image);
            let segments = segments_of(&image, &log_dir);
            if finals.contains_key(COPY) || segments.is_empty() {
                return;
            }
            between += 1;
            let mut sim = Sim::with_files(at as u64, Config::default(), image);
            let opened = api::run(&mut sim, |rt| {
                Database::open_in(rt, COPY, &Log::new([APP]), &options()).map(Database::close)
            });
            assert!(
                matches!(&opened, Err(adtb::OpenError::Corrupt { detail })
                    if detail == "the log directory holds a log that no file here wrote"),
                "event {at}: {opened:?}"
            );
            assert!(
                segments == segments_of(&sim.files(), &log_dir),
                "event {at}: the refusal changed the log"
            );
        },
    );
    assert_reached("a crash between the renames", between, 1);
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
            assert!(
                restored.is_ok() || opened.is_ok(),
                "op {at:?}: {restored:?} {opened:?}"
            );
            let tally = if restored.is_ok() {
                &restore_won
            } else {
                &open_won
            };
            tally.fetch_add(1, Ordering::Relaxed);
            let db = open_at(rt.clone(), COPY).expect("the winner's database opens");
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
    search::each_start(&base, 95, STARTS, workload, |at, files| {
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
    api::run(&mut sim, |rt| {
        let db = open(rt);
        insert(&db, 0..rows);
        db.close().unwrap();
    });
    sim.files()
}

/// Refuse a log another file wrote, placed beside a file, as long, shorter or longer than its own.
#[test]
fn a_foreign_log_beside_a_file_is_refused() {
    let log_dir = format!("{FILE}-log/");
    let ours = with_rows_of_seed(7, 8);
    for rows in [8, 0, 40] {
        let theirs = with_rows_of_seed(8, rows);
        let mut files = ours.clone();
        files.retain(|path, _| !path.starts_with(&log_dir));
        files.extend(
            theirs
                .into_iter()
                .filter(|(path, _)| path.starts_with(&log_dir)),
        );
        let mut sim = Sim::with_files(9, Config::default(), files);
        let got = api::run(&mut sim, |rt| open_at(rt, FILE).map(Database::close));
        assert!(
            matches!(got, Err(adtb::OpenError::Corrupt { .. })),
            "a log of {rows} rows: {got:?}"
        );
    }
}

/// Refuse a log of this file whose tail holds a block another file wrote, naming it foreign.
#[test]
fn a_foreign_block_past_the_end_of_a_log_is_refused_as_foreign() {
    const BLOCK: usize = 512;
    let segment = format!("{FILE}-log/");
    let mut files = with_rows_of_seed(7, 8);
    let theirs = with_rows_of_seed(8, 40);
    let (name, ours) = files
        .iter_mut()
        .find(|(path, _)| path.starts_with(&segment))
        .expect("a segment");
    let end = ours
        .chunks(BLOCK)
        .position(|block| block.iter().all(|&b| b == 0))
        .expect("the log ends before the segment does")
        * BLOCK;
    let tail = &theirs[name.as_str()][end..end + BLOCK];
    assert!(tail.iter().any(|&b| b != 0), "their log passes ours");
    ours[end..end + BLOCK].copy_from_slice(tail);
    let mut sim = Sim::with_files(9, Config::default(), files);
    let (got, cause) = api::run(&mut sim, |rt| {
        let got = open_at(rt.clone(), FILE).map(Database::close);
        (got, Pager::open(rt, &Options::new(FILE)).err())
    });
    let Some(
        cause @ storage::OpenError::Log(storage::LogFinding {
            problem: storage::LogProblem::Foreign,
            ..
        }),
    ) = cause
    else {
        panic!("the pager refuses the log as foreign: {cause:?}");
    };
    assert!(
        matches!(&got, Err(adtb::OpenError::Corrupt { detail }) if *detail == format!("{cause:?}")),
        "{got:?}"
    );
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
    api::run(&mut sim, |rt| {
        let db = open_at(rt.clone(), FILE).unwrap();
        let mut io = rt.io();
        let file = common::open(&rt, &mut io, name, false).unwrap().0;
        common::write(&rt, &mut io, file, last as u64, &foreign).unwrap();
        let text = format!("backup to {TARGET:?}");
        let got = db.query_text(&text, &[], &CallOptions::default());
        assert!(got.is_err(), "{got:?}");
        assert!(
            storage::read_manifest(&rt, TARGET).is_err(),
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
            assert!(
                first.is_ok() != second.is_ok(),
                "op {at:?}: {first:?} {second:?}"
            );
            is_invalid_naming(&first.and(second), "exists");
        }
    };
    let base = backed_up(SEARCH_ROWS);
    search::each_start(&base, 95, STARTS, workload, |at, files| {
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
    api::run(&mut sim, |rt| {
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
    search::each_failure(&base, 96, FAILURES, workload, |failed| {
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
    search::each_failure(&base, 96, FAILURES, workload, |failed| {
        let label = format!("op {} {}", failed.at, failed.struck);
        assert!(
            matches!(failed.out, Err(_) | Ok(Proof::Proven)),
            "{label}: a false verdict: {:?}",
            failed.out
        );
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
        workload,
        |image| image.contains_key(&proof),
        |at, image| {
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
    search::each_start(&base, 98, STARTS, workload, |at, files| {
        assert!(!files.keys().any(|p| is_aside(p)), "op {at}");
        let mut finals = final_files(&files);
        finals.remove(&format!("{TARGET}/{}", storage::PROOF));
        assert!(
            finals == final_files(&base),
            "op {at}: a change outside the proof"
        );
    });
}

/// Keep a directory a failed restore made above its path, and nothing else of the run.
#[test]
fn a_failed_restore_keeps_the_directories_above_it() {
    let mut sim = Sim::with_files(99, Config::default(), backed_up(2));
    api::run(&mut sim, |rt| {
        rt.plan(Plan::new().rule(Match::kind(OpKind::Write), Action::Fail(IoError::Full)));
        let got = storage::restore_file(&rt, TARGET, "deep/er/copy.adt", "deep/er/log");
        assert!(matches!(got, Err(BackupError::Io { .. })), "{got:?}");
        rt.plan(Plan::new());
        let mut io = rt.io();
        let dir = "deep/er".to_owned();
        let listed = common::complete(&rt, &mut io, Op::List { dir }).result;
        assert_eq!(Ok(Done::Listed(Vec::new())), listed);
        storage::restore_file(&rt, TARGET, "deep/er/copy.adt", "deep/er/log").unwrap();
    });
}

/// Answer a failed read of a part as a storage failure: it shows no damage, and keeps the proof.
#[test]
fn a_failed_read_of_a_part_is_not_damage() {
    let mut sim = Sim::with_files(100, Config::default(), backed_up(2));
    api::run(&mut sim, |rt| {
        let log = Log::new([APP]);
        prove_in(rt.clone(), &archive(), &log, &no_create()).unwrap();
        rt.plan(fail_reads_after_open(
            &rt,
            Match::kind(OpKind::Open).prefixed("bk/pages/"),
        ));
        let got = prove_in(rt.clone(), &archive(), &log, &no_create());
        assert!(
            matches!(
                got,
                Err(Error::Fatal {
                    reason: FatalReason::Storage { .. }
                })
            ),
            "{got:?}"
        );
        rt.plan(Plan::new());
        assert_eq!(Proof::Proven, proof_in(&rt, &archive()).unwrap());
    });
}

/// Answer `restore from` a regular file as no backup there, and a failed read of a proof as a
/// storage failure.
#[test]
fn a_file_as_a_source_is_absent_and_a_proof_read_can_fail() {
    let mut sim = Sim::with_files(101, Config::default(), backed_up(2));
    api::run(&mut sim, |rt| {
        let elsewhere = ArchiveTarget::path(FILE).unwrap();
        let got = restore_in(
            rt.clone(),
            &elsewhere,
            COPY,
            RestorePoint::Latest,
            &no_create(),
        );
        is_invalid_naming(&got.map(|_| ()), "no backup");
        let log = Log::new([APP]);
        prove_in(rt.clone(), &archive(), &log, &no_create()).unwrap();
        rt.plan(fail_reads_after_open(
            &rt,
            Match::kind(OpKind::Open).path("bk/proof"),
        ));
        let got = proof_in(&rt, &archive());
        assert!(
            matches!(
                got,
                Err(Error::Fatal {
                    reason: FatalReason::Storage { .. }
                })
            ),
            "{got:?}"
        );
    });
}

/// Refuse a manifest whose checkpoint is in bounds and names no checkpoint record.
#[test]
fn a_manifest_naming_no_checkpoint_record_is_damaged() {
    let files = resealed(backed_up(1), |body| {
        body.lines()
            .map(|line| match line.strip_prefix("checkpoint ") {
                Some(lsn) => format!("checkpoint {}\n", lsn.parse::<u64>().unwrap() + 1),
                None => format!("{line}\n"),
            })
            .collect()
    });
    let mut sim = Sim::with_files(102, Config::default(), files);
    api::run(&mut sim, |rt| {
        assert_eq!("log", damaged_part(restore(&rt)));
        assert!(!sim_has(&rt, COPY));
    });
}

/// Show no restored file under its final name while the restore can still fail.
#[test]
fn a_restore_that_fails_its_check_never_shows_a_file() {
    let mut sim = Sim::with_files(80, Config::default(), backed_up(2));
    api::run(&mut sim, |rt| {
        let names_the_file = Match::any_of([
            Match::kind(OpKind::Open).creating(),
            Match::kinds([OpKind::CreateNew, OpKind::Rename]),
        ]);
        let shown = rt.counted(names_the_file.path(COPY));
        rt.plan(fail_reads_after_open(
            &rt,
            Match::kind(OpKind::Open).prefixed("copy.adt-log"),
        ));
        let got = restore_in(rt, &archive(), COPY, RestorePoint::Latest, &no_create());
        assert!(got.is_err(), "{got:?}");
        assert_eq!(
            0,
            shown.get(),
            "an op put a file another thread could open at {COPY}"
        );
    });
}

/// Answer a whole manifest with a field out of bounds as a damaged manifest, on the live source's
/// thread.
#[test]
fn a_resealed_manifest_out_of_bounds_is_damaged() {
    let replaced = |key: &'static str, value: &'static str| {
        move |body: &str| {
            body.lines()
                .map(|l| {
                    let kept = !l.starts_with(&format!("{key} "));
                    let line = if kept {
                        l.to_owned()
                    } else {
                        format!("{key} {value}")
                    };
                    line + "\n"
                })
                .collect::<String>()
        }
    };
    let edits = [
        ("log_block", "0"),
        ("log_block", "1000"),
        ("segment_size", "0"),
        ("segment_size", "4097"),
        ("page_size", "0"),
        ("page_size", "5000"),
        ("page_formats", "2 1"),
        ("file_pages", "0"),
        ("catalog_root", "18446744073709551615"),
        ("quarantine_page", "18446744073709551615"),
        ("start", "1/18446744073709551615"),
        ("end", "1/0"),
        ("checkpoint", "0"),
    ];
    for (key, value) in edits {
        let files = resealed(backed_up(1), replaced(key, value));
        let mut sim = Sim::with_files(83, Config::default(), files);
        api::run(&mut sim, |rt| {
            assert_eq!(
                storage::MANIFEST,
                damaged_part(restore(&rt)),
                "{key} {value}"
            );
        });
    }
    let parts = [
        "part ../outside 0 00\n",
        "part pages/a/b 0 00\n",
        "damaged 18446744073709551615\n",
    ];
    for line in parts {
        let files = resealed(backed_up(1), |body| format!("{body}{line}"));
        let mut sim = Sim::with_files(84, Config::default(), files);
        api::run(&mut sim, |rt| {
            assert_eq!(storage::MANIFEST, damaged_part(restore(&rt)), "{line}");
        });
    }
}
