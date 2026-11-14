use std::cell::OnceCell;
use std::collections::{BTreeMap, BTreeSet};

use crate::ast::{self, Decl, RelationClause};
use crate::lex::{Tok, lex};
use crate::schema::{CtorRef, FieldRef, Schema, TypeDefKind, ValueDef};
use crate::span::{FileId, Span};
use crate::types::*;

use super::key::{KeyKind, ObjectKey, Owner, Part, PathStep};
use super::named::anonymous_row;
use super::reach::{Correspondence, Replacement};
use super::{CatalogIds, FIRST_LOG_ID, Slot, Space};

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
    /// A field of an engine type, by its fixed id.
    FixedField(u64),
    /// A constructor of an engine type, by its fixed id.
    FixedCtor(CtorRef, u64),
    Reach(Vec<Vec<Step>>),
}

/// A step of a path an index element reaches.
#[derive(Clone, Debug)]
enum Step {
    Of(Slot),
    Fixed(PathStep),
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
    /// names. No key names an object of the last round.
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
            Holder::Object(s) => Owner::Object(s.space(), id(s)),
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
                Named::FixedField(f) => Part::Field(*f),
                Named::FixedCtor(_, c) => Part::Ctor(*c),
                Named::Reach(paths) => Part::Reach(
                    paths
                        .iter()
                        .map(|p| {
                            p.iter()
                                .map(|s| match s {
                                    Step::Of(s @ (Slot::Ctor(_) | Slot::ViewCtor(_))) => {
                                        PathStep::Ctor(id(s))
                                    }
                                    Step::Of(s) => PathStep::Field(id(s)),
                                    Step::Fixed(p) => *p,
                                })
                                .collect()
                        })
                        .collect(),
                ),
            })
            .collect();
        ObjectKey::new(self.kind, owner, parts)
    }
}

