//! What a run prints: a line per case that fails or is updated, a summary, and each failure with
//! its rows as a diff.

use std::fmt::Write as _;

use crate::Found;

use super::{Report, Verdict};

/// The unchanged rows shown on each side of a change.
const CONTEXT: usize = 2;

#[derive(Clone, Copy)]
enum Paint {
    Red,
    Green,
    Yellow,
    Cyan,
    Dim,
    Bold,
}

fn paint(color: bool, p: Paint, text: &str) -> String {
    if !color {
        return text.to_owned();
    }
    let code = match p {
        Paint::Red => "31",
        Paint::Green => "32",
        Paint::Yellow => "33",
        Paint::Cyan => "36",
        Paint::Dim => "2",
        Paint::Bold => "1",
    };
    format!("\x1b[{code}m{text}\x1b[0m")
}

/// Returns the line a case that fails or is updated prints when it ends, `None` for any other.
pub(super) fn status(name: &str, verdict: &Verdict, color: bool) -> Option<String> {
    let mark = match verdict {
        Verdict::Failed(_) => paint(color, Paint::Red, if color { "✗" } else { "FAIL" }),
        Verdict::Updated => paint(color, Paint::Yellow, if color { "~" } else { "UPDATED" }),
        Verdict::Passed | Verdict::Skipped => return None,
    };
    Some(format!("  {mark} {name}\n"))
}

/// Returns the counts of `report`, then each failed case with its differences.
pub(super) fn summary(report: &Report, color: bool) -> String {
    let mut out = String::new();
    let (passed, failed) = (report.passed(), report.failed());
    let counts = format!(
        "{passed} passed, {failed} failed, {} updated, {} skipped",
        report.updated(),
        report.skipped()
    );
    let paint_as = if failed > 0 { Paint::Red } else { Paint::Green };
    writeln!(out, "\n{}", paint(color, paint_as, &counts)).expect("a string takes text");
    if report.stopped {
        writeln!(out, "stopped at the first failure").expect("a string takes text");
    }
    for p in &report.problems {
        writeln!(out, "\n{}", paint(color, Paint::Red, &p.shown())).expect("a string takes text");
    }
    for (name, verdict) in &report.cases {
        let Verdict::Failed(found) = verdict else {
            continue;
        };
        writeln!(out, "\n{}", paint(color, Paint::Bold, name)).expect("a string takes text");
        for f in found {
            out.push_str(&difference(f, color));
        }
    }
    out
}

/// Returns `f` as its head, then its values or its rows as a diff of the expected against what it
/// got.
pub fn difference(f: &Found, color: bool) -> String {
    let mut out = format!("  {}\n", paint(color, Paint::Cyan, &f.head));
    let (expected, actual) = match (&f.rows, &f.values) {
        (Some((e, a)), _) => (e.clone(), a.clone()),
        (None, Some((e, a))) => (vec![e.clone()], vec![a.clone()]),
        (None, None) => return out,
    };
    for (op, line) in shown_ops(&expected, &actual) {
        let (mark, p) = match op {
            Op::Keep => (' ', Paint::Dim),
            Op::Remove => ('-', Paint::Red),
            Op::Add => ('+', Paint::Green),
        };
        writeln!(out, "    {}", paint(color, p, &format!("{mark} {line}")))
            .expect("a string takes text");
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Keep,
    Remove,
    Add,
}

/// Returns the edit from `expected` to `actual` by a longest common subsequence of their lines,
/// each kept line more than `CONTEXT` lines from a change left out.
fn shown_ops<'a>(expected: &'a [String], actual: &'a [String]) -> Vec<(Op, &'a str)> {
    let ops = edit(expected, actual);
    let near = |i: usize| {
        let lo = i.saturating_sub(CONTEXT);
        let hi = (i + CONTEXT + 1).min(ops.len());
        ops[lo..hi].iter().any(|(op, _)| *op != Op::Keep)
    };
    ops.iter()
        .enumerate()
        .filter(|(i, (op, _))| *op != Op::Keep || near(*i))
        .map(|(_, o)| *o)
        .collect()
}

fn edit<'a>(expected: &'a [String], actual: &'a [String]) -> Vec<(Op, &'a str)> {
    let (m, n) = (expected.len(), actual.len());
    let mut common = vec![vec![0usize; n + 1]; m + 1];
    for i in (0..m).rev() {
        for j in (0..n).rev() {
            common[i][j] = if expected[i] == actual[j] {
                common[i + 1][j + 1] + 1
            } else {
                common[i + 1][j].max(common[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    while i < m || j < n {
        if i < m && j < n && expected[i] == actual[j] {
            out.push((Op::Keep, expected[i].as_str()));
            i += 1;
            j += 1;
        } else if i < m && (j == n || common[i + 1][j] >= common[i][j + 1]) {
            out.push((Op::Remove, expected[i].as_str()));
            i += 1;
        } else {
            out.push((Op::Add, actual[j].as_str()));
            j += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn a_diff_keeps_two_unchanged_rows_beside_each_change() {
        let expected = lines("a b c d e f g h");
        let actual = lines("a b c d x f g h");
        let found = Found {
            line: Some(3),
            head: "f.test:3: got".into(),
            values: None,
            rows: Some((expected, actual)),
        };
        assert_eq!(
            "  f.test:3: got\n      c\n      d\n    - e\n    + x\n      f\n      g\n",
            difference(&found, false)
        );
    }

    #[test]
    fn a_value_shows_as_one_removed_and_one_added_line() {
        let found = Found {
            line: None,
            head: "f.test:1: refused otherwise".into(),
            values: Some(("A".into(), "B".into())),
            rows: None,
        };
        assert_eq!(
            "  f.test:1: refused otherwise\n    - A\n    + B\n",
            difference(&found, false)
        );
    }
}
