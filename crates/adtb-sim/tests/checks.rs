#[expect(dead_code, reason = "this binary uses part of the device helpers")]
mod common;
#[path = "support/seeds.rs"]
mod seeds;

use std::ops::Range;
use std::time::Duration;

use adtb_sim::{Config, Faults, FindingKind, Ledger, Outcome, Sim, Violation, check};
use common::{Durability, commit_all, recover, run, sleep};
use seeds::{share, sweep};

fn sim(seed: u64, faults: Faults) -> Sim {
    Sim::new(
        seed,
        Config {
            faults,
            ..Config::default()
        },
    )
}

/// Commit ids 0..8 with `durability`, crash at a seed-chosen time, reopen, and return the check's verdict
/// with the number of commits acknowledged before the crash.
fn crash_and_check(seed: u64, durability: Durability, faults: Faults) -> (Vec<Violation>, usize) {
    let mut sim = sim(seed, faults);
    let ledger = Ledger::new();
    let crash = Duration::from_micros(200 + seed * 97 % 3000);
    sim.crash_at(sim.now() + crash);
    let l = ledger.clone();
    let outcome = sim.run(move |rt| {
        commit_all(&rt, &l, &(0..8).collect::<Vec<_>>(), durability);
        let mut io = adtb::runtime::Runtime::io(&rt);
        loop {
            sleep(&rt, &mut io, Duration::from_millis(1));
        }
    });
    assert_eq!(Outcome::Crashed, outcome);
    let store = run(&mut sim, |rt, _| recover(rt));
    let acked = (0..8)
        .filter(|&id| ledger.fate(id) == Some(adtb_sim::Fate::Acknowledged))
        .count();
    (check(&ledger, &store), acked)
}

#[test]
fn forced_commits_pass_after_any_crash() {
    const SEEDS: Range<u64> = 0..64;
    let mut partial = 0;
    for seed in sweep(SEEDS) {
        let (violations, acked) = crash_and_check(seed, Durability::Forced, Faults::default());
        assert_eq!(Vec::<Violation>::new(), violations, "seed {seed}");
        partial += usize::from(acked > 0 && acked < 8);
    }
    assert!(
        partial > share(SEEDS, 16),
        "only {partial} crashes fell inside the workload"
    );
}

#[test]
fn clean_run_reports_exactly_the_acknowledged_commits() {
    let mut sim = sim(1, Faults::default());
    let ledger = Ledger::new();
    let store = run(&mut sim, |rt, _| {
        commit_all(rt, &ledger, &[0, 1, 2, 3], Durability::Forced);
        recover(rt)
    });
    assert_eq!(
        (0..4).collect::<std::collections::BTreeSet<u64>>(),
        store.ids
    );
    assert!(check(&ledger, &store).is_empty());
}

#[test]
fn acknowledging_before_the_force_is_caught() {
    let caught = (0..64).any(|seed| {
        crash_and_check(seed, Durability::Unforced, Faults::default())
            .0
            .iter()
            .any(|v| matches!(v, Violation::Lost(_)))
    });
    assert!(caught, "no seed lost an unforced acknowledged commit");
}

#[test]
fn reporting_an_aborted_commit_is_caught() {
    let mut sim = sim(2, Faults::default());
    let ledger = Ledger::new();
    let store = run(&mut sim, |rt, _| {
        commit_all(rt, &ledger, &[0, 1], Durability::AbortAfterWrite);
        recover(rt)
    });
    assert_eq!(
        vec![Violation::Phantom(0), Violation::Phantom(1)],
        check(&ledger, &store)
    );
}

#[test]
fn reporting_a_never_submitted_commit_is_caught() {
    let mut sim = sim(3, Faults::default());
    let ledger = Ledger::new();
    let store = run(&mut sim, |rt, _| {
        commit_all(rt, &Ledger::new(), &[5], Durability::Forced);
        recover(rt)
    });
    assert_eq!(vec![Violation::Phantom(5)], check(&ledger, &store));
}

#[test]
fn rotted_record_is_a_structure_finding() {
    let mut sim = sim(4, Faults::default());
    let ledger = Ledger::new();
    run(&mut sim, |rt, _| {
        commit_all(rt, &ledger, &[0, 1, 2], Durability::Forced);
    });
    sim.set_faults(Faults {
        corrupt_p: 1.0,
        ..Faults::default()
    });
    let store = run(&mut sim, |rt, _| recover(rt));
    let violations = check(&ledger, &store);
    assert!(
        violations
            .iter()
            .any(|v| matches!(v, Violation::Found(f) if f.kind == FindingKind::Structure)),
        "{violations:?}"
    );
    assert!(
        violations.iter().any(|v| matches!(v, Violation::Lost(_))),
        "{violations:?}"
    );
}

#[test]
fn in_doubt_commits_may_go_either_way() {
    let (mut present, mut absent) = (false, false);
    for seed in 0..64 {
        let mut sim = sim(seed, Faults::default());
        let ledger = Ledger::new();
        sim.crash_at(sim.now() + Duration::from_millis(5));
        let l = ledger.clone();
        sim.run(move |rt| {
            let mut io = adtb::runtime::Runtime::io(&rt);
            let (file, _) = common::open(&rt, &mut io, "log", true).unwrap();
            common::sync_dir(&rt, &mut io, "").unwrap();
            l.submit(0);
            common::write(&rt, &mut io, file, 0, &common::record(0)).unwrap();
            loop {
                sleep(&rt, &mut io, Duration::from_millis(1));
            }
        });
        let store = run(&mut sim, |rt, _| recover(rt));
        assert!(check(&ledger, &store).is_empty(), "seed {seed}");
        if store.ids.contains(&0) {
            present = true;
        } else {
            absent = true;
        }
    }
    assert!(present && absent);
}
