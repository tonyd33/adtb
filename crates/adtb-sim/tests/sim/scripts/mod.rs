//! Engine scripts: the scripts under `scripts/AREA/` run on the simulator, one test per area.
//!
//! A script is read as a conformance script is, with these directives:
//!
//! - `seed N`, first: the simulator's seed, 0 without it.
//! - `use NAME`: the directives of `scripts/fixtures/NAME.fixture`, in place.
//! - `option query_memory_bytes N`: the open option for each later open; `default` clears it.
//! - `migrate [at V] [refused VALUE]`: the body's migrations, each starting at a `module` line,
//!   appended to the log, after its first V - 1 when `at V`, and the file reopened with it;
//!   `at V` with no body reopens the file with the first V - 1.
//! - `reopen [refused VALUE]`: the file reopened with the same log.
//! - `log`: the body's migrations appended to the log the next open applies.
//! - `bundle HEADER [use NAME] [refused VALUE]`: the bundle file of `HEADER` and the body, or
//!   of `scripts/fixtures/NAME.bundle`, registered at the next open; with no HEADER the file
//!   has none. With `refused`, the file is refused with VALUE and not registered. A script opens
//!   the file after each bundle.
//! - `call NAME [ORDINAL [committed] [rowsort]] [error VALUE]`: bind NAME and call ORDINAL with
//!   the value of each body line, a query, as an argument. The rows after `----` are the
//!   elements of a set result in printed order, or the one value of another; `rowsort` compares
//!   them as a multiset. `committed` states that the call is of a transaction and commits, its
//!   rows the transaction's value. With no ORDINAL, the call takes no body and no rows, and the
//!   handle bound is kept: later calls of NAME in the same open go through it.
//! - `advance N(s|m|h|d)`: the simulated clock advanced.
//! - `role NAME`: the role created.
//! - `host NAME...`: host functions supplied at each later open, each of version `1.0`,
//!   deterministic, answering its own name as text.
//! - `grant [error VALUE]`, `revoke [error VALUE]`: each line of the body, one grant or revoke
//!   statement, given in order; only the last may be refused, with VALUE.
//! - `register HEADER [use NAME] [error VALUE]`: the bundle file `bundle` states, registered
//!   now.
//! - `run`, `query [rowsort]`: as in a conformance script.
//! - `command [rowsort]`: the body, a command or other text, one call in process, its one result
//!   compared with the rows as a `query`'s.
//! - `error VALUE`: as in a conformance script, the last statement refused with exactly the
//!   reason VALUE in textual syntax.
//! - `adhoc error VALUE`: the body, one ad hoc call in process committing on its own, refused
//!   with exactly VALUE.
//! - `as ROLE` before `run`, `query`, `error`, `call`, `grant`, `revoke` or `register`: the step
//!   acts through a session of ROLE, a server's own path. A `run`, `query` or `error` body is
//!   then one ad hoc call, refused as a whole, and a `call` binds through the session. Without
//!   it, `grant`, `revoke` and `register` act as `owner` and the others in process.
//! - `failed VALUE`, after `as ROLE`: the body is admitted and fails at completion with VALUE;
//!   `error` takes only a refusal at the submit.
//! - `drop role NAME [error VALUE]`: the role dropped.
//! - `loose NAME`: the body, registration NAME with no bundle file, registered at the next open.
//! - `begin [read only] [deferrable] NAME [error VALUE]`, `commit NAME [position P] [error
//!   VALUE]`, `rollback NAME`: interactive transaction NAME; one open at the next open differs.
//!   `$P` in a later body or compared value stands for the position the commit took, in textual
//!   syntax; a second binding of P differs.
//! - `in NAME` before `run`, `query`, `error` or a `call` with an ordinal: one call of
//!   transaction NAME; `committed` states a registered transaction committing with it.
//! - `assert VALUE` before `run`, `query`, `error`, `call` or `begin`, after any `as`: the call
//!   or transaction runs for principal VALUE. With `in` or `assert` a body is one call.
//!
//! `pins N` with no hash in a header or body gains the hash of the first N migrations of the log:
//! of that open's in a bundle file, else of the script's, each `log` step's included.
//!
//! An open refused with VALUE, the open refusal in textual syntax, leaves the file closed. A
//! refusal `MigrationRefused` of version V leaves the log at its first V - 1 migrations. Any
//! other refusal leaves the log as before the step, the migrations of each `log` step since
//! the last open included. Each open is a new simulated process. No open runs the background
//! vacuum. A script holds a directive besides `seed` and `use`; an area holds only scripts and
//! directories.
//!
//! Set `ADTB_SCRIPTS` to a newline-separated list of script files and directories and run
//! `scripts::by_hand` to run them; `work/tools/script` does that.

