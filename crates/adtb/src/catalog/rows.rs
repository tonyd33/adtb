use std::collections::{BTreeMap, BTreeSet};

use adtb_lang::ast::{ExprKind, QName};
use adtb_lang::evolve::Source;
use adtb_lang::schema::{IndexDef, Schema, Stage, TypeDefKind, ValueDef};
use adtb_lang::types::{RelId, Ty, TypeId};

use crate::key::Value;

use super::layout;

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
    b.ctor_states();
    b.value_states();
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
    pub fields: Vec<String>,
    kind: &'static str,
    clause: String,
}

/// Returns the key index and the uniques and indexes of every relation of `schema` with
/// catalog ids, by index id.
///
/// Panics if an index is over an expression other than a field.
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
            kind: "PrimaryKey",
            clause: key,
        };
        out.insert(id, spec);
    }
    for (&(r, i), &id) in &schema.ids.indexes {
        let def = &schema.relation(r).indexes[i];
        let fields = indexed_fields(def).expect("every index of the schema is over fields");
        let spec = IndexSpec {
            id,
            relation: r,
            unique: def.unique,
            ctors: def.scope.clone().unwrap_or_else(|| all_ctors(schema, r)),
            clause: clause(schema, r, def, &fields),
            fields,
            kind: match (def.unique, def.interval) {
                (true, _) => "Unique",
                (false, true) => "Interval",
                (false, false) => "Plain",
            },
        };
        out.insert(id, spec);
    }
    out
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
