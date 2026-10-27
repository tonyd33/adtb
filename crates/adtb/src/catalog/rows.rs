use std::collections::{BTreeMap, BTreeSet};

use adtb_lang::ast::{self, Decl, ExprKind, QName, RelationClause, RelationDecl};
use adtb_lang::evolve::{Code, Source};
use adtb_lang::kernel::PathStep;
use adtb_lang::lex::lex;
use adtb_lang::schema::{IndexDef, Schema, Stage, TypeDefKind, ValueDef};
use adtb_lang::span::Span;
use adtb_lang::types::{RelId, Ty, TypeId};

use crate::key::Value;
use crate::plan::{Element, declared_elements};

use super::{layout, oplog};

/// Rows of relations, each with its constructor index and as a heap of that constructor holds it.
pub type Rows = BTreeMap<RelId, Vec<(u32, Vec<Value>)>>;

/// Returns the rows `catalog::` relations hold for the objects of `schema`, built from `source`,
/// that have catalog ids, with every relation of `schema` present.
pub fn declared(schema: &Schema, source: &Source) -> Rows {
    let mut b = Builder {
        schema,
        source,
        rows: schema
            .ids
            .relations
            .keys()
            .map(|r| (*r, Vec::new()))
            .collect(),
    };
    b.types();
    b.ctors();
    b.fields();
    b.relations();
    b.sets();
    b.indexes();
    b.constraints();
    b.defaults();
    b.ctor_states();
    b.value_states();
    b.functions();
    b.log();
    b.rows
}

/// Returns the row of `catalog::migrations` for the last migration of `source`, applied at
/// `applied` microseconds since 1970-01-01T00:00:00Z, or no row if the log is empty.
pub fn applied(schema: &Schema, source: &Source, applied: i64) -> Rows {
    let mut b = Builder {
        schema,
        source,
        rows: Rows::new(),
    };
    if let Some(m) = source.log.last() {
        let fields = vec![
            ("version", key(m.number)),
            (
                "hash",
                Value::Bytes(oplog::version_hash(schema, source).to_vec()),
            ),
            ("text", Value::Text(m.text.clone())),
            ("applied", Value::Instant(applied)),
        ];
        b.push("migrations", "Migration", fields);
    }
    b.rows
}

/// One index of a relation, as the engine stores it.
#[derive(Clone, Debug)]
pub struct IndexSpec {
    pub id: u64,
    pub relation: RelId,
    pub unique: bool,
    /// Constructor indexes, in order.
    pub ctors: Vec<u32>,
    /// Each element as written: a field, a path through a field's value-level sum, as
    /// `login.Google.gid`, or an expression, as `lower(email)`.
    pub fields: Vec<String>,
    pub elements: Vec<Element>,
    kind: &'static str,
    /// The clause as the catalog writes it.
    pub clause: String,
}

/// Returns every index of the relations of `schema` with catalog ids, by index id: the key
/// indexes, the declared uniques and indexes, and the implied indexes.
pub fn indexes(schema: &Schema) -> BTreeMap<u64, IndexSpec> {
    let mut out = BTreeMap::new();
    for (&r, &id) in &schema.ids.keys {
        let rel = schema.relation(r);
        let key = if let [k] = rel.key.as_slice() {
            format!("key {k}")
        } else {
            format!("key ({})", rel.key.join(", "))
        };
        let spec = IndexSpec {
            id,
            relation: r,
            unique: true,
            ctors: all_ctors(schema, r),
            fields: rel.key.clone(),
            elements: rel.key.iter().cloned().map(Element::Field).collect(),
            kind: "PrimaryKey",
            clause: key,
        };
        out.insert(id, spec);
    }
    for (&(r, i), &id) in &schema.ids.indexes {
        let def = &schema.relation(r).indexes[i];
        let fields: Vec<String> = def
            .fields
            .iter()
            .map(|e| field_path(e).unwrap_or_else(|| expr_text(e)))
            .collect();
        let elements = declared_elements(i, def);
        let spec = IndexSpec {
            id,
            relation: r,
            unique: def.unique,
            ctors: def.scope.clone().unwrap_or_else(|| all_ctors(schema, r)),
            clause: clause(schema, r, def, &fields),
            fields,
            elements,
            kind: match (def.unique, def.interval) {
                (true, _) => "Unique",
                (false, true) => "Interval",
                (false, false) => "Plain",
            },
        };
        out.insert(id, spec);
    }
    for ((r, ctor, field), &id) in &schema.ids.implied {
        let ctors = ctor.map_or_else(|| all_ctors(schema, *r), |c| vec![c]);
        let clause = match ctor {
            None => format!("index ({field})"),
            Some(c) => {
                let td = schema.ty(schema.relation(*r).elem_def);
                format!("index ({field}) in {}", td.ctors()[*c as usize].name)
            }
        };
        let spec = IndexSpec {
            id,
            relation: *r,
            unique: false,
            ctors,
            fields: vec![field.clone()],
            elements: vec![Element::Field(field.clone())],
            kind: "Implied",
            clause,
        };
        out.insert(id, spec);
    }
    out
}

