use std::fmt;

use adtb_lang::canon::Unnamed;
use adtb_lang::evolve::{self, LogGrant};
use adtb_lang::kernel::Invalid;
use adtb_lang::norm;
use adtb_lang::span::{Diag, FileDiag};
use adtb_lang::types::Ty;

use crate::exec::{self, Val};
use crate::key::Value;
use crate::{plan, write};

use super::Error;
use super::change::Failed;
use super::ids::{CatConstraintId, CatCtorId, CatFieldId, CatIndexId, CatRelId};
use super::root::{Changing, Unended};

/// A guarantee of the checker, the planner, the executor or the catalog itself that does not
/// hold in the catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Broken {
    /// The engine's own declarations do not check; each diagnostic.
    Declarations(Vec<FileDiag>),
    /// A migration of the log does not resolve; each diagnostic.
    Unresolved(Vec<Diag>),
    /// A term of a built schema writes a name its source does not resolve.
    Unnamed(Unnamed),
    /// A settled source's schema takes a relation as added later.
    Unsettled,
    /// A checked schema has no normal form.
    NotNormal(norm::Error),
    /// A term of a checked schema or of its normal form breaks the kernel's invariants.
    IllTyped(Invalid),
    /// A stored schema with a derived index element has no normal form.
    StoredNotNormal(norm::Error),
    /// The planner refuses a constraint of a checked schema.
    Plan(plan::Error),
    /// The executor fails computing an index key.
    IndexKey(exec::Error),
    /// The executor fails validating a constraint.
    Validation(exec::Error),
    /// A constraint's query returns this value, not a set.
    NotASet(Val),
    /// A constraint's query has this type, not a set's.
    NotSetTyped(Ty),
    /// Compiling the incremental checks of a schema breaks a guarantee of the checker.
    Admission(write::Broken),
    /// A change of the catalog ended while its root page records this other change in progress.
    ChangeMoved(Changing),
    KeyCounter(write::Error),
    GrantId(write::Error),
    /// A grant row id is drawn with no page of id counters named.
    NoSequences,
    /// A log grant names an object the schema lacks.
    GrantTarget(LogGrant),
    /// A relation kept across a migration lacks an id for one of its constructors.
    KeptCtor(CatRelId, CatCtorId),
    /// The schema keeping pending drops has other relations than the version's.
    PendingRelations,
    /// The schema keeping pending drops lacks this id of this pending-dropped constraint or check.
    KeptConstraint(String, CatConstraintId),
    /// A checked schema gives this constraint or check no catalog id.
    ConstraintId(String),
    /// A checked schema's normal form holds no term of this constraint or check.
    ConstraintTerm(String),
    /// The old key a pending drop holds has no image in the schema.
    NoImage {
        module: String,
        name: String,
        index: CatIndexId,
    },
    /// The executor fails a data map.
    Map(exec::Error),
    /// A fill or routing takes this many parameters, not the row alone.
    RowParams(usize),
    /// A constructor has no field of this name.
    NoField(String),
    /// A routed row has no value for the field of this name.
    Unrouted(String),
    /// The field of this name has no counterpart in the row before.
    NoCounterpart(String),
    /// A row has no field of this name to copy.
    NotCopied(String),
    /// A row before a rekey has no value for the field of this id.
    NotRekeyed(CatFieldId),
    /// A field of this name has no catalog id.
    NoFieldId(String),
    /// A relation has no constructor of this name.
    NoCtor(String),
    /// A discriminator has no constructor of this name.
    NotDiscriminated(String),
    /// A nested sum has no constructor of this name.
    NotNested(String),
    /// A merged constructor has no discriminator field of this name.
    NoDiscriminator(String),
    /// The constructor of this name has no field.
    Fieldless(String),
    /// A nesting constructor's last field holds no declared sum.
    LastNotSum,
    /// A routing gives a value that is not a constructor.
    RoutedNotCtor,
    /// An unnested field holds a value that is not a constructor.
    UnnestedNotCtor,
    /// The schema after a rekey lacks the rekeyed relation.
    RekeyedGone(CatRelId),
    /// The data holds a constructor the schema before lacks.
    ForeignCtor(CatCtorId),
    /// A stored value has another shape than its type stores.
    Unshaped(Value),
    /// The migration model fails on a migration, for this reason.
    Model(String),
}

impl From<Broken> for Error {
    fn from(i: Broken) -> Error {
        Error::Internal(Box::new(i))
    }
}

impl From<Unended> for Error {
    fn from(e: Unended) -> Error {
        match e {
            Unended::Page(e) => Error::Page(e),
            Unended::Moved(c) => Error::from(Broken::ChangeMoved(c)),
        }
    }
}

impl From<Broken> for Failed {
    fn from(i: Broken) -> Failed {
        Failed::Internal(Box::new(i))
    }
}

