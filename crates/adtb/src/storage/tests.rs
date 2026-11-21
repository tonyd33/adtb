use std::collections::{BTreeMap, BTreeSet};

use adtb_sys::os::{Backend, Faults, OsRuntime};
use adtb_testkit::scratch::Scratch;

use super::alloc::diff;
use super::block::{
    InMemory, LogFinding, LogImage, LogProblem, Mode, batch_at, frame, resolutions_from, scan,
};
use super::crc::{crc32c, crc32c_skipping};
use super::error::{Damage, OpenError};
use super::header::{FileHeader, choose};
use super::page;
use super::pager::{Options, PageError, Pager};
use super::record::{Edit, Malformed, MapRoot, Record};
use super::{Lsn, Pid, Position};

const BLOCK: usize = 128;

#[test]
fn an_older_checkpoint_keeps_the_group_count_and_a_lower_one_never_lowers_it() {
    let scratch = Scratch::new("storage", "group-count");
    let rt = OsRuntime::with_faults(
        scratch.path(),
        Backend::Pool { threads: 1 },
        Faults::none(4096),
    );
    let options = Options {
        pool_frames: 16,
        max_pages: 1024,
        ..Options::new("count.adt")
    };
    let pager = Pager::create(rt, &options).expect("create pager");
    pager.write_header(Some((Lsn(u64::MAX / 2), 7)));
    pager.write_header(Some((Lsn(1), 9)));
    pager.write_header(Some((Lsn(u64::MAX / 2), 3)));
    let at_header = pager.checkpoint_lsn();
    assert_eq!(Lsn(u64::MAX / 2), at_header);
    assert_eq!(9, pager.read_header(|h| h.bitmap_groups));
    pager.write_header(None);
    pager.close();
    let file = std::fs::read(scratch.path().join("count.adt")).expect("read the file");
    let page = pager.page_size();
    let copies = [
        FileHeader::decode(0, &file[..page]),
        FileHeader::decode(1, &file[page..2 * page]),
    ];
    let [first, second] = copies;
    let (_, newest) = choose(first, second).expect("a copy verifies");
    assert_eq!(9, newest.bitmap_groups);
}

const SEGMENT: u64 = 16 * BLOCK as u64;

#[test]
fn a_map_root_whose_root_has_no_record_in_its_batch_refuses_the_open() {
    let scratch = Scratch::new("storage", "unproven-root");
    let rt = || {
        OsRuntime::with_faults(
            scratch.path(),
            Backend::Pool { threads: 1 },
            Faults::none(4096),
        )
    };
    let options = Options {
        pool_frames: 16,
        max_pages: 1024,
        ..Options::new("root.adt")
    };
    let pager = Pager::create(rt(), &options).expect("create pager");
    pager
        .fold(&BTreeMap::from([(Pid(1), Lsn(1))]))
        .expect("fold");
    let root = pager.map_root().expect("the fold made a map").root;
    pager.flush_log();
    let at = pager.log().append(&Record::MapRoot { root, height: 1 });
    pager.flush_log();
    drop(pager);
    let damage = Damage::LogUnproven { at };
    assert_eq!(
        Some(OpenError::Page(PageError { pid: root, damage })),
        Pager::open(rt(), &options).err()
    );
}

/// Space: a change of the map root outside a fold, then a checkpoint in the same session or in
/// the open after a crash that replays the change.
#[test]
fn a_checkpoint_names_the_map_root_at_its_last_change_in_session_or_replayed() {
    for crash in [false, true] {
        let scratch = Scratch::new("storage", "root-lsn");
        let rt = || {
            OsRuntime::with_faults(
                scratch.path(),
                Backend::Pool { threads: 1 },
                Faults::none(4096),
            )
        };
        let options = Options {
            pool_frames: 16,
            max_pages: 1024,
            ..Options::new("root-lsn.adt")
        };
        let mut pager = Pager::create(rt(), &options).expect("create pager");
        pager
            .fold(&BTreeMap::from([(Pid(1), Lsn(1))]))
            .expect("fold");
        pager.checkpoint();
        let root = pager.map_root().expect("the fold made a map").root;
        let changed = pager
            .update(&[root], |p| p[0][page::HEADER_LEN + 16] = 3)
            .expect("change the root");
        if crash {
            pager.flush_log();
            drop(pager);
            pager = Pager::open(rt(), &options).expect("reopen").0;
        }
        pager.checkpoint();
        let at = pager.checkpoint_lsn();
        let (_pin, mut img) = pager.load_log().expect("load the log");
        let start = img.batch_start(at).expect("the checkpoint's batch");
        let block = img.block;
        let records = batch_at(&mut img, start)
            .expect("the batch")
            .records(block)
            .expect("records");
        let named = records.into_iter().find_map(|(lsn, r)| match r {
            Record::Checkpoint { map, .. } if lsn == at => map,
            _ => None,
        });
        assert_eq!(
            Some((root, changed)),
            named.map(|m| (m.root, m.lsn)),
            "crashed: {crash}"
        );
        drop(img);
        pager.close();
    }
}

