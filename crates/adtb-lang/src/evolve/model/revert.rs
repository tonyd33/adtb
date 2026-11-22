use super::field::{Applied, Maps};
use super::op::{Fail, Refused, Res, broken, refuse, refuse_at, unmodelled};
use super::run::Run;
use super::*;
use crate::ast::{self, Fill, OperatorKind};
use crate::canon::Names;
use crate::evolve::revert::Replayed;
use crate::evolve::revert::{
    DeclOf, Inverse, Late as Gone, NamedId, Recreated, Spelled, dropped, holding, recreate, word,
};
use crate::evolve::{Clause, Cx, Logged, Resolved, StateOf, Written};
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
    applied: Option<(String, String, ast::Operator)>,
}

impl Stepped {
    pub(super) fn after(&self) -> &Model {
        &self.after
    }

    pub(super) fn maps(&self) -> &[crate::evolve::RowMap] {
        &self.maps
    }

    /// Returns the module and the text of the operator the step applies, when it applies one.
    pub(super) fn applied(&self) -> Option<(&str, &str)> {
        self.applied
            .as_ref()
            .map(|(module, text, op)| (module.as_str(), &text[op.span.range()]))
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
    fn step(
        &mut self,
        after: &Model,
        maps: Maps,
        applied: Option<(String, String, ast::Operator)>,
    ) {
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

/// Returns the sources migration `logged` passes through as the model applies its operators
/// again, and the names of its text, each operand naming an object tied to its id; a refusal of
/// one of them is at `span`.
fn replayed(logged: &Logged, span: Span) -> Res<Replayed> {
    let model = of(&logged.before).map_err(Fail::Broken)?;
    let m = MigrationFile {
        module: ast::Ident {
            text: logged.module.clone(),
            span: Span::default(),
        },
        operators: logged.operators.clone(),
    };
    let got = model
        .migrate_keeping(&m, &logged.text, &[], false, true)
        .map_err(Fail::Broken)?;
    if let Some(kind) = got.stopped() {
        return unmodelled(kind.to_string());
    }
    if let Some(Err(why)) = got.ops().iter().find(|o| o.is_err()) {
        return refuse_at(span, format!("the log does not replay: {}", why.why()));
    }
    let mut first = model;
    first.version += 1;
    first.expecting(&m);
    let states: Vec<Source> = std::iter::once(first.replay_source())
        .chain(got.afters().iter().map(Model::replay_source))
        .collect();
    let spelling = match got.names() {
        Some(Ok(names)) => {
            let mut names = names.clone();
            for (k, op) in logged.operators.iter().enumerate() {
                crate::evolve::operands::tie_operands(
                    &states[k],
                    &states[k + 1],
                    &logged.module,
                    op,
                    &mut names,
                );
            }
            Ok(names)
        }
        Some(Err(ds)) => Err(ds.clone()),
        None => Err(vec![Diag::new(span, "the logged migration does not check")]),
    };
    Ok(Replayed::new(states, Some(spelling)))
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
        run: &Run,
        migration: &str,
        fills: &[(ast::FieldPath, Fill)],
    ) -> Res<Applied> {
        let mut src = self.source();
        src.accepting = run.is_accepting();
        src.replaying = self.replaying.clone();
        let number = src.logged_at(migration).map(|pos| src.log[pos].number);
        let inverted = {
            let cx = Cx {
                src: &mut src,
                module: run.module().to_string(),
                text: run.text(),
                span: run.span(),
                noted: None,
                copying: true,
                steps: None,
            };
            let failed = std::cell::RefCell::new(None);
            let mine = cx.inverted_with(migration, fills, |logged| {
                replayed(logged, run.span()).map_err(|e| {
                    failed.replace(Some(e));
                    Vec::new()
                })
            });
            let failed = failed.into_inner();
            if enabled() && !matches!(failed, Some(Fail::Unmodelled(_))) {
                let layer = cx.inverted(migration, fills);
                match &failed {
                    Some(e) => compare::replay_failed(run.text(), &layer, e),
                    None => compare::same_inverses(run.text(), &layer, &mine),
                }
            }
            if let Some(e) = failed {
                return Err(e);
            }
            mine
        };
        let (steps, gone, linked, filled) = match inverted {
            Ok(i) => i.into_parts(),
            Err(ds) => return refused(&ds),
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
        let maps = match &undone.steps {
            Ok(_) => Ok(Vec::new()),
            Err(why) => Err(why.clone()),
        };
        let after = self.ids().unwrap_or_else(|_| self.named_keys());
        let mut clauses = Vec::new();
        for key in &filled {
            let Some(key) = key else { continue };
            let Some(&field) = after.get(key).or_else(|| src.ids.get(key)) else {
                continue;
            };
            let Some(fill) = undone.fills.get(&field) else {
                return unmodelled("a revert fill no operator of its inverses resolves");
            };
            clauses.push(Clause::Fill {
                field,
                fill: fill.clone(),
            });
        }
        run.reverted(undone.steps);
        Ok((Resolved::Revert { migration: number }, clauses, maps))
    }

    /// Apply the steps of the inverse of each run of operators of a migration, last run first,
    /// `gone` holding the objects of the source after the reverted migration.
    fn undo_all(
        &mut self,
        run: &Run,
        steps: Vec<crate::evolve::revert::Undoing>,
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
    pub(super) fn spell(&self, text: &Spelled, of: &str, gone: &Gone) -> Res<String> {
        self.spell_copies(text, of, gone).map(|w| w.text)
    }

    /// Returns `text` spelled as [`Model::spell`] does, with the ranges of the revert's text it
    /// copies.
    pub(super) fn spell_copies(&self, text: &Spelled, of: &str, gone: &Gone) -> Res<Written> {
        let now = self.spellings();
        let now: BTreeMap<Obj, &str> = now.iter().map(|(&o, n)| (o, n.as_str())).collect();
        match text.spelled(&now, gone, of) {
            Ok(w) => Ok(w),
            Err(why) => refuse(why),
        }
    }

    /// Apply one step of an inverse, `gone` holding the objects of the source after the reverted
    /// migration.
    fn undo(&mut self, run: &Run, inv: Inverse, gone: &Gone, undone: &mut Undone) -> Res<()> {
        match inv {
            Inverse::Restructure {
                drops,
                op,
                adds,
                how,
                checks,
                of,
            } => self.restructure(run, (drops, op, adds, how, checks), &of, gone, undone),
            Inverse::Apply { text, of } => {
                let text = self.spell_copies(&text, &of, gone)?;
                self.undo_ops(run, run.module(), text, How::Written, &[], undone)
            }
            Inverse::Rename { object, to, of } => {
                let schema = self.schema_now(run)?.into_schema();
                let Some((module, name)) = object.spelled(&schema) else {
                    return refuse(dropped(&of, object.word()));
                };
                let text = format!("rename {} {name} to {to}", object.word());
                self.undo_ops(run, &module, text.into(), How::Written, &[], undone)
            }
            Inverse::DropSet { set, of } => {
                let schema = self.schema_now(run)?.into_schema();
                let Some((module, set)) = NamedId::Set(set).spelled(&schema) else {
                    return refuse(dropped(&of, "set"));
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
                of,
            } => self.add_set_back(run, relation, &name, members, &of, undone),
            Inverse::Unadd { ctor, record, of } => self.unadd(run, ctor.raw(), record, &of),
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
                    (ty.raw(), ctor.raw(), &name),
                    &text,
                    (after, joins, arms),
                    &of,
                    gone,
                )
            }
            Inverse::States { states, of } => self.set_states(states, &of),
            Inverse::Grants { back } => {
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
                    return refuse(dropped(&of, decl.word()));
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
            Inverse::Reorder {
                ty,
                ctor,
                order,
                of,
            } => self.reorder(
                ty.raw(),
                ctor.map(crate::ids::CatCtorId::raw),
                &order,
                &of,
                gone,
            ),
            Inverse::KeyClause { relation, text, of } => {
                let r = relation.raw();
                if !self.relations.contains_key(&r) {
                    return refuse(dropped(&of, "relation"));
                }
                let text = self.spell(&text, &of, gone)?;
                self.key_clause(r, &text)
            }
            Inverse::Replay {
                text, again, of, ..
            } => {
                let text = self.spell(&text, &of, gone)?;
                self.replay(run, &text, again, &of, undone)
            }
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
        run: &Run,
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
                ) => self
                    .merge(&sub, (ty, sel), into, arms.as_deref(), clauses, true)
                    .map(|(op, clauses, maps)| (crate::evolve::ResolvedOp { op, clauses }, maps)),
                (OperatorKind::MoveField { path, fill, .. }, How::UpFrom(from)) => self
                    .move_up_from(&sub, path, from, fill.as_ref())
                    .map(|(op, clauses, maps)| (crate::evolve::ResolvedOp { op, clauses }, maps)),
                (kind, _) => self.operator(&sub, kind),
            };
            let (resolved, maps) = match applied {
                Ok(x) => x,
                Err(Fail::Refused(r)) => return refuse(wrap(r.why())),
                Err(other) => return Err(other),
            };
            sub.take_reverted();
            match resolved.op {
                Resolved::AddField { field, fill, .. }
                | Resolved::MoveField {
                    field,
                    fill: Some(fill),
                    ..
                } => {
                    undone.fills.insert(field, fill);
                }
                _ => {}
            }
            self.restate_held()?;
            self.rebind(&before);
            undone.step(
                self,
                maps.map_err(|why| wrap(&why)),
                Some((module.to_string(), full.clone(), op.clone())),
            );
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

    /// Add set `name` of relation variable `relation` with constructors `members` back, refusing as
    /// the inverse of `of` when a later migration dropped one.
    fn add_set_back(
        &mut self,
        run: &Run,
        relation: crate::ids::CatRelId,
        name: &str,
        members: Vec<crate::ids::CatCtorId>,
        of: &str,
        undone: &mut Undone,
    ) -> Res<()> {
        let schema = self.schema_now(run)?.into_schema();
        let Some(r) = schema.ids().relation_of(relation) else {
            return refuse(dropped(of, "relation"));
        };
        let rd = schema.relation(r);
        let mut ctors = Vec::new();
        for c in members {
            let Some(c) = schema.ids().ctor_of(c) else {
                return refuse(dropped(of, "constructor"));
            };
            ctors.push(schema.ty(c.def()).ctor_def(c).name.clone());
        }
        let module = schema.module(rd.module).name.clone();
        let text = format!("add set {name} of {} = {}", rd.name, ctors.join(" | "));
        self.undo_ops(run, &module, text.into(), How::Written, &[], undone)
    }

    /// Set each write state of `states` to what it states, refusing as the inverse of `of` when a
    /// later migration dropped its relation variable or constructor.
    fn set_states(&mut self, states: Vec<(StateOf, Option<State>)>, of: &str) -> Res<()> {
        for (s, _) in &states {
            let (rel, ctor) = match *s {
                StateOf::Relation(r, c) => (Some(r.raw()), c.raw()),
                StateOf::Value(c) => (None, c.raw()),
            };
            if rel
                .is_some_and(|r| !self.relations.contains_key(&r) && !self.stores.contains_key(&r))
            {
                return refuse(dropped(of, "relation"));
            }
            if !self.ctors.contains_key(&ctor) {
                return refuse(dropped(of, "constructor"));
            }
        }
        for (s, state) in states {
            match (s, state) {
                (StateOf::Relation(r, c), Some(v)) => {
                    self.states.insert((r.raw(), c.raw()), v);
                }
                (StateOf::Relation(r, c), None) => {
                    self.states.remove(&(r.raw(), c.raw()));
                }
                (StateOf::Value(c), Some(v)) => {
                    self.value_states.insert(c.raw(), v);
                }
                (StateOf::Value(c), None) => {
                    self.value_states.remove(&c.raw());
                }
            }
        }
        Ok(())
    }

    /// Add declaration `text` of kind `kind` named `name` to module `module`, refusing while the
    /// module declares that name.
    fn restore(
        &mut self,
        run: &Run,
        (module, kind, name): (&str, &str, &str),
        text: &Spelled,
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
        run: &Run,
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
            _ => return unmodelled("a declaration other than a body set back by a revert"),
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
        run: &Run,
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
        run: &Run,
        text: &str,
        again: Recreated,
        of: &str,
        undone: &mut Undone,
    ) -> Res<()> {
        let m = match parse_migration(text, Options::default()).ok() {
            Ok(m) => m,
            Err(ds) => {
                let shown: Vec<String> = ds.into_iter().map(|d| d.message).collect();
                return refuse(format!(
                    "the inverse of `{of}` does not parse: {}",
                    shown.join("; ")
                ));
            }
        };
        let ahead = self.ahead.clone();
        self.expecting(&m);
        let replaying = self.replaying.replace(again);
        let mut sub = Run::new(&m, text).accepting(run.is_accepting()).nested();
        let mut out = Ok(());
        for (k, op) in m.operators.iter().enumerate() {
            sub.begin(k, op.span);
            let before = self.clone();
            match self.operator(&sub, &op.kind) {
                Ok((r, maps)) => {
                    if let Resolved::AddRelation { relation, .. } = r.op {
                        self.mark(Adder::Operator(k), Added::As(relation));
                        self.terms_mut(&mut |t| t.added(Adder::Operator(k), relation));
                    }
                    self.rebind(&before);
                    match sub.take_reverted() {
                        Some(steps) => undone.append(steps),
                        None => undone.step(
                            self,
                            maps,
                            Some((m.module.text.clone(), text.to_string(), op.clone())),
                        ),
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

    /// Put the fields of constructor `ctor` of type `ty`, or of its header, in `order`, refusing
    /// as the inverse of `of` when a later migration dropped one of them.
    fn reorder(
        &mut self,
        ty: u64,
        ctor: Option<u64>,
        order: &[crate::ids::CatFieldId],
        of: &str,
        gone: &Gone,
    ) -> Res<()> {
        let held = |space: Space, id: u64, kind: KeyKind| -> Res<bool> {
            let live = match space {
                Space::Field => self.fields.contains_key(&id),
                _ => self.ctors.contains_key(&id),
            };
            if !live && gone.contains_key(&(space, id)) {
                return refuse(dropped(of, word(kind)));
            }
            Ok(live)
        };
        if let Some(c) = ctor
            && !held(Space::Shared, c, KeyKind::Ctor)?
        {
            return Ok(());
        }
        let mut fields = Vec::new();
        for f in order {
            if held(Space::Field, f.raw(), KeyKind::Field)? {
                fields.push(f.raw());
            }
        }
        if !self.types.contains_key(&ty) {
            return refuse(dropped(of, "type"));
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
        ast::Decl::Store(s) => s.view.name.text.clone(),
    })
}

/// Returns the offset in declaration text `text` just past its name `name`.
fn name_end(text: &str, name: &str) -> Option<usize> {
    let toks = crate::lex::lex(text).0;
    toks.iter()
        .find(|t| &text[t.span.range()] == name)
        .map(|t| t.span.end as usize)
}
