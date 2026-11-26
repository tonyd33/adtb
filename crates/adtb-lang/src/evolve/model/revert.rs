use super::field::{Applied, Maps};
use super::op::{Fail, Refused, Res, StateKey, broken, refuse, refuse_at};
use super::run::Run;
use super::*;
use crate::ast::{self, Fill, OperatorKind};
use crate::canon::Names;
use crate::evolve::model::inverse::Replayed;
use crate::evolve::model::inverse::{
    DeclOf, Inverse, Late as Gone, Recreated, Term, UNHELD, holding, recreate,
};
use crate::evolve::{Clause, Cx, Failed, Logged, Resolved, Written};
use crate::ids::KeyKind;
use crate::parse::{Options, parse_migration};
use crate::span::{Diag, Span};

/// How a step of an inverse applies an operator of its text.
#[derive(Clone, Copy)]
pub(super) enum How<'a> {
    Written,
    /// A merge whose arms' fixed values tell its constructors apart.
    Derived,
    /// A move to the header of the field of each of these constructors.
    UpFrom(&'a [u64]),
}

/// What a revert's steps gather as they apply: each step, or why the first operator of their
/// texts that has no row maps has none, and the canonical form of the fill each `add field` or
/// move to the header among them takes, by the field.
pub(super) struct Undone {
    steps: Result<Vec<Stepped>, String>,
    fills: BTreeMap<u64, String>,
}

/// One step of a revert: the model after it, how it moves rows, and the operator it applies,
/// as its module, the text holding it and the operator, when it applies one.
#[derive(Debug)]
pub(super) struct Stepped {
    after: Model,
    maps: Vec<crate::evolve::RowMap>,
    applied: Option<crate::evolve::ResolvedOp>,
}

impl Stepped {
    pub(super) fn after(&self) -> &Model {
        &self.after
    }

    pub(super) fn maps(&self) -> &[crate::evolve::RowMap] {
        &self.maps
    }

    /// Returns the step as a source step, `after` in place of its own model when given.
    pub(super) fn into_step(self, after: Option<&Model>) -> crate::evolve::Step {
        let after = after.unwrap_or(&self.after);
        crate::evolve::Step::new(
            after.replay_source(),
            self.maps,
            self.applied,
            after.step_images(),
        )
    }
}

/// The steps of a revert, or why the first operator of its inverses that has no row maps has
/// none.
pub(super) type Steps = Result<Vec<Stepped>, String>;

impl Default for Undone {
    fn default() -> Undone {
        Undone {
            steps: Ok(Vec::new()),
            fills: BTreeMap::new(),
        }
    }
}

impl Undone {
    /// Add a step leaving `after` that moves rows as `maps` and applies `applied`; keep the
    /// first failure.
    fn step(&mut self, after: &Model, maps: Maps, applied: Option<crate::evolve::ResolvedOp>) {
        let Ok(steps) = &mut self.steps else {
            return;
        };
        match maps {
            Ok(maps) => steps.push(Stepped {
                after: after.clone(),
                maps,
                applied,
            }),
            Err(why) => self.steps = Err(why),
        }
    }

    /// Add the steps of a revert applied among the steps; keep the first failure.
    fn append(&mut self, more: Steps) {
        match (&mut self.steps, more) {
            (Ok(all), Ok(more)) => all.extend(more),
            (Ok(_), Err(why)) => self.steps = Err(why),
            (Err(_), _) => {}
        }
    }

    /// Add a step leaving `after` that moves no rows and applies no operator.
    pub(super) fn unmapped(&mut self, after: &Model) {
        self.step(after, Ok(Vec::new()), None);
    }
}

/// A migration of the log applied again by the model: the source before its first operator and
/// after each; the names of its text, or the copies a later operator rewrote before they were
/// named; and what the model gave.
pub(in crate::evolve) type Replay = (Vec<Source>, Result<Names, Vec<Diag>>, Migrated);

/// Returns the migration file of `logged`.
pub(in crate::evolve) fn logged_file(logged: &Logged) -> MigrationFile {
    MigrationFile {
        module: ast::Ident {
            text: logged.module().to_string(),
            span: Span::default(),
        },
        operators: logged.operators().to_vec(),
    }
}

