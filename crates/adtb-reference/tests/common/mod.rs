#![allow(dead_code)]

use std::collections::BTreeMap;

use adtb_lang::ast::{AdhocItem, BundleItem, ExampleItem};
use adtb_lang::check::{Checker, Context, Program, check_adhoc, check_query, check_schema};
use adtb_lang::parse::{Options, parse_adhoc, parse_example, parse_schema, parse_type};
use adtb_lang::resolve::Visible;
use adtb_lang::schema::{Schema, build};
use adtb_lang::span::FileId;
use adtb_lang::types::{RelId, Ty};
use adtb_reference::{Abort, Database, Fault, Interpreter, Value};

pub struct Fixture {
    pub schema: Schema,
    pub program: Program,
    pub db: Database,
}

impl Fixture {
    /// Builds the schema `src` declares, a module file without its `module` line.
    pub fn new(src: &str) -> Fixture {
        let text = format!("module app\n{src}");
        let file = parse_schema(&text, Options::default())
            .ok()
            .unwrap_or_else(|ds| panic!("the schema does not parse: {ds:?}"));
        let schema = build(&[(FileId(0), file)])
            .unwrap_or_else(|ds| panic!("the schema does not build: {ds:?}"));
        let program =
            check_schema(&schema).unwrap_or_else(|ds| panic!("the schema does not check: {ds:?}"));
        Fixture {
            schema,
            program,
            db: Database::new(),
        }
    }

    pub fn rel(&self, name: &str) -> RelId {
        let i = self
            .schema
            .relations
            .iter()
            .position(|r| r.name == name)
            .unwrap_or_else(|| panic!("no relation `{name}`"));
        RelId(i as u32)
    }

    pub fn interp(&self) -> Interpreter<'_> {
        Interpreter::new(&self.schema, &self.program, &self.db)
    }

    /// Adds to relation `rel` the rows the set expression `rows` denotes.
    pub fn rows(&mut self, rel: &str, rows: &str) {
        self.rows_with(rel, rows, &[]);
    }

    /// Adds to relation `rel` the rows the set expression `rows` denotes over `params`.
    pub fn rows_with(&mut self, rel: &str, rows: &str, params: &[(&str, &str, Value)]) {
        let r = self.rel(rel);
        let Ok(Value::Set(s)) = self.eval_with(rows, params) else {
            panic!("`{rows}` is not a set")
        };
        for row in s {
            let c = row.tag().expect("a row is a constructor value");
            self.db.entry((r, c)).or_default().insert(row);
        }
    }

    fn ty(&self, text: &str) -> Ty {
        let parsed = parse_type(text, Options::default())
            .ok()
            .unwrap_or_else(|_| panic!("`{text}` is not a type"));
        Checker::new(&self.schema, Visible::all(&self.schema), Context::Adhoc)
            .resolve_type(&parsed)
            .unwrap_or_else(|d| panic!("`{text}`: {}", d.message))
    }

    /// Evaluates the one statement of ad hoc text `text`, each parameter given as name, type and
    /// value.
    pub fn eval_with(&self, text: &str, params: &[(&str, &str, Value)]) -> Result<Value, Abort> {
        let items: Vec<AdhocItem> = parse_adhoc(text, Options::default())
            .ok()
            .unwrap_or_else(|ds| panic!("`{text}` does not parse: {ds:?}"));
        let tys: BTreeMap<String, Ty> = params
            .iter()
            .map(|(n, t, _)| (n.to_string(), self.ty(t)))
            .collect();
        let adhoc = check_adhoc(&self.schema, &self.program, &items, &tys)
            .unwrap_or_else(|ds| panic!("`{text}` does not check: {ds:?}"));
        let [item] = adhoc.items.as_slice() else {
            panic!("`{text}` holds one statement")
        };
        let bound: Vec<_> = params
            .iter()
            .map(|(n, _, v)| (adhoc.params[*n], v.clone()))
            .collect();
        self.interp().eval(&adhoc.terms, *item, &bound)
    }

    pub fn eval(&self, text: &str) -> Result<Value, Abort> {
        self.eval_with(text, &[])
    }

    /// Returns the value of `text`, which must not abort.
    pub fn value(&self, text: &str) -> Value {
        self.eval(text)
            .unwrap_or_else(|a| panic!("`{text}` aborts: {a:?}"))
    }

    /// Calls the registered query `decl` with `args`.
    pub fn call_query(&self, decl: &str, args: Vec<Value>) -> Result<Value, Abort> {
        let items = parse_example(decl, Options::default())
            .ok()
            .unwrap_or_else(|ds| panic!("`{decl}` does not parse: {ds:?}"));
        let [ExampleItem::BundleItem(BundleItem::Query(q))] = items.as_slice() else {
            panic!("`{decl}` is one query")
        };
        let module = self.schema.module_named("app").expect("the module");
        let query = check_query(&self.schema, &self.program, module, q)
            .unwrap_or_else(|d| panic!("`{decl}` does not check: {}", d.message));
        self.interp().apply(&query.terms, query.term, args)
    }

    /// Returns the one element of the set `text` denotes.
    pub fn one(&self, text: &str) -> Value {
        let Value::Set(s) = self.value(text) else {
            panic!("`{text}` is not a set")
        };
        assert_eq!(1, s.len(), "`{text}` has one element");
        s.into_iter().next().expect("one element")
    }

    pub fn fault(&self, f: Fault) -> Value {
        self.interp().fault_value(f)
    }
}

pub fn set(vs: impl IntoIterator<Item = Value>) -> Value {
    Value::set(vs)
}

pub fn int(i: i64) -> Value {
    Value::Int(i)
}

pub fn text(t: &str) -> Value {
    Value::text(t)
}

pub fn texts<'a>(ts: impl IntoIterator<Item = &'a str>) -> Value {
    Value::set(ts.into_iter().map(Value::text))
}

pub fn ints(is: impl IntoIterator<Item = i64>) -> Value {
    Value::set(is.into_iter().map(Value::Int))
}

pub fn key(i: i64) -> Value {
    Value::key(Value::Int(i))
}

pub fn keys(is: impl IntoIterator<Item = i64>) -> Value {
    Value::set(is.into_iter().map(key))
}
