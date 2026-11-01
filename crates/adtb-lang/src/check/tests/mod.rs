use super::*;
use crate::ast::{BundleItem, ExampleItem};
use crate::kernel::{Const, Prim};
use crate::parse::{Options, parse_adhoc, parse_example, parse_schema, parse_type};
use crate::schema::build;

mod adhoc;
mod binding_sites;
mod or_patterns;
mod principal;
mod row_provenance;
mod runtime;
mod writes;

fn source(extra: &str) -> String {
    let running = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/checker/running.schema"
    ))
    .expect("the checker running schema is readable");
    format!("{running}\n{extra}")
}

fn schema(extra: &str) -> Result<Schema, Vec<String>> {
    let src = source(extra);
    let parsed = parse_schema(&src, Options::default());
    if !parsed.diags.is_empty() {
        panic!(
            "{extra}\ndoes not parse: {:?}",
            parsed
                .diags
                .iter()
                .map(|d| d.render(&src))
                .collect::<Vec<_>>()
        );
    }
    let file = parsed.ok().expect("parsed");
    build(&[(FileId(0), file)]).map_err(|ds| ds.into_iter().map(|d| d.diag.message).collect())
}

fn program(extra: &str) -> Result<(Schema, Program), Vec<String>> {
    check_program(schema(extra)?)
}

fn check_program(s: Schema) -> Result<(Schema, Program), Vec<String>> {
    match check_schema(&s) {
        Ok(p) => {
            assert_one_binding_site("checked", &p.terms, &program_roots(&p));
            one_binding_site(&s, &p);
            Ok((s, p))
        }
        Err(ds) => Err(ds.into_iter().map(|d| d.diag.message).collect()),
    }
}

/// Assert that no variable of `terms` has two binding sites among the terms under `roots`.
pub(crate) fn assert_one_binding_site(what: &str, terms: &Terms, roots: &[Tm]) {
    if let Some((v, a, b)) = terms.rebound(roots.iter().copied()) {
        panic!(
            "{what}: `{}` is bound at two binding sites, {:?} and {:?}",
            terms.var_info(v).name,
            terms.kind(a),
            terms.kind(b)
        );
    }
}

/// Returns the checked bodies of `p`.
pub(crate) fn program_roots(p: &Program) -> Vec<Tm> {
    p.functions
        .iter()
        .flatten()
        .chain(&p.constraints)
        .chain(&p.checks)
        .chain(p.domains.values().map(|(_, t)| t))
        .chain(p.retags.values())
        .chain(p.defaults.values())
        .chain(p.computed.values().map(|(_, t)| t))
        .chain(p.index_fields.values().map(|f| &f.value))
        .copied()
        .chain(
            p.aggregates
                .iter()
                .flatten()
                .flat_map(|a| [a.init, a.step, a.merge, a.finish]),
        )
        .collect()
}

/// Assert that every variable of the normal forms of the bodies of `p` has one binding site.
fn one_binding_site(s: &Schema, p: &Program) {
    let n = crate::norm::program(s, p).unwrap_or_else(|e| panic!("normalises: {e:?}"));
    let roots: Vec<Tm> = n
        .functions
        .iter()
        .flatten()
        .map(|(_, t)| t)
        .chain(&n.constraints)
        .chain(&n.checks)
        .chain(n.domains.values().map(|(_, t)| t))
        .chain(n.retags.values().map(|(_, t)| t))
        .chain(n.defaults.values())
        .chain(n.computed.values().map(|(_, t)| t))
        .chain(n.index_fields.values().map(|(_, f)| &f.value))
        .copied()
        .chain(
            n.aggregates
                .iter()
                .flatten()
                .flat_map(|a| [a.init, a.step.1, a.merge.1, a.finish.1]),
        )
        .collect();
    assert_one_binding_site("normalised", &n.terms, &roots);
}

/// Returns the items and command terms of ad hoc text.
pub(crate) fn adhoc_roots(items: &[Tm], commands: &[AdhocCommand]) -> Vec<Tm> {
    items
        .iter()
        .copied()
        .chain(commands.iter().flat_map(|c| c.command.terms()))
        .collect()
}

