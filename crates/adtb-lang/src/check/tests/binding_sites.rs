use super::*;
use crate::analysis::tests::reachable;
use crate::kernel::Search;

const FS: &str = "fn fs () -> Set Follow = from f in follows where f.src != f.dst select f";

/// Assert that the checked bodies of schema text `extra` bind each variable once, and return them.
fn checked_once(extra: &str) -> (Schema, Program) {
    let (s, p) = program(extra).unwrap_or_else(|e| panic!("{extra}\nrefused: {e:?}"));
    assert_one_binding_site("checked", &p.terms, &program_roots(&p));
    (s, p)
}

/// Returns the normal form of function `name` of `p`.
fn normal_body(s: &Schema, n: &crate::norm::NormalProgram, name: &str) -> Tm {
    let i = s.functions.iter().position(|f| f.name == name).unwrap();
    n.functions[i].clone().expect("a normal form").1
}

/// Returns the variable the `For` that `t` ranges over binds, below any `Let`s and `If`s.
fn generator_var(terms: &Terms, t: Tm) -> Var {
    let mut found = None;
    terms.search(t, |s| match terms.kind(s) {
        Kind::For(x, ..) => {
            found = Some(*x);
            Search::Found
        }
        k => Search::Into(k.children()),
    });
    found.expect("a generator")
}

#[test]
fn a_presented_query_binds_its_element_apart_from_its_generator() {
    for (params, text) in [
        (&[][..], "from u in users select u order by u.name"),
        (
            &[("c", "Option (Key users)")],
            "from u in users select u order by u.id take 50 after c",
        ),
        (
            &[("c", "Option ({ at: Instant, id: Key posts })")],
            "from p in posts select p order by (p.created_at desc, key p) take 20 after c",
        ),
    ] {
        let (_, a) = adhoc(params, text).unwrap_or_else(|e| panic!("{text}\nrefused: {e:?}"));
        assert_one_binding_site("checked", &a.terms, &adhoc_roots(&a.items, &a.commands));
        let Kind::Present {
            var, source, keys, ..
        } = a.terms.kind(a.items[0])
        else {
            panic!("{text}\npresents, found {:?}", a.terms.kind(a.items[0]));
        };
        assert_ne!(*var, generator_var(&a.terms, *source), "{text}");
        for (k, _) in keys {
            assert!(
                a.terms.any(*k, |x| matches!(x, Kind::Var(v) if v == var)),
                "{text}\na key reads the presented element"
            );
        }
    }
    query_accepts("query q () -> Set User = from u in users select u order by u.name");
}

#[test]
fn top_binds_its_element_apart_from_its_generator() {
    let (s, p) = checked_once(
        "fn f () -> Set Post = top 3 by (p.created_at desc, key p) (from p in posts select p)",
    );
    let Kind::Top { var, source, .. } = p.terms.kind(body_of(&s, &p, "f")) else {
        panic!("a top");
    };
    assert_ne!(*var, generator_var(&p.terms, *source));
}

#[test]
fn an_aggregate_binds_its_element_apart_from_its_generators() {
    for text in [
        "fn f () -> I64 = sum(o.total for o in orders where o.qty > 1)",
        "fn f () -> I64 = sum(o.total + o.qty for u in users, o in orders where o.user == key u)",
        "fn f () -> Set I64 = from o in orders group o by o.user into g
           select sum(x.total for x in g where x.qty > 1) + count(y for y in g)",
    ] {
        checked_once(text);
    }
}

#[test]
fn a_group_binds_each_binder_once_per_use() {
    for text in [
        "fn f () -> Set ({ user: Key users, n: I64, t: I64 }) =
           from o in orders group o by o.user into g
           select { user: g.key, n: count(x for x in g), t: sum(x.total for x in g) }",
        "fn f () -> Set (Key users) = from o in orders group o by o.user into g
           where count(x for x in g) > 1 select g.key",
        "fn f () -> Set ({ k: Key users, a: Set (Set (Key users)), b: I64 }) =
           from o in orders, u in users
           group (from f in follows where f.src == o.user select f.dst) by key u into g
           select { k: g.key, a: g, b: count(x for x in g) }",
    ] {
        checked_once(text);
    }
}

#[test]
fn a_filtered_update_binds_its_row_apart_from_its_filter() {
    let (_, a) = adhoc(
        &[("u", "Key users"), ("g2", "Text")],
        "update g in identities.Google where g.user == u set Google { g with gid: g2 }",
    )
    .unwrap();
    let Kind::Replace(_, rows, f) = a.terms.kind(a.items[0]) else {
        panic!("a replace");
    };
    let Kind::Lam(x, _) = a.terms.kind(*f) else {
        panic!("a lambda");
    };
    assert_ne!(*x, generator_var(&a.terms, *rows));
}

#[test]
fn a_for_over_a_generator_binds_it_afresh_in_one_root() {
    let text = format!(
        "{FS}\nfn g () -> I64 = count(y for y in fs()) + count(z for z in (from x in fs() select key x))"
    );
    let (s, p) = checked_once(&text);
    let n = crate::norm::program(&s, &p).expect("normalises");
    assert_one_binding_site("normalised", &n.terms, &[normal_body(&s, &n, "g")]);
}

