//! In-shape damage of the commit log: a page whose checksum holds over contents no write of the
//! engine gives it, which an open, or a write after it, must refuse and mark, never panic on.

use std::collections::{BTreeMap, BTreeSet};
use std::panic::{AssertUnwindSafe, catch_unwind};

use adtb::CallOptions;
use adtb::testing::{CommitLogPages, Damage, Pid, Quarantine, Shape, page};
use adtb_sim::{Config, Sim, SimRuntime};
use adtb_testkit::process;
use adtb_testkit::search::Files;

use super::world::{self, DATA, Op, Rel};

/// The sequence each case reshapes the file of: commits and aborts enough to fill the start of a
/// status page, closed cleanly.
fn ops() -> Vec<Op> {
    vec![
        Op::Insert(Rel::Users, vec![1, 2, 3]),
        Op::Update(Rel::Users, vec![1]),
        Op::Insert(Rel::Docs, vec![1]),
        Op::Delete(Rel::Users, vec![2]),
        Op::Checkpoint,
        Op::Update(Rel::Docs, vec![1]),
    ]
}

const SEED: u64 = 17;

const STATUS_NUMBER: usize = page::HEADER_LEN;
const STATUSES: usize = page::HEADER_LEN + 8;
const DIRECTORY_NEXT: usize = page::HEADER_LEN;
const DIRECTORY_COUNT: usize = page::HEADER_LEN + 8;
const DIRECTORY_ENTRIES: usize = page::HEADER_LEN + 16;
const ENTRY: usize = 16;
const ROOT_RESERVED: usize = page::HEADER_LEN;
const ROOT_BOUND: usize = page::HEADER_LEN + 16;

/// How a case reshapes a commit log page, keeping its checksum sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reshape {
    /// The first status byte of status page 0 set to bits naming no status for each of its ids.
    StatusBits,
    /// Status page 0 holding the number of status page 1.
    StatusNumber,
    /// The last directory's count raised past what its page holds.
    CountPastCapacity,
    /// The last directory's count lowered by one, losing its last entry.
    LostEntry,
    /// Status page 0 labelled as the first directory is, a commit log page of another role.
    StatusKind,
    /// The last directory naming the first as the next, a chain returning to a page.
    LoopedChain,
    /// The last directory's last entry named again after it, an entry not past the one before.
    EntryAgain,
    /// The last directory's last entry numbering a status page far past the reservation.
    EntryOutOfRange,
    /// The root's truncation bound past its reservation.
    RootBounds,
    /// The bits of the first id past the reservation recording it committed, met by the commit of
    /// a write after the open.
    EndedAhead,
}

const RESHAPES: [Reshape; 10] = [
    Reshape::StatusBits,
    Reshape::StatusNumber,
    Reshape::CountPastCapacity,
    Reshape::LostEntry,
    Reshape::StatusKind,
    Reshape::LoopedChain,
    Reshape::EntryAgain,
    Reshape::EntryOutOfRange,
    Reshape::RootBounds,
    Reshape::EndedAhead,
];

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    let mut word = [0; 8];
    word.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(word)
}

/// Returns `files` with the page `reshape` edits of `pages` reshaped and sealed, and the page
/// an open or write must refuse.
fn reshaped(files: &Files, pages: &CommitLogPages, reshape: Reshape) -> (Files, Pid) {
    let last = *pages.directory().last().expect("a last directory");
    let first = pages.directory()[0];
    let (pid, refused) = match reshape {
        Reshape::StatusBits | Reshape::StatusNumber | Reshape::EndedAhead => {
            (pages.status()[&0], pages.status()[&0])
        }
        Reshape::StatusKind => (pages.status()[&0], first),
        Reshape::CountPastCapacity
        | Reshape::LostEntry
        | Reshape::LoopedChain
        | Reshape::EntryAgain
        | Reshape::EntryOutOfRange => (last, last),
        Reshape::RootBounds => (pages.root(), pages.root()),
    };
    let mut files = files.clone();
    let directory = DATA.page_mut(&mut files, first).to_vec();
    let reserved = u64_at(DATA.page_mut(&mut files, pages.root()), ROOT_RESERVED);
    let bytes = DATA.page_mut(&mut files, pid);
    let count = u64_at(bytes, DIRECTORY_COUNT);
    let entry = |i: u64| DIRECTORY_ENTRIES + i as usize * ENTRY;
    match reshape {
        Reshape::StatusBits => bytes[STATUSES] = 0xff,
        Reshape::StatusNumber => {
            bytes[STATUS_NUMBER..STATUS_NUMBER + 8].copy_from_slice(&1_u64.to_le_bytes());
        }
        Reshape::CountPastCapacity => {
            let past = (world::PAGE as u64).to_le_bytes();
            bytes[DIRECTORY_COUNT..DIRECTORY_COUNT + 8].copy_from_slice(&past);
        }
        Reshape::LostEntry => {
            assert!(count > 0, "the last directory names a status page");
            let lowered = (count - 1).to_le_bytes();
            bytes[DIRECTORY_COUNT..DIRECTORY_COUNT + 8].copy_from_slice(&lowered);
        }
        Reshape::StatusKind => {
            let role = page::FIRST_EDITABLE..page::HEADER_LEN;
            bytes[role.clone()].copy_from_slice(&directory[role]);
        }
        Reshape::LoopedChain => {
            bytes[DIRECTORY_NEXT..DIRECTORY_NEXT + 8].copy_from_slice(&first.0.to_le_bytes());
        }
        Reshape::EntryAgain => {
            let again = bytes[entry(count - 1)..entry(count)].to_vec();
            bytes[entry(count)..entry(count + 1)].copy_from_slice(&again);
            bytes[DIRECTORY_COUNT..DIRECTORY_COUNT + 8].copy_from_slice(&(count + 1).to_le_bytes());
        }
        Reshape::EntryOutOfRange => {
            let at = entry(count - 1);
            bytes[at..at + 8].copy_from_slice(&1_000_000_u64.to_le_bytes());
        }
        Reshape::RootBounds => {
            bytes[ROOT_BOUND..ROOT_BOUND + 8].copy_from_slice(&(reserved + 1).to_le_bytes());
        }
        Reshape::EndedAhead => {
            let at = STATUSES + reserved as usize / 4;
            bytes[at] |= 0b01 << (reserved % 4 * 2);
        }
    }
    page::seal(pid, bytes);
    (files, refused)
}

