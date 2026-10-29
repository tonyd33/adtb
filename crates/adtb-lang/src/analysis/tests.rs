use std::collections::BTreeMap;

use super::*;
use crate::ast::{BundleItem, ExampleItem};
use crate::check::tests::{adhoc_roots, assert_one_binding_site, program_roots};
use crate::check::{Checker, Context, check_adhoc, check_query, check_schema, check_txn};
use crate::kernel::{Agg, Const, Prim, Var};
use crate::norm::{NormalAdhoc, NormalProgram, NormalQuery};
use crate::parse::{Options, parse_adhoc, parse_example, parse_schema, parse_type};
use crate::resolve::Visible;
use crate::schema::build;
use crate::types::{RelId, Ty};

const BASE: &str = r"
module auth
type User = { id: Key users, name: Text }
type Follow = { src: Key users, dst: Key users }
type Order = { id: Key orders, user: Key users, total: I64, qty: I64 }
type Post = { id: Key posts, author: Key users, title: Text }
type Reading = { id: Key readings, value: F64 }
type Shape { id: Key shapes } = Circle { r: F64 } | Square { side: F64 }
relation users : User key id generate sequence
relation follows : Follow key (src, dst)
relation orders : Order key id generate sequence
relation posts : Post key id generate sequence
relation readings : Reading key id generate sequence
relation shapes : Shape key id generate sequence
type Session = { id: Key sessions, owner: Option (Key users) }
relation sessions : Session key id generate sequence
";

fn schema(extra: &str) -> Schema {
    let src = format!("{BASE}\n{extra}");
    let parsed = parse_schema(&src, Options::default());
    assert!(
        parsed.diags.is_empty(),
        "{extra}\ndoes not parse: {:?}",
        parsed
            .diags
            .iter()
            .map(|d| d.render(&src))
            .collect::<Vec<_>>()
    );
    let file = parsed.ok().expect("parsed");
    match build(&[(FileId(0), file)]) {
        Ok(s) => s,
        Err(ds) => panic!(
            "{extra}\ndoes not build: {:?}",
            ds.into_iter().map(|d| d.diag.message).collect::<Vec<_>>()
        ),
    }
}

fn program(extra: &str) -> Result<(Schema, Program), Vec<String>> {
    let s = schema(extra);
    match check_schema(&s) {
        Ok(p) => {
            assert_one_binding_site("checked", &p.terms, &program_roots(&p));
            Ok((s, p))
        }
        Err(ds) => Err(ds.into_iter().map(|d| d.diag.message).collect()),
    }
}

fn accepts(extra: &str) {
    if let Err(errs) = program(extra) {
        panic!("{extra}\nrefused: {errs:?}");
    }
}

fn refuses(extra: &str, needle: &str) {
    match program(extra) {
        Ok(_) => panic!("{extra}\naccepted; expected an error containing {needle:?}"),
        Err(errs) => assert!(
            errs.iter().any(|e| e.contains(needle)),
            "{extra}\nno error containing {needle:?} in {errs:?}"
        ),
    }
}

fn normal(extra: &str) -> (Schema, NormalProgram) {
    let (s, p) = program(extra).unwrap_or_else(|e| panic!("{extra}\nrefused: {e:?}"));
    let n = norm::program(&s, &p).expect("normalises");
    (s, n)
}

/// The normal form of function `name`.
fn body(s: &Schema, n: &NormalProgram, name: &str) -> Tm {
    let i = s
        .functions
        .iter()
        .position(|f| f.name == name)
        .expect("a function of that name");
    n.functions[i].as_ref().expect("first-order").1
}

fn adhoc(extra: &str, params: &[(&str, &str)], text: &str) -> Result<(Schema, Adhoc), Vec<String>> {
    let s = schema(extra);
    let mut tys = BTreeMap::new();
    {
        let c = Checker::new(&s, Visible::all(&s), Context::Adhoc);
        for (name, t) in params {
            let parsed = parse_type(t, Options::default()).ok().expect("a type");
            tys.insert(
                name.to_string(),
                c.resolve_type(&parsed).expect("a known type"),
            );
        }
    }
    let parsed = parse_adhoc(text, Options::default());
    assert!(parsed.diags.is_empty(), "{text}\ndoes not parse");
    let items = parsed.ok().expect("parsed");
    let program = check_schema(&s).expect("the schema checks");
    match check_adhoc(&s, &program, &items, &tys) {
        Ok(a) => {
            assert_one_binding_site("checked", &a.terms, &adhoc_roots(&a.items, &a.commands));
            Ok((s, a))
        }
        Err(ds) => Err(ds.into_iter().map(|d| d.message).collect()),
    }
}

fn adhoc_accepts(params: &[(&str, &str)], text: &str) {
    if let Err(errs) = adhoc("", params, text) {
        panic!("{text}\nrefused: {errs:?}");
    }
}

fn adhoc_refuses(params: &[(&str, &str)], text: &str, needle: &str) {
    match adhoc("", params, text) {
        Ok(_) => panic!("{text}\naccepted; expected an error containing {needle:?}"),
        Err(errs) => assert!(
            errs.iter().any(|e| e.contains(needle)),
            "{text}\nno error containing {needle:?} in {errs:?}"
        ),
    }
}

/// The normal form of the single item of ad hoc `text`, with the declarations of `extra`.
fn adhoc_normal(extra: &str, params: &[(&str, &str)], text: &str) -> (Schema, NormalAdhoc) {
    let (s, a) = adhoc(extra, params, text).unwrap_or_else(|e| panic!("{text}\nrefused: {e:?}"));
    let p = check_schema(&s).expect("the schema checks");
    let n = norm::adhoc(&s, &p, &a).expect("normalises");
    (s, n)
}