#[test]
fn the_parent_of_a_path_keeps_the_root() {
    use super::create::parent;
    assert_eq!("/", parent("/a"));
    assert_eq!("/a", parent("/a/b"));
    assert_eq!("a", parent("a/b"));
    assert_eq!("", parent("a"));
}

#[test]
fn crc32c_check_value() {
    assert_eq!(0xE306_9283, crc32c(b"123456789"));
    assert_eq!(0, crc32c(b""));
}

#[test]
fn a_page_checksum_keeps_its_value_on_disk() {
    let page: Vec<u8> = (0..4096u32).map(|i| (i * 7 + i / 5) as u8).collect();
    assert_eq!(0xD23B_3D67, crc32c_skipping(Some(7), &page, 8));
}

fn header(seq: u64) -> FileHeader {
    FileHeader {
        seq,
        file_id: [7; 16],
        page_size: 8192,
        log_block: 512,
        segment_size: 1 << 20,
        lowest_format: 1,
        highest_format: 2,
        upgrade_state: 0,
        atomic_page_writes: true,
        checkpoint: Lsn(1234),
        catalog_root: Pid(9),
        quarantine: Pid(11),
        epoch: 3,
        encryption: 0,
        bitmap_groups: 3,
        history: vec![(1, Lsn(500)), (2, Lsn(900))],
        log_start: Lsn(900),
        damage_list: Pid(13),
        log_damage: vec![LogFinding {
            lsn: Lsn(4096),
            problem: LogProblem::Stale,
        }],
        log_damage_full: true,
    }
}

#[test]
fn header_copy_round_trips_and_binds_its_copy_number() {
    let bytes = header(4).encode(1);
    assert_eq!(Some(header(4)), FileHeader::decode(1, &bytes));
    assert_eq!(None, FileHeader::decode(0, &bytes));
    let mut torn = bytes;
    torn[70] ^= 0x10;
    assert_eq!(None, FileHeader::decode(1, &torn));
}

#[test]
fn a_header_written_names_the_marks_and_list_pages_as_they_are_at_the_write() {
    let scratch = Scratch::new("storage", "header-owners");
    let rt = OsRuntime::with_faults(
        scratch.path(),
        Backend::Pool { threads: 1 },
        Faults::none(4096),
    );
    let options = Options {
        pool_frames: 16,
        max_pages: 1024,
        ..Options::new("owners.adt")
    };
    let pager = Pager::create(rt, &options).expect("create pager");
    let first = (pager.quarantine_page(), pager.damage_list_page());
    let (new, marks) = pager.reserve_marks().expect("reserve a marks page");
    pager
        .format_marks(new, marks)
        .expect("format the marks page");
    pager.create_damage_list().expect("format a list page");
    let now = (pager.quarantine_page(), pager.damage_list_page());
    assert_ne!(first.0, now.0);
    assert_ne!(first.1, now.1);
    assert_eq!(now, pager.read_header(|h| (h.quarantine, h.damage_list)));
    pager.write_header(None);
    let file = std::fs::read(scratch.path().join("owners.adt")).expect("read the file");
    let page = pager.page_size();
    let copies = [
        FileHeader::decode(0, &file[..page]),
        FileHeader::decode(1, &file[page..]),
    ];
    let [first_copy, second_copy] = copies;
    let (_, newest) = choose(first_copy, second_copy).expect("a copy verifies");
    assert_eq!(now, (newest.quarantine, newest.damage_list));
    pager.close();
}

#[test]
fn choose_takes_the_valid_copy_with_the_higher_sequence() {
    assert_eq!(
        Some((1, header(5))),
        choose(Some(header(4)), Some(header(5)))
    );
    assert_eq!(
        Some((0, header(6))),
        choose(Some(header(6)), Some(header(5)))
    );
    assert_eq!(Some((1, header(3))), choose(None, Some(header(3))));
    assert_eq!(None, choose(None, None));
}

