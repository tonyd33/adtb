//! Migrations drawn over a generated schema: operators of every kind the log holds but
//! `install`, `uninstall` and `adopt tzdb`, each either meeting the preconditions its operator
//! states, so that the model of the schema after it is known, or breaking one stated
//! precondition, so that the migration is refused naming it.

use std::collections::{BTreeMap, BTreeSet};

use super::draw::{Draw, Keys};
use super::{Action, Case, Field, Schema, Scope, Stage, Ty, literal};
use crate::Rng;

mod model;
mod operators;
mod rows;
mod structural;

use rows::Effect;
pub use rows::Rows;

/// The role every drawn grant names; the database must hold it.
pub const ROLE: &str = "support";

/// The kind of a drawn operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    AddModule,
    DropModule,
    AddType,
    DropType,
    RenameType,
    AddRelation,
    DropRelation,
    RenameRelation,
    AddSet,
    DropSet,
    RenameSet,
    AddConstructor,
    RemoveConstructor,
    RenameConstructor,
    Enable,
    Disable,
    Deprecate,
    Split,
    Merge,
    Nest,
    Unnest,
    AddField,
    DropField,
    RenameField,
    MoveFieldUp,
    MoveFieldDown,
    AlterField,
    AddUnique,
    DropUnique,
    AddIndex,
    DropIndex,
    Rekey,
    AddDefault,
    DropDefault,
    DefineFn,
    RedefineFn,
    DropFn,
    DefineAggregate,
    DropAggregate,
    AddConstraint,
    DropConstraint,
    AddCheck,
    DropCheck,
    Store,
    Unstore,
    AddComputed,
    RedefineComputed,
    DropComputed,
    Grant,
    Revoke,
    Script,
}

/// The kinds drawn beside a broken operator: none removes or renames what it names.
const ADDING: [Kind; 10] = [
    Kind::AddModule,
    Kind::AddType,
    Kind::AddRelation,
    Kind::AddField,
    Kind::AddIndex,
    Kind::AddDefault,
    Kind::DefineFn,
    Kind::DefineAggregate,
    Kind::AddConstraint,
    Kind::AddComputed,
];

impl Kind {
    /// Every kind, a `remove` after the `deprecate` it needs in an earlier migration.
    pub const ALL: [Kind; 51] = [
        Kind::AddModule,
        Kind::DropModule,
        Kind::AddType,
        Kind::DropType,
        Kind::RenameType,
        Kind::AddRelation,
        Kind::DropRelation,
        Kind::RenameRelation,
        Kind::AddSet,
        Kind::DropSet,
        Kind::RenameSet,
        Kind::AddConstructor,
        Kind::RenameConstructor,
        Kind::Enable,
        Kind::Disable,
        Kind::Deprecate,
        Kind::RemoveConstructor,
        Kind::Split,
        Kind::Merge,
        Kind::Nest,
        Kind::Unnest,
        Kind::AddField,
        Kind::DropField,
        Kind::RenameField,
        Kind::MoveFieldUp,
        Kind::MoveFieldDown,
        Kind::AlterField,
        Kind::AddUnique,
        Kind::DropUnique,
        Kind::AddIndex,
        Kind::DropIndex,
        Kind::Rekey,
        Kind::AddDefault,
        Kind::DropDefault,
        Kind::DefineFn,
        Kind::RedefineFn,
        Kind::DropFn,
        Kind::DefineAggregate,
        Kind::DropAggregate,
        Kind::AddConstraint,
        Kind::DropConstraint,
        Kind::AddCheck,
        Kind::DropCheck,
        Kind::Store,
        Kind::Unstore,
        Kind::AddComputed,
        Kind::RedefineComputed,
        Kind::DropComputed,
        Kind::Grant,
        Kind::Revoke,
        Kind::Script,
    ];

    /// Returns whether a `revert` of the operator restores the data it found.
    fn exact(self) -> bool {
        !matches!(
            self,
            Kind::DropField | Kind::MoveFieldDown | Kind::Rekey | Kind::Script
        )
    }
}