/// Assert that every variable of the normal forms of ad hoc `a` has one binding site.
fn adhoc_one_binding_site(s: &Schema, p: &Program, a: &Adhoc) {
    assert_one_binding_site("checked", &a.terms, &adhoc_roots(&a.items, &a.commands));
    let n = crate::norm::adhoc(a.schema(s), p, a).unwrap_or_else(|e| panic!("normalises: {e:?}"));
    assert_one_binding_site("normalised", &n.terms, &adhoc_roots(&n.items, &n.commands));
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

/// Runs `f` on a thread with the 2 MiB stack a spawned thread gets by default.
fn on_small_stack<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|s| {
        std::thread::Builder::new()
            .stack_size(2 << 20)
            .spawn_scoped(s, f)
            .expect("a test thread starts")
            .join()
            .unwrap_or_else(|p| std::panic::resume_unwind(p))
    })
}

fn rel_id(s: &Schema, name: &str) -> RelId {
    RelId(
        s.relations
            .iter()
            .position(|r| r.name == name)
            .expect("a relation of that name") as u32,
    )
}

/// Checks ad hoc `text` against the running schema, its parameters given as `name: Type`.
fn adhoc(params: &[(&str, &str)], text: &str) -> Result<(Schema, Adhoc), Vec<String>> {
    adhoc_in(&[], params, text)
}

/// A registered query of a live bundle: family, version, query name and parameter types.
type RegisteredQuery<'a> = (&'a str, &'a str, &'a str, &'a [&'a str]);

/// Checks ad hoc `text` as [`adhoc`] does, knowing the registered queries `bundles`.
fn adhoc_in(
    bundles: &[RegisteredQuery],
    params: &[(&str, &str)],
    text: &str,
) -> Result<(Schema, Adhoc), Vec<String>> {
    let s = schema("").expect("the running schema builds");
    let mut tys = BTreeMap::new();
    let mut known = Bundles::default();
    {
        let c = Checker::new(&s, Visible::all(&s), Context::Adhoc);
        let resolve = |t: &str| {
            let parsed = parse_type(t, Options::default()).ok().expect("a type");
            c.resolve_type(&parsed).expect("a known type")
        };
        for (name, t) in params {
            tys.insert(name.to_string(), resolve(t));
        }
        for (family, version, query, ps) in bundles {
            known.insert(
                family,
                version,
                query,
                ps.iter().map(|t| resolve(t)).collect(),
            );
        }
    }
    let parsed = parse_adhoc(text, Options::default());
    if !parsed.diags.is_empty() {
        panic!(
            "{text}\ndoes not parse: {:?}",
            parsed
                .diags
                .iter()
                .map(|d| d.render(text))
                .collect::<Vec<_>>()
        );
    }
    let items = parsed.ok().expect("parsed");
    let program = check_schema(&s).expect("the schema checks");
    match check_adhoc_recording(&s, &program, &items, &tys, &known, None) {
        Ok(a) => {
            adhoc_one_binding_site(&s, &program, &a);
            Ok((s, a))
        }
        Err(ds) => Err(ds.into_iter().map(|d| d.message).collect()),
    }
}

fn adhoc_accepts(params: &[(&str, &str)], text: &str) {
    if let Err(errs) = adhoc(params, text) {
        panic!("{text}\nrefused: {errs:?}");
    }
}

fn adhoc_refuses(params: &[(&str, &str)], text: &str, needle: &str) {
    match adhoc(params, text) {
        Ok(_) => panic!("{text}\naccepted; expected an error containing {needle:?}"),
        Err(errs) => assert!(
            errs.iter().any(|e| e.contains(needle)),
            "{text}\nno error containing {needle:?} in {errs:?}"
        ),
    }
}

/// Checks ad hoc `text` against the running schema extended by `extra`.
fn adhoc_over(extra: &str, text: &str) -> Result<Adhoc, Vec<String>> {
    let s = schema(extra).expect("the schema builds");
    let program = check_schema(&s).expect("the schema checks");
    let items = parse_adhoc(text, Options::default()).ok().expect("parsed");
    let a = check_adhoc(&s, &program, &items, &BTreeMap::new())
        .map_err(|ds| ds.into_iter().map(|d| d.message).collect::<Vec<_>>())?;
    adhoc_one_binding_site(&s, &program, &a);
    Ok(a)
}

