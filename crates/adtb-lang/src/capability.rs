//! What each kind of root reads and what it may reach, and the judge of a root over its call closure.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;

use crate::ast::FnBody;
use crate::check::{AsSchema, Compiled, ProgramBody};
use crate::kernel::{Agg, Kind, Prim, Terms, Tm};
use crate::norm::Root;
use crate::schema::Schema;
use crate::types::FnId;

/// A body the checker admits, by what it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RootKind {
    Constraint,
    Check,
    Domain,
    DeleteAction,
    Default,
    Computed,
    Index,
    StoredView,
    Function,
    Aggregate,
    Query,
    HidingQuery,
    Adhoc,
    Script,
    AtBody,
    Fill,
    Routing,
}

/// The state a root reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reads {
    /// The snapshot of the statement.
    Snapshot,
    /// The state after a statement, before its checks pass.
    Unchecked,
    /// The row being written, before checks run.
    Written,
    /// The row it is read on, in that row's state.
    Row,
    /// The state the root that calls it reads.
    Inherited,
    /// The state the operators of a migration before it leave.
    Script,
    /// A past state.
    Past,
    /// The snapshot, through a pin that hides constructors added after it.
    HidingPin,
    /// The value it constrains or the literals it holds.
    Value,
}

/// Something a body may reach, directly or through the functions and aggregates it calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Reach {
    Volatile,
    Caller,
    Nondeterministic,
    File,
    Transition,
    Write,
    Deref,
    UserAggregate,
    Relation,
    Generate,
}

/// What a kind of root reads and the reaches it is refused.
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    reads: Reads,
    forbids: &'static [Reach],
}

impl Entry {
    /// Returns whether every deref in the root is partial whatever its key's provenance.
    pub fn derefs_partial(&self) -> bool {
        matches!(
            self.reads,
            Reads::Unchecked | Reads::Script | Reads::Past | Reads::HidingPin
        )
    }

    /// Returns whether the root reads through a pin hiding constructors added after it.
    pub fn hiding(&self) -> bool {
        self.reads == Reads::HidingPin
    }
}

const PURE: &[Reach] = &[
    Reach::Volatile,
    Reach::Caller,
    Reach::Nondeterministic,
    Reach::File,
    Reach::Transition,
    Reach::Write,
];

/// Returns the entry of `kind`.
pub fn entry(kind: RootKind) -> Entry {
    use Reach::*;
    let (reads, forbids): (Reads, &'static [Reach]) = match kind {
        RootKind::Constraint => (
            Reads::Unchecked,
            &[Volatile, Caller, Nondeterministic, File, Write],
        ),
        RootKind::Check => (Reads::Unchecked, PURE),
        RootKind::DeleteAction => (Reads::Value, PURE),
        RootKind::Domain | RootKind::Default => (
            Reads::Value,
            &[
                Volatile,
                Caller,
                Nondeterministic,
                File,
                Transition,
                Write,
                Deref,
                Relation,
            ],
        ),
        RootKind::Computed => (Reads::Row, PURE),
        RootKind::Index => (
            Reads::Written,
            &[
                Volatile,
                Caller,
                Nondeterministic,
                File,
                Transition,
                Write,
                Deref,
            ],
        ),
        RootKind::StoredView => (Reads::Snapshot, PURE),
        RootKind::Function | RootKind::Aggregate => (Reads::Inherited, &[File, Transition, Write]),
        RootKind::Query | RootKind::Adhoc => (Reads::Snapshot, &[Transition]),
        RootKind::HidingQuery => (Reads::HidingPin, &[Transition]),
        RootKind::Script => (Reads::Script, &[Caller, Transition]),
        RootKind::AtBody => (Reads::Past, &[Write]),
        RootKind::Fill | RootKind::Routing => (
            Reads::Row,
            &[
                Volatile,
                Caller,
                Nondeterministic,
                File,
                Transition,
                Write,
                Deref,
                Relation,
                Generate,
            ],
        ),
    };
    Entry { reads, forbids }
}

impl fmt::Display for RootKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            RootKind::Constraint => "a constraint",
            RootKind::Check => "a row check",
            RootKind::Domain => "a domain predicate",
            RootKind::DeleteAction => "a delete action",
            RootKind::Default => "a default",
            RootKind::Computed => "a computed field",
            RootKind::Index => "an index expression",
            RootKind::StoredView => "a stored view",
            RootKind::Function => "a function",
            RootKind::Aggregate => "a user aggregate",
            RootKind::Query => "a registered query or transaction",
            RootKind::HidingQuery => "a registered query or transaction hiding new constructors",
            RootKind::Adhoc => "ad hoc text",
            RootKind::Script => "a migration script",
            RootKind::AtBody => "an `at` body",
            RootKind::Fill => "a fill",
            RootKind::Routing => "a routing",
        })
    }
}

impl fmt::Display for Reach {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Reach::Volatile => "read a volatile relation",
            Reach::Caller => "read `caller`",
            Reach::Nondeterministic => "call a nondeterministic function",
            Reach::File => "read a file",
            Reach::Transition => "read the change of a statement",
            Reach::Write => "write",
            Reach::Deref => "dereference a key",
            Reach::UserAggregate => "call a user aggregate",
            Reach::Relation => "read a relation",
            Reach::Generate => "generate a key",
        })
    }
}

/// A root refused for reaching what its entry forbids.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refusal {
    root: RootKind,
    reach: Reach,
}

impl Refusal {
    pub fn new(root: RootKind, reach: Reach) -> Self {
        Refusal { root, reach }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} must not {}", self.root, self.reach)
    }
}

