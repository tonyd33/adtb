use std::collections::BTreeSet;

use super::evidence::{Against, Evidence, Judged, Met, refusal_for_test};
use super::page::Verdict;
use crate::runtime::ThreadId;
use crate::storage::error::Damage;
use crate::storage::log::Log;
use crate::storage::pager::{appending_for_test, wrote_for_test};
use crate::storage::quarantine::Quarantine;
use crate::storage::recovery::replaying_for_test;
use crate::storage::{Lsn, Pid};

const P: Pid = Pid(9);
const BY: ThreadId = ThreadId(3);
const STALE: Damage = Damage::Stale {
    expected: Lsn(40),
    found: Lsn(20),
};
const SETTLED: Damage = Damage::Settled {
    expected: Lsn(40),
    found: Lsn(20),
};

fn log() -> Log {
    Log::new(512, 1, [0; 16])
}

/// Returns evidence past recovery, recording and logging.
fn recording(log: &mut Log) -> Evidence {
    let mut e = Evidence::new();
    e.start_recording(&appending_for_test(), log, false);
    e
}

fn refuse(e: &mut Evidence, log: &mut Log, pid: Pid, damage: Damage) -> Option<Lsn> {
    let refusal = refusal_for_test(pid);
    e.refuse(&appending_for_test(), log, &refusal, damage, false, BY)
}

fn logged(e: &mut Evidence, pid: Pid, met: Met) {
    e.logged_page(&appending_for_test(), pid, Lsn(100), Some(met));
}

fn folded(e: &mut Evidence) -> Option<Damage> {
    e.snapshot(usize::MAX).pages.get(&P).copied()
}

#[test]
fn a_refusal_is_recorded_by_the_step_that_refuses() {
    let mut log = log();
    let mut e = Evidence::new();
    assert_eq!(None, refuse(&mut e, &mut log, P, Damage::Checksum));
    assert_eq!(
        (Some(Damage::Checksum), None),
        (e.refusal(P), e.damage(P)),
        "recovery records nothing"
    );
    e.start_recording(&appending_for_test(), &mut log, false);
    assert_eq!(Some(Damage::Checksum), e.damage(P), "the end of recovery");
    let q = Pid(10);
    let end = refuse(&mut e, &mut log, q, STALE);
    assert_eq!(Some(log.next_lsn()), end);
    assert_eq!((Some(STALE), Some(STALE)), (e.refusal(q), e.damage(q)));
    assert_eq!(Some(BY), e.detected_by(q));
    assert_eq!(None, refuse(&mut e, &mut log, q, STALE), "refused already");
    assert_eq!(None, refuse(&mut e, &mut log, q, Damage::Unreadable));
    assert_eq!(
        (Some(Damage::Unreadable), Some(STALE)),
        (e.refusal(q), e.damage(q)),
        "an unreadable page is refused unrecorded"
    );
}

#[test]
fn a_full_image_drops_damage_but_a_settle() {
    let mut log = log();
    let mut e = recording(&mut log);
    let a = &appending_for_test();
    for (damage, kept) in [(Damage::Checksum, None), (SETTLED, Some(SETTLED))] {
        e.record(a, &mut log, P, damage, false);
        e.hold(P, damage);
        logged(&mut e, P, Met::Image);
        assert_eq!(kept, e.damage(P), "{damage:?} held");
        assert_eq!(kept, folded(&mut e), "{damage:?} recorded");
        logged(&mut e, P, Met::Start);
        assert_eq!((None, None), (e.damage(P), folded(&mut e)), "{damage:?}");
    }
    e.record(a, &mut log, P, SETTLED, false);
    e.free(a, &mut log, P, false);
    assert_eq!((None, None), (e.damage(P), folded(&mut e)), "a free");
}

#[test]
fn damage_between_a_free_and_the_next_chain_start_is_held() {
    let mut log = log();
    let mut e = recording(&mut log);
    let a = &appending_for_test();
    e.free(a, &mut log, P, false);
    let at = log.next_lsn();
    assert_eq!(None, refuse(&mut e, &mut log, P, Damage::Checksum));
    assert_eq!(at, log.next_lsn(), "nothing logged");
    assert_eq!(
        (Some(Damage::Checksum), None),
        (e.damage(P), folded(&mut e))
    );
    e.unrefuse(P);
    logged(&mut e, P, Met::Start);
    assert!(refuse(&mut e, &mut log, P, Damage::Checksum).is_some());
    assert_eq!(
        Some(Damage::Checksum),
        folded(&mut e),
        "after the chain start"
    );
    let mut replayed = recording(&mut log);
    replayed.replay_met(&replaying_for_test(), P, Met::Freed, false);
    assert_eq!(None, refuse(&mut replayed, &mut log, P, Damage::Checksum));
    assert_eq!(None, folded(&mut replayed), "a replayed free");
}

#[test]
fn the_mapping_goes_with_the_damage() {
    let mut log = log();
    let mut e = recording(&mut log);
    let objects = BTreeSet::from([Quarantine::Database]);
    refuse(&mut e, &mut log, P, Damage::Checksum);
    e.mapped(false, vec![(P, Damage::Checksum, Some(objects.clone()))]);
    assert_eq!(Some(objects), e.mapped_to(P));
    assert_eq!(Some(BY), e.detected_by(P));
    logged(&mut e, P, Met::Image);
    assert_eq!((None, None), (e.mapped_to(P), e.detected_by(P)));
    e.unrefuse(P);
    refuse(&mut e, &mut log, P, Damage::Checksum);
    assert_eq!(
        (vec![(P, Damage::Checksum)], false),
        e.unmapped(true),
        "the same damage again is unmapped"
    );
}