/// Returns index element `e`, an expression that is not a field path, as the catalog writes it.
fn expr_text(e: &ast::Expr) -> String {
    let name = |q: &QName| match &q.module {
        Some(m) => format!("{}::{}", m.text, q.name.text),
        None => q.name.text.clone(),
    };
    let nested = |x: &ast::Expr| match &x.kind {
        ExprKind::Binary(..) | ExprKind::If(..) => format!("({})", expr_text(x)),
        _ => expr_text(x),
    };
    match &e.kind {
        ExprKind::Lit(l) => match l {
            ast::Lit::Int(t) | ast::Lit::Decimal(t) | ast::Lit::Float(t) => t.clone(),
            ast::Lit::Text(t) => format!("{t:?}"),
            ast::Lit::Duration { amount, unit } => format!("{amount}{unit}"),
            ast::Lit::Bool(b) => b.to_string(),
        },
        ExprKind::Name(q) => name(q),
        ExprKind::Call {
            callee,
            args: ast::CallArgs::List(xs),
            pins: None,
        } => {
            let args: Vec<String> = xs.iter().map(expr_text).collect();
            format!("{}({})", name(callee), args.join(", "))
        }
        ExprKind::Unit => "()".into(),
        ExprKind::Unary(op, x) => {
            let op = match op {
                ast::UnOp::Neg => "-",
                ast::UnOp::Not => "not ",
                ast::UnOp::Key => "key ",
                ast::UnOp::Try => "try ",
            };
            format!("{op}{}", nested(x))
        }
        ExprKind::Binary(op, a, b) => {
            let op = match op {
                ast::BinOp::Union => "|",
                ast::BinOp::Or => "or",
                ast::BinOp::And => "and",
                ast::BinOp::Eq => "==",
                ast::BinOp::Ne => "!=",
                ast::BinOp::Lt => "<",
                ast::BinOp::Le => "<=",
                ast::BinOp::Gt => ">",
                ast::BinOp::Ge => ">=",
                ast::BinOp::Add => "+",
                ast::BinOp::Sub => "-",
                ast::BinOp::Concat => "++",
                ast::BinOp::Mul => "*",
                ast::BinOp::Div => "/",
                ast::BinOp::Rem => "%",
            };
            format!("{} {op} {}", nested(a), nested(b))
        }
        ExprKind::Field(x, f) => format!("{}.{}", nested(x), f.text),
        ExprKind::Select(x, ast::CtorSel::One(c)) => format!("{}.{}", nested(x), c.text),
        ExprKind::If(c, a, b) => format!(
            "if {} then {} else {}",
            expr_text(c),
            expr_text(a),
            expr_text(b)
        ),
        other => format!("{other:?}"),
    }
}

/// Returns the fields an index's clause names, or `None` if one of its expressions is not a
/// field.
pub fn indexed_fields(def: &IndexDef) -> Option<Vec<String>> {
    def.fields
        .iter()
        .map(|e| match &e.kind {
            ExprKind::Name(QName { module: None, name }) => Some(name.text.clone()),
            _ => None,
        })
        .collect()
}

