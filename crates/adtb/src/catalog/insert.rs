use adtb_lang::types::RelId;

use crate::btree::{self, Index};
use crate::key::{Entry, Value};
use crate::runtime::Runtime;

use super::ids::{ctor_id, stored};
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
    /// over `c`.
    pub(super) fn insert<R: Runtime>(
        &self,
        m: &Migrating<'_, R>,
        r: RelId,
        c: u32,
        row: &[Value],
    ) -> Result<(), Error> {
        self.insert_all(m, r, c, [row])
    }

    /// Insert `rows` of constructor `c` into relation `r` as `m.x`, in order, with an entry in
    /// every index over `c`. Refuse at the first row in order that is too large or has a key too
    /// long for its index.
    pub(super) fn insert_all<R: Runtime, T: AsRef<[Value]>>(
        &self,
        m: &Migrating<'_, R>,
        r: RelId,
        c: u32,
        rows: impl IntoIterator<Item = T>,
    ) -> Result<(), Error> {
        let pager = m.pager;
        let ctor = stored(ctor_id(&self.schema, r, c));
        let keyed = rows.into_iter().map(|row| {
            let mut entries = Vec::new();
            for (spec, index, keys) in self.indexes.values() {
                if spec.relation != r {
                    continue;
                }
                let Some(key) = row_key(keys, index.schema(), c, row.as_ref())? else {
                    continue;
                };
                index.check_key(pager.page_size(), &key, ctor).map_err(|e| match e {
                    btree::Error::TooLong { len, max } => {
                        let rel = self.schema.relation(r);
                        let module = &self.schema.module(rel.module).name;
                        let size = index.schema().encode(&key).len();
                        let limit = max - (len - size);
                        Error::Refused(vec![format!(
                            "index `{module}::{}.{}` needs a key payload of {size} bytes; the limit is {limit} bytes",
                            rel.name, spec.clause
                        )])
                    }
                    e => e.into(),
                })?;
                entries.push((index, key));
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
