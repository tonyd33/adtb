use std::ffi::OsString;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

use crate::listing;
use crate::scope;
use crate::step::{self, say};
use crate::workspace::Workspace;

pub const STEP: &str = "tests";

/// Lines of the output printed when it holds no summary.
const TAIL: usize = 20;

/// The first words of nextest's lines for a test that did not pass.
const OUTCOMES: [&str; 5] = ["FAIL", "SIGKILL", "SIGABRT", "SIGSEGV", "TIMEOUT"];

/// Build the test binaries ARGS select with `cargo nextest run --no-run ARGS`, then run
/// `cargo nextest run --no-fail-fast ARGS` with their listing named, its full output in
/// `target/test.txt`, and print the build errors, nextest's summary and the scope's peak memory.
/// Returns the build's or nextest's result; a failure to record or print the output is printed as
/// a note.
pub fn run(workspace: &Workspace, args: &[OsString]) -> Result<(), String> {
    let (out, mut log) = create_log(workspace)?;
    let mut build = workspace.cargo(&["nextest", "run"]);
    build
        .args(["--no-run", "--cargo-message-format", "json"])
        .args(args);
    let (status, binaries) = drive(&mut build, &out, &mut log)?;
    if !status.success() {
        return finish(&build, status, "the build failed: ", &out, &mut log);
    }
    write_listing(workspace, &binaries)?;
    let mut nextest = workspace.cargo(&["nextest", "run"]);
    nextest
        .args(["--no-fail-fast", "--cargo-message-format", "json"])
        .args(args);
    record(workspace, nextest, &out, &mut log)
}

/// Run the test binaries `build` last built with `cargo nextest run --no-fail-fast ARGS`, building
/// none, as `run` does.
pub fn run_built(workspace: &Workspace, args: &[OsString]) -> Result<(), String> {
    let (out, mut log) = create_log(workspace)?;
    let mut nextest = reuse(workspace);
    nextest.arg("--no-fail-fast").args(args);
    record(workspace, nextest, &out, &mut log)
}

/// Returns `cargo nextest run` of the test binaries `build` last built, building none.
pub fn reuse(workspace: &Workspace) -> Command {
    let built = Built::of(workspace);
    let mut nextest = workspace.command("cargo");
    nextest
        .current_dir(&workspace.root)
        .args(["nextest", "run", "--binaries-metadata"])
        .arg(built.binaries)
        .arg("--cargo-metadata")
        .arg(built.metadata);
    nextest
}

/// Build every test binary of the workspace, and write nextest's listing of them, the workspace's
/// metadata and the binaries' listing under `test-binaries` in the target directory. Returns
/// cargo's output. On a build failure, write cargo's output to `target/test.txt`, marked as a
/// build failure, and print it.
pub fn build(workspace: &Workspace) -> Result<Vec<u8>, String> {
    let built = Built::of(workspace);
    let mut list = workspace.cargo(&["nextest", "list"]);
    list.args([
        "--workspace",
        "--list-type",
        "binaries-only",
        "--message-format",
        "json",
    ]);
    let listed = list
        .output()
        .map_err(|e| format!("{STEP}: {} did not start: {e}", step::shown(&list)))?;
    if !listed.status.success() {
        let out = workspace.target.join("test.txt");
        let mut text = listed.stderr.clone();
        text.extend(format!("xtask test: the build failed: {}\n", listed.status).bytes());
        std::fs::write(&out, text)
            .map_err(|e| format!("{STEP}: writing {}: {e}", out.display()))?;
        say(String::from_utf8_lossy(&listed.stderr).trim_end())?;
        say(format_args!("output in {}", out.display()))?;
    }
    step::succeeded(STEP, &list, listed.status)?;
    let cargo_metadata = step::output(
        STEP,
        workspace
            .cargo(&["metadata"])
            .args(["--format-version", "1"]),
    )?;
    let nextest_listing: serde_json::Value = serde_json::from_slice(&listed.stdout)
        .map_err(|e| format!("{STEP}: nextest's listing is no JSON: {e}"))?;
    let binaries: Vec<String> = nextest_listing["rust-binaries"]
        .as_object()
        .ok_or_else(|| format!("{STEP}: nextest's listing has no `rust-binaries`"))?
        .values()
        .filter_map(|binary| binary["binary-path"].as_str().map(str::to_owned))
        .collect();
    write_listing(workspace, &binaries)?;
    for (path, bytes) in [
        (&built.binaries, &listed.stdout),
        (&built.metadata, &cargo_metadata),
    ] {
        std::fs::write(path, bytes)
            .map_err(|e| format!("{STEP}: writing {}: {e}", path.display()))?;
    }
    Ok(listed.stderr)
}

