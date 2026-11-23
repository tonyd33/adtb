use std::sync::Arc;

use adtb_lang::kernel::{Tm, Valid, Var};
use adtb_lang::schema::Schema;

use crate::{exec, plan};

use super::{Broken, Catalog, Error, Rot};

impl Catalog {
    /// Bind the host function bodies of `hosts` to the index keys, data maps and validations of
    /// later migrations.
    pub fn set_hosts(&mut self, hosts: Arc<exec::Hosts>) {
        for (_, _, keys) in self.indexes.values_mut() {
            let mut bound = (**keys).clone();
            bound.set_hosts(&hosts);
            *keys = Arc::new(bound);
        }
        self.hosts = hosts;
    }
}

/// Returns the scalar of `body` over `var`, calling the host function bodies of `hosts`.
///
/// Fail as [`classify`] does, naming `object`, when the executor cannot compile it.
pub(super) fn scalar(
    object: &str,
    (schema, terms): (&Schema, Valid<'_>),
    (var, body): (Var, Tm),
    hosts: &Arc<exec::Hosts>,
    internal: fn(exec::Error) -> Broken,
) -> Result<exec::Scalar, Error> {
    let mut scalar =
        exec::Scalar::new(schema, terms, var, body).map_err(|e| classify(object, e, internal))?;
    scalar.set_hosts(hosts.clone());
    Ok(scalar)
}

/// Returns the program of `p`, planned against `schema` and `catalog`, calling the host function
/// bodies of `hosts`.
///
/// Fail as [`classify`] does, naming `object`, when the executor cannot compile it.
pub(super) fn program(
    object: &str,
    (schema, catalog): (&Schema, &plan::Catalog),
    p: &plan::Plan,
    hosts: &Arc<exec::Hosts>,
    internal: fn(exec::Error) -> Broken,
) -> Result<exec::Program, Error> {
    let mut program =
        exec::compile(schema, catalog, p, None).map_err(|e| classify(object, e, internal))?;
    program.shared.hosts = hosts.clone();
    Ok(program)
}

/// Returns the error of a migration whose evaluation `e` reached a host function with no body
/// bound at the version its declaration names, or `None` when `e` is another error.
pub fn unbound(e: &exec::Error) -> Option<Error> {
    match e {
        exec::Error::MissingFunction(function) => Some(Error::MissingFunction(function.clone())),
        exec::Error::OtherVersion(symbol) => Some(Error::OtherVersion(symbol.clone())),
        _ => None,
    }
}

/// Returns `e`, a failure of the executor on `object`, as the catalog's: a form the executor
/// does not run as a refusal naming `object`, damage and a page failure as themselves, a call of
/// a host function with no body bound as its own error, and any other as `internal` of it.
pub(super) fn classify(object: &str, e: exec::Error, internal: fn(exec::Error) -> Broken) -> Error {
    if let Some(e) = unbound(&e) {
        return e;
    }
    match e {
        exec::Error::Unsupported(why) => {
            Error::Refused(vec![format!("{object} is not admitted: {why}")])
        }
        exec::Error::Page(e) => Error::Page(e),
        exec::Error::Spill(e) => Error::Spill(e),
        exec::Error::Corrupt(rot) => Rot::Exec(rot).into(),
        exec::Error::OrphanEntry(index) => Rot::Orphaned(index).into(),
        other => Error::from(internal(other)),
    }
}
