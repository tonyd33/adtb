use std::collections::{BTreeMap, BTreeSet};

use adtb_lang::ast::{self, Decl, ExprKind, QName, RelationClause, RelationDecl};
use adtb_lang::canon::canonical_term;
use adtb_lang::evolve::{Code, Source};
use adtb_lang::ids::{engine_ctor as k, engine_rel};
use adtb_lang::kernel::PathStep;
use adtb_lang::lex::lex;
use adtb_lang::schema::{IndexDef, Schema, Stage, TypeDefKind, field_path};
use adtb_lang::span::Span;
use adtb_lang::types::{RelId, Ty, TypeId};

pub use crate::exec::layout::Step;
use crate::exec::{self, Broken};
use crate::key::Value;
use crate::plan::{Element, declared_elements};

use super::codec::{
    Codec, Decode, Field, MIGRATION_APPLIED, MIGRATION_HASH, MIGRATION_TEXT, MIGRATION_VERSION,
    MigrationRow, Sum,
};
use super::described::{
    AGGREGATE_BODY, AGGREGATE_HEAD, COMPUTED_DROPPED, COMPUTED_FIELD_TYPE, COMPUTED_ID,
    COMPUTED_NAME, COMPUTED_RELATION, COMPUTED_TEXT, CONSTRAINT_DROPPED, CONSTRAINT_ID,
    CONSTRAINT_MODULE, CONSTRAINT_NAME, CONSTRAINT_TEXT, CONSTRUCTOR_DROPPED, CONSTRUCTOR_ID,
    CONSTRUCTOR_NAME, CONSTRUCTOR_POSITION, CONSTRUCTOR_SUM, CTOR_STATE_CTOR, CTOR_STATE_RELATION,
    CTOR_STATE_STAGE, ComputedRow, ConstraintRow, ConstructorRow, CtorStateRow, DEFAULT_DROPPED,
    DEFAULT_FIELD, DEFAULT_ID, DEFAULT_RELATION, DEFAULT_TEXT, DefaultRow, FIELD_CTOR,
    FIELD_HEADER_HEAD, FIELD_PAYLOAD_HEAD, FIELD_SUM, FieldRow, FunctionRow, HOST_HEAD,
    HOST_SYMBOL, HOST_VERSION, INDEX_CLAUSE, INDEX_CTOR_CTOR, INDEX_CTOR_INDEX, INDEX_DROPPED,
    INDEX_ID, INDEX_KIND, INDEX_RELATION, IndexCtorRow, IndexRow, LANGUAGE_BODY, LANGUAGE_HEAD,
    NAMED_SET_DROPPED, NAMED_SET_ID, NAMED_SET_NAME, NAMED_SET_RELATION, NamedSetRow,
    RELATION_DROPPED, RELATION_ID, RELATION_KIND, RELATION_MODULE, RELATION_NAME, RELATION_SUM,
    RelationRow, SET_MEMBER_CTOR, SET_MEMBER_SET, SetMemberRow, TYPE_NEWTYPE_HEAD, TYPE_ORDERED,
    TYPE_OVER, TYPE_SUM_HEAD, TypeRow, VALUE_STATE_CTOR, VALUE_STATE_STAGE, ValueStateRow,
};
use super::ids::{CatCtorId, engine_relation, held_id, type_ctor_id};
use super::operators::{
    MIGRATION_OP_MIGRATION, MIGRATION_OP_OP, MIGRATION_OP_ORDINAL, OP_CLAUSE_BODY,
    OP_CLAUSE_CLAUSE, OP_CLAUSE_MIGRATION, OP_CLAUSE_ORDINAL,
};
use super::{layout, oplog, values};

/// Rows of relations, each with its constructor index and as a heap of that constructor holds it.
pub type Rows = BTreeMap<RelId, Vec<(u32, Vec<Value>)>>;

