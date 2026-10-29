use super::*;

const OR: &str = "fn f (i: Identity) -> Key users = match i
  | Google g | Apple g -> g.user
  | EmailPass e        -> e.user";

const GUARDED: &str = "fn f (s: Status, t: Instant) -> Instant = match s
  | Sent { at } | Paid { at } if at > t -> at
  | Sent _ | Paid _                     -> t
  | Draft                               -> t";

const ONE_CTOR: &str = "type T = A { n: I64, m: I64 } | B
fn f (t: T) -> I64 = match t
  | A { n: 1, m: x } | A { n: 2, m: x } -> x + x
  | A _                                 -> 0
  | B                                   -> 0";

const NESTED_SET: &str = "type Pay { n: I64 } = Card { last4: Text } | Bank { iban: Text } | Cash
relation pays : Pay key n
set Online of pays = Card | Bank
type W = A { p: Pay } | B
fn f (w: W) -> I64 = match w
  | A { p: Online o } -> o.n
  | A { p: Cash _ }   -> 0
  | B                 -> 1";

/// Assert that no variable of the checked bodies of `p` has two binding sites.
fn checked_once(p: &Program) {
    assert_one_binding_site("checked", &p.terms, &program_roots(p));
}

/// Returns function `f` of `text` checked and normalised, each printed in full.
fn printed(text: &str) -> (String, String) {
    let (s, p) = program(text).unwrap();
    let checked = sexp(&p.terms, body_of(&s, &p, "f"));
    let n = crate::norm::program(&s, &p).expect("normalises");
    let i = s.functions.iter().position(|f| f.name == "f").unwrap();
    let (_, body) = n.functions[i].clone().expect("a normal form");
    (checked, sexp(&n.terms, body))
}

#[test]
fn every_match_binder_has_one_binding_site() {
    for text in [
        OR,
        GUARDED,
        ONE_CTOR,
        NESTED_SET,
        "fn f (i: Identity) -> Key users = match i | OAuth o -> o.user | EmailPass e -> e.user",
        "fn f (i: Identity) -> Text = match i | OAuth o if o.user == o.user -> display_name(o) | OAuth _ -> \"\" | EmailPass e -> e.email",
        "fn rs () -> Set (A { n: I64 } | B { n: I64 } | C) = [C]
         fn f () -> Set I64 = from r in rs() select match r | A x | B x -> x.n | C -> 0",
    ] {
        let (_, p) = program(text).unwrap();
        checked_once(&p);
    }
    let (_, a) = adhoc(
        &[("i", "Identity")],
        "match i | Google g -> g.gid | x -> display_name(x)",
    )
    .unwrap();
    assert_one_binding_site("checked", &a.terms, &adhoc_roots(&a.items, &a.commands));
}

#[test]
fn an_or_pattern_binds_each_name_once_from_the_scrutinee() {
    let (checked, normal) = printed(OR);
    assert_eq!(
        checked,
        "(let ξ0 i1 (case ξ0 \
           (0 ξ2 (let e3 ξ2 e3.user)) \
           (1 ξ4 (let g5 (case ξ0 (0 ξ6 unreachable) (1 ξ7 ξ7) (2 ξ8 ξ8)) g5.user)) \
           (2 ξ9 (let g5 (case ξ0 (0 ξ6 unreachable) (1 ξ7 ξ7) (2 ξ8 ξ8)) g5.user))))"
    );
    assert_eq!(
        normal,
        "(case i0 (0 ξ1 ξ1.user) (1 ξ2 ξ2.user) (2 ξ3 ξ3.user))"
    );
}

#[test]
fn a_guarded_or_pattern_binds_its_guard_names_once() {
    let (checked, normal) = printed(GUARDED);
    let guard = "(let at5 (case ξ0 (0 ξ6 unreachable) (1 ξ7 ξ7.at) (2 ξ8 ξ8.at)) (Gt at5 t3))";
    let body = "(let at9 (case ξ0 (0 ξ10 unreachable) (1 ξ11 ξ11.at) (2 ξ12 ξ12.at)) at9)";
    assert_eq!(
        checked,
        format!(
            "(let ξ0 s1 (case ξ0 (0 ξ2 t3) (1 ξ4 (if {guard} {body} t3)) (2 ξ13 (if {guard} {body} t3))))"
        )
    );
    assert_eq!(
        normal,
        "(case s0 (0 ξ1 t2) (1 ξ3 (if (Gt ξ3.at t2) ξ3.at t2)) (2 ξ4 (if (Gt ξ4.at t2) ξ4.at t2)))"
    );
}

#[test]
fn alternatives_of_one_constructor_that_bind_alike_select_without_a_test() {
    let (checked, normal) = printed(ONE_CTOR);
    assert_eq!(
        checked,
        "(let ξ0 t1 (case ξ0 \
           (0 ξ2 (if (if (Eq ξ2.n 1) #1 (Eq ξ2.n 2)) \
             (let x3 (case ξ0 (0 ξ4 ξ4.m) (1 ξ5 unreachable)) (Add x3 x3)) \
             0)) \
           (1 ξ6 0)))"
    );
    assert_eq!(
        normal,
        "(case t0 (0 ξ1 (if (if (Eq ξ1.n 1) #1 (Eq ξ1.n 2)) (Add ξ1.m ξ1.m) 0)) (1 ξ2 0))"
    );
}

