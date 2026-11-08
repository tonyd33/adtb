//! The continuous log archive: complete segments copied in order under the file's id and epoch.

use adtb::testing::{Knobs, Probe};
use std::collections::BTreeMap;
use std::time::Duration;

use adtb::runtime::{IoError, OpKind, Runtime};
use adtb::storage::{
    self, ArchiveStatus, Archiver, BackupError, BackupRun, Envelope, Lsn, Options, PageKind, Pager,
    Pid, Position,
};
use adtb::{CallOptions, CreateOptions, Database, Log, OpenOptions};
use adtb_sim::{Action, Config, Match, Plan, Sim, SimRuntime};
use adtb_simkit::open;
use adtb_simkit::search::{self, Files};
use adtb_simkit::{api, common};

const FILE: &str = "archived.adt";
const COPY: &str = "copy.adt";
const DEST: &str = "store";
const SEGMENT: u64 = 1 << 14;
const NO_PARENT: &str = "adtb archive epoch\nparent none\n";

fn options(name: &str) -> Options {
    Options {
        page_size: 4096,
        segment_size: SEGMENT,
        pool_frames: 64,
        max_pages: 4096,
        ..Options::new(name)
    }
}

/// Log changes until the first `segments` segments are complete.
fn fill<R: Runtime>(pager: &Pager<R>, segments: u64) {
    fill_page(
        pager,
        pager.allocate(1, PageKind::HeapSlotted, 7).unwrap(),
        segments,
    );
}

fn fill_page<R: Runtime>(pager: &Pager<R>, pid: Pid, segments: u64) {
    while Probe::of(pager).durable().0 < segments * SEGMENT {
        (pager.update(&[pid], |v| v[0][100] = v[0][100].wrapping_add(1))).unwrap();
        pager.flush_log();
    }
}

/// Returns the first LSN of each segment in `log_dir`, in order.
fn segments(files: &Files, log_dir: &str) -> Vec<u64> {
    let names = files
        .keys()
        .filter_map(|p| p.strip_prefix(&format!("{log_dir}/")));
    names.map(|n| u64::from_str_radix(n, 16).unwrap()).collect()
}

/// Returns a closed file whose first three segments are complete, none archived.
fn filled() -> Files {
    let mut sim = rooted(1);
    api::run(&mut sim, |rt| {
        let pager = Pager::create(rt, &options(FILE)).unwrap();
        fill(&pager, 3);
        pager.close();
    });
    sim.files()
}

/// Open `FILE` of `files`, run `body` with it and an archiver of it, and return the status of a
/// last pass and the files after.
fn archived(
    seed: u64,
    files: Files,
    body: impl FnOnce(&Pager<SimRuntime>),
) -> (ArchiveStatus, Files) {
    let mut sim = Sim::with_files(seed, Config::default(), files);
    let status = api::run(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt, &options(FILE)).unwrap();
        body(&pager);
        let archiver = Archiver::new(pager.clone(), DEST);
        archiver.archive();
        let status = archiver.status();
        pager.close();
        status
    });
    (status, sim.files())
}

/// Returns a simulation whose device holds the archive's root.
fn rooted(seed: u64) -> Sim {
    let root = Files::from([(format!("{DEST}/keep"), Vec::new())]);
    Sim::with_files(seed, Config::default(), root)
}

fn ok(through: u64) -> ArchiveStatus {
    ArchiveStatus {
        through: Some(Lsn(through)),
        failure: None,
    }
}

fn stopped(through: Option<u64>, failure: BackupError) -> ArchiveStatus {
    ArchiveStatus {
        through: through.map(Lsn),
        failure: Some(failure),
    }
}

fn absent(what: impl Into<String>) -> BackupError {
    BackupError::Io {
        what: what.into(),
        error: IoError::NotFound,
    }
}

/// Assert the archive holds one epoch directory, as [`assert_archived`] states, and return its
/// epoch file's text.
fn assert_one(files: &Files, log_dir: &str, run: (u64, u64)) -> String {
    let dirs = epochs(files);
    assert_eq!(1, dirs.len());
    assert_archived(dirs.values().next().unwrap(), files, log_dir, run)
}