#[test]
fn records_round_trip() {
    let records = [
        Record::Change {
            pid: Pid(3),
            prev: Lsn(40),
            edits: vec![
                Edit {
                    offset: 24,
                    bytes: vec![1, 2, 3],
                },
                Edit {
                    offset: 900,
                    bytes: vec![],
                },
            ],
        },
        Record::Image {
            pid: Pid(4),
            prev: Lsn(0),
            page: vec![5; 4096],
        },
        Record::Commit {
            xid: 8,
            wall_nanos: 99,
        },
        Record::Abort { xid: 9 },
        Record::XidAllocation { next: 1000 },
        Record::Checkpoint {
            redo: Lsn(36),
            dirty: vec![(Pid(2), Lsn(36)), (Pid(7), Lsn(80))],
            map: None,
        },
        Record::Checkpoint {
            redo: Lsn(36),
            dirty: vec![],
            map: Some(MapRoot {
                root: Pid(9),
                lsn: Lsn(70),
                height: 2,
            }),
        },
        Record::MapRoot {
            root: Pid(9),
            height: 3,
        },
        Record::SafeSnapshot {
            at: Position {
                epoch: 3,
                lsn: Lsn(4096),
            },
            snapshot: vec![1, 2, 3, 4, 5],
        },
        Record::SafeSnapshot {
            at: Position {
                epoch: 1,
                lsn: Lsn(36),
            },
            snapshot: Vec::new(),
        },
    ];
    let mut bytes = Vec::new();
    for r in &records {
        r.encode(&mut bytes);
    }
    let mut at = 0;
    for r in &records {
        let (got, n) = Record::decode(&bytes[at..]).unwrap();
        assert_eq!(*r, got);
        at += n;
    }
    assert_eq!(bytes.len(), at);
    assert_eq!(Err(Malformed), Record::decode(&bytes[..3]));
}

#[test]
fn diff_edits_turn_old_into_new_at_every_alignment() {
    let old: Vec<u8> = (0..100u32).map(|i| (i * 13) as u8).collect();
    for a in 0..old.len() {
        for b in [a, a + 1, a + 7, a + 8, a + 17, a + 40] {
            let mut new = old.clone();
            new[a] ^= 0xff;
            if b < new.len() {
                new[b] ^= 0x0f;
            }
            let mut rebuilt = old.clone();
            for e in diff(&old, &new) {
                let at = e.offset as usize;
                assert_ne!(old[at], new[at], "edit at {at} starts on an unchanged byte");
                rebuilt[at..at + e.bytes.len()].copy_from_slice(&e.bytes);
            }
            assert_eq!(new, rebuilt, "changes at {a} and {b}");
        }
    }
}

fn records(n: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for xid in 0..n as u64 {
        Record::Commit { xid, wall_nanos: 0 }.encode(&mut out);
    }
    out
}

/// Returns a one-segment log holding batches of `sizes` commit records each, numbered from 1,
/// and the first LSN of each batch followed by the end.
fn image(sizes: &[usize]) -> (LogImage<InMemory>, Vec<Lsn>) {
    let mut seg = vec![0; SEGMENT as usize];
    let mut at = 0;
    let mut starts = Vec::new();
    for (i, &n) in sizes.iter().enumerate() {
        let b = frame(
            Lsn(at as u64),
            i as u64 + 1,
            1,
            &[0; 16],
            &records(n),
            BLOCK,
        );
        seg[at..at + b.len()].copy_from_slice(&b);
        starts.push(Lsn(at as u64));
        at += b.len();
    }
    starts.push(Lsn(at as u64));
    let img = LogImage::in_memory(BLOCK, SEGMENT, BTreeMap::from([(0, Some(seg))]));
    (img, starts)
}

fn seg(img: &mut LogImage<InMemory>) -> &mut Vec<u8> {
    img.source.0.get_mut(&0).unwrap().as_mut().unwrap()
}

/// What a scan of a log found: the start and record count of each batch it accepted, the index
/// of each batch it accepted after a gap, the end, the finding that stops recovery and every
/// finding.
#[derive(Debug, PartialEq, Eq)]
struct Scanned {
    batches: Vec<(Lsn, usize)>,
    resumed: Vec<usize>,
    end: Lsn,
    fatal: Option<LogFinding>,
    findings: Vec<LogFinding>,
}

