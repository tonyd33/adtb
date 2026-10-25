#![allow(dead_code)]

use std::collections::BTreeMap;

use adtb_lang::ast::{AdhocItem, BundleItem, ExampleItem};
use adtb_lang::check::{
    Checker, Context, Program, check_adhoc, check_query, check_schema, check_txn,
};
use adtb_lang::parse::{Options, parse_adhoc, parse_example, parse_schema, parse_type};
use adtb_lang::resolve::Visible;
use adtb_lang::schema::{Schema, ValueDef, build};
use adtb_lang::span::FileId;
use adtb_lang::types::{RelId, Ty, TypeId};
use adtb_reference::{
    Abort, Database, Fault, Interpreter, Outcome, Rejection, Stage, Supply, Value,
};

pub struct Fixture {
    pub schema: Schema,
    pub program: Program,
    pub db: Database,
    pub supply: Supply,
    /// The constructors not open for writes.
    pub closed: BTreeMap<(TypeId, u32), Stage>,
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
            supply: Supply::default(),
            closed: BTreeMap::new(),
        }
    }

    /// Returns the declared type `name` of the module and the index of its constructor `ctor`.
    pub fn ctor(&self, name: &str, ctor: &str) -> (TypeId, u32) {
        let t = self.schema.modules
            [self.schema.module_named("app").expect("the module").0 as usize]
            .types[name];
        let c = self.schema.ty(t).ctor_index(ctor).expect("the constructor");
        (t, c)
    }

    pub fn rel(&self, name: &str) -> RelId {
        let m = self.schema.module_named("app").expect("the module");
        match self.schema.module(m).values.get(name) {
            Some(&ValueDef::Relation(r)) => r,
            _ => panic!("no relation `{name}`"),
        }
    }

    pub fn interp(&self) -> Interpreter<'_> {
        Interpreter::new(&self.schema, &self.program, &self.db).with_closed(self.closed.clone())
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

    /// Returns the type `text` names in the fixture's schema.
    pub fn ty(&self, text: &str) -> Ty {
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

    /// Runs the one statement of ad hoc text `text` over `params`; when it succeeds, its state
    /// becomes the fixture's database.
    pub fn run_with(
        &mut self,
        text: &str,
        params: &[(&str, &str, Value)],
    ) -> Result<Outcome, Abort> {
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
        let normal = adtb_lang::norm::adhoc(&self.schema, &self.program, &adhoc)
            .unwrap_or_else(|e| panic!("`{text}` has no normal form: {e:?}"));
        let normal_bound: Vec<_> = params
            .iter()
            .map(|(n, _, v)| (normal.params[*n], v.clone()))
            .collect();
        let mut normal_supply = self.supply.clone();
        let normal_out = self.interp().run(
            &normal.terms,
            normal.items[0],
            &normal_bound,
            &mut normal_supply,
        );
        let mut supply = std::mem::take(&mut self.supply);
        let out = self.interp().run(&adhoc.terms, *item, &bound, &mut supply);
        self.supply = supply;
        assert_eq!(out, normal_out, "`{text}` and its normal form agree");
        if let Ok(o) = &out {
            self.db = o.db.clone();
        }
        out
    }

    pub fn run(&mut self, text: &str) -> Result<Outcome, Abort> {
        self.run_with(text, &[])
    }

    /// Runs `text`, which must succeed, and returns its result.
    pub fn ok(&mut self, text: &str) -> Value {
        self.run(text)
            .unwrap_or_else(|a| panic!("`{text}` aborts: {a:?}"))
            .result
    }

    /// Runs `text`, which must be rejected, and returns the reason; the database is unchanged.
    pub fn rejected(&mut self, text: &str) -> Rejection {
        let before = self.db.clone();
        match self.run(text) {
            Err(Abort::Reject(r)) => {
                assert_eq!(before, self.db, "a rejected statement changes nothing");
                r
            }
            other => panic!("`{text}` is not rejected: {other:?}"),
        }
    }

    /// Calls the registered transaction `decl` with `args`; when it succeeds, its state becomes
    /// the fixture's database.
    pub fn call_txn(&mut self, decl: &str, args: Vec<Value>) -> Result<Outcome, Abort> {
        let items = parse_example(decl, Options::default())
            .ok()
            .unwrap_or_else(|ds| panic!("`{decl}` does not parse: {ds:?}"));
        let [ExampleItem::BundleItem(BundleItem::Txn(t))] = items.as_slice() else {
            panic!("`{decl}` is one transaction")
        };
        let module = self.schema.module_named("app").expect("the module");
        let txn = check_txn(&self.schema, &self.program, module, t)
            .unwrap_or_else(|d| panic!("`{decl}` does not check: {}", d.message));
        let normal = adtb_lang::norm::txn(&self.schema, &self.program, &txn)
            .unwrap_or_else(|e| panic!("`{decl}` has no normal form: {e:?}"));
        let normal_bound: Vec<_> = if normal.params.len() == args.len() {
            normal
                .params
                .iter()
                .copied()
                .zip(args.iter().cloned())
                .collect()
        } else {
            normal.params.iter().map(|p| (*p, Value::unit())).collect()
        };
        let mut normal_supply = self.supply.clone();
        let normal_out = self.interp().run(
            &normal.terms,
            normal.body,
            &normal_bound,
            &mut normal_supply,
        );
        let mut supply = std::mem::take(&mut self.supply);
        let out = self
            .interp()
            .run_call(&txn.terms, txn.term, args, &mut supply);
        self.supply = supply;
        assert_eq!(out, normal_out, "`{decl}` and its normal form agree");
        if let Ok(o) = &out {
            self.db = o.db.clone();
        }
        out
    }

    /// Returns the rows of relation `rel`.
    pub fn table(&self, rel: &str) -> Value {
        let r = self.rel(rel);
        Value::set(
            self.db
                .iter()
                .filter(|((x, _), _)| *x == r)
                .flat_map(|(_, rows)| rows.iter().cloned()),
        )
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
