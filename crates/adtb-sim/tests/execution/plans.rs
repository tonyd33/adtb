use std::sync::{Arc, Mutex};
use std::time::Duration;

use adtb::runtime::{AlignedBuf, Clock, Done, Faults, Io, IoError, Op, OpKind, Runtime, Threads};
use adtb_harness::common::{self, complete, open, run, sector, sleep, write};
use adtb_sim::{Action, Config, Match, Outcome, Plan, Planned, Sim};
use adtb_testkit::profile::assert_reached;

fn held_writes() -> Plan {
    Plan::new().rule(Match::kind(OpKind::Write), Action::Hold)
}

#[test]
fn the_clock_skips_a_held_op_to_the_next_timer_and_the_crash() {
    let mut sim = Sim::new(1, Config::default());
    let crash = sim.now() + Duration::from_secs(1);
    sim.crash_at(crash);
    let mut woke = None;
    let outcome = sim.run(|rt| {
        let mut io = rt.io();
        let (file, _) = open(&rt, &mut io, "f", true).unwrap();
        rt.plan(held_writes());
        let start = rt.now();
        let writer = rt.clone();
        rt.spawn("writer", move || {
            let mut io = writer.io();
            let held = write(&writer, &mut io, file, 0, &sector(1));
            unreachable!("a held write completed as {held:?}");
        });
        sleep(&rt, &mut io, Duration::from_millis(10));
        woke = Some(rt.now().saturating_since(start));
        let held = write(&rt, &mut io, file, 512, &sector(2));
        unreachable!("a held write completed as {held:?}");
    });
    assert_eq!(Outcome::Crashed, outcome);
    let woke = woke.expect("the timer fires with a write held");
    assert!(
        woke >= Duration::from_millis(10) && woke < Duration::from_millis(11),
        "woke after {woke:?}"
    );
    assert_eq!(crash, sim.now());
}

#[test]
#[should_panic(expected = "the run ends with an op held: Write \"f\"")]
fn a_run_that_returns_with_an_op_held_fails() {
    let mut sim = Sim::new(2, Config::default());
    sim.run(|rt| {
        let mut io = rt.io();
        let (file, _) = open(&rt, &mut io, "f", true).unwrap();
        rt.plan(held_writes());
        io.submit(Op::Write {
            file,
            offset: 0,
            buf: AlignedBuf::from_slice(&sector(1)),
        });
    });
}

#[test]
#[should_panic(expected = "the run ends with an op held: Write \"f\"")]
fn a_run_blocked_on_a_held_op_with_no_crash_fails() {
    let mut sim = Sim::new(2, Config::default());
    sim.run(|rt| {
        let mut io = rt.io();
        let (file, _) = open(&rt, &mut io, "f", true).unwrap();
        rt.plan(held_writes());
        let held = write(&rt, &mut io, file, 0, &sector(1));
        unreachable!("a held write completed as {held:?}");
    });
}

/// A completed write lands whole at a crash when unforced writes survive; a held one lands by
/// the in-flight coin, so some seed loses it.
#[test]
fn a_crash_takes_a_held_write_as_in_flight() {
    let config = Config {
        faults: Faults {
            lose_unsynced: false,
            ..Faults::none(512)
        },
        ..Config::default()
    };
    let (mut landed, mut lost) = (0, 0);
    for seed in 0..16 {
        let mut sim = Sim::new(seed, config.clone());
        common::run_then_crash(&mut sim, |rt, io| {
            let (file, _) = open(rt, io, "f", true).unwrap();
            write(rt, io, file, 0, &sector(1)).unwrap();
            rt.plan(held_writes());
            io.submit(Op::Write {
                file,
                offset: 0,
                buf: AlignedBuf::from_slice(&sector(2)),
            });
        });
        let got = &sim.files()["f"];
        if *got == sector(2) {
            landed += 1;
        } else {
            assert_eq!(sector(1), *got, "seed {seed}");
            lost += 1;
        }
    }
    assert_reached("a held write landing at the crash", landed, 1);
    assert_reached("a held write lost at the crash", lost, 1);
}