fn is_total(params: &[(&str, &str)], text: &str) -> bool {
    let (s, n) = adhoc_normal("", params, text);
    Totality::new(&s, false).total(&n.terms, n.items[0])
}

/// Returns whether the outermost `fix` in the normal form of ad hoc `text` is bounded.
fn fix_bounded(params: &[(&str, &str)], text: &str) -> bool {
    let (_, n) = adhoc_normal("", params, text);
    let fix = reachable(&n.terms, n.items[0])
        .into_iter()
        .find(|t| matches!(n.terms.kind(*t), Kind::Fix(..)))
        .expect("a fix");
    bounded(&n.terms, fix)
}

fn query(extra: &str, text: &str) -> Result<(), String> {
    let s = schema(extra);
    let items = parse_example(text, Options::default())
        .ok()
        .expect("parses");
    let Some(ExampleItem::BundleItem(BundleItem::Query(q))) = items.into_iter().next() else {
        panic!("{text}\nis not a query");
    };
    let module = s.module_named("auth").expect("module auth");
    let program = check_schema(&s).expect("the schema checks");
    check_query(&s, &program, module, &q)
        .map(|_| ())
        .map_err(|d| d.message)
}

/// Every term reachable from `root`.
pub(crate) fn reachable(terms: &Terms, root: Tm) -> Vec<Tm> {
    let mut seen = HashSet::new();
    let mut stack = vec![root];
    let mut out = Vec::new();
    while let Some(t) = stack.pop() {
        if seen.insert(t) {
            out.push(t);
            stack.extend(terms.children(t));
        }
    }
    out
}

/// Asserts the postconditions of a normal form: no λ but a `Replace`'s row function, no application, no call of a schema function, and a `let` only over a bound expression that is partial or draws a fresh value.
fn assert_normal(s: &Schema, terms: &Terms, root: Tm) {
    let mut tot = Totality::new(s, false);
    let all = reachable(terms, root);
    let row_functions: HashSet<Tm> = all
        .iter()
        .filter_map(|t| match terms.kind(*t) {
            Kind::Replace(_, _, f) => Some(*f),
            _ => None,
        })
        .collect();
    for t in all {
        if row_functions.contains(&t) {
            continue;
        }
        match terms.kind(t) {
            Kind::Lam(..) => panic!("a λ remains: {:?}", terms.node(t)),
            Kind::App(..) => panic!("an application remains: {:?}", terms.node(t)),
            Kind::Call { .. } => panic!("a call remains: {:?}", terms.node(t)),
            Kind::Let(_, b, _) => assert!(
                !tot.total(terms, *b) || fresh_draws(terms, *b) > 0,
                "a `let` of a total expression that draws nothing remains: {:?}",
                terms.node(*b)
            ),
            _ => {}
        }
        assert!(
            !terms.ty(t).mentions_fn() || row_functions.contains(&t),
            "a term of function type remains: {:?}",
            terms.node(t)
        );
    }
}

fn mentions_param(t: &Ty) -> bool {
    match t {
        Ty::Param(_) => true,
        Ty::Base(_) | Ty::Key { .. } | Ty::Unit | Ty::Change => false,
        Ty::Data { args, .. } => args.iter().any(mentions_param),
        Ty::Set(t) | Ty::NonEmpty(t) | Ty::Tag(t) => mentions_param(t),
        Ty::Group(k, t) | Ty::Fn(k, t) => mentions_param(k) || mentions_param(t),
        Ty::Record(fs) => fs.iter().any(|(_, t)| mentions_param(t)),
        Ty::Sum { ctors, .. } => ctors
            .iter()
            .any(|(_, fs)| fs.iter().any(|(_, t)| mentions_param(t))),
    }
}

// Normalisation

const HIGHER_ORDER: &str = "
fn twice (x: I64) -> I64 = x + x
fn const_one (x: I64) -> I64 = 1
fn ap (g: I64 -> I64, n: I64) -> I64 = g(n)
fn having (p: Order -> Bool) -> Set (Key orders) = from o in orders where p(o) select key o
fn big () -> Set (Key orders) = having(fn (o: Order) -> o.qty > 10)
fn via_twice (y: I64) -> I64 = ap(twice, y)
fn partial_arg (y: I64) -> I64 = const_one(10 / y)
fn curried () -> I64 = ap(fn (a: I64) -> a * 3, 7)
fn chosen (b: Bool, n: I64) -> I64 = ap(if b then fn (x: I64) -> x else fn (x: I64) -> x + 1, n)
fn or_else [a] (p: Option a, d: a) -> a = match p | None -> d | Some s -> s.value
fn generic_use (p: Option I64) -> I64 = or_else(p, 0)
constraint big_orders = from k in big() where k == k select k
";

#[test]
fn normal_forms_have_no_lambda_application_or_schema_call() {
    let (s, n) = normal(HIGHER_ORDER);
    for f in [
        "twice",
        "const_one",
        "big",
        "via_twice",
        "partial_arg",
        "curried",
        "chosen",
        "generic_use",
    ] {
        assert_normal(&s, &n.terms, body(&s, &n, f));
    }
    for c in &n.constraints {
        assert_normal(&s, &n.terms, *c);
    }
}

#[test]
fn functions_taking_functions_have_no_standalone_normal_form() {
    let (s, n) = normal(HIGHER_ORDER);
    for f in ["ap", "having"] {
        let i = s.functions.iter().position(|d| d.name == f).unwrap();
        assert!(n.functions[i].is_none(), "`{f}` takes a function");
    }
}

#[test]
fn a_total_argument_is_substituted_and_shared() {
    let (s, n) = normal(HIGHER_ORDER);
    let t = body(&s, &n, "via_twice");
    let Kind::Prim(Prim::Add, xs) = n.terms.kind(t) else {
        panic!("expected `y + y`, found {:?}", n.terms.node(t));
    };
    assert_eq!(xs[0], xs[1], "the argument is one shared term");
    assert!(matches!(n.terms.kind(xs[0]), Kind::Var(_)));
}

