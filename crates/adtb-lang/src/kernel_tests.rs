use std::collections::{BTreeMap, BTreeSet};

use super::*;

fn int() -> Ty {
    Ty::Base(Base::I64)
}

fn free(terms: &Terms, t: Tm) -> BTreeSet<Var> {
    (*Frees::default().of(terms, t)).clone()
}

#[test]
fn a_let_variable_read_in_its_own_value_is_free() {
    let mut terms = Terms::default();
    let x = terms.var("x", int());
    let read = terms.add(Kind::Var(x), int());
    let t = terms.add(Kind::Let(x, read, read), int());
    assert_eq!(free(&terms, t), BTreeSet::from([x]));
}

#[test]
fn a_for_variable_read_in_its_own_source_is_free() {
    let mut terms = Terms::default();
    let set = Ty::Set(Box::new(int()));
    let x = terms.var("x", set.clone());
    let read = terms.add(Kind::Var(x), set.clone());
    let t = terms.add(Kind::For(x, read, read), set);
    assert_eq!(free(&terms, t), BTreeSet::from([x]));
}

#[test]
fn a_variable_bound_around_every_child_is_not_free() {
    let mut terms = Terms::default();
    let x = terms.var("x", int());
    let read = terms.add(Kind::Var(x), int());
    let lam = terms.add(Kind::Lam(x, read), Ty::Fn(Box::new(int()), Box::new(int())));
    let fix = terms.add(Kind::Fix(vec![(x, read)], read), int());
    assert_eq!(free(&terms, lam), BTreeSet::new());
    assert_eq!(free(&terms, fix), BTreeSet::new());
}

#[test]
fn renaming_copies_a_term_whose_binder_is_free_in_its_own_value() {
    let mut terms = Terms::default();
    let x = terms.var("x", int());
    let y = terms.var("y", int());
    let read = terms.add(Kind::Var(x), int());
    let test = terms.add(Kind::Const(Const::Int(1)), int());
    let inner = terms.add(Kind::Let(x, read, read), int());
    let t = terms.add(Kind::If(test, inner, inner), int());
    let c = terms.renamed(t, &BTreeMap::from([(x, y)]), &mut Frees::default());
    assert_ne!(c, t);
    assert_eq!(free(&terms, c), BTreeSet::from([y]));
}

struct Fixture {
    schema: crate::schema::Schema,
    ts: RelId,
    us: RelId,
    t: TypeId,
    f: FnId,
}

impl Fixture {
    fn new() -> Fixture {
        let src = "module m
type T { id: Key ts } = A { x: I64 } | B { x: I64 } | C
relation ts : T key id generate sequence
type U = { id: Key us }
relation us : U key id generate sequence
fn f (n: I64) -> I64 = n";
        let parsed = crate::parse::parse_schema(src, crate::parse::Options::default());
        let file = parsed.ok().expect("the schema parses");
        let schema = crate::schema::build(&[(crate::span::FileId(0), file)])
            .unwrap_or_else(|_| panic!("the schema builds"));
        let rel = |name: &str| {
            RelId(
                schema
                    .relations
                    .iter()
                    .position(|r| r.name == name)
                    .expect("a relation of that name") as u32,
            )
        };
        let (ts, us) = (rel("ts"), rel("us"));
        let t = schema.relation(ts).elem_def;
        let m = schema.module(schema.module_named("m").expect("module m"));
        let Some(&crate::schema::ValueDef::Function(f)) = m.values.get("f") else {
            panic!("no function `f`");
        };
        Fixture {
            schema,
            ts,
            us,
            t,
            f,
        }
    }

    fn row(&self, ctors: Option<CtorSet>) -> Ty {
        Ty::Data {
            def: self.t,
            args: Vec::new(),
            ctors,
        }
    }

    fn prelude(&self, name: &str) -> Ty {
        Ty::Data {
            def: self.schema.prelude_type(name),
            args: Vec::new(),
            ctors: None,
        }
    }
}

fn text() -> Ty {
    Ty::Base(Base::Text)
}

fn set(t: Ty) -> Ty {
    Ty::Set(Box::new(t))
}

fn one(terms: &mut Terms) -> Tm {
    terms.add(Kind::Const(Const::Int(1)), int())
}

fn word(terms: &mut Terms) -> Tm {
    terms.add(Kind::Const(Const::Text("w".into())), text())
}

/// Returns a hand-built root, its parameters and the node that breaks an invariant.
type Build = fn(&Fixture, &mut Terms) -> (Tm, Vec<Var>, Tm);

