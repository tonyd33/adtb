//! The conformance suite on the engine, over the simulator.
//!
//! Each file runs in a process of the `adtb-conformance` binary, so a file that overflows the
//! stack, aborts or does not finish fails alone.

use std::path::{Path, PathBuf};
use std::thread;

use adtb_conformance::runner::{self, Corpus, Format, Options, Sides};
use adtb_lang::canon::canonical_term;
use adtb_lang::evolve::Source;
use adtb_lang::parse::parse_migration;
use adtb_lang::span::{FileId, Span};
use adtb_testkit::fuzz::{Kind, case, spec_corpus};
use adtb_testkit::profile::{self, assert_reached};
use adtb_testkit::running;
use adtb_testkit::spec::blocks;

/// How many files run at once.
const MOST_WORKERS: usize = 8;

/// The first fuzz seed and the number of fuzz cases drawn, a quarter of them schemas.
const FUZZ_SEED: u64 = 0x5eed_0000;
const FUZZ_CASES: u64 = 400;

fn suite() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance")
}

fn conforms(part: &str) {
    let corpus = Corpus::new(Format::Conformance, suite());
    let dir = suite().join(part);
    let jobs = thread::available_parallelism().map_or(4, |n| n.get().min(MOST_WORKERS));
    let program = PathBuf::from(env!("CARGO_BIN_EXE_adtb-conformance"));
    let options = Options::new(Sides::Engine, jobs).isolated(program);
    let files = runner::select(&corpus, &dir).cases().len();
    let passed = runner::passes(&corpus, &dir, &options).unwrap_or_else(|out| panic!("{out}"));
    assert_eq!(files, passed, "{part}");
}

#[test]
fn model() {
    conforms("model");
}

#[test]
fn queries() {
    conforms("queries");
}

#[test]
fn writes() {
    conforms("writes");
}

#[test]
fn constraints() {
    conforms("constraints");
}

#[test]
fn conformance() {
    conforms("conformance");
}

#[test]
fn interop() {
    conforms("interop");
}

/// Returns whether each migration of schema `body` is accepted, recording it as the engine does.
///
/// Fail when an accepted operator does not resolve, or a declaration of the source the
/// migrations leave writes a name the source does not resolve.
fn spelled(body: &str) -> Result<bool, String> {
    let mut source = Source::default();
    for text in adtb_conformance::migrations(body) {
        let Ok(m) = parse_migration(&text, adtb_lang::parse::Options::default()).ok() else {
            return Ok(false);
        };
        let Ok((next, _, recorded)) = source.record(&m, &text, true) else {
            return Ok(false);
        };
        if let Err(ds) = recorded.resolved() {
            return Err(format!("the operators do not resolve: {ds:?}"));
        }
        source = next;
    }
    let names = source.names().map_err(|ds| format!("{ds:?}"))?;
    for (i, names) in names.iter().enumerate() {
        let text = source.file_text(FileId(i as u32));
        canonical_term(&text, Span::new(0, text.len()), names).map_err(|u| u.to_string())?;
    }
    Ok(true)
}

#[test]
fn every_name_an_accepted_schema_writes_resolves() {
    let mut bodies: Vec<(String, String)> = Vec::new();
    let blocks = blocks();
    let base = running::version_12(&blocks);
    let mut schemas = vec![("version 12".to_string(), base.clone())];
    schemas.extend(
        running::parts(&blocks, &base)
            .into_iter()
            .map(|(part, entries)| (part.to_string(), entries)),
    );
    for (name, entries) in schemas {
        let mut modules: Vec<&str> = Vec::new();
        for e in &entries {
            if !modules.contains(&e.module.as_str()) {
                modules.push(&e.module);
            }
        }
        let mut body = String::new();
        for m in modules {
            body.push_str(&format!("module {m}\n"));
            for e in entries.iter().filter(|e| e.module == m) {
                body.push_str(&e.text);
                body.push('\n');
            }
        }
        bodies.push((format!("spec {name}"), body));
    }
    let corpus = spec_corpus(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/spec"));
    for seed in FUZZ_SEED..FUZZ_SEED + profile::deep(FUZZ_CASES) {
        let c = case(seed, &corpus);
        if c.kind == Kind::Schema {
            bodies.push((format!("fuzz seed {seed}"), c.text));
        }
    }
    let jobs = thread::available_parallelism().map_or(4, |n| n.get().min(MOST_WORKERS));
    let results: Vec<Result<bool, String>> = thread::scope(|s| {
        let chunks: Vec<_> = bodies
            .chunks(bodies.len().div_ceil(jobs))
            .map(|chunk| s.spawn(|| chunk.iter().map(|(_, b)| spelled(b)).collect::<Vec<_>>()))
            .collect();
        chunks
            .into_iter()
            .flat_map(|h| h.join().expect("a worker finishes"))
            .collect()
    });
    let (mut spec, mut fuzz) = (0, 0);
    let mut failures = Vec::new();
    for ((name, _), result) in bodies.iter().zip(&results) {
        match result {
            Ok(true) if name.starts_with("spec ") => spec += 1,
            Ok(true) => fuzz += 1,
            Ok(false) => {}
            Err(why) => failures.push(format!("{name}: {why}")),
        }
    }
    assert!(
        failures.is_empty(),
        "{} schemas fail:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert_reached("accepted spec schemas", spec, 8);
    assert_reached("accepted fuzz schemas", fuzz, 30);
}
