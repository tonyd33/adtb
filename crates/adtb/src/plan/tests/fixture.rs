use std::collections::BTreeMap;

use adtb_lang::ast::AdhocItem;
use adtb_lang::ast::{BundleItem, ExampleItem};
use adtb_lang::check::check_query;
use adtb_lang::check::{Checker, Context, Program, check_adhoc, check_schema};
use adtb_lang::kernel::{Kind, Tm};
use adtb_lang::norm::{self, NormalAdhoc, NormalProgram, NormalQuery};
use adtb_lang::parse::{Options, parse_adhoc, parse_example, parse_schema, parse_type};
use adtb_lang::resolve::Visible;
use adtb_lang::schema::{Schema, ValueDef, build};
use adtb_lang::span::FileId;
use adtb_lang::types::{RelId, Ty};
use adtb_reference::{Database, Interpreter, R, Value};
use adtb_testkit::uuid;

use crate::plan::{Catalog, HeapId, IndexDesc, IndexId, RelationDesc, Rewrites};

pub struct Fixture {
    pub schema: Schema,
    pub program: Program,
    pub normal: NormalProgram,
    pub catalog: Catalog,
    pub db: Database,
}

impl Fixture {
    pub fn new(src: &str) -> Fixture {
        let text = format!("module app\n{src}");
        let file = parse_schema(&text, Options::default())
            .ok()
            .unwrap_or_else(|ds| panic!("the schema does not parse: {ds:?}"));
        let schema = build(&[(FileId(0), file)])
            .unwrap_or_else(|ds| panic!("the schema does not build: {ds:?}"));
        let program =
            check_schema(&schema).unwrap_or_else(|ds| panic!("the schema does not check: {ds:?}"));
        let normal = norm::program(&schema, &program).expect("the schema's bodies normalise");
        let catalog = catalog(&schema, &program);
        Fixture {
            schema,
            program,
            normal,
            catalog,
            db: Database::new(),
        }
    }

    pub fn rel(&self, name: &str) -> RelId {
        let m = self.schema.module_named("app").expect("the module");
        match self.schema.module(m).values.get(name) {
            Some(&ValueDef::Relation(r)) => r,
            _ => panic!("no relation `{name}`"),
        }
    }

    pub fn ty(&self, text: &str) -> Ty {
        let parsed = parse_type(text, Options::default())
            .ok()
            .unwrap_or_else(|_| panic!("`{text}` is not a type"));
        Checker::new(&self.schema, Visible::all(&self.schema), Context::Adhoc)
            .resolve_type(&parsed)
            .unwrap_or_else(|d| panic!("`{text}`: {}", d.message))
    }

    /// Returns the normal form of the one statement of `text`, each parameter given as name and type.
    pub fn normal(&self, text: &str, params: &[(&str, &str)]) -> (NormalAdhoc, Tm) {
        let items: Vec<AdhocItem> = parse_adhoc(text, Options::default())
            .ok()
            .unwrap_or_else(|ds| panic!("`{text}` does not parse: {ds:?}"));
        let tys: BTreeMap<String, Ty> = params
            .iter()
            .map(|(n, t)| (n.to_string(), self.ty(t)))
            .collect();
        let adhoc = check_adhoc(&self.schema, &self.program, &items, &tys)
            .unwrap_or_else(|ds| panic!("`{text}` does not check: {ds:?}"));
        let normal = norm::adhoc(&self.schema, &self.program, &adhoc)
            .unwrap_or_else(|e| panic!("`{text}` has no normal form: {e:?}"));
        let [item] = normal.items.as_slice() else {
            panic!("`{text}` holds one statement")
        };
        let item = *item;
        (normal, item)
    }

    /// Returns the normal form of the registered query `decl`.
    pub fn query(&self, decl: &str) -> NormalQuery {
        let items = parse_example(decl, Options::default())
            .ok()
            .unwrap_or_else(|ds| panic!("`{decl}` does not parse: {ds:?}"));
        let [ExampleItem::BundleItem(BundleItem::Query(q))] = items.as_slice() else {
            panic!("`{decl}` is one query")
        };
        let module = self.schema.module_named("app").expect("the module");
        let query = check_query(&self.schema, &self.program, module, q)
            .unwrap_or_else(|d| panic!("`{decl}` does not check: {}", d.message));
        norm::query(&self.schema, &self.program, &query)
            .unwrap_or_else(|e| panic!("`{decl}` has no normal form: {e:?}"))
    }