/// Returns the refusals of binding, constructor sets, branches and functions.
fn binding_refusals() -> Vec<(&'static str, Build)> {
    vec![
        ("`x` is read where nothing binds it", |_, terms| {
            let x = terms.var("x", int());
            let read = terms.add(Kind::Var(x), int());
            (read, Vec::new(), read)
        }),
        ("the variable's type is `I64`, not `Text`", |_, terms| {
            let x = terms.var("x", int());
            let read = terms.add(Kind::Var(x), text());
            (read, vec![x], read)
        }),
        (
            "constructor set [0, 1, 2] names every constructor, so is `None`",
            |fx, terms| {
                let all = Some(vec![0, 1, 2]);
                let r = terms.add(Kind::Rel(fx.ts, all.clone()), set(fx.row(all)));
                (r, Vec::new(), r)
            },
        ),
        (
            "constructor set [1, 0] is not sorted and distinct",
            |fx, terms| {
                let s = Some(vec![1, 0]);
                let r = terms.add(Kind::Rel(fx.ts, s.clone()), set(fx.row(s)));
                (r, Vec::new(), r)
            },
        ),
        (
            "constructor set [0, 5] names a constructor past 3",
            |fx, terms| {
                let s = Some(vec![0, 5]);
                let r = terms.add(Kind::Rel(fx.ts, s.clone()), set(fx.row(s)));
                (r, Vec::new(), r)
            },
        ),
        ("record type fields are not sorted by name", |_, terms| {
            let ty = set(Ty::Record(vec![("b".into(), int()), ("a".into(), int())]));
            let e = terms.add(Kind::EmptySet, ty);
            (e, Vec::new(), e)
        }),
        ("the relation is `Set I64`, not `Set T`", |fx, terms| {
            let r = terms.add(Kind::Rel(fx.ts, None), set(int()));
            (r, Vec::new(), r)
        }),
        ("the condition is not `Bool`", |_, terms| {
            let c = one(terms);
            let t = terms.add(Kind::If(c, c, c), int());
            (t, Vec::new(), t)
        }),
        (
            "the then branch is `Text`, not a subtype of `I64`",
            |fx, terms| {
                let b = terms.var("b", fx.prelude("Bool"));
                let test = terms.add(Kind::Var(b), fx.prelude("Bool"));
                let (w, c) = (word(terms), one(terms));
                let t = terms.add(Kind::If(test, w, c), int());
                (t, vec![b], t)
            },
        ),
        (
            "the bound value is `Text`, not a subtype of `I64`",
            |_, terms| {
                let x = terms.var("x", int());
                let w = word(terms);
                let read = terms.add(Kind::Var(x), int());
                let t = terms.add(Kind::Let(x, w, read), int());
                (t, Vec::new(), t)
            },
        ),
        (
            "the lambda is `I64 -> I64`, not a subtype of `Text -> I64`",
            |_, terms| {
                let x = terms.var("x", int());
                let read = terms.add(Kind::Var(x), int());
                let ty = Ty::Fn(Box::new(text()), Box::new(int()));
                let t = terms.add(Kind::Lam(x, read), ty);
                (t, Vec::new(), t)
            },
        ),
        ("the applied term is not a function", |_, terms| {
            let c = one(terms);
            let t = terms.add(Kind::App(c, c), int());
            (t, Vec::new(), t)
        }),
        ("`f` takes 1 arguments, given 0", |fx, terms| {
            let call = Kind::Call {
                func: fx.f,
                type_args: Vec::new(),
                args: Vec::new(),
            };
            let t = terms.add(call, int());
            (t, Vec::new(), t)
        }),
    ]
}

