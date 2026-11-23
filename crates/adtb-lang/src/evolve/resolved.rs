use super::*;
use crate::canon::canonical_term;
use crate::ids::Space;

#[path = "functions.rs"]
mod functions;
#[cfg(test)]
#[path = "../resolved_tests.rs"]
mod tests;

/// An operator of a logged migration, naming every object it touches by its catalog id, with each
/// operand written in the language in canonical token form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolved {
    AddModule {
        module: String,
    },
    AddType {
        sum: u64,
        decl: String,
    },
    AddRelation {
        relation: u64,
        decl: String,
    },
    AddSet {
        set: u64,
        relation: u64,
        name: String,
    },
    AddUnique {
        index: u64,
        relation: u64,
        fields: String,
    },
    AddIndex {
        index: u64,
        relation: u64,
        interval: bool,
        fields: String,
    },
    DefineFn {
        function: u64,
        decl: String,
    },
    DefineAggregate {
        function: u64,
        decl: String,
    },
    AddConstraint {
        constraint: u64,
        body: String,
    },
    AddCheck {
        constraint: u64,
        body: String,
    },
    DropModule {
        module: String,
    },
    DropType {
        sum: u64,
    },
    DropRelation {
        relation: u64,
    },
    DropSet {
        set: u64,
    },
    AddConstructor {
        ctor: u64,
        sum: u64,
        decl: String,
        place: Place,
    },
    RemoveConstructor {
        ctor: u64,
    },
    Enable {
        target: Target,
    },
    Disable {
        target: Target,
    },
    Deprecate {
        target: Target,
    },
    RenameConstructor {
        ctor: u64,
        name: String,
    },
    RenameType {
        sum: u64,
        name: String,
    },
    RenameRelation {
        relation: u64,
        name: String,
    },
    RenameSet {
        set: u64,
        name: String,
    },
    RenameField {
        field: u64,
        name: String,
    },
    Split {
        ctor: u64,
        decl: String,
        routing: String,
    },
    Merge {
        ctor: u64,
        decl: String,
        routing: Option<String>,
    },
    Nest {
        ctor: u64,
        decl: String,
    },
    Unnest {
        ctor: u64,
    },
    AddField {
        field: u64,
        decl: String,
        fill: String,
    },
    DropField {
        field: u64,
    },
    AlterField {
        field: u64,
        action: String,
    },
    MoveField {
        field: u64,
        header: bool,
        fill: Option<String>,
    },
    DropUnique {
        index: u64,
    },
    DropIndex {
        index: u64,
    },
    Rekey {
        relation: u64,
        key: u64,
        fields: String,
    },
    RedefineFn {
        function: u64,
        decl: String,
    },
    DropFn {
        function: u64,
    },
    DropAggregate {
        function: u64,
    },
    DropConstraint {
        constraint: u64,
    },
    DropCheck {
        constraint: u64,
    },
    Revert {
        migration: u64,
    },
    /// One for each field the default fills, the first of them named.
    AddDefault {
        default: u64,
        relation: u64,
        field: u64,
        value: String,
    },
    DropDefault {
        default: u64,
    },
    Store {
        view: u64,
        function: u64,
        decl: String,
    },
    Unstore {
        view: u64,
    },
    AddComputed {
        computed: u64,
        relation: u64,
        name: String,
        decl: String,
    },
    RedefineComputed {
        computed: u64,
        decl: String,
    },
    DropComputed {
        computed: u64,
    },
    /// The first privilege of the statement; a `Privilege` clause holds each later one.
    Grant {
        privilege: String,
        target: String,
        grantee: String,
        grant_option: bool,
    },
    /// The first privilege of the statement; a `Privilege` clause holds each later one.
    Revoke {
        privilege: String,
        target: String,
        grantee: String,
    },
    /// `package` is the package's contents.
    Install {
        module: String,
        version: String,
        package: String,
    },
    Uninstall {
        module: String,
    },
    /// `data` is the release's data.
    AdoptTzdb {
        release: String,
        data: String,
    },
    Script {
        write: String,
    },
}

/// The constructor relation, or the constructor of a type, a write state operator names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    InRelation { relation: u64, ctor: u64 },
    OfType { ctor: u64 },
}

/// Where an added constructor goes in its type's declaration order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    Last,
    Before { ctor: u64 },
    After { ctor: u64 },
}

