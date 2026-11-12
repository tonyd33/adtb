use std::cell::OnceCell;

use super::*;
use crate::schema::FieldRef;

/// The names of a migration's text, each operand naming an object tied to its id, computed once.
pub(super) type Spelling = OnceCell<Result<Names, Vec<Diag>>>;

/// An operator of migration `logged` being inverted against `before`, the source before it,
/// `after`, the source it gives, and `start`, the source before the run of operators it ends or
/// belongs to.
#[derive(Clone, Copy)]
pub(super) struct Inverting<'a> {
    before: &'a Source,
    after: &'a Source,
    start: &'a Source,
    logged: &'a Logged,
    spelling: &'a Spelling,
    op: &'a Operator,
    fills: &'a [(ast::FieldPath, Fill)],
}

/// The parts of [`Inverting`] one logged migration's operators share.
#[derive(Clone, Copy)]
pub(super) struct Run<'a> {
    start: &'a Source,
    logged: &'a Logged,
    spelling: &'a Spelling,
    fills: &'a [(ast::FieldPath, Fill)],
}

impl<'a> Run<'a> {
    pub(super) fn new(
        start: &'a Source,
        logged: &'a Logged,
        spelling: &'a Spelling,
        fills: &'a [(ast::FieldPath, Fill)],
    ) -> Self {
        Run {
            start,
            logged,
            spelling,
            fills,
        }
    }

    pub(super) fn at(
        self,
        before: &'a Source,
        after: &'a Source,
        op: &'a Operator,
    ) -> Inverting<'a> {
        Inverting {
            before,
            after,
            start: self.start,
            logged: self.logged,
            spelling: self.spelling,
            op,
            fills: self.fills,
        }
    }
}

/// Returns the refusal of a revert whose log does not resolve, at `span`.
fn unresolved(span: Span, ds: &[Diag]) -> Vec<Diag> {
    ds.iter()
        .map(|d| Diag::new(span, format!("the log does not resolve: {}", d.message)))
        .collect()
}