const INCOMPLETE: Option<BTreeSet<Quarantine>> = None;

fn database() -> Option<BTreeSet<Quarantine>> {
    Some(BTreeSet::from([Quarantine::Database]))
}

#[test]
fn a_list_that_overflows_with_every_record_mapped_is_unmapped() {
    let mut log = log();
    let mut e = recording(&mut log);
    let flag = e.unmapped_flag();
    e.record(&appending_for_test(), &mut log, P, Damage::Checksum, false);
    e.mapped(false, vec![(P, Damage::Checksum, database())]);
    assert!(!flag.any(), "every record is mapped");
    e.snapshot(0);
    assert_eq!((Vec::new(), true), e.unmapped(true));
    assert!(flag.any(), "the full list is unmapped");
    e.mapped(true, Vec::new());
    assert!(!flag.any(), "the full list is mapped");
}

#[test]
fn a_write_out_that_exposes_an_unmapped_record_raises_the_flag() {
    let mut log = log();
    let mut e = recording(&mut log);
    let flag = e.unmapped_flag();
    e.record(&appending_for_test(), &mut log, P, STALE, false);
    e.hold(P, Damage::Checksum);
    e.mapped(false, vec![(P, Damage::Checksum, database())]);
    assert!(!flag.any(), "the held damage is mapped");
    e.written_out(&wrote_for_test(P));
    assert_eq!((vec![(P, STALE)], false), e.unmapped(true));
    assert!(flag.any(), "the record the hold covered is unmapped");
    e.mapped(false, vec![(P, STALE, INCOMPLETE)]);
    assert!(flag.any(), "an incomplete mapping leaves it unmapped");
    e.mapped(false, vec![(P, STALE, database())]);
    assert!(!flag.any(), "the last record is mapped");
}

const END: Lsn = Lsn(100);

/// Returns evidence whose page `P` must reach LSN 50 when read in.
fn expecting() -> Evidence {
    let mut e = Evidence::new();
    e.arm_created();
    e.logged_page(&appending_for_test(), P, Lsn(50), None);
    e
}

fn judged(e: &Evidence, pid: Pid, found: Lsn, against: Against) -> Judged {
    e.judge(pid, Ok((found, Verdict::Valid)), against, END)
}

fn stale(expected: u64, found: u64) -> Judged {
    Judged::Stale(Damage::Stale {
        expected: Lsn(expected),
        found: Lsn(found),
    })
}

#[test]
fn a_copy_that_fails_to_read_or_verify_is_bad_whatever_it_is_judged_against() {
    let e = expecting();
    for against in [
        Against::Expected,
        Against::Resident(Lsn(60)),
        Against::LogEnd,
    ] {
        let unreadable = e.judge(P, Err(Damage::Unreadable), against, END);
        assert_eq!(Judged::Bad(Damage::Unreadable), unreadable, "{against:?}");
        let corrupt = e.judge(P, Ok((Lsn(50), Verdict::Corrupt)), against, END);
        assert_eq!(Judged::Bad(Damage::Checksum), corrupt, "{against:?}");
    }
}

#[test]
fn against_the_expected_lsn_a_copy_below_it_or_past_the_log_is_stale() {
    let e = expecting();
    let a = Against::Expected;
    assert_eq!(Judged::Sound, judged(&e, P, Lsn(50), a));
    assert_eq!(stale(50, 49), judged(&e, P, Lsn(49), a));
    let blank = e.judge(P, Ok((Lsn(0), Verdict::Blank)), a, END);
    assert_eq!(
        stale(50, 0),
        blank,
        "a blank copy of a page with an expected LSN"
    );
    assert_eq!(stale(100, 100), judged(&e, P, END, a));
    assert_eq!(
        Judged::Sound,
        judged(&e, Pid(10), Lsn(1), a),
        "no expected LSN"
    );
    let unarmed = Evidence::new();
    assert_eq!(Judged::Sound, judged(&unarmed, P, Lsn(1), a), "recovering");
}

#[test]
fn against_the_resident_lsn_a_copy_below_it_or_past_the_log_is_stale() {
    let e = expecting();
    let a = Against::Resident(Lsn(60));
    assert_eq!(Judged::Sound, judged(&e, P, Lsn(60), a));
    assert_eq!(
        stale(60, 55),
        judged(&e, P, Lsn(55), a),
        "the expected LSN plays no part"
    );
    assert_eq!(stale(100, 120), judged(&e, P, Lsn(120), a));
}

#[test]
fn against_the_log_end_only_a_copy_past_it_is_stale() {
    let e = expecting();
    let a = Against::LogEnd;
    assert_eq!(Judged::Sound, judged(&e, P, Lsn(1), a));
    assert_eq!(Judged::Sound, judged(&e, P, Lsn(0), a));
    assert_eq!(stale(100, 100), judged(&e, P, END, a));
    let blank = e.judge(P, Ok((Lsn(0), Verdict::Blank)), a, Lsn(0));
    assert_eq!(Judged::Sound, blank, "LSN 0 is never past the log");
}
