mod valid;

use adtb_lang::kernel::{Kind, Terms, Tm};

use super::fixture::{Fixture, running, variants};
use super::*;

fn plan_with(f: &Fixture, terms: adtb_lang::kernel::Valid<'_>, t: Tm, rewrites: Rewrites) -> Plan {
    let options = Options {
        rewrites,
        hiding: false,
    };
    plan(&f.schema, &f.normal, &f.catalog, terms, t, options)
        .unwrap_or_else(|e| panic!("no plan: {e}"))
}

fn adhoc_plan(f: &Fixture, text: &str, params: &[(&str, &str)], rw: Rewrites) -> Plan {
    let (n, t) = f.normal(text, params);
    plan_with(f, n.valid(), t, rw)
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
fn a_pipeline_per_arm_emits_without_routing_by_tag() {
    let f = running();
    let p = adhoc_plan(&f, MATCH, &[], Rewrites::ALL);
    let ps = pipelines(&query_of(&p.root).spine);
    assert_eq!(3, ps.len());
    for x in ps {
        assert!(
            emits(&x.chain)
                .iter()
                .all(|t| !routes_by_tag(&f, &p.terms, *t))
        );
    }
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
fn an_option_parameter_chooses_the_pipelines_at_start() {
    let f = running();
    let q = f.query(SEARCH);
    let p = plan_with(&f, q.valid(), q.body(), Rewrites::ALL);
    let spine = &query_of(&p.root).spine;
    assert!(matches!(spine, Spine::Case(..)), "{spine:?}");
    assert_eq!(4, pipelines(spine).len());
    let p = plan_with(
        &f,
        q.valid(),
        q.body(),
        Rewrites {
            unswitch: false,
            ..Rewrites::ALL
        },
    );
    assert!(matches!(query_of(&p.root).spine, Spine::Pipeline(_)));
}

#[test]
fn a_write_is_refused() {
    let f = running();
    let (n, t) = f.normal("delete u in users where u.name == \"cy\"", &[]);
    let got = plan(
        &f.schema,
        &f.normal,
        &f.catalog,
        n.valid(),
        t,
        Options::default(),
    )
    .map(|_| ());
    assert_eq!(Err(Error::NotAQuery("a write")), got);
}

/// Returns the bounded plan of `text` over the set parameter `xs` of type `ty`.
fn bounded(f: &Fixture, text: &str, ty: &str) -> Result<Plan, Error> {
    let (normal, t) = f.normal(text, &[("xs", ty)]);
    plan_bounded(&f.schema, &f.normal, &f.catalog, normal.valid(), t)
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