/// Returns the refusals of records, fields, constructors and cases.
fn data_refusals() -> Vec<(&'static str, Build)> {
    vec![
        ("the record's fields are not sorted by name", |_, terms| {
            let c = one(terms);
            let ty = Ty::Record(vec![("a".into(), int()), ("b".into(), int())]);
            let t = terms.add(Kind::Record(vec![("b".into(), c), ("a".into(), c)]), ty);
            (t, Vec::new(), t)
        }),
        ("a record is not typed by a record", |_, terms| {
            let t = terms.add(Kind::Record(Vec::new()), int());
            (t, Vec::new(), t)
        }),
        ("the record's labels are not its type's", |_, terms| {
            let c = one(terms);
            let ty = Ty::Record(vec![("b".into(), int())]);
            let t = terms.add(Kind::Record(vec![("a".into(), c)]), ty);
            (t, Vec::new(), t)
        }),
        (
            "`T` has no field `x` every constructor holds",
            |fx, terms| {
                let r = terms.var("r", fx.row(None));
                let read = terms.add(Kind::Var(r), fx.row(None));
                let t = terms.add(Kind::Field(read, "x".into()), int());
                (t, vec![r], t)
            },
        ),
        (
            "constructor 2 is not one of the node's type",
            |fx, terms| {
                let ctor = Kind::Ctor {
                    ctor: 2,
                    fields: Vec::new(),
                };
                let t = terms.add(ctor, fx.row(Some(vec![0, 1])));
                (t, Vec::new(), t)
            },
        ),
        (
            "the constructor's fields are not its declaration's",
            |fx, terms| {
                let ctor = Kind::Ctor {
                    ctor: 0,
                    fields: Vec::new(),
                };
                let t = terms.add(ctor, fx.row(None));
                (t, Vec::new(), t)
            },
        ),
        ("a constructor is not typed by a sum", |_, terms| {
            let ctor = Kind::Ctor {
                ctor: 0,
                fields: Vec::new(),
            };
            let t = terms.add(ctor, int());
            (t, Vec::new(), t)
        }),
        (
            "the arms are [1, 0, 2], the scrutinee's constructors [0, 1, 2]",
            |fx, terms| {
                let r = terms.var("r", fx.row(None));
                let read = terms.add(Kind::Var(r), fx.row(None));
                let c = one(terms);
                let arms = [1, 0, 2]
                    .into_iter()
                    .map(|k| (k, terms.var("a", fx.row(Some(vec![k]))), c))
                    .collect();
                let t = terms.add(
                    Kind::Case {
                        scrutinee: read,
                        arms,
                    },
                    int(),
                );
                (t, vec![r], t)
            },
        ),
        ("an arm's variable is `T`, not `T.A`", |fx, terms| {
            let r = terms.var("r", fx.row(None));
            let read = terms.add(Kind::Var(r), fx.row(None));
            let c = one(terms);
            let arms = [0, 1, 2]
                .into_iter()
                .map(|k| (k, terms.var("a", fx.row(None)), c))
                .collect();
            let t = terms.add(
                Kind::Case {
                    scrutinee: read,
                    arms,
                },
                int(),
            );
            (t, vec![r], t)
        }),
        ("the scrutinee is not a sum", |_, terms| {
            let c = one(terms);
            let scrutinee = Kind::Case {
                scrutinee: c,
                arms: Vec::new(),
            };
            let t = terms.add(scrutinee, int());
            (t, Vec::new(), t)
        }),
    ]
}

/// Returns the refusals of constants, primitives, sets, keys and writes.
fn value_refusals() -> Vec<(&'static str, Build)> {
    vec![
        ("the constant is typed `I64`", |_, terms| {
            let t = terms.add(Kind::Const(Const::Text("w".into())), int());
            (t, Vec::new(), t)
        }),
        ("`Add` takes 2 arguments, given 1", |_, terms| {
            let c = one(terms);
            let t = terms.add(Kind::Prim(Prim::Add, vec![c]), int());
            (t, Vec::new(), t)
        }),
        ("a comparison is not `Bool`", |_, terms| {
            let c = one(terms);
            let t = terms.add(Kind::Prim(Prim::Eq, vec![c, c]), int());
            (t, Vec::new(), t)
        }),
        ("a set term is not a set", |_, terms| {
            let t = terms.add(Kind::EmptySet, int());
            (t, Vec::new(), t)
        }),
        (
            "the element is `Text`, not a subtype of `I64`",
            |_, terms| {
                let w = word(terms);
                let t = terms.add(Kind::Singleton(w), set(int()));
                (t, Vec::new(), t)
            },
        ),
        ("a union of a term that is not a set", |_, terms| {
            let c = one(terms);
            let t = terms.add(Kind::Union(c, c), set(int()));
            (t, Vec::new(), t)
        }),
        ("a generator over a term that is not a set", |_, terms| {
            let x = terms.var("x", int());
            let c = one(terms);
            let t = terms.add(Kind::For(x, c, c), set(int()));
            (t, Vec::new(), t)
        }),
        ("a test of a term that is not a set", |fx, terms| {
            let c = one(terms);
            let t = terms.add(Kind::IsEmpty(c), fx.prelude("Bool"));
            (t, Vec::new(), t)
        }),
        ("a set test is not `Bool`", |_, terms| {
            let e = terms.add(Kind::EmptySet, set(int()));
            let t = terms.add(Kind::Exists(e), int());
            (t, Vec::new(), t)
        }),
        ("an aggregate over a term that is not a set", |_, terms| {
            let x = terms.var("x", int());
            let c = one(terms);
            let agg = Kind::Agg {
                op: Agg::Count,
                var: x,
                source: c,
                body: c,
            };
            let t = terms.add(agg, int());
            (t, Vec::new(), t)
        }),
        ("the count is `Text`, not `I64`", |_, terms| {
            let x = terms.var("x", int());
            let w = word(terms);
            let e = terms.add(Kind::EmptySet, set(int()));
            let top = Kind::Top {
                count: w,
                keys: Vec::new(),
                var: x,
                source: e,
            };
            let t = terms.add(top, set(int()));
            (t, Vec::new(), t)
        }),
        ("the point is `I64`", |_, terms| {
            let c = one(terms);
            let at = Kind::At {
                instant: true,
                point: c,
                pins: None,
                body: c,
            };
            let t = terms.add(at, int());
            (t, Vec::new(), t)
        }),
        ("a restriction of a term that is not a set", |fx, terms| {
            let c = one(terms);
            let t = terms.add(Kind::Restrict(c, vec![0]), set(fx.row(Some(vec![0]))));
            (t, Vec::new(), t)
        }),
        ("a dereference of a term that is not a key", |fx, terms| {
            let c = one(terms);
            let deref = Kind::Deref {
                key: c,
                reference: false,
            };
            let t = terms.add(deref, fx.row(None));
            (t, Vec::new(), t)
        }),
        ("`key` of a term that is not a row", |fx, terms| {
            let c = one(terms);
            let ty = Ty::Key {
                rel: RelRef::Declared(fx.ts),
                ctors: None,
            };
            let t = terms.add(Kind::Key(c), ty);
            (t, Vec::new(), t)
        }),
        (
            "`tag` of a term that is not of a declared sum",
            |fx, terms| {
                let c = one(terms);
                let t = terms.add(Kind::Tag(c), Ty::Tag(Box::new(fx.row(None))));
                (t, Vec::new(), t)
            },
        ),
        ("`try` is not a `Result`", |_, terms| {
            let c = one(terms);
            let t = terms.add(Kind::Try(c), int());
            (t, Vec::new(), t)
        }),
        ("the write is `I64`, not `a write`", |_, terms| {
            let t = terms.add(Kind::Skip, int());
            (t, Vec::new(), t)
        }),
        ("the failure's text is `I64`, not `Text`", |_, terms| {
            let c = one(terms);
            let t = terms.add(Kind::Fail(c), Ty::Change);
            (t, Vec::new(), t)
        }),
    ]
}