/// Returns the rows `catalog::` relations hold for the objects of `schema`, built from `source`,
/// that have catalog ids, with every relation of `schema` present, each index element that is not
/// a field path as `texts` holds it.
#[cfg(test)]
pub fn declared(schema: &Schema, source: &Source, texts: &ElementTexts) -> Rows {
    let mut rows = objects(schema, source, texts);
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

pub(super) fn objects(schema: &Schema, source: &Source, texts: &ElementTexts) -> Rows {
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
    b.indexes(texts);
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
    let hash = oplog::version_hash(schema, source).expect("the log resolves");
    applied_hash(schema, source, hash, applied)
}

fn applied_hash(schema: &Schema, source: &Source, hash: oplog::Hash, applied: i64) -> Rows {
    let mut rows = Rows::new();
    if let Some(m) = source.log.last() {
        let codec = Codec::<MigrationRow>::new(schema);
        let row = codec.encode(
            k::MIGRATION,
            [
                (MIGRATION_VERSION, key(m.number)),
                (MIGRATION_HASH, Value::Bytes(hash.to_vec())),
                (MIGRATION_TEXT, Value::Text(m.text.clone())),
                (MIGRATION_APPLIED, Value::Instant(applied)),
            ],
        );
        rows.insert(codec.relation(), vec![row]);
    }
    rows
}

pub(super) fn recorded(
    schema: &Schema,
    source: &Source,
    ops: &[oplog::OperatorRow],
    hash: oplog::Hash,
    applied: i64,
) -> Rows {
    let log = oplog::Log::new(schema);
    let rows = applied_hash(schema, source, hash, applied);
    let mut b = Builder {
        schema,
        source,
        rows,
    };
    b.migration(&log, source.version, ops);
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
    pub elements: Vec<Element>,
    kind: CatCtorId,
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
            elements: fields.into_iter().map(Element::Field).collect(),
            kind: k::INDEX_PRIMARY_KEY,
            clause,
        }
    }
}

/// The canonical source text of each declared index element that is not a field path, by
/// relation, index position and element position.
#[derive(Clone, Debug, Default)]
pub struct ElementTexts(BTreeMap<(RelId, usize, usize), String>);

impl ElementTexts {
    /// Returns the texts of the index elements of `schema`, the schema `source` builds.
    ///
    /// Fail with [`super::Broken::Unnamed`] when an element writes a name the source does not
    /// resolve.
    pub fn of(source: &Source, schema: &Schema) -> Result<ElementTexts, super::Error> {
        let mut names = None;
        let mut texts = BTreeMap::new();
        let mut files = BTreeMap::new();
        for ((r, i), _) in schema.ids().index_ids() {
            let rel = schema.relation(r);
            for (j, e) in rel.indexes[i].fields.iter().enumerate() {
                if field_path(e).is_some() {
                    continue;
                }
                let names = names.get_or_insert_with(|| {
                    source.names().unwrap_or_else(|ds| {
                        panic!("the source of a built schema names its objects: {ds:?}")
                    })
                });
                let names = names.get(rel.file.0 as usize).unwrap_or_else(|| {
                    panic!("an index element of relation `{}` is not a field path and has no source file", rel.name)
                });
                let text = files
                    .entry(rel.file)
                    .or_insert_with(|| source.file_text(rel.file));
                let canonical = canonical_term(text, e.span, names)
                    .map_err(|u| super::Error::from(super::Broken::Unnamed(u)))?;
                texts.insert((r, i, j), canonical);
            }
        }
        Ok(ElementTexts(texts))
    }

    /// Returns the text of element `j` of declared index `i` of `r`.
    pub(super) fn get(&self, r: RelId, i: usize, j: usize) -> String {
        self.0
            .get(&(r, i, j))
            .unwrap_or_else(|| {
                panic!("every declared index element that is not a field path has a canonical text, and element {j} of index {i} of {r:?} has none")
            })
            .clone()
    }
}

