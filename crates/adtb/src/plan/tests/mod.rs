mod fixture;
mod interp;

use std::collections::BTreeSet;

use adtb_lang::kernel::{Kind, Terms, Tm, Var};
use adtb_reference::{Abort, Fault, R, Value};
use adtb_testkit::uuid;

use super::*;
use fixture::{Fixture, running, variants};
use interp::Interp;

fn plan_with(f: &Fixture, terms: &Terms, t: Tm, params: &[Var], rewrites: Rewrites) -> Plan {
    let options = Options {
        rewrites,
        hiding: false,
    };
    plan(&f.schema, &f.catalog, terms, t, params, options)
        .unwrap_or_else(|e| panic!("no plan: {e}"))
}

fn run(f: &Fixture, p: &Plan, bound: &[(Var, Value)]) -> R<Value> {
    Interp::new(&f.schema, &f.program, &f.db, &f.catalog, p).run(p, bound)
}

/// Plans ad hoc `text` under every variant and checks each plan's outcome against the reference interpreter's.
fn outcome(f: &Fixture, text: &str, params: &[(&str, &str, Value)]) -> R<Value> {
    let types: Vec<(&str, &str)> = params.iter().map(|(n, t, _)| (*n, *t)).collect();
    let (n, t) = f.normal(text, &types);
    let bound: Vec<(Var, Value)> = params
        .iter()
        .map(|(name, _, v)| (n.params[*name], v.clone()))
        .collect();
    let vars: Vec<Var> = bound.iter().map(|(v, _)| *v).collect();
    let want = f.reference(text, params);
    for (name, rw) in variants() {
        let p = plan_with(f, &n.terms, t, &vars, rw);
        assert_eq!(want, run(f, &p, &bound), "`{text}` with {name}");
    }
    want
}

/// Checks that ad hoc `text` has a value, and that every variant's plan computes it.
fn agrees(f: &Fixture, text: &str, params: &[(&str, &str, Value)]) {
    if let Err(a) = outcome(f, text, params) {
        panic!("`{text}` aborts: {a:?}")
    }
}

/// Checks that ad hoc `text` aborts, and that every variant's plan aborts the same way.
fn aborts(f: &Fixture, text: &str) -> Abort {
    outcome(f, text, &[]).expect_err("an abort")
}

/// Plans registered query `decl` under every variant and checks that each plan computes the reference interpreter's value on `args`.
fn agrees_query(f: &Fixture, decl: &str, args: Vec<Value>) {
    let q = f.query(decl);
    let bound: Vec<(Var, Value)> = q.params.iter().copied().zip(args).collect();
    let want = adtb_reference::Interpreter::new(&f.schema, &f.program, &f.db)
        .eval(&q.terms, q.body, &bound);
    for (name, rw) in variants() {
        let p = plan_with(f, &q.terms, q.body, &q.params, rw);
        assert_eq!(want, run(f, &p, &bound), "`{decl}` with {name}");
    }
    assert!(want.is_ok(), "`{decl}` aborts: {want:?}");
}

fn adhoc_plan(f: &Fixture, text: &str, params: &[(&str, &str)], rw: Rewrites) -> Plan {
    let (n, t) = f.normal(text, params);
    let vars: Vec<Var> = params.iter().map(|(name, _)| n.params[*name]).collect();
    plan_with(f, &n.terms, t, &vars, rw)
}

fn key(i: i64) -> Value {
    Value::key(Value::Int(i))
}

fn ident(f: &Fixture, n: u8) -> Value {
    f.reference(&uuid(n), &[]).expect("a key")
}

fn heap(f: &Fixture, rel: &str, ctor: u32) -> HeapId {
    f.catalog.heap(f.rel(rel), ctor)
}

fn query_of(e: &Eval) -> &Query {
    match e {
        Eval::Query(q) => q,
        other => panic!("not a query: {other:?}"),
    }
}

fn pipelines(s: &Spine) -> Vec<&Pipeline> {
    match s {
        Spine::Pipeline(p) => vec![p],
        Spine::Union(ss) => ss.iter().flat_map(pipelines).collect(),
        Spine::If(_, a, b) => pipelines(a).into_iter().chain(pipelines(b)).collect(),
        Spine::Case(_, arms) => arms.iter().flat_map(|(_, _, s)| pipelines(s)).collect(),
        Spine::Let(_, _, b) => pipelines(b),
    }
}

