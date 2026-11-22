use std::collections::BTreeMap;

use adtb_lang::textual::key_names;
use adtb_lang::types::RelId;

use crate::exec::{Val, to_key};
use crate::heap::Refused;
use crate::key::Tid;
use crate::runtime::Runtime;

use super::eval::Stmt;
use super::{Broken, Error, Rot};

/// Rows of one key, each with its constructor and the TID of its version.
pub type KeyRows = Vec<(u32, Tid, Val)>;

impl<R: Runtime> Stmt<'_, R> {
    /// Returns the row of `rel` with key `key` the statement's reads see, with its constructor
    /// and the TID of its version.
    ///
    /// Errors:
    /// - [`Broken::DuplicateKey`] when the statement's reads see two rows of `key`.
    pub fn by_key(&self, rel: RelId, key: &Val) -> Result<Option<(u32, Tid, Val)>, Error> {
        let (prefix, mut rows) = self.keyed(rel, key)?;
        if rows.len() > 1 {
            let index = &self.db.bindings.indexes()[&self.rel(rel).key_index].index;
            return Err(Broken::DuplicateKey(index.schema().encode(&prefix)).into());
        }
        Ok(rows.pop())
    }

    /// Refuse the page of the version of `row`, a row of constructor `c` of `rel` with key `key`
    /// that the statement read and its reads no longer see, while the version is still there as
    /// read. Returns when the version changed before the refusal: the caller reads again.
    ///
    /// Errors:
    /// - The refusal of the version's page.
    /// - [`Rot::Unkeyed`] when the key index names no version of `row`.
    pub fn refuse_unseen(&self, rel: RelId, key: &Val, c: u32, row: &Val) -> Result<(), Error> {
        let r = self.rel(rel);
        let (prefix, _) = self.keyed(rel, key)?;
        let pager = &self.db.pager;
        let heap = &self.db.bindings.heaps()[&r.ctors[c as usize].heap].heap;
        for n in self.probe(r.key_index, &prefix, &r.routes(r.all_ctors()))? {
            if n.ctor != c {
                continue;
            }
            for link in heap.hot_run(pager, n.tid)? {
                let Some(v) = heap.read(pager, link.tid)? else {
                    continue;
                };
                if self.row(rel, c, &v.row)? != *row {
                    continue;
                }
                return match v.refuse_frame(pager) {
                    Refused::Page(e) => Err(e.into()),
                    Refused::Gone => Ok(()),
                };
            }
        }
        Err(Rot::Unkeyed(r.qualified.clone()).into())
    }

    /// Returns every row of `rel` with key `key` the statement's reads see, with its constructor
    /// and the TID of its version.
    pub fn all_by_key(&self, rel: RelId, key: &Val) -> Result<KeyRows, Error> {
        Ok(self.keyed(rel, key)?.1)
    }

    /// Returns `key` as the key index of `rel` stores it, with every row of `rel` of that key the
    /// statement's reads see, each version once.
    fn keyed(&self, rel: RelId, key: &Val) -> Result<(Vec<crate::key::Value>, KeyRows), Error> {
        let r = self.rel(rel);
        let parts: Vec<Val> = if r.single_key {
            vec![key.clone()]
        } else {
            let names = key_names(self.schema(), rel);
            let fs = key.fields()?;
            r.key_fields
                .iter()
                .map(|n| fs[names.iter().position(|m| m == n).expect("a key field")].clone())
                .collect()
        };
        let types = self.store.key_types(r.key_index)?;
        let prefix: Vec<crate::key::Value> = types
            .iter()
            .zip(&parts)
            .map(|(t, v)| to_key(t, v))
            .collect::<Result<_, _>>()?;
        let all = r.routes(r.all_ctors());
        let mut out = BTreeMap::new();
        for n in self.probe(r.key_index, &prefix, &all)? {
            let c = n.ctor;
            let Some((at, stored)) = self.store.fetch(r.ctors[c as usize].heap, &n)? else {
                continue;
            };
            let v = self.row(rel, c, &stored)?;
            if self.key_of(rel, &v)? == *key {
                out.insert((c, at), v);
            }
        }
        Ok((
            prefix,
            out.into_iter().map(|((c, at), v)| (c, at, v)).collect(),
        ))
    }
}
