use adtb_lang::types::RelId;

use crate::btree::Index;
use crate::key::{Entry, Value};
use crate::runtime::Runtime;

use super::admit::Admitted;
use super::ids::{ctor_id, ctor_index, relation, stored};
use super::keying::row_key;
use super::{Catalog, Error, Migrating};

/// A row and the entry each index over its constructor takes for it.
struct Keyed<'c, T> {
    row: T,
    entries: Vec<(&'c Index, Vec<Value>)>,
}

impl<T: AsRef<[Value]>> AsRef<[Value]> for Keyed<'_, T> {
    fn as_ref(&self) -> &[Value] {
        self.row.as_ref()
    }
}

impl Catalog {
    /// Insert `row` of constructor `c` into relation `r` as `m.x`, with an entry in every index
    /// over `c`, once [`Catalog::admitted`] admits it.
    pub(super) fn insert<R: Runtime>(
        &self,
        m: &Migrating<'_, R>,
        r: RelId,
        c: u32,
        row: &[Value],
    ) -> Result<(), Error> {
        self.insert_all(m, &self.admitted(m.pager, r, c, [row.to_vec()])?)
    }

    /// Insert `rows` as `m.x`, each constructor relation's in order, with an entry in every index
    /// over its constructor.
    ///
    /// Panics:
    /// - `rows` holds a relation or constructor this catalog's schema lacks.
    pub(super) fn insert_all<R: Runtime>(
        &self,
        m: &Migrating<'_, R>,
        rows: &Admitted,
    ) -> Result<(), Error> {
        for (&rid, ctors) in rows.rows() {
            let r =
                relation(self.schema(), rid).expect("admitted rows are of the catalog's relations");
            for (&cid, rows) in ctors {
                let c = ctor_index(self.schema(), r, cid).expect("a constructor of the relation");
                self.insert_ctor(m, r, c, rows)?;
            }
        }
        Ok(())
    }

    fn insert_ctor<R: Runtime>(
        &self,
        m: &Migrating<'_, R>,
        r: RelId,
        c: u32,
        rows: &[Vec<Value>],
    ) -> Result<(), Error> {
        let pager = m.pager;
        let ctor = stored(ctor_id(self.schema(), r, c));
        let keyed = rows.iter().map(|row| {
            let mut entries = Vec::new();
            for (spec, index, keys) in self.indexes.values() {
                if spec.relation != r {
                    continue;
                }
                if let Some(key) = row_key(keys, index.schema(), c, row)? {
                    entries.push((index, key));
                }
            }
            Ok(Keyed { row, entries })
        });
        m.insert_all(self.heap(r, c), keyed, |k, tid| {
            for (index, key) in k.entries {
                let fresh = index.insert(pager, &Entry { key, ctor, tid })?;
                assert!(fresh, "a new row's entry is not in its index yet");
            }
            Ok(())
        })
    }
}
