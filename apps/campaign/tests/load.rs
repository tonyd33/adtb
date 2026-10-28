#[path = "load/mod.rs"]
mod load;

use adtb_sim::{Config, Outcome, Sim};
use adtb_testkit::profile::sweep;
use load::fixture::{Files, Fixture};
use std::panic::{AssertUnwindSafe, catch_unwind};

const DEFAULT_SEEDS: u64 = 1;

struct Run {
    files: Files,
    ledger: String,
    trace: String,
}

fn run(files: Files, fixture: &Fixture, seed: u64, pass: usize) -> Run {
    let mut sim = Sim::with_files(
        seed,
        Config {
            switch_rate: 0.3,
            ..Config::default()
        },
        files,
    );
    let ledger = load::Ledger::default();
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        assert_eq!(
            sim.run(|rt| load::actors::run(rt, fixture, &ledger, seed)),
            Outcome::Finished,
            "seed={seed}"
        );
        assert_eq!(
            sim.run(|rt| load::checks::reopen(rt, fixture, &ledger, seed)),
            Outcome::Finished,
            "seed={seed}"
        );
    }));
    let run_id = std::env::var("NEXTEST_RUN_ID").unwrap_or_else(|_| "cargo-test".into());
    let output = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("campaign-load")
        .join(run_id);
    std::fs::create_dir_all(&output).unwrap();
    let ledger = ledger.report();
    let trace = sim.trace();
    std::fs::write(
        output.join(format!("seed-{seed}-pass-{pass}.ledger")),
        &ledger,
    )
    .unwrap();
    std::fs::write(
        output.join(format!("seed-{seed}-pass-{pass}.trace")),
        &trace,
    )
    .unwrap();
    if let Err(panic) = outcome {
        let message = panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&str>().copied())
            .unwrap_or("non-string panic");
        panic!("seed={seed}: {message}; evidence {}", output.display());
    }
    Run {
        files: sim.files(),
        ledger,
        trace,
    }
}

#[test]
fn simulated_campaign_collisions_preserve_committed_state() {
    let (files, fixture) = load::fixture::population();
    for seed in sweep(4..4 + DEFAULT_SEEDS) {
        let first = run(files.clone(), &fixture, seed, 0);
        let replay = run(files.clone(), &fixture, seed, 1);
        assert_eq!(
            first.ledger, replay.ledger,
            "seed={seed}: operation ledger replay"
        );
        assert!(
            first.trace == replay.trace,
            "seed={seed}: scheduling trace replay"
        );
        assert!(
            first.files == replay.files,
            "seed={seed}: database byte replay"
        );
    }
}