fn ends(c: &Chain) -> Vec<&End> {
    let mut out = vec![&c.end];
    match &c.end {
        End::Fork(cs) => out.extend(cs.iter().flat_map(ends)),
        End::If(_, a, b) => {
            out.extend(ends(a));
            out.extend(ends(b));
        }
        End::Case(_, arms) => out.extend(arms.iter().flat_map(|(_, _, c)| ends(c))),
        End::Index { routes, .. } => out.extend(routes.iter().flat_map(|r| ends(&r.chain))),
        End::Nothing | End::Emit(_) => {}
    }
    out
}

fn steps(c: &Chain) -> Vec<&Step> {
    let mut out: Vec<&Step> = c.steps.iter().collect();
    match &c.end {
        End::Fork(cs) => out.extend(cs.iter().flat_map(steps)),
        End::If(_, a, b) => {
            out.extend(steps(a));
            out.extend(steps(b));
        }
        End::Case(_, arms) => out.extend(arms.iter().flat_map(|(_, _, c)| steps(c))),
        End::Index { routes, .. } => out.extend(routes.iter().flat_map(|r| steps(&r.chain))),
        End::Nothing | End::Emit(_) => {}
    }
    out
}

const MATCH: &str = "from i in identities select match i | EmailPass e -> e.email | Google g -> g.gid | Apple a -> a.sub";

const JOIN: &str = "from i in identities, u in users where i.user == key u
                    select { name: u.name, login: match i | EmailPass e -> e.email | Google g -> g.gid | Apple a -> a.sub }";

const REACH: &str = "fix reach = (from f in follows where f.src == me select f.dst)
          union (from r in reach, f in follows where f.src == r select f.dst)
in from u in users where exists r in reach where r == key u select u";

const SEARCH: &str = "query search (name: Option Text, after: Option Instant) -> Set Post =
               from p in posts
               where match name  | None -> true | Some { value: n } -> p.title == n
               where match after | None -> true | Some { value: t } -> p.created_at > t
               select p";

const GROUP: &str =
    "from i in identities group i by i.user into g select { user: g.key, n: count(x for x in g) }";

fn emits(c: &Chain) -> Vec<Tm> {
    ends(c)
        .into_iter()
        .filter_map(|e| match e {
            End::Emit(t) => Some(*t),
            _ => None,
        })
        .collect()
}

fn has_case(terms: &Terms, t: Tm) -> bool {
    matches!(terms.kind(t), Kind::Case { .. })
        || terms.children(t).into_iter().any(|c| has_case(terms, c))
}

#[test]
fn a_match_over_a_relation_runs_one_pipeline_per_arm() {
    let f = running();
    agrees(&f, MATCH, &[]);
    let p = adhoc_plan(&f, MATCH, &[], Rewrites::ALL);
    let ps = pipelines(&query_of(&p.root).spine);
    assert_eq!(3, ps.len());
    for (c, x) in ps.iter().enumerate() {
        assert!(matches!(x.source, Source::Scan { ctor, .. } if ctor == c as u32));
        assert!(emits(&x.chain).iter().all(|t| !has_case(&p.terms, *t)));
    }
}

#[test]
fn without_static_routing_each_row_is_routed_by_its_tag() {
    let f = running();
    let p = adhoc_plan(
        &f,
        MATCH,
        &[],
        Rewrites {
            static_route: false,
            ..Rewrites::ALL
        },
    );
    let ps = pipelines(&query_of(&p.root).spine);
    assert_eq!(3, ps.len());
    for x in ps {
        assert!(emits(&x.chain).iter().all(|t| has_case(&p.terms, *t)));
    }
    let p = adhoc_plan(
        &f,
        JOIN,
        &[],
        Rewrites {
            static_route: false,
            ..Rewrites::ALL
        },
    );
    let ps = pipelines(&query_of(&p.root).spine);
    for x in ps {
        assert!(emits(&x.chain).iter().all(|t| has_case(&p.terms, *t)));
    }
}

#[test]
fn a_constructor_relation_reads_only_its_heaps() {
    let f = running();
    let text = "from g in identities.Google select g.gid";
    agrees(&f, text, &[]);
    let p = adhoc_plan(&f, text, &[], Rewrites::ALL);
    assert_eq!(BTreeSet::from([heap(&f, "identities", 1)]), p.reads.heaps);
    let p = adhoc_plan(
        &f,
        text,
        &[],
        Rewrites {
            prune: false,
            ..Rewrites::ALL
        },
    );
    assert_eq!(3, p.reads.heaps.len());
}

