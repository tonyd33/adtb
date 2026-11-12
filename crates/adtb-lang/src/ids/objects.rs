use std::collections::BTreeMap;

use crate::ast::{self, Decl, RelationClause};
use crate::lex::{Tok, lex};
use crate::schema::{FieldRef, Schema, TypeDefKind, ValueDef};
use crate::span::{FileId, Span};
use crate::types::*;

use super::key::{KeyKind, ObjectKey, Owner, Part};
use super::{FIRST_LOG_ID, Slot, Space};

/// What holds an object, before the ids of the objects are known.
#[derive(Clone, Debug)]
enum Holder {
    Module(String),
    Object(Slot),
}

/// A part of an object's key, before the ids of the objects it names are known.
#[derive(Clone, Debug)]
enum Named {
    Name(String),
    Token(String),
    Next,
    Field(Slot),
    Ctor(Slot),
    Set(Slot),
}

/// One object of a user module, with what its key names.
#[derive(Clone, Debug)]
pub struct Object {
    slot: Slot,
    kind: KeyKind,
    owner: Holder,
    parts: Vec<Named>,
}

impl Object {
    pub fn space(&self) -> Space {
        self.slot.space()
    }

    pub fn slot(&self) -> &Slot {
        &self.slot
    }

    /// Returns the round of [`assign`] that gives the object its id: after every object its key
    /// names.
    fn round(&self) -> u8 {
        match self.space() {
            Space::Shared | Space::Field | Space::Function | Space::Constraint => 0,
            Space::Set | Space::Computed => 1,
            Space::Index | Space::Default => 2,
        }
    }

    /// Returns the object's key, `id` giving the id of each object it names.
    fn key(&self, id: &dyn Fn(&Slot) -> u64) -> ObjectKey {
        let owner = match &self.owner {
            Holder::Module(m) => Owner::Module(m.clone()),
            Holder::Object(s) => Owner::Object(id(s)),
        };
        let parts = self
            .parts
            .iter()
            .map(|p| match p {
                Named::Name(n) => Part::Name(n.clone()),
                Named::Token(t) => Part::Token(t.clone()),
                Named::Next => Part::Next,
                Named::Field(s) => Part::Field(id(s)),
                Named::Ctor(s) => Part::Ctor(id(s)),
                Named::Set(s) => Part::Set(id(s)),
            })
            .collect();
        ObjectKey::new(self.kind, owner, parts)
    }
}

/// Returns the key and the id of each of `objs`: the id `known` holds for its key, else the next
/// id `next` gives its space.
///
/// Preconditions: `objs` are in creation order; no two keys of `known` hold one id.
/// Postconditions: the keys are distinct, objects agreeing on kind, owner and parts told apart by
/// how many of them come before; each space's new ids follow the order of `objs`.
pub fn assign(
    objs: &[Object],
    known: &BTreeMap<ObjectKey, u64>,
    next: &mut BTreeMap<Space, u64>,
) -> Vec<(ObjectKey, u64)> {
    let mut ids: BTreeMap<Slot, u64> = BTreeMap::new();
    let mut seen: BTreeMap<ObjectKey, usize> = BTreeMap::new();
    let mut out: Vec<Option<(ObjectKey, u64)>> = vec![None; objs.len()];
    for round in 0..3 {
        for (at, o) in objs.iter().enumerate().filter(|(_, o)| o.round() == round) {
            let key = o.key(&|s| {
                *ids.get(s)
                    .expect("an object a key names takes its id in an earlier round or before it")
            });
            let n = seen.entry(key.clone()).or_default();
            let key = key.nth(*n);
            *n += 1;
            let id = known.get(&key).copied().unwrap_or_else(|| {
                let n = next.entry(o.space()).or_insert(FIRST_LOG_ID);
                *n += 1;
                *n - 1
            });
            ids.insert(o.slot.clone(), id);
            out[at] = Some((key, id));
        }
    }
    out.into_iter()
        .map(|k| k.expect("every object takes an id in one round"))
        .collect()
}

