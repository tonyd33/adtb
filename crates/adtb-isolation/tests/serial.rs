mod common;

use adtb_isolation::{Outcome, Step, Txn, check, run};
use common::{Memory, Mode, Rng};

const KEYS: u64 = 4;

fn serial_script(rng: &mut Rng) -> Vec<Step> {
    let mut next_elem = [0; KEYS as usize];
    let mut steps = Vec::new();
    for t in 0..=rng.below(12) {
        steps.push(Step::Begin(t));
        for _ in 0..=rng.below(6) {
            let key = rng.below(KEYS);
            if rng.below(2) == 0 {
                next_elem[key as usize] += 1;
                steps.push(Step::Append(t, key as i64, next_elem[key as usize]));
            } else {
                steps.push(Step::Read(t, key as i64));
            }
        }
        steps.push(if rng.below(8) == 0 {
            Step::Abort(t)
        } else {
            Step::Commit(t)
        });
    }
    steps.push(Step::Begin(u64::MAX));
    for key in 0..KEYS {
        steps.push(Step::Read(u64::MAX, key as i64));
    }
    steps.push(Step::Commit(u64::MAX));
    steps
}

fn obscure(history: &mut [Txn], rng: &mut Rng) {
    for txn in history.iter_mut() {
        if rng.below(5) == 0 {
            txn.outcome = Outcome::InDoubt;
        }
    }
    for i in (1..history.len()).rev() {
        history.swap(i, rng.below(i as u64 + 1) as usize);
    }
}

#[test]
fn serial_histories_pass() {
    for seed in 0..2000 {
        let mut rng = Rng(seed);
        let steps = serial_script(&mut rng);
        for mode in [Mode::Optimistic, Mode::Snapshot, Mode::ReadCommitted] {
            let mut history = run(&mut Memory::new(mode), &steps);
            assert_eq!(check(&history), Ok(vec![]), "seed {seed}, {mode:?}");
            obscure(&mut history, &mut rng);
            assert_eq!(
                check(&history),
                Ok(vec![]),
                "seed {seed}, {mode:?}, obscured"
            );
        }
    }
}

fn interleaved_script(rng: &mut Rng) -> Vec<Step> {
    let mut next_elem = [0; KEYS as usize];
    let mut pending: Vec<Vec<Step>> = (0..2 + rng.below(6))
        .map(|t| {
            let mut ops = vec![Step::Begin(t)];
            for _ in 0..=rng.below(5) {
                let key = rng.below(KEYS);
                ops.push(if rng.below(2) == 0 {
                    next_elem[key as usize] += 1;
                    Step::Append(t, key as i64, next_elem[key as usize])
                } else {
                    Step::Read(t, key as i64)
                });
            }
            ops.push(Step::Commit(t));
            ops.reverse();
            ops
        })
        .collect();
    let mut steps = Vec::new();
    while !pending.is_empty() {
        let i = rng.below(pending.len() as u64) as usize;
        steps.push(pending[i].pop().expect("non-empty"));
        if pending[i].is_empty() {
            pending.swap_remove(i);
        }
    }
    steps.push(Step::Begin(u64::MAX));
    for key in 0..KEYS {
        steps.push(Step::Read(u64::MAX, key as i64));
    }
    steps.push(Step::Commit(u64::MAX));
    steps
}

#[test]
fn interleavings_under_a_serializable_database_pass() {
    for seed in 0..2000 {
        let mut rng = Rng(seed);
        let steps = interleaved_script(&mut rng);
        let history = run(&mut Memory::new(Mode::Optimistic), &steps);
        assert_eq!(check(&history), Ok(vec![]), "seed {seed}");
    }
}

#[test]
fn interleavings_under_snapshot_isolation_show_only_g2() {
    for seed in 0..2000 {
        let mut rng = Rng(seed);
        let steps = interleaved_script(&mut rng);
        let history = run(&mut Memory::new(Mode::Snapshot), &steps);
        let found = common::kinds(&history);
        assert!(found.iter().all(|k| *k == "G2"), "seed {seed}: {found:?}");
    }
}
