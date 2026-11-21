//! A constraint or row check reads the state after a statement, which is not known to satisfy its
//! references: an unused deref of a reference field of every row it reads faults `NoSuchRow` when
//! the key names no row, on the reference and on the engine. Each row source is a declaration, a
//! statement leaving the key naming no row, and one leaving it naming a row.

use std::collections::BTreeSet;

use adtb_conformance::{Difference, Got, parse, run_engine, run_reference};
use adtb_sim::{Config, Outcome, Sim};
use adtb_testkit::sections::sections;

const SCHEMA: &str = "module test
type User = { id: Key users, name: Text, boss: Key users }
type Phone = { id: Key phones, owner: Key users, seen: I64 }
relation users : User key id generate sequence
relation phones : Phone key id generate sequence
  computed probe: I64 = { a: self.owner.name, b: self.seen }.b
fn all_phones () -> Set Phone = from p in phones select p
fn seens () -> Set I64 = from p in phones select { a: p.owner.name, b: p.seen }.b
";

const ROWS: &str = "insert users { id: users.key(1), name: \"a\", boss: users.key(1) }
insert phones { id: phones.key(1), owner: users.key(1), seen: 1 }
";

/// Every row source of a constraint or a check, by its section in the fixture.
const SOURCES: [&str; 12] = [
    "relation",
    "inserted",
    "deleted",
    "prior",
    "function-rows",
    "function-body",
    "computed",
    "deref",
    "check-self",
    "check-field",
    "check-computed",
    "check-deref",
];

fn indented(text: &str) -> String {
    text.lines().map(|l| format!("  {l}\n")).collect()
}

/// Returns how the statements of the script declaring `declaration` and running `dangling` then
/// `named` differ from succeeding, on the reference and on the engine.
///
/// Postconditions:
/// - every difference is at the `run` of `dangling`.
fn differences(declaration: &str, dangling: &str, named: &str) -> (Vec<Got>, Vec<Got>) {
    let text = format!(
        "schema\n{}run\n{}run\n{}run\n{}",
        indented(&format!("{SCHEMA}{declaration}")),
        indented(ROWS),
        indented(dangling),
        indented(named)
    );
    let script = parse(&text).unwrap_or_else(|d| panic!("{d}"));
    let at = script.directives()[2].line();
    let mut engine = Vec::new();
    let outcome = Sim::new(0, Config::default()).run(|rt| engine = run_engine(rt, &script));
    assert_eq!(Outcome::Finished, outcome);
    let side = |found: Vec<Difference>| {
        found
            .into_iter()
            .map(|d| match d.got {
                Some(got) if d.line == at => got,
                _ => panic!("{text}: {d}"),
            })
            .collect()
    };
    (side(run_reference(&script)), side(engine))
}

fn no_such_row(detail: &str) -> Vec<Got> {
    vec![Got::Refused {
        reason: "Faulted".into(),
        detail: detail.into(),
    }]
}

#[test]
fn an_unused_deref_through_every_row_a_check_reads_faults_on_a_key_naming_no_row() {
    let cases = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/unchecked-state.txt"),
    )
    .expect("the row sources are readable");
    let mut reached = BTreeSet::new();
    let mut failures = Vec::new();
    for s in sections(&cases) {
        let name = s.kind.as_str();
        let (reference, engine) = differences(&s.text, s.part("dangling"), s.part("named"));
        if reference != no_such_row("NoSuchRow") || engine != no_such_row("fault `NoSuchRow`") {
            failures.push(format!(
                "{name}: reference {reference:?}, engine {engine:?}"
            ));
        }
        reached.insert(name.to_owned());
    }
    assert_eq!(Vec::<String>::new(), failures);
    assert_eq!(BTreeSet::from(SOURCES.map(str::to_owned)), reached);
}
