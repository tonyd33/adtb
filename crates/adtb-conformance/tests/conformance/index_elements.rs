//! Which types an index, unique or interval index element may have: one whose type contains no
//! set or function type at any depth is admitted and keys its rows, and any other is refused when
//! the schema is checked, naming its type. The cases are `tests/index-elements.txt`: type
//! constructor, depth, the clauses it runs as, the field `v` of the row type or `-`, the insert of
//! one row with `$` for its id, the element, and `admitted`, the refusal, or `entries:` and the
//! engine's refusal. An index element may be a set of such a type, one entry for each element:
//! the reference admits it and the engine refuses it as a feature it does not run yet. The nextest
//! profile `deep` takes each case as each of its clauses, on the reference and on the engine, and
//! other profiles as one of them, taken in turn. A unique refuses a second row with an equal
//! element.

use std::collections::BTreeSet;

use adtb::{CreateOptions, Database, Error, Log, OpenError, OpenOptions, RejectReason};
use adtb_conformance::{Script, parse, run_engine, run_reference};
use adtb_lang::check::check_schema;
use adtb_lang::parse::{Options, parse_schema};
use adtb_lang::schema::build;
use adtb_lang::span::{Diag, FileDiag, FileId, Span};
use adtb_sim::{Config, Outcome, Sim};
use adtb_testkit::profile;

const CONSTRUCTORS: [&str; 28] = [
    "base:I8",
    "base:I16",
    "base:I32",
    "base:I64",
    "base:Decimal",
    "base:F32",
    "base:F64",
    "base:Text",
    "base:Bytes",
    "base:SortKey",
    "base:Instant",
    "base:Date",
    "base:Time",
    "base:DateTime",
    "base:ZonedDateTime",
    "base:Duration",
    "base:Period",
    "sum",
    "record",
    "newtype",
    "option",
    "reference",
    "tag",
    "anon-record",
    "anon-sum",
    "unit",
    "set",
    "function",
];

/// How many admitted cases one script holds.
const CHUNK: usize = 6;

const DEPTHS: [&str; 3] = ["bare", "record", "sum"];

/// The cases beyond every constructor at every depth.
const BEYOND: [(&str, &str); 4] = [
    ("set", "anon-sum"),
    ("set-of-record", "bare"),
    ("set-of-set", "bare"),
    ("nonempty", "record"),
];

const DECLARATIONS: &str = "  module test
  type Pt = { x: I64, y: I64 }
  type Email = new Text
  type Shape = Dot | Line { len: I64 }
  type Other = { id: Key others }
  relation others : Other key id generate sequence
  fn anon (n: I64) -> (A { n: I64 } | B) = A { n: n }
  fn anonset (n: I64) -> (A { n: Set I64 } | B) = A { n: [n] }
";

struct Case {
    name: String,
    ctor: String,
    depth: String,
    field: String,
    insert: String,
    element: String,
    clause: String,
    expected: String,
}

/// Returns the declarations of relation `items{k}` holding the case's element in its clause.
fn relation(case: &Case, k: usize) -> String {
    let v = match case.field.as_str() {
        "-" => String::new(),
        f => format!(", {f}"),
    };
    format!(
        "  type Item{k} = {{ id: I64, n: I64{v} }}\n  relation items{k} : Item{k} key id\n    {} ({})\n",
        case.clause, case.element
    )
}

/// Returns each message of the checker's refusal of `text`, none when it is admitted.
fn refusal(text: &str) -> Vec<String> {
    let file = parse_schema(text, Options::default())
        .ok()
        .unwrap_or_else(|d| panic!("{text}: {d:?}"));
    let schema = build(&[(FileId(0), file)]).unwrap_or_else(|d| panic!("{text}: {d:?}"));
    match check_schema(&schema) {
        Ok(_) => Vec::new(),
        Err(diags) => diags.into_iter().map(|d| d.diag.message).collect(),
    }
}

