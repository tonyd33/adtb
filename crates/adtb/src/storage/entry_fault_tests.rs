//! Each API entry with a read the device refuses at each file and page it reads: the caller is
//! told a typed failure naming the page or file, or the entry reports the page as damage.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Mutex;

use adtb_sim::{Action, Config, Match, Plan, Sim, SimRuntime};
use adtb_testkit::damage::DataFile;
use adtb_testkit::process;
use adtb_testkit::profile::{assert_reached, count, every_case, spread};
use adtb_testkit::refused_as;
use adtb_testkit::search::Files;

use crate::runtime::{IoError, OpKind};
use crate::storage::{PageKind, Pid, kind, read_manifest};
use crate::test_world::api;
use crate::test_world::load as open;
use crate::{
    ArchiveTarget, CallOptions, Database, Error, FatalReason, Log, OpenError, OpenOptions,
    RestorePoint, TxnCallOptions, TxnOptions, restore_in,
};

const FILE: &str = "entry.adt";
const PAGE: usize = 8192;
const ARCHIVE: &str = "bk";
const SECOND: &str = "bk2";
const COPY: &str = "copy.adt";
const SEED: u64 = 5;
const APP: &str = r"module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
  index (name)
";

/// The reads of each entry a default run fails.
const READS: u64 = 3;

/// A call a client makes that reads a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Entry {
    Open,
    Call,
    Commit,
    Verify,
    Backup,
    Dump,
    Restore,
}

impl Entry {
    /// Returns every entry, in the order of their declaration.
    fn every() -> Vec<Entry> {
        std::iter::successors(Some(Entry::Open), |e| match e {
            Entry::Open => Some(Entry::Call),
            Entry::Call => Some(Entry::Commit),
            Entry::Commit => Some(Entry::Verify),
            Entry::Verify => Some(Entry::Backup),
            Entry::Backup => Some(Entry::Dump),
            Entry::Dump => Some(Entry::Restore),
            Entry::Restore => None,
        })
        .collect()
    }

    /// Returns the reads of the files the entry is for.
    fn reads(self) -> Match {
        match self {
            Entry::Restore => Match::kind(OpKind::Read).prefixed(ARCHIVE),
            Entry::Open
            | Entry::Call
            | Entry::Commit
            | Entry::Verify
            | Entry::Backup
            | Entry::Dump => Match::kind(OpKind::Read).path(FILE),
        }
    }
}

/// What a caller was told.
#[derive(Debug, PartialEq)]
enum Told {
    Open(std::io::ErrorKind, String),
    Refused(Error),
}

impl fmt::Display for Told {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Told::Open(_, text) => f.write_str(text),
            Told::Refused(e) => write!(f, "{e}"),
        }
    }
}

/// What an entry that finished reported of the damage it met.
#[derive(Debug, PartialEq)]
enum Reported {
    Nothing,
    /// The `where` of each checksum finding `verify` answered.
    Findings(Vec<String>),
    /// The pages a backup's manifest records as damaged.
    Damaged(Vec<Pid>),
    /// Verify answered no checksum finding, and the close that followed wrote these pages.
    Rewritten(BTreeSet<Pid>),
}

fn log() -> Log {
    Log::new([APP])
}

fn options() -> OpenOptions {
    OpenOptions {
        create: None,
        ..open::options(2)
    }
}

