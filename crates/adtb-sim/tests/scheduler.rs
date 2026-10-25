mod common;

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use adtb::runtime::{Clock, Entropy, Instant, Runtime, Threads};
use adtb_sim::{Config, Outcome, Sim, SimRuntime};
use common::{run, sleep};

fn interleaving(seed: u64) -> Vec<(usize, usize)> {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut sim = Sim::new(seed, Config::default());
    let shared = log.clone();
    let outcome = sim.run(move |rt| {
        let workers: Vec<_> = (0..4)
            .map(|w| {
                let (rt2, log) = (rt.clone(), shared.clone());
                rt.spawn("worker", move || {
                    for step in 0..5 {
                        log.lock().unwrap().push((w, step));
                        rt2.yield_now();
                    }
                })
            })
            .collect();
        for t in workers {
            rt.join(t);
        }
    });
    assert_eq!(Outcome::Finished, outcome);
    Arc::try_unwrap(log).unwrap().into_inner().unwrap()
}

#[test]
fn same_seed_same_interleaving() {
    assert_eq!(interleaving(11), interleaving(11));
}

#[test]
fn seeds_explore_interleavings() {
    let first = interleaving(0);
    assert!((1..16).any(|seed| interleaving(seed) != first));
}

#[test]
fn every_thread_runs_to_completion() {
    let mut log = interleaving(3);
    assert_eq!(20, log.len());
    log.sort();
    let expected: Vec<_> = (0..4).flat_map(|w| (0..5).map(move |s| (w, s))).collect();
    assert_eq!(expected, log);
}

#[test]
fn unpark_wakes_a_parked_thread() {
    let mut sim = Sim::new(1, Config::default());
    let flag = Arc::new(Mutex::new(false));
    let seen = flag.clone();
    sim.run(move |rt| {
        let main = rt.current();
        let rt2 = rt.clone();
        let child = rt.spawn("setter", move || {
            *seen.lock().unwrap() = true;
            rt2.unpark(main);
        });
        while !*flag.lock().unwrap() {
            rt.park();
        }
        rt.join(child);
    });
}

#[test]
fn unpark_before_park_is_not_lost() {
    let mut sim = Sim::new(2, Config::default());
    sim.run(|rt| {
        rt.unpark(rt.current());
        rt.park();
    });
}

#[test]
#[should_panic(expected = "deadlock")]
fn parking_with_nothing_to_wake_is_a_deadlock() {
    let mut sim = Sim::new(3, Config::default());
    sim.run(|rt| rt.park());
}

#[test]
#[should_panic(expected = "deadlock")]
fn a_thread_left_parked_when_main_returns_is_a_deadlock() {
    let mut sim = Sim::new(3, Config::default());
    sim.run(|rt| {
        let rt2 = rt.clone();
        rt.spawn("stuck", move || rt2.park());
    });
}

#[test]
#[should_panic(expected = "boom")]
fn a_panic_in_any_thread_fails_the_run() {
    let mut sim = Sim::new(4, Config::default());
    sim.run(|rt| {
        let t = rt.spawn("bad", || panic!("boom"));
        rt.join(t);
    });
}

#[test]
fn timer_fires_at_its_time() {
    let mut sim = Sim::new(5, Config::default());
    run(&mut sim, |rt, io| {
        let before = rt.now();
        sleep(rt, io, Duration::from_millis(5));
        assert!(rt.now() >= before + Duration::from_millis(5));
    });
}

#[test]
fn clock_never_goes_back_across_runs() {
    let mut sim = Sim::new(6, Config::default());
    let mut last = Instant::from_nanos(0);
    for _ in 0..3 {
        last = run(&mut sim, |rt, io| {
            assert!(rt.now() >= last);
            sleep(rt, io, Duration::from_millis(1));
            rt.now()
        });
    }
}

#[test]
fn wall_clock_follows_the_monotonic_clock() {
    let mut sim = Sim::new(7, Config::default());
    run(&mut sim, |rt, io| {
        let (m0, w0) = (rt.now(), rt.wall());
        sleep(rt, io, Duration::from_secs(1));
        let (m1, w1) = (rt.now(), rt.wall());
        assert_eq!(m1.saturating_since(m0), w1.duration_since(w0).unwrap());
        assert!(w0 > SystemTime::UNIX_EPOCH);
    });
}

fn draws(seed: u64) -> Vec<u64> {
    let mut sim = Sim::new(seed, Config::default());
    run(&mut sim, |rt: &SimRuntime, _| {
        (0..4).map(|_| rt.next_u64()).collect()
    })
}

#[test]
fn entropy_is_seeded() {
    assert_eq!(draws(8), draws(8));
    assert_ne!(draws(8), draws(9));
}

#[test]
fn fill_uses_the_seeded_stream() {
    let mut sim = Sim::new(10, Config::default());
    let bytes = run(&mut sim, |rt, _| {
        let mut b = [0u8; 12];
        rt.fill(&mut b);
        b
    });
    assert_ne!([0u8; 12], bytes);
}

#[test]
fn crash_stops_every_thread_and_the_next_run_starts_clean() {
    let mut sim = Sim::new(12, Config::default());
    sim.crash_at(sim.now() + Duration::from_millis(1));
    let outcome = sim.run(|rt| {
        for _ in 0..3 {
            let rt2 = rt.clone();
            rt.spawn("spinner", move || {
                let mut io = rt2.io();
                loop {
                    sleep(&rt2, &mut io, Duration::from_micros(10));
                }
            });
        }
        let mut io = rt.io();
        loop {
            sleep(&rt, &mut io, Duration::from_micros(10));
        }
    });
    assert_eq!(Outcome::Crashed, outcome);
    run(&mut sim, |rt, io| sleep(rt, io, Duration::from_millis(1)));
}
