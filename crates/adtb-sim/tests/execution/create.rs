//! Creating a database: a crash anywhere in it leaves no database or a whole one, a retried
//! create works with nothing removed by hand, and a create removes nothing it did not make.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use adtb::runtime::{OpKind, Runtime};
use adtb::storage::{aside, creating, marker, marker_bytes};
use adtb::{Database, Log, OpenError, OpenOptions, TxnCallOptions, TxnOptions};
use adtb_sim::{Config, Match, Sim, SimRuntime};
use adtb_simkit::api;
use adtb_simkit::open;
use adtb_simkit::search::{self, Files};
use adtb_testkit::profile::{assert_reached, every_case};

const FILE: &str = "create.adt";
const APP: &str = r"module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
";
const CRASH_POINTS: u64 = 1;
const HEADER: usize = 4096;
const RESERVED: &str = "the reserved directory of a create holds what no create left";
const OTHER: [u8; 16] = [9; 16];

fn log() -> Log {
    Log::new([APP])
}

fn no_create() -> OpenOptions {
    OpenOptions {
        create: None,
        ..open::options(2)
    }
}

/// Where a crash left the create, by the names the image holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Stage {
    Building,
    Built,
    BetweenRenames,
    Published,
}

fn stage(image: &Files) -> Stage {
    let reserved = format!("{}/", creating(FILE));
    let built = image.iter().any(|(p, b)| {
        p.strip_prefix(&reserved)
            .is_some_and(|f| f.len() == 32 && b.iter().take(HEADER).any(|&x| x != 0))
    });
    if image.contains_key(FILE) {
        Stage::Published
    } else if image.keys().any(|p| p.starts_with(&format!("{FILE}-log/"))) {
        Stage::BetweenRenames
    } else if built {
        Stage::Built
    } else {
        Stage::Building
    }
}

fn every_op() -> Match {
    Match::any().except(Match::kinds([OpKind::Close, OpKind::Timer]))
}

/// Open `FILE` without `create`, else create it, then commit a row. Returns the names of the
/// users a later open reads.
fn opened_or_retried<R: Runtime>(
    open: impl Fn(&OpenOptions) -> Result<Database<R>, OpenError>,
) -> Vec<String> {
    let names = |db: &Database<R>| {
        let rows = db.query_text("from u in users select u.name", &[], &Default::default());
        api::sorted(rows.unwrap().remove(0))
    };
    let db = match open(&no_create()) {
        Ok(db) => {
            assert_eq!(Vec::<String>::new(), names(&db));
            db
        }
        Err(OpenError::NotFound) => open(&open::options(2)).expect("the retried create"),
        Err(e) => panic!("an open after the crash: {e:?}"),
    };
    if every_case() {
        assert!(db.testing().verify_holding().unwrap().next().is_none());
    }
    let mut t = db.begin(&TxnOptions::default()).unwrap();
    let insert = r#"insert users { id: users.key(1), name: "ann" }"#;
    t.query_text(insert, &[], &TxnCallOptions::default())
        .unwrap();
    t.commit(None).unwrap();
    db.close().unwrap();
    let db = open(&no_create()).expect("the reopen");
    let read = names(&db);
    db.close().unwrap();
    read
}

fn reserved(names: impl IntoIterator<Item = String>) -> Vec<String> {
    let aside = creating(FILE);
    names
        .into_iter()
        .filter(|p| p.starts_with(&aside))
        .collect()
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("create")
        .join(name);
    match std::fs::remove_dir_all(&dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => panic!("clear {dir:?}: {e}"),
        _ => {}
    }
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_image(dir: &Path, image: &Files) {
    for (path, bytes) in image {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let used = bytes.iter().rposition(|&b| b != 0).map_or(0, |at| at + 1);
        std::fs::write(&path, &bytes[..used]).unwrap();
        let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_len(bytes.len() as u64).unwrap();
    }
}

fn read_image(dir: &Path, at: &Path, into: &mut Files) {
    for entry in std::fs::read_dir(at).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            read_image(dir, &path, into);
        } else {
            let name = path.strip_prefix(dir).unwrap().to_str().unwrap().to_owned();
            into.insert(name, std::fs::read(&path).unwrap());
        }
    }
}