#[test]
fn a_numbered_action_counts_across_a_reopen() {
    let mut sim = Sim::new(3, Config::default());
    sim.plan(Plan::new().at(2, Action::Fail(IoError::Full)));
    let first = run(&mut sim, |rt, io| {
        rt.arm(Match::kind(OpKind::Write).path("f"));
        let (file, _) = open(rt, io, "f", true).unwrap();
        [
            write(rt, io, file, 0, &sector(1)),
            write(rt, io, file, 512, &sector(1)),
        ]
    });
    assert_eq!([Ok(()), Ok(())], first);
    let writes = sim.counted(Match::kind(OpKind::Write));
    let second = run(&mut sim, |rt, io| {
        let (file, _) = open(rt, io, "f", false).unwrap();
        let got = [
            write(rt, io, file, 0, &sector(2)),
            write(rt, io, file, 512, &sector(2)),
        ];
        (got, rt.disarm())
    });
    assert_eq!(([Err(IoError::Full), Ok(())], 4), second);
    assert_eq!(2, writes.get());
    let numbered = sim.numbered();
    let struck = numbered.struck().expect("op 2 came up");
    assert_eq!(
        (2, OpKind::Write, Some("f")),
        (struck.at(), struck.kind(), struck.path())
    );
    assert_eq!(vec![OpKind::Write; 4], numbered.kinds());
}

#[test]
fn a_run_action_runs_on_the_submitter_before_its_op_queues() {
    let mut sim = Sim::new(4, Config::default());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let submitter = run(&mut sim, |rt, io| {
        let (file, _) = open(rt, io, "f", true).unwrap();
        let (inner, log) = (rt.clone(), Arc::clone(&seen));
        let run = Action::Run(Box::new(move || {
            let mut io = inner.io();
            sleep(&inner, &mut io, Duration::from_secs(1));
            let size = complete(&inner, &mut io, Op::Size { file }).result;
            log.lock().unwrap().push((inner.current(), size));
        }));
        rt.plan(Plan::new().first(Match::kind(OpKind::Write), run));
        write(rt, io, file, 0, &sector(1)).unwrap();
        write(rt, io, file, 512, &sector(1)).unwrap();
        rt.current()
    });
    assert_eq!(vec![(submitter, Ok(Done::Size(0)))], *seen.lock().unwrap());
    assert_eq!(1024, sim.files()["f"].len());
}

#[test]
fn a_lost_write_completes_as_written_and_changes_nothing() {
    let mut sim = Sim::new(5, Config::default());
    let got = run(&mut sim, |rt, io| {
        let (file, _) = open(rt, io, "f", true).unwrap();
        write(rt, io, file, 0, &sector(1)).unwrap();
        rt.plan(Plan::new().rule(Match::kind(OpKind::Write), Action::Lose));
        write(rt, io, file, 0, &sector(2))
    });
    assert_eq!(Ok(()), got);
    assert_eq!(sector(1), sim.files()["f"]);
}

#[test]
fn counted_ops_are_the_matching_ones_submitted() {
    let mut sim = Sim::new(6, Config::default());
    let counts = run(&mut sim, |rt, io| {
        let reads = rt.counted(Match::kind(OpKind::Read).prefixed("a"));
        let (a, _) = open(rt, io, "a", true).unwrap();
        let (b, _) = open(rt, io, "b", true).unwrap();
        write(rt, io, a, 0, &sector(1)).unwrap();
        for _ in 0..3 {
            common::read(rt, io, a, 0, 512).unwrap();
            common::read(rt, io, b, 0, 512).unwrap();
        }
        (reads.get(), rt.files_open())
    });
    assert_eq!((3, 2), counts);
}

#[test]
fn a_rename_matches_by_its_new_name() {
    let mut sim = Sim::new(7, Config::default());
    let got = run(&mut sim, |rt, io| {
        let (file, _) = open(rt, io, "a", true).unwrap();
        complete(rt, io, Op::Close { file });
        rt.plan(Plan::new().rule(
            Match::kind(OpKind::Rename).path("b"),
            Action::Fail(IoError::Failed),
        ));
        let rename = |to: &str| Op::Rename {
            from: "a".into(),
            to: to.into(),
            replace: false,
        };
        [
            complete(rt, io, rename("b")).result,
            complete(rt, io, rename("c")).result,
        ]
    });
    assert_eq!([Err(IoError::Failed), Ok(Done::Renamed)], got);
}