#[test]
fn a_partial_argument_is_bound_once_before_the_body() {
    let (s, n) = normal(HIGHER_ORDER);
    let t = body(&s, &n, "partial_arg");
    let Kind::Let(_, bound, rest) = n.terms.kind(t) else {
        panic!("expected a `let`, found {:?}", n.terms.node(t));
    };
    assert!(matches!(n.terms.kind(*bound), Kind::Prim(Prim::Div, _)));
    assert!(matches!(n.terms.kind(*rest), Kind::Const(Const::Int(1))));
}

#[test]
fn a_lambda_applied_to_literals_reduces() {
    let (s, n) = normal(HIGHER_ORDER);
    let t = body(&s, &n, "curried");
    let Kind::Prim(Prim::Mul, xs) = n.terms.kind(t) else {
        panic!("expected `7 * 3`, found {:?}", n.terms.node(t));
    };
    assert!(matches!(n.terms.kind(xs[0]), Kind::Const(Const::Int(7))));
    assert!(matches!(n.terms.kind(xs[1]), Kind::Const(Const::Int(3))));
}

#[test]
fn a_function_chosen_by_a_condition_is_applied_in_each_branch() {
    let (s, n) = normal(HIGHER_ORDER);
    let t = body(&s, &n, "chosen");
    let Kind::If(_, a, b) = n.terms.kind(t) else {
        panic!("expected an `if`, found {:?}", n.terms.node(t));
    };
    assert!(matches!(n.terms.kind(*a), Kind::Var(_)));
    assert!(matches!(n.terms.kind(*b), Kind::Prim(Prim::Add, _)));
}

#[test]
fn a_generic_call_is_inlined_at_its_type_arguments() {
    let (s, n) = normal(HIGHER_ORDER);
    let t = body(&s, &n, "generic_use");
    for u in reachable(&n.terms, t) {
        assert!(
            !mentions_param(n.terms.ty(u)),
            "a type parameter remains: {:?}",
            n.terms.node(u)
        );
    }
}

#[test]
fn a_predicate_parameter_is_inlined_into_the_comprehension() {
    let (s, n) = normal(HIGHER_ORDER);
    let t = body(&s, &n, "big");
    let Kind::For(_, src, inner) = n.terms.kind(t) else {
        panic!("expected a comprehension, found {:?}", n.terms.node(t));
    };
    assert!(matches!(n.terms.kind(*src), Kind::Rel(..)));
    let Kind::If(c, _, _) = n.terms.kind(*inner) else {
        panic!("expected a `where`, found {:?}", n.terms.node(*inner));
    };
    assert!(matches!(n.terms.kind(*c), Kind::Prim(Prim::Gt, _)));
}

#[test]
fn a_generator_over_a_comprehension_flattens() {
    let (_, n) = adhoc_normal(
        "",
        &[],
        "from x in (from o in orders select o.total) select x",
    );
    let t = n.items[0];
    let Kind::For(_, src, inner) = n.terms.kind(t) else {
        panic!("expected a comprehension, found {:?}", n.terms.node(t));
    };
    assert!(matches!(n.terms.kind(*src), Kind::Rel(..)));
    let Kind::Singleton(e) = n.terms.kind(*inner) else {
        panic!("expected `{{o.total}}`, found {:?}", n.terms.node(*inner));
    };
    assert!(matches!(n.terms.kind(*e), Kind::Field(_, f) if f == "total"));
}

#[test]
fn a_generator_over_a_union_splits() {
    let (_, n) = adhoc_normal("", &[], "from x in [1, 2] select x + 1");
    let mut sums = 0;
    for t in reachable(&n.terms, n.items[0]) {
        match n.terms.kind(t) {
            Kind::For(..) => panic!("a generator remains: {:?}", n.terms.node(t)),
            Kind::Singleton(e) => {
                assert!(matches!(n.terms.kind(*e), Kind::Prim(Prim::Add, _)));
                sums += 1;
            }
            _ => {}
        }
    }
    assert_eq!(2, sums);
}

#[test]
fn a_field_of_a_record_reduces_when_the_other_fields_are_total() {
    let (_, n) = adhoc_normal("", &[("n", "I64")], "{ a: 1, b: n }.a");
    assert!(matches!(
        n.terms.kind(n.items[0]),
        Kind::Const(Const::Int(1))
    ));
    let (_, n) = adhoc_normal("", &[("n", "I64")], "{ a: 1, b: 10 / n }.a");
    assert!(
        matches!(n.terms.kind(n.items[0]), Kind::Field(..)),
        "a partial field is still evaluated: {:?}",
        n.terms.node(n.items[0])
    );
}

#[test]
fn a_case_of_a_constructor_selects_its_arm() {
    let (_, n) = adhoc_normal(
        "fn wrap (n: I64) -> Option I64 = Some { value: n }",
        &[],
        "match wrap(1) | None -> 0 | Some s -> s.value",
    );
    assert!(
        matches!(n.terms.kind(n.items[0]), Kind::Const(Const::Int(1))),
        "{:?}",
        n.terms.node(n.items[0])
    );
}

#[test]
fn ad_hoc_text_inlines_schema_functions() {
    let (s, n) = adhoc_normal(HIGHER_ORDER, &[("y", "I64")], "via_twice(y)");
    assert_normal(&s, &n.terms, n.items[0]);
    assert!(matches!(n.terms.kind(n.items[0]), Kind::Prim(Prim::Add, _)));
}

