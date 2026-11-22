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

#[test]
fn a_field_moved_down_or_up_keeps_its_delete_action() {
    let src = source(&[
        ("m", "type R = { id: Key rs }"),
        ("m", "relation rs : R key id"),
        (
            "m",
            "type T { id: Key ts, p: Key rs on delete cascade } = A { a: I64 } | B { b: I64 }",
        ),
        ("m", "relation ts : T key id"),
        (
            "m",
            "type U { id: Key us } = C { q: Key rs on delete cascade }",
        ),
        ("m", "relation us : U key id"),
    ]);
    assert_eq!(
        migrated(
            &src,
            "module m\nmove field T.p to T.(A | B)\nmove field U.C.q to U"
        ),
        expected(&[
            ("m", "type R = { id: Key rs }"),
            ("m", "relation rs: R key id"),
            (
                "m",
                "type T { id: Key ts } = A { a: I64, p: Key rs on delete cascade } | B { b: I64, p: Key rs on delete cascade }",
            ),
            ("m", "relation ts: T key id"),
            (
                "m",
                "type U { id: Key us, q: Key rs on delete cascade } = C"
            ),
            ("m", "relation us: U key id"),
        ])
    );
}

#[test]
fn a_unique_over_a_field_every_member_of_its_scope_declares_is_added() {
    let src = source(&[
        (
            "m",
            "type T { id: Key ts } = A { u: I64 } | B { u: I64 } | C",
        ),
        ("m", "relation ts : T key id"),
        ("m", "set S of ts = A | B"),
    ]);
    assert_eq!(
        migrated(
            &src,
            "module m\nadd unique (u) in ts.(A | B)\nadd index (u) in ts.S"
        ),
        expected(&[
            (
                "m",
                "type T { id: Key ts } = A { u: I64 } | B { u: I64 } | C"
            ),
            (
                "m",
                "relation ts: T key id unique (u) in (A | B) index (u) in S"
            ),
            ("m", "set S of ts = A | B"),
        ])
    );
}

#[test]
fn a_key_type_in_an_expression_names_a_relation_a_later_operator_adds() {
    let src = source(&[("m", "type Z = { a: I64 }")]);
    let got = migrated(
        &src,
        "module m
add type R = { id: Key rs }
add type Q = { id: Key qs, r: Key rs, n: I64 }
define fn keep (f: Key rs -> Bool) -> I64 = 1
add relation qs : Q
  key id
  default n = keep(fn (k: Key rs) -> true)
  computed held: I64 = keep(fn (k: Key rs) -> k == k)
add check c on qs (keep(fn (k: Key rs) -> k == k) == n)
add relation rs : R key id
rename relation rs to rs2",
    );
    assert_eq!(
        got,
        expected(&[
            ("m", "type Z = { a: I64 }"),
            ("m", "type R = { id: Key rs2 }"),
            ("m", "type Q = { id: Key qs, r: Key rs2, n: I64 }"),
            ("m", "fn keep (f: Key rs2 -> Bool) -> I64 = 1"),
            (
                "m",
                "relation qs: Q key id default n = keep(fn (k: Key rs2) -> true) computed held: I64 = keep(fn (k: Key rs2) -> k == k)",
            ),
            ("m", "check c on qs (keep(fn (k: Key rs2) -> k == k) == n)"),
            ("m", "relation rs2: R key id"),
        ])
    );
}

/// Returns the reason the model refuses each operator of migration `text` over `src` with, and
/// the text at the span it names, `None` for an operator it applies.
fn refusals(src: &Source, text: &str) -> Vec<Option<(String, Option<String>)>> {
    let m = parse_migration(text, Options::default())
        .ok()
        .expect("the migration parses");
    let got = of(src)
        .expect("the source lifts")
        .migrate(&m, text)
        .expect("the model applies the migration");
    assert_eq!(got.stopped(), None, "the model applies every operator");
    got.ops()
        .iter()
        .map(|op| {
            op.as_ref().err().map(|r| {
                let at = r.span().map(|s| text[s.range()].to_string());
                (r.why().to_string(), at)
            })
        })
        .collect()
}

#[test]
fn a_unique_over_a_field_a_member_of_its_scope_lacks_or_types_otherwise_is_refused_at_the_member() {
    let src = source(&[
        (
            "m",
            "type T { id: Key ts } = A { u: I64 } | B { u: Text } | C",
        ),
        ("m", "relation ts : T key id"),
        ("m", "set S of ts = A | C"),
    ]);
    let refused = |why: &str, at: &str| Some((why.to_string(), Some(at.to_string())));
    assert_eq!(
        refusals(
            &src,
            "module m
add unique (u) in ts.(A | C)
add unique (u) in ts.(A | B)
add unique (u) in ts.S"
        ),
        vec![
            refused("member `C` of `T` has no field `u`", "C"),
            refused("member `B` declares `u` as `Text`, not `I64`", "B"),
            refused("member `C` of `T` has no field `u`", "S"),
        ]
    );
}
