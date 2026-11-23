//! Hold a source as declarations keyed by catalog id, and apply migration operators to it as
//! functions of that model.

use std::collections::{BTreeMap, BTreeSet};

use super::{LogGrant, Source, State};
use crate::ast::{self, MigrationFile, OperatorKind};
use crate::ids::{FIRST_LOG_ID, ObjectKey, Space};
use crate::schema::{Adder, AheadKind};

mod admin;
mod apply;
mod arms;
mod clause;
mod ctor;
mod decl;
mod field;
mod lift;
mod moved;
mod naming;
mod op;
mod print;
mod reselect;
mod revert;
mod run;
mod slots;
mod source;
mod structural;
mod term;
#[cfg(test)]
mod tests;
mod undo;

pub(super) use apply::Migrated;
use decl::{cat_ctor, cat_field, cat_rel, cat_type};
pub(super) use revert::{logged_file, replay};
use term::Term;

/// A catalog object, by the space of its id and the id.
pub(super) type Obj = (Space, u64);

/// A source as a model: every declaration of the user modules, each object held by its catalog
/// id, with the write states, grants and id counters of the source.
///
/// A name written inside a declaration is held as the ids of the objects it resolves to, so a
/// declaration prints each name it writes as that object's name now.
#[derive(Clone, Debug, Default)]
pub(super) struct Model {
    /// The uniques, indexes and defaults added while the declarations did not build, by their
    /// space and id.
    unbuilt_clauses: BTreeSet<Obj>,
    /// The computed fields added or redefined while the declarations did not build, by their
    /// relation variable and id.
    computed_unbuilt: BTreeSet<(u64, u64)>,
    /// The canonical form the first build after it gave each computed field added or redefined
    /// while the declarations did not build, by its id.
    gave_computed: BTreeMap<u64, Option<String>>,
    /// What the first build after it gave each unique, index or default added while the
    /// declarations did not build, by its space and id.
    gave_clauses: BTreeMap<Obj, run::Gave>,
    modules: BTreeSet<String>,
    installed: BTreeSet<String>,
    /// The declarations in source order; declaration `i` prints file `i`.
    decls: Vec<Decl>,
    types: BTreeMap<u64, TypeM>,
    ctors: BTreeMap<u64, CtorM>,
    fields: BTreeMap<u64, FieldM>,
    relations: BTreeMap<u64, RelM>,
    sets: BTreeMap<u64, SetM>,
    /// Functions and aggregates.
    functions: BTreeMap<u64, Body>,
    /// Constraints and checks.
    denials: BTreeMap<u64, Body>,
    /// Stored views, by the id of the relation variable each holds.
    stores: BTreeMap<u64, StoreM>,
    states: BTreeMap<(u64, u64), State>,
    value_states: BTreeMap<u64, State>,
    grants: Vec<LogGrant>,
    next: BTreeMap<Space, u64>,
    version: u64,
    /// The log: each migration's number, module, text and count of operators.
    log: Vec<LogM>,
    /// Each relation variable an operator of the migration being applied adds.
    ahead: Vec<AheadM>,
    /// The packages and release data an `install` or `adopt tzdb` may name, by file.
    packages: BTreeMap<String, String>,
    /// The grants a structural operator of the migration being applied could not carry, which
    /// a later grant of it must state.
    unstated: Vec<crate::evolve::carry::Unstated>,
    /// Each declaration an operator added while the declarations did not build, by its key in
    /// [`Item::Held`], until they build.
    held: BTreeMap<u64, HeldM>,
    /// The declarations whose terms an operator wrote as text while the declarations did not
    /// build, each lifted again once they build.
    unlifted: Vec<Item>,
    /// The declaration each held one became once the declarations built, by its key.
    lifted: BTreeMap<u64, Item>,
    /// The id of each named object a held declaration declares, by its key, given when the
    /// operator adding it applied.
    held_ids: BTreeMap<ObjectKey, u64>,
    /// The log as the source the model was lifted from holds it, with the source before each
    /// migration.
    logged: Vec<super::Logged>,
    /// While the operators being applied replay a logged migration as an inverse, the id each
    /// object the inverse created again takes, by the id it had.
    replaying: Option<super::revert::Recreated>,
    /// The text each declaration takes within the step of the structural operator being applied
    /// as an inverse, set before that operator first builds.
    restating: Vec<(Item, String)>,
    /// The key the next arm an operator writes takes.
    next_arm: u64,
    /// The target of each arm an operator wrote while the declarations did not build, by its
    /// key, until they build.
    arms: BTreeMap<u64, Item>,
    /// The canonical form each arm of [`Model::arms`] took once the declarations built, `None`
    /// when its target no longer held it or a name of it did not resolve, or why it could not
    /// be read.
    arms_written: BTreeMap<u64, Result<Option<String>, String>>,
    /// The names of the text of the migration being applied that its operators copied.
    naming: naming::Naming,
}