/// Checks a registered query `text` against the running schema extended by `extra`.
fn query(extra: &str, text: &str) -> Result<(), String> {
    let s = schema(extra).expect("the schema builds");
    let items = parse_example(text, Options::default())
        .ok()
        .expect("parses");
    let Some(ExampleItem::BundleItem(BundleItem::Query(q))) = items.into_iter().next() else {
        panic!("{text}\nis not a query");
    };
    let module = s.module_named("auth").expect("module auth");
    let program = check_schema(&s).expect("the schema checks");
    let checked = check_query(&s, &program, module, &q).map_err(|d| d.message)?;
    assert_one_binding_site("checked", &checked.terms, &[checked.term]);
    let n =
        crate::norm::query(&s, &program, &checked).unwrap_or_else(|e| panic!("normalises: {e:?}"));
    assert_one_binding_site("normalised", &n.terms, &[n.body]);
    Ok(())
}

fn query_accepts(text: &str) {
    if let Err(e) = query("", text) {
        panic!("{text}\nrefused: {e}");
    }
}

/// Returns `t` printed in full, each variable named by its name and the order of its first
/// occurrence.
fn sexp(terms: &Terms, t: Tm) -> String {
    let mut p = Printer {
        terms,
        names: BTreeMap::new(),
        out: String::new(),
    };
    p.tm(t);
    p.out
}

struct Printer<'t> {
    terms: &'t Terms,
    names: BTreeMap<Var, String>,
    out: String,
}

impl Printer<'_> {
    fn var(&mut self, v: Var) {
        let n = self.names.len();
        let name = self
            .names
            .entry(v)
            .or_insert_with(|| format!("{}{n}", self.terms.var_info(v).name));
        self.out.push_str(name);
    }

    fn list(&mut self, xs: &[Tm]) {
        for x in xs {
            self.out.push(' ');
            self.tm(*x);
        }
    }

    fn tm(&mut self, t: Tm) {
        match self.terms.kind(t) {
            Kind::Var(v) => self.var(*v),
            Kind::Const(Const::Int(i)) => self.out.push_str(&i.to_string()),
            Kind::Const(Const::Text(x)) => self.out.push_str(&format!("{x:?}")),
            Kind::Ctor { ctor, fields } if fields.is_empty() => {
                self.out.push_str(&format!("#{ctor}"));
            }
            Kind::Field(e, l) => {
                self.tm(*e);
                self.out.push_str(&format!(".{l}"));
            }
            Kind::Let(x, e, b) => {
                self.out.push_str("(let ");
                self.var(*x);
                self.list(&[*e, *b]);
                self.out.push(')');
            }
            Kind::Case { scrutinee, arms } => {
                self.out.push_str("(case");
                self.list(&[*scrutinee]);
                for (c, x, e) in arms {
                    self.out.push_str(&format!(" ({c} "));
                    self.var(*x);
                    self.list(&[*e]);
                    self.out.push(')');
                }
                self.out.push(')');
            }
            Kind::Prim(p, args) => {
                self.out.push_str(&format!("({p:?}"));
                self.list(args);
                self.out.push(')');
            }
            Kind::Unreachable => self.out.push_str("unreachable"),
            k => {
                let shown = format!("{k:?}");
                let head = shown.split(['(', ' ', '{']).next().unwrap_or_default();
                self.out.push_str(&format!("({}", head.to_lowercase()));
                self.list(&k.children());
                self.out.push(')');
            }
        }
    }
}

fn anon_ctors(terms: &Terms, t: Tm) -> Vec<(Vec<String>, String, Vec<String>)> {
    runtime::constructors(terms, t)
        .into_iter()
        .map(|(names, _, name, fields)| (names, name, fields))
        .collect()
}

/// The body of function `name`, inside its `Lam`.
fn body_of(s: &Schema, p: &Program, name: &str) -> Tm {
    let i = s
        .functions
        .iter()
        .position(|f| f.name == name)
        .expect("a function of that name");
    let mut t = p.functions[i].expect("checked");
    for _ in 0..s.functions[i].params.len().max(1) {
        t = match p.terms.kind(t) {
            Kind::Lam(_, body) => *body,
            k => panic!("a function is one Lam per parameter, found {k:?}"),
        };
    }
    t
}

