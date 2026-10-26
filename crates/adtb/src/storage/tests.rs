use std::collections::BTreeMap;

use super::alloc::diff;
use super::block::{
    Batch, BlockState, InMemory, LogImage, LogProblem, Mode, Scan, frame, read_block, scan,
};
use super::crc::crc32c;
use super::header::{FileHeader, choose};
use super::page::{self, Verdict};
use super::record::{Edit, Record};
use super::{Lsn, Pid, Position};

const BLOCK: usize = 128;
const SEGMENT: u64 = 16 * BLOCK as u64;

#[test]
fn crc32c_check_value() {
    assert_eq!(0xE306_9283, crc32c(b"123456789"));
    assert_eq!(0, crc32c(b""));
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
    }
}

#[test]
fn header_copy_round_trips_and_binds_its_copy_number() {
    let bytes = header(4).encode(1);
    assert_eq!(Some(header(4)), FileHeader::decode(1, &bytes));
    assert_eq!(None, FileHeader::decode(0, &bytes));
    let mut torn = bytes.clone();
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
        let b = frame(Lsn(at as u64), i as u64 + 1, 1, &records(n), BLOCK);
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
    let b = frame(Lsn(256), 4, 1, &records(10), BLOCK);
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
    let discarded = frame(starts[1], 2, 1, &records(9), BLOCK);
    let rewritten = frame(starts[1], 2, 1, &records(1), BLOCK);
    let newer = frame(Lsn(starts[1].0 + BLOCK as u64), 3, 1, &records(1), BLOCK);
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
    let wrong = frame(starts[1], 7, 1, &records(2), BLOCK);
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