/// Returns what a scan of `img` from its start in `mode` finds.
fn scanned(img: &mut LogImage<InMemory>, mode: Mode) -> Scanned {
    let mut batches = Vec::new();
    let mut resumed = Vec::new();
    let s = scan(img, Lsn(0), mode, |b, after_gap| {
        if after_gap {
            resumed.push(batches.len());
        }
        batches.push((b.start, b.records(BLOCK).unwrap().len()));
    });
    assert_eq!(s.batches, batches.len());
    assert_eq!(s.resumed, resumed.len());
    Scanned {
        batches,
        resumed,
        end: s.end,
        fatal: s.fatal,
        findings: s.findings,
    }
}

/// Returns a scan of a log that ends cleanly at `end` after `batches`, with `findings`.
fn ended(batches: Vec<(Lsn, usize)>, end: Lsn, findings: Vec<LogFinding>) -> Scanned {
    Scanned {
        batches,
        resumed: Vec::new(),
        end,
        fatal: None,
        findings,
    }
}

fn found(lsn: Lsn, problem: LogProblem) -> LogFinding {
    LogFinding { lsn, problem }
}

/// Space: each kind of block past the last accepted batch, a gap, a skipped sequence number and
/// an unreadable segment, each scanned for recovery, and for verify but the last three.
#[test]
fn a_scan_accepts_whole_batches_in_sequence_and_stops_or_reports_at_the_rest() {
    use LogProblem::{Damaged, Gap, Stale, Unreadable};
    let mut got = Vec::new();
    let mut want = Vec::new();
    let mut row =
        |name: &str, mut img: LogImage<InMemory>, recovery: Scanned, verify: Option<Scanned>| {
            let verify = verify.map(|v| (Mode::Verify, v));
            for (mode, scan) in [(Mode::Recovery, recovery)].into_iter().chain(verify) {
                got.push((name.to_owned(), mode, scanned(&mut img, mode)));
                want.push((name.to_owned(), mode, scan));
            }
        };

    let (img, s) = image(&[3, 9, 1]);
    let whole = || ended(vec![(s[0], 3), (s[1], 9), (s[2], 1)], s[3], Vec::new());
    row("zero past the end", img, whole(), Some(whole()));

    let (mut img, s) = image(&[3, 9]);
    let second = s[1].0 as usize + BLOCK;
    seg(&mut img)[second..second + BLOCK].fill(0);
    let first = || ended(vec![(s[0], 3)], s[1], Vec::new());
    row("a batch missing a block", img, first(), Some(first()));

    let (mut img, s) = image(&[3, 9]);
    seg(&mut img)[second + 50] ^= 4;
    let damaged = vec![found(Lsn(s[1].0 + BLOCK as u64), Damaged)];
    row(
        "a torn block",
        img,
        first(),
        Some(ended(vec![(s[0], 3)], s[1], damaged)),
    );

    let (mut img, s) = image(&[3]);
    let elsewhere = frame(
        Lsn(s[1].0 + BLOCK as u64),
        2,
        1,
        &[0; 16],
        &records(1),
        BLOCK,
    );
    let at = s[1].0 as usize;
    seg(&mut img)[at..at + BLOCK].copy_from_slice(&elsewhere);
    let damaged = vec![found(s[1], Damaged)];
    row(
        "a block valid elsewhere",
        img,
        first(),
        Some(ended(vec![(s[0], 3)], s[1], damaged)),
    );

    let (mut img, s) = image(&[3, 9, 2]);
    seg(&mut img)[s[1].0 as usize + 40] ^= 1;
    let gap = found(s[1], Gap);
    let stopped = Scanned {
        fatal: Some(gap),
        ..ended(vec![(s[0], 3)], s[1], vec![gap])
    };
    row("a batch after a bad one", img, stopped, None);

    let (mut img, s) = image(&[2]);
    let at = s[1].0 as usize;
    let discarded = frame(s[1], 2, 1, &[0; 16], &records(9), BLOCK);
    let rewritten = frame(s[1], 2, 1, &[0; 16], &records(1), BLOCK);
    let newer = frame(
        Lsn(s[1].0 + BLOCK as u64),
        3,
        1,
        &[0; 16],
        &records(1),
        BLOCK,
    );
    seg(&mut img)[at..at + discarded.len()].copy_from_slice(&discarded);
    seg(&mut img)[at..at + rewritten.len()].copy_from_slice(&rewritten);
    seg(&mut img)[at + BLOCK..at + BLOCK + newer.len()].copy_from_slice(&newer);
    let end = Lsn(s[1].0 + 2 * BLOCK as u64);
    let stale: Vec<LogFinding> = (2..discarded.len() / BLOCK)
        .map(|i| found(Lsn(s[1].0 + (i * BLOCK) as u64), Stale))
        .collect();
    let accepted = vec![(s[0], 2), (s[1], 1), (Lsn(s[1].0 + BLOCK as u64), 1)];
    let rewrite = || ended(accepted.clone(), end, stale.clone());
    row("blocks of an older batch", img, rewrite(), Some(rewrite()));

    let (mut img, s) = image(&[2, 2]);
    let skipped = frame(s[1], 7, 1, &[0; 16], &records(2), BLOCK);
    let at = s[1].0 as usize;
    seg(&mut img)[at..at + skipped.len()].copy_from_slice(&skipped);
    let gap = found(s[1], Gap);
    let stopped = Scanned {
        fatal: Some(gap),
        ..ended(vec![(s[0], 2)], s[1], vec![gap])
    };
    row("a sequence number skipped", img, stopped, None);

    let (mut img, s) = image(&[2]);
    img.source.0.insert(SEGMENT, None);
    img.bases.push(SEGMENT);
    let unreadable = found(Lsn(SEGMENT), Unreadable);
    let stopped = Scanned {
        fatal: Some(unreadable),
        ..ended(vec![(s[0], 2)], s[1], vec![unreadable])
    };
    row("an unreadable segment", img, stopped, None);

    assert_eq!(13, got.len());
    assert_eq!(want, got);
}