// Signatures and inference

// Subtyping

// Joins

// Records, constructors and fields

#[test]
fn navigation_desugars_to_deref() {
    runtime::probes("navigation_desugars_to_deref", |fixture| {
        let (s, p) = program(fixture.text("text")).unwrap();
        let body = body_of(&s, &p, fixture.text("body"));
        let Kind::Field(inner, name) = p.terms.kind(body) else {
            panic!("a field, found {:?}", p.terms.kind(body));
        };
        assert_eq!(name, fixture.text("expected_field"));
        assert!(matches!(
            p.terms.kind(*inner),
            Kind::Deref { reference, .. } if *reference == fixture.number::<bool>("reference_deref")
        ));
        let (s, p) = program(fixture.text("text_2")).unwrap();
        let body = body_of(&s, &p, fixture.text("body"));
        let Kind::Field(inner, _) = p.terms.kind(body) else {
            panic!("a field");
        };
        assert!(matches!(
            p.terms.kind(*inner),
            Kind::Deref { reference, .. } if *reference == fixture.number::<bool>("key_deref")
        ));
    });
}

// key

// Navigation and backlinks

#[test]
fn a_backlink_desugars_to_a_filtered_comprehension() {
    runtime::probes(
        "a_backlink_desugars_to_a_filtered_comprehension",
        |fixture| {
            let (s, p) = program(fixture.text("text")).unwrap();
            let body = body_of(&s, &p, fixture.text("body"));
            let Kind::For(_, src, inner) = p.terms.kind(body) else {
                panic!("a for, found {:?}", p.terms.kind(body));
            };
            assert!(matches!(p.terms.kind(*src), Kind::Rel(..)));
            let Kind::If(test, _, _) = p.terms.kind(*inner) else {
                panic!("an if");
            };
            let Kind::Prim(Prim::Eq, args) = p.terms.kind(*test) else {
                panic!("an equality");
            };
            assert!(
                matches!(p.terms.kind(args[1]), Kind::Ctor { ctor, .. } if *ctor == fixture.number::<u32>("some_ctor"))
            );
        },
    );
}

// Comprehensions and nested results

#[test]
fn a_comprehension_is_nested_for_if_singleton() {
    runtime::probes("a_comprehension_is_nested_for_if_singleton", |fixture| {
        let (s, p) = program(fixture.text("text")).unwrap();
        let body = body_of(&s, &p, fixture.text("body"));
        let Kind::For(_, src, inner) = p.terms.kind(body) else {
            panic!("a for, found {:?}", p.terms.kind(body));
        };
        assert!(matches!(p.terms.kind(*src), Kind::Rel(..)));
        let Kind::For(_, _, inner) = p.terms.kind(*inner) else {
            panic!("a second for");
        };
        let Kind::If(_, yes, no) = p.terms.kind(*inner) else {
            panic!("an if");
        };
        assert!(matches!(p.terms.kind(*yes), Kind::Singleton(_)));
        assert!(matches!(p.terms.kind(*no), Kind::EmptySet));
    });
}

#[test]
fn exists_tests_emptiness() {
    runtime::probes("exists_tests_emptiness", |fixture| {
        let (s, p) = program(fixture.text("text_3")).unwrap();
        assert!(matches!(
            p.terms.kind(body_of(&s, &p, fixture.text("body"))),
            Kind::Exists(_)
        ));
    });
}

#[test]
fn and_or_not_are_ifs() {
    runtime::probes("and_or_not_are_ifs", |fixture| {
        let (s, p) = program(fixture.text("text")).unwrap();
        let Kind::If(_, rhs, f) = p.terms.kind(body_of(&s, &p, fixture.text("body"))) else {
            panic!("an if");
        };
        assert!(
            matches!(p.terms.kind(*f), Kind::Ctor { ctor, .. } if *ctor == fixture.number::<u32>("false_ctor"))
        );
        assert!(matches!(p.terms.kind(*rhs), Kind::If(..)));
    });
}

