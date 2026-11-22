//! Deployment facts the roots heap holds beside the catalog's own: the durable owner of each
//! bundle family and each bundle's successful registration.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::heap::Heap;
use crate::mvcc::{Hold, Scope, Transactions, Xid};
use crate::runtime::Runtime;
use crate::storage::Pager;

use super::roots::RootsRow;
use super::{Catalog, Error, ROOTS, visible};

/// Who registered a bundle: an open given it, or a role through the registration command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Open,
    Command,
}

/// A bundle's successful registration: the id of the role that registered it, how, and the
/// text of its file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mark {
    pub role: i64,
    pub origin: Origin,
    pub text: String,
}

/// The deployment facts the latest commit holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Deployed {
    /// Whether the file records family owners; a file written before it did has none.
    pub marked: bool,
    /// The id of each family's owner, by family.
    pub families: BTreeMap<String, i64>,
    /// The successful registration of each bundle, by bundle id.
    pub registered: BTreeMap<i64, Mark>,
    /// The latest renewal latched as expired of each bundle, by bundle id. A later renewal is
    /// not latched.
    pub expired: BTreeMap<i64, i64>,
}

/// One deployment fact to write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fact {
    Deployed,
    Family(String, i64),
    Registered(i64, Mark),
    /// The lease of the bundle of this id, last renewed at this instant, stays expired.
    Expired(i64, i64),
}

/// The writer of deployment facts into transactions of the write path.
#[derive(Clone)]
pub struct Deployment {
    roots: Arc<Heap>,
}

impl std::fmt::Debug for Deployment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Deployment").finish_non_exhaustive()
    }
}

impl Deployed {
    /// Hold `facts` as well, once they commit.
    pub fn record(&mut self, facts: &[Fact]) {
        for f in facts {
            match f {
                Fact::Deployed => self.marked = true,
                Fact::Family(family, role) => {
                    self.families.insert(family.clone(), *role);
                }
                Fact::Registered(bundle, m) => {
                    self.registered.insert(*bundle, m.clone());
                }
                Fact::Expired(bundle, renewed) => self.latch(*bundle, *renewed),
            }
        }
    }

    fn latch(&mut self, bundle: i64, renewed: i64) {
        let latest = self.expired.entry(bundle).or_insert(renewed);
        *latest = renewed.max(*latest);
    }
}

impl Catalog {
    /// Returns the deployment facts the latest commit holds.
    pub fn deployed<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
    ) -> Result<Deployed, Error> {
        let hold = txns.take(Scope::Heaps([ROOTS].into()))?;
        let out = self.deployment().at(pager, txns, &hold);
        drop(hold);
        out
    }

    /// Returns the writer of deployment facts.
    pub fn deployment(&self) -> Deployment {
        Deployment {
            roots: self.roots.clone(),
        }
    }

    pub(super) fn write_facts<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        x: Xid,
        facts: &[Fact],
    ) -> Result<(), Error> {
        self.deployment().insert(pager, txns, x, facts)
    }
}

impl Deployment {
    /// Returns the deployment facts the snapshot of `hold` sees.
    pub fn at<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        hold: &Hold,
    ) -> Result<Deployed, Error> {
        let mut out = Deployed::default();
        for row in visible(pager, txns, hold, &self.roots)? {
            match RootsRow::decode(&row)? {
                RootsRow::Deployment(f) => out.record(std::slice::from_ref(&f)),
                RootsRow::HeapRoot { .. }
                | RootsRow::IndexRoot { .. }
                | RootsRow::OrderKey { .. }
                | RootsRow::Migration { .. }
                | RootsRow::HistoryHeap { .. }
                | RootsRow::HistoryIndex { .. }
                | RootsRow::Held { .. }
                | RootsRow::Dormant { .. }
                | RootsRow::Kept { .. } => {}
            }
        }
        Ok(out)
    }

    #[doc(hidden)]
    #[cfg(test)]
    pub fn roots(&self) -> &Heap {
        &self.roots
    }

    /// Write `facts` in transaction `x`, so that they commit or abort with it.
    ///
    /// Precondition: `x` is a transaction of the write path over the pager this catalog opened.
    pub fn insert<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        x: Xid,
        facts: &[Fact],
    ) -> Result<(), Error> {
        let rows = facts.iter().map(|f| RootsRow::Deployment(f.clone()));
        let rows = super::admit_roots(&self.roots, rows)?;
        super::insert_roots(pager, txns, &self.roots, x, &rows)
    }
}