/// A shape of operator a generated migration must reach under the profile `deep`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reach {
    /// A merge or nest of a declared and an enabled constructor.
    MergeAcrossStages,
    /// A `move field` down of a reference field whose delete action is not `restrict`.
    MovedDownAction,
    /// An `add constructor` that a function's match lacks an arm for until a later `redefine
    /// fn` of the migration.
    ArmLater,
    /// A merge of constructors a check over one of them names.
    MergeUnderCheck,
    /// An `add relation` after a `redefine fn` of a function whose signature names its key.
    RewrittenWaiting,
    /// An `add relation` after a `drop type` of a type whose field names its key.
    DroppedWaiting,
    /// A split of a constructor a unique names while the declarations do not build.
    ReplacedWhileUnbuilt,
    /// A broken `disable` of an enabled constructor a delete action retags into.
    DisableRetagTarget,
    /// A broken `deprecate` of an enabled constructor a delete action retags into.
    DeprecateRetagTarget,
    /// An operator whose inverse refuses while it is built, on a type and relation the
    /// migration adds and drops.
    RefusedInverseOfTransient,
}

impl Reach {
    pub const ALL: [Reach; 10] = [
        Reach::MergeAcrossStages,
        Reach::MovedDownAction,
        Reach::ArmLater,
        Reach::MergeUnderCheck,
        Reach::RewrittenWaiting,
        Reach::DroppedWaiting,
        Reach::ReplacedWhileUnbuilt,
        Reach::DisableRetagTarget,
        Reach::DeprecateRetagTarget,
        Reach::RefusedInverseOfTransient,
    ];
}

/// An operator of a drawn migration.
#[derive(Clone, Debug)]
pub struct Op {
    kind: Kind,
    /// The operator's text, without the migration's `module` line.
    text: String,
    /// The operator breaks a precondition its operator states.
    broken: bool,
    /// The operator replaces constructors a clause, check, constraint or default names where
    /// that object has no exact image, which breaks the precondition that it has one.
    unimaged: bool,
    /// Its preconditions read data, which may refuse it whole.
    data: bool,
    /// The relation or constructor relation whose rows a write deletes before the migration, so
    /// that a precondition asking it empty can hold.
    empties: Option<String>,
    effect: Effect,
    reach: Option<Reach>,
}

impl Op {
    /// Returns the operator's kind.
    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// Returns the operator's text, without the migration's `module` line.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns whether the operator breaks a precondition its operator states.
    pub fn broken(&self) -> bool {
        self.broken
    }

    /// Returns whether the operator replaces constructors an object with no exact image under it
    /// names.
    pub fn unimaged(&self) -> bool {
        self.unimaged
    }

    /// Returns whether the engine may name the operator refusing for data: its preconditions
    /// read data, and each line of its text is an operator the refusal may name.
    pub fn names(&self, operator: &str) -> bool {
        self.data && self.text.lines().any(|l| l.trim() == operator)
    }

    /// Returns whether the operator's preconditions read data.
    pub fn reads(&self) -> bool {
        self.data
    }

    /// Returns the shape the operator reaches, if any.
    pub fn reach(&self) -> Option<Reach> {
        self.reach
    }
}

/// A drawn migration of module `app`, and the schema it leaves when it applies.
#[derive(Clone, Debug)]
pub struct Migration {
    ops: Vec<Op>,
    after: Schema,
    keys: Keys,
    /// After each drawn step: the operators so far and the schema and keys they leave.
    steps: Vec<(usize, Schema, Keys)>,
}

impl Migration {
    /// Returns the migration file.
    pub fn text(&self) -> String {
        let mut out = "module app\n".to_owned();
        for op in &self.ops {
            out.push_str(&op.text);
            out.push('\n');
        }
        out
    }

    pub fn ops(&self) -> &[Op] {
        &self.ops
    }

