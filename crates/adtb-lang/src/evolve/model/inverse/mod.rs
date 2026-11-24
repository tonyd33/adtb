use crate::evolve::*;
use crate::ids::{KeyKind, Owner, Space};

use super::op::{Kind as OpKind, Op, StateKey};
use super::{IndexKind, Item, Model, Obj, TypeBody};

type R<T> = Replaying<T>;

mod by_id;
mod of_op;
mod structural;

pub(in crate::evolve) use super::Term;
pub(super) use by_id::Recreated;
use by_id::removed;
pub(in crate::evolve) use by_id::{DeclOf, dropped, holding, recreate};
use of_op::Run;
pub(in crate::evolve) use structural::{PlacedArms, Undo};

/// The `fill` clauses of a revert, each with the key of the field an inverse took it for.
pub(super) struct Fills<'a> {
    given: &'a [(ast::FieldPath, Fill)],
    taken: RefCell<Vec<Option<ObjectKey>>>,
}

impl<'a> Fills<'a> {
    fn new(given: &'a [(ast::FieldPath, Fill)]) -> Self {
        Fills {
            given,
            taken: RefCell::new(vec![None; given.len()]),
        }
    }
}

/// The parts of the inverse of an operator, with each object the operator removed, by its key
/// before it, and its id.
pub(in crate::evolve::model) type Undoing = (Vec<Inverse>, Vec<(ObjectKey, u64)>);

