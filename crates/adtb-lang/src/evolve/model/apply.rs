use super::field::Maps;
use super::lift::Lifting;
use super::op::{Fail, Op, Refused, Res, Resolving, broken, refuse};
use super::run::{Built, Decls, Gave, Late, Run};
use super::*;
use crate::ast::{MigrationFile, OperatorKind};
use crate::canon::Names;
use crate::evolve::grant::IdKind;
use crate::evolve::source::rekey_leaves_no_old_reference;
use crate::evolve::{Clause, Kept, Resolved, ResolvedOp};
use crate::span::{Diag, FileDiag, FileId, Span};

/// A migration applied to a model.
pub(in crate::evolve) struct Migrated {
    /// The model before the migration's first operator.
    start: Model,
    model: Model,
    /// For each operator applied, its resolved form, or why it was refused.
    ops: Vec<Result<ResolvedOp, Refused>>,
    /// For each operator applied, the row maps of its one step, or why they fail.
    maps: Vec<Maps>,
    /// For each operator applied, its steps when it is a `revert`.
    steps: Vec<Option<revert::Steps>>,
    /// For each operator applied, the model it leaves.
    afters: Vec<Model>,
    /// Whether the result checks.
    checks: bool,
    /// The names of the migration's text, when they were asked for, or the copies a later
    /// operator rewrote before they were named.
    names: Option<Result<Names, Vec<Diag>>>,
    /// Each copy of the migration's text its applied operators made: the range it copies and
    /// where it is.
    copies: Vec<(std::ops::Range<usize>, naming::At)>,
    /// For each structural operator that cannot carry a grant no later grant operator states,
    /// every such grant's refusal.
    uncarried: BTreeMap<usize, Vec<Diag>>,
}

impl Migrated {
    /// Returns, for each operator applied, its resolved form, or why it was refused.
    pub(super) fn ops(&self) -> &[Result<ResolvedOp, Refused>] {
        &self.ops
    }

    /// Returns, for each operator applied, the model it leaves.
    pub(super) fn afters(&self) -> &[Model] {
        &self.afters
    }

    /// Returns the names of the migration's text, when they were asked for.
    pub(in crate::evolve) fn names(&self) -> Option<&Result<Names, Vec<Diag>>> {
        self.names.as_ref()
    }

    /// Returns why each operator of `m` that does not apply is refused, at its span, with the
    /// operator's index.
    pub(in crate::evolve) fn refusals(&self, m: &MigrationFile) -> Vec<(usize, Diag)> {
        self.ops
            .iter()
            .zip(&m.operators)
            .enumerate()
            .flat_map(|(k, (o, op))| {
                let ds = match (o, self.uncarried.get(&k)) {
                    (Err(_), Some(ds)) => ds.clone(),
                    (Err(r), None) if r.reported() => {
                        let first = r.diag(op.span);
                        let more = r.more().iter();
                        let more =
                            more.map(|(at, why)| Diag::new(at.unwrap_or(op.span), why.clone()));
                        std::iter::once(first).chain(more).collect()
                    }
                    _ => Vec::new(),
                };
                ds.into_iter().map(move |d| (k, d))
            })
            .collect()
    }

    /// Returns the index of the operator each of `refusals` is blamed on, and for each of
    /// `recheck`, failures of the declarations the migration leaves, the index of the operator it
    /// is blamed on, or `None` when a refusal already reports it.
    ///
    /// Postconditions:
    /// - A refusal of operator `k` reports a failure of another declaration when that
    ///   declaration failed with its message, or with the failure the refusal repeats, in the
    ///   model before `k`; it is blamed as that failure is. Any other refusal is blamed on `k`.
    /// - A recheck failure is reported by a refusal giving its message when its declaration
    ///   failed with that message in the model before the refused operator.
    /// - A failure is blamed on the first operator from which every model after it holds it, or
    ///   on the first operator when the model before the migration holds it too.
    pub(in crate::evolve) fn ledger(
        &self,
        refusals: &[(usize, Diag)],
        recheck: &[FileDiag],
    ) -> (Vec<usize>, Vec<Option<usize>>) {
        let held: Vec<BTreeSet<(Item, String)>> = std::iter::once(&self.start)
            .chain(&self.afters)
            .map(failures)
            .collect();
        let last = self.ops.len().saturating_sub(1);
        let blame = |key: &(Item, String)| {
            let since = held.iter().rposition(|f| !f.contains(key));
            since.unwrap_or(0).min(last)
        };
        let refused = refusals
            .iter()
            .map(|(k, d)| {
                let why = self.repeated(*k, d);
                held[*k]
                    .iter()
                    .find(|(_, held)| held == why)
                    .map_or(*k, blame)
            })
            .collect();
        let rechecked = recheck
            .iter()
            .map(|d| {
                let Some(item) = self.model.decls.get(d.file.0 as usize).map(|x| x.item) else {
                    return Some(last);
                };
                let key = (item, d.diag.message.clone());
                let reported = refusals
                    .iter()
                    .any(|(k, r)| r.message == d.diag.message && held[*k].contains(&key));
                (!reported).then(|| blame(&key))
            })
            .collect();
        (refused, rechecked)
    }