/// Returns every index of the relations of `schema` with catalog ids, by index id: the key
/// indexes, the declared uniques and indexes, and the implied indexes, each element that is not a
/// field path as `texts` holds it.
pub fn indexes(schema: &Schema, texts: &ElementTexts) -> BTreeMap<u64, IndexSpec> {
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
            .enumerate()
            .map(|(j, e)| field_path(e).unwrap_or_else(|| texts.get(r, i, j)))
            .collect();
        let elements = declared_elements(i, def);
        let spec = IndexSpec {
            id,
            relation: r,
            unique: def.unique,
            ctors: def.scope.clone().unwrap_or_else(|| all_ctors(schema, r)),
            clause: clause(schema, r, def, &fields),
            elements,
            kind: match (def.unique, def.interval) {
                (true, _) => k::INDEX_UNIQUE,
                (false, true) => k::INDEX_INTERVAL,
                (false, false) => k::INDEX_PLAIN,
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
            elements: vec![Element::Field(field.to_string())],
            kind: k::INDEX_IMPLIED,
            clause,
        };
        out.insert(id, spec);
    }
    out
}

/// Returns the relation, uniqueness and constructors of every index of `schema` with a catalog
/// id, by index id.
pub fn index_heads(schema: &Schema) -> BTreeMap<u64, (RelId, bool, Vec<u32>)> {
    let keys = schema
        .ids()
        .key_ids()
        .map(|(r, id)| (id.raw(), (r, true, all_ctors(schema, r))));
    let declared = schema.ids().index_ids().map(|((r, i), id)| {
        let def = &schema.relation(r).indexes[i];
        let ctors = def.scope.clone().unwrap_or_else(|| all_ctors(schema, r));
        (id.raw(), (r, def.unique, ctors))
    });
    let implied = schema.ids().implied_ids().map(|(r, f, id)| {
        let ctors = f.ctor().map_or_else(|| all_ctors(schema, r), |c| vec![c]);
        (id.raw(), (r, false, ctors))
    });
    keys.chain(declared).chain(implied).collect()
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

/// Returns the steps of field path `path` through a row of constructor `c` of `r`, and the type
/// of the value it reaches.
///
/// Errors:
/// - `exec::Error::Internal` if `path` names no field or constructor at a step.
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
/// Errors:
/// - `exec::Error::Corrupt` when a value on the way does not have the shape the steps
///   give it.
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
    /// Add the row of constructor `id` of `codec`'s relation holding `given`.
    fn put<T: Decode>(&mut self, codec: &Codec<T>, id: CatCtorId, given: Vec<(Field, Value)>) {
        let row = codec.encode(id, given);
        self.rows.entry(codec.relation()).or_default().push(row);
    }

    fn none(&self) -> Value {
        values::none()
    }

    /// Returns the value of `s` in sum field `stage` of `codec`'s relation.
    fn stage<T: Decode>(&self, codec: &Codec<T>, stage: Sum, s: Stage) -> Value {
        let id = match s {
            Stage::Declared => k::STAGE_DECLARED,
            Stage::Enabled => k::STAGE_ENABLED,
            Stage::Deprecated => k::STAGE_DEPRECATED,
        };
        tag(codec, stage, id)
    }

    fn module_of(&self, def: TypeId) -> Value {
        let td = self.schema.ty(def);
        Value::Text(self.schema.module(td.module).name.clone())
    }

    fn types(&mut self) {
        let codec = Codec::<TypeRow>::new(self.schema);
        for (def, id) in self.schema.ids().type_ids() {
            let td = self.schema.ty(def);
            let (ctor, head, own) = match &td.kind {
                TypeDefKind::Sum { .. } => (
                    k::TYPE_SUM,
                    TYPE_SUM_HEAD,
                    (TYPE_ORDERED, values::boolean(td.ordered)),
                ),
                TypeDefKind::Newtype { base, .. } => {
                    let over = self.schema.show_ty_with(base, &td.params);
                    (
                        k::TYPE_NEWTYPE,
                        TYPE_NEWTYPE_HEAD,
                        (TYPE_OVER, Value::Text(over)),
                    )
                }
            };
            let fields = vec![
                (head.id(), key(id.raw())),
                (head.module(), self.module_of(def)),
                (head.name(), Value::Text(td.name.clone())),
                (head.params(), Value::I32(td.params.len() as i32)),
                (head.dropped(), self.none()),
                own,
            ];
            self.put(&codec, ctor, fields);
        }
    }

    fn ctors(&mut self) {
        let codec = Codec::<ConstructorRow>::new(self.schema);
        for (c, id) in self.schema.ids().ctor_ids() {
            let (def, at) = (c.def(), c.index());
            let td = self.schema.ty(def);
            let name = match &td.kind {
                TypeDefKind::Sum { .. } => td.ctor_def(c).name.clone(),
                TypeDefKind::Newtype { .. } => td.name.clone(),
            };
            let fields = vec![
                (CONSTRUCTOR_ID, key(id.raw())),
                (
                    CONSTRUCTOR_SUM,
                    key(held_id(self.schema.ids().type_id(def)).raw()),
                ),
                (CONSTRUCTOR_NAME, Value::Text(name)),
                (CONSTRUCTOR_POSITION, Value::I32(at as i32 + 1)),
                (CONSTRUCTOR_DROPPED, self.none()),
            ];
            self.put(&codec, k::CONSTRUCTOR, fields);
        }
    }

    fn fields(&mut self) {
        let codec = Codec::<FieldRow>::new(self.schema);
        let mut held: Vec<_> = self.schema.ids().field_ids().collect();
        held.sort_by_key(|(f, _)| self.schema.field_order(*f));
        for (f, id) in held {
            let td = self.schema.ty(f.def());
            let declared = td.field_def(f);
            let field_type = self.schema.show_ty_with(&declared.ty, &td.params);
            let (ctor, head, own) = match f.ctor() {
                None => (
                    k::FIELD_HEADER,
                    FIELD_HEADER_HEAD,
                    (
                        FIELD_SUM,
                        key(held_id(self.schema.ids().type_id(f.def())).raw()),
                    ),
                ),
                Some(c) => (
                    k::FIELD_PAYLOAD,
                    FIELD_PAYLOAD_HEAD,
                    (FIELD_CTOR, key(type_ctor_id(self.schema, f.def(), c).raw())),
                ),
            };
            let fields = vec![
                (head.id(), key(id.raw())),
                (head.name(), Value::Text(declared.name.clone())),
                (head.field_type(), Value::Text(field_type)),
                (head.position(), Value::I32(f.pos() as i32 + 1)),
                (head.dropped(), self.none()),
                own,
            ];
            self.put(&codec, ctor, fields);
        }
    }

    fn relations(&mut self) {
        let catalog = (self.schema)
            .relation(engine_relation(self.schema, engine_rel::TYPES))
            .module;
        let mut rows = Vec::new();
        for (r, id) in self.schema.ids().relation_ids() {
            let rel = self.schema.relation(r);
            let module = &self.schema.module(rel.module).name;
            let kind = match (rel.volatile, rel.module == catalog) {
                (true, _) => k::REL_VOLATILE,
                (false, true) => k::REL_CATALOG,
                (false, false) => k::REL_BASE,
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
            rows.push((id, module, view.name.clone(), sum, k::REL_STORED));
        }
        let codec = Codec::<RelationRow>::new(self.schema);
        for (id, module, name, sum, kind) in rows {
            let fields = vec![
                (RELATION_ID, key(id.raw())),
                (RELATION_MODULE, Value::Text(module)),
                (RELATION_NAME, Value::Text(name)),
                (RELATION_SUM, key(sum.raw())),
                (RELATION_KIND.field(), tag(&codec, RELATION_KIND, kind)),
                (RELATION_DROPPED, self.none()),
            ];
            self.put(&codec, k::RELATION, fields);
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
        let codec = Codec::<ComputedRow>::new(self.schema);
        for (id, r, name, field_type, text) in rows {
            let fields = vec![
                (COMPUTED_ID, key(id.raw())),
                (
                    COMPUTED_RELATION,
                    key(held_id(self.schema.ids().relation_id(r)).raw()),
                ),
                (COMPUTED_NAME, Value::Text(name)),
                (COMPUTED_FIELD_TYPE, Value::Text(field_type)),
                (COMPUTED_TEXT, Value::Text(text)),
                (COMPUTED_DROPPED, self.none()),
            ];
            self.put(&codec, k::COMPUTED, fields);
        }
    }

    fn sets(&mut self) {
        let sets = Codec::<NamedSetRow>::new(self.schema);
        let members = Codec::<SetMemberRow>::new(self.schema);
        for (r, name, id) in self.schema.ids().set_ids() {
            let id = id.raw();
            let rel = key(held_id(self.schema.ids().relation_id(r)).raw());
            let fields = vec![
                (NAMED_SET_ID, key(id)),
                (NAMED_SET_RELATION, rel),
                (NAMED_SET_NAME, Value::Text(name.to_string())),
                (NAMED_SET_DROPPED, self.none()),
            ];
            self.put(&sets, k::NAMED_SET, fields);
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
                self.put(
                    &members,
                    k::SET_MEMBER,
                    vec![(SET_MEMBER_SET, key(id)), (SET_MEMBER_CTOR, ctor)],
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
        let codec = Codec::<DefaultRow>::new(self.schema);
        for (id, r, field, text) in rows {
            let fields = vec![
                (DEFAULT_ID, key(id.raw())),
                (
                    DEFAULT_RELATION,
                    key(held_id(self.schema.ids().relation_id(r)).raw()),
                ),
                (DEFAULT_FIELD, key(field)),
                (DEFAULT_TEXT, Value::Text(text)),
                (DEFAULT_DROPPED, self.none()),
            ];
            self.put(&codec, k::DEFAULT, fields);
        }
    }

    fn constraints(&mut self) {
        let declared = self.schema.ids().constraint_ids().map(|(i, id)| {
            let c = self.schema.constraint(i);
            (id, c.module, c.name.clone(), c.file)
        });
        let checks = self.schema.ids().check_ids().map(|(i, id)| {
            let c = self.schema.check(i);
            (id, c.module, c.name.clone(), c.file)
        });
        let all: Vec<_> = declared.chain(checks).collect();
        let codec = Codec::<ConstraintRow>::new(self.schema);
        for (id, module, name, file) in all {
            let fields = vec![
                (CONSTRAINT_ID, key(id.raw())),
                (
                    CONSTRAINT_MODULE,
                    Value::Text(self.schema.module(module).name.clone()),
                ),
                (CONSTRAINT_NAME, Value::Text(name)),
                (
                    CONSTRAINT_TEXT,
                    Value::Text(self.source.decls[file.0 as usize].text.clone()),
                ),
                (CONSTRAINT_DROPPED, self.none()),
            ];
            self.put(&codec, k::CONSTRAINT, fields);
        }
    }

    fn indexes(&mut self, texts: &ElementTexts) {
        let codec = Codec::<IndexRow>::new(self.schema);
        let ctors = Codec::<IndexCtorRow>::new(self.schema);
        for (id, spec) in indexes(self.schema, texts) {
            let fields = vec![
                (INDEX_ID, key(id)),
                (
                    INDEX_RELATION,
                    key(held_id(self.schema.ids().relation_id(spec.relation)).raw()),
                ),
                (INDEX_KIND.field(), tag(&codec, INDEX_KIND, spec.kind)),
                (INDEX_CLAUSE, Value::Text(spec.clause.clone())),
                (INDEX_DROPPED, self.none()),
            ];
            self.put(&codec, k::INDEX, fields);
            let def = self.schema.relation(spec.relation).elem_def;
            for c in &spec.ctors {
                let ctor = key(type_ctor_id(self.schema, def, *c).raw());
                self.put(
                    &ctors,
                    k::INDEX_CTOR,
                    vec![(INDEX_CTOR_INDEX, key(id)), (INDEX_CTOR_CTOR, ctor)],
                );
            }
        }
    }

    fn ctor_states(&mut self) {
        let codec = Codec::<CtorStateRow>::new(self.schema);
        for (r, id) in self.schema.ids().relation_ids() {
            let def = self.schema.relation(r).elem_def;
            for c in all_ctors(self.schema, r) {
                let stage = self.stage(&codec, CTOR_STATE_STAGE, self.schema.stage(r, c));
                let fields = vec![
                    (CTOR_STATE_RELATION, key(id.raw())),
                    (
                        CTOR_STATE_CTOR,
                        key(type_ctor_id(self.schema, def, c).raw()),
                    ),
                    (CTOR_STATE_STAGE.field(), stage),
                ];
                self.put(&codec, k::CTOR_STATE, fields);
            }
        }
    }

    fn functions(&mut self) {
        let functions = self
            .source
            .functions()
            .unwrap_or_else(|ds| panic!("the functions of a checked source resolve: {ds:?}"));
        let codec = Codec::<FunctionRow>::new(self.schema);
        for f in functions {
            let (ctor, head, hashed, own) = match &f.code {
                Code::Language { body, canonical } => (
                    k::FUNCTION_LANGUAGE,
                    LANGUAGE_HEAD,
                    canonical,
                    vec![(LANGUAGE_BODY, Value::Text(body.clone()))],
                ),
                Code::Aggregate { body, canonical } => (
                    k::FUNCTION_AGGREGATE,
                    AGGREGATE_HEAD,
                    canonical,
                    vec![(AGGREGATE_BODY, Value::Text(body.clone()))],
                ),
                Code::Host { symbol, version } => (
                    k::FUNCTION_HOST,
                    HOST_HEAD,
                    version,
                    vec![
                        (HOST_SYMBOL, Value::Text(symbol.clone())),
                        (HOST_VERSION, Value::Text(version.clone())),
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
                (head.id(), key(f.id)),
                (head.module(), Value::Text(f.module)),
                (head.name(), Value::Text(f.name)),
                (head.signature(), Value::Text(f.signature)),
                (head.deterministic(), values::boolean(f.deterministic)),
                (
                    head.hash(),
                    Value::Bytes(blake3::hash(hashed.as_bytes()).as_bytes().to_vec()),
                ),
                (head.dropped(), self.none()),
            ];
            fields.extend(own);
            self.put(&codec, ctor, fields);
        }
    }

    /// Add the rows of `migration_ops` and `migration_clauses` for every migration of the log.
    #[cfg(test)]
    fn log(&mut self) {
        let log = oplog::Log::new(self.schema);
        for l in &self.source.log {
            let ops = oplog::operators(&log, self.source, l.number);
            self.migration(&log, l.number, &ops);
        }
    }

    fn migration(&mut self, log: &oplog::Log, version: u64, ops: &[oplog::OperatorRow]) {
        let number = key(version);
        for (at, (op, clauses)) in ops.iter().enumerate() {
            let ordinal = Value::I32(i32::try_from(at + 1).expect("an ordinal fits 31 bits"));
            for (j, body) in clauses.iter().enumerate() {
                let clause = Value::I32(i32::try_from(j + 1).expect("a clause fits 31 bits"));
                let fields = vec![
                    (OP_CLAUSE_MIGRATION, number.clone()),
                    (OP_CLAUSE_ORDINAL, ordinal.clone()),
                    (OP_CLAUSE_CLAUSE, clause),
                    (OP_CLAUSE_BODY.field(), body.clone()),
                ];
                self.put(log.clauses(), k::OP_CLAUSE, fields);
            }
            let fields = vec![
                (MIGRATION_OP_MIGRATION, number.clone()),
                (MIGRATION_OP_ORDINAL, ordinal),
                (MIGRATION_OP_OP.field(), op.clone()),
            ];
            self.put(log.ops(), k::MIGRATION_OP, fields);
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
        let codec = Codec::<ValueStateRow>::new(self.schema);
        for def in sums {
            for c in 0..self.schema.ty(def).ctors().len() as u32 {
                let found = self.schema.ty(def).ctor_at(c);
                let Some(id) = found.and_then(|c| self.schema.ids().ctor_id(c)) else {
                    continue;
                };
                let stage = self.stage(&codec, VALUE_STATE_STAGE, self.schema.value_stage(def, c));
                let fields = vec![
                    (VALUE_STATE_CTOR, key(id.raw())),
                    (VALUE_STATE_STAGE.field(), stage),
                ];
                self.put(&codec, k::VALUE_STATE, fields);
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

/// Returns the value of nullary constructor `id` of sum field `s` of `codec`'s relation.
fn tag<T: Decode>(codec: &Codec<T>, s: Sum, id: CatCtorId) -> Value {
    Value::Sum {
        ctor: codec.tag(s, id),
        fields: Vec::new(),
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
