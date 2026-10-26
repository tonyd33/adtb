mod catalog;
mod lower;
mod terms;

#[cfg(test)]
mod tests;

pub use catalog::{Catalog, HeapId, IndexDesc, IndexId, RelationDesc};
pub use lower::{plan, plan_bounded};
pub(crate) use terms::{Facts, with_children};

use std::collections::BTreeSet;

use adtb_lang::kernel::{Agg, Terms, Tm, Var};
use adtb_lang::types::{CtorSet, RelId, Ty};

/// A physical plan: the computation of one value from the database and the parameters.
#[derive(Clone, Debug)]
pub struct Plan {
    /// The arena of every scalar expression of the plan.
    pub terms: Terms,
    pub params: Vec<Var>,
    pub root: Eval,
    pub reads: Reads,
}

/// The heaps and indexes a plan may read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reads {
    pub heaps: BTreeSet<HeapId>,
    pub indexes: BTreeSet<IndexId>,
}

/// The computation of one value in an environment of bound variables.
#[derive(Clone, Debug)]
pub enum Eval {
    /// A scalar expression of the plan's arena.
    Expr(Tm),
    Let(Var, Box<Eval>, Box<Eval>),
    If(Tm, Box<Eval>, Box<Eval>),
    /// One arm per constructor the scrutinee may hold, each binding its variable to the scrutinee's value.
    Case(Tm, Vec<(u32, Var, Eval)>),
    /// Returns `Ok { value }` of the inner value, or `Err { error }` of the fault it raises.
    Try(Box<Eval>),
    Lookup(Lookup),
    Query(Box<Query>),
    Fix(Box<Fix>),
}

/// The row whose key index entry has the key values, fetched from the heap of its constructor.
#[derive(Clone, Debug)]
pub struct Lookup {
    pub index: IndexId,
    pub key: Vec<Tm>,
    pub routes: Vec<(u32, HeapId)>,
}

/// Pipelines that push values into one sink, and the value the sink forms from them.
#[derive(Clone, Debug)]
pub struct Query {
    pub builds: Vec<Build>,
    pub spine: Spine,
    pub sink: Sink,
}

/// The decisions taken once per evaluation of a query, before its pipelines run, that choose which pipelines run.
#[derive(Clone, Debug)]
pub enum Spine {
    Pipeline(Pipeline),
    Union(Vec<Spine>),
    If(Tm, Box<Spine>, Box<Spine>),
    Case(Tm, Vec<(u32, Var, Spine)>),
    Let(Var, Eval, Box<Spine>),
}

/// A hash table over the distinct values its pipelines emit, keyed by `key` over `var`.
#[derive(Clone, Debug)]
pub struct Build {
    pub var: Var,
    pub key: Vec<Tm>,
    pub pipelines: Vec<Pipeline>,
}

/// An index into the builds of the query a step belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BuildId(pub u32);

#[derive(Clone, Debug)]
pub struct Pipeline {
    pub source: Source,
    pub chain: Chain,
}

#[derive(Clone, Debug)]
pub enum Source {
    /// One row that binds nothing.
    Unit,
    /// One row per row of the heap, binding `var` to it.
    Scan { heap: HeapId, ctor: u32, var: Var },
}

/// Steps applied to each row in turn, then the end that consumes it.
#[derive(Clone, Debug)]
pub struct Chain {
    pub steps: Vec<Step>,
    pub end: End,
}

#[derive(Clone, Debug)]
pub enum Step {
    /// Keep the row when the expression is `True`.
    Filter(Tm),
    /// Bind the variable to the value computed in the row's environment.
    Bind(Var, Eval),
    /// Replace the row by one row per element of the set expression, binding the variable to it.
    Unnest(Var, Tm),
    /// Replace the row by one row per element of the build whose key equals `key`, binding the variable to it.
    Probe {
        build: BuildId,
        key: Vec<Tm>,
        var: Var,
    },
}

#[derive(Clone, Debug)]
pub enum End {
    /// Drop the row.
    Nothing,
    /// Push the expression's value into the query's sink.
    Emit(Tm),
    /// Pass the row to every chain.
    Fork(Vec<Chain>),
    If(Tm, Box<Chain>, Box<Chain>),
    /// Route the row by the constructor of the scrutinee's value, binding the arm's variable to it.
    Case(Tm, Vec<(u32, Var, Chain)>),
    /// Probe the index with the key values, for a prefix of its fields, and route each entry by its constructor to a fetch of its row from that heap, bound to `var`; drop entries of constructors with no route.
    Index {
        index: IndexId,
        key: Vec<Tm>,
        var: Var,
        routes: Vec<Route>,
    },
}

