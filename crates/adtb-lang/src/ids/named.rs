use std::collections::{BTreeMap, BTreeSet};

use crate::ast::{self, Decl, RelationClause};
use crate::schema::TypeName;
use crate::span::FileId;

use super::key::{KeyKind, ObjectKey, Owner};
use super::{FIRST_LOG_ID, Space};

/// What holds a named object: a module, an object listed before it, or the relation variable of
/// a module of that name.
#[derive(Clone, Debug)]
enum Holder {
    Module(String),
    Object(usize),
    Relation(String, String),
}

/// A named object of a parsed source.
#[derive(Clone, Debug)]
struct Named {
    kind: KeyKind,
    owner: Holder,
    name: String,
}

impl Named {
    /// Returns the round that gives the object its id: after every object its owner is.
    fn round(&self) -> u8 {
        match self.kind.space() {
            Space::Set | Space::Computed => 1,
            _ => 0,
        }
    }
}

/// Returns whether `kind` names an object by its owner and name alone.
pub fn is_named(kind: KeyKind) -> bool {
    !matches!(kind.space(), Space::Index | Space::Default)
}

/// Returns the key and the id of each named object `files` declare: the id `known` holds for its
/// key, else the next id `next` gives its space, which `created` records.
///
/// Preconditions: no two keys of `known` hold one id.
/// Postconditions: each space's new ids follow declaration order; a set whose relation variable
/// is not declared takes no id.
pub fn named_ids(
    files: &[(FileId, ast::SchemaFile)],
    known: &BTreeMap<ObjectKey, u64>,
    next: &mut BTreeMap<Space, u64>,
    created: &mut BTreeSet<(Space, u64)>,
) -> Vec<(ObjectKey, u64)> {
    let objs = listed(files);
    let mut ids: Vec<Option<u64>> = vec![None; objs.len()];
    let mut out = Vec::new();
    let mut seen: BTreeMap<ObjectKey, usize> = BTreeMap::new();
    let mut taken: BTreeSet<(Space, u64)> = BTreeSet::new();
    for round in 0..2 {
        for (at, o) in objs.iter().enumerate().filter(|(_, o)| o.round() == round) {
            let owner = match &o.owner {
                Holder::Module(m) => Some(Owner::Module(m.clone())),
                Holder::Object(j) => ids[*j].map(|id| Owner::Object(objs[*j].kind.space(), id)),
                Holder::Relation(m, r) => objs
                    .iter()
                    .position(|x| {
                        x.kind == KeyKind::Relation
                            && x.name == *r
                            && matches!(&x.owner, Holder::Module(xm) if xm == m)
                    })
                    .and_then(|j| ids[j])
                    .map(|id| Owner::Object(Space::Shared, id)),
            };
            let Some(owner) = owner else { continue };
            let key = ObjectKey::named(o.kind, owner, &o.name);
            let n = seen.entry(key.clone()).or_default();
            let key = key.nth(*n);
            *n += 1;
            let space = o.kind.space();
            let id = match known.get(&key) {
                Some(&id) if taken.insert((space, id)) => id,
                _ => {
                    let n = next.entry(space).or_insert(FIRST_LOG_ID);
                    *n += 1;
                    created.insert((space, *n - 1));
                    *n - 1
                }
            };
            ids[at] = Some(id);
            out.push((key, id));
        }
    }
    out
}