/// Apply migration `logged` again to the source before it.
///
/// Fail with the refusals of its operators that do not apply.
///
/// Fail with an internal diagnostic when the source before it does not lift or the model fails
/// on it.
pub(in crate::evolve) fn replay(logged: &Logged) -> crate::evolve::Replaying<Replay> {
    let m = logged_file(logged);
    let got =
        migrated(logged.before(), &m, logged.text(), &[], false, true).map_err(Failed::Broken)?;
    let refused: Vec<Diag> = got.refusals(&m).into_iter().map(|(_, d)| d).collect();
    if !refused.is_empty() || got.ops().iter().any(Result::is_err) {
        return Err(Failed::Refused(refused));
    }
    let mut first = of(logged.before())
        .map_err(|e| Failed::Broken(fault(format!("the source does not lift: {e}"))))?;
    first.version += 1;
    first.expecting(&m);
    let states: Vec<Source> = std::iter::once(first.replay_source())
        .chain(got.afters().iter().map(Model::replay_source))
        .collect();
    let names = match got.names() {
        Some(names) => names.clone(),
        None => Err(vec![Diag::new(
            Span::default(),
            "the logged migration does not check",
        )]),
    };
    Ok((states, names, got))
}

/// Returns the sources migration `logged` passes through as the model applies its operators
/// again, and the names of its text, each operand naming an object tied to its id; a refusal of
/// one of them is at `span`.
fn replayed(logged: &Logged, span: Span) -> Res<Replayed> {
    let (states, names, got) = match replay(logged) {
        Ok(r) => r,
        Err(Failed::Broken(d)) => return Err(Fail::Broken(d.message)),
        Err(Failed::Refused(ds)) => {
            let why = ds
                .first()
                .map_or("an operator does not apply", |d| d.message.as_str());
            return refuse_at(span, format!("the log does not replay: {why}"));
        }
    };
    let models = std::iter::once(got.start().clone())
        .chain(got.afters().iter().cloned())
        .collect();
    let ops = got.inputs().iter().flatten().cloned().collect();
    Ok(Replayed::new(
        (states, models),
        ops,
        Some(names.map_err(Failed::Refused)),
    ))
}

/// Returns the refusal of the first of `ds`.
fn refused<T>(ds: &[Diag]) -> Res<T> {
    match ds.first() {
        Some(d) => Err(Fail::Refused(Refused::at(d.span, d.message.clone()))),
        None => broken("a refusal with no reason"),
    }
}

impl Model {
    /// Apply `revert migration`, with fills `fills`, as the inverses of the migration's
    /// operators, last first, through the model's own operators.
    pub(super) fn revert(
        &mut self,
        run: &mut Run,
        migration: &str,
        fills: &[(ast::FieldPath, Fill)],
    ) -> Res<Applied> {
        let mut src = self.source();
        let number = src.logged_at(migration).map(|pos| src.log[pos].number());
        let inverted = {
            let cx = Cx {
                src: &mut src,
                module: run.module().to_string(),
                text: run.text(),
                span: run.span(),
                copying: true,
                accepting: run.is_accepting(),
                replaying: self.replaying.is_some(),
            };
            let failed = std::cell::RefCell::new(None);
            let inverted = cx.inverted_with(migration, fills, |logged| {
                replayed(logged, run.span()).map_err(|e| {
                    failed.replace(Some(e));
                    Failed::Refused(Vec::new())
                })
            });
            if let Some(e) = failed.into_inner() {
                return Err(e);
            }
            inverted
        };
        let (steps, gone, linked, filled) = match inverted {
            Ok(i) => i.into_parts(),
            Err(Failed::Broken(d)) => return Err(Fail::Broken(d.message)),
            Err(Failed::Refused(ds)) => return refused(&ds),
        };
        let number = number.expect("a reverted migration is in the log");
        let restoring: Vec<AheadM> = linked
            .iter()
            .map(|e| {
                let (module, name, typed, by) = e.parts();
                AheadM {
                    module: module.to_string(),
                    name: name.to_string(),
                    typed,
                    by,
                    added: Added::Not,
                    kind: AheadKind::Relation,
                }
            })
            .collect();
        let ahead = self.ahead.clone();
        self.ahead.extend(restoring);
        let mut undone = Undone::default();
        let applied = self.undo_all(run, steps, &gone, &mut undone);
        self.ahead = ahead;
        applied?;
        if let Err(why) = &undone.steps {
            return refuse(why.clone());
        }
        let after = self.ids().unwrap_or_else(|_| self.named_keys());
        let mut clauses = Vec::new();
        for key in &filled {
            let Some(key) = key else { continue };
            let Some(&field) = after.get(key).or_else(|| src.ids.get(key)) else {
                continue;
            };
            let Some(fill) = undone.fills.get(&field) else {
                return broken("a revert fill no operator of its inverses resolves");
            };
            clauses.push(Clause::Fill {
                field,
                fill: fill.clone(),
            });
        }
        run.reverted(undone.steps);
        Ok((
            Resolved::Revert { migration: number },
            clauses,
            Ok(Vec::new()),
        ))
    }