/// The objects of a schema, in creation order.
struct Objects<'a> {
    schema: &'a Schema,
    text: &'a dyn Fn(FileId, Span) -> String,
    out: Vec<Object>,
}

impl Objects<'_> {
    fn push(&mut self, slot: Slot, kind: KeyKind, owner: Holder, parts: Vec<Named>) {
        self.out.push(Object {
            slot,
            kind,
            owner,
            parts,
        });
    }

    fn named(&mut self, slot: Slot, kind: KeyKind, owner: Holder, name: &str) {
        self.push(slot, kind, owner, vec![Named::Name(name.to_string())]);
    }
}

/// Every object the user modules of `files` declare, in creation order, `text` giving the source
/// text of a span of a file for the keys of uniques and indexes.
pub fn objects(
    schema: &Schema,
    files: &[(FileId, ast::SchemaFile)],
    text: &dyn Fn(FileId, Span) -> String,
) -> Vec<Object> {
    let mut o = Objects {
        schema,
        text,
        out: Vec::new(),
    };
    for (file, sf) in files {
        if file.is_engine() {
            continue;
        }
        let Some(mid) = schema.module_named(&sf.module.text) else {
            continue;
        };
        let m = &sf.module.text;
        let module = || Holder::Module(m.clone());
        let values = &schema.module(mid).values;
        for d in &sf.decls {
            match d {
                Decl::Type(t) => o.type_objects(mid, m, t),
                Decl::Relation(r) => {
                    if let Some(&ValueDef::Relation(rel)) = values.get(&r.name.name.text) {
                        o.relation_objects(*file, m, rel, r);
                    }
                }
                Decl::Set(s) => {
                    if let Some(&ValueDef::Relation(rel)) = values.get(&s.relation.text) {
                        o.named(
                            Slot::Set(rel, s.name.text.clone()),
                            KeyKind::Set,
                            Holder::Object(Slot::Relation(rel)),
                            &s.name.text,
                        );
                    }
                }
                Decl::Fn(f) => {
                    if let Some(&ValueDef::Function(id)) = values.get(&f.name.text) {
                        o.named(
                            Slot::Function(id),
                            KeyKind::Function,
                            module(),
                            &f.name.text,
                        );
                    }
                }
                Decl::Aggregate(a) => {
                    if let Some(&ValueDef::Aggregate(i)) = values.get(&a.name.text) {
                        o.named(
                            Slot::Aggregate(i as usize),
                            KeyKind::Function,
                            module(),
                            &a.name.text,
                        );
                    }
                }
                Decl::Constraint(c) => {
                    if let Some(&ValueDef::Constraint(i)) = values.get(&c.name.text) {
                        o.named(
                            Slot::Constraint(i as usize),
                            KeyKind::Constraint,
                            module(),
                            &c.name.text,
                        );
                    }
                }
                Decl::Check(c) => {
                    if let Some(&ValueDef::Check(i)) = values.get(&c.name.text) {
                        o.named(
                            Slot::Check(i as usize),
                            KeyKind::Constraint,
                            module(),
                            &c.name.text,
                        );
                    }
                }
                Decl::Store(s) => {
                    if let Some(&ValueDef::Function(f)) = values.get(&s.view.name.text) {
                        o.view_objects(files, *file, m, f, s);
                    }
                }
            }
        }
    }
    o.out
}