#[test]
fn a_registered_query_inlines_schema_functions() {
    let s = schema(HIGHER_ORDER);
    let items = parse_example("query q (y: I64) -> I64 = via_twice(y)", Options::default())
        .ok()
        .expect("parses");
    let Some(ExampleItem::BundleItem(BundleItem::Query(q))) = items.into_iter().next() else {
        panic!("a query");
    };
    let module = s.module_named("auth").unwrap();
    let p = check_schema(&s).unwrap();
    let checked = check_query(&s, &p, module, &q).expect("checks");
    let n = norm::query(&s, &p, &checked).expect("normalises");
    assert_eq!(1, n.params.len());
    assert_normal(&s, &n.terms, n.body);
    let Kind::Prim(Prim::Add, xs) = n.terms.kind(n.body) else {
        panic!("expected `y + y`");
    };
    assert!(matches!(n.terms.kind(xs[0]), Kind::Var(v) if *v == n.params[0]));
}

#[test]
fn normalisation_preserves_types() {
    let (s, n) = normal(HIGHER_ORDER);
    for (f, def) in s.functions.iter().enumerate() {
        if let Some((_, b)) = &n.functions[f] {
            let ty = n.terms.ty(*b);
            assert!(
                crate::check::subtype(&s, ty, &def.result),
                "`{}` normalises to {}, declared {}",
                def.name,
                s.show_ty(ty),
                s.show_ty(&def.result)
            );
        }
    }
}

// Stratification

#[test]
fn the_spec_reachability_fix_is_accepted() {
    adhoc_accepts(
        &[("me", "Key users")],
        "fix reach = (from f in follows where f.src == me select f.dst)
                  union (from r in reach, f in follows where f.src == r select f.dst)
         in from u in users where exists r in reach where r == key u select u",
    );
}

#[test]
fn the_spec_mutual_recursion_is_accepted() {
    adhoc_accepts(
        &[("me", "Key users")],
        "fix even = (from u in users where key u == me select key u)
                   union (from o in odd, f in follows where f.src == o select f.dst),
             odd  = from v in even, f in follows where f.src == v select f.dst
         in from u in users where exists o in odd where o == key u select u",
    );
}

#[test]
fn negating_a_complete_fix_outside_it_is_accepted() {
    adhoc_accepts(
        &[("me", "Key users")],
        "fix reach = (from f in follows where f.src == me select f.dst)
                  union (from r in reach, f in follows where f.src == r select f.dst)
         in from u in users where not exists r in reach where r == key u select u",
    );
}

#[test]
fn negating_a_binder_inside_its_own_fix_is_refused() {
    adhoc_refuses(
        &[("me", "Key users")],
        "fix a = (from f in follows where f.src == me select f.dst)
              union (from u in users where not exists r in a where r == key u select key u)
         in a",
        "`a` occurs in a body of its own `fix` at a position that is not positive (the operand of `not`)",
    );
}

#[test]
fn negating_a_binder_of_the_same_stratum_is_refused() {
    adhoc_refuses(
        &[("me", "Key users")],
        "fix even = (from u in users where key u == me select key u)
                   union (from u in users where not exists o in odd where o == key u select key u),
             odd  = from v in even, f in follows where f.src == v select f.dst
         in even",
        "`odd` occurs in a body of its own `fix` at a position that is not positive",
    );
}

#[test]
fn aggregating_a_binder_inside_its_own_fix_is_refused() {
    adhoc_refuses(
        &[("me", "Key users")],
        "fix a = (from f in follows where f.src == me select f.dst)
              union (from u in users where count(r for r in a) > 3 select key u)
         in a",
        "the range or body of an aggregate",
    );
}

#[test]
fn a_binder_in_a_disjunction_with_a_negation_is_refused() {
    adhoc_refuses(
        &[("me", "Key users")],
        "fix a = (from f in follows where f.src == me select f.dst)
              union (from u in users where key u == me or not exists r in a where r == key u select key u)
         in a",
        "not positive",
    );
}

#[test]
fn a_binder_under_and_and_or_is_positive() {
    adhoc_accepts(
        &[("me", "Key users")],
        "fix a = (from f in follows where f.src == me select f.dst)
              union (from u in users
                     where key u != me and (exists r in a where r == key u or key u == me)
                     select key u)
         in a",
    );
}

#[test]
fn a_match_in_a_fix_body_stays_positive() {
    adhoc_accepts(
        &[],
        "fix a = (from s in shapes select key s)
              union (from s in shapes, k in a where k == key s select match s
                       | Circle c -> key c
                       | Square q -> key q)
         in a",
    );
}

#[test]
fn a_negation_reached_through_an_inlined_function_is_refused() {
    refuses(
        "fn absent (s: Set (Key users), k: Key users) -> Bool = not exists r in s where r == k
         fn bad (me: Key users) -> Set (Key users) =
           fix a = (from f in follows where f.src == me select f.dst)
                 union (from u in users where absent(a, key u) select key u)
           in a",
        "`a` occurs in a body of its own `fix` at a position that is not positive",
    );
}

#[test]
fn a_negation_through_a_predicate_argument_is_refused() {
    refuses(
        "fn keep (p: Key users -> Bool) -> Set (Key users) = from u in users where p(key u) select key u
         fn bad (me: Key users) -> Set (Key users) =
           fix a = (from f in follows where f.src == me select f.dst)
                 union keep(fn (k: Key users) -> not exists r in a where r == k)
           in a",
        "`a` occurs in a body of its own `fix` at a position that is not positive",
    );
}

#[test]
fn an_unstratified_constraint_is_refused() {
    refuses(
        "constraint c =
           fix a = (from f in follows select f.dst)
                 union (from u in users where not exists r in a where r == key u select key u)
           in from u in users where exists r in a where r == key u select u",
        "not positive",
    );
}