/// Returns each epoch directory of the archive, as `ID/EPOCH`, with its files by name.
fn epochs(files: &Files) -> BTreeMap<String, BTreeMap<String, Vec<u8>>> {
    let mut out: BTreeMap<String, BTreeMap<String, Vec<u8>>> = BTreeMap::new();
    for (path, bytes) in files {
        if let Some((dir, name)) = path
            .strip_prefix(&format!("{DEST}/"))
            .and_then(|rest| rest.rsplit_once('/'))
        {
            out.entry(dir.to_owned())
                .or_default()
                .insert(name.to_owned(), bytes.clone());
        }
    }
    out
}

/// Returns the name and bytes of the segment at `base` in `entries`.
fn at_base(entries: &BTreeMap<String, Vec<u8>>, base: u64) -> (&String, &Vec<u8>) {
    let stem = format!("{base:016x}-");
    entries.iter().find(|(n, _)| n.starts_with(&stem)).unwrap()
}

fn named(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// Assert `entries` hold exactly an epoch file named by its hash and the segments of `log_dir` in
/// `files` from `first` below `through`, each named by its first LSN and hash. Returns the epoch
/// file's text.
fn assert_archived(
    entries: &BTreeMap<String, Vec<u8>>,
    files: &Files,
    log_dir: &str,
    (first, through): (u64, u64),
) -> String {
    let mut want = BTreeMap::new();
    for base in (first..through).step_by(SEGMENT as usize) {
        let bytes = &files[&format!("{log_dir}/{base:016x}")];
        want.insert(format!("{base:016x}-{}", named(bytes)), bytes.clone());
    }
    let (epoch, text) = entries
        .iter()
        .find(|(name, _)| name.starts_with("epoch-"))
        .expect("the epoch file is published");
    assert_eq!(format!("epoch-{}", named(text)), *epoch);
    want.insert(epoch.clone(), text.clone());
    assert_eq!(
        want.keys().collect::<Vec<_>>(),
        entries.keys().collect::<Vec<_>>()
    );
    assert!(
        want == *entries,
        "every published segment holds the log's bytes"
    );
    let end = text.iter().position(|&b| b == 0).unwrap_or(text.len());
    String::from_utf8(text[..end].to_vec()).unwrap()
}

#[test]
fn every_complete_segment_is_archived_in_order_under_its_hash() {
    let mut sim = rooted(2);
    let (first, last, epoch) = api::run(&mut sim, |rt| {
        let pager = Pager::create(rt, &options(FILE)).unwrap();
        let archiver = Archiver::new(pager.clone(), DEST);
        assert_eq!(None, archiver.archived_through());
        fill(&pager, 2);
        archiver.archive();
        let first = archiver.status();
        fill(&pager, 5);
        archiver.archive();
        let last = archiver.status();
        let epoch = pager.position(Lsn(0)).epoch;
        pager.close();
        (first, last, epoch)
    });
    assert_eq!(ok(2 * SEGMENT), first);
    assert_eq!(ok(5 * SEGMENT), last);
    let files = sim.files();
    let dir = epochs(&files).into_keys().next().unwrap();
    assert_eq!(
        Some((32, format!("{epoch:016x}"))),
        dir.split_once('/').map(|(i, e)| (i.len(), e.to_owned()))
    );
    let log_dir = format!("{FILE}-log");
    assert_eq!(NO_PARENT, assert_one(&files, &log_dir, (0, 5 * SEGMENT)));
    assert!(
        files.contains_key(&format!("{log_dir}/{:016x}", 5 * SEGMENT)),
        "the incomplete tail is not archived"
    );
}

#[test]
fn an_epoch_file_is_written_once_and_another_is_never_replaced() {
    let (status, files) = archived(3, filled(), |_| {});
    assert_eq!(ok(3 * SEGMENT), status);
    let other = "adtb archive epoch\nparent 7 9\n";
    let held = format!("epoch-{}", named(other.as_bytes()));
    let published = files.keys().find(|p| p.contains("/epoch-")).unwrap();
    let dir = format!("{}/", published.rsplit_once('/').unwrap().0);
    let mut planted: Files = files
        .clone()
        .into_iter()
        .filter(|(p, _)| !p.starts_with(&dir))
        .collect();
    planted.insert(format!("{dir}{held}"), other.into());
    let (status, files) = archived(4, planted.clone(), |_| {});
    assert_eq!(stopped(None, BackupError::Damaged { part: held }), status);
    assert_eq!(
        epochs(&planted),
        epochs(&files),
        "only the planted epoch file is there, as it was"
    );
}

#[test]
fn a_segment_missing_from_the_log_stops_the_archive_with_a_recorded_failure() {
    let mut files = filled();
    let missing = format!("{FILE}-log/{SEGMENT:016x}");
    assert!(files.remove(&missing).is_some());
    let (status, files) = archived(5, files, |_| {});
    assert_eq!(stopped(Some(SEGMENT), absent(missing)), status);
    assert_one(&files, &format!("{FILE}-log"), (0, SEGMENT));
    let (_, mut files) = archived(16, filled(), |_| {});
    files.retain(|p, _| !p.ends_with("/0000000000000000") && !p.contains("/0000000000000000-"));
    let gap = absent(format!("{FILE}-log/{:016x}", 0));
    assert_eq!(
        stopped(None, gap),
        archived(17, files, |_| {}).0,
        "a run not from the first"
    );
}

/// Fail a pass at each of a copy's creation, its rename, and the directory force after the rename:
/// `through` stays at the last forced segment, and the next pass catches up gapless.
#[test]
fn a_failed_copy_holds_through_and_the_next_pass_catches_up_gapless() {
    for kind in ["create", "rename", "force"] {
        let mut sim = rooted(6);
        let (failed, caught_up) = api::run(&mut sim, |rt| {
            let pager = Pager::create(rt.clone(), &options(FILE)).unwrap();
            let archiver = Archiver::new(pager.clone(), DEST);
            fill(&pager, 2);
            archiver.archive();
            let full = || Action::Fail(IoError::Full);
            let renames = Match::kind(OpKind::Rename).prefixed(DEST);
            let plan = match kind {
                "create" => Plan::new().rule(Match::kind(OpKind::CreateNew).prefixed(DEST), full()),
                "rename" => Plan::new().rule(renames, full()),
                _ => {
                    let planner = rt.clone();
                    let forces = Match::kind(OpKind::SyncDir).prefixed(DEST);
                    let after = Action::Run(Box::new(move || {
                        planner.plan(Plan::new().rule(forces, full()));
                    }));
                    Plan::new().first(renames, after)
                }
            };
            rt.plan(plan);
            fill(&pager, 4);
            archiver.archive();
            let failed = archiver.status();
            rt.plan(Plan::new());
            archiver.archive();
            let caught_up = archiver.status();
            pager.close();
            (failed, caught_up)
        });
        assert_eq!(Some(Lsn(2 * SEGMENT)), failed.through, "{kind}");
        let refused = matches!(
            failed.failure,
            Some(BackupError::Io {
                error: IoError::Full,
                ..
            })
        );
        assert!(refused, "{kind}: {:?}", failed.failure);
        assert_eq!(ok(4 * SEGMENT), caught_up, "{kind}");
        let files = sim.files();
        assert_one(&files, &format!("{FILE}-log"), (0, 4 * SEGMENT));
    }
}

/// Crash a pass at each device event: no partial segment is published, and the next pass finishes
/// the archive.
#[test]
fn a_crash_during_an_archive_copy_publishes_no_partial_segment_and_the_next_pass_finishes() {
    let workload = |rt: &SimRuntime| {
        let (pager, _) = Pager::open(rt.clone(), &options(FILE)).unwrap();
        let archiver = Archiver::new(pager.clone(), DEST);
        rt.arm(Match::any().prefixed(DEST));
        archiver.archive();
        rt.disarm();
        pager.close();
    };
    let is_segment = |path: &str| {
        path.starts_with(&format!("{DEST}/"))
            && !path.contains("/epoch-")
            && !path.contains(".partial-")
            && !path.ends_with("/keep")
    };
    let log_dir = format!("{FILE}-log");
    search::each_crash(
        &filled(),
        7,
        8,
        2,
        workload,
        |image| image.keys().any(|p| is_segment(p)),
        |at, image| {
            for (path, bytes) in image.iter().filter(|(p, _)| is_segment(p)) {
                let (base, hash) = path.rsplit_once('/').unwrap().1.split_once('-').unwrap();
                assert_eq!(named(bytes), hash, "event {at}: {path}");
                assert_eq!(
                    &image[&format!("{log_dir}/{base}")],
                    bytes,
                    "event {at}: {path}"
                );
            }
            let (status, files) = archived(at as u64, image, |_| {});
            assert_eq!(ok(3 * SEGMENT), status, "event {at}");
            assert_one(&files, &log_dir, (0, 3 * SEGMENT));
        },
    );
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

/// Archive a log to 6 segments, then pass over a copy of the file as it was at 3: unchanged, and
/// with other writes to 4.
#[test]
fn an_archive_of_another_history_is_claimed_only_where_it_matches_the_log() {
    let mut sim = rooted(10);
    api::run(&mut sim, |rt| {
        let pager = Pager::create(rt, &options(FILE)).unwrap();
        fill(&pager, 3);
        pager.close();
    });
    let mut rolled = sim.files();
    api::run(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt, &options(FILE)).unwrap();
        fill(&pager, 6);
        let archiver = Archiver::new(pager.clone(), DEST);
        archiver.archive();
        assert_eq!(ok(6 * SEGMENT), archiver.status());
        pager.close();
    });
    let other = sim.files();
    rolled.extend(
        other
            .into_iter()
            .filter(|(p, _)| p.starts_with(&format!("{DEST}/"))),
    );
    let (status, files) = archived(11, rolled.clone(), |_| {});
    assert_eq!(ok(3 * SEGMENT), status, "no claim past the log's end");
    assert_eq!(epochs(&rolled), epochs(&files));
    let (status, files) = archived(12, rolled.clone(), |pager| {
        pager.allocate(1, PageKind::HeapSlotted, 9).unwrap();
        fill(pager, 4);
    });
    let dirs = epochs(&rolled);
    let held = at_base(dirs.values().next().unwrap(), 3 * SEGMENT)
        .0
        .clone();
    let refused = BackupError::Damaged { part: held };
    assert_eq!(stopped(Some(3 * SEGMENT), refused), status);
    assert_eq!(epochs(&rolled), epochs(&files), "nothing more is published");
}