/// Returns the key and the id of each of `objs`: a named object takes the id `known` holds for
/// its key; a unique, index or default takes the id `known` holds for its key, else for a key
/// whose elements correspond to it through `replaced`, else the next id `next` gives its space.
///
/// Preconditions: `objs` are in creation order; `known` holds the key of every named object of
/// `objs`; no two keys of `known` hold one id; `replaced`
/// holds what the operators taking the schema of `known` to that of `objs` replaced; `created`
/// holds the objects of `known` those operators created, and `readded` the fields they retired
/// and added again under their names.
/// Postconditions: the keys are distinct, objects agreeing on kind, owner and parts told apart by
/// how many of them come before; no two objects of a space take one id; each space's new ids
/// follow the order of `objs`.
pub fn assign(
    objs: &[Object],
    known: &BTreeMap<ObjectKey, u64>,
    next: &mut BTreeMap<Space, u64>,
    replaced: &[Replacement],
    (created, readded): (&BTreeSet<(Space, u64)>, &BTreeSet<u64>),
) -> Vec<(ObjectKey, u64)> {
    let mut ids: BTreeMap<Slot, u64> = BTreeMap::new();
    let mut seen: BTreeMap<ObjectKey, usize> = BTreeMap::new();
    let mut out: Vec<Option<(ObjectKey, u64)>> = vec![None; objs.len()];
    let mut taken: BTreeSet<(Space, u64)> = BTreeSet::new();
    let mut fresh = |space: Space| {
        let n = next.entry(space).or_insert(FIRST_LOG_ID);
        *n += 1;
        *n - 1
    };
    for round in 0..3 {
        let mut pending = Vec::new();
        for (at, o) in objs.iter().enumerate().filter(|(_, o)| o.round() == round) {
            let key = o.key(&|s| {
                *ids.get(s)
                    .expect("an object a key names takes its id in an earlier round or before it")
            });
            let n = seen.entry(key.clone()).or_default();
            let key = key.nth(*n);
            *n += 1;
            let id = match known.get(&key) {
                Some(&id) if round < 2 => id,
                None if round < 2 => panic!("no id for named object {key:?}"),
                Some(&id) if taken.insert((o.space(), id)) => id,
                _ => {
                    pending.push((at, key));
                    continue;
                }
            };
            ids.insert(o.slot.clone(), id);
            out[at] = Some((key, id));
        }
        if pending.is_empty() {
            continue;
        }
        let matched = Correspondence::new(known, (created, readded), &ids, replaced);
        for (at, key) in pending {
            let o = &objs[at];
            let reached = key.parts().iter().any(|p| matches!(p, Part::Reach(_)));
            let found = known.iter().find(|(k, id)| {
                reached && !taken.contains(&(o.space(), **id)) && matched.same(k, &key)
            });
            let id = match found {
                Some((_, &id)) => {
                    taken.insert((o.space(), id));
                    id
                }
                None => fresh(o.space()),
            };
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
    engine: OnceCell<CatalogIds>,
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
        engine: OnceCell::new(),
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
            let field = self.element(Some(Slot::Field(i.field_ref())), &i.field);
            self.push(
                Slot::Implied(rel, i.field_ref()),
                KeyKind::Implied,
                Holder::Object(Slot::Relation(rel)),
                vec![field],
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
        if let Some(row) = view_row(files, m, vn) {
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
            for name in row {
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
                    let row = self.row(of);
                    let every = row.ctors(self.schema, &[]);
                    let parts = names
                        .iter()
                        .flat_map(|n| self.element_parts(&row, &every, &[n.text.as_str()]))
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
                    let parts = self.index_parts(file, fs, scope.as_ref(), of);
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

    /// Returns the parts of the key of a unique or index of `of`'s clauses: its elements, each a
    /// run of names reaching fields of the rows its scope reaches, or a token, then each
    /// constructor or set of its scope.
    fn index_parts(
        &self,
        file: FileId,
        fields: &[ast::Expr],
        scope: Option<&ast::CtorScope>,
        of: Clauses,
    ) -> Vec<Named> {
        let names: Vec<&ast::Ident> = match scope {
            None => Vec::new(),
            Some(ast::CtorScope::One(c)) => vec![c],
            Some(ast::CtorScope::Many(cs)) => cs.iter().collect(),
        };
        let scope: Vec<Named> = names.iter().map(|n| self.scope_part(of, &n.text)).collect();
        let row = self.row(of);
        let ctors = row.ctors(self.schema, &scope);
        let mut out = Vec::new();
        for e in fields {
            let text = (self.text)(file, e.span);
            let toks: Vec<(Tok, &str)> = lex(&text)
                .0
                .into_iter()
                .filter(|t| t.tok != Tok::Eof)
                .map(|t| (t.tok, &text[t.span.range()]))
                .collect();
            let mut at = 0;
            while at < toks.len() {
                let name = |t: Tok| matches!(t, Tok::Lower | Tok::Upper | Tok::Kw(_));
                let after_dot = at > 0 && toks[at - 1].0 == Tok::Dot;
                if !name(toks[at].0) || after_dot {
                    out.push(Named::Token(toks[at].1.to_string()));
                    at += 1;
                    continue;
                }
                let mut run = vec![toks[at].1];
                while at + 2 * run.len() < toks.len()
                    && toks[at + 2 * run.len() - 1].0 == Tok::Dot
                    && name(toks[at + 2 * run.len()].0)
                {
                    run.push(toks[at + 2 * run.len()].1);
                }
                out.extend(self.element_parts(&row, &ctors, &run));
                at += 2 * run.len() - 1;
            }
            out.push(Named::Next);
        }
        out.extend(scope);
        out
    }

    /// Returns the parts of a dotted run of names `run` of an element over the rows of `ctors`:
    /// the paths its longest resolving prefix reaches, then a token for each dot and name after
    /// it; a token for each when no prefix resolves.
    fn element_parts(&self, row: &Row, ctors: &[CtorRef], run: &[&str]) -> Vec<Named> {
        let (paths, resolved) = match row {
            Row::Data(def) => self.reach(*def, ctors, run),
            Row::Anonymous(f) => (
                vec![vec![Step::Of(Slot::ViewField(*f, run[0].to_string()))]],
                1,
            ),
            Row::None => (Vec::new(), 0),
        };
        let mut out = Vec::new();
        if resolved > 0 {
            out.push(Named::Reach(paths));
        } else {
            out.push(Named::Token(run[0].to_string()));
        }
        for w in &run[resolved.max(1)..] {
            out.push(Named::Token(".".to_string()));
            out.push(Named::Token((*w).to_string()));
        }
        out
    }

    /// Returns the paths of fields and constructors that the longest prefix of `run` reaches from the rows of `ctors` of type `def`, with the length of that prefix: a
    /// name is a field of the header or of a constructor reached, else, after a field, a
    /// constructor of its type.
    fn reach(&self, def: TypeId, ctors: &[CtorRef], run: &[&str]) -> (Vec<Vec<Step>>, usize) {
        let schema = self.schema;
        let mut level: Vec<(Vec<Step>, Option<TypeId>, Vec<CtorRef>)> =
            vec![(Vec::new(), Some(def), ctors.to_vec())];
        let mut resolved = 0;
        for w in run {
            let mut next = Vec::new();
            let typed = level.iter().filter_map(|(p, d, c)| d.map(|d| (p, d, c)));
            for (path, def, ctors) in typed {
                let td = schema.ty(def);
                let fields: BTreeSet<FieldRef> = match td.field(None, w) {
                    Some(f) => BTreeSet::from([f]),
                    None => ctors.iter().filter_map(|&c| td.field(Some(c), w)).collect(),
                };
                for f in fields {
                    let inner = match &td.field_def(f).ty {
                        Ty::Data { def, .. } => Some(*def),
                        _ => None,
                    };
                    let every = inner
                        .map(|d| schema.ty(d).ctor_refs().collect())
                        .unwrap_or_default();
                    next.push((extended(path, self.field_step(f)), inner, every));
                }
                let after_field = matches!(
                    path.last(),
                    Some(Step::Of(Slot::Field(_)) | Step::Fixed(PathStep::Field(_)))
                );
                if let Some(c) = td
                    .ctor_named(w)
                    .filter(|c| after_field && ctors.contains(c))
                {
                    next.push((extended(path, self.ctor_step(c)), Some(def), vec![c]));
                }
            }
            if next.is_empty() {
                break;
            }
            level = next;
            resolved += 1;
        }
        let paths = level.into_iter().map(|(p, _, _)| p).collect();
        (if resolved == 0 { Vec::new() } else { paths }, resolved)
    }

    /// Returns the row of the rows `of`'s clauses range over.
    fn row(&self, of: Clauses) -> Row {
        let schema = self.schema;
        let def = match of {
            Clauses::Relation(r) => schema.relation(r).elem_def,
            Clauses::View(f) => match &schema.functions[f.0 as usize].result {
                Ty::Set(elem) => match &**elem {
                    Ty::Data { def, .. } => *def,
                    _ => return Row::Anonymous(f),
                },
                _ => return Row::None,
            },
        };
        Row::Data(def)
    }

    /// Returns the key part of an element `name` that names `field`: the field, by its fixed id
    /// when an engine module declares it; the name when it names none.
    fn element(&self, field: Option<Slot>, name: &str) -> Named {
        match field {
            Some(Slot::Field(f)) if self.engine_type(f.def()) => {
                Named::FixedField(self.fixed_field(f))
            }
            Some(slot) => Named::Field(slot),
            None => Named::Token(name.to_string()),
        }
    }

    fn engine_type(&self, def: TypeId) -> bool {
        self.schema.module(self.schema.ty(def).module).engine
    }

    fn engine_ids(&self) -> &CatalogIds {
        self.engine.get_or_init(|| super::engine(self.schema))
    }

    fn fixed_field(&self, f: FieldRef) -> u64 {
        self.engine_ids()
            .field_id(f)
            .expect("every field of an engine type has a fixed id")
            .raw()
    }

    fn fixed_ctor(&self, c: CtorRef) -> u64 {
        self.engine_ids()
            .ctor_id(c)
            .expect("every constructor of an engine type has a fixed id")
            .raw()
    }

    fn field_step(&self, f: FieldRef) -> Step {
        if self.engine_type(f.def()) {
            Step::Fixed(PathStep::Field(self.fixed_field(f)))
        } else {
            Step::Of(Slot::Field(f))
        }
    }

    fn ctor_step(&self, c: CtorRef) -> Step {
        if self.engine_type(c.def()) {
            Step::Fixed(PathStep::Ctor(self.fixed_ctor(c)))
        } else {
            Step::Of(Slot::Ctor(c))
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
        };
        if let Some(c) = def.and_then(|d| schema.ty(d).ctor_named(name)) {
            if self.engine_type(c.def()) {
                return Named::FixedCtor(c, self.fixed_ctor(c));
            }
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

/// Returns the fields of the rows of view `view` of module `module` when they are of no declared
/// type, as its result type writes them.
fn view_row(files: &[(FileId, ast::SchemaFile)], module: &str, view: &str) -> Option<Vec<String>> {
    files
        .iter()
        .filter(|(_, sf)| sf.module.text == module)
        .flat_map(|(_, sf)| &sf.decls)
        .find_map(|d| match d {
            Decl::Fn(f) if f.name.text == view => Some(&f.result),
            _ => None,
        })
        .and_then(anonymous_row)
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
    let td = schema.ty(def);
    if let Some(f) = td.field(None, field) {
        return vec![f];
    }
    td.ctor_refs()
        .filter(|&c| ctor.is_none_or(|n| n.text == td.ctor_def(c).name))
        .filter_map(|c| td.field(Some(c), field))
        .collect()
}

/// The rows a run of clauses ranges over.
enum Row {
    /// Rows of a declared type.
    Data(TypeId),
    /// The anonymous rows of a stored view.
    Anonymous(FnId),
    None,
}

impl Row {
    /// Returns the constructors of the row that scope `scope` reaches: those it lists and the
    /// members of the sets it names, every constructor for no scope.
    fn ctors(&self, schema: &Schema, scope: &[Named]) -> Vec<CtorRef> {
        let Row::Data(def) = *self else {
            return Vec::new();
        };
        let td = schema.ty(def);
        if scope.is_empty() {
            return td.ctor_refs().collect();
        }
        let mut out = BTreeSet::new();
        for part in scope {
            match part {
                Named::Ctor(Slot::Ctor(c)) | Named::FixedCtor(c, _) => {
                    out.insert(*c);
                }
                Named::Set(Slot::Set(r, name)) => {
                    let set = schema.relation(*r).sets.iter().find(|s| s.name == *name);
                    out.extend(
                        set.into_iter()
                            .flat_map(|s| s.ctors.iter())
                            .filter_map(|&c| td.ctor_at(c)),
                    );
                }
                _ => {}
            }
        }
        out.into_iter().collect()
    }
}

fn extended(path: &[Step], step: Step) -> Vec<Step> {
    let mut out = path.to_vec();
    out.push(step);
    out
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