#[test]
fn constructor_relations_are_relations() {
    runtime::probes("constructor_relations_are_relations", |fixture| {
        let (s, p) = program(fixture.text("text")).unwrap();
        assert!(matches!(
            p.terms.kind(body_of(&s, &p, fixture.text("body"))),
            Kind::Rel(_, Some(_))
        ));
        let (s, p) = program(fixture.text("text_2")).unwrap();
        assert!(matches!(
            p.terms.kind(body_of(&s, &p, fixture.text("body"))),
            Kind::Restrict(..)
        ));
    });
}

// Result sums

#[test]
fn a_constructor_of_four_thousand_fields_is_checked_for_coverage() {
    let fields: Vec<String> = (0..4000).map(|i| format!("f{i}: I64")).collect();
    let ty = format!("type Wide = A {{ {} }} | B", fields.join(", "));
    on_small_stack(|| {
        accepts(&format!(
            "{ty}\nfn f (w: Wide) -> I64 = match w | A {{ f3999: 1 }} -> 1 | A _ -> 2 | B -> 3"
        ));
        refuses(
            &format!("{ty}\nfn f (w: Wide) -> I64 = match w | A {{ f3999: 1 }} -> 1 | B -> 3"),
            "match does not cover `A _`",
        );
    });
}

// Match

#[test]
fn a_match_on_a_relation_row_is_a_case_per_constructor() {
    runtime::probes(
        "a_match_on_a_relation_row_is_a_case_per_constructor",
        |fixture| {
            let (s, p) = program(fixture.text("text")).unwrap();
            let _ = body_of(&s, &p, fixture.text("body"));
            let i = s
                .functions
                .iter()
                .position(|f| f.name == fixture.text("function"))
                .unwrap();
            let Kind::Lam(_, body) = p.terms.kind(p.functions[i].unwrap()) else {
                panic!("a lam");
            };
            let Kind::Let(_, _, case) = p.terms.kind(*body) else {
                panic!("a let, found {:?}", p.terms.kind(*body));
            };
            let Kind::Case { arms, .. } = p.terms.kind(*case) else {
                panic!("a case, found {:?}", p.terms.kind(*case));
            };
            assert_eq!(
                arms.iter().map(|(c, _, _)| *c).collect::<Vec<_>>(),
                fixture.numbers::<u32>("constructors")
            );
            for (_, x, arm) in arms {
                let Kind::Let(z, y, _) = p.terms.kind(*arm) else {
                    panic!("an arm binds its binder, found {:?}", p.terms.kind(*arm));
                };
                assert_ne!(z, x);
                assert_eq!(p.terms.kind(*y), &Kind::Var(*x));
            }
        },
    );
}

#[test]
fn a_match_on_a_constructor_set_cases_over_the_set() {
    runtime::probes(
        "a_match_on_a_constructor_set_cases_over_the_set",
        |fixture| {
            let (s, p) = program(fixture.text("text")).unwrap();
            let Kind::Let(_, _, case) = p.terms.kind(body_of(&s, &p, fixture.text("body"))) else {
                panic!("a let");
            };
            let Kind::Case { arms, .. } = p.terms.kind(*case) else {
                panic!("a case");
            };
            assert_eq!(
                arms.iter().map(|(c, _, _)| *c).collect::<Vec<_>>(),
                fixture.numbers::<u32>("constructors")
            );
        },
    );
}

#[test]
fn an_or_pattern_shares_its_body() {
    runtime::probes("an_or_pattern_shares_its_body", |fixture| {
        let (s, p) = program(fixture.text("text")).unwrap();
        let Kind::Let(_, _, case) = p.terms.kind(body_of(&s, &p, fixture.text("body"))) else {
            panic!("a let");
        };
        let Kind::Case { arms, .. } = p.terms.kind(*case) else {
            panic!("a case");
        };
        let bodies: Vec<(Var, Tm)> = arms
            .iter()
            .filter(|(c, _, _)| *c != fixture.number::<u32>("excluded_ctor"))
            .map(|(_, _, t)| match p.terms.kind(*t) {
                Kind::Let(v, _, b) => (*v, *b),
                k => panic!("a let, found {k:?}"),
            })
            .collect();
        assert_eq!(bodies[0], bodies[1]);
    });
}