/// What an open of a reshaped file showed.
#[derive(Debug, PartialEq, Eq)]
struct Opened {
    /// The open's failure, by its text, or `None` when it opened.
    failed: Option<String>,
    /// The reply to a write after the open, by its text, `None` when none ran.
    wrote: Option<String>,
    /// The damage the pager recorded of each page.
    recorded: BTreeMap<Pid, Damage>,
    marks: BTreeSet<Quarantine>,
    panic: Option<String>,
}

/// Returns the commit log pages of `files`, as an open of them reads them.
fn commit_log(files: Files) -> CommitLogPages {
    let mut sim = Sim::with_files(SEED, Config::default(), files);
    let mut pages = None;
    process::run(&mut sim, |rt| {
        let (open, _) = world::open_own(&rt, 0, &world::options(), false);
        let db = open.expect("the clean file opens");
        pages = Some(db.testing().commit_log());
        db.close().expect("the clean file closes");
    });
    pages.expect("the open ran")
}

/// Returns what an open of `files`, and a write after it when `write`, shows, then what a second
/// open of the file it left shows.
fn seen_twice(files: Files, write: bool) -> (Opened, Opened) {
    let mut sim = Sim::with_files(SEED, Config::default(), files);
    let once = |sim: &mut Sim, write: bool| {
        let mut seen = Opened {
            failed: None,
            wrote: None,
            recorded: BTreeMap::new(),
            marks: BTreeSet::new(),
            panic: None,
        };
        let ran = catch_unwind(AssertUnwindSafe(|| {
            process::run(sim, |rt| {
                let (open, _) = world::open_own(&rt, 0, &world::options(), false);
                match open {
                    Ok(db) => {
                        if write {
                            let text = r#"insert users { id: users.key(100), name: "ann" }"#;
                            let got = db.query_text(text, &[], &CallOptions::default());
                            seen.wrote = Some(format!("{:?}", got.map(drop)));
                        }
                        let probe = db.testing().probe();
                        seen.recorded = probe.records().pages().clone();
                        seen.marks = db.testing().probe().marks();
                        drop(db.close());
                    }
                    Err(e) => seen.failed = Some(format!("{e:?}")),
                }
            });
        }));
        if let Err(payload) = ran {
            seen.panic = Some(
                (payload.downcast_ref::<String>().cloned())
                    .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                    .unwrap_or_else(|| "a panic".to_owned()),
            );
        }
        seen
    };
    let first = once(&mut sim, write);
    let second = once(&mut sim, false);
    (first, second)
}

#[test]
fn a_commit_log_page_out_of_shape_under_a_sound_checksum_is_refused_and_marked() {
    let count = |_: &SimRuntime| {};
    let ran = world::run(SEED, &ops(), &count, false, false, false);
    assert_eq!(BTreeMap::new(), ran.failed, "the sequence runs clean");
    let pages = commit_log(ran.files.clone());
    let mut got = Vec::new();
    let mut want = Vec::new();
    for reshape in RESHAPES {
        let (files, pid) = reshaped(&ran.files, &pages, reshape);
        let write = reshape == Reshape::EndedAhead;
        let (first, second) = seen_twice(files, write);
        got.push((reshape, first, second));
        let refused = |wrote: Option<String>| Opened {
            failed: None,
            wrote,
            recorded: BTreeMap::from([(pid, Damage::Shape(Shape::Frame))]),
            marks: BTreeSet::from([Quarantine::Database]),
            panic: None,
        };
        let quarantined = r#"Err(Fatal { reason: Quarantined { what: "the database" } })"#;
        let wrote = write.then(|| quarantined.to_owned());
        want.push((reshape, refused(wrote), refused(None)));
    }
    assert_eq!(want, got);
}
