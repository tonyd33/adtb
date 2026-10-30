//! Writes of deployment state: relations of the catalog whose rows no migration writes.

use std::collections::BTreeMap;

use adtb_lang::types::RelId;

use crate::key::Value;
use crate::mvcc::{Scope, Transactions};
use crate::runtime::Runtime;
use crate::storage::Pager;

use super::{Catalog, Error, Fact, Migrating, delete_rows};

/// The rows of one relation [`Catalog::deploy`] deletes and inserts, each a constructor and its
/// stored fields.
#[derive(Clone, Debug)]
pub struct Rewrite {
    pub relation: RelId,
    pub deleted: Vec<(u32, Vec<Value>)>,
    pub inserted: Vec<(u32, Vec<Value>)>,
}

impl Catalog {
    /// Returns the rows of `r` the latest commit holds.
    pub fn committed_rows<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        r: RelId,
    ) -> Result<Vec<(u32, Vec<Value>)>, Error> {
        let (id, snapshot) = txns.take(Scope::Every)?;
        let rows = self.rows(pager, txns, &snapshot, r);
        txns.release(id);
        Ok(rows?)
    }

    /// Delete and insert the rows of `writes` as one transaction, returning once it is durable.
    ///
    /// Preconditions:
    /// - Every relation `writes` names is a relation of deployment state, whose rows no
    ///   migration writes.
    /// - Every deleted row is a row the latest commit holds.
    pub fn deploy<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        writes: &[Rewrite],
    ) -> Result<(), Error> {
        self.deploy_with(pager, txns, writes, &[])
    }

    /// Delete and insert the rows of `writes` and write the deployment facts `facts` as one
    /// transaction, returning once it is durable, under the preconditions of [`Catalog::deploy`].
    pub fn deploy_with<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        writes: &[Rewrite],
        facts: &[Fact],
    ) -> Result<(), Error> {
        let x = txns.begin(pager)?;
        let (id, snapshot) = txns.take(Scope::Every)?;
        let m = Migrating {
            pager,
            txns,
            x,
            snapshot: &snapshot,
        };
        let done = writes
            .iter()
            .try_for_each(|w| {
                let mut gone: BTreeMap<u32, Vec<Vec<Value>>> = BTreeMap::new();
                for (c, row) in &w.deleted {
                    gone.entry(*c).or_default().push(row.clone());
                }
                for (c, rows) in gone {
                    delete_rows(&m, self.heap(w.relation, c), &rows)?;
                }
                w.inserted
                    .iter()
                    .try_for_each(|(c, row)| self.insert(pager, x, w.relation, *c, row))
            })
            .and_then(|()| self.write_facts(pager, x, facts));
        txns.release(id);
        match done {
            Ok(()) => {
                let lsn = txns.commit(pager, x)?;
                pager.wait_durable(lsn);
                Ok(())
            }
            Err(e) => {
                txns.abort(pager, x)?;
                Err(e)
            }
        }
    }
}