/// One part of the inverse of an operator.
#[derive(Clone, Debug)]
pub(in crate::evolve::model) enum Inverse {
    /// Rename object `object`, a `word`, to `to`; `of` is the text of the operator inverted.
    Rename {
        object: Obj,
        word: &'static str,
        to: String,
        of: String,
    },
    /// Drop the named set of an id, whatever its current name and relation; `of` is the text of
    /// the operator inverted.
    DropSet { set: u64, of: String },
    /// Add named set `name` of a relation over constructors, each by id; `of` is the text of the
    /// operator inverted.
    AddSet {
        relation: u64,
        name: String,
        members: Vec<u64>,
        of: String,
    },
    /// Drop each unique and index of `drops`, then apply operator `op` as `how` says, setting
    /// each check of `checks` to its text in the same step, then the operators of `adds`, each a
    /// text whose names are held by id applied in the migration's module; `of` is the text of the
    /// operator inverted.
    Restructure {
        drops: Vec<u64>,
        op: Term,
        adds: Term,
        how: Undo,
        checks: Vec<(u64, Term)>,
        of: String,
    },
    /// Apply operators, as a text whose names are held by id, in the migration's module; `of` is
    /// the text of the operator inverted.
    Apply { text: Term, of: String },
    /// Remove the constructor of an id, with its arms; with `record`, then set its type, when a
    /// sum of one constructor named after it, to a record of its fields, those of `record` in
    /// that order first; `of` is the text of the operator inverted.
    Unadd {
        ctor: u64,
        record: Option<Vec<u64>>,
        of: String,
    },
    /// Put back constructor `name` of a type as `text`, after the first constructor of `after`
    /// the type holds, else first, as a member of each set of `joins`, with its arms in each constraint of `arms`, in
    /// the match over the type at the position among those of the constraint, after the arm over
    /// the constructor beside them, else last, or first without one; `ctor` is the id it held,
    /// which the arms name it by; `of` is the text of the operator inverted.
    Readd {
        ty: u64,
        ctor: u64,
        name: String,
        text: Term,
        after: Vec<u64>,
        joins: Vec<u64>,
        arms: Vec<PlacedArms>,
        of: String,
    },
    /// Set write states back, `None` removing the state; `of` is the text of the operator
    /// inverted.
    States {
        states: Vec<(StateKey, Option<State>)>,
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
        text: Term,
        of: String,
    },
    /// Add back the declaration `text` of relation variable `name` of module `module`, `typed`
    /// when the migration also drops its type; `of` is the text of the operator inverted.
    RestoreRelation {
        module: String,
        name: String,
        text: Term,
        of: String,
        typed: bool,
    },
    /// Set the declaration of an object to `text`, `None` removing it; `of` is the text of the
    /// operator inverted.
    Redeclare {
        decl: DeclOf,
        text: Option<Term>,
        of: String,
    },
    /// Add module `name` with the declaration texts `decls`, or with `present` false remove it.
    Module {
        name: String,
        present: bool,
        installed: bool,
        decls: Vec<Term>,
        of: String,
    },
    /// Apply a logged migration's operators again, each as its text of `texts` spelled with the
    /// names the model holds as it reaches the operator, a `revert` among them naming each
    /// object the migration removed and the operator inverted created again by its id in
    /// `again`; `of` is the text of the operator inverted.
    Replay {
        logged: Box<Logged>,
        texts: Vec<Term>,
        again: Recreated,
        of: String,
    },
    /// Set a relation's key clause to its text, generator included; `of` is the text of the
    /// operator inverted.
    KeyClause {
        relation: u64,
        text: Term,
        of: String,
    },
    /// Put the fields of a constructor, or of the header when `ctor` is `None`, in `order`; `of`
    /// is the text of the operator inverted.
    Reorder {
        ty: u64,
        ctor: Option<u64>,
        order: Vec<u64>,
        of: String,
    },
    /// Put each constructor of `placed` of a type, in turn, after the nearest constructor before
    /// it in `order` the type holds, else first.
    PlaceCtors {
        ty: u64,
        order: Vec<u64>,
        placed: Vec<u64>,
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
    pub(in crate::evolve) fn dependants(&self, pos: usize) -> R<Vec<(u64, String)>> {
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
    pub(in crate::evolve) fn created(&self, pos: usize) -> BTreeSet<Reached> {
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
    pub(in crate::evolve) fn recreated_by(
        &self,
        pos: usize,
        before: &Source,
        after: &Source,
    ) -> Recreated {
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

/// Returns the relation variables the steps of `steps` add back, each added by its position.
fn linked(steps: &[Undoing]) -> Vec<Expected> {
    let mut out = Vec::new();
    for (s, (step, _)) in steps.iter().enumerate() {
        for (j, inv) in step.iter().enumerate() {
            if let Inverse::RestoreRelation {
                module,
                name,
                typed,
                ..
            } = inv
            {
                out.push(Expected::new(
                    module.clone(),
                    name.clone(),
                    *typed,
                    Adder::Inverse(s, j),
                ));
            }
        }
    }
    out
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

/// A migration of the log applied again: the source and the model before its first operator and
/// after each, the operators as the model resolved them, and the names of its text, each operand
/// naming an object tied to its id, when the replay gives them.
pub(in crate::evolve::model) struct Replayed {
    states: Vec<Source>,
    models: Vec<Model>,
    ops: Vec<Op>,
    spelling: Option<Replaying<Names>>,
}

impl Replayed {
    /// Returns the replay that passes through `states` and `models` applying `ops`, with
    /// `spelling` the names of its text.
    pub(in crate::evolve::model) fn new(
        (states, models): (Vec<Source>, Vec<Model>),
        ops: Vec<Op>,
        spelling: Option<Replaying<Names>>,
    ) -> Replayed {
        Replayed {
            states,
            models,
            ops,
            spelling,
        }
    }
}

/// The inverse of a migration of the log as a revert applies it: the steps of each run of its
/// operators, the objects of the source after it, the relation variables the steps add back,
/// and for each fill of the revert the key of the field an inverse took it for.
pub(in crate::evolve::model) struct Inverted {
    steps: Vec<Undoing>,
    late: Late,
    linked: Vec<Expected>,
    filled: Vec<Option<ObjectKey>>,
}

impl Inverted {
    /// Returns the steps of each run, the objects of the source after the reverted migration,
    /// the relation variables the steps restore, and the key of the field each fill was taken
    /// for.
    pub(in crate::evolve::model) fn into_parts(
        self,
    ) -> (Vec<Undoing>, Late, Vec<Expected>, Vec<Option<ObjectKey>>) {
        (self.steps, self.late, self.linked, self.filled)
    }
}

impl Cx<'_> {
    /// Returns the inverse of migration `migration` of the log, with the revert's fills `fills`,
    /// `replay` giving the sources the migration's operators pass through.
    ///
    /// Fail as a revert is refused before its inverse applies.
    pub(in crate::evolve::model) fn inverted_with(
        &self,
        migration: &str,
        fills: &[(ast::FieldPath, Fill)],
        replay: impl FnOnce(&Logged) -> R<Replayed>,
    ) -> R<Inverted> {
        let Some(pos) = self.src.logged_at(migration) else {
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
        let dependants = if self.replaying {
            Vec::new()
        } else {
            self.src
                .dependants(pos)
                .map_err(|f| f.map_refused(unresolved))?
        };
        if let Some((later, object)) = dependants.first() {
            return self.err(format!(
                "`revert {migration}` is refused while migration {later:04} depends on it: it names {object}"
            ));
        }
        let Replayed {
            states,
            models,
            ops,
            spelling,
        } = replay(&logged)?;
        let mut steps = Vec::new();
        let mut start = &states[0];
        let cell = std::cell::OnceCell::new();
        if let Some(names) = spelling {
            cell.set(names).ok();
        }
        let fills = Fills::new(fills);
        for (i, op) in logged.operators.iter().enumerate() {
            let (before, state) = (&states[i], &states[i + 1]);
            if i > 0 && before.built().is_ok() {
                start = before;
            }
            let around = ((before, &models[i]), (state, &models[i + 1]));
            let run = Run::new(&logged, &cell, &fills);
            let mut step = self.inverse(&run.at(around, start, op, &ops[i]))?;
            // A replay applies the grants and revokes of the migration it applies again.
            if !step.iter().any(|s| matches!(s, Inverse::Replay { .. })) {
                let (gone, kept) = grants_back(before, state);
                if let Some(gone) = gone {
                    step.insert(0, gone);
                }
                step.extend(kept);
            }
            steps.push((step, removed(before, state)));
        }
        if self.accepting && !self.replaying {
            self.untaken(migration, &fills)?;
        }
        Ok(Inverted {
            linked: linked(&steps),
            steps,
            late: late(self.src.after(pos)),
            filled: fills.taken.into_inner(),
        })
    }

    /// Refuse the first fill of a revert of migration `migration` that no inverse took.
    fn untaken(&self, migration: &str, fills: &Fills) -> R<()> {
        let taken = fills.taken.borrow();
        let Some(i) = taken.iter().position(Option::is_none) else {
            return Ok(());
        };
        let p = &fills.given[i].0;
        let from = p.ty.module.as_ref().map_or(p.ty.name.span, |m| m.span);
        let shown = self.slice(from.to(p.field.span));
        let keys = fill_keys(&self.src.ids, &self.module_of(&p.ty), p);
        if taken.iter().flatten().any(|k| keys.contains(k)) {
            return self.err(format!("the revert fills `{shown}` more than once"));
        }
        self.err(format!(
            "no inverse of migration {migration} needs `fill {shown}`"
        ))
    }

    /// Returns the text of the first fill not yet taken whose path names field `id` of `src`,
    /// taking it.
    ///
    /// A path names the field by its key in the source the revert applies to: the field's own
    /// key there while it holds `id`, else its key in `src`.
    pub(super) fn fill_for(&self, fills: &Fills, src: &Source, id: Option<u64>) -> Option<Written> {
        let key_in = |s: &Source| {
            s.ids
                .iter()
                .find(|(k, x)| k.kind() == KeyKind::Field && Some(**x) == id)
                .map(|(k, _)| k.clone())
        };
        let field = key_in(self.src).or_else(|| key_in(src))?;
        let mut taken = fills.taken.borrow_mut();
        let i = (0..fills.given.len()).find(|&i| {
            let p = &fills.given[i].0;
            taken[i].is_none()
                && fill_keys(&self.src.ids, &self.module_of(&p.ty), p).contains(&field)
        })?;
        taken[i] = Some(field);
        Some(match &fills.given[i].1 {
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
}

/// Returns the keys field path `p` of a fill, of module `m`, can name among `ids`: the owner's
/// catalog id and the field's name, the owner the constructor `p` names, else the header or the
/// constructor a record type names after itself.
pub(super) fn fill_keys(
    ids: &BTreeMap<ObjectKey, u64>,
    m: &str,
    p: &ast::FieldPath,
) -> Vec<ObjectKey> {
    let t = p.ty.name.text.as_str();
    let owners = match &p.ctor {
        Some(c) => vec![ids::field_owner(ids, m, t, Some(&c.text))],
        None => vec![
            ids::field_owner(ids, m, t, None),
            ids::field_owner(ids, m, t, Some(t)),
        ],
    };
    owners
        .into_iter()
        .flatten()
        .map(|o| ObjectKey::named(KeyKind::Field, o, &p.field.text))
        .collect()
}

/// The kind of each object the source after a reverted migration holds, by the space of its id
/// and the id.
pub(in crate::evolve) type Late = BTreeMap<(Space, u64), KeyKind>;

/// Returns the kind of each object `src` holds, by the space of its id and the id.
pub(super) fn late(src: &Source) -> Late {
    src.ids
        .iter()
        .map(|(k, &id)| ((k.kind().space(), id), k.kind()))
        .collect()
}

/// Returns the name of each named object `src` holds, by the space of its id and the id.
pub(super) fn spellings(src: &Source) -> BTreeMap<(Space, u64), &str> {
    src.ids
        .iter()
        .filter_map(|(k, &id)| Some(((k.kind().space(), id), k.name()?)))
        .collect()
}

/// Returns the word a refusal names an object of kind `kind` by.
pub(in crate::evolve) fn word(kind: KeyKind) -> &'static str {
    match kind {
        KeyKind::Type => "type",
        KeyKind::Ctor => "constructor",
        KeyKind::Relation => "relation",
        KeyKind::Field => "field",
        KeyKind::Set => "set",
        KeyKind::Function => "function",
        KeyKind::Constraint => "constraint",
        KeyKind::Computed => "computed field",
        KeyKind::Key
        | KeyKind::Unique
        | KeyKind::Index
        | KeyKind::IntervalIndex
        | KeyKind::Implied => "index",
        KeyKind::Generated | KeyKind::Default => "default",
    }
}