#[test]
fn an_arm_whose_condition_is_false_reads_no_heap() {
    let f = running();
    let text = r#"from i in identities where match i | Google g -> g.gid == "root" | _ -> false select key i"#;
    agrees(&f, text, &[]);
    let p = adhoc_plan(&f, text, &[], Rewrites::ALL);
    assert_eq!(BTreeSet::from([heap(&f, "identities", 1)]), p.reads.heaps);
}

#[test]
fn a_key_probe_routes_each_entry_to_its_arm() {
    let f = running();
    let text = "from i in identities where i.id == k
                select match i | EmailPass e -> e.email | Google g -> g.gid | Apple a -> a.sub";
    for n in 1..=4 {
        agrees(&f, text, &[("k", "Key identities", ident(&f, n))]);
    }
    let params = [("k", "Key identities")];
    let p = adhoc_plan(&f, text, &params, Rewrites::ALL);
    let ps = pipelines(&query_of(&p.root).spine);
    assert_eq!(1, ps.len());
    let End::Index { index, routes, .. } = &ps[0].chain.end else {
        panic!("a key probe: {:?}", ps[0].chain)
    };
    assert_eq!(f.catalog.relation(f.rel("identities")).key_index, *index);
    assert_eq!(
        vec![0, 1, 2],
        routes.iter().map(|r| r.ctor).collect::<Vec<_>>()
    );
    let p = adhoc_plan(
        &f,
        text,
        &params,
        Rewrites {
            fuse_probe: false,
            ..Rewrites::ALL
        },
    );
    assert_eq!(3, pipelines(&query_of(&p.root).spine).len());
    let p = adhoc_plan(
        &f,
        text,
        &params,
        Rewrites {
            index_scan: false,
            ..Rewrites::ALL
        },
    );
    assert!(p.reads.indexes.is_empty());
}

#[test]
fn a_probe_of_an_index_over_one_constructor_reads_only_it() {
    let f = running();
    let text = r#"from i in identities.Google where i.gid == "root" select key i"#;
    agrees(&f, text, &[]);
    let p = adhoc_plan(&f, text, &[], Rewrites::ALL);
    let ix = &f.catalog.relation(f.rel("identities")).indexes;
    let gid = ix
        .iter()
        .find(|i| i.fields == ["gid"])
        .expect("the gid index")
        .id;
    assert_eq!(BTreeSet::from([gid]), p.reads.indexes);
    assert_eq!(BTreeSet::from([heap(&f, "identities", 1)]), p.reads.heaps);
}

#[test]
fn a_join_into_every_arm_shares_one_build() {
    let f = running();
    agrees(&f, JOIN, &[]);
    let p = adhoc_plan(&f, JOIN, &[], Rewrites::ALL);
    let q = query_of(&p.root);
    assert_eq!(1, q.builds.len());
    let ps = pipelines(&q.spine);
    assert_eq!(3, ps.len());
    for p in ps {
        let probes = steps(&p.chain)
            .into_iter()
            .filter(|s| matches!(s, Step::Probe { build: BuildId(0), key, .. } if key.len() == 1))
            .count();
        assert_eq!(1, probes, "{:?}", p.chain);
    }
}

#[test]
fn a_join_without_hash_keys_is_a_nested_loop() {
    let f = running();
    let rw = Rewrites {
        hash_join: false,
        index_join: false,
        ..Rewrites::ALL
    };
    let p = adhoc_plan(&f, JOIN, &[], rw);
    let q = query_of(&p.root);
    assert_eq!(1, q.builds.len());
    assert!(q.builds[0].key.is_empty());
}

#[test]
fn a_point_lookup_joins_through_an_index() {
    let f = running();
    let text = "from u in users, i in identities where key u == me and i.user == key u select i.id";
    agrees(&f, text, &[("me", "Key users", key(1))]);
    let p = adhoc_plan(&f, text, &[("me", "Key users")], Rewrites::ALL);
    let q = query_of(&p.root);
    assert!(q.builds.is_empty());
    let ps = pipelines(&q.spine);
    assert_eq!(1, ps.len());
    let probes = ends(&ps[0].chain)
        .into_iter()
        .filter(|e| matches!(e, End::Index { .. }))
        .count();
    assert_eq!(2, probes);
}

