//! The conformance suite on the engine, over the simulator. A file the engine fails must be on
//! `engine-failing.txt`, and a file on it must fail.
//!
//! Each file runs in a child process of this test binary, so a file that overflows the stack,
//! aborts or does not finish fails alone.

use std::collections::BTreeMap;
use std::io::Read;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use adtb_conformance::{panic_message, parse, run_engine, scripts};
use adtb_sim::{Config, Outcome, Sim};

const SEED: u64 = 0;

/// The script a child process runs, when set.
const CHILD: &str = "ADTB_CONFORMANCE_ENGINE_FILE";

const DEADLINE: Duration = Duration::from_secs(180);
const POLL: Duration = Duration::from_millis(20);

/// A child prints each difference after this, and this harness reads from it to the end of the
/// line: the test harness may print its own text before it on the line.
const DIFF: &str = "adtb-conformance-difference: ";
const DONE: &str = "adtb-conformance-done";

fn suite() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance")
}

/// Returns the reason of each file on the list, by its path under the suite.
fn failing() -> BTreeMap<String, String> {
    include_str!("engine-failing.txt")
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|l| {
            let (path, reason) = l.split_once(' ').unwrap_or((l, ""));
            (path.to_owned(), reason.trim().to_owned())
        })
        .collect()
}

/// Returns each difference of the script at `path` on the engine.
fn differences(path: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let script = match parse(&text) {
        Ok(s) => s,
        Err(d) => return vec![d.to_string()],
    };
    let mut found = Vec::new();
    let ran = panic::catch_unwind(AssertUnwindSafe(|| {
        Sim::new(SEED, Config::default()).run(|rt| found = run_engine(rt, &script))
    }));
    match ran {
        Ok(Outcome::Finished) => found.iter().map(ToString::to_string).collect(),
        Ok(Outcome::Crashed) => vec!["the simulated process crashes".into()],
        Err(p) => vec![format!("the engine panics: {}", panic_message(&*p))],
    }
}

/// Run the script `CHILD` names, printing each difference on a line and then `DONE`. Does
/// nothing in a run that is not a child's.
#[test]
fn child() {
    let Ok(path) = std::env::var(CHILD) else {
        return;
    };
    for d in differences(Path::new(&path)) {
        println!("{DIFF}{}", d.replace('\n', "\\n"));
    }
    println!("{DONE}");
}

/// Returns each difference of the script at `path`, run in a child process.
fn in_child(path: &Path) -> Vec<String> {
    let mut child = Command::new(std::env::current_exe().expect("the test binary has a path"))
        .args(["--exact", "child", "--nocapture", "--test-threads", "1"])
        .env(CHILD, path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the child starts");
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let mut stderr = child.stderr.take().expect("stderr is piped");
    let drain = |r: &mut dyn Read| {
        let mut text = String::new();
        r.read_to_string(&mut text).expect("a pipe reads");
        text
    };
    let (status, out, err) = thread::scope(|s| {
        let out = s.spawn(|| drain(&mut stdout));
        let err = s.spawn(|| drain(&mut stderr));
        let mut status = None;
        for _ in 0..DEADLINE.as_millis() / POLL.as_millis() {
            status = child.try_wait().expect("the child is waited on");
            if status.is_some() {
                break;
            }
            thread::sleep(POLL);
        }
        if status.is_none() {
            child.kill().expect("the child is killed");
            child.wait().expect("the killed child is reaped");
        }
        let joined = |h: thread::ScopedJoinHandle<'_, String>| {
            h.join().expect("a pipe reader does not panic")
        };
        (status, joined(out), joined(err))
    });
    let Some(status) = status else {
        return vec![format!("does not finish within {DEADLINE:?}")];
    };
    if !out.contains(DONE) {
        let tail: Vec<&str> = err.lines().filter(|l| !l.trim().is_empty()).collect();
        let tail = tail[tail.len().saturating_sub(3)..].join(" / ");
        return vec![format!("the engine aborts ({status}): {tail}")];
    }
    out.lines()
        .filter_map(|l| l.split_once(DIFF).map(|(_, d)| d))
        .map(ToOwned::to_owned)
        .collect()
}

/// Returns each file of `part` with its differences, run in child processes in parallel.
fn run_part(part: &str) -> Vec<(String, Vec<String>)> {
    let dir = suite().join(part);
    let files = scripts(&dir);
    assert!(!files.is_empty(), "no scripts in {}", dir.display());
    let next = Mutex::new(files.iter());
    let done = Mutex::new(Vec::new());
    let workers = thread::available_parallelism().map_or(4, |n| n.get().min(8));
    thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                loop {
                    let Some(file) = next.lock().expect("the queue locks").next() else {
                        break;
                    };
                    let name = file
                        .strip_prefix(suite())
                        .expect("a script lies under the suite")
                        .display()
                        .to_string();
                    let found = in_child(file);
                    done.lock().expect("the results lock").push((name, found));
                }
            });
        }
    });
    let mut done = done.into_inner().expect("the results lock");
    done.sort();
    done
}

fn conforms(part: &str) {
    let listed: BTreeMap<String, String> = failing()
        .into_iter()
        .filter(|(p, _)| p.starts_with(&format!("{part}/")))
        .collect();
    let ran = run_part(part);
    let mut wrong = Vec::new();
    for (name, found) in &ran {
        match (listed.contains_key(name), found.is_empty()) {
            (false, false) => wrong.push(format!(
                "{name} fails on the engine:\n  {}",
                found.join("\n  ")
            )),
            (true, true) => wrong.push(format!(
                "{name} passes on the engine; take it off engine-failing.txt"
            )),
            _ => {}
        }
    }
    wrong.extend(
        listed
            .keys()
            .filter(|p| !ran.iter().any(|(name, _)| name == *p))
            .map(|p| format!("{p} is on engine-failing.txt but is not in the suite")),
    );
    assert!(wrong.is_empty(), "seed {SEED}:\n{}", wrong.join("\n"));
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
