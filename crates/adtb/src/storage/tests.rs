use std::collections::BTreeMap;

use adtb_sys::os::{Backend, Faults, OsRuntime};
use adtb_testkit::scratch::Scratch;

use super::alloc::diff;
use super::block::{
    Batch, BlockState, InMemory, LogImage, LogProblem, Mode, Scan, commits_from, frame, read_block,
    scan,
};
use super::crc::{Crc32c, crc32c, crc32c_skipping};
use super::header::{FileHeader, choose};
use super::page::{self, Verdict};
use super::pager::{Damage, Options, Pager};
use super::record::{Edit, Record};
use super::{Lsn, Pid, Position};

const BLOCK: usize = 128;

#[test]
fn an_older_checkpoint_keeps_the_group_count_and_a_lower_one_never_lowers_it() {
    let scratch = Scratch::new("storage", "group-count");
    let rt = OsRuntime::with_faults(
        scratch.path(),
        Backend::Pool { threads: 1 },
        Faults::default(),
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
    let at_header = pager.header.lock(&pager.rt).0.checkpoint;
    assert_eq!(Lsn(u64::MAX / 2), at_header);
    assert_eq!(9, pager.header.lock(&pager.rt).0.bitmap_groups);
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
fn the_parent_of_a_path_keeps_the_root() {
    use super::pager::parent;
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

#[test]
fn crc32c_is_the_same_however_the_bytes_are_split() {
    let bytes: Vec<u8> = (0..300u32).map(|i| (i * 7 + i / 5) as u8).collect();
    let whole = crc32c(&bytes);
    for cut in 0..bytes.len() {
        let (a, b) = bytes.split_at(cut);
        assert_eq!(
            whole,
            Crc32c::new().update(a).update(b).finish(),
            "cut at {cut}"
        );
    }
    let bytewise = bytes
        .iter()
        .fold(Crc32c::new(), |c, b| c.update(std::slice::from_ref(b)));
    assert_eq!(whole, bytewise.finish());
}

#[test]
fn sealed_page_verifies_only_at_its_own_id() {
    let mut p = vec![0; 4096];
    page::set_lsn(&mut p, Lsn(77));
    p[100] = 9;
    page::seal(Pid(5), &mut p);
    assert_eq!(Verdict::Valid, page::verify(Pid(5), &p));
    assert_eq!(Verdict::Corrupt, page::verify(Pid(6), &p));
    p[4000] ^= 1;
    assert_eq!(Verdict::Corrupt, page::verify(Pid(5), &p));
    assert_eq!(Verdict::Blank, page::verify(Pid(5), &[0; 4096]));
}

#[test]
fn header_edit_fills_the_fields_layouts_own() {
    let mut p = vec![0; 4096];
    let e = page::header_edit(3, page::PageKind::IndexLeaf, 0xdead, 2);
    p[page::FIRST_EDITABLE..page::HEADER_LEN].copy_from_slice(&e);
    assert_eq!(3, page::format(&p));
    assert_eq!(page::PageKind::IndexLeaf as u16, page::kind(&p));
    assert_eq!(0xdead, page::owner(&p));
    assert_eq!(2, page::flags(&p));
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
    assert!(Record::decode(&bytes[..3]).is_err());
}

#[test]
fn diff_logs_only_changed_bytes() {
    let old = vec![0; 200];
    let mut new = old.clone();
    new[30] = 1;
    new[35] = 2;
    new[150] = 3;
    let edits = diff(&old, &new);
    assert_eq!(2, edits.len());
    assert_eq!((30, 6), (edits[0].offset, edits[0].bytes.len()));
    assert_eq!((150, 1), (edits[1].offset, edits[1].bytes.len()));
    assert!(diff(&old, &old).is_empty());
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

/// Scan `img` and return the scan with every batch it accepted and the index of each batch that
/// follows a gap.
fn scanned(img: &mut LogImage<InMemory>, mode: Mode) -> (Scan, Vec<Batch>, Vec<usize>) {
    let mut batches = Vec::new();
    let mut resumed = Vec::new();
    let s = scan(img, Lsn(0), mode, |b, after_gap| {
        if after_gap {
            resumed.push(batches.len());
        }
        batches.push(b.clone());
    });
    assert_eq!(s.batches, batches.len());
    (s, batches, resumed)
}

#[test]
fn frames_verify_where_they_lie() {
    let b = frame(Lsn(256), 4, 1, &[0; 16], &records(10), BLOCK);
    assert!(b.len() > BLOCK, "ten commits span blocks");
    let BlockState::Valid(h, _) = read_block(Lsn(256), &b[..BLOCK]) else {
        panic!("block 0 does not verify");
    };
    assert_eq!((4, 0), (h.seq, h.index));
    assert_eq!(BlockState::Invalid, read_block(Lsn(384), &b[..BLOCK]));
    assert_eq!(BlockState::Zero, read_block(Lsn(0), &[0; BLOCK]));
}

#[test]
fn scan_accepts_every_whole_batch() {
    let (mut img, starts) = image(&[3, 9, 1]);
    let (s, batches, _) = scanned(&mut img, Mode::Recovery);
    assert_eq!(3, batches.len());
    assert_eq!(starts[3], s.end);
    assert_eq!(Some(3), s.last_seq);
    assert!(s.findings.is_empty() && s.fatal.is_none());
    let counts: Vec<usize> = batches
        .iter()
        .map(|b| b.records(BLOCK).unwrap().len())
        .collect();
    assert_eq!(vec![3, 9, 1], counts);
}

#[test]
fn a_partly_written_last_batch_ends_the_log_quietly() {
    let (mut img, starts) = image(&[3, 9]);
    let second_block = starts[1].0 as usize + BLOCK;
    seg(&mut img)[second_block..second_block + BLOCK].fill(0);
    let (s, batches, _) = scanned(&mut img, Mode::Recovery);
    assert_eq!(1, batches.len());
    assert_eq!(starts[1], s.end);
    assert!(s.findings.is_empty(), "{:?}", s.findings);
}

#[test]
fn a_torn_block_past_the_end_ends_the_log_quietly() {
    let (mut img, starts) = image(&[3, 9]);
    let second_block = starts[1].0 as usize + BLOCK;
    seg(&mut img)[second_block + 50] ^= 4;
    let (s, _, _) = scanned(&mut img, Mode::Recovery);
    assert_eq!(starts[1], s.end);
    assert!(s.findings.is_empty(), "{:?}", s.findings);
    assert!(s.fatal.is_none());
}

#[test]
fn verify_reports_a_damaged_block_past_the_end() {
    let (mut img, starts) = image(&[3, 9]);
    let second_block = starts[1].0 as usize + BLOCK;
    seg(&mut img)[second_block + 50] ^= 4;
    let (s, _, _) = scanned(&mut img, Mode::Verify);
    assert_eq!(starts[1], s.end);
    assert_eq!(LogProblem::Damaged, s.findings[0].problem);
}

#[test]
fn a_later_batch_after_a_bad_one_is_a_fatal_gap() {
    let (mut img, starts) = image(&[3, 9, 2]);
    let at = starts[1].0 as usize + 40;
    seg(&mut img)[at] ^= 1;
    let (s, _, _) = scanned(&mut img, Mode::Recovery);
    assert_eq!(starts[1], s.end);
    assert_eq!(Some(LogProblem::Gap), s.fatal.map(|f| f.problem));
}

#[test]
fn verify_resumes_after_a_gap() {
    let (mut img, starts) = image(&[3, 9, 2]);
    let at = starts[1].0 as usize + 40;
    seg(&mut img)[at] ^= 1;
    let (s, batches, resumed) = scanned(&mut img, Mode::Verify);
    assert_eq!(2, batches.len());
    assert_eq!(vec![1], resumed);
    assert_eq!(starts[2], batches[1].start);
    assert!(s.findings.iter().any(|f| f.problem == LogProblem::Gap));
}

#[test]
fn a_stale_block_of_an_older_batch_is_never_a_continuation() {
    let (mut img, starts) = image(&[2]);
    let at = starts[1].0 as usize;
    let discarded = frame(starts[1], 2, 1, &[0; 16], &records(9), BLOCK);
    let rewritten = frame(starts[1], 2, 1, &[0; 16], &records(1), BLOCK);
    let newer = frame(
        Lsn(starts[1].0 + BLOCK as u64),
        3,
        1,
        &[0; 16],
        &records(1),
        BLOCK,
    );
    seg(&mut img)[at..at + discarded.len()].copy_from_slice(&discarded);
    seg(&mut img)[at..at + rewritten.len()].copy_from_slice(&rewritten);
    let at = at + BLOCK;
    seg(&mut img)[at..at + newer.len()].copy_from_slice(&newer);
    let (s, batches, _) = scanned(&mut img, Mode::Recovery);
    assert_eq!(3, batches.len());
    assert!(batches.iter().all(|b| b.records(BLOCK).unwrap().len() < 3));
    assert!(!s.findings.is_empty());
    assert!(
        s.findings.iter().all(|f| f.problem == LogProblem::Stale),
        "{:?}",
        s.findings
    );
}

#[test]
fn batch_numbers_must_follow_on() {
    let (mut img, starts) = image(&[2, 2]);
    let wrong = frame(starts[1], 7, 1, &[0; 16], &records(2), BLOCK);
    let at = starts[1].0 as usize;
    seg(&mut img)[at..at + wrong.len()].copy_from_slice(&wrong);
    let (s, batches, _) = scanned(&mut img, Mode::Recovery);
    assert_eq!(1, batches.len());
    assert_eq!(Some(LogProblem::Gap), s.fatal.map(|f| f.problem));
}

#[test]
fn an_unreadable_segment_stops_recovery() {
    let (mut img, _) = image(&[2]);
    img.source.0.insert(SEGMENT, None);
    img.bases.push(SEGMENT);
    let (s, _, _) = scanned(&mut img, Mode::Recovery);
    assert_eq!(Some(LogProblem::Unreadable), s.fatal.map(|f| f.problem));
}

mod evicted {
    use super::super::evicted::EvictedLsns;
    use super::super::{Lsn, Pid};

    #[test]
    fn remembers_what_it_was_told() {
        let mut t = EvictedLsns::new(8);
        t.insert(Pid(5), Lsn(40));
        t.insert(Pid(6), Lsn(41));
        assert_eq!(Some(Lsn(40)), t.get(Pid(5)));
        assert_eq!(Some(Lsn(41)), t.get(Pid(6)));
        assert_eq!(None, t.get(Pid(7)));
        t.insert(Pid(5), Lsn(90));
        assert_eq!(Some(Lsn(90)), t.get(Pid(5)));
    }

    #[test]
    fn never_grows_past_its_capacity() {
        let mut t = EvictedLsns::new(10);
        let capacity = t.capacity();
        assert!(capacity >= 10);
        for n in 0..100_000 {
            t.insert(Pid(n), Lsn(n + 1));
        }
        assert_eq!(capacity, t.capacity());
        assert!(t.len() <= capacity);
    }

    #[test]
    fn a_page_it_dropped_is_never_reported_with_another_page_lsn() {
        let mut t = EvictedLsns::new(4);
        for n in 0..64 {
            t.insert(Pid(n), Lsn(1000 + n));
        }
        for n in 0..64 {
            if let Some(l) = t.get(Pid(n)) {
                assert_eq!(Lsn(1000 + n), l);
            }
        }
        let kept = (0..64).filter(|&n| t.get(Pid(n)).is_some()).count();
        assert_eq!(t.capacity(), kept, "the newest pages fill every slot");
    }
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

#[test]
fn a_page_at_the_next_lsn_is_past_the_log_end_and_one_below_it_is_not() {
    let next = Lsn(4096);
    let stale = Damage::Stale {
        expected: next,
        found: next,
    };
    assert_eq!(Some(stale), Damage::past_log_end(next, next));
    assert_eq!(None, Damage::past_log_end(Lsn(4095), next));
}

/// Returns three segments of one-block batches, the batch at block `n` holding the commit of
/// transaction `n`.
fn committed_segments() -> BTreeMap<u64, Option<Vec<u8>>> {
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

fn commits(segments: BTreeMap<u64, Option<Vec<u8>>>, since: Lsn) -> Result<Vec<u64>, Lsn> {
    let mut img = LogImage::in_memory(BLOCK, SEGMENT, segments);
    commits_from(&mut img, since, block_at(48)).map(|c| c.into_iter().collect())
}

#[test]
fn commits_from_a_whole_log_are_those_from_the_batch_holding_the_start() {
    let got = commits(committed_segments(), Lsn(block_at(20).0 + 5));
    assert_eq!(Ok((20..48).collect()), got);
}

#[test]
fn commits_from_before_the_first_segment_start_at_its_first_whole_batch() {
    let mut held = committed_segments();
    held.remove(&0);
    assert_eq!(Ok((16..48).collect()), commits(held, block_at(3)));
}

#[test]
fn commits_from_a_log_damaged_before_its_end_fail_at_the_damage() {
    let mut unreadable = committed_segments();
    unreadable.insert(SEGMENT, None);
    assert_eq!(Err(block_at(16)), commits(unreadable, block_at(2)));
    let mut missing = committed_segments();
    missing.remove(&SEGMENT);
    assert_eq!(Err(block_at(16)), commits(missing.clone(), block_at(2)));
    assert_eq!(Err(block_at(18)), commits(missing, block_at(18)));
    let mut short = committed_segments();
    short.remove(&(2 * SEGMENT));
    assert_eq!(Err(block_at(32)), commits(short, block_at(2)));
    let mut undecoded = committed_segments();
    let at = (block_at(20).0 - SEGMENT) as usize;
    let bad = frame(block_at(20), 21, 1, &[0; 16], &[0xff; 8], BLOCK);
    undecoded.get_mut(&SEGMENT).unwrap().as_mut().unwrap()[at..at + BLOCK].copy_from_slice(&bad);
    assert_eq!(Err(block_at(20)), commits(undecoded, block_at(2)));
}

#[test]
fn commits_from_a_log_damaged_only_past_its_end_are_whole() {
    let mut short = committed_segments();
    short.remove(&(2 * SEGMENT));
    let mut img = LogImage::in_memory(BLOCK, SEGMENT, short);
    let got = commits_from(&mut img, block_at(2), block_at(32));
    assert_eq!(Ok((2..32).collect()), got);
}

#[test]
fn commits_from_a_batch_begun_before_the_first_segment_start_at_its_first_whole_batch() {
    let mut held = committed_segments();
    let straddling = frame(block_at(15), 16, 1, &[0; 16], &records(6), BLOCK);
    assert_eq!(2 * BLOCK, straddling.len());
    let seg = held.get_mut(&SEGMENT).unwrap().as_mut().unwrap();
    seg[..BLOCK].copy_from_slice(&straddling[BLOCK..]);
    held.remove(&0);
    let since = Lsn(block_at(16).0 + 50);
    assert_eq!(Ok((17..48).collect()), commits(held, since));
}