#[test]
fn a_path_dereferences_through_the_key_index() {
    let f = running();
    let text = "from i in identities select i.user.name";
    agrees(&f, text, &[]);
    let p = adhoc_plan(&f, text, &[], Rewrites::ALL);
    let users = f.catalog.relation(f.rel("users")).key_index;
    assert!(p.reads.indexes.contains(&users));
}

#[test]
fn a_correlated_count_probes_the_index_per_row() {
    let f = running();
    let text = "from u in users select { name: u.name, n: count(i for i in u.<identities.user) }";
    agrees(&f, text, &[]);
    let p = adhoc_plan(&f, text, &[], Rewrites::ALL);
    let ix = &f.catalog.relation(f.rel("identities")).indexes;
    let user = ix
        .iter()
        .find(|i| i.fields == ["user"])
        .expect("the user index")
        .id;
    assert!(p.reads.indexes.contains(&user));
}

#[test]
fn grouped_aggregates_are_computed_by_the_group() {
    let f = running();
    agrees(&f, GROUP, &[]);
    let p = adhoc_plan(&f, GROUP, &[], Rewrites::ALL);
    let ps = pipelines(&query_of(&p.root).spine);
    let group = steps(&ps[0].chain)
        .into_iter()
        .find_map(|s| match s {
            Step::Bind(_, Eval::Query(q)) => match &q.sink {
                Sink::Group { rows, aggs, .. } => Some((*rows, aggs.len())),
                _ => None,
            },
            _ => None,
        })
        .expect("a group");
    assert_eq!((false, 1), group);
}

#[test]
fn a_partial_sum_in_a_strict_position_is_computed_by_the_group() {
    let f = running();
    agrees(
        &f,
        "from o in orders group o by o.user into g select { user: g.key, total: sum(x.total for x in g) }",
        &[],
    );
    agrees(
        &f,
        "from o in orders group o by o.user into g
         select { user: g.key, top: max(x.total for x in g), n: count(x for x in g where x.qty > 0) }",
        &[],
    );
    agrees(
        &f,
        "from o in orders group o by o.user into g
         select { user: g.key, ids: from x in g select x.id, n: count(x for x in g) }",
        &[],
    );
}

#[test]
fn a_sum_under_a_guard_is_left_to_the_body() {
    let f = running();
    agrees(
        &f,
        "from o in orders group o by o.user into g
         select { user: g.key, big: if count(x for x in g) > 1 then sum(x.total for x in g) else 0 }",
        &[],
    );
}

#[test]
fn a_fixpoint_iterates_on_deltas() {
    let f = running();
    for me in 1..=3 {
        agrees(&f, REACH, &[("me", "Key users", key(me))]);
    }
    let params = [("me", "Key users")];
    let p = adhoc_plan(&f, REACH, &params, Rewrites::ALL);
    let Eval::Fix(fix) = &p.root else {
        panic!("a fixpoint: {:?}", p.root)
    };
    assert!(fix.binds.iter().all(|b| b.step.is_some()));
    let p = adhoc_plan(
        &f,
        REACH,
        &params,
        Rewrites {
            semi_naive: false,
            ..Rewrites::ALL
        },
    );
    let Eval::Fix(fix) = &p.root else {
        panic!("a fixpoint: {:?}", p.root)
    };
    assert!(fix.binds.iter().all(|b| b.step.is_none()));
}

#[test]
fn mutually_recursive_relations_iterate_together() {
    let f = running();
    agrees(
        &f,
        "fix even = (from u in users where key u == me select key u)
                    union (from o in odd, f in follows where f.src == o select f.dst),
             odd  = from v in even, f in follows where f.src == v select f.dst
         in from u in users where exists o in odd where o == key u select u",
        &[("me", "Key users", key(1))],
    );
}

#[test]
fn a_lower_stratum_is_negated() {
    let f = running();
    agrees(
        &f,
        "fix reach = (from f in follows where f.src == me select f.dst)
                   union (from r in reach, f in follows where f.src == r select f.dst)
         in from u in users where not exists r in reach where r == key u select u",
        &[("me", "Key users", key(1))],
    );
}

