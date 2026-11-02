//! What a `let` on a single-row upsert binds, by the branch taken and the stored constructor. A
//! read of `w.body` is refused unless every completion binds a row of `Plain`; the bound row is read
//! on the reference and on the engine. The cases are `tests/upsert-results.txt`: name, stored
//! constructor, branch, read (`field`, `match` or `deleted`), the value bound or `!UserError`, and
//! the rows of `notes` after.

use std::collections::{BTreeMap, BTreeSet};

use adtb_conformance::{Script, parse, run_engine, run_reference};
use adtb_lang::check::{Program, check_adhoc, check_schema};
use adtb_lang::parse::{Options, parse_adhoc, parse_schema};
use adtb_lang::schema::{Schema, build};
use adtb_lang::span::FileId;
use adtb_sim::{Config, Outcome, Sim};

const NOTES: &str = "  module test
  type Note { id: Key notes } = Plain { body: Text } | Link { url: Text }
  relation notes : Note key id generate sequence
  type Copy = { id: Key copies, text: Text }
  relation copies : Copy key id generate sequence
";
const NO_BODY: &str = "Note has no field `body`; only header fields are reached on the whole sum: match on the constructor";
const DELETED: &str =
    "a `let` on an upsert binds exactly one row, and this else branch can delete it";
const FIELD: &str = "w.body";
const MATCHED: &str = "match w | Plain p -> p.body | Link l -> l.url";

fn transaction(branch: &str, read: &str) -> String {
    format!(
        "transact
  let w = insert notes.Plain {{ id: notes.key(1), body: \"new\" }}
    unless conflict on key else {branch}
  insert copies {{ id: copies.key(1), text: {read} }}
"
    )
}

fn checked() -> (Schema, Program) {
    let file = parse_schema(NOTES, Options::default())
        .ok()
        .expect("the schema parses");
    let schema = build(&[(FileId(0), file)]).expect("the schema builds");
    let program = check_schema(&schema).expect("the schema checks");
    (schema, program)
}

/// Returns the refusal of `text`, each message.
fn refusal(schema: &Schema, program: &Program, text: &str) -> Vec<String> {
    let items = parse_adhoc(text, Options::default())
        .ok()
        .unwrap_or_else(|d| panic!("{text}: {d:?}"));
    match check_adhoc(schema, program, &items, &BTreeMap::new()) {
        Ok(_) => Vec::new(),
        Err(diags) => diags.into_iter().map(|d| d.message).collect(),
    }
}

fn indented(text: &str) -> String {
    text.lines().map(|l| format!("  {l}\n")).collect()
}

/// Returns every difference of `script` on the reference and on the engine, with its line.
fn differences(script: &Script) -> Vec<(usize, String)> {
    let mut found: Vec<_> = run_reference(script)
        .into_iter()
        .map(|d| (d.line, format!("reference: {d}")))
        .collect();
    let mut engine = Vec::new();
    let outcome = Sim::new(0, Config::default()).run(|rt| engine = run_engine(rt, script));
    assert_eq!(Outcome::Finished, outcome);
    found.extend(engine.into_iter().map(|d| (d.line, format!("engine: {d}"))));
    found
}

/// Returns the text storing `stored` over empty relations, running `run`, then stating the copy
/// and the rows `after`.
fn case(stored: &str, run: &str, copy: Option<String>, after: &str) -> String {
    let row = match stored {
        "plain" => r#"insert notes.Plain { id: notes.key(1), body: "a" }"#,
        "link" => r#"insert notes.Link { id: notes.key(1), url: "u" }"#,
        other => panic!("a stored constructor is plain or link, not {other}"),
    };
    let mut text = format!(
        "run\n  delete n in notes\n  delete c in copies\nrun\n  {row}\n  insert notes.Plain {{ id: notes.key(9), body: \"other\" }}\n{run}query\n  from c in copies select c.text\n----\n"
    );
    text.extend(copy.into_iter().map(|c| c + "\n"));
    text.push_str("query rowsort\n  from n in notes select n\n----\n");
    text.extend(after.split(';').map(|r| format!("{r}\n")));
    text
}

#[test]
fn an_upsert_result_types_every_row_a_completed_call_can_bind() {
    let cases = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/upsert-results.txt"),
    )
    .expect("the upsert results are readable");
    let (schema, program) = checked();
    let mut reached = BTreeSet::new();
    let mut failures = Vec::new();
    let mut text = format!("schema\n{NOTES}");
    let mut starts = Vec::new();
    for line in cases.lines() {
        let [name, stored, branch, read, bound, after] = line.split('\t').collect::<Vec<_>>()[..]
        else {
            panic!("a case is name, stored, branch, read, bound, after: {line}");
        };
        let field = transaction(branch, FIELD);
        let matched = transaction(branch, MATCHED);
        let (field_refused, matched_refused, shown): (&[&str], &[&str], &str) = match read {
            "field" => (&[], &[], FIELD),
            "match" => (&[NO_BODY], &[], MATCHED),
            "deleted" => (&[DELETED], &[DELETED], ""),
            other => panic!("{name}: a read is field, match or deleted, not {other}"),
        };
        let refused = refusal(&schema, &program, &field);
        if refused != field_refused {
            failures.push(format!("{name}: the field read is refused by {refused:?}"));
        }
        let refused = refusal(&schema, &program, &matched);
        if read != "field" && refused != matched_refused {
            failures.push(format!("{name}: the match read is refused by {refused:?}"));
        }
        let mut run = String::new();
        if read != "field" {
            run.push_str(&format!("error Invalid\n{}", indented(&field)));
        }
        let mut copy = None;
        match (read, bound) {
            ("deleted", _) => {}
            (_, "!UserError") => {
                let t = transaction(branch, shown);
                run.push_str(&format!("error UserError\n{}", indented(&t)));
            }
            _ => {
                let t = transaction(branch, shown);
                run.push_str(&format!("run\n{}", indented(&t)));
                copy = Some(format!("\"{bound}\""));
            }
        }
        starts.push((text.lines().count(), name.to_owned()));
        text.push_str(&case(stored, &run, copy, after));
        reached.insert((read.to_owned(), stored.to_owned()));
    }
    let script = parse(&text).unwrap_or_else(|d| panic!("{d}"));
    for (line, found) in differences(&script) {
        let at = starts.partition_point(|(start, _)| *start < line);
        let name = at.checked_sub(1).map_or("the schema", |i| &starts[i].1);
        failures.push(format!("{name}: {found}"));
    }
    assert_eq!(Vec::<String>::new(), failures);
    let every: BTreeSet<_> = ["field", "match", "deleted"]
        .into_iter()
        .flat_map(|r| ["plain", "link"].map(|s| (r.to_owned(), s.to_owned())))
        .collect();
    assert_eq!(every, reached);
}
