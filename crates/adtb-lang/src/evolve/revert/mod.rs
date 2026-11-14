use super::*;
use crate::ids::{KeyKind, Owner, Space};

mod by_id;
mod inverse;
mod spelled;
mod structural;

pub(super) use by_id::Recreated;
use by_id::{DeclOf, NamedId, StateOf, again_id, dropped, holding, recreate, removed};
use inverse::Run;
use spelled::{Late, Spelled, late, spellings};
use structural::Undo;

/// The steps of the inverse of a run of operators, each run ending at a source that builds, with
/// each object the run removed, by its key before the run, and its id.
type Undoing = (Vec<Inverse>, Vec<(ObjectKey, u64)>);

/// How an inverse applies an operator of its text.
#[derive(Clone, Copy)]
pub(super) enum How<'a> {
    /// As the operator written.
    Written,
    /// A merge whose arms' fixed values tell its constructors apart.
    Derived,
    /// A move to the header of the field of every constructor of the slice.
    UpFrom(&'a [String]),
}

/// One step of the inverse of an operator.
#[derive(Clone, Debug)]
pub(super) enum Inverse {
    /// Rename the object of an id to `to`; `of` is the text of the operator inverted.
    Rename {
        object: NamedId,
        to: String,
        of: String,
    },
    /// Drop the named set of an id, whatever its current name and relation; `of` is the text of
    /// the operator inverted.
    DropSet { set: CatSetId, of: String },
    /// Add named set `name` of a relation over constructors, each by id; `of` is the text of the
    /// operator inverted.
    AddSet {
        relation: CatRelId,
        name: String,
        members: Vec<CatCtorId>,
        of: String,
    },
    /// Drop each unique and index of `drops`, then apply operator `op` as `how` says, setting
    /// each check of `checks` to its text in the same step, then the operators of `adds`, each a
    /// text whose names are held by id applied in the migration's module; `of` is the text of the
    /// operator inverted.
    Restructure {
        drops: Vec<CatIndexId>,
        op: Spelled,
        adds: Spelled,
        how: Undo,
        checks: Vec<(CatConstraintId, Spelled)>,
        of: String,
    },
    /// Apply operators, as a text whose names are held by id, in the migration's module; `of` is
    /// the text of the operator inverted.
    Apply { text: Spelled, of: String },
    /// Remove the constructor of an id, with its arms; with `record`, then set its type, when a
    /// sum of one constructor named after it, to a record of its fields, those of `record` in
    /// that order first; `of` is the text of the operator inverted.
    Unadd {
        ctor: CatCtorId,
        record: Option<Vec<CatFieldId>>,
        of: String,
    },
    /// Put back constructor `name` of a type as `text`, after the constructor of `after`, else
    /// first, as a member of each set of `joins`, with its arms in each constraint of `arms`, in
    /// the match over the type at the position among those of the constraint, after the arm over
    /// the constructor beside them, else last, or first without one; `ctor` is the id it held,
    /// which the arms name it by; `of` is the text of the operator inverted.
    Readd {
        ty: CatTypeId,
        ctor: CatCtorId,
        name: String,
        text: Spelled,
        after: Option<CatCtorId>,
        joins: Vec<CatSetId>,
        arms: Vec<structural::PlacedArms>,
        of: String,
    },
    /// Set write states back, `None` removing the state; `of` is the text of the operator
    /// inverted.
    States {
        states: Vec<(StateOf, Option<State>)>,
        of: String,
    },
    /// Set what each role holds of a privilege back.
    Grants { back: Vec<HeldGrant> },
    /// Add declaration `text`, of kind `kind` and name `name`, to module `module`; `of` is the
    /// text of the operator inverted.
    Restore {
        module: String,
        kind: &'static str,
        name: String,
        text: Spelled,
        of: String,
    },
    /// Set the declaration of an object to `text`, `None` removing it; `of` is the text of the
    /// operator inverted.
    Redeclare {
        decl: DeclOf,
        text: Option<Spelled>,
        of: String,
    },
    /// Add module `name` with the declaration texts `decls`, or with `present` false remove it.
    Module {
        name: String,
        present: bool,
        installed: bool,
        decls: Vec<Spelled>,
        of: String,
    },
    /// Apply a logged migration's operators again, as `text`, a `revert` among them naming each
    /// object the migration removed and the operator inverted created again by its id in
    /// `again`; `of` is the text of the operator inverted.
    Replay {
        logged: Box<Logged>,
        text: Spelled,
        again: Recreated,
        of: String,
    },
    /// Set a relation's key clause to its text, generator included; `of` is the text of the
    /// operator inverted.
    KeyClause {
        relation: CatRelId,
        text: Spelled,
        of: String,
    },
    /// Put the fields of a constructor, or of the header when `ctor` is `None`, in `order`; `of`
    /// is the text of the operator inverted.
    Reorder {
        ty: CatTypeId,
        ctor: Option<CatCtorId>,
        order: Vec<CatFieldId>,
        of: String,
    },
}