/// A clause of a resolved operator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Clause {
    /// A constructor a named set lists, a merge or nest replaces, a moved field goes to, or a
    /// unique or index written with `in` names.
    Member { ctor: u64 },
    /// A named set an added constructor joins.
    Joins { set: u64 },
    /// A named set the `in` scope of a unique or index names.
    Scope { set: u64 },
    /// An `arm` clause: what its arm ends the match of, and the arm.
    Arm { target: ArmTarget, arm: String },
    /// A `unique` or `index` clause of a split: the index it adds, its kind and its fields.
    Replacement {
        index: u64,
        kind: IndexKind,
        fields: String,
    },
    /// A field a rekey carries.
    Carry { field: u64 },
    /// A field a revert fills, with its fill.
    Fill { field: u64, fill: String },
    /// A privilege of a grant or revoke after its first.
    Privilege { privilege: String, target: String },
}

/// The schema object an `arm` clause's arm belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArmTarget {
    OfFn { function: u64 },
    OfConstraint { constraint: u64 },
    OfComputed { computed: u64 },
}

/// The kind of an index a split's replacement clause adds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexKind {
    Unique,
    Plain,
    Interval,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedOp {
    pub op: Resolved,
    /// In clause order.
    pub clauses: Vec<Clause>,
}

/// An object an operator names: a catalog object, by the space of its id and the id, or a module.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Reached {
    Object(Space, u64),
    Module(String),
}

impl ResolvedOp {
    /// Returns every object the operator names as an operand or in a clause.
    pub(super) fn operands(&self) -> Vec<Reached> {
        use Reached::{Module, Object};
        let shared = |id: &u64| Object(Space::Shared, *id);
        let mut out = match &self.op {
            Resolved::AddModule { module }
            | Resolved::DropModule { module }
            | Resolved::Install { module, .. }
            | Resolved::Uninstall { module } => vec![Module(module.clone())],
            Resolved::AddType { sum: id, .. }
            | Resolved::DropType { sum: id }
            | Resolved::RenameType { sum: id, .. }
            | Resolved::AddRelation { relation: id, .. }
            | Resolved::DropRelation { relation: id }
            | Resolved::RenameRelation { relation: id, .. }
            | Resolved::RemoveConstructor { ctor: id }
            | Resolved::RenameConstructor { ctor: id, .. }
            | Resolved::Split { ctor: id, .. }
            | Resolved::Merge { ctor: id, .. }
            | Resolved::Nest { ctor: id, .. }
            | Resolved::Unnest { ctor: id }
            | Resolved::Unstore { view: id } => vec![shared(id)],
            Resolved::AddSet { set, relation, .. } => {
                vec![Object(Space::Set, *set), shared(relation)]
            }
            Resolved::DropSet { set } | Resolved::RenameSet { set, .. } => {
                vec![Object(Space::Set, *set)]
            }
            Resolved::AddUnique {
                index, relation, ..
            }
            | Resolved::AddIndex {
                index, relation, ..
            }
            | Resolved::Rekey {
                relation,
                key: index,
                ..
            } => vec![Object(Space::Index, *index), shared(relation)],
            Resolved::DropUnique { index } | Resolved::DropIndex { index } => {
                vec![Object(Space::Index, *index)]
            }
            Resolved::DefineFn { function, .. }
            | Resolved::DefineAggregate { function, .. }
            | Resolved::RedefineFn { function, .. }
            | Resolved::DropFn { function }
            | Resolved::DropAggregate { function } => vec![Object(Space::Function, *function)],
            Resolved::AddConstraint { constraint, .. }
            | Resolved::AddCheck { constraint, .. }
            | Resolved::DropConstraint { constraint }
            | Resolved::DropCheck { constraint } => vec![Object(Space::Constraint, *constraint)],
            Resolved::AddConstructor {
                ctor, sum, place, ..
            } => {
                let mut out = vec![shared(ctor), shared(sum)];
                if let Place::Before { ctor } | Place::After { ctor } = place {
                    out.push(shared(ctor));
                }
                out
            }
            Resolved::Enable { target }
            | Resolved::Disable { target }
            | Resolved::Deprecate { target } => match target {
                Target::InRelation { relation, ctor } => vec![shared(relation), shared(ctor)],
                Target::OfType { ctor } => vec![shared(ctor)],
            },
            Resolved::AddField { field, .. }
            | Resolved::DropField { field }
            | Resolved::AlterField { field, .. }
            | Resolved::RenameField { field, .. }
            | Resolved::MoveField { field, .. } => vec![Object(Space::Field, *field)],
            Resolved::AddDefault {
                default,
                relation,
                field,
                ..
            } => vec![
                Object(Space::Default, *default),
                shared(relation),
                Object(Space::Field, *field),
            ],
            Resolved::DropDefault { default } => vec![Object(Space::Default, *default)],
            Resolved::Store { view, function, .. } => {
                vec![shared(view), Object(Space::Function, *function)]
            }
            Resolved::AddComputed {
                computed, relation, ..
            } => vec![Object(Space::Computed, *computed), shared(relation)],
            Resolved::RedefineComputed { computed, .. } | Resolved::DropComputed { computed } => {
                vec![Object(Space::Computed, *computed)]
            }
            Resolved::Revert { .. }
            | Resolved::Grant { .. }
            | Resolved::Revoke { .. }
            | Resolved::AdoptTzdb { .. }
            | Resolved::Script { .. } => Vec::new(),
        };
        for c in &self.clauses {
            match c {
                Clause::Member { ctor } => out.push(shared(ctor)),
                Clause::Joins { set } | Clause::Scope { set } => {
                    out.push(Object(Space::Set, *set));
                }
                Clause::Arm { target, .. } => out.push(match target {
                    ArmTarget::OfFn { function } => Object(Space::Function, *function),
                    ArmTarget::OfConstraint { constraint } => {
                        Object(Space::Constraint, *constraint)
                    }
                    ArmTarget::OfComputed { computed } => Object(Space::Computed, *computed),
                }),
                Clause::Replacement { index, .. } => out.push(Object(Space::Index, *index)),
                Clause::Carry { field } | Clause::Fill { field, .. } => {
                    out.push(Object(Space::Field, *field));
                }
                Clause::Privilege { .. } => {}
            }
        }
        out
    }
}

