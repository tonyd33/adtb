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

/// The type of the single item of ad hoc text.
fn adhoc_ty(params: &[(&str, &str)], text: &str) -> String {
    match adhoc(params, text) {
        Ok((s, a)) => s.show_ty(a.terms.ty(a.items[0])),
        Err(errs) => panic!("{text}\nrefused: {errs:?}"),
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

fn query_refuses(text: &str, needle: &str) {
    match query("", text) {
        Ok(()) => panic!("{text}\naccepted; expected an error containing {needle:?}"),
        Err(e) => assert!(e.contains(needle), "{text}\nerror {e:?} lacks {needle:?}"),
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

#[test]
fn adhoc_free_names_are_typed_by_their_descriptors() {
    adhoc_accepts(
        &[("u", "Key users")],
        "from i in identities where i.user == u select i",
    );
    adhoc_refuses(
        &[],
        "from i in identities where i.user == u select i",
        "the call supplies no value for it",
    );
}

// Subtyping

#[test]
fn anonymous_sum_constructors_are_numbered_in_name_order() {
    let (s, p) = program(
        "fn ba () -> B { b: Text } | A { a: I64 } = B { b: \"x\" }
         fn ab () -> A { a: I64 } | B { b: Text } = B { b: \"x\" }",
    )
    .unwrap();
    for name in ["ba", "ab"] {
        let body = body_of(&s, &p, name);
        assert!(
            matches!(p.terms.kind(body), Kind::Ctor { ctor: 1, .. }),
            "{name}: {:?}",
            p.terms.kind(body)
        );
        let Ty::Sum { ctors, .. } = p.terms.ty(body) else {
            panic!("{name}: an anonymous sum");
        };
        let names: Vec<&str> = ctors.iter().map(|(c, _)| c.as_str()).collect();
        assert_eq!(names, ["A", "B"], "{name}");
    }
    let (s, a) = adhoc(
        &[("i", "Identity")],
        "match i | Google g -> Zed { n: 1 } | Apple a -> Alpha | EmailPass e -> Zed { n: 2 }",
    )
    .unwrap();
    assert_eq!(s.show_ty(a.terms.ty(a.items[0])), "Alpha | Zed { n: I64 }");
}

/// Each anonymous constructor reachable from `t`: the constructors of its node's type, its name there, and the field names it is given.
fn anon_ctors(terms: &Terms, t: Tm) -> Vec<(Vec<String>, String, Vec<String>)> {
    let mut out = Vec::new();
    let mut stack = vec![t];
    while let Some(t) = stack.pop() {
        if let (Kind::Ctor { ctor, fields }, Ty::Sum { ctors, .. }) = (terms.kind(t), terms.ty(t)) {
            out.push((
                ctors.iter().map(|(c, _)| c.clone()).collect(),
                ctors[*ctor as usize].0.clone(),
                fields.iter().map(|(f, _)| f.clone()).collect(),
            ));
        }
        stack.extend(terms.children(t));
    }
    out.sort();
    out.dedup();
    out
}

#[test]
fn adhoc_anonymous_constructors_take_their_index_in_the_joined_sum() {
    let sum = vec!["Alpha".to_string(), "Zed".to_string()];
    let want = vec![
        (sum.clone(), "Alpha".to_string(), Vec::new()),
        (sum, "Zed".to_string(), vec!["n".to_string()]),
    ];
    for (params, text) in [
        (
            vec![("i", "Identity")],
            "match i | Google g -> Zed { n: 1 } | Apple a -> Alpha | EmailPass e -> Zed { n: 2 }",
        ),
        (vec![], "[Zed { n: 1 }, Alpha]"),
        (vec![], "[Zed { n: 1 }] union [Alpha]"),
        (vec![("c", "Bool")], "if c then Zed { n: 1 } else Alpha"),
        (vec![], "Alpha == Zed { n: 1 }"),
        (vec![("z", "Zed { n: I64 }")], "if true then z else Alpha"),
    ] {
        let (_, a) = adhoc(&params, text).unwrap_or_else(|e| panic!("{text}: {e:?}"));
        assert_eq!(anon_ctors(&a.terms, a.items[0]), want, "{text}");
    }
}

// Joins

#[test]
fn match_arms_join_by_constructor_set() {
    assert_eq!(
        adhoc_ty(
            &[],
            "from i in identities select match i
               | Google g    -> g
               | Apple a     -> a
               | EmailPass e -> e"
        ),
        "Set Identity"
    );
    assert_eq!(
        adhoc_ty(
            &[("i", "Identity")],
            "match i | Google g -> key g | Apple a -> key a | EmailPass e -> key e"
        ),
        "Key identities"
    );
}

#[test]
fn partial_constructor_sets_join_to_their_union() {
    assert_eq!(
        adhoc_ty(
            &[("i", "Identity")],
            "from x in identities select match x | Google g -> key g | Apple a -> key a | EmailPass e -> key e"
        ),
        "Set (Key identities)"
    );
    assert_eq!(
        adhoc_ty(
            &[],
            "(from g in identities.Google select g) union (from a in identities.Apple select a)"
        ),
        "Set Identity.(Google | Apple)"
    );
}

#[test]
fn record_arms_join_only_when_equal() {
    adhoc_refuses(
        &[("i", "Identity")],
        "match i
           | Google g    -> { sub: g.gid, team: g.gid }
           | Apple a     -> { sub: a.sub }
           | EmailPass e -> { sub: e.email }",
        "field team missing",
    );
    adhoc_refuses(
        &[("i", "Identity")],
        "match i | Google g -> { n: 1 } | Apple a -> { n: a.sub } | EmailPass e -> { n: 2 }",
        "no common type",
    );
}

#[test]
fn different_sums_have_no_join() {
    adhoc_refuses(
        &[("i", "Identity"), ("u", "User")],
        "match i | Google g -> g | Apple a -> u | EmailPass e -> e",
        "no common type",
    );
}

// Records, constructors and fields

#[test]
fn a_constructor_application_synthesizes_its_row_type() {
    assert_eq!(
        adhoc_ty(
            &[("u", "Key users"), ("k", "Key identities")],
            "Google { id: k, user: u, gid: \"g\" }"
        ),
        "Identity.Google"
    );
    assert_eq!(adhoc_ty(&[], "Draft"), "Status.Draft");
}

#[test]
fn navigation_desugars_to_deref() {
    let (s, p) = program("fn f (i: Identity) -> Text = i.user.name").unwrap();
    let body = body_of(&s, &p, "f");
    let Kind::Field(inner, name) = p.terms.kind(body) else {
        panic!("a field, found {:?}", p.terms.kind(body));
    };
    assert_eq!(name, "name");
    assert!(matches!(
        p.terms.kind(*inner),
        Kind::Deref {
            reference: true,
            ..
        }
    ));
    let (s, p) = program("fn f (k: Key users) -> Text = k.name").unwrap();
    let body = body_of(&s, &p, "f");
    let Kind::Field(inner, _) = p.terms.kind(body) else {
        panic!("a field");
    };
    assert!(matches!(
        p.terms.kind(*inner),
        Kind::Deref {
            reference: false,
            ..
        }
    ));
}

// key

// Navigation and backlinks

#[test]
fn a_backlink_desugars_to_a_filtered_comprehension() {
    let (s, p) = program("fn f (u: User) -> Set Session = u.<sessions.owner").unwrap();
    let body = body_of(&s, &p, "f");
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
    assert!(matches!(p.terms.kind(args[1]), Kind::Ctor { ctor: 1, .. }));
}

// Comprehensions and nested results

#[test]
fn a_comprehension_is_nested_for_if_singleton() {
    let (s, p) = program(
        "fn f () -> Set Text = from u in users, i in identities where i.user == key u select u.name",
    )
    .unwrap();
    let body = body_of(&s, &p, "f");
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
}

#[test]
fn exists_tests_emptiness() {
    accepts(
        "fn active () -> Set User = from u in users where exists i in u.<identities.user select u",
    );
    accepts(
        "fn f () -> Set User = from u in users where not exists i in u.<identities.user where i.user == key u select u",
    );
    let (s, p) = program("fn f (u: User) -> Bool = exists i in u.<identities.user").unwrap();
    assert!(matches!(
        p.terms.kind(body_of(&s, &p, "f")),
        Kind::Exists(_)
    ));
}

#[test]
fn and_or_not_are_ifs() {
    let (s, p) = program("fn f (a: Bool, b: Bool) -> Bool = a and not b").unwrap();
    let Kind::If(_, rhs, f) = p.terms.kind(body_of(&s, &p, "f")) else {
        panic!("an if");
    };
    assert!(matches!(p.terms.kind(*f), Kind::Ctor { ctor: 0, .. }));
    assert!(matches!(p.terms.kind(*rhs), Kind::If(..)));
}

#[test]
fn constructor_relations_are_relations() {
    let (s, p) = program("fn f () -> Set (Identity.Google) = identities.Google").unwrap();
    assert!(matches!(
        p.terms.kind(body_of(&s, &p, "f")),
        Kind::Rel(_, Some(_))
    ));
    let (s, p) = program("fn rs () -> Set Identity = from i in identities select i\nfn f () -> Set (Identity.Google) = rs().Google").unwrap();
    assert!(matches!(
        p.terms.kind(body_of(&s, &p, "f")),
        Kind::Restrict(..)
    ));
}

// Result sums

#[test]
fn result_sums_are_checked_against_the_signature() {
    query_accepts(
        "query login_key (u: Key users) -> Set (ByEmail { email: Text } | ByGoogle { gid: Text }) =
           from i in identities.(EmailPass | Google) where i.user == u
           select match i
             | EmailPass e -> ByEmail { email: e.email }
             | Google g    -> ByGoogle { gid: g.gid }",
    );
    query_refuses(
        "query login_key (u: Key users) -> Set (ByEmail { email: Text } | ByGoogle { gid: Text }) =
           from i in identities.(EmailPass | Google) where i.user == u
           select match i
             | EmailPass e -> ByEmail { email: e.email }
             | Google g    -> ByApple { gid: g.gid }",
        "ByApple",
    );
}

#[test]
fn adhoc_result_sums_are_inferred() {
    assert_eq!(
        adhoc_ty(
            &[],
            "from i in identities select match i
               | EmailPass e -> ByEmail { email: e.email }
               | Google g    -> ByGoogle { gid: g.gid }
               | Apple a     -> ByGoogle { gid: a.sub }"
        ),
        "Set (ByEmail { email: Text } | ByGoogle { gid: Text })"
    );
    adhoc_refuses(
        &[],
        "from i in identities select match i
           | EmailPass e -> A { x: e.email }
           | Google g    -> A { x: 1 }
           | Apple a     -> B",
        "no common type",
    );
}

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
    let (s, p) = program("fn f (u: Key users) -> Set Text = from i in identities where i.user == u select display_name(i)").unwrap();
    let _ = body_of(&s, &p, "f");
    let i = s
        .functions
        .iter()
        .position(|f| f.name == "display_name")
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
        vec![0, 1, 2]
    );
    for (_, x, arm) in arms {
        let Kind::Let(z, y, _) = p.terms.kind(*arm) else {
            panic!("an arm binds its binder, found {:?}", p.terms.kind(*arm));
        };
        assert_ne!(z, x);
        assert_eq!(p.terms.kind(*y), &Kind::Var(*x));
    }
}

#[test]
fn a_match_on_a_constructor_set_cases_over_the_set() {
    let (s, p) = program(
        "fn f (o: Identity.OAuth) -> Text = match o | Google g -> g.gid | Apple a -> a.sub",
    )
    .unwrap();
    let Kind::Let(_, _, case) = p.terms.kind(body_of(&s, &p, "f")) else {
        panic!("a let");
    };
    let Kind::Case { arms, .. } = p.terms.kind(*case) else {
        panic!("a case");
    };
    assert_eq!(
        arms.iter().map(|(c, _, _)| *c).collect::<Vec<_>>(),
        vec![1, 2]
    );
}

#[test]
fn a_false_guard_falls_through_to_the_next_arm() {
    let (s, p) = program(
        "fn f (i: Identity) -> Text = match i
           | Google g if g.gid == \"corp\" -> \"staff\"
           | Google _                    -> \"guest\"
           | EmailPass _                 -> \"guest\"
           | Apple _                     -> \"guest\"",
    )
    .unwrap();
    assert_eq!(
        sexp(&p.terms, body_of(&s, &p, "f")),
        "(let ξ0 i1 (case ξ0 \
           (0 ξ2 \"guest\") \
           (1 ξ3 (if (let g4 ξ3 (Eq g4.gid \"corp\")) (let g5 ξ3 \"staff\") \"guest\")) \
           (2 ξ6 \"guest\")))"
    );
}

#[test]
fn a_nested_pattern_cases_on_the_field() {
    let (s, p) = program(
        "fn f (s: Shipment) -> Text = match s
           | Shipped { via: Courier c } -> c.name
           | Shipped { via: Mail m }    -> m.tracking
           | Pending                    -> \"pending\"",
    )
    .unwrap();
    assert_eq!(
        sexp(&p.terms, body_of(&s, &p, "f")),
        "(let ξ0 s1 (case ξ0 \
           (0 ξ2 \"pending\") \
           (1 ξ3 (if (case ξ3.via (0 ξ4 #1) (1 ξ5 #0)) \
             (case ξ3.via (0 ξ6 (let c7 ξ6 c7.name)) (1 ξ8 unreachable)) \
             (case ξ3.via (0 ξ9 unreachable) (1 ξ10 (let m11 ξ10 m11.tracking)))))))"
    );
}

#[test]
fn an_or_pattern_shares_its_body() {
    let (s, p) = program(
        "fn f (i: Identity) -> Key users = match i | Google g | Apple g -> g.user | EmailPass e -> e.user",
    )
    .unwrap();
    let Kind::Let(_, _, case) = p.terms.kind(body_of(&s, &p, "f")) else {
        panic!("a let");
    };
    let Kind::Case { arms, .. } = p.terms.kind(*case) else {
        panic!("a case");
    };
    let bodies: Vec<(Var, Tm)> = arms
        .iter()
        .filter(|(c, _, _)| *c != 0)
        .map(|(_, _, t)| match p.terms.kind(*t) {
            Kind::Let(v, _, b) => (*v, *b),
            k => panic!("a let, found {k:?}"),
        })
        .collect();
    assert_eq!(bodies[0], bodies[1]);
}

#[test]
fn a_scalar_match_is_a_chain_without_case() {
    let (s, p) =
        program("fn f (c: Text) -> I64 = match c | \"US\" -> 0 | \"CA\" -> 1 | _ -> 2").unwrap();
    let Kind::Let(_, _, chain) = p.terms.kind(body_of(&s, &p, "f")) else {
        panic!("a let");
    };
    let Kind::If(test, _, rest) = p.terms.kind(*chain) else {
        panic!("an if, found {:?}", p.terms.kind(*chain));
    };
    assert!(matches!(p.terms.kind(*test), Kind::Prim(Prim::Eq, _)));
    assert!(matches!(p.terms.kind(*rest), Kind::If(..)));
}

#[test]
fn a_match_on_a_key_derefs_it() {
    let (s, p) =
        program("fn f (r: Reset) -> Text = match r.identity | EmailPass e -> e.email").unwrap();
    let Kind::Let(_, scrutinee, _) = p.terms.kind(body_of(&s, &p, "f")) else {
        panic!("a let");
    };
    assert!(matches!(
        p.terms.kind(*scrutinee),
        Kind::Deref {
            reference: true,
            ..
        }
    ));
    let (s, p) =
        program("fn f (k: Key identities.EmailPass) -> Text = match k | EmailPass e -> e.email")
            .unwrap();
    let Kind::Let(_, scrutinee, _) = p.terms.kind(body_of(&s, &p, "f")) else {
        panic!("a let");
    };
    assert!(matches!(
        p.terms.kind(*scrutinee),
        Kind::Deref {
            reference: false,
            ..
        }
    ));
}

// Patterns and binder types

// Exhaustiveness

#[test]
fn adhoc_wildcards_cover_the_remaining_constructors() {
    adhoc_accepts(
        &[("i", "Identity")],
        "match i | Google g -> g.gid | _ -> \"other\"",
    );
    adhoc_accepts(
        &[],
        "from i in identities select match i | Google g -> g.gid | _ -> \"other\"",
    );
    adhoc_accepts(
        &[("i", "Identity")],
        "match i | Google g -> g.gid | x -> display_name(x)",
    );
    adhoc_refuses(
        &[("i", "Identity")],
        "match i | Google g -> g.gid | _ -> \"other\" | Apple a -> a.sub",
        "unreachable",
    );
    adhoc_refuses(
        &[("i", "Identity")],
        "match i | Google g -> g.gid",
        "match does not cover",
    );
}

// Grouping and aggregates

#[test]
fn a_group_desugars_to_group_over_binder_records() {
    let (s, p) = program(
        "fn f () -> Set (Key users) = from o in orders group o by o.user into g select g.key",
    )
    .unwrap();
    let Kind::Let(_, value, body) = p.terms.kind(body_of(&s, &p, "f")) else {
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
    use crate::kernel::Agg;
    for (op, value, want) in [
        ("min", "x.total", Agg::Min),
        ("max", "x.total", Agg::Max),
        ("avg", "f64(x.total)", Agg::Avg),
    ] {
        let (s, p) = program(&format!(
            "fn f () -> Set ({{ a: {t}, b: Option {t} }}) =
               from o in orders group o by o.user into g
               select {{ a: {op}({value} for x in g), b: {op}({value} for x in g where x.qty > 1) }}",
            t = if op == "avg" { "F64" } else { "I64" },
        ))
        .unwrap();
        let both = vec![(want, false), (want, true)];
        assert_eq!(agg_sources(&p.terms, body_of(&s, &p, "f")), both, "{op}");
        let n = crate::norm::program(&s, &p).expect("normalises");
        let (_, body) = n.functions[s.functions.iter().position(|f| f.name == "f").unwrap()]
            .clone()
            .expect("a normal form");
        assert_eq!(agg_sources(&n.terms, body), both, "{op} normalised");
    }
    let (s, p) = program("fn f () -> Option I64 = min(o.total for o in orders)").unwrap();
    assert_eq!(
        agg_sources(&p.terms, body_of(&s, &p, "f")),
        vec![(Agg::Min, false)]
    );
}

#[test]
fn adhoc_text_takes_the_key_of_a_view_row() {
    let view = "fn all_follows () -> Set Follow = from f in follows select f";
    if let Err(errs) = adhoc_over(view, "from f in all_follows() select key f") {
        panic!("refused: {errs:?}");
    }
}

#[test]
fn constructor_fields_pair_with_their_written_names() {
    let (_, a) = adhoc(
        &[],
        "Order { qty: 2, total: 1, user: users.key(1), id: orders.key(1) }",
    )
    .unwrap_or_else(|e| panic!("refused: {e:?}"));
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
    assert_eq!(1, int("total"));
    assert_eq!(2, int("qty"));
}

#[test]
fn an_aggregate_is_an_aggregate_node() {
    let (s, p) = program("fn f () -> I64 = sum(o.total for o in orders where o.qty > 1)").unwrap();
    let Kind::Agg { source, .. } = p.terms.kind(body_of(&s, &p, "f")) else {
        panic!("an agg");
    };
    assert!(matches!(p.terms.kind(*source), Kind::For(..)));
}

// Ordering and limits

#[test]
fn an_ordering_key_is_a_function_of_the_selected_value() {
    adhoc_refuses(
        &[],
        "from u in users, p in posts select p order by u.name",
        "an ordering key reads only the selected value",
    );
    adhoc_accepts(&[], "from u in users select { n: u.name } order by u.name");
    adhoc_refuses(
        &[],
        "from i in invoices select i order by i.status",
        "an ordering key has an ordered type",
    );
    adhoc_refuses(
        &[],
        "from u in users select { n: u.name } order by { n: u.name }",
        "an ordering key has an ordered type",
    );
}

#[test]
fn take_and_after_need_a_determinate_order() {
    query_accepts(
        "query page (cursor: Option (Key users)) -> Set User =
           from u in users select u order by u.id take 50 after cursor",
    );
    query_accepts(
        "query feed_page (cursor: Option ({ at: Instant, id: Key posts })) -> Set Post =
           from p in posts select p order by (p.created_at desc, key p) take 20 after cursor",
    );
    query_refuses(
        "query q () -> Set User = from u in users select u order by u.name take 5",
        "need a determinate order",
    );
    adhoc_accepts(
        &[],
        "from i in identities.EmailPass select i order by i.email take 5",
    );
    adhoc_refuses(
        &[],
        "from i in identities select i order by i.user take 5",
        "need a determinate order",
    );
    adhoc_accepts(
        &[],
        "from p in posts select p.author order by p.author take 5",
    );
    query_refuses(
        "query feed_page (cursor: Option ({ at: Instant, id: Key users })) -> Set Post =
           from p in posts select p order by (p.created_at desc, key p) take 20 after cursor",
        "field `id` of the `after` record",
    );
    query_refuses(
        "query page (cursor: Option Text) -> Set User =
           from u in users select u order by u.id take 50 after cursor",
        "expected Option (Key users)",
    );
}

#[test]
fn top_is_a_set_at_any_depth() {
    accepts(
        "fn f () -> Set ({ user: User, latest: Set Post }) = from u in users
           select { user: u,
                    latest: top 3 by (p.created_at desc, key p)
                              (from p in posts where p.author == key u select p) }",
    );
    refuses(
        "fn f (u: User) -> Set Post = top 3 by p.created_at (from p in posts where p.author == key u select p)",
        "`top` needs a determinate order",
    );
    let parsed = parse_schema(
        &source("fn f (xs: Set Post) -> Set Post = top 3 by 1 (xs)"),
        Options::default(),
    );
    assert!(
        parsed
            .diags
            .iter()
            .any(|d| d.message.contains("the operand of `top`, a comprehension")),
        "{:?}",
        parsed.diags
    );
    let (s, p) =
        program("fn f () -> Set Post = top 3 by key p (from p in posts select p)").unwrap();
    assert!(matches!(
        p.terms.kind(body_of(&s, &p, "f")),
        Kind::Top { .. }
    ));
}

// Recursion

#[test]
fn fix_is_a_kernel_fix() {
    let (s, p) = program(
        "fn f (me: Key users) -> Set (Key users) =
           fix reach = (from f in follows where f.src == me select f.dst)
                     union (from r in reach, f in follows where f.src == r select f.dst)
           in reach",
    )
    .unwrap();
    let Kind::Fix(binds, _) = p.terms.kind(body_of(&s, &p, "f")) else {
        panic!("a fix");
    };
    assert_eq!(binds.len(), 1);
}

// Functions, generics and lambdas

// Literals and numeric types

// Prelude overloads

#[test]
fn optional_filters_match_option_parameters() {
    query_accepts(
        "query search (name: Option Text, after: Option Instant) -> Set Post =
           from p in posts
           where match name  | None -> true | Some { value: n } -> p.title == n
           where match after | None -> true | Some { value: t } -> p.created_at > t
           select p",
    );
    query_accepts(
        "query by_status (s: Status, since: Instant) -> Set Invoice =
           from i in invoices where i.status == s and i.issued >= since select i",
    );
    query_refuses(
        "query q (p: I64 -> I64) -> I64 = p(1)",
        "never has a function type",
    );
}

// Constraints and other bodies

#[test]
fn writes_are_checked() {
    adhoc_accepts(
        &[("u", "Key users"), ("g", "Text")],
        "insert identities.Google { user: u, gid: g }",
    );
}

// Least types of arms

#[test]
fn constructor_arms_meet_at_their_selection() {
    assert_eq!(
        adhoc_ty(
            &[("b", "Bool"), ("t", "Instant")],
            "if b then Draft else Sent { at: t }"
        ),
        "Status.(Draft | Sent)"
    );
    assert_eq!(
        adhoc_ty(
            &[("b", "Bool"), ("t", "Instant")],
            "match b | true -> Draft | false -> Sent { at: t }"
        ),
        "Status.(Draft | Sent)"
    );
    assert_eq!(
        adhoc_ty(
            &[("s", "Status")],
            "match s | Draft -> Draft | Sent x -> Draft | Paid p -> Draft"
        ),
        "Status.Draft"
    );
}

#[test]
fn a_generic_constructor_arm_takes_its_arguments_from_the_other_arms() {
    assert_eq!(
        adhoc_ty(&[("b", "Bool")], "if b then None else Some { value: 1 }"),
        "Option I64"
    );
    assert_eq!(
        adhoc_ty(&[("b", "Bool")], "if b then Some { value: 1 } else None"),
        "Option I64"
    );
    assert_eq!(
        adhoc_ty(
            &[("b", "Bool")],
            "match b | true -> None | false -> Some { value: 1 }"
        ),
        "Option I64"
    );
}

#[test]
fn arms_with_no_least_type_are_refused() {
    adhoc_refuses(
        &[("b", "Bool")],
        "match b | true -> None | false -> None",
        "the type argument `a` of `Option` is not known here",
    );
    adhoc_refuses(&[("b", "Bool")], "if b then None else 1", "expected");
    adhoc_refuses(
        &[("b", "Bool"), ("t", "Instant")],
        "if b then Draft else Pending",
        "no common type",
    );
}

// Compound cursors

#[test]
fn a_cursor_record_matches_the_keys_by_field_name_order() {
    query_accepts(
        "query feed_page (cursor: Option ({ zz: Key posts, aa: Instant })) -> Set Post =
           from p in posts select p order by (p.created_at desc, key p) take 20 after cursor",
    );
    query_refuses(
        "query feed_page (cursor: Option ({ aa: Key posts, zz: Instant })) -> Set Post =
           from p in posts select p order by (p.created_at desc, key p) take 20 after cursor",
        "field `aa` of the `after` record",
    );
}

#[test]
fn a_literal_in_an_after_record_is_checked_against_its_ordering_key() {
    adhoc_accepts(
        &[],
        "from s in sales select s order by (s.qty, key s) take 5 after Some { value: { a: 3, b: sales.key(1) } }",
    );
    adhoc_refuses(
        &[],
        "from s in sales select s order by (s.qty, key s) take 5 after Some { value: { a: 3000000000, b: sales.key(1) } }",
        "literal out of range of I32",
    );
    adhoc_refuses(
        &[],
        "from p in posts select p order by (p.created_at desc, key p) take 5 after Some { value: { a: 40, b: posts.key(1) } }",
        "expected Instant, found I64",
    );
}

// Redundant alternatives

// Unary functions in the kernel

#[test]
fn a_function_is_one_lam_per_parameter() {
    let (s, p) = program("fn add (a: I64, b: I64) -> I64 = a + b").unwrap();
    let i = s.functions.iter().position(|f| f.name == "add").unwrap();
    let Kind::Lam(_, inner) = p.terms.kind(p.functions[i].unwrap()) else {
        panic!("a lam");
    };
    let Kind::Lam(_, body) = p.terms.kind(*inner) else {
        panic!("a second lam, found {:?}", p.terms.kind(*inner));
    };
    assert!(matches!(p.terms.kind(*body), Kind::Prim(Prim::Add, _)));
}

#[test]
fn a_view_is_a_lam_over_unit() {
    let (s, p) = program("fn v () -> I64 = 1").unwrap();
    let i = s.functions.iter().position(|f| f.name == "v").unwrap();
    let Kind::Lam(x, _) = p.terms.kind(p.functions[i].unwrap()) else {
        panic!("a lam");
    };
    assert_eq!(p.terms.var_info(*x).ty, Ty::Unit);
}

#[test]
fn applying_a_function_value_nests_apps() {
    let (s, p) = program("fn ap (g: I64 -> I64 -> I64) -> I64 = g(1, 2)").unwrap();
    let Kind::App(inner, two) = p.terms.kind(body_of(&s, &p, "ap")) else {
        panic!("an app");
    };
    assert!(matches!(p.terms.kind(*two), Kind::Const(_)));
    let Kind::App(g, _) = p.terms.kind(*inner) else {
        panic!("a nested app, found {:?}", p.terms.kind(*inner));
    };
    assert!(matches!(p.terms.kind(*g), Kind::Var(_)));
    let (s, p) = program("fn ap (g: () -> I64) -> I64 = g()").unwrap();
    let Kind::App(_, unit) = p.terms.kind(body_of(&s, &p, "ap")) else {
        panic!("an app");
    };
    assert_eq!(p.terms.ty(*unit), &Ty::Unit);
}

#[test]
fn a_declared_call_keeps_its_arguments() {
    let (s, p) = program(
        "fn add (a: I64, b: I64) -> I64 = a + b
         fn h () -> I64 = add(1, 2)",
    )
    .unwrap();
    let Kind::Call { args, .. } = p.terms.kind(body_of(&s, &p, "h")) else {
        panic!("a call");
    };
    assert_eq!(args.len(), 2);
}

#[test]
fn a_function_named_without_a_call_is_nested_lams_over_its_call() {
    let (s, p) = program(
        "fn add (a: I64, b: I64) -> I64 = a + b
         fn h () -> I64 -> I64 -> I64 = add",
    )
    .unwrap();
    let Kind::Lam(_, inner) = p.terms.kind(body_of(&s, &p, "h")) else {
        panic!("a lam");
    };
    let Kind::Lam(_, call) = p.terms.kind(*inner) else {
        panic!("a second lam");
    };
    assert!(matches!(p.terms.kind(*call), Kind::Call { args, .. } if args.len() == 2));
}

#[test]
fn a_lambda_of_several_parameters_is_nested_lams() {
    let (s, p) = program("fn h () -> I64 -> I64 -> I64 = fn (a: I64, b: I64) -> a + b").unwrap();
    let Kind::Lam(_, inner) = p.terms.kind(body_of(&s, &p, "h")) else {
        panic!("a lam");
    };
    assert!(matches!(p.terms.kind(*inner), Kind::Lam(..)));
}

// Group is never written

#[test]
fn text_literals_elaborate_to_nfc() {
    let (s, p) = program("fn f () -> Text = \"cafe\u{301}\"").expect("checks");
    assert_eq!(
        p.terms.kind(body_of(&s, &p, "f")),
        &Kind::Const(Const::Text("caf\u{e9}".to_string()))
    );
}

const BILLS: &str = "
type Bill = { id: Key bills, status: Status, ship: Shipment, note: Text }
relation bills : Bill
  key id generate sequence
  unique (status.Paid.at)
  index (ship.Shipped.via.Mail.tracking)
  index (status.Paid)
  unique (lower(note))
";

fn index_path(s: &Schema, p: &Program, i: usize) -> Vec<PathStep> {
    let rel = RelId(
        s.relations
            .iter()
            .position(|r| r.name == "bills")
            .expect("bills") as u32,
    );
    p.index_fields[&(rel, i, 0)].path.clone()
}

#[test]
fn an_index_field_through_a_value_level_constructor_names_its_path() {
    let (s, p) = program(BILLS).unwrap_or_else(|e| panic!("refused: {e:?}"));
    let field = |f: &str| PathStep::Field(f.into());
    assert_eq!(
        vec![field("status"), PathStep::Ctor(2), field("at")],
        index_path(&s, &p, 0)
    );
    assert_eq!(
        vec![
            field("ship"),
            PathStep::Ctor(1),
            field("via"),
            PathStep::Ctor(1),
            field("tracking")
        ],
        index_path(&s, &p, 1)
    );
    assert_eq!(
        vec![field("status"), PathStep::Ctor(2)],
        index_path(&s, &p, 2)
    );
    assert!(index_path(&s, &p, 3).is_empty());
}

#[test]
fn a_fix_binder_joins_anonymous_sums_across_its_union_operands() {
    let sum = vec!["Alpha".to_string(), "Zed".to_string()];
    let want = vec![
        (sum.clone(), "Alpha".to_string(), Vec::new()),
        (sum, "Zed".to_string(), vec!["n".to_string()]),
    ];
    for text in [
        "fix a = [Zed { n: 1 }] union (from x in a select Alpha) in a",
        "fix a = (from x in a select Zed { n: 1 }) union [Alpha] in a",
        "fix a = [Zed { n: 1 }] union [Alpha] union (from x in a select x) in a",
    ] {
        let (s, a) = adhoc(&[], text).unwrap_or_else(|e| panic!("{text}: {e:?}"));
        assert_eq!(
            "Set (Alpha | Zed { n: I64 })",
            s.show_ty(a.terms.ty(a.items[0])),
            "{text}"
        );
        assert_eq!(anon_ctors(&a.terms, a.items[0]), want, "{text}");
    }
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