/// A declaration held as its text: the name it is found by and its text without the module
/// header.
#[derive(Clone, Debug)]
struct HeldM {
    name: String,
    text: String,
    /// Whether it takes its first write states once it lifts.
    enabling: bool,
}

/// A relation variable an operator of the migration being applied adds: named `name` in module
/// `module`, `typed` when the migration also adds its type. Or a type or function a refused
/// operator of it would have declared, by `kind`.
#[derive(Clone, Debug)]
struct AheadM {
    module: String,
    name: String,
    typed: bool,
    by: Adder,
    added: Added,
    kind: AheadKind,
}

/// Whether the operator adding an [`AheadM`] has applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Added {
    Not,
    Applying,
    /// It was refused: its name stays bound, poisoned, to the end of the migration.
    Refused,
    /// It has, adding the relation variable of this id.
    As(u64),
}

/// A migration of the log, as its number, module, text and count of operators.
#[derive(Clone, Debug, PartialEq, Eq)]
struct LogM {
    number: u64,
    module: String,
    text: String,
    operators: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Decl {
    module: String,
    item: Item,
}

/// A declaration, by the id of the object it declares.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Item {
    Type(u64),
    Relation(u64),
    Set(u64),
    Fn(u64),
    Aggregate(u64),
    Constraint(u64),
    Check(u64),
    /// A stored view, by its relation variable.
    Store(u64),
    /// A declaration of this kind not lifted yet, by its key in [`Model::held`].
    Held(op::Kind, u64),
}

#[derive(Clone, Debug)]
struct TypeM {
    name: String,
    params: Vec<String>,
    ordered: bool,
    /// The header's fields; `None` when the declaration writes no header.
    header: Option<Vec<u64>>,
    body: TypeBody,
}

#[derive(Clone, Debug)]
enum TypeBody {
    /// A newtype over `base`, with its one constructor.
    New {
        ctor: u64,
        base: Term,
        domain: Option<Term>,
    },
    /// A record, its one constructor named after the type.
    Record {
        ctor: u64,
    },
    Ctors(Vec<u64>),
}

#[derive(Clone, Debug)]
struct CtorM {
    ty: u64,
    name: String,
    /// `None` for a constructor written without braces.
    fields: Option<Vec<u64>>,
}

#[derive(Clone, Debug)]
struct FieldM {
    /// The type, for a header field, or the constructor.
    owner: u64,
    name: String,
    ty: Term,
    action: Option<Term>,
}

#[derive(Clone, Debug)]
struct RelM {
    name: String,
    ty: Term,
    clauses: Vec<ClauseM>,
    /// The index each reference field implies, by the field.
    implied: BTreeMap<Implied, u64>,
}

/// A reference field that implies an index: by its catalog id, or, for a field of a prelude or
/// engine type, which holds no catalog id, by the qualified type's and the field's names.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Implied {
    Field(u64),
    Builtin(String, String),
}

