//! The runner's command line over corpora in a scratch workspace.

use std::fs;
use std::path::Path;
use std::process::Command;

use adtb_testkit::scratch::Scratch;

const SCRIPTS: &str = "crates/adtb-sim/scripts";

/// The signal the heap cap aborts with.
const SIGABRT: i32 = 6;

/// Returns a workspace holding both corpora, each file of `files` written at its path.
fn workspace(label: &str, files: &[(&str, &str)]) -> Scratch {
    let scratch = Scratch::new("runner-cli", label);
    fs::create_dir_all(scratch.join("conformance")).expect("the conformance corpus is made");
    fs::create_dir_all(scratch.join(SCRIPTS)).expect("the scripts corpus is made");
    for (path, text) in files {
        let path = scratch.join(path);
        fs::create_dir_all(path.parent().expect("a file lies in a directory"))
            .expect("the directory is made");
        fs::write(path, text).expect("the case is written");
    }
    scratch
}

/// Run the runner in `dir` with `args`, returning its exit code and stdout.
fn runner(dir: &Path, args: &[&str]) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_adtb-conformance"))
        .args(args)
        .arg("--no-color")
        .current_dir(dir)
        .output()
        .expect("the runner starts");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let code = out
        .status
        .code()
        .unwrap_or_else(|| panic!("killed: {stderr}"));
    (
        code,
        String::from_utf8(out.stdout).expect("stdout is UTF-8") + &stderr,
    )
}

const ROWS: &str = "schema
  module test
  type T = { id: I64, n: I64 }
  relation ts : T key id
run
  insert ts { id: 1, n: 1 }
  insert ts { id: 2, n: 2 }
query
  from t in ts select t.n
----
1
3

# the end
";

#[test]
fn a_query_with_other_rows_fails_showing_them_as_a_diff() {
    let ws = workspace("diff", &[("conformance/q.test", ROWS)]);
    let (code, out) = runner(&ws, &["--reference"]);
    assert_eq!(
        "  FAIL q\n\n0 passed, 1 failed, 0 updated, 0 skipped\n\nq\n  conformance/q.test:8: \
         reference: got [1, 2]\n      1\n    - 3\n    + 2\n",
        out
    );
    assert_eq!(1, code);
}

#[test]
fn update_writes_the_rows_got_and_the_case_then_passes() {
    let ws = workspace("update", &[("conformance/q.test", ROWS)]);
    let (code, out) = runner(&ws, &["--update", "--file-name", "conformance"]);
    assert_eq!(
        "  UPDATED q\n\n0 passed, 0 failed, 1 updated, 0 skipped\n",
        out
    );
    assert_eq!(0, code);
    let written = fs::read_to_string(ws.join("conformance/q.test")).expect("the case reads");
    assert_eq!(ROWS.replace("----\n1\n3\n", "----\n1\n2\n"), written);
    let (code, out) = runner(&ws, &["--file-name", "conformance"]);
    assert_eq!("\n1 passed, 0 failed, 0 updated, 0 skipped\n", out);
    assert_eq!(0, code);
}

#[test]
fn update_writes_the_rows_of_an_engine_script() {
    let script = "migrate
  module test
  add module test
  add type T = { id: I64, n: I64 }
  add relation ts : T key id
run
  insert ts { id: 1, n: 7 }
query
  from t in ts select t.n
----
8
";
    let path = format!("{SCRIPTS}/area/s.test");
    let ws = workspace("update-script", &[(&path, script)]);
    let (code, out) = runner(&ws, &["--update", "--file-name", &path]);
    assert_eq!(
        "  UPDATED area/s\n\n0 passed, 0 failed, 1 updated, 0 skipped\n",
        out
    );
    assert_eq!(0, code);
    let written = fs::read_to_string(ws.join(&path)).expect("the script reads");
    assert_eq!(script.replace("----\n8\n", "----\n7\n"), written);
}

#[test]
fn update_leaves_a_case_whose_difference_states_no_rows() {
    let refused = "schema
  module test
  type T = { id: I64 }
  relation ts : T key id
error KeyViolation
  insert ts { id: 1 }
";
    let ws = workspace("update-refusal", &[("conformance/e.test", refused)]);
    let (code, out) = runner(&ws, &["--reference", "--update"]);
    assert_eq!(
        "  FAIL e\n\n0 passed, 1 failed, 0 updated, 0 skipped\n\ne\n  conformance/e.test:5: \
         reference: succeeds with (), not refused with KeyViolation\n",
        out
    );
    assert_eq!(1, code);
    let kept = fs::read_to_string(ws.join("conformance/e.test")).expect("the case reads");
    assert_eq!(refused, kept);
}

