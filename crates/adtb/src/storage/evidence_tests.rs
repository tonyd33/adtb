use std::collections::BTreeSet;

use super::evidence::{Against, Begun, Evidence, Judged, Met, refusal_for_test};
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
    let mut e = Evidence::new(Pid(0));
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

/// Returns each page whose damage is unmapped, with that damage, and whether a full list is.
fn unmapped(e: &Evidence) -> (Vec<(Pid, Damage)>, bool) {
    let (pages, full, _) = e.begin_mapping(true);
    (
        pages
            .iter()
            .filter_map(|p| Some((p.pid(), e.damage(p.pid())?)))
            .collect(),
        full,
    )
}

/// Enter the mapping of each record of `begun` to the database.
fn map(e: &mut Evidence, (pending, _, at): Begun) {
    e.mapped(
        false,
        at,
        pending.into_iter().map(|p| (p, database())).collect(),
    );
}

#[test]
fn a_refusal_is_recorded_by_the_step_that_refuses() {
    let mut log = log();
    let mut e = Evidence::new(Pid(0));
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
    e.unrefuse(&refusal_for_test(P));
    logged(&mut e, P, Met::Start);
    assert!(refuse(&mut e, &mut log, P, Damage::Checksum).is_some());
    assert_eq!(
        Some(Damage::Checksum),
        folded(&mut e),
        "after the chain start"
    );
    let mut replayed = recording(&mut log);
    replayed.replay_met(&replaying_for_test(), P, Met::Freed, Lsn(1));
    assert_eq!(None, refuse(&mut replayed, &mut log, P, Damage::Checksum));
    assert_eq!(None, folded(&mut replayed), "a replayed free");
}

