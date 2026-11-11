use std::collections::BTreeSet;

use adtb_lang::kernel::{Kind, Var};
use adtb_lang::types::{Base, Ty};

use super::super::validate::validate;
use super::*;

/// An arm of a `Spine::Case`.
type SpineArm = (u32, Var, Spine);

fn check(f: &Fixture, p: &Plan) -> Result<(), Broken> {
    validate(&f.schema, &f.catalog, &mut Facts::default(), p)
}

fn spine_mut(p: &mut Plan) -> &mut Spine {
    match &mut p.root {
        Eval::Query(q) => &mut q.spine,
        other => panic!("not a query: {other:?}"),
    }
}

fn first_pipeline(s: &mut Spine) -> &mut Pipeline {
    match s {
        Spine::Pipeline(p) => p,
        Spine::Union(ss) => first_pipeline(&mut ss[0]),
        Spine::If(_, a, _) => first_pipeline(a),
        Spine::Case(_, arms) => first_pipeline(&mut arms[0].2),
        Spine::Let(_, _, b) => first_pipeline(b),
    }
}

/// Returns the plan of ad hoc `text`, every rewrite applied, and the variable of each of its
/// parameters, after checking that the plan passes.
fn valid(f: &Fixture, text: &str, params: &[(&str, &str)]) -> (Plan, Vec<Var>) {
    let (n, t) = f.normal(text, params);
    let vars: Vec<Var> = params.iter().map(|(name, _)| n.params[*name]).collect();
    let p = plan_with(f, &n.terms, t, Rewrites::ALL);
    assert_eq!(Ok(()), check(f, &p), "`{text}`");
    (p, vars)
}

fn index_end(p: &mut Plan) -> &mut End {
    let end = &mut first_pipeline(spine_mut(p)).chain.end;
    assert!(
        matches!(end, End::Index { .. } | End::Range { .. }),
        "{end:?}"
    );
    end
}

fn routes(end: &mut End) -> &mut Vec<Route> {
    match end {
        End::Index { routes, .. } | End::Range { routes, .. } => routes,
        other => panic!("not a probe: {other:?}"),
    }
}

const KEY_PROBE: &str = "from i in identities where i.id == k select key i";

const GID_PROBE: &str = r#"from i in identities.Google where i.gid == "root" select key i"#;

const RANGE: &str = "from o in orders where o.id > orders.key(2) select o.total";

fn at(p: &mut Plan) -> &mut AtRead {
    match &mut p.root {
        Eval::At(a) => a,
        other => panic!("a read of the past: {other:?}"),
    }
}

const PAST: &str = "at commit p { from i in identities select key i }";

const PAST_LOOKUP: &str = "at commit p { from n in [users.key(1)] select n.name }";

fn gid_index(f: &Fixture) -> IndexId {
    let ix = &f.catalog.relation(f.rel("identities")).indexes;
    ix.iter()
        .find(|i| i.fields() == Some(vec!["gid"]))
        .expect("the gid index")
        .id
}

#[test]
fn a_variable_read_outside_its_binder_is_refused() {
    let f = running();
    let (mut p, _) = valid(&f, "from p in posts select p.author", &[]);
    let pipeline = first_pipeline(spine_mut(&mut p));
    let Source::Scan { var, .. } = pipeline.source else {
        panic!("a scan: {:?}", pipeline.source)
    };
    let End::Emit(t) = pipeline.chain.end else {
        panic!("an emit: {:?}", pipeline.chain.end)
    };
    p.root = Eval::Expr(t);
    assert_eq!(Err(Broken::Unbound(var)), check(&f, &p));
}

#[test]
fn a_parameter_is_bound_only_when_the_plan_takes_it() {
    let f = running();
    let (mut p, vars) = valid(&f, KEY_PROBE, &[("k", "Key identities")]);
    p.params.clear();
    assert_eq!(Err(Broken::Unbound(vars[0])), check(&f, &p));
}