fn key(rel: RelId, ctors: Option<CtorSet>) -> Ty {
    Ty::Key {
        rel: RelRef::Declared(rel),
        ctors,
    }
}

/// Returns a variable of `T` at `ctors` and a read of it.
fn row_var(fx: &Fixture, terms: &mut Terms, ctors: Option<CtorSet>) -> (Var, Tm) {
    let r = terms.var("r", fx.row(ctors.clone()));
    let read = terms.add(Kind::Var(r), fx.row(ctors));
    (r, read)
}

/// Returns the refusals of calls, keys of values, generated values, `try` and empty sets.
fn kind_refusals() -> Vec<(&'static str, Build)> {
    vec![
        (
            "argument `n` is `Text`, not a subtype of `I64`",
            |fx, terms| {
                let w = word(terms);
                let call = Kind::Call {
                    func: fx.f,
                    type_args: Vec::new(),
                    args: vec![w],
                };
                let t = terms.add(call, int());
                (t, Vec::new(), t)
            },
        ),
        ("`caller` is `I64`, not `Principal`", |_, terms| {
            let t = terms.add(Kind::Caller, int());
            (t, Vec::new(), t)
        }),
        (
            "the key value is `Text`, not a subtype of `I64`",
            |fx, terms| {
                let w = word(terms);
                let t = terms.add(Kind::KeyOf(fx.ts, w), key(fx.ts, None));
                (t, Vec::new(), t)
            },
        ),
        ("a clock value is `I64`, not `Instant`", |fx, terms| {
            let t = terms.add(Kind::Fresh(Generator::Clock, fx.ts), int());
            (t, Vec::new(), t)
        }),
        ("a generated key is `Key us`", |fx, terms| {
            let fresh = Kind::Fresh(Generator::Sequence, fx.ts);
            let t = terms.add(fresh, key(fx.us, None));
            (t, Vec::new(), t)
        }),
        ("a clock value is `I64`, not `Instant`", |fx, terms| {
            let placeholder = Kind::Placeholder {
                occurrence: 0,
                generator: Generator::Clock,
                rel: fx.ts,
                scope: Vec::new(),
            };
            let t = terms.add(placeholder, int());
            (t, Vec::new(), t)
        }),
        ("a generated key is `Key us`", |fx, terms| {
            let placeholder = Kind::Placeholder {
                occurrence: 0,
                generator: Generator::Sequence,
                rel: fx.ts,
                scope: Vec::new(),
            };
            let t = terms.add(placeholder, key(fx.us, None));
            (t, Vec::new(), t)
        }),
        (
            "a placeholder's scope holds a term that is not a variable",
            |fx, terms| {
                let c = one(terms);
                let placeholder = Kind::Placeholder {
                    occurrence: 0,
                    generator: Generator::Sequence,
                    rel: fx.ts,
                    scope: vec![c],
                };
                let t = terms.add(placeholder, key(fx.ts, None));
                (t, Vec::new(), t)
            },
        ),
        ("an `Only` is never produced", |_, terms| {
            let e = terms.add(Kind::EmptySet, set(int()));
            let t = terms.add(Kind::Only(e), int());
            (t, Vec::new(), t)
        }),
        (
            "the failure of `try` is `Text`, not `Fault`",
            |fx, terms| {
                let c = one(terms);
                let result = Ty::Data {
                    def: fx.schema.prelude_type("Result"),
                    args: vec![int(), text()],
                    ctors: None,
                };
                let t = terms.add(Kind::Try(c), result);
                (t, Vec::new(), t)
            },
        ),
        ("an empty set is typed `Set⁺`", |_, terms| {
            let t = terms.add(Kind::EmptySet, Ty::NonEmpty(Box::new(int())));
            (t, Vec::new(), t)
        }),
    ]
}