    /// Returns each relation or constructor relation a write empties before the migration, as
    /// the schema before it names it; none the migration adds.
    pub fn emptied(&self) -> Vec<String> {
        let mut out = Vec::new();
        'ops: for (k, o) in self.ops.iter().enumerate() {
            let Some(target) = &o.empties else {
                continue;
            };
            let (mut rel, mut ctor) = match target.split_once('.') {
                Some((r, c)) => (r.to_owned(), Some(c.to_owned())),
                None => (target.clone(), None),
            };
            for before in self.ops[..k].iter().rev() {
                match &before.effect {
                    Effect::AddRel(r) if *r == rel => continue 'ops,
                    Effect::RenameRel(from, to) if *to == rel => rel.clone_from(from),
                    Effect::RenameCtor { rel: r, from, to }
                        if *r == rel && ctor.as_ref() == Some(to) =>
                    {
                        ctor = Some(from.clone());
                    }
                    _ => {}
                }
            }
            out.push(match ctor {
                Some(c) => format!("{rel}.{c}"),
                None => rel,
            });
        }
        out
    }

    /// Returns the range of operator `k`'s text in [`Migration::text`].
    pub fn span(&self, k: usize) -> std::ops::Range<usize> {
        let start = "module app\n".len()
            + self.ops[..k]
                .iter()
                .map(|o| o.text.len() + 1)
                .sum::<usize>();
        start..start + self.ops[k].text.len()
    }

    /// Returns the operator that breaks a stated precondition, if one does.
    pub fn broken(&self) -> Option<usize> {
        self.ops.iter().position(|o| o.broken)
    }

    /// Returns whether a `revert` of the migration, right after it, restores the data and
    /// schema before it, each operator drawn where no switch steers a known finding.
    pub fn exact(&self) -> bool {
        self.ops.iter().all(|o| {
            let transient = o.reach == Some(Reach::RefusedInverseOfTransient);
            !o.broken && (o.kind.exact() || transient)
        })
    }

    /// Returns the rows the migration leaves of `rows`, those of the case it was drawn over, or
    /// an error on an operator whose rows the model does not follow.
    pub fn rows(&self, rows: &Rows) -> Result<Rows, String> {
        let mut out = rows.clone();
        for o in &self.ops {
            o.effect.apply(&mut out)?;
        }
        Ok(out)
    }

    /// Returns the first operator that replaces a constructor a tag of `rows`, the rows before
    /// the migration, names, by no one constructor, with each refusal line the engine may give
    /// it; `None` when no operator does, or when an operator before it moves rows the model does
    /// not follow.
    pub fn tag_refusal(&self, rows: &Rows) -> Option<(&Op, BTreeSet<String>)> {
        let mut held = rows.clone();
        for o in &self.ops {
            let lines = o.effect.tagged(&held);
            if !lines.is_empty() {
                return Some((o, lines));
            }
            o.effect.apply(&mut held).ok()?;
        }
        None
    }

    /// Returns the relation and constructors the rows of constructors `ctors` of relation `rel`
    /// go to, by the model of each operator's effect. Fails on an operator the model does not
    /// follow.
    pub fn ctors(
        &self,
        rel: &str,
        ctors: &BTreeSet<String>,
    ) -> Result<(String, BTreeSet<String>), String> {
        let (mut rel, mut cs) = (rel.to_owned(), ctors.clone());
        for o in &self.ops {
            o.effect.ctors(&mut rel, &mut cs)?;
        }
        Ok((rel, cs))
    }

    /// Returns the name field `field` of the constructors `ctors` of relation `rel` takes, by
    /// the model of each operator's effect.
    ///
    /// Fail when an effect on the constructors fails, or when they give the field different
    /// names.
    pub fn field(
        &self,
        rel: &str,
        ctors: &BTreeSet<String>,
        field: &str,
    ) -> Result<String, String> {
        let mut names = BTreeSet::new();
        for c in ctors {
            let (mut rel, mut cs) = (rel.to_owned(), BTreeSet::from([c.clone()]));
            let mut name = field.to_owned();
            for o in &self.ops {
                o.effect.rename_field(&rel, &cs, &mut name);
                o.effect.ctors(&mut rel, &mut cs)?;
            }
            names.insert(name);
        }
        let mut names = names.into_iter();
        match (names.next(), names.next()) {
            (Some(name), None) => Ok(name),
            (None, _) => Err(format!("`{field}` of `{rel}` names no constructor")),
            (Some(a), Some(b)) => Err(format!(
                "`{field}` of `{rel}` takes the names `{a}` and `{b}` across {ctors:?}"
            )),
        }
    }

    /// Returns the name field `field` of the constructors `ctors` of relation `rel` takes after
    /// the migration on each constructor it reaches.
    pub fn fields(
        &self,
        rel: &str,
        ctors: &BTreeSet<String>,
        field: &str,
    ) -> Result<BTreeMap<String, String>, String> {
        let mut out = BTreeMap::new();
        for c in ctors {
            let mut at = vec![(
                rel.to_owned(),
                BTreeSet::from([c.clone()]),
                field.to_owned(),
            )];
            for o in &self.ops {
                let mut next = Vec::new();
                for (mut rel, mut cs, mut name) in at {
                    o.effect.rename_field(&rel, &cs, &mut name);
                    o.effect.ctors(&mut rel, &mut cs)?;
                    if cs.is_empty() {
                        next.push((rel, cs, name));
                        continue;
                    }
                    for p in cs {
                        next.push((rel.clone(), BTreeSet::from([p]), name.clone()));
                    }
                }
                at = next;
            }
            for (_, cs, name) in at {
                if cs.is_empty() {
                    out.insert(c.clone(), name.clone());
                }
                for p in cs {
                    out.insert(p, name.clone());
                }
            }
        }
        if out.is_empty() {
            return Err(format!("`{field}` of `{rel}` names no constructor"));
        }
        Ok(out)
    }

    /// Returns each split, merge, nest and unnest of the migration: its kind, relation, the
    /// constructors it replaces and those it makes.
    pub fn replaced(&self) -> Vec<(Kind, String, Vec<String>, Vec<String>)> {
        self.ops
            .iter()
            .filter_map(|o| match &o.effect {
                Effect::Split {
                    rel,
                    ctor,
                    products,
                    ..
                } => Some((o.kind, rel.clone(), vec![ctor.clone()], products.clone())),
                Effect::Merge {
                    rel,
                    sources,
                    product,
                    ..
                } => Some((o.kind, rel.clone(), sources.clone(), vec![product.clone()])),
                Effect::Unnest {
                    rel, ctor, ctors, ..
                } => Some((o.kind, rel.clone(), vec![ctor.clone()], ctors.clone())),
                _ => None,
            })
            .collect()
    }

    /// Returns the migration of its first `n` drawn steps, each an operator with those it
    /// needs before it, and the schema they leave; `None` past its steps.
    pub fn prefix(&self, n: usize) -> Option<Migration> {
        let (ops, after, keys) = self.steps.get(n.checked_sub(1)?)?.clone();
        Some(Migration {
            ops: self.ops[..ops].to_vec(),
            after,
            keys,
            steps: self.steps[..n].to_vec(),
        })
    }

    /// Returns the number of drawn steps.
    pub fn drawn_steps(&self) -> usize {
        self.steps.len()
    }

    /// Returns the migration of the operators before `k`, and the migration of those from `k`
    /// on, which leaves the schema this one does.
    pub fn split_at(&self, k: usize) -> (Migration, Migration) {
        let (mut a, mut b) = (self.clone(), self.clone());
        a.ops.truncate(k);
        b.ops.drain(..k);
        (a, b)
    }

    /// Returns the migration without operator `k`, the operators after it drawn on the model
    /// before `k` left unchanged.
    pub fn without(&self, k: usize) -> Migration {
        let mut m = self.clone();
        m.ops.remove(k);
        m
    }

    /// Returns the case of the schema the migration leaves, holding `n` statements drawn with
    /// `seed` after those of the case it was drawn over.
    pub fn applied(&self, seed: u64, n: usize) -> Case {
        resumed(&self.after, &self.keys, seed, n)
    }
}