    /// Returns the value of `text` over `params`, each given as name, type and value.
    pub fn reference(&self, text: &str, params: &[(&str, &str, Value)]) -> R<Value> {
        let types: Vec<(&str, &str)> = params.iter().map(|(n, t, _)| (*n, *t)).collect();
        let (n, t) = self.normal(text, &types);
        let bound: Vec<_> = params
            .iter()
            .map(|(name, _, v)| (n.params[*name], v.clone()))
            .collect();
        Interpreter::new(&self.schema, &self.program, &self.db).eval(&n.terms, t, &bound)
    }

    /// Adds to relation `rel` the rows the set expression `rows` denotes over `params`.
    pub fn rows(&mut self, rel: &str, rows: &str, params: &[(&str, &str, Value)]) {
        let r = self.rel(rel);
        let Ok(Value::Set(s)) = self.reference(rows, params) else {
            panic!("`{rows}` is not a set")
        };
        for row in s {
            let c = row.tag().expect("a row is a constructor value");
            self.db.entry((r, c)).or_default().insert(row);
        }
    }
}

/// Returns a catalog with a heap per constructor, the key index, every index of the schema whose fields are fields of the row, and every implied index.
pub fn catalog(schema: &Schema, program: &Program) -> Catalog {
    let normal = norm::program(schema, program).expect("the schema's bodies normalise");
    let mut out = Catalog::default();
    for (i, r) in schema.relations.iter().enumerate() {
        let rel = RelId(i as u32);
        let base = 100 * i as u64;
        let n = schema.ty(r.elem_def).ctors().len() as u32;
        let all: Vec<u32> = (0..n).collect();
        let ids = &schema.ids;
        let key_index = IndexId(ids.keys[&rel]);
        let mut indexes = vec![IndexDesc {
            id: key_index,
            ctors: all.clone(),
            fields: r.key.clone(),
            unique: true,
            ordered: true,
        }];
        for (j, def) in r.indexes.iter().enumerate() {
            let mut fields = Vec::new();
            for k in 0..def.fields.len() {
                let (_, f) = &normal.index_fields[&(rel, j, k)];
                match normal.terms.kind(f.value) {
                    Kind::Field(x, name)
                        if f.path.is_empty() && matches!(normal.terms.kind(*x), Kind::Var(_)) =>
                    {
                        fields.push(name.clone());
                    }
                    _ => break,
                }
            }
            if fields.len() < def.fields.len() {
                continue;
            }
            indexes.push(IndexDesc {
                id: IndexId(ids.indexes[&(rel, j)]),
                ctors: def.scope.clone().unwrap_or_else(|| all.clone()),
                fields,
                unique: def.unique,
                ordered: true,
            });
        }
        for implied in schema.implied_indexes(rel) {
            let Some(&id) = ids.implied.get(&(rel, implied.ctor, implied.field.clone())) else {
                assert!(
                    schema.module(r.module).engine,
                    "every implied index of a user relation has an id"
                );
                continue;
            };
            indexes.push(IndexDesc {
                id: IndexId(id),
                ctors: implied.ctors,
                fields: vec![implied.field],
                unique: false,
                ordered: true,
            });
        }
        out.relations.insert(
            rel,
            RelationDesc {
                heaps: all
                    .iter()
                    .map(|c| (*c, HeapId(base + u64::from(*c))))
                    .collect(),
                key_index,
                indexes,
            },
        );
    }
    out
}

pub const SCHEMA: &str = r"
type User = { id: Key users, name: Text }
type Identity { id: Key identities, user: Key users }
  = EmailPass { email: Text, hash: Bytes }
  | Google    { gid: Text }
  | Apple     { sub: Text }
relation users : User key id generate sequence
relation identities : Identity
  key id generate uuidv7
  unique (email) in EmailPass
  unique (gid) in Google
  index (user)
set OAuth of identities = Google | Apple

type Post = { id: Key posts, author: Key users, created_at: Instant, title: Text }
type Phone = { id: Key phones, owner: Key users, seen: Instant }
type Follow = { src: Key users, dst: Key users }
type Order = { id: Key orders, user: Key users, total: I64, qty: I64 }
relation posts : Post key id generate sequence
  index (author)
