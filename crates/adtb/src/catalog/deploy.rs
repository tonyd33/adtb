//! The writes of deployment state: relations of the catalog whose rows no migration writes.

use std::collections::{BTreeMap, BTreeSet};

use adtb_lang::ids::engine_rel;
use adtb_lang::types::RelId;

use crate::key::Value;
use crate::mvcc::{Scope, Transactions};
use crate::runtime::Runtime;
use crate::storage::{Pager, Position};

use super::codec::{Codec, Decode, GrantKind, RoleGrant};
use super::ids::{CatRelId, rel_id};
use super::{Broken, Catalog, Change, Error, Fact, Snapshot, Writer, delete_rows};

/// The relations of deployment state, the only relations a deploy writes.
const DEPLOYED: [CatRelId; 4] = [
    engine_rel::ROLES,
    engine_rel::GRANTS,
    engine_rel::BUNDLES,
    engine_rel::REGISTERED,
];

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
    pub(super) fn committed_rows<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        r: RelId,
    ) -> Result<Vec<(u32, Vec<Value>)>, Error> {
        let hold = txns.take(Scope::Every)?;
        let rows = self.rows(&Snapshot::new(pager, txns, (&hold, None)), r);
        drop(hold);
        rows
    }

    /// Returns the rows of `codec`'s relation the latest commit holds, each read by `codec`.
    pub fn committed<T: Decode, R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        codec: &Codec<T>,
    ) -> Result<Vec<T>, Error> {
        self.committed_rows(pager, txns, codec.relation())?
            .into_iter()
            .map(|(c, row)| codec.decode(c, &row).map_err(Error::from))
            .collect()
    }

    /// Delete and insert the rows of `writes` as one transaction. Returns its commit position,
    /// once it is durable.
    ///
    /// Fail as broken, writing nothing, when `writes` names a relation other than the engine's
    /// `roles`, `grants`, `bundles` and `registered`, or a row of `grants` that is not a
    /// deployment grant.
    ///
    /// Precondition: every deleted row is a row the latest commit holds.
    pub fn deploy<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        writes: &[Rewrite],
    ) -> Result<Position, Error> {
        self.deploy_with(pager, txns, writes, &[])
    }

    /// Delete and insert the rows of `writes` and write the deployment facts `facts` as one
    /// transaction, under the preconditions of [`Catalog::deploy`]. Returns its commit position,
    /// once it is durable.
    pub fn deploy_with<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        writes: &[Rewrite],
        facts: &[Fact],
    ) -> Result<Position, Error> {
        let grants = Codec::<RoleGrant>::new(&self.schema);
        for w in writes {
            let id = rel_id(&self.schema, w.relation);
            if !DEPLOYED.contains(&id) {
                return Err(Broken::Undeployed(id).into());
            }
            if w.relation != grants.relation() {
                continue;
            }
            for (c, row) in w.deleted.iter().chain(&w.inserted) {
                if grants.decode(*c, row)?.kind() != GrantKind::Deployment {
                    return Err(Broken::LogGrantDeployed.into());
                }
            }
        }
        self.rewrite(pager, txns, writes, facts)
    }

    /// Write the rows of `writes` and the facts `facts` as [`Catalog::deploy_with`] does, into
    /// any relation, for a test that forges what no deploy writes.
    #[cfg(test)]
    pub(crate) fn forge<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        writes: &[Rewrite],
    ) -> Result<Position, Error> {
        self.rewrite(pager, txns, writes, &[])
    }

    fn rewrite<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        writes: &[Rewrite],
        facts: &[Fact],
    ) -> Result<Position, Error> {
        let change = Change::begin(pager, txns, Scope::Every)?;
        let m = change.writes();
        let done = writes
            .iter()
            .try_for_each(|w| {
                let mut gone: BTreeMap<u32, Vec<Vec<Value>>> = BTreeMap::new();
                for (c, row) in &w.deleted {
                    gone.entry(*c).or_default().push(row.clone());
                }
                for (c, rows) in gone {
                    delete_rows(m, self.heap(w.relation, c), &rows)?;
                }
                w.inserted
                    .iter()
                    .try_for_each(|(c, row)| self.insert(m, w.relation, *c, row))
            })
            .and_then(|()| self.write_facts(pager, txns, Writer::new(&m.hold, m.x), facts));
        if let Err(e) = done {
            return Err(change.abandon(e));
        }
        let lsn = self.end_landed(pager, txns, change.commit()?, (&[], &BTreeSet::new()));
        Ok(pager.position(lsn))
    }
}