#[test]
fn a_first_segment_opening_on_a_batch_begun_before_it_starts_at_the_next_batch() {
    let block = 512;
    let segment = 2 * block as u64;
    let long = frame(Lsn(0), 1, 1, &[0; 16], &vec![7; block * 2], block);
    let blocks = long.len() / block;
    assert_eq!(3, blocks);
    let mut held = vec![0; segment as usize];
    held[..block].copy_from_slice(&long[2 * block..]);
    let mut img = LogImage::in_memory(block, segment, BTreeMap::from([(segment, Some(held))]));
    assert_eq!(Some(Lsn(segment + block as u64)), img.first_batch());
    let whole = frame(Lsn(segment), 1, 1, &[0; 16], &[7; 8], block);
    let mut held = vec![0; segment as usize];
    held[..block].copy_from_slice(&whole);
    let mut img = LogImage::in_memory(block, segment, BTreeMap::from([(segment, Some(held))]));
    assert_eq!(Some(Lsn(segment)), img.first_batch());
}

/// The segments of a log by their first LSN, `None` for one the device cannot read.
type Segments = BTreeMap<u64, Option<Vec<u8>>>;

/// Returns three segments of one-block batches, the batch at block `n` holding the commit of
/// transaction `n`.
fn committed_segments() -> Segments {
    let per = SEGMENT / BLOCK as u64;
    let segment = |s: u64| {
        let mut seg = vec![0; SEGMENT as usize];
        for (i, b) in seg.chunks_mut(BLOCK).enumerate() {
            let n = s * per + i as u64;
            let mut data = Vec::new();
            Record::Commit {
                xid: n,
                wall_nanos: 0,
            }
            .encode(&mut data);
            b.copy_from_slice(&frame(
                Lsn(n * BLOCK as u64),
                n + 1,
                1,
                &[0; 16],
                &data,
                BLOCK,
            ));
        }
        (s * SEGMENT, Some(seg))
    };
    (0..3).map(segment).collect()
}

fn block_at(n: u64) -> Lsn {
    Lsn(n * BLOCK as u64)
}