/// A clause of a relation variable. An id is `None` until the schema the clause is in builds,
/// when the clause takes the next id of its space.
#[derive(Clone, Debug)]
enum ClauseM {
    Key {
        id: Option<u64>,
        fields: Vec<Term>,
        /// Whether the fields are written in parentheses.
        listed: bool,
        generate: Option<(String, Option<u64>)>,
    },
    Volatile,
    Index {
        id: Option<u64>,
        kind: IndexKind,
        elems: Vec<Term>,
        scope: Option<Scope>,
    },
    Default {
        ctor: Option<Term>,
        field: Term,
        value: Term,
        /// One default for each field the clause fills, by the field's id.
        ids: Vec<(u64, u64)>,
    },
    Computed {
        id: Option<u64>,
        name: String,
        ty: Term,
        expr: Term,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum IndexKind {
    Unique,
    Plain,
    Interval,
}

/// The `in` scope of a unique or index: constructors and named sets.
#[derive(Clone, Debug)]
struct Scope {
    parts: Vec<Term>,
    /// Whether it is written in parentheses.
    listed: bool,
}

#[derive(Clone, Debug)]
struct SetM {
    relation: u64,
    name: String,
    members: Vec<Term>,
}

/// A function, aggregate, constraint or check: its name and the text after it.
#[derive(Clone, Debug)]
struct Body {
    name: String,
    rest: Term,
}

/// A stored view: the function it stores, the text after the view's name, and the ids of the
/// objects it declares.
#[derive(Clone, Debug)]
struct StoreM {
    function: u64,
    /// The function's name as the declaration writes it.
    written: String,
    rest: Term,
    ids: BTreeMap<ViewSlot, u64>,
}

/// An object a stored view declares.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ViewSlot {
    Key,
    Index(usize),
    Type,
    Ctor,
    Field(String),
}

impl LogM {
    fn of(l: &super::Logged) -> LogM {
        LogM {
            number: l.number,
            module: l.module.clone(),
            text: l.text.clone(),
            operators: l.operators.len(),
        }
    }
}

impl Model {
    /// Hold each relation variable an `add relation` of `m` adds as one its operator has not
    /// added yet.
    fn expecting(&mut self, m: &MigrationFile) {
        let module = m.module.text.as_str();
        let types: BTreeSet<&str> = m
            .operators
            .iter()
            .filter_map(|op| match &op.kind {
                OperatorKind::AddType(t) => Some(t.name.text.as_str()),
                _ => None,
            })
            .collect();
        self.ahead = m
            .operators
            .iter()
            .enumerate()
            .filter_map(|(k, op)| match &op.kind {
                OperatorKind::AddRelation(r) => Some(AheadM {
                    module: module.to_string(),
                    name: r.name.name.text.clone(),
                    typed: matches!(
                        &r.ty.kind,
                        ast::TypeKind::Named { name, .. }
                            if name.module.as_ref().is_none_or(|m| m.text == module)
                                && types.contains(name.name.text.as_str())
                    ),
                    by: Adder::Operator(k),
                    added: Added::Not,
                    kind: AheadKind::Relation,
                }),
                _ => None,
            })
            .collect();
    }

    /// Set whether the operator `by` has added its relation variable.
    fn mark(&mut self, by: Adder, added: Added) {
        for e in self.ahead.iter_mut().filter(|e| e.by == by) {
            e.added = added;
        }
    }

    /// Refuse when an id taken so far, by an operator or a build, lies past its space's width:
    /// 32 bits for types, constructors, relation variables and indexes, 63 for the others.
    fn ids_fit(&self) -> op::Res<()> {
        for (&space, &next) in &self.next {
            let (bits, kind) = match space {
                Space::Shared => (32, "type, constructor and relation"),
                Space::Index => (32, "unique and index"),
                Space::Field => (63, "field"),
                Space::Set => (63, "named set"),
                Space::Constraint => (63, "constraint and check"),
                Space::Function => (63, "function"),
                Space::Default => (63, "default"),
                Space::Computed => (63, "computed field"),
            };
            if next.saturating_sub(1) >> bits != 0 {
                return op::refuse(format!("no {kind} id is left: those ids fit {bits} bits"));
            }
        }
        Ok(())
    }

