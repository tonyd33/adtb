//! A test's rewrite of the rows of `catalog::migrations`, as damage that keeps every page whole
//! leaves them.

use crate::key::Value;
use crate::mvcc::{Scope, Transactions};
use crate::runtime::Runtime;
use crate::storage::Pager;

use super::codec::{Codec, MigrationRow};
use super::{Catalog, Change, Error, Hash, Migrating, Rot, Snapshot, delete_rows, layout};

impl Catalog {
    /// Replace the hash the row of version `version` of `catalog::migrations` holds by `hash`, or
    /// delete the row when `hash` is `None`, as one committed transaction.
    ///
    /// Fail with [`Rot::NoMigration`] when no row has that version.
    pub fn restamp<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        version: u64,
        hash: Option<Hash>,
    ) -> Result<(), Error> {
        let change = Change::begin(pager, txns, Scope::Every)?;
        match self.restamp_in(change.writes(), version, hash) {
            Ok(()) => {
                let landed = change.commit()?;
                self.end_landed(pager, txns, landed, &[]);
                Ok(())
            }
            Err(e) => Err(change.abandon(e)),
        }
    }

    fn restamp_in<R: Runtime>(
        &self,
        m: &Migrating<'_, R>,
        version: u64,
        hash: Option<Hash>,
    ) -> Result<(), Error> {
        let codec = Codec::<MigrationRow>::new(self.schema());
        let r = codec.relation();
        let mut found = None;
        for (c, row) in self.rows(&Snapshot::of(m), r)? {
            if codec.decode(c, &row)?.version() == version {
                found = Some((c, row));
            }
        }
        let (c, row) = found.ok_or(Rot::NoMigration(version))?;
        delete_rows(m, self.heap(r, c), std::slice::from_ref(&row))?;
        let Some(hash) = hash else {
            return Ok(());
        };
        let at = layout::row(self.schema(), r, c)
            .iter()
            .position(|(n, _)| n == "hash")
            .expect("a row of catalog::migrations has a hash");
        let mut restamped = row;
        restamped[at] = Value::Bytes(hash.to_vec());
        self.insert_all(m, &self.admitted(m.pager, r, c, [restamped])?)
    }
}
