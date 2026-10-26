//! The `adtb` binary run against a real database file.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const SCHEMA: &str = r"module test
add module test
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
add type Identity { id: Key identities, user: Key users }
  = EmailPass { email: Text, hash: Bytes }
  | Google    { gid: Text }
  | Apple     { sub: Text }
add relation identities : Identity
  key id generate sequence
  unique (email) in EmailPass
  unique (gid) in Google
  unique (sub) in Apple
add set OAuth of identities = Google | Apple
";

const SIGN_INS: &str = r#"from i in identities select match i
  | EmailPass e -> e.email
  | Google g    -> "google " ++ g.gid
  | Apple a     -> "apple " ++ a.sub"#;

/// Returns a fresh directory holding `schema.adt`, and the path of a database file in it.
fn scratch(name: &str) -> (PathBuf, PathBuf) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("shell")
        .join(name);
    match std::fs::remove_dir_all(&dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => panic!("clear {dir:?}: {e}"),
        _ => {}
    }
    std::fs::create_dir_all(&dir).unwrap();
    let schema = dir.join("schema.adt");
    std::fs::write(&schema, SCHEMA).unwrap();
    (schema, dir.join("db.adt"))
}

fn adtb(args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_adtb"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the shell starts");
    child
        .stdin
        .take()
        .expect("a piped stdin")
        .write_all(stdin.as_bytes())
        .expect("the shell reads its stdin");
    child.wait_with_output().expect("the shell exits")
}

fn create(schema: &Path, db: &Path) {
    let out = adtb(
        &[
            "--create",
            "--schema",
            schema.to_str().unwrap(),
            db.to_str().unwrap(),
        ],
        "",
    );
    assert!(out.status.success(), "create: {out:?}");
}

fn run(db: &Path, text: &str) -> Output {
    adtb(&[db.to_str().unwrap(), "-c", text], "")
}