/// Returns the case of `schema` holding `n` statements drawn with `seed` after those that
/// issued `keys`.
fn resumed(schema: &Schema, keys: &Keys, seed: u64, n: usize) -> Case {
    let mut rng = Rng::new(seed);
    let mut d = Draw::resume(schema, keys.clone());
    let stmts = (0..n).map(|_| d.stmt(&mut rng)).collect();
    Case {
        schema: schema.clone(),
        stmts,
        keys: d.keys(),
    }
}

/// What migrations add beyond relations and their clauses.
#[derive(Clone, Debug, Default)]
pub(super) struct Extra {
    /// The modules added, each empty.
    modules: Vec<String>,
    /// The sums of nullary constructors added: name and constructors.
    types: Vec<(String, Vec<String>)>,
    fns: Vec<Func>,
    /// Each computed field: its relation, name and the integer it gives.
    computed: Vec<(usize, String, u64)>,
    /// Each privilege [`ROLE`] holds: its privilege and relation.
    grants: Vec<(String, usize)>,
    /// The next number of a fresh name.
    next: u64,
}

#[derive(Clone, Debug)]
struct Func {
    name: String,
    /// The relation its set of keys reads; `None` for a function of an integer.
    reads: Option<usize>,
    stored: bool,
    aggregate: bool,
    /// What a function of an integer adds to it.
    adds: u64,
}