    /// Returns the failure of a declaration refusal `d` of operator `k` reports.
    fn repeated<'a>(&'a self, k: usize, d: &'a Diag) -> &'a str {
        match &self.ops[k] {
            Err(r) if r.why() == d.message => r.repeated(),
            _ => &d.message,
        }
    }

    /// Returns the diagnostic at `at` of why the declarations the migration leaves do not check.
    pub(in crate::evolve) fn unchecked(&self, at: Span) -> Diag {
        self.model.unchecked(at)
    }

    /// Returns whether every operator applied and the result checks.
    /// Returns the index of the first operator whose steps fail, and why they do at its span.
    pub(in crate::evolve) fn failing_steps(&self, m: &MigrationFile) -> Option<(usize, Diag)> {
        m.operators
            .iter()
            .zip(self.maps.iter().zip(&self.steps))
            .enumerate()
            .find_map(|(k, (op, step))| match step {
                (_, Some(Err(why))) | (Err(why), None) => {
                    Some((k, Diag::new(op.span, why.clone())))
                }
                _ => None,
            })
    }

    pub(in crate::evolve) fn applies(&self) -> bool {
        self.checks && self.ops.iter().all(Result::is_ok)
    }

    /// Returns the source the migration leaves, with the relation variables its operators add.
    pub(in crate::evolve) fn source(&self) -> Source {
        self.model.replay_source()
    }

    /// Returns the operators resolved.
    pub(in crate::evolve) fn resolved(&self) -> Vec<ResolvedOp> {
        self.ops.iter().flatten().cloned().collect()
    }

    /// Returns the steps of each operator of `m` in order, ending at the first operator whose
    /// steps fail.
    ///
    /// Preconditions:
    /// - Every operator of `m` applied.
    pub(in crate::evolve) fn op_steps(&mut self, m: &MigrationFile) -> Vec<crate::evolve::OpSteps> {
        let mut out = Vec::new();
        let maps = std::mem::take(&mut self.maps);
        let steps = std::mem::take(&mut self.steps);
        for (k, ((op, maps), steps)) in m.operators.iter().zip(maps).zip(steps).enumerate() {
            let after = &self.afters[k];
            let failed = |why: String| Err(vec![Diag::new(op.span, why)]);
            let stepped = match (steps, maps) {
                (Some(Ok(steps)), _) if steps.is_empty() => Ok(vec![crate::evolve::Step {
                    after: after.replay_source(),
                    maps: Vec::new(),
                    applied: None,
                }]),
                (Some(Ok(steps)), _) => {
                    let last = steps.len() - 1;
                    Ok(steps
                        .into_iter()
                        .enumerate()
                        .map(|(j, s)| s.into_step((j == last).then_some(after)))
                        .collect())
                }
                (Some(Err(why)), _) | (None, Err(why)) => failed(why),
                (None, Ok(maps)) => match &self.ops[k] {
                    Ok(resolved) => Ok(vec![crate::evolve::Step {
                        after: after.replay_source(),
                        maps,
                        applied: Some(resolved.clone()),
                    }]),
                    Err(r) => failed(r.why().to_string()),
                },
            };
            let stop = stepped.is_err();
            out.push(stepped);
            if stop {
                break;
            }
        }
        out
    }