    /// Apply the steps of the inverse of each run of operators of a migration, last run first,
    /// `gone` holding the objects of the source after the reverted migration.
    fn undo_all(
        &mut self,
        run: &mut Run,
        steps: Vec<inverse::Undoing>,
        gone: &Gone,
        undone: &mut Undone,
    ) -> Res<()> {
        let mut again = self.replaying.clone().unwrap_or_default();
        let held = holding(&self.ids_now());
        for (s, (step, removed)) in steps.into_iter().enumerate().rev() {
            for (j, inv) in step.into_iter().enumerate() {
                let stepping = !matches!(
                    inv,
                    Inverse::Restructure { .. }
                        | Inverse::Apply { .. }
                        | Inverse::RestoreIndex { .. }
                        | Inverse::Rename { .. }
                        | Inverse::DropSet { .. }
                        | Inverse::AddSet { .. }
                        | Inverse::Replay { .. }
                );
                let restored = match &inv {
                    Inverse::RestoreRelation { module, name, .. } => {
                        Some((module.clone(), name.clone()))
                    }
                    _ => None,
                };
                self.undo(run, inv.mapped(&again), gone, undone)?;
                self.settle_ids(run);
                if let Some((module, name)) = restored
                    && let Some(r) = self.relation_named(&module, &name)
                {
                    let by = Adder::Inverse(s, j);
                    self.mark(by, Added::As(r));
                    self.terms_mut(&mut |t| t.added(by, r));
                }
                recreate(&mut again, &removed, &held, &self.ids_now());
                if stepping {
                    undone.unmapped(self);
                }
            }
        }
        Ok(())
    }

    /// Returns the id of each object of the declarations by its key; of each named one alone
    /// while they do not build.
    fn ids_now(&self) -> BTreeMap<ObjectKey, u64> {
        self.ids().unwrap_or_else(|_| self.named_keys())
    }

    /// Returns the name of each named object, by the space of its id and the id.
    fn spellings(&self) -> BTreeMap<Obj, String> {
        self.named_keys()
            .into_iter()
            .filter_map(|(k, id)| Some(((k.kind().space(), id), k.name()?.to_string())))
            .collect()
    }

    /// Returns `text` with each name spelled as the model names its object now, refusing as the
    /// inverse of operator text `of` when a later migration dropped one.
    pub(super) fn spell(&self, text: &Term, of: &str, gone: &Gone) -> Res<String> {
        self.spell_copies(text, of, gone).map(|w| w.text)
    }

    /// Returns `text` spelled as [`Model::spell`] does, with the ranges of the revert's text it
    /// copies.
    pub(super) fn spell_copies(&self, text: &Term, of: &str, gone: &Gone) -> Res<Written> {
        let now = self.spellings();
        let now: BTreeMap<Obj, &str> = now.iter().map(|(&o, n)| (o, n.as_str())).collect();
        match text.spelled(&now, gone, of) {
            Ok(w) => Ok(w),
            Err(why) => refuse(why),
        }
    }