fn insert(id: i64) -> String {
    format!(r#"insert users {{ id: users.key({id}), name: "user {id}" }}"#)
}

/// Returns a file of 40 users, closed cleanly, with a backup of it in [`ARCHIVE`].
fn seeded() -> Files {
    let mut sim = Sim::new(SEED, Config::default());
    process::run(&mut sim, |rt| {
        let db = open::opened(&rt, FILE, &log(), &open::options(2));
        for id in 0..40 {
            api::results(&db, &insert(id));
        }
        api::results(&db, &format!("backup to \"{ARCHIVE}\""));
        db.close().unwrap();
    });
    sim.files()
}

/// Run `entry` on `rt`, calling `window` with `true` where the entry begins and `false` where it
/// ends. Returns what it reported, or what its caller was told.
fn run(entry: Entry, rt: &SimRuntime, window: &dyn Fn(bool)) -> Result<Reported, Told> {
    let refused = Told::Refused;
    let within = |body: &dyn Fn() -> Result<Reported, Told>| {
        window(true);
        let told = body();
        window(false);
        told
    };
    let call = |db: &Database<SimRuntime>, text: &str| {
        db.query_text(text, &[], &CallOptions::default())
            .map(|_| Reported::Nothing)
            .map_err(refused)
    };
    match entry {
        Entry::Open => {
            window(true);
            let db = match open::open_in(rt, FILE, &log(), &options()) {
                Ok(db) => db,
                Err(OpenError::Io(e)) => {
                    window(false);
                    return Err(Told::Open(e.kind(), e.to_string()));
                }
                Err(e) => panic!("the open is told {e:?}"),
            };
            window(false);
            call(&db, "from u in users select u.id")
        }
        Entry::Call => {
            let db = open::opened(rt, FILE, &log(), &options());
            within(&|| call(&db, &insert(100)))
        }
        Entry::Commit => {
            let db = open::opened(rt, FILE, &log(), &options());
            within(&|| {
                let mut t = db.begin(&TxnOptions::default()).map_err(refused)?;
                t.query_text(&insert(100), &[], &TxnCallOptions::default())
                    .map_err(refused)?;
                t.commit(None).map(|_| Reported::Nothing).map_err(refused)
            })
        }
        Entry::Verify => {
            let db = open::opened(rt, FILE, &log(), &options());
            within(&|| db.verify(None).map(|_| Reported::Nothing).map_err(refused))?;
            let found = "from f in sys::verify_findings.Checksum select f.where";
            let found = api::query(&db, found);
            if !found.is_empty() {
                return Ok(Reported::Findings(found));
            }
            let writes = rt.logged(Match::kind(OpKind::Write).path(FILE));
            db.close().unwrap();
            let page = PAGE as u64;
            let spans = writes.entries();
            let spans = spans
                .iter()
                .filter_map(|op| Some((op.offset()?, op.length()?)));
            let pages = spans.flat_map(|(at, len)| (at / page..(at + len).div_ceil(page)).map(Pid));
            Ok(Reported::Rewritten(pages.collect()))
        }
        Entry::Backup => {
            let db = open::opened(rt, FILE, &log(), &options());
            within(&|| call(&db, &format!("backup to \"{SECOND}\"")))?;
            let (manifest, _) = read_manifest(rt, SECOND).unwrap();
            Ok(Reported::Damaged(manifest.damaged))
        }
        Entry::Dump => {
            let db = open::opened(rt, FILE, &log(), &options());
            within(&|| call(&db, "dump to \"d\""))
        }
        Entry::Restore => within(&|| {
            let archive = ArchiveTarget::path(ARCHIVE).unwrap();
            restore_in(rt.clone(), &archive, COPY, RestorePoint::Latest, &options())
                .map(|_| Reported::Nothing)
                .map_err(refused)
        }),
    }
}

/// Returns each file and offset `entry` reads between its start and its end.
fn reads_of(files: &Files, entry: Entry) -> Vec<(String, u64)> {
    let mut sim = Sim::with_files(SEED, Config::default(), files.clone());
    let logged = Mutex::new(None);
    let read = Mutex::new(BTreeSet::new());
    process::run(&mut sim, |rt| {
        let window = |open: bool| {
            let mut logged = logged.lock().unwrap();
            if open {
                *logged = Some(rt.logged(entry.reads()));
            } else if let Some(log) = logged.take() {
                let ops = log.entries();
                let ops = ops
                    .iter()
                    .filter_map(|op| Some((op.path()?.to_owned(), op.offset()?)));
                read.lock().unwrap().extend(ops);
            }
        };
        run(entry, &rt, &window).unwrap();
    });
    read.into_inner().unwrap().into_iter().collect()
}

/// Returns what `entry` reported, or what its caller was told, when every read of `path` covering
/// `at` fails between its start and its end.
fn failed_at(files: &Files, entry: Entry, path: &str, at: u64) -> Result<Reported, Told> {
    let mut sim = Sim::with_files(SEED, Config::default(), files.clone());
    process::run(&mut sim, |rt| {
        let window = |open: bool| {
            let mut plan = Plan::new();
            if open {
                let refused = Match::kind(OpKind::Read).path(path).covers(at);
                plan = plan.rule(refused, Action::Fail(IoError::Failed));
            }
            rt.plan(plan);
        };
        run(entry, &rt, &window)
    })
}

/// Returns the kind of page `pid` of [`FILE`] in `files`.
fn kind_of(files: &Files, pid: Pid) -> Option<PageKind> {
    let data = DataFile::new(FILE, PAGE);
    data.bytes(files)
        .get(data.range(pid))
        .and_then(|page| PageKind::from_u16(kind(page)))
}

fn storage(detail: String) -> (Told, String) {
    let reason = format!("fatal: storage failure: {detail}");
    let told = Told::Refused(Error::Fatal {
        reason: FatalReason::Storage { detail },
    });
    (told, reason)
}

/// Returns the failure the caller of `entry` is told when the read of `path` at `at`, a page of
/// kind `kind`, fails, and its text. An open past an unreadable damage list page holds the
/// database for that open: a call on it is refused.
fn expected(entry: Entry, path: &str, at: u64, kind: Option<PageKind>) -> (Told, String) {
    let page = at / PAGE as u64;
    let unreadable = format!("page {page} is unreadable");
    match entry {
        Entry::Open if kind == Some(PageKind::DamageList) => {
            let what = "the database".to_owned();
            let told = Told::Refused(Error::Fatal {
                reason: FatalReason::Quarantined { what },
            });
            (told, "fatal: the database is quarantined".to_owned())
        }
        Entry::Open => (
            Told::Open(std::io::ErrorKind::Other, unreadable.clone()),
            unreadable,
        ),
        Entry::Restore => storage(format!("`{path}`: the device refused the operation")),
        Entry::Call | Entry::Commit | Entry::Verify | Entry::Backup | Entry::Dump => {
            storage(unreadable)
        }
    }
}

/// Returns what `entry` reports when it finishes past an unreadable `pid`, or past a read of no
/// page.
fn reported(entry: Entry, pid: Option<Pid>) -> Reported {
    let page = |pid: Pid| format!("\"page {}: Unreadable\"", pid.0);
    match entry {
        Entry::Verify => Reported::Findings(pid.map(page).into_iter().collect()),
        Entry::Backup => Reported::Damaged(pid.into_iter().collect()),
        Entry::Open | Entry::Call | Entry::Commit | Entry::Dump | Entry::Restore => {
            Reported::Nothing
        }
    }
}

/// Returns whether `entry` may finish past an unreadable page of kind `kind`: verify and a backup
/// report it, and the rest go on past a header copy the other copy stands in for, a free space or
/// visibility map page, and an allocation bitmap page, whose pages count as allocated.
fn finishes_past(entry: Entry, kind: Option<PageKind>) -> bool {
    match entry {
        Entry::Verify | Entry::Backup => true,
        Entry::Open | Entry::Call | Entry::Commit | Entry::Dump => matches!(
            kind,
            None | Some(
                PageKind::FreeSpaceMap | PageKind::VisibilityMap | PageKind::AllocationBitmap
            )
        ),
        Entry::Restore => false,
    }
}

#[test]
fn every_entry_refused_a_read_is_told_a_typed_failure_naming_the_page_or_reports_it() {
    let files = seeded();
    let mut refused: BTreeMap<Entry, usize> = BTreeMap::new();
    let mut finished: BTreeMap<Entry, usize> = BTreeMap::new();
    let mut unexpected = Vec::new();
    for entry in Entry::every() {
        let read = reads_of(&files, entry);
        let every: Vec<usize> = (0..read.len()).collect();
        let chosen = if every_case() {
            every
        } else {
            spread(&every, count(READS))
        };
        for (path, at) in chosen.into_iter().map(|i| &read[i]) {
            let pid = Pid(at / PAGE as u64);
            let what = format!("{entry:?} failing the read of {path} at {at}");
            let page = (path == FILE).then(|| kind_of(&files, pid)).flatten();
            match failed_at(&files, entry, path, *at) {
                Ok(got) => {
                    if !finishes_past(entry, page) {
                        unexpected.push(format!("{what}: finished past {page:?}"));
                    }
                    let want = reported(entry, page.map(|_| pid));
                    let rewritten = matches!(&got, Reported::Rewritten(p) if p.contains(&pid));
                    if want != got && !rewritten {
                        unexpected.push(format!("{what}, page {page:?}: {got:?}, not {want:?}"));
                    }
                    *finished.entry(entry).or_default() += 1;
                }
                told => {
                    let (want, reason) = expected(entry, path, *at, page);
                    if told.as_ref().err() != Some(&want) {
                        unexpected.push(format!("{what}: {told:?}"));
                        continue;
                    }
                    refused_as(told, &want, &reason);
                    *refused.entry(entry).or_default() += 1;
                }
            }
        }
    }
    assert_eq!(Vec::<String>::new(), unexpected);
    for entry in Entry::every() {
        let named = if finishes_past(entry, Some(PageKind::HeapSlotted)) {
            finished.get(&entry)
        } else {
            refused.get(&entry)
        };
        let what = format!("{entry:?} told or reporting a failed read by name");
        assert_reached(&what, named.copied().unwrap_or(0), 1);
    }
}
