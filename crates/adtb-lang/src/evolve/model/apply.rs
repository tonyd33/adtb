use super::field::Maps;
use super::lift::Lifting;
use super::op::{Fail, Op, Refused, Res, Resolving};
use super::run::{Late, Run};
use super::*;
use crate::ast::{MigrationFile, OperatorKind};
use crate::evolve::grant::IdKind;
use crate::evolve::source::rekey_leaves_no_old_reference;
use crate::evolve::{Kept, Resolved, ResolvedOp};
use crate::lex::{Tok, lex};
use crate::span::{FileId, Span};

/// A migration applied to a model.
pub(in crate::evolve) struct Migrated {
    model: Model,
    /// For each operator applied, its resolved form, or why it was refused.
    ops: Vec<Result<ResolvedOp, Refused>>,
    /// For each operator applied, the row maps of its one step, or why they fail.
    maps: Vec<Maps>,
    /// For each operator applied, the model it leaves.
    afters: Vec<Model>,
    /// Whether the result checks; false when an operator stopped the run.
    checks: bool,
    /// The kind of the first operator the model does not apply, which stops the run there.
    stopped: Option<String>,
}

impl Migrated {
    pub(super) fn model(&self) -> &Model {
        &self.model
    }

    /// Returns, for each operator applied, its resolved form, or why it was refused.
    pub(super) fn ops(&self) -> &[Result<ResolvedOp, Refused>] {
        &self.ops
    }

    /// Returns, for each operator applied, the model it leaves.
    pub(super) fn afters(&self) -> &[Model] {
        &self.afters
    }

    /// Returns the kind of the operator that stopped the run, if one did.
    pub(super) fn stopped(&self) -> Option<&str> {
        self.stopped.as_deref()
    }

    /// Returns, for each operator applied, the row maps of its one step, or why they fail.
    pub(super) fn maps(&self) -> &[Maps] {
        &self.maps
    }

    /// Returns whether the result checks.
    pub(super) fn checks(&self) -> bool {
        self.checks
    }
}

impl Model {
    /// Apply migration `m`, whose text is `text`, numbered one past the model's version.
    ///
    /// Fail with why the model does not apply it.
    pub(in crate::evolve) fn migrate(
        &self,
        m: &MigrationFile,
        text: &str,
    ) -> Result<Migrated, String> {
        self.migrate_keeping(m, text, &[])
    }

    /// Apply migration `m` as [`Model::migrate`] does, putting back after each operator every
    /// constraint and check of `kept` it drops, with its catalog id.
    pub(in crate::evolve) fn migrate_keeping(
        &self,
        m: &MigrationFile,
        text: &str,
        kept: &[Kept],
    ) -> Result<Migrated, String> {
        let module = m.module.text.as_str();
        let mut run = Run::new(m, text);
        let mut model = self.clone();
        model.version += 1;
        model.expecting(m);
        let mut ops = Vec::new();
        let mut maps = Vec::new();
        let mut afters = Vec::new();
        let mut stopped = None;
        let mut rekeyed = Vec::new();
        for (k, op) in m.operators.iter().enumerate() {
            let by = Adder::Operator(k);
            model.mark(by, Added::Applying);
            run.begin(k, op.span);
            let mut next = model.clone();
            match next.operator(&run, &op.kind) {
                Ok((r, m)) => {
                    if let OperatorKind::Rekey { relation, .. } = &op.kind {
                        rekeyed.push((k, relation.clone(), op.span));
                    }
                    maps.push(m);
                    if let Resolved::AddRelation { relation, .. } = r.op {
                        next.mark(by, Added::As(relation));
                        next.terms_mut(&mut |t| t.added(by, relation));
                    }
                    next.keep(&model, kept);
                    next.rebind(&model);
                    model = next;
                    ops.push(Ok(r));
                }
                Err(Fail::Refused(why)) => {
                    maps.push(Ok(Vec::new()));
                    run.forget_late(k);
                    model.mark(by, Added::Not);
                    ops.push(Err(why.or_at(op.span)));
                }
                Err(Fail::Unmodelled(kind)) => {
                    stopped = Some(kind);
                    break;
                }
                Err(Fail::Broken(why)) => return Err(why),
            }
            afters.push(model.clone());
        }
        for u in std::mem::take(&mut model.unstated) {
            if let Some(k) = m.operators.iter().position(|op| op.span == u.span) {
                ops[k] = Err(Refused::at(u.span, u.message));
            }
        }
        let late = run.take_late();
        let mut checks = false;
        if stopped.is_none() {
            checks =
                (late.is_empty() || model.resolve_late(&late, &mut ops)?) && model.checks(&[])?;
            if checks && !rekeyed.is_empty() {
                match model.built(&[], &[]) {
                    Ok((built, _)) => {
                        for (k, relation, span) in &rekeyed {
                            if let Err(d) = rekey_leaves_no_old_reference(
                                built.schema(),
                                module,
                                relation,
                                *span,
                            ) {
                                ops[*k] = Err(Refused::at(d.span, d.message));
                            }
                        }
                    }
                    Err(_) => {
                        stopped =
                            Some("a rekey's end check over declarations that do not build".into());
                    }
                }
            }
            if checks && late.iter().any(|(_, l)| matches!(l, Late::Unresolved)) {
                stopped = Some("a clause naming what only the migration's end resolves".into());
            }
        }
        if checks && ops.iter().all(Result::is_ok) {
            model.log.push(LogM {
                number: model.version,
                module: module.to_string(),
                text: text.to_string(),
                operators: m.operators.len(),
            });
        }
        Ok(Migrated {
            model,
            ops,
            maps,
            afters,
            checks,
            stopped,
        })
    }