mod read;
mod role;
mod session;
mod txn;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use adtb::OpenError;

use session::{open_value, run_file};

const BY_HAND: &str = "ADTB_SCRIPTS";

/// How many scripts run at once: the slots each area test holds.
const WORKERS: usize = 3;

fn crate_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn root() -> PathBuf {
    crate_dir().join("scripts")
}

/// Returns `path` relative to the crate when it lies under it.
fn shown(path: &Path) -> String {
    path.strip_prefix(crate_dir())
        .unwrap_or(path)
        .display()
        .to_string()
}

/// Add each script under `dir` to `files` and each entry that is no script to `problems`.
fn collect(dir: &Path, files: &mut Vec<PathBuf>, problems: &mut Vec<String>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => return problems.push(format!("{}: {e}", shown(dir))),
    };
    for entry in entries {
        let path = match entry {
            Ok(entry) => entry.path(),
            Err(e) => {
                problems.push(format!("{}: {e}", shown(dir)));
                continue;
            }
        };
        if path.is_dir() {
            collect(&path, files, problems);
        } else if path.extension().is_some_and(|x| x == "test") {
            files.push(path);
        } else {
            problems.push(format!("{}: not a `.test` script", shown(&path)));
        }
    }
}

/// Run every script under each of `paths`. Returns each difference and each entry that is no
/// script, shown.
fn differences(paths: &[PathBuf]) -> Vec<String> {
    let mut files = Vec::new();
    let mut problems = Vec::new();
    for p in paths {
        if p.is_dir() {
            collect(p, &mut files, &mut problems);
        } else {
            files.push(p.clone());
        }
    }
    files.sort();
    problems.sort();
    if files.is_empty() {
        let under: Vec<String> = paths.iter().map(|p| shown(p)).collect();
        problems.push(format!("no scripts under {}", under.join(", ")));
    }
    let next = AtomicUsize::new(0);
    let mut each: Vec<(usize, Vec<String>)> = thread::scope(|s| {
        let workers: Vec<_> = (0..WORKERS)
            .map(|_| {
                s.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(f) = files.get(i) else {
                            return done;
                        };
                        done.push((i, run_file(f)));
                    }
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().expect("a worker returns"))
            .collect()
    });
    each.sort_by_key(|(i, _)| *i);
    problems.extend(each.into_iter().flat_map(|(_, d)| d));
    problems
}

/// Run every script under `dir` and fail naming each difference.
fn run(dir: &Path) {
    let found = differences(&[dir.to_path_buf()]);
    assert!(
        found.is_empty(),
        "{} differences:\n{}",
        found.len(),
        found.join("\n")
    );
}

#[test]
fn oplog() {
    run(&root().join("oplog"));
}

#[test]
fn bundles() {
    run(&root().join("bundles"));
}

#[test]
fn roles() {
    run(&root().join("roles"));
}

#[test]
fn authority() {
    run(&root().join("authority"));
}

#[test]
fn registered() {
    run(&root().join("registered"));
}

#[test]
fn registration() {
    run(&root().join("registration"));
}

#[test]
fn commands() {
    run(&root().join("commands"));
}

#[test]
fn api() {
    run(&root().join("api"));
}

#[test]
fn past() {
    run(&root().join("past"));
}

