use std::collections::{BTreeMap, BTreeSet};

use adtb_lang::ast::{self, Decl, ExprKind, QName, RelationClause, RelationDecl};
use adtb_lang::evolve::{Code, Source};
use adtb_lang::kernel::PathStep;
use adtb_lang::lex::lex;
use adtb_lang::schema::{IndexDef, Schema, Stage, TypeDefKind, field_path};
use adtb_lang::span::Span;
use adtb_lang::types::{RelId, Ty, TypeId};

use crate::exec::{self, Broken};
use crate::key::Value;
use crate::plan::{Element, declared_elements};

use super::ids::{held_id, type_ctor_id};
use super::{layout, oplog, values};

/// Rows of relations, each with its constructor index and as a heap of that constructor holds it.
pub type Rows = BTreeMap<RelId, Vec<(u32, Vec<Value>)>>;

/// Returns the rows `catalog::` relations hold for the objects of `schema`, built from `source`,
/// that have catalog ids, with every relation of `schema` present.
#[cfg(test)]
pub fn declared(schema: &Schema, source: &Source) -> Rows {
    let mut rows = objects(schema, source);
    let mut b = Builder {
        schema,
        source,
        rows: Rows::new(),
    };
    b.log();
    for (r, logged) in b.rows {
        rows.entry(r).or_default().extend(logged);
    }
    rows
}

