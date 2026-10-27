//! The `adtb-probe` binary over the schemas in `fixtures/`.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// Runs the probe with `args` and `stdin`, returning its exit code and stdout.
fn probe_with(args: &[&str], stdin: &str) -> (i32, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_adtb-probe"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the probe starts");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(stdin.as_bytes())
        .expect("stdin is written");
    let out = child.wait_with_output().expect("the probe ends");
    let stdout = String::from_utf8(out.stdout).expect("stdout is UTF-8");
    let stderr = String::from_utf8_lossy(&out.stderr);
    (
        out.status
            .code()
            .unwrap_or_else(|| panic!("killed: {stderr}")),
        stdout,
    )
}

fn probe(args: &[&str]) -> (i32, String) {
    probe_with(args, "")
}

fn loaded(more: &[&str]) -> Vec<String> {
    let mut args = vec![
        "--schema".to_owned(),
        fixture("app.adt").display().to_string(),
        "--data".to_owned(),
        fixture("data.adt").display().to_string(),
    ];
    args.extend(more.iter().map(|&s| s.to_owned()));
    args
}

fn strs(args: &[String]) -> Vec<&str> {
    args.iter().map(String::as_str).collect()
}

#[test]
fn a_query_runs_on_both_sides_and_agrees() {
    let args = loaded(&["from u in users where u.age > 35 select u.name"]);
    let (code, out) = probe(&strs(&args));
    assert_eq!(
        out, "reference: [\"bob\"]\nengine: [\"bob\"]\nagree\n",
        "exit {code}"
    );
    assert_eq!(code, 0);
}

#[test]
fn an_empty_result_prints_as_an_empty_set() {
    let args = loaded(&["from u in users where u.age > 99 select u.name"]);
    let (code, out) = probe(&strs(&args));
    assert_eq!(out, "reference: []\nengine: []\nagree\n", "exit {code}");
}

#[test]
fn a_refused_text_prints_its_line_and_column_and_runs_nothing() {
    let args = loaded(&["from u in users\nselect u.nam"]);
    let (code, out) = probe(&strs(&args));
    assert_eq!(
        out,
        "TEXT:2:10: User has no field `nam`\n  select u.nam\n           ^\n"
    );
    assert_eq!(code, 2);
}

#[test]
fn a_refused_schema_names_the_line_in_its_file() {
    let schema = fixture("two-modules.adt").display().to_string();
    let (code, out) = probe(&["--schema", &schema, "1"]);
    assert!(
        out.starts_with(&format!("{schema}:6:36: no type `Txt`")),
        "{out}"
    );
    assert_eq!(code, 2);
}

#[test]
fn a_write_both_sides_refuse_for_one_reason_agrees() {
    let args = loaded(&["insert users { name: \"kid\", age: 3 }"]);
    let (code, out) = probe(&strs(&args));
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 3, "{out}");
    assert!(lines[0].starts_with("reference: "), "{out}");
    assert!(lines[0].contains("refused with Violated"), "{out}");
    assert!(
        lines[1].starts_with("engine: refused with Violated"),
        "{out}"
    );
    assert_eq!(lines[2], "agree");
    assert_eq!(code, 0);
}

#[test]
fn a_write_then_a_query_shows_the_write() {
    let args = loaded(&[
        "insert users { name: \"cy\", age: 50 }\nfrom u in users where u.age > 35 select u.name",
    ]);
    let (_, out) = probe(&strs(&args));
    assert_eq!(
        out,
        "reference: [\"bob\", \"cy\"]\nengine: [\"bob\", \"cy\"]\nagree\n"
    );
}

#[test]
fn explain_prints_the_engine_plan() {
    let args = loaded(&["--explain", "from u in users select u.name"]);
    let (code, out) = probe(&strs(&args));
    assert!(out.contains("engine plan:\n  "), "{out}");
    assert!(out.contains("Scan users.User"), "{out}");
    assert!(!out.contains("reference plan"), "{out}");
    assert_eq!(code, 0, "{out}");
}

#[test]
fn one_side_runs_alone_and_prints_no_verdict() {
    let args = loaded(&["--engine", "from u in users select u.age"]);
    let (code, out) = probe(&strs(&args));
    assert_eq!(out, "engine: [30, 41]\n");
    assert_eq!(code, 0);
}

#[test]
fn a_dash_reads_the_text_from_stdin() {
    let args = loaded(&["--reference", "-"]);
    let (_, out) = probe_with(&strs(&args), "from u in users select u.age\n");
    assert_eq!(out, "reference: [30, 41]\n");
}

#[test]
fn data_the_reference_refuses_is_reported_as_loading() {
    let schema = fixture("app.adt").display().to_string();
    let data = fixture("two-modules.adt").display().to_string();
    let (code, out) = probe(&["--schema", &schema, "--data", &data, "users"]);
    assert!(out.contains("reference: the data: "), "{out}");
    assert!(out.contains("engine: the data: "), "{out}");
    assert!(out.ends_with("differ\n"), "{out}");
    assert_eq!(code, 1);
}
