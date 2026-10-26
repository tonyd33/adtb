use std::collections::BTreeMap;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use adtb_testkit::fuzz::{Case, Front, Kind, Reached, case, spec_corpus};

/// Cases per run unless `ADTB_FUZZ_CASES` says otherwise.
const CASES: u64 = 3000;

/// The first seed unless `ADTB_FUZZ_SEED` says otherwise.
const FIRST_SEED: u64 = 0x5eed_0000;

/// The stack each case runs on, the default of a spawned thread.
const STACK: usize = 2 << 20;

/// A case slower than this is reported as a hang.
const SLOW: Duration = Duration::from_secs(10);

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name).map_or(default, |v| {
        v.parse()
            .unwrap_or_else(|_| panic!("{name} is not a number: {v}"))
    })
}

fn corpus() -> Vec<String> {
    spec_corpus(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/spec"))
}

/// Runs `c` on a thread of its own named after `seed`, so a stack overflow names it too.
#[expect(
    clippy::disallowed_methods,
    reason = "a case that hangs is reported with its seed by the wall clock"
)]
fn run(front: &Front, seed: u64, c: &Case) -> Result<Reached, String> {
    thread::scope(|s| {
        let started = Instant::now();
        let handle = thread::Builder::new()
            .name(format!("fuzz seed {seed}"))
            .stack_size(STACK)
            .spawn_scoped(s, || front.run(c))
            .expect("a fuzz thread starts");
        let reached = handle.join().map_err(|p| {
            p.downcast_ref::<String>()
                .cloned()
                .or_else(|| p.downcast_ref::<&str>().map(ToString::to_string))
                .unwrap_or_else(|| "a panic".to_string())
        })?;
        if started.elapsed() > SLOW {
            return Err(format!("took {:?}", started.elapsed()));
        }
        Ok(reached)
    })
}

#[test]
fn the_front_end_accepts_or_refuses_every_drawn_program() {
    let front = Front::new();
    let corpus = corpus();
    let first = env_u64("ADTB_FUZZ_SEED", FIRST_SEED);
    let cases = env_u64("ADTB_FUZZ_CASES", CASES);
    let mut failures = Vec::new();
    for seed in first..first + cases {
        let c = case(seed, &corpus);
        if let Err(why) = run(&front, seed, &c) {
            failures.push(format!(
                "seed {seed} ({:?}, mutated {}): {why}\n{}",
                c.kind, c.mutated, c.text
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {cases} cases from seed {first} crashed:\n{}",
        failures.len(),
        failures.join("\n---\n")
    );
}

#[test]
fn most_drawn_programs_reach_the_normaliser() {
    let front = Front::new();
    let mut reached: BTreeMap<Kind, Vec<Reached>> = BTreeMap::new();
    for seed in FIRST_SEED..FIRST_SEED + 400 {
        let c = case(seed, &[]);
        if !c.mutated {
            let r = run(&front, seed, &c).unwrap_or_else(|why| panic!("seed {seed}: {why}"));
            reached.entry(c.kind).or_default().push(r);
        }
    }
    for (kind, rs) in reached {
        let normalised = rs
            .iter()
            .filter(|r| matches!(r, Reached::Normalised))
            .count();
        assert!(
            normalised * 10 >= rs.len() * 7,
            "{kind:?}: {normalised} of {} unmutated cases normalise",
            rs.len()
        );
    }
}