impl fmt::Display for Broken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Broken::Declarations(ds) => write!(f, "the engine's declarations check: {ds:?}"),
            Broken::Unresolved(ds) => write!(f, "every logged migration resolves: {ds:?}"),
            Broken::Unnamed(u) => write!(
                f,
                "a term of a built schema writes a name its source does not resolve: {u}"
            ),
            Broken::Unsettled => {
                f.write_str("a settled source's schema takes no relation as added later")
            }
            Broken::NotNormal(e) => write!(f, "a checked schema normalises: {e:?}"),
            Broken::StoredNotNormal(e) => write!(f, "the schema normalises: {e:?}"),
            Broken::IllTyped(e) => write!(
                f,
                "a checked schema's kernel term {:?} keeps the kernel's invariants: {}",
                e.at(),
                e.reason()
            ),
            Broken::Plan(e) => write!(f, "{e}"),
            Broken::IndexKey(exec::Error::Internal(why)) => write!(f, "{why}"),
            Broken::IndexKey(e) => write!(f, "an index key evaluates: {}", exec::detail(e)),
            Broken::Validation(e) => f.write_str(&exec::detail(e)),
            Broken::NotASet(v) => write!(f, "a constraint returns a set, found {v:?}"),
            Broken::NotSetTyped(t) => write!(f, "a constraint is typed as a set, found {t:?}"),
            Broken::Admission(b) => write!(f, "{b}"),
            Broken::ChangeMoved(c) => {
                write!(f, "a change of the catalog ended during its change {c:?}")
            }
            Broken::KeyCounter(e) => write!(f, "advancing a key counter: {}", write::detail(e)),
            Broken::GrantId(e) => write!(f, "drawing a grant id: {}", write::detail(e)),
            Broken::NoSequences => f.write_str("a grant row id needs a named page of id counters"),
            Broken::GrantTarget(g) => write!(f, "a log grant names live objects: {g:?}"),
            Broken::KeptCtor(r, c) => write!(f, "relation {r} keeps constructor {c}"),
            Broken::PendingRelations => {
                f.write_str("the schema keeping pending drops has the relations of the version")
            }
            Broken::KeptConstraint(object, id) => write!(
                f,
                "the schema keeping pending drops lacks id {id} of {object}"
            ),
            Broken::ConstraintId(name) => {
                write!(f, "constraint or check {name} has no catalog id")
            }
            Broken::ConstraintTerm(name) => {
                write!(f, "constraint or check {name} has no normal form")
            }
            Broken::NoImage { module, name, .. } => {
                write!(f, "pending-dropped {module}::{name} has no image")
            }
            Broken::Map(e) => write!(f, "a data map fails: {}", exec::detail(e)),
            Broken::RowParams(n) => write!(f, "a fill or routing takes the row, not {n} values"),
            Broken::NoField(n) => write!(f, "no field `{n}`"),
            Broken::Unrouted(n) => write!(f, "field `{n}` has no value in the routed row"),
            Broken::NoCounterpart(n) => write!(f, "field `{n}` has no counterpart"),
            Broken::NotCopied(n) => write!(f, "the row has no field `{n}` to move"),
            Broken::NotRekeyed(id) => write!(f, "field {id} has no value before the rekey"),
            Broken::NoFieldId(n) => write!(f, "field `{n}` has no catalog id"),
            Broken::NoCtor(n) => write!(f, "no constructor `{n}`"),
            Broken::NotDiscriminated(n) => write!(f, "the discriminator has no `{n}`"),
            Broken::NotNested(n) => write!(f, "the nested sum has no `{n}`"),
            Broken::NoDiscriminator(n) => write!(f, "no discriminator `{n}`"),
            Broken::Fieldless(n) => write!(f, "`{n}` has no field"),
            Broken::LastNotSum => f.write_str("the constructor's last field holds no declared sum"),
            Broken::RoutedNotCtor => f.write_str("a routing gives a constructor"),
            Broken::UnnestedNotCtor => f.write_str("the unnested field holds a constructor"),
            Broken::RekeyedGone(r) => write!(f, "the rekeyed relation {r} is gone"),
            Broken::Model(why) => write!(f, "{why}"),
            Broken::Unshaped(v) => {
                write!(f, "a stored value has another shape than its type: {v:?}")
            }
            Broken::ForeignCtor(c) => write!(
                f,
                "the data holds only constructors of the schema before, not {c}"
            ),
        }
    }
}

/// Returns [`Broken::Model`] when the migration model failed on the migration `f` stands for.
pub fn model_fault<R>(f: &evolve::Failed<R>) -> Option<Broken> {
    match f {
        evolve::Failed::Refused(_) => None,
        evolve::Failed::Broken(d) => Some(Broken::Model(d.message.clone())),
    }
}

/// Returns the refusal of failure `f`, or [`Broken::Model`] when the migration model failed on
/// the migration.
pub fn refused_by<R>(f: evolve::Failed<R>) -> Result<R, Box<Broken>> {
    match f {
        evolve::Failed::Refused(r) => Ok(r),
        evolve::Failed::Broken(d) => Err(Box::new(Broken::Model(d.message))),
    }
}

/// Returns what a log failing to resolve with `f` breaks: [`Broken::Model`] when the migration
/// model failed on a migration, else [`Broken::Unresolved`].
pub fn unresolved(f: evolve::Failed<Vec<Diag>>) -> Broken {
    refused_by(f).map_or_else(|b| *b, Broken::Unresolved)
}

/// Returns the error of failure `f`: [`Broken::Model`] when the migration model failed on the
/// migration, else `refused` of the message of each operator it refuses, then of each file its
/// recheck refuses.
pub(super) fn refusal(f: evolve::Failed, refused: impl FnOnce(Vec<String>) -> Error) -> Error {
    let r = match refused_by(f) {
        Ok(r) => r,
        Err(b) => return (*b).into(),
    };
    let ops = r.operators.into_iter().map(|d| d.message);
    refused(
        ops.chain(r.recheck.into_iter().map(|d| d.diag.message))
            .collect(),
    )
}