/// A planned outcome replaces the write, so a crash with it still queued lands nothing of it.
#[test]
fn a_crash_lands_no_planned_write_still_queued() {
    let config = Config {
        latency: (Duration::from_millis(1), Duration::from_millis(1)),
        ..Config::default()
    };
    for seed in 0..16 {
        let mut sim = Sim::new(seed, config.clone());
        run(&mut sim, |rt, io| {
            let (file, _) = open(rt, io, "f", true).unwrap();
            write(rt, io, file, 0, &sector(1)).unwrap();
            complete(rt, io, Op::Sync { file }).result.unwrap();
            let dir = String::new();
            complete(rt, io, Op::SyncDir { dir }).result.unwrap();
        });
        let writes = sim.logged(Match::kind(OpKind::Write));
        sim.crash_at(sim.now() + Duration::from_micros(1500));
        let outcome = sim.run(|rt| {
            let mut io = rt.io();
            let (file, _) = open(&rt, &mut io, "f", false).unwrap();
            rt.plan(
                Plan::new()
                    .first(Match::kind(OpKind::Write), Action::Lose)
                    .rule(Match::kind(OpKind::Write), Action::Fail(IoError::Failed)),
            );
            for (offset, fill) in [(0, 2), (512, 3)] {
                io.submit(Op::Write {
                    file,
                    offset,
                    buf: AlignedBuf::from_slice(&sector(fill)),
                });
            }
            sleep(&rt, &mut io, Duration::from_secs(1));
            unreachable!("the crash comes first");
        });
        assert_eq!(Outcome::Crashed, outcome, "seed {seed}");
        let queued: Vec<_> = writes
            .entries()
            .iter()
            .map(|w| (w.planned(), w.completed()))
            .collect();
        assert_eq!(
            vec![
                (Some(Planned::Lose), None),
                (Some(Planned::Fail(IoError::Failed)), None)
            ],
            queued,
            "seed {seed}"
        );
        assert_eq!(sector(1), sim.files()["f"], "seed {seed}");
    }
}

#[test]
fn a_rule_ends_with_its_run() {
    let mut sim = Sim::new(8, Config::default());
    sim.plan(Plan::new().rule(Match::kind(OpKind::Write), Action::Lose));
    let first = run(&mut sim, |rt, io| {
        let (file, _) = open(rt, io, "f", true).unwrap();
        write(rt, io, file, 0, &sector(1))
    });
    let second = run(&mut sim, |rt, io| {
        let (file, _) = open(rt, io, "f", false).unwrap();
        write(rt, io, file, 0, &sector(2))
    });
    assert_eq!((Ok(()), Ok(())), (first, second));
    assert_eq!(sector(2), sim.files()["f"]);
}

#[test]
#[should_panic(expected = "a failed close leaves its file open")]
fn a_failure_planned_for_any_close_is_refused() {
    drop(Plan::new().rule(
        Match::any().except(Match::kind(OpKind::Timer)),
        Action::Fail(IoError::Failed),
    ));
}

#[test]
#[should_panic(expected = "search owns this simulation's plan: a workload under it does not plan")]
fn a_plan_under_an_owned_plan_is_refused() {
    let mut sim = Sim::new(9, Config::default());
    sim.own_plan("search", Plan::new());
    run(&mut sim, |rt, _| rt.plan(Plan::new()));
}

#[test]
fn a_lasting_rule_acts_after_a_restart() {
    let mut sim = Sim::new(10, Config::default());
    let lost = || Plan::new().rule(Match::kind(OpKind::Write), Action::Lose);
    sim.plan(lost().lasting());
    run(&mut sim, |rt, io| {
        let (file, _) = open(rt, io, "f", true).unwrap();
        write(rt, io, file, 0, &sector(1)).unwrap();
    });
    run(&mut sim, |rt, io| {
        let (file, _) = open(rt, io, "f", false).unwrap();
        write(rt, io, file, 0, &sector(2)).unwrap();
    });
    assert_eq!(Vec::<u8>::new(), sim.files()["f"]);
}

#[test]
#[should_panic(expected = "only a write can be lost")]
fn a_loss_planned_for_any_op_is_refused() {
    drop(Plan::new().rule(Match::any(), Action::Lose));
}