#[test]
fn an_option_parameter_chooses_the_pipelines_at_start() {
    let f = running();
    for args in [
        vec![Value::some(Value::text("again")), Value::none()],
        vec![Value::none(), Value::some(Value::Instant(35))],
        vec![Value::none(), Value::none()],
        vec![
            Value::some(Value::text("fifth")),
            Value::some(Value::Instant(15)),
        ],
    ] {
        agrees_query(&f, SEARCH, args);
    }
    let q = f.query(SEARCH);
    let p = plan_with(&f, &q.terms, q.body, &q.params, Rewrites::ALL);
    let spine = &query_of(&p.root).spine;
    assert!(matches!(spine, Spine::Case(..)), "{spine:?}");
    assert_eq!(4, pipelines(spine).len());
    let p = plan_with(
        &f,
        &q.terms,
        q.body,
        &q.params,
        Rewrites {
            unswitch: false,
            ..Rewrites::ALL
        },
    );
    assert!(matches!(query_of(&p.root).spine, Spine::Pipeline(_)));
}

#[test]
fn order_by_lists_the_final_result() {
    let f = running();
    agrees(&f, "from u in users select u order by u.name desc", &[]);
    agrees(
        &f,
        "from p in posts select p order by (p.created_at desc, key p) take 2",
        &[],
    );
    agrees_query(
        &f,
        "query page (cursor: Option (Key users)) -> Set User =
           from u in users select u order by u.id take 50 after cursor",
        vec![Value::some(key(1))],
    );
}

#[test]
fn top_takes_the_first_per_row() {
    let f = running();
    agrees(
        &f,
        "from u in users
         select { user: u.name,
                  latest: top 2 by (p.created_at desc, key p) (from p in posts where p.author == key u select p) }",
        &[],
    );
}

#[test]
fn a_guard_keeps_a_partial_operand_from_running() {
    let f = running();
    agrees(
        &f,
        "from o in orders where o.qty > 0 and o.total / o.qty > 10 select o.id",
        &[],
    );
    let got = aborts(
        &f,
        "from o in orders where o.total / o.qty > 10 select o.id",
    );
    assert_eq!(Abort::Fault(Fault::DivisionByZero), got);
}

#[test]
fn a_partial_condition_stays_ahead_of_the_join() {
    let f = running();
    let got = aborts(
        &f,
        "from o in orders, u in users where o.total / o.qty > 10 and o.user == key u select u.name",
    );
    assert_eq!(Abort::Fault(Fault::DivisionByZero), got);
    agrees(
        &f,
        "from o in orders, u in users where o.qty > 0 and o.total / o.qty > 10 and o.user == key u select u.name",
        &[],
    );
}

#[test]
fn nested_results_keep_rows_with_nothing_related() {
    let f = running();
    agrees(
        &f,
        "from u in users select { name: u.name, phones: from d in phones where d.owner == key u select d.seen }",
        &[],
    );
    agrees(
        &f,
        "from r in (from u in users select { name: u.name, ds: u.<phones.owner }), d in r.ds
         select { name: r.name, seen: d.seen }",
        &[],
    );
}

#[test]
fn queries_of_the_running_schema_agree() {
    let f = running();
    for text in [
        "from i in identities, u in users where key u == i.user select u.name",
        "from p in posts select p.author",
        "from u in users, i in u.<identities.user select { name: u.name, id: key i }",
        "from i in identities select match i | Google g -> g | Apple a -> a | EmailPass e -> e",
        "from i in identities.(EmailPass | Google) select match i | EmailPass e -> e.email | Google g -> g.gid",
        "from i in identities group i by tag(i) into g select { kind: g.key, n: count(x for x in g) }",
        "count(x for x in (from i in identities select match i | EmailPass e -> e.email | Google g -> g.gid | Apple a -> a.sub))",
        "from u in users select { name: u.name, top: max(o.total for o in orders where o.user == key u) }",
        "from u in users select { name: u.name, n: count(s for s in u.<sessions.owner) }",
        "from o in orders select match checked_div(o.total, o.qty) | None -> 0 | Some { value: q } -> q",
        "from o in orders select { id: o.id, r: try (o.total / o.qty) }",
        "[1, 2] union (from o in orders select o.qty)",
        "from u in users where exists p in posts where p.author == key u select u.name",
        "from u in users where not exists p in posts where p.author == key u select u.name",
        "from f in follows, g in follows where f.dst == g.src and g.dst == f.src select { a: f.src, b: g.src }",
        "fix a = (from u in users select key u) union (from x in a where { k: x } != { k: me } select x) in a",
        "{ n: count(u for u in users), names: from u in users select u.name }",
        "if exists u in users where u.name == \"bo\" then count(p for p in posts) else 0",
        "from s in sessions select match s.owner | None -> 0 | Some { value: o } -> count(p for p in posts where p.author == o)",
    ] {
        agrees(&f, text, &[("me", "Key users", key(1))]);
    }
    agrees(
        &f,
        "sum(o.total for o in orders where o.user == u)",
        &[("u", "Key users", key(1))],
    );
}