#[test]
fn a_scalar_match_is_a_chain_without_case() {
    runtime::probes("a_scalar_match_is_a_chain_without_case", |fixture| {
        let (s, p) = program(fixture.text("text")).unwrap();
        let Kind::Let(_, _, chain) = p.terms.kind(body_of(&s, &p, fixture.text("body"))) else {
            panic!("a let");
        };
        let Kind::If(test, _, rest) = p.terms.kind(*chain) else {
            panic!("an if, found {:?}", p.terms.kind(*chain));
        };
        assert!(matches!(p.terms.kind(*test), Kind::Prim(Prim::Eq, _)));
        assert!(matches!(p.terms.kind(*rest), Kind::If(..)));
    });
}

#[test]
fn a_match_on_a_key_derefs_it() {
    runtime::probes("a_match_on_a_key_derefs_it", |fixture| {
        let (s, p) = program(fixture.text("text")).unwrap();
        let Kind::Let(_, scrutinee, _) = p.terms.kind(body_of(&s, &p, fixture.text("body"))) else {
            panic!("a let");
        };
        assert!(matches!(
            p.terms.kind(*scrutinee),
            Kind::Deref { reference, .. } if *reference == fixture.number::<bool>("reference_deref")
        ));
        let (s, p) = program(fixture.text("text_2")).unwrap();
        let Kind::Let(_, scrutinee, _) = p.terms.kind(body_of(&s, &p, fixture.text("body"))) else {
            panic!("a let");
        };
        assert!(matches!(
            p.terms.kind(*scrutinee),
            Kind::Deref { reference, .. } if *reference == fixture.number::<bool>("key_deref")
        ));
    });
}

// Patterns and binder types

// Exhaustiveness

// Grouping and aggregates

#[test]
fn a_group_desugars_to_group_over_binder_records() {
    runtime::probes("a_group_desugars_to_group_over_binder_records", |fixture| {
        let (s, p) = program(fixture.text("text")).unwrap();
        let Kind::Let(_, value, body) = p.terms.kind(body_of(&s, &p, fixture.text("body"))) else {
            panic!("a let of the grouped value");
        };
        assert!(matches!(p.terms.kind(*value), Kind::Lam(..)));
        let Kind::For(_, src, _) = p.terms.kind(*body) else {
            panic!("a for");
        };
        let Kind::Group { source, .. } = p.terms.kind(*src) else {
            panic!("a group, found {:?}", p.terms.kind(*src));
        };
        assert!(matches!(p.terms.kind(*source), Kind::For(..)));
    });
}

/// The aggregates reachable from `t`, each with whether its source has the nonempty type.
fn agg_sources(terms: &Terms, t: Tm) -> Vec<(crate::kernel::Agg, bool)> {
    let mut out = Vec::new();
    let mut stack = vec![t];
    while let Some(t) = stack.pop() {
        if let Kind::Agg { op, source, .. } = terms.kind(t) {
            out.push((*op, matches!(terms.ty(*source), Ty::NonEmpty(_))));
        }
        stack.extend(terms.children(t));
    }
    out.sort_by_key(|(_, nonempty)| *nonempty);
    out
}

#[test]
fn an_aggregate_over_a_group_has_a_nonempty_source() {
    runtime::probes(
        "an_aggregate_over_a_group_has_a_nonempty_source",
        |fixture| {
            let (s, p) = program(fixture.text("text")).unwrap();
            let expected = fixture.aggregates("aggregates");
            assert_eq!(
                agg_sources(&p.terms, body_of(&s, &p, fixture.text("body"))),
                expected
            );
            if fixture.number::<bool>("normalize") {
                let n = crate::norm::program(&s, &p).expect("normalises");
                let (_, body) = n.functions[s
                    .functions
                    .iter()
                    .position(|f| f.name == fixture.text("body"))
                    .unwrap()]
                .clone()
                .expect("a normal form");
                assert_eq!(agg_sources(&n.terms, body), expected);
            }
        },
    );
}