/// Returns the steps that set back each privilege an operator changed for a role, as `before`
/// held it, from `after`: the first removes what `after` holds and `before` does not, named by
/// the ids of `after`, and runs before the operator's inverse; the second restores what `before`
/// held, named by the ids of `before`, and runs after it. Either is `None` when it changes
/// nothing.
fn grants_back(before: &Source, after: &Source) -> (Option<Inverse>, Option<Inverse>) {
    let held = |grants: &[LogGrant], g: &LogGrant| {
        grants
            .iter()
            .position(|x| x.role == g.role && x.privilege == g.privilege)
            .map(|i| (i, grants[i].grant_option, grants[i].dormant))
    };
    let mut back = Vec::new();
    for g in before.grants.iter().chain(&after.grants) {
        let was = held(&before.grants, g);
        let is = held(&after.grants, g);
        let changed = was.map(|(_, o, d)| (o, d)) != is.map(|(_, o, d)| (o, d));
        if changed
            && !back
                .iter()
                .any(|h: &HeldGrant| h.role == g.role && h.privilege == g.privilege)
        {
            back.push(HeldGrant {
                role: g.role.clone(),
                privilege: g.privilege.clone(),
                held: was,
            });
        }
    }
    let (gone, kept): (Vec<HeldGrant>, Vec<HeldGrant>) =
        back.into_iter().partition(|h| h.held.is_none());
    (
        (!gone.is_empty()).then_some(Inverse::Grants { back: gone }),
        (!kept.is_empty()).then_some(Inverse::Grants { back: kept }),
    )
}

impl Source {
    /// Returns the number of each migration after the one at `pos` of the log that names an object
    /// the one at `pos` created, by operand, through an object an operand names inside, in a text
    /// it copies into the schema, or by its module, with the least such object as the source
    /// before it names it.
    ///
    /// Fail with the diagnostics of a later migration that does not resolve.
    pub(super) fn dependants(&self, pos: usize) -> Result<Vec<(u64, String)>, Vec<Diag>> {
        let made = self.created(pos);
        if made.is_empty() {
            return Ok(Vec::new());
        }
        let every: Vec<&Source> = self.log.iter().map(|l| &*l.before).chain([self]).collect();
        let all = Keys::of(&every);
        let mut out = Vec::new();
        for (i, later) in self.log.iter().enumerate().skip(pos + 1) {
            let keys = Spelling {
                near: Keys::of(&[&later.before, self.after(i), self.after(pos)]),
                all: &all,
            };
            let named = keys.with_owners(self.named_by(later.number)?);
            if let Some(o) = named.intersection(&made).next() {
                out.push((later.number, keys.shown(o)));
            }
        }
        Ok(out)
    }

    /// Returns the catalog objects and modules the source after the migration at `pos` of the log
    /// holds and the source before it does not.
    pub(super) fn created(&self, pos: usize) -> BTreeSet<Reached> {
        let held = |s: &Source| -> BTreeSet<Reached> {
            s.ids
                .iter()
                .map(|(k, &id)| Reached::Object(k.kind().space(), id))
                .chain(s.modules.iter().cloned().map(Reached::Module))
                .collect()
        };
        let before = held(&self.log[pos].before);
        held(self.after(pos)).difference(&before).cloned().collect()
    }