/// Refuse `root` when one of `reaches` is forbidden by its entry, naming the first.
pub fn admit(root: RootKind, reaches: &BTreeSet<Reach>) -> Result<(), Refusal> {
    match reaches.iter().find(|r| entry(root).forbids.contains(r)) {
        Some(&reach) => Err(Refusal { root, reach }),
        None => Ok(()),
    }
}

/// Returns the kind of `root`, an ad hoc item or command taken as ad hoc text.
pub fn kind_of(schema: &Schema, root: Root) -> RootKind {
    match root {
        Root::Function(f) if schema.function(f).stored.is_some() => RootKind::StoredView,
        Root::Function(_) => RootKind::Function,
        Root::Constraint(_) => RootKind::Constraint,
        Root::Check(_) => RootKind::Check,
        Root::Domain(_) => RootKind::Domain,
        Root::Retag(_) => RootKind::DeleteAction,
        Root::Default(..) => RootKind::Default,
        Root::Computed(..) => RootKind::Computed,
        Root::IndexField(..) => RootKind::Index,
        Root::Aggregate(_) => RootKind::Aggregate,
        Root::Item(_) | Root::Command(_) => RootKind::Adhoc,
        Root::Query => RootKind::Query,
    }
}

/// A function or a user aggregate of a checked program.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Callee {
    Function(FnId),
    Aggregate(u32),
}

/// What a body reaches itself and the functions and aggregates it calls.
#[derive(Debug, Default)]
struct Scan {
    reaches: BTreeSet<Reach>,
    calls: BTreeSet<Callee>,
}

/// The reaches of every function and aggregate of a checked program, each through everything it
/// calls, cycles included.
#[derive(Debug)]
pub struct Judge<'a> {
    schema: &'a Schema,
    closed: HashMap<Callee, BTreeSet<Reach>>,
}

impl<'a> Judge<'a> {
    pub fn new(c: &'a Compiled<impl AsSchema>) -> Self {
        let (schema, program) = (c.schema(), c.program());
        let mut scans: HashMap<Callee, Scan> = HashMap::new();
        for (i, def) in schema.functions.iter().enumerate() {
            let scan = match &def.body {
                FnBody::Host {
                    deterministic: false,
                    ..
                } => Scan {
                    reaches: BTreeSet::from([Reach::Nondeterministic]),
                    calls: BTreeSet::new(),
                },
                _ => Scan::default(),
            };
            scans.insert(Callee::Function(FnId(i as u32)), scan);
        }
        for (i, _) in schema.aggregates.iter().enumerate() {
            scans.insert(Callee::Aggregate(i as u32), Scan::default());
        }
        for body in program.roots() {
            let callee = match body {
                ProgramBody::Function(f, _) => Callee::Function(f),
                ProgramBody::Aggregate(i, _) => Callee::Aggregate(i as u32),
                _ => continue,
            };
            let terms: Vec<Tm> = body.terms().into_iter().map(|(t, _)| t).collect();
            scans.insert(callee, scan(schema, &program.terms, &terms));
        }
        let mut closed: HashMap<Callee, BTreeSet<Reach>> =
            scans.iter().map(|(c, s)| (*c, s.reaches.clone())).collect();
        let mut changed = true;
        while changed {
            changed = false;
            for (c, s) in &scans {
                let mut r = closed[c].clone();
                for d in &s.calls {
                    r.extend(closed.get(d).into_iter().flatten().copied());
                }
                if r.len() != closed[c].len() {
                    closed.insert(*c, r);
                    changed = true;
                }
            }
        }
        Judge { schema, closed }
    }

    /// Refuse `t` of `terms`, a body of kind `root`, when it reaches what the entry of `root`
    /// forbids, through every function and aggregate it calls, dead terms included.
    pub fn judge(&self, root: RootKind, terms: &Terms, t: Tm) -> Result<(), Refusal> {
        admit(root, &self.reaches(terms, t))
    }

    /// Returns everything `t` of `terms` reaches.
    pub fn reaches(&self, terms: &Terms, t: Tm) -> BTreeSet<Reach> {
        let Scan { mut reaches, calls } = scan(self.schema, terms, &[t]);
        for c in calls {
            reaches.extend(self.closed.get(&c).into_iter().flatten().copied());
        }
        reaches
    }
}

/// Returns what the terms under `roots` reach themselves and what they call.
fn scan(schema: &Schema, terms: &Terms, roots: &[Tm]) -> Scan {
    let mut out = Scan::default();
    let mut seen = HashSet::new();
    let mut todo = roots.to_vec();
    while let Some(t) = todo.pop() {
        if !seen.insert(t) {
            continue;
        }
        let reach = match terms.kind(t) {
            Kind::Rel(r, _) => {
                if schema.relation(*r).volatile {
                    out.reaches.insert(Reach::Volatile);
                }
                Some(Reach::Relation)
            }
            Kind::Transition(_, r, _) => {
                if schema.relation(*r).volatile {
                    out.reaches.insert(Reach::Volatile);
                }
                out.reaches.insert(Reach::Relation);
                Some(Reach::Transition)
            }
            Kind::Caller => Some(Reach::Caller),
            Kind::Deref { .. } => Some(Reach::Deref),
            Kind::Fresh(..) => Some(Reach::Generate),
            Kind::Prim(Prim::ReadFile { .. }, _) => Some(Reach::File),
            Kind::Call { func, .. } => {
                out.calls.insert(Callee::Function(*func));
                None
            }
            Kind::Agg {
                op: Agg::User(i), ..
            } => {
                out.calls.insert(Callee::Aggregate(*i));
                Some(Reach::UserAggregate)
            }
            k if k.is_write() => Some(Reach::Write),
            _ => None,
        };
        out.reaches.extend(reach);
        todo.extend(terms.children(t));
    }
    out
}