#[test]
fn two_names_for_one_segment_are_refused() {
    let (status, mut files) = archived(13, filled(), |_| {});
    assert_eq!(ok(3 * SEGMENT), status);
    let first = files.keys().find(|p| p.contains("/0000000000000000-"));
    let dir = first.unwrap().rsplit_once('/').unwrap().0.to_owned();
    files.insert(
        format!("{dir}/0000000000000000-{}", named(b"other")),
        b"other".to_vec(),
    );
    let (status, _) = archived(14, files, |_| {});
    let refused = BackupError::Damaged {
        part: "0".repeat(16),
    };
    assert_eq!(stopped(None, refused), status);
}

#[test]
fn a_missing_destination_is_refused_and_not_made() {
    let mut sim = Sim::new(15, Config::default());
    let status = api::run(&mut sim, |rt| {
        let pager = Pager::create(rt, &options(FILE)).unwrap();
        fill(&pager, 1);
        let archiver = Archiver::new(pager.clone(), DEST);
        archiver.archive();
        pager.close();
        archiver.status()
    });
    assert_eq!(stopped(None, absent(DEST)), status);
    assert!(sim.files().keys().all(|p| !p.starts_with(DEST)));
}

/// Restore a backup whose log spans two segments: the restored epoch archives from the segment
/// holding its branch, under its own directory, naming where it branched.
#[test]
fn a_restored_epoch_archives_from_its_branch_to_its_own_directory() {
    let mut sim = rooted(8);
    let end = api::run(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &options(FILE)).unwrap();
        fill(&pager, 2);
        let pid = pager.allocate(1, PageKind::HeapSlotted, 7).unwrap();
        let manifest = pager
            .backup(&mut BackupRun {
                target: "bk",
                envelope: envelope(),
                visit: &mut |_, _| {
                    fill_page(&pager, pid, 3);
                    true
                },
                name: &|q| format!("{q:?}"),
            })
            .unwrap();
        fill(&pager, 5);
        let source = Archiver::new(pager.clone(), DEST);
        source.archive();
        assert_eq!(ok(5 * SEGMENT), source.status());
        pager.close();
        storage::restore_file(&rt, "bk", COPY, &format!("{COPY}-log")).unwrap();
        let (copy, _) = Pager::open(rt, &options(COPY)).unwrap();
        fill(&copy, 6);
        let archiver = Archiver::new(copy.clone(), DEST);
        archiver.archive();
        assert_eq!(ok(6 * SEGMENT), archiver.status());
        copy.close();
        manifest.end
    });
    let files = sim.files();
    let dirs = epochs(&files);
    let id = dirs.keys().next().unwrap().split_once('/').unwrap().0;
    let source = format!("{id}/{:016x}", end.epoch);
    assert_eq!(2, dirs.len());
    let log = format!("{FILE}-log");
    assert_eq!(
        NO_PARENT,
        assert_archived(&dirs[&source], &files, &log, (0, 5 * SEGMENT))
    );
    assert!(end.lsn.0 < 5 * SEGMENT, "the backup's end is covered");
    let (restored, entries) = dirs.iter().find(|(d, _)| **d != source).unwrap();
    assert_eq!(id, restored.split_once('/').unwrap().0);
    let branch = end.lsn.0 - end.lsn.0 % SEGMENT;
    let lowest = segments(&files, &format!("{COPY}-log"))[0];
    assert!(lowest < branch, "the backup's log spans two segments");
    let parent = assert_archived(
        entries,
        &files,
        &format!("{COPY}-log"),
        (branch, 6 * SEGMENT),
    );
    assert_eq!(
        format!("adtb archive epoch\nparent {} {}\n", end.epoch, end.lsn.0),
        parent
    );
    let (source, restored) = (at_base(&dirs[&source], branch), at_base(entries, branch));
    assert_ne!(
        source.1, restored.1,
        "the two epochs wrote that segment differently"
    );
}

