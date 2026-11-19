use std::collections::BTreeSet;

use super::evidence::{Against, Begun, Evidence, Judged, Met, refusal_for_test};
use super::page::Verdict;
use crate::storage::error::Damage;
use crate::storage::log::Log;
use crate::storage::pager::{appending_for_test, wrote_for_test};
use crate::storage::quarantine::Quarantine;
use crate::storage::{Lsn, Pid};

const P: Pid = Pid(9);
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
    e.refuse(&appending_for_test(), log, &refusal, damage, false)
}

fn logged(e: &mut Evidence, pid: Pid, met: Met) {
    e.logged_page(&appending_for_test(), pid, Lsn(100), Some(met));
}

fn folded(e: &mut Evidence) -> Option<Damage> {
    e.snapshot(usize::MAX).pages.get(&P).copied()
}

/// Returns each page whose damage is unmapped, with that damage, and whether a full list is.
///
/// Panic if the unmapped flag differs from whether any is.
fn unmapped(e: &Evidence) -> (Vec<(Pid, Damage)>, bool) {
    let (pages, full, _) = e.begin_mapping(true);
    assert_eq!(
        !pages.is_empty() || full,
        e.unmapped_flag().any(),
        "the flag says whether a record or a full list is unmapped"
    );
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
fn the_mapping_goes_with_the_damage() {
    let mut log = log();
    let mut e = recording(&mut log);
    let objects = BTreeSet::from([Quarantine::Database]);
    refuse(&mut e, &mut log, P, Damage::Checksum);
    let begun = e.begin_mapping(true);
    map(&mut e, begun);
    assert_eq!(Some(objects), e.mapped_to(P));
    logged(&mut e, P, Met::Image);
    assert_eq!(None, e.mapped_to(P));
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
fn a_list_that_overflows_with_every_record_mapped_is_unmapped_until_it_maps() {
    let mut log = log();
    let mut e = recording(&mut log);
    e.record(&appending_for_test(), &mut log, P, Damage::Checksum, false);
    let begun = e.begin_mapping(true);
    map(&mut e, begun);
    assert_eq!((Vec::new(), false), unmapped(&e), "every record is mapped");
    e.snapshot(0);
    assert_eq!(
        (Vec::new(), true),
        unmapped(&e),
        "the full list is unmapped"
    );
    let (_, _, at) = e.begin_mapping(true);
    e.mapped(true, at, Vec::new());
    assert_eq!((Vec::new(), false), unmapped(&e), "the full list is mapped");
}

#[test]
fn a_write_out_that_exposes_a_record_a_hold_covered_leaves_it_unmapped_until_it_maps() {
    let mut log = log();
    let mut e = recording(&mut log);
    e.record(&appending_for_test(), &mut log, P, STALE, false);
    e.hold(P, Damage::Checksum);
    let begun = e.begin_mapping(true);
    map(&mut e, begun);
    assert_eq!(
        (Vec::new(), false),
        unmapped(&e),
        "the held damage is mapped"
    );
    e.written_out(&wrote_for_test(P));
    assert_eq!((vec![(P, STALE)], false), unmapped(&e));
    let (pending, _, at) = e.begin_mapping(true);
    e.mapped(false, at, vec![(pending[0], INCOMPLETE)]);
    assert_eq!(
        (vec![(P, STALE)], false),
        unmapped(&e),
        "an incomplete mapping leaves it unmapped"
    );
    map(&mut e, (pending, false, at));
    assert_eq!(
        (Vec::new(), false),
        unmapped(&e),
        "the last record is mapped"
    );
}

const END: Lsn = Lsn(100);

/// Returns evidence whose page `P` must reach LSN 50 when read in.
fn expecting() -> Evidence {
    let mut e = Evidence::new(Pid(0));
    e.arm_created();
    e.logged_page(&appending_for_test(), P, Lsn(50), None);
    e
}

fn behind(expected: u64, found: u64) -> Judged {
    Judged::Behind(Damage::Stale {
        expected: Lsn(expected),
        found: Lsn(found),
    })
}

fn stale(found: u64) -> Judged {
    Judged::Stale(Damage::Stale {
        expected: END,
        found: Lsn(found),
    })
}

/// A copy of a page, with its judgement against the expected LSN of a live page expecting 50, of
/// a live page expecting none, of a freed page that expected 50 and of a page while recovery
/// runs, then against a resident LSN of 60 and against the log's end alone.
type Row = (Result<(Lsn, Verdict), Damage>, [Judged; 6]);

#[test]
fn a_copy_is_judged_by_its_read_its_checksum_the_log_end_and_what_it_must_reach() {
    use Judged::{Bad, Sound};
    let (unreadable, rot) = (Bad(Damage::Unreadable), Bad(Damage::Checksum));
    let valid = |lsn| Ok((Lsn(lsn), Verdict::Valid));
    let rows: [Row; 8] = [
        (Err(Damage::Unreadable), [unreadable; 6]),
        (Ok((Lsn(50), Verdict::Corrupt)), [rot; 6]),
        (
            Ok((Lsn(0), Verdict::Blank)),
            [behind(50, 0), Sound, Sound, Sound, behind(60, 0), Sound],
        ),
        (
            valid(49),
            [behind(50, 49), Sound, Sound, Sound, behind(60, 49), Sound],
        ),
        (
            valid(50),
            [Sound, Sound, Sound, Sound, behind(60, 50), Sound],
        ),
        (valid(60), [Sound; 6]),
        (valid(100), [stale(100); 6]),
        (valid(120), [stale(120); 6]),
    ];
    let live = expecting();
    let mut freed = expecting();
    freed.free(&appending_for_test(), &mut log(), P, false);
    let recovering = Evidence::new(Pid(0));
    let judges = [
        (&live, P, Against::Expected),
        (&live, Pid(10), Against::Expected),
        (&freed, P, Against::Expected),
        (&recovering, P, Against::Expected),
        (&live, P, Against::Resident(Lsn(60))),
        (&live, P, Against::LogEnd),
    ];
    let mut cells = 0;
    for (copy, expected) in rows {
        for (&(e, pid, against), want) in judges.iter().zip(expected) {
            let got = e.judge(pid, copy, against, END);
            assert_eq!(want, got, "{copy:?} of {pid:?} against {against:?}");
            cells += 1;
        }
    }
    assert_eq!(rows.len() * judges.len(), cells);
    let blank = live.judge(P, Ok((Lsn(0), Verdict::Blank)), Against::LogEnd, Lsn(0));
    assert_eq!(Sound, blank, "LSN 0 is never past the log");
}

#[test]
fn a_freed_pages_next_incarnation_is_judged_by_its_own_expected_lsn() {
    let mut e = expecting();
    e.free(&appending_for_test(), &mut log(), P, false);
    e.logged_page(&appending_for_test(), P, Lsn(60), Some(Met::Start));
    let found = e.judge(P, Ok((Lsn(55), Verdict::Valid)), Against::Expected, END);
    assert_eq!(behind(60, 55), found);
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
fn only_a_change_of_damage_incarnation_or_retirement_supersedes_a_record() {
    let mut log = log();
    let mut e = recording(&mut log);
    refuse(&mut e, &mut log, P, Damage::Checksum);
    let begun = e.begin_mapping(true);
    refuse(&mut e, &mut log, P, Damage::Checksum);
    assert_eq!(vec![P], current(&e, &begun), "the same damage again");
    e.retire([P]);
    assert_eq!(Vec::<Pid>::new(), current(&e, &begun), "retired");
    let begun = e.begin_mapping(true);
    e.unretire(&[P]);
    assert_eq!(Vec::<Pid>::new(), current(&e, &begun), "unretired");
    let begun = e.begin_mapping(true);
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
