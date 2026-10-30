//! The `adtb` binary run against a real database file.

use std::io::{Read, Write};
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
fn recent_commits_are_listed_and_readable_after_reopen() {
    let (schema, db) = scratch("recent");
    create(&schema, &db);
    let out = adtb(
        &[db.to_str().unwrap()],
        "set setting RetentionWindow = { length: 1h }\ninsert users { name: \"ann\" }\n",
    );
    assert!(out.status.success(), "{out:?}");
    let out = run(
        &db,
        "from c in sys::commit_times, n in (at commit c.position { from u in users select u.name }) select n",
    );
    assert!(out.status.success(), "{out:?}");
    assert_eq!(vec![r#""ann""#], stdout(&out));
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
    assert_eq!(
        "constraint `test::identities.unique(gid) in Google` is violated by 2 rows\n",
        err
    );
}

#[test]
fn a_text_that_does_not_check_is_rejected_as_invalid() {
    let (schema, db) = scratch("invalid");
    create(&schema, &db);
    let out = run(&db, "from u in nowhere select u");
    assert_eq!(Some(1), out.status.code(), "{out:?}");
    assert!(
        stderr(&out).starts_with("`nowhere` is a parameter of this text"),
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
    assert!(stderr(&out).contains("Usage: adtb "), "{out:?}");
}

const PETS: &str = "module demo
relation pets : Pet key name
type Pet { name: Text } = Cat { lives: I64 } | Dog { good: Bool }
";

/// Returns a fresh directory holding `name.adt` with `text` for each of `files`, and the path
/// of a database file in it.
fn scratch_with(dir: &str, files: &[(&str, &str)]) -> (PathBuf, PathBuf) {
    let (_, db) = scratch(dir);
    let dir = db.parent().expect("a directory").to_path_buf();
    for (name, text) in files {
        std::fs::write(dir.join(format!("{name}.adt")), text).unwrap();
    }
    (dir, db)
}

#[test]
fn a_schema_file_of_declarations_creates_a_database() {
    let (dir, db) = scratch_with("declarations", &[("pets", PETS)]);
    let schema = dir.join("pets.adt");
    create(&schema, &db);
    let out = run(&db, r#"insert pets.Cat { name: "tom", lives: 9 }"#);
    assert!(out.status.success(), "{out:?}");
    let out = run(&db, "from p in pets select p");
    assert_eq!(vec![r#"Cat { name: "tom", lives: 9 }"#], stdout(&out));
}

#[test]
fn a_later_schema_flag_applies_the_next_migration() {
    let added = "module demo\nadd field Pet.Cat.age: I64 fill 0\n";
    let (dir, db) = scratch_with("next", &[("pets", PETS), ("age", added)]);
    let (pets, age) = (dir.join("pets.adt"), dir.join("age.adt"));
    create(&pets, &db);
    let out = run(&db, r#"insert pets.Cat { name: "tom", lives: 9 }"#);
    assert!(out.status.success(), "{out:?}");
    let out = adtb(
        &[
            "--schema",
            pets.to_str().unwrap(),
            "--schema",
            age.to_str().unwrap(),
            db.to_str().unwrap(),
            "-c",
            "from p in pets select p",
        ],
        "",
    );
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        vec![r#"Cat { name: "tom", lives: 9, age: 0 }"#],
        stdout(&out)
    );
}

#[test]
fn a_refused_create_leaves_nothing_at_the_path() {
    let bad =
        "module demo\ntype P = { id: Key ps, n: Int }\nrelation ps : P key id generate sequence\n";
    let (dir, db) = scratch_with("refused", &[("bad", bad)]);
    let out = adtb(
        &[
            "--create",
            "--schema",
            dir.join("bad.adt").to_str().unwrap(),
            db.to_str().unwrap(),
        ],
        "",
    );
    assert_eq!(Some(1), out.status.code(), "{out:?}");
    assert!(!db.exists(), "{db:?}");
    assert!(!dir.join("db.adt-log").exists(), "{dir:?}");
}

#[test]
fn a_refused_migration_prints_one_line_per_message() {
    let bad =
        "module demo\ntype P = { id: Key ps, n: Int }\nrelation ps : P key id generate sequence\n";
    let (dir, db) = scratch_with("messages", &[("bad", bad)]);
    let out = adtb(
        &[
            "--create",
            "--schema",
            dir.join("bad.adt").to_str().unwrap(),
            db.to_str().unwrap(),
        ],
        "",
    );
    let err = stderr(&out);
    assert!(
        err.contains(":\n  no type `Int`; did you mean `I64`?\n"),
        "{err}"
    );
    assert!(!err.contains("Rejected"), "{err}");
}

#[test]
fn a_schema_file_that_does_not_parse_reports_where() {
    let bad = "module demo\ntype P = { id: Key ps\nrelation ps : P key id\n";
    let (dir, db) = scratch_with("parse", &[("bad", bad)]);
    let path = dir.join("bad.adt");
    let out = adtb(
        &[
            "--create",
            "--schema",
            path.to_str().unwrap(),
            db.to_str().unwrap(),
        ],
        "",
    );
    assert_eq!(Some(1), out.status.code(), "{out:?}");
    assert_eq!(
        format!("{}: 3:1: expected `}}`, found `relation`\n", path.display()),
        stderr(&out)
    );
    assert!(!db.exists(), "{db:?}");
}

#[test]
fn an_unqualified_name_two_modules_declare_says_to_qualify_it() {
    let a = "module a\ntype X = { id: Key xs }\nrelation xs : X key id generate sequence\n";
    let b = "module b\nadd module b\nadd type X = { id: Key xs }\nadd relation xs : X key id generate sequence\n";
    let (dir, db) = scratch_with("ambiguous", &[("a", a), ("b", b)]);
    let out = adtb(
        &[
            "--create",
            "--schema",
            dir.join("a.adt").to_str().unwrap(),
            "--schema",
            dir.join("b.adt").to_str().unwrap(),
            db.to_str().unwrap(),
            "-c",
            "from x in xs select x",
        ],
        "",
    );
    assert_eq!(Some(1), out.status.code(), "{out:?}");
    assert_eq!(
        "`xs` is declared in modules `a` and `b`; qualify it\n",
        stderr(&out)
    );
    let out = run(&db, "from x in a::xs select x");
    assert!(out.status.success(), "{out:?}");
}

#[test]
fn help_prints_the_flags() {
    let out = adtb(&["--help"], "");
    assert!(out.status.success(), "{out:?}");
    let help = String::from_utf8(out.stdout).unwrap();
    assert!(help.contains("Usage: adtb [OPTIONS] <FILE>"), "{help}");
    assert!(help.contains("--schema <FILE>"), "{help}");
}

#[test]
fn backslash_help_lists_the_commands() {
    let (schema, db) = scratch("help");
    create(&schema, &db);
    let out = adtb(&[db.to_str().unwrap()], "\\help\n");
    assert!(out.status.success(), "{out:?}");
    let help = stdout(&out);
    for command in ["\\begin", "\\commit", "\\rollback", "\\help", "\\q"] {
        assert!(
            help.iter().any(|l| l.starts_with(command)),
            "{command} in {help:?}"
        );
    }
}

#[test]
fn verify_of_a_sound_file_prints_nothing_and_succeeds() {
    let (schema, db) = scratch("verify");
    create(&schema, &db);
    seed(&db);
    let out = adtb(&["verify", db.to_str().unwrap()], "");
    assert!(out.status.success(), "{out:?}");
    assert!(stdout(&out).is_empty(), "{out:?}");
}

#[test]
fn verify_exits_1_on_a_finding_and_2_when_the_file_refuses_open() {
    let (schema, db) = scratch("verify-damage");
    create(&schema, &db);
    seed(&db);
    let sound = std::fs::read(&db).unwrap();
    let log = PathBuf::from(format!("{}-log", db.display()));
    let segments: Vec<(PathBuf, Vec<u8>)> = std::fs::read_dir(&log)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let bytes = std::fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect();
    let restore_log = || {
        for (path, bytes) in &segments {
            std::fs::write(path, bytes).unwrap();
        }
    };
    let last = sound.len() / 8192 - 1;
    for page in [last, last / 2, last / 4, 12, 9, 4] {
        let mut flipped = sound.clone();
        flipped[page * 8192 + 300] ^= 0x10;
        std::fs::write(&db, &flipped).unwrap();
        restore_log();
        let out = adtb(&["verify", db.to_str().unwrap()], "");
        assert_eq!(Some(1), out.status.code(), "page {page}: {out:?}");
        assert!(
            stdout(&out).iter().any(|r| r.starts_with("Checksum")),
            "page {page}: {out:?}"
        );
    }
    restore_log();
    let mut flipped = sound;
    flipped[..2 * 8192].fill(0x5a);
    std::fs::write(&db, &flipped).unwrap();
    let out = adtb(&["verify", db.to_str().unwrap()], "");
    assert_eq!(Some(2), out.status.code(), "{out:?}");
    assert!(!stderr(&out).is_empty(), "{out:?}");
}

#[test]
fn verify_exits_2_while_another_process_holds_the_file() {
    let (schema, db) = scratch("verify-held");
    create(&schema, &db);
    let mut holder = Command::new(env!("CARGO_BIN_EXE_adtb"))
        .arg(db.to_str().unwrap())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the shell starts");
    let mut input = holder.stdin.take().expect("a piped stdin");
    input.write_all(b"count(u for u in users)\n\n").unwrap();
    input.flush().unwrap();
    let mut first = [0u8; 1];
    holder
        .stdout
        .take()
        .expect("a piped stdout")
        .read_exact(&mut first)
        .expect("the holder has opened the file");
    let out = adtb(&["verify", db.to_str().unwrap()], "");
    drop(input);
    holder.wait().unwrap();
    assert_eq!(Some(2), out.status.code(), "{out:?}");
    assert!(stderr(&out).contains("the file is locked by"), "{out:?}");
}

#[test]
fn verify_of_a_missing_file_exits_2_and_says_why() {
    let (_, db) = scratch("verify-missing");
    let out = adtb(&["verify", db.to_str().unwrap()], "");
    assert_eq!(Some(2), out.status.code(), "{out:?}");
    assert!(stderr(&out).contains("db.adt"), "{out:?}");
}