/// Returns every difference of `script` on the reference and on the engine, with its line.
fn differences(script: &Script) -> Vec<(usize, String)> {
    let mut found: Vec<_> = run_reference(script)
        .into_iter()
        .map(|d| (d.line, format!("reference: {d}")))
        .collect();
    let mut engine = Vec::new();
    let outcome = Sim::new(0, Config::default()).run(|rt| engine = run_engine(rt, script));
    if outcome != Outcome::Finished {
        found.push((0, format!("engine: {outcome:?}")));
    }
    found.extend(engine.into_iter().map(|d| (d.line, format!("engine: {d}"))));
    found
}

/// Returns the directives storing one row of `items{k}`, then a second with an equal element:
/// kept by an index, refused by a unique.
fn stored(case: &Case, k: usize) -> String {
    let row = |id: &str| format!("  insert items{k} {}\n", case.insert.replace('$', id));
    let (second, count) = match case.clause.as_str() {
        "unique" => ("error Violated", 1),
        _ => ("run", 2),
    };
    format!(
        "run\n{}{second}\n{}query\n  count(i for i in items{k})\n----\n{count}\n",
        row("1"),
        row("2"),
    )
}

/// Returns the migration adding the module of the schema `body`, each declaration in its order,
/// so the engine judges declarations the checker refuses.
fn migration(body: &str) -> String {
    let mut out = String::new();
    for line in body.lines() {
        match line.split_whitespace().next() {
            Some("module") => out.push_str(&format!("{line}\nadd module {}\n", &line[7..])),
            Some("fn") => out.push_str(&format!("define {line}\n")),
            _ if line.starts_with(' ') => out.push_str(&format!("{line}\n")),
            _ => out.push_str(&format!("add {line}\n")),
        }
    }
    out
}

/// Returns the detail of the engine's refusal of the migration of `text`.
fn engine_refusal(text: &str) -> Result<String, String> {
    let body: String = text.lines().map(|l| format!("{}\n", &l[2..])).collect();
    let text = migration(&body);
    let log = Log::new([text.as_str()]);
    let options = OpenOptions {
        create: Some(CreateOptions::default()),
        ..OpenOptions::default()
    };
    let mut got = Err("the engine did not run".to_owned());
    let outcome = Sim::new(0, Config::default()).run(|rt| {
        got = match Database::open_in(rt, "index.adt", &log, &options) {
            Err(OpenError::MigrationRefused {
                version: 1,
                reason:
                    Error::Rejected {
                        reason: RejectReason::SchemaRefused { operator, detail },
                    },
            }) if operator.is_empty() || text.lines().any(|l| l.trim() == operator) => Ok(detail),
            Err(other) => Err(format!("refused with {other:?}")),
            Ok(db) => {
                drop(db.close());
                Err("admitted".to_owned())
            }
        };
    });
    if outcome != Outcome::Finished {
        return Err(format!("{outcome:?}"));
    }
    got
}

/// Returns the reference's difference for the schema `text` refused at the case's element.
fn reference_refusal(text: &str, case: &Case) -> String {
    let body: String = text.lines().map(|l| format!("{}\n", &l[2..])).collect();
    let start = body
        .rfind(&format!("({})", case.element))
        .expect("the clause holds the element")
        + 1;
    let span = Span {
        start: u32::try_from(start).expect("a schema text is short"),
        end: u32::try_from(start + case.element.len()).expect("a schema text is short"),
    };
    let refused = vec![FileDiag {
        file: FileId(0),
        diag: Diag::new(span, case.expected.clone()),
    }];
    format!("line 1: the schema is refused: {refused:?}")
}

fn cases() -> Vec<Case> {
    let text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/index-elements.txt"),
    )
    .expect("the index element cases are readable");
    let mut out = Vec::new();
    for (row, line) in text.lines().enumerate() {
        let [ctor, depth, clauses, field, insert, element, expected] =
            line.split('\t').collect::<Vec<_>>()[..]
        else {
            panic!(
                "a case is constructor, depth, clauses, field, insert, element, outcome: {line}"
            );
        };
        let clauses: Vec<&str> = clauses.split(',').collect();
        let clauses = if profile::every_case() {
            &clauses[..]
        } else {
            std::slice::from_ref(&clauses[row % clauses.len()])
        };
        out.extend(clauses.iter().map(|&clause| Case {
            name: format!("{ctor} {depth} {clause}"),
            ctor: ctor.to_owned(),
            depth: depth.to_owned(),
            field: field.to_owned(),
            insert: insert.to_owned(),
            element: element.to_owned(),
            clause: clause.to_owned(),
            expected: expected.to_owned(),
        }));
    }
    out
}