#[test]
fn a_for_over_a_generator_binds_it_afresh_across_roots() {
    let text = format!(
        "{FS}
         fn a () -> Set (Key follows) = from x in fs() select key x
         fn b () -> Set (Key users) = from x in fs() select x.src"
    );
    let (s, p) = checked_once(&text);
    let n = crate::norm::program(&s, &p).expect("normalises");
    for name in ["a", "b"] {
        assert_one_binding_site("normalised", &n.terms, &[normal_body(&s, &n, name)]);
    }
    assert_one_binding_site(
        "normalised",
        &n.terms,
        &[normal_body(&s, &n, "a"), normal_body(&s, &n, "b")],
    );
}

#[test]
fn a_function_chosen_by_a_match_binds_afresh_at_each_application() {
    for text in [
        "fn ap (f: I64 -> I64, n: I64) -> I64 = f(n) + f(n + 1)
         fn h (s: Status, n: I64) -> I64 =
           ap(match s | Draft -> (fn x -> x) | Sent a -> (fn x -> x + 1) | Paid p -> (fn x -> x + 2), n)",
        "fn ap (f: I64 -> I64, n: I64) -> I64 = f(n) + f(n + 1)
         fn k (s: Status) -> I64 -> I64 =
           match s | Draft -> (fn x -> x) | Sent a -> (fn x -> x + 1) | Paid p -> (fn x -> x + 2)
         fn h (s: Status, n: I64) -> I64 = ap(k(s), n) + ap(k(s), n)",
    ] {
        checked_once(text);
    }
}

/// Returns schema text for `f`, `depth` groups deep, each group's value holding the level below.
fn nested_groups(depth: u32) -> String {
    let c = (0..depth).fold("count(z for z in orders)".to_string(), |c, _| {
        format!(
            "count(z for z in (from o in orders group (o.qty + {c}) by o.user into g \
             select count(x for x in g) + count(y for y in g)))"
        )
    });
    format!("fn f () -> I64 = {c}")
}

#[test]
fn a_closed_group_value_is_shared_by_each_use_of_its_group() {
    let sizes = [4, 8].map(|d| {
        let (s, p) = checked_once(&nested_groups(d));
        let n = crate::norm::program(&s, &p).expect("normalises");
        (
            reachable(&p.terms, body_of(&s, &p, "f")).len(),
            reachable(&n.terms, normal_body(&s, &n, "f")).len(),
        )
    });
    let [(checked4, normal4), (checked8, normal8)] = sizes;
    assert!(
        checked8 <= 2 * checked4,
        "checked nodes: {checked4} at depth 4, {checked8} at depth 8"
    );
    assert!(
        normal8 <= 2 * normal4,
        "normal nodes: {normal4} at depth 4, {normal8} at depth 8"
    );
}

/// Returns schema text for `f`, `depth` groups deep, each group's value reading the generator of
/// the level above and holding the level below, its group used by `select`.
fn dependent_groups(depth: u32, select: &dyn Fn(u32) -> String) -> String {
    fn level(j: u32, p: &str, select: &dyn Fn(u32) -> String) -> String {
        if j == 0 {
            return p.to_string();
        }
        let below = level(j - 1, &format!("o{j}.qty"), select);
        format!(
            "count(z{j} for z{j} in (from o{j} in orders \
             group (o{j}.qty + {p} + {below}) by o{j}.user into g{j} select {}))",
            select(j)
        )
    }
    format!("fn f (n: I64) -> I64 = {}", level(depth, "n", select))
}

fn both_uses(j: u32) -> String {
    format!("count(x{j} for x{j} in g{j}) + count(y{j} for y{j} in g{j})")
}

fn one_use_folded(j: u32) -> String {
    format!("if true then count(x{j} for x{j} in g{j}) else count(y{j} for y{j} in g{j})")
}

/// Returns the distinct reachable nodes of the checked and the normal form of `f` in the text
/// `at` makes for depths 2, 3 and 4.
fn sizes_by_depth(at: impl Fn(u32) -> String) -> [(usize, usize); 3] {
    [2, 3, 4].map(|d| {
        let (s, p) = checked_once(&at(d));
        let n = crate::norm::program(&s, &p).expect("normalises");
        (
            reachable(&p.terms, body_of(&s, &p, "f")).len(),
            reachable(&n.terms, normal_body(&s, &n, "f")).len(),
        )
    })
}

/// Assert that `sizes` grow by one constant per level.
fn assert_linear(form: &str, sizes: [usize; 3]) {
    assert_eq!(
        sizes[1] - sizes[0],
        sizes[2] - sizes[1],
        "{form} nodes at depths 2, 3, 4: {sizes:?}"
    );
}

#[test]
fn a_group_value_reading_an_outer_generator_is_bound_once_for_every_use() {
    let sizes = sizes_by_depth(|d| dependent_groups(d, &both_uses));
    assert_linear("checked", sizes.map(|s| s.0));
}

#[test]
fn a_group_value_used_in_one_folded_branch_is_normalised_once() {
    let sizes = sizes_by_depth(|d| dependent_groups(d, &one_use_folded));
    assert_linear("checked", sizes.map(|s| s.0));
    assert_linear("normal", sizes.map(|s| s.1));
}