/// A function or aggregate of a source, as the catalog records it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Function {
    pub id: u64,
    pub module: String,
    pub name: String,
    /// The type parameters, parameters and result type, as written.
    pub signature: String,
    /// Whether every function it calls, transitively, is deterministic, a host function only when
    /// declared so.
    pub deterministic: bool,
    pub code: Code,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Code {
    /// `body` as written, and `canonical` its canonical token form.
    Language {
        body: String,
        canonical: String,
    },
    /// Everything after the `=` of the declaration, as written and in canonical token form.
    Aggregate {
        body: String,
        canonical: String,
    },
    Wasm {
        module: String,
        export: String,
    },
    Host {
        symbol: String,
        version: String,
    },
}

impl Source {
    /// Returns the operators of migration `number` of the log in ordinal order, each resolved as
    /// the migration applies again to the source before it.
    ///
    /// Fail with the operators whose data the source lacks, at their spans, or with the operands
    /// whose names do not resolve, or when the log holds no migration `number`.
    pub fn resolved(&self, number: u64) -> Replaying<Vec<ResolvedOp>> {
        self.resolving(number).map(|(ops, _)| ops)
    }

    /// Returns every object the operators of migration `number` name: as an operand, in a clause,
    /// or in a text they copy into the schema. A `revert k` names what migration `k` named and
    /// created.
    ///
    /// Fail as [`Source::resolved`] does.
    pub(super) fn named_by(&self, number: u64) -> Replaying<BTreeSet<Reached>> {
        let (ops, names) = self.resolving(number)?;
        let mut out: BTreeSet<Reached> = ops
            .iter()
            .flat_map(ResolvedOp::operands)
            .chain(names.ids().map(|(space, id)| Reached::Object(space, id)))
            .collect();
        for op in &ops {
            if let Resolved::Revert { migration } = op.op {
                out.extend(self.named_by(migration)?);
                out.extend(self.created(self.at(migration)?));
            }
        }
        Ok(out)
    }

    /// Returns the position of migration `number` in the log.
    ///
    /// Fail when the log holds no migration `number`.
    fn at(&self, number: u64) -> Result<usize, Vec<Diag>> {
        self.log
            .iter()
            .position(|l| l.number == number)
            .ok_or_else(|| {
                vec![Diag::new(
                    Span::default(),
                    format!("migration {number} is not in the log"),
                )]
            })
    }

    /// Returns the position in the log of the migration whose number `migration` spells.
    pub(super) fn logged_at(&self, migration: &str) -> Option<usize> {
        self.at(migration.parse().ok()?).ok()
    }

    /// Resolve migration `number` as [`Source::resolved`] does. Returns its operators resolved and
    /// the names of its text.
    fn resolving(&self, number: u64) -> Replaying<(Vec<ResolvedOp>, Names)> {
        let logged = &self.log[self.at(number)?];
        let unavailable = self.unavailable(&logged.operators);
        if !unavailable.is_empty() {
            return Err(Failed::Refused(unavailable));
        }
        let (_, names, got) = model::replay(logged)?;
        Ok((got.resolved(), names?))
    }
}