/// Space: a whole log, logs lacking their first segment, a middle segment unreadable or missing,
/// the last missing, an undecodable batch and a batch straddling the first segment, each read
/// from a start before or inside the damage, up to the end of the log or before the damage.
#[test]
fn commits_from_a_log_are_those_from_the_batch_holding_the_start_or_fail_at_its_damage() {
    let whole = committed_segments;
    let without = |base: u64| {
        let mut held = committed_segments();
        held.remove(&base);
        held
    };
    let mut unreadable = committed_segments();
    unreadable.insert(SEGMENT, None);
    let mut undecoded = committed_segments();
    let at = (block_at(20).0 - SEGMENT) as usize;
    let bad = frame(block_at(20), 21, 1, &[0; 16], &[0xff; 8], BLOCK);
    undecoded.get_mut(&SEGMENT).unwrap().as_mut().unwrap()[at..at + BLOCK].copy_from_slice(&bad);
    let mut straddled = without(0);
    let straddling = frame(block_at(15), 16, 1, &[0; 16], &records(6), BLOCK);
    assert_eq!(2 * BLOCK, straddling.len());
    straddled.get_mut(&SEGMENT).unwrap().as_mut().unwrap()[..BLOCK]
        .copy_from_slice(&straddling[BLOCK..]);
    let rows = [
        (
            "whole",
            whole(),
            Lsn(block_at(20).0 + 5),
            48,
            Ok((20..48).collect()),
        ),
        (
            "first missing",
            without(0),
            block_at(3),
            48,
            Ok((16..48).collect()),
        ),
        (
            "middle unreadable",
            unreadable,
            block_at(2),
            48,
            Err(block_at(16)),
        ),
        (
            "middle missing",
            without(SEGMENT),
            block_at(2),
            48,
            Err(block_at(16)),
        ),
        (
            "middle missing, from it",
            without(SEGMENT),
            block_at(18),
            48,
            Err(block_at(18)),
        ),
        (
            "last missing",
            without(2 * SEGMENT),
            block_at(2),
            48,
            Err(block_at(32)),
        ),
        (
            "last missing, to it",
            without(2 * SEGMENT),
            block_at(2),
            32,
            Ok((2..32).collect()),
        ),
        ("undecodable", undecoded, block_at(2), 48, Err(block_at(20))),
        (
            "straddling",
            straddled,
            Lsn(block_at(16).0 + 50),
            48,
            Ok((17..48).collect()),
        ),
    ];
    for (name, segments, since, to, want) in rows {
        let mut img = LogImage::in_memory(BLOCK, SEGMENT, segments);
        let got: Result<Vec<u64>, Lsn> =
            resolutions_from(&mut img, since, block_at(to), &BTreeSet::new())
                .map(|r| r.commits.into_iter().collect());
        assert_eq!(want, got, "{name}");
    }
}

#[test]
fn a_chain_break_after_a_replayed_change_refuses_the_page() {
    let scratch = Scratch::new("storage", "chain-break");
    let rt = || {
        OsRuntime::with_faults(
            scratch.path(),
            Backend::Pool { threads: 1 },
            Faults::none(4096),
        )
    };
    let options = Options {
        pool_frames: 16,
        max_pages: 1024,
        ..Options::new("chain.adt")
    };
    let pager = Pager::create(rt(), &options).expect("create pager");
    let p = pager
        .allocate(1, super::PageKind::HeapSlotted, 7)
        .expect("allocate");
    let updated = pager
        .update(&[p], |x| x[0][page::HEADER_LEN] = 1)
        .expect("update");
    pager.flush_log();
    pager.log().append(&Record::Change {
        pid: p,
        prev: Lsn(1),
        edits: vec![],
    });
    pager.flush_log();
    drop(pager);
    let (pager, opening) = Pager::open(rt(), &options).expect("open");
    let recovery = opening.recovery().cloned().expect("the open recovered");
    let damage = Damage::Stale {
        expected: updated,
        found: Lsn(1),
    };
    assert_eq!(vec![(p, damage)], recovery.damaged);
    pager.close();
}

#[test]
fn verify_reports_a_chain_break_of_a_page_freed_since() {
    let scratch = Scratch::new("storage", "freed-break");
    let rt = OsRuntime::with_faults(
        scratch.path(),
        Backend::Pool { threads: 1 },
        Faults::none(4096),
    );
    let options = Options {
        pool_frames: 16,
        max_pages: 1024,
        ..Options::new("freed.adt")
    };
    let pager = Pager::create(rt, &options).expect("create pager");
    let p = pager
        .allocate(1, super::PageKind::HeapSlotted, 7)
        .expect("allocate");
    let updated = pager
        .update(&[p], |x| x[0][page::HEADER_LEN] = 1)
        .expect("update");
    pager.log().append(&Record::Change {
        pid: p,
        prev: Lsn(1),
        edits: vec![],
    });
    pager.free(p).expect("free");
    pager.flush_log();
    let broken = super::Finding::Page {
        pid: p,
        damage: Damage::Stale {
            expected: updated,
            found: Lsn(1),
        },
    };
    let of_p = |f: &super::Finding| matches!(f, super::Finding::Page { pid, .. } if *pid == p);
    let found: Vec<super::Finding> = pager.verify().findings.into_iter().filter(of_p).collect();
    assert_eq!(vec![broken], found);
    pager.close();
}