    /// Put back each constraint and check of `kept` that `prior` declares and this model does not.
    fn keep(&mut self, prior: &Model, kept: &[Kept]) {
        for k in kept {
            let held = prior.decls.iter().find(|d| {
                d.module == k.module
                    && match d.item {
                        Item::Constraint(c) => !k.check && prior.denials[&c].name == k.name,
                        Item::Check(c) => k.check && prior.denials[&c].name == k.name,
                        _ => false,
                    }
            });
            let Some(d) = held else { continue };
            let (Item::Constraint(c) | Item::Check(c)) = d.item else {
                continue;
            };
            if let std::collections::btree_map::Entry::Vacant(e) = self.denials.entry(c) {
                e.insert(prior.denials[&c].clone());
                self.decls.push(d.clone());
            }
        }
    }

    /// Lift again each declaration of `late` from the declarations as the migration leaves them,
    /// and give the operator that wrote it, or a field of it, the canonical form it has there.
    /// Returns false when the declarations do not build or a late name does not resolve.
    fn resolve_late(
        &mut self,
        late: &[(usize, Late)],
        ops: &mut [Result<ResolvedOp, Refused>],
    ) -> Result<bool, String> {
        let Ok(built) = self.build_adopting(&[], &[]) else {
            return Ok(false);
        };
        let broken = Fail::reason;
        for &(k, late) in late {
            let canonical = match late {
                Late::Decl(item) => {
                    let Some(i) = self.decl_at(item) else {
                        continue;
                    };
                    let file = FileId(i as u32);
                    let d = built
                        .decl(file)
                        .ok_or("a declaration the model printed does not parse")?;
                    let names = built.names(file);
                    Lifting::new(
                        built.schema(),
                        file,
                        built.text(file),
                        &names,
                        &[],
                        &self.decls[i].module.clone(),
                    )
                    .decl(self, d)?;
                    self.decls.pop();
                    built.canonical(file).map_err(broken)?
                }
                Late::Field { field, action_only } => {
                    let Some(t) = self.field_type(field) else {
                        continue;
                    };
                    let owner = self.fields[&field].owner;
                    let ctor = self.ctors.get(&owner).map(|c| c.name.clone());
                    let i = self.decl_at(Item::Type(t)).expect("a declared type");
                    Self::field_canonical(
                        &built,
                        FileId(i as u32),
                        ctor.as_deref(),
                        &self.fields[&field].name,
                        action_only,
                    )
                    .map_err(broken)?
                }
                Late::Ctor { ctor, record } => {
                    let Some(t) = self.ctors.get(&ctor).map(|c| c.ty) else {
                        continue;
                    };
                    let Some(i) = self.decl_at(Item::Type(t)) else {
                        continue;
                    };
                    self.ctor_written(&built, FileId(i as u32), ctor, record)
                        .map_err(broken)?
                }
                Late::Default { relation, default } => {
                    let held = self.relations.get(&relation).is_some_and(|x| {
                        x.clauses.iter().any(|c| {
                            matches!(c, ClauseM::Default { ids, .. } if ids.iter().any(|&(_, d)| d == default))
                        })
                    });
                    if !held {
                        continue;
                    }
                    self.default_canonical(&built, relation, default)
                }
                Late::Computed { relation, computed } => {
                    let held = self.relations.get(&relation).is_some_and(|x| {
                        x.clauses.iter().any(|c| {
                            matches!(c, ClauseM::Computed { id: Some(id), .. } if *id == computed)
                        })
                    });
                    if !held {
                        continue;
                    }
                    self.computed_canonical(&built, relation, computed)
                }
                Late::Unresolved => continue,
            };
            let Some(canonical) = canonical else {
                return Ok(false);
            };
            if let Some(Ok(op)) = ops.get_mut(k) {
                set_decl(&mut op.op, canonical);
            }
        }
        Ok(true)
    }