/// The aggregate every drawn `define aggregate` declares, under its name.
const AGGREGATE: &str = "(x: Text) -> Text =
  state { best: Text }
  init { best: \"\" }
  step (s, x) -> if scalar_len(x) > scalar_len(s.best) then { best: x } else s
  merge (a, b) -> if scalar_len(b.best) > scalar_len(a.best) then b else a
  finish s -> s.best";

/// The model a migration's operators are drawn over, one at a time.
#[derive(Clone)]
struct Model {
    s: Schema,
    keys: Keys,
    /// The version the migration produces.
    version: u64,
    /// Whether the declarations do not build before the operator drawn next.
    unbuilt: bool,
}

/// What a scope of constructors becomes when constructors are replaced: each old constructor's
/// new constructors, by its old index.
type Remap = Vec<Vec<usize>>;

fn remapped(map: &Remap, cs: &[usize]) -> Vec<usize> {
    let mut out: Vec<usize> = cs.iter().flat_map(|c| map[*c].iter().copied()).collect();
    out.sort_unstable();
    out.dedup();
    out
}

fn op(kind: Kind, text: String) -> Op {
    Op {
        kind,
        text,
        broken: false,
        unimaged: false,
        data: false,
        empties: None,
        effect: Effect::None,
        reach: None,
    }
}

impl Op {
    /// Returns the operator doing `effect` to the rows it finds.
    fn with(self, effect: Effect) -> Op {
        Op { effect, ..self }
    }

    /// Returns the operator, reaching `reach` when it is some.
    fn reaching(self, reach: Option<Reach>) -> Op {
        Op { reach, ..self }
    }
}

fn broken(kind: Kind, text: String) -> Op {
    Op {
        broken: true,
        ..op(kind, text)
    }
}

fn unimaged(kind: Kind, text: String) -> Op {
    Op {
        unimaged: true,
        ..broken(kind, text)
    }
}

fn data(kind: Kind, text: String) -> Op {
    Op {
        data: true,
        ..op(kind, text)
    }
}

fn emptying(kind: Kind, text: String, target: String) -> Op {
    Op {
        empties: Some(target),
        ..data(kind, text)
    }
}

/// Returns a literal of `ty` an `add field` fill or a `move field` fill gives.
fn fill(ty: &Ty) -> Option<String> {
    match ty {
        Ty::Int | Ty::Text | Ty::Bool | Ty::OptInt | Ty::OptRef(..) | Ty::Sum(_) | Ty::Tag(..) => {
            Some(literal(ty, false))
        }
        Ty::Ref(..) | Ty::TagOf(_) => None,
    }
}

/// Returns type `ty` as the checker shows it: a reference to every constructor of its relation
/// as `Key r`, to some of them by their names in declaration order.
fn shape_ty(s: &Schema, ty: &Ty) -> String {
    let key = |j: usize, scope: &Scope| {
        let r = &s.rels[j];
        let cs: Vec<usize> = match scope {
            Scope::All => (0..r.ctors.len()).collect(),
            Scope::Ctors(cs) => cs.clone(),
            Scope::Named => r.set.clone().unwrap_or_default(),
        };
        let names: Vec<&str> = cs.iter().map(|c| r.ctors[*c].name.as_str()).collect();
        match names.as_slice() {
            _ if cs.len() == r.ctors.len() => format!("Key {}", r.name),
            [one] => format!("Key {}.{one}", r.name),
            _ => format!("Key {}.({})", r.name, names.join(" | ")),
        }
    };
    match ty {
        Ty::Ref(j, scope) => key(*j, scope),
        Ty::OptRef(j, scope) => format!("Option ({})", key(*j, scope)),
        _ => s.ty_name(ty),
    }
}

impl Case {
    /// Returns the case holding, in place of its statements, a delete of every row of each of
    /// `targets`.
    pub fn emptying(&self, targets: &[String]) -> Case {
        Case {
            stmts: targets
                .iter()
                .map(|t| super::Stmt::write(format!("delete x in {t}")))
                .collect(),
            ..self.clone()
        }
    }

    /// Returns the case holding, in place of its statements, a query of each function and each
    /// computed field the migrations added.
    pub fn probes(&self) -> Case {
        let s = &self.schema;
        let mut stmts = Vec::new();
        for f in s.extra.fns.iter().filter(|f| !f.aggregate) {
            stmts.push(super::Stmt::query(match f.reads {
                Some(_) => format!("{}()", f.name),
                None => format!("{}(0)", f.name),
            }));
        }
        for (i, name, _) in &s.extra.computed {
            let r = &s.rels[*i].name;
            stmts.push(super::Stmt::query(format!("from x in {r} select x.{name}")));
        }
        Case {
            stmts,
            ..self.clone()
        }
    }

    /// Returns each constructor relation not enabled: its relation, constructor and state.
    pub fn closed(&self) -> Vec<(String, u32, adtb_lang::schema::Stage)> {
        let mut out = Vec::new();
        for r in &self.schema.rels {
            for (c, st) in r.states.iter().enumerate() {
                let st = match st {
                    Stage::Enabled => continue,
                    Stage::Declared => adtb_lang::schema::Stage::Declared,
                    Stage::Deprecated(_) => adtb_lang::schema::Stage::Deprecated,
                };
                out.push((r.name.clone(), c as u32, st));
            }
        }
        out
    }

    /// Returns the case of this schema holding `n` statements drawn with `seed` after its own.
    pub fn resumed(&self, seed: u64, n: usize) -> Case {
        resumed(&self.schema, &self.keys, seed, n)
    }

    /// Returns a migration of 1 to 4 drawn operators over the case's schema with `seed`,
    /// producing version `version`, its first of `kind` and broken when `broke`, or `None` when
    /// the schema holds nothing an operator of `kind` applies to. Each operator after it is
    /// drawn valid; an operator another needs comes before it.
    pub fn migration(&self, seed: u64, kind: Kind, broke: bool, version: u64) -> Option<Migration> {
        let mut rng = Rng::new(seed);
        let mut m = Model {
            s: self.schema.clone(),
            keys: self.keys.clone(),
            version,
            unbuilt: false,
        };
        let mut ops = m.draw(&mut rng, kind, broke)?;
        let mut steps = vec![(ops.len(), m.s.clone(), m.keys.clone())];
        let more = rng.below(4);
        for _ in 0..more {
            for _ in 0..8 {
                let kind = *rng.pick(if broke { &ADDING } else { &Kind::ALL[..] });
                let mut next = m.clone();
                if let Some(drawn) = next.draw(&mut rng, kind, false) {
                    if ops.len() + drawn.len() <= 6 {
                        m = next;
                        ops.extend(drawn);
                        steps.push((ops.len(), m.s.clone(), m.keys.clone()));
                    }
                    break;
                }
            }
        }
        Some(Migration {
            ops,
            after: m.s,
            keys: m.keys,
            steps,
        })
    }

    /// Returns each line of the shape migrations change: each relation with its key, header,
    /// constructors with their states and fields, uniques, indexes and named set, and the
    /// names of the checks, constraints and functions of module `app` and of the modules
    /// added.
    pub fn shape(&self) -> Vec<String> {
        let s = &self.schema;
        let mut out = Vec::new();
        for r in &s.rels {
            let key: Vec<&str> = match r.keys {
                0 => vec!["id"],
                k => r.header[..k].iter().map(|f| f.name.as_str()).collect(),
            };
            out.push(format!(
                "relation {} : {} key ({})",
                r.name,
                r.ty,
                key.join(", ")
            ));
            let field = |f: &Field| {
                let action = match f.action {
                    _ if f.ty.target().is_none() => String::new(),
                    Action::Unstated | Action::Restrict => " on delete restrict".into(),
                    Action::Cascade => " on delete cascade".into(),
                    Action::Clear => " on delete clear".into(),
                    Action::Retag => {
                        let d = r.retag.expect("a retag action names its constructor");
                        format!(" on delete retag {}", r.ctors[d].name)
                    }
                };
                format!("{}: {}{action}", f.name, shape_ty(s, &f.ty))
            };
            let mut header: Vec<String> = Vec::new();
            if r.keys == 0 {
                header.push(format!("id: Key {} on delete restrict", r.name));
            }
            header.extend(r.header.iter().map(field));
            if r.sum {
                out.push(format!("header {}: {}", r.name, header.join(", ")));
            }
            for (c, ctor) in r.ctors.iter().enumerate() {
                let mut fs: Vec<String> = if r.sum { Vec::new() } else { header.clone() };
                fs.extend(ctor.fields.iter().map(field));
                let stage = match r.states[c] {
                    Stage::Enabled => "Enabled",
                    Stage::Declared => "Declared",
                    Stage::Deprecated(_) => "Deprecated",
                };
                out.push(format!(
                    "constructor {}.{} {stage}: {}",
                    r.name,
                    ctor.name,
                    fs.join(", ")
                ));
            }
            let names = |cs: &Option<Vec<usize>>| -> String {
                match cs {
                    Some(cs) if r.sum => {
                        let mut ns: Vec<&str> =
                            cs.iter().map(|c| r.ctors[*c].name.as_str()).collect();
                        ns.sort_unstable();
                        format!(" in {}", ns.join(" | "))
                    }
                    _ => String::new(),
                }
            };
            for u in &r.uniques {
                out.push(format!(
                    "unique {} ({}){}",
                    r.name,
                    u.field,
                    names(&u.ctors)
                ));
            }
            for x in &r.indexes {
                out.push(format!("index {} ({}){}", r.name, x.field, names(&x.ctors)));
            }
            if r.set.is_some() {
                out.push(format!("set {} of {}{}", r.set_name, r.name, names(&r.set)));
            }
            for k in &r.checks {
                out.push(format!("check {}", k.name));
            }
        }
        for k in &s.constraints {
            out.push(format!("constraint {}", k.name));
        }
        out.extend(s.extra.shape(s));
        out.sort();
        out
    }
}

impl Extra {
    /// Returns the lines of the shape of what the migrations added, and of the types, defaults
    /// and computed fields of `s`, as [`Case::shape`] writes them.
    fn shape(&self, s: &Schema) -> Vec<String> {
        let mut out = Vec::new();
        for sum in super::SUMS {
            let (name, ctors) = sum
                .trim_start_matches("type ")
                .split_once(" = ")
                .expect("a sum declaration");
            let names: Vec<&str> = ctors
                .split(" | ")
                .map(|c| c.split_whitespace().next().unwrap_or(c))
                .collect();
            out.push(format!("type {name} = {}", names.join(" | ")));
        }
        for r in &s.rels {
            let names: Vec<&str> = r.ctors.iter().map(|c| c.name.as_str()).collect();
            out.push(format!("type {} = {}", r.ty, names.join(" | ")));
            for (c, f, v) in &r.defaults {
                let v = v.trim_matches(['(', ')']);
                if r.sum {
                    out.push(format!("default {}.{}.{f} = {v}", r.name, r.ctors[*c].name));
                } else {
                    out.push(format!("default {}.{f} = {v}", r.name));
                }
            }
        }
        for (t, cs) in &self.types {
            out.push(format!("type {t} = {}", cs.join(" | ")));
        }
        for (i, name, n) in &self.computed {
            out.push(format!("computed {}.{name}: I64 = {n}", s.rels[*i].name));
        }
        for f in &self.fns {
            match (f.aggregate, f.reads) {
                (true, _) => out.push(format!("aggregate {}", f.name)),
                (false, Some(_)) => out.push(format!("fn {} = query", f.name)),
                (false, None) => out.push(format!("fn {} = x + {}", f.name, f.adds)),
            }
            if f.stored {
                out.push(format!("stored {}", f.name));
            }
        }
        for (p, i) in &self.grants {
            out.push(format!("grant {p} app::{}", s.rels[*i].name));
        }
        for m in &self.modules {
            out.push(format!("module {m}"));
        }
        out
    }

    /// Returns the declarations of module `app` the migrations added: types, functions,
    /// aggregates and stored views.
    fn decls(&self, s: &Schema) -> Vec<String> {
        let mut out = Vec::new();
        for (t, cs) in &self.types {
            out.push(format!("type {t} = {}", cs.join(" | ")));
        }
        for f in &self.fns {
            if f.aggregate {
                out.push(format!("aggregate {} {AGGREGATE}", f.name));
                continue;
            }
            out.push(fn_decl(s, "fn", &f.name, f.reads, f.adds));
            if f.stored {
                out.push(format!("store {}\n  key a", f.name));
            }
        }
        out
    }

    /// Returns the clauses of relation `i` the migrations added: its computed fields.
    fn clauses(&self, i: usize) -> Vec<String> {
        self.computed
            .iter()
            .filter(|(r, _, _)| *r == i)
            .map(|(_, name, n)| format!("computed {name}: I64 = {n}"))
            .collect()
    }
}

/// Returns the declaration of function `name` after `verb`: of a set of keys of relation
/// `reads`, else of an integer `x` plus `n`.
fn fn_decl(s: &Schema, verb: &str, name: &str, reads: Option<usize>, n: u64) -> String {
    match reads {
        Some(j) => format!(
            "{verb} {name} () -> Set ({{ a: Key {0} }}) = from x in {0} select {{ a: key x }}",
            s.rels[j].name
        ),
        None => format!("{verb} {name} (x: I64) -> I64 = x + {n}"),
    }
}

impl Schema {
    /// Returns the declarations the migrations added, and the clauses they added to relation
    /// `i` when `i` is given.
    pub(super) fn added(&self, i: Option<usize>) -> Vec<String> {
        match i {
            Some(i) => self.extra.clauses(i),
            None => self.extra.decls(self),
        }
    }
}