#[test]
fn the_mapping_goes_with_the_damage() {
    let mut log = log();
    let mut e = recording(&mut log);
    let objects = BTreeSet::from([Quarantine::Database]);
    refuse(&mut e, &mut log, P, Damage::Checksum);
    let begun = e.begin_mapping(true);
    map(&mut e, begun);
    assert_eq!(Some(objects), e.mapped_to(P));
    assert_eq!(Some(BY), e.detected_by(P));
    logged(&mut e, P, Met::Image);
    assert_eq!((None, None), (e.mapped_to(P), e.detected_by(P)));
    e.unrefuse(&refusal_for_test(P));
    refuse(&mut e, &mut log, P, Damage::Checksum);
    assert_eq!(
        (vec![(P, Damage::Checksum)], false),
        unmapped(&e),
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
    let begun = e.begin_mapping(true);
    map(&mut e, begun);
    assert!(!flag.any(), "every record is mapped");
    e.snapshot(0);
    assert_eq!((Vec::new(), true), unmapped(&e));
    assert!(flag.any(), "the full list is unmapped");
    let (_, _, at) = e.begin_mapping(true);
    e.mapped(true, at, Vec::new());
    assert!(!flag.any(), "the full list is mapped");
}

#[test]
fn a_write_out_that_exposes_an_unmapped_record_raises_the_flag() {
    let mut log = log();
    let mut e = recording(&mut log);
    let flag = e.unmapped_flag();
    e.record(&appending_for_test(), &mut log, P, STALE, false);
    e.hold(P, Damage::Checksum);
    let begun = e.begin_mapping(true);
    map(&mut e, begun);
    assert!(!flag.any(), "the held damage is mapped");
    e.written_out(&wrote_for_test(P));
    assert_eq!((vec![(P, STALE)], false), unmapped(&e));
    assert!(flag.any(), "the record the hold covered is unmapped");
    let (pending, _, at) = e.begin_mapping(true);
    e.mapped(false, at, vec![(pending[0], INCOMPLETE)]);
    assert!(flag.any(), "an incomplete mapping leaves it unmapped");
    map(&mut e, (pending, false, at));
    assert!(!flag.any(), "the last record is mapped");
}

const END: Lsn = Lsn(100);

/// Returns evidence whose page `P` must reach LSN 50 when read in.
fn expecting() -> Evidence {
    let mut e = Evidence::new(Pid(0));
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

fn behind(expected: u64, found: u64) -> Judged {
    Judged::Behind(Damage::Stale {
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
fn against_the_expected_lsn_a_copy_below_it_is_behind_and_one_past_the_log_stale() {
    let e = expecting();
    let a = Against::Expected;
    assert_eq!(Judged::Sound, judged(&e, P, Lsn(50), a));
    assert_eq!(behind(50, 49), judged(&e, P, Lsn(49), a));
    let blank = e.judge(P, Ok((Lsn(0), Verdict::Blank)), a, END);
    assert_eq!(
        behind(50, 0),
        blank,
        "a blank copy of a page with an expected LSN"
    );
    assert_eq!(stale(100, 100), judged(&e, P, END, a));
    assert_eq!(
        Judged::Sound,
        judged(&e, Pid(10), Lsn(1), a),
        "no expected LSN"
    );
    let unarmed = Evidence::new(Pid(0));
    assert_eq!(Judged::Sound, judged(&unarmed, P, Lsn(1), a), "recovering");
}

#[test]
fn against_the_resident_lsn_a_copy_below_it_is_behind_and_one_past_the_log_stale() {
    let e = expecting();
    let a = Against::Resident(Lsn(60));
    assert_eq!(Judged::Sound, judged(&e, P, Lsn(60), a));
    assert_eq!(
        behind(60, 55),
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

#[test]
fn a_free_holds_a_refusal_as_the_free_copys_damage_and_ends_a_stale_one() {
    for (damage, refused, held) in [
        (
            Damage::Checksum,
            Some(Damage::Checksum),
            Some(Damage::Checksum),
        ),
        (STALE, None, None),
        (Damage::Unreadable, Some(Damage::Unreadable), None),
    ] {
        let mut log = log();
        let mut e = recording(&mut log);
        refuse(&mut e, &mut log, P, damage);
        e.free(&appending_for_test(), &mut log, P, false);
        assert_eq!(
            (refused, held, None),
            (e.refusal(P), e.damage(P), folded(&mut e)),
            "{damage:?}: nothing folded"
        );
        let mapped = held.map(|_| BTreeSet::new());
        let after_free = (unmapped(&e), e.mapped_to(P));
        let at = log.next_lsn();
        refuse(&mut e, &mut log, P, damage);
        assert_eq!(at, log.next_lsn(), "{damage:?}: nothing logged");
        let again = (damage != Damage::Unreadable).then_some(damage);
        assert_eq!(again, e.damage(P), "{damage:?} refused again is held");
        assert_eq!(
            ((Vec::new(), false), mapped),
            after_free,
            "{damage:?}: the free copy's damage maps to nothing"
        );
    }
}

#[test]
fn a_mapping_of_one_incarnation_maps_nothing_of_the_next() {
    let mut log = log();
    let mut e = recording(&mut log);
    refuse(&mut e, &mut log, P, Damage::Checksum);
    let begun = e.begin_mapping(true);
    e.unrefuse(&refusal_for_test(P));
    logged(&mut e, P, Met::Start);
    refuse(&mut e, &mut log, P, Damage::Checksum);
    map(&mut e, begun);
    assert_eq!(
        (vec![(P, Damage::Checksum)], false),
        unmapped(&e),
        "after a chain start"
    );
    let begun = e.begin_mapping(true);
    e.free(&appending_for_test(), &mut log, P, false);
    map(&mut e, begun);
    assert_eq!(Some(BTreeSet::new()), e.mapped_to(P), "after a free");
}

#[test]
fn a_freed_pages_copy_is_not_judged_by_the_dead_incarnations_expected_lsn() {
    let mut log = log();
    let mut e = expecting();
    e.free(&appending_for_test(), &mut log, P, false);
    let a = Against::Expected;
    assert_eq!(Judged::Sound, judged(&e, P, Lsn(20), a));
    let blank = e.judge(P, Ok((Lsn(0), Verdict::Blank)), a, END);
    assert_eq!(Judged::Sound, blank, "a blank copy");
    assert_eq!(stale(100, 100), judged(&e, P, END, a), "past the log");
    e.logged_page(&appending_for_test(), P, Lsn(60), Some(Met::Start));
    assert_eq!(
        behind(60, 55),
        judged(&e, P, Lsn(55), a),
        "the next incarnation"
    );
}

/// Returns the records of `begun` still current, each mapped to the database.
fn current(e: &Evidence, (pending, _, at): &Begun) -> Vec<Pid> {
    let mapped = pending.iter().map(|p| (*p, database())).collect();
    e.current(*at, mapped)
        .iter()
        .map(|(p, _)| p.pid())
        .collect()
}

#[test]
fn a_record_superseded_after_its_mapping_begins_maps_nothing_and_its_successor_maps() {
    let mut log = log();
    let mut e = recording(&mut log);
    refuse(&mut e, &mut log, P, STALE);
    let begun = e.begin_mapping(true);
    refuse(&mut e, &mut log, P, Damage::Checksum);
    assert_eq!(Vec::<Pid>::new(), current(&e, &begun), "superseded");
    map(&mut e, begun);
    assert_eq!(
        (None, (vec![(P, Damage::Checksum)], false)),
        (e.mapped_to(P), unmapped(&e)),
        "the successor is unmapped"
    );
    let begun = e.begin_mapping(true);
    map(&mut e, begun);
    assert_eq!(database(), e.mapped_to(P), "the successor maps on its own");
}

#[test]
fn only_a_change_of_damage_or_incarnation_supersedes_a_record() {
    let mut log = log();
    let mut e = recording(&mut log);
    refuse(&mut e, &mut log, P, Damage::Checksum);
    let begun = e.begin_mapping(true);
    refuse(&mut e, &mut log, P, Damage::Checksum);
    e.retire([P]);
    assert_eq!(vec![P], current(&e, &begun), "the same damage again");
    e.unretire(&[P]);
    e.unrefuse(&refusal_for_test(P));
    refuse(&mut e, &mut log, P, STALE);
    e.unrefuse(&refusal_for_test(P));
    refuse(&mut e, &mut log, P, Damage::Checksum);
    assert_eq!(
        Vec::<Pid>::new(),
        current(&e, &begun),
        "replaced and found again"
    );
}

#[test]
fn a_fold_holds_the_redo_bound_until_its_change_of_the_list_is_logged() {
    let mut log = log();
    let mut e = recording(&mut log);
    let list = Pid(2);
    e.create_list(list);
    let at = log.next_lsn();
    refuse(&mut e, &mut log, P, Damage::Checksum);
    assert_eq!(Some(at), e.redo_bound(), "the record");
    assert_eq!(Some(Damage::Checksum), folded(&mut e));
    assert_eq!(
        (Some(at), Some(list)),
        (e.redo_bound(), e.to_fold()),
        "the fold taken, not logged"
    );
    e.fold_logged();
    assert_eq!(
        (None, None),
        (e.redo_bound(), e.to_fold()),
        "the fold logged"
    );
}
