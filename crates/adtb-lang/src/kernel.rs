use crate::schema::Generator;
use crate::types::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Tm(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Var(pub u32);

#[derive(Clone, Debug, PartialEq)]
pub enum Const {
    /// An integer at the node's integer type.
    Int(i128),
    /// The unscaled digits of a decimal at the node's scale.
    Decimal(i128),
    Float(f64),
    /// NFC text.
    Text(String),
    /// Microseconds.
    Duration(i64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Prim {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Neg,
    Concat,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    CheckedAdd,
    CheckedSub,
    CheckedMul,
    CheckedDiv,
    CheckedRem,
    CheckedNeg,
    DecimalDiv,
    Round,
    Rescale,
    Abs,
    /// A numeric conversion to the node's type, or to its `Option` when checked.
    Convert {
        checked: bool,
    },
    Wrap,
    Unwrap,
    KeyValue,
    ParseUuid,
    ToText,
    BytesLen,
    ScalarLen,
    GraphemeLen,
    StartsWith,
    EndsWith,
    Contains,
    Matches,
    Lower,
    Upper,
    Fold,
    Collate,
    Utf8,
    Bytes,
    Micros,
    Millis,
    Seconds,
    Minutes,
    Hours,
    Days,
    Weeks,
    Months,
    Years,
    AddPeriod,
    Zoned,
    InZone,
    ToInstant,
    Span,
    Start,
    End,
    Overlaps,
    Covers,
    /// `T.parse(t)`, at the node's type `Option T`.
    Parse,
    Print,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Agg {
    Count,
    Sum,
    Min,
    Max,
    Avg,
    User(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Transition {
    Inserted,
    Deleted,
    Prior,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Var(Var),
    Const(Const),
    Prim(Prim, Vec<Tm>),
    /// Fields sorted by name.
    Record(Vec<(String, Tm)>),
    Field(Tm, String),
    /// A constructor value of the node's sum, every field given, sorted by name.
    Ctor {
        ctor: u32,
        fields: Vec<(String, Tm)>,
    },
    /// One arm per constructor of the scrutinee's constructor set, in constructor order.
    Case {
        scrutinee: Tm,
        arms: Vec<(u32, Var, Tm)>,
    },
    If(Tm, Tm, Tm),
    Let(Var, Tm, Tm),
    Lam(Var, Tm),
    App(Tm, Tm),
    Call {
        func: FnId,
        type_args: Vec<Ty>,
        args: Vec<Tm>,
    },
    Caller,
    Key(Tm),
    KeyOf(RelId, Tm),
    /// The row a key names; `reference` when the key is a reference field's value, so the row exists.
    Deref {
        key: Tm,
        reference: bool,
    },
    Tag(Tm),
    Try(Tm),
    Rel(RelId, Option<CtorSet>),
    Transition(Transition, RelId, Option<CtorSet>),
    Restrict(Tm, CtorSet),
    EmptySet,
    Singleton(Tm),
    Union(Tm, Tm),
    For(Var, Tm, Tm),
    IsEmpty(Tm),
    Exists(Tm),
    Agg {
        op: Agg,
        var: Var,
        source: Tm,
        body: Tm,
    },
    Group {
        var: Var,
        source: Tm,
        by: Tm,
    },
    Top {
        count: Tm,
        keys: Vec<(Tm, bool)>,
        var: Var,
        source: Tm,
    },
    Fix(Vec<(Var, Tm)>, Tm),
    Fresh(Generator, RelId),
    Only(Tm),
    At {
        instant: bool,
        point: Tm,
        body: Tm,
    },
    /// The final result delivered in order; each key is `(expr, desc)` over `var`.
    Present {
        var: Var,
        source: Tm,
        keys: Vec<(Tm, bool)>,
        take: Option<Tm>,
        after: Option<Tm>,
    },
    Reject(Tm),
    /// A position exhaustiveness proves no evaluation reaches.
    Unreachable,
    Skip,
    /// Insert every row of the set, whole rows with their fills explicit.
    Insert(RelId, Tm),
    /// Delete every row of the set that is a row of the relation in the snapshot.
    Delete(RelId, Tm),
    /// Replace each row of the set, a row of the relation in the snapshot, by the function's value on it; fail with `KeyChanged` when that value has another key, and leave the row alone when it is equal.
    Replace(RelId, Tm, Tm),
    /// Insert each row of `rows` that no snapshot row matches on `on`; for each one that a snapshot row matches, evaluate the write `otherwise` instead, with `old` bound to the snapshot row and `new` to the proposed one.
    Upsert {
        rel: RelId,
        rows: Tm,
        on: Conflict,
        old: Var,
        new: Var,
        otherwise: Tm,
        /// Whether a binding of this upsert is one row rather than a set.
        single: bool,
    },
    /// Evaluate `insert`, an `Insert` or `Upsert` whose rows are a `For` over one source relation, record by record; insert a row into `into` for each record whose computation reaches `Reject` or fails to convert, in place of its rows.
    Rejects {
        into: RelId,
        insert: Tm,
    },
    /// Join the effects of every write; their order chooses nothing.
    Par(Vec<Tm>),
    /// Reject the statement with the text as a user error.
    Fail(Tm),
    /// Join the write's effects to the statement's and bind the variable to the rows it binds: a `Replace`'s new values, an `Upsert`'s binding, the rows a `Rejects` inserts into its target.
    LetBang(Var, Tm, Tm),
    /// Join the write's effects to the statement's. Returns the second term's value.
    Block(Tm, Tm),
    /// Run the block as one statement against one snapshot, under the options. Returns the block's result.
    Transact(Tm, TxnOptions),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Conflict {
    Key,
    /// The unique at this position of the relation's indexes.
    Unique(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Durability {
    Local,
    Quorum,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TxnOptions {
    pub retry: Option<u64>,
    pub read_only: bool,
    pub deferrable: bool,
    pub durability: Option<Durability>,
}

/// One field of an index.
#[derive(Clone, Debug, PartialEq)]
pub struct IndexField {
    /// The field's value at a row that holds every constructor `path` names.
    pub value: Tm,
    /// The steps of an element that is a path through a constructor of a value-level sum; empty for any other element.
    pub path: Vec<PathStep>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathStep {
    Field(String),
    /// The value the steps before it reach holds this constructor.
    Ctor(u32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub kind: Kind,
    pub ty: Ty,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VarInfo {
    pub name: String,
    pub ty: Ty,
}

/// An arena of typed kernel terms. A term may be shared by several parents.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Terms {
    pub nodes: Vec<Node>,
    pub vars: Vec<VarInfo>,
}

impl Terms {
    pub fn add(&mut self, kind: Kind, ty: Ty) -> Tm {
        self.nodes.push(Node { kind, ty });
        Tm(self.nodes.len() as u32 - 1)
    }

    pub fn var(&mut self, name: &str, ty: Ty) -> Var {
        self.vars.push(VarInfo {
            name: name.to_string(),
            ty,
        });
        Var(self.vars.len() as u32 - 1)
    }

    pub fn node(&self, t: Tm) -> &Node {
        &self.nodes[t.0 as usize]
    }

    pub fn kind(&self, t: Tm) -> &Kind {
        &self.nodes[t.0 as usize].kind
    }

    pub fn ty(&self, t: Tm) -> &Ty {
        &self.nodes[t.0 as usize].ty
    }

    pub fn var_info(&self, v: Var) -> &VarInfo {
        &self.vars[v.0 as usize]
    }

    /// Returns the direct subterms of `t`, in evaluation order.
    pub fn children(&self, t: Tm) -> Vec<Tm> {
        match self.kind(t) {
            Kind::Var(_)
            | Kind::Const(_)
            | Kind::Caller
            | Kind::Rel(..)
            | Kind::Transition(..)
            | Kind::EmptySet
            | Kind::Fresh(..)
            | Kind::Unreachable
            | Kind::Skip => Vec::new(),
            Kind::Prim(_, xs) | Kind::Par(xs) => xs.clone(),
            Kind::Record(fs) | Kind::Ctor { fields: fs, .. } => {
                fs.iter().map(|(_, t)| *t).collect()
            }
            Kind::Field(x, _)
            | Kind::Key(x)
            | Kind::KeyOf(_, x)
            | Kind::Deref { key: x, .. }
            | Kind::Tag(x)
            | Kind::Try(x)
            | Kind::Restrict(x, _)
            | Kind::Singleton(x)
            | Kind::IsEmpty(x)
            | Kind::Exists(x)
            | Kind::Only(x)
            | Kind::Reject(x)
            | Kind::Insert(_, x)
            | Kind::Delete(_, x)
            | Kind::Fail(x)
            | Kind::Transact(x, _)
            | Kind::Rejects { insert: x, .. }
            | Kind::Lam(_, x) => vec![*x],
            Kind::Upsert {
                rows, otherwise, ..
            } => vec![*rows, *otherwise],
            Kind::Case { scrutinee, arms } => std::iter::once(*scrutinee)
                .chain(arms.iter().map(|(_, _, t)| *t))
                .collect(),
            Kind::If(a, b, c) => vec![*a, *b, *c],
            Kind::Let(_, a, b)
            | Kind::LetBang(_, a, b)
            | Kind::Union(a, b)
            | Kind::For(_, a, b)
            | Kind::Replace(_, a, b)
            | Kind::Block(a, b)
            | Kind::App(a, b) => vec![*a, *b],
            Kind::Call { args, .. } => args.clone(),
            Kind::Agg { source, body, .. } => vec![*source, *body],
            Kind::Group { source, by, .. } => vec![*source, *by],
            Kind::Top {
                count,
                keys,
                source,
                ..
            } => std::iter::once(*count)
                .chain(std::iter::once(*source))
                .chain(keys.iter().map(|(k, _)| *k))
                .collect(),
            Kind::Fix(binds, body) => binds
                .iter()
                .map(|(_, t)| *t)
                .chain(std::iter::once(*body))
                .collect(),
            Kind::At { point, body, .. } => vec![*point, *body],
            Kind::Present {
                source,
                keys,
                take,
                after,
                ..
            } => std::iter::once(*source)
                .chain(keys.iter().map(|(k, _)| *k))
                .chain(take.iter().copied())
                .chain(after.iter().copied())
                .collect(),
        }
    }
}