    /// Returns, for each of `diags`, diagnostics of the declarations the migration leaves, the
    /// span of the migration's text `text` an operator copied the diagnosed text from, when it
    /// copied it.
    pub(in crate::evolve) fn copied_from(
        &self,
        text: &str,
        diags: &[FileDiag],
    ) -> Vec<Option<Span>> {
        let parsed = self.model.parsed();
        diags
            .iter()
            .map(|d| {
                let span = d.diag.span;
                let placed = self.copies.iter().find_map(|(from, at)| {
                    let (file, at) = self.model.span_of(&parsed, *at)?;
                    (file == d.file && at.start <= span.start && span.end <= at.end)
                        .then(|| {
                            naming::copied_span(
                                parsed.text(file),
                                at.range(),
                                text,
                                from.clone(),
                                span,
                            )
                        })
                        .flatten()
                });
                placed.or_else(|| {
                    self.model
                        .naming
                        .arm_copies()
                        .iter()
                        .find_map(|(from, item)| {
                            let file = FileId(self.model.decl_at(*item)? as u32);
                            (file == d.file)
                                .then(|| {
                                    naming::searched_span(
                                        parsed.text(file),
                                        text,
                                        from.clone(),
                                        span,
                                    )
                                })
                                .flatten()
                        })
                })
            })
            .collect()
    }
}