impl Objects<'_> {
    fn type_objects(&mut self, mid: ModuleId, m: &str, t: &ast::TypeDecl) {
        let schema = self.schema;
        let def = if t.name.text == "Principal" {
            Some(schema.principal)
        } else {
            schema.module(mid).types.get(&t.name.text).copied()
        };
        let Some(def) = def else { return };
        let td = schema.ty(def);
        let tn = &td.name;
        self.named(
            Slot::Type(def),
            KeyKind::Type,
            Holder::Module(m.to_string()),
            tn,
        );
        let newtype = matches!(td.kind, TypeDefKind::Newtype { .. });
        for c in td.ctor_refs() {
            let cn = if newtype { tn } else { &td.ctor_def(c).name };
            self.named(
                Slot::Ctor(c),
                KeyKind::Ctor,
                Holder::Object(Slot::Type(def)),
                cn,
            );
        }
        for (f, d) in td.field_refs() {
            let owner = match f.ctor().and_then(|c| td.ctor_at(c)) {
                Some(c) => Slot::Ctor(c),
                None => Slot::Type(def),
            };
            self.named(
                Slot::Field(f),
                KeyKind::Field,
                Holder::Object(owner),
                &d.name,
            );
        }
    }

    fn relation_objects(&mut self, file: FileId, m: &str, rel: RelId, r: &ast::RelationDecl) {
        self.named(
            Slot::Relation(rel),
            KeyKind::Relation,
            Holder::Module(m.to_string()),
            &r.name.name.text,
        );
        self.clause_objects(file, &r.clauses, Clauses::Relation(rel));
        for i in self.schema.implied_indexes(rel) {
            self.push(
                Slot::Implied(rel, i.field_ref()),
                KeyKind::Implied,
                Holder::Object(Slot::Relation(rel)),
                vec![Named::Field(Slot::Field(i.field_ref()))],
            );
        }
    }

    fn view_objects(
        &mut self,
        files: &[(FileId, ast::SchemaFile)],
        file: FileId,
        m: &str,
        f: FnId,
        s: &ast::StoreDecl,
    ) {
        let vn = &s.view.name.text;
        self.named(
            Slot::View(f),
            KeyKind::Relation,
            Holder::Module(m.to_string()),
            vn,
        );
        if matches!(
            &self.schema.functions[f.0 as usize].result,
            Ty::Set(elem) if !matches!(**elem, Ty::Data { .. })
        ) {
            self.named(
                Slot::ViewType(f),
                KeyKind::Type,
                Holder::Object(Slot::View(f)),
                vn,
            );
            self.named(
                Slot::ViewCtor(f),
                KeyKind::Ctor,
                Holder::Object(Slot::ViewType(f)),
                vn,
            );
            for name in anonymous_row(files, m, vn) {
                self.named(
                    Slot::ViewField(f, name.clone()),
                    KeyKind::Field,
                    Holder::Object(Slot::ViewCtor(f)),
                    &name,
                );
            }
        }
        self.clause_objects(file, &s.clauses, Clauses::View(f));
    }

    /// The objects a relation's or a stored view's clauses create, in written order.
    fn clause_objects(&mut self, file: FileId, clauses: &[RelationClause], of: Clauses) {
        let owner = || Holder::Object(of.slot());
        let (mut ix, mut dx, mut cx) = (0, 0, 0);
        for c in clauses {
            match (c, of) {
                (RelationClause::Key(names, generate), _) => {
                    let key = match of {
                        Clauses::Relation(r) => Slot::Key(r),
                        Clauses::View(f) => Slot::ViewKey(f),
                    };
                    let parts = names
                        .iter()
                        .map(|n| self.element(self.row_field(of, Range::Key, &n.text), &n.text))
                        .collect();
                    self.push(key, KeyKind::Key, owner(), parts);
                    if let (Some(_), Clauses::Relation(r)) = (generate, of) {
                        self.push(Slot::Generated(r), KeyKind::Generated, owner(), Vec::new());
                    }
                }
                (
                    RelationClause::Unique(fs, scope)
                    | RelationClause::Index {
                        fields: fs, scope, ..
                    },
                    _,
                ) => {
                    let kind = match c {
                        RelationClause::Unique(..) => KeyKind::Unique,
                        RelationClause::Index { interval: true, .. } => KeyKind::IntervalIndex,
                        _ => KeyKind::Index,
                    };
                    let slot = match of {
                        Clauses::Relation(r) => Slot::Index(r, ix),
                        Clauses::View(f) => Slot::ViewIndex(f, ix),
                    };
                    let parts = self.index_parts(file, fs, scope.as_ref(), of, ix);
                    self.push(slot, kind, owner(), parts);
                    ix += 1;
                }
                (RelationClause::Default { ctor, field, .. }, Clauses::Relation(r)) => {
                    for f in default_fields(self.schema, r, ctor.as_ref(), &field.text) {
                        self.push(
                            Slot::Default(r, dx, f),
                            KeyKind::Default,
                            owner(),
                            vec![self.element(Some(Slot::Field(f)), &field.text)],
                        );
                    }
                    dx += 1;
                }
                (RelationClause::Computed(n, _, _), Clauses::Relation(r)) => {
                    self.named(Slot::Computed(r, cx), KeyKind::Computed, owner(), &n.text);
                    cx += 1;
                }
                _ => {}
            }
        }
    }

    /// Returns the parts of the key of the unique or index at position `ix` of `of`'s clauses:
    /// its elements, each a field of the row it ranges over or a token, then each constructor or
    /// set of its scope.
    fn index_parts(
        &self,
        file: FileId,
        fields: &[ast::Expr],
        scope: Option<&ast::CtorScope>,
        of: Clauses,
        ix: usize,
    ) -> Vec<Named> {
        let names: Vec<&ast::Ident> = match scope {
            None => Vec::new(),
            Some(ast::CtorScope::One(c)) => vec![c],
            Some(ast::CtorScope::Many(cs)) => cs.iter().collect(),
        };
        let scope: Vec<Named> = names.iter().map(|n| self.scope_part(of, &n.text)).collect();
        let listed = !scope.is_empty() && scope.iter().all(|p| matches!(p, Named::Ctor(_)));
        let range = if listed {
            Range::Listed(ix)
        } else {
            Range::Unlisted
        };
        let mut out = Vec::new();
        for e in fields {
            let text = (self.text)(file, e.span);
            let mut after_dot = false;
            for t in lex(&text).0 {
                let word = &text[t.span.range()];
                let field = matches!(t.tok, Tok::Lower | Tok::Upper | Tok::Kw(_))
                    .then_some(())
                    .filter(|()| !after_dot)
                    .and_then(|()| self.row_field(of, range, word));
                if t.tok != Tok::Eof {
                    out.push(self.element(field, word));
                }
                after_dot = t.tok == Tok::Dot;
            }
            out.push(Named::Next);
        }
        out.extend(scope);
        out
    }

    /// Returns the field named `name` of the row of `of` that `range` ranges over: a header field,
    /// else the field of the first constructor in the range that has one. An unlisted range of a
    /// sum of several constructors has only the header.
    fn row_field(&self, of: Clauses, range: Range, name: &str) -> Option<Slot> {
        let schema = self.schema;
        let (def, listed) = match of {
            Clauses::Relation(r) => {
                let rd = schema.relation(r);
                let scope = match range {
                    Range::Listed(ix) => rd.indexes.get(ix).and_then(|i| i.scope.clone()),
                    Range::Key | Range::Unlisted => None,
                };
                (rd.elem_def, scope)
            }
            Clauses::View(f) => match &schema.functions[f.0 as usize].result {
                Ty::Set(elem) => match &**elem {
                    Ty::Data { def, .. } => (*def, None),
                    _ => return Some(Slot::ViewField(f, name.to_string())),
                },
                _ => return None,
            },
        };
        if def.0 == u32::MAX {
            return None;
        }
        let td = schema.ty(def);
        if let Some(f) = td.field(None, name) {
            return Some(Slot::Field(f));
        }
        if matches!(range, Range::Unlisted) && td.ctors().len() > 1 {
            return None;
        }
        td.ctor_refs()
            .filter(|c| listed.as_ref().is_none_or(|r| r.contains(&c.index())))
            .find_map(|c| td.field(Some(c), name))
            .map(Slot::Field)
    }

    /// Returns the key part of an element `name` that names `field`: the field when a user module
    /// declares it, else the name.
    fn element(&self, field: Option<Slot>, name: &str) -> Named {
        let user = |f: &FieldRef| !self.schema.module(self.schema.ty(f.def()).module).engine;
        match field {
            Some(Slot::Field(f)) if !user(&f) => Named::Token(name.to_string()),
            Some(slot) => Named::Field(slot),
            None => Named::Token(name.to_string()),
        }
    }

    /// Returns what a scope's name `name` names: a constructor of the row's type, else a named
    /// set of the relation.
    fn scope_part(&self, of: Clauses, name: &str) -> Named {
        let schema = self.schema;
        let def = match of {
            Clauses::Relation(r) => Some(schema.relation(r).elem_def),
            Clauses::View(f) => match &schema.functions[f.0 as usize].result {
                Ty::Set(elem) => match &**elem {
                    Ty::Data { def, .. } => Some(*def),
                    _ => None,
                },
                _ => None,
            },
        }
        .filter(|d| d.0 != u32::MAX);
        if let Some(c) = def.and_then(|d| schema.ty(d).ctor_named(name)) {
            return Named::Ctor(Slot::Ctor(c));
        }
        match of {
            Clauses::Relation(r) if schema.relation(r).sets.iter().any(|s| s.name == name) => {
                Named::Set(Slot::Set(r, name.to_string()))
            }
            _ => Named::Token(name.to_string()),
        }
    }
}

