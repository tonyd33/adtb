//! Every kind of root the checker admits against every reach it can be refused, written directly
//! and through a function, used and dead.

use super::*;
use crate::capability::{self, Reach, Refusal, RootKind};

const ALL_REFUSED: &[Reach] = &[
    Reach::Volatile,
    Reach::Caller,
    Reach::Nondeterministic,
    Reach::File,
    Reach::Transition,
    Reach::Write,
];

const ROW_ONLY: &[Reach] = &[
    Reach::Volatile,
    Reach::Caller,
    Reach::Nondeterministic,
    Reach::File,
    Reach::Transition,
    Reach::Write,
    Reach::Deref,
    Reach::Relation,
    Reach::Generate,
];

/// Each kind of root with the reaches the spec refuses it, and the sections that do.
const REFUSED: [(RootKind, &[Reach], &str); 17] = [
    (
        RootKind::Constraint,
        &[
            Reach::Volatile,
            Reach::Caller,
            Reach::Nondeterministic,
            Reach::File,
            Reach::Write,
        ],
        "constraints.md 1.2, 6.1; catalog.md 1.3; security.md 1.2; interop.md 9.3; writes.md 1",
    ),
    (
        RootKind::Check,
        ALL_REFUSED,
        "constraints.md 2.2: the constraint over one row; security.md 1.2",
    ),
    (
        RootKind::Domain,
        &[
            Reach::Volatile,
            Reach::Caller,
            Reach::Nondeterministic,
            Reach::File,
            Reach::Transition,
            Reach::Write,
            Reach::Deref,
            Reach::Relation,
        ],
        "constraints.md 2.3: reads only its value",
    ),
    (
        RootKind::DeleteAction,
        ALL_REFUSED,
        "catalog.md 1.3; security.md 1.2",
    ),
    (
        RootKind::Default,
        &[
            Reach::Volatile,
            Reach::Caller,
            Reach::Nondeterministic,
            Reach::File,
            Reach::Transition,
            Reach::Write,
            Reach::Deref,
            Reach::Relation,
        ],
        "derived.md 4.1: closed and deterministic, calls allowed",
    ),
    (
        RootKind::Computed,
        ALL_REFUSED,
        "catalog.md 1.3; security.md 1.2; interop.md 9.3",
    ),
    (
        RootKind::Index,
        &[
            Reach::Volatile,
            Reach::Caller,
            Reach::Nondeterministic,
            Reach::File,
            Reach::Transition,
            Reach::Write,
            Reach::Deref,
        ],
        "derived.md 5.2: an expression of the row; catalog.md 1.3; security.md 1.2",
    ),
    (
        RootKind::StoredView,
        ALL_REFUSED,
        "catalog.md 1.3; security.md 1.2; interop.md 9.3",
    ),
    (
        RootKind::Function,
        &[Reach::File, Reach::Transition, Reach::Write],
        "constraints.md 6.1; interop.md 3; writes.md 1",
    ),
    (
        RootKind::Aggregate,
        &[Reach::File, Reach::Transition, Reach::Write],
        "interop.md 10: as a function",
    ),
    (
        RootKind::Query,
        &[Reach::Transition],
        "catalog.md 1.3; interop.md 9.3; constraints.md 6.1",
    ),
    (
        RootKind::HidingQuery,
        &[Reach::Transition],
        "as a registered query",
    ),
    (
        RootKind::Adhoc,
        &[Reach::Transition],
        "as a registered query",
    ),
    (
        RootKind::Script,
        &[Reach::Caller, Reach::Transition],
        "evolution.md 4.14: a script is a write; security.md 1.2",
    ),
    (RootKind::AtBody, &[Reach::Write], "derived.md 6.2"),
    (RootKind::Fill, ROW_ONLY, "a fill reads only its row"),
    (RootKind::Routing, ROW_ONLY, "a routing reads only its row"),
];

const REACHES: [Reach; 10] = [
    Reach::Volatile,
    Reach::Caller,
    Reach::Nondeterministic,
    Reach::File,
    Reach::Transition,
    Reach::Write,
    Reach::Deref,
    Reach::UserAggregate,
    Reach::Relation,
    Reach::Generate,
];

fn refused(kind: RootKind) -> &'static [Reach] {
    REFUSED
        .iter()
        .find(|(k, _, _)| *k == kind)
        .map(|(_, r, _)| *r)
        .expect("every kind is listed")
}