    /// Returns the id each object the migration at `pos` of the log removed takes in `after`,
    /// for each one `after` holds again under an id `before` lacks.
    pub(super) fn recreated_by(&self, pos: usize, before: &Source, after: &Source) -> Recreated {
        let mut again = Recreated::new();
        let gone = removed(&self.log[pos].before, self.after(pos));
        recreate(&mut again, &gone, &holding(&before.ids), &after.ids);
        again
    }

    /// Returns the source after the migration at `pos` of the log.
    fn after(&self, pos: usize) -> &Source {
        self.log.get(pos + 1).map_or(self, |l| &*l.before)
    }
}

/// Returns the relation variables the inverse of `logged` adds back, each typed when the
/// migration also drops the type the source before it declares the relation over.
fn inverse_ahead(logged: &Logged) -> Vec<schema::Ahead> {
    let module_of = |q: &QName| {
        q.module
            .as_ref()
            .map_or(logged.module.clone(), |m| m.text.clone())
    };
    let dropped: BTreeSet<(String, String)> = logged
        .operators
        .iter()
        .filter_map(|op| match &op.kind {
            OperatorKind::DropType(t) => Some((module_of(t), t.name.text.clone())),
            _ => None,
        })
        .collect();
    logged
        .operators
        .iter()
        .filter_map(|op| match &op.kind {
            OperatorKind::DropRelation(r) => {
                let m = module_of(r);
                let ty = logged.before.relation_type(&m, &r.name.text);
                Some(schema::Ahead {
                    module: m,
                    relation: r.name.text.clone(),
                    typed: ty.is_some_and(|t| dropped.contains(&t)),
                })
            }
            _ => None,
        })
        .collect()
}

/// The keys of the objects of several sources, by the space of the id and the id, the first
/// source holding an id giving its key.
struct Keys<'a>(BTreeMap<(Space, u64), &'a ObjectKey>);

impl<'a> Keys<'a> {
    fn of(sources: &[&'a Source]) -> Keys<'a> {
        let mut out = BTreeMap::new();
        for s in sources {
            for (k, &id) in &s.ids {
                out.entry((k.kind().space(), id)).or_insert(k);
            }
        }
        Keys(out)
    }
}

/// The keys of the sources around a later migration, else of any source of the log.
struct Spelling<'a> {
    near: Keys<'a>,
    all: &'a Keys<'a>,
}

impl<'a> Spelling<'a> {
    /// Returns the key of an object, `None` when no source of the log holds it.
    fn get(&self, space: Space, id: u64) -> Option<&'a ObjectKey> {
        let at = (space, id);
        self.near
            .0
            .get(&at)
            .or_else(|| self.all.0.get(&at))
            .copied()
    }