fn stdout(out: &Output) -> Vec<String> {
    String::from_utf8(out.stdout.clone())
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn stderr(out: &Output) -> String {
    String::from_utf8(out.stderr.clone()).unwrap()
}

fn seed(db: &Path) {
    let out = run(
        db,
        r#"transact
  let ann = insert users { name: "ann" }
  let bo = insert users { name: "bo" }
  insert identities.EmailPass { user: key ann, email: "ann@example.com", hash: bytes("h1") }
  insert identities.Google { user: key ann, gid: "g-ann" }
  insert identities.Apple { user: key bo, sub: "a-bo" }"#,
    );
    assert!(out.status.success(), "seed: {out:?}");
    assert!(stdout(&out).is_empty(), "a write prints nothing: {out:?}");
}

#[test]
fn a_created_database_answers_a_match_over_constructors() {
    let (schema, db) = scratch("match");
    create(&schema, &db);
    seed(&db);
    let out = run(&db, SIGN_INS);
    assert!(out.status.success(), "{out:?}");
    let mut rows = stdout(&out);
    rows.sort();
    assert_eq!(
        vec![
            r#""ann@example.com""#,
            r#""apple a-bo""#,
            r#""google g-ann""#
        ],
        rows
    );
}

#[test]
fn query_rows_print_one_per_line_in_the_textual_syntax() {
    let (schema, db) = scratch("rows");
    create(&schema, &db);
    seed(&db);
    let out = run(
        &db,
        "from u in users select { name: u.name } order by u.name",
    );
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        vec![r#"{ name: "ann" }"#, r#"{ name: "bo" }"#],
        stdout(&out)
    );
}

#[test]
fn a_failing_text_prints_its_error_and_exits_with_one() {
    let (schema, db) = scratch("fails");
    create(&schema, &db);
    seed(&db);
    let out = run(
        &db,
        r#"insert identities.Google { user: users.key(1), gid: "g-ann" }"#,
    );
    assert_eq!(Some(1), out.status.code(), "{out:?}");
    let err = stderr(&out);
    assert_eq!(1, err.lines().count(), "one line: {err}");
    assert!(
        err.starts_with(
            r#"Rejected { reason: Violated { constraint: "test::identities.unique(gid) in Google", "#
        ),
        "{err}"
    );
}

#[test]
fn a_text_that_does_not_check_is_rejected_as_invalid() {
    let (schema, db) = scratch("invalid");
    create(&schema, &db);
    let out = run(&db, "from u in nowhere select u");
    assert_eq!(Some(1), out.status.code(), "{out:?}");
    assert!(
        stderr(&out).starts_with("Rejected { reason: Invalid { detail: \""),
        "{out:?}"
    );
}

#[test]
fn a_script_on_stdin_runs_each_text_in_turn() {
    let (schema, db) = scratch("script");
    create(&schema, &db);
    let script = r#"-- two users, then a query split over lines
insert users { name: "ann" }
insert users {
  name: "bo"
}
from u in users
  select u.name
  order by u.name
"#;
    let out = adtb(&[db.to_str().unwrap()], script);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(vec![r#""ann""#, r#""bo""#], stdout(&out));
}

#[test]
fn an_indented_line_continues_a_text_in_a_script() {
    let (schema, db) = scratch("indented");
    create(&schema, &db);
    seed(&db);
    let script = format!("{SIGN_INS}\nfrom u in users select u.name\n  order by u.name\n");
    let out = adtb(&[db.to_str().unwrap()], &script);
    assert!(out.status.success(), "{out:?}");
    let rows = stdout(&out);
    let mut sign_ins = rows[..3].to_vec();
    sign_ins.sort();
    assert_eq!(
        vec![
            r#""ann@example.com""#,
            r#""apple a-bo""#,
            r#""google g-ann""#
        ],
        sign_ins
    );
    assert_eq!(vec![r#""ann""#, r#""bo""#], rows[3..]);
}

#[test]
fn a_transact_block_ends_at_an_unindented_line_in_a_script() {
    let (schema, db) = scratch("transact-ends");
    create(&schema, &db);
    let script = r#"transact
  insert users { name: "ann" }
  insert users { name: "bo" }
from u in users select u.name order by u.name
"#;
    let out = adtb(&[db.to_str().unwrap()], script);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(vec![r#""ann""#, r#""bo""#], stdout(&out));
}

#[test]
fn a_script_with_a_failing_text_runs_on_and_exits_with_one() {
    let (schema, db) = scratch("script-fails");
    create(&schema, &db);
    let script = r#"insert users { name: 1 }
insert users { name: "bo" }
from u in users select u.name
"#;
    let out = adtb(&[db.to_str().unwrap()], script);
    assert_eq!(Some(1), out.status.code(), "{out:?}");
    assert_eq!(vec![r#""bo""#], stdout(&out));
    assert_eq!(1, stderr(&out).lines().count(), "{out:?}");
}

#[test]
fn a_blank_line_runs_an_unfinished_text_so_its_error_shows() {
    let (schema, db) = scratch("blank");
    create(&schema, &db);
    let script = "from u in users\n\nfrom u in users select u.name\n";
    let out = adtb(&[db.to_str().unwrap()], script);
    assert_eq!(Some(1), out.status.code(), "{out:?}");
    assert_eq!(1, stderr(&out).lines().count(), "{out:?}");
}

#[test]
fn a_transact_block_runs_on_until_a_blank_line() {
    let (schema, db) = scratch("transact");
    create(&schema, &db);
    let script = r#"transact
  insert users { name: "ann" }
  insert users { name: "bo" }

from u in users select u.name
"#;
    let out = adtb(&[db.to_str().unwrap()], script);
    assert!(out.status.success(), "{out:?}");
    let mut rows = stdout(&out);
    rows.sort();
    assert_eq!(vec![r#""ann""#, r#""bo""#], rows);
}

#[test]
fn an_interactive_transaction_spans_texts() {
    let (schema, db) = scratch("interactive");
    create(&schema, &db);
    let script = r#"\begin
insert users { name: "ann" }
insert users { name: "bo" }
from u in users select u.name order by u.name
\commit
\begin
insert users { name: "cy" }
\rollback
from u in users select u.name order by u.name
"#;
    let out = adtb(&[db.to_str().unwrap()], script);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        vec![r#""ann""#, r#""bo""#, r#""ann""#, r#""bo""#],
        stdout(&out)
    );
}

#[test]
fn a_failed_text_inside_a_transaction_leaves_it_open() {
    let (schema, db) = scratch("txn-fails");
    create(&schema, &db);
    let script = r#"\begin
insert users { name: "ann" }
insert users { name: 1 }
\commit
from u in users select u.name
"#;
    let out = adtb(&[db.to_str().unwrap()], script);
    assert_eq!(Some(1), out.status.code(), "{out:?}");
    assert_eq!(vec![r#""ann""#], stdout(&out));
}

#[test]
fn commit_without_a_transaction_fails() {
    let (schema, db) = scratch("no-txn");
    create(&schema, &db);
    let out = adtb(&[db.to_str().unwrap()], "\\commit\n");
    assert_eq!(Some(1), out.status.code(), "{out:?}");
    assert_eq!("no transaction is open\n", stderr(&out));
}

#[test]
fn a_transaction_open_at_the_end_of_input_is_rolled_back() {
    let (schema, db) = scratch("dangling");
    create(&schema, &db);
    let out = adtb(
        &[db.to_str().unwrap()],
        "\\begin\ninsert users { name: \"ann\" }\n",
    );
    assert!(out.status.success(), "{out:?}");
    assert_eq!("the open transaction is rolled back\n", stderr(&out));
    let out = run(&db, "from u in users select u.name");
    assert!(stdout(&out).is_empty(), "{out:?}");
}

#[test]
fn quit_stops_reading() {
    let (schema, db) = scratch("quit");
    create(&schema, &db);
    let out = adtb(
        &[db.to_str().unwrap()],
        "\\q\ninsert users { name: \"ann\" }\n",
    );
    assert!(out.status.success(), "{out:?}");
    let out = run(&db, "from u in users select u.name");
    assert!(stdout(&out).is_empty(), "{out:?}");
}

#[test]
fn committed_rows_survive_reopening_the_file() {
    let (schema, db) = scratch("reopen");
    create(&schema, &db);
    let out = adtb(
        &[db.to_str().unwrap()],
        "\\begin\ninsert users { name: \"ann\" }\n\\commit\n",
    );
    assert!(out.status.success(), "{out:?}");
    let out = run(&db, r#"insert users { name: "bo" }"#);
    assert!(out.status.success(), "{out:?}");
    let out = run(&db, "from u in users select u.name order by u.name");
    assert!(out.status.success(), "{out:?}");
    assert_eq!(vec![r#""ann""#, r#""bo""#], stdout(&out));
}

#[test]
fn opening_a_missing_file_without_create_fails() {
    let (_, db) = scratch("missing");
    let out = run(&db, "from u in users select u.name");
    assert_eq!(Some(1), out.status.code(), "{out:?}");
    assert!(
        stderr(&out).ends_with("the file does not exist\n"),
        "{out:?}"
    );
}

#[test]
fn a_bad_argument_prints_the_usage() {
    let out = adtb(&["--nope"], "");
    assert_eq!(Some(2), out.status.code(), "{out:?}");
    assert!(stderr(&out).contains("usage: adtb FILE"), "{out:?}");
}
