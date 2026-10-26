use std::collections::{BTreeMap, HashMap};

use adtb_lang::analysis::Totality;
use adtb_lang::check::check_schema;
use adtb_lang::kernel::{Kind, Prim, Terms, Transition, Var};
use adtb_lang::norm::{self, NormalProgram};
use adtb_lang::parse::{Options, parse_schema};
use adtb_lang::schema::{Schema, ValueDef, build};
use adtb_lang::span::FileId;
use adtb_lang::types::{Base, RelId, Ty};

use super::{At, Binder, Compiler, Occurrence, Origin, Row};
use crate::plan::{self, Catalog, Facts};

fn schema() -> Schema {
    let text = "module app\ntype U = { id: Key us }\nrelation us : U key id generate sequence\n";
    let file = parse_schema(text, Options::default())
        .ok()
        .unwrap_or_else(|ds| panic!("the schema does not parse: {ds:?}"));
    let schema = build(&[(FileId(0), file)])
        .unwrap_or_else(|ds| panic!("the schema does not build: {ds:?}"));
    check_schema(&schema).unwrap_or_else(|ds| panic!("the schema does not check: {ds:?}"));
    schema
}

fn normal(schema: &Schema) -> NormalProgram {
    let program =
        check_schema(schema).unwrap_or_else(|ds| panic!("the schema does not check: {ds:?}"));
    norm::program(schema, &program).expect("the schema's bodies normalise")
}

fn us(schema: &Schema) -> RelId {
    let m = schema.module_named("app").expect("the module");
    match schema.module(m).values.get("us") {
        Some(&ValueDef::Relation(r)) => r,
        _ => panic!("no relation `us`"),
    }
}

fn compiler<'a>(
    schema: &'a Schema,
    normal: &'a NormalProgram,
    catalog: &'a Catalog,
    terms: &'a mut Terms,
    prior: Option<Var>,
) -> Compiler<'a> {
    let rel = us(schema);
    Compiler {
        schema,
        normal,
        catalog,
        terms,
        facts: Facts::default(),
        total: Totality::new(schema, false),
        transitions: prior
            .map(|v| (v, (Transition::Prior, rel, None)))
            .into_iter()
            .collect(),
        inputs: BTreeMap::new(),
    }
}

fn internal<T: std::fmt::Debug>(r: Result<T, plan::Error>) -> String {
    match r {
        Err(plan::Error::Internal(what)) => what,
        other => panic!("{other:?} is not an internal error"),
    }
}

fn int() -> Ty {
    Ty::Base(Base::I64)
}

fn bare(schema: &Schema, node: adtb_lang::kernel::Tm, origin: Origin) -> Occurrence {
    let rel = us(schema);
    Occurrence {
        rel,
        ctors: [0].into_iter().collect(),
        origin,
        node,
        at: At::Nested {
            row: Row::Bare,
            path: Vec::new(),
            guards: Vec::new(),
        },
    }
}

#[test]
fn a_prior_relation_not_a_set_is_an_internal_error() {
    let s = schema();
    let n = normal(&s);
    let catalog = Catalog::default();
    let mut terms = Terms::default();
    let v = terms.var("p", int());
    let t = terms.add(Kind::Var(v), int());
    let mut cx = compiler(&s, &n, &catalog, &mut terms, Some(v));
    let what = internal(cx.priors(t));
    assert!(what.contains("a prior relation is a set"), "{what}");
}

#[test]
fn a_top_level_read_outside_a_for_is_an_internal_error() {
    let s = schema();
    let n = normal(&s);
    let catalog = Catalog::default();
    let mut terms = Terms::default();
    let q = terms.add(Kind::EmptySet, Ty::Set(Box::new(int())));
    let mut cx = compiler(&s, &n, &catalog, &mut terms, None);
    let o = bare(&s, q, Origin::State);
    let what = internal(cx.top_delta(q, q, &o));
    assert!(what.contains("the source of a `For`"), "{what}");
}

#[test]
fn a_read_of_rows_not_a_set_is_an_internal_error() {
    let s = schema();
    let n = normal(&s);
    let catalog = Catalog::default();
    let mut terms = Terms::default();
    let q = terms.add(Kind::EmptySet, Ty::Set(Box::new(int())));
    let node = terms.add(Kind::Rel(us(&s), None), int());
    let mut cx = compiler(&s, &n, &catalog, &mut terms, None);
    let o = bare(&s, node, Origin::State);
    let what = internal(cx.nested_delta(q, &o, &Row::Bare, &[], &[], &HashMap::new()));
    assert!(what.contains("a read of rows is a set"), "{what}");
}

#[test]
fn a_change_set_input_not_a_set_is_an_internal_error() {
    let s = schema();
    let n = normal(&s);
    let catalog = Catalog::default();
    let mut terms = Terms::default();
    let q = terms.add(Kind::EmptySet, Ty::Set(Box::new(int())));
    let v = terms.var("ins", int());
    let node = terms.add(Kind::Var(v), int());
    let y = terms.var("y", int());
    let mut cx = compiler(&s, &n, &catalog, &mut terms, None);
    let o = bare(&s, node, Origin::Inserted);
    let what = internal(cx.nested_delta(q, &o, &Row::Gen(y), &[], &[], &HashMap::new()));
    assert!(what.contains("a change set input is a set"), "{what}");
}

#[test]
fn a_top_level_binder_outside_a_for_or_let_is_an_internal_error() {
    let s = schema();
    let n = normal(&s);
    let catalog = Catalog::default();
    let mut terms = Terms::default();
    let set = Ty::Set(Box::new(int()));
    let q = terms.add(Kind::EmptySet, set.clone());
    let rows = terms.var("rows", set.clone());
    let node = terms.add(Kind::Var(rows), set);
    let x = terms.var("x", int());
    let xv = terms.add(Kind::Var(x), int());
    let bool_ty = Ty::Data {
        def: s.prelude_type("Bool"),
        args: Vec::new(),
        ctors: None,
    };
    let guard = terms.add(Kind::Prim(Prim::Eq, vec![xv, xv]), bool_ty);
    let y = terms.var("y", int());
    let mut cx = compiler(&s, &n, &catalog, &mut terms, None);
    let o = bare(&s, node, Origin::Inserted);
    let binders = HashMap::from([(x, Binder { node: q, order: 0 })]);
    let what = internal(cx.nested_delta(q, &o, &Row::Gen(y), &[], &[guard], &binders));
    assert!(
        what.contains("a top level binder is a `For` or a `Let`"),
        "{what}"
    );
}
