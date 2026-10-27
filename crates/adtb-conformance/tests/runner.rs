use adtb_conformance::{Got, Kind, parse, run_reference};

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

fn refused(script: &str) -> (usize, String) {
    let d = parse(script).expect_err("the script does not read");
    (d.line, d.message)
}

#[test]
fn an_indented_line_outside_a_body_does_not_read() {
    assert_eq!(
        (2, "an indented line outside a directive's body".to_string()),
        refused("# a comment\n  insert ts { id: 1, n: 2 }\n")
    );
}

#[test]
fn an_unknown_directive_does_not_read() {
    for line in ["querry", "query sorted", "error", "error A B", "run now"] {
        assert_eq!(
            (1, format!("`{line}` is not a directive")),
            refused(&format!("{line}\n  1\n")),
            "{line}"
        );
    }
}

#[test]
fn a_query_without_its_rule_does_not_read() {
    assert_eq!(
        (1, "a query's rows follow `----`".to_string()),
        refused("query\n  1\n1\n")
    );
}

#[test]
fn a_directive_with_no_body_does_not_read() {
    assert_eq!(
        (1, "a directive with no body".to_string()),
        refused("run\nquery\n  1\n----\n1\n")
    );
    assert_eq!(
        (1, "a directive with no body".to_string()),
        refused("query\n----\n1\n")
    );
}

#[test]
fn a_body_holds_blank_lines_and_rows_end_at_a_comment() {
    let s = parse("run\n  insert ts { id: 1, n: 2 }\n\n  insert ts { id: 2, n: 3 }\nquery\n  1\n----\n1\n# a comment\nquery\n  2\n----\n\nquery\n  3\n----\n3\n").expect("the script reads");
    assert_eq!(4, s.directives.len());
    assert_eq!(
        "insert ts { id: 1, n: 2 }\n\ninsert ts { id: 2, n: 3 }\n",
        s.directives[0].body
    );
    let rows = |i: usize| match &s.directives[i].kind {
        Kind::Query { rows, .. } => rows.clone(),
        k => panic!("directive {i} is {k:?}"),
    };
    assert_eq!(vec!["1".to_string()], rows(1));
    assert_eq!(Vec::<String>::new(), rows(2));
    assert_eq!(vec!["3".to_string()], rows(3));
}

#[test]
fn a_schema_after_another_directive_differs() {
    let found = differences("schema\n  module other\n  type U = { id: I64 }\n");
    assert_eq!(vec!["`schema` is the first directive".to_string()], found);
}

#[test]
fn a_refused_schema_differs() {
    let script = parse("schema\n  module test\n  type T = { id: Key nowhere }\n").unwrap();
    let found = run_reference(&script);
    assert_eq!(1, found.len());
    assert!(
        found[0].message.starts_with("the schema is refused"),
        "{found:?}"
    );
}

#[test]
fn a_script_without_a_schema_holds_the_prelude() {
    let script = parse("query\n  Some { value: 1 }\n----\nSome { value: 1 }\n").unwrap();
    assert_eq!(
        Vec::<String>::new(),
        run_reference(&script)
            .into_iter()
            .map(|d| d.message)
            .collect::<Vec<_>>()
    );
}

#[test]
fn an_expected_row_that_does_not_read_differs() {
    let found = differences("query rowsort\n  from t in ts select t.n\n----\n1\n\"two\"\n");
    assert_eq!(1, found.len());
    assert!(
        found[0].starts_with("expected row 2 `\"two\"` does not read"),
        "{found:?}"
    );
}

#[test]
fn a_set_result_with_a_repeated_expected_row_differs_as_a_script_error() {
    for head in ["query", "query rowsort"] {
        let found = differences(&format!(
            "{head}\n  from t in ts select t.n\n----\n1\n2\n2\n"
        ));
        assert_eq!(
            vec!["expected row 3 repeats row 2; a result holds each row once".to_string()],
            found,
            "{head}"
        );
    }
}

#[test]
fn a_query_of_two_statements_differs() {
    let found = differences("query\n  count(t for t in ts)\n  (count(t for t in ts))\n----\n2\n");
    assert_eq!(vec!["a query holds one statement".to_string()], found);
}

#[test]
fn an_error_whose_text_does_not_parse_is_refused_as_invalid() {
    assert_eq!(
        Vec::<String>::new(),
        differences("error Invalid\n  from t in ) ts\n")
    );
}

#[test]
fn a_query_with_no_rows_is_the_empty_set() {
    assert_eq!(
        Vec::<String>::new(),
        differences("query\n  from t in ts where t.n > 5 select t.n\n----\n\n")
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

fn got(rest: &str) -> Vec<Option<Got>> {
    let script = parse(&format!("{SCHEMA}{rest}")).expect("the script reads");
    run_reference(&script).into_iter().map(|d| d.got).collect()
}

#[test]
fn a_differing_query_carries_its_rows_and_a_refusal_its_reason() {
    let found = got("query rowsort
  from t in ts select t.n
----
3
query
  count(t for t in ts)
----
5
run
  insert ts { id: 1, n: 9 }
query
  from t in ts select 1 / (t.n - t.n)
----
");
    assert_eq!(4, found.len());
    assert_eq!(Some(Got::Rows(vec!["1".into(), "2".into()])), found[0]);
    assert_eq!(Some(Got::Rows(vec!["2".into()])), found[1]);
    let reasons: Vec<&str> = found[2..]
        .iter()
        .map(|g| match g {
            Some(Got::Refused { reason, .. }) => reason.as_str(),
            _ => panic!("a refusal carries its reason: {g:?}"),
        })
        .collect();
    assert_eq!(vec!["KeyViolation", "Faulted"], reasons);
}