#[test]
fn an_unstratified_registered_query_is_refused() {
    let e = query(
        "",
        "query q (me: Key users) -> Set (Key users) =
           fix a = (from f in follows where f.src == me select f.dst)
                 union (from u in users where not exists r in a where r == key u select key u)
           in a",
    )
    .expect_err("refused");
    assert!(e.contains("not positive"), "{e}");
}

#[test]
fn an_outer_fix_may_be_negated_in_an_inner_one() {
    adhoc_accepts(
        &[("me", "Key users")],
        "fix outer = from f in follows where f.src == me select f.dst
         in fix inner = (from u in users where not exists o in outer where o == key u select key u)
                      union (from i in inner, f in follows where f.src == i select f.dst)
            in inner",
    );
}

// Boundedness and totality

#[test]
fn a_reachability_fix_is_bounded_and_total() {
    assert!(is_total(
        &[("me", "Key users")],
        "fix reach = (from f in follows where f.src == me select f.dst)
                  union (from r in reach, f in follows where f.src == r select f.dst)
         in reach",
    ));
}

#[test]
fn a_fix_building_pairs_from_fields_is_bounded() {
    assert!(is_total(
        &[],
        "fix p = (from f in follows select { a: f.src, b: f.dst })
              union (from x in p, f in follows where x.b == f.src select { a: x.a, b: f.dst })
         in p",
    ));
}

#[test]
fn a_fix_applying_arithmetic_to_its_own_elements_is_unbounded_and_partial() {
    assert!(!is_total(
        &[],
        "fix n = [1] union (from x in n where x < 10 select x + 1) in n",
    ));
}

#[test]
fn a_fix_comparing_a_record_of_its_elements_is_bounded() {
    assert!(is_total(
        &[("me", "Key users")],
        "fix a = (from u in users select key u)
              union (from x in a where { k: x } != { k: me } select x)
         in a",
    ));
}

#[test]
fn a_fix_using_arithmetic_only_in_a_condition_is_bounded() {
    assert!(fix_bounded(
        &[],
        "fix p = (from f in follows select f.dst)
              union (from x in p, f in follows where f.src == x and value(x) + 1 > 0 select f.dst)
         in p",
    ));
    assert!(!is_total(
        &[],
        "fix p = (from f in follows select f.dst)
              union (from x in p, f in follows where f.src == x and value(x) + 1 > 0 select f.dst)
         in p",
    ));
}

#[test]
fn a_fix_comparing_through_a_backlink_is_bounded() {
    assert!(is_total(
        &[("me", "Key users")],
        "fix reach = (from f in follows where f.src == me select f.dst)
                  union (from r in reach, s in r.<sessions.owner, f in follows where f.src == r select f.dst)
         in reach",
    ));
}

#[test]
fn a_fix_selecting_a_value_built_by_a_partial_let_is_unbounded() {
    assert!(!is_total(
        &[],
        "fix n = [1] union (from x in n, y in [x + 1] where y < 10 select y) in n",
    ));
}

#[test]
fn boundedness_is_judged_on_the_normal_form() {
    assert!(is_total(
        &[],
        "fix a = (from u in users select key u)
              union (from x in a, y in [{ k: x }] select y.k)
         in a",
    ));
}

#[test]
fn faulting_primitives_are_partial_and_checked_forms_total() {
    assert!(!is_total(&[("n", "I64")], "10 / n"));
    assert!(!is_total(&[("n", "I64")], "n + 1"));
    assert!(!is_total(&[("n", "I64")], "-n"));
    assert!(is_total(&[("x", "F64")], "10.0 / x"));
    assert!(is_total(&[("x", "F64")], "-x"));
    assert!(is_total(&[("n", "I64")], "checked_div(10, n)"));
    assert!(is_total(&[("n", "I64")], "try (10 / n)"));
    assert!(is_total(&[("t", "Text")], "lower(t) ++ \"x\""));
    assert!(!is_total(&[("t", "Text")], "uuid(t)"));
    assert!(is_total(&[("n", "I64")], "n == 1 and n < 3"));
}

#[test]
fn try_of_a_reject_is_partial() {
    let (s, n) = adhoc_normal(
        "",
        &[],
        "insert posts from (from u in users select { author: key u, title: print(try (lower(reject \"x\"))) })",
    );
    let tries: Vec<Tm> = reachable(&n.terms, n.items[0])
        .into_iter()
        .filter(|t| matches!(n.terms.kind(*t), Kind::Try(_)))
        .collect();
    assert_eq!(1, tries.len());
    assert!(!Totality::new(&s, false).total(&n.terms, tries[0]));
}

#[test]
fn widening_conversions_are_total_and_narrowing_ones_partial() {
    assert!(is_total(&[("x", "I32")], "i64(x)"));
    assert!(!is_total(&[("x", "I64")], "i32(x)"));
    assert!(is_total(&[("x", "I32")], "f64(x)"));
    assert!(!is_total(&[("x", "F64")], "f32(x)"));
    assert!(is_total(&[("x", "I64")], "checked_i32(x)"));
}

#[test]
fn calendar_arithmetic_is_partial_whatever_its_overflow() {
    assert!(!is_total(
        &[("d", "Date"), ("p", "Period")],
        "add_period(d, p, Reject)"
    ));
    assert!(!is_total(
        &[("d", "Date"), ("p", "Period")],
        "add_period(d, p, Clamp)"
    ));
}

#[test]
fn integer_sums_are_partial_and_float_sums_total() {
    assert!(!is_total(&[], "sum(o.total for o in orders)"));
    assert!(is_total(&[], "sum(r.value for r in readings)"));
    assert!(is_total(&[], "count(o for o in orders)"));
    assert!(is_total(&[], "max(o.total for o in orders)"));
}