/// The files `build` and `run` write under `test-binaries` in the target directory.
struct Built {
    /// nextest's listing of the test binaries.
    binaries: PathBuf,
    /// The workspace's `cargo metadata`.
    metadata: PathBuf,
    /// The path of each test binary, one to a line, which the test of every test binary's heap
    /// cap reads.
    listing: PathBuf,
}

impl Built {
    fn of(workspace: &Workspace) -> Self {
        let dir = workspace.target.join("test-binaries");
        Self {
            binaries: dir.join("binaries.json"),
            metadata: dir.join("cargo-metadata.json"),
            listing: dir.join("listing.txt"),
        }
    }
}

/// Write `binaries`, one to a line, as the listing the test of every test binary's heap cap reads.
fn write_listing(workspace: &Workspace, binaries: &[String]) -> Result<(), String> {
    let listing = Built::of(workspace).listing;
    if let Some(dir) = listing.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("{STEP}: creating {}: {e}", dir.display()))?;
    }
    let text: String = binaries
        .iter()
        .map(|binary| format!("{binary}\n"))
        .collect();
    std::fs::write(&listing, text)
        .map_err(|e| format!("{STEP}: writing {}: {e}", listing.display()))
}

/// Create `target/test.txt`, and return its path and the file.
fn create_log(workspace: &Workspace) -> Result<(PathBuf, File), String> {
    let out = workspace.target.join("test.txt");
    std::fs::create_dir_all(&workspace.target)
        .map_err(|e| format!("{STEP}: creating {}: {e}", workspace.target.display()))?;
    let log = File::create(&out).map_err(|e| format!("{STEP}: creating {}: {e}", out.display()))?;
    Ok((out, log))
}

/// Run `nextest` with the listing of the test binaries named to the test of every test binary's
/// heap cap, its output to `log`, and print the summary, the scope's peak memory and `out`.
fn record(
    workspace: &Workspace,
    mut nextest: Command,
    out: &Path,
    log: &mut File,
) -> Result<(), String> {
    nextest.env(listing::TEST_BINARIES, Built::of(workspace).listing);
    let (status, _) = drive(&mut nextest, out, log)?;
    finish(&nextest, status, "", out, log)
}

/// Run `command`, its standard error and its compiler messages to `log`, printing the errors
/// among them. Returns its exit and the path of each test binary cargo built.
fn drive(
    command: &mut Command,
    out: &Path,
    log: &mut File,
) -> Result<(ExitStatus, Vec<String>), String> {
    command.stdout(Stdio::piped()).stderr(
        log.try_clone()
            .map_err(|e| format!("{STEP}: sharing {}: {e}", out.display()))?,
    );
    let mut child = command
        .spawn()
        .map_err(|e| format!("{STEP}: {} did not start: {e}", step::shown(command)))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("{STEP}: nextest has no standard output"))?;
    let copied = copy_messages(BufReader::new(stdout), log);
    let status = child
        .wait()
        .map_err(|e| format!("{STEP}: waiting for nextest: {e}"))?;
    Ok((status, copied?))
}

/// Write `xtask test: WHAT STATUS` to `log`, print the summary in it, the scope's peak memory and
/// `out`. Returns whether `command` succeeded.
fn finish(
    command: &Command,
    status: ExitStatus,
    what: &str,
    out: &Path,
    log: &mut File,
) -> Result<(), String> {
    let reported = writeln!(log, "xtask test: {what}{status}")
        .and_then(|()| log.flush())
        .map_err(|e| format!("{STEP}: writing {}: {e}", out.display()))
        .and_then(|()| report(out));
    if let Err(e) = reported {
        say(format_args!("xtask: {e}"))?;
    }
    if let Some(cgroup) = scope::own_scope() {
        let unit = cgroup.rsplit('/').next().unwrap_or_default();
        match scope::peak_mib(&cgroup) {
            Ok(peak) => say(format_args!("peak {peak} MiB in {unit}"))?,
            Err(e) => say(format_args!("peak unknown in {unit}: {e}"))?,
        }
    }
    say(format_args!("output in {}", out.display()))?;
    step::succeeded(STEP, command, status)
}