/// Returns the fields of the rows of view `view` of module `module`, as its result type writes
/// them: none unless its element type is an anonymous record.
fn anonymous_row(files: &[(FileId, ast::SchemaFile)], module: &str, view: &str) -> Vec<String> {
    let result = files
        .iter()
        .filter(|(_, sf)| sf.module.text == module)
        .flat_map(|(_, sf)| &sf.decls)
        .find_map(|d| match d {
            Decl::Fn(f) if f.name.text == view => Some(&f.result),
            _ => None,
        });
    let Some(ast::Type {
        kind: ast::TypeKind::Named { args, .. },
        ..
    }) = result
    else {
        return Vec::new();
    };
    let [ast::TypeArg::Type(elem)] = args.as_slice() else {
        return Vec::new();
    };
    match &elem.kind {
        ast::TypeKind::Record(fs) => fs.iter().map(|f| f.name.text.clone()).collect(),
        _ => Vec::new(),
    }
}

/// Returns the fields of relation `r` a default of `field`, of constructor `ctor` when one is
/// named, fills: the header's field, else the field of each constructor with it in declaration
/// order.
fn default_fields(
    schema: &Schema,
    r: RelId,
    ctor: Option<&ast::Ident>,
    field: &str,
) -> Vec<FieldRef> {
    let def = schema.relation(r).elem_def;
    if def.0 == u32::MAX {
        return Vec::new();
    }
    let td = schema.ty(def);
    if let Some(f) = td.field(None, field) {
        return vec![f];
    }
    td.ctor_refs()
        .filter(|&c| ctor.is_none_or(|n| n.text == td.ctor_def(c).name))
        .filter_map(|c| td.field(Some(c), field))
        .collect()
}

/// The constructors an element of a key, unique or index ranges over.
#[derive(Clone, Copy)]
enum Range {
    /// Every constructor of the row, for a key.
    Key,
    /// The constructors the scope of the unique or index at this position of the clauses lists.
    Listed(usize),
    /// Every constructor, or those of a named set: the clause lists none.
    Unlisted,
}

/// What declares a run of clauses.
#[derive(Clone, Copy)]
enum Clauses {
    Relation(RelId),
    View(FnId),
}

impl Clauses {
    fn slot(self) -> Slot {
        match self {
            Clauses::Relation(r) => Slot::Relation(r),
            Clauses::View(f) => Slot::View(f),
        }
    }
}