#[test]
fn top_is_total_only_with_a_literal_count() {
    assert!(is_total(
        &[],
        "top 3 by o.id desc (from o in orders select o)"
    ));
    assert!(!is_total(
        &[("n", "I64")],
        "top n by o.id desc (from o in orders select o)"
    ));
}

#[test]
fn a_path_through_a_reference_field_is_total_and_through_a_key_partial() {
    assert!(is_total(&[], "from p in posts select p.author.name"));
    assert!(!is_total(&[("k", "Key users")], "k.name"));
}

#[test]
fn hiding_new_constructors_makes_every_reference_path_partial() {
    let (s, n) = adhoc_normal("", &[], "from p in posts select p.author.name");
    assert!(!Totality::new(&s, true).total(&n.terms, n.items[0]));
}

#[test]
fn a_call_is_partial_exactly_when_its_inlined_body_is() {
    let (s, n) = adhoc_normal(
        "fn inv (n: I64) -> I64 = 100 / n",
        &[("y", "I64")],
        "inv(y)",
    );
    assert!(!Totality::new(&s, false).total(&n.terms, n.items[0]));
    let (s, n) = adhoc_normal("fn same (n: I64) -> I64 = n", &[("y", "I64")], "same(y)");
    assert!(Totality::new(&s, false).total(&n.terms, n.items[0]));
}

#[test]
fn constraint_and_check_bodies_may_be_partial() {
    accepts("constraint few = from o in orders where 100 / o.qty > 3 select o");
    accepts("check per_unit on orders (total / qty > 0)");
}

// Aggregates over bindings

#[test]
fn sum_over_a_key_preserving_comprehension_is_accepted() {
    adhoc_accepts(
        &[],
        "sum(x.total for x in (from o in orders where o.qty > 1 select o))",
    );
    adhoc_accepts(&[], "sum(o.total for o in orders)");
    adhoc_accepts(&[("s", "Set I64")], "sum(x for x in s)");
    adhoc_accepts(&[("s", "Set I64")], "sum(x for x in s where x > 0)");
    adhoc_accepts(
        &[],
        "sum(o.total for o in orders, u in users where o.user == key u)",
    );
}

#[test]
fn sum_over_merged_values_is_refused() {
    adhoc_refuses(
        &[],
        "sum(t for t in (from o in orders select o.total))",
        "`sum` ranges over rows",
    );
}

#[test]
fn sum_over_merged_values_in_a_multi_generator_aggregate_is_refused() {
    adhoc_refuses(
        &[],
        "sum(t for t in (from o in orders select o.total), u in users)",
        "`sum` ranges over rows",
    );
}

#[test]
fn sum_over_a_group_is_accepted() {
    adhoc_accepts(
        &[],
        "from o in orders group o by o.user into g select sum(x.total for x in g)",
    );
}

#[test]
fn sum_over_values_merged_behind_a_function_argument_is_refused() {
    refuses(
        "fn total_of (q: I64 -> Set I64) -> I64 = sum(t for t in q(0))
         fn h () -> I64 = total_of(fn (n: I64) -> from o in orders where o.qty > n select o.total)",
        "`sum` ranges over rows",
    );
}

#[test]
fn sum_over_rows_behind_a_function_argument_is_accepted() {
    accepts(
        "fn total_of (q: I64 -> Set Order) -> I64 = sum(t.total for t in q(0))
         fn h () -> I64 = total_of(fn (n: I64) -> from o in orders where o.qty > n select o)",
    );
}

#[test]
fn avg_over_merged_values_behind_a_function_argument_is_refused() {
    refuses(
        "fn mean_of (q: I64 -> Set F64) -> Option F64 = avg(v for v in q(0))
         fn h () -> Option F64 = mean_of(fn (n: I64) -> from r in readings where r.value > 0.0 select r.value)",
        "`avg` ranges over rows",
    );
}

// Writes and stored bodies

fn txn(extra: &str, text: &str) -> Result<(Schema, NormalQuery), String> {
    let s = schema(extra);
    let items = parse_example(text, Options::default())
        .ok()
        .expect("parses");
    let Some(ExampleItem::BundleItem(BundleItem::Txn(t))) = items.into_iter().next() else {
        panic!("{text}\nis not a txn");
    };
    let module = s.module_named("auth").expect("module auth");
    let p = check_schema(&s).expect("the schema checks");
    let t = check_txn(&s, &p, module, &t).map_err(|d| d.message)?;
    let n = norm::txn(&s, &p, &t).expect("normalises");
    Ok((s, n))
}

#[test]
fn an_update_through_a_function_normalises_its_row_function() {
    let (s, n) = adhoc_normal(
        "fn zeroed (o: Order) -> Order = Order { o with qty: 0 }",
        &[],
        "update o in orders where o.qty > 0 set zeroed(o)",
    );
    assert_normal(&s, &n.terms, n.items[0]);
    let Kind::Replace(_, _, f) = n.terms.kind(n.items[0]) else {
        panic!("expected a replace, found {:?}", n.terms.node(n.items[0]));
    };
    let Kind::Lam(x, body) = n.terms.kind(*f) else {
        panic!("expected a row function, found {:?}", n.terms.node(*f));
    };
    let Kind::Ctor { fields, .. } = n.terms.kind(*body) else {
        panic!("expected the new row, found {:?}", n.terms.node(*body));
    };
    let qty = fields.iter().find(|(l, _)| l == "qty").unwrap().1;
    assert!(matches!(n.terms.kind(qty), Kind::Const(Const::Int(0))));
    let id = fields.iter().find(|(l, _)| l == "id").unwrap().1;
    assert!(
        matches!(n.terms.kind(id), Kind::Field(r, _) if matches!(n.terms.kind(*r), Kind::Var(v) if v == x))
    );
}