/// Returns the field path `e` is, written `f.C.g`.
pub fn field_path(e: &ast::Expr) -> Option<String> {
    match &e.kind {
        ExprKind::Name(QName { module: None, name }) => Some(name.text.clone()),
        ExprKind::Field(x, f) => Some(format!("{}.{}", field_path(x)?, f.text)),
        ExprKind::Select(x, ast::CtorSel::One(c)) => Some(format!("{}.{}", field_path(x)?, c.text)),
        _ => None,
    }
}

/// One step of a field path through a row: a field by its position, or the constructor by its
/// position that the value the steps before it reach must hold.
#[derive(Clone, Copy, Debug)]
pub enum Step {
    Field(usize),
    Ctor(u32),
}

/// Returns the steps of field path `path` through a row of constructor `c` of `r`, and the type
/// of the value it reaches.
///
/// Panics if `path` names no field or constructor at a step.
pub fn path_steps(schema: &Schema, r: RelId, c: u32, path: &[PathStep]) -> (Vec<Step>, Ty) {
    let Some((PathStep::Field(first), rest)) = path.split_first() else {
        panic!("a path starts at a field: {path:?}");
    };
    let row = layout::row(schema, r, c);
    let at = row
        .iter()
        .position(|(n, _)| n == first)
        .unwrap_or_else(|| panic!("a row holds indexed field `{first}`"));
    let mut steps = vec![Step::Field(at)];
    let mut ty = row[at].1.clone();
    let mut fields: Option<Vec<(String, Ty)>> = None;
    for step in rest {
        let name = match step {
            PathStep::Ctor(k) => {
                steps.push(Step::Ctor(*k));
                fields = Some(
                    layout::ctors(schema, &layout::repr(schema, &ty))
                        .swap_remove(*k as usize)
                        .1,
                );
                continue;
            }
            PathStep::Field(name) => name,
        };
        let fs = match fields.take() {
            Some(fs) => fs,
            None => match layout::repr(schema, &ty) {
                Ty::Record(fs) => fs,
                t => layout::ctors(schema, &t).swap_remove(0).1,
            },
        };
        let at = fs
            .iter()
            .position(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("a value of path {path:?} has field `{name}`"));
        steps.push(Step::Field(at));
        ty = fs[at].1.clone();
    }
    (steps, ty)
}

/// Returns the value `steps` reach in `row`, or `None` when a value on the way holds another
/// constructor than a step names.
pub fn at_path<'v>(row: &'v [Value], steps: &[Step]) -> Option<&'v Value> {
    let (first, rest) = steps.split_first()?;
    let Step::Field(i) = first else {
        panic!("a path starts at a field");
    };
    let mut at = &row[*i];
    for step in rest {
        at = match (step, at) {
            (Step::Ctor(k), Value::Sum { ctor, .. }) => {
                if ctor != k {
                    return None;
                }
                at
            }
            (Step::Field(i), Value::Sum { fields, .. } | Value::Record(fields)) => &fields[*i],
            (s, v) => panic!("path step {s:?} meets {v:?}"),
        };
    }
    Some(at)
}

fn all_ctors(schema: &Schema, r: RelId) -> Vec<u32> {
    let n = schema.ty(schema.relation(r).elem_def).ctors().len();
    (0..n as u32).collect()
}

fn clause(schema: &Schema, r: RelId, def: &IndexDef, fields: &[String]) -> String {
    let verb = match (def.unique, def.interval) {
        (true, _) => "unique",
        (false, true) => "interval index",
        (false, false) => "index",
    };
    let mut out = format!("{verb} ({})", fields.join(", "));
    if let Some(scope) = &def.scope {
        let td = schema.ty(schema.relation(r).elem_def);
        let names: Vec<&str> = scope
            .iter()
            .map(|&c| td.ctors()[c as usize].name.as_str())
            .collect();
        match names.as_slice() {
            [one] => out.push_str(&format!(" in {one}")),
            many => out.push_str(&format!(" in ({})", many.join(" | "))),
        }
    }
    out
}

