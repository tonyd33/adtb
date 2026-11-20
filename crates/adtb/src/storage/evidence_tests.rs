use std::collections::BTreeSet;

use super::evidence::{Against, Begun, Cause, Evidence, Judged, Met, Pending, refusal_for_test};
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

fn unlogged(at: u64, found: u64) -> Judged {
    Judged::Unlogged(Damage::Stale {
        expected: Lsn(at),
        found: Lsn(found),
    })
}

/// Returns evidence of a reopened file: page `P`'s last record at 50 and page 10's map entry at
/// 40 in a log retained with no gap from before 80, the end recovery kept, and from its start
/// when `complete`.
fn armed_file(complete: bool) -> Evidence {
    let mut e = Evidence::new(Pid(0));
    let mut folded = vec![Lsn(0); 11];
    folded[10] = Lsn(40);
    e.arm_for_test(folded, &[(P, Lsn(50))], None, Some(Lsn(80)), complete);
    e
}

/// Returns evidence of a file whose retained log has its last gap at `gap` and page `P`'s last
/// record at 50.
fn gapped(gap: u64) -> Evidence {
    let mut e = Evidence::new(Pid(0));
    e.arm_for_test(Vec::new(), &[(P, Lsn(50))], Some(Lsn(gap)), None, false);
    e
}

const JUDGES: usize = 11;

/// A copy of a page, with its judgement by a read of a live page of a created file expecting 50,
/// of one expecting none, of a freed page that expected 50 and of a page while recovery runs; by a
/// read of the live page holding the pool's copy at 60; by a read of a reopened file's page with
/// its last record at 50 and of one with a map entry of 40; by a read of a page of a log retained
/// from its start with no record; of a page whose last record at 50 lies before a gap, by
/// `verify` and by a read; and of one whose last record at 50 lies after the last gap, by `verify`.
type Row = (Result<(Lsn, Verdict), Damage>, [Judged; JUDGES]);

#[test]
fn a_copy_is_judged_by_its_read_its_checksum_the_log_end_and_what_it_must_reach() {
    use Judged::{Bad, Orphan, Sound};
    let (unreadable, rot) = (Bad(Damage::Unreadable), Bad(Damage::Checksum));
    let valid = |lsn| Ok((Lsn(lsn), Verdict::Valid));
    let (b, u, s) = (behind, unlogged, Sound);
    let rows: [Row; 9] = [
        (Err(Damage::Unreadable), [unreadable; JUDGES]),
        (Ok((Lsn(50), Verdict::Corrupt)), [rot; JUDGES]),
        (
            Ok((Lsn(0), Verdict::Blank)),
            [
                b(50, 0),
                s,
                s,
                s,
                b(60, 0),
                b(50, 0),
                b(40, 0),
                s,
                b(50, 0),
                s,
                b(50, 0),
            ],
        ),
        (
            valid(49),
            [
                b(50, 49),
                u(0, 49),
                s,
                s,
                b(60, 49),
                b(50, 49),
                s,
                Orphan,
                b(50, 49),
                s,
                b(50, 49),
            ],
        ),
        (
            valid(50),
            [s, u(0, 50), s, s, b(60, 50), s, s, Orphan, s, s, s],
        ),
        (
            valid(60),
            [
                u(0, 60),
                u(0, 60),
                s,
                s,
                u(0, 60),
                u(50, 60),
                s,
                Orphan,
                s,
                s,
                u(50, 60),
            ],
        ),
        (
            valid(85),
            [
                u(0, 85),
                u(0, 85),
                s,
                s,
                u(0, 85),
                u(80, 85),
                u(80, 85),
                u(80, 85),
                s,
                s,
                u(50, 85),
            ],
        ),
        (valid(100), [stale(100); JUDGES]),
        (valid(120), [stale(120); JUDGES]),
    ];
    let live = expecting();
    let mut freed = expecting();
    freed.free(&appending_for_test(), &mut log(), P, false);
    let recovering = Evidence::new(Pid(0));
    let (recycled, complete) = (armed_file(false), armed_file(true));
    let (before_gap, after_gap) = (gapped(60), gapped(40));
    let judges: [(&Evidence, Pid, Against); JUDGES] = [
        (&live, P, Against::Read(None)),
        (&live, Pid(10), Against::Read(None)),
        (&freed, P, Against::Read(None)),
        (&recovering, P, Against::Read(None)),
        (&live, P, Against::Read(Some(Lsn(60)))),
        (&recycled, P, Against::Read(None)),
        (&recycled, Pid(10), Against::Read(None)),
        (&complete, Pid(11), Against::Read(None)),
        (&before_gap, P, Against::Verify),
        (&before_gap, P, Against::Read(None)),
        (&after_gap, P, Against::Verify),
    ];
    let mut cells = 0;
    for (copy, expected) in rows {
        for (&(e, pid, against), want) in judges.iter().zip(expected) {
            let got = e.judge(pid, copy, against, END);
            assert_eq!(want, got, "{copy:?} of {pid:?} against {against:?}");
            cells += 1;
        }
    }
    assert_eq!(rows.len() * JUDGES, cells);
    let blank = recovering.judge(P, Ok((Lsn(0), Verdict::Blank)), Against::Read(None), Lsn(0));
    assert_eq!(Sound, blank, "LSN 0 is never past the log");
}

#[test]
fn a_freed_pages_next_incarnation_is_judged_by_its_own_expected_lsn() {
    let mut e = expecting();
    e.free(&appending_for_test(), &mut log(), P, false);
    e.logged_page(&appending_for_test(), P, Lsn(60), Some(Met::Start));
    let found = e.judge(P, Ok((Lsn(55), Verdict::Valid)), Against::Read(None), END);
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

/// Returns the records of `begun` still current.
fn current(e: &Evidence, (pending, _, at): &Begun) -> Vec<Pid> {
    let current = pending
        .iter()
        .filter(|p| e.stands(&Cause::Record(**p, *at)));
    current.map(Pending::pid).collect()
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