#[test]
fn an_index_element_is_admitted_exactly_when_its_type_has_a_key_form() {
    let mut failures = Vec::new();
    let mut matched = BTreeSet::new();
    let mut missed = BTreeSet::new();
    let mut admitted = Vec::new();
    let cases = cases();
    for (k, case) in cases.iter().enumerate() {
        let text = format!("{DECLARATIONS}{}", relation(case, 0));
        let refused = refusal(&text);
        let at = (case.ctor.clone(), case.depth.clone());
        if let Some(detail) = case.expected.strip_prefix("entries:") {
            let engine = engine_refusal(&text);
            let script = format!(
                "schema\n{text}run\n  insert others {{ id: others.key(1) }}\n{}",
                stored(case, 0)
            );
            let reference: Vec<String> = run_reference(&parse(&script).expect("the script reads"))
                .into_iter()
                .map(|d| d.to_string())
                .collect();
            if refused.is_empty() && engine.as_deref() == Ok(detail) && reference.is_empty() {
                matched.insert(at);
            } else {
                failures.push(format!(
                    "{}: checker {refused:?}, engine {engine:?}, reference {reference:?}",
                    case.name
                ));
                missed.insert(at);
            }
            continue;
        }
        if case.expected == "admitted" {
            if refused.is_empty() {
                admitted.push((k, case));
                matched.insert(at);
            } else {
                failures.push(format!("{}: refused by {refused:?}", case.name));
                missed.insert(at);
            }
            continue;
        }
        let engine = engine_refusal(&text);
        let reference: Vec<String> =
            run_reference(&parse(&format!("schema\n{text}")).expect("the script reads"))
                .into_iter()
                .map(|d| d.to_string())
                .collect();
        if refused == [case.expected.clone()]
            && engine.as_deref() == Ok(case.expected.as_str())
            && reference == [reference_refusal(&text, case)]
        {
            matched.insert(at);
        } else {
            failures.push(format!(
                "{}: checker {refused:?}, engine {engine:?}, reference {reference:?}",
                case.name
            ));
            missed.insert(at);
        }
    }
    for chunk in admitted.chunks(CHUNK) {
        let mut schema = DECLARATIONS.to_owned();
        let mut runs = "run\n  insert others { id: others.key(1) }\n".to_owned();
        let mut starts = Vec::new();
        for &(k, case) in chunk {
            schema.push_str(&relation(case, k));
            starts.push((runs.lines().count(), case));
            runs.push_str(&stored(case, k));
        }
        let offset = schema.lines().count() + 1;
        let script = parse(&format!("schema\n{schema}{runs}")).unwrap_or_else(|d| panic!("{d}"));
        for (line, found) in differences(&script) {
            let i = starts.partition_point(|(start, _)| start + offset < line);
            let cases = match i.checked_sub(1) {
                Some(i) => &starts[i..=i],
                None => &starts[..],
            };
            for (_, case) in cases {
                failures.push(format!("{}: {found}", case.name));
                missed.insert((case.ctor.clone(), case.depth.clone()));
            }
        }
    }
    assert_eq!(Vec::<String>::new(), failures);
    let reached: BTreeSet<_> = matched.difference(&missed).cloned().collect();
    let every: BTreeSet<_> = CONSTRUCTORS
        .into_iter()
        .flat_map(|c| DEPTHS.map(|d| (c.to_owned(), d.to_owned())))
        .chain(BEYOND.map(|(c, d)| (c.to_owned(), d.to_owned())))
        .collect();
    assert_eq!(every, reached);
}
