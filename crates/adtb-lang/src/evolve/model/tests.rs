use super::*;
use crate::evolve::SourceDecl;
use crate::parse::{Options, parse_migration};

fn source(decls: &[(&str, &str)]) -> Source {
    let decls = decls
        .iter()
        .map(|(m, t)| SourceDecl {
            module: m.to_string(),
            text: t.to_string(),
        })
        .collect();
    Source::new(decls).expect("the source builds")
}

fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn expected(decls: &[(&str, &str)]) -> Vec<(String, String)> {
    decls
        .iter()
        .map(|(m, t)| (m.to_string(), flat(t)))
        .collect()
}

/// Returns each declaration the model of `src` prints after migration `text`, as its module
/// and its text with whitespace flattened.
fn migrated(src: &Source, text: &str) -> Vec<(String, String)> {
    let m = parse_migration(text, Options::default())
        .ok()
        .expect("the migration parses");
    let got = of(src)
        .expect("the source lifts")
        .migrate(&m, text)
        .expect("the model applies the migration");
    assert_eq!(got.stopped(), None, "the model applies every operator");
    assert!(
        got.ops().iter().all(Result::is_ok),
        "every operator applies: {:?}",
        got.ops()
    );
    assert!(got.checks(), "the migrated declarations check");
    got.model()
        .source_decls()
        .expect("the declarations print")
        .into_iter()
        .map(|d| (d.module, flat(&d.text)))
        .collect()
}

#[test]
fn rename_constructor_rewrites_only_selections_of_the_renamed_constructor() {
    let src = source(&[
        ("a", "type T = C | D"),
        ("b", "type T = C | D"),
        ("b", "fn f (x: T.C) -> I64 = 1"),
        ("a", "fn g (x: T.C) -> I64 = 1"),
    ]);
    assert_eq!(
        migrated(&src, "module a\nrename constructor T.C to Z"),
        expected(&[
            ("a", "type T = Z | D"),
            ("b", "type T = C | D"),
            ("b", "fn f (x: T.C) -> I64 = 1"),
            ("a", "fn g (x: T.Z) -> I64 = 1"),
        ])
    );
}

#[test]
fn rename_type_renames_only_the_constructor_of_a_record() {
    let src = source(&[
        ("a", "type T = T | D"),
        ("a", "type R = { x: I64 }"),
        ("a", "fn f (x: T.T) -> R = R { x: 1 }"),
    ]);
    assert_eq!(
        migrated(&src, "module a\nrename type T to U\nrename type R to S"),
        expected(&[
            ("a", "type U = T | D"),
            ("a", "type S = { x: I64 }"),
            ("a", "fn f (x: U.T) -> S = S { x: 1 }"),
        ])
    );
}

#[test]
fn a_scope_naming_a_dropped_set_names_the_set_added_under_its_name() {
    let src = source(&[
        ("a", "type T { id: Key ts, k: I64 } = C | D"),
        (
            "a",
            "relation ts: T key id generate sequence unique (k) in S",
        ),
        ("a", "set S of ts = C | D"),
    ]);
    assert_eq!(
        migrated(
            &src,
            "module a\ndrop set S of ts\nadd set S of ts = C\nrename set S of ts to V"
        ),
        expected(&[
            ("a", "type T { id: Key ts, k: I64 } = C | D"),
            (
                "a",
                "relation ts: T key id generate sequence unique (k) in V"
            ),
            ("a", "set V of ts = C"),
        ])
    );
}