fn order_pager(name: &str) -> (Scratch, Pager<OsRuntime>) {
    let scratch = Scratch::new("storage", name);
    let rt = OsRuntime::with_faults(
        scratch.path(),
        Backend::Pool { threads: 1 },
        Faults::none(4096),
    );
    let options = Options {
        pool_frames: 16,
        max_pages: 1024,
        ..Options::new("order.adt")
    };
    let pager = Pager::create(rt, &options).expect("create pager");
    (scratch, pager)
}

#[test]
#[should_panic(expected = "the log lock is never taken under the meta lock")]
fn taking_the_log_under_the_meta_lock_panics() {
    let (_scratch, pager) = order_pager("meta-then-log");
    let _meta = pager.dirty_table();
    drop(pager.log());
}

#[test]
#[should_panic(expected = "the meta lock is taken under the log lock only by an append")]
fn taking_the_meta_lock_under_the_log_outside_an_append_panics() {
    let (_scratch, pager) = order_pager("log-then-meta");
    let _log = pager.log();
    drop(pager.dirty_table());
}

#[test]
#[should_panic(expected = "a thread holding the log lock does not take it again")]
fn an_append_under_a_held_log_guard_panics() {
    let (_scratch, pager) = order_pager("append-under-log");
    let _log = pager.log();
    drop(pager.append());
}

/// The kinds of block a scan reports past a blank run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Planted {
    /// A block of the batch with sequence 1, a byte flipped.
    Torn,
    /// A whole block of the batch with sequence 1.
    Stale,
    /// A whole block of the next batch, written for another file.
    Foreign,
}

/// Returns a segment of `len` bytes holding batches of one commit record, sequence 1 to
/// `batches`, one block each from the start, then zeros, and `planted` as the block at `at`.
fn tailed(len: usize, batches: u64, planted: Planted, at: u64) -> Vec<u8> {
    let block = TAIL_BLOCK as u64;
    let mut seg = vec![0; len];
    for i in 0..batches {
        let b = frame(Lsn(i * block), i + 1, 1, &[0; 16], &records(1), TAIL_BLOCK);
        seg[(i * block) as usize..][..TAIL_BLOCK].copy_from_slice(&b);
    }
    let lsn = Lsn(at * block);
    let mut b = match planted {
        Planted::Torn | Planted::Stale => frame(lsn, 1, 1, &[0; 16], &records(1), TAIL_BLOCK),
        Planted::Foreign => frame(lsn, batches + 1, 1, &[7; 16], &records(1), TAIL_BLOCK),
    };
    if let Planted::Torn = planted {
        b[TAIL_BLOCK - 1] ^= 1;
    }
    seg[(at * block) as usize..][..TAIL_BLOCK].copy_from_slice(&b);
    seg
}

/// Returns what a scan in `mode` of a log of `batches` one-block batches followed by blank blocks
/// up to `planted` at block `at` finds, from the log format: a torn block is damage only to
/// verify, a stale or foreign block is reported to both, and a foreign one stops recovery.
fn tail_expected(mode: Mode, batches: u64, planted: Planted, at: u64) -> Scanned {
    let block = TAIL_BLOCK as u64;
    let lsn = Lsn(at * block);
    let finding = match (planted, mode) {
        (Planted::Torn, Mode::Recovery) => None,
        (Planted::Torn, Mode::Verify) => Some(found(lsn, LogProblem::Damaged)),
        (Planted::Stale, _) => Some(found(lsn, LogProblem::Stale)),
        (Planted::Foreign, _) => Some(found(lsn, LogProblem::Foreign)),
    };
    let fatal = finding.filter(|f| mode == Mode::Recovery && f.problem == LogProblem::Foreign);
    Scanned {
        batches: (0..batches).map(|i| (Lsn(i * block), 1)).collect(),
        resumed: Vec::new(),
        end: Lsn(batches * block),
        fatal,
        findings: finding.into_iter().collect(),
    }
}