#[test]
fn a_replace_is_partial() {
    assert!(!is_total(
        &[],
        "update o in orders set Order { o with qty: 0 }"
    ));
    assert!(is_total(&[], "delete o in orders where o.qty == 0"));
}

#[test]
fn a_registered_transaction_inlines_and_is_stratified() {
    let (s, n) = txn(
        HIGHER_ORDER,
        "txn t (y: I64) -> I64 =
           transact
             delete o in orders where key o == key o
             return via_twice(y)",
    )
    .unwrap_or_else(|e| panic!("refused: {e}"));
    assert_normal(&s, &n.terms, n.body);
    let e = txn(
        "",
        "txn t (me: Key users) -> Set (Key users) =
           transact
             return fix a = (from f in follows where f.src == me select f.dst)
                          union (from u in users where not exists r in a where r == key u select key u)
                    in a",
    )
    .map(|_| ())
    .expect_err("refused");
    assert!(e.contains("not positive"), "{e}");
}

const STORED: &str = "
fn twice (x: F64) -> F64 = x + x
type Disc = { id: Key discs, r: F64 }
relation discs : Disc
  key id generate sequence
  computed d: F64 = twice(self.r)
  index (twice(r))
";

#[test]
fn computed_fields_and_index_fields_are_normalised() {
    let (s, n) = normal(STORED);
    let discs = s
        .relations
        .iter()
        .position(|r| r.name == "discs")
        .expect("discs");
    assert_eq!(1, n.computed.len());
    let index_fields: Vec<_> = n
        .index_fields
        .iter()
        .filter(|((rel, ..), _)| rel.0 as usize == discs)
        .map(|(_, (_, f))| f.value)
        .collect();
    assert_eq!(1, index_fields.len());
    let computed = n.computed.values().map(|(_, t)| *t);
    for t in computed.chain(index_fields) {
        assert_normal(&s, &n.terms, t);
        assert!(matches!(n.terms.kind(t), Kind::Prim(Prim::Add, _)));
    }
}

#[test]
fn retags_and_defaults_are_normalised() {
    let (s, n) = normal(
        "type Grant { id: Key grants } = Live { holder: Key users on delete retag Gone { } } | Gone
         relation grants : Grant key id generate sequence
         type Counter = { id: Key counters, n: I64 }
         relation counters : Counter
           key id generate sequence
           default n = 0",
    );
    assert_eq!(1, n.retags.len());
    assert_eq!(1, n.defaults.len());
    for (_, t) in n.retags.values() {
        assert_normal(&s, &n.terms, *t);
    }
    for t in n.defaults.values() {
        assert!(matches!(n.terms.kind(*t), Kind::Const(Const::Int(0))));
    }
}

#[test]
fn an_unstratified_computed_field_is_refused() {
    refuses(
        "type Hub = { id: Key hubs, owner: Key users }
         relation hubs : Hub
           key id generate sequence
           computed far: I64 = count(k for k in (
             fix a = (from f in follows where f.src == self.owner select f.dst)
                   union (from u in users where not exists r in a where r == key u select key u)
             in a))",
        "not positive",
    );
}

/// Returns the number of `Fresh` occurrences in `root` read as a tree, a shared node counting once per parent.
fn fresh_draws(terms: &Terms, root: Tm) -> usize {
    match terms.kind(root) {
        Kind::Fresh(..) => 1,
        _ => terms
            .children(root)
            .into_iter()
            .map(|c| fresh_draws(terms, c))
            .sum(),
    }
}

#[test]
fn a_let_bound_to_an_inserted_row_draws_its_key_once() {
    let (s, n) = adhoc_normal(
        "",
        &[],
        r#"transact
             let u = insert users { name: "eve" }
             insert follows { src: key u, dst: key u }
             return key u"#,
    );
    assert_normal(&s, &n.terms, n.items[0]);
    assert_eq!(1, fresh_draws(&n.terms, n.items[0]));
    let lets = reachable(&n.terms, n.items[0])
        .into_iter()
        .filter(|t| {
            matches!(n.terms.kind(*t), Kind::Let(_, b, _)
                if fresh_draws(&n.terms, *b) == 1)
        })
        .count();
    assert_eq!(1, lets, "the inserted row stays bound by one `let`");
}

#[test]
fn a_nested_insert_draws_its_key_once() {
    let (s, n) = adhoc_normal(
        "",
        &[],
        r#"insert follows { src: key (insert users { name: "nest" }), dst: key (insert users { name: "other" }) }"#,
    );
    assert_normal(&s, &n.terms, n.items[0]);
    assert_eq!(2, fresh_draws(&n.terms, n.items[0]));
}

/// Returns the number of distinct calls of host function `name` reachable from `root`.
fn host_calls(s: &Schema, terms: &Terms, root: Tm, name: &str) -> usize {
    reachable(terms, root)
        .into_iter()
        .filter(
            |t| matches!(terms.kind(*t), Kind::Call { func, .. } if s.function(*func).name == name),
        )
        .count()
}

const STAMPS: &str = r#"fn stamp (t: Text) -> Text = host "stamp" version "1"
fn stamped (t: Text) -> Text = stamp(t)
fn twice (t: Text) -> Bool = stamped(t) == stamped(t)
"#;

#[test]
fn two_call_sites_of_a_volatile_host_function_with_the_same_text_stay_two_calls() {
    let cases = [
        "from u in users select stamp(u.name) == stamp(u.name)",
        "from u in users select stamped(u.name) == stamped(u.name)",
        "from u in users select twice(u.name)",
        r#"stamp("x") == stamp("x")"#,
        r#"stamped("x") == stamped("x")"#,
        r#"twice("x")"#,
    ];
    for text in cases {
        let (s, n) = adhoc_normal(STAMPS, &[], text);
        assert_eq!(2, host_calls(&s, &n.terms, n.items[0], "stamp"), "`{text}`");
    }
}

