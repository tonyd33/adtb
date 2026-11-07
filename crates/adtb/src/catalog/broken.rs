use std::fmt;

use adtb_lang::evolve::LogGrant;
use adtb_lang::norm;
use adtb_lang::span::FileDiag;
use adtb_lang::types::Ty;

use crate::exec::{self, Val};
use crate::key::Value;
use crate::{plan, write};

use super::Error;
use super::change::Failed;
use super::ids::{CatCtorId, CatFieldId, CatIndexId, CatRelId};
use super::root::Changing;

/// A guarantee of the checker, the planner, the executor or the catalog itself that does not
/// hold in the catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Broken {
    /// The engine's own declarations do not check; each diagnostic.
    Declarations(Vec<FileDiag>),
    /// A checked schema has no normal form.
    NotNormal(norm::Error),
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
    /// A rebuild of indexes ended during this other change of the catalog.
    ReindexDuring(Changing),
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
    /// The old key a pending drop holds has no image in the schema.
    NoImage {
        module: String,
        name: String,
        index: CatIndexId,
    },
    /// A pending-dropped key or unique is declared again.
    PendingRedeclared {
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
}

impl From<Broken> for Error {
    fn from(i: Broken) -> Error {
        Error::Internal(Box::new(i))
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
            Broken::NotNormal(e) => write!(f, "a checked schema normalises: {e:?}"),
            Broken::StoredNotNormal(e) => write!(f, "the schema normalises: {e:?}"),
            Broken::Plan(e) => write!(f, "{e}"),
            Broken::IndexKey(exec::Error::Internal(why)) => write!(f, "{why}"),
            Broken::IndexKey(e) => write!(f, "an index key evaluates: {}", exec::detail(e)),
            Broken::Validation(e) => f.write_str(&exec::detail(e)),
            Broken::NotASet(v) => write!(f, "a constraint returns a set, found {v:?}"),
            Broken::NotSetTyped(t) => write!(f, "a constraint is typed as a set, found {t:?}"),
            Broken::Admission(b) => write!(f, "{b}"),
            Broken::ReindexDuring(c) => write!(
                f,
                "a rebuild of indexes ended during the catalog's change {c:?}"
            ),
            Broken::KeyCounter(e) => write!(f, "advancing a key counter: {}", write::detail(e)),
            Broken::GrantId(e) => write!(f, "drawing a grant id: {}", write::detail(e)),
            Broken::NoSequences => f.write_str("a grant row id needs a named page of id counters"),
            Broken::GrantTarget(g) => write!(f, "a log grant names live objects: {g:?}"),
            Broken::KeptCtor(r, c) => write!(f, "relation {r} keeps constructor {c}"),
            Broken::PendingRelations => {
                f.write_str("the schema keeping pending drops has the relations of the version")
            }
            Broken::NoImage { module, name, .. } => {
                write!(f, "pending-dropped {module}::{name} has no image")
            }
            Broken::PendingRedeclared { module, name, .. } => {
                write!(
                    f,
                    "pending-dropped {module}::{name} is never declared again"
                )
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
