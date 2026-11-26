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
    internal: fn(exec::Broken) -> Broken,
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
    internal: fn(exec::Broken) -> Broken,
) -> Result<exec::Program, Error> {
    let mut program =
        exec::compile(schema, catalog, p, None).map_err(|e| classify(object, e, internal))?;
    program.shared.hosts = hosts.clone();
    Ok(program)
}

/// Returns `e`, a failure of the executor on `object`, as the catalog's: a form the executor
/// does not run as a refusal naming `object`, a call of a host function with no body bound as
/// its own error, a broken guarantee as `internal` of it, and any other in its own class.
pub(super) fn classify(
    object: &str,
    e: exec::Error,
    internal: fn(exec::Broken) -> Broken,
) -> Error {
    match e {
        exec::Error::Internal(b) => Error::from(internal(*b)),
        exec::Error::Unsupported(why) => {
            Error::Refused(vec![format!("{object} is not admitted: {why}")])
        }
        exec::Error::Page(e) => Error::Page(e),
        exec::Error::Spill(e) => Error::Spill(e),
        exec::Error::Corrupt(rot) => Rot::Exec(rot).into(),
        exec::Error::OrphanEntry(index) => Rot::Orphaned(index).into(),
        exec::Error::Serialization(c) => Error::Conflict(c),
        exec::Error::MissingFunction(function) => Error::MissingFunction(function),
        exec::Error::OtherVersion(symbol) => Error::OtherVersion(symbol),
        e @ (exec::Error::Fault(_)
        | exec::Error::StatementLimit(_)
        | exec::Error::NoCaller
        | exec::Error::Rejected(_)
        | exec::Error::Cancelled
        | exec::Error::Overloaded
        | exec::Error::DeadlineExceeded
        | exec::Error::HistoryUnavailable { .. }
        | exec::Error::WrongVersion { .. }) => Error::Exec(ExecFailure(e)),
    }
}

/// An executor failure of a query the catalog runs that a statement's query would fail with too:
/// a fault, a statement limit, a cancellation, a deadline, an overload, a rejection, a read of
/// `caller` or of the past. Only [`classify`] builds one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecFailure(exec::Error);

impl ExecFailure {
    pub fn error(&self) -> &exec::Error {
        &self.0
    }

    pub fn into_error(self) -> exec::Error {
        self.0
    }
}