#[test]
fn every_entry_refuses_exactly_the_reaches_the_spec_refuses() {
    let mut failures = Vec::new();
    for (kind, forbids, source) in REFUSED {
        for reach in REACHES {
            let want = if forbids.contains(&reach) {
                Err(Refusal::new(kind, reach))
            } else {
                Ok(())
            };
            let got = capability::admit(kind, &BTreeSet::from([reach]));
            if got != want {
                failures.push(format!("{kind} {reach:?} ({source}): got {got:?}"));
            }
        }
    }
    assert_eq!(Vec::<String>::new(), failures);
    let kinds: BTreeSet<RootKind> = REFUSED.iter().map(|(k, _, _)| *k).collect();
    assert_eq!(17, kinds.len());
}

const DECLS: &str = "fn stamp (n: I64) -> I64 = host \"stamp\" version \"1.0\"
aggregate tot (x: I64) -> I64 = state { s: I64 } init { s: 0 } step (st, x) -> { s: st.s + x } merge (a, b) -> { s: a.s + b.s } finish st -> st.s";

/// Each `Bool` expression with everything it reaches. A file read, a write and a generated key
/// have no `Bool` expression a body of every kind can hold: the table above covers them.
const EXPRESSIONS: [(&[Reach], &str); 7] = [
    (
        &[Reach::Volatile, Reach::Relation],
        "count(s for s in sys::sessions) == 0",
    ),
    (
        &[Reach::Caller],
        "match caller | Anonymous -> true | EndUser _ -> false | Service _ -> false",
    ),
    (&[Reach::Nondeterministic], "stamp(1) == 1"),
    (
        &[Reach::Transition, Reach::Relation],
        "count(r for r in inserted(raws)) == 0",
    ),
    (&[Reach::Deref], "users.key(1).name == \"\""),
    (
        &[Reach::UserAggregate],
        "tot(x for x in [1]) == Some { value: 0 }",
    ),
    (&[Reach::Relation], "count(r for r in raws) == 0"),
];

/// Each kind of root a text can declare, with a declaration or text holding the `Bool`
/// expression `{X}`. A delete action holds only literals, a hiding query is a registered query,
/// and an `at` body is judged as the text holding it but for writes.
const ROOTS: [(RootKind, &str); 14] = [
    (
        RootKind::Constraint,
        "constraint c = from r in raws where {X} select r",
    ),
    (RootKind::Check, "check c on raws ({X})"),
    (RootKind::Domain, "type D = new I64 where {X}"),
    (
        RootKind::Default,
        "type Tick = { id: Key ticks, b: Bool }\nrelation ticks : Tick key id generate sequence default b = {X}",
    ),
    (
        RootKind::Computed,
        "type Tick = { id: Key ticks, n: I64 }\nrelation ticks : Tick key id generate sequence computed z: Bool = {X}",
    ),
    (
        RootKind::Index,
        "type Tick = { id: Key ticks, n: I64 }\nrelation ticks : Tick key id generate sequence\n  index ({X})",
    ),
    (
        RootKind::StoredView,
        "fn v () -> Set Bool = from r in raws select {X}\nstore v",
    ),
    (RootKind::Function, "fn v () -> Bool = {X}"),
    (
        RootKind::Aggregate,
        "aggregate t (x: I64) -> Bool = state { s: Bool } init { s: {X} } step (st, x) -> st merge (a, b) -> a finish st -> st.s",
    ),
    (
        RootKind::Query,
        "query q () -> Set Bool = from r in raws select {X}",
    ),
    (RootKind::Adhoc, "from r in raws select {X}"),
    (RootKind::Script, "delete r in raws where {X}"),
    (RootKind::Fill, "{X}"),
    (RootKind::Routing, "{X}"),
];

const VOLATILE_WRITE: &str = "a write must not depend on a volatile relation: not in its target, its predicate, its values, or a value or condition it depends on";

/// Returns the refusal of a root of `kind` reaching `reaches`, naming the first its entry forbids.
fn refusal(kind: RootKind, reaches: &[Reach]) -> Option<String> {
    let mut sorted = reaches.to_vec();
    sorted.sort();
    sorted
        .into_iter()
        .find(|r| refused(kind).contains(r))
        .map(|r| Refusal::new(kind, r).to_string())
}

/// Returns the refusals of a root of `kind` holding an expression reaching `reaches`, through a
/// function of the schema when `through`.
fn expected(kind: RootKind, reaches: &[Reach], through: bool) -> Vec<String> {
    let function = through
        .then(|| refusal(RootKind::Function, reaches))
        .flatten();
    let outside = matches!(
        kind,
        RootKind::Query | RootKind::Adhoc | RootKind::Script | RootKind::Fill | RootKind::Routing
    );
    if function.is_some() && outside {
        return function.into_iter().collect();
    }
    let own = match refusal(kind, reaches) {
        None if kind == RootKind::Script && reaches.contains(&Reach::Volatile) => {
            Some(VOLATILE_WRITE.to_string())
        }
        r => r,
    };
    function.into_iter().chain(own).collect()
}

