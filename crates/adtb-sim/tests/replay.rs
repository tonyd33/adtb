mod common;

use std::collections::BTreeMap;
use std::time::Duration;

use adtb::runtime::{Entropy, Runtime, Threads};
use adtb_sim::{Config, Faults, Outcome, Sim, SimRuntime};
use common::{SECTOR, open, read, record, sleep, sync, sync_dir, write};

fn faults() -> Faults {
    Faults {
        torn_p: 0.5,
        corrupt_p: 0.02,
        misdirect_p: 0.02,
        lost_write_p: 0.02,
        latent_p: 0.02,
        fail_p: 0.02,
        lose_unsynced: true,
        delay_completions: true,
    }
}

fn worker(rt: &SimRuntime, w: u64) {
    let mut io = rt.io();
    let Ok((file, _)) = open(rt, &mut io, "data", true) else {
        return;
    };
    sync_dir(rt, &mut io, "").ok();
    for i in 0..10 {
        let slot = w * 16 + i;
        let mut rec = record(slot);
        rec.extend(record(slot + 1000));
        write(rt, &mut io, file, slot * 2 * SECTOR as u64, &rec).ok();
        if rt.next_u64().is_multiple_of(2) {
            sync(rt, &mut io, file).ok();
        }
        let other = rt.next_u64() % 64;
        read(rt, &mut io, file, other * 2 * SECTOR as u64, 2 * SECTOR).ok();
        sleep(rt, &mut io, Duration::from_micros(rt.next_u64() % 200));
    }
}

/// Returns the trace and final files of a run of four writers crashed partway, then a restart that reads everything.
fn scenario(seed: u64) -> (String, BTreeMap<String, Vec<u8>>) {
    let mut sim = Sim::new(
        seed,
        Config {
            faults: faults(),
            ..Config::default()
        },
    );
    sim.crash_at(sim.now() + Duration::from_millis(2));
    let outcome = sim.run(|rt| {
        let threads: Vec<_> = (0..4)
            .map(|w| {
                let rt2 = rt.clone();
                rt.spawn("writer", move || worker(&rt2, w))
            })
            .collect();
        for t in threads {
            rt.join(t);
        }
    });
    assert_eq!(
        Outcome::Crashed,
        outcome,
        "the workload outlasts the crash time"
    );
    let outcome = sim.run(|rt| {
        let mut io = rt.io();
        if let Ok((file, len)) = open(&rt, &mut io, "data", false) {
            for offset in (0..len).step_by(SECTOR) {
                read(&rt, &mut io, file, offset, SECTOR).ok();
            }
        }
    });
    assert_eq!(Outcome::Finished, outcome);
    (sim.trace(), sim.files())
}

#[test]
fn same_seed_replays_byte_for_byte() {
    for seed in [0, 1, 42, 0xdead_beef] {
        let (trace_a, files_a) = scenario(seed);
        let (trace_b, files_b) = scenario(seed);
        assert_eq!(trace_a.as_bytes(), trace_b.as_bytes(), "seed {seed}");
        assert_eq!(files_a, files_b, "seed {seed}");
    }
}

#[test]
fn trace_records_faults_and_the_crash() {
    let traces: Vec<String> = (0..8).map(|seed| scenario(seed).0).collect();
    for needle in [
        "crash\n",
        "fault ",
        "spawn ",
        "submit ",
        "complete ",
        " run ",
    ] {
        assert!(
            traces.iter().any(|t| t.contains(needle)),
            "no trace mentions {needle:?}"
        );
    }
}

#[test]
fn different_seeds_differ() {
    assert_ne!(scenario(1).0, scenario(2).0);
}
