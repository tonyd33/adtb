//! The conformance suite on the engine, over the simulator.
//!
//! Each file runs in a process of the `adtb-conformance` binary, so a file that overflows the
//! stack, aborts or does not finish fails alone.

use std::path::{Path, PathBuf};
use std::thread;

use adtb_conformance::runner::{self, Corpus, Format, Options, Sides};

/// How many files run at once.
const MOST_WORKERS: usize = 8;

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