relation phones : Phone key id generate sequence
relation follows : Follow key (src, dst)
relation orders : Order key id generate sequence
type Session = { id: Key sessions, owner: Option (Key users) }
relation sessions : Session key id generate sequence
";

pub fn running() -> Fixture {
    let mut f = Fixture::new(SCHEMA);
    let times: Vec<(&str, &str, Value)> = (1..=5)
        .map(|i| {
            (
                ["t1", "t2", "t3", "t4", "t5"][i - 1],
                "Instant",
                Value::Instant(10 * i as i64),
            )
        })
        .collect();
    f.rows(
        "users",
        r#"[User { id: users.key(1), name: "ann" },
            User { id: users.key(2), name: "bo" },
            User { id: users.key(3), name: "cy" }]"#,
        &[],
    );
    f.rows(
        "identities",
        &format!(
            r#"[EmailPass {{ id: {}, user: users.key(1), email: "ann@example.com", hash: bytes("h1") }},
                Google {{ id: {}, user: users.key(1), gid: "g-ann" }},
                Google {{ id: {}, user: users.key(2), gid: "root" }},
                Apple {{ id: {}, user: users.key(2), sub: "a-bo" }}]"#,
            uuid(1),
            uuid(2),
            uuid(3),
            uuid(4)
        ),
        &[],
    );
    f.rows(
        "posts",
        r#"[Post { id: posts.key(1), author: users.key(1), created_at: t1, title: "hello" },
            Post { id: posts.key(2), author: users.key(1), created_at: t2, title: "again" },
            Post { id: posts.key(3), author: users.key(2), created_at: t3, title: "bo's" },
            Post { id: posts.key(4), author: users.key(1), created_at: t4, title: "fourth" },
            Post { id: posts.key(5), author: users.key(1), created_at: t5, title: "fifth" }]"#,
        &times,
    );
    f.rows(
        "phones",
        r"[Phone { id: phones.key(1), owner: users.key(1), seen: t1 },
            Phone { id: phones.key(2), owner: users.key(1), seen: t2 }]",
        &times,
    );
    f.rows(
        "sessions",
        r"[Session { id: sessions.key(1), owner: Some { value: users.key(1) } },
            Session { id: sessions.key(2), owner: None }]",
        &[],
    );
    f.rows(
        "follows",
        r"[Follow { src: users.key(1), dst: users.key(2) },
            Follow { src: users.key(2), dst: users.key(3) },
            Follow { src: users.key(3), dst: users.key(2) }]",
        &[],
    );
    f.rows(
        "orders",
        r"[Order { id: orders.key(1), user: users.key(1), total: 100, qty: 2 },
            Order { id: orders.key(2), user: users.key(1), total: 100, qty: 0 },
            Order { id: orders.key(3), user: users.key(2), total: 30, qty: 3 },
            Order { id: orders.key(4), user: users.key(1), total: 50, qty: 5 }]",
        &[],
    );
    f
}

/// Returns every rewrite on, every rewrite off, and each rewrite turned off alone, by name.
pub fn variants() -> Vec<(&'static str, Rewrites)> {
    let all = Rewrites::ALL;
    vec![
        ("all", all),
        ("none", Rewrites::NONE),
        (
            "index_scan off",
            Rewrites {
                index_scan: false,
                ..all
            },
        ),
        (
            "index_join off",
            Rewrites {
                index_join: false,
                ..all
            },
        ),
        (
            "hash_join off",
            Rewrites {
                hash_join: false,
                ..all
            },
        ),
        (
            "fuse_probe off",
            Rewrites {
                fuse_probe: false,
                ..all
            },
        ),
        (
            "static_route off",
            Rewrites {
                static_route: false,
                ..all
            },
        ),
        (
            "prune off",
            Rewrites {
                prune: false,
                ..all
            },
        ),
        (
            "unswitch off",
            Rewrites {
                unswitch: false,
                ..all
            },
        ),
        (
            "group_agg off",
            Rewrites {
                group_agg: false,
                ..all
            },
        ),
        (
            "semi_naive off",
            Rewrites {
                semi_naive: false,
                ..all
            },
        ),
    ]
}
