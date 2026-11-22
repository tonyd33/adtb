use super::*;
use crate::evolve::{Failed, SourceDecl};
use crate::parse::{Options, parse_migration};
use crate::span::{Diag, Span};

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
    src.migrate(&m, text)
        .unwrap_or_else(|r| panic!("the migration applies: {r:?}"))
        .decls
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

/// The declarations a recording leaves, as their modules and flattened texts, or the operators'
/// refusals.
type Recorded = Result<Vec<(String, String)>, Vec<Diag>>;

/// Returns the declarations recording migration `text` on `src` leaves, as their modules and
/// texts with whitespace flattened, or the operators' refusals.
fn recorded(src: &Source, text: &str) -> Recorded {
    let m = parse_migration(text, Options::default())
        .ok()
        .expect("the migration parses");
    src.record(&m, text, false)
        .map(|(s, _, _)| {
            s.decls
                .into_iter()
                .map(|d| (d.module, flat(&d.text)))
                .collect()
        })
        .map_err(|f| match f {
            Failed::Refused(r) => r.operators,
            Failed::Broken(d) => panic!("{text}\nthe model fails: {}", d.message),
        })
}

fn arms_source() -> Source {
    source(&[
        ("m", "type K = A | B"),
        ("m", "type Inner = A { x: Text } | B { x: Text }"),
        (
            "m",
            "type T { id: Key ts } = A { x: Text } | B { x: Text } | C { x: Text }",
        ),
        ("m", "relation ts : T key id"),
        (
            "m",
            "fn f (t: T) -> Text = match t | A a -> a.x | B b -> b.x | C c -> c.x",
        ),
    ])
}

#[test]
fn an_arm_naming_a_function_a_later_operator_defines_is_refused() {
    let src = arms_source();
    let nest = "nest constructor T.(A | B) into N { inner: Inner }
  arm f: N n -> match n.inner | A a -> a.x | B b -> b.x";
    let cases = [
        (
            "",
            "split constructor T.A into A1 { x: Text } | A2 { x: Text }
  by match
    | A { x } -> A1 { x: x }
  arm f: A1 a -> h(a.x)
  arm f: A2 a -> a.x"
                .to_string(),
        ),
        (
            "",
            "merge constructor T.(A | B) into M { k: K, x: Text }
  by match
    | A a -> M { k: A, x: a.x }
    | B b -> M { k: B, x: b.x }
  arm f: M m -> h(m.x)"
                .to_string(),
        ),
        ("", nest.replace("-> a.x |", "-> h(a.x) |")),
        (
            nest,
            "unnest constructor T.N\n  arm f: A a -> h(a.x)\n  arm f: B b -> b.x".to_string(),
        ),
        (
            "",
            "add constructor T.D { x: Text }\n  arm f: D d -> h(d.x)".to_string(),
        ),
    ];
    for (first, op) in cases {
        let head = if first.is_empty() {
            "module m\n".to_string()
        } else {
            format!("module m\n{first}\n")
        };
        let text = format!("{head}{op}\ndefine fn h (x: Text) -> Text = x");
        let at = Span::new(head.len(), head.len() + op.len());
        assert_eq!(
            recorded(&src, &text),
            Err(vec![Diag::new(at, "no `h` in scope")]),
            "{text}"
        );
    }
}

#[test]
fn an_arm_follows_a_rename_of_its_constructor_by_id() {
    let split = "split constructor T.A into A1 { x: Text } | A2 { x: Text }
  by match
    | A { x } -> A1 { x: x }
  arm f: A1 a -> a.x ++ \"A1\"
  arm f: A2 a -> a.x
rename constructor T.A1 to Z";
    let after = |rel: &str| {
        expected(&[
            ("m", "type K = A | B"),
            ("m", "type Inner = A { x: Text } | B { x: Text }"),
            (
                "m",
                "type T { id: Key ts } = Z { x: Text } | A2 { x: Text } | B { x: Text } | C { x: Text }",
            ),
            ("m", rel),
            (
                "m",
                "fn f (t: T) -> Text = match t | B b -> b.x | C c -> c.x | Z a -> a.x ++ \"A1\" | A2 a -> a.x",
            ),
        ])
    };
    assert_eq!(
        recorded(&arms_source(), &format!("module m\n{split}")),
        Ok(after("relation ts: T key id"))
    );
    let mut src = arms_source();
    src.decls[3].text = "relation ts : T key id default A.x = \"a\"".to_string();
    let src = Source::new(src.decls).expect("the source builds");
    let text = format!("module m\n{split}\ndrop default ts.A.x");
    let at = text.find("rename").expect("the rename");
    assert_eq!(
        recorded(&src, &text),
        Err(vec![Diag::new(
            Span::new(at, at + "rename constructor T.A1 to Z".len()),
            "`A` is not a constructor of `T`"
        )]),
        "a rename while the declarations do not build is refused"
    );
}

#[test]
fn a_rekey_to_a_key_no_field_holds_is_refused_at_the_recheck() {
    let src = source(&[
        ("m", "type T { id: Key ts, n: I64 } = T"),
        ("m", "relation ts : T key id"),
    ]);
    let text = "module m\nrekey relation ts key zz";
    let m = parse_migration(text, Options::default())
        .ok()
        .expect("the migration parses");
    let Err(Failed::Refused(refusal)) = src.record(&m, text, false).map(|_| ()) else {
        panic!("the rekey is refused");
    };
    assert_eq!(
        (
            refusal.operators,
            refusal
                .recheck
                .iter()
                .map(|d| d.diag.message.clone())
                .collect::<Vec<_>>(),
        ),
        (
            Vec::new(),
            vec![
                "key field `zz` is not a header field or a field of the only constructor of `T`"
                    .to_string()
            ],
        )
    );
}