impl Inverting<'_> {
    fn of(&self) -> String {
        self.text(self.op.span)
    }

    /// Returns `range` of the migration's text, each name of an object in it held by its id.
    fn op_text(&self, range: Range<usize>) -> R<Spelled> {
        let names = self
            .spelling
            .get_or_init(|| self.logged.spelling())
            .as_ref()
            .map_err(|ds| unresolved(self.op.span, ds))?;
        let mut out = Spelled::default();
        out.copy(&self.logged.text, range, names, 0);
        Ok(out)
    }

    /// Returns the migration's text from `q` to `end`, `q` qualified by its module.
    fn qualified(&self, q: &QName, end: usize) -> R<Spelled> {
        let mut out = match &q.module {
            Some(_) => Spelled::default(),
            None => Spelled::from(format!("{}::", self.logged.module)),
        };
        let start = q
            .module
            .as_ref()
            .map_or(q.name.span.start, |m| m.span.start);
        out.append(self.op_text(start as usize..end)?);
        Ok(out)
    }

    /// Returns field path `p` qualified by its module.
    fn path(&self, p: &ast::FieldPath) -> R<Spelled> {
        self.qualified(&p.ty, p.field.span.end as usize)
    }

    /// Returns `range` of the text of declaration `i` of `before`, each name of an object in it
    /// held by its id, as `before` resolves it, else as the source at the start of the run does
    /// when it holds the same text and `before` does not build.
    fn decl_range(&self, i: usize, range: Range<usize>) -> R<Spelled> {
        let d = &self.before.decls[i];
        let names = match self.before.names() {
            Ok(mut all) => all.swap_remove(i),
            Err(_) => {
                let j = self
                    .start
                    .decls
                    .iter()
                    .position(|s| s.module == d.module && s.text == d.text);
                match (j, self.start.names()) {
                    (Some(j), Ok(mut all)) => all.swap_remove(j),
                    _ => {
                        return Err(vec![Diag::new(
                            self.op.span,
                            "the log does not resolve the names of the declaration the operator changes",
                        )]);
                    }
                }
            }
        };
        let mut out = Spelled::default();
        out.copy(&d.text, range, &names, header(&d.module).len());
        Ok(out)
    }

    /// Returns the operator's text after its first word, `verb` in its place and the relation of
    /// `on` qualified by its module.
    fn clause(&self, verb: &str, on: &ast::RelationRef) -> R<Spelled> {
        let span = self.op.span.range();
        let text = &self.logged.text[span.clone()];
        let word = text.find(char::is_whitespace).unwrap_or(text.len());
        let rest = span.start + text.len() - text[word..].trim_start().len();
        let q = &on.relation;
        let rel = q
            .module
            .as_ref()
            .map_or(q.name.span.start, |m| m.span.start) as usize;
        let mut out = Spelled::from(format!("{verb} "));
        out.append(self.op_text(rest..rel)?);
        out.append(self.qualified(q, span.end)?);
        Ok(out)
    }

    /// Returns the declaration of `before` declaring the type of field path `p`, and the field.
    fn field_decl(&self, p: &ast::FieldPath) -> R<(usize, ast::FieldType)> {
        let m = self.module_of(&p.ty);
        let missing = || vec![Diag::new(self.op.span, "the log does not hold the field")];
        let i = self
            .before
            .find("type", &m, &p.ty.name.text)
            .ok_or_else(missing)?;
        let Some(Decl::Type(t)) = self.before.parsed(i) else {
            return Err(missing());
        };
        let record = matches!(t.body, ast::TypeBody::Record(_));
        decl_fields(&t)
            .into_iter()
            .find(|(c, f)| {
                let here = match (&p.ctor, c) {
                    (None, None) => true,
                    (None, Some(c)) => record && *c == t.name.text,
                    (Some(x), Some(c)) => x.text == *c,
                    (Some(_), None) => false,
                };
                here && f.name.text == p.field.text
            })
            .map(|(_, f)| (i, f.clone()))
            .ok_or_else(missing)
    }

    /// Returns the whole text of declaration `i` of `before` as [`Inverting::decl_range`] does.
    fn decl(&self, i: usize) -> R<Spelled> {
        self.decl_range(i, 0..self.before.decls[i].text.len())
    }

    /// Returns the step adding back the declaration of kind `kind` and name `name` of module `m`
    /// that the operator removes.
    fn restore(&self, kind: &'static str, m: &str, name: &str) -> R<Inverse> {
        let i = self.before.find(kind, m, name).ok_or_else(|| {
            vec![Diag::new(
                self.op.span,
                "the log does not hold the declaration the operator removes",
            )]
        })?;
        Ok(Inverse::Restore {
            module: m.to_string(),
            kind,
            name: name.to_string(),
            text: self.decl(i)?,
            of: self.of(),
        })
    }

    /// Returns the step removing the object of kind `kind` and name `name` of module `m` that
    /// the operator adds, by its id in `after`.
    fn unadd(&self, kind: KeyKind, m: &str, name: &ast::Ident) -> R<Inverse> {
        let key = ObjectKey::named(kind, Owner::Module(m.to_string()), &name.text);
        let id = self.after.ids.get(&key).copied().ok_or_else(|| {
            vec![Diag::new(
                self.op.span,
                "the log does not hold the object the operator adds",
            )]
        })?;
        let decl = match self.op.kind {
            OperatorKind::Store(_) => DeclOf::Store(id),
            _ => DeclOf::Object(kind, id),
        };
        Ok(Inverse::Redeclare {
            decl,
            text: None,
            of: self.of(),
        })
    }

    /// Returns the module `n` names, else the migration's.
    fn module_of(&self, n: &QName) -> String {
        n.module
            .as_ref()
            .map_or(self.logged.module.clone(), |m| m.text.clone())
    }

    fn text(&self, s: Span) -> String {
        self.logged.text[s.range()].to_string()
    }

    /// Returns the inverse renaming back to `to` the object `find` gives in the schema before the
    /// operator.
    fn renamed(
        &self,
        find: impl FnOnce(&Schema) -> Option<NamedId>,
        to: &ast::Ident,
    ) -> Option<Inverse> {
        let schema = self.before.schema().ok()?;
        Some(Inverse::Rename {
            object: find(&schema)?,
            to: to.text.clone(),
            of: self.text(self.op.span),
        })
    }

    /// Returns the inverse adding back set `name` of relation `r` that the operator drops.
    fn dropped_set(&self, name: &ast::Ident, r: &QName) -> Option<Inverse> {
        let m = self.module_of(r);
        let schema = self.before.schema().ok()?;
        let rel = relation_named(&schema, &m, &r.name.text)?;
        let i = self
            .before
            .find("set", &m, &format!("{}.{}", r.name.text, name.text))?;
        let Some(Decl::Set(d)) = self.before.parsed(i) else {
            return None;
        };
        let td = schema.ty(schema.relation(rel).elem_def);
        let members = d
            .members
            .iter()
            .map(|c| schema.ids.ctor_id(td.ctor_named(&c.text)?))
            .collect::<Option<Vec<CatCtorId>>>()?;
        Some(Inverse::AddSet {
            relation: schema.ids.relation_id(rel)?,
            name: name.text.clone(),
            members,
            of: self.text(self.op.span),
        })
    }

    /// Returns the step setting back the write state of each constructor `before` holds whose
    /// key `pick` takes, of a constructor relation when `relation`.
    fn states_back(&self, relation: bool, pick: impl Fn(&StateKey) -> bool) -> Option<Inverse> {
        let held = if relation {
            &self.before.states
        } else {
            &self.before.value_states
        };
        let states = held
            .iter()
            .filter(|(k, _)| pick(k))
            .map(|(k, v)| Some((StateOf::of(self.before, relation, k)?, Some(*v))))
            .collect::<Option<Vec<_>>>()?;
        Some(Inverse::States {
            states,
            of: self.text(self.op.span),
        })
    }
}