struct Builder<'s> {
    schema: &'s Schema,
    source: &'s Source,
    rows: Rows,
}

impl Builder<'_> {
    fn relation(&self, name: &str) -> RelId {
        let m = self
            .schema
            .module_named("catalog")
            .expect("the catalog module");
        match self.schema.module(m).values.get(name) {
            Some(&ValueDef::Relation(r)) => r,
            _ => panic!("the catalog declares relation `{name}`"),
        }
    }

    /// Add a row of constructor `ctor` of `catalog::{rel}`, its fields given by name.
    fn push(&mut self, rel: &str, ctor: &str, fields: Vec<(&str, Value)>) {
        let r = self.relation(rel);
        let td = self.schema.ty(self.schema.relation(r).elem_def);
        let c = td
            .ctor_index(ctor)
            .unwrap_or_else(|| panic!("`{rel}` has constructor `{ctor}`"));
        let names = layout::row(self.schema, r, c);
        assert_eq!(
            names.len(),
            fields.len(),
            "a row of `{rel}.{ctor}` gives every field"
        );
        let row = names
            .iter()
            .map(|(n, _)| {
                fields
                    .iter()
                    .find(|(f, _)| f == n)
                    .unwrap_or_else(|| panic!("a row of `{rel}.{ctor}` gives `{n}`"))
                    .1
                    .clone()
            })
            .collect();
        self.rows.entry(r).or_default().push((c, row));
    }

    /// Returns the value of the nullary constructor `ctor` of type `module::ty`.
    fn tag(&self, module: &str, ty: &str, ctor: &str) -> Value {
        let m = self.schema.module_named(module).expect("an engine module");
        let def = self.schema.module(m).types[ty];
        let c = self
            .schema
            .ty(def)
            .ctor_index(ctor)
            .unwrap_or_else(|| panic!("`{ty}` has constructor `{ctor}`"));
        Value::Sum {
            ctor: c,
            fields: Vec::new(),
        }
    }

    fn boolean(&self, b: bool) -> Value {
        self.tag("prelude", "Bool", if b { "True" } else { "False" })
    }

    fn none(&self) -> Value {
        self.tag("prelude", "Option", "None")
    }

    fn stage(&self, s: Stage) -> Value {
        let name = match s {
            Stage::Declared => "Declared",
            Stage::Enabled => "Enabled",
            Stage::Deprecated => "Deprecated",
        };
        self.tag("catalog", "CtorStage", name)
    }

    fn module_of(&self, def: TypeId) -> Value {
        let td = self.schema.ty(def);
        Value::Text(self.schema.module(td.module).name.clone())
    }

    fn types(&mut self) {
        for (&def, &id) in &self.schema.ids.types {
            let td = self.schema.ty(def);
            let mut fields = vec![
                ("id", key(id)),
                ("module", self.module_of(def)),
                ("name", Value::Text(td.name.clone())),
                ("params", Value::I32(td.params.len() as i32)),
                ("dropped", self.none()),
            ];
            let ctor = match &td.kind {
                TypeDefKind::Sum { .. } => {
                    fields.push(("ordered", self.boolean(td.ordered)));
                    "SumType"
                }
                TypeDefKind::Newtype { base, .. } => {
                    let over = self.schema.show_ty_with(base, &td.params);
                    fields.push(("over", Value::Text(over)));
                    "Newtype"
                }
            };
            self.push("types", ctor, fields);
        }
    }

    fn ctors(&mut self) {
        for (&(def, k), &id) in &self.schema.ids.ctors {
            let td = self.schema.ty(def);
            let name = match &td.kind {
                TypeDefKind::Sum { ctors, .. } => ctors[k as usize].name.clone(),
                TypeDefKind::Newtype { .. } => td.name.clone(),
            };
            let fields = vec![
                ("id", key(id)),
                ("sum", key(self.schema.ids.types[&def])),
                ("name", Value::Text(name)),
                ("position", Value::I32(k as i32 + 1)),
                ("dropped", self.none()),
            ];
            self.push("constructors", "Constructor", fields);
        }
    }

    fn fields(&mut self) {
        for ((def, ctor, name), &id) in &self.schema.ids.fields {
            let td = self.schema.ty(*def);
            let declared = match ctor {
                None => td.header(),
                Some(c) => &td.ctors()[*c as usize].fields[..],
            };
            let position = declared
                .iter()
                .position(|f| &f.name == name)
                .unwrap_or_else(|| panic!("`{}` declares field `{name}`", td.name));
            let field_type = self.schema.show_ty_with(&declared[position].ty, &td.params);
            let mut fields = vec![
                ("id", key(id)),
                ("name", Value::Text(name.clone())),
                ("field_type", Value::Text(field_type)),
                ("position", Value::I32(position as i32 + 1)),
                ("dropped", self.none()),
            ];
            let ctor = match ctor {
                None => {
                    fields.push(("sum", key(self.schema.ids.types[def])));
                    "Header"
                }
                Some(c) => {
                    fields.push(("ctor", key(self.schema.ids.ctors[&(*def, *c)])));
                    "Payload"
                }
            };
            self.push("fields", ctor, fields);
        }
    }

    fn relations(&mut self) {
        for (&r, &id) in &self.schema.ids.relations {
            let rel = self.schema.relation(r);
            let kind = if rel.volatile { "Volatile" } else { "Catalog" };
            let fields = vec![
                ("id", key(id)),
                (
                    "module",
                    Value::Text(self.schema.module(rel.module).name.clone()),
                ),
                ("name", Value::Text(rel.name.clone())),
                ("sum", key(self.schema.ids.types[&rel.elem_def])),
                ("kind", self.tag("catalog", "RelKind", kind)),
                ("dropped", self.none()),
            ];
            self.push("relations", "Relation", fields);
        }
    }

    fn sets(&mut self) {
        for (&(r, ref name), &id) in &self.schema.ids.sets {
            let rel = key(self.schema.ids.relations[&r]);
            let fields = vec![
                ("id", key(id)),
                ("relation", rel),
                ("name", Value::Text(name.clone())),
                ("dropped", self.none()),
            ];
            self.push("named_sets", "NamedSet", fields);
            let def = self.schema.relation(r).elem_def;
            let set = self
                .schema
                .relation(r)
                .sets
                .iter()
                .find(|s| &s.name == name)
                .unwrap_or_else(|| panic!("relation {r:?} declares set `{name}`"));
            for c in &set.ctors {
                let ctor = key(self.schema.ids.ctors[&(def, *c)]);
                self.push(
                    "set_members",
                    "SetMember",
                    vec![("set", key(id)), ("ctor", ctor)],
                );
            }
        }
    }

    /// Returns the declaration of relation `r` as `source` holds it, and the text of its file.
    fn relation_decl(&self, r: RelId) -> (RelationDecl, String) {
        let file = self.schema.relation(r).file;
        let files = self.source.files().expect("the source parses");
        let (_, f) = files
            .into_iter()
            .find(|(id, _)| *id == file)
            .unwrap_or_else(|| panic!("the source holds the file of relation {r:?}"));
        match f.decls.into_iter().next() {
            Some(Decl::Relation(d)) => (d, self.source.file_text(file)),
            _ => panic!("the file of relation {r:?} declares it"),
        }
    }

    /// Returns the id of field `name` of constructor `ctor` of relation `r`'s rows, `None`
    /// naming the header.
    fn field_id(&self, r: RelId, ctor: Option<u32>, name: &str) -> u64 {
        let def = self.schema.relation(r).elem_def;
        self.schema.ids.fields[&(def, ctor, name.to_string())]
    }

    fn defaults(&mut self) {
        let mut rows = Vec::new();
        for (&r, &id) in &self.schema.ids.generated {
            let (decl, text) = self.relation_decl(r);
            let (field, generator) = decl
                .clauses
                .iter()
                .find_map(|c| match c {
                    RelationClause::Key(fs, Some(g)) => Some((fs[0].text.clone(), g.span)),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("relation {r:?} generates its key"));
            let written = generate_text(&text, generator);
            let def = self.schema.relation(r).elem_def;
            let header = self.schema.ty(def).header().iter().any(|f| f.name == field);
            let ctor = if header { None } else { Some(0) };
            rows.push((id, r, self.field_id(r, ctor, &field), written));
        }
        for (&(r, i, ctor), &id) in &self.schema.ids.defaults {
            let (decl, text) = self.relation_decl(r);
            let (field, value) = decl
                .clauses
                .iter()
                .filter_map(|c| match c {
                    RelationClause::Default { field, value, .. } => Some((field, value)),
                    _ => None,
                })
                .nth(i)
                .unwrap_or_else(|| panic!("relation {r:?} declares default {i}"));
            let written = match value {
                ast::DefaultValue::Generate(g) => generate_text(&text, g.span),
                ast::DefaultValue::Expr(e) => text[e.span.range()].to_string(),
            };
            rows.push((id, r, self.field_id(r, ctor, &field.text), written));
        }
        for (id, r, field, text) in rows {
            let fields = vec![
                ("id", key(id)),
                ("relation", key(self.schema.ids.relations[&r])),
                ("field", key(field)),
                ("text", Value::Text(text)),
                ("dropped", self.none()),
            ];
            self.push("defaults", "Default", fields);
        }
    }

    fn constraints(&mut self) {
        let declared = self.schema.ids.constraints.iter().map(|(&i, &id)| {
            let c = &self.schema.constraints[i];
            (id, c.module, c.name.clone(), c.file)
        });
        let checks = self.schema.ids.checks.iter().map(|(&i, &id)| {
            let c = &self.schema.checks[i];
            (id, c.module, c.name.clone(), c.file)
        });
        let all: Vec<_> = declared.chain(checks).collect();
        for (id, module, name, file) in all {
            let fields = vec![
                ("id", key(id)),
                (
                    "module",
                    Value::Text(self.schema.module(module).name.clone()),
                ),
                ("name", Value::Text(name)),
                (
                    "text",
                    Value::Text(self.source.decls[file.0 as usize].text.clone()),
                ),
                ("dropped", self.none()),
            ];
            self.push("constraints", "Constraint", fields);
        }
    }

    fn indexes(&mut self) {
        for (id, spec) in indexes(self.schema) {
            let fields = vec![
                ("id", key(id)),
                ("relation", key(self.schema.ids.relations[&spec.relation])),
                ("kind", self.tag("catalog", "IndexKind", spec.kind)),
                ("clause", Value::Text(spec.clause.clone())),
                ("dropped", self.none()),
            ];
            self.push("indexes", "Index", fields);
            let def = self.schema.relation(spec.relation).elem_def;
            for c in &spec.ctors {
                let ctor = key(self.schema.ids.ctors[&(def, *c)]);
                self.push(
                    "index_ctors",
                    "IndexCtor",
                    vec![("index", key(id)), ("ctor", ctor)],
                );
            }
        }
    }

    fn ctor_states(&mut self) {
        for (&r, &id) in &self.schema.ids.relations {
            let def = self.schema.relation(r).elem_def;
            for c in all_ctors(self.schema, r) {
                let fields = vec![
                    ("relation", key(id)),
                    ("ctor", key(self.schema.ids.ctors[&(def, c)])),
                    ("stage", self.stage(self.schema.stage(r, c))),
                ];
                self.push("ctor_states", "CtorState", fields);
            }
        }
    }

    fn functions(&mut self) {
        let functions = self
            .source
            .functions()
            .unwrap_or_else(|ds| panic!("the functions of a checked source resolve: {ds:?}"));
        for f in functions {
            let (ctor, hashed, own) = match &f.code {
                Code::Language { body, canonical } => (
                    "Language",
                    canonical,
                    vec![("body", Value::Text(body.clone()))],
                ),
                Code::Aggregate { body, canonical } => (
                    "Aggregate",
                    canonical,
                    vec![("body", Value::Text(body.clone()))],
                ),
                Code::Host { symbol, version } => (
                    "Host",
                    version,
                    vec![
                        ("symbol", Value::Text(symbol.clone())),
                        ("version", Value::Text(version.clone())),
                    ],
                ),
                Code::Wasm { module, .. } => {
                    panic!(
                        "function `{}` names Wasm module `{module}`, which no migration installs",
                        f.name
                    )
                }
            };
            let mut fields = vec![
                ("id", key(f.id)),
                ("module", Value::Text(f.module)),
                ("name", Value::Text(f.name)),
                ("signature", Value::Text(f.signature)),
                ("deterministic", self.boolean(f.deterministic)),
                (
                    "hash",
                    Value::Bytes(blake3::hash(hashed.as_bytes()).as_bytes().to_vec()),
                ),
                ("dropped", self.none()),
            ];
            fields.extend(own);
            self.push("functions", ctor, fields);
        }
    }

    /// Add the rows of `migration_ops` and `migration_clauses` for every migration of the log.
    fn log(&mut self) {
        for l in &self.source.log {
            let number = key(l.number);
            for (k, (op, clauses)) in oplog::operators(self.schema, self.source, l.number)
                .into_iter()
                .enumerate()
            {
                let ordinal = Value::I32(i32::try_from(k + 1).expect("an ordinal fits 31 bits"));
                for (j, body) in clauses.into_iter().enumerate() {
                    let clause = Value::I32(i32::try_from(j + 1).expect("a clause fits 31 bits"));
                    let fields = vec![
                        ("migration", number.clone()),
                        ("ordinal", ordinal.clone()),
                        ("clause", clause),
                        ("body", body),
                    ];
                    self.push("migration_clauses", "OpClause", fields);
                }
                let fields = vec![
                    ("migration", number.clone()),
                    ("ordinal", ordinal),
                    ("op", op),
                ];
                self.push("migration_ops", "MigrationOp", fields);
            }
        }
    }

    fn value_states(&mut self) {
        let mut sums = BTreeSet::new();
        for &def in self.schema.ids.types.keys() {
            let td = self.schema.ty(def);
            let declared = td
                .header()
                .iter()
                .chain(td.ctors().iter().flat_map(|c| &c.fields));
            for f in declared {
                field_sums(self.schema, &f.ty, &mut sums);
            }
        }
        for def in sums {
            for c in 0..self.schema.ty(def).ctors().len() as u32 {
                let Some(&id) = self.schema.ids.ctors.get(&(def, c)) else {
                    continue;
                };
                let fields = vec![
                    ("ctor", key(id)),
                    ("stage", self.stage(self.schema.value_stage(def, c))),
                ];
                self.push("value_states", "ValueState", fields);
            }
        }
    }
}