    /// Apply one step of an inverse, `gone` holding the objects of the source after the reverted
    /// migration.
    fn undo(&mut self, run: &mut Run, inv: Inverse, gone: &Gone, undone: &mut Undone) -> Res<()> {
        self.gate(&inv, gone)?;
        match inv {
            Inverse::Restructure {
                drops,
                op,
                adds,
                how,
                checks,
                of,
            } => self.restructure(run, (drops, op, adds, how, checks), &of, gone, undone),
            Inverse::Apply { text, of } | Inverse::RestoreIndex { text, of, .. } => {
                let text = self.spell_copies(&text, &of, gone)?;
                self.undo_ops(run, run.module(), text, How::Written, &[], undone)
            }
            Inverse::Rename {
                object, word, to, ..
            } => {
                let Some((module, name)) = self.spelled_obj(object) else {
                    return broken(UNHELD);
                };
                let text = format!("rename {word} {name} to {to}");
                self.undo_ops(run, &module, text.into(), How::Written, &[], undone)
            }
            Inverse::DropSet { set, .. } => {
                let Some((module, set)) = self.spelled_obj((Space::Set, set)) else {
                    return broken(UNHELD);
                };
                self.undo_ops(
                    run,
                    &module,
                    format!("drop set {set}").into(),
                    How::Written,
                    &[],
                    undone,
                )
            }
            Inverse::AddSet {
                relation,
                name,
                members,
                ..
            } => self.add_set_back(run, relation, &name, members, undone),
            Inverse::Unadd { ctor, record, .. } => self.unadd(run, ctor, record),
            Inverse::Readd {
                ty,
                ctor,
                name,
                text,
                after,
                joins,
                arms,
                of,
            } => {
                let text = self.spell(&text, &of, gone)?;
                self.readd(
                    run,
                    (ty, ctor, &name),
                    &text,
                    (after, joins, arms),
                    &of,
                    gone,
                )
            }
            Inverse::States { states, .. } => self.set_states(states),
            Inverse::Grants { back, .. } => {
                crate::evolve::grant::hold(&mut self.grants, back);
                Ok(())
            }
            Inverse::Restore {
                module,
                kind,
                name,
                text,
                of,
            } => self.restore(run, (&module, kind, &name), &text, &of, gone),
            Inverse::RestoreRelation {
                module,
                name,
                text,
                of,
                ..
            } => self.restore(run, (&module, "relation", &name), &text, &of, gone),
            Inverse::Redeclare { decl, text, of } => {
                let Some(item) = self.decl_of(decl) else {
                    return broken(UNHELD);
                };
                match text {
                    Some(text) => {
                        let text = self.spell(&text, &of, gone)?;
                        self.redeclare(item, &text)
                    }
                    None => {
                        let before = self.clone();
                        self.remove(item);
                        self.rebind(&before);
                        Ok(())
                    }
                }
            }
            Inverse::Module {
                name,
                present,
                installed,
                decls,
                of,
            } => {
                let mut texts = Vec::new();
                for d in &decls {
                    texts.push(self.spell(d, &of, gone)?);
                }
                self.restore_module(run, &name, present, installed, texts)
            }
            Inverse::PlaceCtors {
                ty, order, placed, ..
            } => {
                self.place_ctors(ty, &order, &placed);
                Ok(())
            }
            Inverse::Reorder {
                ty, ctor, order, ..
            } => self.reorder(ty, ctor, &order),
            Inverse::KeyClause { relation, text, of } => {
                let text = self.spell(&text, &of, gone)?;
                self.key_clause(relation, &text)
            }
            Inverse::Replay {
                logged,
                texts,
                again,
                of,
            } => self.replay(run, (&logged, &texts), again, (&of, gone), undone),
        }
    }