fn ops(s: String) -> R<Vec<Inverse>> {
    Ok(vec![Inverse::Ops(s.into())])
}

/// Returns the one step applying `text` as the inverse of operator text `of`.
fn apply(text: Spelled, of: String) -> R<Vec<Inverse>> {
    Ok(vec![Inverse::Apply { text, of }])
}

/// Returns type `t` of module `m` of `schema`.
fn type_named(schema: &Schema, m: &str, t: &str) -> Option<TypeId> {
    let mid = schema.module_named(m)?;
    schema.module(mid).types.get(t).copied()
}

/// Returns relation `r` of module `m` of `schema`.
fn relation_named(schema: &Schema, m: &str, r: &str) -> Option<RelId> {
    let mid = schema.module_named(m)?;
    match schema.module(mid).values.get(r) {
        Some(ValueDef::Relation(r)) => Some(*r),
        _ => None,
    }
}

/// Returns the field path `p` names in type `def` of `schema`.
fn field_named(schema: &Schema, def: TypeId, p: &ast::FieldPath) -> Option<FieldRef> {
    let td = schema.ty(def);
    let ctor = match &p.ctor {
        Some(c) => Some(td.ctor_named(&c.text)?),
        None if matches!(td.kind, TypeDefKind::Sum { record: true, .. }) => td.ctor_at(0),
        None => None,
    };
    td.field(ctor, &p.field.text)
}

