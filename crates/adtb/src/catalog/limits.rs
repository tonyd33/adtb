use adtb_lang::check::Compiled;
use adtb_lang::evolve::Failed;
use adtb_lang::norm::{self, NormalProgram};
use adtb_lang::schema::Schema;
use adtb_lang::span::{Diag, DiagKind, FileDiag};

use crate::plan;

use super::broken::refusal;
use super::{Broken, Error, IndexSpec};

/// Returns [`Error::PastLimits`] when one of the checker's diagnostics `ds` names a stored body
/// past this build's limits.
pub(crate) fn past_limits<'d>(ds: impl IntoIterator<Item = &'d Diag>) -> Option<Error> {
    ds.into_iter().find_map(|d| match &d.kind {
        DiagKind::PastLimits {
            object: Some(object),
        } => Some(Error::PastLimits(object.clone())),
        _ => None,
    })
}

/// Returns the error of `e`, normalising a program of `schema`: [`Error::PastLimits`] when a
/// body expands past this build's limits, else what `other` makes of `e`.
pub(crate) fn not_normal(
    schema: &Schema,
    e: norm::Error,
    other: impl FnOnce(norm::Error) -> Error,
) -> Error {
    match (&e.cause, e.root.object(schema)) {
        (norm::Cause::TooLarge, Some(object)) => Error::PastLimits(object),
        _ => other(e),
    }
}

/// Returns the refusal of stored migration `version` that `f` makes: [`Error::PastLimits`] for a
/// body past this build's limits, the model's failure through [`refusal`].
pub(super) fn unaccepted(version: u64, f: Failed) -> Error {
    let limits = match &f {
        Failed::Refused(r) => {
            past_limits(r.operators.iter().chain(r.recheck.iter().map(|d| &d.diag)))
        }
        Failed::Broken(_) => None,
    };
    limits.unwrap_or_else(|| refusal(f, |why| Error::Unaccepted(version, why)))
}

/// Returns the refusal of the stored source at `version` that the checker refuses with `ds`:
/// [`Error::PastLimits`] for a body past this build's limits.
pub(super) fn unchecked(version: u64, ds: Vec<FileDiag>) -> Error {
    past_limits(ds.iter().map(|d| &d.diag)).unwrap_or_else(|| {
        Error::Unaccepted(version, ds.into_iter().map(|d| d.diag.message).collect())
    })
}

/// Returns the normal program of `compiled` when an index of `specs` has an element other than a
/// field, failing with [`Broken::StoredNotNormal`] for a body that has none for a reason other
/// than this build's limits.
pub(super) fn index_program<'s>(
    compiled: &Compiled,
    mut specs: impl Iterator<Item = &'s IndexSpec>,
) -> Result<Option<NormalProgram>, Error> {
    let derived = specs.any(|s| {
        s.elements
            .iter()
            .any(|e| matches!(e, plan::Element::Derived { .. }))
    });
    if !derived {
        return Ok(None);
    }
    norm::program(compiled).map(Some).map_err(|e| {
        not_normal(compiled.schema(), e, |e| {
            Error::from(Broken::StoredNotNormal(e))
        })
    })
}
