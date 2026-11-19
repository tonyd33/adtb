mod valid;

use std::collections::BTreeSet;

use adtb_lang::kernel::{Kind, Terms, Tm};

use super::fixture::{Fixture, running, variants};
use super::*;

fn plan_with(f: &Fixture, terms: &Terms, t: Tm, rewrites: Rewrites) -> Plan {
    let options = Options {
        rewrites,
        hiding: false,
    };
    plan(&f.schema, &f.normal, &f.catalog, terms, t, options)
        .unwrap_or_else(|e| panic!("no plan: {e}"))
}

fn adhoc_plan(f: &Fixture, text: &str, params: &[(&str, &str)], rw: Rewrites) -> Plan {
    let (n, t) = f.normal(text, params);
    plan_with(f, &n.terms, t, rw)
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
        End::Index { routes, .. } | End::Range { routes, .. } => {
            out.extend(routes.iter().flat_map(|r| ends(&r.chain)));
        }
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
        End::Index { routes, .. } | End::Range { routes, .. } => {
            out.extend(routes.iter().flat_map(|r| steps(&r.chain)));
        }
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

fn routes_by_tag(f: &Fixture, terms: &Terms, t: Tm) -> bool {
    crate::exec::routes_by_tag(&f.schema, terms, t).expect("an emitted term compiles")
}

#[test]
fn a_match_over_a_relation_runs_one_pipeline_per_arm() {
    let f = running();
    let p = adhoc_plan(&f, MATCH, &[], Rewrites::ALL);
    let ps = pipelines(&query_of(&p.root).spine);
    assert_eq!(3, ps.len());
    for (c, x) in ps.iter().enumerate() {
        assert!(matches!(x.source, Source::Scan { ctor, .. } if ctor == c as u32));
        assert!(
            emits(&x.chain)
                .iter()
                .all(|t| !routes_by_tag(&f, &p.terms, *t))
        );
    }
}

#[test]
fn a_constructor_relation_reads_only_its_heaps() {
    let f = running();
    let text = "from g in identities.Google select g.gid";
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
    let p = adhoc_plan(&f, text, &[], Rewrites::ALL);
    assert_eq!(BTreeSet::from([heap(&f, "identities", 1)]), p.reads.heaps);
}

#[test]
fn a_key_probe_routes_each_entry_to_its_arm() {
    let f = running();
    let text = "from i in identities where i.id == k
                select match i | EmailPass e -> e.email | Google g -> g.gid | Apple a -> a.sub";
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
fn a_probe_of_an_index_over_one_constructor_reads_it_and_the_heaps_its_fetch_walks() {
    let f = running();
    let text = r#"from i in identities.Google where i.gid == "root" select key i"#;
    let p = adhoc_plan(&f, text, &[], Rewrites::ALL);
    let ix = &f.catalog.relation(f.rel("identities")).indexes;
    let gid = ix
        .iter()
        .find(|i| i.fields() == Some(vec!["gid"]))
        .expect("the gid index")
        .id;
    assert_eq!(BTreeSet::from([gid]), p.reads.indexes);
    let sum: BTreeSet<HeapId> = (0..3).map(|c| heap(&f, "identities", c)).collect();
    assert_eq!(sum, p.reads.heaps);
}

#[test]
fn a_join_into_every_arm_shares_one_build() {
    let f = running();
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
fn a_point_lookup_joins_through_an_index() {
    let f = running();
    let text = "from u in users, i in identities where key u == me and i.user == key u select i.id";
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
    let p = adhoc_plan(&f, text, &[], Rewrites::ALL);
    let users = f.catalog.relation(f.rel("users")).key_index;
    assert!(p.reads.indexes.contains(&users));
}

#[test]
fn grouped_aggregates_are_computed_by_the_group() {
    let f = running();
    assert_eq!((false, 1), group_of(&f, GROUP));
}

/// Returns whether the group of `text` keeps its rows, and how many aggregates it computes.
fn group_of(f: &Fixture, text: &str) -> (bool, usize) {
    let p = adhoc_plan(f, text, &[], Rewrites::ALL);
    let ps = pipelines(&query_of(&p.root).spine);
    steps(&ps[0].chain)
        .into_iter()
        .find_map(|s| match s {
            Step::Bind(_, Eval::Query(q)) => match &q.sink {
                Sink::Group { rows, aggs, .. } => Some((*rows, aggs.len())),
                _ => None,
            },
            _ => None,
        })
        .expect("a group")
}

#[test]
fn a_partial_sum_in_a_strict_position_is_computed_by_the_group() {
    let f = running();
    for (text, group) in [
        (
            "from o in orders group o by o.user into g select { user: g.key, total: sum(x.total for x in g) }",
            (false, 1),
        ),
        (
            "from o in orders group o by o.user into g
         select { user: g.key, top: max(x.total for x in g), n: count(x for x in g where x.qty > 0) }",
            (false, 2),
        ),
        (
            "from o in orders group o by o.user into g
         select { user: g.key, ids: from x in g select x.id, n: count(x for x in g) }",
            (true, 1),
        ),
    ] {
        assert_eq!(group, group_of(&f, text), "{text}");
    }
}

#[test]
fn no_field_of_a_fused_group_plan_reads_a_label_its_record_lacks() {
    let f = running();
    assert_eq!((false, 1), group_of(&f, GROUP));
    let p = adhoc_plan(&f, GROUP, &[], Rewrites::ALL);
    let missing: Vec<String> = p
        .terms
        .iter()
        .filter_map(|n| match &n.kind {
            Kind::Field(e, l) => match p.terms.ty(*e) {
                Ty::Record(fs) if !fs.iter().any(|(name, _)| name == l) => Some(l.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(Vec::<String>::new(), missing);
}

#[test]
fn a_sum_under_a_guard_is_left_to_the_body() {
    let f = running();
    let text = "from o in orders group o by o.user into g
         select { user: g.key, big: if count(x for x in g) > 1 then sum(x.total for x in g) else 0 }";
    assert_eq!((true, 1), group_of(&f, text));
}

#[test]
fn a_fixpoint_iterates_on_deltas() {
    let f = running();
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
fn an_option_parameter_chooses_the_pipelines_at_start() {
    let f = running();
    let q = f.query(SEARCH);
    let p = plan_with(&f, &q.terms, q.body, Rewrites::ALL);
    let spine = &query_of(&p.root).spine;
    assert!(matches!(spine, Spine::Case(..)), "{spine:?}");
    assert_eq!(4, pipelines(spine).len());
    let p = plan_with(
        &f,
        &q.terms,
        q.body,
        Rewrites {
            unswitch: false,
            ..Rewrites::ALL
        },
    );
    assert!(matches!(query_of(&p.root).spine, Spine::Pipeline(_)));
}

#[test]
fn a_call_of_a_host_function_plans() {
    let f = Fixture::new(
        "type User = { id: Key users, name: Text }
         relation users : User key id generate sequence
         fn slugify (t: Text) -> Text = host \"slugify\" version \"1.0\" deterministic",
    );
    let (n, t) = f.normal("from u in users select slugify(u.name)", &[]);
    let got = plan(
        &f.schema,
        &f.normal,
        &f.catalog,
        &n.terms,
        t,
        Options::default(),
    )
    .map(|_| ());
    assert_eq!(Ok(()), got);
}

#[test]
fn a_write_is_refused() {
    let f = running();
    let (n, t) = f.normal("delete u in users where u.name == \"cy\"", &[]);
    let got = plan(
        &f.schema,
        &f.normal,
        &f.catalog,
        &n.terms,
        t,
        Options::default(),
    )
    .map(|_| ());
    assert_eq!(Err(Error::NotAQuery("a write")), got);
}

/// Returns the bounded plan of `text` over the set parameter `xs` of type `ty`.
fn bounded(f: &Fixture, text: &str, ty: &str) -> Result<Plan, Error> {
    let (normal, t) = f.normal(text, &[("xs", ty)]);
    plan_bounded(&f.schema, &f.normal, &f.catalog, &normal.terms, t)
}

#[test]
fn a_bounded_plan_refuses_a_read_no_index_reaches() {
    let f = running();
    let e = bounded(
        &f,
        "from x in xs, p in phones where p.seen == x select p",
        "Set Instant",
    )
    .expect_err("phones has no index on seen");
    assert_eq!(
        Error::Unbounded {
            rel: f.rel("phones"),
            ctors: vec![0],
            fields: vec!["seen".to_string()],
        },
        e
    );
}

fn probes_only(text: &str, ty: &str) {
    let f = running();
    let p = bounded(&f, text, ty).unwrap_or_else(|e| panic!("no plan for `{text}`: {e}"));
    let shown = format!("{:?}", p.root);
    assert!(!shown.contains("Scan {"), "{text}: {shown}");
}

#[test]
fn a_bounded_plan_narrows_a_generator_matched_to_one_constructor() {
    probes_only(
        "from x in xs, i in identities \
         where (match i | EmailPass e -> false | Google g -> g.gid == x | Apple a -> false) \
         select key i",
        "Set Text",
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

fn invoices() -> Fixture {
    let mut f = Fixture::new(
        "type Status = Draft | Paid { ref: Text }
type Invoice = { id: Key invoices, status: Status, email: Text }
relation invoices : Invoice key id generate sequence
  unique (status.Paid.ref)
  index (lower(email))
",
    );
    f.rows(
        "invoices",
        r#"[Invoice { id: invoices.key(1), status: Draft, email: "Ann@X" },
            Invoice { id: invoices.key(2), status: Paid { ref: "r1" }, email: "bo@x" },
            Invoice { id: invoices.key(3), status: Paid { ref: "r2" }, email: "ANN@x" }]"#,
        &[],
    );
    f
}

/// Returns the id of the index of `invoices` whose key is `elements`.
fn invoice_index(f: &Fixture, elements: &[Element]) -> IndexId {
    f.catalog
        .relation(f.rel("invoices"))
        .indexes
        .iter()
        .find(|i| i.elements == elements)
        .expect("the index")
        .id
}

fn lower_email(f: &Fixture) -> IndexId {
    invoice_index(
        f,
        &[Element::Derived {
            index: 1,
            element: 0,
        }],
    )
}

fn paid_ref(f: &Fixture) -> IndexId {
    invoice_index(
        f,
        &[Element::Derived {
            index: 0,
            element: 0,
        }],
    )
}

#[test]
fn an_equality_on_an_indexed_expression_probes_its_index() {
    let f = invoices();
    let text = r#"from i in invoices where lower(i.email) == "ann@x" select key i"#;
    let p = adhoc_plan(&f, text, &[], Rewrites::ALL);
    assert_eq!(BTreeSet::from([lower_email(&f)]), p.reads.indexes);
}

#[test]
fn an_equality_on_another_expression_does_not_probe_an_expression_index() {
    let f = invoices();
    let text = r#"from i in invoices where upper(i.email) == "ANN@X" select key i"#;
    let p = adhoc_plan(&f, text, &[], Rewrites::ALL);
    assert!(p.reads.indexes.is_empty(), "{:?}", p.reads.indexes);
}

#[test]
fn an_arm_naming_a_constructor_probes_a_path_index() {
    let f = invoices();
    for text in [
        r#"from i in invoices, k in (match i.status
             | Paid p -> if p.ref == "r1" then [key i] else []
             | Draft -> []) select k"#,
        r#"from i in invoices where (match i.status | Paid p -> p.ref == "r1" | Draft -> false) select key i"#,
    ] {
        let p = adhoc_plan(&f, text, &[], Rewrites::ALL);
        assert_eq!(BTreeSet::from([paid_ref(&f)]), p.reads.indexes, "`{text}`");
    }
}

#[test]
fn a_path_index_is_not_probed_where_another_arm_yields_rows() {
    let f = invoices();
    let text = r#"from i in invoices, k in (match i.status
         | Paid p -> if p.ref == "r1" then [key i] else []
         | Draft -> [key i]) select k"#;
    let p = adhoc_plan(&f, text, &[], Rewrites::ALL);
    assert!(p.reads.indexes.is_empty(), "{:?}", p.reads.indexes);
}

fn range_ends(p: &Plan) -> Vec<(usize, Option<RangeBound>, Option<RangeBound>)> {
    pipelines(&query_of(&p.root).spine)
        .into_iter()
        .flat_map(|x| ends(&x.chain))
        .filter_map(|e| match e {
            End::Range { key, lo, hi, .. } => Some((key.len(), *lo, *hi)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_comparison_on_an_ordered_index_field_probes_a_range() {
    let f = running();
    for (text, lo, hi) in [
        (
            "from o in orders where o.id > orders.key(2) select o.total",
            Some(false),
            None,
        ),
        (
            "from o in orders where orders.key(2) >= o.id select o.total",
            None,
            Some(true),
        ),
        (
            "from o in orders where o.id >= orders.key(2) and o.id < orders.key(4) select o.total",
            Some(true),
            Some(false),
        ),
        (
            "from o in orders where orders.key(3) < o.id and o.qty > 0 select o.total",
            Some(false),
            None,
        ),
    ] {
        let p = adhoc_plan(&f, text, &[], Rewrites::ALL);
        let got: Vec<(usize, Option<bool>, Option<bool>)> = range_ends(&p)
            .into_iter()
            .map(|(k, l, h)| (k, l.map(|b| b.inclusive), h.map(|b| b.inclusive)))
            .collect();
        assert_eq!(vec![(0, lo, hi)], got, "`{text}`");
        let off = Rewrites {
            range_probe: false,
            ..Rewrites::ALL
        };
        assert!(
            range_ends(&adhoc_plan(&f, text, &[], off)).is_empty(),
            "`{text}`"
        );
    }
}

#[test]
fn a_range_probe_follows_an_equality_on_the_leading_field() {
    let f = running();
    let text =
        "from x in follows where x.src == users.key(2) and x.dst > users.key(2) select x.dst";
    let p = adhoc_plan(&f, text, &[], Rewrites::ALL);
    let got: Vec<usize> = range_ends(&p).into_iter().map(|(k, _, _)| k).collect();
    assert_eq!(vec![1], got);
}

#[test]
fn a_range_probe_joins_on_a_bound_from_the_outer_row() {
    let f = running();
    let text = "from a in orders, b in orders where b.id > a.id and b.qty < a.qty select { a: a.id, b: b.id }";
    let p = adhoc_plan(&f, text, &[], Rewrites::ALL);
    let ends = range_ends(&p);
    assert_eq!(1, ends.len(), "{ends:?}");
    assert!(ends[0].1.is_some() && ends[0].2.is_none(), "{ends:?}");
}

#[test]
fn an_index_that_is_not_ordered_is_not_probed_for_a_range() {
    let mut f = running();
    let orders = f.rel("orders");
    let mut relations = f.catalog.relations().clone();
    for i in &mut relations.get_mut(&orders).expect("orders").indexes {
        i.ordered = false;
    }
    f.catalog = Catalog::new(relations);
    let text = "from o in orders where o.id > orders.key(2) select o.total";
    let p = adhoc_plan(&f, text, &[], Rewrites::ALL);
    assert!(range_ends(&p).is_empty());
}

#[test]
fn a_range_probe_keeps_its_comparisons_as_filters() {
    let f = running();
    let text = "from o in orders where o.id > orders.key(1) and o.id > orders.key(2) select o.id";
    let p = adhoc_plan(&f, text, &[], Rewrites::ALL);
    let filters = pipelines(&query_of(&p.root).spine)
        .into_iter()
        .flat_map(|x| steps(&x.chain))
        .filter(|s| matches!(s, Step::Filter(_)))
        .count();
    assert_eq!(2, filters);
}

#[test]
fn an_existence_test_over_total_pipelines_is_marked_total() {
    let f = running();
    for (text, total) in [
        ("exists p in posts where p.author == users.key(1)", true),
        ("exists o in orders where o.total / o.qty > 10", false),
        (
            "exists o in orders where o.qty > 0 and o.total / o.qty > 10",
            false,
        ),
        ("exists p in posts where p.title == \"hello\"", true),
        (
            "top 0 by (o.total, key o) (from o in orders select o)",
            true,
        ),
        (
            "top 1 by (o.total / o.qty, key o) (from o in orders select o)",
            false,
        ),
    ] {
        let p = adhoc_plan(&f, text, &[], Rewrites::ALL);
        assert_eq!(total, query_of(&p.root).total, "`{text}`");
        let off = Rewrites {
            early_stop: false,
            ..Rewrites::ALL
        };
        assert!(
            !query_of(&adhoc_plan(&f, text, &[], off).root).total,
            "`{text}`"
        );
    }
}

fn has_group_rows(e: &Eval) -> bool {
    match e {
        Eval::GroupRows { .. } => true,
        Eval::Let(_, a, b) | Eval::If(_, a, b) => has_group_rows(a) || has_group_rows(b),
        Eval::Case(_, arms) => arms.iter().any(|(_, _, a)| has_group_rows(a)),
        Eval::Try(a) => has_group_rows(a),
        _ => false,
    }
}

/// Returns how many groupings the root query's spine binds, and how many steps of its pipelines
/// read the rows of a group.
fn flattened(p: &Plan) -> (usize, usize) {
    let q = query_of(&p.root);
    let mut groupings = 0;
    let mut s = &q.spine;
    while let Spine::Let(_, e, rest) = s {
        if matches!(e, Eval::Query(g) if matches!(g.sink, Sink::Group { .. })) {
            groupings += 1;
        }
        s = rest;
    }
    let reads = pipelines(&q.spine)
        .into_iter()
        .flat_map(|x| steps(&x.chain))
        .filter(|s| matches!(s, Step::Bind(_, e) if has_group_rows(e)))
        .count();
    (groupings, reads)
}

#[test]
fn a_correlated_nested_result_is_flattened_into_one_grouping() {
    let f = running();
    for text in [
        "from u in users select { name: u.name, phones: from d in phones where d.owner == key u select d.seen }",
        "from u in users select { name: u.name, n: count(i for i in u.<identities.user) }",
        "from u in users where exists p in posts where p.author == key u select u.name",
        "from u in users select { u: u.name, out: from x in follows where x.src == key u and x.dst == key u select x.dst }",
        "from u in users select from p in posts where p.author == key u and p.title != u.name select p.title",
    ] {
        let p = adhoc_plan(&f, text, &[], Rewrites::ALL);
        assert_eq!((1, 1), flattened(&p), "`{text}`");
        let off = Rewrites {
            flatten: false,
            ..Rewrites::ALL
        };
        assert_eq!(
            (0, 0),
            flattened(&adhoc_plan(&f, text, &[], off)),
            "`{text}`"
        );
    }
}

#[test]
fn a_nested_result_over_one_row_is_not_flattened() {
    let f = running();
    let text = "{ n: count(o for o in orders where o.user == me) }";
    let p = adhoc_plan(&f, text, &[("me", "Key users")], Rewrites::ALL);
    let Eval::Let(_, count, _) = &p.root else {
        panic!("the count is bound before the record: {:?}", p.root);
    };
    assert!(matches!(query_of(count).spine, Spine::Pipeline(_)));
}

fn memberships() -> Fixture {
    let mut f = Fixture::new(
        "type Membership = { id: Key memberships, user: Text }
relation memberships : Membership key id generate sequence unique (user)
type Assignment = { id: Key assignments, mine: Bool, user: Text, other: Text }
relation assignments : Assignment key id generate sequence
fn same (a: Text, b: Text) -> Bool = a == b
fn same2 (a: Text, b: Text) -> Bool = a == b and b == a
",
    );
    f.rows(
        "memberships",
        r#"[Membership { id: memberships.key(1), user: "ann" },
            Membership { id: memberships.key(2), user: "bo" }]"#,
        &[],
    );
    f.rows(
        "assignments",
        r#"[Assignment { id: assignments.key(1), mine: false, user: "ann", other: "bo" },
            Assignment { id: assignments.key(2), mine: true, user: "bo", other: "cy" }]"#,
        &[],
    );
    f
}

#[test]
fn a_condition_on_a_branching_argument_probes_the_index() {
    let f = memberships();
    for same in ["same", "same2"] {
        let text = format!(
            "from a in assignments where key a == assignments.key(2) select \
             (from m in memberships where {same}(m.user, if a.mine then a.user else a.other) select key m)"
        );
        let p = adhoc_plan(&f, &text, &[], Rewrites::ALL);
        let shown = explain::Describe::new(&f.schema, &f.catalog)
            .plan(&p)
            .text();
        assert!(
            shown.contains("Probe memberships (user)") && !shown.contains("Scan memberships"),
            "`{same}` probes memberships:\n{shown}"
        );
    }
}

#[test]
fn a_range_condition_on_a_branching_argument_probes_the_range() {
    let mut f = Fixture::new(
        "type M = { id: Key ms, n: I64 }
relation ms : M key id generate sequence index (n)
type A = { id: Key as_, mine: Bool, x: I64, y: I64 }
relation as_ : A key id generate sequence
fn near (a: I64, b: I64) -> Bool = a >= b and a < b + 10
",
    );
    f.rows(
        "ms",
        "[M { id: ms.key(1), n: 3 }, M { id: ms.key(2), n: 12 }, M { id: ms.key(3), n: 25 }]",
        &[],
    );
    f.rows(
        "as_",
        "[A { id: as_.key(1), mine: true, x: 10, y: 0 }, A { id: as_.key(2), mine: false, x: 0, y: 1 }]",
        &[],
    );
    for k in [1, 2] {
        let text = format!(
            "from a in as_ where key a == as_.key({k}) select \
             (from m in ms where near(m.n, if a.mine then a.x else a.y) select key m)"
        );
        let p = adhoc_plan(&f, &text, &[], Rewrites::ALL);
        let shown = explain::Describe::new(&f.schema, &f.catalog)
            .plan(&p)
            .text();
        assert!(
            shown.contains("Range probe ms (n)") && !shown.contains("Scan ms"),
            "`as_.key({k})` probes the range of ms:\n{shown}"
        );
    }
}