/// The engine's worker archives as segments complete, and its close archives the rest.
#[test]
fn an_engine_shows_its_archive_and_archives_into_it() {
    let mut sim = rooted(9);
    let through = api::run(&mut sim, |rt| {
        let options = OpenOptions {
            log_archive: Some(DEST.into()),
            create: Some(CreateOptions::default()),
            testing: Knobs::default().with_segment_size(SEGMENT),
            ..open::options(2)
        };
        let db = Database::open_in(rt.clone(), "db.adt", &Log::new([APP]), &options).unwrap();
        let shown = api::query(
            &db,
            r#"from o in sys::process_options where o.name == "log_archive" select o.value"#,
        );
        assert_eq!(vec![format!("\"{DEST}\"")], shown);
        insert(&db, 0..ROWS);
        common::sleep(&rt, &mut rt.io(), Duration::from_secs(3));
        let through = db.testing().archive_status().expect("an archive is set");
        insert(&db, ROWS..2 * ROWS);
        db.close().unwrap();
        through
    });
    assert_eq!(None, through.failure);
    let through = through.through.expect("the worker ran").0;
    let files = sim.files();
    let log = "db.adt-log";
    let last = *segments(&files, log).last().unwrap();
    assert!(
        0 < through && through < last,
        "the worker archived some, the close the rest: {through} {last}"
    );
    assert_eq!(NO_PARENT, assert_one(&files, log, (0, last)));
}