impl Model {
    /// Apply migration `m`, whose text is `text`, numbered one past the model's version, putting
    /// back after each operator every constraint and check of `kept` it drops, with its catalog
    /// id, and with a `revert` refusing a fill no inverse takes when `accepting`; naming its text
    /// when `naming`.
    pub(in crate::evolve) fn migrate_keeping(
        &self,
        m: &MigrationFile,
        text: &str,
        kept: &[Kept],
        accepting: bool,
        naming: bool,
    ) -> Result<Migrated, String> {
        let module = m.module.text.as_str();
        let mut run = Run::new(m, text).accepting(accepting);
        let mut model = self.clone();
        model.version += 1;
        model.lifted.clear();
        model.expecting(m);
        model.naming = naming::Naming::of(text);
        let mut ops = Vec::new();
        let mut maps = Vec::new();
        let mut steps = Vec::new();
        let mut afters = Vec::new();
        let mut rekeyed = Vec::new();
        let mut copies = Vec::new();
        for (k, op) in m.operators.iter().enumerate() {
            let by = Adder::Operator(k);
            model.mark(by, Added::Applying);
            run.begin(k, op.span);
            let mut next = model.clone();
            let renames = matches!(
                op.kind,
                OperatorKind::RenameType(..)
                    | OperatorKind::RenameRelation(..)
                    | OperatorKind::RenameSet(..)
                    | OperatorKind::RenameConstructor(..)
                    | OperatorKind::RenameField(..)
            );
            let applied = next.operator(&mut run, &op.kind).and_then(|x| {
                if renames {
                    next.faithful(&model, &mut run)?;
                }
                next.identify();
                next.ids_fit().map(|()| x)
            });
            match applied {
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
                    copies.extend(next.copies_of(&mut run, op, &r, &mut Vec::new()));
                    if naming {
                        next.name_copies(&mut run, op, &r);
                    }
                    steps.push(run.keep());
                    model = next;
                    ops.push(Ok(r));
                }
                Err(Fail::Refused(why)) => {
                    maps.push(Ok(Vec::new()));
                    steps.push(None);
                    run.discard();
                    model.mark(by, Added::Refused);
                    if let Some((name, kind)) = crate::evolve::expected::poisons(&op.kind) {
                        model.ahead.push(AheadM {
                            module: module.to_string(),
                            name: name.to_string(),
                            typed: false,
                            by,
                            added: Added::Refused,
                            kind,
                        });
                    }
                    ops.push(Err(why.or_at(op.span)));
                }
                Err(Fail::Broken(why)) => return Err(why),
            }
            afters.push(model.clone());
        }
        let mut uncarried: BTreeMap<usize, Vec<Diag>> = BTreeMap::new();
        for u in std::mem::take(&mut model.unstated) {
            if let Some(k) = m.operators.iter().position(|op| op.span == u.span) {
                uncarried
                    .entry(k)
                    .or_default()
                    .push(Diag::new(u.span, u.message.clone()));
                ops[k] = Err(Refused::at(u.span, u.message));
            }
        }
        let late = run.take_late();
        let mut unresolved = late.iter().any(|(_, l)| matches!(l, Late::Unresolved));
        let checks = (late.is_empty() || model.resolve_late(&late, &mut ops, &mut unresolved)?)
            && model.checks(&[])?;
        if checks && !rekeyed.is_empty() {
            let Ok((built, _)) = model.built(&[], &[]) else {
                return Err("declarations that check and do not build".into());
            };
            for (k, relation, span) in &rekeyed {
                if let Err(d) =
                    rekey_leaves_no_old_reference(built.schema(), module, relation, *span)
                {
                    ops[*k] = Err(Refused::at(d.span, d.message));
                }
            }
        }
        if checks && unresolved {
            return Err("a clause naming what only the migration's end resolves".into());
        }
        if checks && ops.iter().all(Result::is_ok) {
            model.log.push(LogM {
                number: model.version,
                module: module.to_string(),
                text: text.to_string(),
                operators: m.operators.len(),
            });
        }
        let names = (naming && checks).then(|| {
            model.name_late(&mut run);
            let missed = run.take_missed();
            if !missed.is_empty() {
                return Err(missed);
            }
            let copied = std::mem::take(&mut model.naming).into_names()?;
            let mut names = run.take_noted();
            names.extend(copied);
            Ok(names)
        });
        Ok(Migrated {
            start: self.clone(),
            model,
            ops,
            maps,
            steps,
            afters,
            checks,
            names,
            copies,
            uncarried,
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

    /// Returns what `built`, the build at the migration's end, gives unique or index `index` of
    /// relation variable `relation`, or what the first build after it gave the clause when an
    /// operator removed it since.
    fn gave(&self, built: &Built, relation: u64, index: u64) -> Result<Gave, String> {
        let held = self.relations.get(&relation).and_then(|x| {
            x.clauses
                .iter()
                .position(|c| matches!(c, ClauseM::Index { id: Some(x), .. } if *x == index))
        });
        match held {
            Some(at) => Ok(Gave::new(
                self.index_canonical(built, relation, at),
                self.index_scope(relation, at),
            )),
            None => self
                .gave_clauses
                .get(&(Space::Index, index))
                .cloned()
                .ok_or_else(|| unbuilt_gone("a unique or index")),
        }
    }

    /// Lift again each declaration of `late` from the declarations as the migration leaves them,
    /// and give the operator that wrote it, or a field of it, the canonical form it has there.
    /// Returns false when the declarations do not build or a late name does not resolve. Set
    /// `unresolved` when an arm has no canonical form.
    fn resolve_late(
        &mut self,
        late: &[(usize, Late)],
        ops: &mut [Result<ResolvedOp, Refused>],
        unresolved: &mut bool,
    ) -> Result<bool, String> {
        let Ok(built) = self.build_adopting(&[], &[]) else {
            return Ok(false);
        };
        let broken = Fail::reason;
        for &(k, late) in late {
            let canonical = match late {
                Late::Decl(item) => match self.late_decl(&built, item)? {
                    Some(canonical) => canonical,
                    None => continue,
                },
                Late::Held(key) => {
                    let Some(&item) = self.lifted.get(&key) else {
                        return Ok(false);
                    };
                    let Some(canonical) = self.late_decl(&built, item)? else {
                        continue;
                    };
                    let Some(canonical) = canonical else {
                        return Ok(false);
                    };
                    let (op, clauses) = self.added(item, canonical);
                    if let Some(Ok(r)) = ops.get_mut(k) {
                        *r = ResolvedOp { op, clauses };
                    }
                    continue;
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
                    match (held, self.gave_clauses.get(&(Space::Default, default))) {
                        (true, _) => self.default_canonical(&built, relation, default),
                        (false, Some(gave)) => gave.canonical().cloned(),
                        (false, None) => continue,
                    }
                }
                Late::Computed { relation, computed } => {
                    let held = self.relations.get(&relation).is_some_and(|x| {
                        x.clauses.iter().any(|c| {
                            matches!(c, ClauseM::Computed { id: Some(id), .. } if *id == computed)
                        })
                    });
                    match (held, self.gave_computed.get(&computed)) {
                        (true, _) => self.computed_canonical(&built, relation, computed),
                        (false, Some(canonical)) => canonical.clone(),
                        (false, None) => return Err(unbuilt_gone("a computed field")),
                    }
                }
                Late::Ctors {
                    first,
                    last,
                    record,
                } => {
                    let Some(t) = self.ctors.get(&first).map(|c| c.ty) else {
                        continue;
                    };
                    let Some(i) = self.decl_at(Item::Type(t)) else {
                        continue;
                    };
                    if !self.ctors.contains_key(&last) {
                        continue;
                    }
                    self.ctors_written(&built, FileId(i as u32), (first, last), record)
                        .map_err(broken)?
                }
                Late::Arm { key, nth } => {
                    let arm = match self.arms_written.get(&key) {
                        Some(Ok(Some(arm))) => arm.clone(),
                        Some(Err(e)) => return Err(e.clone()),
                        _ => {
                            *unresolved = true;
                            continue;
                        }
                    };
                    let held = ops
                        .get_mut(k)
                        .and_then(|op| op.as_mut().ok())
                        .and_then(|op| {
                            op.clauses
                                .iter_mut()
                                .filter_map(|c| match c {
                                    Clause::Arm { arm, .. } => Some(arm),
                                    _ => None,
                                })
                                .nth(nth)
                        });
                    if let Some(held) = held {
                        *held = arm;
                    }
                    continue;
                }
                Late::Index { relation, index } => {
                    let gave = self.gave(&built, relation, index)?;
                    let Some(Ok(op)) = ops.get_mut(k) else {
                        continue;
                    };
                    let (canonical, clauses) = gave.into_parts();
                    op.clauses = clauses;
                    canonical
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

    /// Lift declaration `item` again from `built`, the declarations as the migration leaves them.
    /// Returns its canonical form there, `None` inside when a name of it does not resolve there,
    /// and `None` when no declaration is `item`.
    fn late_decl(&mut self, built: &Built, item: Item) -> Result<Option<Option<String>>, String> {
        let Some(i) = self.decl_at(item) else {
            return Ok(None);
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
        built.canonical(file).map(Some).map_err(Fail::reason)
    }

    /// Apply one operator. Returns it resolved.
    pub(super) fn operator(&mut self, run: &mut Run, op: &OperatorKind) -> Res<(ResolvedOp, Maps)> {
        if matches!(
            op,
            OperatorKind::RenameConstructor(..)
                | OperatorKind::Enable(_)
                | OperatorKind::Disable(_)
                | OperatorKind::Deprecate(_)
        ) {
            self.schema_now(run)?;
        }
        let resolved = Resolving::of(self, run.module(), run.text()).op(op)?;
        let plain = |(op, clauses): (Resolved, Vec<Clause>)| (op, clauses, Ok(Vec::new()));
        let (out, clauses, maps) = match resolved {
            Op::AddModule(m) => plain(self.add_module(&m)?),
            Op::DropModule(m) => plain(self.drop_module(&m)?),
            Op::Add { kind, name, span } => plain(self.add(run, kind, &name, span)?),
            Op::RedefineFn { function, span } => plain(self.redefine_fn(run, function, span, op)?),
            Op::Drop(item) => plain(self.drop(run, item)?),
            Op::Rename { obj, to } => plain(self.rename(run, obj, &to)?),
            Op::Stage {
                key,
                to,
                verb,
                shown,
            } => plain(self.stage(run, key, to, verb, &shown)?),
            Op::AddField {
                part,
                path,
                ty,
                action,
                fill,
            } => self.add_field(run, part, path, ty, action, fill)?,
            Op::DropField(f) => self.drop_field(f)?,
            Op::AlterField {
                field,
                part,
                path,
                action,
            } => self.alter_field(run, (field, part), path, action)?,
            Op::AddIndex {
                kind,
                relation,
                fields,
                on,
            } => self.add_index(run, kind, relation, fields, on)?,
            Op::DropIndex {
                kind,
                relation,
                fields,
                on,
            } => self.drop_index(run, kind, relation, fields, on)?,
            Op::AddDefault {
                relation,
                ctor,
                field,
                value,
            } => self.add_default(run, relation, ctor, field, value)?,
            Op::DropDefault {
                relation,
                ctor,
                field,
            } => self.drop_default(run, relation, ctor, field)?,
            Op::AddComputed {
                relation,
                field,
                ty,
                expr,
            } => self.add_computed(run, relation, field, ty, expr)?,
            Op::RedefineComputed {
                relation,
                field,
                ty,
                expr,
            } => self.redefine_computed(run, relation, field, ty, expr)?,
            Op::DropComputed { relation, field } => self.drop_computed(run, relation, field)?,
            Op::AddConstructor {
                ty,
                path,
                record,
                place,
                joins,
                arms,
            } => self.add_constructor(run, (ty, path), record, place, joins, arms)?,
            Op::RemoveConstructor { ty, ctor } => self.remove_constructor(run, ty, ctor)?,
            Op::Split {
                ty,
                ctor,
                path,
                into,
                arms,
                clauses,
            } => self.split(run, (ty, ctor), path, into, arms, clauses)?,
            Op::Merge {
                ty,
                sel,
                into,
                arms,
                clauses,
            } => self.merge(run, (ty, sel), into, arms, clauses, false)?,
            Op::Nest {
                ty,
                sel,
                into,
                clauses,
            } => self.nest(run, ty, sel, into, clauses)?,
            Op::Unnest {
                ty,
                ctor,
                path,
                clauses,
            } => self.unnest(run, (ty, ctor), path, clauses)?,
            Op::MoveField {
                field,
                part,
                path,
                sel,
                fill,
            } => self.move_field(run, (field, part), path, sel, fill)?,
            Op::Rekey {
                relation,
                key,
                carry,
            } => self.rekey(run, relation, key, carry)?,
            Op::Grant(g) => self.grant(run, g)?,
            Op::Revoke(r) => self.revoke(run, r)?,
            Op::Script(w) => self.script(run, w)?,
            Op::Install {
                name,
                version,
                from,
            } => self.install(run, name, version, from)?,
            Op::Uninstall(m) => self.uninstall(m)?,
            Op::AdoptTzdb { release, from } => self.adopt_tzdb(release, from)?,
            Op::Revert { migration, fills } => self.revert(run, migration, fills)?,
        };
        self.settle();
        Ok((ResolvedOp { op: out, clauses }, maps))
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
        | Resolved::RedefineComputed { decl: d, .. }
        | Resolved::AddUnique { fields: d, .. }
        | Resolved::AddIndex { fields: d, .. }
        | Resolved::AddConstructor { decl: d, .. }
        | Resolved::Merge { decl: d, .. }
        | Resolved::Nest { decl: d, .. }
        | Resolved::Split { decl: d, .. }
        | Resolved::AddDefault { value: d, .. } => *d = decl,
        _ => {}
    }
}

/// Returns why the model does not resolve `what` an operator wrote while the declarations did not
/// build, which a later operator removed before any build gave it an id.
fn unbuilt_gone(what: &str) -> String {
    format!("{what} written while the declarations did not build was removed before any build")
}

/// Returns each failure of the declarations `model` holds, by the declaration it is in and its
/// message.
fn failures(model: &Model) -> BTreeSet<(Item, String)> {
    let Err(ds) = model.replay_source().check() else {
        return BTreeSet::new();
    };
    ds.into_iter()
        .filter(|d| d.diag.kind != crate::span::DiagKind::Poisoned)
        .filter_map(|d| Some((model.decls.get(d.file.0 as usize)?.item, d.diag.message)))
        .collect()
}

impl Model {
    /// Returns each name of every declaration as the model prints it, with the objects it
    /// holds.
    fn named(&self) -> Vec<(String, Vec<Obj>)> {
        let mut out = Vec::new();
        let printing = self.clone();
        self.clone()
            .terms_mut(&mut |t| out.extend(t.named(&printing)));
        out
    }

    /// Refuse when a name of an object `before` names otherwise resolves otherwise in the build
    /// of a declaration that can bind names: a name a local binder of its spelling captures,
    /// naming the declaration and the binder. Accept declarations that do not build.
    ///
    /// Fail with a model fault on such a name the build resolves to other objects.
    fn faithful(&self, before: &Model, run: &mut Run) -> Res<()> {
        let all = self.named();
        let was: Vec<&(String, Vec<Obj>)> = all
            .iter()
            .filter(|(n, ids)| {
                ids.iter()
                    .any(|&o| before.name(o).is_some_and(|b| b != n.as_str()))
            })
            .collect();
        if was.is_empty() {
            return Ok(());
        }
        let Ok((built, _)) = self.built(&[], &run.ahead(self)) else {
            return Ok(());
        };
        for i in 0..self.decls.len() {
            if matches!(
                self.decls[i].item,
                Item::Type(_) | Item::Set(_) | Item::Held(..)
            ) {
                continue;
            }
            let mut lifted = self.clone();
            lifted.relift(&built, self.decls[i].item)?;
            let now = lifted.named();
            for (name, ids) in was.iter().copied() {
                let count = |m: &[(String, Vec<Obj>)], same: bool| {
                    m.iter()
                        .filter(|(n, x)| n == name && (!same || x == ids))
                        .count()
                };
                if count(&now, true) >= count(&all, true) {
                    continue;
                }
                if count(&now, false) >= count(&all, false) {
                    return broken(format!(
                        "`{name}` in {} resolves to other objects than it names",
                        self.shown_decl(i)
                    ));
                }
                return refuse(format!(
                    "`{name}` in {} would name the local binder `{name}`",
                    self.shown_decl(i)
                ));
            }
        }
        Ok(())
    }
}