#[test]
fn longkey() {
    run(&root().join("longkey"));
}

/// Run the scripts `ADTB_SCRIPTS` names. Does nothing when it is unset.
#[test]
fn by_hand() {
    let Ok(paths) = std::env::var(BY_HAND) else {
        return;
    };
    let found = differences(&paths.lines().map(PathBuf::from).collect::<Vec<_>>());
    assert!(
        found.is_empty(),
        "{} differences:\n{}",
        found.len(),
        found.join("\n")
    );
}

/// Returns the override blocks of the nextest configuration.
fn overrides() -> Vec<String> {
    let config = crate_dir().join("../../.config/nextest.toml");
    let text = fs::read_to_string(&config).expect("the nextest configuration reads");
    text.split("[[profile.default.overrides]]")
        .skip(1)
        .map(str::to_owned)
        .collect()
}

#[test]
fn every_area_has_its_test_holding_its_slots() {
    let source = include_str!("mod.rs");
    let blocks = overrides();
    let mut areas = 0;
    for entry in fs::read_dir(root()).expect("the scripts directory reads") {
        let path = entry.expect("a directory entry").path();
        let name = path.file_name().and_then(|n| n.to_str()).expect("a name");
        if !path.is_dir() || ["fixtures", "probes"].contains(&name) {
            continue;
        }
        areas += 1;
        let test = format!("fn {name}() {{\n    run(&root().join(\"{name}\"));");
        assert!(source.contains(&test), "scripts/{name} has no test");
        let filter = format!("test(=scripts::{name})");
        let slots = format!("threads-required = {WORKERS}\n");
        assert!(
            blocks
                .iter()
                .any(|b| b.contains(&filter) && b.contains(&slots)),
            "no override in .config/nextest.toml gives `{filter}` {WORKERS} slots"
        );
    }
    assert!(areas > 0, "no area under {}", root().display());
}

/// Returns the expected differences of probe `path`: the lines of the file beside it named
/// with `.differs`.
fn expected(path: &Path) -> Vec<String> {
    let file = path.with_extension("differs");
    let text = fs::read_to_string(&file)
        .unwrap_or_else(|e| panic!("{} does not read: {e}", file.display()));
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        match out.last_mut() {
            Some(last) if line.starts_with("  ") => {
                last.push('\n');
                last.push_str(line);
            }
            _ => out.push(line.to_owned()),
        }
    }
    out
}

#[test]
fn each_probe_differs_as_it_states() {
    let probes = root().join("probes");
    let mut ran = 0;
    for entry in fs::read_dir(&probes).expect("the probes directory reads") {
        let path = entry.expect("a directory entry").path();
        let got = match path.extension().and_then(|x| x.to_str()) {
            Some("test") => run_file(&path),
            Some("differs") => continue,
            _ if path.is_dir() => differences(std::slice::from_ref(&path)),
            _ => panic!("{} is no probe", path.display()),
        };
        assert_eq!(expected(&path), got, "{}", shown(&path));
        ran += 1;
    }
    assert!(ran > 0, "no probes under {}", probes.display());
}

#[test]
fn an_area_that_does_not_read_or_holds_no_script_fails() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("scripts-unreadable-area");
    let locked = dir.join("locked");
    fs::create_dir_all(&locked).expect("the directory is created");
    let mode = |m| {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&locked, fs::Permissions::from_mode(m)).expect("the mode is set");
    };
    mode(0o000);
    let got = differences(std::slice::from_ref(&dir));
    mode(0o755);
    let shown = dir.display();
    assert_eq!(
        vec![
            format!("{shown}/locked: Permission denied (os error 13)"),
            format!("no scripts under {shown}"),
        ],
        got
    );
}

#[test]
fn an_open_refusal_without_a_textual_value_is_a_difference() {
    assert_eq!(
        Err("the open is refused with `Locked`, which has no textual value".to_owned()),
        open_value(&OpenError::Locked { pid: Some(7) })
    );
}