/// Returns the text of `text` from the `generate` before the generator at `generator` to the
/// generator's end.
fn generate_text(text: &str, generator: Span) -> String {
    let (tokens, _) = lex(text);
    let word = tokens
        .iter()
        .rev()
        .find(|t| t.span.end <= generator.start && &text[t.span.range()] == "generate")
        .expect("a generator follows `generate`");
    text[word.span.start as usize..generator.end as usize].to_string()
}

fn key(id: u64) -> Value {
    Value::I64(i64::try_from(id).expect("a catalog id fits 63 bits"))
}

/// Add to `out` every declared sum a value of `ty` may hold at any depth.
fn field_sums(schema: &Schema, ty: &Ty, out: &mut BTreeSet<TypeId>) {
    match ty {
        Ty::Data { def, args, .. } => {
            if matches!(schema.ty(*def).kind, TypeDefKind::Sum { .. }) {
                out.insert(*def);
            }
            for a in args {
                field_sums(schema, a, out);
            }
        }
        Ty::Record(fs) => {
            for (_, t) in fs {
                field_sums(schema, t, out);
            }
        }
        Ty::Sum { ctors, .. } => {
            for (_, fs) in ctors {
                for (_, t) in fs {
                    field_sums(schema, t, out);
                }
            }
        }
        Ty::Set(t) | Ty::NonEmpty(t) | Ty::Tag(t) => field_sums(schema, t, out),
        _ => {}
    }
}