pub(super) fn objects(schema: &Schema, source: &Source) -> Rows {
    let mut b = Builder {
        schema,
        source,
        rows: schema
            .ids()
            .relation_ids()
            .map(|(r, _)| (r, Vec::new()))
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
    b.computed();
    b.ctor_states();
    b.value_states();
    b.functions();
    b.rows
}

/// Returns the row of `catalog::migrations` for the last migration of `source`, applied at
/// `applied` microseconds since 1970-01-01T00:00:00Z, or no row if the log is empty.
#[cfg(test)]
pub fn applied(schema: &Schema, source: &Source, applied: i64) -> Rows {
    applied_hash(schema, source, oplog::version_hash(schema, source), applied)
}

fn applied_hash(schema: &Schema, source: &Source, hash: oplog::Hash, applied: i64) -> Rows {
    let mut b = Builder {
        schema,
        source,
        rows: Rows::new(),
    };
    if let Some(m) = source.log.last() {
        let fields = vec![
            ("version", key(m.number)),
            ("hash", Value::Bytes(hash.to_vec())),
            ("text", Value::Text(m.text.clone())),
            ("applied", Value::Instant(applied)),
        ];
        b.push("migrations", "Migration", fields);
    }
    b.rows
}

pub(super) fn recorded(
    schema: &Schema,
    source: &Source,
    ops: &[oplog::OperatorRow],
    previous: oplog::Hash,
    applied: i64,
) -> Rows {
    let rows = applied_hash(
        schema,
        source,
        oplog::chained(schema, previous, ops),
        applied,
    );
    let mut b = Builder {
        schema,
        source,
        rows,
    };
    b.migration(source.version, ops);
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

impl IndexSpec {
    /// Returns the key index `id` of relation `r` of `schema` over `fields`.
    pub fn key(schema: &Schema, r: RelId, id: u64, fields: Vec<String>) -> IndexSpec {
        let clause = if let [k] = fields.as_slice() {
            format!("key {k}")
        } else {
            format!("key ({})", fields.join(", "))
        };
        IndexSpec {
            id,
            relation: r,
            unique: true,
            ctors: all_ctors(schema, r),
            elements: fields.iter().cloned().map(Element::Field).collect(),
            fields,
            kind: "PrimaryKey",
            clause,
        }
    }
}

/// Returns every index of the relations of `schema` with catalog ids, by index id: the key
/// indexes, the declared uniques and indexes, and the implied indexes.
pub fn indexes(schema: &Schema) -> BTreeMap<u64, IndexSpec> {
    let mut out = BTreeMap::new();
    for (r, id) in schema.ids().key_ids() {
        let id = id.raw();
        out.insert(
            id,
            IndexSpec::key(schema, r, id, schema.relation(r).key.clone()),
        );
    }
    for ((r, i), id) in schema.ids().index_ids() {
        let id = id.raw();
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
    for (r, f, id) in schema.ids().implied_ids() {
        let (ctor, field) = (f.ctor(), &schema.ty(f.def()).field_def(f).name);
        let id = id.raw();
        let ctors = ctor.map_or_else(|| all_ctors(schema, r), |c| vec![c]);
        let clause = match ctor {
            None => format!("index ({field})"),
            Some(c) => {
                let td = schema.ty(schema.relation(r).elem_def);
                format!("index ({field}) in {}", td.ctors()[c as usize].name)
            }
        };
        let spec = IndexSpec {
            id,
            relation: r,
            unique: false,
            ctors,
            fields: vec![field.to_string()],
            elements: vec![Element::Field(field.to_string())],
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

/// One step of a field path through a row: a field by its position, or the constructor by its
/// position that the value the steps before it reach must hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Field(usize),
    Ctor(u32),
}

/// Returns the steps of field path `path` through a row of constructor `c` of `r`, and the type
/// of the value it reaches.
///
/// Fails with `exec::Error::Internal` if `path` names no field or constructor at a step.
pub fn path_steps(
    schema: &Schema,
    r: RelId,
    c: u32,
    path: &[PathStep],
) -> Result<(Vec<Step>, Ty), exec::Error> {
    let Some((PathStep::Field(first), rest)) = path.split_first() else {
        return Err(Broken::PathStart(path.to_vec()).into());
    };
    let reached = |field: &str| Broken::PathValueField {
        path: path.to_vec(),
        field: field.into(),
    };
    let row = layout::row(schema, r, c);
    let at = row
        .iter()
        .position(|(n, _)| n == first)
        .ok_or_else(|| Broken::PathRowField {
            path: path.to_vec(),
            field: first.clone(),
        })?;
    let mut steps = vec![Step::Field(at)];
    let mut ty = row[at].1.clone();
    let mut fields: Option<Vec<(String, Ty)>> = None;
    for step in rest {
        let name = match step {
            PathStep::Ctor(k) => {
                steps.push(Step::Ctor(*k));
                let mut ctors = layout::ctors(schema, &layout::repr(schema, &ty));
                if *k as usize >= ctors.len() {
                    return Err(Broken::PathCtor {
                        path: path.to_vec(),
                        ctor: *k,
                    }
                    .into());
                }
                fields = Some(ctors.swap_remove(*k as usize).1);
                continue;
            }
            PathStep::Field(name) => name,
        };
        let fs = match fields.take() {
            Some(fs) => fs,
            None => match layout::repr(schema, &ty) {
                Ty::Record(fs) => fs,
                t => match layout::ctors(schema, &t).into_iter().next() {
                    Some((_, fs)) => fs,
                    None => return Err(reached(name).into()),
                },
            },
        };
        let at = fs
            .iter()
            .position(|(n, _)| n == name)
            .ok_or_else(|| reached(name))?;
        steps.push(Step::Field(at));
        ty = fs[at].1.clone();
    }
    Ok((steps, ty))
}

/// Returns the value `steps` reach in `row`, or `None` when a value on the way holds another
/// constructor than a step names.
///
/// Fails with `exec::Error::Corrupt` when a value on the way does not have the shape the steps
/// give it.
pub fn at_path<'v>(row: &'v [Value], steps: &[Step]) -> Result<Option<&'v Value>, exec::Error> {
    let corrupt = || exec::Error::from(exec::Rot::StoredPath(steps.to_vec()));
    let Some((first, rest)) = steps.split_first() else {
        return Ok(None);
    };
    let Step::Field(i) = first else {
        return Err(Broken::StepsStart(steps.to_vec()).into());
    };
    let mut at = row.get(*i).ok_or_else(corrupt)?;
    for step in rest {
        at = match (step, at) {
            (Step::Ctor(k), Value::Sum { ctor, .. }) => {
                if ctor != k {
                    return Ok(None);
                }
                at
            }
            (Step::Field(i), Value::Sum { fields, .. } | Value::Record(fields)) => {
                fields.get(*i).ok_or_else(corrupt)?
            }
            _ => return Err(corrupt()),
        };
    }
    Ok(Some(at))
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
    /// Add a row of constructor `ctor` of `catalog::{rel}`, its fields given by name.
    fn push(&mut self, rel: &str, ctor: &str, fields: Vec<(&str, Value)>) {
        let r = values::relation(self.schema, rel);
        let td = self.schema.ty(self.schema.relation(r).elem_def);
        let c = td
            .ctor_index(ctor)
            .unwrap_or_else(|| panic!("`{rel}` has constructor `{ctor}`"));
        let row = values::row(self.schema, r, c, fields);
        self.rows.entry(r).or_default().push((c, row));
    }

    fn none(&self) -> Value {
        values::tag(self.schema, "prelude", "Option", "None")
    }

    fn stage(&self, s: Stage) -> Value {
        let name = match s {
            Stage::Declared => "Declared",
            Stage::Enabled => "Enabled",
            Stage::Deprecated => "Deprecated",
        };
        values::tag(self.schema, "catalog", "CtorStage", name)
    }

    fn module_of(&self, def: TypeId) -> Value {
        let td = self.schema.ty(def);
        Value::Text(self.schema.module(td.module).name.clone())
    }

    fn types(&mut self) {
        for (def, id) in self.schema.ids().type_ids() {
            let td = self.schema.ty(def);
            let mut fields = vec![
                ("id", key(id.raw())),
                ("module", self.module_of(def)),
                ("name", Value::Text(td.name.clone())),
                ("params", Value::I32(td.params.len() as i32)),
                ("dropped", self.none()),
            ];
            let ctor = match &td.kind {
                TypeDefKind::Sum { .. } => {
                    fields.push(("ordered", values::boolean(self.schema, td.ordered)));
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
        for (c, id) in self.schema.ids().ctor_ids() {
            let (def, k) = (c.def(), c.index());
            let td = self.schema.ty(def);
            let name = match &td.kind {
                TypeDefKind::Sum { .. } => td.ctor_def(c).name.clone(),
                TypeDefKind::Newtype { .. } => td.name.clone(),
            };
            let fields = vec![
                ("id", key(id.raw())),
                ("sum", key(held_id(self.schema.ids().type_id(def)).raw())),
                ("name", Value::Text(name)),
                ("position", Value::I32(k as i32 + 1)),
                ("dropped", self.none()),
            ];
            self.push("constructors", "Constructor", fields);
        }
    }

    fn fields(&mut self) {
        let mut held: Vec<_> = self.schema.ids().field_ids().collect();
        held.sort_by_key(|(f, _)| self.schema.field_order(*f));
        for (f, id) in held {
            let td = self.schema.ty(f.def());
            let declared = td.field_def(f);
            let field_type = self.schema.show_ty_with(&declared.ty, &td.params);
            let mut fields = vec![
                ("id", key(id.raw())),
                ("name", Value::Text(declared.name.clone())),
                ("field_type", Value::Text(field_type)),
                ("position", Value::I32(f.pos() as i32 + 1)),
                ("dropped", self.none()),
            ];
            let ctor = match f.ctor() {
                None => {
                    fields.push((
                        "sum",
                        key(held_id(self.schema.ids().type_id(f.def())).raw()),
                    ));
                    "Header"
                }
                Some(c) => {
                    fields.push(("ctor", key(type_ctor_id(self.schema, f.def(), c).raw())));
                    "Payload"
                }
            };
            self.push("fields", ctor, fields);
        }
    }

    fn relations(&mut self) {
        let mut rows = Vec::new();
        for (r, id) in self.schema.ids().relation_ids() {
            let rel = self.schema.relation(r);
            let module = &self.schema.module(rel.module).name;
            let kind = match (rel.volatile, module.as_str()) {
                (true, _) => "Volatile",
                (false, "catalog") => "Catalog",
                (false, _) => "Base",
            };
            let sum = held_id(self.schema.ids().type_id(rel.elem_def));
            rows.push((id, module.clone(), rel.name.clone(), sum, kind));
        }
        for (f, id) in self.schema.ids().view_ids() {
            let view = &self.schema.functions[f.0 as usize];
            let sum = match &view.result {
                Ty::Set(elem) => match &**elem {
                    Ty::Data { def, .. } => self.schema.ids().type_id(*def),
                    _ => self.schema.ids().view_type_id(f),
                },
                _ => None,
            }
            .unwrap_or_else(|| panic!("the rows of stored view `{}` have a type id", view.name));
            let module = self.schema.module(view.module).name.clone();
            rows.push((id, module, view.name.clone(), sum, "Stored"));
        }
        for (id, module, name, sum, kind) in rows {
            let fields = vec![
                ("id", key(id.raw())),
                ("module", Value::Text(module)),
                ("name", Value::Text(name)),
                ("sum", key(sum.raw())),
                ("kind", values::tag(self.schema, "catalog", "RelKind", kind)),
                ("dropped", self.none()),
            ];
            self.push("relations", "Relation", fields);
        }
    }

    fn computed(&mut self) {
        let mut rows = Vec::new();
        for ((r, i), id) in self.schema.ids().computed_ids() {
            let (decl, text) = self.relation_decl(r);
            let (name, ty, expr) = decl
                .clauses
                .iter()
                .filter_map(|c| match c {
                    RelationClause::Computed(name, ty, expr) => Some((name, ty, expr)),
                    _ => None,
                })
                .nth(i)
                .unwrap_or_else(|| panic!("relation {r:?} declares computed field {i}"));
            rows.push((
                id,
                r,
                name.text.clone(),
                text[ty.span.range()].to_string(),
                text[expr.span.range()].to_string(),
            ));
        }
        for (id, r, name, field_type, text) in rows {
            let fields = vec![
                ("id", key(id.raw())),
                (
                    "relation",
                    key(held_id(self.schema.ids().relation_id(r)).raw()),
                ),
                ("name", Value::Text(name)),
                ("field_type", Value::Text(field_type)),
                ("text", Value::Text(text)),
                ("dropped", self.none()),
            ];
            self.push("computed", "Computed", fields);
        }
    }

    fn sets(&mut self) {
        for (r, name, id) in self.schema.ids().set_ids() {
            let id = id.raw();
            let rel = key(held_id(self.schema.ids().relation_id(r)).raw());
            let fields = vec![
                ("id", key(id)),
                ("relation", rel),
                ("name", Value::Text(name.to_string())),
                ("dropped", self.none()),
            ];
            self.push("named_sets", "NamedSet", fields);
            let def = self.schema.relation(r).elem_def;
            let set = self
                .schema
                .relation(r)
                .sets
                .iter()
                .find(|s| s.name == name)
                .unwrap_or_else(|| panic!("relation {r:?} declares set `{name}`"));
            for c in &set.ctors {
                let ctor = key(type_ctor_id(self.schema, def, *c).raw());
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

    fn defaults(&mut self) {
        let mut rows = Vec::new();
        for (r, id) in self.schema.ids().generated_ids() {
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
            let td = self.schema.ty(self.schema.relation(r).elem_def);
            let filled = td.field(td.ctor_at(0), &field);
            let filled = held_id(filled.and_then(|f| self.schema.ids().field_id(f)));
            rows.push((id, r, filled.raw(), written));
        }
        for ((r, i, f), id) in self.schema.ids().default_ids() {
            let (decl, text) = self.relation_decl(r);
            let value = decl
                .clauses
                .iter()
                .filter_map(|c| match c {
                    RelationClause::Default { value, .. } => Some(value),
                    _ => None,
                })
                .nth(i)
                .unwrap_or_else(|| panic!("relation {r:?} declares default {i}"));
            let written = match value {
                ast::DefaultValue::Generate(g) => generate_text(&text, g.span),
                ast::DefaultValue::Expr(e) => text[e.span.range()].to_string(),
            };
            let filled = held_id(self.schema.ids().field_id(f)).raw();
            rows.push((id, r, filled, written));
        }
        for (id, r, field, text) in rows {
            let fields = vec![
                ("id", key(id.raw())),
                (
                    "relation",
                    key(held_id(self.schema.ids().relation_id(r)).raw()),
                ),
                ("field", key(field)),
                ("text", Value::Text(text)),
                ("dropped", self.none()),
            ];
            self.push("defaults", "Default", fields);
        }
    }

    fn constraints(&mut self) {
        let declared = self.schema.ids().constraint_ids().map(|(i, id)| {
            let c = &self.schema.constraints[i];
            (id, c.module, c.name.clone(), c.file)
        });
        let checks = self.schema.ids().check_ids().map(|(i, id)| {
            let c = &self.schema.checks[i];
            (id, c.module, c.name.clone(), c.file)
        });
        let all: Vec<_> = declared.chain(checks).collect();
        for (id, module, name, file) in all {
            let fields = vec![
                ("id", key(id.raw())),
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
                (
                    "relation",
                    key(held_id(self.schema.ids().relation_id(spec.relation)).raw()),
                ),
                (
                    "kind",
                    values::tag(self.schema, "catalog", "IndexKind", spec.kind),
                ),
                ("clause", Value::Text(spec.clause.clone())),
                ("dropped", self.none()),
            ];
            self.push("indexes", "Index", fields);
            let def = self.schema.relation(spec.relation).elem_def;
            for c in &spec.ctors {
                let ctor = key(type_ctor_id(self.schema, def, *c).raw());
                self.push(
                    "index_ctors",
                    "IndexCtor",
                    vec![("index", key(id)), ("ctor", ctor)],
                );
            }
        }
    }

    fn ctor_states(&mut self) {
        for (r, id) in self.schema.ids().relation_ids() {
            let def = self.schema.relation(r).elem_def;
            for c in all_ctors(self.schema, r) {
                let fields = vec![
                    ("relation", key(id.raw())),
                    ("ctor", key(type_ctor_id(self.schema, def, c).raw())),
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
                (
                    "deterministic",
                    values::boolean(self.schema, f.deterministic),
                ),
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
    #[cfg(test)]
    fn log(&mut self) {
        for l in &self.source.log {
            self.migration(
                l.number,
                &oplog::operators(self.schema, self.source, l.number),
            );
        }
    }

    fn migration(&mut self, version: u64, ops: &[oplog::OperatorRow]) {
        let number = key(version);
        for (k, (op, clauses)) in ops.iter().enumerate() {
            let ordinal = Value::I32(i32::try_from(k + 1).expect("an ordinal fits 31 bits"));
            for (j, body) in clauses.iter().enumerate() {
                let clause = Value::I32(i32::try_from(j + 1).expect("a clause fits 31 bits"));
                let fields = vec![
                    ("migration", number.clone()),
                    ("ordinal", ordinal.clone()),
                    ("clause", clause),
                    ("body", body.clone()),
                ];
                self.push("migration_clauses", "OpClause", fields);
            }
            let fields = vec![
                ("migration", number.clone()),
                ("ordinal", ordinal),
                ("op", op.clone()),
            ];
            self.push("migration_ops", "MigrationOp", fields);
        }
    }

    fn value_states(&mut self) {
        let mut sums = BTreeSet::new();
        for (def, _) in self.schema.ids().type_ids() {
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
                let found = self.schema.ty(def).ctor_at(c);
                let Some(id) = found.and_then(|c| self.schema.ids().ctor_id(c)) else {
                    continue;
                };
                let fields = vec![
                    ("ctor", key(id.raw())),
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