#[test]
fn a_call_of_a_host_function_is_refused() {
    let f = Fixture::new(
        "type User = { id: Key users, name: Text }
         relation users : User key id generate sequence
         fn slugify (t: Text) -> Text = host \"slugify\" version \"1.0\" deterministic",
    );
    let (n, t) = f.normal("from u in users select slugify(u.name)", &[]);
    let got = plan(&f.schema, &f.catalog, &n.terms, t, &[], Options::default()).map(|_| ());
    assert_eq!(Err(Error::UserCode("slugify".into())), got);
}

#[test]
fn a_write_is_refused() {
    let f = running();
    let (n, t) = f.normal("delete u in users where u.name == \"cy\"", &[]);
    let got = plan(&f.schema, &f.catalog, &n.terms, t, &[], Options::default()).map(|_| ());
    assert_eq!(Err(Error::NotAQuery("a write")), got);
}

#[test]
fn a_function_value_in_a_normal_form_is_an_internal_error() {
    let f = running();
    let mut terms = Terms::default();
    let int = Ty::Base(adtb_lang::types::Base::I64);
    let x = terms.var("x", int.clone());
    let body = terms.add(Kind::Var(x), int.clone());
    let t = terms.add(
        Kind::Lam(x, body),
        Ty::Fn(Box::new(int.clone()), Box::new(int)),
    );
    let got = plan(&f.schema, &f.catalog, &terms, t, &[], Options::default()).map(|_| ());
    assert_eq!(
        Err(Error::Internal(
            "a normal form holds no function value".into()
        )),
        got
    );
}

#[test]
fn a_dereference_of_a_value_that_is_not_a_key_is_an_internal_error() {
    let f = running();
    let mut terms = Terms::default();
    let int = Ty::Base(adtb_lang::types::Base::I64);
    let key = terms.add(Kind::Const(adtb_lang::kernel::Const::Int(1)), int);
    let t = terms.add(
        Kind::Deref {
            key,
            reference: false,
        },
        f.ty("User"),
    );
    let got = plan(&f.schema, &f.catalog, &terms, t, &[], Options::default()).map(|_| ());
    assert_eq!(
        Err(Error::Internal("a dereference follows a key".into())),
        got
    );
}

/// Returns the bounded plan of `text` over the set parameter `xs` of type `ty`.
fn bounded(f: &Fixture, text: &str, ty: &str) -> Result<Plan, Error> {
    let (normal, t) = f.normal(text, &[("xs", ty)]);
    plan_bounded(
        &f.schema,
        &f.catalog,
        &normal.terms,
        t,
        &[normal.params["xs"]],
    )
}

#[test]
fn a_bounded_plan_reads_every_relation_through_an_index_probe() {
    let f = running();
    let text = "from x in xs, i in identities where i.user == x \
                and not exists p in posts where p.author == x select i.id";
    let p = bounded(&f, text, "Set (Key users)").unwrap_or_else(|e| panic!("no plan: {e}"));
    let shown = format!("{:?}", p.root);
    assert!(!shown.contains("Scan {"), "{shown}");
    let (normal, t) = f.normal(text, &[("xs", "Set (Key users)")]);
    let none = plan_with(&f, &normal.terms, t, &[normal.params["xs"]], Rewrites::NONE);
    assert!(format!("{:?}", none.root).contains("Scan {"));
}

#[test]
fn a_bounded_plan_refuses_a_read_no_index_reaches() {
    let f = running();
    let e = bounded(
        &f,
        "from x in xs, p in phones where p.owner == x select p",
        "Set (Key users)",
    )
    .expect_err("phones has no index on owner");
    assert_eq!(
        Error::Unbounded {
            rel: f.rel("phones"),
            ctors: vec![0],
            fields: vec!["owner".to_string()],
        },
        e
    );
}

#[test]
fn a_bounded_plan_refuses_a_fixpoint() {
    let f = running();
    let text = "fix reach = xs union (from r in reach, g in follows where g.src == r select g.dst) \
                in reach";
    let e = bounded(&f, text, "Set (Key users)").expect_err("a fix is outside the bound");
    assert_eq!(Error::Outside("fix"), e);
}