    /// Returns the declaration of `decl`, when the model holds it.
    pub(super) fn decl_of(&self, decl: DeclOf) -> Option<Item> {
        let item = match decl {
            DeclOf::Store(f) => {
                let (&v, _) = self.stores.iter().find(|(_, s)| s.function == f)?;
                Item::Store(v)
            }
            DeclOf::Object(kind, id) => {
                let candidates: &[Item] = match kind {
                    KeyKind::Type => &[Item::Type(id)],
                    KeyKind::Relation => &[Item::Relation(id)],
                    KeyKind::Set => &[Item::Set(id)],
                    KeyKind::Function => &[Item::Fn(id), Item::Aggregate(id)],
                    KeyKind::Constraint => &[Item::Constraint(id), Item::Check(id)],
                    _ => &[],
                };
                return candidates
                    .iter()
                    .copied()
                    .find(|&i| self.decl_at(i).is_some());
            }
        };
        self.decl_at(item).map(|_| item)
    }

    /// Apply operators given as text `text` in module `module` as `how` says, the revert being
    /// `run`'s operator, each declaration of `checks` taking its text within the last operator's
    /// step, before that operator first builds.
    pub(super) fn undo_ops(
        &mut self,
        run: &mut Run,
        module: &str,
        written: Written,
        how: How,
        checks: &[(Item, String)],
        undone: &mut Undone,
    ) -> Res<()> {
        let text = written.text.as_str();
        let head = format!("module {module}\n");
        let full = format!("{head}{text}");
        let m = match parse_migration(&full, Options::default()).ok() {
            Ok(m) => m,
            Err(ds) => {
                return refuse(format!(
                    "the inverse `{text}` does not apply: {}",
                    ds.first().map_or("", |d| d.message.as_str())
                ));
            }
        };
        let mut sub = Run::new(&m, &full).accepting(run.is_accepting()).nested();
        let n = m.operators.len();
        for (k, op) in m.operators.iter().enumerate() {
            sub.begin(k, op.span);
            let before = self.clone();
            if k + 1 == n {
                self.restating = checks.to_vec();
            }
            let wrap = |why: &str| {
                format!(
                    "the inverse `{}` does not apply: {why}",
                    &full[op.span.range()]
                )
            };
            let applied = match (&op.kind, how) {
                (
                    OperatorKind::MergeConstructor {
                        ty,
                        sel,
                        into,
                        arms,
                        clauses,
                    },
                    How::Derived,
                ) => op::Resolving::new(self, module)
                    .ty(ty)
                    .and_then(|t| {
                        self.merge(&mut sub, (t, sel), into, arms.as_deref(), clauses, true)
                    })
                    .map(|(op, clauses, maps)| (crate::evolve::ResolvedOp { op, clauses }, maps)),
                (OperatorKind::MoveField { path, fill, .. }, How::UpFrom(from)) => self
                    .move_up_from(&mut sub, path, from, fill.as_ref())
                    .map(|(op, clauses, maps)| (crate::evolve::ResolvedOp { op, clauses }, maps)),
                (kind, _) => self.operator(&mut sub, kind),
            };
            let (resolved, maps) = match applied {
                Ok(x) => x,
                Err(Fail::Refused(r)) => return refuse(wrap(r.why())),
                Err(other) => return Err(other),
            };
            sub.keep();
            match &resolved.op {
                Resolved::AddField { field, fill, .. }
                | Resolved::MoveField {
                    field,
                    fill: Some(fill),
                    ..
                } => {
                    undone.fills.insert(*field, fill.clone());
                }
                _ => {}
            }
            self.restate_held()?;
            self.rebind(&before);
            undone.step(self, maps.map_err(|why| wrap(&why)), Some(resolved));
        }
        let noted = sub.take_noted();
        let mut copied = Names::default();
        for (o, from) in &written.copies {
            let at = head.len() + o;
            copied.copy(&noted, at..at + from.len(), from.start);
        }
        run.note_names(copied);
        Ok(())
    }

    /// Add set `name` of relation variable `relation` with constructors `members` back.
    fn add_set_back(
        &mut self,
        run: &mut Run,
        relation: u64,
        name: &str,
        members: Vec<u64>,
        undone: &mut Undone,
    ) -> Res<()> {
        let (Some(rel), Some(module)) = (
            self.relations.get(&relation),
            self.module_of((Space::Shared, relation)),
        ) else {
            return broken(UNHELD);
        };
        let (rel, module) = (rel.name.clone(), module.to_string());
        let mut ctors = Vec::new();
        for c in members {
            let Some(ctor) = self.ctors.get(&c) else {
                return broken(UNHELD);
            };
            ctors.push(ctor.name.clone());
        }
        let text = format!("add set {name} of {rel} = {}", ctors.join(" | "));
        self.undo_ops(run, &module, text.into(), How::Written, &[], undone)
    }