/// Returns the named objects of `files` in declaration order.
fn listed(files: &[(FileId, ast::SchemaFile)]) -> Vec<Named> {
    let mut out: Vec<Named> = Vec::new();
    let push = |out: &mut Vec<Named>, kind, owner, name: &str| {
        out.push(Named {
            kind,
            owner,
            name: name.to_string(),
        });
        out.len() - 1
    };
    let fns: BTreeMap<(&str, &str), &ast::Type> = files
        .iter()
        .flat_map(|(_, sf)| sf.decls.iter().map(move |d| (sf, d)))
        .filter_map(|(sf, d)| match d {
            Decl::Fn(f) => Some(((sf.module.text.as_str(), f.name.text.as_str()), &f.result)),
            _ => None,
        })
        .collect();
    for (_, sf) in files {
        let m = &sf.module.text;
        let module = || Holder::Module(m.clone());
        for d in &sf.decls {
            match d {
                Decl::Type(t) => {
                    let ty = push(&mut out, KeyKind::Type, module(), &t.name.text);
                    let ctors: Vec<(&str, &[ast::FieldType])> = match &t.body {
                        ast::TypeBody::New(..) => vec![(&t.name.text, &[])],
                        ast::TypeBody::Record(fs) => vec![(&t.name.text, fs)],
                        ast::TypeBody::Ctors(cs) => cs
                            .iter()
                            .map(|c| (c.name.text.as_str(), c.record.as_deref().unwrap_or(&[])))
                            .collect(),
                    };
                    let at: Vec<usize> = ctors
                        .iter()
                        .map(|(c, _)| push(&mut out, KeyKind::Ctor, Holder::Object(ty), c))
                        .collect();
                    for f in t.header.iter().flatten() {
                        push(&mut out, KeyKind::Field, Holder::Object(ty), &f.name.text);
                    }
                    for (c, (_, fs)) in at.into_iter().zip(ctors) {
                        for f in fs {
                            push(&mut out, KeyKind::Field, Holder::Object(c), &f.name.text);
                        }
                    }
                }
                Decl::Relation(r) => {
                    let rel = push(&mut out, KeyKind::Relation, module(), &r.name.name.text);
                    for c in &r.clauses {
                        if let RelationClause::Computed(n, _, _) = c {
                            push(&mut out, KeyKind::Computed, Holder::Object(rel), &n.text);
                        }
                    }
                }
                Decl::Set(s) => {
                    let owner = Holder::Relation(m.clone(), s.relation.text.clone());
                    push(&mut out, KeyKind::Set, owner, &s.name.text);
                }
                Decl::Fn(f) => {
                    push(&mut out, KeyKind::Function, module(), &f.name.text);
                }
                Decl::Aggregate(a) => {
                    push(&mut out, KeyKind::Function, module(), &a.name.text);
                }
                Decl::Constraint(c) => {
                    push(&mut out, KeyKind::Constraint, module(), &c.name.text);
                }
                Decl::Check(c) => {
                    push(&mut out, KeyKind::Constraint, module(), &c.name.text);
                }
                Decl::Store(s) => {
                    let vn = &s.view.name.text;
                    let Some(result) = fns.get(&(m.as_str(), vn.as_str())) else {
                        continue;
                    };
                    let view = push(&mut out, KeyKind::Relation, module(), vn);
                    if let Some(row) = anonymous_row(result) {
                        let ty = push(&mut out, KeyKind::Type, Holder::Object(view), vn);
                        let ctor = push(&mut out, KeyKind::Ctor, Holder::Object(ty), vn);
                        for f in row {
                            push(&mut out, KeyKind::Field, Holder::Object(ctor), &f);
                        }
                    }
                }
            }
        }
    }
    out
}

/// Returns the fields of the rows of a stored view whose function returns `result`, as it writes
/// them, when its rows are of no declared type: none unless its element type is an anonymous
/// record.
pub(super) fn anonymous_row(result: &ast::Type) -> Option<Vec<String>> {
    let ast::TypeKind::Named { name, args, .. } = &result.kind else {
        return None;
    };
    let [ast::TypeArg::Type(elem)] = args.as_slice() else {
        return None;
    };
    if name.module.is_some() || name.name.text != "Set" {
        return None;
    }
    match &elem.kind {
        ast::TypeKind::Record(fs) => Some(fs.iter().map(|f| f.name.text.clone()).collect()),
        ast::TypeKind::Named { name, .. } if declared(name) => None,
        _ => Some(Vec::new()),
    }
}

/// Returns whether `name` is neither a scalar nor a built-in type former.
fn declared(name: &ast::QName) -> bool {
    name.module.is_some() || TypeName::built_in(&name.name.text).is_none()
}