    /// Apply one operator. Returns it resolved.
    fn operator(&mut self, run: &Run, op: &OperatorKind) -> Res<(ResolvedOp, Maps)> {
        let structural = match op {
            OperatorKind::AddField {
                path,
                ty,
                on_delete,
                decl,
                fill,
            } => {
                let action = on_delete
                    .as_ref()
                    .map(|_| action_span(run.text(), ty.span, *decl));
                Some(self.add_field(run, path, ty.span, action, fill)?)
            }
            OperatorKind::DropField(path) => Some(self.drop_field(run, path)?),
            OperatorKind::AlterField { path, span, .. } => {
                Some(self.alter_field(run, path, *span)?)
            }
            OperatorKind::AddUnique(fields, on) => {
                Some(self.add_index(run, IndexKind::Unique, fields, on)?)
            }
            OperatorKind::AddIndex {
                interval,
                fields,
                on,
            } => Some(self.add_index(run, index_kind(*interval), fields, on)?),
            OperatorKind::DropUnique(fields, on) => {
                Some(self.drop_index(run, IndexKind::Unique, fields, on)?)
            }
            OperatorKind::DropIndex {
                interval,
                fields,
                on,
            } => Some(self.drop_index(run, index_kind(*interval), fields, on)?),
            OperatorKind::AddDefault {
                relation,
                ctor,
                field,
                value,
            } => Some(self.add_default(run, relation, ctor.as_ref(), field, value)?),
            OperatorKind::DropDefault {
                relation,
                ctor,
                field,
            } => Some(self.drop_default(run, relation, ctor.as_ref(), field)?),
            OperatorKind::AddComputed {
                relation,
                field,
                ty,
                expr,
            } => Some(self.add_computed(run, relation, field, ty, expr)?),
            OperatorKind::RedefineComputed {
                relation,
                field,
                ty,
                expr,
            } => Some(self.redefine_computed(run, relation, field, ty, expr)?),
            OperatorKind::DropComputed(relation, field) => {
                Some(self.drop_computed(run, relation, field)?)
            }
            OperatorKind::AddConstructor {
                path,
                record,
                place,
                joins,
                arms,
            } => Some(self.add_constructor(
                run,
                path,
                record.as_deref(),
                place.as_ref(),
                joins,
                arms,
            )?),
            OperatorKind::MoveField {
                path,
                to,
                sel,
                fill,
            } => Some(self.move_field(run, path, to, sel.as_ref(), fill.as_ref())?),
            OperatorKind::Rekey {
                relation,
                key,
                carry,
            } => Some(self.rekey(run, relation, key, carry)?),
            OperatorKind::RemoveConstructor(p) => Some(self.remove_constructor(run, p)?),
            OperatorKind::Grant(g) => Some(self.grant(run, g)?),
            OperatorKind::Revoke(r) => Some(self.revoke(run, r)?),
            OperatorKind::Script(w) => Some(self.script(run, w)?),
            OperatorKind::Install {
                name,
                version,
                from,
            } => Some(self.install(run, name, version, from)?),
            OperatorKind::Uninstall(m) => Some(self.uninstall(m)?),
            OperatorKind::AdoptTzdb { release, from } => Some(self.adopt_tzdb(release, from)?),
            op => self.restructuring(run, op)?,
        };
        if let Some((op, clauses, maps)) = structural {
            self.settle();
            return Ok((ResolvedOp { op, clauses }, maps));
        }
        if matches!(
            op,
            OperatorKind::RenameConstructor(..)
                | OperatorKind::Enable(_)
                | OperatorKind::Disable(_)
                | OperatorKind::Deprecate(_)
        ) {
            self.schema_now(run)?;
        }
        let resolved = Resolving::new(self, run.module()).op(op)?;
        let out = match resolved {
            Op::AddModule(m) => self.add_module(&m)?,
            Op::DropModule(m) => self.drop_module(&m)?,
            Op::Add { kind, name, span } => self.add(run, kind, &name, span, op)?,
            Op::RedefineFn { function, span } => self.redefine_fn(run, function, span, op)?,
            Op::Drop(item) => self.drop(run, item)?,
            Op::Rename { obj, to } => self.rename(run, obj, &to)?,
            Op::Stage {
                key,
                to,
                verb,
                shown,
            } => self.stage(run, key, to, verb, &shown)?,
        };
        self.settle();
        Ok((
            ResolvedOp {
                op: out.0,
                clauses: out.1,
            },
            Ok(Vec::new()),
        ))
    }