/// Returns the refusals of groupings.
fn group_refusals() -> Vec<(&'static str, Build)> {
    vec![
        ("a grouping of a term that is not a set", |_, terms| {
            let x = terms.var("x", int());
            let c = one(terms);
            let group = Kind::Group {
                var: x,
                source: c,
                by: c,
            };
            let ty = set(Ty::Record(vec![
                ("key".into(), int()),
                ("rows".into(), Ty::NonEmpty(Box::new(int()))),
            ]));
            let t = terms.add(group, ty);
            (t, Vec::new(), t)
        }),
        (
            "the grouping's element is `Text`, not a subtype of `I64`",
            |_, terms| {
                let x = terms.var("x", int());
                let c = one(terms);
                let source = terms.add(Kind::EmptySet, set(text()));
                let group = Kind::Group {
                    var: x,
                    source,
                    by: c,
                };
                let ty = set(Ty::Record(vec![
                    ("key".into(), int()),
                    ("rows".into(), Ty::NonEmpty(Box::new(int()))),
                ]));
                let t = terms.add(group, ty);
                (t, Vec::new(), t)
            },
        ),
        (
            "the grouping is not a `Set` of `key` and `rows` records",
            |_, terms| {
                let x = terms.var("x", int());
                let c = one(terms);
                let source = terms.add(Kind::EmptySet, set(int()));
                let group = Kind::Group {
                    var: x,
                    source,
                    by: c,
                };
                let t = terms.add(group, set(int()));
                (t, Vec::new(), t)
            },
        ),
        (
            "the grouping is not a `Set` of `key` and `rows` records",
            |_, terms| {
                let x = terms.var("x", int());
                let c = one(terms);
                let source = terms.add(Kind::EmptySet, set(int()));
                let group = Kind::Group {
                    var: x,
                    source,
                    by: c,
                };
                let ty = Ty::NonEmpty(Box::new(Ty::Record(vec![
                    ("key".into(), int()),
                    ("rows".into(), Ty::NonEmpty(Box::new(int()))),
                ])));
                let t = terms.add(group, ty);
                (t, Vec::new(), t)
            },
        ),
        (
            "the grouping's key is `Text`, not a subtype of `I64`",
            |_, terms| {
                let x = terms.var("x", int());
                let by = word(terms);
                let source = terms.add(Kind::EmptySet, set(int()));
                let group = Kind::Group { var: x, source, by };
                let ty = set(Ty::Record(vec![
                    ("key".into(), int()),
                    ("rows".into(), Ty::NonEmpty(Box::new(int()))),
                ]));
                let t = terms.add(group, ty);
                (t, Vec::new(), t)
            },
        ),
        ("the grouping's rows is `Text`, not `I64`", |_, terms| {
            let x = terms.var("x", int());
            let c = one(terms);
            let source = terms.add(Kind::EmptySet, set(int()));
            let group = Kind::Group {
                var: x,
                source,
                by: c,
            };
            let ty = set(Ty::Record(vec![
                ("key".into(), int()),
                ("rows".into(), Ty::NonEmpty(Box::new(text()))),
            ]));
            let t = terms.add(group, ty);
            (t, Vec::new(), t)
        }),
    ]
}

