use adtb_conformance::{Kind, parse, run_reference};

const SCHEMA: &str = "schema
  module test
  type T = { id: I64, n: I64 }
  relation ts : T key id
run
  insert ts { id: 1, n: 2 }
  insert ts { id: 2, n: 1 }
";

fn differences(rest: &str) -> Vec<String> {
    let script = parse(&format!("{SCHEMA}{rest}")).expect("the script reads");
    run_reference(&script)
        .into_iter()
        .map(|d| d.message)
        .collect()
}

#[test]
fn a_script_reads_into_its_directives() {
    let s = parse(&format!(
        "{SCHEMA}# a comment\nquery rowsort\n  from t in ts\n    select t.n\n----\n1\n2\nerror Invalid\n  from t in ts select t.m\n"
    ))
    .unwrap();
    let kinds: Vec<&Kind> = s.directives.iter().map(|d| &d.kind).collect();
    assert_eq!(4, kinds.len());
    assert_eq!(
        &Kind::Query {
            rowsort: true,
            rows: vec!["1".into(), "2".into()]
        },
        kinds[2]
    );
    assert_eq!("from t in ts\n  select t.n\n", s.directives[2].body);
    assert_eq!(15, s.directives[3].line);
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
  insert ts { id: 1, n: 5 }
query
  from t in ts select t.n
----
1
2
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
fn rows_without_rowsort_are_in_printed_order() {
    let found = differences("query\n  from t in ts select t.n\n----\n2\n1\n");
    assert_eq!(1, found.len());
    assert!(found[0].contains("printed order"), "{found:?}");
    assert!(differences("query rowsort\n  from t in ts select t.n\n----\n2\n1\n").is_empty());
}

#[test]
fn a_refusal_with_another_reason_differs() {
    let found = differences("error Violated\n  insert ts { id: 1, n: 5 }\n");
    assert_eq!(1, found.len());
    assert!(
        found[0].starts_with("refused with KeyViolation, not Violated"),
        "{found:?}"
    );
}

#[test]
fn a_statement_that_succeeds_where_a_refusal_is_expected_differs() {
    let found = differences("error KeyViolation\n  insert ts { id: 3, n: 5 }\n");
    assert_eq!(1, found.len());
    assert!(found[0].starts_with("succeeds"), "{found:?}");
}

#[test]
fn text_that_does_not_check_is_refused_as_invalid() {
    assert!(differences("error Invalid\n  from t in ts select t.m\n").is_empty());
    let found = differences("query\n  from t in ts select t.m\n----\n1\n");
    assert!(found[0].starts_with("refused with Invalid"), "{found:?}");
}

#[test]
fn a_query_that_writes_differs() {
    let found = differences("query\n  insert ts { id: 3, n: 5 }\n----\n{}\n");
    assert_eq!(vec!["a query changes the database".to_string()], found);
}

#[test]
fn an_ordered_result_compares_in_order() {
    assert!(
        differences("query\n  from t in ts select t.n order by t.n desc\n----\n2\n1\n").is_empty()
    );
    assert_eq!(
        1,
        differences("query\n  from t in ts select t.n order by t.n desc\n----\n1\n2\n").len()
    );
}

#[test]
fn only_the_last_statement_of_an_error_may_be_refused() {
    let found = differences(
        "error KeyViolation\n  insert ts { id: 1, n: 5 }\n  insert ts { id: 4, n: 5 }\n",
    );
    assert_eq!(1, found.len());
    assert!(found[0].starts_with("statement 1 of 2"), "{found:?}");
    assert!(
        differences(
            "error KeyViolation\n  insert ts { id: 4, n: 5 }\n  insert ts { id: 4, n: 6 }\n"
        )
        .is_empty()
    );
}

#[test]
fn nothing_an_error_directive_runs_commits() {
    let found = differences(
        "error KeyViolation\n  insert ts { id: 4, n: 5 }\n  insert ts { id: 1, n: 6 }\nquery\n  count(t for t in ts)\n----\n2\n",
    );
    assert_eq!(Vec::<String>::new(), found);
}