    /// Returns the next id of `space`, taking it.
    /// Give each key, unique, index and computed field an operator made or changed without an id
    /// a fresh one, in the order of their relation variables' ids and their clauses.
    fn identify(&mut self) {
        let mut next = std::mem::take(&mut self.next);
        let mut fresh = |space: Space| {
            let n = next.entry(space).or_insert(FIRST_LOG_ID);
            *n += 1;
            *n - 1
        };
        for rel in self.relations.values_mut() {
            for c in &mut rel.clauses {
                match c {
                    ClauseM::Key { id: id @ None, .. } | ClauseM::Index { id: id @ None, .. } => {
                        *id = Some(fresh(Space::Index));
                    }
                    ClauseM::Computed { id: id @ None, .. } => *id = Some(fresh(Space::Computed)),
                    _ => {}
                }
            }
        }
        self.next = next;
    }

    fn fresh(&mut self, space: Space) -> u64 {
        let n = self.next.entry(space).or_insert(FIRST_LOG_ID);
        *n += 1;
        *n - 1
    }

    /// Returns the name the object of `obj` has now, `None` for an object the model does not
    /// hold or one without a name.
    fn name(&self, (space, id): Obj) -> Option<&str> {
        match space {
            Space::Shared => self
                .types
                .get(&id)
                .map(|t| t.name.as_str())
                .or_else(|| self.ctors.get(&id).map(|c| c.name.as_str()))
                .or_else(|| self.relations.get(&id).map(|r| r.name.as_str())),
            Space::Field => self.fields.get(&id).map(|f| f.name.as_str()),
            Space::Set => self.sets.get(&id).map(|s| s.name.as_str()),
            Space::Function => self.functions.get(&id).map(|f| f.name.as_str()),
            Space::Constraint => self.denials.get(&id).map(|c| c.name.as_str()),
            Space::Computed => self.relations.values().find_map(|r| {
                r.clauses.iter().find_map(|c| match c {
                    ClauseM::Computed { id: x, name, .. } if *x == Some(id) => Some(name.as_str()),
                    _ => None,
                })
            }),
            Space::Index | Space::Default => None,
        }
    }

    /// Returns the module of the declaration declaring the object of `obj`.
    fn module_of(&self, (space, id): Obj) -> Option<&str> {
        let item = match space {
            Space::Shared if self.types.contains_key(&id) => Item::Type(id),
            Space::Shared if self.relations.contains_key(&id) => Item::Relation(id),
            Space::Shared if self.stores.contains_key(&id) => Item::Store(id),
            Space::Shared => Item::Type(self.ctors.get(&id)?.ty),
            Space::Set => Item::Relation(self.sets.get(&id)?.relation),
            Space::Function => {
                return self
                    .decls
                    .iter()
                    .find(|d| matches!(d.item, Item::Fn(x) | Item::Aggregate(x) if x == id))
                    .map(|d| d.module.as_str());
            }
            Space::Constraint => {
                return self
                    .decls
                    .iter()
                    .find(|d| matches!(d.item, Item::Constraint(x) | Item::Check(x) if x == id))
                    .map(|d| d.module.as_str());
            }
            Space::Field | Space::Index | Space::Default | Space::Computed => return None,
        };
        self.decls
            .iter()
            .find(|d| d.item == item)
            .map(|d| d.module.as_str())
    }

    /// Returns the position of the declaration `item`.
    fn decl_at(&self, item: Item) -> Option<usize> {
        self.decls.iter().position(|d| d.item == item)
    }

    /// Returns the type a field belongs to.
    fn field_type(&self, field: u64) -> Option<u64> {
        let owner = self.fields.get(&field)?.owner;
        if self.types.contains_key(&owner) {
            Some(owner)
        } else {
            self.ctors.get(&owner).map(|c| c.ty)
        }
    }

    /// Returns whether a declaration holds a name of dropped objects no operator added again.
    fn dropped_names(&self) -> bool {
        let mut dropped = false;
        self.clone().terms_mut(&mut |t| dropped |= t.dropped());
        dropped
    }