#[test]
fn a_case_missing_repeating_or_inventing_an_arm_is_refused() {
    let f = running();
    let q = f.query(SEARCH);
    let p = plan_with(&f, &q.terms, q.body, Rewrites::ALL);
    assert_eq!(Ok(()), check(&f, &p));
    let broken = |edit: &dyn Fn(&mut Vec<SpineArm>)| {
        let mut p = p.clone();
        let Spine::Case(_, arms) = spine_mut(&mut p) else {
            panic!("a case at the spine's root")
        };
        assert_eq!(vec![0, 1], arms.iter().map(|a| a.0).collect::<Vec<_>>());
        edit(arms);
        check(&f, &p)
    };
    assert_eq!(
        Err(Broken::NoArm(0)),
        broken(&|arms| {
            arms.remove(0);
        })
    );
    assert_eq!(
        Err(Broken::ArmTwice(1)),
        broken(&|arms| arms.push(arms[1].clone()))
    );
    assert_eq!(
        Err(Broken::ArmNotCtor(2)),
        broken(&|arms| {
            let mut extra = arms[1].clone();
            extra.0 = 2;
            arms.push(extra);
        })
    );
    let mut p = p.clone();
    let not_sum = Ty::Set(Box::new(Ty::Base(Base::I64)));
    let empty = p.terms.add(Kind::EmptySet, not_sum.clone());
    let Spine::Case(s, _) = spine_mut(&mut p) else {
        panic!("a case at the spine's root")
    };
    *s = empty;
    assert_eq!(Err(Broken::CaseNotSum(not_sum)), check(&f, &p));
}

#[test]
fn a_scan_of_another_constructors_heap_is_refused() {
    let f = running();
    let (mut p, _) = valid(&f, MATCH, &[]);
    let other = heap(&f, "identities", 1);
    let Source::Scan { heap, ctor, .. } = &mut first_pipeline(spine_mut(&mut p)).source else {
        panic!("a scan")
    };
    assert_eq!(0, *ctor);
    *heap = other;
    assert_eq!(Err(Broken::ScanHeap(other)), check(&f, &p));
}

#[test]
fn a_route_outside_its_index_repeated_or_to_another_heap_is_refused() {
    let f = running();
    let params = [("k", "Key identities")];
    let (p, _) = valid(&f, KEY_PROBE, &params);
    let key_index = f.catalog.relation(f.rel("identities")).key_index;
    let broken = |p: &Plan, edit: &dyn Fn(&mut End)| {
        let mut p = p.clone();
        edit(index_end(&mut p));
        check(&f, &p)
    };
    assert_eq!(
        Err(Broken::RouteTwice {
            index: key_index,
            ctor: 0
        }),
        broken(&p, &|e| {
            let r = routes(e);
            r.push(r[0].clone());
        })
    );
    let h1 = heap(&f, "identities", 1);
    assert_eq!(
        Err(Broken::RouteHeap { ctor: 0, heap: h1 }),
        broken(&p, &|e| routes(e)[0].heap = h1)
    );
    let unknown = IndexId(u64::MAX);
    assert_eq!(
        Err(Broken::UnknownIndex(unknown)),
        broken(&p, &|e| {
            let End::Index { index, .. } = e else {
                panic!("an index probe")
            };
            *index = unknown;
        })
    );
    let (gid, _) = valid(&f, GID_PROBE, &[]);
    let h0 = heap(&f, "identities", 0);
    assert_eq!(
        Err(Broken::RouteNotOfIndex {
            index: gid_index(&f),
            ctor: 0
        }),
        broken(&gid, &|e| {
            let r = &mut routes(e)[0];
            assert_eq!(1, r.ctor);
            r.ctor = 0;
            r.heap = h0;
        })
    );
}

#[test]
fn a_lookup_of_an_index_other_than_the_key_index_is_refused() {
    let f = running();
    let (mut p, _) = valid(&f, GID_PROBE, &[]);
    let gid = gid_index(&f);
    p.root = Eval::Lookup(Lookup {
        index: gid,
        key: Vec::new(),
        routes: Vec::new(),
    });
    assert_eq!(Err(Broken::LookupNotKey(gid)), check(&f, &p));
}

#[test]
fn a_range_of_an_unordered_index_or_past_its_fields_is_refused() {
    let f = running();
    let (mut p, _) = valid(&f, RANGE, &[]);
    let index = f.catalog.relation(f.rel("orders")).key_index;
    let mut relations = f.catalog.relations().clone();
    let rel = relations.get_mut(&f.rel("orders")).expect("orders");
    rel.indexes
        .iter_mut()
        .filter(|i| i.id == index)
        .for_each(|i| i.ordered = false);
    let unordered = Catalog::new(relations);
    assert_eq!(
        Err(Broken::RangeNotOrdered(index)),
        validate(&f.schema, &unordered, &mut Facts::default(), &p)
    );
    let End::Range {
        index: i, key, lo, ..
    } = index_end(&mut p)
    else {
        panic!("a range")
    };
    assert_eq!(index, *i);
    key.push(lo.expect("a lower bound").value);
    assert_eq!(Err(Broken::RangeNotField(index)), check(&f, &p));
}