    /// Returns the module of object `obj` and its name as an operator naming it by its
    /// qualified name writes it, `None` when the model does not hold it.
    pub(super) fn spelled_obj(&self, obj: Obj) -> Option<(String, String)> {
        let ty = |t: u64| -> Option<(String, String)> {
            let m = self.module_of((Space::Shared, t))?.to_string();
            let q = format!("{m}::{}", self.types.get(&t)?.name);
            Some((m, q))
        };
        match obj {
            (Space::Shared, x) if self.types.contains_key(&x) => ty(x),
            (Space::Shared, x) if self.ctors.contains_key(&x) => {
                let c = &self.ctors[&x];
                let (m, t) = ty(c.ty)?;
                Some((m, format!("{t}.{}", c.name)))
            }
            (Space::Shared, x) => {
                let r = self.relations.get(&x)?;
                let m = self.module_of(obj)?.to_string();
                Some((m.clone(), format!("{m}::{}", r.name)))
            }
            (Space::Set, x) => {
                let set = self.sets.get(&x)?;
                let (m, r) = self.spelled_obj((Space::Shared, set.relation))?;
                Some((m, format!("{} of {r}", set.name)))
            }
            (Space::Field, x) => {
                let f = self.fields.get(&x)?;
                let t = self.field_type(x)?;
                let (m, q) = ty(t)?;
                Some(match self.ctors.get(&f.owner) {
                    Some(c) if !matches!(self.types[&t].body, TypeBody::Record { .. }) => {
                        (m, format!("{q}.{}.{}", c.name, f.name))
                    }
                    _ => (m, format!("{q}.{}", f.name)),
                })
            }
            _ => None,
        }
    }

    /// Set each write state of `states` to what it states.
    fn set_states(&mut self, states: Vec<(StateKey, Option<State>)>) -> Res<()> {
        let held = |s: &StateKey| match *s {
            StateKey::Relation(r, c) => {
                (self.relations.contains_key(&r) || self.stores.contains_key(&r))
                    && self.ctors.contains_key(&c)
            }
            StateKey::Value(c) => self.ctors.contains_key(&c),
        };
        if !states.iter().all(|(s, _)| held(s)) {
            return broken(UNHELD);
        }
        for (s, state) in states {
            match (s, state) {
                (StateKey::Relation(r, c), Some(v)) => {
                    self.states.insert((r, c), v);
                }
                (StateKey::Relation(r, c), None) => {
                    self.states.remove(&(r, c));
                }
                (StateKey::Value(c), Some(v)) => {
                    self.value_states.insert(c, v);
                }
                (StateKey::Value(c), None) => {
                    self.value_states.remove(&c);
                }
            }
        }
        Ok(())
    }

    /// Add declaration `text` of kind `kind` named `name` to module `module`, refusing while the
    /// module declares that name.
    fn restore(
        &mut self,
        run: &mut Run,
        (module, kind, name): (&str, &str, &str),
        text: &Term,
        of: &str,
        gone: &Gone,
    ) -> Res<()> {
        let k = op::Kind::named(kind);
        if k.is_some_and(|k| self.find(module, k, name).is_some()) {
            return refuse(format!(
                "module `{module}` already declares {kind} `{name}`"
            ));
        }
        let text = self.spell(text, of, gone)?;
        self.push_decl(run, module, k, name, text)
    }

