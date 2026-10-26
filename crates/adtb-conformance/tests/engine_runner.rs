//! The runner's judgement of the engine's outcomes, over the simulator.

use adtb_conformance::{parse, run_engine};
use adtb_sim::{Config, Outcome, Sim};

const SCHEMA: &str = "schema
  module test
  type T = { id: Key ts, n: I64 }
  relation ts : T key id generate sequence
run
  insert ts { id: ts.key(1), n: 2 }
  insert ts { id: ts.key(2), n: 1 }
";

fn engine(script: &str) -> Vec<String> {
    let script = parse(script).expect("the script reads");
    let mut found = Vec::new();
    let outcome = Sim::new(0, Config::default()).run(|rt| found = run_engine(rt, &script));
    assert_eq!(Outcome::Finished, outcome);
    found.into_iter().map(|d| d.message).collect()
}

fn differences(rest: &str) -> Vec<String> {
    engine(&format!("{SCHEMA}{rest}"))
}

#[test]
fn matching_rows_and_reasons_pass() {
    let found = differences(
        "query
  from t in ts select t.n
----
1
2
query
  count(t for t in ts)
----
2
error KeyViolation
  insert ts { id: ts.key(1), n: 5 }
query rowsort
  from t in ts select t.n
----
2
1
",
    );
    assert_eq!(Vec::<String>::new(), found);
}

#[test]
fn a_query_with_other_rows_differs() {
    let found = differences("query rowsort\n  from t in ts select t.n\n----\n1\n3\n");
    assert_eq!(vec!["got [1, 2]".to_string()], found);
}

#[test]
fn a_set_result_without_rowsort_is_in_printed_order() {
    assert_eq!(
        1,
        differences("query\n  from t in ts select t.n\n----\n2\n1\n").len()
    );
}

#[test]
fn a_refusal_with_another_reason_differs() {
    let found = differences("error Violated\n  insert ts { id: ts.key(1), n: 5 }\n");
    assert_eq!(1, found.len());
    assert!(
        found[0].starts_with("refused with KeyViolation"),
        "{found:?}"
    );
}

#[test]
fn a_statement_that_succeeds_where_a_refusal_is_expected_differs() {
    let found = differences("error KeyViolation\n  insert ts { id: ts.key(3), n: 5 }\n");
    assert_eq!(1, found.len());
    assert!(found[0].starts_with("succeeds"), "{found:?}");
}

#[test]
fn text_that_does_not_check_is_refused_as_invalid() {
    assert_eq!(
        Vec::<String>::new(),
        differences("error Invalid\n  from t in ts select t.m\n")
    );
    assert_eq!(
        Vec::<String>::new(),
        differences("error Invalid\n  from t in ts select t.m\n  insert ts { n: 1 }\n")
    );
    assert_eq!(
        Vec::<String>::new(),
        differences("error Invalid\n  from t in ) ts\n")
    );
    let found = differences("query\n  from t in ts select t.m\n----\n1\n");
    assert!(found[0].starts_with("refused with Invalid"), "{found:?}");
}

#[test]
fn a_query_that_writes_differs() {
    assert_eq!(
        1,
        differences("query\n  insert ts { id: ts.key(3), n: 5 }\n----\n()\n").len()
    );
}

#[test]
fn a_query_of_two_statements_differs() {
    let found = differences("query\n  count(t for t in ts)\n  (count(t for t in ts))\n----\n2\n");
    assert_eq!(vec!["a query holds one statement".to_string()], found);
}

#[test]
fn an_ordered_result_compares_in_order() {
    let q = "query\n  from t in ts select t.n order by t.n desc\n----\n";
    assert_eq!(Vec::<String>::new(), differences(&format!("{q}2\n1\n")));
    assert_eq!(1, differences(&format!("{q}1\n2\n")).len());
}

#[test]
fn only_the_last_statement_of_an_error_may_be_refused() {
    let found = differences(
        "error KeyViolation\n  insert ts { id: ts.key(1), n: 5 }\n  insert ts { id: ts.key(4), n: 5 }\n",
    );
    assert_eq!(1, found.len());
    assert!(
        found[0].starts_with("a statement before the last"),
        "{found:?}"
    );
    assert_eq!(
        Vec::<String>::new(),
        differences(
            "error KeyViolation\n  insert ts { id: ts.key(4), n: 5 }\n  insert ts { id: ts.key(4), n: 6 }\n"
        )
    );
}

#[test]
fn nothing_an_error_directive_runs_commits() {
    let found = differences(
        "error KeyViolation\n  insert ts { id: ts.key(4), n: 5 }\n  insert ts { id: ts.key(1), n: 6 }\nquery\n  count(t for t in ts)\n----\n2\n",
    );
    assert_eq!(Vec::<String>::new(), found);
}

#[test]
fn a_failing_run_differs() {
    let found = differences("run\n  insert ts { id: ts.key(1), n: 5 }\n");
    assert_eq!(1, found.len());
    assert!(
        found[0].starts_with("refused with KeyViolation"),
        "{found:?}"
    );
}

#[test]
fn a_schema_the_engine_refuses_differs() {
    let found = engine("schema\n  module test\n  type T = { id: Key nowhere }\n");
    assert_eq!(1, found.len());
    assert!(
        found[0].starts_with("the database does not open"),
        "{found:?}"
    );
}

#[test]
fn a_script_without_a_schema_holds_the_prelude() {
    assert_eq!(
        Vec::<String>::new(),
        engine("query\n  Some { value: 1 }\n----\nSome { value: 1 }\n")
    );
}

#[test]
fn an_ordered_result_with_a_repeated_expected_row_differs_as_a_script_error() {
    for head in ["query", "query rowsort"] {
        let found = differences(&format!(
            "{head}\n  from t in ts select t.n order by t.n desc\n----\n2\n1\n1\n"
        ));
        assert_eq!(
            vec!["expected row 3 repeats row 2; a result holds each row once".to_string()],
            found,
            "{head}"
        );
    }
}

#[test]
fn a_set_result_with_a_repeated_expected_row_differs_as_a_script_error() {
    let found = differences("query rowsort\n  from t in ts select t.n\n----\n1\n2\n2\n");
    assert_eq!(
        vec!["expected row 3 repeats row 2; a result holds each row once".to_string()],
        found
    );
}