#[test]
fn constructor_fields_pair_with_their_written_names() {
    runtime::probes(
        "constructor_fields_pair_with_their_written_names",
        |fixture| {
            let (_, a) =
                adhoc(&[], fixture.text("text")).unwrap_or_else(|e| panic!("refused: {e:?}"));
            let Kind::Ctor { fields, .. } = a.terms.kind(a.items[0]) else {
                panic!("a constructor, found {:?}", a.terms.kind(a.items[0]));
            };
            let int = |name: &str| {
                let (_, t) = fields.iter().find(|(f, _)| f == name).expect("the field");
                match a.terms.kind(*t) {
                    Kind::Const(Const::Int(n)) => *n,
                    k => panic!("an integer, found {k:?}"),
                }
            };
            assert_eq!(
                fixture.number::<i128>("total"),
                int(fixture.text("total_field"))
            );
            assert_eq!(
                fixture.number::<i128>("qty"),
                int(fixture.text("qty_field"))
            );
        },
    );
}

#[test]
fn an_aggregate_is_an_aggregate_node() {
    runtime::probes("an_aggregate_is_an_aggregate_node", |fixture| {
        let (s, p) = program(fixture.text("text")).unwrap();
        let Kind::Agg { source, .. } = p.terms.kind(body_of(&s, &p, fixture.text("body"))) else {
            panic!("an agg");
        };
        assert!(matches!(p.terms.kind(*source), Kind::For(..)));
    });
}

// Ordering and limits

#[test]
fn top_is_a_set_at_any_depth() {
    runtime::probes("top_is_a_set_at_any_depth", |fixture| {
        let parsed = parse_schema(&source(fixture.text("text_3")), Options::default());
        assert_eq!(
            parsed
                .diags
                .iter()
                .map(|d| d.message.as_str())
                .collect::<Vec<_>>(),
            fixture.text("parse_refusal").lines().collect::<Vec<_>>(),
            "exact parser diagnostic set"
        );
        let (s, p) = program(fixture.text("text_4")).unwrap();
        assert!(matches!(
            p.terms.kind(body_of(&s, &p, fixture.text("body"))),
            Kind::Top { .. }
        ));
    });
}

// Recursion

#[test]
fn fix_is_a_kernel_fix() {
    runtime::probes("fix_is_a_kernel_fix", |fixture| {
        let (s, p) = program(fixture.text("text")).unwrap();
        let Kind::Fix(binds, _) = p.terms.kind(body_of(&s, &p, fixture.text("body"))) else {
            panic!("a fix");
        };
        assert_eq!(binds.len(), fixture.number::<usize>("binders"));
    });
}

// Functions, generics and lambdas

// Literals and numeric types

// Prelude overloads

// Constraints and other bodies

// Least types of arms

// Compound cursors

// Redundant alternatives

// Unary functions in the kernel

#[test]
fn a_function_is_one_lam_per_parameter() {
    runtime::probes("a_function_is_one_lam_per_parameter", |fixture| {
        let (s, p) = program(fixture.text("text")).unwrap();
        let i = s
            .functions
            .iter()
            .position(|f| f.name == fixture.text("function"))
            .unwrap();
        let Kind::Lam(_, inner) = p.terms.kind(p.functions[i].unwrap()) else {
            panic!("a lam");
        };
        let Kind::Lam(_, body) = p.terms.kind(*inner) else {
            panic!("a second lam, found {:?}", p.terms.kind(*inner));
        };
        assert!(matches!(p.terms.kind(*body), Kind::Prim(Prim::Add, _)));
    });
}

#[test]
fn a_view_is_a_lam_over_unit() {
    runtime::probes("a_view_is_a_lam_over_unit", |fixture| {
        let (s, p) = program(fixture.text("text")).unwrap();
        let i = s
            .functions
            .iter()
            .position(|f| f.name == fixture.text("function"))
            .unwrap();
        let Kind::Lam(x, _) = p.terms.kind(p.functions[i].unwrap()) else {
            panic!("a lam");
        };
        assert_eq!(p.terms.var_info(*x).ty, Ty::Unit);
    });
}