#[test]
fn one_call_site_of_a_volatile_host_function_bound_by_let_stays_one_call() {
    let (s, n) = adhoc_normal(
        STAMPS,
        &[],
        "from u in users, s in [stamp(u.name)] select s == s",
    );
    assert_eq!(1, host_calls(&s, &n.terms, n.items[0], "stamp"));
}

/// The depth of the `if`s over a shared term.
const SHARED_DEPTH: u32 = 64;

/// Returns an arena with a test term and a variable for a row.
fn shared_arena() -> (Terms, Tm, Var) {
    let mut terms = Terms::default();
    let c = terms.var("c", Ty::Unit);
    let test = terms.add(Kind::Var(c), Ty::Unit);
    let row = terms.var("x", Ty::Unit);
    (terms, test, row)
}

#[test]
fn terms_any_visits_a_shared_term_once() {
    let (mut terms, test, row) = shared_arena();
    let leaf = terms.add(Kind::Var(row), Ty::Unit);
    let t = terms.doubled(test, leaf, 20);
    let mut visits = 0;
    assert!(!terms.any(t, |_| {
        visits += 1;
        false
    }));
    assert_eq!(22, visits);
    let t = terms.doubled(test, leaf, SHARED_DEPTH);
    assert!(!terms.writes(t));
}

#[test]
fn a_write_under_shared_branches_is_checked_for_volatile_reads_once() {
    let s = schema("");
    let (mut terms, test, row) = shared_arena();
    let rows = terms.add(Kind::Var(row), Ty::Unit);
    let insert = terms.add(Kind::Insert(RelId(0), rows), Ty::Change);
    let t = terms.doubled(test, insert, SHARED_DEPTH);
    assert!(!write_depends_on_volatile(&s, &terms, t));
}

#[test]
fn a_sum_over_shared_branches_is_checked_for_merged_bindings_once() {
    let (mut terms, test, row) = shared_arena();
    let users = terms.add(Kind::Rel(RelId(0), None), Ty::Unit);
    let element = terms.add(Kind::Var(row), Ty::Unit);
    let yielded = terms.add(Kind::Singleton(element), Ty::Unit);
    let each = terms.add(Kind::For(row, users, yielded), Ty::Unit);
    let shared_comprehensions = terms.doubled(test, each, SHARED_DEPTH);
    let shared_source = {
        let source = terms.doubled(test, users, SHARED_DEPTH);
        terms.add(Kind::For(row, source, yielded), Ty::Unit)
    };
    let shared_select = {
        let body = terms.doubled(test, yielded, SHARED_DEPTH);
        terms.add(Kind::For(row, users, body), Ty::Unit)
    };
    for (what, source) in [
        ("comprehensions", shared_comprehensions),
        ("source", shared_source),
        ("select", shared_select),
    ] {
        let y = terms.var("y", Ty::Unit);
        let body = terms.add(Kind::Var(y), Ty::Unit);
        let sum = terms.add(
            Kind::Agg {
                op: Agg::Sum,
                var: y,
                source,
                body,
            },
            Ty::Unit,
        );
        assert_eq!(Ok(()), ranges_over_bindings(&terms, sum), "shared {what}");
    }
}

#[test]
fn ad_hoc_text_over_a_doubling_chain_of_functions_checks() {
    let doubling: String = (0..40)
        .map(|i| format!("fn d{} (x: I64) -> I64 = d{i}(x) + d{i}(x)\n", i + 1))
        .collect();
    let extra = format!("fn d0 (x: I64) -> I64 = x + 1\n{doubling}");
    for text in [
        "from o in orders select d40(o.total)",
        "transact insert orders { user: users.key(1), total: d40(2), qty: 1 }",
        "transact if d40(1) > 0 then insert orders { user: users.key(1), total: d40(2), qty: 1 } else skip",
    ] {
        if let Err(e) = adhoc(&extra, &[], text) {
            panic!("`{text}` is refused: {e:?}");
        }
    }
}

#[test]
fn a_fresh_draw_is_total_and_not_copyable() {
    let (s, n) = adhoc_normal("", &[], r#"insert users { name: "eve" }"#);
    let fresh = reachable(&n.terms, n.items[0])
        .into_iter()
        .find(|t| matches!(n.terms.kind(*t), Kind::Fresh(..)))
        .expect("a fresh draw");
    let mut tot = Totality::new(&s, false);
    assert!(tot.total(&n.terms, fresh));
    assert!(tot.draws(&n.terms, fresh));
    assert!(!tot.copyable(&n.terms, fresh));
}

#[test]
fn a_call_of_a_host_function_draws_only_when_volatile() {
    let extra = r#"fn pure (t: Text) -> Text = host "pure" version "1" deterministic"#;
    let (s, n) = adhoc_normal(
        &format!("{STAMPS}\n{extra}"),
        &[],
        r#"[stamp("x"), pure("x")]"#,
    );
    let mut tot = Totality::new(&s, false);
    let call = |name: &str| {
        reachable(&n.terms, n.items[0])
            .into_iter()
            .find(|t| matches!(n.terms.kind(*t), Kind::Call { func, .. } if s.function(*func).name == name))
            .expect("a call")
    };
    assert!(tot.draws(&n.terms, call("stamp")));
    assert!(!tot.draws(&n.terms, call("pure")));
}

#[test]
fn a_constant_is_copyable() {
    let (s, n) = adhoc_normal("", &[], "[1]");
    let constant = reachable(&n.terms, n.items[0])
        .into_iter()
        .find(|t| matches!(n.terms.kind(*t), Kind::Const(_)))
        .expect("a constant");
    assert!(Totality::new(&s, false).copyable(&n.terms, constant));
}