    fn key(&self, space: Space, id: u64) -> &'a ObjectKey {
        self.get(space, id).unwrap_or_else(|| {
            panic!("an object a migration created, and each object owning it, is held by some source of the log: {space:?} {id}")
        })
    }

    /// Returns `named` with what owns each object of it, objects and modules, transitively.
    fn with_owners(&self, named: BTreeSet<Reached>) -> BTreeSet<Reached> {
        let mut out = named.clone();
        let mut todo: Vec<Reached> = named.into_iter().collect();
        while let Some(r) = todo.pop() {
            let Reached::Object(space, id) = r else {
                continue;
            };
            let owner = match self.get(space, id).map(ObjectKey::owner) {
                Some(Owner::Module(m)) => Reached::Module(m.clone()),
                Some(Owner::Object(space, o)) => Reached::Object(*space, *o),
                None => continue,
            };
            if out.insert(owner.clone()) {
                todo.push(owner);
            }
        }
        out
    }

    /// Returns `r` as a refusal names it: its kind and its name qualified by its owners.
    fn shown(&self, r: &Reached) -> String {
        match r {
            Reached::Module(m) => format!("module `{m}`"),
            Reached::Object(space, id) => {
                let key = self.key(*space, *id);
                let word = match key.kind() {
                    KeyKind::Type => "type",
                    KeyKind::Ctor => "constructor",
                    KeyKind::Relation => "relation",
                    KeyKind::Field => "field",
                    KeyKind::Set => "set",
                    KeyKind::Function => "function",
                    KeyKind::Constraint => "constraint",
                    KeyKind::Computed => "computed field",
                    KeyKind::Key => "the key index of",
                    KeyKind::Unique => "a unique of",
                    KeyKind::Index | KeyKind::IntervalIndex => "an index of",
                    KeyKind::Implied => "an implied index of",
                    KeyKind::Generated => "the generated default of",
                    KeyKind::Default => "a default of",
                };
                format!("{word} `{}`", self.qualified(key))
            }
        }
    }

    /// Returns the name of the object of `key` qualified by its owners, the owner's alone for an
    /// object with no name.
    fn qualified(&self, key: &ObjectKey) -> String {
        let owner = match key.owner() {
            Owner::Module(m) => m.clone(),
            Owner::Object(space, o) => self.qualified(self.key(*space, *o)),
        };
        match (key.name(), key.owner()) {
            (None, _) => owner,
            (Some(n), Owner::Module(_)) => format!("{owner}::{n}"),
            (Some(n), Owner::Object(..)) => format!("{owner}.{n}"),
        }
    }
}