/// Returns the refusals of tags, keys, aggregates, constructors and self-key fields.
fn shape_refusals() -> Vec<(&'static str, Build)> {
    vec![
        ("`tag` of `T` is `Tag Bool`", |fx, terms| {
            let (r, read) = row_var(fx, terms, None);
            let t = terms.add(Kind::Tag(read), Ty::Tag(Box::new(fx.prelude("Bool"))));
            (t, vec![r], t)
        }),
        (
            "the key's relation does not hold rows of `T`",
            |fx, terms| {
                let (r, read) = row_var(fx, terms, None);
                let t = terms.add(Kind::Key(read), key(fx.us, None));
                (t, vec![r], t)
            },
        ),
        (
            "the aggregate is `Option I64`, not a subtype of `I64`",
            |_, terms| {
                let x = terms.var("x", int());
                let source = terms.add(Kind::EmptySet, set(int()));
                let body = terms.add(Kind::Var(x), int());
                let agg = Kind::Agg {
                    op: Agg::Max,
                    var: x,
                    source,
                    body,
                };
                let t = terms.add(agg, int());
                (t, Vec::new(), t)
            },
        ),
        ("the sum has no constructor 1", |_, terms| {
            let sum = Ty::Sum {
                ctors: vec![("P".into(), vec![("a".into(), int())])],
                sel: None,
            };
            let ctor = Kind::Ctor {
                ctor: 1,
                fields: Vec::new(),
            };
            let t = terms.add(ctor, sum);
            (t, Vec::new(), t)
        }),
        ("field `a` is `Text`, not a subtype of `I64`", |_, terms| {
            let sum = Ty::Sum {
                ctors: vec![("P".into(), vec![("a".into(), int())])],
                sel: None,
            };
            let w = word(terms);
            let ctor = Kind::Ctor {
                ctor: 0,
                fields: vec![("a".into(), w)],
            };
            let t = terms.add(ctor, sum);
            (t, Vec::new(), t)
        }),
        (
            "constructor 5 is past the 3 of the tag's sum",
            |fx, terms| {
                let ctor = Kind::Ctor {
                    ctor: 5,
                    fields: Vec::new(),
                };
                let t = terms.add(ctor, Ty::Tag(Box::new(fx.row(None))));
                (t, Vec::new(), t)
            },
        ),
        ("a tag constructor is not of a declared sum", |_, terms| {
            let ctor = Kind::Ctor {
                ctor: 0,
                fields: Vec::new(),
            };
            let t = terms.add(ctor, Ty::Tag(Box::new(int())));
            (t, Vec::new(), t)
        }),
        (
            "field `id` is `Key ts.A`, not a subtype of `Key ts.B`",
            |fx, terms| {
                let (r, read) = row_var(fx, terms, Some(vec![0]));
                let field = Kind::Field(read, "id".into());
                let t = terms.add(field, key(fx.ts, Some(vec![1])));
                (t, vec![r], t)
            },
        ),
    ]
}

/// Returns the refusals of writes and blocks.
fn write_refusals() -> Vec<(&'static str, Build)> {
    vec![
        ("the rows of a write are not a set", |fx, terms| {
            let c = one(terms);
            let t = terms.add(Kind::Insert(fx.ts, c), Ty::Change);
            (t, Vec::new(), t)
        }),
        (
            "a row of the write is `I64`, not a subtype of `T`",
            |fx, terms| {
                let e = terms.add(Kind::EmptySet, set(int()));
                let t = terms.add(Kind::Delete(fx.ts, e), Ty::Change);
                (t, Vec::new(), t)
            },
        ),
        (
            "the new value of a `Replace` is not a function",
            |fx, terms| {
                let rows = terms.add(Kind::Rel(fx.ts, None), set(fx.row(None)));
                let c = one(terms);
                let t = terms.add(Kind::Replace(fx.ts, rows, c), Ty::Change);
                (t, Vec::new(), t)
            },
        ),
        (
            "a replaced row is `T`, not a subtype of `T.A`",
            |fx, terms| {
                let rows = terms.add(Kind::Rel(fx.ts, None), set(fx.row(None)));
                let (r, read) = row_var(fx, terms, Some(vec![0]));
                let ty = Ty::Fn(Box::new(fx.row(Some(vec![0]))), Box::new(fx.row(None)));
                let f = terms.add(Kind::Lam(r, read), ty);
                let t = terms.add(Kind::Replace(fx.ts, rows, f), Ty::Change);
                (t, Vec::new(), t)
            },
        ),
        ("a new value is `I64`, not a subtype of `T`", |fx, terms| {
            let rows = terms.add(Kind::Rel(fx.ts, None), set(fx.row(None)));
            let r = terms.var("r", fx.row(None));
            let c = one(terms);
            let f = terms.add(
                Kind::Lam(r, c),
                Ty::Fn(Box::new(fx.row(None)), Box::new(int())),
            );
            let t = terms.add(Kind::Replace(fx.ts, rows, f), Ty::Change);
            (t, Vec::new(), t)
        }),
        (
            "a proposed row is `T`, not a subtype of `T.A`",
            |fx, terms| {
                let rows = terms.add(Kind::EmptySet, set(fx.row(None)));
                let old = terms.var("old", fx.row(None));
                let new = terms.var("new", fx.row(Some(vec![0])));
                let otherwise = terms.add(Kind::Skip, Ty::Change);
                let upsert = Kind::Upsert {
                    rel: fx.ts,
                    rows,
                    on: Conflict::Key,
                    old,
                    new,
                    otherwise,
                    single: false,
                };
                let t = terms.add(upsert, Ty::Change);
                (t, Vec::new(), t)
            },
        ),
        (
            "the conflicting row is `I64`, not a subtype of `T`",
            |fx, terms| {
                let rows = terms.add(Kind::EmptySet, set(fx.row(None)));
                let old = terms.var("old", int());
                let new = terms.var("new", fx.row(None));
                let otherwise = terms.add(Kind::Skip, Ty::Change);
                let upsert = Kind::Upsert {
                    rel: fx.ts,
                    rows,
                    on: Conflict::Key,
                    old,
                    new,
                    otherwise,
                    single: false,
                };
                let t = terms.add(upsert, Ty::Change);
                (t, Vec::new(), t)
            },
        ),
        (
            "the conflict branch is `I64`, not `a write`",
            |fx, terms| {
                let rows = terms.add(Kind::EmptySet, set(fx.row(None)));
                let old = terms.var("old", fx.row(None));
                let new = terms.var("new", fx.row(None));
                let otherwise = one(terms);
                let upsert = Kind::Upsert {
                    rel: fx.ts,
                    rows,
                    on: Conflict::Key,
                    old,
                    new,
                    otherwise,
                    single: false,
                };
                let t = terms.add(upsert, Ty::Change);
                (t, Vec::new(), t)
            },
        ),
        (
            "a `LetBang` binds a write other than `Replace`, `Upsert` or `Rejects`",
            |_, terms| {
                let x = terms.var("x", int());
                let skip = terms.add(Kind::Skip, Ty::Change);
                let c = one(terms);
                let t = terms.add(Kind::LetBang(x, skip, c), int());
                (t, Vec::new(), t)
            },
        ),
    ]
}