    /// Add declaration `text`, of kind `kind` named `name`, to module `module` after the others:
    /// lifted when the declarations build with it, else held.
    fn push_decl(
        &mut self,
        run: &mut Run,
        module: &str,
        kind: Option<op::Kind>,
        name: &str,
        text: String,
    ) -> Res<()> {
        let decl = crate::evolve::SourceDecl {
            module: module.to_string(),
            text,
        };
        let at = crate::span::FileId(self.decls.len() as u32);
        match self.build_adopting(std::slice::from_ref(&decl), &run.ahead(self)) {
            Ok(built) => {
                let Some(d) = built.decl(at) else {
                    return broken("a restored declaration that does not parse alone");
                };
                let names = built.names(at);
                let ahead = built.ahead_ranges(at);
                lift::Lifting::new(built.schema(), at, built.text(at), &names, &ahead, module)
                    .decl(self, d)
                    .map_err(Fail::Broken)?;
                Ok(())
            }
            Err(_) => {
                let Some(kind) = kind.or_else(|| decl_kind(&decl.text)) else {
                    return broken("a restored declaration of no kind");
                };
                self.hold_text(module, kind, name, decl.text, false);
                Ok(())
            }
        }
    }

    /// Set each declaration [`Model::restating`] holds that is still declared to its text.
    pub(super) fn restate_held(&mut self) -> Res<()> {
        for (item, text) in std::mem::take(&mut self.restating) {
            if self.decl_at(item).is_some() {
                self.redeclare(item, &text)?;
            }
        }
        Ok(())
    }

    /// Set declaration `item` to text `text`, its terms lifted again once the declarations
    /// build.
    pub(super) fn redeclare(&mut self, item: Item, text: &str) -> Res<()> {
        let body = match item {
            Item::Fn(f) | Item::Aggregate(f) => self.functions.get_mut(&f),
            Item::Constraint(c) | Item::Check(c) => self.denials.get_mut(&c),
            _ => return broken("a declaration other than a body set back by a revert"),
        };
        let Some(body) = body else {
            return broken("a body the model does not hold");
        };
        let Some(at) = name_end(text, &body.name) else {
            return broken("a body set back whose text does not name it");
        };
        body.rest = Term::text(&text[at..]);
        if !self.unlifted.contains(&item) {
            self.unlifted.push(item);
        }
        Ok(())
    }

    /// Add module `name` with declarations `texts`, or without `present` remove it, as the
    /// inverse of adding, dropping, installing or uninstalling it.
    fn restore_module(
        &mut self,
        run: &mut Run,
        name: &str,
        present: bool,
        installed: bool,
        texts: Vec<String>,
    ) -> Res<()> {
        if present {
            self.modules.insert(name.to_string());
            for t in texts {
                let kind = decl_kind(&t);
                let named = decl_name(&t).unwrap_or_default();
                self.push_decl(run, name, kind, &named, t)?;
            }
            if installed {
                self.installed.insert(name.to_string());
            }
            return Ok(());
        }
        if installed {
            let _uninstalled = self.uninstall(&ast::Ident {
                text: name.to_string(),
                span: run.span(),
            })?;
            return Ok(());
        }
        if self.decls.iter().any(|d| d.module == name) {
            return refuse(format!(
                "`drop module {name}` is refused while the module holds a declaration"
            ));
        }
        self.modules.remove(name);
        self.installed.remove(name);
        Ok(())
    }

    /// Apply a logged migration's operators again as text `text`, its module header included,
    /// `again` mapping each object a `revert` among them names to the id the inverted operator
    /// created it again under.
    fn replay(
        &mut self,
        run: &mut Run,
        (logged, texts): (&Logged, &[Term]),
        again: Recreated,
        (of, gone): (&str, &Gone),
        undone: &mut Undone,
    ) -> Res<()> {
        let ahead = self.ahead.clone();
        self.expecting(&logged_file(logged));
        let replaying = self.replaying.replace(again);
        let mut out = Ok(());
        for (k, spelled) in texts.iter().enumerate() {
            let text = match self.spell(spelled, of, gone) {
                Ok(t) => t,
                Err(e) => {
                    out = Err(e);
                    break;
                }
            };
            let m = match parse_migration(&text, Options::default()).ok() {
                Ok(m) => m,
                Err(ds) => {
                    let shown: Vec<String> = ds.into_iter().map(|d| d.message).collect();
                    out = refuse(format!(
                        "the inverse of `{of}` does not parse: {}",
                        shown.join("; ")
                    ));
                    break;
                }
            };
            let Some(op) = m.operators.first() else {
                out = broken("a replayed operator's text holds no operator");
                break;
            };
            let mut sub = Run::new(&m, &text).accepting(run.is_accepting()).nested();
            sub.begin(k, op.span);
            let before = self.clone();
            match self.operator(&mut sub, &op.kind) {
                Ok((r, maps)) => {
                    if let Resolved::AddRelation { relation, .. } = &r.op {
                        let relation = *relation;
                        self.mark(Adder::Operator(k), Added::As(relation));
                        self.terms_mut(&mut |t| t.added(Adder::Operator(k), relation));
                    }
                    self.rebind(&before);
                    match sub.keep() {
                        Some(steps) => undone.append(steps),
                        None => undone.step(self, maps, Some(r)),
                    }
                }
                Err(e) => {
                    out = Err(e);
                    break;
                }
            }
        }
        self.replaying = replaying;
        self.ahead = ahead;
        out
    }