/// Write each compiler message cargo prints as JSON to `log` as rustc renders it, and print the
/// errors among them. Any other line goes to `log` as it is. Returns the path of each test binary
/// cargo reports built.
fn copy_messages(stdout: impl BufRead, log: &mut File) -> Result<Vec<String>, String> {
    let mut binaries = Vec::new();
    for line in stdout.lines() {
        let line = line.map_err(|e| format!("{STEP}: reading nextest's output: {e}"))?;
        let message: serde_json::Value = serde_json::from_str(&line).unwrap_or_default();
        let text = if message["reason"] == "compiler-message" {
            let rendered = message["message"]["rendered"].as_str().unwrap_or_default();
            if message["message"]["level"] == "error" {
                say(rendered.trim_end())?;
            }
            rendered.to_owned()
        } else if message.is_null() {
            format!("{line}\n")
        } else {
            if message["reason"] == "compiler-artifact"
                && message["profile"]["test"] == true
                && let Some(binary) = message["executable"].as_str()
            {
                binaries.push(binary.to_owned());
            }
            continue;
        };
        log.write_all(text.as_bytes())
            .map_err(|e| format!("{STEP}: writing the output: {e}"))?;
    }
    Ok(binaries)
}

fn report(out: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(out)
        .map_err(|e| format!("{STEP}: reading {}: {e}", out.display()))?;
    summary(&text).into_iter().try_for_each(say)
}

/// Returns nextest's summary in `output`: its last line `Summary [` to the end, which lists every
/// test that did not pass. Without one, returns each line of a test that did not pass before the
/// last lines, then the last lines.
fn summary(output: &str) -> Vec<&str> {
    let lines: Vec<&str> = output.lines().collect();
    if let Some(start) = lines
        .iter()
        .rposition(|line| line.trim_start().starts_with("Summary ["))
    {
        return lines[start..].to_vec();
    }
    let tail = lines.len().saturating_sub(TAIL);
    let outcome = |line: &str| {
        line.split_whitespace()
            .next()
            .is_some_and(|word| OUTCOMES.contains(&word))
    };
    lines[..tail]
        .iter()
        .filter(|line| outcome(line))
        .chain(&lines[tail..])
        .copied()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_summary_runs_from_the_last_summary_line_to_the_end() {
        let output = "\
     Summary [   1.000s] stale
        PASS [   0.001s] (1/3) a one
────────────
     Summary [   2.002s] 3 tests run: 1 passed, 1 failed, 1 timed out
        FAIL [   0.005s] (2/3) a two
     TIMEOUT [   2.002s] (3/3) a three
error: test run failed";
        assert_eq!(
            summary(output),
            [
                "     Summary [   2.002s] 3 tests run: 1 passed, 1 failed, 1 timed out",
                "        FAIL [   0.005s] (2/3) a two",
                "     TIMEOUT [   2.002s] (3/3) a three",
                "error: test run failed",
            ]
        );
    }

    #[test]
    fn without_a_summary_each_test_that_did_not_pass_precedes_the_last_lines() {
        let mut lines = vec![
            "        PASS [   0.001s] (1/9) a one".to_owned(),
            "        FAIL [   0.002s] (2/9) a two".to_owned(),
            "     SIGKILL [   0.003s] (3/9) a three".to_owned(),
            "     SIGABRT [   0.004s] (4/9) a four".to_owned(),
            "     SIGSEGV [   0.005s] (5/9) a five".to_owned(),
            "     TIMEOUT [   0.006s] (6/9) a six".to_owned(),
            "    FAILURES are words in a test's output".to_owned(),
        ];
        let tail: Vec<String> = (0..TAIL).map(|i| format!("line {i}")).collect();
        lines.extend(tail.iter().cloned());
        let output = lines.join("\n");
        let expected: Vec<&str> = lines[1..6]
            .iter()
            .chain(&tail)
            .map(String::as_str)
            .collect();
        assert_eq!(summary(&output), expected);
    }

    #[test]
    fn a_short_output_without_a_summary_prints_whole() {
        let output = "error: no such package\n    FAIL x";
        assert_eq!(summary(output), ["error: no such package", "    FAIL x"]);
    }
}