/// Returns what a scan of `img` from its start in `mode` finds.
fn tail_scanned<S: super::block::LogSource>(img: &mut LogImage<S>, mode: Mode) -> Scanned {
    let mut batches = Vec::new();
    let s = scan(img, Lsn(0), mode, |b, _| {
        batches.push((b.start, b.records(TAIL_BLOCK).unwrap().len()));
    });
    Scanned {
        batches,
        resumed: Vec::new(),
        end: s.end,
        fatal: s.fatal,
        findings: s.findings,
    }
}

const TAIL_BLOCK: usize = 512;

/// Space: each kind of block a scan reports, after a blank run of every length from one block to
/// past a chunk's end, at a chunk's first, middle and last block and at the segment's last, read
/// from memory and from segment files, scanned for recovery and verify; and the tail clear of
/// each file, which leaves the log's batches and zeros every byte after them.
#[test]
fn a_block_after_a_blank_run_of_any_length_is_found_and_cleared() {
    use super::driver::Driver;
    use super::log::{CHUNK, FileSource, Segments};
    use crate::runtime::AlignedBuf;

    let block = TAIL_BLOCK as u64;
    let chunk = (CHUNK / TAIL_BLOCK) as u64;
    let segment = 3 * CHUNK as u64;
    let last = segment / block - 1;
    let ats = [chunk, chunk + chunk / 2, 2 * chunk - 1, last];
    let kinds = [Planted::Torn, Planted::Stale, Planted::Foreign];
    let mut cases = Vec::new();
    for at in ats {
        for run in 1..=(chunk + 2).min(at - 1) {
            for planted in kinds {
                cases.push((planted, at, at - run));
            }
        }
    }
    let crossing = cases
        .iter()
        .filter(|&&(_, at, batches)| batches / chunk != at / chunk)
        .count();
    assert!(crossing > 0, "a run crosses a chunk's end");

    let mut got = Vec::new();
    let mut want = Vec::new();
    for &(planted, at, batches) in &cases {
        let seg = tailed(segment as usize, batches, planted, at);
        let mut img = LogImage::in_memory(TAIL_BLOCK, segment, BTreeMap::from([(0, Some(seg))]));
        for mode in [Mode::Recovery, Mode::Verify] {
            got.push((planted, at, batches, mode, tail_scanned(&mut img, mode)));
            want.push((
                planted,
                at,
                batches,
                mode,
                tail_expected(mode, batches, planted, at),
            ));
        }
    }

    let mut sim = adtb_sim::Sim::new(1, adtb_sim::Config::default());
    let mut cleared = 0;
    let mut left = Vec::new();
    crate::test_world::finished(&mut sim, |rt| {
        let mut drv = Driver::new(rt.clone());
        drv.create_dir("log");
        let (file, _) = drv
            .open("log/0000000000000000", true)
            .unwrap_or_else(|f| f.fatal("open the segment"));
        for &(planted, at, batches) in &cases {
            let seg = tailed(segment as usize, batches, planted, at);
            drv.write_all(vec![(file, 0, AlignedBuf::from_slice(&seg))]);
            let image = || {
                let files = BTreeMap::from([(0, file)]);
                FileSource::image(rt.clone(), TAIL_BLOCK, segment, [0; 16], files)
            };
            for mode in [Mode::Recovery, Mode::Verify] {
                got.push((planted, at, batches, mode, tail_scanned(&mut image(), mode)));
                want.push((
                    planted,
                    at,
                    batches,
                    mode,
                    tail_expected(mode, batches, planted, at),
                ));
            }
            let mut segments = Segments::new("log".into(), TAIL_BLOCK, segment, [0; 16]);
            segments.clear_tail(&mut drv, &mut image(), Lsn(batches * block));
            segments.close(&mut drv);
            let after = drv
                .read(file, 0, segment as usize)
                .unwrap_or_else(|f| f.fatal("read the segment"));
            let mut kept = seg;
            kept[(batches * block) as usize..].fill(0);
            cleared += 1;
            if *after != *kept {
                left.push((planted, at, batches));
            }
        }
        drv.close(file);
    });

    assert_eq!(cases.len(), 3 * (chunk - 1 + 3 * (chunk + 2)) as usize);
    assert_eq!(4 * cases.len(), got.len());
    assert_eq!(want, got);
    assert_eq!(cases.len(), cleared);
    assert_eq!(
        Vec::<(Planted, u64, u64)>::new(),
        left,
        "clears that left a byte"
    );
}