impl Cx<'_> {
    /// Returns the one step `inv`, refusing when the log does not hold what the operator names.
    fn by_id(&self, inv: Option<Inverse>) -> R<Vec<Inverse>> {
        inv.map(|i| vec![i]).ok_or_else(|| {
            vec![Diag::new(
                self.span,
                "the log does not hold the object the operator names",
            )]
        })
    }

    /// The inverse of the operator `at` inverts.
    pub(super) fn inverse(&self, at: &Inverting) -> R<Vec<Inverse>> {
        let at = *at;
        let Inverting {
            before, after, op, ..
        } = at;
        match &op.kind {
            OperatorKind::AddModule(..)
            | OperatorKind::Install { .. }
            | OperatorKind::DropModule(..)
            | OperatorKind::Uninstall(..) => self.module_inverse(&at),
            OperatorKind::AddType(..)
            | OperatorKind::DropType(..)
            | OperatorKind::RenameType(..) => self.type_inverse(&at),
            OperatorKind::AddRelation(..)
            | OperatorKind::DropRelation(..)
            | OperatorKind::RenameRelation(..)
            | OperatorKind::AddSet(..)
            | OperatorKind::DropSet(..)
            | OperatorKind::RenameSet(..)
            | OperatorKind::AddUnique(..)
            | OperatorKind::AddIndex { .. }
            | OperatorKind::DropUnique(..)
            | OperatorKind::DropIndex { .. }
            | OperatorKind::Rekey { .. }
            | OperatorKind::AddDefault { .. }
            | OperatorKind::DropDefault { .. }
            | OperatorKind::AddComputed { .. }
            | OperatorKind::RedefineComputed { .. }
            | OperatorKind::DropComputed(..) => self.relation_inverse(&at),
            OperatorKind::AddConstructor { .. }
            | OperatorKind::RemoveConstructor(..)
            | OperatorKind::RenameConstructor(..)
            | OperatorKind::Enable(..)
            | OperatorKind::Disable(..)
            | OperatorKind::Deprecate(..)
            | OperatorKind::NestConstructor { .. }
            | OperatorKind::UnnestConstructor(..)
            | OperatorKind::SplitConstructor { .. }
            | OperatorKind::MergeConstructor { .. } => self.ctor_inverse(&at),
            OperatorKind::AddField { .. }
            | OperatorKind::AlterField { .. }
            | OperatorKind::DropField(..)
            | OperatorKind::RenameField(..)
            | OperatorKind::MoveField { .. } => self.field_inverse(&at),
            OperatorKind::DefineFn(..)
            | OperatorKind::RedefineFn(..)
            | OperatorKind::DropFn(..)
            | OperatorKind::DefineAggregate(..)
            | OperatorKind::DropAggregate(..)
            | OperatorKind::AddConstraint(..)
            | OperatorKind::DropConstraint(..)
            | OperatorKind::AddCheck(..)
            | OperatorKind::DropCheck(..)
            | OperatorKind::Store(..)
            | OperatorKind::Unstore(..) => self.body_inverse(&at),
            OperatorKind::Grant(_) | OperatorKind::Revoke(_) | OperatorKind::AdoptTzdb { .. } => {
                Ok(Vec::new())
            }
            OperatorKind::Script(_) => self.err("a `script` has no inverse"),
            OperatorKind::Revert { migration, .. } => {
                let n: u64 = migration.parse().unwrap_or(u64::MAX);
                let Some(k) = self.src.log.iter().position(|l| l.number == n) else {
                    return self.err(format!("migration {migration} is not in the log"));
                };
                let logged = &self.src.log[k];
                let names = logged.spelling().map_err(|ds| unresolved(op.span, &ds))?;
                let mut text = Spelled::default();
                text.copy(&logged.text, 0..logged.text.len(), &names, 0);
                Ok(vec![Inverse::Replay {
                    logged: Box::new(logged.clone()),
                    text,
                    again: self.src.recreated_by(k, before, after),
                    of: at.of(),
                }])
            }
        }
    }

    fn module_inverse(&self, at: &Inverting) -> R<Vec<Inverse>> {
        let Inverting { before, op, .. } = *at;
        match &op.kind {
            OperatorKind::AddModule(m) => Ok(vec![Inverse::Module {
                name: m.text.clone(),
                present: false,
                installed: false,
                decls: Vec::new(),
                of: at.of(),
            }]),
            OperatorKind::Install { name, .. } => Ok(vec![Inverse::Module {
                name: name.text.clone(),
                present: false,
                installed: true,
                decls: Vec::new(),
                of: at.of(),
            }]),
            OperatorKind::DropModule(m) | OperatorKind::Uninstall(m) => Ok(vec![Inverse::Module {
                name: m.text.clone(),
                present: true,
                installed: before.installed.contains(&m.text),
                decls: (0..before.decls.len())
                    .filter(|&i| before.decls[i].module == m.text)
                    .map(|i| at.decl(i))
                    .collect::<R<Vec<Spelled>>>()?,
                of: at.of(),
            }]),
            _ => unreachable!("module_inverse is given a module operator"),
        }
    }

    fn type_inverse(&self, at: &Inverting) -> R<Vec<Inverse>> {
        let Inverting { op, .. } = *at;
        match &op.kind {
            OperatorKind::AddType(t) => {
                Ok(vec![at.unadd(KeyKind::Type, &at.logged.module, &t.name)?])
            }
            OperatorKind::DropType(t) => {
                let m = at.module_of(t);
                let states = at.states_back(false, |(sm, st, _)| *sm == m && *st == t.name.text);
                let mut out = vec![at.restore("type", &m, &t.name.text)?];
                out.extend(self.by_id(states)?);
                Ok(out)
            }
            OperatorKind::RenameType(t, _) => self.by_id(at.renamed(
                |s| {
                    let def = type_named(s, &at.module_of(t), &t.name.text)?;
                    s.ids.type_id(def).map(NamedId::Type)
                },
                &t.name,
            )),
            _ => unreachable!("type_inverse is given a type operator"),
        }
    }

    fn relation_inverse(&self, at: &Inverting) -> R<Vec<Inverse>> {
        let Inverting {
            before, logged, op, ..
        } = *at;
        match &op.kind {
            OperatorKind::AddRelation(r) => Ok(vec![at.unadd(
                KeyKind::Relation,
                &logged.module,
                &r.name.name,
            )?]),
            OperatorKind::DropRelation(r) => {
                let m = at.module_of(r);
                let states = at.states_back(true, |(sm, sr, _)| *sm == m && *sr == r.name.text);
                let mut out = vec![at.restore("relation", &m, &r.name.text)?];
                for (i, d) in before.decls.iter().enumerate() {
                    if d.module == m
                        && let Some(d) = before.parsed(i)
                        && let Decl::Set(s) = &d
                        && s.relation.text == r.name.text
                    {
                        let (kind, name) = key_of(&d);
                        out.push(Inverse::Restore {
                            module: m.clone(),
                            kind,
                            name,
                            text: at.decl(i)?,
                            of: at.of(),
                        });
                    }
                }
                out.extend(self.by_id(states)?);
                Ok(out)
            }
            OperatorKind::RenameRelation(r, _) => self.by_id(at.renamed(
                |s| {
                    let rel = relation_named(s, &at.module_of(r), &r.name.text)?;
                    s.ids.relation_id(rel).map(NamedId::Relation)
                },
                &r.name,
            )),
            OperatorKind::AddSet(d) => {
                let set = at.after.schema().ok().and_then(|s| {
                    let rel = relation_named(&s, &logged.module, &d.relation.text)?;
                    s.ids.set_id(rel, &d.name.text)
                });
                self.by_id(set.map(|set| Inverse::DropSet {
                    set,
                    of: at.text(op.span),
                }))
            }
            OperatorKind::DropSet(name, r) => self.by_id(at.dropped_set(name, r)),
            OperatorKind::RenameSet(set, r, _) => self.by_id(at.renamed(
                |s| {
                    let rel = relation_named(s, &at.module_of(r), &r.name.text)?;
                    s.ids.set_id(rel, &set.text).map(NamedId::Set)
                },
                set,
            )),
            OperatorKind::AddUnique(_, on) | OperatorKind::AddIndex { on, .. } => {
                apply(at.clause("drop", on)?, at.of())
            }
            OperatorKind::DropUnique(_, on) | OperatorKind::DropIndex { on, .. } => {
                apply(at.clause("add", on)?, at.of())
            }
            OperatorKind::Rekey {
                relation, carry, ..
            } => self.rekey_inverse(at, relation, carry),
            OperatorKind::AddDefault {
                relation,
                ctor,
                field,
                ..
            }
            | OperatorKind::DropDefault {
                relation,
                ctor,
                field,
            } => self.default_inverse(at, relation, ctor.as_ref(), field),
            OperatorKind::AddComputed {
                relation, field, ..
            } => {
                let mut text = Spelled::from("drop computed ");
                text.append(at.qualified(relation, field.span.end as usize)?);
                apply(text, at.of())
            }
            OperatorKind::RedefineComputed {
                relation, field, ..
            }
            | OperatorKind::DropComputed(relation, field) => {
                self.computed_inverse(at, relation, field)
            }
            _ => unreachable!("relation_inverse is given a relation operator"),
        }
    }

    fn ctor_inverse(&self, at: &Inverting) -> R<Vec<Inverse>> {
        let Inverting {
            before, logged, op, ..
        } = *at;
        match &op.kind {
            OperatorKind::AddConstructor { path, .. } => {
                let record = before
                    .find("type", &at.module_of(&path.ty), &path.ty.name.text)
                    .filter(|&i| {
                        matches!(
                            before.parsed(i),
                            Some(Decl::Type(ast::TypeDecl {
                                body: ast::TypeBody::Record(_),
                                ..
                            }))
                        )
                    })
                    .map(|i| before.decls[i].text.clone());
                Ok(vec![Inverse::Unadd {
                    path: path.clone(),
                    record,
                }])
            }
            OperatorKind::RemoveConstructor(path) => self.removed_ctor_inverse(at, path),
            OperatorKind::RenameConstructor(path, _) => self.by_id(at.renamed(
                |s| {
                    let def = type_named(s, &at.module_of(&path.ty), &path.ty.name.text)?;
                    let c = s.ty(def).ctor_named(&path.ctor.text)?;
                    s.ids.ctor_id(c).map(NamedId::Ctor)
                },
                &path.ctor,
            )),
            OperatorKind::Enable(t) | OperatorKind::Disable(t) | OperatorKind::Deprecate(t) => {
                let cx = Cx {
                    src: &mut before.clone(),
                    module: logged.module.clone(),
                    text: &logged.text,
                    span: op.span,
                    noted: None,
                    copying: false,
                    steps: None,
                };
                let (relation, key, _) = cx.stage_key(t)?;
                let prior = if relation {
                    before.states.get(&key)
                } else {
                    before.value_states.get(&key)
                };
                let state = StateOf::of(before, relation, &key);
                self.by_id(state.map(|s| Inverse::States {
                    states: vec![(s, prior.copied())],
                    of: at.text(op.span),
                }))
            }
            OperatorKind::NestConstructor {
                ty,
                sel,
                into,
                clauses,
            } => {
                let m = at.module_of(ty);
                let mut s = format!(
                    "unnest constructor {m}::{}.{}",
                    ty.name.text, into.name.text
                );
                let names: Vec<String> = sel_idents(sel).iter().map(|x| x.text.clone()).collect();
                s.push_str(&self.restored_arms(before, clauses, &names));
                ops(s)
            }
            OperatorKind::UnnestConstructor(path, clauses) => {
                self.unnest_inverse(at, path, clauses)
            }
            OperatorKind::SplitConstructor {
                path,
                into,
                arms,
                clauses,
            } => {
                let arms_of: Vec<ast::ArmClause> = clauses
                    .iter()
                    .filter_map(|c| match c {
                        ast::SplitClause::Arm(a) => Some((**a).clone()),
                        _ => None,
                    })
                    .collect();
                self.split_inverse(before, logged, path, into, arms, &arms_of)
            }
            OperatorKind::MergeConstructor {
                ty,
                sel,
                into,
                arms,
                clauses,
            } => self.merge_inverse(before, ty, sel, into, arms.as_deref(), clauses),
            _ => unreachable!("ctor_inverse is given a constructor operator"),
        }
    }

    fn field_inverse(&self, at: &Inverting) -> R<Vec<Inverse>> {
        let Inverting {
            before, op, fills, ..
        } = *at;
        match &op.kind {
            OperatorKind::AddField { path, .. } => {
                let mut text = Spelled::from("drop field ");
                text.append(at.path(path)?);
                apply(text, at.of())
            }
            OperatorKind::AlterField { path, .. } => {
                let (i, f) = at.field_decl(path)?;
                let mut text = Spelled::from("alter field ");
                text.append(at.path(path)?);
                text.push(" on delete ");
                match &f.on_delete {
                    Some(a) => text.append(at.decl_range(i, before.local(i, a.span))?),
                    None => text.push("restrict"),
                }
                apply(text, at.of())
            }
            OperatorKind::DropField(path) => {
                let Some(fill) = self.fill_for(fills, &path_text(path)) else {
                    return self.err(format!(
                        "the inverse of `drop field {}` needs a fill: `fill {} = e`",
                        path_text(path),
                        path_text(path)
                    ));
                };
                let (i, f) = at.field_decl(path)?;
                let mut text = Spelled::from("add field ");
                text.append(at.path(path)?);
                text.push(": ");
                text.append(at.decl_range(i, before.local(i, f.ty.span))?);
                if let Some(a) = &f.on_delete {
                    text.push(" on delete ");
                    text.append(at.decl_range(i, before.local(i, a.span))?);
                }
                text.push(" fill ");
                text.push(fill);
                Ok(vec![
                    Inverse::Apply { text, of: at.of() },
                    self.reorder(
                        before,
                        path,
                        path.ctor.as_ref().map(|c| c.text.clone()),
                        at.of(),
                    )?,
                ])
            }
            OperatorKind::RenameField(path, _) => self.by_id(at.renamed(
                |s| {
                    let def = type_named(s, &at.module_of(&path.ty), &path.ty.name.text)?;
                    s.ids
                        .field_id(field_named(s, def, path)?)
                        .map(NamedId::Field)
                },
                &path.field,
            )),
            OperatorKind::MoveField { path, to, sel, .. } => {
                let m = at.module_of(&path.ty);
                match (&path.ctor, sel) {
                    (Some(c), None) => Ok(vec![
                        Inverse::Ops(
                            format!(
                                "move field {m}::{}.{} to {m}::{}.{}",
                                to.name.text, path.field.text, to.name.text, c.text
                            )
                            .into(),
                        ),
                        self.reorder(before, path, Some(c.text.clone()), at.of())?,
                    ]),
                    (None, Some(sel)) => {
                        let names: Vec<String> =
                            sel_idents(sel).iter().map(|x| x.text.clone()).collect();
                        let first = &names[0];
                        let key = format!("{}.{first}.{}", path.ty.name.text, path.field.text);
                        let schema = before
                            .schema()
                            .map_err(|_| vec![Diag::new(self.span, "the log does not replay")])?;
                        let def = self.type_id(&schema, &path.ty)?;
                        let mut s = Written::from(format!(
                            "move field {m}::{}.{first}.{} to {m}::{}",
                            path.ty.name.text, path.field.text, path.ty.name.text
                        ));
                        if schema.ty(def).ctors().len() > names.len() {
                            let Some(fill) = self.fill_for(fills, &key) else {
                                return self.err(format!(
                                    "the inverse of moving `{}` down needs a fill: `fill {key} = e`",
                                    path_text(path)
                                ));
                            };
                            s.push(" fill ");
                            s.append(fill);
                        }
                        Ok(vec![Inverse::MoveUp {
                            text: s,
                            from: names,
                        }])
                    }
                    _ => Ok(Vec::new()),
                }
            }
            _ => unreachable!("field_inverse is given a field operator"),
        }
    }

    fn body_inverse(&self, at: &Inverting) -> R<Vec<Inverse>> {
        let Inverting {
            before, logged, op, ..
        } = *at;
        let lm = &logged.module;
        let step = match &op.kind {
            OperatorKind::DefineFn(f) => at.unadd(KeyKind::Function, lm, &f.name)?,
            OperatorKind::RedefineFn(f) => {
                let missing = || {
                    vec![Diag::new(
                        self.span,
                        "the log does not hold the redefined function",
                    )]
                };
                let i = before.find("fn", lm, &f.name.text).ok_or_else(missing)?;
                let key =
                    ObjectKey::named(KeyKind::Function, Owner::Module(lm.clone()), &f.name.text);
                let id = before.ids.get(&key).copied().ok_or_else(missing)?;
                Inverse::Redeclare {
                    decl: DeclOf::Object(KeyKind::Function, id),
                    text: Some(at.decl(i)?),
                    of: at.of(),
                }
            }
            OperatorKind::DropFn(f) => at.restore("fn", &at.module_of(f), &f.name.text)?,
            OperatorKind::DefineAggregate(a) => at.unadd(KeyKind::Function, lm, &a.name)?,
            OperatorKind::DropAggregate(a) => {
                at.restore("aggregate", &at.module_of(a), &a.name.text)?
            }
            OperatorKind::AddConstraint(c) => at.unadd(KeyKind::Constraint, lm, &c.name)?,
            OperatorKind::DropConstraint(c) => {
                at.restore("constraint", &at.module_of(c), &c.name.text)?
            }
            OperatorKind::AddCheck(c) => at.unadd(KeyKind::Constraint, lm, &c.name)?,
            OperatorKind::DropCheck(c) => at.restore("check", &at.module_of(c), &c.name.text)?,
            OperatorKind::Store(s) => {
                at.unadd(KeyKind::Function, &at.module_of(&s.view), &s.view.name)?
            }
            OperatorKind::Unstore(v) => at.restore("store", &at.module_of(v), &v.name.text)?,
            _ => unreachable!(
                "body_inverse is given a function, aggregate, constraint, check or store operator"
            ),
        };
        Ok(vec![step])
    }

    fn rekey_inverse(
        &self,
        at: &Inverting,
        relation: &QName,
        carry: &[ast::FieldPath],
    ) -> R<Vec<Inverse>> {
        let Inverting { before, .. } = *at;
        let m = at.module_of(relation);
        let missing = || vec![Diag::new(self.span, "the log does not hold the key")];
        let i = before
            .find("relation", &m, &relation.name.text)
            .ok_or_else(missing)?;
        let Some(Decl::Relation(rd)) = before.parsed(i) else {
            return Err(missing());
        };
        let ranges =
            clause_ranges(&before.decls[i].text, header(&m).len(), &rd).ok_or_else(missing)?;
        let Some((k, fields)) = rd.clauses.iter().enumerate().find_map(|(k, c)| match c {
            RelationClause::Key(fields, _) => Some((k, fields)),
            _ => None,
        }) else {
            return Err(missing());
        };
        let key = ObjectKey::named(KeyKind::Relation, Owner::Module(m), &relation.name.text);
        let rel = before.ids.get(&key).copied().ok_or_else(missing)?;
        let mut text = Spelled::from("rekey relation ");
        text.append(at.qualified(relation, relation.name.span.end as usize)?);
        text.push(" key (");
        for (n, f) in fields.iter().enumerate() {
            if n > 0 {
                text.push(", ");
            }
            text.append(at.decl_range(i, before.local(i, f.span))?);
        }
        text.push(")");
        for (n, p) in carry.iter().enumerate() {
            text.push(if n == 0 { " carry " } else { ", " });
            text.append(at.path(p)?);
        }
        Ok(vec![
            Inverse::Apply { text, of: at.of() },
            Inverse::KeyClause {
                relation: CatRelId::from_raw(rel),
                text: at.decl_range(i, ranges[k].clone())?,
                of: at.of(),
            },
        ])
    }

    fn default_inverse(
        &self,
        at: &Inverting,
        relation: &QName,
        ctor: Option<&ast::Ident>,
        field: &ast::Ident,
    ) -> R<Vec<Inverse>> {
        let Inverting { before, op, .. } = *at;
        let path = at.qualified(relation, field.span.end as usize)?;
        if matches!(op.kind, OperatorKind::AddDefault { .. }) {
            let mut text = Spelled::from("drop default ");
            text.append(path);
            return apply(text, at.of());
        }
        let m = at.module_of(relation);
        let i = before
            .find("relation", &m, &relation.name.text)
            .ok_or_else(|| vec![Diag::new(self.span, "the log does not hold the default")])?;
        let Some(Decl::Relation(rd)) = before.parsed(i) else {
            return self.err("the log does not hold the default");
        };
        let Some(value) = rd.clauses.iter().find_map(|c| match c {
            RelationClause::Default {
                ctor: dc,
                field: f,
                value,
            } if f.text == field.text
                && dc.as_ref().map(|x| &x.text) == ctor.as_ref().map(|x| &x.text) =>
            {
                Some(value)
            }
            _ => None,
        }) else {
            return self.err("the log does not hold the default");
        };
        let mut text = Spelled::from("add default ");
        text.append(path);
        text.push(" = ");
        match value {
            ast::DefaultValue::Generate(g) => text.push(format!("generate {}", g.text)),
            ast::DefaultValue::Expr(e) => text.append(at.decl_range(i, before.local(i, e.span))?),
        }
        apply(text, at.of())
    }

    fn computed_inverse(
        &self,
        at: &Inverting,
        relation: &QName,
        field: &ast::Ident,
    ) -> R<Vec<Inverse>> {
        let Inverting { before, op, .. } = *at;
        let m = at.module_of(relation);
        let missing = || {
            vec![Diag::new(
                self.span,
                "the log does not hold the computed field",
            )]
        };
        let i = before
            .find("relation", &m, &relation.name.text)
            .ok_or_else(missing)?;
        let Some(Decl::Relation(rd)) = before.parsed(i) else {
            return Err(missing());
        };
        let Some((t, e)) = rd.clauses.iter().find_map(|c| match c {
            RelationClause::Computed(n, t, e) if n.text == field.text => Some((t.span, e.span)),
            _ => None,
        }) else {
            return Err(missing());
        };
        let verb = if matches!(op.kind, OperatorKind::DropComputed(..)) {
            "add"
        } else {
            "redefine"
        };
        let mut text = Spelled::from(format!("{verb} computed "));
        text.append(at.qualified(relation, field.span.end as usize)?);
        text.push(": ");
        text.append(at.decl_range(i, before.local(i, t))?);
        text.push(" = ");
        text.append(at.decl_range(i, before.local(i, e))?);
        apply(text, at.of())
    }

    fn removed_ctor_inverse(&self, at: &Inverting, path: &ast::CtorPath) -> R<Vec<Inverse>> {
        let Inverting { before, .. } = *at;
        let m = at.module_of(&path.ty);
        let Some(i) = before.find("type", &m, &path.ty.name.text) else {
            return self.err("the log does not hold the removed constructor");
        };
        let Some(Decl::Type(t)) = before.parsed(i) else {
            return self.err("the log does not hold the removed constructor");
        };
        let h = header(&m).len();
        let ctors = ctor_extents(&before.decls[i].text, h, &t);
        let Some(k) = ctors.iter().position(|c| c.0 == path.ctor.text) else {
            return self.err("the log does not hold the removed constructor");
        };
        let ctext = before.decls[i].text[ctors[k].1.start..ctors[k].2].to_string();
        let after = (k > 0).then(|| ctors[k - 1].0.clone());
        let schema = before
            .schema()
            .map_err(|_| vec![Diag::new(self.span, "the log does not replay")])?;
        let refs = check::schema_refs(&schema);
        let mut arms = Vec::new();
        for j in 0..before.decls.len() {
            let Some(d @ Decl::Constraint(_)) = before.parsed(j) else {
                continue;
            };
            let jh = header(&before.decls[j].module).len();
            let mut texts = Vec::new();
            for (f, span, r) in &refs {
                if f.0 as usize != j || !matches!(r, Ref::Match(_)) {
                    continue;
                }
                for a in match_arms(&d, *span).unwrap_or_default() {
                    if let Some(ast::Pattern {
                        kind: ast::PatternKind::Ctor { path: p, ctor, .. },
                        ..
                    }) = a.patterns.first()
                        && ctor.as_ref().unwrap_or(&p.name).text == path.ctor.text
                    {
                        let start = a.patterns[0].span.start as usize - jh;
                        let end = a.body.span.end as usize - jh;
                        texts.push(before.decls[j].text[start..end].to_string());
                    }
                }
            }
            if !texts.is_empty() {
                let (_, name) = key_of(&d);
                arms.push((name, texts));
            }
        }
        let mut out = vec![Inverse::Readd {
            ty: path.ty.clone(),
            text: ctext,
            after,
            arms,
        }];
        let def = type_named(&schema, &m, &path.ty.name.text);
        let of_type: BTreeSet<(String, String)> = schema
            .relations
            .iter()
            .filter(|r| Some(r.elem_def) == def)
            .map(|r| (schema.module(r.module).name.clone(), r.name.clone()))
            .collect();
        let held = at.states_back(true, |k| {
            k.2 == path.ctor.text && of_type.contains(&(k.0.clone(), k.1.clone()))
        });
        out.extend(self.by_id(held)?);
        let held = at.states_back(false, |k| {
            k.0 == m && k.1 == path.ty.name.text && k.2 == path.ctor.text
        });
        out.extend(self.by_id(held)?);
        Ok(out)
    }

    fn unnest_inverse(
        &self,
        at: &Inverting,
        path: &ast::CtorPath,
        clauses: &[ast::ArmClause],
    ) -> R<Vec<Inverse>> {
        let Inverting { before, .. } = *at;
        let m = at.module_of(&path.ty);
        let Some(i) = before.find("type", &m, &path.ty.name.text) else {
            return self.err("the log does not hold the unnested constructor");
        };
        let Some(Decl::Type(t)) = before.parsed(i) else {
            return self.err("the log does not hold the unnested constructor");
        };
        let h = header(&m).len();
        let ctors = ctor_extents(&before.decls[i].text, h, &t);
        let Some(k) = ctors.iter().position(|c| c.0 == path.ctor.text) else {
            return self.err("the log does not hold the unnested constructor");
        };
        let ctext = before.decls[i].text[ctors[k].1.start..ctors[k].2].to_string();
        let schema = before
            .schema()
            .map_err(|_| vec![Diag::new(self.span, "the log does not replay")])?;
        let def = schema
            .module_named(&m)
            .and_then(|mid| schema.module(mid).types.get(&path.ty.name.text).copied());
        let products: Vec<String> = def
            .and_then(|d| {
                let td = schema.ty(d);
                let c = td.ctor_index(&path.ctor.text)?;
                match &td.ctors()[c as usize].fields.first()?.ty {
                    Ty::Data { def: sd, .. } => Some(
                        schema
                            .ty(*sd)
                            .ctors()
                            .iter()
                            .map(|x| x.name.clone())
                            .collect(),
                    ),
                    _ => None,
                }
            })
            .unwrap_or_default();
        let mut s = format!(
            "nest constructor {m}::{}.({}) into {ctext}",
            path.ty.name.text,
            products.join(" | ")
        );
        s.push_str(&self.restored_arms(before, clauses, std::slice::from_ref(&path.ctor.text)));
        ops(s)
    }
}