#[derive(Clone, Debug)]
pub struct Route {
    pub ctor: u32,
    pub heap: HeapId,
    pub chain: Chain,
}

/// What a query forms from the set of values its pipelines emit.
#[derive(Clone, Debug)]
pub enum Sink {
    Set,
    Exists,
    IsEmpty,
    /// The aggregate of `body` over the emitted values, each bound to `var`.
    Agg {
        op: Agg,
        var: Var,
        body: Eval,
        ty: Ty,
        body_ty: Ty,
    },
    /// One record per distinct value of `by` over the emitted values bound to `var`.
    Group {
        var: Var,
        by: Eval,
        rows: bool,
        aggs: Vec<GroupAgg>,
    },
    Top {
        count: Eval,
        keys: Vec<(Eval, bool)>,
        var: Var,
    },
    Present {
        var: Var,
        keys: Vec<(Eval, bool)>,
        take: Option<Eval>,
        after: Option<Eval>,
    },
}

/// An aggregate computed per group over the group's rows.
#[derive(Clone, Debug)]
pub struct GroupAgg {
    pub field: String,
    pub op: Agg,
    pub ty: Ty,
    pub body_ty: Ty,
    pub row: Var,
    pub filter: Vec<Eval>,
    pub elem: Var,
    pub element: Eval,
    pub body: Eval,
}

/// A simultaneous least fixpoint of set variables.
#[derive(Clone, Debug)]
pub struct Fix {
    pub binds: Vec<FixBind>,
    pub body: Eval,
}

#[derive(Clone, Debug)]
pub struct FixBind {
    pub var: Var,
    pub body: Eval,
    /// The elements the previous step added.
    pub delta: Var,
    /// A superset of the elements a step adds, over the current sets and the deltas.
    pub step: Option<Eval>,
}

/// The rewrites the planner may apply. A plan with any of them off has the same results.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rewrites {
    pub index_scan: bool,
    pub index_join: bool,
    pub hash_join: bool,
    pub fuse_probe: bool,
    pub static_route: bool,
    pub prune: bool,
    pub unswitch: bool,
    pub group_agg: bool,
    pub semi_naive: bool,
}

impl Rewrites {
    pub const ALL: Rewrites = Rewrites {
        index_scan: true,
        index_join: true,
        hash_join: true,
        fuse_probe: true,
        static_route: true,
        prune: true,
        unswitch: true,
        group_agg: true,
        semi_naive: true,
    };

    pub const NONE: Rewrites = Rewrites {
        index_scan: false,
        index_join: false,
        hash_join: false,
        fuse_probe: false,
        static_route: false,
        prune: false,
        unswitch: false,
        group_agg: false,
        semi_naive: false,
    };
}

impl Default for Rewrites {
    fn default() -> Rewrites {
        Rewrites::ALL
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub rewrites: Rewrites,
    /// Set for a bundle declaring `hide new constructors`.
    pub hiding: bool,
}

/// Why a term has no plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The term calls the Wasm or host function, or the user-defined aggregate, of this name.
    UserCode(String),
    /// The term is a write, a block, a transition read or a history read.
    NotAQuery(&'static str),
    /// The term breaks the invariant this text names, one the checker or normaliser guarantees.
    Internal(String),
    /// Under [`plan_bounded`], the term reads `ctors` of `rel` where no index covering them
    /// has leading fields among `fields`, the fields its conditions fix.
    Unbounded {
        rel: RelId,
        ctors: CtorSet,
        fields: Vec<String>,
    },
    /// Under [`plan_bounded`], the term holds this operator.
    Outside(&'static str),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::UserCode(name) => write!(f, "a plan cannot run user-defined code yet: `{name}`"),
            Error::NotAQuery(what) => write!(f, "a plan computes a query, not {what}"),
            Error::Internal(what) => write!(f, "a broken invariant: {what}"),
            Error::Unbounded { rel, fields, .. } => {
                write!(
                    f,
                    "no index probe reaches {rel:?} on ({})",
                    fields.join(", ")
                )
            }
            Error::Outside(what) => write!(f, "`{what}` is outside the bound"),
        }
    }
}
