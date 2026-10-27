use std::sync::Arc;

use crate::exec;

use super::{Catalog, Error};

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

/// Returns the error of a migration whose evaluation `e` reached a host function with no body
/// bound at the version its declaration names, or `None` when `e` is another error.
pub fn unbound(e: &exec::Error) -> Option<Error> {
    match e {
        exec::Error::MissingFunction(function) => Some(Error::MissingFunction(function.clone())),
        exec::Error::OtherVersion(symbol) => Some(Error::OtherVersion(symbol.clone())),
        _ => None,
    }
}