const ROWS: i64 = 8;
const APP: &str = r"module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
";

fn insert<R: Runtime>(db: &Database<R>, ids: std::ops::Range<i64>) {
    for id in ids {
        let name = "x".repeat(4000);
        let text = format!(r#"insert users {{ id: users.key({id}), name: "{name}" }}"#);
        db.query_text(&text, &[], &CallOptions::default())
            .unwrap_or_else(|e| panic!("`{text}` fails: {e:?}"));
    }
}

/// A file made after a dumped position starts its own log at 0: it archives from there and names
/// no parent.
#[test]
fn a_new_file_after_a_position_archives_its_own_log_from_the_start() {
    let mut sim = rooted(18);
    let status = api::run(&mut sim, |rt| {
        let after = Position {
            epoch: 1,
            lsn: Lsn(10 * SEGMENT + 5),
        };
        let pager = Pager::create(
            rt,
            &Options {
                after: Some(after),
                ..options(FILE)
            },
        )
        .unwrap();
        fill(&pager, 3);
        let archiver = Archiver::new(pager.clone(), DEST);
        archiver.archive();
        pager.close();
        archiver.status()
    });
    assert_eq!(ok(3 * SEGMENT), status);
    assert_eq!(
        NO_PARENT,
        assert_one(&sim.files(), &format!("{FILE}-log"), (0, 3 * SEGMENT))
    );
}

#[test]
fn a_segment_size_off_the_log_block_is_refused() {
    let mut sim = rooted(19);
    let refused = api::run(&mut sim, |rt| {
        let options = OpenOptions {
            create: Some(CreateOptions::default()),
            testing: Knobs::default().with_segment_size(SEGMENT + 1),
            ..open::options(2)
        };
        match Database::open_in(rt, "db.adt", &Log::new([APP]), &options) {
            Err(adtb::OpenError::Io(e)) => (e.kind(), e.to_string()),
            other => panic!("not refused as invalid input: {:?}", other.err()),
        }
    });
    let text = format!(
        "segment size {} is not a positive multiple of the log block",
        SEGMENT + 1
    );
    assert_eq!((std::io::ErrorKind::InvalidInput, text), refused);
}

/// Open `db.adt` of `sim` with an archive when `archive`, failing every op `refused` names,
/// insert `rows` and close. Returns the archive's status after the open, or the open's refusal.
fn engine_session(
    sim: &mut Sim,
    archive: bool,
    refused: Match,
    rows: std::ops::Range<i64>,
) -> Result<Option<ArchiveStatus>, adtb::OpenError> {
    api::run(sim, |rt| {
        rt.plan(Plan::new().rule(refused, Action::Fail(IoError::Full)));
        let options = OpenOptions {
            log_archive: archive.then(|| DEST.into()),
            create: Some(CreateOptions::default()),
            testing: Knobs::default().with_segment_size(SEGMENT),
            ..open::options(2)
        };
        let db = Database::open_in(rt, "db.adt", &Log::new([APP]), &options)?;
        let status = db.testing().archive_status();
        insert(&db, rows);
        db.close().unwrap();
        Ok(status)
    })
}

fn none() -> Match {
    Match::kinds([])
}

fn archive_creates() -> Match {
    Match::kind(OpKind::CreateNew).prefixed(DEST)
}

#[test]
fn an_engine_recycles_only_the_segments_its_archive_holds() {
    let mut sim = rooted(20);
    engine_session(&mut sim, true, none(), 0..3 * ROWS).unwrap();
    engine_session(&mut sim, true, archive_creates(), 3 * ROWS..6 * ROWS).unwrap();
    let behind = sim.files();
    let log = "db.adt-log";
    let status = engine_session(&mut sim, true, archive_creates(), 0..0).unwrap();
    let status = status.expect("an archive is set");
    let through = status
        .through
        .expect("the open's pass listed the archive")
        .0;
    let kept = segments(&sim.files(), log);
    assert!(
        through > SEGMENT && *segments(&behind, log).last().unwrap() > through + SEGMENT,
        "the archive holds some of the log, and less than all: {through}"
    );
    assert_eq!(through, kept[0], "{kept:?}");
    let mut unarchived = Sim::with_files(21, Config::default(), behind);
    engine_session(&mut unarchived, false, none(), 0..0).unwrap();
    let first = segments(&unarchived.files(), log)[0];
    assert!(
        first > through,
        "without the archive the recycle removes more: {first}"
    );
}

#[test]
fn a_create_that_fails_to_publish_its_log_archives_nothing() {
    let mut sim = rooted(22);
    let published = Match::kind(OpKind::Rename).path("db.adt-log");
    let refused = engine_session(&mut sim, true, published, 0..0).is_err();
    let under: Vec<String> = sim
        .files()
        .into_keys()
        .filter(|p| p.starts_with(DEST))
        .collect();
    assert_eq!((true, vec![format!("{DEST}/keep")]), (refused, under));
}