#[test]
fn applying_a_function_value_nests_apps() {
    runtime::probes("applying_a_function_value_nests_apps", |fixture| {
        let (s, p) = program(fixture.text("text")).unwrap();
        let Kind::App(inner, two) = p.terms.kind(body_of(&s, &p, fixture.text("body"))) else {
            panic!("an app");
        };
        assert!(matches!(p.terms.kind(*two), Kind::Const(_)));
        let Kind::App(g, _) = p.terms.kind(*inner) else {
            panic!("a nested app, found {:?}", p.terms.kind(*inner));
        };
        assert!(matches!(p.terms.kind(*g), Kind::Var(_)));
        let (s, p) = program(fixture.text("text_2")).unwrap();
        let Kind::App(_, unit) = p.terms.kind(body_of(&s, &p, fixture.text("body"))) else {
            panic!("an app");
        };
        assert_eq!(p.terms.ty(*unit), &Ty::Unit);
    });
}

#[test]
fn a_declared_call_keeps_its_arguments() {
    runtime::probes("a_declared_call_keeps_its_arguments", |fixture| {
        let (s, p) = program(fixture.text("text")).unwrap();
        let Kind::Call { args, .. } = p.terms.kind(body_of(&s, &p, fixture.text("body"))) else {
            panic!("a call");
        };
        assert_eq!(args.len(), fixture.number::<usize>("arity"));
    });
}

#[test]
fn a_function_named_without_a_call_is_nested_lams_over_its_call() {
    runtime::probes(
        "a_function_named_without_a_call_is_nested_lams_over_its_call",
        |fixture| {
            let (s, p) = program(fixture.text("text")).unwrap();
            let Kind::Lam(_, inner) = p.terms.kind(body_of(&s, &p, fixture.text("body"))) else {
                panic!("a lam");
            };
            let Kind::Lam(_, call) = p.terms.kind(*inner) else {
                panic!("a second lam");
            };
            assert!(
                matches!(p.terms.kind(*call), Kind::Call { args, .. } if args.len() == fixture.number::<usize>("arity"))
            );
        },
    );
}

#[test]
fn a_lambda_of_several_parameters_is_nested_lams() {
    runtime::probes("a_lambda_of_several_parameters_is_nested_lams", |fixture| {
        let (s, p) = program(fixture.text("text")).unwrap();
        let Kind::Lam(_, inner) = p.terms.kind(body_of(&s, &p, fixture.text("body"))) else {
            panic!("a lam");
        };
        assert!(matches!(p.terms.kind(*inner), Kind::Lam(..)));
    });
}

// Group is never written

#[test]
fn text_literals_elaborate_to_nfc() {
    runtime::probes("text_literals_elaborate_to_nfc", |fixture| {
        let (s, p) = program(fixture.text("text")).expect("checks");
        assert_eq!(
            p.terms.kind(body_of(&s, &p, fixture.text("body"))),
            &Kind::Const(Const::Text(fixture.text("expected_text").to_string()))
        );
    });
}

fn index_path(s: &Schema, p: &Program, name: &str, i: usize) -> Vec<PathStep> {
    let rel = RelId(
        s.relations
            .iter()
            .position(|r| r.name == name)
            .expect("bills") as u32,
    );
    p.index_fields[&(rel, i, 0)].path.clone()
}

#[test]
fn an_index_field_through_a_value_level_constructor_names_its_path() {
    runtime::probes(
        "an_index_field_through_a_value_level_constructor_names_its_path",
        |fixture| {
            let (s, p) = program(fixture.text("text")).unwrap_or_else(|e| panic!("refused: {e:?}"));
            for (i, expected) in fixture.paths("paths").into_iter().enumerate() {
                assert_eq!(expected, index_path(&s, &p, fixture.text("relation"), i));
            }
        },
    );
}

#[test]
fn a_row_body_under_shared_branches_is_walked_once() {
    let (_, p) = program("").expect("the running schema checks");
    let mut terms = Terms::default();
    let c = terms.var("c", Ty::Unit);
    let test = terms.add(Kind::Var(c), Ty::Unit);
    let row = terms.var("row", Ty::Unit);
    let leaf = terms.add(Kind::Var(row), Ty::Unit);
    let body = terms.doubled(test, leaf, 64);
    assert!(reads_only_row(&terms, body, &p, &mut BTreeSet::new()));
}