#[test]
fn validate_refuses_a_broken_child_at_the_child_under_a_parent_that_holds() {
    let fx = Fixture::new();
    let mut terms = Terms::default();
    let child = terms.add(Kind::Const(Const::Text("w".into())), int());
    let parent = terms.add(Kind::Singleton(child), set(int()));
    let got = validate(&fx.schema, &terms, &[(parent, Vec::new())])
        .map_err(|e| (e.at(), e.reason().to_string()));
    assert_eq!(Err((child, "the constant is typed `I64`".to_string())), got);
}

#[test]
fn validate_accepts_a_self_key_field_at_the_key_of_the_rows_constructor_set() {
    let fx = Fixture::new();
    let mut terms = Terms::default();
    let (r, read) = row_var(&fx, &mut terms, Some(vec![0]));
    let field = terms.add(Kind::Field(read, "id".into()), key(fx.ts, Some(vec![0])));
    assert_eq!(Ok(()), validate(&fx.schema, &terms, &[(field, vec![r])]));
}

#[test]
fn validate_refuses_each_broken_invariant_at_the_node_that_breaks_it() {
    let fx = Fixture::new();
    let cases = [
        binding_refusals(),
        data_refusals(),
        value_refusals(),
        kind_refusals(),
        group_refusals(),
        shape_refusals(),
        write_refusals(),
    ]
    .concat();
    let mut differ = Vec::new();
    for (reason, build) in &cases {
        let mut terms = Terms::default();
        let (root, params, at) = build(&fx, &mut terms);
        let want = Err((at, reason.to_string()));
        let got = validate(&fx.schema, &terms, &[(root, params)])
            .map_err(|e| (e.at(), e.reason().to_string()));
        if got != want {
            differ.push(format!("{reason}: {got:?}"));
        }
    }
    assert_eq!(Vec::<String>::new(), differ);
    assert_eq!(76, cases.len());
}

#[test]
fn validate_accepts_a_parameter_read_at_its_type_and_a_lambda_at_a_wider_one() {
    let fx = Fixture::new();
    let mut terms = Terms::default();
    let x = terms.var("x", fx.row(Some(vec![0])));
    let read = terms.add(Kind::Var(x), fx.row(Some(vec![0])));
    let ty = Ty::Fn(Box::new(fx.row(Some(vec![0]))), Box::new(fx.row(None)));
    let lam = terms.add(Kind::Lam(x, read), ty);
    let y = terms.var("y", int());
    let param = terms.add(Kind::Var(y), int());
    let roots = [(lam, Vec::new()), (param, vec![y])];
    assert_eq!(Ok(()), validate(&fx.schema, &terms, &roots));
}