#[test]
fn alternatives_that_bind_apart_select_by_their_tests() {
    let (checked, normal) = printed(
        "type T = A { n: I64, m: I64, k: I64 } | B
         fn f (t: T) -> I64 = match t
           | A { n: 1, m: x } | A { n: 2, k: x } -> x
           | A _                                 -> 0
           | B                                   -> 0",
    );
    assert_eq!(
        checked,
        "(let ξ0 t1 (case ξ0 \
           (0 ξ2 (if (if (Eq ξ2.n 1) #1 (Eq ξ2.n 2)) \
             (let x3 (case ξ0 (0 ξ4 (if (Eq ξ4.n 1) ξ4.m ξ4.k)) (1 ξ5 unreachable)) x3) \
             0)) \
           (1 ξ6 0)))"
    );
    assert_eq!(
        normal,
        "(case t0 (0 ξ1 (if (if (Eq ξ1.n 1) #1 (Eq ξ1.n 2)) (if (Eq ξ1.n 1) ξ1.m ξ1.k) 0)) (1 ξ2 0))"
    );
}

/// Returns the tree sizes of the checked term and the normal form of a match whose first arm
/// has `alts` alternatives and a body of `uses` uses of `x`, where alternative `i` binds `x` to
/// field `m`, or to field `k` for an odd `i` when `apart`.
fn sizes(alts: usize, uses: usize, apart: bool) -> (usize, usize) {
    let limit = 1 << 20;
    let pats: Vec<String> = (1..=alts)
        .map(|i| {
            let f = if apart && i % 2 == 1 { "k" } else { "m" };
            format!("A {{ n: {i}, {f}: x }}")
        })
        .collect();
    let body: Vec<String> = (0..uses).map(|i| format!("x * {i}")).collect();
    let text = format!(
        "type T = A {{ n: I64, m: I64, k: I64 }} | B
         fn f (t: T) -> I64 = match t | {} -> {} | A _ -> 0 | B -> 0",
        pats.join(" | "),
        body.join(" + ")
    );
    let (s, p) = program(&text).unwrap();
    let checked = p.terms.tree_size(body_of(&s, &p, "f"), limit);
    let n = crate::norm::program(&s, &p).expect("normalises");
    let i = s.functions.iter().position(|f| f.name == "f").unwrap();
    let (_, body) = n.functions[i].clone().expect("a normal form");
    let normal = n.terms.tree_size(body, limit);
    (
        checked.expect("the checked term is under the limit"),
        normal.expect("the normal form is under the limit"),
    )
}

#[test]
fn a_nested_constructor_set_binds_its_name_once() {
    let (checked, normal) = printed(NESTED_SET);
    assert_eq!(
        checked,
        "(let ξ0 w1 (case ξ0 \
           (0 ξ2 (if (case ξ2.p (0 ξ3 #1) (1 ξ4 #1) (2 ξ5 #0)) \
             (let o6 (case ξ0 (0 ξ7 (case ξ7.p (0 ξ8 ξ8) (1 ξ9 ξ9) (2 ξ10 unreachable))) (1 ξ11 unreachable)) o6.n) \
             0)) \
           (1 ξ12 1)))"
    );
    assert_eq!(
        normal,
        "(case w0 (0 ξ1 \
           (if (case ξ1.p (0 ξ2 #1) (1 ξ3 #1) (2 ξ4 #0)) \
             (case ξ1.p (0 ξ5 ξ5) (1 ξ6 ξ6) (2 ξ7 unreachable)).n \
             0)) \
           (1 ξ8 1))"
    );
}

#[test]
fn a_declared_sum_of_one_constructor_selects_without_a_case() {
    let (_, normal) = printed(
        "type T = A { n: I64, m: I64, k: I64 }
         fn f (t: T) -> I64 = match t
           | A { n: 1, m: x } | A { n: 2, k: x } -> x
           | A _                                 -> 0",
    );
    assert_eq!(
        normal,
        "(if (if (Eq t0.n 1) #1 (Eq t0.n 2)) (if (Eq t0.n 1) t0.m t0.k) 0)"
    );
}

#[test]
fn an_anonymous_sum_of_one_constructor_selects_without_a_case() {
    let (_, normal) = printed(
        "fn f (t: A { n: I64, m: I64, k: I64 }) -> I64 = match t
           | A { n: 1, m: x } | A { n: 2, k: x } -> x
           | A _                                 -> 0",
    );
    assert_eq!(
        normal,
        "(if (if (Eq t0.n 1) #1 (Eq t0.n 2)) (if (Eq t0.n 1) t0.m t0.k) 0)"
    );
}

#[test]
fn an_or_pattern_of_many_alternatives_stays_linear() {
    for apart in [false, true] {
        let (c16, n16) = sizes(16, 40, apart);
        let (c32, n32) = sizes(32, 40, apart);
        assert!(
            c32 - c16 <= 16 * 16,
            "apart {apart}: 16 more alternatives add {} checked terms",
            c32 - c16
        );
        assert!(
            n32 - n16 <= 16 * 16,
            "apart {apart}: 16 more alternatives add {} normal terms",
            n32 - n16
        );
    }
}

#[test]
fn a_name_bound_apart_is_selected_once_for_all_its_uses() {
    let (c20, n20) = sizes(8, 20, true);
    let (c40, n40) = sizes(8, 40, true);
    assert!(
        c40 - c20 <= 8 * 20,
        "20 more uses add {} checked terms",
        c40 - c20
    );
    assert!(
        n40 - n20 <= 8 * 20,
        "20 more uses add {} normal terms",
        n40 - n20
    );
}