type Outside<'f> = &'f dyn Fn(&Compiled) -> Result<(), Vec<String>>;

/// Returns the messages checking `kind`'s text `body`, with `decl` declared in the schema, gives.
fn outcome(kind: RootKind, decl: &str, body: &str) -> Vec<String> {
    let outside = |f: Outside| program(decl).and_then(|c| f(&c));
    let r = match kind {
        RootKind::Query => outside(&|_| query(decl, body).map_err(|e| vec![e])),
        RootKind::Adhoc => outside(&|_| adhoc_over(decl, body).map(drop)),
        RootKind::Script => outside(&|c| {
            let items = parse_adhoc(body, Options::default()).ok().expect("parsed");
            let [ast::AdhocItem::Write(w)] = items.as_slice() else {
                panic!("{body}: one write");
            };
            let r = check_script(c, w, None);
            r.map_err(|ds| ds.into_iter().map(|d| d.message).collect())
        }),
        RootKind::Fill | RootKind::Routing => outside(&|compiled| {
            let s = compiled.committed();
            let items = parse_adhoc(body, Options::default()).ok().expect("parsed");
            let [ast::AdhocItem::Expr(e)] = items.as_slice() else {
                panic!("{body}: one expression");
            };
            let c = Checker::of_adhoc(s, Visible::all(s));
            let raw = parse_type("Raw", Options::default()).ok().expect("a type");
            let row = c
                .resolve_type(&raw, FnTypes::Allowed)
                .expect("a known type");
            let b = c.bool_ty();
            let module = s.module_named("auth").expect("module auth");
            let r = check_row(compiled, module, &row, RowBody::Expr(e), Some(&b), kind);
            r.map(drop).map_err(|d| vec![d.message])
        }),
        _ => program(&format!("{decl}\n{body}")).map(drop),
    };
    r.err().unwrap_or_default()
}

#[test]
fn every_root_is_refused_exactly_the_reaches_its_entry_forbids() {
    let mut failures = Vec::new();
    let mut cases = 0;
    let mut refusals = 0;
    for (kind, text) in ROOTS {
        for (reaches, e) in EXPRESSIONS {
            let helper = format!("{DECLS}\nfn g () -> Bool = {e}");
            let forms = [
                (false, DECLS.to_string(), e.to_string()),
                (false, DECLS.to_string(), format!("{{ a: {e}, b: true }}.b")),
                (true, helper.clone(), "g()".to_string()),
                (true, helper, "{ a: g(), b: true }.b".to_string()),
            ];
            for (through, decl, x) in forms {
                let want = expected(kind, reaches, through);
                refusals += usize::from(!want.is_empty());
                let got = outcome(kind, &decl, &text.replace("{X}", &x));
                if got != want {
                    failures.push(format!(
                        "{kind} {reaches:?} `{x}`: want {want:?}, got {got:?}"
                    ));
                }
                cases += 1;
            }
        }
    }
    assert_eq!(Vec::<String>::new(), failures);
    assert_eq!(392, cases);
    assert_eq!(REFUSALS, refusals);
}

const REFUSALS: usize = 206;

#[test]
fn an_at_body_never_writes() {
    let at = |x: &str| {
        format!(
            "insert follows from (at commit Position {{ epoch: 0, lsn: 0 }} {{ [Follow {{ src: {x}, dst: users.key(1) }}] }})"
        )
    };
    let write = "key (insert users { name: \"a\" })";
    let want = vec![Refusal::new(RootKind::AtBody, Reach::Write).to_string()];
    for x in [
        write.to_string(),
        format!("{{ a: {write}, b: users.key(1) }}.b"),
    ] {
        assert_eq!(Err(want.clone()), adhoc_over("", &at(&x)).map(drop), "{x}");
    }
}

/// Returns the `reference` flag of each deref the normal form of ad hoc `text` keeps.
fn normal_derefs(text: &str) -> Vec<bool> {
    let s = schema("").expect("the running schema builds");
    let p = check_schema(&s).expect("the schema checks");
    let items = parse_adhoc(text, Options::default()).ok().expect("parsed");
    let a = check_adhoc(&p, &items, &BTreeMap::new()).expect("checks");
    let n = crate::norm::adhoc(&p, &a).expect("normalises");
    deref_flags(n.terms(), n.items()[0])
}

#[test]
fn an_unused_deref_in_an_at_body_is_partial_and_kept() {
    let now = "from p in phones select { a: p.owner.name, b: 1 }.b";
    let past = "from p in phones select at commit Position { epoch: 0, lsn: 0 } { from x in [1] select { a: p.owner.name, b: x }.b }";
    assert_eq!(Vec::<bool>::new(), normal_derefs(now));
    assert_eq!(vec![false], normal_derefs(past));
}