/// Crash a create at each device event: an open finds an empty database or nothing, then a
/// retried create works, a commit reads back and no reserved name is left. Under the deep
/// profile the database verifies whole, and one image of each stage that leaves reserved names
/// is retried again on a real file through the host's open.
#[test]
fn a_create_crashed_anywhere_is_retried() {
    let workload = |rt: &SimRuntime| {
        rt.arm(every_op());
        let db = Database::open_in(rt.clone(), FILE, &log(), &open::options(2)).unwrap();
        rt.disarm();
        db.close().unwrap();
    };
    let mut reached = BTreeMap::new();
    let mut images = BTreeMap::new();
    search::each_crash(
        &Files::new(),
        1,
        CRASH_POINTS,
        if every_case() { 2 } else { 1 },
        workload,
        |image| image.contains_key(FILE),
        |at, image| {
            let stage = stage(&image);
            *reached.entry(stage).or_insert(0) += 1;
            if every_case() && stage != Stage::Building {
                images.entry(stage).or_insert_with(|| image.clone());
            }
            let first_block = |files: &Files| {
                let segment = files.get(&format!("{FILE}-log/0000000000000000"));
                segment.map(|b| b[..512].to_vec())
            };
            let logged = first_block(&image);
            let mut sim = Sim::with_files(at as u64, Config::default(), image);
            let names = api::run(&mut sim, |rt| {
                opened_or_retried(|o| Database::open_in(rt.clone(), FILE, &log(), o))
            });
            assert_eq!(vec![r#""ann""#], names, "event {at}, {stage:?}");
            let left = reserved(sim.files().into_keys().chain(sim.dirs()));
            assert_eq!(Vec::<String>::new(), left, "event {at}, {stage:?}");
            if stage == Stage::BetweenRenames {
                let now = first_block(&sim.files());
                assert_eq!(logged, now, "event {at}: rolled forward");
            }
        },
    );
    for stage in [
        Stage::Building,
        Stage::Built,
        Stage::BetweenRenames,
        Stage::Published,
    ] {
        let count = reached.get(&stage).copied().unwrap_or(0);
        assert_reached(&format!("a crash {stage:?}"), count, 1);
    }
    for (stage, image) in images {
        let dir = scratch(&format!("{stage:?}"));
        write_image(&dir, &image);
        let path = dir.join(FILE);
        let names = opened_or_retried(|o| adtb::open(&path, &log(), o));
        assert_eq!(vec![r#""ann""#], names, "{stage:?} on the host");
        let left = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap());
        assert_eq!(
            Vec::<String>::new(),
            reserved(left),
            "{stage:?} on the host"
        );
    }
}

/// A create under a path whose parent is a regular file answers the host's error of that kind,
/// on the simulator and on the host.
#[test]
fn a_create_under_a_regular_file_is_not_a_directory() {
    let plain = Files::from([("plain".to_owned(), Vec::new())]);
    let mut sim = Sim::with_files(1, Config::default(), plain);
    let got = api::run(&mut sim, |rt| {
        Database::open_in(rt, "plain/create.adt", &log(), &open::options(2)).map(drop)
    });
    let Err(OpenError::Io(e)) = got else {
        panic!("{got:?}")
    };
    assert_eq!(std::io::ErrorKind::NotADirectory, e.kind());

    let dir = scratch("under-a-file");
    std::fs::write(dir.join("plain"), b"").unwrap();
    let got = adtb::open(dir.join("plain").join(FILE), &log(), &open::options(2)).map(drop);
    let Err(OpenError::Io(e)) = got else {
        panic!("{got:?}")
    };
    assert_eq!(std::io::ErrorKind::NotADirectory, e.kind());
}

/// A create in a directory that does not exist answers the host's error, not damage.
#[test]
fn a_create_in_a_missing_directory_is_not_found() {
    let got = api::once(1, |rt| {
        Database::open_in(rt, "missing/create.adt", &log(), &open::options(2)).map(drop)
    });
    let Err(OpenError::Io(e)) = got else {
        panic!("{got:?}")
    };
    assert_eq!(std::io::ErrorKind::NotFound, e.kind());
}

/// Returns the files a create of `FILE` that finished on seed `seed` leaves.
fn created(seed: u64) -> Files {
    static MADE: Mutex<BTreeMap<u64, Files>> = Mutex::new(BTreeMap::new());
    let mut made = MADE.lock().unwrap();
    let files = made.entry(seed).or_insert_with(|| {
        let mut sim = Sim::new(seed, Config::default());
        api::run(&mut sim, |rt| {
            let db = Database::open_in(rt, FILE, &log(), &open::options(2)).unwrap();
            db.close().unwrap();
        });
        sim.files()
    });
    files.clone()
}

/// Returns `files` with every name under `from` moved under `to`.
fn moved(files: Files, from: &str, to: &str) -> Files {
    files
        .into_iter()
        .map(|(p, b)| match p.strip_prefix(from) {
            Some(rest) => (format!("{to}{rest}"), b),
            None => (p, b),
        })
        .collect()
}

/// Returns a database a create left in the reserved directory with its marker and its log
/// directory, and the create's id.
fn built() -> (Files, [u8; 16]) {
    let mut files = created(1);
    let data = files.remove(FILE).unwrap();
    let id: [u8; 16] = data[24..40].try_into().unwrap();
    let file = aside(FILE, &id);
    let mut files = moved(files, &format!("{FILE}-log"), &format!("{file}-log"));
    files.insert(file, data);
    files.insert(marker(FILE), marker_bytes(&id));
    (files, id)
}

/// Create `FILE` over `files` on the simulator. Returns the outcome and the files after it.
fn create_over(files: Files) -> (Result<(), OpenError>, Files) {
    let mut sim = Sim::with_files(1, Config::default(), files);
    let got = api::run(&mut sim, |rt| {
        Database::open_in(rt, FILE, &log(), &open::options(2)).map(|db| db.close().unwrap())
    });
    (got, sim.files())
}

fn is_reserved(got: &Result<(), OpenError>) -> bool {
    matches!(got, Err(OpenError::Corrupt { detail }) if detail == RESERVED)
}

/// Plant at the reserved names every shape a create could mistake for its own: a database at
/// the reserved directory's name, a database with its log inside it, a marker naming another
/// create than the file beside it, a marker beside a file no create makes, a torn marker, a
/// file of no create without a marker. A create of `FILE` over each refuses as reserved and
/// every planted byte stays.
#[test]
fn a_create_removes_nothing_it_did_not_make() {
    let dir = creating(FILE);
    let inside = format!("{dir}/{FILE}");
    let (base, id) = built();
    let mut torn = marker_bytes(&id);
    torn[10] ^= 1;
    let mut cases: Vec<(&str, Files)> = vec![
        (
            "a database named as the reserved directory",
            moved(created(1), FILE, &dir),
        ),
        (
            "a database inside the reserved directory",
            moved(created(1), FILE, &inside),
        ),
        ("a user's file named as the marker", {
            Files::from([(marker(FILE), b"mine".to_vec())])
        }),
    ];
    let mut other = base.clone();
    other.insert(marker(FILE), marker_bytes(&OTHER));
    cases.push(("a marker of another create", other));
    let mut beside = base.clone();
    beside.insert(format!("{dir}/notes"), b"keep".to_vec());
    cases.push(("a marker beside a user's file", beside));
    let mut nested = base.clone();
    nested.insert(format!("{}-log/notes", aside(FILE, &id)), b"keep".to_vec());
    cases.push(("a user's file in the built log directory", nested));
    let mut torn_built = base.clone();
    torn_built.insert(marker(FILE), torn);
    cases.push(("a torn marker", torn_built));
    let mut unmarked = base;
    unmarked.remove(&marker(FILE));
    cases.push(("no marker", unmarked));
    let mut padded = vec![0; 1 << 16];
    padded.extend_from_slice(b"data");
    cases.push((
        "a user's file of zeros then data named as the marker",
        Files::from([(marker(FILE), padded)]),
    ));
    for (case, files) in cases {
        let (got, after) = create_over(files.clone());
        assert!(is_reserved(&got), "{case}: {got:?}");
        assert_eq!(files, after, "{case}");
    }
}

/// A create finding a build of an earlier create, with or without its log directory beside it,
/// and a log at `FILE`'s own name that no create here wrote refuses, leaving that log and never
/// publishing the build. With no log at `FILE`'s own name it removes the build and creates anew.
#[test]
fn a_create_leaves_a_log_no_create_here_wrote() {
    let foreign = moved(created(2), FILE, &format!("{FILE}.old"));
    let foreign = moved(foreign, &format!("{FILE}.old-log"), &format!("{FILE}-log"));
    let file = aside(FILE, &OTHER);
    let build = Files::from([
        (marker(FILE), marker_bytes(&OTHER)),
        (file.clone(), b"built".to_vec()),
    ]);
    let logs = |files: Files| -> Files {
        let log = format!("{FILE}-log/");
        files
            .into_iter()
            .filter(|(p, _)| p.starts_with(&log))
            .collect()
    };
    let log_present = "the log directory holds a log that no file here wrote";
    for own_log in [true, false] {
        let mut files = build.clone();
        if own_log {
            files.insert(format!("{file}-log/0000000000000000"), vec![0; 512]);
        }
        files.extend(foreign.clone());
        let (got, after) = create_over(files);
        assert!(
            matches!(&got, Err(OpenError::Corrupt { detail }) if *detail == log_present),
            "own log {own_log}: {got:?}"
        );
        assert!(!after.contains_key(FILE), "own log {own_log}");
        assert_eq!(logs(foreign.clone()), logs(after), "own log {own_log}");
    }
    let (got, after) = create_over(build);
    got.unwrap();
    assert!(after.get(FILE).is_some_and(|b| b[24..40] != OTHER));
    assert_eq!(Vec::<String>::new(), reserved(after.into_keys()));
}

/// On the host a create refuses as reserved when the reserved directory holds a symbolic link,
/// leaving what the link names.
#[test]
fn a_create_over_a_link_in_the_reserved_directory_is_reserved() {
    let dir = scratch("linked");
    let file = aside(FILE, &OTHER);
    write_image(
        &dir,
        &Files::from([
            (marker(FILE), marker_bytes(&OTHER)),
            (file.clone(), b"built".to_vec()),
            ("mine/0000000000000000".to_owned(), b"keep".to_vec()),
        ]),
    );
    std::os::unix::fs::symlink(dir.join("mine"), dir.join(format!("{file}-log"))).unwrap();
    let got = adtb::open(dir.join(FILE), &log(), &open::options(2)).map(drop);
    assert!(is_reserved(&got), "{got:?}");
    let kept = std::fs::read(dir.join("mine/0000000000000000")).unwrap();
    assert_eq!(b"keep".to_vec(), kept);
}

/// A create finds the reserved directory holding only a marker of zeros, as a crash before the
/// marker's force leaves it, and creates in it.
#[test]
fn a_create_reuses_a_reserved_directory_with_nothing_written() {
    let (got, after) = create_over(Files::from([(marker(FILE), vec![0; 512])]));
    got.unwrap();
    assert!(after.contains_key(FILE));
    assert_eq!(Vec::<String>::new(), reserved(after.into_keys()));
}

/// On the host a create refuses as locked while another open holds the reserved file of an
/// earlier build, with every byte left, and removes the build once the lock is released.
#[test]
fn a_create_over_a_held_reserved_file_is_locked() {
    let dir = scratch("held");
    let file = aside(FILE, &OTHER);
    let planted = Files::from([
        (marker(FILE), marker_bytes(&OTHER)),
        (file.clone(), b"built".to_vec()),
        (format!("{file}-log/0000000000000000"), vec![0; 512]),
    ]);
    write_image(&dir, &planted);
    let held = std::fs::File::open(dir.join(&file)).unwrap();
    held.try_lock().unwrap();
    let got = adtb::open(dir.join(FILE), &log(), &open::options(2)).map(drop);
    assert!(
        matches!(got, Err(OpenError::Locked { pid: None })),
        "{got:?}"
    );
    let mut after = Files::new();
    read_image(&dir, &dir, &mut after);
    assert_eq!(planted, after);
    drop(held);
    let db = adtb::open(dir.join(FILE), &log(), &open::options(2)).expect("the create");
    db.close().unwrap();
    assert!(!dir.join(creating(FILE)).exists());
}

/// Returns the id of the database `created(1)` leaves.
fn created_id() -> [u8; 16] {
    created(1)[FILE][24..40].try_into().unwrap()
}

fn under_reserved(files: &Files) -> Files {
    let reserved = format!("{}/", creating(FILE));
    files
        .iter()
        .filter(|(p, _)| p.starts_with(&reserved))
        .map(|(p, b)| (p.clone(), b.clone()))
        .collect()
}

/// Returns the open options without `create`, and under the deep profile with it too.
fn either_open() -> Vec<OpenOptions> {
    let mut each = vec![no_create()];
    if every_case() {
        each.push(open::options(2));
    }
    each
}

/// Open `FILE` over `files` on the simulator with each of [`either_open`]. Returns the files and
/// the directories after each open.
fn opened_over(files: &Files) -> Vec<(Files, BTreeSet<String>)> {
    either_open()
        .into_iter()
        .map(|options| {
            let mut sim = Sim::with_files(1, Config::default(), files.clone());
            api::run(&mut sim, |rt| {
                let db = Database::open_in(rt, FILE, &log(), &options).expect("the open");
                db.close().unwrap();
            });
            (sim.files(), sim.dirs())
        })
        .collect()
}

/// An open of a whole database, with or without `create`, removes a reserved directory holding
/// only a whole marker naming the database's id, and leaves byte for byte one holding a marker
/// of another id, a torn marker, or another entry beside the marker, the file or the log
/// directory it names among them. On the host it leaves a
/// linked marker too.
#[test]
fn a_whole_open_removes_only_its_own_leftover_marker() {
    let id = created_id();
    let mut torn = marker_bytes(&id);
    torn[10] ^= 1;
    let own = Files::from([(marker(FILE), marker_bytes(&id))]);
    for (after, dirs) in opened_over(&created(1).into_iter().chain(own).collect()) {
        assert_eq!(
            Vec::<String>::new(),
            reserved(after.into_keys().chain(dirs))
        );
    }
    let file = aside(FILE, &id);
    let cases = [
        (
            "a marker of another id",
            vec![(marker(FILE), marker_bytes(&OTHER))],
        ),
        ("a torn marker", vec![(marker(FILE), torn)]),
        (
            "another entry beside the marker",
            vec![
                (marker(FILE), marker_bytes(&id)),
                (format!("{}/notes", creating(FILE)), b"keep".to_vec()),
            ],
        ),
        (
            "the file the marker names beside it",
            vec![
                (marker(FILE), marker_bytes(&id)),
                (file.clone(), b"keep".to_vec()),
            ],
        ),
        (
            "the log directory the marker names beside it",
            vec![
                (marker(FILE), marker_bytes(&id)),
                (format!("{file}-log/0000000000000000"), b"keep".to_vec()),
            ],
        ),
    ];
    let mut ran = 0;
    for (case, planted) in cases {
        let files: Files = created(1).into_iter().chain(planted).collect();
        for (after, dirs) in opened_over(&files) {
            assert_eq!(under_reserved(&files), under_reserved(&after), "{case}");
            assert!(dirs.contains(&creating(FILE)), "{case}: {dirs:?}");
            ran += 1;
        }
    }
    assert_reached("an open over a reserved directory it does not own", ran, 5);
    linked_leftover_markers_stay(&id);
}

/// On the host a whole open leaves a marker naming its id that is a symbolic link, or in a
/// reserved directory that is one, and still opens.
fn linked_leftover_markers_stay(id: &[u8; 16]) {
    let mut ran = 0;
    for linked_dir in [false, true] {
        let dir = scratch(&format!("leftover-linked-{linked_dir}"));
        write_image(&dir, &created(1));
        let target = dir.join("mine");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("id"), marker_bytes(id)).unwrap();
        let reserved = dir.join(creating(FILE));
        if linked_dir {
            std::os::unix::fs::symlink(&target, &reserved).unwrap();
        } else {
            std::fs::create_dir(&reserved).unwrap();
            std::os::unix::fs::symlink(target.join("id"), dir.join(marker(FILE))).unwrap();
        }
        for options in either_open() {
            let db = adtb::open(dir.join(FILE), &log(), &options).expect("the open");
            db.close().unwrap();
            let what = format!("linked dir {linked_dir}");
            assert_eq!(
                marker_bytes(id),
                std::fs::read(target.join("id")).unwrap(),
                "{what}"
            );
            assert!(reserved.symlink_metadata().is_ok(), "{what}");
            assert!(dir.join(marker(FILE)).symlink_metadata().is_ok(), "{what}");
            ran += 1;
        }
    }
    assert_reached("a host open over a linked leftover marker", ran, 2);
}

/// An empty file beside the log directory its own create published, the file still in the
/// reserved directory, opens without `create` as a file with no header and is rolled forward by
/// a create.
#[test]
fn an_empty_file_beside_its_own_published_log_is_rolled_forward() {
    let (built, id) = built();
    let mut files = moved(
        built,
        &format!("{}-log", aside(FILE, &id)),
        &format!("{FILE}-log"),
    );
    files.insert(FILE.to_owned(), Vec::new());
    let mut sim = Sim::with_files(1, Config::default(), files.clone());
    let got = api::run(&mut sim, |rt| {
        Database::open_in(rt, FILE, &log(), &no_create()).map(drop)
    });
    assert!(
        matches!(&got, Err(OpenError::Corrupt { detail }) if detail == "NoHeader"),
        "{got:?}"
    );
    assert_eq!(files, sim.files());
    let (got, after) = create_over(files);
    got.unwrap();
    assert_eq!(id, after[FILE][24..40]);
    assert_eq!(Vec::<String>::new(), reserved(after.into_keys()));
}