    /// Put each constructor of `placed` of type `ty`, in turn, after the nearest constructor
    /// before it in `order` the type holds, else first.
    fn place_ctors(&mut self, ty: u64, order: &[u64], placed: &[u64]) {
        let Some(TypeBody::Ctors(list)) = self.types.get_mut(&ty).map(|t| &mut t.body) else {
            return;
        };
        for x in placed {
            let (Some(at), Some(k)) = (
                list.iter().position(|c| c == x),
                order.iter().position(|c| c == x),
            ) else {
                continue;
            };
            let c = list.remove(at);
            let anchor = order[..k]
                .iter()
                .rev()
                .find_map(|y| list.iter().position(|c| c == y));
            list.insert(anchor.map_or(0, |p| p + 1), c);
        }
    }

    /// Put the fields of constructor `ctor` of type `ty`, or of its header, in `order`.
    fn reorder(&mut self, ty: u64, ctor: Option<u64>, order: &[u64]) -> Res<()> {
        if ctor.is_some_and(|c| !self.ctors.contains_key(&c)) {
            return Ok(());
        }
        let fields: Vec<u64> = order
            .iter()
            .copied()
            .filter(|f| self.fields.contains_key(f))
            .collect();
        if !self.types.contains_key(&ty) {
            return broken(UNHELD);
        }
        let part = match (&self.types[&ty].body, ctor) {
            (TypeBody::Record { ctor }, _) => Some(*ctor),
            (_, c) => c,
        };
        let list = match part {
            Some(c) => &mut self.ctors.get_mut(&c).expect("the constructor").fields,
            None => &mut self.types.get_mut(&ty).expect("the type").header,
        };
        if let Some(list) = list {
            list.sort_by_key(|f| fields.iter().position(|x| x == f).unwrap_or(usize::MAX));
        }
        Ok(())
    }
}

/// Returns the kind of declaration text `text` declares.
fn decl_kind(text: &str) -> Option<op::Kind> {
    let first = text.split_whitespace().next()?;
    op::Kind::named(first)
}

/// Returns the name declaration text `text` declares: a set's as `relation.set`.
fn decl_name(text: &str) -> Option<String> {
    let src = format!("module m\n{text}");
    let file = crate::parse::parse_schema(&src, Options { elisions: true })
        .ok()
        .ok()?;
    Some(match file.decls.first()? {
        ast::Decl::Type(t) => t.name.text.clone(),
        ast::Decl::Relation(r) => r.name.name.text.clone(),
        ast::Decl::Set(s) => format!("{}.{}", s.relation.text, s.name.text),
        ast::Decl::Fn(f) => f.name.text.clone(),
        ast::Decl::Aggregate(a) => a.name.text.clone(),
        ast::Decl::Constraint(c) => c.name.text.clone(),
        ast::Decl::Check(c) => c.name.text.clone(),
        ast::Decl::Store(s) => s.written(),
    })
}

/// Returns the offset in declaration text `text` just past its name `name`.
fn name_end(text: &str, name: &str) -> Option<usize> {
    let toks = crate::lex::lex(text).0;
    toks.iter()
        .find(|t| &text[t.span.range()] == name)
        .map(|t| t.span.end as usize)
}
