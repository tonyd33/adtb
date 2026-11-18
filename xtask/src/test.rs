use std::ffi::OsString;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::Stdio;

use crate::scope;
use crate::step::{self, say};
use crate::workspace::Workspace;

pub const STEP: &str = "tests";

/// Lines of the output printed when it holds no summary.
const TAIL: usize = 20;

/// The first words of nextest's lines for a test that did not pass.
const OUTCOMES: [&str; 5] = ["FAIL", "SIGKILL", "SIGABRT", "SIGSEGV", "TIMEOUT"];

/// Run `cargo nextest run --no-fail-fast ARGS`, its full output in `target/test.txt`, and print
/// the build errors, nextest's summary and the scope's peak memory. Returns nextest's result; a
/// failure to record or print the output is printed as a note.
pub fn run(workspace: &Workspace, args: &[OsString]) -> Result<(), String> {
    let out = workspace.target.join("test.txt");
    std::fs::create_dir_all(&workspace.target)
        .map_err(|e| format!("{STEP}: creating {}: {e}", workspace.target.display()))?;
    let mut log =
        File::create(&out).map_err(|e| format!("{STEP}: creating {}: {e}", out.display()))?;
    let mut nextest = workspace.cargo(&["nextest", "run"]);
    nextest
        .args(["--no-fail-fast", "--cargo-message-format", "json"])
        .args(args)
        .stdout(Stdio::piped())
        .stderr(
            log.try_clone()
                .map_err(|e| format!("{STEP}: sharing {}: {e}", out.display()))?,
        );
    let mut child = nextest
        .spawn()
        .map_err(|e| format!("{STEP}: {} did not start: {e}", step::shown(&nextest)))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("{STEP}: nextest has no standard output"))?;
    let written = copy_messages(BufReader::new(stdout), &mut log);
    let status = child
        .wait()
        .map_err(|e| format!("{STEP}: waiting for nextest: {e}"))?;
    let reported = written
        .and_then(|()| {
            writeln!(log, "xtask test: {status}")
                .and_then(|()| log.flush())
                .map_err(|e| format!("{STEP}: writing {}: {e}", out.display()))
        })
        .and_then(|()| report(&out));
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
    step::succeeded(STEP, &nextest, status)
}

/// Write each compiler message cargo prints as JSON to `log` as rustc renders it, and print the
/// errors among them. Any other line goes to `log` as it is.
fn copy_messages(stdout: impl BufRead, log: &mut File) -> Result<(), String> {
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
            continue;
        };
        log.write_all(text.as_bytes())
            .map_err(|e| format!("{STEP}: writing the output: {e}"))?;
    }
    Ok(())
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