    /// Hold each name of an object `before` held and this model does not by the object's key,
    /// and each such name whose key this model holds by the object of that key.
    fn rebind(&mut self, before: &Model) {
        let was: BTreeMap<Obj, ObjectKey> = before
            .named_keys()
            .into_iter()
            .map(|(k, id)| ((k.kind().space(), id), k))
            .collect();
        let now: BTreeMap<ObjectKey, Obj> = self
            .named_keys()
            .into_iter()
            .map(|(k, id)| {
                let o = (k.kind().space(), id);
                (k, o)
            })
            .collect();
        let live: BTreeSet<Obj> = now.values().copied().collect();
        self.terms_mut(&mut |t| t.rebind(&was, &now, &|o| live.contains(&o)));
    }

    /// Call `f` on every term of every declaration.
    fn terms_mut(&mut self, f: &mut dyn FnMut(&mut Term)) {
        for t in self.types.values_mut() {
            if let TypeBody::New { base, domain, .. } = &mut t.body {
                f(base);
                domain.iter_mut().for_each(&mut *f);
            }
        }
        for x in self.fields.values_mut() {
            f(&mut x.ty);
            x.action.iter_mut().for_each(&mut *f);
        }
        for r in self.relations.values_mut() {
            f(&mut r.ty);
            for c in &mut r.clauses {
                match c {
                    ClauseM::Key { fields, .. } => fields.iter_mut().for_each(&mut *f),
                    ClauseM::Index { elems, scope, .. } => {
                        elems.iter_mut().for_each(&mut *f);
                        if let Some(s) = scope {
                            s.parts.iter_mut().for_each(&mut *f);
                        }
                    }
                    ClauseM::Default {
                        ctor, field, value, ..
                    } => {
                        ctor.iter_mut().for_each(&mut *f);
                        f(field);
                        f(value);
                    }
                    ClauseM::Computed { ty, expr, .. } => {
                        f(ty);
                        f(expr);
                    }
                    ClauseM::Volatile => {}
                }
            }
        }
        for s in self.sets.values_mut() {
            s.members.iter_mut().for_each(&mut *f);
        }
        for b in self.functions.values_mut().chain(self.denials.values_mut()) {
            f(&mut b.rest);
        }
        for s in self.stores.values_mut() {
            f(&mut s.rest);
        }
    }

    /// Returns the name stored view `v` prints: its function's now, or as written once the
    /// function is gone.
    fn store_name(&self, v: u64) -> &str {
        let st = &self.stores[&v];
        self.functions
            .get(&st.function)
            .map_or(st.written.as_str(), |f| f.name.as_str())
    }

    /// Returns the constructors of type `t`, in declaration order.
    fn ctors_of(&self, t: u64) -> Vec<u64> {
        match self.types.get(&t).map(|t| &t.body) {
            Some(TypeBody::New { ctor, .. } | TypeBody::Record { ctor }) => vec![*ctor],
            Some(TypeBody::Ctors(cs)) => cs.clone(),
            None => Vec::new(),
        }
    }

    /// Returns every field of type `t`: the header's, then each constructor's.
    fn fields_of(&self, t: u64) -> Vec<u64> {
        let Some(ty) = self.types.get(&t) else {
            return Vec::new();
        };
        let mut out: Vec<u64> = ty.header.iter().flatten().copied().collect();
        for c in self.ctors_of(t) {
            out.extend(self.ctors[&c].fields.iter().flatten().copied());
        }
        out
    }
}

/// Returns the model of `src`, or why it has none.
pub(super) fn of(src: &Source) -> Result<Model, String> {
    lift::source(src)
}

/// Apply migration `m`, whose text is `text`, to the model of `src` as
/// [`Model::migrate_keeping`] does.
///
/// Fail with an internal diagnostic when `src` does not lift or the model fails on the migration.
pub(super) fn migrated(
    src: &Source,
    m: &MigrationFile,
    text: &str,
    kept: &[super::Kept],
    accepting: bool,
    naming: bool,
) -> Result<Migrated, crate::span::Diag> {
    of(src)
        .map_err(|e| fault(format!("the source does not lift: {e}")))?
        .migrate_keeping(m, text, kept, accepting, naming)
        .map_err(fault)
}

/// Returns the internal diagnostic of a failure `why` of the migration model.
pub(super) fn fault(why: String) -> crate::span::Diag {
    crate::span::Diag::internal(
        crate::span::Span::default(),
        format!("the migration model fails: {why}"),
    )
}