#[test]
fn include_skips_each_case_whose_name_lacks_the_pattern() {
    let ws = workspace(
        "include",
        &[
            ("conformance/one/kept.test", ROWS),
            ("conformance/two/left.test", ROWS),
        ],
    );
    let (code, out) = runner(&ws, &["--reference", "--include", "one/"]);
    assert!(
        out.ends_with("\n0 passed, 1 failed, 0 updated, 1 skipped\n\none/kept\n  conformance/one/kept.test:8: reference: got [1, 2]\n      1\n    - 3\n    + 2\n"),
        "{out}"
    );
    assert_eq!(1, code);
}

#[test]
fn a_file_name_runs_only_that_case() {
    let ws = workspace(
        "file-name",
        &[
            ("conformance/a.test", ROWS),
            ("conformance/b.test", &ROWS.replace("3\n", "2\n")),
        ],
    );
    let (code, out) = runner(&ws, &["--reference", "--file-name", "conformance/b.test"]);
    assert_eq!("\n1 passed, 0 failed, 0 updated, 0 skipped\n", out);
    assert_eq!(0, code);
}

#[test]
fn fail_fast_stops_after_the_first_failing_case() {
    let ws = workspace(
        "fail-fast",
        &[("conformance/a.test", ROWS), ("conformance/b.test", ROWS)],
    );
    let (code, out) = runner(&ws, &["--reference", "--fail-fast"]);
    assert!(
        out.starts_with(
            "  FAIL a\n\n0 passed, 1 failed, 0 updated, 0 skipped\nstopped at the first failure\n"
        ),
        "{out}"
    );
    assert_eq!(1, code);
}

#[test]
fn a_file_name_outside_both_corpora_is_a_usage_error() {
    let ws = workspace("outside", &[("elsewhere/a.test", ROWS)]);
    let (code, out) = runner(&ws, &["--file-name", "elsewhere/a.test"]);
    assert_eq!(
        "adtb-conformance: elsewhere/a.test: under neither `conformance` nor \
         `crates/adtb-sim/scripts` of the working directory\n",
        out
    );
    assert_eq!(2, code);
}

#[test]
fn a_stray_file_among_the_engine_scripts_fails() {
    let ws = workspace("stray", &[(&format!("{SCRIPTS}/area/notes.txt"), "")]);
    let (code, out) = runner(&ws, &["--file-name", SCRIPTS]);
    assert!(
        out.contains("\nscripts/area/notes.txt: not a `.test` script\n"),
        "{out}"
    );
    assert_eq!(1, code);
}

#[test]
fn update_with_only_the_engine_is_refused_for_conformance_files() {
    let ws = workspace("update-engine", &[("conformance/q.test", ROWS)]);
    let (code, out) = runner(&ws, &["--engine", "--update", "--file-name", "conformance"]);
    assert_eq!(
        "adtb-conformance: `--update` takes the reference's rows; the engine never writes the \
         expected rows of a conformance file\n",
        out
    );
    assert_eq!(2, code);
    let kept = fs::read_to_string(ws.join("conformance/q.test")).expect("the case reads");
    assert_eq!(ROWS, kept);
}

#[test]
fn a_rowsort_difference_shows_both_sides_sorted() {
    let sorted = "schema
  module test
  type T = { id: I64 }
  relation ts : T key id
run
  insert ts { id: 1 }
  insert ts { id: 2 }
  insert ts { id: 3 }
query rowsort
  from t in ts select t.id
----
3
2
9
";
    let ws = workspace("rowsort", &[("conformance/r.test", sorted)]);
    let (code, out) = runner(&ws, &["--reference"]);
    assert!(
        out.ends_with("reference: got [1, 2, 3]\n    + 1\n      2\n      3\n    - 9\n"),
        "{out}"
    );
    assert_eq!(1, code);
}

/// Run the runner's `engine-case` over `file` in `dir` with `cap` bytes of heap.
fn engine_case(dir: &Path, file: &str, cap: Option<&str>) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_adtb-conformance"));
    command
        .args(["engine-case", "conformance", file])
        .current_dir(dir);
    if let Some(cap) = cap {
        command.env("ADTB_TEST_HEAP_CAP", cap);
    }
    command.output().expect("the runner starts")
}

#[test]
fn an_engine_case_that_selects_no_file_fails() {
    let ws = workspace("engine-case-missing", &[]);
    let out = engine_case(&ws, "conformance/none.test", None);
    assert_eq!(Some(2), out.status.code());
    assert_eq!(
        "adtb-conformance engine-case: conformance/none.test selects 0 cases, not one: \
         conformance/none.test: No such file or directory (os error 2); no cases under \
         conformance/none.test\n",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!("", String::from_utf8_lossy(&out.stdout));
}

#[test]
fn the_engine_case_child_aborts_at_its_heap_cap() {
    use std::os::unix::process::ExitStatusExt;
    let ws = workspace("engine-case-cap", &[("conformance/q.test", ROWS)]);
    let out = engine_case(&ws, "conformance/q.test", Some("1"));
    assert_eq!(Some(SIGABRT), out.status.signal(), "{out:?}");
}