impl Cx<'_> {
    pub(super) fn revert(&mut self, migration: &str, fills: &[(ast::FieldPath, Fill)]) -> R<()> {
        let number: u64 = migration.parse().unwrap_or(u64::MAX);
        let Some(pos) = self.src.log.iter().position(|l| l.number == number) else {
            return self.err(format!("migration {migration} is not in the log"));
        };
        let logged = self.src.log[pos].clone();
        if logged
            .operators
            .iter()
            .any(|o| matches!(o.kind, OperatorKind::Script(_)))
        {
            return self.err(format!(
                "the inverse of migration {migration} is undefined: it holds a `script`"
            ));
        }
        let unresolved = |ds: Vec<Diag>| -> Vec<Diag> {
            ds.into_iter()
                .map(|d| {
                    Diag::new(
                        self.span,
                        format!("the log does not resolve: {}", d.message),
                    )
                })
                .collect()
        };
        let dependants = if self.src.replaying.is_some() {
            Vec::new()
        } else {
            self.src.dependants(pos).map_err(unresolved)?
        };
        if let Some((later, object)) = dependants.first() {
            return self.err(format!(
                "`revert {migration}` is refused while migration {later:04} depends on it: it names {object}"
            ));
        }
        let mut state = (*logged.before).clone();
        state.version = logged.number;
        state.expecting(&logged.module, &logged.operators);
        let mut steps = Vec::new();
        let mut start = state.clone();
        let mut group: Vec<Vec<Inverse>> = Vec::new();
        let spelling = std::cell::OnceCell::new();
        for (i, op) in logged.operators.iter().enumerate() {
            let before = state.clone();
            if let Err(ds) = state.apply(&logged.module, op, &logged.text) {
                return Err(ds
                    .into_iter()
                    .map(|d| {
                        Diag::new(self.span, format!("the log does not replay: {}", d.message))
                    })
                    .collect());
            }
            let run = Run::new(&start, &logged, &spelling, fills);
            group.push(self.inverse(&run.at(&before, &state, op))?);
            if state.built().is_err() && i + 1 < logged.operators.len() {
                continue;
            }
            let mut step: Vec<Inverse> = group.drain(..).rev().flatten().collect();
            let (gone, kept) = grants_back(&start, &state);
            if let Some(gone) = gone {
                step.insert(0, gone);
            }
            step.extend(kept);
            steps.push((step, removed(&start, &state)));
            start = state.clone();
        }
        let ahead = self.src.ahead.clone();
        self.src.ahead.extend(inverse_ahead(&logged));
        let late = late(self.src.after(pos));
        let undone = self.undo_all(steps, &late);
        self.src.ahead = ahead;
        undone
    }

    /// Apply the steps of the inverse of each run of operators of a migration, last run first.
    fn undo_all(&mut self, steps: Vec<Undoing>, late: &Late) -> R<()> {
        let mut again = self.src.replaying.clone().unwrap_or_default();
        let held = holding(&self.src.ids);
        for (step, gone) in steps.into_iter().rev() {
            for inv in step {
                let recorded = matches!(
                    inv,
                    Inverse::Restructure { .. }
                        | Inverse::Apply { .. }
                        | Inverse::Rename { .. }
                        | Inverse::DropSet { .. }
                        | Inverse::AddSet { .. }
                        | Inverse::Replay { .. }
                );
                self.undo(inv.mapped(&again), late)?;
                self.src.reconcile_ids();
                recreate(&mut again, &gone, &held, &self.src.ids);
                if let (Some(steps), false) = (self.steps, recorded) {
                    steps.borrow_mut().push(Step {
                        after: self.src.clone(),
                        maps: Vec::new(),
                        applied: None,
                    });
                }
            }
        }
        Ok(())
    }

    /// The text of the fill a revert gives for field path `path`.
    pub(super) fn fill_for(&self, fills: &[(ast::FieldPath, Fill)], path: &str) -> Option<Written> {
        fills
            .iter()
            .find(|(p, _)| path_text(p) == path)
            .map(|(_, f)| match f {
                Fill::Expr(e) => self.copy(e.span),
                Fill::Match(arms) => {
                    let start = arms[0].patterns[0].span.start as usize;
                    let end = arms.last().expect("an arm").body.span.end as usize;
                    Written::concat(["match\n  | ".into(), self.copy_range(start..end)])
                }
            })
    }

    /// Returns path `t.c.f` of module `m`, or `t.f` without `c`, each part that `src` holds an id
    /// for spelled as the source names that object now, the others as written.
    pub(super) fn named_now(
        &self,
        src: &Source,
        m: &str,
        t: &str,
        c: Option<&str>,
        f: &str,
    ) -> String {
        let now = spellings(self.src);
        let named = |space: Space, id: Option<u64>, written: &str| {
            id.and_then(|id| now.get(&(space, id)))
                .map_or(written, |n| n)
                .to_string()
        };
        let key = ObjectKey::named(KeyKind::Type, Owner::Module(m.to_string()), t);
        let mut out = named(KeyKind::Type.space(), src.ids.get(&key).copied(), t);
        if let Some(c) = c {
            out.push('.');
            out.push_str(&named(Space::Shared, src.ctor_key_id(m, t, c), c));
        }
        out.push('.');
        out.push_str(&named(Space::Field, src.field_key_id(m, t, c, f), f));
        out
    }

    /// The step that puts the fields of a part of `path`'s type back in the order `before` declares
    /// them, the operator inverted being `of`.
    pub(super) fn reorder(
        &self,
        before: &Source,
        path: &ast::FieldPath,
        ctor: Option<String>,
        of: String,
    ) -> R<Inverse> {
        let m = self.module_of(&path.ty);
        let t = &path.ty.name.text;
        let missing = || {
            vec![Diag::new(
                self.span,
                "the log does not hold the reordered type",
            )]
        };
        let ty = before
            .ids
            .get(&ObjectKey::named(
                KeyKind::Type,
                Owner::Module(m.clone()),
                t,
            ))
            .copied()
            .ok_or_else(missing)?;
        let i = before.find("type", &m, t).ok_or_else(missing)?;
        let Some(Decl::Type(td)) = before.parsed(i) else {
            return Err(missing());
        };
        let record = matches!(td.body, ast::TypeBody::Record(_));
        let part = if record { Some(t.clone()) } else { ctor };
        let owner = before
            .field_owner(&m, t, part.as_deref())
            .ok_or_else(missing)?;
        let order = decl_fields(&td)
            .into_iter()
            .filter(|(c, _)| c.map(str::to_string) == part)
            .filter_map(|(_, f)| {
                let key = ObjectKey::named(KeyKind::Field, owner.clone(), &f.name.text);
                before.ids.get(&key).map(|&id| CatFieldId::from_raw(id))
            })
            .collect();
        let ctor = match part {
            Some(c) if !record => Some(CatCtorId::from_raw(
                before.ctor_key_id(&m, t, &c).ok_or_else(missing)?,
            )),
            _ => None,
        };
        Ok(Inverse::Reorder {
            ty: CatTypeId::from_raw(ty),
            ctor,
            order,
            of,
        })
    }

    /// Set type `ty`, a sum of one constructor named after it, to record text `text`, its header
    /// fields keeping their ids as fields of that constructor.
    pub(super) fn unsum(&mut self, ty: &QName, text: Written) -> R<()> {
        let (i, _) = self.type_decl(ty)?;
        self.src.rewrite(i, text);
        let (m, t) = (self.module_of(ty), &ty.name.text);
        if let (Some(header), Some(ctor)) = (
            self.src.field_owner(&m, t, None),
            self.src.field_owner(&m, t, Some(t)),
        ) {
            let fields: Vec<ObjectKey> = self
                .src
                .ids
                .keys()
                .filter(|k| k.kind() == KeyKind::Field && *k.owner() == header)
                .cloned()
                .collect();
            for f in fields {
                let to = f.owned_by(ctor.clone());
                self.src.rekey(&f, to);
            }
        }
        Ok(())
    }

    /// Apply operators given as source text in the migration's module as `how` says, then set
    /// the declaration of each of `checks` to its text within the last operator's step.
    fn undo_ops(&mut self, text: &Written, how: How, checks: &[(DeclOf, Written)]) -> R<()> {
        let span = self.span;
        let wrap = |ds: Vec<Diag>, what: &str| -> Vec<Diag> {
            ds.into_iter()
                .map(|d| {
                    Diag::new(
                        span,
                        format!("the inverse `{what}` does not apply: {}", d.message),
                    )
                })
                .collect()
        };
        let head = format!("module {}\n", self.module);
        let full = format!("{head}{}", text.text);
        let m = crate::parse::parse_migration(&full, Options::default())
            .ok()
            .map_err(|ds| wrap(ds, &text.text))?;
        let noted = RefCell::new(Names::default());
        for (k, op) in m.operators.iter().enumerate() {
            let before = self.src.clone();
            let mut sub = Cx {
                src: &mut *self.src,
                module: self.module.clone(),
                text: &full,
                span: op.span,
                noted: self.noted.map(|_| &noted),
                copying: false,
                steps: None,
            };
            match (&op.kind, how) {
                (
                    OperatorKind::MergeConstructor {
                        ty,
                        sel,
                        into,
                        arms,
                        clauses,
                    },
                    How::Derived,
                ) => sub.merge(ty, sel, into, arms.as_deref(), clauses, true),
                (OperatorKind::MoveField { path, fill, .. }, How::UpFrom(from)) => {
                    sub.move_up(path, from, fill.as_ref())
                }
                (kind, _) => sub.operator(kind),
            }
            .map_err(|ds| wrap(ds, &full[op.span.range()]))?;
            self.src.carry = None;
            self.src.rename_ids(&before, &self.module, &op.kind);
            if k + 1 == m.operators.len() {
                for (decl, check) in checks {
                    if let Some((i, ..)) = decl.find(self.src) {
                        self.src.rewrite(i, check.clone());
                    }
                }
            }
            self.src.reconcile_ids();
            if let Some(steps) = self.steps {
                let maps = before
                    .row_maps(&self.module, op, &full, self.src, how)
                    .map_err(|ds| wrap(ds, &full[op.span.range()]))?;
                steps.borrow_mut().push(Step {
                    after: self.src.clone(),
                    maps,
                    applied: Some(Applied {
                        module: self.module.clone(),
                        text: full.clone(),
                        op: op.clone(),
                    }),
                });
            }
        }
        if let Some(out) = self.noted {
            let noted = noted.into_inner();
            for (o, from) in &text.copies {
                let at = head.len() + o;
                out.borrow_mut()
                    .copy(&noted, at..at + from.len(), from.start);
            }
        }
        Ok(())
    }

    /// Apply one step of an inverse to the source, `late` holding the objects of the source after
    /// the reverted migration.
    pub(super) fn undo(&mut self, inv: Inverse, late: &Late) -> R<()> {
        match inv {
            Inverse::Restructure {
                drops,
                op,
                adds,
                how,
                checks,
                of,
            } => self.restructure(drops, [op, adds], how, checks, &of, late),
            Inverse::Apply { text, of } => {
                let text = self.spell(&text, &of, late)?;
                self.undo_ops(&text, How::Written, &[])
            }
            Inverse::Rename { object, to, of } => {
                let schema = self.schema()?;
                let Some((module, name)) = object.spelled(&schema) else {
                    return self.err(dropped(&of, object.word()));
                };
                self.undo_in(module, format!("rename {} {name} to {to}", object.word()))
            }
            Inverse::DropSet { set, of } => {
                let schema = self.schema()?;
                let Some((module, set)) = NamedId::Set(set).spelled(&schema) else {
                    return self.err(dropped(&of, "set"));
                };
                self.undo_in(module, format!("drop set {set}"))
            }
            Inverse::AddSet {
                relation,
                name,
                members,
                of,
            } => {
                let schema = self.schema()?;
                let Some(r) = schema.ids().relation_of(relation) else {
                    return self.err(dropped(&of, "relation"));
                };
                let rd = schema.relation(r);
                let mut ctors = Vec::new();
                for c in members {
                    let Some(c) = schema.ids().ctor_of(c) else {
                        return self.err(dropped(&of, "constructor"));
                    };
                    ctors.push(schema.ty(c.def()).ctor_def(c).name.clone());
                }
                let module = schema.module(rd.module).name.clone();
                let text = format!("add set {name} of {} = {}", rd.name, ctors.join(" | "));
                self.undo_in(module, text)
            }
            Inverse::Unadd { ctor, record, of } => self.unadd(ctor, record, &of),
            Inverse::Readd {
                ty,
                ctor,
                name,
                text,
                after,
                joins,
                arms,
                of,
            } => self.readd((ty, ctor, &name), &text, (after, joins, arms), &of, late),
            Inverse::States { states, of } => {
                let mut keyed = Vec::new();
                for (s, state) in states {
                    match s.key(self.src) {
                        Ok((relation, key)) => keyed.push((relation, key, state)),
                        Err(word) => return self.err(dropped(&of, word)),
                    }
                }
                for (relation, key, state) in keyed {
                    let map = if relation {
                        &mut self.src.states
                    } else {
                        &mut self.src.value_states
                    };
                    match state {
                        Some(s) => {
                            map.insert(key, s);
                        }
                        None => {
                            map.remove(&key);
                        }
                    }
                }
                Ok(())
            }
            Inverse::Grants { back } => {
                self.src.hold(back);
                Ok(())
            }
            Inverse::Restore {
                module,
                kind,
                name,
                text,
                of,
            } => self.restore_spelled(&module, kind, &name, &text, &of, late),
            Inverse::Redeclare { decl, text, of } => {
                let Some((i, kind, module, name)) = decl.find(self.src) else {
                    return self.err(dropped(&of, decl.word()));
                };
                match text {
                    Some(text) => {
                        let text = self.spell(&text, &of, late)?;
                        self.src.rewrite(i, text);
                    }
                    None => self.remove_decl(i, kind, &module, &name),
                }
                Ok(())
            }
            Inverse::Module {
                name,
                present,
                installed,
                decls,
                of,
            } => {
                let decls = decls
                    .iter()
                    .map(|d| self.spell(d, &of, late))
                    .collect::<R<Vec<Written>>>()?;
                self.restore_module(name, present, installed, decls)
            }
            Inverse::Reorder {
                ty,
                ctor,
                order,
                of,
            } => self.reorder_by_id(ty, ctor, order, &of, late),
            Inverse::KeyClause { relation, text, of } => {
                let Some(q) = NamedId::Relation(relation).qualified(self.src) else {
                    return self.err(dropped(&of, "relation"));
                };
                let text = self.spell(&text, &of, late)?;
                let (i, rd, ranges) = self.clauses(&q)?;
                if let Some(k) = rd
                    .clauses
                    .iter()
                    .position(|c| matches!(c, RelationClause::Key(..)))
                {
                    self.src.edit(i, vec![(ranges[k].clone(), text)]);
                }
                Ok(())
            }
            Inverse::Replay {
                logged,
                text,
                again,
                of,
            } => self.replay_spelled(*logged, &text, again, &of, late),
        }
    }

    /// Apply operator text `text` as an inverse in module `module`.
    fn undo_in(&mut self, module: String, text: String) -> R<()> {
        let reverting = std::mem::replace(&mut self.module, module);
        let done = self.undo_ops(&text.into(), How::Written, &[]);
        self.module = reverting;
        done
    }

    pub(super) fn replay(&mut self, l: &Logged) -> R<()> {
        for op in &l.operators {
            match self.steps {
                Some(steps) => {
                    let mut more = self.src.steps(&l.module, op, &l.text)?;
                    steps.borrow_mut().append(&mut more);
                }
                None => self.src.apply(&l.module, op, &l.text)?,
            }
        }
        Ok(())
    }

    /// Remove declaration `i`, of kind `kind` and name `name` in module `module`, with the
    /// write states of a relation or a type it declares.
    fn remove_decl(&mut self, i: usize, kind: &str, module: &str, name: &str) {
        self.src.remove_decl(i);
        if kind == "relation" {
            self.src
                .states
                .retain(|(m, r, _), _| !(*m == module && *r == name));
        }
        if kind == "type" {
            self.src
                .value_states
                .retain(|(m, t, _), _| !(*m == module && *t == name));
        }
    }

    fn restore_module(
        &mut self,
        name: String,
        present: bool,
        installed: bool,
        decls: Vec<Written>,
    ) -> R<()> {
        if present {
            self.src.modules.insert(name.clone());
            for d in decls {
                self.src.push(&name, d);
            }
            if installed {
                self.src.installed.insert(name);
            }
        } else if installed {
            self.uninstall(&ast::Ident {
                text: name,
                span: self.span,
            })?;
        } else {
            if self.src.decls.iter().any(|d| d.module == name) {
                return self.err(format!(
                    "`drop module {name}` is refused while the module holds a declaration"
                ));
            }
            self.src.modules.remove(&name);
            self.src.installed.remove(&name);
        }
        Ok(())
    }

    pub(super) fn reorder_fields(
        &mut self,
        ty: &QName,
        ctor: Option<&str>,
        order: &[String],
    ) -> R<()> {
        let (i, t) = self.type_decl(ty)?;
        let h = header(&self.src.decls[i].module).len();
        let text = self.src.decls[i].text.clone();
        let Some(braces) = field_braces(&text, h, &t, ctor) else {
            return Ok(());
        };
        let part = match (&t.body, &ctor) {
            (ast::TypeBody::Record(_), _) => Part::Ctor(t.name.text.clone()),
            (_, Some(c)) => Part::Ctor(c.to_string()),
            (_, None) => Part::Header,
        };
        let mut items: Vec<(String, String)> = Vec::new();
        for (c, f) in decl_fields(&t) {
            let here = match (&part, c) {
                (Part::Header, None) => true,
                (Part::Ctor(p), Some(c)) => p == c,
                _ => false,
            };
            if here {
                let start = f.name.span.start as usize - h;
                let end = item_end(&text, start);
                items.push((f.name.text.clone(), text[start..end].trim_end().to_string()));
            }
        }
        if items.is_empty() {
            return Ok(());
        }
        items.sort_by_key(|(n, _)| order.iter().position(|o| o == n).unwrap_or(usize::MAX));
        let inner: Vec<String> = items.into_iter().map(|(_, t)| t).collect();
        self.src
            .edit(i, vec![(braces, format!("{{ {} }}", inner.join(", ")))]);
        Ok(())
    }
}