#[test]
fn a_read_its_plan_does_not_list_is_refused() {
    let f = running();
    let (mut p, _) = valid(&f, GID_PROBE, &[]);
    let gid = gid_index(&f);
    let identities: Vec<HeapId> = (0..3).map(|c| heap(&f, "identities", c)).collect();
    assert_eq!(
        identities.iter().copied().collect::<BTreeSet<_>>(),
        p.reads.heaps
    );
    for h in identities {
        let mut q = p.clone();
        q.reads.heaps.remove(&h);
        assert_eq!(Err(Broken::UnlistedHeap(h)), check(&f, &q));
    }
    p.reads.indexes.remove(&gid);
    assert_eq!(Err(Broken::UnlistedIndex(gid)), check(&f, &p));
}

#[test]
fn a_read_of_the_past_breaking_its_shape_is_refused() {
    let f = running();
    let params = [("p", "Position")];
    let (p, _) = valid(&f, PAST, &params);
    let mut nested = p.clone();
    let a = at(&mut nested);
    a.body = Eval::At(Box::new(a.clone()));
    assert_eq!(Err(Broken::NestedPast), check(&f, &nested));
    let h0 = heap(&f, "identities", 0);
    let mut unlisted = p;
    assert!(at(&mut unlisted).reads.heaps.remove(&h0));
    assert_eq!(Err(Broken::PastUnlistedHeap(h0)), check(&f, &unlisted));
    let (mut lookup, _) = valid(&f, PAST_LOOKUP, &params);
    let users = f.catalog.relation(f.rel("users")).key_index;
    assert!(at(&mut lookup).reads.indexes.remove(&users));
    assert_eq!(Err(Broken::PastUnlistedIndex(users)), check(&f, &lookup));
    let (mut probe, vars) = valid(&f, KEY_PROBE, &[("k", "Key identities")]);
    let point = probe.terms.add(Kind::Var(vars[0]), f.ty("Key identities"));
    probe.root = Eval::At(Box::new(AtRead {
        instant: false,
        point,
        pins: None,
        body: probe.root.clone(),
        reads: probe.reads.clone(),
    }));
    let key_index = f.catalog.relation(f.rel("identities")).key_index;
    assert_eq!(Err(Broken::PastIndex(key_index)), check(&f, &probe));
}

/// Ad hoc texts of the planner's tests over the running schema, each with its parameters.
const CORPUS: &[(&str, &[(&str, &str)])] = &[
    (MATCH, &[]),
    (JOIN, &[]),
    (REACH, &[("me", "Key users")]),
    (GROUP, &[]),
    (KEY_PROBE, &[("k", "Key identities")]),
    (GID_PROBE, &[]),
    (RANGE, &[]),
    (PAST, &[("p", "Position")]),
    (PAST_LOOKUP, &[("p", "Position")]),
    ("from i in identities select i.user.name", &[]),
    ("from u in users select u order by u.name desc", &[]),
    ("top 2 by (u.name, key u) (from u in users select u)", &[]),
    (
        "from o in orders select { id: o.id, r: try (o.total / o.qty) }",
        &[],
    ),
    (
        "sum(o.total for o in orders where o.user == u)",
        &[("u", "Key users")],
    ),
    (
        "from s in sessions select match s.owner | None -> 0 | Some { value: o } -> count(p for p in posts where p.author == o)",
        &[],
    ),
    (
        "from u in users where not exists p in posts where p.author == key u select u.name",
        &[],
    ),
    (
        "fix a = (from u in users select key u) union (from x in a where { k: x } != { k: me } select x) in a",
        &[("me", "Key users")],
    ),
];

#[test]
fn every_plan_of_the_planner_corpus_keeps_the_plan_invariants() {
    let f = running();
    let before = checked();
    let mut made: u64 = 0;
    for (text, params) in CORPUS {
        let (n, t) = f.normal(text, params);
        for (name, rw) in variants() {
            if let Err(e) = plan(
                &f.schema,
                &f.normal,
                &f.catalog,
                &n.terms,
                t,
                Options {
                    rewrites: rw,
                    hiding: false,
                },
            ) {
                panic!("`{text}` with {name}: {e}");
            }
            made += 1;
        }
    }
    let q = f.query(SEARCH);
    for (name, rw) in variants() {
        plan(
            &f.schema,
            &f.normal,
            &f.catalog,
            &q.terms,
            q.body,
            Options {
                rewrites: rw,
                hiding: false,
            },
        )
        .unwrap_or_else(|e| panic!("the search with {name}: {e}"));
        made += 1;
    }
    let after = checked();
    let ran = |c: Check| after.get(c) - before.get(c);
    assert_eq!(made, ran(Check::Plans));
    assert_eq!(0, ran(Check::Refused));
    for c in Check::CHECKS {
        assert!(ran(c) > 0, "no plan of the corpus ran the {c:?} check");
    }
}