    /// Forget the write states and grants of objects that are gone.
    fn settle(&mut self) {
        let ctors = &self.ctors;
        let rels = &self.relations;
        let stores = &self.stores;
        self.states.retain(|(r, c), _| {
            (rels.contains_key(r) || stores.contains_key(r))
                && (ctors.contains_key(c) || *c < FIRST_LOG_ID)
        });
        self.value_states.retain(|c, _| ctors.contains_key(c));
        let live = |k: IdKind, x: u64| match k {
            IdKind::Relation => self.relations.contains_key(&x) || self.stores.contains_key(&x),
            IdKind::Ctor => self.ctors.contains_key(&x),
            IdKind::Set => self.sets.contains_key(&x),
            IdKind::Field => self.fields.contains_key(&x),
            IdKind::Function => self.functions.contains_key(&x),
        };
        self.grants = crate::evolve::carry::pruned(&self.grants, &live);
    }
}

fn index_kind(interval: bool) -> IndexKind {
    if interval {
        IndexKind::Interval
    } else {
        IndexKind::Plain
    }
}

/// Give `op`, an operator adding a declaration, the canonical form `decl`.
fn set_decl(op: &mut Resolved, decl: String) {
    match op {
        Resolved::AddType { decl: d, .. }
        | Resolved::AddRelation { decl: d, .. }
        | Resolved::DefineFn { decl: d, .. }
        | Resolved::DefineAggregate { decl: d, .. }
        | Resolved::Store { decl: d, .. }
        | Resolved::AddConstraint { body: d, .. }
        | Resolved::AddCheck { body: d, .. }
        | Resolved::AddField { decl: d, .. }
        | Resolved::AlterField { action: d, .. }
        | Resolved::AddComputed { decl: d, .. }
        | Resolved::AddConstructor { decl: d, .. }
        | Resolved::Merge { decl: d, .. }
        | Resolved::Nest { decl: d, .. }
        | Resolved::AddDefault { value: d, .. } => *d = decl,
        _ => {}
    }
}

/// Returns the span of the delete action of a field written at `decl` whose type is at `ty`: from
/// the token after `on delete` to the end.
fn action_span(text: &str, ty: Span, decl: Span) -> Span {
    let from = ty.end as usize;
    let toks: Vec<_> = lex(&text[from..decl.end as usize])
        .0
        .into_iter()
        .filter(|t| t.tok != Tok::Eof)
        .collect();
    let start = toks
        .get(2)
        .map_or(decl.end as usize, |t| from + t.span.start as usize);
    Span::new(start, decl.end as usize)
}