#[test]
fn validate_accepts_a_grouping_at_a_wider_key_than_its_by() {
    let fx = Fixture::new();
    let mut terms = Terms::default();
    let (r, read) = row_var(&fx, &mut terms, Some(vec![0]));
    let by = terms.add(Kind::Field(read, "id".into()), key(fx.ts, Some(vec![0])));
    let source = terms.add(Kind::EmptySet, set(int()));
    let x = terms.var("x", int());
    let group = Kind::Group { var: x, source, by };
    let ty = set(Ty::Record(vec![
        ("key".into(), key(fx.ts, None)),
        ("rows".into(), Ty::NonEmpty(Box::new(int()))),
    ]));
    let t = terms.add(group, ty);
    assert_eq!(Ok(()), validate(&fx.schema, &terms, &[(t, vec![r])]));
}

#[test]
fn validate_accepts_a_min_over_a_nonempty_source_at_its_body_type_or_an_option_of_it() {
    let fx = Fixture::new();
    let mut terms = Terms::default();
    let option = Ty::Data {
        def: fx.schema.prelude_type("Option"),
        args: vec![int()],
        ctors: None,
    };
    let mut roots = Vec::new();
    for ty in [int(), option] {
        let x = terms.var("x", int());
        let r = terms.var("s", Ty::NonEmpty(Box::new(int())));
        let source = terms.add(Kind::Var(r), Ty::NonEmpty(Box::new(int())));
        let body = terms.add(Kind::Var(x), int());
        let agg = Kind::Agg {
            op: Agg::Min,
            var: x,
            source,
            body,
        };
        roots.push((terms.add(agg, ty), vec![r]));
    }
    assert_eq!(Ok(()), validate(&fx.schema, &terms, &roots));
}

#[test]
fn validate_normal_refuses_each_term_the_normal_form_excludes() {
    let fx = Fixture::new();
    let normal: [(&str, Build); 5] = [
        ("a `Lam` remains", |_, terms| {
            let x = terms.var("x", int());
            let read = terms.add(Kind::Var(x), int());
            let ty = Ty::Fn(Box::new(int()), Box::new(int()));
            let t = terms.add(Kind::Lam(x, read), ty);
            (t, Vec::new(), t)
        }),
        ("an `App` remains", |_, terms| {
            let x = terms.var("x", int());
            let read = terms.add(Kind::Var(x), int());
            let ty = Ty::Fn(Box::new(int()), Box::new(int()));
            let lam = terms.add(Kind::Lam(x, read), ty);
            let c = one(terms);
            let t = terms.add(Kind::App(lam, c), int());
            (t, Vec::new(), t)
        }),
        ("a call of a schema function remains", |fx, terms| {
            let c = one(terms);
            let call = Kind::Call {
                func: fx.f,
                type_args: Vec::new(),
                args: vec![c],
            };
            let t = terms.add(call, int());
            (t, Vec::new(), t)
        }),
        ("an `Only` remains", |_, terms| {
            let e = terms.add(Kind::EmptySet, set(int()));
            let t = terms.add(Kind::Only(e), int());
            (t, Vec::new(), t)
        }),
        (
            "the new value of a `Replace` is not a `Lam`",
            |fx, terms| {
                let rows = terms.add(Kind::Rel(fx.ts, None), set(fx.row(None)));
                let r = terms.var("r", fx.row(None));
                let read = terms.add(Kind::Var(r), fx.row(None));
                let t = terms.add(Kind::Replace(fx.ts, rows, read), Ty::Change);
                (t, vec![r], t)
            },
        ),
    ];
    let mut differ = Vec::new();
    for (reason, build) in &normal {
        let mut terms = Terms::default();
        let (root, params, at) = build(&fx, &mut terms);
        let want = Err((at, reason.to_string()));
        let got = validate_normal(&fx.schema, &terms, &[(root, params)])
            .map_err(|e| (e.at(), e.reason().to_string()));
        if got != want {
            differ.push(format!("{reason}: {got:?}"));
        }
    }
    assert_eq!(Vec::<String>::new(), differ);
    assert_eq!(5, normal.len());
}

#[test]
fn validate_normal_accepts_the_new_value_function_of_a_replace() {
    let fx = Fixture::new();
    let mut terms = Terms::default();
    let rows = terms.add(Kind::Rel(fx.ts, None), set(fx.row(None)));
    let r = terms.var("r", fx.row(None));
    let read = terms.add(Kind::Var(r), fx.row(None));
    let ty = Ty::Fn(Box::new(fx.row(None)), Box::new(fx.row(None)));
    let lam = terms.add(Kind::Lam(r, read), ty);
    let t = terms.add(Kind::Replace(fx.ts, rows, lam), Ty::Change);
    let roots = [(t, Vec::new())];
    assert_eq!(Ok(()), validate_normal(&fx.schema, &terms, &roots));
    assert_eq!(Ok(()), validate(&fx.schema, &terms, &roots));
}
